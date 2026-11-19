//! Durable notifications for leptoskit apps.
//!
//! An app's server sends a notification to explicit users, to a topic's
//! subscribers, to the people a condition matches over topics and the
//! attributes the server set, or to everyone the service knows, and the
//! service stores one durable inbox row per recipient, in a database of
//! its own. The recipients read their own inbox from any device their
//! app signs in on: the web, a native app, or the server while it
//! renders. When the operator configures a push transport — APNs, FCM,
//! or Web Push — the same send also reaches the devices its recipients
//! enrolled ([`push`]): one durable delivery per notification and
//! endpoint, attempted by a background job with bounded retries, and
//! reported back to the app as sanitized status. Nothing here claims a
//! person saw a banner: provider acceptance is the farthest the service
//! can see, and delivery is at-least-once.
//!
//! The inbox queries are live, and time is one of the things that changes
//! what they answer: a scheduled row appears when it is due, an expiring
//! one leaves when it lapses, and an opt-out hides what was already sent.
//! Each of those re-runs every open subscription, through the wake rows
//! and jobs described on [`Wake`] and [`wake_send`].
//!
//! Without features, this crate is the operations' types, which an app
//! compiles to call the service, and the shared validation its server runs
//! before it sends. With the `service` feature, it's the service itself,
//! which `lk` runs when an app declares `[services.notifications]`.
//!
//! Who may call what:
//!
//! - [`send`], [`cancel`], [`send_status`], [`send_report`],
//!   [`subscribe_to_topic`], [`unsubscribe_from_topic`],
//!   [`set_person_attributes`], [`person_attributes`], [`forget_person`],
//!   [`push_deliveries`], and
//!   [`import_push_endpoints`](push::import_push_endpoints) answer only the
//!   app's server, signed with the service's token
//!   ([`rpc::client::call_as_app`]). A recipient has no operation that sends,
//!   steers a push, or describes anyone with attributes — segments must not be
//!   forgeable from the client side.
//! - [`inbox`], [`unread_count`], [`category_preferences`], [`mark_read`],
//!   [`mark_opened`], [`mark_dismissed`], [`set_category_preference`],
//!   [`register_push_endpoint`], [`push_endpoints`], [`revoke_push_endpoint`],
//!   [`subscribe`], [`unsubscribe`], and [`my_topics`] answer the signed-in
//!   person the app forwarded ([`rpc::server::ForwardedIdentity`]), and only
//!   ever touch their own rows. Anonymous calls, and calls that forge a user id
//!   in any header, are refused; the recipient is always read from the app's
//!   forwarded identity, never from an argument.
//!
//! On the web, the [`browser`] module is the person's own half of
//! enrollment: it serves the receiver worker that shows what arrives, and
//! enrolls, reconciles, and disables the browser's subscription from the
//! app's client, with the same forwarded identity and no service token. On
//! an iOS or macOS device, the [`native`] module is the same half for APNs:
//! it turns the device token the generated app delegate forwarded into the
//! same enrollment, with the same boundaries (and, on Android, the FCM
//! registration token).

use leptoskit_db::table;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

pub mod campaign;
pub mod catalogue;
pub mod condition;
pub mod history;
pub mod local;
pub mod options;
pub mod preview;

#[cfg(feature = "browser")]
pub mod browser;

#[cfg(feature = "native")]
pub mod native;

#[cfg(any(feature = "browser", feature = "native"))]
pub mod receive;

#[cfg(feature = "service")]
pub mod service;

pub mod push;

pub use history::HourCount;
pub use history::ListSends;
pub use history::PushCounts;
pub use history::ReasonCount;
pub use history::SendEntry;
pub use history::SendFilter;
pub use history::TargetKind;
pub use history::TargetSummary;
#[cfg(feature = "service")]
pub use history::list_sends;
pub use local::LocalTime;
pub use local::Variant;
pub use options::AndroidOptions;
pub use options::AppleOptions;
pub use options::BadgeSetting;
pub use options::InterruptionLevel;
pub use options::MessageKind;
pub use options::MessageOptions;
pub use options::NotificationSound;
pub use options::SendPriority;
pub use options::TextDirection;
pub use options::WebOptions;
pub use options::WebPushAction;
pub use push::DeviceDetails;

/// The service's name: the `[services.notifications]` table in the app's
/// `leptoskit.toml` must use it, because every operation is addressed to it.
pub const SERVICE: &str = "notifications";

/// The longest idempotency key, in bytes.
pub const MAX_KEY_LEN: usize = 255;

/// The longest category name, in bytes.
pub const MAX_CATEGORY_LEN: usize = 64;

/// The longest title, in bytes.
pub const MAX_TITLE_LEN: usize = 200;

/// The longest body, in bytes.
pub const MAX_BODY_LEN: usize = 2_000;

/// The largest `data`, a JSON object, in bytes.
pub const MAX_DATA_LEN: usize = 4_096;

/// The most recipients one send may name; duplicates collapse.
pub const MAX_RECIPIENTS: usize = 1_000;

/// The most category preferences one person may hold, so a client can't
/// grow the table without bound.
pub const MAX_PREFERENCES: usize = 100;

/// The most items one [`inbox`] page reads. Live pagination clamps again,
/// but tighter, so a screen scrolls a bounded amount.
pub const MAX_INBOX_PAGE: u32 = 100;

/// The most rows a pinned inbox page — one whose arguments name the row it
/// ends at, as a loaded page of a live list does — reads, whatever arrived
/// inside it since it was loaded. A few [`MAX_INBOX_PAGE`]s, not the
/// framework's ten thousand: the page's arguments come from the client.
pub const MAX_PINNED_INBOX_ROWS: u32 = 5 * MAX_INBOX_PAGE;

/// The longest topic name, in bytes: FCM's bound.
pub const MAX_TOPIC_LEN: usize = 128;

/// The most topics one person may hold. A person's devices all receive
/// what the person receives, so a topic list is per person, never per
/// device.
pub const MAX_TOPICS_PER_PERSON: usize = 500;

/// The most persons one [`subscribe_to_topic`] or
/// [`unsubscribe_from_topic`] call may name, mirroring FCM's batch bound.
pub const MAX_SUBSCRIBE_PERSONS: usize = 1_000;

/// The most items one [`my_topics`] page reads.
pub const MAX_TOPICS_PAGE: u32 = 100;

/// The longest attribute key, in bytes: `[a-z0-9_]`, the shape a
/// condition's `user.<key>` can name.
pub const MAX_ATTRIBUTE_KEY_LEN: usize = 64;

/// The longest attribute value, in bytes.
pub const MAX_ATTRIBUTE_VALUE_LEN: usize = 256;

/// The most attributes one person may hold, so a segment's evaluation
/// stays bounded.
pub const MAX_ATTRIBUTES_PER_PERSON: usize = 50;

/// The most persons one [`set_person_attributes`] call may name,
/// mirroring the topic batch bound.
pub const MAX_ATTRIBUTE_PERSONS: usize = 1_000;

/// The most keys one [`set_person_attributes`] call may name, `set` and
/// `remove` together: as many as a person can hold to set, and as many
/// again to remove, and no more — every key is a parameter of the call's
/// queries and a write per person.
pub const MAX_ATTRIBUTE_CHANGES: usize = 2 * MAX_ATTRIBUTES_PER_PERSON;

/// How many people a fanout job step delivers to, in one transaction: the
/// batch size a topic or condition send walks its audience in.
pub const FANOUT_BATCH: usize = 500;

/// The farthest ahead a notification may be scheduled or set to expire, in
/// seconds: about a year.
pub const MAX_SCHEDULE_SECS: i64 = 366 * 24 * 60 * 60;

/// The longest spec, as the JSON the service stores it as, in bytes: a
/// topic, condition, or everyone send keeps its spec for the fanout job.
/// Every send is held to it, whatever its target, so a spec that passes
/// [`check_spec`] always stores: `data` that is valid at
/// [`MAX_DATA_LEN`] can still grow past this once escaped inside the spec.
pub const MAX_SPEC_LEN: usize = 8_192;

/// The longest topic, condition, or everyone target, as the JSON the
/// service stores it as for the fanout job, in bytes.
pub const MAX_TARGET_LEN: usize = 1_200;

/// The longest [label](SendSpec::label) a send may carry, in bytes.
pub const MAX_LABEL_LEN: usize = 64;

/// One durable inbox row: the recipient's copy of a notification a send
/// delivered to them. The read, dismissed, and cancelled state lives here,
/// shared by every device signed in as [`Notification::person`], never
/// per-device.
///
/// Rows the app cancelled before they were due ([`cancel`]) stay in the
/// table with `cancelled_at` set, so a send's history stays explainable;
/// they never become visible.
///
/// One row per send and person, enforced by the database: a batch of a
/// fanout that replays after a crash can never write a person a second
/// copy of the same send.
#[table(
	name = "notifications",
	live,
	unique(send_key, person),
	index(person, id),
	index(person, kind, dismissed_at, read_at)
)]
pub struct Notification {
	/// A UUID v7 the service assigns, so rows order by arrival and pages
	/// have a stable cursor.
	#[db(key)]
	pub id: uuid::Uuid,
	/// The idempotency key of the send that created this row.
	#[db(min_len = 1, max_len = 255)]
	pub send_key: String,
	/// The recipient, as the app's server named them.
	pub person: uuid::Uuid,
	#[db(min_len = 1, max_len = 64)]
	pub category: String,
	#[db(min_len = 1, max_len = 200)]
	pub title: String,
	#[db(max_len = 2_000)]
	pub body: String,
	/// The send's `data`, as a JSON object; `{}` when it carried none.
	#[db(max_len = 4_096)]
	pub data: String,
	/// When it becomes visible: a scheduled send is invisible until this.
	pub visible_at: leptoskit_db::Timestamp,
	/// When it stops being visible. `None` never expires.
	pub expires_at: Option<leptoskit_db::Timestamp>,
	/// When the app cancelled it before it was due.
	pub cancelled_at: Option<leptoskit_db::Timestamp>,
	/// When the recipient read it, on any device.
	pub read_at: Option<leptoskit_db::Timestamp>,
	/// When the recipient dismissed it, on any device.
	pub dismissed_at: Option<leptoskit_db::Timestamp>,
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
	/// When the recipient first opened it — tapped its push, or one of
	/// the push's action buttons — on any device ([`mark_opened`]). The
	/// first open stands; a row nobody opened, and every row written
	/// before opens were recorded, holds `None`.
	pub opened_at: Option<leptoskit_db::Timestamp>,
	/// The row's kind: `notification`, the inbox row this has always
	/// been, or `data` — a silent background message's durable half. A
	/// data row never surfaces in an inbox or a badge count; it exists so
	/// the send's deliveries, status, cancellation, and idempotency work
	/// exactly as they do for a visible one.
	#[db(one_of("notification", "data"), default = "notification")]
	pub kind: String,
	/// The send's options, as the JSON it was sent as; `None` when the
	/// send carried none. The push engine reads it per delivery, so a
	/// badge set to the recipient's unread count resolves at push time.
	#[db(max_len = 4096)]
	pub options: Option<String>,
}

/// One send the app's server made. Keyed by its idempotency key, so a
/// retried send is recognised after a restart: the same key with the same
/// intent answers the stored result, and the same key with a different one
/// is a conflict.
///
/// A send made since the service kept a history also holds a
/// [`history_id`](Self::history_id), which is the order [`list_sends`]
/// pages in, and its category, which [`categories`] reads.
#[table(name = "sends", index(history_id), index(category, history_id))]
pub struct AppSend {
	#[db(key, min_len = 1, max_len = 255)]
	pub idempotency_key: String,
	/// A SHA-256 of the send's canonical intent: category, content,
	/// target, schedule, and expiry. Same key, same hash; anything else
	/// is a different send that must use another key.
	#[db(min_len = 64, max_len = 64)]
	pub intent: String,
	/// How many inbox rows the send created. For a send whose fanout runs
	/// as a job, the rows written so far.
	pub delivered: i64,
	/// How many recipients it skipped because they opted out of the
	/// category.
	pub skipped_opted_out: i64,
	/// How many people the target matched so far: every named recipient
	/// for a recipients send, every person the fanout matched for a topic
	/// or condition one.
	#[db(default = 0)]
	pub targeted: i64,
	/// Where the send stands, as [`SendState`]'s names. A recipients send
	/// is complete the moment it is made; a topic or condition send fans
	/// out until this says otherwise.
	#[db(one_of("fanning_out", "complete", "cancelled"), default = "complete")]
	pub state: String,
	/// The stored target, for a send whose fanout runs as a job. A
	/// recipients send needs none: its fanout happened in its own
	/// transaction.
	#[db(max_len = 1_200)]
	pub target: Option<String>,
	/// The stored spec, for the same sends, as the JSON it was sent as.
	#[db(max_len = 8_192)]
	pub spec: Option<String>,
	/// The last person the fanout delivered to: where a replayed job step
	/// picks up after. `None` before the first batch.
	pub cursor: Option<uuid::Uuid>,
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
	/// The send's [label](SendSpec::label), when it carried one: kept
	/// here for every kind of send, so [`send_report`] can answer it for
	/// a recipients send too, which stores no spec.
	#[db(max_len = 64)]
	pub label: Option<String>,
	/// A UUID v7 the service assigns when the send is made: the order
	/// [`list_sends`] pages in, newest first, and its cursor. `None` for
	/// a send made before the service kept a history, which the list
	/// leaves out.
	pub history_id: Option<uuid::Uuid>,
	/// The send's category, copied from its spec so [`categories`] can
	/// find each category's latest send through an index. `None` for a
	/// send made before the service kept a history.
	#[db(max_len = 64)]
	pub category: Option<String>,
}

/// One person's subscription to one topic. Topics are per person, not per
/// device: the inbox is per person, and every device signed in as
/// [`TopicSubscription::person`] receives what the person receives.
///
/// The app's server subscribes people in batches ([`subscribe_to_topic`]);
/// a person may subscribe themselves, to topics the operator opened for
/// it, with [`subscribe`]. The table is live, so an open [`my_topics`]
/// page follows both.
#[table(
	name = "topic_subscriptions",
	live,
	unique(person, topic),
	index(topic, person)
)]
pub struct TopicSubscription {
	pub id: leptoskit_db::Id<Self>,
	pub person: uuid::Uuid,
	/// The topic's name: 1 to [`MAX_TOPIC_LEN`] bytes of the characters
	/// FCM accepts, checked everywhere a topic is named.
	#[db(min_len = 1, max_len = 128)]
	pub topic: String,
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
}

/// One person's opt-out for one category. A send skips recipients who opted
/// out of its category, and an inbox hides rows whose category its reader
/// has since opted out of, so an opt-out takes hold on what is already
/// stored as well as on what comes after.
///
/// The table is live: setting a preference re-runs the open inbox and
/// badge subscriptions of the person who set it, and the preferences
/// query they may be watching, without any other write.
#[table(
	name = "category_preferences",
	live,
	unique(person, category),
	index(category, opted_out)
)]
pub struct Preference {
	pub id: leptoskit_db::Id<Self>,
	pub person: uuid::Uuid,
	#[db(min_len = 1, max_len = 64)]
	pub category: String,
	pub opted_out: bool,
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
}

/// One person the service knows. The registry every audience that isn't
/// a topic's subscribers or an explicit list walks: [`send`] with
/// [`SendTarget::Everyone`] reaches every row here, and an unanchored
/// condition — one that can match a person holding none of its topics —
/// walks the same rows.
///
/// A person becomes known through an act, never by being named in
/// someone else's: the app's server setting their attributes
/// ([`set_person_attributes`]), a push enrollment, a topic
/// subscription, or a category preference. [`forget_person`] removes
/// the row again, with everything else the service holds about them.
#[table(name = "audience_members", unique(person))]
pub struct AudienceMember {
	pub id: leptoskit_db::Id<Self>,
	pub person: uuid::Uuid,
	/// When the service first knew them. An upsert keeps it.
	#[db(created_at)]
	pub first_seen: leptoskit_db::Timestamp,
	/// When the service last wrote about them: an attribute, topic, or
	/// registry change bumps it.
	#[db(updated_at)]
	pub updated_at: leptoskit_db::Timestamp,
}

/// One attribute the app's server set for one person: the `plan` or
/// `country` a segment asks about, never anything a person set for
/// themselves, so an audience can't be forged from the client side.
///
/// At most [`MAX_ATTRIBUTES_PER_PERSON`] rows per person; keys are
/// `[a-z0-9_]{1,64}` and values at most
/// [`MAX_ATTRIBUTE_VALUE_LEN`] bytes, checked everywhere an attribute is
/// named, the condition grammar included.
#[table(name = "person_attributes", unique(person, key), index(key, value))]
pub struct PersonAttribute {
	pub id: leptoskit_db::Id<Self>,
	pub person: uuid::Uuid,
	#[db(min_len = 1, max_len = 64)]
	pub key: String,
	#[db(max_len = 256)]
	pub value: String,
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
	#[db(updated_at)]
	pub updated_at: leptoskit_db::Timestamp,
}

/// One wake row per person: the clock event for their open subscriptions.
///
/// The inbox and badge queries read the reader's wake row and ignore its
/// value, which puts `wakes` in their read set; when a send's schedule
/// turns — it becomes visible, or it expires — the [`wake_send`] job
/// writes the row of everyone the send reached, and each of their open
/// subscriptions re-runs with a fresh `now`, so a new item appears and an
/// expired one leaves without any other write. A wake that changes
/// nothing (the send was cancelled, the row dismissed) re-runs the query
/// and sends nothing, because its result didn't change.
///
/// One row per person, so the table stays as big as the audience, not as
/// the schedule.
#[table(name = "wakes", live, unique(person))]
pub struct Wake {
	pub id: leptoskit_db::Id<Self>,
	/// Whose inbox time changed for. The queries read this and nothing
	/// else of the row.
	pub person: uuid::Uuid,
	/// When they were last woken. The value is a record for operators,
	/// not something a query answers from.
	pub woken_at: leptoskit_db::Timestamp,
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
}

leptoskit_db::export_schema!();

/// Every migration, embedded for the service to apply when it starts.
#[cfg(feature = "service")]
pub static MIGRATIONS: &[leptoskit_db::Migration] = leptoskit_db::migrations!("migrations");

/// The same migrations written for Postgres.
#[cfg(feature = "postgres")]
pub static POSTGRES_MIGRATIONS: &[leptoskit_db::Migration] =
	leptoskit_db::migrations!("migrations-postgres");

/// What a send carries, apart from its key and its recipients: the content,
/// when it shows and stops showing, and the [options](MessageOptions) that
/// shape the notification each platform shows. The app's server builds one
/// per send, and the service stores it on every row — in the language of
/// the [variant](Variant) each person reads, when it carries any. The
/// lengths here are what [`check_send`] and the `notifications` table
/// enforce.
///
/// [`Default`] is every optional field unset, for `..SendSpec::default()`;
/// its empty category and title are not a send [`check_spec`] accepts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SendSpec {
	pub category: String,
	pub title: String,
	#[serde(default)]
	pub body: String,
	/// A JSON object, at most [`MAX_DATA_LEN`] bytes. `None` stores `{}`.
	#[serde(default)]
	pub data: Option<String>,
	/// When it becomes visible. `None` is now; earlier is now too.
	#[serde(default)]
	pub visible_at: Option<leptoskit_db::Timestamp>,
	/// When it stops being visible. Must be after visibility begins.
	#[serde(default)]
	pub expires_at: Option<leptoskit_db::Timestamp>,
	/// How the notification is shaped on each platform: a silent data
	/// message, priority, an image, a sound, a badge, a click link, and
	/// the per-platform overrides. `None` — and an all-default options
	/// value, which serializes to nothing — is the send exactly as it
	/// was before options existed.
	#[serde(default, skip_serializing_if = "MessageOptions::is_empty")]
	pub options: MessageOptions,
	/// The send's text in other languages, keyed by BCP 47 tag: each
	/// person's one inbox row, and its push, carry the text of the
	/// variant their locale matches, or the text above when none does.
	/// See [`local`] for how a person's locale is found and matched. A
	/// send without variants serializes without the field, exactly as it
	/// did before variants existed.
	#[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
	pub variants: std::collections::BTreeMap<String, Variant>,
	/// A wall-clock time the send becomes visible at in each person's own
	/// time zone, instead of the one instant `visible_at` names. A send
	/// sets one or the other. See [`local`] for how a person's time zone
	/// is found. A send without one serializes without the field.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub local_time: Option<LocalTime>,
	/// A short name of your own for the send, such as `spring-sale`, to
	/// tell it apart in reports: 1 to [`MAX_LABEL_LEN`] ASCII letters,
	/// digits, `-`, `_`, and `.`. The service stores it with the send and
	/// answers it in [`send_report`]; no device ever receives it. Several
	/// sends may share one label. It is part of what the idempotency key
	/// stands for, and a send without one serializes without the field.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub label: Option<String>,
}

impl SendSpec {
	/// Whether this is a silent data message: no inbox row, no badge
	/// count, delivery durable all the same.
	#[must_use]
	pub fn is_data(&self) -> bool {
		self.options.is_data()
	}
}

/// Who a send goes to. Explicit recipients are today's send, unchanged;
/// a topic, a condition, or everyone addresses people the service
/// knows, and fans out durably, in batches, until the whole audience has
/// its rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SendTarget {
	/// The people the app named, at most [`MAX_RECIPIENTS`] of them after
	/// duplicates collapse. One transaction, the answer immediately.
	Recipients(Vec<uuid::Uuid>),
	/// Everyone subscribed to one topic now and as the fanout reaches
	/// them.
	Topic(String),
	/// Everyone a [condition](condition) matches, written the way FCM
	/// writes conditions: `'news.sports' in topics && 'alerts.local' in
	/// topics`, over the app's own attributes: `user.plan == 'pro' &&
	/// !('beta' in topics)`, or over the devices people enrolled:
	/// `device.platform == 'apns' && device.app_version < '2.1'`. A
	/// condition that asks about devices is judged per device: the person
	/// is reached when it holds with no device or on one of their live
	/// endpoints, and the push goes only to the endpoints it holds for.
	Condition(String),
	/// Every person the service knows: its [registry](AudienceMember) — the
	/// people whose attributes the app set, who subscribed to a topic,
	/// enrolled a push endpoint, or set a category preference. A send to
	/// this target is the loudest thing the service does; see [`send`]
	/// for what that means.
	Everyone,
}

impl SendTarget {
	/// The stored form, for a send whose fanout runs as a job: the
	/// canonical JSON of the target, whatever the caller's key order was.
	#[cfg(feature = "service")]
	pub(crate) fn stored(&self) -> String {
		serde_json::to_string(self).expect("the target is plain JSON")
	}

	/// The target a stored row names, for the same sends.
	///
	/// # Errors
	///
	/// [`rpc::Error::server`], when a stored target no longer parses,
	/// which is a bug's shape, not a caller's.
	#[cfg(feature = "service")]
	pub(crate) fn from_stored(stored: &str) -> rpc::Result<Self> {
		serde_json::from_str(stored).map_err(|error| {
			rpc::Error::server(format!("a stored send target won't parse: {error}"))
		})
	}
}

/// Where a send stands. A recipients send is [`SendState::Complete`] the
/// moment it is made; a topic or condition send answers
/// [`SendState::FanningOut`] and stays there until its fanout job has
/// walked the whole audience, or the app cancelled it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SendState {
	/// The fanout job is still walking the audience; the counts in the
	/// answer are what it has written so far.
	FanningOut,
	/// Every person the target matched has their row, or was counted
	/// skipped.
	#[default]
	Complete,
	/// The app cancelled it, mid-fanout or after.
	Cancelled,
}

impl SendState {
	/// The stored name.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::FanningOut => "fanning_out",
			Self::Complete => "complete",
			Self::Cancelled => "cancelled",
		}
	}

	/// The state a stored name stands for, or `None` for anything else.
	#[must_use]
	pub fn from_str_value(name: &str) -> Option<Self> {
		match name {
			"fanning_out" => Some(Self::FanningOut),
			"complete" => Some(Self::Complete),
			"cancelled" => Some(Self::Cancelled),
			_ => None,
		}
	}
}

/// What a send came to. The counts describe the first run: a send repeated
/// with the same key answers the stored counts again, marked
/// [`SendResult::duplicate`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SendResult {
	/// How many recipients received a durable inbox row. A send that is
	/// still fanning out answers the rows written so far.
	pub delivered: u32,
	/// How many recipients were skipped because they opted out of the
	/// category.
	pub skipped_opted_out: u32,
	/// How many recipients the send named, after duplicates collapsed.
	/// A topic or condition send names no one, so it answers 0.
	pub recipients: u32,
	/// Whether this answer repeats an earlier send with the same key and
	/// the same intent, and nothing new was written.
	#[serde(default)]
	pub duplicate: bool,
	/// Where the send stands: [`SendState::Complete`] for a recipients
	/// send, [`SendState::FanningOut`] for a fresh topic or condition one.
	pub state: SendState,
	/// How many people the target matched: every named recipient for a
	/// recipients send, the audience found so far for a fanning-out one,
	/// or the audience a `validate_only` send would reach now.
	pub targeted: u32,
}

/// What [`send_status`] answers: where a send stands and what it has done
/// so far. For a recipients send, the final counts the moment it was
/// made; for a topic or condition send, the fanout's progress, complete
/// when its state says so.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SendProgress {
	/// Where the send stands.
	pub state: SendState,
	/// How many people the target matched so far.
	pub targeted: u32,
	/// How many inbox rows the send has written.
	pub delivered: u32,
	/// How many matched people it skipped for opting out of the category.
	pub skipped_opted_out: u32,
}

/// What [`send_report`] answers: where a send stands, and what became of
/// the inbox rows it wrote.
///
/// More will be reported here over time, so the type cannot be built or
/// matched exhaustively outside this crate: read the fields you need.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SendSummary {
	/// What [`send_status`] answers for the same send: its state and what
	/// it delivered and skipped. Kept on the send's own record, so it
	/// stays the same however many of the send's rows are pruned later.
	pub progress: SendProgress,
	/// The send's [label](SendSpec::label), when it carried one.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub label: Option<String>,
	/// When the send was made.
	pub created_at: leptoskit_db::Timestamp,
	/// What became of the send's rows, counted as the service stores
	/// them now.
	#[serde(default)]
	pub inbox: InboxCounts,
	/// The send as it was made: its content, schedule, options, and
	/// variants. `None` only for a recipients send made before the
	/// service kept a history, which stored none.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub spec: Option<SendSpec>,
	/// Who the send was for, without naming anyone.
	#[serde(default)]
	pub target: TargetSummary,
	/// The send's push deliveries, by platform and by where they stand,
	/// counted over the deliveries the service still stores. Empty for a
	/// send that reached nobody with an enrolled device, and on a server
	/// with no push configured.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub push: Vec<PushCounts>,
	/// When the send's rows were read, by the hour: the earliest
	/// [`MAX_REPORT_HOURS`](history::MAX_REPORT_HOURS) hours in which any
	/// was, oldest first. An hour nobody read in is left out.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub reads_by_hour: Vec<HourCount>,
}

/// What became of one send's inbox rows, counted over the rows the service
/// still stores. Retention deletes old rows ([`prune`]), and a pruned row
/// is counted nowhere here: for a send older than the operator's
/// retention, these are the counts of what is left, down to all zeroes,
/// while [`SendSummary::progress`] still says what the send delivered.
///
/// A silent data message is never in an inbox and has no open, so a data
/// send's rows never count as [`opened`](Self::opened).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct InboxCounts {
	/// How many of the send's rows are stored.
	pub rows: u32,
	/// How many of them their recipient has read — in the inbox, by
	/// opening the push, or by dismissing.
	pub read: u32,
	/// How many of them their recipient opened: tapped the push, or one
	/// of its action buttons ([`mark_opened`]).
	#[serde(default)]
	pub opened: u32,
	/// How many of them their recipient dismissed.
	pub dismissed: u32,
	/// How many of them the app cancelled before they were due.
	pub cancelled: u32,
}

/// What one [`send_test`] attempt came to, for one endpoint. Sanitized
/// exactly like a durable delivery's status: no provider text, no
/// credential.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TestSendOutcome {
	/// The endpoint's opaque id, as the person's own list answers it.
	pub endpoint: uuid::Uuid,
	/// Which platform's service it went through.
	pub platform: push::PushPlatform,
	/// Where the one attempt stands: `accepted`, `rejected`, or
	/// `skipped`. There is no queue behind a test send, so nothing is
	/// ever `enqueued` or `retrying` — a transient failure answers
	/// `rejected` with no reason, and calling again is the retry.
	pub state: push::DeliveryState,
	/// Why it ended, when it did and the code knows one.
	pub reason: Option<push::DeliveryReason>,
}

/// What one app-side topic subscription call changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TopicChanges {
	/// How many subscriptions the call wrote. Already-held ones change
	/// nothing, so a retried call answers 0.
	pub subscribed: u32,
	/// How many named persons were skipped: they already hold
	/// [`MAX_TOPICS_PER_PERSON`] topics and this would be one more.
	pub skipped_full: u32,
}

/// What one [`set_person_attributes`] call changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PersonAttributeChanges {
	/// How many persons the call newly registered in the service's
	/// registry. Already-known persons change nothing here, however many
	/// attributes moved.
	pub registered: u32,
	/// How many attribute values the call wrote. A value already stored
	/// exactly as sent changes nothing, so a retried call answers 0.
	pub set: u32,
	/// How many attributes the call removed. Absent ones count nothing.
	pub removed: u32,
	/// How many writes were skipped: the person already holds
	/// [`MAX_ATTRIBUTES_PER_PERSON`] attributes and the key was new.
	pub skipped_full: u32,
}

/// One item of a recipient's inbox: everything a screen needs to show it
/// and act on it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InboxItem {
	pub id: uuid::Uuid,
	pub category: String,
	pub title: String,
	pub body: String,
	/// The send's `data`, as a JSON object; `{}` when it carried none.
	pub data: String,
	/// When the send stored it.
	pub created_at: leptoskit_db::Timestamp,
	/// When the recipient read it, or `None` while unread.
	pub read_at: Option<leptoskit_db::Timestamp>,
	/// When the recipient dismissed it, or `None` while kept.
	pub dismissed_at: Option<leptoskit_db::Timestamp>,
}

/// Which items [`inbox`] reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InboxShow {
	/// Visible, not dismissed: the usual inbox.
	#[default]
	Active,
	/// Visible, not dismissed, not yet read: what a badge counts.
	Unread,
	/// Everything visible, dismissed or not.
	All,
	/// Only the dismissed ones, for a screen that can undo.
	Dismissed,
}

/// One category's opt-out, as a person reads their own preferences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CategoryPreference {
	pub category: String,
	pub opted_out: bool,
}

/// Checks a send before it's made, so an app's server rejects bad input
/// without a round trip and the service rejects it all the same: the
/// immutable shape every retry must keep (see [`check_send_shape`]), and
/// what the clock says about a new one (see [`check_send_timing`]).
///
/// # Errors
///
/// A message naming what's wrong: a length, a control character, `data`
/// that isn't a JSON object, a target that isn't one (an empty recipient
/// list, a malformed topic name, or a condition the parser refuses), or a
/// schedule that doesn't hold together.
pub fn check_send(
	idempotency_key: &str,
	spec: &SendSpec,
	target: &SendTarget,
) -> Result<(), String> {
	check_send_shape(idempotency_key, spec, target)?;
	check_timing(spec)
}

/// What the clock says about a new send's whole schedule: its instants
/// ([`check_send_timing`]) and, when it has one, its local time
/// ([`local::check_local_timing`]).
///
/// # Errors
///
/// A message naming what's wrong.
pub(crate) fn check_timing(spec: &SendSpec) -> Result<(), String> {
	check_send_timing(spec.visible_at, spec.expires_at)?;

	match spec.local_time.as_ref() {
		Some(local) => local::check_local_timing(local),
		None => Ok(()),
	}
}

/// Checks what can never change about a send, whatever the clock says
/// later: lengths, printable text, `data`'s shape, the target's own
/// bounds — the recipient bound, the topic name, or the whole condition
/// grammar — and an expiry that contradicts the visibility it names. A
/// send the service accepted passes this forever, so a retry of it is
/// judged by its key, not by the clock.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_send_shape(
	idempotency_key: &str,
	spec: &SendSpec,
	target: &SendTarget,
) -> Result<(), String> {
	check_key(idempotency_key)?;
	check_spec(spec)?;
	check_target(target)?;

	if let (Some(visible_at), Some(expires_at)) = (spec.visible_at, spec.expires_at)
		&& expires_at <= visible_at
	{
		return Err("an expiry must be after the notification becomes visible".into());
	}

	Ok(())
}

/// Checks a target on its own, the way [`check_send_shape`] checks a
/// send's: the recipient bound, the topic name, or the whole condition
/// grammar, and the stored length of a topic, condition, or everyone
/// target.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_target(target: &SendTarget) -> Result<(), String> {
	match target {
		SendTarget::Recipients(recipients) => {
			if recipients.is_empty() || recipients.len() > MAX_RECIPIENTS {
				return Err(format!("a send names 1 to {MAX_RECIPIENTS} recipients"));
			}
		}
		SendTarget::Topic(topic) => check_topic(topic)?,
		SendTarget::Condition(text) => {
			condition::check_condition(text).map_err(|error| error.to_string())?;
		}
		SendTarget::Everyone => {}
	}

	// A fanout send stores its target as JSON, where a condition's quoted
	// values can grow by escaping.
	if !matches!(target, SendTarget::Recipients(_)) && stored_len(target) > MAX_TARGET_LEN {
		return Err(format!(
			"a target is at most {MAX_TARGET_LEN} bytes once stored as JSON: shorten the condition"
		));
	}

	Ok(())
}

/// Checks a spec's own shape — category, content, `data`, options,
/// variants, and local time — wherever a spec is named: a send, or a
/// [`send_test`](fn@send_test) probe.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_spec(spec: &SendSpec) -> Result<(), String> {
	check_category(&spec.category)?;

	if spec.title.trim().is_empty() || spec.title.len() > MAX_TITLE_LEN || !valid_text(&spec.title)
	{
		return Err(format!(
			"a title is 1 to {MAX_TITLE_LEN} bytes, with no control characters"
		));
	}

	if spec.body.len() > MAX_BODY_LEN || !valid_text(&spec.body) {
		return Err(format!(
			"a body is at most {MAX_BODY_LEN} bytes, with no control characters"
		));
	}

	check_data(spec.data.as_deref())?;

	if let Some(label) = spec.label.as_deref() {
		check_label(label)?;
	}

	options::check_options(&spec.options)?;
	local::check_variants(spec)?;
	local::check_local_time(spec)?;

	if stored_len(spec) > MAX_SPEC_LEN {
		return Err(format!(
			"a send is at most {MAX_SPEC_LEN} bytes once stored as JSON: shorten its data, body, \
			 or options, or carry fewer or shorter variants"
		));
	}

	Ok(())
}

/// How many bytes `value` takes as the JSON the service stores.
pub(crate) fn stored_len(value: &impl Serialize) -> usize {
	serde_json::to_string(value).map_or(usize::MAX, |stored| stored.len())
}

/// Checks a send's [label](SendSpec::label): 1 to [`MAX_LABEL_LEN`] ASCII
/// letters, digits, `-`, `_`, and `.` — a name that is safe in a URL, a
/// file name, and a report's column, with nothing to escape.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_label(label: &str) -> Result<(), String> {
	let allowed = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.');

	if label.is_empty() || label.len() > MAX_LABEL_LEN || !label.bytes().all(allowed) {
		return Err(format!(
			"a label is 1 to {MAX_LABEL_LEN} ASCII letters, digits, `-`, `_`, or `.`"
		));
	}

	Ok(())
}

/// Checks a topic name wherever one is named — a send's target, a
/// subscription, a condition's quotes: 1 to [`MAX_TOPIC_LEN`] bytes of the
/// characters FCM accepts, so every path agrees on what a topic is.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_topic(topic: &str) -> Result<(), String> {
	if topic.is_empty() || topic.len() > MAX_TOPIC_LEN {
		return Err(format!("a topic name is 1 to {MAX_TOPIC_LEN} bytes"));
	}

	if !topic.bytes().all(topic_byte) {
		return Err(
			"a topic name uses the characters FCM accepts: letters, digits, `-`, `.`, `_`, `~`, \
			 and `%`"
				.into(),
		);
	}

	Ok(())
}

/// Whether a byte can appear in a topic name.
pub(crate) fn topic_byte(byte: u8) -> bool {
	byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'%')
}

/// Checks an attribute key wherever one is named — a
/// [`set_person_attributes`] call, or a condition's `user.<key>`: 1 to
/// [`MAX_ATTRIBUTE_KEY_LEN`] bytes of `a-z`, `0-9`, and `_`, lowercase
/// by design so a key in the app's server and one in a condition can't
/// drift by case.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_attribute_key(key: &str) -> Result<(), String> {
	if key.is_empty() || key.len() > MAX_ATTRIBUTE_KEY_LEN {
		return Err(format!(
			"an attribute key is 1 to {MAX_ATTRIBUTE_KEY_LEN} bytes of a-z, 0-9, and _"
		));
	}

	if !key
		.bytes()
		.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
	{
		return Err(
			"an attribute key uses `a-z`, `0-9`, and `_`, as in `plan` or `signup_campaign`".into(),
		);
	}

	Ok(())
}

/// Checks an attribute value: at most [`MAX_ATTRIBUTE_VALUE_LEN`] bytes,
/// with no control characters, so a value is text a screen could show.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_attribute_value(value: &str) -> Result<(), String> {
	if value.len() > MAX_ATTRIBUTE_VALUE_LEN {
		return Err(format!(
			"an attribute value is at most {MAX_ATTRIBUTE_VALUE_LEN} bytes"
		));
	}

	if !valid_text(value) {
		return Err("an attribute value has no control characters".into());
	}

	Ok(())
}

/// Checks an idempotency key.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_key(idempotency_key: &str) -> Result<(), String> {
	if idempotency_key.trim().is_empty() || idempotency_key.len() > MAX_KEY_LEN {
		return Err(format!("an idempotency key is 1 to {MAX_KEY_LEN} bytes"));
	}

	if !valid_text(idempotency_key) {
		return Err("an idempotency key has no control characters".into());
	}

	Ok(())
}

/// Checks a category name.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_category(category: &str) -> Result<(), String> {
	if category.trim().is_empty() || category.len() > MAX_CATEGORY_LEN {
		return Err(format!("a category is 1 to {MAX_CATEGORY_LEN} bytes"));
	}

	if !valid_text(category) {
		return Err("a category has no control characters".into());
	}

	Ok(())
}

/// Checks a send's `data`: absent, or a JSON object within
/// [`MAX_DATA_LEN`].
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_data(data: Option<&str>) -> Result<(), String> {
	let Some(data) = data else {
		return Ok(());
	};

	if data.len() > MAX_DATA_LEN {
		return Err(format!("data is at most {MAX_DATA_LEN} bytes of JSON"));
	}

	let parsed: serde_json::Value =
		serde_json::from_str(data).map_err(|error| format!("data isn't valid JSON: {error}"))?;

	if !parsed.is_object() {
		return Err("data is a JSON object".into());
	}

	Ok(())
}

/// Checks what the clock says about a new send: `visible_at` within
/// [`MAX_SCHEDULE_SECS`] of now, and `expires_at` after visibility begins
/// and within [`MAX_SCHEDULE_SECS`] of now. An expiry already past refuses
/// a new send even when it is after an explicit past `visible_at`: such a
/// notification would never be visible at all. Only a send the service
/// doesn't already hold is judged by this; an accepted one answers from
/// its stored outcome however the clock has moved on.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_send_timing(
	visible_at: Option<leptoskit_db::Timestamp>,
	expires_at: Option<leptoskit_db::Timestamp>,
) -> Result<(), String> {
	let now = chrono::Utc::now();

	if let Some(visible_at) = visible_at
		&& (visible_at - now).num_seconds() > MAX_SCHEDULE_SECS
	{
		return Err(format!(
			"a notification may be scheduled at most {MAX_SCHEDULE_SECS} seconds ahead"
		));
	}

	if let Some(expires_at) = expires_at {
		let begins = visible_at.unwrap_or(now);

		if expires_at <= begins {
			return Err("an expiry must be after the notification becomes visible".into());
		}

		if expires_at <= now {
			return Err(
				"an expiry is already past, so the notification would never be visible".to_owned(),
			);
		}

		if (expires_at - now).num_seconds() > MAX_SCHEDULE_SECS {
			return Err(format!(
				"an expiry may be at most {MAX_SCHEDULE_SECS} seconds ahead"
			));
		}
	}

	Ok(())
}

/// Bounded, printable text: no control characters.
fn valid_text(value: &str) -> bool {
	!value.chars().any(char::is_control)
}

/// Sends a notification to explicit users, a topic's subscribers, the
/// people a condition matches, or everyone the service knows: the app's
/// server only.
///
/// **Recipients** ([`SendTarget::Recipients`]) is the whole send in one
/// transaction: every named recipient gets one durable inbox row, visible
/// from now — or from `spec.visible_at`, when it schedules one — until it
/// expires, and the answer carries the final counts.
///
/// **A topic, a condition, or everyone** ([`SendTarget::Topic`],
/// [`SendTarget::Condition`], [`SendTarget::Everyone`]) can name any
/// number of people, so the send records itself, enqueues a fanout job
/// in the same transaction, and answers immediately with
/// [`SendState::FanningOut`]. The job walks the audience in batches of
/// [`FANOUT_BATCH`], ordered by person id, writing each batch's rows —
/// and its push deliveries — in one transaction, and snapshots the
/// audience as it reaches each person: someone who subscribes after the
/// fanout passed them by is not reached by this send. Ask
/// [`send_status`] how far it has come; a send that is still fanning out
/// answers the counts so far. Opt-outs, schedules, expiry, push
/// enqueueing, and live inbox updates behave exactly as for a recipients
/// send.
///
/// A condition with `device.` predicates is judged once with no device and
/// once per live endpoint of each person it walks — every endpoint of a
/// batch read in one query. The person gets one inbox row when any of
/// those holds, and the push goes only to the endpoints it holds for,
/// judged with each endpoint's details as they stand when the deliveries
/// are written — in the fanout's own transaction, even for a scheduled
/// send, whose deliveries then wait for its time.
///
/// Which audience a fanout walks depends on the target: a topic's
/// subscribers for a topic, the same for a condition that cannot match
/// without one of its topics, and the service's [person
/// registry](AudienceMember) for a condition that can — one anchored only on
/// attributes or devices, like `user.plan == 'pro'` or `device.platform ==
/// 'fcm'` — and for everyone. Everyone is
/// the loudest target: it reaches every registered person, so a send to
/// it should be rare, deliberate, and `validate_only`'d first — and
/// remember that people who opted out of the category are skipped
/// silently, which is the courtesy, not a bug.
///
/// `validate_only` checks everything and answers the audience the send
/// would reach right now — matched by the target, minus the people opted
/// out of the category — and writes nothing. It reads the audience page by
/// page with no transaction open, so a dry run over everyone the service
/// knows never holds up the service's writes.
///
/// It is an [action](rpc::ActionCx) for that reason: the dry run reads
/// outside any transaction, and the send itself is written in one
/// transaction of its own, retried when another gets in its way. A
/// request's idempotency header isn't kept for an action, and doesn't need
/// to be: the idempotency key is what makes a send safe to retry.
///
/// The send is idempotent by idempotency key: the same key with the same
/// intent answers the first run's result and writes nothing, across
/// restarts and however much time has passed — an accepted send with an
/// expiry still answers its stored outcome after the expiry. The same key
/// with a different intent is a conflict. Generate one key per logical
/// send and keep it stable on retry.
///
/// **Variants and local time.** A spec may carry its text in other
/// languages ([`SendSpec::variants`]) and may name a wall-clock time
/// instead of an instant ([`SendSpec::local_time`]). Both resolve per
/// person as the fanout reaches them — the app's `locale` or `time_zone`
/// attribute, then the person's most recently seen live endpoint, then the
/// send's default — with two reads per batch, and each person still gets
/// exactly one row: in their language, visible when the time arrives where
/// they are, or at once when it already has. A local-time send's pushes
/// are queued for their own instants, and it enqueues one [`wake_local`]
/// job per batch and distinct instant. See [`local`].
///
/// A send enqueues a [`wake_send`] job when it becomes visible — now for
/// an immediate send, `visible_at` for a scheduled one — and another at
/// its `expires_at`, in the send's own transaction: when the time comes,
/// the job re-runs every open inbox and badge subscription of everyone
/// the send reached, so a new item appears and an expiring one leaves
/// without any other write.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::Invalid`] for input the checks refuse — the immutable
/// shape always, the schedule of a send the service doesn't already hold
/// — and [`rpc::Error::Conflict`] when the key was used for a different
/// send.
#[rpc::action(service = "notifications")]
pub async fn send(
	cx: &rpc::ActionCx,
	idempotency_key: String,
	spec: SendSpec,
	target: SendTarget,
	#[serde(default)] validate_only: bool,
) -> rpc::Result<SendResult> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app sends notifications: call this from its server, with the service's token",
		));
	}

	// What the send is, not when it is: a retry of an accepted send must be
	// judged by its key alone, or an expiry would make it unanswerable.
	check_send_shape(&idempotency_key, &spec, &target).map_err(rpc::Error::invalid)?;

	if validate_only {
		// A dry run follows the new-send rules — the clock applies — but
		// no key is read and nothing is written.
		check_timing(&spec).map_err(rpc::Error::invalid)?;
		service::local::check_default_zone(&spec)?;
		let audience = cx
			.query(service::DryRun {
				spec,
				target: target.clone(),
			})
			.await?;

		return Ok(SendResult {
			delivered: audience.delivered,
			skipped_opted_out: audience.skipped_opted_out,
			recipients: matches!(target, SendTarget::Recipients(_))
				.then_some(audience.targeted)
				.unwrap_or(0),
			duplicate: false,
			state: match target {
				SendTarget::Recipients(_) => SendState::Complete,
				_ => SendState::FanningOut,
			},
			targeted: audience.targeted,
		});
	}

	// Boxed: the write's future holds the whole spec and target, which is
	// more than a caller's stack frame should carry.
	Box::pin(cx.mutate(service::SendWrite {
		idempotency_key,
		spec,
		target,
	}))
	.await
}

/// The body of [`send`](fn@send)'s write, in its own transaction: the
/// stored-send lookup, then the whole recipients fanout or the fanout
/// send's record and its first job.
///
/// # Errors
///
/// As [`send`](fn@send).
#[cfg(feature = "service")]
pub(crate) async fn send_write(
	cx: &rpc::MutationCx,
	idempotency_key: String,
	spec: SendSpec,
	target: SendTarget,
) -> rpc::Result<SendResult> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app sends notifications: call this from its server, with the service's token",
		));
	}

	let intent = service::intent_hash(&spec, &target);

	if let Some(existing) = service::find_send(cx.tx(), &idempotency_key).await? {
		return service::answer_send(&existing, &intent);
	}

	// New send, so the clock applies: how far ahead it may schedule. So
	// does the time zone database, for a local time's default zone.
	check_timing(&spec).map_err(rpc::Error::invalid)?;
	service::local::check_default_zone(&spec)?;

	// A recipients send is the whole fanout in this transaction.
	if let SendTarget::Recipients(named) = &target {
		let recipients: std::collections::BTreeSet<uuid::Uuid> = named.iter().copied().collect();
		let targeted = recipients.len().try_into().unwrap_or(u32::MAX);

		match service::write_send(cx.tx(), &idempotency_key, &intent, targeted, &spec).await {
			Ok(()) => {}
			// Another send with this key committed between the read and
			// the write. On Postgres the unique violation ends this
			// transaction, so its outcome can't be read here: the
			// mutation runs again from the start, finds the send, and
			// answers from the store.
			Err(leptoskit_db::DbError::Conflict { .. }) => return Err(rpc::Error::Contended),
			Err(error) => return Err(error.into()),
		}

		// Each recipient's language and local time, for the whole send in
		// two reads — and in none for a send with neither.
		let resolved =
			service::local::resolve(cx.tx(), &spec, &recipients, leptoskit_db::now()).await?;
		let fanout =
			service::fanout(cx.tx(), &idempotency_key, &spec, &recipients, &resolved).await?;
		service::record_outcome(cx.tx(), &idempotency_key, &fanout).await?;
		service::schedule_wakes(cx, &idempotency_key, &spec, Some(&fanout)).await?;

		// A send on a server with push configured fans its deliveries
		// out; on one without, or from a route that carries no
		// configuration, the inbox is the whole send.
		if service::push::Push::of(cx).is_ok() {
			service::push::schedule(cx, &idempotency_key, &spec, &fanout).await?;
		}

		return Ok(SendResult {
			delivered: fanout.delivered,
			skipped_opted_out: fanout.skipped_opted_out,
			recipients: targeted,
			duplicate: false,
			state: SendState::Complete,
			targeted,
		});
	}

	// A topic or a condition can name any number of people: the send
	// records itself — target, spec, and its key — and the fanout job
	// walks the audience from here. The job exists exactly when the send
	// lands, and its key keeps a raced or retried send from doubling it.
	match service::write_fanout_send(cx.tx(), &idempotency_key, &intent, &target, &spec).await {
		Ok(()) => {}
		Err(leptoskit_db::DbError::Conflict { .. }) => return Err(rpc::Error::Contended),
		Err(error) => return Err(error.into()),
	}

	cx.enqueue(FanoutSend {
		send_key: idempotency_key.clone(),
		step: 0,
	})
	.at(leptoskit_db::now())
	.key(service::job_key("fanout", &idempotency_key, 0))
	.await?;

	service::schedule_wakes(cx, &idempotency_key, &spec, None).await?;

	Ok(SendResult {
		delivered: 0,
		skipped_opted_out: 0,
		recipients: 0,
		duplicate: false,
		state: SendState::FanningOut,
		targeted: 0,
	})
}

/// One step of a topic, condition, or everyone send's fanout: the batch
/// of people the job [`send`](fn@send) enqueued walks next, delivered in
/// one transaction with their push deliveries, and the successor step
/// enqueued when more audience remains. The stored cursor is where a
/// replayed step picks up after, and the send's row is what a step checks
/// before it writes, so a [`cancel`] that lands between steps ends the
/// fanout with nothing more written. One send's steps run one at a time,
/// and a step that finds its batch taken by another run leaves the next
/// step in place before it ends, so the fanout always reaches its end.
///
/// Only the service's own workers run it; the app never enqueues it.
#[rpc::job(
	service = "notifications",
	queue = "fanout",
	retries = 8,
	timeout = 5m,
	serialize_by = send_key
)]
pub async fn fanout_send(cx: &rpc::JobCx, send_key: String, step: u32) -> rpc::Result<()> {
	service::fanout_step(cx, &send_key, step).await
}

/// Where a send stands and what it has done so far: the answer for the
/// app's server, from the send's own bookkeeping row. A recipients send
/// answers its final counts; a topic or condition send answers the
/// fanout's progress, and once its state is [`SendState::Complete`] the
/// counts are final too.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::NotFound`] for a key no send holds, and
/// [`rpc::Error::Invalid`] for a malformed key.
#[rpc::query(service = "notifications")]
pub async fn send_status(cx: &rpc::QueryCx, idempotency_key: String) -> rpc::Result<SendProgress> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app reads send status: call this from its server, with the service's token",
		));
	}

	check_key(&idempotency_key).map_err(rpc::Error::invalid)?;

	let Some(send) = service::find_send(cx.db(), &idempotency_key).await? else {
		return Err(rpc::Error::not_found(
			"no send with this idempotency key exists",
		));
	};

	Ok(service::progress_of(&send))
}

/// What became of a send: its [progress](SendProgress), as
/// [`send_status`] answers it, its label, when it was made, the send as
/// it was made and who it was for ([`TargetSummary`]), how many of the
/// rows it wrote were read, opened, dismissed, and cancelled
/// ([`InboxCounts`]), its push deliveries by platform and state
/// ([`PushCounts`]), and when its rows were read, by the hour
/// ([`HourCount`]).
///
/// The counts are three reads over the send's own rows and deliveries,
/// each found through an index that leads with the send's key — work in
/// step with the size of the send, never with the size of the table — so
/// ask for a report when someone looks at it, not on a timer for every
/// send. They count the rows the
/// service still stores: after retention has pruned a send's rows they
/// shrink, down to zero, while the progress stays as it was; and once the
/// send's own record is pruned the send is not found.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::NotFound`] for a key no send holds, and
/// [`rpc::Error::Invalid`] for a malformed key.
#[rpc::query(service = "notifications")]
pub async fn send_report(cx: &rpc::QueryCx, idempotency_key: String) -> rpc::Result<SendSummary> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app reads a send's report: call this from its server, with the service's \
			 token",
		));
	}

	check_key(&idempotency_key).map_err(rpc::Error::invalid)?;

	let Some(send) = service::find_send(cx.db(), &idempotency_key).await? else {
		return Err(rpc::Error::not_found(
			"no send with this idempotency key exists",
		));
	};

	Ok(SendSummary {
		progress: service::progress_of(&send),
		inbox: service::inbox_counts(cx.db(), &idempotency_key).await?,
		spec: service::history::spec_of(&send),
		target: service::history::target_of(&send),
		push: service::history::push_counts(cx.db(), &idempotency_key).await?,
		reads_by_hour: service::history::reads_by_hour(cx.db(), &idempotency_key).await?,
		created_at: send.created_at,
		label: send.label,
	})
}

/// Wakes the people a send reached, when its schedule turns: the job
/// every send enqueues when it becomes visible, and any send with an
/// expiry at its `expires_at`. It writes one [`Wake`] row per person the
/// send delivered to, in bounded transactions, which re-run their open
/// inbox and badge subscriptions with a fresh `now`.
///
/// The wake runs whether or not a screen is watching, and whether or not
/// the send was cancelled or its rows dismissed in the meantime: a re-run
/// that changes nothing sends nothing. Only the service's own workers run
/// it ([`service::workers`]); the app never enqueues it.
#[rpc::job(service = "notifications")]
pub async fn wake_send(cx: &rpc::JobCx, send_key: String) -> rpc::Result<()> {
	service::wake(cx.db(), &send_key).await
}

/// Wakes one fanout batch's people when the send becomes visible: the
/// people with rows of the send in `(after, through]`, at most
/// [`FANOUT_BATCH`] of them. A batch that is visible when it commits wakes
/// its people in its own transaction; this job is for a batch that commits
/// while its send is still scheduled, so its people hear about it however
/// the send's own wake at `visible_at` raced its commit.
///
/// Only the service's own workers run it; the app never enqueues it.
#[rpc::job(service = "notifications")]
pub async fn wake_batch(
	cx: &rpc::JobCx,
	send_key: String,
	after: Option<uuid::Uuid>,
	through: uuid::Uuid,
) -> rpc::Result<()> {
	service::wake_range(cx.db(), &send_key, after, through, None).await
}

/// Wakes the people of a [local-time](LocalTime) send whose time has just
/// arrived: those with rows of the send in `(after, through]` that become
/// visible at `at`. A local-time send is visible at a different instant in
/// each time zone, so a recipients send enqueues one of these per distinct
/// instant still ahead, and a fanout one per batch and distinct instant:
/// the jobs grow with the time zones an audience spans, never with its
/// people, and each person is woken once, at their own time.
///
/// Only the service's own workers run it; the app never enqueues it.
#[rpc::job(service = "notifications")]
pub async fn wake_local(
	cx: &rpc::JobCx,
	send_key: String,
	after: Option<uuid::Uuid>,
	through: uuid::Uuid,
	at: leptoskit_db::Timestamp,
) -> rpc::Result<()> {
	service::wake_range(cx.db(), &send_key, after, through, Some(at)).await
}

/// Cancels the rows of a large fanout send that [`cancel`] left, in
/// batches of [`FANOUT_BATCH`], each in a transaction of its own, so a
/// cancel never writes a whole audience in one transaction.
///
/// Only the service's own workers run it; the app never enqueues it.
#[rpc::job(service = "notifications", serialize_by = send_key)]
pub async fn cancel_rows(cx: &rpc::JobCx, send_key: String) -> rpc::Result<()> {
	service::cancel_rows(cx.db(), &send_key, None).await
}

/// Cancels the rows of a large [local-time](LocalTime) send that were
/// still ahead when [`cancel`] was asked — the ones not yet visible at
/// `hidden_after`, and any silent data row — in batches of
/// [`FANOUT_BATCH`], leaving what people had already seen.
///
/// Only the service's own workers run it; the app never enqueues it.
#[rpc::job(service = "notifications", serialize_by = send_key)]
pub async fn cancel_hidden_rows(
	cx: &rpc::JobCx,
	send_key: String,
	hidden_after: leptoskit_db::Timestamp,
) -> rpc::Result<()> {
	service::cancel_rows(cx.db(), &send_key, Some(hidden_after)).await
}

/// Erases the rest of a forgotten person's inbox — the rows
/// [`forget_person`] left because there were more than one batch of them —
/// with those rows' push deliveries, in batches of [`FANOUT_BATCH`], each a
/// write of its own. `through` is the newest row id the erasure covers:
/// rows a later send writes for the person are not its to remove.
///
/// Only the service's own workers run it; the app never enqueues it.
#[rpc::job(service = "notifications", serialize_by = person)]
pub async fn forget_rows(
	cx: &rpc::JobCx,
	person: uuid::Uuid,
	through: uuid::Uuid,
) -> rpc::Result<()> {
	service::forget_rows(cx.db(), person, through).await
}

/// Starts the hourly pruning sweep: deletes what the operator's retention
/// lets go of — notifications nobody can see any more, inbox rows past
/// their age, finished push deliveries, and old send records — in batches,
/// each a short write of its own. What the service still works with is
/// never deleted, whatever its age: [`service::retention`] has the rules,
/// and [`service::config::Retention`] the settings and their defaults.
///
/// A sweep that outlasts this run's share of time goes on in
/// [`prune_rows`]. Only the service's own workers run it; the app never
/// enqueues it.
#[rpc::cron("17 * * * *")]
#[rpc::job(service = "notifications", queue = "prune", timeout = 5m)]
pub async fn prune(cx: &rpc::JobCx) -> rpc::Result<()> {
	service::retention::step(
		cx,
		leptoskit_db::now().timestamp(),
		service::retention::Stage::FIRST,
		None,
	)
	.await
}

/// Goes on with a pruning sweep where the step before it stopped: `stage`
/// is the table it was in, `after` the last row it considered there, and
/// `sweep` when the sweep began, in Unix seconds — a sweep still going when
/// the next hour's starts ends, and that one covers what it had left.
///
/// Only the service's own workers run it; the app never enqueues it.
#[rpc::job(service = "notifications", queue = "prune", timeout = 5m)]
pub async fn prune_rows(
	cx: &rpc::JobCx,
	sweep: i64,
	stage: String,
	after: Option<String>,
) -> rpc::Result<()> {
	let stage = service::retention::Stage::from_str_value(&stage)
		.ok_or_else(|| rpc::Error::server(format!("`{stage}` isn't a pruning stage")))?;

	service::retention::step(cx, sweep, stage, after).await
}

/// Cancels a scheduled send before it's due: every row it created stops
/// being visible, forever. Once a row of the send is visible the send can
/// no longer be cancelled, and a key the service doesn't know is not found.
/// A send the service knows with no rows left to cancel — every recipient
/// opted out, or it was already cancelled — answers `0`, whenever it's
/// asked: a known send is always cancellable to a point, idempotently.
///
/// A send that is still fanning out can be cancelled mid-fanout: the
/// fanout stops before its next batch, the rows written so far are
/// cancelled — visible or not, because an audience this large can't wait
/// for the schedule to turn — and the send answers
/// [`SendState::Cancelled`] from then on, and pushes nothing more.
///
/// A [local-time](LocalTime) send is due at a different moment for each
/// person, so it stays cancellable while anyone is still waiting: once
/// some of its rows are visible, a cancel stops exactly the rows that are
/// not — and any silent data row — and leaves what people have seen. Asked
/// again, it answers `0`; with nobody left waiting and nothing ever
/// stopped, it is the conflict above.
///
/// The answer is how many rows the cancel stops. A topic, condition, or
/// everyone send with more than [`FANOUT_BATCH`] of them has them
/// cancelled in batches by a background job, so one cancel never writes a
/// whole audience in one transaction: until the job is through, asking
/// again answers how many are left.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::NotFound`] for an unknown key, and
/// [`rpc::Error::Conflict`] once the send is complete and visible.
#[rpc::mutation(service = "notifications")]
pub async fn cancel(cx: &rpc::MutationCx, idempotency_key: String) -> rpc::Result<u32> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app cancels sends: call this from its server, with the service's token",
		));
	}

	check_key(&idempotency_key).map_err(rpc::Error::invalid)?;

	service::cancel(cx, &idempotency_key).await
}

/// The signed-in person's durable inbox, newest first, one page at a time.
/// Each page is a live query: a send that reaches this person, or a read or
/// dismissal from another device, updates the screen watching it.
///
/// Only visible rows are here: scheduled ones wait for their time,
/// cancelled ones never arrive, and expired ones have gone. Categories the
/// reader opted out of are hidden too, whatever was sent before they
/// opted out.
///
/// A page is at most [`MAX_INBOX_PAGE`] rows. A pinned page — one that
/// names the row it ends at — reads what lies inside it, at most
/// [`MAX_PINNED_INBOX_ROWS`] rows, keeping the newest.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person.
#[rpc::query(service = "notifications")]
pub async fn inbox(
	cx: &rpc::QueryCx,
	page: rpc::PageArgs,
	#[serde(default)] show: InboxShow,
) -> rpc::Result<rpc::Page<InboxItem>> {
	let me = service::me(cx.identity())?;
	let mut page = page;
	page.limit = page.limit.clamp(1, MAX_INBOX_PAGE);
	let now = leptoskit_db::now();
	let opted_out = service::opted_out_categories(cx.db(), me).await?;
	// The clock is part of the answer, so the person's wake row is read
	// too: it puts `wakes` in this query's read set, and the wake job a
	// schedule or expiry runs writes it, re-running every open
	// subscription of theirs with a fresh `now`.
	service::follow_wake(cx.db(), me).await?;

	let mut query = Notification::query()
		.filter(|notification| service::visible(notification, me, now, &opted_out));

	query = match show {
		InboxShow::Active => query.filter(|notification| notification.dismissed_at.is_null()),
		InboxShow::Unread => {
			query.filter(|notification| {
				notification.dismissed_at.is_null() & notification.read_at.is_null()
			})
		}
		InboxShow::All => query,
		InboxShow::Dismissed => {
			query.filter(|notification| notification.dismissed_at.is_not_null())
		}
	};

	let page_of_rows = service::paginate_bounded(
		cx.db(),
		query,
		&page,
		MAX_PINNED_INBOX_ROWS,
		|notification| notification.id.desc(),
		|notification| notification.id,
	)
	.await?;

	Ok(rpc::Page {
		items: page_of_rows
			.items
			.into_iter()
			.map(service::inbox_item)
			.collect(),
		end: page_of_rows.end,
		has_more: page_of_rows.has_more,
	})
}

/// How many visible inbox items the signed-in person hasn't read and
/// hasn't dismissed: what a badge shows. Live, like [`inbox`].
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person.
#[rpc::query(service = "notifications")]
pub async fn unread_count(cx: &rpc::QueryCx) -> rpc::Result<u64> {
	let me = service::me(cx.identity())?;
	let now = leptoskit_db::now();
	let opted_out = service::opted_out_categories(cx.db(), me).await?;
	// As in [`inbox`]: the wake row makes time a tracked read.
	service::follow_wake(cx.db(), me).await?;

	let count = Notification::query()
		.filter(|notification| service::visible(notification, me, now, &opted_out))
		.filter(|notification| notification.read_at.is_null() & notification.dismissed_at.is_null())
		.count(cx.db())
		.await?;

	Ok(count)
}

/// Marks one of the signed-in person's notifications read, on every device
/// at once. Reading an already-read one does nothing.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person,
/// and [`rpc::Error::NotFound`] unless the notification is theirs.
#[rpc::mutation(service = "notifications")]
pub async fn mark_read(cx: &rpc::MutationCx, notification: uuid::Uuid) -> rpc::Result<()> {
	let me = service::me(cx.identity())?;
	let row = service::own_row(cx.tx(), me, notification).await?;

	if row.read_at.is_none() {
		let now = leptoskit_db::now();

		Notification::update()
			.filter(|row| row.id.eq(notification) & row.read_at.is_null())
			.set(|row| row.read_at, Some(now))
			.run(cx.tx())
			.await?;
	}

	Ok(())
}

/// Records that the signed-in person opened one of their notifications:
/// they tapped its push, or one of the push's action buttons. Call it from
/// the place your app handles the tap — [`receive::Opened::mark_opened`]
/// does, in one line, on every platform.
///
/// The first open is the one recorded. Opening the same notification
/// again, from this device or another, changes nothing and succeeds, so a
/// retried call is safe. Opening also marks the notification read, in the
/// same write, when it wasn't already: someone who tapped it has seen it,
/// and their badge on every device says so.
///
/// A silent data message is never shown, so it has no open: its id is
/// answered like one that isn't there.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person,
/// and [`rpc::Error::NotFound`] unless the notification is theirs.
#[rpc::mutation(service = "notifications")]
pub async fn mark_opened(cx: &rpc::MutationCx, notification: uuid::Uuid) -> rpc::Result<()> {
	let me = service::me(cx.identity())?;
	let row = service::own_row(cx.tx(), me, notification).await?;

	if row.kind != MessageKind::Notification.as_str() {
		return Err(service::not_in_inbox());
	}

	if row.opened_at.is_none() {
		let now = leptoskit_db::now();
		let read_at = row.read_at.or(Some(now));

		Notification::update()
			.filter(|row| row.id.eq(notification) & row.opened_at.is_null())
			.set(|row| row.opened_at, Some(now))
			.set(|row| row.read_at, read_at)
			.run(cx.tx())
			.await?;
	}

	Ok(())
}

/// Dismisses one of the signed-in person's notifications, on every device
/// at once: it leaves their inbox, though [`InboxShow::Dismissed`] still
/// finds it. Dismissing also marks it read. Dismissing an already-dismissed
/// one does nothing.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person,
/// and [`rpc::Error::NotFound`] unless the notification is theirs.
#[rpc::mutation(service = "notifications")]
pub async fn mark_dismissed(cx: &rpc::MutationCx, notification: uuid::Uuid) -> rpc::Result<()> {
	let me = service::me(cx.identity())?;
	let row = service::own_row(cx.tx(), me, notification).await?;
	let now = leptoskit_db::now();
	let read_at = row.read_at.or(Some(now));

	if row.dismissed_at.is_none() {
		Notification::update()
			.filter(|row| row.id.eq(notification) & row.dismissed_at.is_null())
			.set(|row| row.dismissed_at, Some(now))
			.set(|row| row.read_at, read_at)
			.run(cx.tx())
			.await?;
	}

	Ok(())
}

/// Opts the signed-in person into or out of one category. Out means sends
/// to this category skip them from now on, and their inbox stops showing
/// what it already holds of it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person,
/// [`rpc::Error::Invalid`] for a bad category name, and
/// [`rpc::Error::Invalid`] when they already hold [`MAX_PREFERENCES`]
/// categories and this is a new one.
#[rpc::mutation(service = "notifications")]
pub async fn set_category_preference(
	cx: &rpc::MutationCx,
	category: String,
	opted_out: bool,
) -> rpc::Result<()> {
	let me = service::me(cx.identity())?;
	check_category(&category).map_err(rpc::Error::invalid)?;

	let existing = Preference::query()
		.filter(|preference| preference.person.eq(me) & preference.category.eq(category.clone()))
		.first(cx.tx())
		.await?;

	if let Some(mut preference) = existing {
		preference.opted_out = opted_out;
		preference.save(cx.tx()).await?;
		service::register_persons(cx.tx(), &std::collections::BTreeSet::from([me])).await?;

		return Ok(());
	}

	if Preference::query()
		.filter(|preference| preference.person.eq(me))
		.count(cx.tx())
		.await?
		>= u64::try_from(MAX_PREFERENCES).unwrap_or(u64::MAX)
	{
		return Err(rpc::Error::invalid(format!(
			"a person holds at most {MAX_PREFERENCES} category preferences"
		)));
	}

	let inserted: leptoskit_db::Result<()> = NewPreference {
		person: me,
		category,
		opted_out,
	}
	.insert(cx.tx())
	.await
	.map(|_| ());

	match inserted {
		// Another device inserted this category between the read and
		// the write. On Postgres the unique violation ends this
		// transaction, so its row can't be read here: the mutation
		// runs again from the start, finds the row the other device
		// wrote, and sets it, so the last setting to commit is the one
		// that holds — neither device's answer is silently dropped.
		Err(leptoskit_db::DbError::Conflict { .. }) => Err(rpc::Error::Contended),
		other => {
			other.map_err(rpc::Error::from)?;
			service::register_persons(cx.tx(), &std::collections::BTreeSet::from([me])).await?;

			Ok(())
		}
	}
}

/// The signed-in person's category preferences, by category name.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person.
#[rpc::query(service = "notifications")]
pub async fn category_preferences(cx: &rpc::QueryCx) -> rpc::Result<Vec<CategoryPreference>> {
	let me = service::me(cx.identity())?;

	let rows = Preference::query()
		.filter(|preference| preference.person.eq(me))
		.order_by(|preference| preference.category.asc())
		.limit(u64::try_from(MAX_PREFERENCES).unwrap_or(u64::MAX))
		.all(cx.db())
		.await?;

	Ok(rows
		.into_iter()
		.map(|preference| {
			CategoryPreference {
				category: preference.category,
				opted_out: preference.opted_out,
			}
		})
		.collect())
}

/// Subscribes people to a topic, in one batch: the app's server only, the
/// way FCM's batch subscribe is. At most [`MAX_SUBSCRIBE_PERSONS`] people
/// per call, duplicates collapse, and the call is idempotent: people who
/// already hold the topic change nothing and count nothing. A person
/// holds at most [`MAX_TOPICS_PER_PERSON`] topics; people at that bound
/// are skipped and counted in [`TopicChanges::skipped_full`], so one full
/// person in a large batch never refuses the rest.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a bad topic name or an empty or oversized
/// batch.
#[rpc::mutation(service = "notifications")]
pub async fn subscribe_to_topic(
	cx: &rpc::MutationCx,
	topic: String,
	persons: Vec<uuid::Uuid>,
) -> rpc::Result<TopicChanges> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app subscribes people in batches: call this from its server, with the \
			 service's token",
		));
	}

	service::subscribe_batch(cx.tx(), &topic, persons).await
}

/// Unsubscribes people from a topic, in one batch: the app's server only,
/// the mirror of [`subscribe_to_topic`]. People who didn't hold the topic
/// count nothing, so a retried call answers 0.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a bad topic name or an empty or oversized
/// batch.
#[rpc::mutation(service = "notifications")]
pub async fn unsubscribe_from_topic(
	cx: &rpc::MutationCx,
	topic: String,
	persons: Vec<uuid::Uuid>,
) -> rpc::Result<u32> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app unsubscribes people in batches: call this from its server, with the \
			 service's token",
		));
	}

	service::unsubscribe_batch(cx.tx(), &topic, persons).await
}

/// Subscribes the signed-in person to a topic, for themselves only: the
/// person is read from the forwarded identity, never from an argument,
/// so nobody can subscribe anyone else.
///
/// Self-subscription is open only to topics the operator declared open —
/// patterns like `news.*` in `LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS` — and
/// nothing is open by default. The reason is whose choice a subscription
/// is: the app's server subscribes whoever its product logic says, while
/// a person may only join the audiences the operator published as
/// joinable. A closed topic is refused with a clear forbidden error that
/// names nothing but the topic's closedness.
///
/// A person holds at most [`MAX_TOPICS_PER_PERSON`] topics; past that the
/// call is refused, unlike the batch call, which skips.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person
/// or the topic isn't open to self-subscription, [`rpc::Error::Invalid`]
/// for a bad topic name or a person at their topic bound.
#[rpc::mutation(service = "notifications")]
pub async fn subscribe(cx: &rpc::MutationCx, topic: String) -> rpc::Result<()> {
	let me = service::me(cx.identity())?;
	check_topic(&topic).map_err(rpc::Error::invalid)?;
	service::check_open_topic(cx, &topic)?;

	service::subscribe_self(cx.tx(), me, &topic).await
}

/// Unsubscribes the signed-in person from a topic, for themselves only.
/// Every topic is unsubscribable, open or not: leaving an audience is
/// always the person's to do. Not holding the topic is not an error.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person,
/// and [`rpc::Error::Invalid`] for a bad topic name.
#[rpc::mutation(service = "notifications")]
pub async fn unsubscribe(cx: &rpc::MutationCx, topic: String) -> rpc::Result<()> {
	let me = service::me(cx.identity())?;
	check_topic(&topic).map_err(rpc::Error::invalid)?;

	service::unsubscribe_self(cx.tx(), me, &topic).await
}

/// The topics the signed-in person holds, oldest first, one bounded page
/// at a time. A live query: the app's batch subscriptions and the
/// person's own joins and leaves reach a screen that has it open.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person.
#[rpc::query(service = "notifications")]
pub async fn my_topics(cx: &rpc::QueryCx, page: rpc::PageArgs) -> rpc::Result<rpc::Page<String>> {
	let me = service::me(cx.identity())?;
	let mut page = page;
	page.limit = page.limit.clamp(1, MAX_TOPICS_PAGE);

	let page_of_rows = service::paginate_bounded(
		cx.db(),
		TopicSubscription::query().filter(|subscription| subscription.person.eq(me)),
		&page,
		u32::try_from(MAX_TOPICS_PER_PERSON).unwrap_or(u32::MAX),
		|subscription| subscription.id.asc(),
		|subscription| subscription.id,
	)
	.await?;

	Ok(rpc::Page {
		items: page_of_rows
			.items
			.into_iter()
			.map(|subscription| subscription.topic)
			.collect(),
		end: page_of_rows.end,
		has_more: page_of_rows.has_more,
	})
}

/// Sets and removes person attributes in one batch: the app's server
/// only, because an attribute is the app's own statement about a person,
/// and a segment built from client-set attributes would be a forgeable
/// audience. A person has no operation that sets or reads attributes,
/// their own or anyone else's.
///
/// At most [`MAX_ATTRIBUTE_PERSONS`] persons per call, duplicates
/// collapse, and the call is idempotent: a value already stored exactly
/// as sent changes nothing and counts nothing. `remove` is the last
/// word: a key in both `set` and `remove` is removed, never set. Each
/// person holds at most [`MAX_ATTRIBUTES_PER_PERSON`] attributes; a new
/// key past that bound is skipped and counted in
/// [`PersonAttributeChanges::skipped_full`], per person and key, so one
/// full person never refuses the batch. One call names at most
/// [`MAX_ATTRIBUTE_CHANGES`] keys across `set` and `remove`, and a larger
/// one is refused before anything is read or written.
///
/// Setting any attribute registers the named persons in the service's
/// [registry](AudienceMember) — the audience [`SendTarget::Everyone`] and
/// attribute-anchored conditions walk.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a bad key or value, an empty or oversized
/// batch, more than [`MAX_ATTRIBUTE_CHANGES`] keys across `set` and
/// `remove`, or a call that neither sets nor removes anything.
#[rpc::mutation(service = "notifications")]
pub async fn set_person_attributes(
	cx: &rpc::MutationCx,
	persons: Vec<uuid::Uuid>,
	#[serde(default)] set: std::collections::BTreeMap<String, String>,
	#[serde(default)] remove: Vec<String>,
) -> rpc::Result<PersonAttributeChanges> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app describes people with attributes: call this from its server, with the \
			 service's token",
		));
	}

	service::set_attributes(cx.tx(), persons, set, remove).await
}

/// Reads one person's attributes as the app set them, keyed and sorted:
/// the app's own read-back of what a segment will see. A person the
/// service doesn't know, or who holds none, answers an empty map.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server.
#[rpc::query(service = "notifications")]
pub async fn person_attributes(
	cx: &rpc::QueryCx,
	person: uuid::Uuid,
) -> rpc::Result<std::collections::BTreeMap<String, String>> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app reads person attributes: call this from its server, with the service's \
			 token",
		));
	}

	service::read_attributes(cx.db(), person).await
}

/// Erases everything the service holds about one person: the app's server
/// only, for a deleted account or a withdrawn consent. Their inbox rows,
/// their push endpoints with the credentials in them, their push
/// deliveries, their category preferences, their topic subscriptions,
/// their attributes, and their place in the [registry](AudienceMember)
/// all go. Nothing is pushed to their devices again, no audience reaches
/// them, and [`inbox`] answers nothing for them.
///
/// It stays in effect. A device of theirs learns at its next launch that
/// its enrollment is gone and drops it, rather than enrolling again
/// ([`push::register_push_endpoint`]'s `renews`); the service hears of
/// them again only through a new act — the person's own gesture to enroll,
/// set a preference, or subscribe, or the app's server naming them in a
/// send, a subscription, or attributes. An app that must keep a person out
/// stops offering those and stops naming them.
///
/// Everything bounded per person is erased before this answers. An inbox
/// longer than [`FANOUT_BATCH`] rows is finished by a background job
/// ([`forget_rows`]), a batch per write, so an erasure never writes a
/// whole history in one transaction; the rest follows within moments.
///
/// What the service doesn't hold per person is not touched: a send's own
/// record keeps its counts, and the framework's job and receipt tables
/// prune on their own schedule.
///
/// Idempotent: forgetting a person the service doesn't know does
/// nothing and succeeds.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server.
#[rpc::mutation(service = "notifications")]
pub async fn forget_person(cx: &rpc::MutationCx, person: uuid::Uuid) -> rpc::Result<()> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app forgets a person: call this from its server, with the service's token",
		));
	}

	service::forget_person(cx, person).await
}

/// Pushes a spec to one person's live endpoints, right now, as a test:
/// the app's server only, the way FCM's "test on device" is. Nothing is
/// written — no inbox row, no delivery row, no send bookkeeping, no
/// count anywhere — and each endpoint answers its own outcome, because
/// a test is a question about a device, not a send.
///
/// The spec takes the same [options](MessageOptions) a send does —
/// priority, image, sound, a badge, a link, and the per-platform blocks,
/// silent data messages included — so what a device receives is what a
/// real send to it would carry. A badge set to `unread` resolves against
/// the person's live count at the moment of the test.
///
/// The blast radius is bounded by the person's own enrollment: at most
/// their [`MAX_ENDPOINTS`](crate::push::MAX_ENDPOINTS) live endpoints,
/// each attempted exactly once. Name an endpoint id to probe one device;
/// an id that isn't the person's own live endpoint is not found. There
/// is no idempotency key to reuse — a test answers, it does not record —
/// so a retried call pushes again.
///
/// It is an [action](rpc::ActionCx): the endpoints are read first, and the
/// provider requests go out with no transaction open, so however slow a
/// provider is, it holds up nothing else the service writes.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::Invalid`] for input the checks refuse or a server with
/// no push transport configured, and [`rpc::Error::NotFound`] for an
/// endpoint id the person doesn't hold live.
#[rpc::action(service = "notifications")]
pub async fn send_test(
	cx: &rpc::ActionCx,
	spec: SendSpec,
	person: uuid::Uuid,
	#[serde(default)] endpoint: Option<uuid::Uuid>,
) -> rpc::Result<Vec<TestSendOutcome>> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app tests sends: call this from its server, with the service's token",
		));
	}

	check_spec(&spec).map_err(rpc::Error::invalid)?;
	check_timing(&spec).map_err(rpc::Error::invalid)?;

	service::send_test(cx, &spec, person, endpoint).await
}
