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

/// The most invalidated endpoints one person keeps beside their live ones.
/// An invalidated endpoint stays listed, so a device list can explain it,
/// until its owner revokes it or re-enrolls it; past this many, the next
/// enrollment drops the oldest, so a person's endpoints — and their list,
/// which is one page of [`MAX_ENDPOINTS`] plus this — never grow without
/// bound.
pub const MAX_ENDPOINT_HISTORY: usize = 10;

/// How the service's answer begins when a renewal names an enrollment the
/// caller no longer holds — revoked from another device, taken over by the
/// device's next person, or erased with
/// [`forget_person`](crate::forget_person). The enrollment clients match it, so
/// a not-found from anywhere else is never mistaken for a revocation.
pub const ENROLLMENT_GONE: &str = "this enrollment is gone";

/// Whether `cause` is the service saying a renewed enrollment is
/// [gone](ENROLLMENT_GONE): the cue to drop the saved enrollment and offer
/// the switch again, rather than retry.
#[must_use]
pub fn enrollment_gone(cause: &rpc::FnError) -> bool {
	matches!(cause, rpc::FnError::Server(message) if message.starts_with(ENROLLMENT_GONE))
}

/// The longest APNs device token, in hex characters. Tokens are 64 today;
/// the bound leaves room for longer ones rather than guessing a ceiling.
pub const MAX_APNS_TOKEN_CHARS: usize = 200;

/// The shortest APNs device token, in hex characters.
pub const MIN_APNS_TOKEN_CHARS: usize = 32;

/// The longest FCM registration token, in bytes.
pub const MAX_FCM_TOKEN_LEN: usize = 4_096;

/// The longest Web Push subscription endpoint URL, in bytes.
pub const MAX_WEB_ENDPOINT_LEN: usize = 2_048;

/// The longest locale tag a device reports, in bytes: the 35 characters
/// RFC 5646 asks every implementation to accept.
pub const MAX_LOCALE_LEN: usize = 35;

/// The longest time zone name a device reports, in bytes. IANA names are
/// far shorter; the bound leaves room without guessing a ceiling.
pub const MAX_TIME_ZONE_LEN: usize = 64;

/// The longest app version a device reports, in bytes.
pub const MAX_APP_VERSION_LEN: usize = 64;

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

/// Deliveries stop 24 hours after visibility, even without notification expiry.
#[cfg(feature = "service")]
pub(crate) const DELIVERY_HORIZON_SECS: i64 = 24 * 3_600;

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

/// Which of Apple's two push servers a device token belongs to. A token
/// issued to a development build (`aps-environment` `development`) is
/// only known to the sandbox server, and a token issued to a TestFlight or
/// App Store build only to the production one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApnsEnvironment {
	/// The development server, `api.sandbox.push.apple.com`.
	Sandbox,
	/// The production server, `api.push.apple.com`.
	Production,
}

impl ApnsEnvironment {
	/// The stored name, and the value `APNS_ENVIRONMENT` takes.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Sandbox => "sandbox",
			Self::Production => "production",
		}
	}

	/// The environment a stored name stands for, or `None` for anything
	/// else.
	#[must_use]
	pub fn from_str_value(name: &str) -> Option<Self> {
		match name {
			"sandbox" => Some(Self::Sandbox),
			"production" => Some(Self::Production),
			_ => None,
		}
	}
}

/// What a device says about itself when it enrolls: the language it
/// prefers, the time zone it is in, and the version of the app running on
/// it. Every detail is optional — a browser knows no app version unless
/// the app names one — and the clients fill them in on their own, so an app
/// author writes no code for them. A send's condition reads them as
/// `device.locale`, `device.language`, `device.time_zone`, and
/// `device.app_version` (see [`crate::condition`]).
///
/// They belong to the endpoint, not the person: one person's phone and
/// laptop can differ, and re-enrolling the same endpoint replaces what it
/// said before. Only the endpoint's owner reads them back
/// ([`EndpointSummary::device`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceDetails {
	/// The preferred language, as a BCP 47 tag such as `en-GB` or
	/// `zh-Hant-TW`: at most [`MAX_LOCALE_LEN`] bytes. Stored in canonical
	/// case — `en_gb` becomes `en-GB` — by [`canonical_locale`].
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub locale: Option<String>,
	/// The time zone, as an IANA name such as `Europe/London`: at most
	/// [`MAX_TIME_ZONE_LEN`] bytes, checked for shape by
	/// [`check_time_zone`].
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub time_zone: Option<String>,
	/// The app's version, as free text such as `2.1.0`: at most
	/// [`MAX_APP_VERSION_LEN`] bytes with no control characters.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub app_version: Option<String>,
	/// The APNs environment the build on the device is signed for, which
	/// the iOS and macOS enrollment reads from the app's provisioning. The
	/// service pushes the endpoint through that environment's server.
	/// `None` — a build that predates the report, or one that could not
	/// tell — uses the environment the service is configured for
	/// (`APNS_ENVIRONMENT`). Kept only for an APNs endpoint; conditions do
	/// not read it.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub apns_environment: Option<ApnsEnvironment>,
}

impl DeviceDetails {
	/// The details as the service stores them: blank details dropped,
	/// the rest trimmed, and the locale in canonical case.
	///
	/// # Errors
	///
	/// A message naming the detail that fails its check: [`canonical_locale`],
	/// [`check_time_zone`], or [`check_app_version`].
	pub fn normalized(&self) -> Result<Self, String> {
		Ok(Self {
			locale: present(self.locale.as_deref())
				.map(canonical_locale)
				.transpose()?,
			time_zone: present(self.time_zone.as_deref())
				.map(|zone| check_time_zone(zone).map(|()| zone.to_owned()))
				.transpose()?,
			app_version: present(self.app_version.as_deref())
				.map(|version| check_app_version(version).map(|()| version.to_owned()))
				.transpose()?,
			apns_environment: self.apns_environment,
		})
	}

	/// The details a client can send without its enrollment being refused:
	/// each one [normalized](Self::normalized), and any that fails its check
	/// dropped rather than sent. The enrollment clients run this on what the
	/// platform reports, so an odd value from one device never costs it its
	/// push.
	#[must_use]
	pub fn sanitized(&self) -> Self {
		Self {
			locale: present(self.locale.as_deref()).and_then(|tag| canonical_locale(tag).ok()),
			time_zone: present(self.time_zone.as_deref())
				.filter(|zone| check_time_zone(zone).is_ok())
				.map(str::to_owned),
			app_version: present(self.app_version.as_deref())
				.filter(|version| check_app_version(version).is_ok())
				.map(str::to_owned),
			apns_environment: self.apns_environment,
		}
	}

	/// The locale's primary language subtag, lower case: `fr` for `fr-CA`.
	/// What a condition's `device.language` reads.
	#[must_use]
	pub fn language(&self) -> Option<&str> {
		self.locale.as_deref().and_then(language_of)
	}

	/// Whether the device said nothing about itself.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.locale.is_none()
			&& self.time_zone.is_none()
			&& self.app_version.is_none()
			&& self.apns_environment.is_none()
	}

	/// The details an endpoint of `platform` keeps: the APNs environment
	/// means nothing anywhere else, so only an APNs endpoint keeps one.
	#[must_use]
	pub fn for_platform(mut self, platform: PushPlatform) -> Self {
		if platform != PushPlatform::Apns {
			self.apns_environment = None;
		}

		self
	}
}

/// A detail with something in it, trimmed: blank is the same as absent.
fn present(value: Option<&str>) -> Option<&str> {
	value.map(str::trim).filter(|value| !value.is_empty())
}

/// The primary language subtag of a locale tag: what comes before the
/// first `-` or `_`. Answered as written; a stored tag is already
/// canonical, so it is lower case there.
pub(crate) fn language_of(locale: &str) -> Option<&str> {
	locale
		.split(['-', '_'])
		.next()
		.filter(|language| !language.is_empty())
}

/// Checks a device's details before they're sent, so a client can drop a
/// bad one and the service refuses it all the same. See
/// [`DeviceDetails::normalized`].
///
/// # Errors
///
/// A message naming the detail that's wrong.
pub fn check_device_details(details: &DeviceDetails) -> Result<(), String> {
	details.normalized().map(|_| ())
}

/// A BCP 47 locale tag in canonical case: the language subtag lower case,
/// a four-letter script subtag in title case, a two-letter region subtag
/// upper case, and everything else lower case — and everything after a
/// single-character extension subtag lower case too. `_` separators, which
/// some platforms report, become `-`. So `EN_gb` is `en-GB`, and
/// `zh-hant-tw` is `zh-Hant-TW`.
///
/// The check is of shape, not of the registry: subtags of 1 to 8 letters
/// and digits, the first one 2 to 8 letters, at most [`MAX_LOCALE_LEN`]
/// bytes in all.
///
/// # Errors
///
/// A message naming the rule the tag breaks.
pub fn canonical_locale(tag: &str) -> Result<String, String> {
	let tag = tag.trim();
	let complaint = || {
		format!(
			"a device's locale is a BCP 47 tag such as `en-GB`: at most {MAX_LOCALE_LEN} bytes of \
			 letter-and-digit subtags joined by `-`, starting with a language"
		)
	};

	if tag.is_empty() || tag.len() > MAX_LOCALE_LEN {
		return Err(complaint());
	}

	let mut canonical = String::with_capacity(tag.len());
	let mut extension = false;

	for (index, subtag) in tag.split(['-', '_']).enumerate() {
		if subtag.is_empty()
			|| subtag.len() > 8
			|| !subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
		{
			return Err(complaint());
		}

		let letters = subtag.bytes().all(|byte| byte.is_ascii_alphabetic());

		if index == 0 {
			if subtag.len() < 2 || !letters {
				return Err(complaint());
			}

			canonical.push_str(&subtag.to_ascii_lowercase());
			continue;
		}

		canonical.push('-');
		extension |= subtag.len() == 1;

		match subtag.len() {
			2 if letters && !extension => canonical.push_str(&subtag.to_ascii_uppercase()),
			4 if letters && !extension => {
				let lower = subtag.to_ascii_lowercase();
				canonical.push_str(&lower[..1].to_ascii_uppercase());
				canonical.push_str(&lower[1..]);
			}
			_ => canonical.push_str(&subtag.to_ascii_lowercase()),
		}
	}

	Ok(canonical)
}

/// Checks a time zone name's shape: one or more `/`-separated parts, each
/// starting with a letter and made of letters, digits, `_`, `-`, and `+`,
/// at most [`MAX_TIME_ZONE_LEN`] bytes in all — `Europe/London`,
/// `America/Argentina/Buenos_Aires`, `Etc/GMT+5`, `UTC`.
///
/// Only the shape: whether the name is in a time zone database is not
/// checked, because a device's database can be newer than the server's,
/// and an enrollment must never fail over a detail.
///
/// # Errors
///
/// A message naming the rule the name breaks.
pub fn check_time_zone(name: &str) -> Result<(), String> {
	let complaint = || {
		format!(
			"a device's time zone is an IANA name such as `Europe/London`: at most \
			 {MAX_TIME_ZONE_LEN} bytes of `/`-separated parts that start with a letter"
		)
	};

	if name.is_empty() || name.len() > MAX_TIME_ZONE_LEN {
		return Err(complaint());
	}

	for part in name.split('/') {
		let shaped = part
			.bytes()
			.next()
			.is_some_and(|byte| byte.is_ascii_alphabetic())
			&& part
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+'));

		if !shaped {
			return Err(complaint());
		}
	}

	Ok(())
}

/// Checks an app version: 1 to [`MAX_APP_VERSION_LEN`] bytes of text with
/// no control characters. Free text on purpose — `2.1.0`, `2.1.0-beta.3`,
/// `2024.11 (812)` — and a condition compares it as a version only when it
/// reads as one (see [`crate::condition::compare_versions`]).
///
/// # Errors
///
/// A message naming the rule the version breaks.
pub fn check_app_version(version: &str) -> Result<(), String> {
	if version.trim().is_empty()
		|| version.len() > MAX_APP_VERSION_LEN
		|| version.chars().any(char::is_control)
	{
		return Err(format!(
			"a device's app version is 1 to {MAX_APP_VERSION_LEN} bytes of text with no control \
			 characters"
		));
	}

	Ok(())
}

/// One enrolled endpoint: the opaque record of a device or browser the
/// person's app pushed them to enroll. The credential is never in it.
#[table(
	name = "push_endpoints",
	unique(fingerprint),
	index(person),
	index(platform, invalidated_at, person)
)]
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
	/// The device's preferred locale, a canonical BCP 47 tag, as it last
	/// enrolled: what a condition's `device.locale` and `device.language`
	/// read. `None` when it said nothing.
	#[db(max_len = 35)]
	pub locale: Option<String>,
	/// The device's IANA time zone name, as it last enrolled.
	#[db(max_len = 64)]
	pub time_zone: Option<String>,
	/// The app's version on the device, as it last enrolled: what a
	/// condition's `device.app_version` reads.
	#[db(max_len = 64)]
	pub app_version: Option<String>,
	/// When the endpoint last told the service about itself: its
	/// enrollment, a renewal, or a change to its details. A send that
	/// resolves a person's language or time zone reads their most recently
	/// seen live endpoint. `None` for an endpoint enrolled before the
	/// column existed, which counts as seen when it was created.
	pub seen_at: Option<leptoskit_db::Timestamp>,
	/// When the app's server imported this endpoint
	/// ([`import_push_endpoints`]) from the tokens it held before the app
	/// enrolled devices itself. Cleared the first time the device's own
	/// app enrolls the same credential for the same person, which makes
	/// the endpoint an ordinary enrollment. `None` for every endpoint a
	/// device enrolled.
	pub imported_at: Option<leptoskit_db::Timestamp>,
	/// The APNs environment the device's build reported when it last
	/// enrolled, as [`ApnsEnvironment`]'s names: which of Apple's servers
	/// this endpoint is pushed through. `None` uses the environment the
	/// service is configured for, as every endpoint did before the column
	/// existed. Always `None` for an endpoint that is not APNs'.
	#[db(max_len = 16)]
	pub apns_environment: Option<String>,
}

#[cfg(feature = "service")]
impl PushEndpoint {
	/// The details the device last enrolled with.
	#[must_use]
	pub(crate) fn device_details(&self) -> DeviceDetails {
		DeviceDetails {
			locale: self.locale.clone(),
			time_zone: self.time_zone.clone(),
			app_version: self.app_version.clone(),
			apns_environment: self.apns_environment(),
		}
	}

	/// The APNs environment the endpoint's device reported, if it did.
	#[must_use]
	pub(crate) fn apns_environment(&self) -> Option<ApnsEnvironment> {
		self.apns_environment
			.as_deref()
			.and_then(ApnsEnvironment::from_str_value)
	}
}

/// How long the [`PushAcceptance`] ledger keeps a row, whatever the
/// operator's retention: long enough for whoever meters pushes to have
/// counted it many times over.
pub const ACCEPTANCES_DAYS: u32 = 90;

/// A push its provider accepted, one row per delivery, written in the same
/// write that marks the delivery accepted: the ledger a host that meters
/// pushes counts, by [`accepted_at`](Self::accepted_at). Unlike a
/// [`PushDelivery`], it names no person, so forgetting one never removes
/// it, and only [`ACCEPTANCES_DAYS`] does.
#[table(name = "push_acceptances", unique(delivery), index(accepted_at))]
pub struct PushAcceptance {
	/// A UUID v7 made when it was accepted.
	#[db(key)]
	pub id: uuid::Uuid,
	/// The delivery that was accepted.
	pub delivery: uuid::Uuid,
	/// The delivery's platform.
	#[db(max_len = 16)]
	pub platform: String,
	/// When its provider accepted it.
	pub accepted_at: leptoskit_db::Timestamp,
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
	index(endpoint, state),
	index(send_key, state, id)
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
	/// don't name: a request it calls malformed.
	ProviderRejected,
	/// The attempts ran out before the provider took the message.
	AttemptsExhausted,
	/// The finite delivery window ended.
	HorizonExceeded,
	/// Retry time arithmetic could not be represented.
	RetryOverflow,
	/// Operator credentials or project configuration prevented delivery:
	/// a provider token, access token, or VAPID signature the provider
	/// refused, an APNs topic or environment the device token doesn't
	/// belong to, or a token from another FCM project. The endpoint stays
	/// enrolled, because the fix is the operator's.
	OperatorConfiguration,
	/// The Web Push subscription can't be encrypted to — its `p256dh`
	/// value isn't a point on P-256 — so no push can ever reach it.
	SubscriptionUnusable,
	/// The platform can't take this kind of message: a silent data
	/// message to a browser, which requires every push to show a
	/// notification.
	PlatformUnsupported,
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
			Self::HorizonExceeded => "horizon_exceeded",
			Self::RetryOverflow => "retry_overflow",
			Self::OperatorConfiguration => "operator_configuration",
			Self::SubscriptionUnusable => "subscription_unusable",
			Self::PlatformUnsupported => "platform_unsupported",
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
			"horizon_exceeded" => Some(Self::HorizonExceeded),
			"retry_overflow" => Some(Self::RetryOverflow),
			"operator_configuration" => Some(Self::OperatorConfiguration),
			"subscription_unusable" => Some(Self::SubscriptionUnusable),
			"platform_unsupported" => Some(Self::PlatformUnsupported),
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
/// platform, when it was enrolled, whether a provider's answer has since
/// invalidated it, and what the device last said about itself. The
/// credential is never here, and nobody but the owner reads this.
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
	/// The locale, time zone, and app version the device last enrolled
	/// with, as the service stored them. Empty when it said nothing — and
	/// when the service predates device details.
	#[serde(default)]
	pub device: DeviceDetails,
	/// Whether the app's server imported this endpoint
	/// ([`import_push_endpoints`]) and the device's own app has not
	/// enrolled it since. An imported endpoint is pushed to like any
	/// other; its details are what the import said, or none.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub imported: bool,
}

/// The most endpoints one [`import_push_endpoints`] call carries.
pub const MAX_IMPORT_ENDPOINTS: usize = 500;

/// One endpoint to import: a token the app's server already holds for a
/// person, from before the app enrolled devices with this service.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ImportEndpoint {
	/// The person the token belongs to, as the app's own records say.
	pub person: uuid::Uuid,
	/// The token: an FCM registration token or an APNs device token. A
	/// Web Push subscription cannot be imported.
	pub credential: PushCredential,
	/// What the app's server knows about the device, each part optional.
	/// The device's own enrollment replaces it later.
	#[serde(default)]
	pub device: DeviceDetails,
}

/// Why one entry of an import made no endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ImportSkip {
	/// Another person already holds this token. An import proves nothing
	/// about who holds a device, so it never moves one.
	HeldByAnother,
	/// The person already holds [`MAX_ENDPOINTS`] live endpoints.
	PersonFull,
}

/// One entry an import skipped, by its place in the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkippedImport {
	/// The entry's index in the call's `endpoints`, from zero.
	pub index: u32,
	/// Why it made no endpoint.
	pub reason: ImportSkip,
}

/// What an import came to. It names entries by their place in the call
/// and never repeats a credential.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ImportedEndpoints {
	/// How many endpoints the call created.
	pub imported: u32,
	/// How many entries named an endpoint their person already holds,
	/// imported earlier or enrolled by the device: left exactly as they
	/// were.
	pub unchanged: u32,
	/// The entries that made no endpoint, in the call's order.
	pub skipped: Vec<SkippedImport>,
}

/// Checks an import's shape, on both sides of the wire: 1 to
/// [`MAX_IMPORT_ENDPOINTS`] entries, each an APNs or FCM credential that
/// passes [`check_credential`] with details that pass
/// [`check_device_details`].
///
/// # Errors
///
/// A message naming the entry by its index, and never its credential.
pub fn check_import(endpoints: &[ImportEndpoint]) -> Result<(), String> {
	if endpoints.is_empty() || endpoints.len() > MAX_IMPORT_ENDPOINTS {
		return Err(format!(
			"an import carries 1 to {MAX_IMPORT_ENDPOINTS} endpoints (MAX_IMPORT_ENDPOINTS): send \
			 the rest in further calls"
		));
	}

	for (index, entry) in endpoints.iter().enumerate() {
		if matches!(entry.credential, PushCredential::WebPush { .. }) {
			return Err(format!(
				"endpoints[{index}].credential is a Web Push subscription, which cannot be \
				 imported: a browser subscribes again itself"
			));
		}

		check_credential(&entry.credential)
			.map_err(|complaint| format!("endpoints[{index}].credential: {complaint}"))?;
		check_device_details(&entry.device)
			.map_err(|complaint| format!("endpoints[{index}].device: {complaint}"))?;
	}

	Ok(())
}

/// Imports push tokens the app's server already holds, so the people they
/// belong to are pushed to from the next send on: before their devices
/// run a build that enrolls with this service. It is for the tokens an
/// app collected while it sent through Firebase Cloud Messaging or
/// straight to APNs — FCM registration tokens (Android, and iOS apps that
/// used the Firebase SDK) and APNs device tokens. At most
/// [`MAX_IMPORT_ENDPOINTS`] entries a call.
///
/// Each entry becomes an endpoint of its person, marked as imported
/// ([`EndpointSummary::imported`]), with the device details the entry
/// states and no others. The rules an enrollment follows hold:
///
/// - **Idempotent.** An entry whose person already holds the token — imported
///   before, or enrolled by the device — changes nothing and counts as
///   `unchanged`, an invalidated endpoint included: a stale token is not
///   revived.
/// - **Never a takeover.** A token another person holds stays theirs and the
///   entry is skipped as `held_by_another`. Only a person presenting the token
///   from the device takes it over ([`register_push_endpoint`]).
/// - **Bounded per person.** An entry past its person's [`MAX_ENDPOINTS`] live
///   endpoints is skipped as `person_full`.
/// - **Replaced by the real enrollment.** When the device's app enrolls the
///   same token for the same person, the endpoint keeps its id and queued
///   deliveries, takes the details the device reports, and is no longer
///   imported.
/// - **Revoked and erased like any endpoint.** The service keeps no record of a
///   revoke or an erasure, so importing a token again after its person turned
///   notifications off, or after [`forget_person`](crate::forget_person),
///   enrolls it again: import from your current records, and stop importing a
///   person you erased.
///
/// The whole call is checked before anything is written, and one bad
/// entry refuses it by its index. The answer counts what happened and
/// names skipped entries by index; no credential is ever answered.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server
/// code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for an import that fails [`check_import`] or
/// names a platform this server has no transport configured for.
#[rpc::mutation(service = "notifications")]
pub async fn import_push_endpoints(
	cx: &rpc::MutationCx,
	endpoints: Vec<ImportEndpoint>,
) -> rpc::Result<ImportedEndpoints> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app imports endpoints: call this from its server, with the service's token",
		));
	}

	let push = crate::service::push::Push::of(cx)?;
	crate::service::push::import(cx.tx(), push, &endpoints).await
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
/// `device` is what the device says about itself — its locale, time zone,
/// and app version, each optional — and the enrollment clients fill it in
/// on their own. A re-enrollment replaces what the endpoint said before;
/// when only the details changed, the credential's generation stays and
/// queued deliveries are untouched. An enrollment that names none, as
/// every one did before details existed, stores none.
///
/// A destination another account holds changes hands only to a caller who
/// presents the byte-identical credential — the same token, or the same
/// Web Push subscription with the same keys — because holding the
/// credential is holding the device. The previous account's endpoint is
/// revoked: deleted, its queued deliveries skipped, so its notifications
/// stop reaching a device its person no longer holds, and the caller gets
/// a fresh endpoint under a fresh id. This is how a shared device passes
/// to its next person when the previous one never signed out cleanly. The
/// same destination under a different credential — another person's Web
/// Push URL with keys of the caller's own — proves nothing and is refused,
/// whether or not a provider has invalidated it.
///
/// `renews` turns the call into a renewal, which is what a launch-time
/// reconciliation sends: it names the endpoint id the device saved when it
/// enrolled, and the service registers only while that endpoint is still
/// the caller's. Once it isn't — revoked from another device, taken over,
/// or erased by [`forget_person`](crate::forget_person) — the answer is
/// not found, beginning with [`ENROLLMENT_GONE`], and nothing is written:
/// a server-side revoke is never undone by a launch, and a renewal never
/// takes a destination over. Leave it out to enroll, which is a person's
/// own gesture.
///
/// A person keeps at most [`MAX_ENDPOINTS`] live endpoints and
/// [`MAX_ENDPOINT_HISTORY`] invalidated ones: an enrollment past the
/// second bound drops the oldest invalidated endpoints.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person,
/// [`rpc::Error::Invalid`] for a credential that fails
/// [`check_credential`], details that fail [`check_device_details`], a
/// host the push-host policy won't push to, an endpoint past
/// [`MAX_ENDPOINTS`], or a platform this server has no transport
/// configured for, [`rpc::Error::Conflict`] when another account holds
/// the destination under a different credential, and
/// [`rpc::Error::NotFound`] when `renews` names an enrollment that is gone.
#[rpc::mutation(service = "notifications")]
pub async fn register_push_endpoint(
	cx: &rpc::MutationCx,
	credential: PushCredential,
	#[serde(default)] device: DeviceDetails,
	#[serde(default, skip_serializing_if = "Option::is_none")] renews: Option<uuid::Uuid>,
) -> rpc::Result<EndpointSummary> {
	let me = crate::service::me(cx.identity())?;
	let push = crate::service::push::Push::of(cx)?;
	crate::service::push::register(cx, push, me, &credential, &device, renews).await
}

/// Whether `endpoint` is still one of the signed-in person's enrollments:
/// one primary-key read, no write. It is how a device that changed nothing
/// since its last launch learns that the server no longer has its
/// enrollment — revoked from another device's list, taken over by the
/// device's next person, or erased — so it can drop its saved state and
/// show notifications as off. An invalidated endpoint still stands: it is
/// the person's until they revoke it, and re-enrolling revives it.
/// Another person's endpoint, like a strange id, answers `false`.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] unless the app forwarded a signed-in person.
#[rpc::query(service = "notifications")]
pub async fn push_endpoint_standing(cx: &rpc::QueryCx, endpoint: uuid::Uuid) -> rpc::Result<bool> {
	let me = crate::service::me(cx.identity())?;
	crate::service::push::standing(cx.db(), me, endpoint).await
}

/// The signed-in person's enrolled endpoints, oldest first, one bounded
/// page at a time: the opaque ids, platforms, enrollment times, whether
/// each has been invalidated, and the details each device last reported
/// ([`EndpointSummary::device`]). Invalidated endpoints are included
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
/// revokes the endpoint it enrolled; a device whose person never got to
/// sign out passes to its next person when they enroll with the same
/// credential ([`register_push_endpoint`]), and the app's server can
/// revoke a person's endpoints itself ([`revoke_person_endpoints`]).
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

/// Revokes every endpoint one person holds, from the app's server: for a
/// session the app ended without the device's help — an expired or
/// revoked session, a sign-out everywhere, a suspended account — where the
/// person's own [`revoke_push_endpoint`] can no longer run. Their
/// endpoints are deleted and their queued deliveries skipped as revoked,
/// so nothing more is pushed to devices they are signed out of. Their
/// inbox, preferences, topics, and attributes stay; [`forget_person`]
/// erases those. The devices learn at their next launch and enroll again
/// only from a gesture.
///
/// Idempotent: the answer is how many endpoints this call removed, `0`
/// for a person who holds none.
///
/// Only the app may call it.
///
/// [`forget_person`]: crate::forget_person
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server.
#[rpc::mutation(service = "notifications")]
pub async fn revoke_person_endpoints(cx: &rpc::MutationCx, person: uuid::Uuid) -> rpc::Result<u32> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app revokes another person's endpoints: call this from its server, with the \
			 service's token",
		));
	}

	crate::service::push::revoke_all(cx.tx(), person).await
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

/// One person's enrolled endpoints, for the app's server: what
/// [`push_endpoints`] answers that person about themselves — the opaque
/// id, the platform, when it enrolled, whether its provider invalidated
/// it, and what the device said about itself — a bounded page at a time.
/// An operator choosing which device to send a test to
/// ([`send_test`](fn@crate::send_test)) reads this.
///
/// No credential is in the answer: not a device token, not a browser's
/// endpoint address or keys, and nothing derived from one.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server
/// code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a cursor that isn't one of this list's.
#[rpc::query(service = "notifications")]
pub async fn person_endpoints(
	cx: &rpc::QueryCx,
	person: uuid::Uuid,
	page: rpc::PageArgs,
) -> rpc::Result<rpc::Page<EndpointSummary>> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app lists a person's endpoints: call this from its server, with the \
			 service's token",
		));
	}

	crate::service::push::list(cx.db(), person, &page).await
}

/// Attempts the due push deliveries of one send, in bounded batches: the
/// job every send with enrolled endpoints enqueues when its notification
/// becomes visible, and again whenever attempts are waiting to retry, or
/// its fanout is still adding deliveries. One round does a bounded share
/// — at most a thousand attempts, started within a minute — and leaves the
/// rest to the next, and one send's rounds run one at a time.
/// Each attempt re-reads the notification's schedule, cancellation,
/// expiry, and the person's latest opt-out at the moment it fires, and
/// never holds a database transaction across the network call. The workers
/// supply the instance runtime from memory.
/// Arguments contain only logical send and round identifiers.
///
/// Delivery requests, including authentication refusals, are bounded by eight
/// attempts and the earlier of expiry or 24 hours after visibility. OAuth
/// exchanges do not spend push attempts; their failures retry within that same
/// horizon with sanitized operator-configuration status. Provider delays are
/// honored without shortening; a deadline wake terminates instead of pushing.
/// Only the service's own workers run it. No deadline: a scheduled send's first
/// round waits as long as its schedule says, like a wake job does.
#[rpc::job(
	service = "notifications",
	queue = "push",
	retries = 20,
	timeout = 5m,
	serialize_by = send_key
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

	/// Locale tags fold to canonical case, `_` separators become `-`, and
	/// anything that isn't a tag's shape is refused.
	#[test]
	fn locales_fold_to_canonical_case() {
		use super::canonical_locale;

		for (tag, canonical) in [
			("en-GB", "en-GB"),
			("EN_gb", "en-GB"),
			("en-gb", "en-GB"),
			("fr", "fr"),
			("zh-hant-tw", "zh-Hant-TW"),
			("ZH-HANS", "zh-Hans"),
			("es-419", "es-419"),
			("sr-Latn-RS", "sr-Latn-RS"),
			("de-DE-1996", "de-DE-1996"),
			("en-US-u-ca-gregory", "en-US-u-ca-gregory"),
			("EN-us-X-PRIVATE", "en-US-x-private"),
			("  pt-br  ", "pt-BR"),
			("und", "und"),
		] {
			assert_eq!(canonical_locale(tag).as_deref(), Ok(canonical), "{tag:?}");
		}

		for bad in [
			"",
			"   ",
			"e",
			"x-private",
			"1en",
			"en--GB",
			"en-",
			"-en",
			"en GB",
			"en-GB!",
			"en-abcdefghi",
			"français",
			&format!("en-{}", "a".repeat(40)),
		] {
			assert!(canonical_locale(bad).is_err(), "{bad:?} is not a tag");
		}
	}

	/// Time zones are checked for shape only, and app versions are free
	/// text with bounds.
	#[test]
	fn time_zones_and_app_versions_are_checked_for_shape() {
		use super::MAX_APP_VERSION_LEN;
		use super::MAX_TIME_ZONE_LEN;
		use super::check_app_version;
		use super::check_time_zone;

		for zone in [
			"Europe/London",
			"America/Argentina/Buenos_Aires",
			"America/Port-au-Prince",
			"Etc/GMT+5",
			"Etc/GMT-14",
			"UTC",
			"EST5EDT",
			// The shape, not the database: a zone newer than the server's
			// still enrolls.
			"Mars/Olympus_Mons",
		] {
			assert_eq!(check_time_zone(zone), Ok(()), "{zone}");
		}

		for bad in [
			"",
			"/Europe/London",
			"Europe/London/",
			"Europe//London",
			"Europe/../London",
			"Europe/London ",
			"1Europe/London",
			"Europe/Lon\u{0}don",
			&format!("Europe/{}", "L".repeat(MAX_TIME_ZONE_LEN)),
		] {
			assert!(check_time_zone(bad).is_err(), "{bad:?}");
		}

		for version in ["2.1.0", "2.1.0-beta.3", "2024.11 (812)", "nightly"] {
			assert_eq!(check_app_version(version), Ok(()), "{version}");
		}

		for bad in [
			"",
			"  ",
			"2.1\n",
			"2.1\u{7f}",
			&"9".repeat(MAX_APP_VERSION_LEN + 1),
		] {
			assert!(check_app_version(bad).is_err(), "{bad:?}");
		}
	}

	/// The service stores details normalized and refuses a bad one by
	/// name; a client's sanitized details drop a bad one instead, so an odd
	/// device never loses its enrollment over one.
	#[test]
	fn details_normalize_for_the_service_and_sanitize_for_clients() {
		use super::DeviceDetails;

		let reported = DeviceDetails {
			locale: Some(" en_gb ".into()),
			time_zone: Some("Europe/London".into()),
			app_version: Some(" 2.1.0 ".into()),
			apns_environment: None,
		};
		let stored = reported.normalized().unwrap();
		assert_eq!(
			stored,
			DeviceDetails {
				locale: Some("en-GB".into()),
				time_zone: Some("Europe/London".into()),
				app_version: Some("2.1.0".into()),
				apns_environment: None,
			}
		);
		assert_eq!(stored.language(), Some("en"));
		assert_eq!(reported.sanitized(), stored);

		// Blank is absent.
		let blank = DeviceDetails {
			locale: Some(String::new()),
			time_zone: Some("  ".into()),
			app_version: None,
			apns_environment: None,
		};
		assert_eq!(blank.normalized(), Ok(DeviceDetails::default()));
		assert!(blank.normalized().unwrap().is_empty());

		// One bad detail refuses the whole enrollment on the service,
		// naming it, and drops alone on a client.
		let odd = DeviceDetails {
			locale: Some("français".into()),
			time_zone: Some("Europe/London".into()),
			app_version: Some("2.1".into()),
			apns_environment: None,
		};
		let refusal = odd.normalized().unwrap_err();
		assert!(refusal.contains("locale"), "{refusal}");
		assert!(super::check_device_details(&odd).is_err());
		assert_eq!(
			odd.sanitized(),
			DeviceDetails {
				locale: None,
				time_zone: Some("Europe/London".into()),
				app_version: Some("2.1".into()),
				apns_environment: None,
			}
		);

		// The wire form leaves absent details out, and an old request with
		// none parses as none.
		assert_eq!(
			serde_json::to_value(&stored).unwrap(),
			serde_json::json!({
				"locale": "en-GB",
				"time_zone": "Europe/London",
				"app_version": "2.1.0",
			})
		);
		assert_eq!(
			serde_json::to_value(DeviceDetails::default()).unwrap(),
			serde_json::json!({})
		);
		let old: super::RegisterPushEndpoint = serde_json::from_value(serde_json::json!({
			"credential": { "apns": { "device_token": "a1b2c3d4".repeat(8) } }
		}))
		.unwrap();
		assert_eq!(old.device, DeviceDetails::default());
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
