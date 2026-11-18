//! The browser half of Web Push enrollment: the receiver worker, the
//! availability check, and the enroll and disable flows an app's web
//! client runs on its person's behalf.
//!
//! The service side of Web Push is the transport in [`crate::service`]:
//! it pushes to enrolled subscriptions with the person's own VAPID
//! identity. What was missing is the client half — the service worker
//! that shows what arrives, and the page code that turns the browser's
//! subscription into a [`crate::push::PushCredential`] — and that is this
//! module. It exists behind the `browser` feature, which carries its own
//! browser transport (`leptoskit_rpc/web`'s same-origin `fetch`), adds no
//! server dependency (the tree is checked by `tests/dependencies.rs`),
//! and every call outside a browser answers the plain "unavailable"
//! errors the [`leptoskit_service_worker`] crate does, so the same app
//! code compiles on native targets and during server rendering.
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
//!    for an active worker. Nothing in this module registers for you: offline
//!    caching rules and scopes stay yours.
//! 3. On **launch**, once the person is signed in, call [`reconcile`] with the
//!    [`Enrollment`] the app saved beside their session when they turned
//!    notifications on — and only with one. Without it nothing registers and
//!    the answer is `None`, so a browser someone else left a subscription in is
//!    never enrolled for the next person by startup code. With it, the
//!    subscription the browser holds is renewed under that enrollment: the
//!    service registers it only while the enrollment still stands, and answers
//!    `None` once it doesn't — revoked from another device, taken over by the
//!    browser's next person, or erased with the person. It never prompts —
//!    reading a subscription needs no permission — so it belongs on startup
//!    paths where a prompt is not allowed. `None` is the app's cue to clear
//!    what it saved and offer the switch.
//! 4. On an **explicit user gesture** — a "turn on notifications" button, never
//!    a mount or a render — call [`enroll`]. It checks [`availability`], waits
//!    for the **active** worker instead of racing its install, and subscribes
//!    only when the browser holds no subscription: a subscription that exists
//!    is reused — and only when it was created for this app's own VAPID key
//!    ([`EnrollError::KeyMismatch`] otherwise, because a subscription made for
//!    one sender cannot receive another's pushes). The result is registered
//!    with [`crate::push::register_push_endpoint`] over the app's own RPC
//!    client: the enrollment is the forwarded signed-in person's, and no
//!    service token or copy of their session is stored anywhere in this module
//!    or in the worker. Call [`ready`] at startup so the worker is settled long
//!    before the gesture: `subscribe` is the one call whose permission the
//!    browser ties to the gesture, and a worker still installing when the
//!    gesture arrives can eat the gesture's window. A browser whose previous
//!    person never signed out passes to this one here: presenting the same
//!    subscription takes the endpoint over, and the previous person's is
//!    revoked.
//!
//!    Every enrollment carries the browser's details — `navigator.language` and
//!    the `Intl` time zone — so a send can target devices with `device.`
//!    conditions, with no code. A browser has no app version of its own: name
//!    yours with [`EnrollOptions::app_version`] to make `device.app_version`
//!    mean something on the web.
//! 5. Keep the [`Enrollment`] endpoint id beside the person's session. Call
//!    [`disable`] while still authenticated before signing out. It requires an
//!    acknowledged server revoke and keeps the origin subscription and OS
//!    permission reusable. Re-enabling can reuse that subscription.
//!
//! Explicit [`EnrollOptions::worker_scope`] scopes are matched exactly during
//! enrollment and reconciliation.
//!
//! Subscription changes the browser makes on its own (a rotated or
//! replaced subscription, `pushsubscriptionchange`) cannot be reconciled
//! from the worker: the worker deliberately holds no credentials, so it
//! has no identity to enroll with. The supported reconciliation is the
//! next authenticated app launch calling [`reconcile`] again: the
//! browser's current subscription is registered under the person whose
//! enrollment it renews, and a stale handle's [`disable`] leaves a replaced
//! subscription alone — its endpoint dies at its push service, whose
//! "unregistered" answer invalidates the stale record on its next
//! delivery. Nothing here reads or mutates read state: the worker shows
//! banners and routes clicks; the authenticated app marks what it reads.
//!
//! ```rust,ignore
//! use leptoskit_notifications::browser;
//!
//! // At launch, after register and ready, without prompting. `saved` is
//! // the enrollment kept beside this person's session, if they opted in:
//! let options = browser::EnrollOptions::new(vapid_public_key);
//! match browser::reconcile(&options, saved.as_ref()).await? {
//!     // Still enrolled: keep the handle.
//!     Some(enrollment) => save(enrollment),
//!     None => {
//!         // Not enrolled, or no longer: clear what was saved and offer
//!         // the switch. From its click handler — with the operator's
//!         // VAPID public key, whose private half never leaves the
//!         // service — enroll, and keep the handle beside the session:
//!         forget_saved();
//!         let enrollment = browser::enroll(&options).await?;
//!         save(enrollment);
//!     }
//! }
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
/// ([`register`]), wait for it to be active ([`ready`]), then
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

/// The server endpoint identity needed to disable this person's enrollment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enrollment {
	/// The endpoint's opaque id, as [`crate::push::EndpointSummary`] lists it.
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
	/// registration that matches this page. The match is exact: a worker
	/// registered for a broader scope never stands in for the one asked
	/// for, because [`EnrollError::Scope`] would be better than enrolling
	/// against a worker the app didn't select.
	pub worker_scope: Option<String>,
	/// The app's version, as a condition's `device.app_version` reads it:
	/// the one device detail a browser can't report on its own. `None`
	/// sends none. The locale and time zone need nothing — enrollment reads
	/// `navigator.language` and the browser's `Intl` time zone itself.
	pub app_version: Option<String>,
}

impl EnrollOptions {
	/// A user-visible enrollment for the given VAPID public key.
	#[must_use]
	pub fn new(vapid_public_key: impl Into<String>) -> Self {
		Self {
			vapid_public_key: vapid_public_key.into(),
			worker_scope: None,
			app_version: None,
		}
	}

	/// The same options, sending `version` as the app's version — say
	/// `env!("CARGO_PKG_VERSION")` — so a send can target this web app's
	/// versions the way it targets a native app's.
	#[must_use]
	pub fn app_version(mut self, version: impl Into<String>) -> Self {
		self.app_version = Some(version.into());
		self
	}
}

/// The details a browser enrollment sends, from what the browser reports —
/// `navigator.language` and the `Intl` time zone — and the app's own
/// version, [sanitized](crate::push::DeviceDetails::sanitized): a detail
/// the service would refuse is dropped rather than costing the browser its
/// enrollment.
#[must_use]
pub fn device_details(
	language: Option<String>,
	time_zone: Option<String>,
	app_version: Option<String>,
) -> crate::push::DeviceDetails {
	crate::push::DeviceDetails {
		locale: language,
		time_zone,
		app_version,
	}
	.sanitized()
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
	/// No registration covers the requested scope. `getRegistration`
	/// answers with whatever registration's scope contains the request,
	/// so a broader worker — the origin's root, say — can answer for a
	/// scope it doesn't cover; this enrollment refuses that stand-in.
	/// Register a worker whose scope is exactly `{0}`, or enroll without
	/// a scope to use the page's own.
	#[error("no registration with scope {0}: a broader worker doesn't stand in for it")]
	Scope(String),
	/// The browser refused the subscription: no user gesture behind the
	/// call, a bad VAPID key shape, or the registration can't push.
	#[error("the browser refused the subscription: {0}")]
	Subscribe(String),
	/// The browser holds a subscription created for a different
	/// application server key — another sender's, or this app's previous
	/// VAPID identity — and a subscription made for one key cannot receive
	/// another's pushes, so claiming enrollment would end in silent
	/// delivery failure. Disable the subscription where it was enrolled,
	/// or unsubscribe it, and enroll again.
	#[error("the browser holds a subscription for a different application server key")]
	KeyMismatch,
	/// The enrollment call failed: the network, or the service's own
	/// answer — the endpoint quota, or the refusal that says another
	/// account holds this subscription's URL under different keys. The
	/// failure is surfaced, never swallowed: nothing was unsubscribed.
	#[error("enrolling failed: {0}")]
	Enroll(leptoskit_rpc::FnError),
}

/// Enrolls this browser for the signed-in person: one subscription, one
/// endpoint, and the opaque [`Enrollment`] handle to disable it with
/// later.
///
/// Call it from a user gesture. The flow is: [`availability`] first (a
/// refusal here prompts nothing), the selected worker's activation
/// awaited rather than raced, the browser's existing subscription reused
/// when it holds one **for this app's own key** — so a second tap
/// re-enrolls the same endpoint instead of prompting again, and two
/// racing calls converge on one subscription — and only when it holds
/// none, `subscribe`, the one call whose permission the browser ties to
/// the gesture. The subscription is registered as the person's own typed
/// [`crate::push::PushCredential`], over the app's RPC client as the
/// forwarded signed-in person. Re-enrolling the same browser under the
/// same account answers the same endpoint id, idempotently; under a
/// different account the subscription changes hands — this person holds
/// the browser now — and the previous account's endpoint is revoked, so
/// its notifications stop arriving here.
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
	{
		let (registration, held) = held_subscription(options).await?;
		let subscription = match held {
			Held::Subscription(existing) => existing,
			Held::Absent => {
				registration
					.push_manager()
					.map_err(|error| EnrollError::Unavailable(error.to_string()))?
					.subscribe(&leptoskit_service_worker::PushOptions::new(
						&options.vapid_public_key,
					))
					.await
					.map_err(|error| EnrollError::Subscribe(error.to_string()))?
			}
		};

		register_subscription(&subscription, options, None).await
	}
}

/// The nonprompting half of enrollment, for app startup — but only for a
/// browser this person previously opted in on: `previous` is the
/// [`Enrollment`] the app saved beside their session when [`enroll`]
/// succeeded. Without it the answer is `None` and nothing is read or
/// registered, so a subscription another account left in this browser is
/// never enrolled for the next person, and a person who turned
/// notifications off is never silently turned back on.
///
/// With it, the subscription the browser holds — if it holds one for this
/// app's key — is renewed under that enrollment, with the browser's
/// current details. The service registers it only while the enrollment
/// still stands. When it doesn't — it was revoked from another device or
/// by the app's server, taken over by the browser's next person, or erased
/// with the person — the answer is `None` and nothing is registered: a
/// launch never re-creates what the server revoked. `None` is also the
/// answer when the browser holds no subscription anymore. Either way it is
/// the app's cue to clear the saved enrollment and offer the switch, whose
/// gesture-gated [`enroll`] can make a new one.
///
/// Reading a subscription needs no permission and prompts nothing, so
/// this belongs on launch paths where a prompt is forbidden. The worker's
/// activation is still waited for, the key still matched
/// ([`EnrollError::KeyMismatch`]), and the registration still the person's
/// own typed credential.
///
/// # Errors
///
/// As [`enroll`], minus the prompts: there is no `Subscribe` failure,
/// because nothing subscribes. A failed call leaves the saved enrollment
/// as it was; the next launch tries again.
#[cfg_attr(
	not(target_arch = "wasm32"),
	allow(clippy::unused_async, reason = "matches the browser signature")
)]
pub async fn reconcile(
	options: &EnrollOptions,
	previous: Option<&Enrollment>,
) -> Result<Option<Enrollment>, EnrollError> {
	#[cfg(not(target_arch = "wasm32"))]
	{
		let _ = (options, previous);
		Err(EnrollError::Unavailable(
			Availability::Unsupported.reason().to_owned(),
		))
	}

	#[cfg(target_arch = "wasm32")]
	{
		let Some(previous) = previous else {
			return Ok(None);
		};

		match held_subscription(options).await? {
			(_, Held::Absent) => Ok(None),
			(_, Held::Subscription(subscription)) => {
				match register_subscription(&subscription, options, Some(previous.endpoint)).await {
					Ok(enrollment) => Ok(Some(enrollment)),
					// The service no longer has the enrollment this launch
					// renews, and registered nothing in its place.
					Err(EnrollError::Enroll(cause)) if crate::push::enrollment_gone(&cause) => {
						Ok(None)
					}
					Err(error) => Err(error),
				}
			}
		}
	}
}

/// Why disabling the service enrollment was not confirmed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DisableError {
	/// No successful revoke acknowledgement: retry while authenticated.
	#[error("revoking the endpoint failed: {0}")]
	Revoke(leptoskit_rpc::FnError),
	/// Outside a browser; no revoke was attempted.
	#[error("browser push is unavailable here: {0}")]
	Unavailable(String),
}

/// Disables this person's service enrollment while still authenticated.
///
/// Never unsubscribes the browser or changes OS permission. Revocation stops
/// future attempts under the service's revocation boundary; provider deliveries
/// already in flight and visible banners are not recalled. The subscription
/// remains reusable, but this service requires enrollment before sending.
/// Origin-wide unsubscribe and permission revocation belong in browser settings
/// or the lower-level worker API, separately from this account's enrollment.
///
/// Retries use the same endpoint-derived idempotency key, including after
/// reload. Receipts belong to the authenticated caller; replay works only while
/// the service retains the receipt. A pruned receipt may leave a retry
/// unconfirmed.
///
/// # Errors
///
/// [`DisableError::Revoke`] for every unconfirmed revoke, including not-found.
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
	revoke_endpoint(enrollment.endpoint).await
}

/// What the browser holds for this app's key when enrollment looks.
#[cfg(target_arch = "wasm32")]
enum Held {
	/// A subscription created for this app's application server key: the
	/// one to enroll.
	Subscription(leptoskit_service_worker::PushSubscription),
	/// No subscription: only a gesture may make one.
	Absent,
}

/// Reads where enrollment stands: availability, the selected worker
/// activated, and the subscription the browser holds for this app's key.
#[cfg(target_arch = "wasm32")]
async fn held_subscription(options: &EnrollOptions) -> Result<(Registration, Held), EnrollError> {
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
	// gesture that reached this call covers it. A subscription is only
	// reusable for the key it was created for, so anything else is an
	// error rather than an enrollment that could never deliver.
	match push.get_subscription().await {
		Ok(Some(existing)) => {
			expect_senders_key(&existing, &options.vapid_public_key)?;
			Ok((registration, Held::Subscription(existing)))
		}
		Ok(None) => Ok((registration, Held::Absent)),
		Err(error) => Err(EnrollError::Subscribe(error.to_string())),
	}
}

/// Registers one subscription as the signed-in person's, with the
/// browser's details, and answers the handle its disable needs.
///
/// The person is read from the app's forwarded identity by the service,
/// never from anything this module holds. `renews` names the enrollment a
/// launch renews, so the service registers nothing once that is gone; a
/// gesture's enrollment passes none.
#[cfg(target_arch = "wasm32")]
async fn register_subscription(
	subscription: &leptoskit_service_worker::PushSubscription,
	options: &EnrollOptions,
	renews: Option<uuid::Uuid>,
) -> Result<Enrollment, EnrollError> {
	let summary = leptoskit_rpc::client::call(crate::push::RegisterPushEndpoint {
		credential: crate::push::PushCredential::WebPush {
			endpoint: subscription.endpoint.clone(),
			keys: crate::push::WebPushKeys {
				p256dh: subscription.keys.p256dh.clone(),
				auth: subscription.keys.auth.clone(),
			},
		},
		device: device_details(
			navigator_language(),
			intl_time_zone(),
			options.app_version.clone(),
		),
		renews,
	})
	.await
	.map_err(EnrollError::Enroll)?;

	Ok(Enrollment {
		endpoint: summary.id,
	})
}

/// `navigator.language`: the browser's preferred language tag.
#[cfg(target_arch = "wasm32")]
fn navigator_language() -> Option<String> {
	let navigator =
		js_sys::Reflect::get(&js_sys::global(), &js_sys::JsString::from("navigator")).ok()?;

	js_sys::Reflect::get(&navigator, &js_sys::JsString::from("language"))
		.ok()?
		.as_string()
}

/// `Intl.DateTimeFormat().resolvedOptions().timeZone`: the IANA name the
/// browser resolves the local time zone to.
#[cfg(target_arch = "wasm32")]
fn intl_time_zone() -> Option<String> {
	let resolved = js_sys::Intl::DateTimeFormat::new(&js_sys::Array::new(), &js_sys::Object::new())
		.resolved_options();

	js_sys::Reflect::get(&resolved, &js_sys::JsString::from("timeZone"))
		.ok()?
		.as_string()
}

/// The registration to enroll against, with its worker active. Subscribe
/// rides a registration whenever it was installed, but a worker that is
/// still installing shows nothing when the push arrives, so activation is
/// waited for instead of raced — for the registration that was selected,
/// never a stand-in: `ready` would answer the page's own, and
/// `getRegistration` answers any scope it covers, so with an explicit scope
/// the answer's scope is verified to be exactly the one asked for.
#[cfg(target_arch = "wasm32")]
async fn active_registration(scope: Option<&str>) -> Result<Registration, EnrollError> {
	let unavailable =
		|error: leptoskit_service_worker::Error| EnrollError::Unavailable(error.to_string());

	let registration = match scope {
		None => {
			get_registration(None)
				.await
				.map_err(unavailable)?
				.ok_or(EnrollError::NoWorker)?
		}
		Some(requested) => {
			let found = get_registration(Some(requested))
				.await
				.map_err(unavailable)?
				.ok_or(EnrollError::NoWorker)?;

			let scope = found.scope().map_err(unavailable)?;
			if scope != resolved_scope(requested)? {
				return Err(EnrollError::Scope(requested.to_owned()));
			}

			found
		}
	};

	registration
		.wait_until_active()
		.await
		.map_err(|error| EnrollError::Unavailable(error.to_string()))?;
	Ok(registration)
}

/// The absolute URL a scope string names on this page's origin, resolved
/// the same way the browser resolves the client URL `getRegistration` is
/// handed, so the comparison with the registration's own scope is exact.
#[cfg(target_arch = "wasm32")]
fn resolved_scope(scope: &str) -> Result<String, EnrollError> {
	use wasm_bindgen::JsCast;

	let page = js_sys::Reflect::get(&js_sys::global(), &js_sys::JsString::from("location"))
		.ok()
		.and_then(|location| js_sys::Reflect::get(&location, &js_sys::JsString::from("href")).ok())
		.and_then(|href| href.as_string())
		.ok_or_else(|| EnrollError::Unavailable("the page has no location".to_owned()))?;

	let constructor = js_sys::Reflect::get(&js_sys::global(), &js_sys::JsString::from("URL"))
		.ok()
		.and_then(|value| value.dyn_into::<js_sys::Function>().ok())
		.ok_or_else(|| EnrollError::Unavailable("URL".to_owned()))?;

	let resolved = js_sys::Reflect::construct(
		&constructor,
		&js_sys::Array::of2(
			&wasm_bindgen::JsValue::from_str(scope),
			&wasm_bindgen::JsValue::from_str(&page),
		),
	)
	.map_err(|error| EnrollError::Unavailable(js_message(&error)))?;

	js_sys::Reflect::get(&resolved, &js_sys::JsString::from("href"))
		.ok()
		.and_then(|href| href.as_string())
		.ok_or_else(|| EnrollError::Unavailable("the scope resolved to no URL".to_owned()))
}

/// The message of a JavaScript error, as the service worker crate reads
/// it.
#[cfg(target_arch = "wasm32")]
fn js_message(error: &wasm_bindgen::JsValue) -> String {
	js_sys::Reflect::get(error, &js_sys::JsString::from("message"))
		.ok()
		.and_then(|message| message.as_string())
		.or_else(|| error.as_string())
		.unwrap_or_else(|| "the browser refused".to_owned())
}

/// Whether the subscription was created for `vapid_public_key`'s sender.
/// The browser reports the commissioned key on the subscription's
/// options; a subscription made for one key cannot receive another's
/// pushes, and re-subscribing to replace it would silently destroy
/// whatever sender holds it — so a mismatch is an error instead.
#[cfg(target_arch = "wasm32")]
fn expect_senders_key(
	subscription: &leptoskit_service_worker::PushSubscription,
	vapid_public_key: &str,
) -> Result<(), EnrollError> {
	let expected = decoded_key(vapid_public_key)?;

	if subscription.application_server_key.as_ref() == Some(&expected) {
		Ok(())
	} else {
		Err(EnrollError::KeyMismatch)
	}
}

/// A base64url VAPID public key as the bytes the browser compares, with
/// or without padding.
#[cfg(target_arch = "wasm32")]
fn decoded_key(key: &str) -> Result<Vec<u8>, EnrollError> {
	use base64::Engine as _;

	base64::engine::general_purpose::URL_SAFE_NO_PAD
		.decode(key.trim_end_matches('='))
		.map_err(|error| {
			EnrollError::Subscribe(format!("the VAPID public key isn't base64url: {error}"))
		})
}

/// Requires the authenticated service's acknowledgement, never a list
/// inference.
#[cfg(target_arch = "wasm32")]
async fn revoke_endpoint(endpoint: uuid::Uuid) -> Result<(), DisableError> {
	leptoskit_rpc::client::call_with(
		crate::push::RevokePushEndpoint { endpoint },
		leptoskit_rpc::client::CallOptions::new()
			.idempotency_key(format!("notifications:disable:{endpoint}")),
	)
	.await
	.map_err(DisableError::Revoke)
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
			// `Notification` is a constructor, and a constructor is a
			// function: filtering for objects alone would never see it.
			.filter(|value| value.is_object() || value.is_function())?;

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

	/// Outside a browser, enrollment, reconciliation, and disabling
	/// answer their unavailable errors without touching anything: no call
	/// is made, no endpoint is revoked, nothing is stranded. (These are
	/// the non-browser stubs, so they run off the browser target.)
	#[cfg(not(target_arch = "wasm32"))]
	#[test]
	fn outside_a_browser_nothing_enrolls_and_nothing_is_stranded() {
		let options = super::availability();
		assert_eq!(options, Availability::Unsupported);

		let options = super::EnrollOptions::new("BNc6");
		let error = futures::executor::block_on(super::enroll(&options));
		assert_eq!(
			error,
			Err(super::EnrollError::Unavailable(
				Availability::Unsupported.reason().to_owned()
			))
		);

		let error = futures::executor::block_on(super::reconcile(&options, Some(&an_enrollment())));
		assert_eq!(
			error,
			Err(super::EnrollError::Unavailable(
				Availability::Unsupported.reason().to_owned()
			))
		);

		let error = futures::executor::block_on(super::disable(&an_enrollment()));
		assert_eq!(
			error,
			Err(super::DisableError::Unavailable(
				Availability::Unsupported.reason().to_owned()
			))
		);
	}

	/// Availability is classified in the order a settings screen can act
	/// on: serving scheme, browser, then the person's refusal.
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

	/// What the browser reports is sent sanitized: `navigator.language`
	/// in canonical case, the `Intl` time zone, and the version the app
	/// named — and a detail the service would refuse is dropped, never the
	/// enrollment. A browser reports no version on its own.
	#[test]
	fn browser_details_are_sanitized_before_they_are_sent() {
		use crate::push::DeviceDetails;

		assert_eq!(
			super::device_details(
				Some("en-gb".into()),
				Some("Europe/London".into()),
				Some("2.1.0".into())
			),
			DeviceDetails {
				locale: Some("en-GB".into()),
				time_zone: Some("Europe/London".into()),
				app_version: Some("2.1.0".into()),
			}
		);
		assert_eq!(
			super::device_details(Some("not a tag".into()), Some("UTC".into()), None),
			DeviceDetails {
				locale: None,
				time_zone: Some("UTC".into()),
				app_version: None,
			}
		);

		let options = super::EnrollOptions::new("key");
		assert_eq!(
			options.app_version, None,
			"no version unless the app names one"
		);
		assert_eq!(
			options.app_version("2.1.0").app_version.as_deref(),
			Some("2.1.0")
		);
	}

	/// A handle for tests: opaque as the browser would have answered it.
	#[cfg(not(target_arch = "wasm32"))]
	fn an_enrollment() -> super::Enrollment {
		super::Enrollment {
			endpoint: "6ec0f4b1-0000-7000-8000-000000000001"
				.parse()
				.expect("a uuid"),
		}
	}
}
