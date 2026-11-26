//! The push delivery engine: enrollment's server side, the deliveries a
//! send fans out to, and the background rounds that attempt them.
//!
//! The shape of the work is deliberate about what it can promise:
//!
//! - **At least once.** An attempt whose answer was lost retries, so a provider
//!   may take a message twice. Every attempt carries the notification's id as
//!   its collapse id, so repeats replace rather than stack where the provider
//!   coalesces — best-effort, never a guarantee a banner shows once — and
//!   nothing here claims exactly-once or that a banner was seen.
//! - **The attempt is the boundary.** Each attempt re-checks the notification's
//!   schedule, cancellation, expiry, the person's latest opt-out, and the
//!   endpoint's liveness immediately before it sends. A revocation that commits
//!   after that check can still see that one attempt — that race is the
//!   boundary, and it is documented rather than pretended away. What a
//!   revocation can never do is bring a delivery back: every outcome's write is
//!   fenced on the row's own state and attempt count, so an in-flight answer
//!   that lands after a revoke, a skip, or a peer round's write finds the row
//!   gone from the queue and writes nothing.
//! - **No transaction crosses the network.** Attempts read, then call the
//!   provider, then write, in that order, with no transaction held across the
//!   call; a send's inbox rows never roll back because one endpoint failed.
//! - **The transports are per service instance, not per process.** A process
//!   may serve several notifications services, each with its own credentials:
//!   routes and workers share the same explicit runtime. Jobs persist only
//!   logical send and round identifiers; credentials stay in worker memory.

use std::collections::HashMap;
use std::sync::Arc;

use leptoskit_db::Db;
use leptoskit_rpc as rpc;
use sha2::Digest as _;
use sha2::Sha256;
use uuid::Uuid;

use super::Facts;
use super::Fanout;
use super::unread_for;
use crate::condition::Condition;
use crate::condition::Device;
use crate::push::DeliveryReason;
use crate::push::DeliveryState;
use crate::push::DeliveryStatus;
use crate::push::DeviceDetails;
use crate::push::EndpointSummary;
use crate::push::NewPushDelivery;
use crate::push::NewPushEndpoint;
use crate::push::PushCredential;
use crate::push::PushDelivery;
use crate::push::PushEndpoint;
use crate::push::PushPlatform;
use crate::push::PushSend;
use crate::service::transport;
use crate::service::transport::Message;
use crate::service::transport::Outcome;
use crate::service::transport::PushRuntime;

/// How long an operator-side stall waits before it tries again: a broken
/// OAuth exchange or a refused token keeps the delivery queued, at a
/// calm pace, until the operator fixes it.
const STALLED_RETRY_SECS: i64 = 60;

/// The service instance's push configuration, carried in the request
/// context the routes build and in worker state. Not a process global — a
/// process may serve several notifications services — and not
/// interchangeable: it is fixed when the routes are built.
#[derive(Clone)]
pub struct Push {
	/// The instance's transports, or `None` when push is unconfigured —
	/// enrollment refuses every platform, and queued deliveries skip as
	/// unconfigured.
	pub runtime: Option<Arc<PushRuntime>>,
	/// Whether whoever runs the service has stopped its pushes, such as a
	/// hosting plan whose allowance is used. Shared by every clone, so it
	/// can be turned on and off without building the routes again.
	pub paused: Paused,
}

/// A switch that stops a service instance's pushes while it is on. Queued
/// deliveries skip as [`DeliveryReason::Paused`] and test sends are
/// refused; enrollment, the inbox and everything else go on. Clones share
/// one switch.
#[derive(Clone, Debug, Default)]
pub struct Paused(Arc<std::sync::atomic::AtomicBool>);

impl Paused {
	/// Turns the switch on or off.
	pub fn set(&self, paused: bool) {
		self.0.store(paused, std::sync::atomic::Ordering::Relaxed);
	}

	/// Whether pushes are stopped now.
	#[must_use]
	pub fn get(&self) -> bool {
		self.0.load(std::sync::atomic::Ordering::Relaxed)
	}
}

impl Push {
	/// The push configuration of the calling request, or the refusal for
	/// a route built without one: push unconfigured on this server.
	pub(crate) fn of(cx: &rpc::MutationCx) -> rpc::Result<&Self> {
		cx.get::<Push>().ok_or_else(push_unconfigured)
	}
}

fn push_unconfigured() -> rpc::Error {
	rpc::Error::invalid(
		"push is not configured on this server: set a transport's configuration to enroll an \
		 endpoint",
	)
}

/// An endpoint's fingerprint: the SHA-256 of its platform and its
/// destination — the token a device was issued, or a Web Push
/// subscription's endpoint URL — which is how an endpoint is recognized
/// without ever searching on the secret itself. It deliberately does not
/// cover a Web Push subscription's keys, so a browser that rotates them
/// keeps its endpoint and its owner, while no other account can take a
/// live endpoint over by presenting new keys for it.
fn fingerprint(platform: PushPlatform, destination: &str) -> String {
	hex::encode(Sha256::digest(
		format!("{}\u{0}{destination}", platform.as_str()).as_bytes(),
	))
}

/// The answer to a renewal of an enrollment the caller no longer holds:
/// not found, in the words the enrollment clients read
/// ([`ENROLLMENT_GONE`](crate::push::ENROLLMENT_GONE)).
fn enrollment_gone() -> rpc::Error {
	rpc::Error::not_found(format!(
		"{}: it was revoked or erased on the server, so nothing was registered; enrolling again \
		 takes the person's own gesture",
		crate::push::ENROLLMENT_GONE
	))
}

/// Registers one endpoint for `me`, over the credential their app
/// presented and the details its device reported. Idempotent for the same
/// person and destination: a Web Push subscription whose browser rotated
/// its keys updates the stored credential in place, keeping the endpoint,
/// its owner, and its queued deliveries. The details are replaced on every
/// registration; a change to them alone is a plain update that leaves the
/// credential's generation, and so every queued delivery, as it was.
///
/// A destination another account holds changes hands when — and only when
/// — the caller presents the byte-identical credential, keys included for
/// a Web Push subscription: holding the credential is holding the device.
/// The previous owner's endpoint is revoked — deleted, its queued
/// deliveries skipped as revoked, live or invalidated alike — and the
/// caller gets a fresh endpoint with a fresh id. The same destination with
/// different keys proves nothing and is refused. Refused for a platform
/// with no transport.
///
/// `renews` makes the call a renewal rather than an enrollment: it names
/// the endpoint the caller's device believes it holds, and the call writes
/// nothing unless that endpoint is still the caller's. A launch-time
/// reconciliation sends it, so an enrollment the server revoked or erased
/// stays gone — and a renewal never takes a destination over from anyone.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] for a credential or details that fail the
/// shared checks, a host the push-host policy won't push to, a platform
/// this server has no transport for, or enrollment past
/// [`MAX_ENDPOINTS`](crate::push::MAX_ENDPOINTS); [`rpc::Error::Conflict`]
/// when another account holds the destination under a different
/// credential; [`rpc::Error::NotFound`] for a renewal of an enrollment the
/// caller no longer holds; [`rpc::Error::Contended`] when the same
/// destination enrolls twice at once.
pub(crate) async fn register(
	cx: &rpc::MutationCx,
	push: &Push,
	me: Uuid,
	credential: &PushCredential,
	device: &DeviceDetails,
	renews: Option<Uuid>,
) -> rpc::Result<EndpointSummary> {
	crate::push::check_credential(credential).map_err(rpc::Error::invalid)?;
	let platform = credential.platform();
	let device = device
		.normalized()
		.map_err(rpc::Error::invalid)?
		.for_platform(platform);

	let runtime = push.runtime.as_ref().ok_or_else(push_unconfigured)?;

	if !runtime.configured(platform) {
		return Err(rpc::Error::invalid(format!(
			"push for {} is not configured on this server",
			platform.as_str()
		)));
	}

	if let PushCredential::WebPush { endpoint, .. } = credential {
		check_web_host(runtime, endpoint).map_err(rpc::Error::invalid)?;
	}

	let destination = credential.destination().map_err(rpc::Error::invalid)?;
	let canonical = credential.canonical().map_err(rpc::Error::invalid)?;
	let fingerprint = fingerprint(platform, &destination);

	// A renewal stands on the enrollment it names. Once that is gone —
	// revoked from another device's list, taken over by the device's next
	// person, erased with its owner — the device hears so, and nothing is
	// written: only a gesture enrolls.
	if let Some(renews) = renews
		&& !standing(cx.tx(), me, renews).await?
	{
		return Err(enrollment_gone());
	}

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
			if existing.invalidated_at.is_some() {
				check_capacity(cx.tx(), me).await?;
			}
			// The same person's device: a re-enrollment rotates the
			// stored credential in place — a browser's new keys, a
			// device's refreshed token — and reactivates an endpoint a
			// provider had invalidated, keeping the endpoint's id, its
			// owner, and its queued deliveries. Nothing about the
			// destination changed.
			// An endpoint the app's server imported becomes the device's
			// own at its first enrollment: the mark goes, and what the
			// device says about itself replaces what the import said.
			let imported = existing.imported_at.is_some();

			if existing.credential != canonical || existing.invalidated_at.is_some() {
				let generation = existing
					.generation
					.checked_add(1)
					.ok_or_else(|| rpc::Error::server("endpoint generation exhausted"))?;
				PushEndpoint::update()
					.filter(|endpoint| endpoint.id.eq(existing.id))
					.set(|endpoint| endpoint.generation, generation)
					.set(|endpoint| endpoint.credential, canonical)
					.set(|endpoint| endpoint.invalidated_at, None)
					.set(|endpoint| endpoint.imported_at, None)
					.set(|endpoint| endpoint.locale, device.locale.clone())
					.set(|endpoint| endpoint.time_zone, device.time_zone.clone())
					.set(|endpoint| endpoint.app_version, device.app_version.clone())
					.set(
						|endpoint| endpoint.apns_environment,
						stored_environment(&device),
					)
					.set(|endpoint| endpoint.seen_at, Some(leptoskit_db::now()))
					.run(cx.tx())
					.await?;
				// Pending work follows the owner's new credentials; terminal
				// history stays fixed.
				PushDelivery::update()
					.filter(|row| {
						row.endpoint.eq(existing.id)
							& row.state.in_list([
								DeliveryState::Enqueued.as_str(),
								DeliveryState::Retrying.as_str(),
							])
					})
					.set(|row| row.generation, generation)
					.set(|row| row.next_attempt_at, Some(leptoskit_db::now()))
					.run(cx.tx())
					.await?;
			} else if imported || existing.device_details() != device {
				// Only what the device says about itself changed — an app
				// update, a new language, a move across time zones — or
				// this is the device's first word about an imported
				// endpoint. The credential and its generation stay, so
				// queued deliveries and in-flight attempts never notice.
				PushEndpoint::update()
					.filter(|endpoint| endpoint.id.eq(existing.id))
					.set(|endpoint| endpoint.imported_at, None)
					.set(|endpoint| endpoint.locale, device.locale.clone())
					.set(|endpoint| endpoint.time_zone, device.time_zone.clone())
					.set(|endpoint| endpoint.app_version, device.app_version.clone())
					.set(
						|endpoint| endpoint.apns_environment,
						stored_environment(&device),
					)
					.set(|endpoint| endpoint.seen_at, Some(leptoskit_db::now()))
					.run(cx.tx())
					.await?;
			} else if seen_long_ago(&existing) {
				// Nothing changed, and that is news too: a send that
				// resolves this person's language or time zone reads
				// their most recently seen endpoint. A browser renews on
				// every launch, so the stamp moves at most once in
				// `SEEN_EVERY_SECS`, not once a launch.
				PushEndpoint::update()
					.filter(|endpoint| endpoint.id.eq(existing.id))
					.set(|endpoint| endpoint.seen_at, Some(leptoskit_db::now()))
					.run(cx.tx())
					.await?;
			}

			// The registry is left alone: holding an endpoint means its
			// first enrollment registered them, and only an erasure
			// unregisters a person — taking the endpoint with it. A
			// re-enrollment that brought them back would undo a
			// `forget_person` at the next launch.
			return Ok(EndpointSummary {
				id: existing.id,
				platform,
				created_at: existing.created_at,
				invalidated_at: None,
				device,
				imported: false,
			});
		}

		// A renewal never moves a destination between accounts: the
		// device's enrollment under this person no longer describes it.
		if renews.is_some() {
			return Err(enrollment_gone());
		}

		// Another account's, and the caller holds something else for the
		// same destination — a Web Push URL under different keys. That
		// proves no possession, so nothing moves.
		if existing.credential != canonical {
			return Err(rpc::Error::conflict(
				"this device belongs to another account; its owner must revoke it first",
			));
		}

		// The byte-identical credential: the device is in the caller's
		// hands now. Its previous person's endpoint is revoked — their
		// notifications stop reaching a device they no longer hold — and
		// the caller enrolls from clean below.
		revoke(cx.tx(), existing.person, existing.id).await?;
	}

	check_capacity(cx.tx(), me).await?;

	let id = Uuid::now_v7();
	let inserted: leptoskit_db::Result<()> = NewPushEndpoint {
		id,
		person: me,
		platform: platform.as_str().to_owned(),
		credential: canonical,
		generation: 1,
		fingerprint,
		invalidated_at: None,
		locale: device.locale.clone(),
		time_zone: device.time_zone.clone(),
		app_version: device.app_version.clone(),
		seen_at: Some(leptoskit_db::now()),
		imported_at: None,
		apns_environment: stored_environment(&device),
	}
	.insert(cx.tx())
	.await
	.map(|_| ());

	match inserted {
		Ok(()) => {}
		// Another enrollment of this destination committed between the
		// read and the write; on Postgres this transaction can no longer
		// read the winner's row, so the mutation runs again from the
		// start and answers from there.
		Err(leptoskit_db::DbError::Conflict { .. }) => return Err(rpc::Error::Contended),
		Err(error) => return Err(error.into()),
	}

	// Enrolling a device is one of the acts that makes a person known:
	// someone push can reach belongs in the registry.
	super::register_persons(cx.tx(), &std::collections::BTreeSet::from([me])).await?;
	trim_history(cx.tx(), me).await?;

	Ok(EndpointSummary {
		id,
		platform,
		created_at: leptoskit_db::now(),
		invalidated_at: None,
		device,
		imported: false,
	})
}

/// The body of [`import_push_endpoints`](crate::push::import_push_endpoints):
/// each entry becomes an endpoint of its person unless that person
/// already holds the token (unchanged), another person does (skipped:
/// an import never moves a device), or the person is at the live-endpoint
/// bound (skipped). Imported endpoints are marked, carry the details the
/// entry states, and have no `seen_at`: no device has said anything yet.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] for an import that fails the shared check or
/// names a platform with no transport, and [`rpc::Error::Contended`] when
/// one of its tokens enrolls at the same moment.
pub(crate) async fn import(
	tx: &leptoskit_db::Tx,
	push: &Push,
	endpoints: &[crate::push::ImportEndpoint],
) -> rpc::Result<crate::push::ImportedEndpoints> {
	crate::push::check_import(endpoints).map_err(rpc::Error::invalid)?;
	let runtime = push.runtime.as_ref().ok_or_else(push_unconfigured)?;

	// Everything that can refuse the call is settled before the first
	// write: a bad entry leaves nothing behind on either engine.
	let mut entries = Vec::with_capacity(endpoints.len());

	for (index, entry) in endpoints.iter().enumerate() {
		let platform = entry.credential.platform();

		if !runtime.configured(platform) {
			return Err(rpc::Error::invalid(format!(
				"endpoints[{index}]: push for {} is not configured on this server",
				platform.as_str()
			)));
		}

		let named = |complaint| rpc::Error::invalid(format!("endpoints[{index}]: {complaint}"));
		let device = entry
			.device
			.normalized()
			.map_err(named)?
			.for_platform(platform);
		let destination = entry.credential.destination().map_err(named)?;
		let canonical = entry.credential.canonical().map_err(named)?;

		entries.push((
			entry.person,
			platform,
			fingerprint(platform, &destination),
			canonical,
			device,
		));
	}

	let mut report = crate::push::ImportedEndpoints::default();
	let mut persons = std::collections::BTreeSet::new();
	let now = leptoskit_db::now();

	for (index, (person, platform, fingerprint, canonical, device)) in
		entries.into_iter().enumerate()
	{
		let index = u32::try_from(index).unwrap_or(u32::MAX);
		let holder = match PushEndpoint::query()
			.filter(|endpoint| endpoint.fingerprint.eq(fingerprint.clone()))
			.one(tx)
			.await
		{
			Ok(existing) => Some(existing.person),
			Err(leptoskit_db::DbError::NotFound { .. }) => None,
			Err(error) => return Err(error.into()),
		};

		match holder {
			Some(holder) if holder == person => {
				report.unchanged += 1;
				continue;
			}
			Some(_) => {
				report.skipped.push(crate::push::SkippedImport {
					index,
					reason: crate::push::ImportSkip::HeldByAnother,
				});
				continue;
			}
			None => {}
		}

		let held = PushEndpoint::query()
			.filter(|endpoint| endpoint.person.eq(person) & endpoint.invalidated_at.is_null())
			.count(tx)
			.await?;

		if held >= u64::try_from(crate::push::MAX_ENDPOINTS).unwrap_or(u64::MAX) {
			report.skipped.push(crate::push::SkippedImport {
				index,
				reason: crate::push::ImportSkip::PersonFull,
			});
			continue;
		}

		let inserted = NewPushEndpoint {
			id: Uuid::now_v7(),
			person,
			platform: platform.as_str().to_owned(),
			credential: canonical,
			generation: 1,
			fingerprint,
			invalidated_at: None,
			apns_environment: stored_environment(&device),
			locale: device.locale,
			time_zone: device.time_zone,
			app_version: device.app_version,
			seen_at: None,
			imported_at: Some(now),
		}
		.insert(tx)
		.await;

		match inserted {
			Ok(_) => {}
			// The device enrolled this token itself between the read and
			// the write: the call runs again and finds it.
			Err(leptoskit_db::DbError::Conflict { .. }) => return Err(rpc::Error::Contended),
			Err(error) => return Err(error.into()),
		}

		report.imported += 1;
		persons.insert(person);
	}

	// Someone push can reach is someone the service knows.
	if !persons.is_empty() {
		super::register_persons(tx, &persons).await?;
	}

	Ok(report)
}

/// The APNs environment `device` reported, as the column stores it.
fn stored_environment(device: &DeviceDetails) -> Option<String> {
	device
		.apns_environment
		.map(|environment| environment.as_str().to_owned())
}

/// How often an unchanged re-enrollment moves an endpoint's `seen_at`, in
/// seconds: an hour, so "most recently seen" is right to within one and a
/// browser's launch-time renewal is usually no write at all.
const SEEN_EVERY_SECS: i64 = 60 * 60;

/// Whether `endpoint` was last seen long enough ago for an unchanged
/// re-enrollment to say so again.
fn seen_long_ago(endpoint: &PushEndpoint) -> bool {
	endpoint
		.seen_at
		.is_none_or(|seen_at| (leptoskit_db::now() - seen_at).num_seconds() >= SEEN_EVERY_SECS)
}

/// Checks every entry into the live set inside the serializable registration
/// transaction.
async fn check_capacity(tx: &leptoskit_db::Tx, me: Uuid) -> rpc::Result<()> {
	let held = PushEndpoint::query()
		.filter(|endpoint| endpoint.person.eq(me) & endpoint.invalidated_at.is_null())
		.count(tx)
		.await?;

	if held >= u64::try_from(crate::push::MAX_ENDPOINTS).unwrap_or(u64::MAX) {
		return Err(rpc::Error::invalid(format!(
			"a person holds at most {} live endpoints; revoke one first",
			crate::push::MAX_ENDPOINTS
		)));
	}

	Ok(())
}

/// Keeps a person's endpoint rows bounded: at most
/// [`MAX_ENDPOINTS`](crate::push::MAX_ENDPOINTS) live ones, which
/// [`check_capacity`] enforces, plus
/// [`MAX_ENDPOINT_HISTORY`](crate::push::MAX_ENDPOINT_HISTORY) invalidated
/// ones. Past that the oldest invalidated rows are revoked, so enrolling
/// credentials a provider keeps invalidating can't grow the table: the
/// list stays one page, and a destination nobody could use frees itself.
async fn trim_history(tx: &leptoskit_db::Tx, me: Uuid) -> rpc::Result<()> {
	let invalidated = PushEndpoint::query()
		.filter(|endpoint| endpoint.person.eq(me) & endpoint.invalidated_at.is_not_null())
		.order_by(|endpoint| endpoint.id.asc())
		.all(tx)
		.await?;
	let excess = invalidated
		.len()
		.saturating_sub(crate::push::MAX_ENDPOINT_HISTORY);

	for endpoint in invalidated.into_iter().take(excess) {
		revoke(tx, me, endpoint.id).await?;
	}

	Ok(())
}

/// Whether `endpoint` is one of `me`'s, live or invalidated: the one
/// primary-key read behind
/// [`push_endpoint_standing`](crate::push::push_endpoint_standing) and
/// every renewal.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn standing(
	db: &impl leptoskit_db::Read,
	me: Uuid,
	endpoint: Uuid,
) -> rpc::Result<bool> {
	Ok(PushEndpoint::query()
		.filter(|row| row.id.eq(endpoint) & row.person.eq(me))
		.exists(db)
		.await?)
}

/// Revokes every endpoint `person` holds, live or invalidated, and skips
/// their queued deliveries as revoked: what
/// [`revoke_person_endpoints`](crate::push::revoke_person_endpoints) and
/// an erasure do. Bounded by the enrollment caps, so it is one short
/// write. Answers how many endpoints it removed.
///
/// # Errors
///
/// Whatever a read or a write fails with.
pub(crate) async fn revoke_all(tx: &leptoskit_db::Tx, person: Uuid) -> rpc::Result<u32> {
	let held = PushEndpoint::query()
		.filter(|endpoint| endpoint.person.eq(person))
		.all(tx)
		.await?;
	let mut revoked = 0;

	for endpoint in held {
		revoke(tx, person, endpoint.id).await?;
		revoked += 1;
	}

	Ok(revoked)
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

/// The person's endpoints, oldest first, one bounded page at a time,
/// invalidated ones included with their state: a device list that hides
/// them can't explain them, and a bounded page can't grow with them.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn list(
	db: &impl leptoskit_db::Read,
	me: Uuid,
	page: &rpc::PageArgs,
) -> rpc::Result<rpc::Page<EndpointSummary>> {
	let most = u32::try_from(crate::push::MAX_ENDPOINTS + crate::push::MAX_ENDPOINT_HISTORY)
		.unwrap_or(u32::MAX);
	let mut page = page.clone();
	page.limit = page.limit.clamp(1, most);

	let page_of_rows = super::paginate_bounded(
		db,
		PushEndpoint::query().filter(|endpoint| endpoint.person.eq(me)),
		&page,
		most,
		|endpoint| endpoint.id.asc(),
		|endpoint| endpoint.id,
	)
	.await?;

	Ok(rpc::Page {
		items: page_of_rows.items.iter().map(summary).collect(),
		end: page_of_rows.end,
		has_more: page_of_rows.has_more,
	})
}

/// The row as its owner sees it.
fn summary(endpoint: &PushEndpoint) -> EndpointSummary {
	EndpointSummary {
		id: endpoint.id,
		platform: PushPlatform::from_str_value(&endpoint.platform).unwrap_or(PushPlatform::Apns),
		created_at: endpoint.created_at,
		invalidated_at: endpoint.invalidated_at,
		device: endpoint.device_details(),
		imported: endpoint.imported_at.is_some(),
	}
}

/// The endpoint as a condition's `device.` predicates read it: its
/// platform and the details it last enrolled with, borrowed from the row.
pub(crate) fn device_of(endpoint: &PushEndpoint) -> Device<'_> {
	Device {
		platform: PushPlatform::from_str_value(&endpoint.platform).unwrap_or(PushPlatform::Apns),
		locale: endpoint.locale.as_deref(),
		time_zone: endpoint.time_zone.as_deref(),
		app_version: endpoint.app_version.as_deref(),
	}
}

/// The live — not invalidated — endpoints of `people`, by person, in one
/// read: what a fanout batch judges a device condition against and fans
/// its deliveries out to.
///
/// # Errors
///
/// Whatever the read fails with.
pub(crate) async fn live_endpoints(
	db: &impl leptoskit_db::Read,
	people: impl IntoIterator<Item = Uuid>,
) -> rpc::Result<HashMap<Uuid, Vec<PushEndpoint>>> {
	let endpoints = PushEndpoint::query()
		.filter(|endpoint| endpoint.person.in_list(people) & endpoint.invalidated_at.is_null())
		.all(db)
		.await?;
	let mut by_person: HashMap<Uuid, Vec<PushEndpoint>> = HashMap::new();

	for endpoint in endpoints {
		by_person.entry(endpoint.person).or_default().push(endpoint);
	}

	Ok(by_person)
}

/// Which of a person's endpoints a send's deliveries go to, when its
/// condition asks about devices: the ones the condition holds for, judged
/// with each endpoint's own platform and details — as they stand when the
/// deliveries are written — and the person's topics and attributes as the
/// batch read them. A send whose condition never mentions `device.` has
/// no filter, and every live endpoint gets its delivery.
pub(crate) struct DeviceFilter<'a> {
	/// The send's condition, which mentions `device.`.
	pub(crate) condition: &'a Condition,
	/// What the batch read about its people.
	pub(crate) facts: &'a Facts,
}

impl DeviceFilter<'_> {
	/// Whether `endpoint`, one of `person`'s, gets a delivery.
	fn admits(&self, person: Uuid, endpoint: &PushEndpoint) -> bool {
		self.facts
			.holds(self.condition, person, Some(device_of(endpoint)))
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

	// Its queued deliveries never attempt again: the endpoint is gone,
	// and the attempt's liveness check would only say so later. The
	// write touches only rows still in the queue, so a delivery that
	// ended meanwhile — accepted, rejected, or skipped by its own
	// attempt — keeps what it became.
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
	// A recipients send has no condition: every live endpoint of every
	// recipient gets its delivery.
	let enqueued = deliveries_in_tx(cx.tx(), send_key, fanout, None).await?;

	if enqueued > 0 {
		cx.enqueue(PushSend {
			send_key: send_key.to_owned(),
			round: 0,
		})
		.at(first_round(spec))
		.key(super::job_key("push", send_key, 0))
		.await?;
	}

	Ok(())
}

/// Writes the deliveries themselves, in whatever transaction the caller
/// holds: the half of [`schedule`] a fanout job step reuses, so a batch's
/// rows and its deliveries commit together. This is where a send's
/// deliveries are decided, so a condition that asks about devices is
/// applied here, through `filter`, against the endpoints as this
/// transaction reads them: only the endpoints the condition holds for get
/// one. With no filter, every live endpoint does.
///
/// # Errors
///
/// Whatever a read or a write fails with.
pub(crate) async fn deliveries_in_tx(
	tx: &leptoskit_db::Tx,
	send_key: &str,
	fanout: &Fanout,
	filter: Option<&DeviceFilter<'_>>,
) -> rpc::Result<usize> {
	if fanout.delivered_to.is_empty() {
		return Ok(0);
	}

	let people: std::collections::BTreeSet<Uuid> = fanout
		.delivered_to
		.iter()
		.map(|(person, ..)| *person)
		.collect();
	let by_person = live_endpoints(tx, people.iter().copied()).await?;

	if by_person.is_empty() {
		return Ok(0);
	}

	let mut enqueued = 0_usize;

	for (person, notification, waits_until) in &fanout.delivered_to {
		for endpoint in by_person.get(person).into_iter().flatten() {
			if filter.is_some_and(|filter| !filter.admits(*person, endpoint)) {
				continue;
			}

			NewPushDelivery {
				id: Uuid::now_v7(),
				notification: *notification,
				send_key: send_key.to_owned(),
				endpoint: endpoint.id,
				generation: endpoint.generation,
				person: *person,
				platform: endpoint.platform.clone(),
				state: DeliveryState::Enqueued.as_str().to_owned(),
				reason: None,
				attempts: 0,
				// A local-time row still ahead: its delivery is queued
				// for its own instant, so the rounds before then read
				// past it without an attempt, and the round after the
				// last attempt is enqueued for the earliest one waiting.
				next_attempt_at: *waits_until,
				attempted_at: None,
				finished_at: None,
			}
			.insert(tx)
			.await?;
			enqueued += 1;
		}
	}

	Ok(enqueued)
}

/// When the notification becomes visible, as the round-zero job's own
/// schedule: a scheduled send's first round waits for its time, the way
/// its wake job does.
pub(crate) fn becomes_visible(spec: &crate::SendSpec) -> leptoskit_db::Timestamp {
	let now = leptoskit_db::now();
	spec.visible_at
		.map_or(now, |visible_at| visible_at.max(now))
}

/// When a send's first push round runs. A send with one schedule waits
/// for it ([`becomes_visible`]). A local-time send has a schedule per
/// person, each on its own delivery: its first round runs now, attempts
/// the people whose time has passed, and enqueues its successor for the
/// earliest instant still waiting.
pub(crate) fn first_round(spec: &crate::SendSpec) -> leptoskit_db::Timestamp {
	if spec.local_time.is_some() {
		return leptoskit_db::now();
	}

	becomes_visible(spec)
}

/// Starts the send's push rounds after a fanout batch committed
/// deliveries: outside any transaction, at the notification's visibility,
/// under one key per send (`what` is `fanout`). One kick is enough for
/// the whole walk: a round that finds its send still fanning out keeps
/// the rounds going until the walk is over, so every later batch's
/// deliveries are found by a round that is already coming.
///
/// # Errors
///
/// Whatever the enqueue fails with.
pub(crate) async fn kick(
	db: &Db,
	send_key: &str,
	spec: &crate::SendSpec,
	what: &str,
) -> rpc::Result<()> {
	rpc::enqueue(
		db,
		PushSend {
			send_key: send_key.to_owned(),
			round: 0,
		},
	)
	.at(first_round(spec))
	.key(super::job_key("push", send_key, what))
	.await?;

	Ok(())
}

/// The fanout's last kick, as it completes: a round of its own whenever a
/// delivery of the send is still queued, so a batch whose kick was lost to
/// a crash, or a round chain that gave up, still has its deliveries
/// attempted.
///
/// # Errors
///
/// Whatever the read or the enqueue fails with.
pub(crate) async fn kick_final(db: &Db, send_key: &str, spec: &crate::SendSpec) -> rpc::Result<()> {
	if PushDelivery::query()
		.filter(|delivery| queued(delivery, send_key))
		.exists(db)
		.await?
	{
		kick(db, send_key, spec, "final").await?;
	}

	Ok(())
}

/// How many deliveries one round attempts at most before it hands the
/// rest to its successor.
const ROUND_DELIVERIES: usize = 1_000;

/// How long one round keeps starting attempts. Attempts already started
/// finish within their requests' own timeouts, so a round ends well
/// inside its job's five-minute timeout however slow a provider is.
const ROUND_BUDGET: std::time::Duration = std::time::Duration::from_secs(60);

/// While a send still fans out, how soon a round looks again for the
/// deliveries later batches add, in seconds.
const FANOUT_POLL_SECS: i64 = 5;

/// One round of the send's push job: the due deliveries, a page of
/// [`DELIVERY_CONCURRENCY`](crate::push::DELIVERY_CONCURRENCY) at a time,
/// until none is due or the round has done its share — at most
/// [`ROUND_DELIVERIES`] attempts, started within [`ROUND_BUDGET`] — and
/// then, while anything still waits, the next round.
///
/// The next round comes at once when this one stopped short; at the
/// earliest moment a waiting delivery may run again otherwise; and every
/// few seconds while the send still fans out — for at most a day — because
/// its later batches add deliveries no round has seen. Its key is that moment,
/// so two chains of rounds that meet at the same moment become one, and a
/// moment still ahead is never a key a finished round holds.
///
/// The job framework runs one round of a send at a time. The deliveries'
/// own writes are fenced regardless (see [`write_outcome`]), so a lease
/// that expires and replays a round while its first run is still in
/// flight can at worst send a delivery twice — which at-least-once
/// already allows — and can never regress a delivery that ended. One
/// delivery that fails inside the service — a read or write the database
/// refused, an attempt that panicked — is put off for a minute on its own
/// row, never failing the round or holding its peers.
///
/// # Errors
///
/// Whatever a page read, the bookkeeping reads, or the successor's enqueue
/// fails with. The job framework retries the round; the deliveries
/// already written are skipped by their state on the retry.
#[doc(hidden)]
pub async fn round(cx: &rpc::JobCx, send_key: &str, round: u32, push: &Push) -> rpc::Result<()> {
	// Pushes are stopped by whoever runs the service: every delivery of
	// this send skips, and says why in the send's status.
	if push.paused.get() {
		skip_all(cx.db(), send_key, DeliveryReason::Paused).await?;
		return Ok(());
	}

	let Some(runtime) = push.runtime.as_ref() else {
		// Nothing is configured: every delivery of this send skips,
		// loudly and visibly in the send's status.
		skip_all(cx.db(), send_key, DeliveryReason::TransportUnavailable).await?;
		return Ok(());
	};

	let db = cx.db();
	let started = std::time::Instant::now();
	// A send cancelled mid-fanout pushes nothing more, whatever its rows
	// say yet: a large one's rows are cancelled in batches after it.
	let cancelled = super::find_send(db, send_key)
		.await?
		.is_some_and(|send| send.state == crate::SendState::Cancelled.as_str());
	let mut after: Option<Uuid> = None;
	let mut attempted = 0_usize;
	let mut spent = [false; QUEUED.len()];

	let stopped_short = loop {
		if attempted >= ROUND_DELIVERIES || started.elapsed() >= ROUND_BUDGET {
			break true;
		}

		let now = leptoskit_db::now();
		let take = crate::push::DELIVERY_CONCURRENCY.min(ROUND_DELIVERIES - attempted);
		let page = due_page(db, send_key, now, after, take, &mut spent).await?;

		let Some(last) = page.last() else {
			break false;
		};

		after = Some(last.id);
		attempted += page.len();
		attempt_page(db, runtime, page, cancelled).await;
	};

	next_round(cx, send_key, round, stopped_short).await
}

/// The states a delivery waits in the queue in, in the order a round reads
/// them.
const QUEUED: [DeliveryState; 2] = [DeliveryState::Enqueued, DeliveryState::Retrying];

/// One page of a round: the send's next `take` due deliveries after
/// `after`, in id order.
///
/// Each queued state is read on its own and the two answers are merged
/// here. `(send_key, state, id)` hands one state's deliveries back in id
/// order straight from the index, starting at the cursor; asked for both
/// states at once in id order, the database instead walks the send through
/// `(send_key, id)` — every delivery that already finished included, from
/// the send's first, at the start of every round. Each state's read is its
/// own first `take` due deliveries, so the first `take` of the two merged
/// are the first `take` of them all: the page the single read answered.
///
/// `spent` remembers a state whose read came back short with all of it in
/// the page: nothing more of that state is due after the cursor, so the
/// round's later pages don't walk its waiting deliveries again to find
/// that out. What comes due in the meantime is the next round's.
///
/// # Errors
///
/// Whatever a read fails with.
async fn due_page(
	db: &impl leptoskit_db::Read,
	send_key: &str,
	now: leptoskit_db::Timestamp,
	after: Option<Uuid>,
	take: usize,
	spent: &mut [bool; QUEUED.len()],
) -> rpc::Result<Vec<PushDelivery>> {
	let mut page: Vec<PushDelivery> = Vec::new();
	let mut short = [None; QUEUED.len()];

	for (index, state) in QUEUED.into_iter().enumerate() {
		if spent[index] {
			continue;
		}

		let rows = PushDelivery::query()
			.filter(|delivery| {
				let mut condition = delivery.send_key.eq(send_key.to_owned())
					& delivery.state.eq(state.as_str())
					& (delivery.next_attempt_at.is_null() | delivery.next_attempt_at.lte(now));
				if let Some(after) = after {
					condition = condition & delivery.id.gt(after);
				}
				condition
			})
			.order_by(|delivery| delivery.id.asc())
			.limit(u64::try_from(take).unwrap_or(u64::MAX))
			.all(db)
			.await?;

		if rows.len() < take {
			// The id of the last one, or nil when there was none: what the
			// page has to reach for the state to be spent.
			short[index] = Some(rows.last().map_or_else(Uuid::nil, |row| row.id));
		}

		page.extend(rows);
	}

	page.sort_by_key(|delivery| delivery.id);
	page.truncate(take);

	let reached = page.last().map_or_else(Uuid::nil, |delivery| delivery.id);

	for (index, last) in short.into_iter().enumerate() {
		if last.is_some_and(|last| last <= reached) {
			spent[index] = true;
		}
	}

	Ok(page)
}

/// Attempts one page of deliveries at once, each owning its row, and puts
/// off any whose attempt failed inside the service.
async fn attempt_page(
	db: &Db,
	runtime: &Arc<PushRuntime>,
	page: Vec<PushDelivery>,
	cancelled: bool,
) {
	let attempts: Vec<_> = page
		.into_iter()
		.map(|row| {
			let fence = (row.id, row.attempts, row.generation);
			let attempt = tokio::spawn(try_delivery(
				db.clone(),
				Arc::clone(runtime),
				row,
				cancelled,
			));
			(fence, attempt)
		})
		.collect();

	for (fence, attempt) in attempts {
		if !matches!(attempt.await, Ok(Ok(()))) {
			defer(db, fence).await;
		}
	}
}

/// Enqueues the round after this one, when anything is left for it.
///
/// # Errors
///
/// Whatever the reads or the enqueue fail with.
async fn next_round(
	cx: &rpc::JobCx,
	send_key: &str,
	round: u32,
	stopped_short: bool,
) -> rpc::Result<()> {
	let db = cx.db();
	let now = leptoskit_db::now();
	let next = PushSend {
		send_key: send_key.to_owned(),
		round: round.saturating_add(1),
	};

	if stopped_short {
		// The rest is due already: the next round starts at once, under a
		// key of this run's own, so a retried run starts one of its own.
		rpc::enqueue(db, next)
			.at(now)
			.key(super::job_key(
				"push",
				send_key,
				format!("after:{}:{}", cx.id(), cx.attempt()),
			))
			.await?;

		return Ok(());
	}

	// Read first: a fanout that completes after this read committed its
	// last deliveries before it, so the queue reads below see them. A
	// fanout still walking a day after its send was made has died, and
	// keeps no round looking for it: what it delivered is still queued,
	// and that alone keeps the rounds coming.
	let horizon = chrono::Duration::seconds(crate::push::DELIVERY_HORIZON_SECS);
	let fanning = super::find_send(db, send_key).await?.is_some_and(|send| {
		send.state == crate::SendState::FanningOut.as_str() && now < send.created_at + horizon
	});

	let waiting = if PushDelivery::query()
		.filter(|delivery| due(delivery, send_key, now))
		.exists(db)
		.await?
	{
		Some(now)
	} else {
		// NULL orders first on SQLite and last on Postgres; the due read
		// above took every row without a time, so this one reads only
		// rows with one, and only the earliest.
		PushDelivery::query()
			.filter(|delivery| queued(delivery, send_key) & delivery.next_attempt_at.is_not_null())
			.order_by(|delivery| delivery.next_attempt_at.asc())
			.first(db)
			.await?
			.and_then(|delivery| delivery.next_attempt_at)
	};
	let poll = now + chrono::Duration::seconds(FANOUT_POLL_SECS);
	let at = match (waiting, fanning) {
		(Some(at), true) => Some(at.min(poll)),
		(None, true) => Some(poll),
		(waiting, false) => waiting,
	};

	if let Some(at) = at {
		// A whole second at least one past both `at` and now: a key that
		// names a moment still ahead can't belong to a round that already
		// ran, even across a little clock skew between workers.
		let second = at.max(now).timestamp().saturating_add(2);
		let at = chrono::DateTime::from_timestamp(second, 0).unwrap_or(at);

		rpc::enqueue(db, next)
			.at(at)
			.key(super::job_key("push", send_key, format!("at:{second}")))
			.await?;
	}

	Ok(())
}

/// A delivery of `send_key` still in the queue.
fn queued(
	delivery: &<PushDelivery as leptoskit_db::Table>::Columns,
	send_key: &str,
) -> leptoskit_db::Expr {
	delivery.send_key.eq(send_key.to_owned())
		& delivery.state.in_list([
			DeliveryState::Enqueued.as_str(),
			DeliveryState::Retrying.as_str(),
		])
}

/// A delivery of `send_key` in the queue whose time has come at `now`.
fn due(
	delivery: &<PushDelivery as leptoskit_db::Table>::Columns,
	send_key: &str,
	now: leptoskit_db::Timestamp,
) -> leptoskit_db::Expr {
	queued(delivery, send_key)
		& (delivery.next_attempt_at.is_null() | delivery.next_attempt_at.lte(now))
}

/// Puts a delivery off for a minute after its attempt failed inside the
/// service, without spending an attempt: the outcome it records, so the
/// next round tries it again rather than this one failing for it. Fenced
/// like every outcome, on the attempts and generation it was read with,
/// and best effort: when even this write fails, the row stays due and the
/// next round finds it.
async fn defer(db: &Db, (id, attempts, generation): (Uuid, i64, i64)) {
	let later = leptoskit_db::now() + chrono::Duration::seconds(STALLED_RETRY_SECS);
	let _ = PushDelivery::update()
		.filter(|row| {
			row.id.eq(id)
				& row.state.in_list([
					DeliveryState::Enqueued.as_str(),
					DeliveryState::Retrying.as_str(),
				])
				& row.attempts.eq(attempts)
				& row.generation.eq(generation)
		})
		.set(|row| row.next_attempt_at, Some(later))
		.run(db)
		.await;
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
/// fenced write. `cancelled` says the send itself was cancelled.
///
/// # Errors
///
/// A read or a write that fails: the round puts this delivery off and
/// goes on with the rest.
async fn try_delivery(
	db: Db,
	runtime: Arc<PushRuntime>,
	delivery: PushDelivery,
	cancelled: bool,
) -> rpc::Result<()> {
	let outcome = attempt(&db, &runtime, &delivery, cancelled).await?;
	write_outcome(&db, &delivery, outcome).await
}

/// The attempt, one delivery's whole story.
///
/// # Errors
///
/// A read that fails: the round puts this delivery off and goes on with
/// the rest.
async fn attempt(
	db: &Db,
	runtime: &PushRuntime,
	delivery: &PushDelivery,
	cancelled: bool,
) -> rpc::Result<Outcome> {
	let now = leptoskit_db::now();

	// The endpoint, live: revoked rows are deleted, invalidated rows
	// stay.
	let endpoint = match PushEndpoint::query()
		.filter(|endpoint| endpoint.id.eq(delivery.endpoint))
		.one(db)
		.await
	{
		Ok(endpoint) if endpoint.generation != delivery.generation => {
			return Ok(Outcome::WaitUntil(now));
		}
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

	if endpoint.person != delivery.person || notification.person != delivery.person {
		return Ok(Outcome::Ended {
			reason: DeliveryReason::EndpointUnavailable,
			invalidate: false,
		});
	}

	// The schedule, at the moment of the attempt: not yet visible waits;
	// expired, cancelled, and opted-out end here, and the inbox — whose
	// visibility rules are the same — never shows what the push didn't.
	if notification.visible_at > now {
		return Ok(Outcome::WaitUntil(notification.visible_at));
	}

	if cancelled || notification.cancelled_at.is_some() {
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

	// The push's own lifetime, when the send set one: the providers hold
	// it no longer than this, and neither does the queue. A zero lifetime
	// is "now or never" at the providers and one attempt here, whenever
	// the queue reaches it; it is never retried.
	let push_ttl = push_ttl_of(&notification);
	let push_expiry =
		transport::push_expiry(notification.visible_at, notification.expires_at, push_ttl);
	let only_attempt = push_ttl == Some(0) && delivery.attempts == 0;

	if !only_attempt && push_expiry.is_some_and(|push_expiry| push_expiry <= now) {
		return Ok(Outcome::Ended {
			reason: DeliveryReason::Expired,
			invalidate: false,
		});
	}

	let window = if only_attempt {
		notification.expires_at
	} else {
		push_expiry
	};

	if let Err(reason) = delivery_deadline(notification.visible_at, window).and_then(|deadline| {
		if deadline <= now {
			Err(DeliveryReason::HorizonExceeded)
		} else {
			Ok(deadline)
		}
	}) {
		return Ok(Outcome::Ended {
			reason,
			invalidate: false,
		});
	}

	if delivery.attempts >= i64::from(crate::push::MAX_DELIVERY_ATTEMPTS) {
		return Ok(Outcome::Ended {
			reason: DeliveryReason::AttemptsExhausted,
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

	// The send's options as the row stored them, and the badge resolved
	// to this recipient's live count at the moment of the push: two
	// recipients of one send badge their own unread numbers.
	let options: crate::MessageOptions = match notification.options.as_deref() {
		None => crate::MessageOptions::default(),
		Some(stored) => {
			let Ok(options) = serde_json::from_str(stored) else {
				// Options this build can't read were written by another —
				// a newer one, before a rollback. Pushing without them
				// would push something else, so the delivery waits at the
				// operator's pace, as a configuration stall, for a build
				// that reads them, until its window closes.
				return Ok(Outcome::Stalled { after: None });
			};
			options
		}
	};
	let badge = match options.badge {
		Some(crate::options::BadgeSetting::Unread) => {
			u32::try_from(unread_for(db, delivery.person).await?).ok()
		}
		Some(crate::options::BadgeSetting::Set(set)) => Some(set),
		None => None,
	};

	let message = Message {
		id: notification.id,
		title: notification.title.clone(),
		body: notification.body.clone(),
		data: notification.data.clone(),
		expires_at: push_expiry,
		options,
		badge,
	};

	Ok(transport.attempt(target_of(&endpoint), &message).await)
}

/// Where one attempt goes: the endpoint's credential, and the APNs
/// environment its device reported.
fn target_of(endpoint: &PushEndpoint) -> transport::Target<'_> {
	transport::Target {
		credential: &endpoint.credential,
		apns_environment: endpoint.apns_environment(),
	}
}

/// The push lifetime the notification's send set, read from the options
/// its row stores. Options this build cannot read name none here; the
/// attempt itself stalls on them.
fn push_ttl_of(notification: &crate::Notification) -> Option<u32> {
	notification
		.options
		.as_deref()
		.and_then(|stored| serde_json::from_str::<crate::MessageOptions>(stored).ok())
		.and_then(|options| options.push_ttl)
}

/// The delivery state an ended reason counts as: the notification's own
/// state or the endpoint's skipped it, and a provider-shaped answer
/// rejected it. A local decision — one that never reached a provider —
/// spends no attempt and stamps no attempt time.
pub(crate) fn state_of(reason: DeliveryReason) -> DeliveryState {
	match reason {
		DeliveryReason::Expired
		| DeliveryReason::Cancelled
		| DeliveryReason::OptedOut
		| DeliveryReason::EndpointRevoked
		| DeliveryReason::EndpointUnavailable
		| DeliveryReason::TransportUnavailable
		| DeliveryReason::Paused
		| DeliveryReason::SubscriptionUnusable
		| DeliveryReason::PlatformUnsupported => DeliveryState::Skipped,
		DeliveryReason::Oversized
		| DeliveryReason::Unregistered
		| DeliveryReason::ProviderRejected
		| DeliveryReason::AttemptsExhausted
		| DeliveryReason::HorizonExceeded
		| DeliveryReason::RetryOverflow
		| DeliveryReason::OperatorConfiguration => DeliveryState::Rejected,
	}
}

/// Writes one attempt's outcome, in its own short write: no transaction
/// is held across the network, so this is the first write since the
/// read — and the row may have moved under it. Every write is fenced on
/// the row still being in the queue with the attempt count it was read
/// with: a revoke that skipped the delivery, an invalidation that ended
/// its sibling, or a replayed round that already wrote — none of them
/// can be overwritten, and an outcome that arrives that late writes
/// nothing. Terminal states never regress.
///
/// # Errors
///
/// A write that fails: the round stops and the framework retries it,
/// skipping what already wrote by its state.
async fn write_outcome(db: &Db, delivery: &PushDelivery, outcome: Outcome) -> rpc::Result<()> {
	// Rotation takes this same endpoint lock. The revision check and all
	// outcome writes commit together, with no transaction spanning HTTP.
	let tx = db.begin().await?;
	let matched = PushEndpoint::update()
		.filter(|row| row.id.eq(delivery.endpoint) & row.generation.eq(delivery.generation))
		.set(|row| row.generation, delivery.generation)
		.run(&tx)
		.await?;
	if matched != 0 {
		apply_outcome(&tx, delivery, outcome).await?;
	}
	tx.commit().await?;
	Ok(())
}

/// Writes `delivery`'s row in the acceptance ledger a metering host counts,
/// in the write that accepts it, so an accepted delivery is never missing
/// from it.
async fn record_acceptance(
	tx: &leptoskit_db::Tx,
	delivery: &PushDelivery,
	now: leptoskit_db::Timestamp,
) -> rpc::Result<()> {
	crate::push::NewPushAcceptance {
		id: Uuid::now_v7(),
		delivery: delivery.id,
		platform: delivery.platform.clone(),
		accepted_at: now,
	}
	.insert(tx)
	.await?;
	Ok(())
}

/// Applies an outcome while the endpoint's exact credential revision is locked.
async fn apply_outcome(
	tx: &leptoskit_db::Tx,
	delivery: &PushDelivery,
	outcome: Outcome,
) -> rpc::Result<()> {
	let now = leptoskit_db::now();
	let attempted = outcome.network_attempted();
	let attempted_at = attempted.then_some(now).or(delivery.attempted_at);
	let attempts = delivery.attempts.saturating_add(i64::from(attempted));
	let still_queued = |row: &<PushDelivery as leptoskit_db::Table>::Columns| {
		row.id.eq(delivery.id)
			& row.state.in_list([
				DeliveryState::Enqueued.as_str(),
				DeliveryState::Retrying.as_str(),
			])
			& row.attempts.eq(delivery.attempts)
			& row.generation.eq(delivery.generation)
	};

	if PushDelivery::query()
		.filter(still_queued)
		.first(tx)
		.await?
		.is_none()
	{
		return Ok(());
	}

	match outcome {
		Outcome::Accepted => {
			PushDelivery::update()
				.filter(still_queued)
				.set(|row| row.state, DeliveryState::Accepted.as_str())
				.set(|row| row.reason, None)
				.set(|row| row.attempts, attempts)
				.set(|row| row.attempted_at, Some(now))
				.set(|row| row.finished_at, Some(now))
				.set(|row| row.next_attempt_at, None)
				.run(tx)
				.await?;
			record_acceptance(tx, delivery, now).await?;
		}
		Outcome::Transient { after }
		| Outcome::Stalled { after }
		| Outcome::AuthRejected { after } => {
			let stalled = matches!(
				outcome,
				Outcome::Stalled { .. } | Outcome::AuthRejected { .. }
			);
			let notification = match crate::Notification::query()
				.filter(|row| row.id.eq(delivery.notification))
				.one(tx)
				.await
			{
				Ok(notification) => notification,
				Err(leptoskit_db::DbError::NotFound { .. }) => {
					return end_gone(tx, delivery, attempts, attempted_at).await;
				}
				Err(error) => return Err(error.into()),
			};
			let delay = retry_delay(delivery, outcome, attempts, after);
			let plan = if attempts >= i64::from(crate::push::MAX_DELIVERY_ATTEMPTS) {
				Err(DeliveryReason::AttemptsExhausted)
			} else {
				retry_time(
					now,
					notification.visible_at,
					transport::push_expiry(
						notification.visible_at,
						notification.expires_at,
						push_ttl_of(&notification),
					),
					delay,
				)
			};
			let (state, reason, next, finished) = match plan {
				Ok(at) => {
					(
						DeliveryState::Retrying,
						stalled.then_some(DeliveryReason::OperatorConfiguration),
						Some(at),
						None,
					)
				}
				Err(reason) => (DeliveryState::Rejected, Some(reason), None, Some(now)),
			};
			PushDelivery::update()
				.filter(still_queued)
				.set(|row| row.state, state.as_str())
				.set(
					|row| row.reason,
					reason.map(|reason| reason.as_str().to_owned()),
				)
				.set(|row| row.attempts, attempts)
				.set(|row| row.attempted_at, attempted_at)
				.set(|row| row.next_attempt_at, next)
				.set(|row| row.finished_at, finished)
				.run(tx)
				.await?;
		}
		Outcome::WaitUntil(at) => {
			PushDelivery::update()
				.filter(still_queued)
				.set(|row| row.next_attempt_at, Some(at))
				.run(tx)
				.await?;
		}
		Outcome::Ended { reason, invalidate } => {
			// Why it ended decides what it counts as: the notification's
			// state or the endpoint's skipped it, the provider's answer
			// rejected it. A local decision — one that never reached a
			// provider — spends no attempt and stamps no attempt time.
			let state = state_of(reason).as_str();

			if invalidate {
				// The credential is dead at the provider: the endpoint
				// stops taking deliveries, and its other queued ones end
				// now instead of failing the same way one by one. One
				// transaction, so the endpoint and its siblings move
				// together — and the siblings' own writes stay fenced on
				// still being in the queue, so a sibling whose attempt
				// answered first keeps its answer.

				PushEndpoint::update()
					.filter(|endpoint| {
						endpoint.id.eq(delivery.endpoint)
							& endpoint.generation.eq(delivery.generation)
							& endpoint.invalidated_at.is_null()
					})
					.set(|endpoint| endpoint.invalidated_at, Some(now))
					.run(tx)
					.await?;

				PushDelivery::update()
					.filter(|row| {
						row.id.ne(delivery.id)
							& row.endpoint.eq(delivery.endpoint)
							& row.generation.eq(delivery.generation)
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
					.run(tx)
					.await?;
			}

			PushDelivery::update()
				.filter(still_queued)
				.set(|row| row.state, state)
				.set(|row| row.reason, reason.as_str())
				.set(|row| row.attempts, attempts)
				.set(|row| row.attempted_at, attempted_at)
				.set(|row| row.finished_at, Some(now))
				.set(|row| row.next_attempt_at, None)
				.run(tx)
				.await?;
		}
	}

	Ok(())
}

/// Ends a delivery whose inbox row went between its attempt and this
/// write, which nothing the service does explains: the way an attempt that
/// finds the row gone ends it, as cancelled, with the request it made
/// counted. Fenced like every outcome.
///
/// # Errors
///
/// Whatever the write fails with.
async fn end_gone(
	tx: &leptoskit_db::Tx,
	delivery: &PushDelivery,
	attempts: i64,
	attempted_at: Option<leptoskit_db::Timestamp>,
) -> rpc::Result<()> {
	PushDelivery::update()
		.filter(|row| {
			row.id.eq(delivery.id)
				& row.state.in_list([
					DeliveryState::Enqueued.as_str(),
					DeliveryState::Retrying.as_str(),
				])
				& row.attempts.eq(delivery.attempts)
				& row.generation.eq(delivery.generation)
		})
		.set(|row| row.state, DeliveryState::Skipped.as_str())
		.set(|row| row.reason, DeliveryReason::Cancelled.as_str())
		.set(|row| row.attempts, attempts)
		.set(|row| row.attempted_at, attempted_at)
		.set(|row| row.finished_at, Some(leptoskit_db::now()))
		.set(|row| row.next_attempt_at, None)
		.run(tx)
		.await?;

	Ok(())
}

/// The wait before a delivery tries again: an operator-side stall waits a
/// minute; anything else waits the engine's backoff — one minute, doubled
/// for every attempt spent, so the eight attempts span about two hours and
/// an hour's outage loses nothing — plus up to a quarter more of jitter
/// that differs from delivery to delivery and attempt to attempt. A
/// provider's own `Retry-After` is honored whenever it is longer, and
/// never shortened.
fn retry_delay(
	delivery: &PushDelivery,
	outcome: Outcome,
	attempts: i64,
	after: Option<std::time::Duration>,
) -> std::time::Duration {
	let wait = if matches!(outcome, Outcome::Stalled { .. }) {
		std::time::Duration::from_secs(STALLED_RETRY_SECS.unsigned_abs())
	} else {
		let backoff = retry_backoff(attempts);
		backoff + retry_jitter(delivery.id, attempts, backoff / 4)
	};

	after.map_or(wait, |after| after.max(wait))
}

/// The earlier of notification expiry and the finite delivery horizon.
fn delivery_deadline(
	visible: leptoskit_db::Timestamp,
	expiry: Option<leptoskit_db::Timestamp>,
) -> Result<leptoskit_db::Timestamp, DeliveryReason> {
	let horizon = visible
		.checked_add_signed(chrono::Duration::seconds(
			crate::push::DELIVERY_HORIZON_SECS,
		))
		.ok_or(DeliveryReason::RetryOverflow)?;
	Ok(expiry.map_or(horizon, |expiry| expiry.min(horizon)))
}

/// Schedules a wake at the deadline instead of retrying earlier than requested.
fn retry_time(
	now: leptoskit_db::Timestamp,
	visible: leptoskit_db::Timestamp,
	expiry: Option<leptoskit_db::Timestamp>,
	delay: std::time::Duration,
) -> Result<leptoskit_db::Timestamp, DeliveryReason> {
	let deadline = delivery_deadline(visible, expiry)?;
	if deadline <= now {
		return Err(if expiry.is_some_and(|expiry| expiry <= now) {
			DeliveryReason::Expired
		} else {
			DeliveryReason::HorizonExceeded
		});
	}
	let delay = chrono::Duration::from_std(delay).map_err(|_| DeliveryReason::RetryOverflow)?;
	let at = now
		.checked_add_signed(delay)
		.ok_or(DeliveryReason::RetryOverflow)?;
	Ok(at.min(deadline))
}

/// The first retry's wait, before jitter: what doubles from there.
const RETRY_BACKOFF_FIRST: std::time::Duration = std::time::Duration::from_secs(60);

/// The longest wait between two attempts, before jitter. Eight attempts
/// never reach it; it keeps the arithmetic bounded.
const RETRY_BACKOFF_MOST: std::time::Duration = std::time::Duration::from_hours(6);

/// The engine's backoff after `attempts` spent: one minute after the
/// first, doubling after each one since.
fn retry_backoff(attempts: i64) -> std::time::Duration {
	let doublings = u32::try_from(attempts.saturating_sub(1).max(0))
		.unwrap_or(u32::MAX)
		.min(16);

	RETRY_BACKOFF_FIRST
		.saturating_mul(1_u32 << doublings)
		.min(RETRY_BACKOFF_MOST)
}

/// Up to `most` of jitter, from the delivery's id and the attempts it has
/// spent: stable for one retry, different from delivery to delivery — the
/// hash covers the id's random bits, not just its timestamp — so a burst
/// of failures doesn't come back as a burst.
fn retry_jitter(id: Uuid, attempts: i64, most: std::time::Duration) -> std::time::Duration {
	let digest = Sha256::new()
		.chain_update(id.as_bytes())
		.chain_update(attempts.to_le_bytes())
		.finalize();
	let mut seed = [0_u8; 8];
	seed.copy_from_slice(&digest[..8]);
	let millis = u64::try_from(most.as_millis()).unwrap_or(u64::MAX);

	std::time::Duration::from_millis(u64::from_le_bytes(seed) % millis.saturating_add(1))
}

#[cfg(test)]
mod retry_tests {
	use std::time::Duration;

	use super::*;

	/// A queued delivery to `platform`, under an id minted now.
	fn delivery(platform: &str) -> PushDelivery {
		PushDelivery {
			id: Uuid::now_v7(),
			notification: Uuid::nil(),
			send_key: "retried".to_owned(),
			endpoint: Uuid::nil(),
			generation: 1,
			person: Uuid::nil(),
			platform: platform.to_owned(),
			state: DeliveryState::Retrying.as_str().to_owned(),
			reason: None,
			attempts: 0,
			next_attempt_at: None,
			attempted_at: None,
			finished_at: None,
			created_at: leptoskit_db::now(),
		}
	}

	/// Seven transient failures in a row wait long enough between them that
	/// the eighth attempt comes after an hour's outage, and all of it ends
	/// well inside the delivery horizon, on every platform.
	#[test]
	fn retries_outlast_an_hours_outage_inside_the_horizon() {
		for platform in ["apns", "fcm", "web_push"] {
			let delivery = delivery(platform);
			let waited: Duration = (1..i64::from(crate::push::MAX_DELIVERY_ATTEMPTS))
				.map(|attempts| {
					retry_delay(
						&delivery,
						Outcome::Transient { after: None },
						attempts,
						None,
					)
				})
				.sum();

			assert!(
				waited > Duration::from_hours(1),
				"{platform}: the last attempt comes {waited:?} after the first"
			);
			assert!(
				waited < Duration::from_secs(crate::push::DELIVERY_HORIZON_SECS.unsigned_abs()),
				"{platform}: and inside the horizon"
			);
		}
	}

	/// Deliveries minted in the same moment — whose ids share their
	/// timestamp bytes — wait different lengths, so a burst of failures
	/// doesn't come back as a burst.
	#[test]
	fn jitter_differs_from_delivery_to_delivery() {
		let waits: std::collections::BTreeSet<Duration> = (0..64)
			.map(|_| {
				retry_delay(
					&delivery("apns"),
					Outcome::Transient { after: None },
					3,
					None,
				)
			})
			.collect();

		assert!(
			waits.len() > 32,
			"64 deliveries waited only {} different lengths",
			waits.len()
		);
	}

	/// A provider's `Retry-After` lengthens a wait and never shortens one,
	/// and an operator-side stall waits a minute.
	#[test]
	fn a_provider_delay_lengthens_a_wait_and_never_shortens_it() {
		let delivery = delivery("apns");
		let backoff = retry_delay(&delivery, Outcome::Transient { after: None }, 1, None);
		let short = Duration::from_secs(30);
		let long = Duration::from_hours(2);

		assert!(backoff >= Duration::from_mins(1), "{backoff:?}");
		assert_eq!(
			retry_delay(
				&delivery,
				Outcome::Transient { after: Some(short) },
				1,
				Some(short)
			),
			backoff,
			"a shorter Retry-After waits the backoff"
		);
		assert_eq!(
			retry_delay(
				&delivery,
				Outcome::Transient { after: Some(long) },
				1,
				Some(long)
			),
			long,
			"a longer one is honored"
		);
		assert_eq!(
			retry_delay(&delivery, Outcome::Stalled { after: None }, 0, None),
			Duration::from_mins(1)
		);
	}

	#[test]
	fn explicit_clock_bounds_all_waits_including_broken_auth_without_expiry() {
		let now = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
		let hour = chrono::Duration::hours(1);
		let delay = Duration::from_hours(7);
		assert_eq!(retry_time(now, now, None, delay), Ok(now + hour * 7));
		assert_eq!(
			retry_time(now, now, Some(now + hour), delay),
			Ok(now + hour)
		);
		assert_eq!(
			retry_time(now, now, None, Duration::from_hours(48)),
			Ok(now + hour * 24)
		);
		assert_eq!(
			retry_time(now + hour * 24, now, None, delay),
			Err(DeliveryReason::HorizonExceeded)
		);
		assert_eq!(
			retry_time(now, now, Some(now), delay),
			Err(DeliveryReason::Expired)
		);
		assert_eq!(
			retry_time(now, now, None, Duration::MAX),
			Err(DeliveryReason::RetryOverflow)
		);
		assert_eq!(
			delivery_deadline(chrono::DateTime::<chrono::Utc>::MAX_UTC, None),
			Err(DeliveryReason::RetryOverflow)
		);
	}
}
