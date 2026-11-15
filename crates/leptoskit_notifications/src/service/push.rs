//! The push delivery engine: enrollment's server side, the deliveries a
//! send fans out to, and the background rounds that attempt them.
//!
//! The shape of the work is deliberate about what it can promise:
//!
//! - **At least once.** An attempt whose answer was lost retries, so a provider
//!   may take a message twice. Every attempt carries the notification's id as
//!   its collapse id, so repeats replace rather than stack, and nothing here
//!   claims exactly-once or that a banner was seen.
//! - **The attempt is the boundary.** Each attempt re-checks the notification's
//!   schedule, cancellation, expiry, the person's latest opt-out, and the
//!   endpoint's liveness immediately before it sends. A revocation that commits
//!   after that check can still see that one attempt — that race is the
//!   boundary, and it is documented rather than pretended away.
//! - **No transaction crosses the network.** Attempts read, then call the
//!   provider, then write, in that order, with no transaction held across the
//!   call; a send's inbox rows never roll back because one endpoint failed.

use std::sync::Arc;

use leptoskit_db::Db;
use leptoskit_rpc as rpc;
use sha2::Digest as _;
use sha2::Sha256;
use uuid::Uuid;

use super::Fanout;
use crate::push::DeliveryReason;
use crate::push::DeliveryState;
use crate::push::DeliveryStatus;
use crate::push::EndpointSummary;
use crate::push::NewPushDelivery;
use crate::push::NewPushEndpoint;
use crate::push::PushCredential;
use crate::push::PushDelivery;
use crate::push::PushEndpoint;
use crate::push::PushPlatform;
use crate::push::PushSend;
use crate::service::transport::Message;
use crate::service::transport::Outcome;
use crate::service::transport::PushRuntime;

/// The process's push runtime, installed once at startup. Tests install
/// their own per scenario, over their own boundary; production installs
/// before the routes or workers start.
static RUNTIME: std::sync::RwLock<Option<Arc<PushRuntime>>> = std::sync::RwLock::new(None);

/// Installs `runtime` as the process's push transports, replacing
/// whatever was there. Passing `None` disables push: enrollment refuses
/// every platform, and queued deliveries skip as unconfigured.
pub fn install(runtime: Option<PushRuntime>) {
	*RUNTIME
		.write()
		.unwrap_or_else(std::sync::PoisonError::into_inner) = runtime.map(Arc::new);
}

/// The installed runtime, if any.
fn installed() -> Option<Arc<PushRuntime>> {
	RUNTIME
		.read()
		.unwrap_or_else(std::sync::PoisonError::into_inner)
		.clone()
}

/// A credential's fingerprint: the SHA-256 of its canonical form, which
/// is how a credential is recognized without ever searching on the
/// secret itself.
fn fingerprint(canonical: &str) -> String {
	hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// Registers one endpoint for `me`, over the credential their app
/// presented. Idempotent for the same person and credential; refused for
/// a credential another live account holds; refused for a platform with
/// no transport.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] for a credential that fails the shared
/// checks, a host the push-host policy won't push to, a platform this
/// server has no transport for, or enrollment past
/// [`MAX_ENDPOINTS`](crate::push::MAX_ENDPOINTS); [`rpc::Error::Conflict`]
/// when another account holds the credential; [`rpc::Error::Contended`]
/// when the same credential enrolls twice at once.
pub(crate) async fn register(
	cx: &rpc::MutationCx,
	me: Uuid,
	credential: &PushCredential,
) -> rpc::Result<EndpointSummary> {
	crate::push::check_credential(credential).map_err(rpc::Error::invalid)?;

	let runtime = installed().ok_or_else(|| {
		rpc::Error::invalid(
			"push is not configured on this server: set a transport's configuration to enroll an \
			 endpoint",
		)
	})?;

	let platform = credential.platform();

	if !runtime.configured(platform) {
		return Err(rpc::Error::invalid(format!(
			"push for {} is not configured on this server",
			platform.as_str()
		)));
	}

	if let PushCredential::WebPush { endpoint, .. } = credential {
		check_web_host(&runtime, endpoint).map_err(rpc::Error::invalid)?;
	}

	let canonical = credential.canonical().map_err(rpc::Error::invalid)?;
	let fingerprint = fingerprint(&canonical);

	let existing = match PushEndpoint::query()
		.filter(|endpoint| endpoint.fingerprint.eq(fingerprint.clone()))
		.one(cx.tx())
		.await
	{
		Ok(existing) => Some(existing),
		Err(leptoskit_db::DbError::NotFound { .. }) => None,
		Err(error) => return Err(error.into()),
	};

	if let Some(existing) = existing {
		if existing.person == me {
			return Ok(summary(&existing));
		}

		return Err(rpc::Error::conflict(
			"this device credential is already enrolled to another account; sign out of the other \
			 account on the device, which revokes it, and enroll again",
		));
	}

	let held = PushEndpoint::query()
		.filter(|endpoint| endpoint.person.eq(me) & endpoint.invalidated_at.is_null())
		.count(cx.tx())
		.await?;

	if held >= u64::try_from(crate::push::MAX_ENDPOINTS).unwrap_or(u64::MAX) {
		return Err(rpc::Error::invalid(format!(
			"a person holds at most {} live endpoints; revoke one first",
			crate::push::MAX_ENDPOINTS
		)));
	}

	let id = Uuid::now_v7();
	let inserted: leptoskit_db::Result<()> = NewPushEndpoint {
		id,
		person: me,
		platform: platform.as_str().to_owned(),
		credential: canonical,
		fingerprint,
		invalidated_at: None,
	}
	.insert(cx.tx())
	.await
	.map(|_| ());

	match inserted {
		Ok(()) => {}
		// Another enrollment of this credential committed between the
		// read and the write; on Postgres this transaction can no longer
		// read the winner's row, so the mutation runs again from the
		// start and answers from there.
		Err(leptoskit_db::DbError::Conflict { .. }) => return Err(rpc::Error::Contended),
		Err(error) => return Err(error.into()),
	}

	Ok(EndpointSummary {
		id,
		platform,
		created_at: leptoskit_db::now(),
	})
}

/// The push-service host check at enrollment: the endpoint URL's host
/// must be one the installed Web Push transport's policy allows.
fn check_web_host(runtime: &PushRuntime, endpoint: &str) -> Result<(), String> {
	let url = url::Url::parse(endpoint)
		.map_err(|_| "a Web Push endpoint is an absolute https URL".to_owned())?;

	match &runtime.web {
		Some(web) => web.policy().check(&url).map(|_| ()),
		None => Err("push for web_push is not configured on this server".to_owned()),
	}
}

/// The person's endpoints, oldest first. Invalidated ones are included:
/// a device list that hides them can't explain them.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn list(
	db: &impl leptoskit_db::Read,
	me: Uuid,
) -> rpc::Result<Vec<EndpointSummary>> {
	let rows = PushEndpoint::query()
		.filter(|endpoint| endpoint.person.eq(me))
		.order_by(|endpoint| endpoint.created_at.asc())
		.limit(u64::try_from(crate::push::MAX_ENDPOINTS).unwrap_or(u64::MAX))
		.all(db)
		.await?;

	Ok(rows.iter().map(summary).collect())
}

/// The row as its owner sees it.
fn summary(endpoint: &PushEndpoint) -> EndpointSummary {
	EndpointSummary {
		id: endpoint.id,
		platform: PushPlatform::from_str_value(&endpoint.platform).unwrap_or(PushPlatform::Apns),
		created_at: endpoint.created_at,
	}
}

/// Revokes one of `me`'s endpoints: the row is deleted, and its queued
/// deliveries end as revoked. Another person's endpoint — or a strange
/// id — is not found, exactly as if it didn't exist.
///
/// # Errors
///
/// [`rpc::Error::NotFound`] unless the endpoint is `me`'s.
pub(crate) async fn revoke(tx: &leptoskit_db::Tx, me: Uuid, endpoint: Uuid) -> rpc::Result<()> {
	let removed = PushEndpoint::query()
		.filter(|row| row.id.eq(endpoint) & row.person.eq(me))
		.delete(tx)
		.await
		.map_err(|error| {
			match error {
				leptoskit_db::DbError::NotFound { .. } => {
					rpc::Error::not_found("no endpoint with this id is yours")
				}
				other => other.into(),
			}
		})?;

	if removed == 0 {
		return Err(rpc::Error::not_found("no endpoint with this id is yours"));
	}

	// Its queued deliveries never attempt: the endpoint is gone, and the
	// attempt's liveness check would only say so later.
	PushDelivery::update()
		.filter(|delivery| {
			delivery.endpoint.eq(endpoint)
				& delivery.state.in_list([
					DeliveryState::Enqueued.as_str(),
					DeliveryState::Retrying.as_str(),
				])
		})
		.set(|delivery| delivery.state, DeliveryState::Skipped.as_str())
		.set(
			|delivery| delivery.reason,
			DeliveryReason::EndpointRevoked.as_str(),
		)
		.set(|delivery| delivery.finished_at, Some(leptoskit_db::now()))
		.set(|delivery| delivery.next_attempt_at, None)
		.run(tx)
		.await?;

	Ok(())
}

/// The app's page of a send's deliveries, oldest first: state and
/// sanitized reason, and never a credential.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn status_page(
	db: &impl leptoskit_db::Read,
	send_key: &str,
	page: &rpc::PageArgs,
) -> rpc::Result<rpc::Page<DeliveryStatus>> {
	let query = PushDelivery::query().filter(|delivery| delivery.send_key.eq(send_key.to_owned()));

	let page_of_rows = rpc::paginate(
		db,
		query,
		page,
		|delivery| delivery.id.asc(),
		|delivery| delivery.id,
	)
	.await?;

	let items = page_of_rows
		.items
		.iter()
		.map(|row| {
			DeliveryStatus {
				notification: row.notification,
				endpoint: row.endpoint,
				person: row.person,
				platform: PushPlatform::from_str_value(&row.platform).unwrap_or(PushPlatform::Apns),
				state: DeliveryState::from_str_value(&row.state).unwrap_or_default(),
				reason: row
					.reason
					.as_deref()
					.and_then(DeliveryReason::from_str_value),
				attempts: u32::try_from(row.attempts).unwrap_or(u32::MAX),
				attempted_at: row.attempted_at,
				finished_at: row.finished_at,
			}
		})
		.collect();

	Ok(rpc::Page {
		items,
		end: page_of_rows.end,
		has_more: page_of_rows.has_more,
	})
}

/// Enqueues the send's push deliveries, in its own transaction: one per
/// delivered notification and live endpoint, and the round-zero job at
/// the notification's visibility, so a scheduled send's push waits for
/// its time the way its inbox row does. A send with no enrolled
/// endpoints enqueues nothing at all.
///
/// # Errors
///
/// Whatever a write or an enqueue fails with.
pub(crate) async fn schedule(
	cx: &rpc::MutationCx,
	send_key: &str,
	spec: &crate::SendSpec,
	fanout: &Fanout,
) -> rpc::Result<()> {
	if fanout.delivered_to.is_empty() {
		return Ok(());
	}

	let people: std::collections::BTreeSet<Uuid> = fanout
		.delivered_to
		.iter()
		.map(|(person, _)| *person)
		.collect();
	let endpoints = PushEndpoint::query()
		.filter(|endpoint| {
			endpoint.person.in_list(people.iter().copied()) & endpoint.invalidated_at.is_null()
		})
		.all(cx.tx())
		.await?;

	if endpoints.is_empty() {
		return Ok(());
	}

	let mut by_person: std::collections::HashMap<Uuid, Vec<Uuid>> =
		std::collections::HashMap::new();

	for endpoint in &endpoints {
		by_person
			.entry(endpoint.person)
			.or_default()
			.push(endpoint.id);
	}

	let mut enqueued = 0_u32;

	for (person, notification) in &fanout.delivered_to {
		for endpoint in by_person.get(person).into_iter().flatten() {
			NewPushDelivery {
				id: Uuid::now_v7(),
				notification: *notification,
				send_key: send_key.to_owned(),
				endpoint: *endpoint,
				person: *person,
				platform: platform_name(&endpoints, endpoint),
				state: DeliveryState::Enqueued.as_str().to_owned(),
				reason: None,
				attempts: 0,
				next_attempt_at: None,
				attempted_at: None,
				finished_at: None,
			}
			.insert(cx.tx())
			.await?;
			enqueued += 1;
		}
	}

	if enqueued == 0 {
		return Ok(());
	}

	// The round-zero job, at the notification's visibility. Its key keeps
	// a raced or retried send from doubling it, exactly as the wake jobs
	// are kept.
	let now = leptoskit_db::now();
	let becomes_visible = spec
		.visible_at
		.map_or(now, |visible_at| visible_at.max(now));
	cx.enqueue(PushSend {
		send_key: send_key.to_owned(),
		round: 0,
	})
	.at(becomes_visible)
	.key(format!("push:{send_key}:0"))
	.await?;

	Ok(())
}

/// The stored platform of endpoint `id`.
fn platform_name(endpoints: &[PushEndpoint], id: &Uuid) -> String {
	endpoints
		.iter()
		.find(|endpoint| &endpoint.id == id)
		.map_or_else(
			|| PushPlatform::Apns.as_str().to_owned(),
			|endpoint| endpoint.platform.clone(),
		)
}

/// One round of the send's push job: the due deliveries, attempted in
/// bounded batches, and — while anything still waits — the next round,
/// at the earliest moment one of them may run again. The round number
/// keeps the job keys apart, so a round enqueues its successor even
/// while a previous round's row lingers until the retention prunes it.
///
/// # Errors
///
/// Whatever a read, a write, or the successor's enqueue fails with. The
/// job framework retries the round; the deliveries already written are
/// skipped by their state on the retry.
pub(crate) async fn round(cx: &rpc::JobCx, send_key: &str, round: u32) -> rpc::Result<()> {
	let Some(runtime) = installed() else {
		// Nothing is configured: every delivery of this send skips,
		// loudly and visibly in the send's status.
		skip_all(cx.db(), send_key, DeliveryReason::TransportUnavailable).await?;
		return Ok(());
	};

	let now = leptoskit_db::now();
	let mut due = PushDelivery::query()
		.filter(|delivery| {
			delivery.send_key.eq(send_key.to_owned())
				& delivery.state.in_list([
					DeliveryState::Enqueued.as_str(),
					DeliveryState::Retrying.as_str(),
				])
				& (delivery.next_attempt_at.is_null() | delivery.next_attempt_at.lte(now))
		})
		.order_by(|delivery| delivery.id.asc())
		.all(cx.db())
		.await?;

	// Bounded batches: at most DELIVERY_CONCURRENCY attempts in flight,
	// each owning its row.
	while !due.is_empty() {
		let take = due.len().min(crate::push::DELIVERY_CONCURRENCY);
		let batch: Vec<PushDelivery> = due.drain(..take).collect();
		let mut attempts = tokio::task::JoinSet::new();

		for row in batch {
			let db = cx.db().clone();
			let runtime = Arc::clone(&runtime);
			attempts.spawn(try_delivery(db, runtime, row));
		}

		while let Some(finished) = attempts.join_next().await {
			// An attempt writes its own outcome; one that failed or
			// panicked fails the round, and the framework retries it.
			finished.map_err(|error| {
				rpc::Error::server(format!("a delivery attempt failed: {error}"))
			})??;
		}
	}

	// While anything still waits — a retry whose time hasn't come, or an
	// attempt pushed out by a Retry-After — the next round runs at the
	// earliest of those moments.
	let waiting = PushDelivery::query()
		.filter(|delivery| {
			delivery.send_key.eq(send_key.to_owned())
				& delivery.state.in_list([
					DeliveryState::Enqueued.as_str(),
					DeliveryState::Retrying.as_str(),
				])
		})
		.order_by(|delivery| delivery.next_attempt_at.asc())
		.first(cx.db())
		.await?;

	if let Some(next) = waiting
		&& let Some(at) = next.next_attempt_at
	{
		rpc::enqueue(
			cx.db(),
			PushSend {
				send_key: send_key.to_owned(),
				round: round.saturating_add(1),
			},
		)
		.at(at.max(leptoskit_db::now()))
		.key(format!("push:{send_key}:{}", round.saturating_add(1)))
		.await?;
	}

	Ok(())
}

/// Marks every queued delivery of a send skipped, for the reason that
/// holds for all of them at once.
///
/// # Errors
///
/// Whatever the write fails with.
async fn skip_all(db: &Db, send_key: &str, reason: DeliveryReason) -> rpc::Result<()> {
	PushDelivery::update()
		.filter(|delivery| {
			delivery.send_key.eq(send_key.to_owned())
				& delivery.state.in_list([
					DeliveryState::Enqueued.as_str(),
					DeliveryState::Retrying.as_str(),
				])
		})
		.set(|delivery| delivery.state, DeliveryState::Skipped.as_str())
		.set(|delivery| delivery.reason, reason.as_str())
		.set(|delivery| delivery.finished_at, Some(leptoskit_db::now()))
		.set(|delivery| delivery.next_attempt_at, None)
		.run(db)
		.await?;

	Ok(())
}

/// Attempts one delivery: the checks at the moment of the attempt, the
/// network call with nothing else held across it, and the outcome's
/// write.
///
/// # Errors
///
/// A read or a write that fails: the round stops and the framework
/// retries it.
async fn try_delivery(
	db: Db,
	runtime: Arc<PushRuntime>,
	delivery: PushDelivery,
) -> rpc::Result<()> {
	let outcome = attempt(&db, &runtime, &delivery).await?;
	write_outcome(&db, &delivery, outcome).await
}

/// The attempt, one delivery's whole story.
///
/// # Errors
///
/// A read that fails: the round stops and the framework retries it.
async fn attempt(db: &Db, runtime: &PushRuntime, delivery: &PushDelivery) -> rpc::Result<Outcome> {
	let now = leptoskit_db::now();

	// The endpoint, live: revoked rows are deleted, invalidated rows
	// stay.
	let endpoint = match PushEndpoint::query()
		.filter(|endpoint| endpoint.id.eq(delivery.endpoint))
		.one(db)
		.await
	{
		Ok(endpoint) if endpoint.invalidated_at.is_none() => endpoint,
		Ok(_) => {
			return Ok(Outcome::Ended {
				reason: DeliveryReason::EndpointUnavailable,
				invalidate: false,
			});
		}
		Err(leptoskit_db::DbError::NotFound { .. }) => {
			return Ok(Outcome::Ended {
				reason: DeliveryReason::EndpointRevoked,
				invalidate: false,
			});
		}
		Err(error) => return Err(error.into()),
	};

	let notification = match crate::Notification::query()
		.filter(|notification| notification.id.eq(delivery.notification))
		.one(db)
		.await
	{
		Ok(notification) => notification,
		Err(leptoskit_db::DbError::NotFound { .. }) => {
			// The inbox row is gone, which nothing this service does
			// explains: treat it as cancelled rather than push nothing
			// forever.
			return Ok(Outcome::Ended {
				reason: DeliveryReason::Cancelled,
				invalidate: false,
			});
		}
		Err(error) => return Err(error.into()),
	};

	// The schedule, at the moment of the attempt: not yet visible waits;
	// expired, cancelled, and opted-out end here, and the inbox — whose
	// visibility rules are the same — never shows what the push didn't.
	if notification.visible_at > now {
		return Ok(Outcome::WaitUntil(notification.visible_at));
	}

	if notification.cancelled_at.is_some() {
		return Ok(Outcome::Ended {
			reason: DeliveryReason::Cancelled,
			invalidate: false,
		});
	}

	if notification
		.expires_at
		.is_some_and(|expires_at| expires_at <= now)
	{
		return Ok(Outcome::Ended {
			reason: DeliveryReason::Expired,
			invalidate: false,
		});
	}

	if crate::Preference::query()
		.filter(|preference| {
			preference.person.eq(delivery.person)
				& preference.category.eq(notification.category.clone())
				& preference.opted_out.eq(true)
		})
		.first(db)
		.await?
		.is_some()
	{
		return Ok(Outcome::Ended {
			reason: DeliveryReason::OptedOut,
			invalidate: false,
		});
	}

	// The attempt boundary is here: everything after this line is the
	// send itself, and a revocation that commits after this point may
	// still see this attempt.

	let Some(transport) = runtime
		.transport(PushPlatform::from_str_value(&delivery.platform).unwrap_or(PushPlatform::Apns))
	else {
		return Ok(Outcome::Ended {
			reason: DeliveryReason::TransportUnavailable,
			invalidate: false,
		});
	};

	let message = Message {
		id: notification.id,
		title: notification.title.clone(),
		body: notification.body.clone(),
		data: notification.data.clone(),
		expires_at: notification.expires_at,
	};

	Ok(transport.attempt(&endpoint.credential, &message).await)
}

/// Writes one attempt's outcome, in its own short write: no transaction
/// is held across the network, so this is the first write since the
/// read.
///
/// # Errors
///
/// A write that fails: the round stops and the framework retries it,
/// skipping what already wrote by its state.
async fn write_outcome(db: &Db, delivery: &PushDelivery, outcome: Outcome) -> rpc::Result<()> {
	let now = leptoskit_db::now();
	let attempts = delivery.attempts.saturating_add(1);

	match outcome {
		Outcome::Accepted => {
			PushDelivery::update()
				.filter(|row| row.id.eq(delivery.id))
				.set(|row| row.state, DeliveryState::Accepted.as_str())
				.set(|row| row.reason, None)
				.set(|row| row.attempts, attempts)
				.set(|row| row.attempted_at, Some(now))
				.set(|row| row.finished_at, Some(now))
				.set(|row| row.next_attempt_at, None)
				.run(db)
				.await?;
		}
		Outcome::Transient { after } => {
			if attempts > i64::from(crate::push::MAX_DELIVERY_ATTEMPTS) {
				// The attempts ran out: rejected, with the reason that
				// says so.
				PushDelivery::update()
					.filter(|row| row.id.eq(delivery.id))
					.set(|row| row.state, DeliveryState::Rejected.as_str())
					.set(|row| row.reason, DeliveryReason::AttemptsExhausted.as_str())
					.set(|row| row.attempts, attempts)
					.set(|row| row.attempted_at, Some(now))
					.set(|row| row.finished_at, Some(now))
					.set(|row| row.next_attempt_at, None)
					.run(db)
					.await?;
			} else {
				// The provider's `Retry-After` wins outright — it knows
				// its own queue — and the engine's backoff is only its
				// own guess when the provider gave nothing. The ceiling
				// holds either way.
				let delay = after.unwrap_or_else(|| {
					std::time::Duration::from_secs(retry_backoff_seconds(attempts))
				});
				let at = now
					+ chrono::Duration::from_std(delay)
						.unwrap_or(chrono::Duration::seconds(crate::push::MAX_RETRY_DELAY_SECS));
				PushDelivery::update()
					.filter(|row| row.id.eq(delivery.id))
					.set(|row| row.state, DeliveryState::Retrying.as_str())
					.set(|row| row.reason, None)
					.set(|row| row.attempts, attempts)
					.set(|row| row.attempted_at, Some(now))
					.set(|row| row.next_attempt_at, Some(at))
					.run(db)
					.await?;
			}
		}
		Outcome::WaitUntil(at) => {
			PushDelivery::update()
				.filter(|row| row.id.eq(delivery.id))
				.set(|row| row.next_attempt_at, Some(at))
				.run(db)
				.await?;
		}
		Outcome::Ended { reason, invalidate } => {
			// Why it ended decides what it counts as: the notification's
			// state or the endpoint's skipped it, the provider's answer
			// rejected it.
			let state = match reason {
				DeliveryReason::Expired
				| DeliveryReason::Cancelled
				| DeliveryReason::OptedOut
				| DeliveryReason::EndpointRevoked
				| DeliveryReason::EndpointUnavailable
				| DeliveryReason::TransportUnavailable => DeliveryState::Skipped.as_str(),
				DeliveryReason::Oversized
				| DeliveryReason::Unregistered
				| DeliveryReason::ProviderRejected
				| DeliveryReason::AttemptsExhausted => DeliveryState::Rejected.as_str(),
			};

			if invalidate {
				// The credential is dead at the provider: the endpoint
				// stops taking deliveries, and its other queued ones end
				// now instead of failing the same way one by one. One
				// transaction, so the endpoint and its siblings move
				// together.
				let tx = db.begin().await?;

				PushEndpoint::update()
					.filter(|endpoint| endpoint.id.eq(delivery.endpoint))
					.set(|endpoint| endpoint.invalidated_at, Some(now))
					.run(&tx)
					.await?;

				PushDelivery::update()
					.filter(|row| {
						row.id.ne(delivery.id)
							& row.endpoint.eq(delivery.endpoint)
							& row.state.in_list([
								DeliveryState::Enqueued.as_str(),
								DeliveryState::Retrying.as_str(),
							])
					})
					.set(|row| row.state, DeliveryState::Skipped.as_str())
					.set(
						|row| row.reason,
						DeliveryReason::EndpointUnavailable.as_str(),
					)
					.set(|row| row.finished_at, Some(now))
					.set(|row| row.next_attempt_at, None)
					.run(&tx)
					.await?;

				tx.commit().await?;
			}

			PushDelivery::update()
				.filter(|row| row.id.eq(delivery.id))
				.set(|row| row.state, state)
				.set(|row| row.reason, reason.as_str())
				.set(|row| row.attempts, attempts)
				.set(|row| row.attempted_at, Some(now))
				.set(|row| row.finished_at, Some(now))
				.set(|row| row.next_attempt_at, None)
				.run(db)
				.await?;
		}
	}

	Ok(())
}

/// The engine's own backoff before attempt `attempt`, in seconds: one
/// second, doubling to five minutes.
fn retry_backoff_seconds(attempt: i64) -> u64 {
	let doublings = u32::try_from(attempt.saturating_sub(1).max(0))
		.unwrap_or(16)
		.min(16);
	let seconds = u64::checked_shl(1, doublings).unwrap_or(u64::MAX).min(300);

	seconds.max(1)
}
