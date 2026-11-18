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
use crate::push::PushEndpoint;

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
/// with its audience count, and nothing stored to fan out later.
///
/// # Errors
///
/// [`leptoskit_db::DbError::Conflict`] when another send holds the key.
pub(crate) async fn write_send(
	tx: &leptoskit_db::Tx,
	key: &str,
	intent: &str,
	targeted: u32,
) -> leptoskit_db::Result<()> {
	NewAppSend {
		idempotency_key: key.to_owned(),
		intent: intent.to_owned(),
		delivered: 0,
		skipped_opted_out: 0,
		targeted: i64::from(targeted),
		state: crate::SendState::Complete.as_str().to_owned(),
		target: None,
		spec: None,
		cursor: None,
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
			kind: kind_of(spec),
			options: options_of(spec),
		}
		.insert(tx)
		.await?;
		fanout.delivered += 1;
		fanout.delivered_to.push((*person, id));
	}

	Ok(fanout)
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

/// The options a spec stores on each row — its kind, its presentation,
/// everything the push engine reads per delivery — or `None` when the
/// send carried none.
fn options_of(spec: &crate::SendSpec) -> Option<String> {
	(!spec.options.is_empty())
		.then(|| serde_json::to_string(&spec.options).expect("the options are plain JSON"))
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
		let now = leptoskit_db::now();

		for notification in &page {
			NewWake {
				person: notification.person,
				woken_at: now,
			}
			.upsert(&tx, |wake| wake.person)
			.await?;
		}

		tx.commit().await?;
	}
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

				let matching = evaluate(db, &walk, page).await?;
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
#[derive(Clone, Copy, Debug, Default)]
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
enum Walk {
	/// The distinct subscribers of the condition's topics, evaluated
	/// against it.
	Topics(Condition),
	/// The person registry, evaluated against the condition when it has
	/// one, matched whole when it doesn't — everyone.
	Registry(Option<Condition>),
}

impl Walk {
	/// The condition to evaluate, if any.
	fn condition(&self) -> Option<&Condition> {
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
fn walk_of(target: &crate::SendTarget) -> rpc::Result<Walk> {
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
const FANOUT_PAGE: usize = crate::FANOUT_BATCH;

/// The people one fanout page evaluates next, ordered by person id,
/// after `cursor`, at most `limit`. Raw SQL because the page is one
/// column's distinct values, which the typed query builder reads as
/// whole rows; the statement still names its table, so a connection that
/// tracks reads records it.
///
/// # Errors
///
/// Whatever the read fails with.
async fn candidates(
	db: &impl leptoskit_db::Read,
	walk: &Walk,
	cursor: Option<Uuid>,
	limit: usize,
) -> rpc::Result<Vec<Uuid>> {
	match walk {
		Walk::Topics(condition) => {
			let dialect = db.dialect();
			let topics = condition.topics();
			let mut sql = String::from(
				"SELECT DISTINCT \"person\" FROM \"topic_subscriptions\" WHERE \"topic\" IN (",
			);
			let mut args: Vec<leptoskit_db::Value> = Vec::new();

			for topic in &topics {
				if !args.is_empty() {
					sql.push_str(", ");
				}

				sql.push_str(&placeholder(dialect, args.len() + 1));
				args.push(leptoskit_db::Value::Text((*topic).to_owned()));
			}

			sql.push(')');

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

			let rows = Statement::new(sql, args)
				.reads::<TopicSubscription>()
				.fetch(db)
				.await?;

			Ok(rows
				.into_iter()
				.filter_map(|row| uuid_of(row.first()?))
				.collect())
		}
		Walk::Registry(_) => {
			let dialect = db.dialect();
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

			let rows = Statement::new(sql, args)
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

/// Which of `people` the walk matches, evaluated against the topics and
/// attributes they hold among the ones its condition names. A registry
/// walk with no condition — everyone — matches every person in the page.
///
/// # Errors
///
/// Whatever the read fails with.
async fn evaluate(
	db: &impl leptoskit_db::Read,
	walk: &Walk,
	people: &[Uuid],
) -> rpc::Result<Vec<Uuid>> {
	let Some(condition) = walk.condition() else {
		return Ok(people.to_vec());
	};

	if people.is_empty() {
		return Ok(Vec::new());
	}

	let topics = condition.topics();
	let mut held: std::collections::HashMap<Uuid, std::collections::BTreeSet<String>> =
		std::collections::HashMap::new();

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
			held.entry(row.person).or_default().insert(row.topic);
		}
	}

	let mut attributes: std::collections::HashMap<
		Uuid,
		std::collections::BTreeMap<String, String>,
	> = std::collections::HashMap::new();
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
			attributes
				.entry(row.person)
				.or_default()
				.insert(row.key, row.value);
		}
	}

	Ok(people
		.iter()
		.copied()
		.filter(|person| {
			condition.matches_over(
				|topic| {
					held.get(person)
						.is_some_and(|topics| topics.contains(topic))
				},
				|key| {
					attributes
						.get(person)
						.and_then(|held| held.get(key).map(String::as_str))
				},
			)
		})
		.collect())
}

/// One step of a topic or condition send's fanout, the body of the
/// [`fanout_send`](crate::fanout_send) job: one batch of
/// [`crate::FANOUT_BATCH`] candidates, evaluated and delivered in one
/// transaction, the send's cursor and counts advanced in the same commit,
/// and the successor enqueued when more audience remains.
///
/// Every write in the step is fenced on the send still fanning out: a
/// [`cancel`](crate::cancel) that commits between steps makes the next
/// step's update match nothing, and the step ends without writing. A
/// step that replays — a crash after commit, before the job recorded its
/// success — finds the people it already delivered by the send's unique
/// row per person, skips them, and changes no count.
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

	if page.is_empty() {
		finish_fanout(db, send_key).await?;

		return Ok(());
	}

	let more = page.len() > FANOUT_PAGE;
	let page = &page[..page.len().saturating_sub(usize::from(more))];
	let last = page.last().copied();
	let matching = evaluate(db, &walk, page).await?;

	// One transaction: the batch's rows, its push deliveries, and the
	// cursor that says the batch ran. Everything inside is fenced on the
	// send still fanning out, so a cancel that lands first empties the
	// whole step.
	let tx = db.begin().await?;
	let already: std::collections::BTreeSet<Uuid> = Notification::query()
		.filter(|notification| {
			notification.send_key.eq(send_key.to_owned())
				& notification.person.in_list(matching.iter().copied())
		})
		.all(&tx)
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
		fanout(&tx, send_key, &spec, &fresh).await?
	};

	let delivered_rows = push::deliveries_in_tx(&tx, send_key, &fanout).await?;

	// The advance is fenced on the send still fanning out and on this
	// batch still being the furthest one — a lease that expired and
	// replays a step while its successor committed can only move the
	// cursor forward, never back.
	let mut advance = AppSend::update()
		.filter(|send| {
			send.idempotency_key.eq(send_key.to_owned())
				& send.state.eq(crate::SendState::FanningOut.as_str())
		})
		.set(|send| send.cursor, last);

	if let Some(last) = last {
		advance = advance.filter(|send| send.cursor.is_null() | send.cursor.lt(last));
	}

	let advanced = advance
		.set(
			|send| send.targeted,
			send.targeted + i64::try_from(fresh.len()).unwrap_or(i64::MAX),
		)
		.set(
			|send| send.delivered,
			send.delivered + i64::from(fanout.delivered),
		)
		.set(
			|send| send.skipped_opted_out,
			send.skipped_opted_out + i64::from(fanout.skipped_opted_out),
		)
		.run(&tx)
		.await?;

	// A cancel or a completed fanout claimed the send between the read and
	// this commit: this step writes nothing, and the cancel's own rows
	// stand as the truth.
	if advanced == 0 {
		tx.rollback().await?;

		return Ok(());
	}

	tx.commit().await?;

	if delivered_rows > 0 {
		push::enqueue_round(db, send_key, &spec).await?;
	}

	// The batch's insert alone can't reach an open inbox: a live read's
	// `visible_at <= now` was taken with an earlier `now`, which a row
	// that didn't exist can't match — the wake is the clock event, the
	// same way a recipients send's is. One wake per step, so a batch's
	// people hear about their rows without waiting for the whole
	// audience.
	rpc::enqueue(
		db,
		crate::WakeSend {
			send_key: send_key.to_owned(),
		},
	)
	.at(leptoskit_db::now())
	.key(format!("wake:{send_key}:fanout:{step}"))
	.await?;

	if !more {
		finish_fanout(db, send_key).await?;

		return Ok(());
	}

	// The successor exists only once this step's batch committed, and its
	// key keeps a replayed step from doubling it.
	rpc::enqueue(
		db,
		crate::FanoutSend {
			send_key: send_key.to_owned(),
			step: step.saturating_add(1),
		},
	)
	.at(leptoskit_db::now())
	.key(format!("fanout:{send_key}:{}", step.saturating_add(1)))
	.await?;

	Ok(())
}

/// Marks a fanning send complete — only while it still is, so a cancel
/// that lands first stays a cancel.
///
/// # Errors
///
/// Whatever the write fails with.
async fn finish_fanout(db: &Db, send_key: &str) -> rpc::Result<()> {
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

/// Removes one person from the registry, with their attributes and
/// topic subscriptions, the body of [`forget_person`](crate::forget_person).
/// Their inbox, preferences, and push endpoints stay: forgetting them
/// ends their audiences, not their history or their own choices.
///
/// # Errors
///
/// Whatever the writes fail with.
pub(crate) async fn forget_person(tx: &leptoskit_db::Tx, person: Uuid) -> rpc::Result<()> {
	PersonAttribute::query()
		.filter(|attribute| attribute.person.eq(person))
		.delete(tx)
		.await?;
	TopicSubscription::query()
		.filter(|subscription| subscription.person.eq(person))
		.delete(tx)
		.await?;
	AudienceMember::query()
		.filter(|row| row.person.eq(person))
		.delete(tx)
		.await?;

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
///
/// # Errors
///
/// [`rpc::Error::Invalid`] when no transport is configured at all,
/// [`rpc::Error::NotFound`] for an endpoint id the person doesn't hold
/// live, and whatever a read fails with.
pub(crate) async fn send_test(
	cx: &rpc::MutationCx,
	spec: &crate::SendSpec,
	person: Uuid,
	endpoint: Option<Uuid>,
) -> rpc::Result<Vec<crate::TestSendOutcome>> {
	let push = push::Push::of(cx)?;

	let Some(runtime) = push.runtime.as_ref() else {
		return Err(rpc::Error::invalid(
			"push is not configured on this server: set a transport's configuration to test a send",
		));
	};

	let mut query =
		PushEndpoint::query().filter(|row| row.person.eq(person) & row.invalidated_at.is_null());

	if let Some(endpoint) = endpoint {
		query = query.filter(|row| row.id.eq(endpoint));
	}

	// The blast radius is the person's own enrollment, bounded again
	// here so nothing about a future bound change widens a test.
	let endpoints = query
		.order_by(|row| row.id.asc())
		.limit(u64::try_from(crate::push::MAX_ENDPOINTS).unwrap_or(u64::MAX))
		.all(cx.tx())
		.await?;

	if endpoint.is_some() && endpoints.is_empty() {
		return Err(rpc::Error::not_found(
			"no live endpoint with this id is this person's",
		));
	}

	// One test message id for the whole call, so every endpoint's
	// attempt collapses to the same logical message.
	let badge = match spec.options.badge {
		Some(crate::options::BadgeSetting::Unread) => {
			u32::try_from(unread_for(cx.tx(), person).await?).ok()
		}
		Some(crate::options::BadgeSetting::Set(set)) => Some(set),
		None => None,
	};
	let message = transport::Message {
		id: Uuid::now_v7(),
		title: spec.title.clone(),
		body: spec.body.clone(),
		data: spec.data.clone().unwrap_or_else(|| "{}".to_owned()),
		expires_at: spec.expires_at,
		options: spec.options.clone(),
		badge,
	};

	let mut outcomes = Vec::with_capacity(endpoints.len());

	for endpoint in endpoints {
		let platform = crate::push::PushPlatform::from_str_value(&endpoint.platform)
			.unwrap_or(crate::push::PushPlatform::Apns);
		let attempt = match runtime.transport(platform) {
			Some(transport) => transport.attempt(&endpoint.credential, &message).await,
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
			endpoint: endpoint.id,
			platform,
			state,
			reason,
		});
	}

	Ok(outcomes)
}

/// The `number`th placeholder for the dialect.
fn placeholder(dialect: leptoskit_db::Dialect, number: usize) -> String {
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
	workers_with_push(db, push::Push { runtime: None })
}

/// Workers sharing the same push configuration as [`routes_with_push`].
/// Rebuild the runtime at startup; pending jobs use this allocation on
/// recovery.
#[must_use]
pub fn workers_with_push(db: Db, push: push::Push) -> rpc::server::Jobs {
	rpc::server::Jobs::new(db)
		.state(push)
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
