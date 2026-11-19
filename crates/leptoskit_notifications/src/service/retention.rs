//! Pruning: deleting what the service no longer needs, so its tables grow
//! with what is live rather than with everything ever sent.
//!
//! What goes, and when, is the operator's [`Retention`]:
//!
//! - **Notifications.** A row nobody can see any more — expired, cancelled, or
//!   a silent data message past its delivery window — goes `ended_days` after
//!   it ended, and a cancelled one never before `ended_days` after the time it
//!   was scheduled for. Any other row goes `inbox_days` after it became
//!   visible, read or not. Its push deliveries go with it.
//! - **Finished deliveries** go `deliveries_days` after they finished.
//! - **Send records** go `sends_days` after the send was made, once no
//!   notification or delivery of the send is left.
//! - **Ended recurrences** go `sends_days` after they ended
//!   ([`recurrence::sweep`](super::recurrence::sweep)).
//!
//! What never goes, whatever its age:
//!
//! - a notification with a delivery still in the queue, or whose send is still
//!   fanning out — the row is what an attempt reads and what keeps a replayed
//!   fanout batch from writing a second copy;
//! - a delivery still in the queue;
//! - a send record that is still fanning out, or that any notification or
//!   delivery still names — the record is what answers a repeated idempotency
//!   key, and a key whose rows remain could never be sent again cleanly.
//!
//! Every one of those conditions only ever moves one way: a finished
//! delivery stays finished, a send that stopped fanning out never starts
//! again, an expiry that has passed stays passed. So a batch is checked with
//! plain reads and deleted in one short write, with nothing to race.
//!
//! The work is a sweep in primary-key order. Row ids are UUID v7, so every
//! row old enough to matter sorts before an id made from the cutoff's time,
//! and a sweep reads only that range, a batch at a time, each batch its own
//! short write. The hourly [`prune`](crate::prune) job starts a sweep and
//! works in steps of [`STEP_BUDGET`]; a step that runs out hands its place
//! to the next ([`prune_rows`](crate::prune_rows)), and a sweep still going
//! when the next hour's begins ends there, so two never overlap for long.

use leptoskit_db::Db;
use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use sha2::Digest as _;
use sha2::Sha256;
use uuid::Uuid;

pub use super::config::Retention;
use crate::AppSend;
use crate::Notification;
use crate::push::DeliveryState;
use crate::push::PushDelivery;

/// How many notifications or deliveries one write deletes at most.
const BATCH: usize = crate::FANOUT_BATCH;

/// How many send records one read considers. A record holds up to
/// [`MAX_SPEC_LEN`](crate::MAX_SPEC_LEN) of spec, so a page of them is kept
/// smaller than a page of rows.
const SEND_BATCH: usize = 100;

/// How long one job step keeps starting batches before it hands over to
/// its successor: well inside the job's timeout, and short enough that a
/// worker is never held for long.
const STEP_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);

/// How long after its start a sweep may still hand over. The next hour's
/// sweep starts from the beginning, so one still going by then stops.
const SWEEP_WINDOW_SECS: i64 = 55 * 60;

/// How many rows a pruning pass deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pruned {
	/// Inbox rows: ended ones and old ones.
	pub notifications: u64,
	/// Push deliveries: finished ones past their age, and the deliveries of
	/// every notification the pass deleted.
	pub deliveries: u64,
	/// Send records.
	pub sends: u64,
}

impl Pruned {
	fn add(&mut self, other: Self) {
		self.notifications += other.notifications;
		self.deliveries += other.deliveries;
		self.sends += other.sends;
	}

	fn is_empty(&self) -> bool {
		*self == Self::default()
	}
}

/// Which table a sweep is in. They go in this order: notifications first,
/// so the send records they named are free by the time the sweep reaches
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
	Notifications,
	Deliveries,
	Sends,
}

impl Stage {
	/// The first stage of a sweep.
	pub(crate) const FIRST: Self = Self::Notifications;

	/// The name a job's arguments carry.
	pub(crate) fn as_str(self) -> &'static str {
		match self {
			Self::Notifications => "notifications",
			Self::Deliveries => "deliveries",
			Self::Sends => "sends",
		}
	}

	/// The stage a job's arguments name.
	pub(crate) fn from_str_value(value: &str) -> Option<Self> {
		match value {
			"notifications" => Some(Self::Notifications),
			"deliveries" => Some(Self::Deliveries),
			"sends" => Some(Self::Sends),
			_ => None,
		}
	}

	fn next(self) -> Option<Self> {
		match self {
			Self::Notifications => Some(Self::Deliveries),
			Self::Deliveries => Some(Self::Sends),
			Self::Sends => None,
		}
	}
}

/// Prunes everything `retention` lets go of as of `now`, in batches, and
/// answers how much went: one whole sweep, run to its end. The hourly
/// [`prune`](crate::prune) job does the same work in bounded steps; this
/// is for an embedded service that keeps its own schedule, and for tests,
/// which pass a `now` in the future rather than wait.
///
/// # Errors
///
/// Whatever a read or a write fails with. What was deleted before it stays
/// deleted; running it again picks up what is left.
pub async fn prune(db: &Db, retention: &Retention, now: Timestamp) -> rpc::Result<Pruned> {
	// Ended recurrences go with the send records, by the same setting.
	// They are not counted in the answer, which counts the three tables a
	// sweep walks.
	super::recurrence::sweep(db, retention, now).await?;

	let mut pruned = Pruned::default();
	let mut stage = Some(Stage::FIRST);
	let mut after: Option<String> = None;

	while let Some(current) = stage {
		let (next, batch) = batch(db, retention, now, current, after.as_deref()).await?;
		pruned.add(batch);

		if next.is_none() {
			stage = current.next();
		}

		after = next;
	}

	Ok(pruned)
}

/// One step of a sweep, the body of the [`prune`](crate::prune) and
/// [`prune_rows`](crate::prune_rows) jobs: batches of `stage` from `after`
/// on, and of the stages after it, until the sweep ends or the step's
/// budget is spent — then its successor is enqueued to go on from there.
///
/// `sweep` is when the sweep began, in Unix seconds. A step that finds the
/// next hour's sweep due ends without a successor: that sweep starts over
/// from the oldest rows, and whatever this one had not reached is in it.
///
/// # Errors
///
/// Whatever a read, a write, or the enqueue fails with. The job framework
/// retries the step; a batch already deleted is simply not found again.
pub(crate) async fn step(
	cx: &rpc::JobCx,
	sweep: i64,
	stage: Stage,
	after: Option<String>,
) -> rpc::Result<()> {
	step_within(cx, sweep, stage, after, STEP_BUDGET).await
}

/// [`step`], handing over once `budget` is spent. A step always does one
/// batch before it looks at the clock, so every step of a sweep moves it
/// on, however small its budget.
async fn step_within(
	cx: &rpc::JobCx,
	sweep: i64,
	stage: Stage,
	after: Option<String>,
	budget: std::time::Duration,
) -> rpc::Result<()> {
	let db = cx.db();
	let retention = cx.state::<Retention>().copied().unwrap_or_default();
	let started = std::time::Instant::now();
	let mut pruned = Pruned::default();
	let mut stage = stage;
	let mut after = after;

	loop {
		let now = leptoskit_db::now();

		if now.timestamp() >= sweep.saturating_add(SWEEP_WINDOW_SECS) {
			break;
		}

		let (next, batch) = batch(db, &retention, now, stage, after.as_deref()).await?;
		pruned.add(batch);

		if next.is_none() {
			let Some(following) = stage.next() else {
				break;
			};

			stage = following;
		}

		after = next;

		if started.elapsed() >= budget {
			rpc::enqueue(
				db,
				crate::PruneRows {
					sweep,
					stage: stage.as_str().to_owned(),
					after: after.clone(),
				},
			)
			.at(now)
			.key(step_key(sweep, stage, after.as_deref()))
			.await?;

			break;
		}
	}

	if !pruned.is_empty() {
		cx.progress(format!(
			"pruned {} notifications, {} deliveries, {} send records",
			pruned.notifications, pruned.deliveries, pruned.sends
		));
	}

	Ok(())
}

/// The key of the step that goes on from `after` in `stage`: one per place
/// in one sweep, so a retried step that enqueues its successor again finds
/// it there.
fn step_key(sweep: i64, stage: Stage, after: Option<&str>) -> String {
	let place = after.map_or_else(
		|| "start".to_owned(),
		|after| hex::encode(&Sha256::digest(after.as_bytes())[..16]),
	);

	format!("prune:{sweep}:{}:{place}", stage.as_str())
}

/// One batch of one stage, from `after`: what it deleted, and the cursor
/// the next batch of the stage starts after — `None` when the stage has
/// nothing more.
async fn batch(
	db: &Db,
	retention: &Retention,
	now: Timestamp,
	stage: Stage,
	after: Option<&str>,
) -> rpc::Result<(Option<String>, Pruned)> {
	// A cursor that doesn't parse is a cursor from another build's job:
	// the stage starts over, which only costs reads.
	let id = after.and_then(|after| Uuid::parse_str(after).ok());

	Ok(match stage {
		Stage::Notifications => {
			let (next, pruned) = notifications(db, retention, now, id).await?;
			(next.map(|next| next.to_string()), pruned)
		}
		Stage::Deliveries => {
			let (next, pruned) = deliveries(db, retention, now, id).await?;
			(next.map(|next| next.to_string()), pruned)
		}
		Stage::Sends => sends(db, retention, now, after).await?,
	})
}

/// `days` before `now`, or `None` when that kind of row is kept forever.
fn cutoff(now: Timestamp, days: Option<u32>) -> Option<Timestamp> {
	days.and_then(|days| now.checked_sub_signed(chrono::Duration::days(i64::from(days))))
}

/// An id every UUID v7 made before `at` sorts before, and none made at or
/// after it does: the time's milliseconds in the id's leading 48 bits and
/// nothing after them. A sweep reads only the ids below it, so it never
/// touches a row too young to prune.
fn id_before(at: Timestamp) -> Uuid {
	let millis = u128::try_from(at.timestamp_millis()).unwrap_or(0);

	Uuid::from_u128((millis & 0xFFFF_FFFF_FFFF) << 80)
}

/// A delivery still waiting in the queue.
fn queued(delivery: &<PushDelivery as leptoskit_db::Table>::Columns) -> leptoskit_db::Expr {
	delivery.state.in_list([
		DeliveryState::Enqueued.as_str(),
		DeliveryState::Retrying.as_str(),
	])
}

/// A delivery that left the queue.
fn finished(delivery: &<PushDelivery as leptoskit_db::Table>::Columns) -> leptoskit_db::Expr {
	delivery.state.ne(DeliveryState::Enqueued.as_str())
		& delivery.state.ne(DeliveryState::Retrying.as_str())
}

/// One batch of notifications: the next rows after `after` that have
/// outlived their retention, less the ones the service still works with,
/// deleted with their deliveries in one write.
async fn notifications(
	db: &Db,
	retention: &Retention,
	now: Timestamp,
	after: Option<Uuid>,
) -> rpc::Result<(Option<Uuid>, Pruned)> {
	let ended = cutoff(now, retention.ended_days);
	let inbox = cutoff(now, retention.inbox_days);

	// Every row either rule lets go of was written before the later of the
	// two cutoffs: nothing ends, or becomes visible, before it exists.
	let Some(newest) = ended.max(inbox) else {
		return Ok((None, Pruned::default()));
	};
	let bound = id_before(newest);
	// A data row is never visible, so it has no inbox age: it ends when its
	// push can no longer go out, a delivery window after its time.
	let silent = ended.and_then(|ended| {
		ended.checked_sub_signed(chrono::Duration::seconds(
			crate::push::DELIVERY_HORIZON_SECS,
		))
	});

	let rows = Notification::query()
		.filter(|notification| {
			let mut gone: Option<leptoskit_db::Expr> = None;
			let mut or = |rule: leptoskit_db::Expr| {
				gone = Some(match gone.take() {
					Some(gone) => gone | rule,
					None => rule,
				});
			};

			if let Some(ended) = ended {
				// A cancelled row stays until its own time is as far back
				// too: the job that would have shown it waits until then
				// under the send's key, and the send's record has to
				// outlast it.
				or(notification.cancelled_at.lte(ended) & notification.visible_at.lte(ended));
				or(notification.expires_at.lte(ended));
			}

			if let Some(silent) = silent {
				or(notification
					.kind
					.eq(crate::options::MessageKind::Data.as_str())
					& notification.visible_at.lte(silent));
			}

			if let Some(inbox) = inbox {
				or(notification.visible_at.lte(inbox));
			}

			let mut range = notification.id.lt(bound);

			if let Some(after) = after {
				range = range & notification.id.gt(after);
			}

			match gone {
				Some(gone) => range & gone,
				None => range,
			}
		})
		.order_by(|notification| notification.id.asc())
		.limit(u64::try_from(BATCH).unwrap_or(u64::MAX))
		.all(db)
		.await?;

	let Some(last) = rows.last().map(|row| row.id) else {
		return Ok((None, Pruned::default()));
	};
	let next = (rows.len() >= BATCH).then_some(last);

	// What the service still works with stays, whatever its age: a row with
	// a delivery in the queue, and a row of a send still fanning out.
	let waiting: std::collections::BTreeSet<Uuid> = PushDelivery::query()
		.filter(|delivery| {
			delivery.notification.in_list(rows.iter().map(|row| row.id)) & queued(delivery)
		})
		.all(db)
		.await?
		.into_iter()
		.map(|delivery| delivery.notification)
		.collect();
	let keys: std::collections::BTreeSet<String> =
		rows.iter().map(|row| row.send_key.clone()).collect();
	let fanning: std::collections::BTreeSet<String> = AppSend::query()
		.filter(|send| {
			send.idempotency_key.in_list(keys)
				& send.state.eq(crate::SendState::FanningOut.as_str())
		})
		.all(db)
		.await?
		.into_iter()
		.map(|send| send.idempotency_key)
		.collect();
	let doomed: Vec<Uuid> = rows
		.iter()
		.filter(|row| !waiting.contains(&row.id) && !fanning.contains(&row.send_key))
		.map(|row| row.id)
		.collect();

	if doomed.is_empty() {
		return Ok((next, Pruned::default()));
	}

	// Deliveries first, as an erasure does it: were the rows to go first and
	// the write after them fail, nothing would name these deliveries again.
	let tx = db.begin().await?;
	let deliveries = PushDelivery::query()
		.filter(|delivery| delivery.notification.in_list(doomed.iter().copied()))
		.delete(&tx)
		.await?;
	let notifications = Notification::query()
		.filter(|notification| notification.id.in_list(doomed.iter().copied()))
		.delete(&tx)
		.await?;
	tx.commit().await?;

	Ok((
		next,
		Pruned {
			notifications,
			deliveries,
			sends: 0,
		},
	))
}

/// One batch of deliveries: the next finished ones after `after` that are
/// older than their retention, deleted in one write.
async fn deliveries(
	db: &Db,
	retention: &Retention,
	now: Timestamp,
	after: Option<Uuid>,
) -> rpc::Result<(Option<Uuid>, Pruned)> {
	let Some(cutoff) = cutoff(now, retention.deliveries_days) else {
		return Ok((None, Pruned::default()));
	};
	let bound = id_before(cutoff);

	let rows = PushDelivery::query()
		.filter(|delivery| {
			let mut condition =
				delivery.id.lt(bound) & finished(delivery) & delivery.finished_at.lte(cutoff);

			if let Some(after) = after {
				condition = condition & delivery.id.gt(after);
			}

			condition
		})
		.order_by(|delivery| delivery.id.asc())
		.limit(u64::try_from(BATCH).unwrap_or(u64::MAX))
		.all(db)
		.await?;

	let Some(last) = rows.last().map(|row| row.id) else {
		return Ok((None, Pruned::default()));
	};

	let deliveries = PushDelivery::query()
		.filter(|delivery| delivery.id.in_list(rows.iter().map(|row| row.id)) & finished(delivery))
		.delete(db)
		.await?;

	Ok((
		(rows.len() >= BATCH).then_some(last),
		Pruned {
			notifications: 0,
			deliveries,
			sends: 0,
		},
	))
}

/// One batch of send records: the next ones after `after`, in key order,
/// that are older than their retention, no longer fanning out, and named
/// by no notification and no delivery, deleted in one write.
async fn sends(
	db: &Db,
	retention: &Retention,
	now: Timestamp,
	after: Option<&str>,
) -> rpc::Result<(Option<String>, Pruned)> {
	let Some(cutoff) = cutoff(now, retention.sends_days) else {
		return Ok((None, Pruned::default()));
	};
	let fanning = crate::SendState::FanningOut.as_str();

	let rows = AppSend::query()
		.filter(|send| {
			let mut condition = send.created_at.lte(cutoff) & send.state.ne(fanning);

			if let Some(after) = after {
				condition = condition & send.idempotency_key.gt(after.to_owned());
			}

			condition
		})
		.order_by(|send| send.idempotency_key.asc())
		.limit(u64::try_from(SEND_BATCH).unwrap_or(u64::MAX))
		.all(db)
		.await?;

	let Some(last) = rows.last().map(|row| row.idempotency_key.clone()) else {
		return Ok((None, Pruned::default()));
	};
	let next = (rows.len() >= SEND_BATCH).then_some(last);
	let mut free = Vec::with_capacity(rows.len());

	// The record answers for its rows: while one notification or delivery
	// still names the key, the key can't be sent again cleanly, so the
	// record that refuses it stays.
	for row in rows {
		let key = row.idempotency_key;

		if Notification::query()
			.filter(|notification| notification.send_key.eq(key.clone()))
			.exists(db)
			.await?
			|| PushDelivery::query()
				.filter(|delivery| delivery.send_key.eq(key.clone()))
				.exists(db)
				.await?
		{
			continue;
		}

		free.push(key);
	}

	if free.is_empty() {
		return Ok((next, Pruned::default()));
	}

	let sends = AppSend::query()
		.filter(|send| send.idempotency_key.in_list(free) & send.state.ne(fanning))
		.delete(db)
		.await?;

	Ok((
		next,
		Pruned {
			notifications: 0,
			deliveries: 0,
			sends,
		},
	))
}

#[cfg(test)]
mod tests {
	use leptoskit_db::Read as _;

	use super::*;

	/// An id minted at the start of the day `days` ago, the `n`th of its
	/// moment: the same id however often it is asked for.
	fn id_from(days: i64, n: u128) -> Uuid {
		use chrono::DurationRound as _;

		let at = (leptoskit_db::now() - chrono::Duration::days(days))
			.duration_trunc(chrono::Duration::days(1))
			.expect("a day truncates");

		Uuid::from_u128(
			(u128::try_from(at.timestamp_millis()).unwrap() << 80) | (0x7 << 76) | (0x8 << 60) | n,
		)
	}

	/// A migrated database holding `count` notifications that expired forty
	/// days ago.
	async fn with_expired(count: u128) -> Db {
		let db = leptoskit_db::testing::sqlite_migrated(crate::MIGRATIONS).await;
		let stamp = |days: i64| {
			(leptoskit_db::now() - chrono::Duration::days(days))
				.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
		};
		let rows: Vec<String> = (0..count)
			.map(|n| {
				format!(
					"(x'{}', 'old-news', x'{}', 'news', 't', '', '{{}}', '{}', '{}', '{}')",
					id_from(50, n).simple(),
					Uuid::from_u128(n + 1).simple(),
					stamp(50),
					stamp(40),
					stamp(50),
				)
			})
			.collect();

		for chunk in rows.chunks(250) {
			db.fetch(leptoskit_db::Statement::new(
				format!(
					"INSERT INTO notifications (id, send_key, person, category, title, body, \
					 data, visible_at, expires_at, created_at) VALUES {}",
					chunk.join(", ")
				),
				Vec::new(),
			))
			.await
			.expect("the rows insert");
		}

		db
	}

	async fn left(db: &Db) -> u64 {
		Notification::query().count(db).await.expect("a count")
	}

	fn cx(db: &Db) -> rpc::JobCx {
		rpc::JobCx::new(
			db.system("run a pruning step in a test"),
			rpc::server::Identity::anonymous(),
			"test:prune".to_owned(),
			1,
		)
	}

	/// A step whose budget is spent after its first batch deletes that
	/// batch, leaves exactly one successor holding its place, and the
	/// successors finish the sweep.
	#[tokio::test(flavor = "multi_thread")]
	async fn a_step_out_of_budget_hands_its_place_to_the_next() {
		let db = with_expired(1_100).await;
		let sweep = leptoskit_db::now().timestamp();

		step_within(
			&cx(&db),
			sweep,
			Stage::FIRST,
			None,
			std::time::Duration::ZERO,
		)
		.await
		.expect("the step runs");

		assert_eq!(left(&db).await, 600, "one batch went");

		let waiting = rpc::server::jobs::JobRow::query()
			.filter(|job| job.name.eq("prune_rows".to_owned()))
			.all(&db.system("read the jobs in a test"))
			.await
			.expect("the jobs read");
		assert_eq!(waiting.len(), 1, "one successor: {waiting:?}");
		assert_eq!(
			waiting[0].key.as_deref(),
			Some(
				step_key(
					sweep,
					Stage::Notifications,
					Some(&id_from(50, 499).to_string())
				)
				.as_str()
			),
			"keyed by the place it goes on from"
		);

		rpc::testing::TestJobs::new(&db)
			.service(Some(crate::SERVICE))
			.run_jobs_until_idle()
			.await;

		assert_eq!(left(&db).await, 0, "the successors finish the sweep");
	}

	/// A step of a sweep that began an hour ago does nothing and leaves no
	/// successor: the next hour's sweep has it.
	#[tokio::test(flavor = "multi_thread")]
	async fn a_step_of_a_stale_sweep_ends_it() {
		let db = with_expired(3).await;

		step(
			&cx(&db),
			leptoskit_db::now().timestamp() - 3_600,
			Stage::FIRST,
			None,
		)
		.await
		.expect("the step runs");

		assert_eq!(left(&db).await, 3, "nothing went");
		assert!(
			rpc::server::jobs::JobRow::query()
				.all(&db.system("read the jobs in a test"))
				.await
				.expect("the jobs read")
				.is_empty(),
			"and nothing was enqueued"
		);

		step(
			&cx(&db),
			leptoskit_db::now().timestamp(),
			Stage::FIRST,
			None,
		)
		.await
		.expect("the step runs");
		assert_eq!(left(&db).await, 0, "a sweep in its own hour prunes");
	}

	/// The bound sorts after every id minted before its time and not after
	/// any minted at or after it.
	#[test]
	fn the_id_bound_splits_ids_at_its_time() {
		let at = chrono::DateTime::from_timestamp_millis(1_790_000_000_000).unwrap();
		let bound = id_before(at);
		let minted = |millis: u64, rest: u128| {
			Uuid::from_u128((u128::from(millis) << 80) | (0x7 << 76) | rest)
		};

		assert!(minted(1_789_999_999_999, (1 << 76) - 1) < bound);
		assert!(minted(1_790_000_000_000, 0) >= bound);
		assert!(minted(1_790_000_000_001, 0) > bound);
		assert!(
			Uuid::now_v7() > bound,
			"an id made now is after a past time's bound"
		);
	}

	#[test]
	fn a_step_key_names_one_place_in_one_sweep() {
		assert_eq!(
			step_key(7, Stage::Notifications, None),
			"prune:7:notifications:start"
		);
		assert_ne!(
			step_key(7, Stage::Sends, Some("a")),
			step_key(7, Stage::Sends, Some("b"))
		);
		assert!(step_key(7, Stage::Sends, Some(&"k".repeat(255))).len() < 255);

		for stage in [Stage::Notifications, Stage::Deliveries, Stage::Sends] {
			assert_eq!(Stage::from_str_value(stage.as_str()), Some(stage));
		}
	}
}
