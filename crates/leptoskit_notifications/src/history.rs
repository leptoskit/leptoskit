//! Send history: the sends the service remembers, newest first, and the
//! pieces of a send's report that describe what it was and where its
//! pushes went.
//!
//! A screen that lists what was sent needs more than the idempotency key
//! the app's server kept: what the send said, who it was for, when it
//! shows, and how far it got. [`list_sends`] answers a page of
//! [`SendEntry`] rows for that, and [`send_report`](crate::send_report)
//! adds the same description — and the push deliveries by platform and
//! state, and reads by the hour — to one send's counts.
//!
//! Nothing here names a person. A recipients send stores how many people
//! it named, never who; a topic, condition, or everyone send stores its
//! target as it was sent. A condition can carry values of the app's own
//! attributes (`user.email == '…'`), so treat the list as you would the
//! send calls themselves: for the app's server and its operators.

use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::LocalTime;
use crate::MessageKind;
use crate::SendState;
use crate::push::DeliveryReason;
use crate::push::PushPlatform;

/// The most sends one page of [`list_sends`] answers.
pub const MAX_SENDS_PAGE: u32 = 100;

/// The most sends a pinned page of [`list_sends`] answers: a page a later
/// page was loaded after reads up to where it ended, however many sends
/// that is, and this bounds it.
pub const MAX_PINNED_SENDS: u32 = 5 * MAX_SENDS_PAGE;

/// The most hours [`SendSummary::reads_by_hour`](crate::SendSummary) names:
/// a week of them.
pub const MAX_REPORT_HOURS: usize = 7 * 24;

/// Which sends [`list_sends`] answers. Every field narrows the list; the
/// default is every send in the history.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SendFilter {
	/// Only sends in this state.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub state: Option<SendState>,
}

impl SendFilter {
	/// Every send.
	#[must_use]
	pub fn all() -> Self {
		Self::default()
	}

	/// Only sends in `state`.
	#[must_use]
	pub fn in_state(mut self, state: SendState) -> Self {
		self.state = Some(state);
		self
	}
}

/// The kind of audience a send was for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
	/// People the app named.
	#[default]
	Recipients,
	/// One topic's subscribers.
	Topic,
	/// Everyone a condition matched.
	Condition,
	/// Everyone the service knows.
	Everyone,
}

/// Who a send was for, without naming anyone: the kind of audience, the
/// topic or condition it named, and for a recipients send how many people.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TargetSummary {
	/// The kind of audience.
	pub kind: TargetKind,
	/// The topic's name, or the condition as it was sent. `None` for
	/// recipients and everyone.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub text: Option<String>,
	/// How many people a recipients send named, after duplicates
	/// collapsed. 0 for every other kind: ask the send's `targeted` count
	/// how many people its audience came to.
	#[serde(default)]
	pub recipients: u32,
}

/// One send in the history, as [`list_sends`] answers it: what it said,
/// who it was for, when it shows, and how far it got.
///
/// More will be listed here over time, so the type cannot be built or
/// matched exhaustively outside this crate: read the fields you need.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SendEntry {
	/// The send's idempotency key: what
	/// [`send_report`](crate::send_report),
	/// [`send_status`](crate::send_status), and [`cancel`](crate::cancel)
	/// name it by.
	pub key: String,
	/// When the send was made.
	pub created_at: leptoskit_db::Timestamp,
	/// Where the send stands.
	pub state: SendState,
	/// The send's [label](crate::SendSpec::label), when it carried one.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub label: Option<String>,
	/// The send's category.
	pub category: String,
	/// The send's title, in the send's own language: a variant's title is
	/// in the report's spec.
	pub title: String,
	/// Whether it was a visible notification or a silent data message.
	#[serde(default)]
	pub kind: MessageKind,
	/// Who it was for.
	pub target: TargetSummary,
	/// The instant the send scheduled itself for. `None` for a send that
	/// showed at once, and for one scheduled by local time.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub visible_at: Option<leptoskit_db::Timestamp>,
	/// The wall-clock time a local-time send shows at.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub local_time: Option<LocalTime>,
	/// When the send stops showing.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub expires_at: Option<leptoskit_db::Timestamp>,
	/// How many people the target matched, so far for a send still
	/// fanning out.
	pub targeted: u32,
	/// How many inbox rows the send wrote.
	pub delivered: u32,
	/// How many matched people it skipped for opting out of the category.
	pub skipped_opted_out: u32,
}

/// One platform's push deliveries for a send, by where they stand. The
/// five states add up to every delivery the service still stores for the
/// send on that platform.
///
/// `accepted` is the provider taking the message: not a device receiving
/// it, and not a person seeing it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PushCounts {
	/// Which platform's service these went through.
	pub platform: PushPlatform,
	/// Waiting for their first attempt: a scheduled send's, before its
	/// time, and a round's backlog.
	#[serde(default)]
	pub enqueued: u32,
	/// Attempted, failed in a way that may pass, and waiting to try again.
	#[serde(default)]
	pub retrying: u32,
	/// Taken by the provider.
	#[serde(default)]
	pub accepted: u32,
	/// Refused for good, or out of attempts.
	#[serde(default)]
	pub rejected: u32,
	/// Never sent: cancelled, expired, opted out, or an endpoint that was
	/// gone by then.
	#[serde(default)]
	pub skipped: u32,
	/// Why the rejected and skipped ones ended, most common first.
	#[serde(default)]
	pub reasons: Vec<ReasonCount>,
}

/// How many of a platform's deliveries ended for one reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReasonCount {
	/// The sanitized reason code.
	pub reason: DeliveryReason,
	/// How many deliveries ended with it.
	pub count: u32,
}

/// How many of a send's rows were first read within one hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HourCount {
	/// The start of the hour, in UTC.
	pub hour: leptoskit_db::Timestamp,
	/// How many rows were read in it.
	pub read: u32,
}

/// The sends the service remembers, newest first, a page at a time: each
/// one's key, content, audience, schedule, state, and counts
/// ([`SendEntry`]). `filter` narrows the list.
///
/// A page is at most [`MAX_SENDS_PAGE`] sends, read through the history's
/// own index in order: its cost is the page, never the table. The list
/// holds every kind of send — recipients, topic, condition, and everyone —
/// made since the service kept a history; a send made before that is not
/// listed, though [`send_report`](crate::send_report) still answers it by
/// key. A send leaves the list when retention prunes its record
/// ([`prune`](crate::prune)).
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a cursor that isn't one of this list's.
#[rpc::query(service = "notifications")]
pub async fn list_sends(
	cx: &rpc::QueryCx,
	page: rpc::PageArgs,
	#[serde(default)] filter: SendFilter,
) -> rpc::Result<rpc::Page<SendEntry>> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app lists sends: call this from its server, with the service's token",
		));
	}

	crate::service::history::list(cx.db(), &page, &filter).await
}
