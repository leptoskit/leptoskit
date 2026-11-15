//! The native half of push enrollment: an iOS app's APNs device token,
//! acquired through the notifications plugin, enrolled under the signed-in
//! person. It is the iOS counterpart of [`crate::browser`], with the same
//! shape and the same boundaries.
//!
//! What the module does and does not touch: the token comes from the
//! generated app delegate via the plugin's broker — never from storage this
//! module keeps — and the enrollment call is the app's own RPC client, so
//! the endpoint belongs to the forwarded signed-in person. No service
//! token, no session copy, and no identity live here or anywhere host-side.
//! The person's session lifecycle stays the app's: launch reconciliation is
//! nonprompting and only enrolls under whoever is signed in right now, and
//! signing out is the app's cue to call [`disable`] while still
//! authenticated, before the next account signs in on the device.
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
//! 3. Still in the gesture, call [`enroll`]: it asks the OS to register for
//!    remote notifications, waits a bounded time for the delegate's answer, and
//!    registers the token as the person's typed
//!    [`crate::push::PushCredential::Apns`]. The answer is an opaque
//!    [`Enrollment`] id: keep it beside the person's session.
//! 4. On **launch**, once the person is signed in, call [`reconcile`]. It reads
//!    the broker's snapshot without prompting and without registering: when the
//!    OS has a token for this install — including a token rotated since the
//!    last run — it re-registers that destination, which answers the same
//!    endpoint id, so refreshes never exhaust the per-person endpoint quota.
//!    `None` is the app's cue to offer the switch.
//! 5. On **sign out**, while this person is still authenticated, call
//!    [`disable`]. It requires an acknowledged server revoke and replays a
//!    stable idempotency key derived from the endpoint id — the same contract
//!    as [`crate::browser::disable`] — so a retried sign-out is safe. It
//!    changes no OS state; the token stays the device's, and the next account
//!    enrolls from clean.
//!
//! iOS only. Android enrolls an FCM registration token and macOS shares the
//! APNs transport but not this plugin path; both are later work, and calls
//! on hosts without the plugin answer `Unavailable` rather than pretending.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

#[cfg(target_arch = "wasm32")]
use crate::push::EndpointSummary;
use crate::push::PushCredential;
#[cfg(target_arch = "wasm32")]
use crate::push::RegisterPushEndpoint;
#[cfg(target_arch = "wasm32")]
use crate::push::RevokePushEndpoint;

/// The server endpoint identity needed to disable this person's enrollment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Enrollment {
	/// The endpoint's opaque id, as [`crate::push::EndpointSummary`] lists it.
	pub endpoint: uuid::Uuid,
}

/// Why enrolling this device did not happen. Every variant is actionable,
/// and none of them changes OS state: a retried [`enroll`] asks the OS
/// again, and nothing is half-enrolled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnrollError {
	/// This platform has no native enrollment. Only an iOS host runs it;
	/// Android and macOS are later work.
	#[error("native enrollment is iOS-only on this host")]
	Unavailable,
	/// The notifications plugin is not installed: add `"notifications"` to
	/// `[native] plugins`. Nothing was prompted and nothing registered.
	#[error("the notifications plugin is not installed: add \"notifications\" to [native] plugins")]
	NoPlugin,
	/// The bridge call itself failed — cancelled, busy, or a native
	/// failure. Nothing was registered with the service.
	#[error("registering for remote notifications failed: {0}")]
	Register(String),
	/// The OS answered no token within the host's registration bound. A
	/// retry from a user gesture asks again; hosts hand tokens out on
	/// launch, so [`reconcile`] also picks it up later.
	#[error("the OS has not answered registration with a device token yet")]
	NoToken,
	/// The OS refused registration; the reason names why, most often a
	/// missing or mismatched `aps-environment` entitlement. Nothing was
	/// registered with the service.
	#[error("the OS refused registration: {0}")]
	Failed(String),
	/// The enrollment call failed: the network, or the service's own answer
	/// — including the refusal that says another live account holds this
	/// device token, which that account's [`disable`] releases. The failure
	/// is surfaced, never swallowed.
	#[error("enrolling failed: {0}")]
	Enroll(leptoskit_rpc::FnError),
}

/// Why disabling the service enrollment was not confirmed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DisableError {
	/// No successful revoke acknowledgement: retry while authenticated.
	#[error("revoking the endpoint failed: {0}")]
	Revoke(leptoskit_rpc::FnError),
	/// Outside an iOS native guest; no revoke was attempted.
	#[error("native enrollment is iOS-only on this host")]
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
/// the OS's own refusal. The token's shape was validated at the plugin
/// boundary; its bytes are forwarded verbatim, never re-derived or stored.
pub fn decide(
	snapshot: &leptoskit_plugin_notifications::ApnsRegistration,
) -> Result<Option<PushCredential>, EnrollError> {
	use leptoskit_plugin_notifications::RemoteState;

	match snapshot.state() {
		RemoteState::None => Ok(None),
		RemoteState::Failed(reason) => Err(EnrollError::Failed(reason.clone())),
		RemoteState::Token(token) => {
			Ok(Some(PushCredential::Apns {
				device_token: token.as_str().to_owned(),
			}))
		}
	}
}

/// Registers one token as the signed-in person's, and answers the handle
/// its disable needs. The person is read from the app's forwarded identity
/// by the service, never from anything this module holds.
#[cfg(target_arch = "wasm32")]
async fn register(credential: PushCredential) -> Result<Enrollment, EnrollError> {
	let summary: EndpointSummary = leptoskit_rpc::client::call(RegisterPushEndpoint { credential })
		.await
		.map_err(EnrollError::Enroll)?;

	Ok(Enrollment {
		endpoint: summary.id,
	})
}

/// Enrolls this device for the signed-in person, from an explicit user
/// gesture: permission is the app's own prior call, so this asks the OS to
/// register for remote notifications, waits the host's bounded time for the
/// delegate's answer, and registers the token. Two racing calls converge on
/// one endpoint: the service answers the same id for the same destination.
///
/// # Errors
///
/// [`EnrollError`] for each step's own failure.
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(clippy::unused_async, reason = "matches the wasm signature")
)]
pub async fn enroll() -> Result<Enrollment, EnrollError> {
	#[cfg(not(target_arch = "wasm32"))]
	return Err(EnrollError::Unavailable);

	#[cfg(target_arch = "wasm32")]
	{
		let snapshot = plugin_snapshot(|notifications| notifications.register_remote()).await?;

		match decide(&snapshot)? {
			Some(credential) => register(credential).await,
			None => Err(EnrollError::NoToken),
		}
	}
}
/// The nonprompting half of enrollment, for app launch: reads the broker's
/// snapshot — a call that neither prompts nor registers — and registers the
/// token the OS already holds for this install, if it holds one, under the
/// person signed in right now.
///
/// A rotated token lands here too: re-registering a destination answers the
/// same endpoint id, so the refresh is one idempotent call, never a new
/// endpoint. `None` means the OS holds no token — the app's cue to offer
/// the switch whose gesture-gated [`enroll`] can make one.
///
/// # Errors
///
/// As [`enroll`], minus the OS registration: there is no
/// [`EnrollError::NoToken`], because nothing registers.
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(clippy::unused_async, reason = "matches the wasm signature"),
	allow(clippy::needless_return)
)]
pub async fn reconcile() -> Result<Option<Enrollment>, EnrollError> {
	#[cfg(not(target_arch = "wasm32"))]
	return Err(EnrollError::Unavailable);

	#[cfg(target_arch = "wasm32")]
	{
		let snapshot = plugin_snapshot(|notifications| notifications.remote_snapshot()).await?;

		match decide(&snapshot)? {
			Some(credential) => Ok(Some(register(credential).await?)),
			None => Ok(None),
		}
	}
}

#[cfg(target_arch = "wasm32")]
use std::future::Future;

/// Resolves the plugin and runs one snapshot read. The plugin host answers
/// `Unavailable` when it isn't installed, which is the app's configuration
/// mistake, not a retryable device state.
#[cfg(target_arch = "wasm32")]
async fn plugin_snapshot<F>(
	read: impl FnOnce(leptoskit_plugin_notifications::guest::Notifications) -> F,
) -> Result<leptoskit_plugin_notifications::ApnsRegistration, EnrollError>
where
	F: Future<
		Output = Result<leptoskit_plugin_notifications::ApnsRegistration, leptoskit_bridge::Error>,
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
	use leptoskit_plugin_notifications::ApnsRegistration;
	use leptoskit_plugin_notifications::DeviceToken;
	use leptoskit_plugin_notifications::RemoteState;

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
	fn fake_host_token(hex: &str) -> ApnsRegistration {
		ApnsRegistration::new(
			1,
			RemoteState::Token(DeviceToken::new(hex).expect("the fake token is in bounds")),
		)
		.expect("a revisioned snapshot")
	}

	/// The snapshot decides enrollment: a token becomes the typed APNs
	/// credential with the exact bytes, silence stays None, and the OS's
	/// refusal is surfaced rather than retried into a fake enrollment.
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

		assert_eq!(decide(&ApnsRegistration::none()).unwrap(), None);

		let refused = ApnsRegistration::new(
			2,
			RemoteState::Failed("no valid aps-environment entitlement".into()),
		)
		.unwrap();
		assert_eq!(
			decide(&refused),
			Err(EnrollError::Failed(
				"no valid aps-environment entitlement".into()
			))
		);
	}

	/// The typed RPC request the guest sends: the credential serialized
	/// exactly as the service parses it, with no identity field to forge.
	/// The identity half runs against the real service in `tests/native.rs`.
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
	/// stubs answer their own unavailable errors without a call.
	#[test]
	fn outside_a_guest_nothing_enrolls_and_nothing_is_stranded() {
		let error = futures::executor::block_on(super::enroll());
		assert_eq!(error, Err(EnrollError::Unavailable));

		let error = futures::executor::block_on(super::reconcile());
		assert_eq!(error, Err(EnrollError::Unavailable));

		let enrollment = super::Enrollment {
			endpoint: "6ec0f4b1-0000-7000-8000-000000000001".parse().unwrap(),
		};
		let error = futures::executor::block_on(super::disable(&enrollment));
		assert_eq!(
			error,
			Err(super::DisableError::Unavailable),
			"no revoke was attempted off the guest"
		);
	}
}
