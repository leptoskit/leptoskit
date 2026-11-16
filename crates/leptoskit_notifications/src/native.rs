//! The native half of push enrollment: the device token the OS issues — an
//! APNs device token on iOS, an FCM registration token on Android — acquired
//! through the notifications plugin and enrolled under the signed-in person.
//! It is the native counterpart of [`crate::browser`], with the same shape
//! and the same boundaries.
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
//! 1. Configure the build: `[native.ios] aps-environment` set to the
//!    environment the app's provisioning profile and the service's
//!    `LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT` name, the `notifications`
//!    plugin in `[native] plugins`, and the `native` feature on the crate.
//!    Without the entitlement the OS refuses registration with a provisioning
//!    failure; nothing here can work around it.
//! 2. On an **explicit user gesture** — a "turn on notifications" switch, never
//!    a mount or a render — request notification permission through the
//!    plugin's existing guest API
//!    ([`leptoskit_plugin_notifications::guest::Notifications::request_permission`]).
//!    Alerts are what the permission gates; registration itself needs no
//!    dialog, but an app that enrolls without asking has quietly promised
//!    banners the person never opted into.
//! 3. Still in the gesture, call [`enroll`]: it asks the OS to register, waits
//!    a bounded time for the answer, and registers the token as the person's
//!    typed [`crate::push::PushCredential`] — `Apns` from an iOS host, `Fcm`
//!    from an Android one. The platform is a transport choice and nothing more:
//!    the token kind the host answers names the provider kind, and the workflow
//!    around it is identical. The answer is an opaque [`Enrollment`] id: keep
//!    it, with the opt-in, beside the person's session.
//! 4. On **launch**, once the person is signed in, call [`reconcile`] with that
//!    saved enrollment — and only with one: it is the record that this person
//!    opted in on this device, and without it nothing registers, so a disabled
//!    device is never silently re-enabled. With it, the module asks the OS to
//!    register again — registration prompts nothing on either platform; iOS
//!    expects it every launch and Android refreshes its token the same way —
//!    waits the bounded answer, and brings the service up to date with whatever
//!    the OS now issues. The answer is an explicit [`Outcome`]: unchanged,
//!    rotated with the old handle's retirement acknowledged, or rotated with a
//!    stale handle — a previous account's leftover — dropped rather than
//!    operated under this one.
//! 5. On **sign out**, while this person is still authenticated, call
//!    [`disable`]. It requires an acknowledged server revoke and replays a
//!    stable idempotency key derived from the endpoint id — the same contract
//!    as [`crate::browser::disable`] — so a retried sign-out is safe. It
//!    changes no OS state; the token stays the device's, and the next account
//!    enrolls from clean. Clear the saved enrollment and opt-in once the revoke
//!    is acknowledged.
//!
//! iOS and Android only. macOS shares the APNs transport but not this plugin
//! path; it is later work, and calls on hosts without the plugin answer
//! `Unavailable` rather than pretending.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

#[cfg(target_arch = "wasm32")]
use crate::push::EndpointSummary;
use crate::push::PushCredential;
#[cfg(target_arch = "wasm32")]
use crate::push::PushEndpoints;
#[cfg(target_arch = "wasm32")]
use crate::push::RegisterPushEndpoint;
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
}

/// Why enrolling this device did not happen, and what still stands. Every
/// variant names an explicit state: nothing is half-enrolled, nothing is
/// silently retried, and the variants that carry handles say which
/// enrollment to keep retrying with.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnrollError {
	/// This host runs no native enrollment. Only an iOS or Android guest
	/// runs it; macOS is later work.
	#[error("native enrollment needs an iOS or Android guest on this host")]
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
	/// or mismatched iOS entitlement, or `firebase-not-configured` when an
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
	/// The service's endpoint listing failed, so ownership of the previous
	/// handle could not be verified and nothing was revoked. As
	/// [`EnrollError::RetireFailed`], both handles stand.
	#[error("could not verify the previous enrollment's owner: {cause}")]
	OwnershipUnknown {
		/// The replacement enrollment, when the rotation got that far.
		current: Option<Enrollment>,
		/// The handle left unverified and unreleased.
		retired: Enrollment,
		/// Why the listing failed: the network, or the service's answer.
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
	#[error("native enrollment needs an iOS or Android guest on this host")]
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
/// answer names — an APNs hex token means iOS, an FCM printable token means
/// Android — and this module never second-guesses it. The token's shape was
/// validated at the plugin boundary; its bytes are forwarded verbatim, never
/// re-derived or stored.
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
	/// The OS still issues the token this enrollment holds: the service
	/// answered the same endpoint id, the refresh was one idempotent
	/// re-registration, and the handle the app saved is still the one to
	/// keep.
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
}

/// How a rotated-out enrollment handle left, when rotation succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retirement {
	/// The service acknowledged the revoke under the same signed-in person
	/// who enrolled the replacement, with the stable [`disable_key`]. The
	/// handle is gone; drop it.
	Revoked(Enrollment),
	/// The handle is not among this person's endpoints — a previous
	/// account's leftover from a session switch, or one already revoked.
	/// It was never operated under this identity: no revoke was attempted,
	/// and no credential moved. Drop it.
	Stale(Enrollment),
}

/// Enrolls this device for the signed-in person, from an explicit user
/// gesture: permission is the app's own prior call, so this asks the OS to
/// register, waits the host's bounded time for the answer, and registers
/// the token — APNs on an iOS host, FCM on an Android one. Two racing calls
/// converge on one endpoint: the service answers the same id for the same
/// destination.
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
				register(&credential)
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
/// prompts nothing on either platform; only permission requests do, and
/// those stay the gesture's — and reconciles whatever the OS now issues,
/// under whoever is signed in right now:
///
/// - The same token keeps the same endpoint id (the service dedupes the
///   destination): [`Outcome::Kept`], one idempotent call.
/// - A rotated token is a new destination. The module verifies the previous
///   handle against this person's endpoint list — cleanup runs only for the
///   authenticated owner — and retires it with the stable [`disable_key`], so
///   rotation never leaks an orphan endpoint. When the person is at the
///   endpoint quota, the refusal is answered by retiring the known-stale handle
///   and re-registering once, so rotation at the bound still succeeds.
/// - A previous handle that is not this person's — a session switch left it
///   behind — is dropped, not operated: no revoke is attempted under the new
///   identity, and no credential moves ([`Retirement::Stale`]).
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
		rotate(previous, &credential).await
	}
}

/// Brings the service up to date with the token the OS just issued:
/// registers it, then retires the previous handle when — and only when —
/// the destination actually changed and the handle is this person's.
#[cfg(target_arch = "wasm32")]
async fn rotate(
	previous: &Enrollment,
	credential: &PushCredential,
) -> Result<Outcome, EnrollError> {
	match register(credential).await {
		// Same destination: one idempotent refresh, nothing to retire.
		Ok(enrollment) if enrollment == *previous => Ok(Outcome::Kept(*previous)),
		Ok(enrollment) => retire(previous, enrollment).await,
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
			// with the retirement this attempt already acknowledged.
			register(credential)
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

/// Registers a token and answers the enrollment its endpoint id names.
#[cfg(target_arch = "wasm32")]
async fn register(credential: &PushCredential) -> Result<Enrollment, leptoskit_rpc::FnError> {
	let summary: EndpointSummary = leptoskit_rpc::client::call(RegisterPushEndpoint {
		credential: credential.clone(),
	})
	.await?;

	Ok(Enrollment {
		endpoint: summary.id,
	})
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

/// Whether `previous` is among the signed-in person's endpoints — the
/// ownership check every retirement runs under the current identity.
#[cfg(target_arch = "wasm32")]
async fn owned(previous: &Enrollment) -> Result<bool, leptoskit_rpc::FnError> {
	let endpoints: leptoskit_rpc::Page<EndpointSummary> =
		leptoskit_rpc::client::call(PushEndpoints {
			page: leptoskit_rpc::PageArgs::first(
				u32::try_from(crate::push::MAX_ENDPOINTS + crate::push::MAX_ENDPOINT_HISTORY)
					.unwrap_or(u32::MAX),
			),
		})
		.await?;

	Ok(endpoints
		.items
		.iter()
		.any(|endpoint| endpoint.id == previous.endpoint))
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
/// its owner. The endpoint list settles whose it is; a revoke that is not
/// acknowledged, and a listing that cannot be read, both leave the new
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
	use leptoskit_plugin_notifications::DeviceToken;
	use leptoskit_plugin_notifications::FcmToken;
	use leptoskit_plugin_notifications::RemoteRegistration;
	use leptoskit_plugin_notifications::RemoteState;
	use leptoskit_plugin_notifications::remote_failure;

	use super::EnrollError;
	use super::decide;
	use super::disable_key;
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
	/// with the exact bytes — APNs from an iOS host, FCM from an Android one
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
	/// exactly as the service parses it, per platform, with no identity
	/// field to forge. The identity half runs against the real service in
	/// `tests/native.rs`.
	#[test]
	fn the_request_is_the_service_typed_envelope_and_nothing_else() {
		let token = "a1b2c3d4e5f67890".repeat(4); // gitleaks:allow (a fake device token a test builds)
		let credential = decide(&fake_host_token(&token)).unwrap().unwrap();
		let request = serde_json::to_value(RegisterPushEndpoint { credential }).unwrap();

		assert_eq!(
			request,
			serde_json::json!({
				"credential": { "apns": { "device_token": token } }
			})
		);

		let fcm = "WebView-Help:dTOkay8-_fake".repeat(6); // gitleaks:allow (a fake FCM token a test builds)
		let credential = decide(&fake_host_fcm_token(&fcm)).unwrap().unwrap();
		let request = serde_json::to_value(RegisterPushEndpoint { credential }).unwrap();

		assert_eq!(
			request,
			serde_json::json!({
				"credential": { "fcm": { "registration_token": fcm } }
			})
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
}
