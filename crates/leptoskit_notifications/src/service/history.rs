//! The bodies of the send history: the page of sends, and the parts of a
//! send's report that describe the send and count its pushes and reads.

use leptoskit_db::Statement;
use leptoskit_rpc as rpc;

use crate::AppSend;
use crate::Notification;
use crate::history::HourCount;
use crate::history::MAX_PINNED_SENDS;
use crate::history::MAX_REPORT_HOURS;
use crate::history::MAX_SENDS_PAGE;
use crate::history::PushCounts;
use crate::history::ReasonCount;
use crate::history::SendEntry;
use crate::history::SendFilter;
use crate::history::TargetKind;
use crate::history::TargetSummary;
use crate::push::DeliveryReason;
use crate::push::DeliveryState;
use crate::push::PushDelivery;
use crate::push::PushPlatform;

/// One page of the history, newest first.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] for a cursor that isn't one of this list's, and
/// whatever the read fails with.
pub(crate) async fn list(
	db: &impl leptoskit_db::Read,
	page: &rpc::PageArgs,
	filter: &SendFilter,
) -> rpc::Result<rpc::Page<SendEntry>> {
	let mut page = page.clone();
	page.limit = page.limit.clamp(1, MAX_SENDS_PAGE);

	let mut query = AppSend::query().filter(|send| send.history_id.is_not_null());

	if let Some(state) = filter.state {
		query = query.filter(|send| send.state.eq(state.as_str().to_owned()));
	}

	let rows = super::paginate_bounded(
		db,
		query,
		&page,
		MAX_PINNED_SENDS,
		|send| send.history_id.desc(),
		|send| send.history_id.unwrap_or_default(),
	)
	.await?;

	Ok(rpc::Page {
		items: rows.items.iter().map(entry).collect(),
		end: rows.end,
		has_more: rows.has_more,
	})
}

/// The spec a send's record stores, when it stores one that still parses.
/// A send from before the history stored none for a recipients send.
pub(crate) fn spec_of(send: &AppSend) -> Option<crate::SendSpec> {
	send.spec
		.as_deref()
		.and_then(|stored| serde_json::from_str(stored).ok())
}

/// Who the send was for, from its record: a missing target is a
/// recipients send, whose record keeps only how many people it named.
pub(crate) fn target_of(send: &AppSend) -> TargetSummary {
	let stored = send
		.target
		.as_deref()
		.and_then(|stored| crate::SendTarget::from_stored(stored).ok());

	match stored {
		Some(crate::SendTarget::Topic(topic)) => {
			TargetSummary {
				kind: TargetKind::Topic,
				text: Some(topic),
				recipients: 0,
			}
		}
		Some(crate::SendTarget::Condition(condition)) => {
			TargetSummary {
				kind: TargetKind::Condition,
				text: Some(condition),
				recipients: 0,
			}
		}
		Some(crate::SendTarget::Everyone) => {
			TargetSummary {
				kind: TargetKind::Everyone,
				text: None,
				recipients: 0,
			}
		}
		Some(crate::SendTarget::Recipients(_)) | None => {
			TargetSummary {
				kind: TargetKind::Recipients,
				text: None,
				recipients: u32::try_from(send.targeted).unwrap_or(u32::MAX),
			}
		}
	}
}

/// The record as the list answers it.
fn entry(send: &AppSend) -> SendEntry {
	let spec = spec_of(send).unwrap_or_default();
	let progress = super::progress_of(send);

	SendEntry {
		key: send.idempotency_key.clone(),
		created_at: send.created_at,
		state: progress.state,
		label: send.label.clone(),
		category: send.category.clone().unwrap_or(spec.category),
		title: spec.title,
		kind: spec.options.kind,
		target: target_of(send),
		visible_at: spec.visible_at,
		local_time: spec.local_time,
		expires_at: spec.expires_at,
		targeted: progress.targeted,
		delivered: progress.delivered,
		skipped_opted_out: progress.skipped_opted_out,
	}
}

/// The read behind a report's push breakdown: the send's deliveries,
/// found through an index that leads with `send_key`, grouped by
/// platform, state, and reason — at most a few dozen groups, however
/// large the send.
pub(crate) fn push_counts_read(dialect: leptoskit_db::Dialect, send_key: &str) -> Statement {
	Statement::new(
		format!(
			"SELECT \"platform\", \"state\", \"reason\", count(*) FROM \"push_deliveries\" WHERE \
			 \"send_key\" = {} GROUP BY \"platform\", \"state\", \"reason\"",
			super::placeholder(dialect, 1)
		),
		vec![leptoskit_db::Value::Text(send_key.to_owned())],
	)
}

/// The send's stored push deliveries by platform and state, with why the
/// ended ones ended. Platforms in a fixed order; a platform the send has
/// no delivery on is left out.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn push_counts(
	db: &impl leptoskit_db::Read,
	send_key: &str,
) -> rpc::Result<Vec<PushCounts>> {
	let rows = push_counts_read(db.dialect(), send_key)
		.reads::<PushDelivery>()
		.fetch(db)
		.await?;
	let mut counts: Vec<PushCounts> = Vec::new();

	for row in rows {
		let (Some(platform), Some(state), count) = (
			text_of(row.first()).and_then(PushPlatform::from_str_value),
			text_of(row.get(1)).and_then(DeliveryState::from_str_value),
			super::count_of(row.get(3)),
		) else {
			continue;
		};
		let reason = text_of(row.get(2)).and_then(DeliveryReason::from_str_value);
		let index = counts
			.iter()
			.position(|held| held.platform == platform)
			.unwrap_or_else(|| {
				counts.push(PushCounts {
					platform,
					enqueued: 0,
					retrying: 0,
					accepted: 0,
					rejected: 0,
					skipped: 0,
					reasons: Vec::new(),
				});
				counts.len() - 1
			});
		let held = &mut counts[index];
		let slot = match state {
			DeliveryState::Enqueued => &mut held.enqueued,
			DeliveryState::Retrying => &mut held.retrying,
			DeliveryState::Accepted => &mut held.accepted,
			DeliveryState::Rejected => &mut held.rejected,
			DeliveryState::Skipped => &mut held.skipped,
		};
		*slot = slot.saturating_add(count);

		if let Some(reason) = reason
			&& matches!(state, DeliveryState::Rejected | DeliveryState::Skipped)
		{
			match held.reasons.iter_mut().find(|held| held.reason == reason) {
				Some(held) => held.count = held.count.saturating_add(count),
				None => held.reasons.push(ReasonCount { reason, count }),
			}
		}
	}

	counts.sort_by_key(|held| held.platform.as_str());

	for held in &mut counts {
		held.reasons.sort_by(|left, right| {
			right
				.count
				.cmp(&left.count)
				.then_with(|| left.reason.as_str().cmp(right.reason.as_str()))
		});
	}

	Ok(counts)
}

/// The read behind a report's reads by hour: the send's read rows, found
/// through the `(send_key, person)` index, grouped by the UTC hour they
/// were read in, the earliest [`MAX_REPORT_HOURS`] hours that have any.
pub(crate) fn reads_by_hour_read(dialect: leptoskit_db::Dialect, send_key: &str) -> Statement {
	// The hour as `YYYY-MM-DDTHH`, which sorts as time does. SQLite stores
	// a timestamp as UTC text in exactly that shape; Postgres is asked for
	// it.
	let hour = match dialect {
		leptoskit_db::Dialect::Sqlite => "substr(\"read_at\", 1, 13)",
		leptoskit_db::Dialect::Postgres => {
			"to_char(\"read_at\" AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24')"
		}
	};

	Statement::new(
		format!(
			"SELECT {hour} AS \"hour\", count(*) FROM \"notifications\" WHERE \"send_key\" = {} \
			 AND \"read_at\" IS NOT NULL GROUP BY 1 ORDER BY 1 ASC LIMIT {MAX_REPORT_HOURS}",
			super::placeholder(dialect, 1)
		),
		vec![leptoskit_db::Value::Text(send_key.to_owned())],
	)
}

/// When the send's stored rows were read, by the hour: the earliest
/// [`MAX_REPORT_HOURS`] hours in which any was.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn reads_by_hour(
	db: &impl leptoskit_db::Read,
	send_key: &str,
) -> rpc::Result<Vec<HourCount>> {
	let rows = reads_by_hour_read(db.dialect(), send_key)
		.reads::<Notification>()
		.fetch(db)
		.await?;

	Ok(rows
		.iter()
		.filter_map(|row| {
			let hour = text_of(row.first())?;
			let hour = chrono::DateTime::parse_from_rfc3339(&format!("{hour}:00:00Z")).ok()?;

			Some(HourCount {
				hour: hour.with_timezone(&chrono::Utc),
				read: super::count_of(row.get(1)),
			})
		})
		.collect())
}

/// A text column's value.
fn text_of(value: Option<&leptoskit_db::Value>) -> Option<&str> {
	match value {
		Some(leptoskit_db::Value::Text(text)) => Some(text),
		_ => None,
	}
}
