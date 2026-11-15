//! Durable notifications for leptoskit apps.
//!
//! An app's server sends a notification to explicit users, and the service
//! stores one durable inbox row per recipient, in a database of its own.
//! The recipients read their own inbox from any device their app signs in
//! on: the web, a native app, or the server while it renders. Everything
//! here is the durable store and the inbox retrieval, delivered while the
//! app runs. OS background push — APNs, FCM, Web Push — and device
//! enrollment are later work, not part of this crate.
//!
//! Without features, this crate is the operations' types, which an app
//! compiles to call the service, and the shared validation its server runs
//! before it sends. With the `service` feature, it's the service itself,
//! which `lk` runs when an app declares `[services.notifications]`.
//!
//! Who may call what:
//!
//! - [`send`] and [`cancel`] answer only the app's server, signed with the
//!   service's token ([`rpc::client::call_as_app`]).
//! - [`inbox`], [`unread_count`], [`category_preferences`], [`mark_read`],
//!   [`mark_dismissed`], and [`set_category_preference`] answer the signed-in
//!   person the app forwarded ([`rpc::server::ForwardedIdentity`]), and only
//!   ever touch their own rows. Anonymous calls, and calls that forge a user id
//!   in any header, are refused; the recipient is always read from the app's
//!   forwarded identity, never from an argument.

use leptoskit_db::table;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

#[cfg(feature = "service")]
pub mod service;

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

/// The farthest ahead a notification may be scheduled or set to expire, in
/// seconds: about a year.
pub const MAX_SCHEDULE_SECS: i64 = 366 * 24 * 60 * 60;

/// One durable inbox row: the recipient's copy of a notification a send
/// delivered to them. The read, dismissed, and cancelled state lives here,
/// shared by every device signed in as [`Notification::person`], never
/// per-device.
///
/// Rows the app cancelled before they were due ([`cancel`]) stay in the
/// table with `cancelled_at` set, so a send's history stays explainable;
/// they never become visible.
#[table(name = "notifications", live, index(person, id))]
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
}

/// One send the app's server made. Keyed by its idempotency key, so a
/// retried send is recognised after a restart: the same key with the same
/// intent answers the stored result, and the same key with a different one
/// is a conflict.
#[table(name = "sends")]
pub struct AppSend {
	#[db(key, min_len = 1, max_len = 255)]
	pub idempotency_key: String,
	/// A SHA-256 of the send's canonical intent: category, content,
	/// recipients, schedule, and expiry. Same key, same hash; anything else
	/// is a different send that must use another key.
	#[db(min_len = 64, max_len = 64)]
	pub intent: String,
	/// How many inbox rows the send created.
	pub delivered: i64,
	/// How many recipients it skipped because they opted out of the
	/// category.
	pub skipped_opted_out: i64,
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
}

/// One person's opt-out for one category. A send skips recipients who opted
/// out of its category, and an inbox hides rows whose category its reader
/// has since opted out of, so an opt-out takes hold on what is already
/// stored as well as on what comes after.
#[table(name = "category_preferences", unique(person, category))]
pub struct Preference {
	pub id: leptoskit_db::Id<Self>,
	pub person: uuid::Uuid,
	#[db(min_len = 1, max_len = 64)]
	pub category: String,
	pub opted_out: bool,
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
/// and when it shows and stops showing. The app's server builds one per
/// send, and the service stores it verbatim on every row. The lengths here
/// are what [`check_send`] and the `notifications` table enforce.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
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
}

/// What a send came to. The counts describe the first run: a send repeated
/// with the same key answers the stored counts again, marked
/// [`SendResult::duplicate`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SendResult {
	/// How many recipients received a durable inbox row.
	pub delivered: u32,
	/// How many recipients were skipped because they opted out of the
	/// category.
	pub skipped_opted_out: u32,
	/// How many recipients the send named, after duplicates collapsed.
	pub recipients: u32,
	/// Whether this answer repeats an earlier send with the same key and
	/// the same intent, and nothing new was written.
	#[serde(default)]
	pub duplicate: bool,
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
/// without a round trip and the service rejects it all the same.
///
/// # Errors
///
/// A message naming what's wrong: a length, a control character, `data`
/// that isn't a JSON object, or a schedule that doesn't hold together.
pub fn check_send(
	idempotency_key: &str,
	spec: &SendSpec,
	recipients: &[uuid::Uuid],
) -> Result<(), String> {
	check_key(idempotency_key)?;
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

	if recipients.is_empty() || recipients.len() > MAX_RECIPIENTS {
		return Err(format!("a send names 1 to {MAX_RECIPIENTS} recipients"));
	}

	check_schedule(spec.visible_at, spec.expires_at)?;

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

/// Checks a send's schedule: `visible_at` within [`MAX_SCHEDULE_SECS`] of
/// now, and `expires_at` after visibility begins.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_schedule(
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

/// Sends a notification to explicit users: the app's server only.
///
/// Every named recipient gets one durable inbox row, in the service's own
/// database, visible from now — or from `spec.visible_at`, when it
/// schedules one — until it expires. Recipients who opted out of the
/// category are skipped and counted, not delivered to.
///
/// The send is idempotent by idempotency key: the same key with the same
/// intent answers the first run's result and writes nothing, across
/// restarts, and the same key with a different intent is a conflict.
/// Generate one key per logical send and keep it stable on retry.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::Invalid`] for input [`check_send`] refuses, and
/// [`rpc::Error::Conflict`] when the key was used for a different send.
#[rpc::mutation(service = "notifications")]
pub async fn send(
	cx: &rpc::MutationCx,
	idempotency_key: String,
	spec: SendSpec,
	recipients: Vec<uuid::Uuid>,
) -> rpc::Result<SendResult> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app sends notifications: call this from its server, with the service's token",
		));
	}

	check_send(&idempotency_key, &spec, &recipients).map_err(rpc::Error::invalid)?;

	let recipients: std::collections::BTreeSet<uuid::Uuid> = recipients.into_iter().collect();
	let intent = service::intent_hash(&spec, &recipients);

	if let Some(existing) = service::find_send(cx.tx(), &idempotency_key).await? {
		return service::answer_send(&existing, &intent);
	}

	match service::write_send(cx.tx(), &idempotency_key, &intent).await {
		Ok(()) => {}
		// Another send with this key committed between the read and the
		// write: compare intents as if it had been there first.
		Err(leptoskit_db::DbError::Conflict { .. }) => {
			let existing = service::find_send(cx.tx(), &idempotency_key)
				.await?
				.ok_or_else(|| rpc::Error::conflict("the send vanished while writing it"))?;
			return service::answer_send(&existing, &intent);
		}
		Err(error) => return Err(error.into()),
	}

	let fanout = service::fanout(cx.tx(), &idempotency_key, &spec, &recipients).await?;
	service::record_outcome(cx.tx(), &idempotency_key, &fanout).await?;

	Ok(SendResult {
		delivered: fanout.delivered,
		skipped_opted_out: fanout.skipped_opted_out,
		recipients: recipients.len().try_into().unwrap_or(u32::MAX),
		duplicate: false,
	})
}

/// Cancels a scheduled send before it's due: every row it created stops
/// being visible, forever. Once a row of the send is visible the send can
/// no longer be cancelled, and a key the service doesn't know is not found.
/// Cancelling an already-cancelled send answers `0`, whenever it happens.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::NotFound`] for an unknown key, and
/// [`rpc::Error::Conflict`] once the send is visible.
#[rpc::mutation(service = "notifications")]
pub async fn cancel(cx: &rpc::MutationCx, idempotency_key: String) -> rpc::Result<u32> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app cancels sends: call this from its server, with the service's token",
		));
	}

	check_key(&idempotency_key).map_err(rpc::Error::invalid)?;

	let now = leptoskit_db::now();
	let rows = Notification::query()
		.filter(|notification| notification.send_key.eq(idempotency_key.clone()))
		.all(cx.tx())
		.await?;

	if rows.is_empty() {
		return Err(rpc::Error::not_found(
			"no send with this idempotency key exists",
		));
	}

	if rows
		.iter()
		.any(|notification| notification.visible_at <= now && notification.cancelled_at.is_none())
	{
		return Err(rpc::Error::conflict(
			"this send is already visible, so it can no longer be cancelled",
		));
	}

	let mut cancelled = 0u32;

	for notification in rows {
		if notification.cancelled_at.is_some() {
			continue;
		}

		Notification::update()
			.filter(|row| row.id.eq(notification.id))
			.set(|row| row.cancelled_at, Some(now))
			.run(cx.tx())
			.await?;
		cancelled += 1;
	}

	Ok(cancelled)
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

	let page_of_rows = rpc::paginate(
		cx.db(),
		query,
		&page,
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

	NewPreference {
		person: me,
		category,
		opted_out,
	}
	.insert(cx.tx())
	.await
	.map(|_| ())
	.or_else(|error| {
		match error {
			// Another device set this category while we read: its answer is
			// the answer.
			leptoskit_db::DbError::Conflict { .. } => Ok(()),
			error => Err(error.into()),
		}
	})
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
