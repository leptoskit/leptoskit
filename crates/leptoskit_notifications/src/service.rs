//! The service's own code, compiled only with the `service` feature: who is
//! calling, which rows are visible, the send's bookkeeping, the routes
//! `lk` serves, and the push delivery engine behind the optional
//! transports.
//!
//! Identity follows the framework's trust boundary exactly: the routes take
//! a [`ServiceCallers`], so a request signed with the service's token is
//! the app ([`rpc::server::APP_CALLER`]), and a request signed with a token
//! the app minted for this service is the person it names
//! ([`rpc::server::ForwardedIdentity`]). Nothing reads a user id from a
//! header or an argument on the recipient's side, so there is nothing to
//! forge.

pub(crate) mod campaign;
pub(crate) mod catalogue;
pub mod config;
pub(crate) mod history;
pub(crate) mod local;
pub(crate) mod preview;
pub mod push;
pub mod recurrence;
pub mod retention;
pub mod transport;

use leptoskit_axum::axum::Router;
use leptoskit_db::Db;
use leptoskit_db::Statement;
use leptoskit_rpc as rpc;
use sha2::Digest as _;
use sha2::Sha256;
use uuid::Uuid;

use crate::AppSend;
use crate::AudienceMember;
use crate::InboxItem;
use crate::NewAppSend;
use crate::NewAudienceMember;
use crate::NewNotification;
use crate::NewPersonAttribute;
use crate::NewTopicSubscription;
use crate::NewWake;
use crate::Notification;
use crate::PersonAttribute;
use crate::Preference;
use crate::SERVICE;
use crate::TopicSubscription;
use crate::Wake;
use crate::condition::Condition;
use crate::push::PushDelivery;
use crate::push::PushEndpoint;

/// The longest job key the job table holds, in bytes.
const MAX_JOB_KEY_LEN: usize = 255;

/// The key of one of a send's jobs: `{kind}:{send_key}:{what}`, the shape
/// these keys have always had, while it fits the job table's key column.
/// An idempotency key may use all of its 255 bytes, so a key that doesn't
/// fit names the send by the SHA-256 of its key instead:
/// `{kind}#{hash}:{what}`. The `#` keeps the two shapes apart, so no
/// hashed key is ever some other send's verbatim one.
pub(crate) fn job_key(kind: &str, send_key: &str, what: impl std::fmt::Display) -> String {
	let verbatim = format!("{kind}:{send_key}:{what}");

	if verbatim.len() <= MAX_JOB_KEY_LEN {
		return verbatim;
	}

	format!(
		"{kind}#{}:{what}",
		hex::encode(Sha256::digest(send_key.as_bytes()))
	)
}

/// The write half of [`send`](crate::send), as the mutation the action
/// runs: its own transaction, retried when another gets in its way. It is
/// called in process only, never routed, so the action's checks always
/// come first.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SendWrite {
	pub(crate) idempotency_key: String,
	pub(crate) spec: crate::SendSpec,
	pub(crate) target: crate::SendTarget,
}

impl rpc::Operation for SendWrite {
	type Context = rpc::MutationCx;
	type Output = crate::SendResult;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "send";
	const SERVICE: Option<&'static str> = Some(SERVICE);

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<Self::Output>> + Send {
		crate::send_write(cx, self.idempotency_key, self.spec, self.target)
	}
}

/// The dry run of [`send`](crate::send), as the query the action runs:
/// the audience walk reads page by page, with no transaction open.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct DryRun {
	pub(crate) spec: crate::SendSpec,
	pub(crate) target: crate::SendTarget,
}

impl rpc::Operation for DryRun {
	type Context = rpc::QueryCx;
	type Output = Audience;

	const KIND: rpc::Kind = rpc::Kind::Query;
	const NAME: &'static str = "send";
	const SERVICE: Option<&'static str> = Some(SERVICE);

	async fn run(self, cx: &rpc::QueryCx) -> rpc::Result<Self::Output> {
		audience_now(cx.db(), &self.spec, &self.target).await
	}
}

/// The person's live endpoints a [`send_test`](crate::send_test) pushes
/// to, credentials included — read in process only, never routed, so
/// they never leave the service — and their unread count when the spec's
/// badge asks for it.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct TestTargets {
	person: Uuid,
	endpoint: Option<Uuid>,
	unread: bool,
	/// Whether the spec carries variants, so the person's locale is
	/// wanted.
	locale: bool,
}

/// What [`TestTargets`] reads.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Targets {
	/// Each live endpoint's id, platform, credential, and the APNs
	/// environment its device reported.
	endpoints: Vec<(Uuid, String, String, Option<crate::push::ApnsEnvironment>)>,
	unread: Option<u64>,
	/// The locale the person reads variants in, when one is known.
	locale: Option<String>,
}

impl rpc::Operation for TestTargets {
	type Context = rpc::QueryCx;
	type Output = Targets;

	const KIND: rpc::Kind = rpc::Kind::Query;
	const NAME: &'static str = "send_test";
	const SERVICE: Option<&'static str> = Some(SERVICE);

	async fn run(self, cx: &rpc::QueryCx) -> rpc::Result<Self::Output> {
		let mut query = PushEndpoint::query()
			.filter(|row| row.person.eq(self.person) & row.invalidated_at.is_null());

		if let Some(endpoint) = self.endpoint {
			query = query.filter(|row| row.id.eq(endpoint));
		}

		// The blast radius is the person's own enrollment, bounded again
		// here so nothing about a future bound change widens a test.
		let endpoints = query
			.order_by(|row| row.id.asc())
			.limit(u64::try_from(crate::push::MAX_ENDPOINTS).unwrap_or(u64::MAX))
			.all(cx.db())
			.await?;
		let unread = if self.unread {
			Some(unread_for(cx.db(), self.person).await?)
		} else {
			None
		};

		let locale = if self.locale {
			local::locale_for(cx.db(), self.person).await?
		} else {
			None
		};

		Ok(Targets {
			endpoints: endpoints
				.into_iter()
				.map(|endpoint| {
					let environment = endpoint.apns_environment();
					(
						endpoint.id,
						endpoint.platform,
						endpoint.credential,
						environment,
					)
				})
				.collect(),
			unread,
			locale,
		})
	}
}

/// The person the app forwarded, or the error a stranger gets: nobody's
/// inbox answers without one, and no argument can name one.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] when the caller isn't a forwarded person.
pub(crate) fn me(identity: &rpc::server::Identity) -> rpc::Result<Uuid> {
	identity
		.context
		.get::<rpc::server::ForwardedIdentity>()
		.map(|me| me.person)
		.ok_or_else(|| rpc::Error::forbidden("sign in to read your notifications"))
}

/// Which notification rows `person` may see at `now`: visible, not
/// cancelled, not expired, not in a category they opted out of, and not
/// a data row — a silent message's durable half never surfaces.
pub(crate) fn visible(
	notification: &<Notification as leptoskit_db::Table>::Columns,
	person: Uuid,
	now: leptoskit_db::Timestamp,
	opted_out: &[String],
) -> leptoskit_db::Expr {
	let mut expr = notification.person.eq(person)
		& notification
			.kind
			.eq(crate::options::MessageKind::Notification.as_str())
		& notification.visible_at.lte(now)
		& notification.cancelled_at.is_null()
		& (notification.expires_at.is_null() | notification.expires_at.gt(now));

	for category in opted_out {
		expr = expr & notification.category.ne(category.clone());
	}

	expr
}

/// The categories `person` opted out of, at most
/// [`MAX_PREFERENCES`](crate::MAX_PREFERENCES).
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn opted_out_categories(
	db: &impl leptoskit_db::Read,
	person: Uuid,
) -> rpc::Result<Vec<String>> {
	let rows = Preference::query()
		.filter(|preference| preference.person.eq(person) & preference.opted_out.eq(true))
		.limit(u64::try_from(crate::MAX_PREFERENCES).unwrap_or(u64::MAX))
		.all(db)
		.await?;

	Ok(rows
		.into_iter()
		.map(|preference| preference.category)
		.collect())
}

/// How many visible, unread, undismissed notification rows `person`
/// holds right now: what a badge shows, and what a send's `unread` badge
/// option resolves to at push time. Data rows never count.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn unread_for(db: &impl leptoskit_db::Read, person: Uuid) -> rpc::Result<u64> {
	let now = leptoskit_db::now();
	let opted_out = opted_out_categories(db, person).await?;

	Notification::query()
		.filter(|notification| visible(notification, person, now, &opted_out))
		.filter(|notification| notification.read_at.is_null() & notification.dismissed_at.is_null())
		.count(db)
		.await
		.map_err(rpc::Error::from)
}

/// The canonical form of what a send intends, hashed: the same send twice
/// hashes the same, and a changed category, content, target, schedule, or
/// expiry changes the hash, so reusing its key fails.
///
/// A recipients send serializes exactly the shape it always did — the
/// spec and the recipient list — so a send recorded before targets
/// existed keeps answering its own retries.
pub(crate) fn intent_hash(spec: &crate::SendSpec, target: &crate::SendTarget) -> String {
	#[derive(serde::Serialize)]
	struct Intent<'a> {
		spec: &'a crate::SendSpec,
		#[serde(skip_serializing_if = "Option::is_none")]
		recipients: Option<std::collections::BTreeSet<Uuid>>,
		#[serde(skip_serializing_if = "Option::is_none")]
		topic: Option<&'a str>,
		#[serde(skip_serializing_if = "Option::is_none")]
		condition: Option<&'a str>,
		#[serde(skip_serializing_if = "Option::is_none")]
		everyone: Option<bool>,
	}

	let intent = match target {
		crate::SendTarget::Recipients(recipients) => {
			Intent {
				spec,
				recipients: Some(recipients.iter().copied().collect()),
				topic: None,
				condition: None,
				everyone: None,
			}
		}
		crate::SendTarget::Topic(topic) => {
			Intent {
				spec,
				recipients: None,
				topic: Some(topic),
				condition: None,
				everyone: None,
			}
		}
		crate::SendTarget::Condition(condition) => {
			Intent {
				spec,
				recipients: None,
				topic: None,
				condition: Some(condition),
				everyone: None,
			}
		}
		crate::SendTarget::Everyone => {
			Intent {
				spec,
				recipients: None,
				topic: None,
				condition: None,
				everyone: Some(true),
			}
		}
	};

	let encoded = serde_json::to_vec(&intent).expect("the intent is plain JSON");

	hex::encode(Sha256::digest(&encoded))
}

/// The send `key` names, if the service has it.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn find_send(
	db: &impl leptoskit_db::Read,
	key: &str,
) -> rpc::Result<Option<AppSend>> {
	AppSend::query()
		.filter(|send| send.idempotency_key.eq(key))
		.first(db)
		.await
		.map_err(rpc::Error::from)
}

/// Writes the send row a recipients send holds: complete from the start,
/// with its audience count and its spec — the content, for the send
/// history — and no target, because nothing is left to fan out and the
/// people it named are never stored.
///
/// # Errors
///
/// [`leptoskit_db::DbError::Conflict`] when another send holds the key.
pub(crate) async fn write_send(
	tx: &leptoskit_db::Tx,
	key: &str,
	intent: &str,
	targeted: u32,
	spec: &crate::SendSpec,
) -> leptoskit_db::Result<()> {
	NewAppSend {
		idempotency_key: key.to_owned(),
		intent: intent.to_owned(),
		delivered: 0,
		skipped_opted_out: 0,
		targeted: i64::from(targeted),
		state: crate::SendState::Complete.as_str().to_owned(),
		// No target: the recipients were named, never stored, and a
		// missing target is how a retry knows this was a recipients send.
		target: None,
		spec: Some(serde_json::to_string(spec).expect("the spec is plain JSON")),
		cursor: None,
		label: spec.label.clone(),
		history_id: Some(Uuid::now_v7()),
		category: Some(spec.category.clone()),
	}
	.insert(tx)
	.await
	.map(|_| ())
}

/// Writes the send row a topic or condition send holds: still fanning
/// out, with its target and spec stored for the job that walks the
/// audience.
///
/// # Errors
///
/// [`leptoskit_db::DbError::Conflict`] when another send holds the key.
pub(crate) async fn write_fanout_send(
	tx: &leptoskit_db::Tx,
	key: &str,
	intent: &str,
	target: &crate::SendTarget,
	spec: &crate::SendSpec,
) -> leptoskit_db::Result<()> {
	NewAppSend {
		idempotency_key: key.to_owned(),
		intent: intent.to_owned(),
		delivered: 0,
		skipped_opted_out: 0,
		targeted: 0,
		state: crate::SendState::FanningOut.as_str().to_owned(),
		target: Some(target.stored()),
		spec: Some(serde_json::to_string(spec).expect("the spec is plain JSON")),
		cursor: None,
		label: spec.label.clone(),
		history_id: Some(Uuid::now_v7()),
		category: Some(spec.category.clone()),
	}
	.insert(tx)
	.await
	.map(|_| ())
}

/// Records what a send came to, so the same key answers the same counts
/// forever.
///
/// # Errors
///
/// Whatever the write fails with.
pub(crate) async fn record_outcome(
	tx: &leptoskit_db::Tx,
	key: &str,
	fanout: &Fanout,
) -> rpc::Result<()> {
	AppSend::update()
		.filter(|send| send.idempotency_key.eq(key))
		.set(|send| send.delivered, i64::from(fanout.delivered))
		.set(
			|send| send.skipped_opted_out,
			i64::from(fanout.skipped_opted_out),
		)
		.run(tx)
		.await?;

	Ok(())
}

/// Answers a repeat of a send: the stored counts when the intent is the
/// same, a conflict when the key carried a different send.
///
/// # Errors
///
/// [`rpc::Error::Conflict`] for a different intent.
pub(crate) fn answer_send(existing: &AppSend, intent: &str) -> rpc::Result<crate::SendResult> {
	if existing.intent != intent {
		return Err(rpc::Error::conflict(
			"this idempotency key was already used for a different send; use a new key for each \
			 send",
		));
	}

	let delivered = u32::try_from(existing.delivered).unwrap_or(u32::MAX);
	let skipped_opted_out = u32::try_from(existing.skipped_opted_out).unwrap_or(u32::MAX);
	let targeted = u32::try_from(existing.targeted).unwrap_or(u32::MAX);
	let state =
		crate::SendState::from_str_value(&existing.state).unwrap_or(crate::SendState::Complete);
	let recipients = existing
		.target
		.as_ref()
		.map_or(delivered.saturating_add(skipped_opted_out), |_| 0);

	Ok(crate::SendResult {
		delivered,
		skipped_opted_out,
		recipients,
		duplicate: true,
		state,
		targeted,
	})
}

/// The send row as [`crate::send_status`] answers it.
pub(crate) fn progress_of(send: &AppSend) -> crate::SendProgress {
	crate::SendProgress {
		state: crate::SendState::from_str_value(&send.state).unwrap_or(crate::SendState::Complete),
		targeted: u32::try_from(send.targeted).unwrap_or(u32::MAX),
		delivered: u32::try_from(send.delivered).unwrap_or(u32::MAX),
		skipped_opted_out: u32::try_from(send.skipped_opted_out).unwrap_or(u32::MAX),
	}
}

/// What a send's fanout came to, with the rows it wrote so the push
/// deliveries can join them.
#[derive(Clone, Debug)]
pub(crate) struct Fanout {
	pub delivered: u32,
	pub skipped_opted_out: u32,
	/// One entry per inbox row the fanout wrote: the recipient, their
	/// notification's id, and — for a row of a local-time send whose time
	/// is still ahead — the instant it waits for. `None` is a row visible
	/// as it is written, and every row of a send with one schedule for
	/// everyone, which the send's own jobs carry.
	pub delivered_to: Vec<(Uuid, Uuid, Option<leptoskit_db::Timestamp>)>,
}

impl Fanout {
	/// The distinct instants the fanout's waiting rows wait for, in
	/// order: as many as the time zones its people span, never as many as
	/// its people.
	pub(crate) fn instants(&self) -> std::collections::BTreeSet<leptoskit_db::Timestamp> {
		self.delivered_to
			.iter()
			.filter_map(|(_, _, waits_until)| *waits_until)
			.collect()
	}
}

/// Writes one durable inbox row per recipient who hasn't opted out of the
/// category, and counts the rest. Each row is written as `resolved` says
/// for its person: in the text of the variant they read, visible at the
/// send's instant or at their own local time's arrival.
///
/// # Errors
///
/// Whatever a write fails with.
pub(crate) async fn fanout(
	tx: &leptoskit_db::Tx,
	send_key: &str,
	spec: &crate::SendSpec,
	recipients: &std::collections::BTreeSet<Uuid>,
	resolved: &local::Resolved,
) -> rpc::Result<Fanout> {
	let data = spec.data.as_deref().unwrap_or("{}");
	// The text each variant in play writes, built once however many
	// people read it.
	let mut texts: std::collections::HashMap<Option<&str>, RowText> =
		std::collections::HashMap::new();
	let opted_out: std::collections::BTreeSet<Uuid> = Preference::query()
		.filter(|preference| {
			preference.category.eq(&spec.category)
				& preference.opted_out.eq(true)
				& preference.person.in_list(recipients.iter().copied())
		})
		.all(tx)
		.await?
		.into_iter()
		.map(|preference| preference.person)
		.collect();

	let mut fanout = Fanout {
		delivered: 0,
		skipped_opted_out: 0,
		delivered_to: Vec::new(),
	};

	for person in recipients {
		if opted_out.contains(person) {
			fanout.skipped_opted_out += 1;
			continue;
		}

		let text = texts
			.entry(resolved.variant(*person))
			.or_insert_with_key(|variant| RowText::of(spec, *variant));
		let id = Uuid::now_v7();
		NewNotification {
			id,
			send_key: send_key.to_owned(),
			person: *person,
			category: spec.category.clone(),
			title: text.title.clone(),
			body: text.body.clone(),
			data: data.to_owned(),
			visible_at: resolved.visible_at(*person),
			expires_at: spec.expires_at,
			cancelled_at: None,
			read_at: None,
			dismissed_at: None,
			opened_at: None,
			kind: kind_of(spec),
			options: text.options.clone(),
		}
		.insert(tx)
		.await?;
		fanout.delivered += 1;
		fanout
			.delivered_to
			.push((*person, id, resolved.waits_until(*person)));
	}

	Ok(fanout)
}

/// What a row stores of a send's text, under one variant.
struct RowText {
	title: String,
	body: String,
	/// The options as the variant shaped them, as stored JSON; `None`
	/// when the send carried none.
	options: Option<String>,
}

impl RowText {
	/// The text `spec` writes under the variant `key` names, or its own
	/// for `None`.
	fn of(spec: &crate::SendSpec, key: Option<&str>) -> Self {
		let text = spec.localized(key);

		Self {
			title: text.title.to_owned(),
			body: text.body.to_owned(),
			options: options_of(&text.options),
		}
	}
}

/// The row kind a spec writes: `data` for a silent send, `notification`
/// otherwise.
fn kind_of(spec: &crate::SendSpec) -> String {
	(if spec.is_data() {
		crate::options::MessageKind::Data
	} else {
		crate::options::MessageKind::Notification
	})
	.as_str()
	.to_owned()
}

/// The options a row stores — the send's kind, its presentation,
/// everything the push engine reads per delivery — or `None` when the
/// send carried none.
fn options_of(options: &crate::MessageOptions) -> Option<String> {
	(!options.is_empty())
		.then(|| serde_json::to_string(options).expect("the options are plain JSON"))
}

/// [`rpc::paginate`], with a pinned page — one whose arguments name the
/// row it ends at (`until`) — bounded at `most` rows.
///
/// The framework reads a pinned page whole, up to
/// [`rpc::MAX_PINNED_ROWS`], whatever `limit` says: the page's arguments
/// come from the client, so `until` alone would let a caller read ten
/// thousand of their rows in one answer, and re-read them on every change
/// while subscribed. A page loaded at a clamped `limit` only passes `most`
/// when that many rows arrive inside it; past it, the page keeps its first
/// `most` rows in page order, as the framework's own bound does.
///
/// # Errors
///
/// As [`rpc::paginate`].
pub(crate) async fn paginate_bounded<T, V>(
	db: &impl leptoskit_db::Read,
	query: leptoskit_db::Query<T>,
	page: &rpc::PageArgs,
	most: u32,
	order: impl FnOnce(&T::Columns) -> leptoskit_db::Order,
	key: impl Fn(&T) -> V,
) -> rpc::Result<rpc::Page<T>>
where
	T: leptoskit_db::Table,
	V: leptoskit_db::Column + serde::Serialize + serde::de::DeserializeOwned,
{
	let Some(until) = page.until.as_deref() else {
		return rpc::paginate(db, query, page, order, key).await;
	};

	let order = order(T::columns());
	let column = leptoskit_db::Col::<T, V>::new(order.column());
	let descending = order.is_descending();
	let cursor = |text: &str| {
		serde_json::from_str::<V>(text)
			.map_err(|_| rpc::Error::invalid(format!("`{text}` isn't a cursor of this query")))
	};
	let mut query = query.order_by(|_| order);

	if let Some(after) = page.after.as_deref() {
		let after = cursor(after)?;
		query = query.filter(|_| {
			if descending {
				column.lt(after)
			} else {
				column.gt(after)
			}
		});
	}

	let last = cursor(until)?;
	let items = query
		.filter(|_| {
			if descending {
				column.gte(last)
			} else {
				column.lte(last)
			}
		})
		.limit(u64::from(most))
		.all(db)
		.await?;

	Ok(rpc::Page {
		items,
		end: Some(until.to_owned()),
		has_more: true,
	})
}

/// The notification `id`, when it belongs to `person`, and not found
/// otherwise: every read and mutation of another person's row looks
/// exactly like a missing one.
///
/// # Errors
///
/// [`rpc::Error::NotFound`] unless it's theirs.
pub(crate) async fn own_row(
	db: &impl leptoskit_db::Read,
	person: Uuid,
	id: Uuid,
) -> rpc::Result<Notification> {
	match Notification::query()
		.filter(|notification| notification.id.eq(id) & notification.person.eq(person))
		.one(db)
		.await
	{
		Ok(row) => Ok(row),
		Err(leptoskit_db::DbError::NotFound { .. }) => Err(not_in_inbox()),
		Err(error) => Err(error.into()),
	}
}

/// What every person-side call answers for an id that is not one of the
/// caller's inbox rows, whatever the reason.
pub(crate) fn not_in_inbox() -> rpc::Error {
	rpc::Error::not_found("no notification with this id is in your inbox")
}

/// The read behind a send's [`InboxCounts`](crate::InboxCounts): one pass
/// over the send's rows, found through the `(send_key, person)` unique
/// index, counting each state's timestamp where it is set. One statement,
/// so the five counts describe the same moment.
fn send_counts_read(dialect: leptoskit_db::Dialect, send_key: &str) -> Statement {
	Statement::new(
		format!(
			"SELECT count(*), count(\"read_at\"), count(\"opened_at\"), count(\"dismissed_at\"), \
			 count(\"cancelled_at\") FROM \"notifications\" WHERE \"send_key\" = {}",
			placeholder(dialect, 1)
		),
		vec![leptoskit_db::Value::Text(send_key.to_owned())],
	)
}

/// What became of `send_key`'s rows, as the service stores them now.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn inbox_counts(
	db: &impl leptoskit_db::Read,
	send_key: &str,
) -> rpc::Result<crate::InboxCounts> {
	let rows = send_counts_read(db.dialect(), send_key)
		.reads::<Notification>()
		.fetch(db)
		.await?;
	let row = rows.first().map(Vec::as_slice).unwrap_or_default();
	let count = |index: usize| {
		match row.get(index) {
			Some(leptoskit_db::Value::Int(count)) => u32::try_from(*count).unwrap_or(u32::MAX),
			_ => 0,
		}
	};

	Ok(crate::InboxCounts {
		rows: count(0),
		read: count(1),
		opened: count(2),
		dismissed: count(3),
		cancelled: count(4),
	})
}

/// Reads the person's wake row, so the queries that follow it carry
/// `wakes` in their read set: the wake job at a schedule's turning point
/// writes that row, and their open subscriptions re-run with a fresh
/// `now`. The row's value is never used — only its change matters.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn follow_wake(db: &impl leptoskit_db::Read, person: Uuid) -> rpc::Result<()> {
	Wake::query()
		.filter(|wake| wake.person.eq(person))
		.first(db)
		.await?;

	Ok(())
}

/// Enqueues the send's wake jobs, in its own transaction: one when it
/// becomes visible, and one at `expires_at` when it expires. Visibility
/// is `visible_at` for a scheduled send and now for an immediate one —
/// an open subscription's read was taken with an earlier `now`, whose
/// `visible_at <= now` can't match a row that did not exist then, so the
/// send itself needs the wake as much as its schedule does. Each job
/// exists exactly when the send lands, and its idempotency key keeps a
/// raced or retried enqueue from doubling it.
///
/// A local-time send has no one moment of visibility. `fanout` is the
/// rows a recipients send just wrote: the people whose time has passed
/// are woken in this transaction, and every distinct instant still ahead
/// gets a [`wake_local`](crate::wake_local) of its own. A
/// local-time send whose fanout runs as a job passes `None`: each of its
/// batches wakes its own people, and only the expiry is the send's.
///
/// # Errors
///
/// Whatever an enqueue fails with.
pub(crate) async fn schedule_wakes(
	cx: &rpc::MutationCx,
	send_key: &str,
	spec: &crate::SendSpec,
	fanout: Option<&Fanout>,
) -> rpc::Result<()> {
	let now = leptoskit_db::now();
	let mut wakes = Vec::with_capacity(2);

	if spec.local_time.is_none() {
		let becomes_visible = spec
			.visible_at
			.map_or(now, |visible_at| visible_at.max(now));
		wakes.push(("visible", becomes_visible));
	} else if let Some(fanout) = fanout {
		// The people whose time has already passed see their rows as
		// this transaction commits, the way a fanout batch's do.
		wake_people(
			cx.tx(),
			fanout
				.delivered_to
				.iter()
				.filter(|(_, _, waits_until)| waits_until.is_none())
				.map(|(person, ..)| *person),
		)
		.await?;

		if let Some(through) = fanout.delivered_to.iter().map(|(person, ..)| *person).max() {
			for at in fanout.instants() {
				cx.enqueue(crate::WakeLocal {
					send_key: send_key.to_owned(),
					after: None,
					through,
					at,
				})
				.at(at)
				.key(job_key("wake", send_key, format!("at:{}", at.timestamp())))
				.await?;
			}
		}
	}

	if let Some(expires_at) = spec.expires_at {
		wakes.push(("expires", expires_at));
	}

	for (turning, at) in wakes {
		cx.enqueue(crate::WakeSend {
			send_key: send_key.to_owned(),
		})
		.at(at)
		.key(job_key("wake", send_key, turning))
		.await?;
	}

	Ok(())
}

/// Writes one wake row per person the send delivered to, in bounded
/// transactions: the clock event that re-runs their open inbox and badge
/// subscriptions. A recipient send reaches at most
/// [`crate::MAX_RECIPIENTS`] people, but a topic or condition send reaches
/// as many as subscribed, so the people are walked in pages of
/// [`crate::FANOUT_BATCH`]; a wake for a send that no longer changes
/// anyone's answer re-runs those queries and sends nothing.
///
/// # Errors
///
/// Whatever the reads or writes fail with.
pub(crate) async fn wake(db: &Db, send_key: &str) -> rpc::Result<()> {
	let mut cursor: Option<Uuid> = None;

	loop {
		let page = Notification::query()
			.filter(|notification| {
				let mut condition = notification.send_key.eq(send_key.to_owned());
				if let Some(cursor) = cursor {
					condition = condition & notification.person.gt(cursor);
				}
				condition
			})
			.order_by(|notification| notification.person.asc())
			.limit(u64::try_from(crate::FANOUT_BATCH).unwrap_or(u64::MAX))
			.all(db)
			.await?;

		if page.is_empty() {
			return Ok(());
		}

		cursor = page.last().map(|notification| notification.person);
		let tx = db.begin().await?;
		wake_people(&tx, page.iter().map(|notification| notification.person)).await?;
		tx.commit().await?;
	}
}

/// Wakes the people of one fanout batch, the body of
/// [`wake_batch`](crate::wake_batch) and
/// [`wake_local`](crate::wake_local): those with a row of the send whose
/// person is in `(after, through]` — and, with `at`, whose row becomes
/// visible in the second `at` begins, the one instant of a local-time
/// send this wake is for. A recipients send's range is the whole send,
/// at most [`crate::MAX_RECIPIENTS`] rows, so it is read a page at a time.
///
/// # Errors
///
/// Whatever a read or a write fails with.
pub(crate) async fn wake_range(
	db: &Db,
	send_key: &str,
	after: Option<Uuid>,
	through: Uuid,
	at: Option<leptoskit_db::Timestamp>,
) -> rpc::Result<()> {
	let mut cursor = after;

	loop {
		let page = Notification::query()
			.filter(|notification| {
				let mut condition = notification.send_key.eq(send_key.to_owned())
					& notification.person.lte(through);
				if let Some(cursor) = cursor {
					condition = condition & notification.person.gt(cursor);
				}
				condition
			})
			.order_by(|notification| notification.person.asc())
			.limit(u64::try_from(crate::FANOUT_BATCH).unwrap_or(u64::MAX))
			.all(db)
			.await?;

		let Some(last) = page.last() else {
			return Ok(());
		};

		cursor = Some(last.person);
		let full = page.len() >= crate::FANOUT_BATCH;
		// A local time's arrival is a whole second, written on the row as
		// it was computed: the rows of this instant are the ones inside
		// it.
		let due: Vec<Uuid> = page
			.iter()
			.filter(|notification| {
				at.is_none_or(|at| {
					notification.visible_at >= at
						&& notification.visible_at < at + chrono::Duration::seconds(1)
				})
			})
			.map(|notification| notification.person)
			.collect();

		if !due.is_empty() {
			let tx = db.begin().await?;
			wake_people(&tx, due.into_iter()).await?;
			tx.commit().await?;
		}

		if !full {
			return Ok(());
		}
	}
}

/// Writes `people`'s wake rows in `tx`: the clock event that re-runs their
/// open inbox and badge subscriptions once `tx` commits.
///
/// # Errors
///
/// Whatever a write fails with.
async fn wake_people(tx: &leptoskit_db::Tx, people: impl Iterator<Item = Uuid>) -> rpc::Result<()> {
	let now = leptoskit_db::now();

	for person in people {
		NewWake {
			person,
			woken_at: now,
		}
		.upsert(tx, |wake| wake.person)
		.await?;
	}

	Ok(())
}

/// The audience a target names, and how many of it a send to `spec`'s
/// category would reach right now: what `validate_only` answers. A
/// recipients target counts its deduplicated list against the opt-outs;
/// every other target walks its audience the same way the fanout will —
/// the topic index or the registry, condition evaluated, nothing
/// written.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] for a stored shape that no longer parses, and
/// whatever a read fails with.
pub(crate) async fn audience_now(
	db: &impl leptoskit_db::Read,
	spec: &crate::SendSpec,
	target: &crate::SendTarget,
) -> rpc::Result<Audience> {
	let mut audience = Audience::default();

	match target {
		crate::SendTarget::Recipients(named) => {
			let recipients: std::collections::BTreeSet<Uuid> = named.iter().copied().collect();
			audience.targeted = recipients.len().try_into().unwrap_or(u32::MAX);
			audience.skipped_opted_out = opted_out_among(db, &spec.category, &recipients)
				.await?
				.try_into()
				.unwrap_or(u32::MAX);
			audience.delivered = audience.targeted.saturating_sub(audience.skipped_opted_out);
		}
		target => {
			let walk = walk_of(target)?;
			let mut cursor: Option<Uuid> = None;

			loop {
				let page = candidates(db, &walk, cursor, FANOUT_PAGE + 1).await?;

				if page.is_empty() {
					break;
				}

				let more = page.len() > FANOUT_PAGE;
				let page = &page[..page.len().saturating_sub(usize::from(more))];
				cursor = page.last().copied();

				let matching = evaluate(db, &walk, page).await?.matching;
				audience.targeted = audience
					.targeted
					.saturating_add(matching.len().try_into().unwrap_or(u32::MAX));

				let matched: std::collections::BTreeSet<Uuid> = matching.iter().copied().collect();
				audience.skipped_opted_out = audience.skipped_opted_out.saturating_add(
					opted_out_among(db, &spec.category, &matched)
						.await?
						.try_into()
						.unwrap_or(u32::MAX),
				);

				if !more {
					break;
				}
			}

			audience.delivered = audience.targeted.saturating_sub(audience.skipped_opted_out);
		}
	}

	Ok(audience)
}

/// What an audience walk came to.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Audience {
	pub(crate) targeted: u32,
	pub(crate) delivered: u32,
	pub(crate) skipped_opted_out: u32,
}

/// How many of `recipients` opted out of `category`.
///
/// # Errors
///
/// Whatever the read fails with.
async fn opted_out_among(
	db: &impl leptoskit_db::Read,
	category: &str,
	recipients: &std::collections::BTreeSet<Uuid>,
) -> rpc::Result<usize> {
	if recipients.is_empty() {
		return Ok(0);
	}

	let rows = Preference::query()
		.filter(|preference| {
			preference.category.eq(category.to_owned())
				& preference.opted_out.eq(true)
				& preference.person.in_list(recipients.iter().copied())
		})
		.all(db)
		.await?;

	Ok(rows.len())
}

/// The condition a target names: a topic is the membership of one name;
/// a condition is parsed. Both were validated when the send was made, so
/// a parse failure here is a bug's shape.
///
/// # Errors
///
/// [`rpc::Error::server`] when a stored condition no longer parses.
fn condition_of(target: &crate::SendTarget) -> rpc::Result<Condition> {
	match target {
		crate::SendTarget::Topic(topic) => Ok(Condition::Topic(topic.clone())),
		crate::SendTarget::Condition(text) => {
			Condition::parse(text).map_err(|error| {
				rpc::Error::server(format!("a stored condition no longer parses: {error}"))
			})
		}
		crate::SendTarget::Recipients(_) | crate::SendTarget::Everyone => {
			Err(rpc::Error::server("this target has no condition"))
		}
	}
}

/// Who a fanout walks, and how it decides who among them matches. A
/// topic, and a condition that cannot hold without one of its topics,
/// walk the topic index — a bounded, cheap set. Everything else walks
/// the whole person registry: an unanchored condition, which can match a
/// person holding none of its topics, and everyone.
pub(crate) enum Walk {
	/// The distinct subscribers of the condition's topics, evaluated
	/// against it.
	Topics(Condition),
	/// The person registry, evaluated against the condition when it has
	/// one, matched whole when it doesn't — everyone.
	Registry(Option<Condition>),
}

impl Walk {
	/// The condition to evaluate, if any.
	pub(crate) fn condition(&self) -> Option<&Condition> {
		match self {
			Walk::Topics(condition) => Some(condition),
			Walk::Registry(condition) => condition.as_ref(),
		}
	}
}

/// The audience `target` walks, from the same parse the app's server
/// validated its send with.
///
/// # Errors
///
/// [`rpc::Error::server`] when a stored condition no longer parses.
pub(crate) fn walk_of(target: &crate::SendTarget) -> rpc::Result<Walk> {
	match target {
		crate::SendTarget::Recipients(_) => {
			Err(rpc::Error::server("a recipients send has no fanout"))
		}
		crate::SendTarget::Everyone => Ok(Walk::Registry(None)),
		crate::SendTarget::Topic(_) | crate::SendTarget::Condition(_) => {
			let condition = condition_of(target)?;

			if condition.topic_anchored() {
				Ok(Walk::Topics(condition))
			} else {
				Ok(Walk::Registry(Some(condition)))
			}
		}
	}
}

/// How many people one fanout page reads past its batch, to see whether
/// more follow without asking for them twice.
pub(crate) const FANOUT_PAGE: usize = crate::FANOUT_BATCH;

/// The people one fanout page evaluates next, ordered by person id,
/// after `cursor`, at most `limit`. Raw SQL because the page is one
/// column's values, which the typed query builder reads as whole rows;
/// each statement still names its table, so a connection that tracks
/// reads records it.
///
/// A walk over several topics reads each topic on its own and merges the
/// answers here. `(topic, person)` hands one topic's subscribers back in
/// person order straight from the index, but not several topics' at once:
/// asked for `topic IN (…) ORDER BY person`, the database reads every
/// remaining subscriber of every topic and sorts them, on each page of
/// the walk. Each topic's read is its first `limit` people after the
/// cursor, so anyone among the first `limit` of them all is in their own
/// topic's read: the merged page is exactly the page the single statement
/// answered, each person once, and the cursor means what it always did.
///
/// # Errors
///
/// Whatever a read fails with.
pub(crate) async fn candidates(
	db: &impl leptoskit_db::Read,
	walk: &Walk,
	cursor: Option<Uuid>,
	limit: usize,
) -> rpc::Result<Vec<Uuid>> {
	let dialect = db.dialect();

	match walk {
		Walk::Topics(condition) => {
			let mut people = std::collections::BTreeSet::new();

			for topic in condition.topics() {
				let rows = subscribers_read(dialect, topic, cursor, limit)
					.reads::<TopicSubscription>()
					.fetch(db)
					.await?;

				people.extend(rows.into_iter().filter_map(|row| uuid_of(row.first()?)));
			}

			Ok(people.into_iter().take(limit).collect())
		}
		Walk::Registry(_) => {
			let rows = registry_read(dialect, cursor, limit)
				.reads::<AudienceMember>()
				.fetch(db)
				.await?;

			Ok(rows
				.into_iter()
				.filter_map(|row| uuid_of(row.first()?))
				.collect())
		}
	}
}

/// The read of one topic's subscribers after `cursor`, in person order, at
/// most `limit`: one range of the `(topic, person)` index, in its order.
fn subscribers_read(
	dialect: leptoskit_db::Dialect,
	topic: &str,
	cursor: Option<Uuid>,
	limit: usize,
) -> Statement {
	let mut sql = String::from("SELECT \"person\" FROM \"topic_subscriptions\" WHERE \"topic\" = ");
	let mut args: Vec<leptoskit_db::Value> = Vec::new();

	sql.push_str(&placeholder(dialect, args.len() + 1));
	args.push(leptoskit_db::Value::Text(topic.to_owned()));

	if let Some(cursor) = cursor {
		sql.push_str(" AND \"person\" > ");
		sql.push_str(&placeholder(dialect, args.len() + 1));
		args.push(leptoskit_db::Value::Uuid(cursor));
	}

	sql.push_str(" ORDER BY \"person\" ASC LIMIT ");
	sql.push_str(&placeholder(dialect, args.len() + 1));
	args.push(leptoskit_db::Value::Int(
		i64::try_from(limit).unwrap_or(i64::MAX),
	));

	Statement::new(sql, args)
}

/// The read of the registry's people after `cursor`, in person order, at
/// most `limit`.
fn registry_read(dialect: leptoskit_db::Dialect, cursor: Option<Uuid>, limit: usize) -> Statement {
	let mut sql = String::from("SELECT \"person\" FROM \"audience_members\"");
	let mut args: Vec<leptoskit_db::Value> = Vec::new();

	if let Some(cursor) = cursor {
		sql.push_str(" WHERE \"person\" > ");
		sql.push_str(&placeholder(dialect, args.len() + 1));
		args.push(leptoskit_db::Value::Uuid(cursor));
	}

	sql.push_str(" ORDER BY \"person\" ASC LIMIT ");
	sql.push_str(&placeholder(dialect, args.len() + 1));
	args.push(leptoskit_db::Value::Int(
		i64::try_from(limit).unwrap_or(i64::MAX),
	));

	Statement::new(sql, args)
}

/// What a walk's condition reads about one page of people: the topics and
/// attributes it names, per person. The fanout keeps it past the audience
/// decision, so the deliveries it writes can judge each endpoint against
/// the same facts.
#[derive(Debug, Default)]
pub(crate) struct Facts {
	topics: std::collections::HashMap<Uuid, std::collections::BTreeSet<String>>,
	attributes: std::collections::HashMap<Uuid, std::collections::BTreeMap<String, String>>,
}

impl Facts {
	/// Whether `condition` holds for `person` on `device` — or with no
	/// device at all, where every device predicate answers false.
	pub(crate) fn holds(
		&self,
		condition: &Condition,
		person: Uuid,
		device: Option<crate::condition::Device<'_>>,
	) -> bool {
		condition.matches_on(
			|topic| {
				self.topics
					.get(&person)
					.is_some_and(|topics| topics.contains(topic))
			},
			|key| {
				self.attributes
					.get(&person)
					.and_then(|held| held.get(key).map(String::as_str))
			},
			device,
		)
	}
}

/// Which of a page's people a walk matches, and what it read to decide.
#[derive(Debug, Default)]
pub(crate) struct Verdict {
	/// The matching people, in the page's order.
	pub(crate) matching: Vec<Uuid>,
	/// What the condition read about the page.
	pub(crate) facts: Facts,
}

/// Which of `people` the walk matches, evaluated against the topics and
/// attributes they hold among the ones its condition names. A registry
/// walk with no condition — everyone — matches every person in the page.
///
/// A condition that asks about devices is judged per device too: once with
/// no device, and once for each of the person's live endpoints, read for
/// the whole page in one query. The person matches when any of those
/// holds.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn evaluate(
	db: &impl leptoskit_db::Read,
	walk: &Walk,
	people: &[Uuid],
) -> rpc::Result<Verdict> {
	let Some(condition) = walk.condition() else {
		return Ok(Verdict {
			matching: people.to_vec(),
			facts: Facts::default(),
		});
	};

	if people.is_empty() {
		return Ok(Verdict::default());
	}

	let mut facts = Facts::default();
	let topics = condition.topics();

	if !topics.is_empty() {
		let rows = TopicSubscription::query()
			.filter(|subscription| {
				subscription.person.in_list(people.iter().copied())
					& subscription
						.topic
						.in_list(topics.iter().map(|topic| (*topic).to_owned()))
			})
			.all(db)
			.await?;

		for row in rows {
			facts
				.topics
				.entry(row.person)
				.or_default()
				.insert(row.topic);
		}
	}

	let keys = condition.attribute_keys();

	if !keys.is_empty() {
		let rows = PersonAttribute::query()
			.filter(|attribute| {
				attribute.person.in_list(people.iter().copied())
					& attribute
						.key
						.in_list(keys.iter().map(|key| (*key).to_owned()))
			})
			.all(db)
			.await?;

		for row in rows {
			facts
				.attributes
				.entry(row.person)
				.or_default()
				.insert(row.key, row.value);
		}
	}

	// One read for the page's devices, and only when the condition asks
	// about one: a condition with no `device.` is judged once per person,
	// exactly as before devices existed.
	let devices = if condition.mentions_device() {
		push::live_endpoints(db, people.iter().copied()).await?
	} else {
		std::collections::HashMap::new()
	};

	let matching = people
		.iter()
		.copied()
		.filter(|person| {
			facts.holds(condition, *person, None)
				|| devices.get(person).is_some_and(|endpoints| {
					endpoints.iter().any(|endpoint| {
						facts.holds(condition, *person, Some(push::device_of(endpoint)))
					})
				})
		})
		.collect();

	Ok(Verdict { matching, facts })
}

/// One step of a topic or condition send's fanout, the body of the
/// [`fanout_send`](crate::fanout_send) job: one batch of
/// [`crate::FANOUT_BATCH`] candidates, evaluated and delivered in one
/// transaction, the send's cursor and counts advanced in the same commit,
/// and the successor enqueued when more audience remains.
///
/// The transaction first claims the batch: it moves the cursor from where
/// this step read it, fenced on the send still fanning out, and writes
/// nothing at all when that matches no row. A [`cancel`](crate::cancel)
/// that commits between steps, a replayed step whose batch already
/// committed, and two runs of one step at once all end up there, and a
/// step that loses its batch that way makes sure the step after the
/// winner's exists before it ends: the winner enqueues it itself, but a
/// winner that crashed before it did would otherwise leave the send
/// fanning out forever. Successors are keyed by the cursor they start
/// from, never by a step number, so no step's key can be one a finished
/// job already holds.
///
/// # Errors
///
/// Whatever a read, a write, or an enqueue fails with. The job framework
/// retries the step; what already committed is not done twice.
pub(crate) async fn fanout_step(cx: &rpc::JobCx, send_key: &str, step: u32) -> rpc::Result<()> {
	let db = cx.db();
	let Some(send) = find_send(db, send_key).await? else {
		// The send never landed: nothing to walk.
		return Ok(());
	};

	if crate::SendState::from_str_value(&send.state) != Some(crate::SendState::FanningOut) {
		return Ok(());
	}

	let target = send
		.target
		.as_deref()
		.map(crate::SendTarget::from_stored)
		.transpose()?
		.ok_or_else(|| rpc::Error::server("a fanning send holds no target"))?;
	let spec: crate::SendSpec = serde_json::from_str(
		send.spec
			.as_deref()
			.ok_or_else(|| rpc::Error::server("a fanning send holds no spec"))?,
	)
	.map_err(|error| rpc::Error::server(format!("a stored spec no longer parses: {error}")))?;
	let walk = walk_of(&target)?;

	let page = candidates(db, &walk, send.cursor, FANOUT_PAGE + 1).await?;
	let more = page.len() > FANOUT_PAGE;
	let page = &page[..page.len().min(FANOUT_PAGE)];

	let Some(&last) = page.last() else {
		// The walk has passed the last person in its audience.
		finish_fanout(db, send_key, &spec).await?;

		return Ok(());
	};

	let verdict = evaluate(db, &walk, page).await?;
	let now = leptoskit_db::now();
	let scheduled = spec.visible_at.is_some_and(|visible_at| visible_at > now);
	// The batch's languages and local times, read for the whole batch
	// before its transaction opens; a send with neither reads nothing.
	let matching: std::collections::BTreeSet<Uuid> = verdict.matching.iter().copied().collect();
	let resolved = local::resolve(db, &spec, &matching, now).await?;

	// One transaction: the claim, the batch's rows, their push deliveries,
	// the counts, and — when the rows are visible as they commit — their
	// people's wakes.
	let tx = db.begin().await?;

	if !claim_batch(&tx, send_key, send.cursor, last).await? {
		tx.rollback().await?;

		return continue_fanout(db, send_key, step).await;
	}

	// The push goes only to the endpoints a device condition holds for,
	// judged where the deliveries are written, against the endpoints as
	// this transaction reads them — even for a scheduled send, whose
	// deliveries wait in the queue for its time. A condition that never
	// mentions `device.` reaches every live endpoint.
	let filter = walk
		.condition()
		.filter(|condition| condition.mentions_device())
		.map(|condition| {
			push::DeviceFilter {
				condition,
				facts: &verdict.facts,
			}
		});
	let batch = deliver_batch(
		&tx,
		send_key,
		&spec,
		&send,
		&verdict.matching,
		&resolved,
		filter.as_ref(),
	)
	.await?;

	if !scheduled {
		// The rows visible as they commit: all of them for a send with
		// one schedule, and for a local-time send the people whose time
		// has already passed.
		wake_people(
			&tx,
			batch
				.fanout
				.delivered_to
				.iter()
				.filter(|(_, _, waits_until)| waits_until.is_none())
				.map(|(person, ..)| *person),
		)
		.await?;
	}

	tx.commit().await?;

	if batch.deliveries > 0 {
		push::kick(db, send_key, &spec, "fanout").await?;
	}

	// A local-time batch's waiting rows turn visible at as many instants
	// as its people span time zones: one wake for each, for this batch's
	// people alone.
	for at in batch.fanout.instants() {
		rpc::enqueue(
			db,
			crate::WakeLocal {
				send_key: send_key.to_owned(),
				after: send.cursor,
				through: last,
				at,
			},
		)
		.at(at)
		.key(job_key(
			"wake",
			send_key,
			format!("batch:{last}:at:{}", at.timestamp()),
		))
		.await?;
	}

	// A batch that commits before its send is visible wakes its own people
	// when the send becomes visible, however the send's own wake at that
	// moment raced this commit.
	if scheduled && batch.fanout.delivered > 0 {
		rpc::enqueue(
			db,
			crate::WakeBatch {
				send_key: send_key.to_owned(),
				after: send.cursor,
				through: last,
			},
		)
		.at(push::becomes_visible(&spec))
		.key(job_key("wake", send_key, format!("batch:{last}")))
		.await?;
	}

	if !more {
		finish_fanout(db, send_key, &spec).await?;

		return Ok(());
	}

	enqueue_step(db, send_key, step.saturating_add(1), Some(last)).await
}

/// Moves a fanning send's cursor from `from`, where the step read it, to
/// `last`, in `tx`: whether this step owns the batch. Anything else — a
/// cancel, another run that took the batch — leaves it unclaimed.
///
/// # Errors
///
/// Whatever the write fails with.
async fn claim_batch(
	tx: &leptoskit_db::Tx,
	send_key: &str,
	from: Option<Uuid>,
	last: Uuid,
) -> rpc::Result<bool> {
	let claim = AppSend::update().filter(|send| {
		send.idempotency_key.eq(send_key.to_owned())
			& send.state.eq(crate::SendState::FanningOut.as_str())
	});
	let claim = match from {
		Some(from) => claim.filter(|send| send.cursor.eq(from)),
		None => claim.filter(|send| send.cursor.is_null()),
	};
	let claimed = claim.set(|send| send.cursor, Some(last)).run(tx).await?;

	Ok(claimed == 1)
}

/// What one claimed batch wrote.
struct Batch {
	fanout: Fanout,
	/// How many push deliveries the batch enqueued.
	deliveries: usize,
}

/// Writes a claimed batch in `tx`: the rows of the people in `matching`
/// who hold none of the send's yet, their push deliveries, and the send's
/// counts. A person who already holds a row — written by a run whose
/// claim this one replays — is skipped and counted nowhere.
///
/// # Errors
///
/// Whatever a read or a write fails with.
async fn deliver_batch(
	tx: &leptoskit_db::Tx,
	send_key: &str,
	spec: &crate::SendSpec,
	send: &AppSend,
	matching: &[Uuid],
	resolved: &local::Resolved,
	filter: Option<&push::DeviceFilter<'_>>,
) -> rpc::Result<Batch> {
	let already: std::collections::BTreeSet<Uuid> = Notification::query()
		.filter(|notification| {
			notification.send_key.eq(send_key.to_owned())
				& notification.person.in_list(matching.iter().copied())
		})
		.all(tx)
		.await?
		.into_iter()
		.map(|notification| notification.person)
		.collect();
	let fresh: std::collections::BTreeSet<Uuid> = matching
		.iter()
		.copied()
		.filter(|person| !already.contains(person))
		.collect();

	let fanout = if fresh.is_empty() {
		Fanout {
			delivered: 0,
			skipped_opted_out: 0,
			delivered_to: Vec::new(),
		}
	} else {
		fanout(tx, send_key, spec, &fresh, resolved).await?
	};

	let deliveries = push::deliveries_in_tx(tx, send_key, &fanout, filter).await?;

	// The claim held the cursor where this step read it, so the counts it
	// read are the counts to add to.
	AppSend::update()
		.filter(|row| row.idempotency_key.eq(send_key.to_owned()))
		.set(
			|row| row.targeted,
			send.targeted + i64::try_from(fresh.len()).unwrap_or(i64::MAX),
		)
		.set(
			|row| row.delivered,
			send.delivered + i64::from(fanout.delivered),
		)
		.set(
			|row| row.skipped_opted_out,
			send.skipped_opted_out + i64::from(fanout.skipped_opted_out),
		)
		.run(tx)
		.await?;

	Ok(Batch { fanout, deliveries })
}

/// What a step that lost its batch still owes the fanout: the step for
/// the cursor as it stands now, under the key that cursor's own step is
/// enqueued with, so it exists exactly once whoever enqueues it first.
/// A send that is no longer fanning out owes nothing.
///
/// # Errors
///
/// Whatever the read or the enqueue fails with.
async fn continue_fanout(db: &Db, send_key: &str, step: u32) -> rpc::Result<()> {
	let Some(send) = find_send(db, send_key).await? else {
		return Ok(());
	};

	if crate::SendState::from_str_value(&send.state) != Some(crate::SendState::FanningOut) {
		return Ok(());
	}

	enqueue_step(db, send_key, step.saturating_add(1), send.cursor).await
}

/// Enqueues the fanout step that walks on from `after`, keyed by it: the
/// first step's key for the start of the audience, and the batch's last
/// person after that.
///
/// # Errors
///
/// Whatever the enqueue fails with.
async fn enqueue_step(db: &Db, send_key: &str, step: u32, after: Option<Uuid>) -> rpc::Result<()> {
	let key = match after {
		Some(after) => job_key("fanout", send_key, format!("after:{after}")),
		None => job_key("fanout", send_key, 0),
	};

	rpc::enqueue(
		db,
		crate::FanoutSend {
			send_key: send_key.to_owned(),
			step,
		},
	)
	.at(leptoskit_db::now())
	.key(key)
	.await?;

	Ok(())
}

/// Marks a fanning send complete — only while it still is, so a cancel
/// that lands first stays a cancel. One more push round goes first: a
/// round that finds the send still fanning out keeps looking, and this one
/// covers any batch whose own kick a crash lost between its commit and its
/// enqueue.
///
/// # Errors
///
/// Whatever the write or the enqueue fails with.
async fn finish_fanout(db: &Db, send_key: &str, spec: &crate::SendSpec) -> rpc::Result<()> {
	push::kick_final(db, send_key, spec).await?;

	AppSend::update()
		.filter(|send| {
			send.idempotency_key.eq(send_key.to_owned())
				& send.state.eq(crate::SendState::FanningOut.as_str())
		})
		.set(|send| send.state, crate::SendState::Complete.as_str())
		.run(db)
		.await?;

	Ok(())
}

/// Marks a fanning send cancelled, in the cancel mutation's transaction:
/// the fanout's next step checks the state before it writes, so nothing
/// more is delivered from the moment this commits.
///
/// # Errors
///
/// Whatever the write fails with.
pub(crate) async fn cancel_fanout(tx: &leptoskit_db::Tx, send_key: &str) -> rpc::Result<()> {
	AppSend::update()
		.filter(|send| {
			send.idempotency_key.eq(send_key.to_owned())
				& send.state.eq(crate::SendState::FanningOut.as_str())
		})
		.set(|send| send.state, crate::SendState::Cancelled.as_str())
		.run(tx)
		.await?;

	Ok(())
}

/// The body of [`cancel`](crate::cancel), set-based: existence, the
/// visible-row rule, and the rows themselves are each one statement, and
/// no row is read into memory. A fanout send with more than one batch of
/// rows to cancel hands them to [`cancel_rows`](crate::cancel_rows), so
/// this transaction never writes a whole audience.
///
/// # Errors
///
/// [`rpc::Error::NotFound`] for an unknown key, [`rpc::Error::Conflict`]
/// once a complete send is visible, and whatever a read or write fails
/// with.
pub(crate) async fn cancel(cx: &rpc::MutationCx, send_key: &str) -> rpc::Result<u32> {
	let tx = cx.tx();
	let now = leptoskit_db::now();
	let send = find_send(tx, send_key).await?;
	let of_send = |notification: &<Notification as leptoskit_db::Table>::Columns| {
		notification.send_key.eq(send_key.to_owned())
	};

	// Existence is the send's own row, not its fanout: a send every
	// recipient opted out of delivered nothing, and cancelling it is still
	// cancelling it.
	if send.is_none() && !Notification::query().filter(of_send).exists(tx).await? {
		return Err(rpc::Error::not_found(
			"no send with this idempotency key exists",
		));
	}

	// A fanout still walking its audience is stoppable whatever its rows
	// say, and one already stopped stays stoppable while its rows are
	// cancelled; a complete one keeps today's rule: what someone has
	// already seen can't be unsent.
	let state = send
		.as_ref()
		.and_then(|send| crate::SendState::from_str_value(&send.state));
	let fanning = state == Some(crate::SendState::FanningOut);

	// A data row is never seen by anyone — it is the silent half of a send,
	// not a visible one — so only a visible notification row can make the
	// send uncancellable.
	let seen = |notification: &<Notification as leptoskit_db::Table>::Columns| {
		of_send(notification)
			& notification
				.kind
				.eq(crate::options::MessageKind::Notification.as_str())
			& notification.visible_at.lte(now)
			& notification.cancelled_at.is_null()
	};
	// What a cancel can still stop once someone has seen the send: the
	// rows not yet visible, and the silent ones.
	let ahead = |notification: &<Notification as leptoskit_db::Table>::Columns| {
		of_send(notification)
			& notification.cancelled_at.is_null()
			& (notification.visible_at.gt(now)
				| notification
					.kind
					.ne(crate::options::MessageKind::Notification.as_str()))
	};
	// A send that isn't fanning out and was never marked cancelled, with a
	// row somebody can see: only what is still ahead can stop. Every row
	// of a send with one schedule turns visible together, so nothing is
	// ahead and the send is past cancelling; a local-time send's later
	// time zones still are.
	let partial = !fanning
		&& state != Some(crate::SendState::Cancelled)
		&& Notification::query().filter(seen).exists(tx).await?;

	if partial && !Notification::query().filter(ahead).exists(tx).await? {
		// Nothing is ahead. Rows already cancelled mean an earlier cancel
		// stopped what it could, and this one repeats it; none mean
		// everything the send wrote has been seen.
		if Notification::query()
			.filter(|notification| of_send(notification) & notification.cancelled_at.is_not_null())
			.exists(tx)
			.await?
		{
			return Ok(0);
		}

		return Err(rpc::Error::conflict(
			"this send is already visible, so it can no longer be cancelled",
		));
	}

	if fanning {
		// The state the fanout's next step checks, and the push rounds
		// too: once this commits, no later batch of this send writes
		// anything, and nothing more of it is pushed.
		cancel_fanout(tx, send_key).await?;
	}

	let stoppable = |notification: &<Notification as leptoskit_db::Table>::Columns| {
		if partial {
			ahead(notification)
		} else {
			of_send(notification) & notification.cancelled_at.is_null()
		}
	};
	let pending = Notification::query().filter(stoppable).count(tx).await?;

	if send.as_ref().is_some_and(|send| send.target.is_some())
		&& pending > u64::try_from(crate::FANOUT_BATCH).unwrap_or(u64::MAX)
	{
		// Unkeyed: a cancel asked again while the job works enqueues a
		// second, which finds whatever the first left, one at a time.
		if partial {
			cx.enqueue(crate::CancelHiddenRows {
				send_key: send_key.to_owned(),
				hidden_after: now,
			})
			.at(now)
			.await?;
		} else {
			cx.enqueue(crate::CancelRows {
				send_key: send_key.to_owned(),
			})
			.at(now)
			.await?;
		}

		return Ok(u32::try_from(pending).unwrap_or(u32::MAX));
	}

	let cancelled = Notification::update()
		.filter(stoppable)
		.set(|notification| notification.cancelled_at, Some(now))
		.run(tx)
		.await?;

	Ok(u32::try_from(cancelled).unwrap_or(u32::MAX))
}

/// The body of [`cancel_rows`](crate::cancel_rows) and
/// [`cancel_hidden_rows`](crate::cancel_hidden_rows): the send's
/// uncancelled rows, one batch at a time, each batch its own short write,
/// until none is left. With `hidden_after` — a local-time send cancelled
/// once some of it was visible — only the rows that were still ahead at
/// that moment, and the silent ones: what people had seen by then stays.
///
/// # Errors
///
/// Whatever a read or a write fails with; the job runs again and picks up
/// what is left.
pub(crate) async fn cancel_rows(
	db: &Db,
	send_key: &str,
	hidden_after: Option<leptoskit_db::Timestamp>,
) -> rpc::Result<()> {
	loop {
		let batch: Vec<Uuid> = Notification::query()
			.filter(|notification| {
				let mut condition = notification.send_key.eq(send_key.to_owned())
					& notification.cancelled_at.is_null();
				if let Some(hidden_after) = hidden_after {
					condition = condition
						& (notification.visible_at.gt(hidden_after)
							| notification
								.kind
								.ne(crate::options::MessageKind::Notification.as_str()));
				}
				condition
			})
			.order_by(|notification| notification.id.asc())
			.limit(u64::try_from(crate::FANOUT_BATCH).unwrap_or(u64::MAX))
			.all(db)
			.await?
			.into_iter()
			.map(|notification| notification.id)
			.collect();

		if batch.is_empty() {
			return Ok(());
		}

		Notification::update()
			.filter(|notification| {
				notification.id.in_list(batch.iter().copied()) & notification.cancelled_at.is_null()
			})
			.set(
				|notification| notification.cancelled_at,
				Some(leptoskit_db::now()),
			)
			.run(db)
			.await?;
	}
}

/// Subscribes a batch of people to `topic`, the body of
/// [`subscribe_to_topic`](crate::subscribe_to_topic): idempotent, and
/// skipping people at [`crate::MAX_TOPICS_PER_PERSON`] rather than
/// refusing the batch for one full person.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] for a bad topic name or batch bound, and
/// whatever a read or write fails with.
pub(crate) async fn subscribe_batch(
	tx: &leptoskit_db::Tx,
	topic: &str,
	persons: Vec<Uuid>,
) -> rpc::Result<crate::TopicChanges> {
	crate::check_topic(topic).map_err(rpc::Error::invalid)?;
	check_persons(&persons)?;

	let persons: std::collections::BTreeSet<Uuid> = persons.into_iter().collect();

	let held = TopicSubscription::query()
		.filter(|subscription| {
			subscription.topic.eq(topic.to_owned())
				& subscription.person.in_list(persons.iter().copied())
		})
		.all(tx)
		.await?
		.into_iter()
		.map(|subscription| subscription.person)
		.collect::<std::collections::BTreeSet<_>>();

	// One grouped read of how many topics each named person already
	// holds, rather than a query per person.
	let cap = u64::try_from(crate::MAX_TOPICS_PER_PERSON).unwrap_or(u64::MAX);
	let counts = topics_held_by(tx, &persons).await?;
	let mut changes = crate::TopicChanges::default();

	// Everyone the call subscribed, or found already subscribed: both
	// hold the topic, and holding it is what registers a person.
	let mut holders: std::collections::BTreeSet<Uuid> = std::collections::BTreeSet::new();

	for person in persons {
		if held.contains(&person) {
			holders.insert(person);
			continue;
		}

		if counts.get(&person).is_some_and(|held| *held >= cap) {
			changes.skipped_full += 1;
			continue;
		}

		let inserted: leptoskit_db::Result<()> = NewTopicSubscription {
			person,
			topic: topic.to_owned(),
		}
		.insert(tx)
		.await
		.map(|_| ());

		match inserted {
			Ok(()) => {
				changes.subscribed += 1;
				holders.insert(person);
			}
			// Two batches, or a batch and a self-subscription, raced on
			// the same person and topic: the other one holds it, which is
			// what this call wanted.
			Err(leptoskit_db::DbError::Conflict { .. }) => {}
			Err(error) => return Err(error.into()),
		}
	}

	register_persons(tx, &holders).await?;

	Ok(changes)
}

/// Unsubscribes a batch of people from `topic`, the body of
/// [`unsubscribe_from_topic`](crate::unsubscribe_from_topic): people who
/// didn't hold it count nothing.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] for a bad topic name or batch bound, and
/// whatever a read or write fails with.
pub(crate) async fn unsubscribe_batch(
	tx: &leptoskit_db::Tx,
	topic: &str,
	persons: Vec<Uuid>,
) -> rpc::Result<u32> {
	crate::check_topic(topic).map_err(rpc::Error::invalid)?;
	check_persons(&persons)?;

	let persons: std::collections::BTreeSet<Uuid> = persons.into_iter().collect();

	let removed = TopicSubscription::query()
		.filter(|subscription| {
			subscription.topic.eq(topic.to_owned())
				& subscription.person.in_list(persons.iter().copied())
		})
		.delete(tx)
		.await?;

	Ok(u32::try_from(removed).unwrap_or(u32::MAX))
}

/// A batch call's person bound.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] for an empty batch or one past
/// [`crate::MAX_SUBSCRIBE_PERSONS`].
fn check_persons(persons: &[Uuid]) -> rpc::Result<()> {
	if persons.is_empty() || persons.len() > crate::MAX_SUBSCRIBE_PERSONS {
		return Err(rpc::Error::invalid(format!(
			"a topic batch names 1 to {} people",
			crate::MAX_SUBSCRIBE_PERSONS
		)));
	}

	Ok(())
}

/// How many topics `person` holds.
///
/// # Errors
///
/// Whatever the read fails with.
async fn topics_held(db: &impl leptoskit_db::Read, person: Uuid) -> rpc::Result<u64> {
	TopicSubscription::query()
		.filter(|subscription| subscription.person.eq(person))
		.count(db)
		.await
		.map_err(rpc::Error::from)
}

/// How many topics each of `persons` holds, in one grouped read.
///
/// # Errors
///
/// Whatever the read fails with.
async fn topics_held_by(
	db: &impl leptoskit_db::Read,
	persons: &std::collections::BTreeSet<Uuid>,
) -> rpc::Result<std::collections::HashMap<Uuid, u64>> {
	if persons.is_empty() {
		return Ok(std::collections::HashMap::new());
	}

	let dialect = db.dialect();
	let mut sql = String::from(
		"SELECT \"person\", count(*) FROM \"topic_subscriptions\" WHERE \"person\" IN (",
	);
	let mut args: Vec<leptoskit_db::Value> = Vec::new();

	for person in persons {
		if !args.is_empty() {
			sql.push_str(", ");
		}

		sql.push_str(&placeholder(dialect, args.len() + 1));
		args.push(leptoskit_db::Value::Uuid(*person));
	}

	sql.push_str(") GROUP BY \"person\"");

	let rows = Statement::new(sql, args)
		.reads::<TopicSubscription>()
		.fetch(db)
		.await?;

	Ok(rows
		.into_iter()
		.filter_map(|row| {
			let person = uuid_of(row.first()?)?;
			let count = match row.get(1)? {
				leptoskit_db::Value::Int(count) => u64::try_from(*count).ok()?,
				_ => return None,
			};
			Some((person, count))
		})
		.collect())
}

/// The person's own subscription, the body of [`subscribe`](crate::subscribe):
/// refused at their topic bound, idempotent beneath it.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] at the bound, and whatever a read or write
/// fails with.
pub(crate) async fn subscribe_self(
	tx: &leptoskit_db::Tx,
	me: Uuid,
	topic: &str,
) -> rpc::Result<()> {
	if topics_held(tx, me).await? >= u64::try_from(crate::MAX_TOPICS_PER_PERSON).unwrap_or(u64::MAX)
	{
		return Err(rpc::Error::invalid(format!(
			"a person holds at most {} topics; unsubscribe from one first",
			crate::MAX_TOPICS_PER_PERSON
		)));
	}

	let inserted: leptoskit_db::Result<()> = NewTopicSubscription {
		person: me,
		topic: topic.to_owned(),
	}
	.insert(tx)
	.await
	.map(|_| ());

	match inserted {
		// A retried subscription, or one raced against a batch, finds the
		// row the unique key refused to duplicate: already subscribed,
		// which is what this call wanted. Either way the person holds
		// the topic now, and holding one is what registers them.
		Ok(()) | Err(leptoskit_db::DbError::Conflict { .. }) => {
			register_persons(tx, &std::collections::BTreeSet::from([me])).await?;
			Ok(())
		}
		Err(error) => Err(error.into()),
	}
}

/// The person's own unsubscription, the body of
/// [`unsubscribe`](crate::unsubscribe).
///
/// # Errors
///
/// Whatever the read or write fails with.
pub(crate) async fn unsubscribe_self(
	tx: &leptoskit_db::Tx,
	me: Uuid,
	topic: &str,
) -> rpc::Result<()> {
	TopicSubscription::query()
		.filter(|subscription| subscription.person.eq(me) & subscription.topic.eq(topic.to_owned()))
		.delete(tx)
		.await?;

	Ok(())
}

/// Registers persons in the service's registry — the audience everyone
/// sends and attribute-anchored conditions walk — without answering
/// anything about them. Idempotent; answers how many were new.
///
/// # Errors
///
/// Whatever a read or write fails with.
pub(crate) async fn register_persons(
	tx: &leptoskit_db::Tx,
	persons: &std::collections::BTreeSet<Uuid>,
) -> rpc::Result<u32> {
	if persons.is_empty() {
		return Ok(0);
	}

	let known: std::collections::BTreeSet<Uuid> = AudienceMember::query()
		.filter(|person| person.person.in_list(persons.iter().copied()))
		.all(tx)
		.await?
		.into_iter()
		.map(|person| person.person)
		.collect();
	let mut registered = 0;

	for person in persons {
		if known.contains(person) {
			continue;
		}

		let inserted: leptoskit_db::Result<()> = NewAudienceMember { person: *person }
			.insert(tx)
			.await
			.map(|_| ());

		match inserted {
			Ok(()) => registered += 1,
			// Another call registered them between the read and the
			// write: the registry holds them, which is what this wanted.
			Err(leptoskit_db::DbError::Conflict { .. }) => {}
			Err(error) => return Err(error.into()),
		}
	}

	Ok(registered)
}

/// Sets and removes attributes for a batch of persons, the body of
/// [`set_person_attributes`](crate::set_person_attributes). `remove` is
/// the last word: a key in both is removed, never set. Idempotent per
/// value, bounded per person, skipping at the bound rather than
/// refusing the batch.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] for a bad key or value, an empty or
/// oversized batch, or a call that changes nothing, and whatever a read
/// or write fails with.
pub(crate) async fn set_attributes(
	tx: &leptoskit_db::Tx,
	persons: Vec<Uuid>,
	set: std::collections::BTreeMap<String, String>,
	remove: Vec<String>,
) -> rpc::Result<crate::PersonAttributeChanges> {
	if persons.is_empty() || persons.len() > crate::MAX_ATTRIBUTE_PERSONS {
		return Err(rpc::Error::invalid(format!(
			"an attribute batch names 1 to {} people",
			crate::MAX_ATTRIBUTE_PERSONS
		)));
	}

	if set.is_empty() && remove.is_empty() {
		return Err(rpc::Error::invalid(
			"an attribute call sets or removes at least one key",
		));
	}

	// Before anything reads: every key is a parameter of the queries
	// below, and a database takes only so many.
	if set.len().saturating_add(remove.len()) > crate::MAX_ATTRIBUTE_CHANGES {
		return Err(rpc::Error::invalid(format!(
			"an attribute call names at most {} keys across `set` and `remove`; split a larger \
			 change into several calls",
			crate::MAX_ATTRIBUTE_CHANGES
		)));
	}

	for (key, value) in &set {
		crate::check_attribute_key(key).map_err(rpc::Error::invalid)?;
		crate::check_attribute_value(value).map_err(rpc::Error::invalid)?;
	}

	for key in &remove {
		crate::check_attribute_key(key).map_err(rpc::Error::invalid)?;
	}

	let persons: std::collections::BTreeSet<Uuid> = persons.into_iter().collect();
	let mut changes = crate::PersonAttributeChanges {
		registered: register_persons(tx, &persons).await?,
		set: 0,
		removed: 0,
		skipped_full: 0,
	};

	// `remove` is the last word: a key in both is removed, never set.
	let removing: std::collections::BTreeSet<&str> = remove.iter().map(String::as_str).collect();
	let set: Vec<(&String, &String)> = set
		.iter()
		.filter(|(key, _)| !removing.contains(key.as_str()))
		.collect();

	let mut keys: std::collections::BTreeSet<&str> =
		set.iter().map(|(key, _)| key.as_str()).collect();
	keys.extend(removing.iter().copied());

	// What each named person already holds among these keys, and how
	// much they hold altogether — the bound a new key has to fit under.
	let mut existing: std::collections::HashMap<
		Uuid,
		std::collections::BTreeMap<String, (leptoskit_db::Id<PersonAttribute>, String)>,
	> = std::collections::HashMap::new();

	for attribute in PersonAttribute::query()
		.filter(|attribute| {
			attribute.person.in_list(persons.iter().copied())
				& attribute.key.in_list(keys.into_iter().map(str::to_owned))
		})
		.all(tx)
		.await?
	{
		existing
			.entry(attribute.person)
			.or_default()
			.insert(attribute.key, (attribute.id, attribute.value));
	}

	let cap = u64::try_from(crate::MAX_ATTRIBUTES_PER_PERSON).unwrap_or(u64::MAX);
	let held_by = attributes_held_by(tx, &persons).await?;

	for person in &persons {
		let rows = existing.get(person);
		let mut held = held_by.get(person).copied().unwrap_or(0);

		for (key, value) in &set {
			match rows.and_then(|rows| rows.get(key.as_str())) {
				// An idempotent re-set changes nothing and counts nothing.
				Some((_, stored)) if stored.as_str() == value.as_str() => {}
				Some((id, _)) => {
					PersonAttribute::update()
						.filter(|attribute| attribute.id.eq(*id))
						.set(|attribute| attribute.value, (*value).clone())
						.run(tx)
						.await?;
					changes.set += 1;
				}
				None => {
					if held >= cap {
						changes.skipped_full += 1;
						continue;
					}

					let inserted: leptoskit_db::Result<()> = NewPersonAttribute {
						person: *person,
						key: (*key).clone(),
						value: (*value).clone(),
					}
					.insert(tx)
					.await
					.map(|_| ());

					match inserted {
						Ok(()) => {
							changes.set += 1;
							held += 1;
						}
						// Another call wrote this key between the read
						// and the write: it holds, which is what this
						// call wanted.
						Err(leptoskit_db::DbError::Conflict { .. }) => {}
						Err(error) => return Err(error.into()),
					}
				}
			}
		}

		for key in &remove {
			let Some((id, _)) = rows.and_then(|rows| rows.get(key.as_str())) else {
				continue;
			};

			PersonAttribute::query()
				.filter(|attribute| attribute.id.eq(*id))
				.delete(tx)
				.await?;
			changes.removed += 1;
		}
	}

	// The registry's `updated_at` says when the service last wrote about
	// them, attributes included.
	if changes.set > 0 || changes.removed > 0 {
		AudienceMember::update()
			.filter(|person| person.person.in_list(persons.iter().copied()))
			.run(tx)
			.await?;
	}

	Ok(changes)
}

/// Reads one person's attributes, the body of
/// [`person_attributes`](crate::person_attributes): keyed and sorted,
/// empty for a person the service holds nothing about.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn read_attributes(
	db: &impl leptoskit_db::Read,
	person: Uuid,
) -> rpc::Result<std::collections::BTreeMap<String, String>> {
	let rows = PersonAttribute::query()
		.filter(|attribute| attribute.person.eq(person))
		.all(db)
		.await?;

	Ok(rows
		.into_iter()
		.map(|attribute| (attribute.key, attribute.value))
		.collect())
}

/// Erases everything the service holds about one person, the body of
/// [`forget_person`](crate::forget_person): their attributes, topic
/// subscriptions, category preferences, registry row, and wake row, their
/// push endpoints — credentials included, their queued deliveries skipped
/// as revoked — and their inbox rows with those rows' deliveries.
///
/// Everything but the inbox is bounded per person, so it goes in this
/// transaction. The inbox is not: the first batch of rows goes here, and
/// a person with more history than that has the rest erased by
/// [`forget_rows`](crate::forget_rows), a batch per write, so an erasure
/// never holds a whole history in one transaction. Only rows that exist
/// now are erased — a send that names the person afterwards is the app's
/// own later act — which is what `through`, a row id made at this moment,
/// bounds.
///
/// # Errors
///
/// Whatever a read, a write, or the enqueue fails with.
pub(crate) async fn forget_person(cx: &rpc::MutationCx, person: Uuid) -> rpc::Result<()> {
	let tx = cx.tx();

	PersonAttribute::query()
		.filter(|attribute| attribute.person.eq(person))
		.delete(tx)
		.await?;
	TopicSubscription::query()
		.filter(|subscription| subscription.person.eq(person))
		.delete(tx)
		.await?;
	Preference::query()
		.filter(|preference| preference.person.eq(person))
		.delete(tx)
		.await?;
	AudienceMember::query()
		.filter(|row| row.person.eq(person))
		.delete(tx)
		.await?;
	Wake::query()
		.filter(|wake| wake.person.eq(person))
		.delete(tx)
		.await?;
	push::revoke_all(tx, person).await?;
	// A stored recurrence that names the person as a recipient stops
	// naming them, so no later occurrence is sent to them.
	recurrence::forget(tx, person).await?;

	// Row ids are UUID v7, so one made now sorts after every row that
	// exists and before the rows of any later send.
	let through = Uuid::now_v7();

	if forget_batch(tx, person, through).await? {
		// Unkeyed, like a large cancel's job: forgetting again while it
		// works enqueues a second, which finds whatever the first left.
		cx.enqueue(crate::ForgetRows { person, through })
			.at(leptoskit_db::now())
			.await?;
	}

	Ok(())
}

/// Erases one batch of `person`'s inbox rows up to `through`, oldest
/// first, with the deliveries of exactly those rows, and answers whether
/// more are left.
///
/// # Errors
///
/// Whatever a read or a write fails with.
async fn forget_batch(
	db: &impl leptoskit_db::Connection,
	person: Uuid,
	through: Uuid,
) -> rpc::Result<bool> {
	let batch: Vec<Uuid> = Notification::query()
		.filter(|notification| notification.person.eq(person) & notification.id.lte(through))
		.order_by(|notification| notification.id.asc())
		.limit(u64::try_from(crate::FANOUT_BATCH).unwrap_or(u64::MAX))
		.all(db)
		.await?
		.into_iter()
		.map(|notification| notification.id)
		.collect();

	if batch.is_empty() {
		return Ok(false);
	}

	// Deliveries first: were the rows to go first and the write after
	// them fail, nothing would name these deliveries' person again.
	PushDelivery::query()
		.filter(|delivery| delivery.notification.in_list(batch.iter().copied()))
		.delete(db)
		.await?;
	Notification::query()
		.filter(|notification| notification.id.in_list(batch.iter().copied()))
		.delete(db)
		.await?;

	Ok(batch.len() >= crate::FANOUT_BATCH)
}

/// The body of [`forget_rows`](crate::forget_rows): the rest of an erased
/// person's inbox, one batch at a time, each batch its own short write,
/// until none is left.
///
/// # Errors
///
/// Whatever a read or a write fails with; the job runs again and picks up
/// what is left.
pub(crate) async fn forget_rows(db: &Db, person: Uuid, through: Uuid) -> rpc::Result<()> {
	while forget_batch(db, person, through).await? {}

	Ok(())
}

/// How many attributes each of `persons` holds, in one grouped read.
///
/// # Errors
///
/// Whatever the read fails with.
async fn attributes_held_by(
	db: &impl leptoskit_db::Read,
	persons: &std::collections::BTreeSet<Uuid>,
) -> rpc::Result<std::collections::HashMap<Uuid, u64>> {
	if persons.is_empty() {
		return Ok(std::collections::HashMap::new());
	}

	let dialect = db.dialect();
	let mut sql = String::from(
		"SELECT \"person\", count(*) FROM \"person_attributes\" WHERE \"person\" IN (",
	);
	let mut args: Vec<leptoskit_db::Value> = Vec::new();

	for person in persons {
		if !args.is_empty() {
			sql.push_str(", ");
		}

		sql.push_str(&placeholder(dialect, args.len() + 1));
		args.push(leptoskit_db::Value::Uuid(*person));
	}

	sql.push_str(") GROUP BY \"person\"");

	let rows = Statement::new(sql, args)
		.reads::<PersonAttribute>()
		.fetch(db)
		.await?;

	Ok(rows
		.into_iter()
		.filter_map(|row| {
			let person = uuid_of(row.first()?)?;
			let count = match row.get(1)? {
				leptoskit_db::Value::Int(count) => u64::try_from(*count).ok()?,
				_ => return None,
			};
			Some((person, count))
		})
		.collect())
}

/// Whether `topic` is one the operator opened to self-subscription, from
/// the request's carried configuration: the patterns of
/// `LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS`, and nothing at all when the
/// route carries none.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] when the topic isn't open, naming the rule
/// and not the person.
pub(crate) fn check_open_topic(cx: &rpc::MutationCx, topic: &str) -> rpc::Result<()> {
	let open = cx.get::<config::OpenTopics>();

	if !open.is_some_and(|open| open.is_open(topic)) {
		return Err(rpc::Error::forbidden(
			"this topic isn't open to self-subscription: the operator opens topics in \
			 LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS, and everything else is the app's to subscribe \
			 people to",
		));
	}

	Ok(())
}

/// A uuid a raw fetch answered, whichever driver's shape it came in.
fn uuid_of(value: &leptoskit_db::Value) -> Option<Uuid> {
	match value {
		leptoskit_db::Value::Uuid(uuid) => Some(*uuid),
		leptoskit_db::Value::Bytes(bytes) => Uuid::from_slice(bytes).ok(),
		_ => None,
	}
}

/// The body of [`send_test`](crate::send_test): the person's live
/// endpoints — the one named, when an id came — each attempted exactly
/// once through its platform's transport, with nothing written anywhere.
/// The endpoints are read first, in a query of their own; the provider
/// requests go out after it, with no transaction open.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] when no transport is configured at all,
/// [`rpc::Error::NotFound`] for an endpoint id the person doesn't hold
/// live, and whatever a read fails with.
pub(crate) async fn send_test(
	cx: &rpc::ActionCx,
	spec: &crate::SendSpec,
	person: Uuid,
	endpoint: Option<Uuid>,
) -> rpc::Result<Vec<crate::TestSendOutcome>> {
	let Some(runtime) = cx
		.get::<push::Push>()
		.and_then(|push| push.runtime.as_ref())
	else {
		return Err(rpc::Error::invalid(
			"push is not configured on this server: set a transport's configuration to test a send",
		));
	};

	let targets = cx
		.query(TestTargets {
			person,
			endpoint,
			unread: spec.options.badge == Some(crate::options::BadgeSetting::Unread),
			locale: !spec.variants.is_empty(),
		})
		.await?;

	if endpoint.is_some() && targets.endpoints.is_empty() {
		return Err(rpc::Error::not_found(
			"no live endpoint with this id is this person's",
		));
	}

	// One test message id for the whole call, so every endpoint's
	// attempt collapses to the same logical message.
	let badge = match spec.options.badge {
		Some(crate::options::BadgeSetting::Unread) => {
			targets.unread.and_then(|unread| u32::try_from(unread).ok())
		}
		Some(crate::options::BadgeSetting::Set(set)) => Some(set),
		None => None,
	};
	// The text this person would read: their variant's, by the rule a
	// real send resolves it with.
	let text = spec.localized(
		targets
			.locale
			.as_deref()
			.and_then(|locale| spec.variant_for(locale)),
	);
	let message = transport::Message {
		id: Uuid::now_v7(),
		title: text.title.to_owned(),
		body: text.body.to_owned(),
		data: spec.data.clone().unwrap_or_else(|| "{}".to_owned()),
		// A test is pushed now, so its push lifetime counts from now.
		expires_at: transport::push_expiry(
			leptoskit_db::now(),
			spec.expires_at,
			text.options.push_ttl,
		),
		options: text.options.into_owned(),
		badge,
	};

	let mut outcomes = Vec::with_capacity(targets.endpoints.len());

	for (id, platform, credential, apns_environment) in targets.endpoints {
		let platform = crate::push::PushPlatform::from_str_value(&platform)
			.unwrap_or(crate::push::PushPlatform::Apns);
		let attempt = match runtime.transport(platform) {
			Some(transport) => {
				transport
					.attempt(
						transport::Target {
							credential: &credential,
							apns_environment,
						},
						&message,
					)
					.await
			}
			None => {
				transport::Outcome::Ended {
					reason: crate::push::DeliveryReason::TransportUnavailable,
					invalidate: false,
				}
			}
		};

		let (state, reason) = match attempt {
			transport::Outcome::Accepted => (crate::push::DeliveryState::Accepted, None),
			// There is no queue behind a test: a transient answer is a
			// rejected one whose retry is calling again.
			transport::Outcome::Transient { .. } => (crate::push::DeliveryState::Rejected, None),
			transport::Outcome::Stalled { .. } | transport::Outcome::AuthRejected { .. } => {
				(
					crate::push::DeliveryState::Rejected,
					Some(crate::push::DeliveryReason::OperatorConfiguration),
				)
			}
			transport::Outcome::WaitUntil(_) => (crate::push::DeliveryState::Skipped, None),
			transport::Outcome::Ended { reason, .. } => (push::state_of(reason), Some(reason)),
		};

		outcomes.push(crate::TestSendOutcome {
			endpoint: id,
			platform,
			state,
			reason,
		});
	}

	Ok(outcomes)
}

/// A count column's value, saturating.
pub(crate) fn count_of(value: Option<&leptoskit_db::Value>) -> u32 {
	match value {
		Some(leptoskit_db::Value::Int(count)) => u32::try_from(*count).unwrap_or(u32::MAX),
		_ => 0,
	}
}

/// The `number`th placeholder for the dialect.
pub(crate) fn placeholder(dialect: leptoskit_db::Dialect, number: usize) -> String {
	match dialect {
		leptoskit_db::Dialect::Postgres => format!("${number}"),
		leptoskit_db::Dialect::Sqlite => format!("?{number}"),
	}
}

/// What a stored row tells a screen.
pub(crate) fn inbox_item(row: Notification) -> InboxItem {
	InboxItem {
		id: row.id,
		category: row.category,
		title: row.title,
		body: row.body,
		data: row.data,
		created_at: row.created_at,
		read_at: row.read_at,
		dismissed_at: row.dismissed_at,
	}
}

/// The statements one page of a fanout's audience walk runs, for the scale
/// tests to ask the database how it would run them. Not part of the
/// service's API.
#[doc(hidden)]
pub mod plans {
	use leptoskit_rpc as rpc;
	use uuid::Uuid;

	/// The reads behind one page of `target`'s candidates after `cursor`,
	/// exactly as the fanout issues them.
	///
	/// # Errors
	///
	/// [`rpc::Error::server`] for a target with no fanout.
	pub fn candidate_reads(
		dialect: leptoskit_db::Dialect,
		target: &crate::SendTarget,
		cursor: Option<Uuid>,
	) -> rpc::Result<Vec<leptoskit_db::Statement>> {
		let limit = super::FANOUT_PAGE + 1;

		Ok(match super::walk_of(target)? {
			super::Walk::Topics(condition) => {
				condition
					.topics()
					.into_iter()
					.map(|topic| super::subscribers_read(dialect, topic, cursor, limit))
					.collect()
			}
			super::Walk::Registry(_) => vec![super::registry_read(dialect, cursor, limit)],
		})
	}

	/// The read behind a send's report counts, exactly as
	/// [`send_report`](crate::send_report) issues it.
	#[must_use]
	pub fn send_counts(dialect: leptoskit_db::Dialect, send_key: &str) -> leptoskit_db::Statement {
		super::send_counts_read(dialect, send_key)
	}

	/// The read behind a report's push breakdown, exactly as
	/// [`send_report`](crate::send_report) issues it.
	#[must_use]
	pub fn push_counts(dialect: leptoskit_db::Dialect, send_key: &str) -> leptoskit_db::Statement {
		super::history::push_counts_read(dialect, send_key)
	}

	/// The read behind a report's reads by hour, exactly as
	/// [`send_report`](crate::send_report) issues it.
	#[must_use]
	pub fn reads_by_hour(
		dialect: leptoskit_db::Dialect,
		send_key: &str,
	) -> leptoskit_db::Statement {
		super::history::reads_by_hour_read(dialect, send_key)
	}

	/// A list of distinct names the catalogue walks.
	#[derive(Clone, Copy, Debug)]
	pub enum Names<'a> {
		/// The topics anyone subscribes to.
		Topics,
		/// The attribute keys anyone holds.
		AttributeKeys,
		/// The values anyone holds for this key.
		AttributeValues(&'a str),
		/// The categories of sends and of preferences.
		Categories,
	}

	/// The read of up to `most` distinct `names` after `after`, starting
	/// at `from` when the walk is of one prefix, exactly as the catalogue
	/// issues it.
	#[must_use]
	pub fn names(
		dialect: leptoskit_db::Dialect,
		names: Names<'_>,
		after: Option<&str>,
		from: Option<&str>,
		most: u32,
	) -> leptoskit_db::Statement {
		use super::catalogue::Names as Walked;

		let walked = match names {
			Names::Topics => Walked::topics(),
			Names::AttributeKeys => Walked::attribute_keys(),
			Names::AttributeValues(key) => Walked::attribute_values(key),
			Names::Categories => Walked::categories(),
		};

		super::catalogue::names_read(dialect, &walked, after, from, most)
	}

	/// The capped count of a topic's subscribers, as the catalogue issues
	/// it.
	#[must_use]
	pub fn topic_count(dialect: leptoskit_db::Dialect, topic: &str) -> leptoskit_db::Statement {
		super::catalogue::topic_count_read(dialect, topic)
	}

	/// The capped count of the people holding `key`, or `value` for it,
	/// as the catalogue issues it.
	#[must_use]
	pub fn attribute_count(
		dialect: leptoskit_db::Dialect,
		key: &str,
		value: Option<&str>,
	) -> leptoskit_db::Statement {
		super::catalogue::attribute_count_read(dialect, key, value)
	}

	/// The capped count of the people opted out of `category`, as the
	/// catalogue issues it.
	#[must_use]
	pub fn opted_out_count(
		dialect: leptoskit_db::Dialect,
		category: &str,
	) -> leptoskit_db::Statement {
		super::catalogue::opted_out_count_read(dialect, category)
	}

	/// The capped count of the people with a live endpoint on `platform`,
	/// as the overview issues it.
	#[must_use]
	pub fn platform_people(
		dialect: leptoskit_db::Dialect,
		platform: crate::push::PushPlatform,
	) -> leptoskit_db::Statement {
		super::catalogue::platform_people_read(dialect, platform)
	}
}

/// The service's routes, under `/_leptoskit/services/notifications`, with
/// the live socket started separately. Callers presenting
/// `LEPTOSKIT_SERVICE_TOKEN` are the app, and callers presenting a token
/// the app minted for this service are the person it names, which needs
/// `forward_identity` in `[services.notifications]` — on by default for a
/// first-party service. Push is not configured; see
/// [`routes_with_push`].
pub fn routes(db: Db, origin: Option<&str>) -> (Router, rpc::server::Live) {
	use rpc::server::Anonymous;
	use rpc::server::ServiceCallers;

	routes_with(db, origin, ServiceCallers::for_service(Anonymous))
}

/// The service's job workers: they run the [`wake_send`](crate::wake_send)
/// jobs a scheduled or expiring send enqueues, the hourly
/// [`prune`](crate::prune), and with them the change log's upkeep. Start them
/// once beside [`routes`] — the service's own binary does — or a wake waits in
/// `rpc_jobs` until someone does.
///
/// They poll every second rather than the framework's five, so a
/// scheduled notification's wake lands close to its time; everything else
/// is the framework's default.
#[must_use]
pub fn workers(db: Db) -> rpc::server::Jobs {
	workers_with_push(db, push::Push { runtime: None })
}

/// Workers sharing the same push configuration as [`routes_with_push`].
/// Rebuild the runtime at startup; pending jobs use this allocation on
/// recovery. Pruning keeps the default [`config::Retention`]; see
/// [`workers_with_settings`].
#[must_use]
pub fn workers_with_push(db: Db, push: push::Push) -> rpc::server::Jobs {
	workers_with_settings(db, push, config::Retention::default())
}

/// [`workers_with_push`], also carrying how long the service keeps what it
/// no longer needs: what the hourly [`prune`](crate::prune) job deletes.
/// The service's own binary reads it from its environment; an embedded one
/// passes what it wants — [`config::Retention::forever`] to prune nothing.
#[must_use]
pub fn workers_with_settings(
	db: Db,
	push: push::Push,
	retention: config::Retention,
) -> rpc::server::Jobs {
	rpc::server::Jobs::new(db)
		.state(push)
		.state(retention)
		.service(Some(SERVICE))
		.poll(std::time::Duration::from_secs(1))
		.retention(Some(std::time::Duration::from_hours(24)))
}

/// [`routes`], identifying callers with `callers`.
pub fn routes_with(
	db: Db,
	origin: Option<&str>,
	callers: impl rpc::server::Identify,
) -> (Router, rpc::server::Live) {
	let server = rpc::server::Server::new(db)
		.service(SERVICE)
		.identify(callers);

	let live = match origin {
		Some(origin) => server.live().origin(origin),
		None => server.live(),
	};

	let router = Router::new().nest(
		&rpc::service_prefix(SERVICE),
		server.router().merge(live.router()),
	);
	(router, live)
}

/// [`routes_with`], carrying push configuration in every request.
/// Pass a clone of the same configuration to [`workers_with_push`].
/// No topic is open to self-subscription; see [`routes_with_settings`].
pub fn routes_with_push(
	db: Db,
	origin: Option<&str>,
	callers: impl rpc::server::Identify,
	push: push::Push,
) -> (Router, rpc::server::Live) {
	routes_with_settings(db, origin, callers, push, config::OpenTopics::default())
}

/// [`routes_with_push`], also carrying which topics the operator opened
/// to self-subscription, beside the push configuration. The service's own
/// binary builds both from its environment; an embedded one passes what
/// it wants.
pub fn routes_with_settings(
	db: Db,
	origin: Option<&str>,
	callers: impl rpc::server::Identify,
	push: push::Push,
	open_topics: config::OpenTopics,
) -> (Router, rpc::server::Live) {
	routes_with(db, origin, WithSettings::new(callers, push, open_topics))
}

/// [`Identify`](rpc::server::Identify) as the wrapped caller would
/// answer, with the service instance's settings added to the context: its
/// push configuration and the topics open to self-subscription — the
/// framework's own way of carrying request-scoped state to an operation's
/// body, the same road [`rpc::server::ForwardedIdentity`] takes. Nothing
/// in the request can change either.
struct WithSettings<I> {
	inner: I,
	push: push::Push,
	open_topics: config::OpenTopics,
}

impl<I: rpc::server::Identify> WithSettings<I> {
	fn new(inner: I, push: push::Push, open_topics: config::OpenTopics) -> Self {
		Self {
			inner,
			push,
			open_topics,
		}
	}
}

impl<I: rpc::server::Identify> rpc::server::Identify for WithSettings<I> {
	fn identify<'a>(
		&'a self,
		parts: &'a http::request::Parts,
	) -> leptoskit_query::BoxFuture<'a, rpc::Result<rpc::server::Identity>> {
		Box::pin(async move {
			let mut identity = self.inner.identify(parts).await?;
			identity.context = identity
				.context
				.with(self.push.clone())
				.with(self.open_topics.clone());
			Ok(identity)
		})
	}
}
