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

pub mod config;
pub mod push;
pub mod transport;

use std::sync::Arc;

use leptoskit_axum::axum::Router;
use leptoskit_db::Db;
use leptoskit_rpc as rpc;
use sha2::Digest as _;
use sha2::Sha256;
use uuid::Uuid;

use crate::AppSend;
use crate::InboxItem;
use crate::NewAppSend;
use crate::NewNotification;
use crate::NewWake;
use crate::Notification;
use crate::Preference;
use crate::SERVICE;
use crate::SendResult;
use crate::Wake;

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
/// cancelled, not expired, and not in a category they opted out of.
pub(crate) fn visible(
	notification: &<Notification as leptoskit_db::Table>::Columns,
	person: Uuid,
	now: leptoskit_db::Timestamp,
	opted_out: &[String],
) -> leptoskit_db::Expr {
	let mut expr = notification.person.eq(person)
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

/// The canonical form of what a send intends, hashed: the same send twice
/// hashes the same, and a changed category, content, recipient list,
/// schedule, or expiry changes the hash, so reusing its key fails.
pub(crate) fn intent_hash(
	spec: &crate::SendSpec,
	recipients: &std::collections::BTreeSet<Uuid>,
) -> String {
	#[derive(serde::Serialize)]
	struct Intent<'a> {
		spec: &'a crate::SendSpec,
		recipients: &'a std::collections::BTreeSet<Uuid>,
	}

	let intent = Intent { spec, recipients };
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

/// Writes the send row that holds a key, so a second send with it is
/// recognised. The counts are filled in once the fanout has run; both
/// writes are in one transaction, so a send is all or nothing.
///
/// # Errors
///
/// [`leptoskit_db::DbError::Conflict`] when another send holds the key.
pub(crate) async fn write_send(
	tx: &leptoskit_db::Tx,
	key: &str,
	intent: &str,
) -> leptoskit_db::Result<()> {
	NewAppSend {
		idempotency_key: key.to_owned(),
		intent: intent.to_owned(),
		delivered: 0,
		skipped_opted_out: 0,
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
pub(crate) fn answer_send(existing: &AppSend, intent: &str) -> rpc::Result<SendResult> {
	if existing.intent != intent {
		return Err(rpc::Error::conflict(
			"this idempotency key was already used for a different send; use a new key for each \
			 send",
		));
	}

	let delivered = u32::try_from(existing.delivered).unwrap_or(u32::MAX);
	let skipped_opted_out = u32::try_from(existing.skipped_opted_out).unwrap_or(u32::MAX);

	Ok(SendResult {
		delivered,
		skipped_opted_out,
		recipients: delivered.saturating_add(skipped_opted_out),
		duplicate: true,
	})
}

/// What a send's fanout came to, with the rows it wrote so the push
/// deliveries can join them.
#[derive(Clone, Debug)]
pub(crate) struct Fanout {
	pub delivered: u32,
	pub skipped_opted_out: u32,
	/// One pair per inbox row the fanout wrote: the recipient and their
	/// notification's id.
	pub delivered_to: Vec<(Uuid, Uuid)>,
}

/// Writes one durable inbox row per recipient who hasn't opted out of the
/// category, and counts the rest.
///
/// # Errors
///
/// Whatever a write fails with.
pub(crate) async fn fanout(
	tx: &leptoskit_db::Tx,
	send_key: &str,
	spec: &crate::SendSpec,
	recipients: &std::collections::BTreeSet<Uuid>,
) -> rpc::Result<Fanout> {
	let now = leptoskit_db::now();
	let visible_at = spec.visible_at.unwrap_or(now);
	let data = spec.data.as_deref().unwrap_or("{}");
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

		let id = Uuid::now_v7();
		NewNotification {
			id,
			send_key: send_key.to_owned(),
			person: *person,
			category: spec.category.clone(),
			title: spec.title.clone(),
			body: spec.body.clone(),
			data: data.to_owned(),
			visible_at,
			expires_at: spec.expires_at,
			cancelled_at: None,
			read_at: None,
			dismissed_at: None,
		}
		.insert(tx)
		.await?;
		fanout.delivered += 1;
		fanout.delivered_to.push((*person, id));
	}

	Ok(fanout)
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
		Err(leptoskit_db::DbError::NotFound { .. }) => {
			Err(rpc::Error::not_found(
				"no notification with this id is in your inbox",
			))
		}
		Err(error) => Err(error.into()),
	}
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
/// # Errors
///
/// Whatever an enqueue fails with.
pub(crate) async fn schedule_wakes(
	cx: &rpc::MutationCx,
	send_key: &str,
	spec: &crate::SendSpec,
) -> rpc::Result<()> {
	let now = leptoskit_db::now();
	let mut wakes = Vec::with_capacity(2);
	let becomes_visible = spec
		.visible_at
		.map_or(now, |visible_at| visible_at.max(now));
	wakes.push(("visible", becomes_visible));

	if let Some(expires_at) = spec.expires_at {
		wakes.push(("expires", expires_at));
	}

	for (turning, at) in wakes {
		cx.enqueue(crate::WakeSend {
			send_key: send_key.to_owned(),
		})
		.at(at)
		.key(format!("wake:{send_key}:{turning}"))
		.await?;
	}

	Ok(())
}

/// Writes one wake row per person the send delivered to, in one
/// transaction: the clock event that re-runs their open inbox and badge
/// subscriptions. Bounded by [`crate::MAX_RECIPIENTS`] rows; a wake for a
/// send that no longer changes anyone's answer re-runs those queries and
/// sends nothing.
///
/// # Errors
///
/// Whatever the reads or writes fail with.
pub(crate) async fn wake(db: &Db, send_key: &str) -> rpc::Result<()> {
	let people: Vec<Uuid> = Notification::query()
		.filter(|notification| notification.send_key.eq(send_key.to_owned()))
		.order_by(|notification| notification.person.asc())
		.all(db)
		.await?
		.into_iter()
		.map(|notification| notification.person)
		.collect();

	let tx = db.begin().await?;
	let now = leptoskit_db::now();

	for person in people {
		NewWake {
			person,
			woken_at: now,
		}
		.upsert(&tx, |wake| wake.person)
		.await?;
	}

	tx.commit().await?;

	Ok(())
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
/// jobs a scheduled or expiring send enqueues, and with them the change
/// log's upkeep. Start them once beside [`routes`] — the service's own
/// binary does — or a wake waits in `rpc_jobs` until someone does.
///
/// They poll every second rather than the framework's five, so a
/// scheduled notification's wake lands close to its time; everything else
/// is the framework's default.
#[must_use]
pub fn workers(db: Db) -> rpc::server::Jobs {
	rpc::server::Jobs::new(db)
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

/// [`routes_with`], carrying `push`'s configuration in every request's
/// context: the transports its enrollments check against, and the
/// instance key its jobs name back. This is what a service with push
/// configured serves; [`routes`] serves one without. The instance key
/// should be stable across restarts of the same service — its database
/// URL is the natural one — so a queued push job finds its transports
/// again after one.
pub fn routes_with_push(
	db: Db,
	origin: Option<&str>,
	callers: impl rpc::server::Identify,
	push: push::Push,
) -> (Router, rpc::server::Live) {
	push::register_instance(Arc::clone(&push.instance), push.runtime.clone());
	routes_with(db, origin, WithPush::new(callers, push))
}

/// [`Identify`](rpc::server::Identify) as the wrapped caller would
/// answer, with the service instance's push configuration added to the
/// context: the framework's own way of carrying request-scoped state to
/// an operation's body, the same road [`rpc::server::ForwardedIdentity`]
/// takes. Nothing in the request can change it.
struct WithPush<I> {
	inner: I,
	push: push::Push,
}

impl<I: rpc::server::Identify> WithPush<I> {
	fn new(inner: I, push: push::Push) -> Self {
		Self { inner, push }
	}
}

impl<I: rpc::server::Identify> rpc::server::Identify for WithPush<I> {
	fn identify<'a>(
		&'a self,
		parts: &'a http::request::Parts,
	) -> leptoskit_query::BoxFuture<'a, rpc::Result<rpc::server::Identity>> {
		Box::pin(async move {
			let mut identity = self.inner.identify(parts).await?;
			identity.context = identity.context.with(self.push.clone());
			Ok(identity)
		})
	}
}
