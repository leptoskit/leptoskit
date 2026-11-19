//! Counting what organisations use.
//!
//! Usage is counted by the control plane, from what it sees itself, into
//! one row per organisation, project, calendar month (UTC), and meter:
//!
//! - **Build minutes.** When a build worker finishes an attempt at a build, the
//!   time from its claim to its finish, by the control plane's clock, rounded
//!   up to a whole minute. An attempt whose worker went quiet is not counted:
//!   nobody should pay for a worker that died.
//! - **Compute seconds.** When a cluster's worker reports what an environment
//!   runs, each process's ready replicas times the seconds since the report
//!   before, at the size the live deployment planned for it. A gap longer than
//!   [`MAX_SAMPLE`] counts as [`MAX_SAMPLE`]: a worker that was away isn't
//!   billed for by guessing what ran meanwhile.
//!
//! Every count is written in the transaction that records what it counts,
//! so a report or a finish that is retried is never counted twice.
//!
//! Database and object storage, egress, dedicated database hours, macOS
//! build minutes, and custom hostnames aren't counted yet.

use leptoskit_cloud_runtime::Observed;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use uuid::Uuid;

use crate::pricing::Meter;
use crate::pricing::Usage;
use crate::tables::Deployment;
use crate::tables::Environment;
use crate::tables::Job;
use crate::tables::NewUsageTotal;
use crate::tables::Organization;
use crate::tables::Project;
use crate::tables::UsageTotal;

/// The longest stretch one status report is counted for.
pub(crate) const MAX_SAMPLE: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

/// The most replicas of one process a report is counted for: far more than
/// any size's `max`, so a report's arithmetic can't run away.
const MAX_REPLICAS: u64 = 1_000;

/// The calendar month `at` is in, UTC, as `2026-11`.
pub(crate) fn period(at: Timestamp) -> String {
	at.format("%Y-%m").to_string()
}

/// Whether `text` is a month as [`period`] writes one.
pub(crate) fn is_period(text: &str) -> bool {
	let bytes = text.as_bytes();
	bytes.len() == 7
		&& bytes[4] == b'-'
		&& bytes[..4].iter().all(u8::is_ascii_digit)
		&& bytes[5..].iter().all(u8::is_ascii_digit)
		&& matches!(text[5..].parse::<u8>(), Ok(1..=12))
}

/// The project ID usage is kept under when its project was gone before it
/// was counted.
pub(crate) fn no_project() -> Id<Project> {
	Id::from_uuid(Uuid::nil())
}

/// Adds `quantity` of `meter` to what `project` used this month.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn record(
	tx: &impl leptoskit_db::Connection,
	organization: Id<Organization>,
	project: Id<Project>,
	meter: Meter,
	quantity: u64,
) -> leptoskit_db::Result<()> {
	if quantity == 0 {
		return Ok(());
	}

	let quantity = i64::try_from(quantity).unwrap_or(i64::MAX);
	let month = period(leptoskit_db::now());
	let code = meter.code();

	if let Some(mut row) = UsageTotal::query()
		.filter(|row| {
			row.organization_id.eq(organization)
				& row.project_id.eq(project)
				& row.period.eq(month.as_str())
				& row.meter.eq(code)
		})
		.first(tx)
		.await?
	{
		row.quantity = row.quantity.saturating_add(quantity);
		row.save(tx).await?;
	} else {
		NewUsageTotal {
			organization_id: organization,
			project_id: project,
			period: month,
			meter: code.to_owned(),
			quantity,
		}
		.insert(tx)
		.await?;
	}

	Ok(())
}

/// What `organization` used in `month`: its rows.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn rows(
	db: &impl leptoskit_db::Read,
	organization: Id<Organization>,
	month: &str,
) -> leptoskit_db::Result<Vec<UsageTotal>> {
	UsageTotal::query()
		.filter(|row| row.organization_id.eq(organization) & row.period.eq(month))
		.order_by(|row| row.project_id.asc())
		.order_by(|row| row.meter.asc())
		.all(db)
		.await
}

/// `rows` added up per meter. A row whose meter this build doesn't know is
/// left out.
pub(crate) fn total(rows: &[UsageTotal]) -> Usage {
	let mut usage = Usage::default();

	for row in rows {
		if let Ok(meter) = row.meter.parse::<Meter>() {
			usage.add(meter, u64::try_from(row.quantity).unwrap_or_default());
		}
	}

	usage
}

/// The whole minutes a build attempt claimed at `claimed` and finished at
/// `finished` is counted as: at least one.
fn build_minutes(claimed: Timestamp, finished: Timestamp) -> u64 {
	let seconds = u64::try_from((finished - claimed).num_seconds()).unwrap_or_default();
	seconds.div_ceil(60).max(1)
}

/// Counts the attempt at the build `job` that its worker has just
/// finished, however it ended.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn build_finished(
	tx: &impl leptoskit_db::Connection,
	job: &Job,
) -> leptoskit_db::Result<()> {
	let Some(claimed) = job.claimed_at else {
		return Ok(());
	};
	let project = match job.deployment_id {
		Some(id) => {
			Deployment::query()
				.filter(|row| row.id.eq(id))
				.first(tx)
				.await?
				.map(|row| row.project_id)
		}
		None => None,
	};
	record(
		tx,
		job.organization_id,
		project.unwrap_or_else(no_project),
		Meter::LinuxBuildMinutes,
		build_minutes(claimed, leptoskit_db::now()),
	)
	.await
}

/// The seconds a report at `now` is counted for, after one at `previous`.
fn sample_seconds(previous: Timestamp, now: Timestamp) -> u64 {
	u64::try_from((now - previous).min(MAX_SAMPLE).num_seconds()).unwrap_or_default()
}

/// Counts what `observed` says `environment` ran since its worker's
/// report at `previous`: nothing for a first report, and nothing for a
/// process the live deployment didn't plan.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn observed(
	tx: &impl leptoskit_db::Connection,
	project: &Project,
	environment: &Environment,
	previous: Timestamp,
	now: Timestamp,
	observed: &Observed,
) -> leptoskit_db::Result<()> {
	let seconds = sample_seconds(previous, now);

	if seconds == 0 {
		return Ok(());
	}

	let environment_id = environment.id;
	let Some(live) = Deployment::query()
		.filter(|row| row.live_environment.eq(Some(environment_id)))
		.first(tx)
		.await?
	else {
		return Ok(());
	};
	let mut usage = Usage::default();

	for process in &observed.processes {
		let Some(planned) = live
			.plan
			.0
			.processes
			.iter()
			.find(|planned| planned.name == process.name)
		else {
			continue;
		};
		// Never more than the release may run, whatever the report says.
		let replicas = u64::from(process.ready_replicas)
			.min(u64::from(planned.max.max(planned.min)))
			.min(MAX_REPLICAS);
		usage.add(
			Meter::Compute(planned.size),
			replicas.saturating_mul(seconds),
		);
	}

	for (meter, quantity) in usage.iter() {
		record(tx, project.organization_id, project.id, meter, quantity).await?;
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn at(text: &str) -> Timestamp {
		text.parse().expect("a time")
	}

	#[test]
	fn a_period_is_a_utc_month() {
		assert_eq!(period(at("2026-11-30T23:59:59Z")), "2026-11");
		assert_eq!(period(at("2026-12-01T00:00:00Z")), "2026-12");
		assert!(is_period("2026-11"));
		assert!(is_period("2026-01"));

		for wrong in [
			"2026-13",
			"2026-00",
			"2026-1",
			"26-11",
			"2026/11",
			"2026-11-01",
			"",
		] {
			assert!(!is_period(wrong), "{wrong}");
		}
	}

	#[test]
	fn a_build_is_counted_in_whole_minutes_from_one() {
		let claimed = at("2026-11-18T10:00:00Z");
		assert_eq!(build_minutes(claimed, claimed), 1);
		assert_eq!(build_minutes(claimed, at("2026-11-18T10:00:59Z")), 1);
		assert_eq!(build_minutes(claimed, at("2026-11-18T10:01:00Z")), 1);
		assert_eq!(build_minutes(claimed, at("2026-11-18T10:01:01Z")), 2);
		assert_eq!(build_minutes(claimed, at("2026-11-18T12:00:00Z")), 120);
		// A clock that went backwards counts the least there is.
		assert_eq!(build_minutes(claimed, at("2026-11-18T09:00:00Z")), 1);
	}

	#[test]
	fn a_report_is_counted_for_at_most_five_minutes() {
		let previous = at("2026-11-18T10:00:00Z");
		assert_eq!(sample_seconds(previous, at("2026-11-18T10:00:30Z")), 30);
		assert_eq!(sample_seconds(previous, at("2026-11-18T10:05:00Z")), 300);
		assert_eq!(sample_seconds(previous, at("2026-11-18T18:00:00Z")), 300);
		assert_eq!(sample_seconds(previous, at("2026-11-18T09:00:00Z")), 0);
	}
}
