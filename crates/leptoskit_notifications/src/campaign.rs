//! Campaigns: a send that is written, saved, reviewed, and then launched.
//!
//! [`send`](fn@crate::send) is one call: the message and its audience go
//! in, and the send is made. A person composing a notification on a
//! screen works differently. They write part of it, leave, come back,
//! try it on their own phone, ask someone to look, pick a time, and only
//! then send. A campaign is where that work is kept: a
//! [`CampaignDraft`] saved in the service, changed as often as it takes,
//! and turned into exactly one send by [`launch_campaign`].
//!
//! What a campaign adds to a send:
//!
//! - **A draft can be incomplete.** [`check_draft`] bounds a draft's size and
//!   nothing else, so a draft with no title or no audience saves. Everything
//!   [`send`](fn@crate::send) checks is checked at launch.
//! - **Two people can't overwrite each other.** Every change names the
//!   [revision](SavedCampaign::revision) it was made from, and is refused when
//!   the campaign has moved on.
//! - **A launch happens once.** The send's idempotency key is made from the
//!   campaign's id and revision, so launching twice — a double press, a retry —
//!   makes one send and answers it both times.
//! - **A stale schedule is refused.** [`send`](fn@crate::send) treats a time
//!   that has passed as "now". A draft scheduled for Tuesday and launched on
//!   Thursday would go out at once, so a launch refuses it.
//! - **Who did what is recorded.** Every operation takes `by`, the name of the
//!   person acting, as the app's server knows it, and [`campaign_events`] lists
//!   what was done.
//!
//! Every operation here is for the app's server only.

use leptoskit_db::table;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::LocalTime;
use crate::MessageKind;
use crate::SendResult;
use crate::SendSpec;
use crate::SendTarget;
use crate::TargetSummary;
use crate::TestSendOutcome;

/// The longest campaign name, in bytes.
pub const MAX_CAMPAIGN_NAME_LEN: usize = 120;

/// The longest campaign note, in bytes.
pub const MAX_CAMPAIGN_NOTE_LEN: usize = 1_000;

/// The longest name of the person acting (`by`), in bytes.
pub const MAX_ACTOR_LEN: usize = 128;

/// The most campaigns that can be drafts at once. Launch, archive, or
/// cancel some to make room.
pub const MAX_DRAFT_CAMPAIGNS: u32 = 1_000;

/// The most test sends one campaign records.
pub const MAX_CAMPAIGN_TESTS: u32 = 100;

/// The most campaigns one page of [`campaigns`] answers.
pub const MAX_CAMPAIGNS_PAGE: u32 = 50;

/// The most events one page of [`campaign_events`] answers.
pub const MAX_CAMPAIGN_EVENTS_PAGE: u32 = 100;

/// The longest a draft's recipients target is once stored: the JSON of
/// [`MAX_RECIPIENTS`](crate::MAX_RECIPIENTS) ids.
pub const MAX_DRAFT_TARGET_LEN: usize = 40_000;

/// What a campaign is while it is being written: the message, who it is
/// for, and a name and a note for the people working on it.
///
/// The schedule is the spec's own: [`SendSpec::visible_at`],
/// [`SendSpec::local_time`], and [`SendSpec::expires_at`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CampaignDraft {
	/// What the people working on it call it. Never sent to a device.
	pub name: String,
	/// The message. It may be incomplete while it is a draft.
	#[serde(default)]
	pub spec: SendSpec,
	/// Who it is for, once that is chosen.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub target: Option<SendTarget>,
	/// A note for the people working on it. Never sent to a device.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub note: Option<String>,
}

/// Where a campaign stands. What became of a launched campaign's send —
/// scheduled, sending, sent — is the send's own state: ask
/// [`send_report`](fn@crate::send_report) with the campaign's
/// [`send_key`](SavedCampaign::send_key).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CampaignState {
	/// Being written. It can be changed, tested, and launched.
	#[default]
	Draft,
	/// Launched: its send was made.
	Launched,
	/// Launched, and its send cancelled before it was due.
	Cancelled,
	/// Put away. It can be read and duplicated, and nothing else.
	Archived,
}

impl CampaignState {
	/// The name the state is stored under.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Draft => "draft",
			Self::Launched => "launched",
			Self::Cancelled => "cancelled",
			Self::Archived => "archived",
		}
	}

	/// The state a stored name stands for.
	#[must_use]
	pub fn from_str_value(name: &str) -> Option<Self> {
		match name {
			"draft" => Some(Self::Draft),
			"launched" => Some(Self::Launched),
			"cancelled" => Some(Self::Cancelled),
			"archived" => Some(Self::Archived),
			_ => None,
		}
	}
}

/// A campaign as the service holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SavedCampaign {
	/// The campaign's id.
	pub id: uuid::Uuid,
	/// How many times its draft was saved, starting at 1. A change names
	/// the revision it was made from.
	pub revision: u32,
	/// Where it stands.
	pub state: CampaignState,
	/// The message, the audience, the name, and the note.
	pub draft: CampaignDraft,
	/// When it was created, and by whom.
	pub created_at: leptoskit_db::Timestamp,
	/// Who created it.
	pub created_by: String,
	/// When it was last changed.
	pub updated_at: leptoskit_db::Timestamp,
	/// Who last changed it.
	pub updated_by: String,
	/// When it was launched.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub launched_at: Option<leptoskit_db::Timestamp>,
	/// Who launched it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub launched_by: Option<String>,
	/// When its send was cancelled.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cancelled_at: Option<leptoskit_db::Timestamp>,
	/// Who cancelled it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cancelled_by: Option<String>,
	/// The idempotency key of the send it was launched as: what
	/// [`send_report`](fn@crate::send_report) and
	/// [`send_status`](fn@crate::send_status) take.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub send_key: Option<String>,
}

/// One campaign in a list: what a row of a table shows, without the whole
/// draft.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct CampaignEntry {
	/// The campaign's id.
	pub id: uuid::Uuid,
	/// Its revision.
	pub revision: u32,
	/// Where it stands.
	pub state: CampaignState,
	/// Its name.
	pub name: String,
	/// The message's category, which may be empty in a draft.
	pub category: String,
	/// The message's title, which may be empty in a draft.
	pub title: String,
	/// Whether it is a notification or a silent data message.
	#[serde(default)]
	pub kind: MessageKind,
	/// Who it is for, once that is chosen.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub target: Option<TargetSummary>,
	/// When it is scheduled to become visible.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub visible_at: Option<leptoskit_db::Timestamp>,
	/// The local time it is scheduled for.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub local_time: Option<LocalTime>,
	/// When it was last changed.
	pub updated_at: leptoskit_db::Timestamp,
	/// Who last changed it.
	pub updated_by: String,
	/// The idempotency key of its send, once launched.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub send_key: Option<String>,
}

/// Which campaigns [`campaigns`] lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct CampaignFilter {
	/// Only campaigns in this state. Every state when left out.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub state: Option<CampaignState>,
}

impl CampaignFilter {
	/// Every campaign.
	#[must_use]
	pub fn all() -> Self {
		Self::default()
	}

	/// Only the campaigns in `state`.
	#[must_use]
	pub fn in_state(mut self, state: CampaignState) -> Self {
		self.state = Some(state);
		self
	}
}

/// What was done to a campaign.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CampaignEventKind {
	/// It was created, new or as a copy.
	Created,
	/// It was sent to one person's devices as a test.
	Tested,
	/// It was launched.
	Launched,
	/// Its send was cancelled.
	Cancelled,
	/// It was archived.
	Archived,
}

impl CampaignEventKind {
	/// The name the kind is stored under.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Created => "created",
			Self::Tested => "tested",
			Self::Launched => "launched",
			Self::Cancelled => "cancelled",
			Self::Archived => "archived",
		}
	}

	/// The kind a stored name stands for.
	#[must_use]
	pub fn from_str_value(name: &str) -> Option<Self> {
		match name {
			"created" => Some(Self::Created),
			"tested" => Some(Self::Tested),
			"launched" => Some(Self::Launched),
			"cancelled" => Some(Self::Cancelled),
			"archived" => Some(Self::Archived),
			_ => None,
		}
	}
}

/// One thing that was done to a campaign, and by whom. Saving a draft is
/// not an event: the campaign's own `updated_at` and `updated_by` say who
/// saved it last.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct CampaignEvent {
	/// The event's id. Events are listed in the order they happened.
	pub id: uuid::Uuid,
	/// What was done.
	pub kind: CampaignEventKind,
	/// Who did it, as the app's server named them.
	pub by: String,
	/// When.
	pub at: leptoskit_db::Timestamp,
	/// The campaign's revision at the time.
	pub revision: u32,
	/// More about it, for a person to read: what a copy was made from,
	/// how a test's pushes ended, how many rows a cancel stopped.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub detail: Option<String>,
}

/// What [`launch_campaign`] answers: the campaign as it now stands, and
/// what [`send`](fn@crate::send) answered for its send.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LaunchedCampaign {
	/// The campaign, now launched.
	pub campaign: SavedCampaign,
	/// The send's answer. [`SendResult::duplicate`] is `true` when this
	/// launch had already been made.
	pub send: SendResult,
}

/// One campaign's stored row. [`SavedCampaign`] is what callers read.
#[table(name = "campaigns", index(state, id))]
pub struct CampaignRow {
	/// A UUID v7: campaigns list newest first by it.
	#[db(key)]
	pub id: uuid::Uuid,
	/// How many times the draft was saved.
	pub revision: i64,
	/// A [`CampaignState`]'s name.
	#[db(one_of("draft", "launched", "cancelled", "archived"))]
	pub state: String,
	/// The campaign's name.
	#[db(min_len = 1, max_len = 120)]
	pub name: String,
	/// The draft's spec, as JSON.
	#[db(max_len = 8_192)]
	pub spec: String,
	/// The draft's target, as JSON, once chosen.
	#[db(max_len = 40_000)]
	pub target: Option<String>,
	/// The draft's note.
	#[db(max_len = 1_000)]
	pub note: Option<String>,
	/// The idempotency key of the send it was launched as.
	#[db(max_len = 255)]
	pub send_key: Option<String>,
	/// Who created it.
	#[db(min_len = 1, max_len = 128)]
	pub created_by: String,
	/// Who last changed it.
	#[db(min_len = 1, max_len = 128)]
	pub updated_by: String,
	/// Who launched it.
	#[db(max_len = 128)]
	pub launched_by: Option<String>,
	/// Who cancelled it.
	#[db(max_len = 128)]
	pub cancelled_by: Option<String>,
	/// When it was launched.
	pub launched_at: Option<leptoskit_db::Timestamp>,
	/// When it was cancelled.
	pub cancelled_at: Option<leptoskit_db::Timestamp>,
	/// When it was created.
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
	/// When it was last changed.
	pub updated_at: leptoskit_db::Timestamp,
}

/// One stored event of a campaign. Append-only.
#[table(name = "campaign_events", index(campaign, id))]
pub struct CampaignEventRow {
	/// A UUID v7: a campaign's events list in the order they happened.
	#[db(key)]
	pub id: uuid::Uuid,
	/// The campaign.
	pub campaign: uuid::Uuid,
	/// A [`CampaignEventKind`]'s name.
	#[db(one_of("created", "tested", "launched", "cancelled", "archived"))]
	pub kind: String,
	/// Who did it.
	#[db(min_len = 1, max_len = 128)]
	pub by: String,
	/// The campaign's revision at the time.
	pub revision: i64,
	/// More about it.
	#[db(max_len = 256)]
	pub detail: Option<String>,
	/// When.
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
}

/// Checks a draft's bounds and nothing about whether it could be sent: a
/// name of 1 to [`MAX_CAMPAIGN_NAME_LEN`] bytes with no control
/// characters, a note of at most [`MAX_CAMPAIGN_NOTE_LEN`] bytes, a spec
/// of at most [`MAX_SPEC_LEN`](crate::MAX_SPEC_LEN) bytes as JSON, and a
/// target no larger than a send's. An empty title, an unknown category,
/// a condition that doesn't parse, and a missing audience all save;
/// [`launch_campaign`] checks them.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_draft(draft: &CampaignDraft) -> Result<(), String> {
	if draft.name.trim().is_empty() || draft.name.len() > MAX_CAMPAIGN_NAME_LEN {
		return Err(format!(
			"a campaign's name is 1 to {MAX_CAMPAIGN_NAME_LEN} bytes"
		));
	}

	if draft.name.chars().any(char::is_control) {
		return Err("a campaign's name has no control characters".into());
	}

	if let Some(note) = &draft.note
		&& note.len() > MAX_CAMPAIGN_NOTE_LEN
	{
		return Err(format!(
			"a campaign's note is at most {MAX_CAMPAIGN_NOTE_LEN} bytes"
		));
	}

	if crate::stored_len(&draft.spec) > crate::MAX_SPEC_LEN {
		return Err(format!(
			"a campaign's message is at most {} bytes once stored as JSON: shorten its data, \
			 body, or variants",
			crate::MAX_SPEC_LEN
		));
	}

	match &draft.target {
		Some(SendTarget::Recipients(recipients)) if recipients.len() > crate::MAX_RECIPIENTS => {
			Err(format!(
				"a campaign names at most {} recipients",
				crate::MAX_RECIPIENTS
			))
		}
		Some(target @ (SendTarget::Topic(_) | SendTarget::Condition(_)))
			if crate::stored_len(target) > crate::MAX_TARGET_LEN =>
		{
			Err(format!(
				"a target is at most {} bytes once stored as JSON: shorten it",
				crate::MAX_TARGET_LEN
			))
		}
		_ => Ok(()),
	}
}

/// Checks the name of the person acting: 1 to [`MAX_ACTOR_LEN`] bytes
/// with no control characters.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_actor(by: &str) -> Result<(), String> {
	if by.trim().is_empty() || by.len() > MAX_ACTOR_LEN {
		return Err(format!(
			"`by` names who is acting, in 1 to {MAX_ACTOR_LEN} bytes"
		));
	}

	if by.chars().any(char::is_control) {
		return Err("`by` has no control characters".into());
	}

	Ok(())
}

/// The idempotency key of the send a campaign's `revision` is launched
/// as. One revision of one campaign is one send, however often it is
/// launched.
#[must_use]
pub fn launch_key(id: uuid::Uuid, revision: u32) -> String {
	format!("campaign:{id}:r{revision}")
}

/// Refuses every caller but the app's server.
#[cfg(feature = "service")]
fn app_only(identity: &rpc::server::Identity, does: &str) -> rpc::Result<()> {
	if identity.caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(format!(
			"only the app {does}: call this from its server, with the service's token"
		)));
	}

	Ok(())
}

/// Saves a new campaign as a draft, at revision 1, and records who
/// created it.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a draft [`check_draft`] refuses, a `by`
/// [`check_actor`] refuses, or when [`MAX_DRAFT_CAMPAIGNS`] drafts
/// already exist.
#[rpc::mutation(service = "notifications")]
pub async fn create_campaign(
	cx: &rpc::MutationCx,
	draft: CampaignDraft,
	by: String,
) -> rpc::Result<SavedCampaign> {
	app_only(cx.identity(), "creates campaigns")?;
	check_actor(&by).map_err(rpc::Error::invalid)?;
	check_draft(&draft).map_err(rpc::Error::invalid)?;

	crate::service::campaign::create(cx.tx(), &draft, &by, None).await
}

/// Saves a changed draft. `revision` is the revision the change was made
/// from: when the campaign has been saved since, the change is refused,
/// so two people editing the same draft can't silently overwrite each
/// other. The saved campaign's revision is one more.
///
/// Only a draft can be changed. Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::Invalid`] for a draft or a `by` the checks refuse,
/// [`rpc::Error::NotFound`] for an unknown id, and
/// [`rpc::Error::Conflict`] for a stale revision or a campaign that is no
/// longer a draft.
#[rpc::mutation(service = "notifications")]
pub async fn update_campaign(
	cx: &rpc::MutationCx,
	id: uuid::Uuid,
	revision: u32,
	draft: CampaignDraft,
	by: String,
) -> rpc::Result<SavedCampaign> {
	app_only(cx.identity(), "changes campaigns")?;
	check_actor(&by).map_err(rpc::Error::invalid)?;
	check_draft(&draft).map_err(rpc::Error::invalid)?;

	crate::service::campaign::update(cx.tx(), id, revision, &draft, &by).await
}

/// One campaign, whole.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::NotFound`] for an unknown id.
#[rpc::query(service = "notifications")]
pub async fn campaign(cx: &rpc::QueryCx, id: uuid::Uuid) -> rpc::Result<SavedCampaign> {
	app_only(cx.identity(), "reads campaigns")?;

	crate::service::campaign::read(cx.db(), id).await
}

/// The campaigns, newest first, a page of at most [`MAX_CAMPAIGNS_PAGE`]
/// at a time, each as a [`CampaignEntry`].
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a cursor that isn't one of this list's.
#[rpc::query(service = "notifications")]
pub async fn campaigns(
	cx: &rpc::QueryCx,
	page: rpc::PageArgs,
	#[serde(default)] filter: CampaignFilter,
) -> rpc::Result<rpc::Page<CampaignEntry>> {
	app_only(cx.identity(), "lists campaigns")?;

	crate::service::campaign::list(cx.db(), &page, filter).await
}

/// Saves a copy of a campaign as a new draft: the same message, audience,
/// and note, a name that says it is a copy, and no schedule. The
/// original's `visible_at`, `local_time`, and `expires_at` are not
/// copied, because a copy of last month's campaign must not inherit last
/// month's times.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::Invalid`] for a `by` [`check_actor`] refuses or when
/// [`MAX_DRAFT_CAMPAIGNS`] drafts already exist, and
/// [`rpc::Error::NotFound`] for an unknown id.
#[rpc::mutation(service = "notifications")]
pub async fn duplicate_campaign(
	cx: &rpc::MutationCx,
	id: uuid::Uuid,
	by: String,
) -> rpc::Result<SavedCampaign> {
	app_only(cx.identity(), "duplicates campaigns")?;
	check_actor(&by).map_err(rpc::Error::invalid)?;

	crate::service::campaign::duplicate(cx.tx(), id, &by).await
}

/// Saves a new draft from a send that was already made, whoever made it:
/// its message, and its topic, condition, or everyone audience, with no
/// schedule. A recipients send's list of people is not kept by the
/// service, so its copy has no audience yet.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::Invalid`] for a key or a `by` the checks refuse or when
/// [`MAX_DRAFT_CAMPAIGNS`] drafts already exist, and
/// [`rpc::Error::NotFound`] for an unknown send or one that stored no
/// message to copy.
#[rpc::mutation(service = "notifications")]
pub async fn duplicate_send(
	cx: &rpc::MutationCx,
	idempotency_key: String,
	by: String,
) -> rpc::Result<SavedCampaign> {
	app_only(cx.identity(), "duplicates sends")?;
	check_actor(&by).map_err(rpc::Error::invalid)?;
	crate::check_key(&idempotency_key).map_err(rpc::Error::invalid)?;

	crate::service::campaign::duplicate_send(cx.tx(), &idempotency_key, &by).await
}

/// Puts a campaign away. An archived campaign can be read and duplicated,
/// and can no longer be changed, launched, or cancelled through its
/// campaign. Archiving a campaign that is already archived answers it as
/// it is.
///
/// Archiving does not cancel anything: a launched campaign's send goes
/// on. Cancel it first if that is what you mean.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::Invalid`] for a `by` [`check_actor`] refuses,
/// [`rpc::Error::NotFound`] for an unknown id, and
/// [`rpc::Error::Conflict`] for a stale revision.
#[rpc::mutation(service = "notifications")]
pub async fn archive_campaign(
	cx: &rpc::MutationCx,
	id: uuid::Uuid,
	revision: u32,
	by: String,
) -> rpc::Result<SavedCampaign> {
	app_only(cx.identity(), "archives campaigns")?;
	check_actor(&by).map_err(rpc::Error::invalid)?;

	crate::service::campaign::archive(cx.tx(), id, revision, &by).await
}

/// Launches a draft: makes its send, exactly as [`send`](fn@crate::send)
/// would with the draft's spec and target, under the key
/// [`launch_key`] makes from the campaign's id and revision.
///
/// - `revision` must be the campaign's current revision: what the person
///   reviewed is what is sent.
/// - Launching the same revision again — a double press, a retry after a
///   timeout — makes no second send. It answers the first one, with
///   [`SendResult::duplicate`] `true`.
/// - A draft whose `visible_at` has passed is refused. Choose a new time, or
///   clear the schedule to send now.
/// - A draft for [`SendTarget::Everyone`] is refused unless `broadcast` is
///   `true`. Sending to everyone is the largest thing this service does, and
///   the flag is the caller saying it has checked that the person launching is
///   allowed to. The launch's event records it.
/// - Everything [`send`](fn@crate::send) checks is checked here, and a draft
///   with no audience is refused.
///
/// The campaign's state and its send are written together: a launch that
/// is refused leaves a draft.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, or for
/// an everyone audience without `broadcast`; [`rpc::Error::Invalid`] for
/// a draft that can't be sent or whose schedule has passed;
/// [`rpc::Error::NotFound`] for an unknown id; and
/// [`rpc::Error::Conflict`] for a stale revision or a campaign that was
/// cancelled or archived.
#[rpc::mutation(service = "notifications")]
pub async fn launch_campaign(
	cx: &rpc::MutationCx,
	id: uuid::Uuid,
	revision: u32,
	by: String,
	#[serde(default)] broadcast: bool,
) -> rpc::Result<LaunchedCampaign> {
	app_only(cx.identity(), "launches campaigns")?;
	check_actor(&by).map_err(rpc::Error::invalid)?;

	crate::service::campaign::launch(cx, id, revision, &by, broadcast).await
}

/// Cancels a launched campaign's send before it is due, by the rules of
/// [`cancel`](fn@crate::cancel): a scheduled send nobody has seen yet is
/// stopped, a send still working through its audience is stopped where
/// it is, and a send people have already seen is refused. Cancelling a
/// campaign that is already cancelled answers it as it is.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::Invalid`] for a `by` [`check_actor`] refuses,
/// [`rpc::Error::NotFound`] for an unknown id, and
/// [`rpc::Error::Conflict`] for a campaign that was never launched or is
/// archived, or once its send is visible.
#[rpc::mutation(service = "notifications")]
pub async fn cancel_campaign(
	cx: &rpc::MutationCx,
	id: uuid::Uuid,
	by: String,
) -> rpc::Result<SavedCampaign> {
	app_only(cx.identity(), "cancels campaigns")?;
	check_actor(&by).map_err(rpc::Error::invalid)?;

	crate::service::campaign::cancel(cx, id, &by).await
}

/// Pushes a campaign's message to one person's devices as a test, the way
/// [`send_test`](fn@crate::send_test) does: straight to the provider,
/// with no inbox row, no delivery record, and no effect on anyone else.
/// The campaign's schedule is ignored, so a test arrives now. What the
/// test came to is recorded as a [`CampaignEventKind::Tested`] event,
/// without the person's id.
///
/// A campaign records at most [`MAX_CAMPAIGN_TESTS`] tests.
///
/// Only the app may call it.
///
/// # Errors
///
/// As [`send_test`](fn@crate::send_test), and [`rpc::Error::NotFound`]
/// for an unknown campaign, [`rpc::Error::Invalid`] for a `by`
/// [`check_actor`] refuses or a campaign that has used its tests.
#[rpc::action(service = "notifications")]
pub async fn test_campaign(
	cx: &rpc::ActionCx,
	id: uuid::Uuid,
	person: uuid::Uuid,
	#[serde(default)] endpoint: Option<uuid::Uuid>,
	by: String,
) -> rpc::Result<Vec<TestSendOutcome>> {
	app_only(cx.identity(), "tests campaigns")?;
	check_actor(&by).map_err(rpc::Error::invalid)?;

	crate::service::campaign::test(cx, id, person, endpoint, &by).await
}

/// What was done to a campaign and by whom, oldest first, a page of at
/// most [`MAX_CAMPAIGN_EVENTS_PAGE`] at a time.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::NotFound`] for an unknown id, and
/// [`rpc::Error::Invalid`] for a cursor that isn't one of this list's.
#[rpc::query(service = "notifications")]
pub async fn campaign_events(
	cx: &rpc::QueryCx,
	id: uuid::Uuid,
	page: rpc::PageArgs,
) -> rpc::Result<rpc::Page<CampaignEvent>> {
	app_only(cx.identity(), "reads campaign events")?;

	crate::service::campaign::events(cx.db(), id, &page).await
}

#[cfg(test)]
mod tests {
	use super::*;

	fn draft() -> CampaignDraft {
		CampaignDraft {
			name: "Spring sale".into(),
			..CampaignDraft::default()
		}
	}

	#[test]
	fn a_draft_may_be_incomplete() {
		// No title, no category, no audience.
		assert_eq!(check_draft(&draft()), Ok(()));

		// A condition that doesn't parse still saves.
		let mut unfinished = draft();
		unfinished.target = Some(SendTarget::Condition("user.plan ==".into()));
		assert_eq!(check_draft(&unfinished), Ok(()));
	}

	#[test]
	fn a_draft_is_bounded() {
		let mut unnamed = draft();
		unnamed.name = "  ".into();
		assert!(check_draft(&unnamed).unwrap_err().contains("name"));

		let mut long = draft();
		long.name = "n".repeat(MAX_CAMPAIGN_NAME_LEN + 1);
		assert!(check_draft(&long).unwrap_err().contains("name"));

		let mut control = draft();
		control.name = "a\nb".into();
		assert!(check_draft(&control).unwrap_err().contains("control"));

		let mut noted = draft();
		noted.note = Some("n".repeat(MAX_CAMPAIGN_NOTE_LEN + 1));
		assert!(check_draft(&noted).unwrap_err().contains("note"));

		let mut big = draft();
		big.spec.body = "b".repeat(crate::MAX_SPEC_LEN);
		assert!(check_draft(&big).unwrap_err().contains("message"));

		let mut many = draft();
		many.target = Some(SendTarget::Recipients(vec![
			uuid::Uuid::nil();
			crate::MAX_RECIPIENTS + 1
		]));
		assert!(check_draft(&many).unwrap_err().contains("recipients"));

		let mut wide = draft();
		wide.target = Some(SendTarget::Condition("x".repeat(crate::MAX_TARGET_LEN)));
		assert!(check_draft(&wide).unwrap_err().contains("target"));

		// The largest recipients target fits its column.
		let full = SendTarget::Recipients(vec![uuid::Uuid::max(); crate::MAX_RECIPIENTS]);
		assert!(crate::stored_len(&full) <= MAX_DRAFT_TARGET_LEN);
	}

	#[test]
	fn an_actor_is_named() {
		assert_eq!(check_actor("ada@example.test"), Ok(()));
		assert!(check_actor("").is_err());
		assert!(check_actor(" ").is_err());
		assert!(check_actor(&"a".repeat(MAX_ACTOR_LEN + 1)).is_err());
		assert!(check_actor("a\tb").is_err());
	}

	#[test]
	fn a_launch_key_is_a_valid_send_key() {
		let key = launch_key(uuid::Uuid::max(), u32::MAX);
		assert_eq!(crate::check_key(&key), Ok(()));
		assert_eq!(
			key,
			"campaign:ffffffff-ffff-ffff-ffff-ffffffffffff:r4294967295"
		);
	}
}
