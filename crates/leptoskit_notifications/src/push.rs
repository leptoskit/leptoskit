//! Push delivery: enrolling the devices a person's app runs on, and the
//! per-endpoint record of what the service pushed to each one.
//!
//! Enrollment is a signed-in person's own act: their app hands the service
//! the credential its platform issued — an APNs device token, an FCM
//! registration token, or a Web Push subscription — and the service stores
//! it under their id, never under an argument's. The credential itself is
//! secret material: it never appears in a list, an error, a status row, or
//! a `Debug` string, and only its owner may revoke it.
//!
//! A send fans one delivery row per recipient's live endpoint, and a
//! background job attempts each one: retried on transient failures with
//! backoff, ended on permanent ones, skipped when the notification's
//! schedule, cancellation, expiry, or a fresh opt-out says so at the
//! moment of the attempt. Delivery is at-least-once: the same
//! notification may reach a device twice, and a provider's acceptance is
//! not a person's receipt. Every attempt carries a stable collapse id —
//! the notification's own id — so repeats replace rather than stack.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use leptoskit_db::table;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// The most endpoints one person may hold live, so enrollment stays
/// bounded and a send's fanout with it. Registering past this refuses, and
/// a re-registration of a held credential is idempotent.
pub const MAX_ENDPOINTS: usize = 10;

/// Additional invalidated entries allowed per endpoint-list page. Ownership
/// records stay until explicit revocation, even after provider invalidation.
pub const MAX_ENDPOINT_HISTORY: usize = 10;

/// The longest APNs device token, in hex characters. Tokens are 64 today;
/// the bound leaves room for longer ones rather than guessing a ceiling.
pub const MAX_APNS_TOKEN_CHARS: usize = 200;

/// The shortest APNs device token, in hex characters.
pub const MIN_APNS_TOKEN_CHARS: usize = 32;

/// The longest FCM registration token, in bytes.
pub const MAX_FCM_TOKEN_LEN: usize = 4_096;

/// The longest Web Push subscription endpoint URL, in bytes.
pub const MAX_WEB_ENDPOINT_LEN: usize = 2_048;

/// The most bytes a provider's answer is read. Every response is a
/// bounded status object, and reading past this would let a strange
/// answer grow the service's memory.
#[cfg(feature = "service")]
pub(crate) const MAX_RESPONSE_BYTES: usize = 16 * 1_024;

/// How many times one delivery is attempted, in all: the first try plus
/// this many retries. Past it the delivery ends rejected.
#[cfg(feature = "service")]
pub(crate) const MAX_DELIVERY_ATTEMPTS: u32 = 8;

/// How many delivery attempts run at once inside one job round.
#[cfg(feature = "service")]
pub(crate) const DELIVERY_CONCURRENCY: usize = 32;

/// The longest a transient failure may push an attempt out, whichever
/// header asked: past this the delivery is retried at this horizon
/// instead of being held hostage by a provider's number.
#[cfg(feature = "service")]
pub(crate) const MAX_RETRY_DELAY_SECS: i64 = 6 * 3_600;

/// Which platform's push service an endpoint speaks to. The platform is
/// decided by the credential the app presents, and everything a provider
/// needs besides the credential — the APNs topic and environment, the FCM
/// project, the VAPID identity — is the operator's server-side
/// configuration, never the caller's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PushPlatform {
	/// Apple Push Notification service, by device token. The app's bundle
	/// id (topic) and sandbox or production server are configured on the
	/// service, not asked of the caller.
	Apns,
	/// Firebase Cloud Messaging, by registration token, over the HTTP v1
	/// API. The project comes from the operator's service account.
	Fcm,
	/// Web Push, by the subscription the browser handed the app: an
	/// endpoint URL plus the keys to encrypt for it.
	WebPush,
}

impl PushPlatform {
	/// The platform's wire name, as stored and reported.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Apns => "apns",
			Self::Fcm => "fcm",
			Self::WebPush => "web_push",
		}
	}

	/// The platform a stored name stands for, or `None` for anything else.
	#[must_use]
	pub fn from_str_value(name: &str) -> Option<Self> {
		match name {
			"apns" => Some(Self::Apns),
			"fcm" => Some(Self::Fcm),
			"web_push" => Some(Self::WebPush),
			_ => None,
		}
	}
}

/// The Web Push subscription's browser keys, base64url without padding,
/// exactly as `PushSubscription.toJSON()` handed them over. Secret
/// material: the `Debug` print shows none of it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WebPushKeys {
	/// The browser's P-256 public key, 65 bytes uncompressed.
	pub p256dh: String,
	/// The authentication secret, 16 bytes.
	pub auth: String,
}

impl std::fmt::Debug for WebPushKeys {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WebPushKeys")
			.field("p256dh", &"<redacted>")
			.field("auth", &"<redacted>")
			.finish()
	}
}

/// The credential an app presents to enroll one endpoint: the thing its
/// platform issued and the only thing the service needs to push to it.
/// Anything else — topics, projects, VAPID identity — is server
/// configuration, so a caller can't steer where a push really goes.
/// Secret material: the `Debug` print shows only the platform.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PushCredential {
	/// An APNs device token, hex.
	Apns {
		/// The token, 32–200 hex characters.
		device_token: String,
	},
	/// An FCM registration token.
	Fcm {
		/// The token, at most [`MAX_FCM_TOKEN_LEN`] bytes of printable
		/// ASCII.
		registration_token: String,
	},
	/// A Web Push subscription.
	WebPush {
		/// The push service's endpoint URL for this browser.
		endpoint: String,
		/// The browser's keys.
		keys: WebPushKeys,
	},
}

impl std::fmt::Debug for PushCredential {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			Self::Apns { .. } => "Apns { device_token: <redacted> }",
			Self::Fcm { .. } => "Fcm { registration_token: <redacted> }",
			Self::WebPush { .. } => "WebPush { endpoint: <redacted>, keys: <redacted> }",
		})
	}
}

impl PushCredential {
	/// Which platform this credential belongs to.
	#[must_use]
	pub fn platform(&self) -> PushPlatform {
		match self {
			Self::Apns { .. } => PushPlatform::Apns,
			Self::Fcm { .. } => PushPlatform::Fcm,
			Self::WebPush { .. } => PushPlatform::WebPush,
		}
	}

	/// Where this credential delivers: the identity an endpoint's
	/// ownership turns on, separate from the credential's whole content.
	/// A device token or an FCM token *is* its destination; a Web Push
	/// subscription's destination is its endpoint URL alone, so the same
	/// browser stays the same endpoint through a key change while a
	/// different account can never take a live URL over by presenting new
	/// keys.
	///
	/// # Errors
	///
	/// [`check_credential`]'s message.
	pub fn destination(&self) -> Result<String, String> {
		match self {
			Self::Apns { device_token } => Ok(device_token.trim().to_ascii_lowercase()),
			Self::Fcm { registration_token } => Ok(registration_token.trim().to_owned()),
			Self::WebPush { endpoint, .. } => {
				check_web_endpoint(endpoint)?;
				let url = url::Url::parse(endpoint.trim())
					.map_err(|_| "a Web Push endpoint is an absolute https URL".to_owned())?;
				Ok(url.into())
			}
		}
	}

	/// The canonical form the service stores and pushes with: hex folded
	/// to lower case, tokens trimmed, and a Web Push subscription
	/// re-serialized with its URL parsed and its keys decoded and
	/// re-encoded unpadded, so padded and unpadded subscriptions from
	/// different browsers or SDK versions store the same bytes. Two
	/// enrollments of the same credential agree on it whatever their
	/// input's case or padding.
	///
	/// # Errors
	///
	/// [`check_credential`]'s message.
	pub fn canonical(&self) -> Result<String, String> {
		match self {
			Self::Apns { device_token } => Ok(device_token.trim().to_ascii_lowercase()),
			Self::Fcm { registration_token } => Ok(registration_token.trim().to_owned()),
			Self::WebPush { endpoint, keys } => canonical_web_subscription(endpoint, keys),
		}
	}
}

/// A Web Push subscription as the service stores it, and as a browser's
/// `PushSubscription.toJSON()` hands it over: the endpoint URL and the
/// keys, so what's stored is what the client saw. Secret material: the
/// `Debug` print shows none of it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WebSubscription {
	/// The push service's endpoint URL.
	pub endpoint: String,
	/// The browser's keys.
	pub keys: WebPushKeys,
}

impl std::fmt::Debug for WebSubscription {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WebSubscription")
			.field("endpoint", &"<redacted>")
			.field("keys", &"<redacted>")
			.finish()
	}
}

/// The Web Push subscription the service stores: the URL re-serialized by
/// the parser and the keys decoded and re-encoded without padding, so
/// equivalent subscriptions store the same bytes whatever form the
/// browser's SDK handed over.
///
/// # Errors
///
/// [`check_web_endpoint`] or [`check_web_keys`]'s message.
fn canonical_web_subscription(endpoint: &str, keys: &WebPushKeys) -> Result<String, String> {
	check_web_endpoint(endpoint)?;
	check_web_keys(keys)?;

	let url = url::Url::parse(endpoint.trim())
		.map_err(|_| "a Web Push endpoint is an absolute https URL".to_owned())?;
	let subscription = WebSubscription {
		endpoint: url.into(),
		keys: WebPushKeys {
			p256dh: encode_canonical_key(&keys.p256dh, "the p256dh key is base64url")?,
			auth: encode_canonical_key(&keys.auth, "the auth secret is base64url")?,
		},
	};

	serde_json::to_string(&subscription)
		.map_err(|error| format!("the subscription won't store: {error}"))
}

/// One key's bytes, re-encoded as unpadded base64url: the one form the
/// service stores, whatever padding the input carried.
///
/// # Errors
///
/// The message naming the key, when it doesn't decode.
fn encode_canonical_key(key: &str, complaint: &str) -> Result<String, String> {
	let decoded = base64_url(key).ok_or_else(|| complaint.to_owned())?;

	Ok(URL_SAFE_NO_PAD.encode(decoded))
}

/// Reads a stored Web Push subscription back, as the transport needs it.
///
/// # Errors
///
/// A message for a stored value that no longer parses, which is a bug's
/// shape, not a caller's.
#[cfg(feature = "service")]
pub(crate) fn stored_web_subscription(credential: &str) -> Result<WebSubscription, String> {
	serde_json::from_str(credential)
		.map_err(|_| "a stored Web Push subscription doesn't parse".to_owned())
}

/// Checks an enrollment before it's made, so an app rejects bad
/// credentials without a round trip and the service rejects them all the
/// same. Shape only: whether the platform's transport is configured is
/// the service's own answer, and where a Web Push endpoint may point is
/// the service's host policy.
///
/// # Errors
///
/// A message naming what's wrong with the credential.
pub fn check_credential(credential: &PushCredential) -> Result<(), String> {
	match credential {
		PushCredential::Apns { device_token } => {
			let token = device_token.trim();
			let hex = token.len() >= MIN_APNS_TOKEN_CHARS
				&& token.len() <= MAX_APNS_TOKEN_CHARS
				&& token
					.bytes()
					.all(|byte| byte.is_ascii_digit() || byte.is_ascii_hexdigit());

			if !hex {
				return Err(format!(
					"an APNs device token is {MIN_APNS_TOKEN_CHARS} to {MAX_APNS_TOKEN_CHARS} hex \
					 characters"
				));
			}
		}
		PushCredential::Fcm { registration_token } => {
			let token = registration_token.trim();

			if token.is_empty()
				|| token.len() > MAX_FCM_TOKEN_LEN
				|| !token.bytes().all(|byte| (0x21..=0x7E).contains(&byte))
			{
				return Err(format!(
					"an FCM registration token is 1 to {MAX_FCM_TOKEN_LEN} printable ASCII \
					 characters with no spaces"
				));
			}
		}
		PushCredential::WebPush { endpoint, keys } => {
			check_web_endpoint(endpoint)?;
			check_web_keys(keys)?;
		}
	}

	Ok(())
}

/// Checks a Web Push endpoint URL's shape: `https`, no credentials in the
/// URL, no fragment, port 443 or none. Which hosts are allowed is the
/// service's push-host policy, checked on the server.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_web_endpoint(endpoint: &str) -> Result<(), String> {
	if endpoint.len() > MAX_WEB_ENDPOINT_LEN {
		return Err(format!(
			"a Web Push endpoint is at most {MAX_WEB_ENDPOINT_LEN} bytes"
		));
	}

	let url = url::Url::parse(endpoint.trim())
		.map_err(|_| "a Web Push endpoint is an absolute https URL".to_owned())?;

	if url.scheme() != "https" {
		return Err("a Web Push endpoint is https".into());
	}

	if !url.username().is_empty() || url.password().is_some() {
		return Err("a Web Push endpoint carries no credentials in its URL".into());
	}

	if url.fragment().is_some() {
		return Err("a Web Push endpoint has no fragment".into());
	}

	if let Some(port) = url.port()
		&& port != 443
	{
		return Err("a Web Push endpoint is on port 443".into());
	}

	if url.host_str().is_none_or(str::is_empty) {
		return Err("a Web Push endpoint names a host".into());
	}

	Ok(())
}

/// Checks a subscription's keys: base64url, a 65-byte uncompressed P-256
/// public key, and a 16-byte authentication secret (the RFC allows up to
/// 32; nothing today issues more than 16).
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_web_keys(keys: &WebPushKeys) -> Result<(), String> {
	let p256dh = base64_url(&keys.p256dh).ok_or("the p256dh key is base64url")?;
	let auth = base64_url(&keys.auth).ok_or("the auth secret is base64url")?;

	if p256dh.len() != 65 || p256dh[0] != 0x04 {
		return Err("the p256dh key is a 65-byte uncompressed P-256 public key".into());
	}

	if auth.len() < 16 || auth.len() > 32 {
		return Err("the auth secret is 16 bytes of base64url".into());
	}

	Ok(())
}

/// Decodes padded or unpadded base64url.
pub(crate) fn base64_url(text: &str) -> Option<Vec<u8>> {
	use base64::Engine as _;

	let trimmed = text.trim();
	let unpadded = trimmed.trim_end_matches('=');

	if trimmed.len() % 4 == 1 {
		return None;
	}

	URL_SAFE_NO_PAD.decode(unpadded).ok().or_else(|| {
		base64::engine::general_purpose::URL_SAFE
			.decode(trimmed)
			.ok()
	})
}

/// One enrolled endpoint: the opaque record of a device or browser the
/// person's app pushed them to enroll. The credential is never in it.
#[table(name = "push_endpoints", unique(fingerprint), index(person))]
pub struct PushEndpoint {
	/// An opaque id the service assigns: what lists and revocations name.
	#[db(key)]
	pub id: uuid::Uuid,
	/// The person whose endpoint it is, from their forwarded identity.
	pub person: uuid::Uuid,
	/// Which platform's service this speaks to.
	#[db(max_len = 16)]
	pub platform: String,
	/// The platform's credential, verbatim: secret material, left out of
	/// the audit log and redacted in debug output.
	#[db(secret, max_len = 6_144)]
	pub credential: String,
	/// Monotonic credential revision, advanced on rotation or reactivation.
	pub generation: i64,
	/// A SHA-256 of the endpoint's platform and destination — its token,
	/// or a Web Push subscription's endpoint URL — in hex: how the service
	/// knows two enrollments name the same endpoint without ever searching
	/// on the secret itself. It deliberately does not cover a Web Push
	/// subscription's keys, so a browser that rotates them keeps its
	/// endpoint and its owner.
	#[db(min_len = 64, max_len = 64)]
	pub fingerprint: String,
	/// When the provider said this credential no longer exists: expired,
	/// uninstalled, or unregistered. The endpoint stays as an explainable
	/// record and takes no further deliveries.
	pub invalidated_at: Option<leptoskit_db::Timestamp>,
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
}

/// One attempt's record, per notification and endpoint: the durable
/// delivery the push job works through, from enqueued to accepted,
/// rejected, or skipped. Unique per pair, so a raced or repeated send
/// cannot double-enroll an endpoint for a notification.
///
/// Delivery is at-least-once: a retry after an answered-but-lost attempt
/// can push twice, and the provider's acceptance says it took the
/// message, not that anyone saw it.
#[table(
	name = "push_deliveries",
	unique(notification, endpoint),
	index(send_key, id),
	index(state, next_attempt_at)
)]
pub struct PushDelivery {
	#[db(key)]
	pub id: uuid::Uuid,
	/// The inbox notification this delivery carries.
	pub notification: uuid::Uuid,
	/// The send that created it, for the app's status pages.
	#[db(max_len = 255)]
	pub send_key: String,
	/// The endpoint it goes to. The endpoint may be gone — revoked —
	/// while this row remains, explaining what happened.
	pub endpoint: uuid::Uuid,
	/// Credential revision this queued delivery follows.
	pub generation: i64,
	/// The recipient, denormalized from the notification.
	pub person: uuid::Uuid,
	/// The endpoint's platform, denormalized: the row outlives its
	/// endpoint.
	#[db(max_len = 16)]
	pub platform: String,
	/// Where the delivery stands, as [`DeliveryState`]'s names.
	#[db(max_len = 16)]
	pub state: String,
	/// Why it left the queue, as [`DeliveryReason`]'s names, when it did.
	#[db(max_len = 32)]
	pub reason: Option<String>,
	/// How many times it has been attempted, the first try included.
	pub attempts: i64,
	/// When the next attempt may run, while it waits or retries.
	pub next_attempt_at: Option<leptoskit_db::Timestamp>,
	/// When the last attempt ran.
	pub attempted_at: Option<leptoskit_db::Timestamp>,
	/// When it left the queue for good: accepted, rejected, or skipped.
	pub finished_at: Option<leptoskit_db::Timestamp>,
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
}

/// Where a delivery stands. `Accepted` is the provider taking the
/// message: not a person seeing it, not a banner being shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
	/// In the queue, not yet attempted.
	#[default]
	Enqueued,
	/// Attempted, failed transiently, and waiting to try again.
	Retrying,
	/// The provider took the message.
	Accepted,
	/// The provider refused it for good, or the attempts ran out.
	Rejected,
	/// It never went out: the notification's state or the endpoint's said
	/// so at the moment of the attempt.
	Skipped,
}

impl DeliveryState {
	/// The stored name.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Enqueued => "enqueued",
			Self::Retrying => "retrying",
			Self::Accepted => "accepted",
			Self::Rejected => "rejected",
			Self::Skipped => "skipped",
		}
	}

	/// The state a stored name stands for, or `None` for anything else.
	#[must_use]
	pub fn from_str_value(name: &str) -> Option<Self> {
		match name {
			"enqueued" => Some(Self::Enqueued),
			"retrying" => Some(Self::Retrying),
			"accepted" => Some(Self::Accepted),
			"rejected" => Some(Self::Rejected),
			"skipped" => Some(Self::Skipped),
			_ => None,
		}
	}
}

/// Why a delivery left the queue without being accepted. Sanitized codes,
/// never a provider's text: the same handful for every operator, with
/// nothing in them that isn't already the operator's knowledge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryReason {
	/// Its endpoint was revoked before the attempt.
	EndpointRevoked,
	/// Its endpoint was invalidated by the provider's answer to an
	/// earlier attempt, or pointed somewhere this service won't push.
	EndpointUnavailable,
	/// The notification expired before the attempt.
	Expired,
	/// The app cancelled the send before the attempt.
	Cancelled,
	/// The person opted out of the category after the delivery was
	/// enqueued.
	OptedOut,
	/// No transport for this platform is configured on this server.
	TransportUnavailable,
	/// The notification's content doesn't fit the provider's payload
	/// limit, even in the minimal id-only envelope.
	Oversized,
	/// The provider answered that the credential no longer exists, so the
	/// endpoint was invalidated.
	Unregistered,
	/// The provider refused the message for good, for a reason the codes
	/// don't name: a topic or project mismatch, a bad request.
	ProviderRejected,
	/// The attempts ran out before the provider took the message.
	AttemptsExhausted,
}

impl DeliveryReason {
	/// The stored name.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::EndpointRevoked => "endpoint_revoked",
			Self::EndpointUnavailable => "endpoint_unavailable",
			Self::Expired => "expired",
			Self::Cancelled => "cancelled",
			Self::OptedOut => "opted_out",
			Self::TransportUnavailable => "transport_unavailable",
			Self::Oversized => "oversized",
			Self::Unregistered => "unregistered",
			Self::ProviderRejected => "provider_rejected",
			Self::AttemptsExhausted => "attempts_exhausted",
		}
	}

	/// The reason a stored name stands for, or `None` for anything else.
	#[must_use]
	pub fn from_str_value(name: &str) -> Option<Self> {
		match name {
			"endpoint_revoked" => Some(Self::EndpointRevoked),
			"endpoint_unavailable" => Some(Self::EndpointUnavailable),
			"expired" => Some(Self::Expired),
			"cancelled" => Some(Self::Cancelled),
			"opted_out" => Some(Self::OptedOut),
			"transport_unavailable" => Some(Self::TransportUnavailable),
			"oversized" => Some(Self::Oversized),
			"unregistered" => Some(Self::Unregistered),
			"provider_rejected" => Some(Self::ProviderRejected),
			"attempts_exhausted" => Some(Self::AttemptsExhausted),
			_ => None,
		}
	}
}

/// What the app's server reads about one delivery: enough to explain a
/// send's outcome, and nothing that names a device or repeats its
/// credential.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeliveryStatus {
	/// The notification the delivery carries.
	pub notification: uuid::Uuid,
	/// The endpoint it goes to, by its opaque id.
	pub endpoint: uuid::Uuid,
	/// The recipient.
	pub person: uuid::Uuid,
	/// Which platform's service it went through.
	pub platform: PushPlatform,
	/// Where the delivery stands.
	pub state: DeliveryState,
	/// Why it left the queue, when it did.
	pub reason: Option<DeliveryReason>,
	/// How many times it was attempted.
	pub attempts: u32,
	/// When the last attempt ran, if one did.
	pub attempted_at: Option<leptoskit_db::Timestamp>,
	/// When it left the queue, if it has.
	pub finished_at: Option<leptoskit_db::Timestamp>,
}

/// One enrolled endpoint, as its owner sees it: the opaque id, the
/// platform, when it was enrolled, and whether a provider's answer has
/// since invalidated it. The credential is never here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EndpointSummary {
	/// The endpoint's opaque id, for revocation.
	pub id: uuid::Uuid,
	/// Which platform's service it speaks to.
	pub platform: PushPlatform,
	/// When it was enrolled.
	pub created_at: leptoskit_db::Timestamp,
	/// When the provider said this credential no longer exists, if it
	/// did: an invalidated endpoint stays listed — it explains itself —
	/// but takes no further deliveries.
	pub invalidated_at: Option<leptoskit_db::Timestamp>,
}

/// Registers one push endpoint for the signed-in person: their app hands
/// over the credential its platform issued, and the service holds it
/// against their id, read from the forwarded identity and never from an
/// argument. Registering a destination this person already holds answers
/// the same endpoint again — idempotent, same opaque id — so a retried
/// enrollment makes no duplicate, and a Web Push subscription whose
/// browser rotated its keys updates the stored credential in place: the
/// endpoint and its queued deliveries stay.
///
/// A destination another **live** account holds is refused rather than
/// reassigned: the device's previous account revokes it first (signing
/// out of the app revokes that device's endpoints), which is what keeps a
/// push from reaching an account that never asked for it. A destination
/// the provider invalidated is not held: registering it takes it over
/// clean, for whoever's device now presents it. The refusal says the
/// destination is taken; the caller holds the credential itself, so this
/// tells them only what their own device is already enrolled to.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person,
/// [`rpc::Error::Invalid`] for a credential that fails
/// [`check_credential`], a host the push-host policy won't push to, an
/// endpoint past [`MAX_ENDPOINTS`], or a platform this server has no
/// transport configured for, and [`rpc::Error::Conflict`] when another
/// account holds the destination live.
#[rpc::mutation(service = "notifications")]
pub async fn register_push_endpoint(
	cx: &rpc::MutationCx,
	credential: PushCredential,
) -> rpc::Result<EndpointSummary> {
	let me = crate::service::me(cx.identity())?;
	let push = crate::service::push::Push::of(cx)?;
	crate::service::push::register(cx, push, me, &credential).await
}

/// The signed-in person's enrolled endpoints, oldest first, one bounded
/// page at a time: the opaque ids, platforms, enrollment times, and
/// whether each has been invalidated. Invalidated endpoints are included
/// — a device list that hides them can't explain them — and the page is
/// bounded, so however many times a device churns the answer stays small.
/// The credentials are never in the answer, and nobody else's endpoints
/// are in it either.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person.
#[rpc::query(service = "notifications")]
pub async fn push_endpoints(
	cx: &rpc::QueryCx,
	page: rpc::PageArgs,
) -> rpc::Result<rpc::Page<EndpointSummary>> {
	let me = crate::service::me(cx.identity())?;
	crate::service::push::list(cx.db(), me, &page).await
}

/// Revokes one of the signed-in person's endpoints: the service deletes
/// it, and its queued deliveries are skipped as revoked. Only the owner
/// can revoke; another person's endpoint — or an id nobody holds — looks
/// exactly like a missing one. Signing out of the app on a device
/// revokes every endpoint it enrolled, so a later account on the same
/// device enrolls from clean.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person,
/// and [`rpc::Error::NotFound`] unless the endpoint is theirs.
#[rpc::mutation(service = "notifications")]
pub async fn revoke_push_endpoint(cx: &rpc::MutationCx, endpoint: uuid::Uuid) -> rpc::Result<()> {
	let me = crate::service::me(cx.identity())?;
	crate::service::push::revoke(cx.tx(), me, endpoint).await
}

/// What the app's server reads about a send's push deliveries, oldest
/// first, one bounded page at a time: per endpoint, where the delivery
/// stands and, when it left the queue, a sanitized reason code. The
/// provider's acceptance is not a person's receipt; `accepted` says the
/// provider took the message and nothing more.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server
/// code. A recipient has no way to send or steer a push, and the answer
/// carries no credential material.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::Invalid`] for a bad idempotency key.
#[rpc::query(service = "notifications")]
pub async fn push_deliveries(
	cx: &rpc::QueryCx,
	send_key: String,
	page: rpc::PageArgs,
) -> rpc::Result<rpc::Page<DeliveryStatus>> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app reads delivery status: call this from its server, with the service's \
			 token",
		));
	}

	crate::check_key(&send_key).map_err(rpc::Error::invalid)?;
	crate::service::push::status_page(cx.db(), &send_key, &page).await
}

/// Attempts the due push deliveries of one send, in bounded batches: the
/// job every send with enrolled endpoints enqueues when its notification
/// becomes visible, and again whenever attempts are waiting to retry.
/// Each attempt re-reads the notification's schedule, cancellation,
/// expiry, and the person's latest opt-out at the moment it fires, and
/// never holds a database transaction across the network call. The workers
/// supply the instance runtime from memory.
/// Arguments contain only logical send and round identifiers.
///
/// Transient failures wait and retry — honoring the provider's
/// `Retry-After` when it gives one — until [`MAX_DELIVERY_ATTEMPTS`] is
/// spent; permanent ones end the delivery and, for a credential the
/// provider no longer knows, invalidate the endpoint so nothing further
/// is pushed to it. An operator-side failure — a refused OAuth exchange,
/// a stale access token — waits and retries without spending the
/// delivery's attempts: the endpoint isn't at fault, and the send's
/// status shows it stuck rather than failed. The job runs whether or not
/// any endpoint is still enrolled; it walks what is there. Only the
/// service's own workers run it. No deadline: a scheduled send's first
/// round waits as long as its schedule says, like a wake job does.
#[rpc::job(
	service = "notifications",
	queue = "push",
	timeout = 5m
)]
pub async fn push_send(cx: &rpc::JobCx, send_key: String, round: u32) -> rpc::Result<()> {
	let push = cx
		.state::<crate::service::push::Push>()
		.ok_or_else(|| rpc::Error::server("push worker state is missing"))?;
	crate::service::push::round(cx, &send_key, round, push).await
}

#[cfg(test)]
mod tests {
	use base64::Engine as _;
	use base64::engine::general_purpose::URL_SAFE_NO_PAD;

	use super::PushCredential;
	use super::WebPushKeys;
	use super::WebSubscription;

	/// A P-256 point and auth secret long enough to pass the shared
	/// checks: the first byte makes it an uncompressed point.
	fn keys() -> (WebPushKeys, WebPushKeys) {
		let point = {
			let mut point = vec![0x04];
			point.extend(std::iter::repeat_n(5_u8, 64));
			point
		};
		let auth = [7_u8; 16];

		(
			WebPushKeys {
				p256dh: URL_SAFE_NO_PAD.encode(&point),
				auth: URL_SAFE_NO_PAD.encode(auth),
			},
			WebPushKeys {
				p256dh: base64::engine::general_purpose::URL_SAFE.encode(&point),
				auth: base64::engine::general_purpose::URL_SAFE.encode(auth),
			},
		)
	}

	fn web_push(endpoint: &str, keys: &WebPushKeys) -> PushCredential {
		PushCredential::WebPush {
			endpoint: endpoint.to_owned(),
			keys: keys.clone(),
		}
	}

	/// Padded and unpadded keys canonicalize to the same stored bytes, so
	/// the same browser can't enroll twice by switching SDKs.
	#[test]
	fn padded_and_unpadded_keys_canonicalize_alike() {
		let (unpadded, padded) = keys();
		let one = web_push(
			"https://updates.push.services.mozilla.com/push/1",
			&unpadded,
		)
		.canonical()
		.unwrap();
		let two = web_push("https://updates.push.services.mozilla.com/push/1", &padded)
			.canonical()
			.unwrap();
		assert_eq!(one, two, "padding changes nothing stored");
		assert!(one.contains("BQUFB"), "the keys are stored unpadded: {one}");

		// A padded pair decodes back to the same subscription.
		let back: WebSubscription = serde_json::from_str(&one).unwrap();
		assert_eq!(back.keys, unpadded);
	}

	/// The endpoint URL is part of the destination and the credential, and
	/// its parsing folds trivial differences: a trailing dot-free host
	/// with a default port is the same destination as one naming it.
	#[test]
	fn a_destination_is_the_platforms_own_identity() {
		let (keys, padded) = keys();

		// A web subscription's destination is its URL alone, not its keys:
		// two enrollments of the same browser through a key rotation name
		// the same endpoint.
		let destination = web_push("https://updates.push.services.mozilla.com/push/1", &keys)
			.destination()
			.unwrap();
		let rotated = web_push("https://updates.push.services.mozilla.com/push/1", &padded)
			.destination()
			.unwrap();
		assert_eq!(
			destination, rotated,
			"key material changes nothing about where a Web Push subscription delivers"
		);
		assert_eq!(
			destination,
			"https://updates.push.services.mozilla.com/push/1"
		);

		// An explicit default port folds into the same URL.
		let ported = web_push(
			"https://updates.push.services.mozilla.com:443/push/1",
			&keys,
		)
		.destination()
		.unwrap();
		assert_eq!(destination, ported, "the default port is nothing");
	}

	/// Nothing secret prints: the credential types' `Debug` shows the
	/// platform and redaction, never the token, the URL, or the keys.
	#[test]
	fn debug_prints_no_secrets() {
		let (keys, padded) = keys();
		let credentials = [
			PushCredential::Apns {
				device_token: "a1b2c3d4".repeat(8),
			},
			PushCredential::Fcm {
				registration_token: "fcm-secret-token".to_owned(),
			},
			web_push("https://updates.push.services.mozilla.com/push/1", &keys),
		];

		for credential in &credentials {
			let printed = format!("{credential:?}");
			assert!(!printed.contains("a1b2"), "an APNs token leaked: {printed}");
			assert!(
				!printed.contains("fcm-secret"),
				"an FCM token leaked: {printed}"
			);
			assert!(
				!printed.contains("mozilla") && !printed.contains("push/1"),
				"an endpoint URL leaked: {printed}"
			);
			assert!(printed.contains("redacted"), "the print says what it hides");
		}

		for shown in [
			format!("{keys:?}"),
			format!("{padded:?}"),
			format!(
				"{:?}",
				WebSubscription {
					endpoint: "https://updates.push.services.mozilla.com/push/1".to_owned(),
					keys: keys.clone(),
				}
			),
		] {
			assert!(shown.contains("redacted"), "the print says what it hides");
			assert!(
				!shown.contains("BQUFB") && !shown.contains("FcOw"),
				"keys leaked: {shown}"
			);
		}
	}
}
