//! The browser half of Web Push enrollment: the receiver worker, the
//! availability check, and the enroll and disable flows an app's web
//! client runs on its person's behalf.
//!
//! The service side of Web Push is the transport in [`crate::service`]:
//! it pushes to enrolled subscriptions with the person's own VAPID
//! identity. What was missing is the client half — the service worker
//! that shows what arrives, and the page code that turns the browser's
//! subscription into a [`crate::push::PushCredential`] — and that is this
//! module. It exists behind the `browser` feature, adds no server
//! dependency (the tree is checked by `tests/dependencies.rs`), and every
//! call outside a browser answers the plain "unavailable" errors the
//! [`leptoskit_service_worker`] crate does, so the same app code compiles
//! on native targets and during server rendering.
//!
//! The whole workflow, in the order an app wires it:
//!
//! 1. Build the receiver script with your inbox route baked in
//!    ([`receiver_script`]) and serve it from your web assets at the origin's
//!    root — `web/sw.js`, served as `/sw.js` with a JavaScript content type —
//!    so its scope covers the app. The raw template is [`RECEIVER_SCRIPT`] if
//!    you need it for a template engine.
//! 2. Register it once at startup: [`register`] with the script's path
//!    (`"/sw.js"`) and [`RegisterOptions::default()`], then [`ready`] to wait
//!    for control. Nothing in [`enroll`] registers for you: offline caching
//!    rules and scopes stay yours.
//! 3. On an **explicit user gesture** — a "turn on notifications" button, never
//!    a mount or a render — call [`enroll`]. It checks [`availability`], waits
//!    for the **active** worker instead of racing its install, reuses the
//!    subscription the browser already holds rather than subscribing twice, and
//!    registers the result with [`crate::push::register_push_endpoint`] over
//!    the app's own RPC client: the enrollment is the forwarded signed-in
//!    person's, and no service token or copy of their session is stored
//!    anywhere in this module or in the worker.
//! 4. Keep the [`Enrollment`] it answers — the endpoint's opaque id, the only
//!    handle revocation takes. On sign-out, while the person is still
//!    authenticated, call [`disable`]: the endpoint is revoked first, then the
//!    browser unsubscribes. A failure of either half is returned, never
//!    swallowed — a disable that couldn't revoke leaves the browser
//!    subscription alone so the whole sequence can be retried while still
//!    signed in, and the next account on this browser is refused
//!    (`EnrollError::Enroll`) until the old account's revoke lands. The service
//!    itself refuses to reassign a live destination to another account, so a
//!    switch can never quietly enroll the previous account's endpoint.
//!
//! Subscription changes the browser makes on its own (a rotated or
//! replaced subscription, `pushsubscriptionchange`) cannot be reconciled
//! from the worker: the worker deliberately holds no credentials, so it
//! has no identity to enroll with. The supported reconciliation is the
//! next authenticated app launch calling [`enroll`] again: the browser's
//! current subscription is registered under the person who is signed in,
//! and the replaced subscription's endpoint dies at its push service,
//! whose "unregistered" answer invalidates the stale record on its next
//! delivery. Nothing here reads or mutates read state: the worker shows
//! banners and routes clicks; the authenticated app marks what it reads.
//!
//! ```rust,ignore
//! use leptoskit_notifications::browser;
//!
//! // From a click handler, with the operator's VAPID public key — the
//! // private half never leaves the service:
//! let enrollment = browser::enroll(&browser::EnrollOptions::new(vapid_public_key)).await?;
//! // `enrollment.endpoint` is the opaque handle: keep it beside the
//! // person's session.
//! ```
//!
//! ```rust,ignore
//! // Signing out, while this person is still authenticated:
//! leptoskit_notifications::browser::disable(&enrollment).await?;
//! ```

pub use leptoskit_service_worker::RegisterOptions;
pub use leptoskit_service_worker::Registration;
/// The page-side worker API this module's workflow sits on, re-exported
/// so an app's enrollment code imports one path: register the receiver
/// ([`register`]), wait for it to control the page ([`ready`]), then
/// [`enroll`]. Outside a browser these answer the same `Unavailable`
/// errors [`leptoskit_service_worker`] documents.
pub use leptoskit_service_worker::get_registration;
pub use leptoskit_service_worker::ready;
pub use leptoskit_service_worker::register;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// The receiver script's template, with the inbox route left as a
/// placeholder. Build the servable script with [`receiver_script`], which
/// validates the route and substitutes it exactly once.
pub const RECEIVER_SCRIPT: &str = include_str!("browser/receiver.js");

/// The placeholder the template carries, quotes included, so the
/// substitution is a plain string replace and nothing else in the script
/// can be touched.
const ROUTE_PLACEHOLDER: &str = "\"__LKN_INBOX_ROUTE__\"";

/// The longest inbox route baked into a script, in bytes.
const MAX_ROUTE_LEN: usize = 512;

/// Builds the receiver script an app serves at its origin's root, with
/// `inbox_route` baked in as the only URL a notification click may open.
///
/// The route must be on this origin: a path beginning with a single `/`
/// (`"/inbox"`), at most [`MAX_ROUTE_LEN`] bytes, with no control
/// characters and no backslash. Anything else — an empty route, a
/// scheme-relative `"//host/path"` that would name another authority, a
/// full URL — is refused here rather than baked into the script. The
/// script checks the route against its own origin again at click time, so
/// even a hand-edited build cannot navigate anywhere off-origin.
///
/// # Errors
///
/// A message naming what's wrong with the route.
pub fn receiver_script(inbox_route: &str) -> Result<String, String> {
	check_route(inbox_route)?;

	// The route is substituted as a JSON string literal: quoting and
	// escaping are one serialization, and a route containing a quote or a
	// backslash still lands as one valid JavaScript string.
	let literal = serde_json::to_string(inbox_route).expect("a checked route is a JSON string");

	Ok(RECEIVER_SCRIPT.replace(ROUTE_PLACEHOLDER, &literal))
}

/// Checks an inbox route's shape before it's baked into a script.
///
/// # Errors
///
/// A message naming what's wrong.
fn check_route(route: &str) -> Result<(), String> {
	if route.is_empty() || route.len() > MAX_ROUTE_LEN {
		return Err(format!(
			"an inbox route is 1 to {MAX_ROUTE_LEN} bytes starting with \"/\""
		));
	}

	if !route.starts_with('/') {
		return Err("an inbox route is a path starting with \"/\"".into());
	}

	if route.starts_with("//") {
		return Err("an inbox route is on this origin: it may not name another authority".into());
	}

	if route.contains('\\') {
		return Err("an inbox route has no backslash".into());
	}

	if route.chars().any(char::is_control) {
		return Err("an inbox route has no control characters".into());
	}

	Ok(())
}

/// Where browser push stands on this device, as far as the page can see
/// before anything is enrolled. [`availability`] reads it from the
/// browser; outside a browser it is [`Availability::Unsupported`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
	/// A secure context with a service worker and the Push API, and no
	/// standing refusal: enrollment may be offered.
	Ready,
	/// The page is not a secure context: push needs `https`, or
	/// `localhost` while developing. Serving the app over `https` is the
	/// fix; nothing in code can work around it.
	InsecureContext,
	/// This browser has no service worker registration API or no Push
	/// API. Enrollment cannot be offered here; the durable inbox is the
	/// whole service.
	Unsupported,
	/// The person has denied notifications for this origin. Only they can
	/// undo it, in the browser's own site settings; an app's honest move
	/// is a settings screen that links there.
	Denied,
}

impl Availability {
	/// What the state means, for a settings screen's copy or an error
	/// message.
	#[must_use]
	pub fn reason(self) -> &'static str {
		match self {
			Self::Ready => "browser push is ready",
			Self::InsecureContext => {
				"this page is not a secure context: browser push needs https or localhost"
			}
			Self::Unsupported => "this browser has no service worker or Push API",
			Self::Denied => "notifications are denied for this origin",
		}
	}

	/// Classifies one browser's readings. The order is the order a
	/// settings screen can act on: fix the serving scheme first, then the
	/// browser, then the person's refusal. The host compiles it for the
	/// classification tests only.
	#[cfg(any(target_arch = "wasm32", test))]
	fn from_browser(
		secure_context: bool,
		service_worker: bool,
		push: bool,
		permission: Option<&str>,
	) -> Self {
		if !secure_context {
			return Self::InsecureContext;
		}

		if !service_worker || !push {
			return Self::Unsupported;
		}

		if permission == Some("denied") {
			return Self::Denied;
		}

		Self::Ready
	}
}

/// Reads where browser push stands on this device. Cheap and side-effect
/// free: it never prompts, so a settings screen may call it on every
/// render. Prompting is the browser's answer to `subscribe`, which only
/// [`enroll`] reaches, and only from a gesture the app already made.
#[must_use]
pub fn availability() -> Availability {
	#[cfg(target_arch = "wasm32")]
	{
		Availability::from_browser(
			secure_context(),
			has_service_worker(),
			has_push_manager(),
			notification_permission().as_deref(),
		)
	}

	#[cfg(not(target_arch = "wasm32"))]
	Availability::Unsupported
}

/// One enrolled browser: the opaque handle the service answered, and all
/// that [`disable`] needs later. Store it beside the person's session on
/// this device. The credential the service holds never appears here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enrollment {
	/// The endpoint's opaque id, as [`crate::push::EndpointSummary`]
	/// lists it.
	pub endpoint: uuid::Uuid,
}

/// What [`enroll`] takes: the public half of the operator's VAPID pair,
/// and, when the receiver was registered under an explicit scope, that
/// scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollOptions {
	/// The VAPID **public** key, base64url, exactly as the service's
	/// `LEPTOSKIT_NOTIFICATIONS_WEB_PUSH_VAPID_PUBLIC_KEY` names it. The
	/// private half is the operator's server secret and is never accepted
	/// here, never stored, and never sent anywhere.
	pub vapid_public_key: String,
	/// The scope the receiver script was registered under, when the app
	/// set one instead of the script path's default. `None` finds the
	/// registration that matches this page.
	pub worker_scope: Option<String>,
}

impl EnrollOptions {
	/// A user-visible enrollment for the given VAPID public key.
	#[must_use]
	pub fn new(vapid_public_key: impl Into<String>) -> Self {
		Self {
			vapid_public_key: vapid_public_key.into(),
			worker_scope: None,
		}
	}
}

/// Why enrollment didn't happen. Every variant is actionable, and none of
/// them can be retried into a second subscription: the browser's own
/// subscription is reused whatever the failure was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnrollError {
	/// The browser can't enroll here; [`availability`] says which fix
	/// applies.
	#[error("browser push is unavailable here: {0}")]
	Unavailable(String),
	/// The person has denied notifications for this origin. Offer the
	/// browser's site settings, not another prompt.
	#[error("notifications are denied for this origin")]
	Denied,
	/// No service worker registration exists yet: serve the receiver
	/// script and register it before enrolling. Nothing was prompted and
	/// nothing was subscribed.
	#[error("no service worker registration: register the receiver script first")]
	NoWorker,
	/// The browser refused the subscription: no user gesture behind the
	/// call, a bad VAPID key shape, or the registration can't push.
	#[error("the browser refused the subscription: {0}")]
	Subscribe(String),
	/// The enrollment call failed: the network, or the service's own
	/// answer — including the refusal that says another live account
	/// holds this browser's subscription, which that account's
	/// [`disable`] releases. The failure is surfaced, never swallowed:
	/// nothing was unsubscribed, and the previous account's endpoint
	/// stays the previous account's.
	#[error("enrolling failed: {0}")]
	Enroll(leptoskit_rpc::FnError),
}

/// Enrolls this browser for the signed-in person: one subscription, one
/// endpoint, and the opaque [`Enrollment`] handle to disable it with
/// later.
///
/// Call it from a user gesture. The flow is: [`availability`] first (a
/// refusal here prompts nothing), the active service worker awaited
/// rather than raced, the browser's existing subscription reused when it
/// holds one — so a second tap re-enrolls the same endpoint instead of
/// prompting again, and two racing calls converge on one subscription —
/// and the subscription registered as the person's own typed
/// [`crate::push::PushCredential`], over the app's RPC client as the
/// forwarded signed-in person. Re-enrolling the same browser under the
/// same account answers the same endpoint id, idempotently; under a
/// different account it is refused until the previous account's
/// [`disable`] has revoked the endpoint.
///
/// # Errors
///
/// [`EnrollError`] for each step's own failure.
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(clippy::unused_async, reason = "matches the browser signature")
)]
pub async fn enroll(options: &EnrollOptions) -> Result<Enrollment, EnrollError> {
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = options;
		Err(EnrollError::Unavailable(
			Availability::Unsupported.reason().to_owned(),
		))
	}

	#[cfg(target_arch = "wasm32")]
	enroll_in_browser(options).await
}

/// Why a disable stopped, and what state it left. Both halves are
/// surfaced: a revoke that failed leaves the browser subscription in
/// place so the sequence can be retried while still signed in, and an
/// unsubscribe that failed after a successful revoke leaves a browser
/// subscription that can push nothing further.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DisableError {
	/// The revoke failed, so the service still holds a live endpoint for
	/// this browser. Nothing was unsubscribed: retry the whole disable
	/// while this person is still authenticated.
	#[error("revoking the endpoint failed: {0}")]
	Revoke(leptoskit_rpc::FnError),
	/// The endpoint is revoked, but the browser couldn't unsubscribe
	/// (no registration on this page, or the browser refused). The
	/// service pushes nothing further to a revoked endpoint; the browser
	/// half can be cleaned up by unconditionally unsubscribing on next
	/// launch.
	#[error("the browser kept its subscription: {0}")]
	Unsubscribe(String),
	/// Not in a browser: nothing was revoked and nothing was
	/// unsubscribed, so a native build never strands an endpoint by
	/// calling this.
	#[error("browser push is unavailable here: {0}")]
	Unavailable(String),
}

/// Disables one enrollment before its person signs out: the endpoint is
/// revoked **while the person is still authenticated**, then the browser
/// unsubscribes. This order is the account-switch guarantee: an endpoint
/// the previous account never revoked is refused to the next one, so
/// nothing of an old account can be quietly re-enrolled. Failures are
/// returned, never swallowed.
///
/// # Errors
///
/// [`DisableError::Revoke`] when the service refused or the network
/// dropped the revoke — the browser subscription is left untouched for a
/// retry; [`DisableError::Unsubscribe`] when the revoke landed but the
/// browser half didn't.
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(clippy::unused_async, reason = "matches the browser signature")
)]
pub async fn disable(enrollment: &Enrollment) -> Result<(), DisableError> {
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = enrollment;
		Err(DisableError::Unavailable(
			Availability::Unsupported.reason().to_owned(),
		))
	}

	#[cfg(target_arch = "wasm32")]
	{
		// Revoke first: unsubscribing first would strand the service's
		// record behind a dead subscription exactly when the person can
		// no longer revoke it.
		leptoskit_rpc::client::call(crate::push::RevokePushEndpoint {
			endpoint: enrollment.endpoint,
		})
		.await
		.map_err(DisableError::Revoke)?;

		unsubscribe_in_browser()
			.await
			.map_err(DisableError::Unsubscribe)?;

		Ok(())
	}
}

#[cfg(target_arch = "wasm32")]
async fn enroll_in_browser(options: &EnrollOptions) -> Result<Enrollment, EnrollError> {
	match availability() {
		Availability::Ready => {}
		Availability::Denied => return Err(EnrollError::Denied),
		other => return Err(EnrollError::Unavailable(other.reason().to_owned())),
	}

	let registration = active_registration(options.worker_scope.as_deref()).await?;
	let push = registration
		.push_manager()
		.map_err(|error| EnrollError::Unavailable(error.to_string()))?;

	// The subscription the browser already holds is the one to enroll,
	// whatever account signed in last: reusing it is what keeps a second
	// tap from prompting again and two racing calls from forking the
	// browser's subscription. Only `subscribe` prompts, and only the
	// gesture that reached this call covers it.
	let subscription = match push.get_subscription().await {
		Ok(Some(existing)) => existing,
		Ok(None) => {
			push.subscribe(&leptoskit_service_worker::PushOptions::new(
				&options.vapid_public_key,
			))
			.await
			.map_err(|error| EnrollError::Subscribe(error.to_string()))?
		}
		Err(error) => return Err(EnrollError::Subscribe(error.to_string())),
	};

	// The person is read from the app's forwarded identity by the
	// service, never from anything this module holds.
	let summary = leptoskit_rpc::client::call(crate::push::RegisterPushEndpoint {
		credential: crate::push::PushCredential::WebPush {
			endpoint: subscription.endpoint,
			keys: crate::push::WebPushKeys {
				p256dh: subscription.keys.p256dh,
				auth: subscription.keys.auth,
			},
		},
	})
	.await
	.map_err(EnrollError::Enroll)?;

	Ok(Enrollment {
		endpoint: summary.id,
	})
}

/// The registration to enroll against, with its worker active. Subscribe
/// rides a registration whenever it was installed, but a worker that is
/// still installing shows nothing when the push arrives, so activation is
/// waited for instead of raced.
#[cfg(target_arch = "wasm32")]
async fn active_registration(scope: Option<&str>) -> Result<Registration, EnrollError> {
	let mut registration = get_registration(scope)
		.await
		.map_err(|error| EnrollError::Unavailable(error.to_string()))?
		.ok_or(EnrollError::NoWorker)?;

	if !has_active_worker(&registration) {
		// `ready` answers the page's own registration; with an explicit
		// scope, the scoped one is re-read once activation has settled,
		// falling back to the page's when the scope is still quiet.
		let ready = ready()
			.await
			.map_err(|error| EnrollError::Unavailable(error.to_string()))?;

		registration = if scope.is_some() {
			get_registration(scope)
				.await
				.map_err(|error| EnrollError::Unavailable(error.to_string()))?
				.filter(has_active_worker)
				.unwrap_or(ready)
		} else {
			ready
		};
	}

	Ok(registration)
}

#[cfg(target_arch = "wasm32")]
fn has_active_worker(registration: &Registration) -> bool {
	registration.active().is_ok_and(|worker| worker.is_some())
}

#[cfg(target_arch = "wasm32")]
async fn unsubscribe_in_browser() -> Result<(), String> {
	let registration = get_registration(None)
		.await
		.map_err(|error| error.to_string())?
		.ok_or_else(|| {
			"this page keeps no service worker registration, so its subscription couldn't be found"
				.to_owned()
		})?;

	let push = registration
		.push_manager()
		.map_err(|error| error.to_string())?;

	push.unsubscribe()
		.await
		.map_err(|error| error.to_string())?;
	Ok(())
}

#[cfg(target_arch = "wasm32")]
fn secure_context() -> bool {
	js_sys::Reflect::get(
		&js_sys::global(),
		&js_sys::JsString::from("isSecureContext"),
	)
	.ok()
	.and_then(|value| value.as_bool())
	.unwrap_or(false)
}

#[cfg(target_arch = "wasm32")]
fn has_service_worker() -> bool {
	js_sys::Reflect::get(&js_sys::global(), &js_sys::JsString::from("navigator"))
		.ok()
		.filter(wasm_bindgen::JsValue::is_object)
		.and_then(|navigator| {
			js_sys::Reflect::get(&navigator, &js_sys::JsString::from("serviceWorker")).ok()
		})
		.is_some_and(|worker| worker.is_object())
}

#[cfg(target_arch = "wasm32")]
fn has_push_manager() -> bool {
	js_sys::Reflect::get(&js_sys::global(), &js_sys::JsString::from("PushManager"))
		.is_ok_and(|value| !value.is_undefined() && !value.is_null())
}

/// The origin's `Notification.permission`, when the browser exposes the
/// interface. `None` means the check has no say: `subscribe` itself is
/// where a prompt would begin.
#[cfg(target_arch = "wasm32")]
fn notification_permission() -> Option<String> {
	let notification =
		js_sys::Reflect::get(&js_sys::global(), &js_sys::JsString::from("Notification"))
			.ok()
			.filter(wasm_bindgen::JsValue::is_object)?;

	js_sys::Reflect::get(&notification, &js_sys::JsString::from("permission"))
		.ok()?
		.as_string()
}

#[cfg(test)]
mod tests {
	use super::Availability;
	use super::RECEIVER_SCRIPT;
	use super::receiver_script;

	/// The route is checked before it's baked in: shapes an app might
	/// typo, and the one shape accepted.
	#[test]
	fn the_route_is_checked() {
		assert_eq!(
			receiver_script("").unwrap_err(),
			"an inbox route is 1 to 512 bytes starting with \"/\""
		);
		assert_eq!(
			receiver_script("inbox").unwrap_err(),
			"an inbox route is a path starting with \"/\""
		);
		assert_eq!(
			receiver_script("//evil.test/inbox").unwrap_err(),
			"an inbox route is on this origin: it may not name another authority"
		);
		assert_eq!(
			receiver_script("https://evil.test/inbox").unwrap_err(),
			"an inbox route is a path starting with \"/\""
		);
		assert_eq!(
			receiver_script("/in\\box").unwrap_err(),
			"an inbox route has no backslash"
		);
		assert_eq!(
			receiver_script("/in\nbox").unwrap_err(),
			"an inbox route has no control characters"
		);

		// A quote is escaped by the serialization rather than refused, and
		// a long but honest path passes at the bound.
		assert!(receiver_script("/in\"box").is_ok());
		assert!(receiver_script(&format!("/{}", "a".repeat(511))).is_ok());
		assert!(receiver_script(&format!("/{}", "a".repeat(512))).is_err());
	}

	/// The built script carries the route exactly once, escaped as a JSON
	/// string literal, and nothing else of the template moves.
	#[test]
	fn the_route_is_baked_in_once() {
		let script = receiver_script("/inbox").unwrap();
		assert!(script.contains("const ROUTE = \"/inbox\";"));
		assert!(!script.contains("__LKN_INBOX_ROUTE__"));
		assert_eq!(script.lines().count(), RECEIVER_SCRIPT.lines().count());

		// A quote in the route stays one string literal.
		let quoted = receiver_script("/a\"b").unwrap();
		assert!(quoted.contains("const ROUTE = \"/a\\\"b\";"));
	}

	/// The template carries exactly one placeholder, so a build can never
	/// substitute two routes or miss the only one.
	#[test]
	fn the_template_carries_one_placeholder() {
		assert_eq!(RECEIVER_SCRIPT.matches("__LKN_INBOX_ROUTE__").count(), 1);
	}

	/// Outside a browser, enrollment and disabling answer their
	/// unavailable errors without touching anything: no call is made, no
	/// endpoint is revoked, nothing is stranded.
	#[test]
	fn outside_a_browser_nothing_enrolls_and_nothing_is_stranded() {
		let options = super::availability();
		assert_eq!(options, Availability::Unsupported);

		let error = futures::executor::block_on(super::enroll(&super::EnrollOptions::new("BNc6")));
		assert_eq!(
			error,
			Err(super::EnrollError::Unavailable(
				Availability::Unsupported.reason().to_owned()
			))
		);

		let error = futures::executor::block_on(super::disable(&super::Enrollment {
			endpoint: uuid::Uuid::nil(),
		}));
		assert_eq!(
			error,
			Err(super::DisableError::Unavailable(
				Availability::Unsupported.reason().to_owned()
			))
		);
	}

	/// Availability is classified in the order a settings screen can act
	/// on it: serving scheme, browser, then the person's refusal.
	#[test]
	fn availability_is_classified_in_action_order() {
		let ready = Availability::from_browser(true, true, true, Some("granted"));
		assert_eq!(ready, Availability::Ready);
		assert_eq!(
			Availability::from_browser(true, true, true, Some("default")),
			Availability::Ready,
			"a prompt that hasn't happened is not a refusal"
		);
		assert_eq!(
			Availability::from_browser(true, true, true, None),
			Availability::Ready,
			"no readable permission has no say"
		);
		assert_eq!(
			Availability::from_browser(false, false, false, Some("denied")),
			Availability::InsecureContext,
			"the serving scheme is reported before anything else"
		);
		assert_eq!(
			Availability::from_browser(true, false, true, Some("granted")),
			Availability::Unsupported
		);
		assert_eq!(
			Availability::from_browser(true, true, false, Some("granted")),
			Availability::Unsupported
		);
		assert_eq!(
			Availability::from_browser(true, true, true, Some("denied")),
			Availability::Denied
		);
		assert_eq!(
			Availability::Unsupported.reason(),
			"this browser has no service worker or Push API"
		);
	}
}
