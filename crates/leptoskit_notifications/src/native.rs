//! The native half of push enrollment: the device token the OS issues — an
//! APNs device token on iOS and macOS, an FCM registration token on Android —
//! acquired through the notifications plugin and enrolled under the
//! signed-in person. It is the native counterpart of [`crate::browser`], with
//! the same shape and the same boundaries.
//!
//! What the module does and does not touch: the token comes from the
//! generated app delegate via the plugin's broker — never from storage this
//! module keeps — and the enrollment call is the app's own RPC client, so
//! the endpoint belongs to the forwarded signed-in person. No service
//! token, no session copy, and no identity live here or anywhere host-side.
//! The person's session lifecycle stays the app's: it persists the opaque
//! [`Enrollment`] handle beside its opt-in flag — never the token, which is
//! the OS's to rotate — and every call here runs under whoever is signed in
//! right now.
//!
//! The whole workflow, in the order an app wires it:
//!
//! 1. Configure the build: the platform's push opt-in — `[native.ios]
//!    aps-environment` or `[native.macos] aps-environment`, set to the
//!    environment the app's signing and the service's
//!    `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT` name, or `[native.android]
//!    push` — the `notifications` plugin in `[native] plugins`, and the
//!    `native` feature on the crate. Without the entitlement the OS refuses
//!    registration with a provisioning failure; nothing here can work around
//!    it.
//! 2. On an **explicit user gesture** — a "turn on notifications" switch, never
//!    a mount or a render — request notification permission through the
//!    plugin's existing guest API
//!    ([`leptoskit_plugin_notifications::guest::Notifications::request_permission`]).
//!    Alerts are what the permission gates; registration itself needs no
//!    dialog, but an app that enrolls without asking has quietly promised
//!    banners the person never opted into.
//! 3. Still in the gesture, call [`enroll`]: it asks the OS to register, waits
//!    a bounded time for the answer, and registers the token as the person's
//!    typed [`crate::push::PushCredential`] — `Apns` from an iOS or macOS host,
//!    `Fcm` from an Android one. The platform is a transport choice and nothing
//!    more: the token kind the host answers names the provider kind, and the
//!    workflow around it is identical. The answer is an opaque [`Enrollment`]
//!    id: keep it, with the opt-in, beside the person's session.
//! 4. On **launch**, once the person is signed in, call [`reconcile`] with that
//!    saved enrollment — and only with one: it is the record that this person
//!    opted in on this device, and without it nothing registers, so a disabled
//!    device is never silently re-enabled. With it, the module asks the OS to
//!    register again — registration prompts nothing on any platform; iOS
//!    expects it every launch, macOS re-reads the OS's answer the same way, and
//!    Android refreshes its token the same way — waits the bounded answer, and
//!    brings the service up to date with whatever the OS now issues and
//!    whatever the device now says about itself. When neither changed since the
//!    enrollment last sent them, nothing is registered: the module only asks
//!    the service, in one read, whether the enrollment still stands. The answer
//!    is an explicit [`Outcome`]: unchanged, rotated with the old handle's
//!    retirement acknowledged, rotated with a stale handle dropped rather than
//!    operated, or [`Outcome::Gone`] — the service no longer has this
//!    enrollment, because it was revoked from another device, taken over by the
//!    device's next person, or erased with the person. A launch never enrolls
//!    again in its place: clear the saved enrollment and the opt-in, and offer
//!    the switch. Persist the enrollment every other outcome carries: it
//!    remembers what was sent.
//!
//! Every enrollment carries the device's details — its preferred locale, its
//! time zone, and the app's version, read from the OS by the notifications
//! plugin ([`device_details`]) — so a send can target devices with
//! `device.` conditions. The app writes no code for them: an app update or a
//! language change reaches the service at the next launch's [`reconcile`].
//! 5. On **sign out**, while this person is still authenticated, call
//!    [`disable`]. It requires an acknowledged server revoke and replays a
//!    stable idempotency key derived from the endpoint id — the same contract
//!    as [`crate::browser::disable`] — so a retried sign-out is safe. It
//!    changes no OS state; the token stays the device's, and the next account
//!    enrolls from clean. Clear the saved enrollment and opt-in once the revoke
//!    is acknowledged. A person whose session ended without this — an expired
//!    session, a sign-out while offline — still holds the device on the service
//!    until the next person [`enroll`]s on it: presenting the same token takes
//!    the endpoint over and revokes theirs. The app's server can also release
//!    it (`revoke_person_endpoints`).
//!
//! iOS, Android, and macOS. Calls on hosts without the plugin answer
//! `Unavailable` rather than pretending.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::push::DeviceDetails;
#[cfg(target_arch = "wasm32")]
use crate::push::EndpointSummary;
use crate::push::PushCredential;
#[cfg(target_arch = "wasm32")]
use crate::push::PushEndpointStanding;
#[cfg(target_arch = "wasm32")]
use crate::push::RevokePushEndpoint;

/// The server endpoint identity needed to disable this person's enrollment.
/// The app persists this opaque handle — and the fact that it opted in —
/// beside the person's session. It never persists the device token: the
/// token is the OS's to rotate, and the service dedupes destinations, so
/// the handle is all reconciliation needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Enrollment {
	/// The endpoint's opaque id, as [`crate::push::EndpointSummary`] lists it.
	pub endpoint: uuid::Uuid,
	/// A [`digest`] of what this enrollment last sent — the token and the
	/// device's details — so [`reconcile`] can tell whether anything changed
	/// without storing either. Not the token: a 64-bit digest can't give it
	/// back. `None` for an enrollment saved before details existed; the
	/// next reconcile sends once and fills it in.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub sent: Option<u64>,
}

impl Enrollment {
	/// Whether this enrollment already carries what a launch would send
	/// now: the same token and the same details. [`reconcile`] sends
	/// nothing when it does, and brings the service up to date when it
	/// doesn't — after an app update, a language change, a move across time
	/// zones, or a rotated token.
	#[must_use]
	pub fn is_current(&self, credential: &PushCredential, details: &DeviceDetails) -> bool {
		self.sent == Some(digest(credential, details))
	}
}

/// What reconciliation compares: a 64-bit FNV-1a digest of what an
/// enrollment sends — the credential's platform and destination, and each
/// device detail, absent and present told apart. Stable across builds and
/// platforms, because the app persists it.
#[must_use]
pub fn digest(credential: &PushCredential, details: &DeviceDetails) -> u64 {
	const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
	const PRIME: u64 = 0x0000_0100_0000_01b3;

	let mut hash = OFFSET;
	let mut feed = |bytes: &[u8]| {
		for byte in bytes {
			hash ^= u64::from(*byte);
			hash = hash.wrapping_mul(PRIME);
		}
	};

	feed(credential.platform().as_str().as_bytes());
	feed(&[0]);
	// A credential that won't canonicalize can't enroll either; its raw
	// shape still digests, so it never compares equal to a real one.
	feed(
		credential
			.destination()
			.unwrap_or_else(|problem| problem)
			.as_bytes(),
	);

	for detail in [&details.locale, &details.time_zone, &details.app_version] {
		match detail {
			None => feed(&[0]),
			Some(value) => {
				feed(&[1]);
				feed(value.as_bytes());
				feed(&[0]);
			}
		}
	}

	hash
}

/// What the device says about itself, as the notifications plugin read it
/// from the OS, made ready to send: each detail
/// [sanitized](DeviceDetails::sanitized), so one the service would refuse is
/// dropped instead of costing the device its enrollment.
#[must_use]
pub fn details_from(info: &leptoskit_plugin_notifications::DeviceInfo) -> DeviceDetails {
	DeviceDetails {
		locale: info.locale.clone(),
		time_zone: info.time_zone.clone(),
		app_version: info.app_version.clone(),
	}
	.sanitized()
}

/// What this device says about itself right now — its preferred locale,
/// its time zone, and the app's version — read through the notifications
/// plugin and [sanitized](details_from). A host that predates the plugin's
/// device method, or answers nothing, answers no details: enrollment goes
/// on without them, as it always did.
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(clippy::unused_async, reason = "matches the wasm signature")
)]
pub async fn device_details() -> DeviceDetails {
	#[cfg(not(target_arch = "wasm32"))]
	return DeviceDetails::default();

	#[cfg(target_arch = "wasm32")]
	{
		let Ok(notifications) = leptoskit_plugin_notifications::guest::Notifications::new() else {
			return DeviceDetails::default();
		};

		notifications
			.device_info()
			.await
			.map(|info| details_from(&info))
			.unwrap_or_default()
	}
}

/// Why enrolling this device did not happen, and what still stands. Every
/// variant names an explicit state: nothing is half-enrolled, nothing is
/// silently retried, and the variants that carry handles say which
/// enrollment to keep retrying with.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnrollError {
	/// This host runs no native enrollment. Only an iOS, macOS, or Android
	/// guest runs it.
	#[error("native enrollment needs an iOS, macOS, or Android guest on this host")]
	Unavailable,
	/// The notifications plugin is not installed: add `"notifications"` to
	/// `[native] plugins`. Nothing was prompted and nothing registered.
	#[error("the notifications plugin is not installed: add \"notifications\" to [native] plugins")]
	NoPlugin,
	/// The bridge call itself failed — cancelled, busy, or a native
	/// failure. Nothing was registered with the service.
	#[error("registering for remote notifications failed: {0}")]
	Register(String),
	/// The OS refused registration; the reason is one of the plugin
	/// contract's stable codes — most often `aps-environment` for a missing
	/// or mismatched Apple entitlement, or `firebase-not-configured` when an
	/// Android app generates without `[native.android] push` or its
	/// `google-services.json`. Nothing was registered with the service.
	#[error("the OS refused registration: {0}")]
	Failed(String),
	/// The OS answered nothing within the host's registration bound. Any
	/// previous enrollment stands exactly as it was: a retry from a user
	/// gesture asks again, and the next launch's [`reconcile`] asks again.
	/// Silence made explicit — never a stale token dressed up as success.
	#[error("the OS has not answered registration yet")]
	Unconfirmed,
	/// The enrollment call failed: the network, or the service's own answer.
	/// The handle names the enrollment that still stands and should be
	/// retried with — `None` when this attempt already retired the previous
	/// handle (or found it stale), so the retry starts from the opt-in
	/// alone.
	#[error("enrolling failed: {1}")]
	Enroll(Option<Enrollment>, leptoskit_rpc::FnError),
	/// Revoking a handle was not acknowledged, so it still stands beside
	/// whatever this attempt enrolled. `current` is the replacement
	/// enrollment when one was made; keep it, and keep retrying the revoke
	/// on `retired` — a later [`reconcile`] retries the retirement too.
	#[error("revoking the previous enrollment failed: {cause}")]
	RetireFailed {
		/// The replacement enrollment, when the rotation got that far.
		current: Option<Enrollment>,
		/// The handle whose revoke was not acknowledged; it still stands.
		retired: Enrollment,
		/// Why the revoke failed: the network, or the service's answer.
		#[source]
		cause: leptoskit_rpc::FnError,
	},
	/// The service's ownership check failed, so ownership of the previous
	/// handle could not be verified and nothing was revoked. As
	/// [`EnrollError::RetireFailed`], both handles stand.
	#[error("could not verify the previous enrollment's owner: {cause}")]
	OwnershipUnknown {
		/// The replacement enrollment, when the rotation got that far.
		current: Option<Enrollment>,
		/// The handle left unverified and unreleased.
		retired: Enrollment,
		/// Why the check failed: the network, or the service's answer.
		#[source]
		cause: leptoskit_rpc::FnError,
	},
}

/// Why disabling the service enrollment was not confirmed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DisableError {
	/// No successful revoke acknowledgement: retry while authenticated.
	#[error("revoking the endpoint failed: {0}")]
	Revoke(leptoskit_rpc::FnError),
	/// Outside a native guest; no revoke was attempted.
	#[error("native enrollment needs an iOS, macOS, or Android guest on this host")]
	Unavailable,
}

/// The stable idempotency key disabling one enrollment replays — the same
/// contract as the browser module's `disable`, derived from the endpoint id
/// so a retried sign-out is safe across processes. [`disable`] sends it;
/// an app that revokes through its own client should reuse it, so the
/// service's receipt book sees one key per enrollment.
#[must_use]
pub fn disable_key(endpoint: uuid::Uuid) -> String {
	format!("notifications:disable:{endpoint}")
}

/// What a registration snapshot means for enrollment: the token to register
/// as the person's credential, `None` when the OS holds no answer yet, or
/// the OS's own refusal. The platform is a transport choice the host's own
/// answer names — an APNs hex token means an iOS or macOS host, an FCM
/// printable token means Android — and this module never second-guesses it.
/// The token's shape was validated at the plugin boundary; its bytes are
/// forwarded verbatim, never re-derived or stored.
pub fn decide(
	snapshot: &leptoskit_plugin_notifications::RemoteRegistration,
) -> Result<Option<PushCredential>, EnrollError> {
	use leptoskit_plugin_notifications::RemoteState;

	match snapshot.state() {
		RemoteState::None => Ok(None),
		RemoteState::Unconfirmed => Err(EnrollError::Unconfirmed),
		RemoteState::Failed(reason) => Err(EnrollError::Failed(reason.clone())),
		RemoteState::Token(token) => {
			Ok(Some(PushCredential::Apns {
				device_token: token.as_str().to_owned(),
			}))
		}
		RemoteState::FcmToken(token) => {
			Ok(Some(PushCredential::Fcm {
				registration_token: token.as_str().to_owned(),
			}))
		}
	}
}

/// What one launch reconciliation found, as an explicit transition the app
/// persists: nothing to do, the same enrollment standing, or a rotation
/// with the old handle's retirement accounted for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
	/// This device holds no enrollment to reconcile: the app never passed a
	/// previous one, so nothing registered and nothing was asked. The app's
	/// cue to offer the opt-in switch.
	Off,
	/// The OS still issues the token this enrollment was made with. When
	/// the device's details are unchanged too, nothing was sent and this is
	/// the enrollment the app passed in; when they changed — an app update,
	/// a new language — the service took the new ones under the same
	/// endpoint id, with no credential rotation, and this enrollment
	/// remembers them. Persist it either way.
	Kept(Enrollment),
	/// The OS now issues a different token. `current` is the new live
	/// enrollment — the handle to keep from now on — and `retired` says how
	/// the previous handle left: revoked under this same signed-in person,
	/// or dropped because it was never this account's to operate.
	Rotated {
		/// The new enrollment; persist this in the previous one's place.
		current: Enrollment,
		/// How the previous handle left.
		retired: Retirement,
	},
	/// The service no longer has this enrollment for the signed-in person:
	/// it was revoked from another device's list or by the app's server,
	/// taken over by the next person to enroll on this device, or erased
	/// with the person. Nothing was registered and nothing will be until
	/// the person turns notifications on again: clear the saved enrollment
	/// and the opt-in, and show the switch as off. The handle is the one
	/// that was passed in, for the app to match against what it saved.
	Gone(Enrollment),
}

/// How a rotated-out enrollment handle left, when rotation succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retirement {
	/// The service acknowledged the revoke under the same signed-in person
	/// who enrolled the replacement, with the stable [`disable_key`]. The
	/// handle is gone; drop it.
	Revoked(Enrollment),
	/// The handle stopped being this person's between the registration
	/// that renewed it and its retirement — revoked meanwhile from another
	/// device. No revoke was attempted. Drop it.
	Stale(Enrollment),
}

/// Enrolls this device for the signed-in person, from an explicit user
/// gesture: permission is the app's own prior call, so this asks the OS to
/// register, waits the host's bounded time for the answer, and registers
/// the token — APNs on an iOS or macOS host, FCM on an Android one — with
/// the device's [details](device_details). Two racing calls converge on one
/// endpoint: the service answers the same id for the same destination.
///
/// # Errors
///
/// [`EnrollError`] for each step's own failure. A silent OS is
/// [`EnrollError::Unconfirmed`], never a stale token.
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(clippy::unused_async, reason = "matches the wasm signature")
)]
pub async fn enroll() -> Result<Enrollment, EnrollError> {
	#[cfg(not(target_arch = "wasm32"))]
	return Err(EnrollError::Unavailable);

	#[cfg(target_arch = "wasm32")]
	{
		let snapshot =
			plugin_snapshot(leptoskit_plugin_notifications::guest::Notifications::register_remote)
				.await?;

		match decide(&snapshot)? {
			Some(credential) => {
				register(&credential, &device_details().await, None)
					.await
					.map_err(|cause| EnrollError::Enroll(None, cause))
			}
			None => Err(EnrollError::Unconfirmed),
		}
	}
}

/// The nonprompting half, for app launch — but only for a device that
/// previously opted in: `previous` is the enrollment the app saved beside
/// its opt-in flag when [`enroll`] succeeded, and it is the record
/// reconciliation runs on. Without it the answer is [`Outcome::Off`]:
/// nothing registers, so a person who disabled notifications is never
/// silently re-enabled, and a fresh install is never enrolled by startup
/// code. With it, the module registers with the OS again — registration
/// prompts nothing on any platform; only permission requests do, and
/// those stay the gesture's — and reconciles whatever the OS now issues,
/// and whatever the device now says about itself, under whoever is signed
/// in right now:
///
/// - The same token and the same [details](device_details) the enrollment last
///   sent: nothing is registered. One read asks the service whether the
///   enrollment still stands; it does, [`Outcome::Kept`]. When the service
///   can't be reached the enrollment is kept as it was, and the next launch
///   asks again.
/// - An enrollment the service no longer has — revoked from another device,
///   taken over by this device's next person, or erased — is [`Outcome::Gone`],
///   whatever else changed. The launch enrolls nothing in its place: a
///   server-side revoke sticks until the person's own gesture.
/// - The same token with changed details — an app update, a language change, a
///   new time zone — keeps the same endpoint id and sends the new details in
///   one idempotent call: [`Outcome::Kept`], with the new details remembered.
///   The service rotates nothing for it.
/// - A rotated token is a new destination. The module verifies the previous
///   handle is still this person's — cleanup runs only for the authenticated
///   owner, and the check is one read by id, right however many endpoints the
///   person holds — and retires it with the stable [`disable_key`], so rotation
///   never leaks an orphan endpoint. When the person is at the endpoint quota,
///   the refusal is answered by retiring the known-stale handle and
///   re-registering once, so rotation at the bound still succeeds.
/// - A previous handle that is not this person's — a session switch left it
///   behind — is [`Outcome::Gone`] like any other enrollment the service no
///   longer has for them: nothing registers under the new identity, no revoke
///   is attempted, and no credential moves.
///
/// # Errors
///
/// [`EnrollError`] for each step's own failure, each naming what still
/// stands: [`EnrollError::Unconfirmed`] leaves the previous enrollment
/// untouched for the next launch, [`EnrollError::Enroll`] carries the
/// handle to keep retrying with (or `None` when the handle was already
/// retired or dropped), and [`EnrollError::RetireFailed`] /
/// [`EnrollError::OwnershipUnknown`] leave any new enrollment live and the
/// old handle still needing its revoke.
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(clippy::unused_async, reason = "matches the wasm signature")
)]
pub async fn reconcile(previous: Option<&Enrollment>) -> Result<Outcome, EnrollError> {
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = previous;
		Err(EnrollError::Unavailable)
	}

	#[cfg(target_arch = "wasm32")]
	{
		let Some(previous) = previous else {
			return Ok(Outcome::Off);
		};

		let snapshot =
			plugin_snapshot(leptoskit_plugin_notifications::guest::Notifications::register_remote)
				.await?;
		let credential = decide(&snapshot)?.ok_or(EnrollError::Unconfirmed)?;
		let details = device_details().await;

		// Nothing changed since this enrollment last sent: the same token,
		// the same details. There is nothing to register — only to learn
		// whether the service still has the enrollment, in one read. An
		// unanswered read changes nothing: the next launch asks again.
		if previous.is_current(&credential, &details) {
			return Ok(match owned(previous).await {
				Ok(false) => Outcome::Gone(*previous),
				Ok(true) | Err(_) => Outcome::Kept(*previous),
			});
		}

		rotate(previous, &credential, &details).await
	}
}

/// Brings the service up to date with the token the OS just issued and the
/// details the device reports: registers them, then retires the previous
/// handle when — and only when — the destination actually changed and the
/// handle is this person's.
#[cfg(target_arch = "wasm32")]
async fn rotate(
	previous: &Enrollment,
	credential: &PushCredential,
	details: &DeviceDetails,
) -> Result<Outcome, EnrollError> {
	match register(credential, details, Some(previous.endpoint)).await {
		// Same destination: one idempotent refresh — new details at most —
		// and nothing to retire.
		Ok(enrollment) if enrollment.endpoint == previous.endpoint => Ok(Outcome::Kept(enrollment)),
		Ok(enrollment) => retire(previous, enrollment).await,
		// The service no longer has the enrollment this launch renews, and
		// so registered nothing. It stays that way until a gesture.
		Err(cause) if crate::push::enrollment_gone(&cause) => Ok(Outcome::Gone(*previous)),
		Err(cause) if quota_refused(&cause) => {
			// The service's own remedy: revoke one first. The previous
			// handle is the one this device owns and is about to discard.
			// Freeing it needs the same ownership check as any retirement;
			// a stale handle at quota is dropped, not operated, and the
			// quota refusal is surfaced for the app to show.
			match owned(previous).await {
				Ok(true) => {}
				Ok(false) => {
					return Err(EnrollError::Enroll(None, cause));
				}
				Err(listing) => {
					return Err(EnrollError::OwnershipUnknown {
						current: None,
						retired: *previous,
						cause: listing,
					});
				}
			}

			if let Err(revoke) = revoke(previous).await {
				return Err(EnrollError::RetireFailed {
					current: None,
					retired: *previous,
					cause: revoke,
				});
			}

			// Room made: one more registration, and the rotation completes
			// with the retirement this attempt already acknowledged. It
			// renews nothing — the handle it stood on is the one just
			// revoked, under this person, moments ago.
			register(credential, details, None)
				.await
				.map(|current| {
					Outcome::Rotated {
						current,
						retired: Retirement::Revoked(*previous),
					}
				})
				.map_err(|cause| EnrollError::Enroll(None, cause))
		}
		Err(cause) => Err(EnrollError::Enroll(Some(*previous), cause)),
	}
}

/// Registers a token with the device's details and answers the enrollment
/// its endpoint id names, remembering what was sent. With `renews`, the
/// call is a launch's renewal of that endpoint and registers nothing once
/// the service no longer has it; without, it is a gesture's enrollment.
#[cfg(target_arch = "wasm32")]
async fn register(
	credential: &PushCredential,
	details: &DeviceDetails,
	renews: Option<uuid::Uuid>,
) -> Result<Enrollment, leptoskit_rpc::FnError> {
	let request = match renews {
		Some(endpoint) => renewal(credential.clone(), details.clone(), endpoint),
		None => request(credential.clone(), details.clone()),
	};
	let summary: EndpointSummary = leptoskit_rpc::client::call(request).await?;

	Ok(Enrollment {
		endpoint: summary.id,
		sent: Some(digest(credential, details)),
	})
}

/// The typed request an enrollment sends: the credential the OS issued and
/// the details the device reports — never an identity, which the service
/// reads from the forwarded session.
#[must_use]
pub fn request(
	credential: PushCredential,
	device: DeviceDetails,
) -> crate::push::RegisterPushEndpoint {
	crate::push::RegisterPushEndpoint {
		credential,
		device,
		renews: None,
	}
}

/// The typed request a launch's reconciliation sends when the token or the
/// details changed: [`request`], naming the endpoint the saved enrollment
/// holds. The service registers only while that endpoint is still this
/// person's, so an enrollment revoked or erased on the server is never
/// brought back by a launch.
#[must_use]
pub fn renewal(
	credential: PushCredential,
	device: DeviceDetails,
	renews: uuid::Uuid,
) -> crate::push::RegisterPushEndpoint {
	crate::push::RegisterPushEndpoint {
		credential,
		device,
		renews: Some(renews),
	}
}

/// Whether `cause` is the service's endpoint-quota refusal, whose words are
/// its documented contract: "a person holds at most … live endpoints;
/// revoke one first". The credential's own shape was validated at the
/// plugin boundary, so that is the one `Invalid` a real OS token can meet
/// here.
#[cfg(target_arch = "wasm32")]
fn quota_refused(cause: &leptoskit_rpc::FnError) -> bool {
	matches!(cause, leptoskit_rpc::FnError::Invalid(message)
		if message.starts_with("a person holds at most"))
}

/// Whether `previous` is still one of the signed-in person's endpoints —
/// the ownership check every retirement runs under the current identity,
/// and what an unchanged launch asks. One read by id: a listing would be
/// one page of a list that can be longer than a page.
#[cfg(target_arch = "wasm32")]
async fn owned(previous: &Enrollment) -> Result<bool, leptoskit_rpc::FnError> {
	leptoskit_rpc::client::call(PushEndpointStanding {
		endpoint: previous.endpoint,
	})
	.await
}

/// Revokes `previous` with the stable [`disable_key`].
#[cfg(target_arch = "wasm32")]
async fn revoke(previous: &Enrollment) -> Result<(), leptoskit_rpc::FnError> {
	leptoskit_rpc::client::call_with(
		RevokePushEndpoint {
			endpoint: previous.endpoint,
		},
		leptoskit_rpc::client::CallOptions::new().idempotency_key(disable_key(previous.endpoint)),
	)
	.await
}

/// The retirement half of a rotation: the previous handle leaves only under
/// its owner. The service's own answer settles whose it is; a revoke that is
/// not acknowledged, and a listing that cannot be read, both leave the new
/// enrollment live and the old handle named in the error for retry.
#[cfg(target_arch = "wasm32")]
async fn retire(previous: &Enrollment, current: Enrollment) -> Result<Outcome, EnrollError> {
	// Never this person's — a previous account's leftover. Operating it
	// under the current identity is exactly what must not happen; drop it
	// and say so.
	if !owned(previous).await.map_err(|cause| {
		EnrollError::OwnershipUnknown {
			current: Some(current),
			retired: *previous,
			cause,
		}
	})? {
		return Ok(Outcome::Rotated {
			current,
			retired: Retirement::Stale(*previous),
		});
	}

	revoke(previous).await.map_err(|cause| {
		EnrollError::RetireFailed {
			current: Some(current),
			retired: *previous,
			cause,
		}
	})?;

	Ok(Outcome::Rotated {
		current,
		retired: Retirement::Revoked(*previous),
	})
}

#[cfg(target_arch = "wasm32")]
use std::future::Future;

/// Resolves the plugin and runs one snapshot read. The plugin host answers
/// `Unavailable` when it isn't installed, which is the app's configuration
/// mistake, not a retryable device state.
#[cfg(target_arch = "wasm32")]
async fn plugin_snapshot<F>(
	read: impl FnOnce(leptoskit_plugin_notifications::guest::Notifications) -> F,
) -> Result<leptoskit_plugin_notifications::RemoteRegistration, EnrollError>
where
	F: Future<
		Output = Result<
			leptoskit_plugin_notifications::RemoteRegistration,
			leptoskit_bridge::Error,
		>,
	>,
{
	let notifications = leptoskit_plugin_notifications::guest::Notifications::new()
		.map_err(|_| EnrollError::NoPlugin)?;
	read(notifications)
		.await
		.map_err(|error| EnrollError::Register(error.to_string()))
}

/// Disables this person's service enrollment while still authenticated.
///
/// Never changes OS registration or permission — the device token stays the
/// device's — and never unsubscribes anything: revocation stops this
/// service's future attempts. Re-enabling the same token answers the same
/// destination a fresh id. Retries replay the endpoint-derived idempotency
/// key, including after the app restarts. Receipts belong to the
/// authenticated caller; a pruned receipt can leave a retry unconfirmed.
///
/// # Errors
///
/// [`DisableError::Revoke`] for every unconfirmed revoke, including
/// not-found.
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(clippy::unused_async, reason = "matches the browser signature")
)]
pub async fn disable(enrollment: &Enrollment) -> Result<(), DisableError> {
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = enrollment;
		Err(DisableError::Unavailable)
	}

	#[cfg(target_arch = "wasm32")]
	{
		leptoskit_rpc::client::call_with(
			RevokePushEndpoint {
				endpoint: enrollment.endpoint,
			},
			leptoskit_rpc::client::CallOptions::new()
				.idempotency_key(disable_key(enrollment.endpoint)),
		)
		.await
		.map_err(DisableError::Revoke)
	}
}

/// What a native item's plugin kind is called here.
#[must_use]
pub fn source(kind: leptoskit_plugin_notifications::ReceivedKind) -> crate::receive::Source {
	match kind {
		leptoskit_plugin_notifications::ReceivedKind::Opened => crate::receive::Source::Opened,
		leptoskit_plugin_notifications::ReceivedKind::Presented => {
			crate::receive::Source::Presented
		}
		leptoskit_plugin_notifications::ReceivedKind::Message => crate::receive::Source::Message,
	}
}

/// Reads one item the host received into the event the app sees, or
/// `None` for a message that carried nothing readable.
#[must_use]
pub fn received_from(
	item: &leptoskit_plugin_notifications::ReceivedItem,
) -> Option<crate::receive::Received> {
	crate::receive::from_host(
		source(item.kind()),
		item.action(),
		item.local(),
		item.payload(),
	)
}

/// What this device receives, in order: a tap on a notification or one of
/// its action buttons, a notification that arrived while the app was
/// open, and a silent data message. Make one when the app starts and ask
/// it for the [`next`](Receiver::next) event in a loop:
///
/// ```rust,ignore
/// let mut receiver = leptoskit_notifications::native::Receiver::new();
///
/// while let Ok(received) = receiver.next().await {
///     if let Received::Opened(opened) = received {
///         navigate(&opened.route("/inbox"));
///     }
/// }
/// ```
///
/// The tap that started the app is the first event, with
/// [`launched`](crate::receive::Opened::launched) set: the host keeps what
/// arrived before the app's code ran. It keeps the latest sixteen; the
/// inbox is the durable record, and this is only what happened on this
/// device just now.
///
/// iOS, macOS, and Android. Everywhere else, and on a host older than the
/// receiving method, [`next`](Receiver::next) answers
/// [`ReceiveError::Unavailable`](crate::receive::ReceiveError::Unavailable).
#[derive(Debug, Default)]
pub struct Receiver {
	#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
	intake: crate::receive::Intake,
}

impl Receiver {
	/// A receiver that has asked the host nothing yet.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Waits for the next event. Dropping the wait loses nothing: what
	/// arrives stays with the host for the next call.
	///
	/// # Errors
	///
	/// [`ReceiveError`](crate::receive::ReceiveError) when this host
	/// delivers nothing; the loop above then ends.
	#[cfg_attr(
		not(target_arch = "wasm32"),
		allow(clippy::unused_async, reason = "matches the wasm signature")
	)]
	pub async fn next(&mut self) -> Result<crate::receive::Received, crate::receive::ReceiveError> {
		#[cfg(not(target_arch = "wasm32"))]
		return Err(crate::receive::ReceiveError::Unavailable);

		#[cfg(target_arch = "wasm32")]
		loop {
			if let Some(received) = self.intake.pop() {
				return Ok(received);
			}

			self.ask(leptoskit_plugin_notifications::MAX_RECEIVED_WAIT_MILLIS)
				.await?;
		}
	}

	/// Everything that has arrived and not been handed over yet, without
	/// waiting: at launch, the tap that started the app, if there was one.
	///
	/// # Errors
	///
	/// [`ReceiveError`](crate::receive::ReceiveError) when this host
	/// delivers nothing.
	#[cfg_attr(
		not(target_arch = "wasm32"),
		allow(clippy::unused_async, reason = "matches the wasm signature")
	)]
	pub async fn pending(
		&mut self,
	) -> Result<Vec<crate::receive::Received>, crate::receive::ReceiveError> {
		#[cfg(not(target_arch = "wasm32"))]
		return Err(crate::receive::ReceiveError::Unavailable);

		#[cfg(target_arch = "wasm32")]
		{
			self.ask(0).await?;
			Ok(self.intake.drain())
		}
	}

	/// Asks the host once and queues what it answers. The first ask never
	/// waits, whatever `wait_millis` says: what it answers is then exactly
	/// what was there before the app asked, which is what marks a tap as
	/// the one that launched the app.
	#[cfg(target_arch = "wasm32")]
	async fn ask(&mut self, wait_millis: u32) -> Result<(), crate::receive::ReceiveError> {
		use crate::receive::ReceiveError;

		// The plugin host answers `Unavailable` when the plugin isn't
		// installed: the app's configuration, not a device state.
		let notifications = leptoskit_plugin_notifications::guest::Notifications::new()
			.map_err(|_| ReceiveError::NoPlugin)?;
		let wait = if self.intake.first() { 0 } else { wait_millis };
		let items = notifications.received(wait).await.map_err(|error| {
			match error {
				leptoskit_bridge::Error::Unavailable => ReceiveError::Unavailable,
				other => ReceiveError::Host(other.to_string()),
			}
		})?;
		self.intake.accept(items.iter().filter_map(received_from));

		Ok(())
	}
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
	use leptoskit_plugin_notifications::DeviceInfo;
	use leptoskit_plugin_notifications::DeviceToken;
	use leptoskit_plugin_notifications::FcmToken;
	use leptoskit_plugin_notifications::RemoteRegistration;
	use leptoskit_plugin_notifications::RemoteState;
	use leptoskit_plugin_notifications::remote_failure;

	use super::EnrollError;
	use super::Receiver;
	use super::decide;
	use super::disable_key;
	use super::received_from;
	use crate::push::DeviceDetails;
	use crate::push::PushCredential;
	use crate::push::RegisterPushEndpoint;

	/// A fake host answer: the exact bytes a real iOS host replies with for
	/// a device token Apple issued, serialized through the plugin's own
	/// codec. The token is deliberately 33 bytes — the shape the boundary
	/// must accept — and mixed case is folded by the host, so the wire only
	/// ever carries lower case.
	fn fake_host_token(hex: &str) -> RemoteRegistration {
		RemoteRegistration::new(
			1,
			RemoteState::Token(DeviceToken::new(hex).expect("the fake token is in bounds")),
		)
		.expect("a revisioned snapshot")
	}

	/// The Android counterpart: a registration token in Firebase's own
	/// alphabet — printable ASCII, never hex, of no assumed length.
	fn fake_host_fcm_token(token: &str) -> RemoteRegistration {
		RemoteRegistration::new(
			1,
			RemoteState::FcmToken(FcmToken::new(token).expect("the fake token is in bounds")),
		)
		.expect("a revisioned snapshot")
	}

	/// The snapshot decides enrollment: a token becomes the typed credential
	/// with the exact bytes — APNs from an Apple host, FCM from an Android one
	/// — silence stays None, the OS's refusal is surfaced rather than retried
	/// into a fake enrollment, and an unconfirmed registration is never
	/// mistaken for a fresh token.
	#[test]
	fn a_snapshot_decides_what_enrollment_registers() {
		let token = "a1b2c3d4e5f67890".repeat(4); // gitleaks:allow (a fake device token a test builds)
		let credential = decide(&fake_host_token(&token)).unwrap().unwrap();
		assert_eq!(
			credential,
			PushCredential::Apns {
				device_token: token,
			}
		);

		let fcm = "WebView-Help:dTOkay8-_fake".repeat(6); // gitleaks:allow (a fake FCM token a test builds)
		let credential = decide(&fake_host_fcm_token(&fcm)).unwrap().unwrap();
		assert_eq!(
			credential,
			PushCredential::Fcm {
				registration_token: fcm,
			}
		);

		assert_eq!(decide(&RemoteRegistration::none()).unwrap(), None);

		let refused = RemoteRegistration::new(
			2,
			RemoteState::Failed(remote_failure::APS_ENVIRONMENT.into()),
		)
		.unwrap();
		assert_eq!(
			decide(&refused),
			Err(EnrollError::Failed(remote_failure::APS_ENVIRONMENT.into()))
		);

		let unconfigured = RemoteRegistration::new(
			2,
			RemoteState::Failed(remote_failure::FIREBASE_NOT_CONFIGURED.into()),
		)
		.unwrap();
		assert_eq!(
			decide(&unconfigured),
			Err(EnrollError::Failed(
				remote_failure::FIREBASE_NOT_CONFIGURED.into()
			))
		);

		assert_eq!(
			decide(&RemoteRegistration::unconfirmed()),
			Err(EnrollError::Unconfirmed)
		);
	}

	/// The typed RPC request the guest sends: the credential serialized
	/// exactly as the service parses it, per platform, and the device's
	/// details, with no identity field to forge. The identity half runs
	/// against the real service in `tests/native.rs`.
	#[test]
	fn the_request_is_the_service_typed_envelope_and_nothing_else() {
		let token = "a1b2c3d4e5f67890".repeat(4); // gitleaks:allow (a fake device token a test builds)
		let credential = decide(&fake_host_token(&token)).unwrap().unwrap();
		let details = super::details_from(&DeviceInfo {
			locale: Some("en_GB".into()),
			time_zone: Some("Europe/London".into()),
			app_version: Some("2.1.0".into()),
		});
		let request = serde_json::to_value(super::request(credential, details)).unwrap();

		assert_eq!(
			request,
			serde_json::json!({
				"credential": { "apns": { "device_token": token } },
				"device": {
					"locale": "en-GB",
					"time_zone": "Europe/London",
					"app_version": "2.1.0",
				},
			})
		);

		// A host that reported nothing sends no details, and the request
		// is the one every service version parses.
		let fcm = "WebView-Help:dTOkay8-_fake".repeat(6); // gitleaks:allow (a fake FCM token a test builds)
		let credential = decide(&fake_host_fcm_token(&fcm)).unwrap().unwrap();
		let request =
			serde_json::to_value(super::request(credential, DeviceDetails::default())).unwrap();

		assert_eq!(
			request,
			serde_json::json!({
				"credential": { "fcm": { "registration_token": fcm } },
				"device": {},
			})
		);
		let parsed: RegisterPushEndpoint = serde_json::from_value(request).unwrap();
		assert_eq!(parsed.device, DeviceDetails::default());
	}

	/// A launch's renewal names the enrollment it renews, so the service
	/// registers nothing once that is gone; a gesture's enrollment names
	/// none. And the service's "gone" is told from every other failure by
	/// its own words, so no stray not-found drops an enrollment.
	#[test]
	fn a_renewal_names_its_enrollment_and_gone_is_recognized() {
		use crate::push::ENROLLMENT_GONE;
		use crate::push::enrollment_gone;

		let token = "a1b2c3d4e5f67890".repeat(4); // gitleaks:allow (a fake device token a test builds)
		let credential = decide(&fake_host_token(&token)).unwrap().unwrap();
		let endpoint: uuid::Uuid = "6ec0f4b1-0000-7000-8000-000000000001".parse().unwrap();
		let renewal = serde_json::to_value(super::renewal(
			credential.clone(),
			DeviceDetails::default(),
			endpoint,
		))
		.unwrap();
		assert_eq!(
			renewal,
			serde_json::json!({
				"credential": { "apns": { "device_token": token } },
				"device": {},
				"renews": endpoint,
			})
		);
		let parsed: RegisterPushEndpoint = serde_json::from_value(renewal).unwrap();
		assert_eq!(parsed.renews, Some(endpoint));

		let enrollment =
			serde_json::to_value(super::request(credential, DeviceDetails::default())).unwrap();
		assert!(
			enrollment.get("renews").is_none(),
			"a gesture's enrollment renews nothing: {enrollment}"
		);
		let parsed: RegisterPushEndpoint = serde_json::from_value(enrollment).unwrap();
		assert_eq!(parsed.renews, None);

		// The service's answer, as it crosses the wire.
		let gone: leptoskit_rpc::FnError = leptoskit_rpc::Error::not_found(format!(
			"{ENROLLMENT_GONE}: it was revoked or erased on the server"
		))
		.into();
		assert!(enrollment_gone(&gone));

		for other in [
			leptoskit_rpc::Error::not_found("no endpoint with this id is yours").into(),
			leptoskit_rpc::Error::invalid(format!("{ENROLLMENT_GONE}: but as a bad request"))
				.into(),
			leptoskit_rpc::FnError::Network(ENROLLMENT_GONE.into()),
			leptoskit_rpc::FnError::Other(ENROLLMENT_GONE.into()),
		] {
			assert!(!enrollment_gone(&other), "{other:?}");
		}
	}

	/// What the OS reports is sanitized before it's sent: the locale in
	/// canonical case, and a detail the service would refuse dropped
	/// rather than costing the device its enrollment.
	#[test]
	fn reported_details_are_sanitized_before_they_are_sent() {
		let details = super::details_from(&DeviceInfo {
			locale: Some("zh_hant_tw".into()),
			time_zone: Some("not a zone".into()),
			app_version: Some(" 3.0 (41) ".into()),
		});
		assert_eq!(
			details,
			DeviceDetails {
				locale: Some("zh-Hant-TW".into()),
				time_zone: None,
				app_version: Some("3.0 (41)".into()),
			}
		);

		assert_eq!(
			super::details_from(&DeviceInfo::default()),
			DeviceDetails::default()
		);

		// Off a guest there is no plugin to ask: no details, no failure.
		assert_eq!(
			futures::executor::block_on(super::device_details()),
			DeviceDetails::default()
		);
	}

	/// The launch decision: the same token and the same details send
	/// nothing; an app update, a language change, a new time zone, a
	/// rotated token, or an enrollment saved before details existed sends
	/// once.
	#[test]
	fn reconcile_sends_only_when_the_token_or_the_details_changed() {
		use super::Enrollment;
		use super::digest;

		let token = "a1b2c3d4e5f67890".repeat(4); // gitleaks:allow (a fake device token a test builds)
		let credential = decide(&fake_host_token(&token)).unwrap().unwrap();
		let details = DeviceDetails {
			locale: Some("en-GB".into()),
			time_zone: Some("Europe/London".into()),
			app_version: Some("2.1.0".into()),
		};
		let endpoint = "6ec0f4b1-0000-7000-8000-000000000001".parse().unwrap();
		let saved = Enrollment {
			endpoint,
			sent: Some(digest(&credential, &details)),
		};

		assert!(
			saved.is_current(&credential, &details),
			"nothing changed: nothing is sent"
		);

		let updated = DeviceDetails {
			app_version: Some("2.2.0".into()),
			..details.clone()
		};
		let french = DeviceDetails {
			locale: Some("fr-FR".into()),
			..details.clone()
		};
		let travelled = DeviceDetails {
			time_zone: Some("America/New_York".into()),
			..details.clone()
		};
		let forgotten = DeviceDetails {
			app_version: None,
			..details.clone()
		};
		for changed in [&updated, &french, &travelled, &forgotten] {
			assert!(
				!saved.is_current(&credential, changed),
				"{changed:?} is a change to send"
			);
		}

		let rotated = decide(&fake_host_token(&"0f1e2d3c4b5a6978".repeat(4)))
			.unwrap()
			.unwrap();
		assert!(!saved.is_current(&rotated, &details), "a rotated token");

		// Absent and empty are told apart, and details can't run into the
		// token or each other.
		let shifted = DeviceDetails {
			locale: None,
			time_zone: Some("en-GB".into()),
			app_version: Some("Europe/London".into()),
		};
		assert_ne!(digest(&credential, &details), digest(&credential, &shifted));

		// An enrollment saved before details existed sends once, and it
		// still reads from storage as it was written.
		let old: Enrollment = serde_json::from_value(serde_json::json!({
			"endpoint": endpoint,
		}))
		.unwrap();
		assert_eq!(old.sent, None);
		assert!(!old.is_current(&credential, &DeviceDetails::default()));
		assert_eq!(
			serde_json::to_value(Enrollment {
				endpoint,
				sent: None
			})
			.unwrap(),
			serde_json::json!({ "endpoint": endpoint })
		);

		// The digest is stable across builds: the app persists it.
		assert_eq!(
			digest(&credential, &DeviceDetails::default()),
			digest(&credential, &DeviceDetails::default())
		);
		assert_eq!(
			digest(
				&PushCredential::Fcm {
					registration_token: "token".into()
				},
				&DeviceDetails::default()
			),
			0x9dcd_6d26_c72e_e0a8_u64,
			"the FNV-1a digest of `fcm\\0token\\0\\0\\0`"
		);
	}

	/// A credential's `Debug` never carries the token, so a logged or
	/// panicked enrollment value leaks nothing, on either platform.
	#[test]
	fn the_credential_debug_redacts_both_token_kinds() {
		let token = "a1b2c3d4e5f67890".repeat(4); // gitleaks:allow (a fake device token a test builds)
		let fcm = "WebView-Help:dTOkay8-_fake".repeat(6); // gitleaks:allow (a fake FCM token a test builds)

		for credential in [
			decide(&fake_host_token(&token)).unwrap().unwrap(),
			decide(&fake_host_fcm_token(&fcm)).unwrap().unwrap(),
		] {
			let printed = format!("{credential:?}");
			assert!(!printed.contains(&token), "{printed}");
			assert!(!printed.contains(&fcm), "{printed}");
		}
	}

	/// Disabling replays the browser module's stable key for the same
	/// endpoint, so the two modules' sign-outs are indistinguishable to the
	/// service's receipt book.
	#[test]
	fn disable_replays_the_browser_contract_key() {
		let endpoint = "6ec0f4b1-0000-7000-8000-000000000001".parse().unwrap();
		assert_eq!(
			disable_key(endpoint),
			format!("notifications:disable:{endpoint}")
		);
	}

	/// Outside a wasm guest, nothing enrolls and nothing is stranded — the
	/// stubs answer their own unavailable errors without a call, whatever
	/// they are handed.
	#[test]
	fn outside_a_guest_nothing_enrolls_and_nothing_is_stranded() {
		let error = futures::executor::block_on(super::enroll());
		assert_eq!(error, Err(EnrollError::Unavailable));

		let enrollment = super::Enrollment {
			endpoint: "6ec0f4b1-0000-7000-8000-000000000001".parse().unwrap(),
			sent: None,
		};
		let error = futures::executor::block_on(super::reconcile(Some(&enrollment)));
		assert_eq!(error, Err(EnrollError::Unavailable));
		assert_eq!(
			futures::executor::block_on(super::reconcile(None)),
			Err(EnrollError::Unavailable),
			"the non-guest stub stays unavailable for both shapes"
		);

		let error = futures::executor::block_on(super::disable(&enrollment));
		assert_eq!(
			error,
			Err(super::DisableError::Unavailable),
			"no revoke was attempted off the guest"
		);
	}

	#[test]
	fn a_host_item_becomes_the_event_the_app_sees() {
		use leptoskit_plugin_notifications::ReceivedItem;
		use leptoskit_plugin_notifications::ReceivedKind;

		use crate::receive::Opened;
		use crate::receive::Received;

		let id = "0190a1b2-0000-7000-8000-000000000001";
		let tap = ReceivedItem::new(
			ReceivedKind::Opened,
			"approve",
			"",
			format!(r#"{{"aps":{{}},"lkn":"{id}","lklink":"/invoice/42"}}"#),
		)
		.unwrap();

		assert_eq!(
			received_from(&tap),
			Some(Received::Opened(Opened {
				notification: Some(id.parse().unwrap()),
				link: Some("/invoice/42".into()),
				action: Some("approve".into()),
				..Opened::default()
			}))
		);

		// A local notification's tap names the id the app posted it with.
		let local = ReceivedItem::new(ReceivedKind::Opened, "", "job-1", "").unwrap();
		assert_eq!(
			received_from(&local),
			Some(Received::Opened(Opened {
				local: Some("job-1".into()),
				..Opened::default()
			}))
		);

		// A message with nothing readable is nothing to hand over.
		let empty = ReceivedItem::new(ReceivedKind::Message, "", "", "").unwrap();
		assert_eq!(received_from(&empty), None);
	}

	#[test]
	fn receiving_outside_a_guest_is_unavailable() {
		use crate::receive::ReceiveError;

		let mut receiver = Receiver::new();

		assert_eq!(
			futures::executor::block_on(receiver.next()),
			Err(ReceiveError::Unavailable)
		);
		assert_eq!(
			futures::executor::block_on(receiver.pending()),
			Err(ReceiveError::Unavailable)
		);
	}
}
