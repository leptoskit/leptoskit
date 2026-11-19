//! The bodies of the campaign operations: drafts saved under a revision,
//! one send per launched revision, and a record of who did what.
//!
//! Every write that depends on what the campaign is — a save, a launch,
//! an archive, a cancel — is one `UPDATE` filtered on the state and the
//! revision it expects, and reads how many rows it changed. Two callers
//! that race are told apart by the database, on both engines, and never
//! by a read made earlier in the transaction.

use leptoskit_rpc as rpc;
use uuid::Uuid;

use crate::AppSend;
use crate::SendSpec;
use crate::SendTarget;
use crate::TargetKind;
use crate::TargetSummary;
use crate::campaign::CampaignDraft;
use crate::campaign::CampaignEntry;
use crate::campaign::CampaignEvent;
use crate::campaign::CampaignEventKind;
use crate::campaign::CampaignEventRow;
use crate::campaign::CampaignFilter;
use crate::campaign::CampaignRow;
use crate::campaign::CampaignState;
use crate::campaign::LaunchedCampaign;
use crate::campaign::MAX_CAMPAIGN_EVENTS_PAGE;
use crate::campaign::MAX_CAMPAIGN_NAME_LEN;
use crate::campaign::MAX_CAMPAIGN_TESTS;
use crate::campaign::MAX_CAMPAIGNS_PAGE;
use crate::campaign::MAX_DRAFT_CAMPAIGNS;
use crate::campaign::NewCampaignEventRow;
use crate::campaign::NewCampaignRow;
use crate::campaign::SavedCampaign;

/// The longest detail an event stores, in bytes.
const MAX_DETAIL_LEN: usize = 256;

/// The stored row of campaign `id`.
///
/// # Errors
///
/// [`rpc::Error::NotFound`] for an unknown id, and whatever the read
/// fails with.
async fn row(db: &impl leptoskit_db::Read, id: Uuid) -> rpc::Result<CampaignRow> {
	CampaignRow::query()
		.filter(|campaign| campaign.id.eq(id))
		.first(db)
		.await?
		.ok_or_else(|| rpc::Error::not_found("no campaign with this id exists"))
}

/// The draft a row stores. A stored spec or target that no longer parses
/// is a bug's shape, not a caller's.
fn draft_of(row: &CampaignRow) -> rpc::Result<CampaignDraft> {
	let spec: SendSpec = serde_json::from_str(&row.spec).map_err(|error| {
		rpc::Error::server(format!("a stored campaign message won't parse: {error}"))
	})?;
	let target = row
		.target
		.as_deref()
		.map(serde_json::from_str::<SendTarget>)
		.transpose()
		.map_err(|error| {
			rpc::Error::server(format!("a stored campaign target won't parse: {error}"))
		})?;

	Ok(CampaignDraft {
		name: row.name.clone(),
		spec,
		target,
		note: row.note.clone(),
	})
}

/// The state a row stores.
fn state_of(row: &CampaignRow) -> CampaignState {
	CampaignState::from_str_value(&row.state).unwrap_or_default()
}

/// A stored revision, as callers count it.
fn revision_of(stored: i64) -> u32 {
	u32::try_from(stored).unwrap_or(u32::MAX)
}

/// The row as callers read it.
fn saved(row: &CampaignRow) -> rpc::Result<SavedCampaign> {
	Ok(SavedCampaign {
		id: row.id,
		revision: revision_of(row.revision),
		state: state_of(row),
		draft: draft_of(row)?,
		created_at: row.created_at,
		created_by: row.created_by.clone(),
		updated_at: row.updated_at,
		updated_by: row.updated_by.clone(),
		launched_at: row.launched_at,
		launched_by: row.launched_by.clone(),
		cancelled_at: row.cancelled_at,
		cancelled_by: row.cancelled_by.clone(),
		send_key: row.send_key.clone(),
	})
}

/// Who a draft's target is for, as a list shows it.
pub(crate) fn summary_of(target: &SendTarget) -> TargetSummary {
	match target {
		SendTarget::Recipients(recipients) => {
			TargetSummary {
				kind: TargetKind::Recipients,
				text: None,
				recipients: u32::try_from(recipients.len()).unwrap_or(u32::MAX),
			}
		}
		SendTarget::Topic(topic) => {
			TargetSummary {
				kind: TargetKind::Topic,
				text: Some(topic.clone()),
				recipients: 0,
			}
		}
		SendTarget::Condition(condition) => {
			TargetSummary {
				kind: TargetKind::Condition,
				text: Some(condition.clone()),
				recipients: 0,
			}
		}
		SendTarget::Everyone => {
			TargetSummary {
				kind: TargetKind::Everyone,
				text: None,
				recipients: 0,
			}
		}
	}
}

/// The row as a list answers it.
fn entry(row: &CampaignRow) -> CampaignEntry {
	let draft = draft_of(row).unwrap_or_default();

	CampaignEntry {
		id: row.id,
		revision: revision_of(row.revision),
		state: state_of(row),
		name: row.name.clone(),
		category: draft.spec.category,
		title: draft.spec.title,
		kind: draft.spec.options.kind,
		target: draft.target.as_ref().map(summary_of),
		visible_at: draft.spec.visible_at,
		local_time: draft.spec.local_time,
		updated_at: row.updated_at,
		updated_by: row.updated_by.clone(),
		send_key: row.send_key.clone(),
	}
}

/// `text`, cut to at most `most` bytes at a character boundary.
fn cut(text: &str, most: usize) -> &str {
	let mut end = text.len().min(most);

	while !text.is_char_boundary(end) {
		end -= 1;
	}

	&text[..end]
}

/// Appends an event to a campaign's record.
async fn record(
	tx: &leptoskit_db::Tx,
	campaign: Uuid,
	kind: CampaignEventKind,
	by: &str,
	revision: i64,
	detail: Option<&str>,
) -> rpc::Result<()> {
	NewCampaignEventRow {
		id: Uuid::now_v7(),
		campaign,
		kind: kind.as_str().to_owned(),
		by: by.to_owned(),
		revision,
		detail: detail.map(|detail| cut(detail, MAX_DETAIL_LEN).to_owned()),
	}
	.insert(tx)
	.await?;

	Ok(())
}

/// The body of [`create_campaign`](crate::campaign::create_campaign), and
/// of both duplicates: a new draft at revision 1, refused when there are
/// already [`MAX_DRAFT_CAMPAIGNS`] of them. `from` says what a copy was
/// made from.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] at the cap, and whatever a write fails with.
pub(crate) async fn create(
	tx: &leptoskit_db::Tx,
	draft: &CampaignDraft,
	by: &str,
	from: Option<&str>,
) -> rpc::Result<SavedCampaign> {
	let drafts = CampaignRow::query()
		.filter(|campaign| campaign.state.eq(CampaignState::Draft.as_str()))
		.count(tx)
		.await?;

	if drafts >= u64::from(MAX_DRAFT_CAMPAIGNS) {
		return Err(rpc::Error::invalid(format!(
			"there are already {MAX_DRAFT_CAMPAIGNS} draft campaigns: launch or archive some \
			 before saving another"
		)));
	}

	let id = Uuid::now_v7();

	NewCampaignRow {
		id,
		revision: 1,
		state: CampaignState::Draft.as_str().to_owned(),
		name: draft.name.clone(),
		spec: serde_json::to_string(&draft.spec).expect("the spec is plain JSON"),
		target: draft
			.target
			.as_ref()
			.map(|target| serde_json::to_string(target).expect("the target is plain JSON")),
		note: draft.note.clone(),
		send_key: None,
		created_by: by.to_owned(),
		updated_by: by.to_owned(),
		launched_by: None,
		cancelled_by: None,
		launched_at: None,
		cancelled_at: None,
		updated_at: leptoskit_db::now(),
	}
	.insert(tx)
	.await?;
	record(tx, id, CampaignEventKind::Created, by, 1, from).await?;

	saved(&row(tx, id).await?)
}

/// Why a write that expected `revision` of a draft changed nothing.
fn stale(row: &CampaignRow, revision: u32, doing: &str) -> rpc::Error {
	let state = state_of(row);

	if state != CampaignState::Draft {
		return rpc::Error::conflict(format!(
			"only a draft can be {doing}, and this campaign is {}",
			state.as_str()
		));
	}

	rpc::Error::conflict(format!(
		"this campaign was saved since revision {revision}: it is at revision {}. Read it again \
		 before you change it",
		row.revision
	))
}

/// The body of [`update_campaign`](crate::campaign::update_campaign).
///
/// # Errors
///
/// [`rpc::Error::NotFound`] for an unknown id, [`rpc::Error::Conflict`]
/// for a stale revision or a campaign that isn't a draft, and whatever a
/// write fails with.
pub(crate) async fn update(
	tx: &leptoskit_db::Tx,
	id: Uuid,
	revision: u32,
	draft: &CampaignDraft,
	by: &str,
) -> rpc::Result<SavedCampaign> {
	let changed = CampaignRow::update()
		.filter(|campaign| {
			campaign.id.eq(id)
				& campaign.revision.eq(i64::from(revision))
				& campaign.state.eq(CampaignState::Draft.as_str())
		})
		.set(|campaign| campaign.revision, i64::from(revision) + 1)
		.set(|campaign| campaign.name, draft.name.clone())
		.set(
			|campaign| campaign.spec,
			serde_json::to_string(&draft.spec).expect("the spec is plain JSON"),
		)
		.set(
			|campaign| campaign.target,
			draft
				.target
				.as_ref()
				.map(|target| serde_json::to_string(target).expect("the target is plain JSON")),
		)
		.set(|campaign| campaign.note, draft.note.clone())
		.set(|campaign| campaign.updated_by, by.to_owned())
		.set(|campaign| campaign.updated_at, leptoskit_db::now())
		.run(tx)
		.await?;
	let now = row(tx, id).await?;

	if changed == 0 {
		return Err(stale(&now, revision, "changed"));
	}

	saved(&now)
}

/// The body of [`campaign`](crate::campaign::campaign).
///
/// # Errors
///
/// [`rpc::Error::NotFound`] for an unknown id, and whatever the read
/// fails with.
pub(crate) async fn read(db: &impl leptoskit_db::Read, id: Uuid) -> rpc::Result<SavedCampaign> {
	saved(&row(db, id).await?)
}

/// The body of [`campaigns`](crate::campaign::campaigns): newest first by
/// id, through the primary key, or through `(state, id)` when one state
/// is asked for.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] for a cursor that isn't one of this list's,
/// and whatever the read fails with.
pub(crate) async fn list(
	db: &impl leptoskit_db::Read,
	page: &rpc::PageArgs,
	filter: CampaignFilter,
) -> rpc::Result<rpc::Page<CampaignEntry>> {
	let mut page = page.clone();
	page.limit = page.limit.clamp(1, MAX_CAMPAIGNS_PAGE);

	let mut query = CampaignRow::query();

	if let Some(state) = filter.state {
		query = query.filter(|campaign| campaign.state.eq(state.as_str()));
	}

	let rows = super::paginate_bounded(
		db,
		query,
		&page,
		5 * MAX_CAMPAIGNS_PAGE,
		|campaign| campaign.id.desc(),
		|campaign| campaign.id,
	)
	.await?;

	Ok(rpc::Page {
		items: rows.items.iter().map(entry).collect(),
		end: rows.end,
		has_more: rows.has_more,
	})
}

/// A name for a copy of `name`, within the name's bound.
fn copy_name(name: &str) -> String {
	const PREFIX: &str = "Copy of ";

	format!(
		"{PREFIX}{}",
		cut(name, MAX_CAMPAIGN_NAME_LEN - PREFIX.len())
	)
}

/// `spec` without its schedule: a copy never inherits when the original
/// went out or when it expired.
fn unscheduled(mut spec: SendSpec) -> SendSpec {
	spec.visible_at = None;
	spec.local_time = None;
	spec.expires_at = None;
	spec
}

/// The body of
/// [`duplicate_campaign`](crate::campaign::duplicate_campaign).
///
/// # Errors
///
/// [`rpc::Error::NotFound`] for an unknown id, and as [`create`].
pub(crate) async fn duplicate(
	tx: &leptoskit_db::Tx,
	id: Uuid,
	by: &str,
) -> rpc::Result<SavedCampaign> {
	let original = draft_of(&row(tx, id).await?)?;
	let copy = CampaignDraft {
		name: copy_name(&original.name),
		spec: unscheduled(original.spec),
		target: original.target,
		note: original.note,
	};

	create(tx, &copy, by, Some(&format!("a copy of campaign {id}"))).await
}

/// The body of [`duplicate_send`](crate::campaign::duplicate_send).
///
/// # Errors
///
/// [`rpc::Error::NotFound`] for an unknown send or one with no stored
/// message, and as [`create`].
pub(crate) async fn duplicate_send(
	tx: &leptoskit_db::Tx,
	send_key: &str,
	by: &str,
) -> rpc::Result<SavedCampaign> {
	let Some(send): Option<AppSend> = super::find_send(tx, send_key).await? else {
		return Err(rpc::Error::not_found(
			"no send with this idempotency key exists",
		));
	};
	let Some(spec) = super::history::spec_of(&send) else {
		return Err(rpc::Error::not_found(
			"this send was made before the service kept a send's message, so there is nothing to \
			 copy",
		));
	};
	// A recipients send keeps no list of who it named.
	let target = send
		.target
		.as_deref()
		.and_then(|stored| SendTarget::from_stored(stored).ok());
	let named = spec
		.label
		.clone()
		.filter(|label| !label.is_empty())
		.unwrap_or_else(|| spec.title.clone());
	let copy = CampaignDraft {
		name: copy_name(if named.trim().is_empty() {
			"a send"
		} else {
			&named
		})
		.replace(char::is_control, " "),
		spec: unscheduled(spec),
		target,
		note: None,
	};

	create(tx, &copy, by, Some(&format!("a copy of send {send_key}"))).await
}

/// The body of [`archive_campaign`](crate::campaign::archive_campaign).
///
/// # Errors
///
/// [`rpc::Error::NotFound`] for an unknown id, [`rpc::Error::Conflict`]
/// for a stale revision, and whatever a write fails with.
pub(crate) async fn archive(
	tx: &leptoskit_db::Tx,
	id: Uuid,
	revision: u32,
	by: &str,
) -> rpc::Result<SavedCampaign> {
	let changed = CampaignRow::update()
		.filter(|campaign| {
			campaign.id.eq(id)
				& campaign.revision.eq(i64::from(revision))
				& campaign.state.ne(CampaignState::Archived.as_str())
		})
		.set(
			|campaign| campaign.state,
			CampaignState::Archived.as_str().to_owned(),
		)
		.set(|campaign| campaign.updated_by, by.to_owned())
		.set(|campaign| campaign.updated_at, leptoskit_db::now())
		.run(tx)
		.await?;
	let now = row(tx, id).await?;

	if changed == 0 {
		// Already archived: asked again, it answers what is there.
		if state_of(&now) == CampaignState::Archived {
			return saved(&now);
		}

		return Err(rpc::Error::conflict(format!(
			"this campaign was saved since revision {revision}: it is at revision {}. Read it \
			 again before you archive it",
			now.revision
		)));
	}

	record(tx, id, CampaignEventKind::Archived, by, now.revision, None).await?;

	saved(&now)
}

/// The body of [`launch_campaign`](crate::campaign::launch_campaign).
///
/// The claim — draft to launched, for exactly this revision — and the
/// send are one transaction. A launch that loses the claim to another
/// asks to be run again, finds the campaign launched, and answers the
/// send the winner made.
///
/// # Errors
///
/// See [`launch_campaign`](crate::campaign::launch_campaign).
pub(crate) async fn launch(
	cx: &rpc::MutationCx,
	id: Uuid,
	revision: u32,
	by: &str,
	broadcast: bool,
) -> rpc::Result<LaunchedCampaign> {
	let tx = cx.tx();
	let found = row(tx, id).await?;
	let state = state_of(&found);

	match state {
		CampaignState::Draft | CampaignState::Launched => {}
		CampaignState::Cancelled | CampaignState::Archived => {
			return Err(rpc::Error::conflict(format!(
				"this campaign is {}, and can't be launched: duplicate it and launch the copy",
				state.as_str()
			)));
		}
	}

	if found.revision != i64::from(revision) {
		return Err(rpc::Error::conflict(format!(
			"this campaign was saved since revision {revision}: it is at revision {}. Read it \
			 again, so that what is launched is what was reviewed",
			found.revision
		)));
	}

	let draft = draft_of(&found)?;
	let Some(target) = draft.target else {
		return Err(rpc::Error::invalid(
			"choose who this campaign is for before launching it",
		));
	};

	if matches!(target, SendTarget::Everyone) && !broadcast {
		return Err(rpc::Error::forbidden(
			"this campaign goes to everyone: launch it with `broadcast: true`, once you have \
			 checked that the person launching may send to everyone",
		));
	}

	let key = crate::campaign::launch_key(id, revision);
	crate::check_send_shape(&key, &draft.spec, &target).map_err(rpc::Error::invalid)?;

	if state == CampaignState::Draft {
		let now = leptoskit_db::now();

		// `send` treats a time that has passed as now. A draft that sat
		// past its schedule must not go out the moment someone presses
		// launch.
		if let Some(at) = draft.spec.visible_at
			&& at <= now
		{
			return Err(rpc::Error::invalid(format!(
				"this campaign was scheduled for {}, which has passed: choose a new time, or \
				 clear the schedule to send it now",
				at.to_rfc3339()
			)));
		}

		let claimed = CampaignRow::update()
			.filter(|campaign| {
				campaign.id.eq(id)
					& campaign.revision.eq(i64::from(revision))
					& campaign.state.eq(CampaignState::Draft.as_str())
			})
			.set(
				|campaign| campaign.state,
				CampaignState::Launched.as_str().to_owned(),
			)
			.set(|campaign| campaign.send_key, Some(key.clone()))
			.set(|campaign| campaign.launched_by, Some(by.to_owned()))
			.set(|campaign| campaign.launched_at, Some(now))
			.set(|campaign| campaign.updated_at, now)
			.run(tx)
			.await?;

		// Someone else launched, saved, or archived it since the read
		// above: run again from the start and answer what is there now.
		if claimed == 0 {
			return Err(rpc::Error::Contended);
		}

		record(
			tx,
			id,
			CampaignEventKind::Launched,
			by,
			i64::from(revision),
			matches!(target, SendTarget::Everyone).then_some("to everyone, as a broadcast"),
		)
		.await?;
	}

	// A repeated launch answers the send it made. Once retention has
	// deleted that send's record there is nothing to answer, and making
	// the send again would be a second send.
	if state == CampaignState::Launched && super::find_send(tx, &key).await?.is_none() {
		return Err(rpc::Error::conflict(
			"this campaign was launched, and the record of its send has since been deleted: \
			 duplicate it to send it again",
		));
	}

	// A new launch makes the send; a repeated one finds it and answers
	// it as a duplicate.
	let send = Box::pin(crate::send_write(cx, key, draft.spec, target)).await?;

	Ok(LaunchedCampaign {
		campaign: saved(&row(tx, id).await?)?,
		send,
	})
}

/// The body of [`cancel_campaign`](crate::campaign::cancel_campaign).
///
/// # Errors
///
/// See [`cancel_campaign`](crate::campaign::cancel_campaign).
pub(crate) async fn cancel(cx: &rpc::MutationCx, id: Uuid, by: &str) -> rpc::Result<SavedCampaign> {
	let tx = cx.tx();
	let found = row(tx, id).await?;

	match state_of(&found) {
		CampaignState::Launched => {}
		// Asked again, it answers what is there.
		CampaignState::Cancelled => return saved(&found),
		CampaignState::Draft => {
			return Err(rpc::Error::conflict(
				"this campaign was never launched, so there is no send to cancel: archive it \
				 instead",
			));
		}
		CampaignState::Archived => {
			return Err(rpc::Error::conflict(
				"this campaign is archived: cancel its send by its key instead",
			));
		}
	}

	let Some(key) = found.send_key.clone() else {
		return Err(rpc::Error::server(
			"a launched campaign has no send key stored",
		));
	};
	// The send's own rule decides: refused once people have seen it.
	let stopped = super::cancel(cx, &key).await?;
	let now = leptoskit_db::now();
	let changed = CampaignRow::update()
		.filter(|campaign| campaign.id.eq(id) & campaign.state.eq(CampaignState::Launched.as_str()))
		.set(
			|campaign| campaign.state,
			CampaignState::Cancelled.as_str().to_owned(),
		)
		.set(|campaign| campaign.cancelled_by, Some(by.to_owned()))
		.set(|campaign| campaign.cancelled_at, Some(now))
		.set(|campaign| campaign.updated_at, now)
		.run(tx)
		.await?;

	if changed == 1 {
		record(
			tx,
			id,
			CampaignEventKind::Cancelled,
			by,
			found.revision,
			Some(&format!("{stopped} rows stopped")),
		)
		.await?;
	}

	saved(&row(tx, id).await?)
}

/// What a test needs from a campaign: its message, its revision, and how
/// many tests it has recorded.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct ForTest {
	id: Uuid,
}

impl rpc::Operation for ForTest {
	type Context = rpc::QueryCx;
	type Output = (SendSpec, i64, u64);

	const KIND: rpc::Kind = rpc::Kind::Query;
	const NAME: &'static str = "test_campaign";
	const SERVICE: Option<&'static str> = Some(crate::SERVICE);

	async fn run(self, cx: &rpc::QueryCx) -> rpc::Result<Self::Output> {
		let found = row(cx.db(), self.id).await?;
		let tests = CampaignEventRow::query()
			.filter(|event| {
				event.campaign.eq(self.id) & event.kind.eq(CampaignEventKind::Tested.as_str())
			})
			.count(cx.db())
			.await?;

		Ok((draft_of(&found)?.spec, found.revision, tests))
	}
}

/// The record of one test.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct RecordTest {
	id: Uuid,
	by: String,
	revision: i64,
	detail: String,
}

impl rpc::Operation for RecordTest {
	type Context = rpc::MutationCx;
	type Output = ();

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "test_campaign";
	const SERVICE: Option<&'static str> = Some(crate::SERVICE);

	async fn run(self, cx: &rpc::MutationCx) -> rpc::Result<Self::Output> {
		record(
			cx.tx(),
			self.id,
			CampaignEventKind::Tested,
			&self.by,
			self.revision,
			Some(&self.detail),
		)
		.await
	}
}

/// The body of [`test_campaign`](crate::campaign::test_campaign): the
/// campaign's message, without its schedule, pushed by
/// [`send_test`](super::send_test), and an event saying how each push
/// ended. The event names platforms and outcomes, never the person or an
/// endpoint.
///
/// # Errors
///
/// See [`test_campaign`](crate::campaign::test_campaign).
pub(crate) async fn test(
	cx: &rpc::ActionCx,
	id: Uuid,
	person: Uuid,
	endpoint: Option<Uuid>,
	by: &str,
) -> rpc::Result<Vec<crate::TestSendOutcome>> {
	let (mut spec, revision, tests) = cx.query(ForTest { id }).await?;

	if tests >= u64::from(MAX_CAMPAIGN_TESTS) {
		return Err(rpc::Error::invalid(format!(
			"this campaign has recorded {MAX_CAMPAIGN_TESTS} tests, which is as many as one \
			 keeps: duplicate it to go on testing"
		)));
	}

	// A test arrives now, whatever the campaign's schedule says.
	spec.visible_at = None;
	spec.local_time = None;
	crate::check_spec(&spec).map_err(rpc::Error::invalid)?;
	crate::check_timing(&spec).map_err(rpc::Error::invalid)?;

	let outcomes = super::send_test(cx, &spec, person, endpoint).await?;
	let detail = if outcomes.is_empty() {
		"no device to push to".to_owned()
	} else {
		outcomes
			.iter()
			.map(|outcome| {
				match outcome.reason {
					Some(reason) => {
						format!(
							"{} {} ({})",
							outcome.platform.as_str(),
							outcome.state.as_str(),
							reason.as_str()
						)
					}
					None => {
						format!("{} {}", outcome.platform.as_str(), outcome.state.as_str())
					}
				}
			})
			.collect::<Vec<_>>()
			.join(", ")
	};

	cx.mutate(RecordTest {
		id,
		by: by.to_owned(),
		revision,
		detail,
	})
	.await?;

	Ok(outcomes)
}

/// The body of [`campaign_events`](crate::campaign::campaign_events):
/// oldest first, through `(campaign, id)`.
///
/// # Errors
///
/// [`rpc::Error::NotFound`] for an unknown campaign,
/// [`rpc::Error::Invalid`] for a cursor that isn't one of this list's,
/// and whatever a read fails with.
pub(crate) async fn events(
	db: &impl leptoskit_db::Read,
	id: Uuid,
	page: &rpc::PageArgs,
) -> rpc::Result<rpc::Page<CampaignEvent>> {
	row(db, id).await?;

	let mut page = page.clone();
	page.limit = page.limit.clamp(1, MAX_CAMPAIGN_EVENTS_PAGE);

	let rows = super::paginate_bounded(
		db,
		CampaignEventRow::query().filter(|event| event.campaign.eq(id)),
		&page,
		5 * MAX_CAMPAIGN_EVENTS_PAGE,
		|event| event.id.asc(),
		|event| event.id,
	)
	.await?;

	Ok(rpc::Page {
		items: rows
			.items
			.iter()
			.filter_map(|event| {
				Some(CampaignEvent {
					id: event.id,
					kind: CampaignEventKind::from_str_value(&event.kind)?,
					by: event.by.clone(),
					at: event.created_at,
					revision: revision_of(event.revision),
					detail: event.detail.clone(),
				})
			})
			.collect(),
		end: rows.end,
		has_more: rows.has_more,
	})
}
