//! The audience catalogue: what an audience can be built from, and how
//! many people each piece stands for.
//!
//! A screen that lets someone choose who a notification goes to needs to
//! show what there is to choose: the topics people are subscribed to, the
//! attributes the app set and the values they hold, the categories people
//! opted out of — and, before anything is sent, about how many people a
//! target would reach. These queries answer that for the app's server.
//!
//! Every count here is bounded. A count stops at a cap and says so
//! ([`Count::exact`] is `false`), and [`estimate_audience`] walks at most
//! [`ESTIMATE_WALK`] people before answering "at least this many", so a
//! screen can ask on every keystroke without the cost growing with the
//! tables. For the exact audience of a send, ask
//! [`send`](fn@crate::send) with `validate_only`, which walks all of it.
//!
//! The lists are ordered as the database orders text, which on Postgres
//! follows the database's collation.
//!
//! Attribute values are the app's own data about people and may be
//! personal. Only the app's server can read these; show them to operators
//! with the care the rest of that data gets.

use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::SendTarget;
use crate::push::PushPlatform;

/// The most rows one page of a catalogue list answers.
pub const MAX_CATALOGUE_PAGE: u32 = 50;

/// What a catalogue count stops at: a topic, attribute key, attribute
/// value, or category with more people than this answers this many and
/// [`Count::exact`] `false`.
pub const COUNT_CAP: u32 = 100_000;

/// What the overview's counts of people stop at.
pub const OVERVIEW_COUNT_CAP: u32 = 1_000_000;

/// What the overview's count of distinct topics stops at.
pub const OVERVIEW_TOPICS_CAP: u32 = 1_000;

/// The most people [`estimate_audience`] walks before it answers what it
/// has, marked inexact: ten fanout batches.
pub const ESTIMATE_WALK: usize = 10 * crate::FANOUT_BATCH;

/// The most distinct names a prefix search examines in one call on
/// Postgres, where text is ordered by the database's collation and the
/// names sharing a prefix need not sit together. A call that examines
/// this many without filling its page answers what it found and says more
/// may follow.
pub const PREFIX_SCAN: u32 = 500;

/// A count that stops at a cap. `exact` is `false` when the true number
/// is larger than `count`: show it as "100,000+".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Count {
	/// How many, up to the cap.
	pub count: u32,
	/// Whether that is the whole number.
	pub exact: bool,
}

/// How many people can be pushed to on one platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PlatformReach {
	/// The platform's push service.
	pub platform: PushPlatform,
	/// How many people hold at least one live endpoint on it. An endpoint
	/// its provider invalidated is not live.
	pub people: Count,
	/// Whether this server has the platform's transport configured. People
	/// can be enrolled on a platform the server can no longer push to.
	pub configured: bool,
}

/// What [`notifications_overview`] answers: the size of what the service
/// knows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct Overview {
	/// How many people the service knows: its registry, which is who
	/// [`SendTarget::Everyone`] reaches. A person is known through an act —
	/// an attribute the app set, a topic, a preference, an enrollment —
	/// not by having an account.
	pub people_known: Count,
	/// How many people can be pushed to, per platform, for every platform.
	pub people_with_push: Vec<PlatformReach>,
	/// How many distinct topics have a subscriber.
	pub topics: Count,
	/// The topic patterns the operator opened to self-subscription.
	pub open_topics: Vec<String>,
}

/// One topic with at least one subscriber.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct TopicSummary {
	/// The topic's name.
	pub topic: String,
	/// How many people are subscribed.
	pub subscribers: Count,
	/// Whether people may subscribe themselves: the operator opened a
	/// pattern that matches it.
	pub open: bool,
}

/// One attribute key at least one person holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AttributeKeySummary {
	/// The key, as a condition names it after `user.`.
	pub key: String,
	/// How many people hold a value for it.
	pub people: Count,
}

/// One value at least one person holds for an attribute key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AttributeValueSummary {
	/// The value. Write it into a condition with
	/// [`quoted`](crate::condition::quoted).
	pub value: String,
	/// How many people hold it.
	pub people: Count,
}

/// One category the service has seen: in a send since it kept a history,
/// or in an opt-out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct CategorySummary {
	/// The category's name.
	pub category: String,
	/// How many people opted out of it.
	pub opted_out: Count,
	/// When the latest send in it was made, among the sends the service
	/// still remembers.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub last_sent: Option<leptoskit_db::Timestamp>,
}

/// How many people on one platform an estimated send would push to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EstimatedReach {
	/// The platform's push service.
	pub platform: PushPlatform,
	/// How many of the people the send would deliver to hold a live
	/// endpoint on it — one the condition admits, when the condition asks
	/// about devices.
	pub people: u32,
}

/// What [`estimate_audience`] answers: the audience a target has now, as
/// far as the estimate walked.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct AudienceEstimate {
	/// How many people the target matched.
	pub targeted: u32,
	/// How many of them opted out of the category, when one was named.
	pub skipped_opted_out: u32,
	/// How many would get an inbox row: `targeted` less the opted out.
	pub delivered: u32,
	/// Whether the walk reached the end of the audience. When `false`,
	/// every count here is of the first [`ESTIMATE_WALK`] people walked:
	/// read each as "at least".
	pub exact: bool,
	/// Of the people who would get a row, how many can be pushed to, per
	/// platform. A platform nobody among them is enrolled on is left out.
	pub reach: Vec<EstimatedReach>,
}

/// The size of what the service knows: people in the registry, people
/// who can be pushed to on each platform and whether that platform's
/// transport is configured here, how many topics have subscribers, and
/// which topic patterns are open to self-subscription.
///
/// Every count stops at a cap ([`OVERVIEW_COUNT_CAP`] people,
/// [`OVERVIEW_TOPICS_CAP`] topics), so the answer costs at most that many
/// index entries however large the tables are. Nothing in the answer
/// names a person, an endpoint, or a credential.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server.
#[rpc::query(service = "notifications")]
pub async fn notifications_overview(cx: &rpc::QueryCx) -> rpc::Result<Overview> {
	app_only(cx, "reads the notifications overview")?;
	crate::service::catalogue::overview(cx).await
}

/// The topics that have at least one subscriber, in name order, a page of
/// at most [`MAX_CATALOGUE_PAGE`] at a time, each with how many people
/// are subscribed (up to [`COUNT_CAP`]) and whether it is open to
/// self-subscription. `prefix` keeps only the topics whose name starts
/// with it, for a picker that narrows as someone types.
///
/// A topic nobody is subscribed to does not exist as far as the service
/// knows, so it is not listed.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a prefix that could not start a topic name
/// or a cursor that isn't one of this list's.
#[rpc::query(service = "notifications")]
pub async fn topics(
	cx: &rpc::QueryCx,
	page: rpc::PageArgs,
	#[serde(default)] prefix: Option<String>,
) -> rpc::Result<rpc::Page<TopicSummary>> {
	app_only(cx, "lists topics")?;

	if let Some(prefix) = prefix.as_deref().filter(|prefix| !prefix.is_empty()) {
		crate::check_topic(prefix).map_err(|problem| {
			rpc::Error::invalid(format!("a prefix is the start of a topic name: {problem}"))
		})?;
	}

	crate::service::catalogue::topics(cx, &page, prefix.as_deref()).await
}

/// The attribute keys at least one person holds, in key order, a page of
/// at most [`MAX_CATALOGUE_PAGE`] at a time, each with how many people
/// hold a value for it (up to [`COUNT_CAP`]).
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a cursor that isn't one of this list's.
#[rpc::query(service = "notifications")]
pub async fn attribute_keys(
	cx: &rpc::QueryCx,
	page: rpc::PageArgs,
) -> rpc::Result<rpc::Page<AttributeKeySummary>> {
	app_only(cx, "lists attribute keys")?;
	crate::service::catalogue::attribute_keys(cx.db(), &page).await
}

/// The values people hold for one attribute key, in value order, a page
/// of at most [`MAX_CATALOGUE_PAGE`] at a time, each with how many people
/// hold it (up to [`COUNT_CAP`]). `prefix` keeps only the values that
/// start with it.
///
/// Every value listed can be written into a condition:
/// [`quoted`](crate::condition::quoted) writes it, spaces and quotes
/// included.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a key that isn't one, a prefix no value
/// could start with, or a cursor that isn't one of this list's.
#[rpc::query(service = "notifications")]
pub async fn attribute_values(
	cx: &rpc::QueryCx,
	key: String,
	page: rpc::PageArgs,
	#[serde(default)] prefix: Option<String>,
) -> rpc::Result<rpc::Page<AttributeValueSummary>> {
	app_only(cx, "lists attribute values")?;
	crate::check_attribute_key(&key).map_err(rpc::Error::invalid)?;

	if let Some(prefix) = prefix.as_deref() {
		crate::check_attribute_value(prefix).map_err(|problem| {
			rpc::Error::invalid(format!(
				"a prefix is the start of an attribute value: {problem}"
			))
		})?;
	}

	crate::service::catalogue::attribute_values(cx.db(), &key, &page, prefix.as_deref()).await
}

/// The categories the service has seen — in a send it remembers, or in
/// someone's opt-out — in name order, a page of at most
/// [`MAX_CATALOGUE_PAGE`] at a time, each with how many people opted out
/// of it (up to [`COUNT_CAP`]) and when the latest send in it was made.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a cursor that isn't one of this list's.
#[rpc::query(service = "notifications")]
pub async fn categories(
	cx: &rpc::QueryCx,
	page: rpc::PageArgs,
) -> rpc::Result<rpc::Page<CategorySummary>> {
	app_only(cx, "lists categories")?;
	crate::service::catalogue::categories(cx.db(), &page).await
}

/// About how many people `target` would reach right now: matched, opted
/// out of `category` when one is named, left to deliver to, and how many
/// of those can be pushed to on each platform.
///
/// The estimate walks the audience the way a send's fanout would — the
/// topic index or the registry, the condition judged per person and per
/// device — but stops after [`ESTIMATE_WALK`] people. An audience it
/// walked to the end answers `exact: true` and the same counts
/// `validate_only` would. A larger one answers `exact: false`, and every
/// count is then of the people walked: at least that many. So the cost is
/// bounded however large the audience, and a screen can ask as someone
/// builds a condition. Before sending, ask [`send`](fn@crate::send) with
/// `validate_only` for the whole count.
///
/// Reach counts only live endpoints: one its provider invalidated is
/// not counted. Nothing is written, and nothing in the answer names a
/// person or an endpoint.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a target or category the checks refuse.
#[rpc::query(service = "notifications")]
pub async fn estimate_audience(
	cx: &rpc::QueryCx,
	target: SendTarget,
	#[serde(default)] category: Option<String>,
) -> rpc::Result<AudienceEstimate> {
	app_only(cx, "estimates an audience")?;
	crate::check_target(&target).map_err(rpc::Error::invalid)?;

	if let Some(category) = category.as_deref() {
		crate::check_category(category).map_err(rpc::Error::invalid)?;
	}

	crate::service::catalogue::estimate(cx.db(), &target, category.as_deref()).await
}

/// Refuses every caller but the app's server.
#[cfg(feature = "service")]
fn app_only(cx: &rpc::QueryCx, does: &str) -> rpc::Result<()> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(format!(
			"only the app {does}: call this from its server, with the service's token"
		)));
	}

	Ok(())
}
