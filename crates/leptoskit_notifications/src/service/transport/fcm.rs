//! The FCM transport: HTTP v1 `messages:send` against the project the
//! operator's service account names, authorized by an OAuth 2.0 access
//! token exchanged for that account's RS256-signed JWT.
//!
//! Google issues service-account keys as RSA, and the token endpoint
//! accepts only RS256 — the [service-account JWT flow][flow] is explicit
//! about both — so the assertion is built and signed here over ring's
//! constant-time RSA, the same primitives APNs signs its provider token
//! with, and a key that won't sign RS256 refuses at startup rather than
//! at the first delivery.
//!
//! The access token is short-lived by design, so it's exchanged on
//! demand, one exchange in flight at a time, and cached only until two
//! minutes before its `expires_in` runs out. A send the provider answers
//! `401` discards the cached token and exchanges once more. The next push
//! goes through the durable queue and spends another delivery attempt.
//! Exchange failures spend no push attempts; recovery ends at the finite
//! delivery deadline with sanitized operator-configuration status.
//!
//! The token endpoint is fixed in this file. A service-account file names
//! one in `token_uri`, and whatever it says is ignored: nothing read from
//! the operator's JSON — or from a caller, who only ever presents a
//! registration token — can steer where the exchange or the send goes.
//! The project comes from the account file, and the send URL from that
//! project.
//!
//! [flow]: https://developers.google.com/identity/protocols/oauth2/service-account
//! #jwt-assertion

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::SystemRandom;
use ring::signature::RSA_PKCS1_SHA256;
use ring::signature::RsaKeyPair;
use serde::Serialize;

use crate::push::DeliveryReason;
use crate::service::config::FcmConfig;
use crate::service::transport::Boundary;
use crate::service::transport::Message;
use crate::service::transport::Outcome;
use crate::service::transport::Shaped;
use crate::service::transport::Target;
use crate::service::transport::Transport;
use crate::service::transport::WireRequest;
use crate::service::transport::WireResponse;

/// The token endpoint the service account's JWT is exchanged at: fixed,
/// first party, and never read from the account's JSON, whose `token_uri`
/// is ignored on purpose.
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// The scope the exchange asks for: sending messages, and nothing else.
const SEND_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

/// The send endpoint's fixed host and path shape.
const SEND_HOST: &str = "https://fcm.googleapis.com/v1/projects";

/// The payload limit, in bytes: FCM's error reference counts "the total
/// size of the payload data included in a message", keys and values both
/// — what a device receives, not the request around it. See
/// [`payload_size`].
const PAYLOAD_LIMIT: usize = 4_096;

/// How early of an access token's lapse a cached one is retired.
const TOKEN_REFRESH_MARGIN_SECS: i64 = 120;

/// How long the assertion claims to live: the maximum the exchange takes.
const ASSERTION_LIFETIME_SECS: i64 = 3_600;

/// The shortest and longest `expires_in` a token answer may carry to be
/// believed: Google answers an hour, and anything outside this window is
/// a malformed answer rather than a cacheable token.
const MIN_EXPIRES_IN_SECS: i64 = 60;
const MAX_EXPIRES_IN_SECS: i64 = 24 * 3_600;

/// The longest an android TTL may be: four weeks, the platform's own
/// ceiling.
const MAX_TTL_SECS: i64 = 28 * 24 * 3_600;

/// The service account, as the operator's JSON file holds it.
pub(crate) struct Account {
	/// The account's email: the JWT's `iss`.
	pub email: String,
	/// The project pushes go out under: the send URL names it.
	pub project: String,
	/// The account's RSA private key, a PKCS#8 PEM string in the file.
	pub key_pem: String,
}

/// Reads the service account out of its JSON file. Only the three fields
/// the transport needs are read; `token_uri` in particular is ignored
/// (see [`TOKEN_ENDPOINT`]).
///
/// # Errors
///
/// A message naming the missing or mistyped field.
pub(crate) fn service_account(json: &str) -> Result<Account, String> {
	let parsed: serde_json::Value =
		serde_json::from_str(json).map_err(|_| "the FCM service account isn't JSON".to_owned())?;
	let field = |name: &str| -> Result<String, String> {
		parsed
			.get(name)
			.and_then(serde_json::Value::as_str)
			.map(str::to_owned)
			.ok_or_else(|| format!("the FCM service account has no {name}"))
	};

	let private_key = field("private_key")?;
	let email = field("client_email")?;
	let project = field("project_id")?;

	if email.is_empty()
		|| !email.contains('@')
		|| email.len() > 254
		|| project.is_empty()
		|| project.len() > 64
		|| project.bytes().any(|byte| {
			!(byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte == b'.')
		}) {
		return Err("the FCM service account's client_email or project_id is malformed".into());
	}

	Ok(Account {
		email,
		project,
		key_pem: private_key,
	})
}

/// One exchange's assertion claims.
#[derive(Serialize)]
struct AssertionClaims<'a> {
	iss: &'a str,
	scope: &'static str,
	aud: &'static str,
	iat: i64,
	exp: i64,
}

/// The FCM sender.
pub struct Fcm {
	account: Account,
	key: RsaKeyPair,
	boundary: std::sync::Arc<dyn Boundary>,
	/// The cached access token, and the epoch second it stops being
	/// trusted. Held in a tokio mutex so one exchange is in flight at a
	/// time: concurrent attempts share its answer instead of hammering
	/// the token endpoint.
	token: tokio::sync::Mutex<Option<(String, i64)>>,
}

impl Fcm {
	/// Builds the transport from the operator's configuration, parsing
	/// the account and its key now: the key must be an RSA private key
	/// that signs RS256, exactly what Google issues, and a key of any
	/// other kind refuses to start.
	///
	/// # Errors
	///
	/// A message naming what's wrong with the account or its key.
	pub fn new(config: &FcmConfig, boundary: std::sync::Arc<dyn Boundary>) -> Result<Self, String> {
		let account = service_account(&config.service_account_json)?;
		let der = super::apns::pem_body(&account.key_pem)
			.ok_or("FCM: the service account's private_key isn't an RSA PEM key".to_owned())?;
		let key = RsaKeyPair::from_pkcs8(&der).map_err(|_| {
			"FCM: the service account's private_key isn't an RSA PEM key".to_owned()
		})?;

		let built = Self {
			account,
			key,
			boundary,
			token: tokio::sync::Mutex::new(None),
		};

		// Prove the key signs RS256 now: a fine PEM of the wrong kind
		// fails here, at startup, not at the first delivery.
		built
			.assert_now(0)
			.map_err(|_| "FCM: the service account key isn't the RSA key RS256 needs".to_owned())?;

		Ok(built)
	}

	/// A signed assertion for `now`: the JWT-bearer grant's whole proof.
	fn assert_now(&self, now: i64) -> Result<String, ring::error::Unspecified> {
		let header = r#"{"alg":"RS256","typ":"JWT"}"#;
		let claims = serde_json::to_string(&AssertionClaims {
			iss: &self.account.email,
			scope: SEND_SCOPE,
			aud: TOKEN_ENDPOINT,
			iat: now,
			exp: now + ASSERTION_LIFETIME_SECS,
		})
		.expect("the assertion claims are plain JSON values");
		let signed = format!(
			"{}.{}",
			URL_SAFE_NO_PAD.encode(header),
			URL_SAFE_NO_PAD.encode(claims)
		);
		let mut signature = vec![0; self.key.public().modulus_len()];
		self.key.sign(
			&RSA_PKCS1_SHA256,
			&SystemRandom::new(),
			signed.as_bytes(),
			&mut signature,
		)?;
		Ok(format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature)))
	}

	/// The access token: cached until near its lapse, exchanged fresh
	/// when it's gone, with one exchange in flight at a time.
	///
	/// # Errors
	///
	/// A message for a refused exchange, an unusable answer, or a network
	/// failure — all the caller's retry policy's to see.
	async fn access_token(&self) -> Result<String, AccessFailure> {
		let now = leptoskit_db::now().timestamp();
		let mut held = self.token.lock().await;

		if let Some((token, until)) = held.as_ref()
			&& now < *until
		{
			return Ok(token.clone());
		}

		let assertion = self.assert_now(now).map_err(|_| AccessFailure::Unusable)?;

		let request = WireRequest {
			url: TOKEN_ENDPOINT.to_owned(),
			headers: vec![(
				"content-type".to_owned(),
				"application/x-www-form-urlencoded".to_owned(),
			)],
			body: format!(
				"grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&\
				 assertion={assertion}"
			)
			.into_bytes(),
			pinned: None,
		};
		let response = self
			.boundary
			.post(request)
			.await
			.map_err(|_| AccessFailure::Network)?;

		if response.status != 200 {
			// The account, its key, or its scopes: retrying immediately
			// won't help, and nothing about it is the endpoint's fault.
			return Err(AccessFailure::Unusable);
		}

		let answer: serde_json::Value =
			serde_json::from_slice(&response.body).map_err(|_| AccessFailure::Unusable)?;
		let access_token = answer
			.get("access_token")
			.and_then(serde_json::Value::as_str)
			.filter(|token| !token.is_empty())
			.ok_or(AccessFailure::Unusable)?;
		let expires_in = answer
			.get("expires_in")
			.and_then(serde_json::Value::as_i64)
			.ok_or(AccessFailure::Unusable)?;

		if !(MIN_EXPIRES_IN_SECS..=MAX_EXPIRES_IN_SECS).contains(&expires_in) {
			// A malformed lifetime is a malformed answer: nothing is
			// cached, and the caller decides again later.
			return Err(AccessFailure::Unusable);
		}

		*held = Some((
			access_token.to_owned(),
			now + expires_in - TOKEN_REFRESH_MARGIN_SECS,
		));

		Ok(access_token.to_owned())
	}

	/// Drops the cached access token: the provider refused it, so nothing
	/// should keep handing it out.
	async fn drop_token(&self) {
		*self.token.lock().await = None;
	}

	/// Posts one encoded message with `access_token`, answering the HTTP
	/// status next to its classified outcome: the retry path reads a 401
	/// from the status, not from a classification it would have to undo.
	async fn post(&self, access_token: &str, body: &[u8]) -> (u16, Outcome) {
		let request = WireRequest {
			url: format!("{SEND_HOST}/{}/messages:send", self.account.project),
			headers: std::iter::once((
				"authorization".to_owned(),
				format!("Bearer {access_token}"),
			))
			.chain(headers())
			.collect(),
			body: body.to_vec(),
			pinned: None,
		};
		let answer = self.boundary.post(request).await;

		match answer {
			Ok(response) => {
				let status = response.status;
				(status, classify(&response))
			}
			Err(_) => (0, Outcome::Transient { after: None }),
		}
	}
}

/// Why an access token couldn't be had: the network — which a delivery
/// retries like any transient — or the operator's account, which a
/// delivery waits on without spending its attempts.
enum AccessFailure {
	Network,
	Unusable,
}

impl Transport for Fcm {
	fn attempt<'a>(
		&'a self,
		target: Target<'a>,
		message: &'a Message,
	) -> Pin<Box<dyn Future<Output = Outcome> + Send + 'a>> {
		let send = async move {
			let credential = target.credential;

			// Size is checked on the payload before any network work,
			// trimmed in the shared order; past the id-only envelope it
			// is a terminal clear result with nothing sent.
			let shaped = shape(credential, message, leptoskit_db::now());

			if !shaped.fits {
				return Outcome::Ended {
					reason: DeliveryReason::Oversized,
					invalidate: false,
				};
			}

			self.send_encoded(shaped.body).await
		};

		Box::pin(send)
	}
}

impl Fcm {
	/// Exchanges an access token and posts one encoded message, answering
	/// the attempt's outcome.
	async fn send_encoded(&self, body: Vec<u8>) -> Outcome {
		// The operator's account configuration is not the endpoint's:
		// an unusable account stalls the delivery — visible, retrying
		// — rather than failing it.
		let access_token = match self.access_token().await {
			Ok(token) => token,
			Err(AccessFailure::Network) => return Outcome::Stalled { after: None },
			Err(AccessFailure::Unusable) => {
				return Outcome::Stalled { after: None };
			}
		};

		let (status, outcome) = self.post(&access_token, &body).await;

		// A 401 that names the project's own APNs or Web Push credential
		// ended already: a new access token wouldn't change it.
		if status == 401 && matches!(outcome, Outcome::AuthRejected { .. }) {
			// The provider refused this authorization: the cached
			// token is stale or wrong, so drop it, exchange once more
			// — bounded, once per request. A
			// token that was refused when fresh is the operator's
			// setup, and the delivery waits rather than ends.
			self.drop_token().await;

			// Refresh once, but send again only through the durable queue:
			// every push request must consume the eight-request budget.
			let _refresh = self.access_token().await;
			return Outcome::AuthRejected {
				after: Some(Duration::from_secs(1)),
			};
		}

		outcome
	}
}

/// One step of the documented degradation order, applied to the encoded
/// message in place: first the platform overrides and the image, then
/// the sound, the badge, and the link, then — the id-only envelope — the
/// send's `data` and everything else optional.
type Trim = fn(&mut serde_json::Value, &str, &Message, leptoskit_db::Timestamp);

/// The headers every FCM request carries beside its authorization.
fn headers() -> Vec<(String, String)> {
	vec![("content-type".to_owned(), "application/json".to_owned())]
}

/// What FCM is sent for `message` to the registration token `credential`,
/// as of `now` — which the remaining life it names as the TTL is measured
/// from: the encoded message, trimmed in the shared order — platform
/// overrides and the image, then sound, badge, and link, then the id-only
/// envelope — until what FCM counts fits its limit ([`payload_size`]).
pub(crate) fn shape(credential: &str, message: &Message, now: leptoskit_db::Timestamp) -> Shaped {
	let mut body_message = fcm_message(credential, message, Include::All, now);
	let mut trims = 0;

	for trim in [
		trim_platform_and_image as Trim,
		trim_sound_badge_link,
		trim_minimal,
	] {
		if payload_size(&body_message) <= PAYLOAD_LIMIT {
			break;
		}

		trim(&mut body_message, credential, message, now);
		trims += 1;
	}

	let bytes = payload_size(&body_message);

	Shaped {
		headers: headers(),
		body: serde_json::to_vec(&body_message).unwrap_or_default(),
		bytes,
		limit: PAYLOAD_LIMIT,
		trims,
		fits: bytes <= PAYLOAD_LIMIT,
	}
}

/// The platform overrides and the image go first. The Android
/// notification block stays — the sound and the badge it also carries
/// go in the next step, each in its own turn.
fn trim_platform_and_image(
	message: &mut serde_json::Value,
	_credential: &str,
	original: &Message,
	_now: leptoskit_db::Timestamp,
) {
	if let Some(root) = message
		.get_mut("message")
		.and_then(|message| message.as_object_mut())
	{
		if let Some(notification) = root.get_mut("notification").and_then(|n| n.as_object_mut()) {
			notification.remove("image");
		}

		if let Some(android) = root.get_mut("android").and_then(|a| a.as_object_mut())
			&& let Some(notification) = android
				.get_mut("notification")
				.and_then(|n| n.as_object_mut())
		{
			for key in ["channel_id", "color", "icon", "tag", "sticky"] {
				notification.remove(key);
			}

			// The tag a collapse key stands for stays with the key.
			if let Some(tag) = collapse_tag(original) {
				notification.insert("tag".to_owned(), serde_json::json!(tag));
			}
		}

		if let Some(apns) = root.get_mut("apns")
			&& let Some(payload) = apns.get_mut("payload").and_then(|p| p.as_object_mut())
		{
			payload.remove("fcm_options");

			if let Some(aps) = payload.get_mut("aps").and_then(|aps| aps.as_object_mut()) {
				for key in [
					"thread-id",
					"category",
					"interruption-level",
					"relevance-score",
					"mutable-content",
				] {
					aps.remove(key);
				}

				// The alert rides only to carry the subtitle.
				aps.remove("alert");
			}
		}

		drop_empty_aps(root);
	}
}

/// The sound, the badge, and the link go next.
fn trim_sound_badge_link(
	message: &mut serde_json::Value,
	_credential: &str,
	_original: &Message,
	_now: leptoskit_db::Timestamp,
) {
	if let Some(root) = message
		.get_mut("message")
		.and_then(|message| message.as_object_mut())
	{
		if let Some(data) = root.get_mut("data").and_then(|d| d.as_object_mut()) {
			data.remove("lklink");
			data.remove("lkbadge");
		}

		if let Some(notification) = root
			.get_mut("android")
			.and_then(|android| android.get_mut("notification"))
			.and_then(|n| n.as_object_mut())
		{
			notification.remove("sound");
			notification.remove("notification_count");
		}

		// The Android notification block may have emptied; an empty one
		// carries nothing and goes.
		if let Some(android) = root.get_mut("android").and_then(|a| a.as_object_mut())
			&& android
				.get("notification")
				.and_then(|n| n.as_object())
				.is_some_and(serde_json::Map::is_empty)
		{
			android.remove("notification");
		}

		if let Some(aps) = root
			.get_mut("apns")
			.and_then(|apns| apns.get_mut("payload"))
			.and_then(|payload| payload.get_mut("aps"))
			.and_then(|aps| aps.as_object_mut())
		{
			aps.remove("badge");
		}

		drop_empty_aps(root);
	}
}

/// An `aps` the trims emptied carries nothing and goes, as the payload
/// built without those options never had one.
fn drop_empty_aps(root: &mut serde_json::Map<String, serde_json::Value>) {
	if let Some(payload) = root
		.get_mut("apns")
		.and_then(|apns| apns.get_mut("payload"))
		.and_then(|payload| payload.as_object_mut())
		&& payload
			.get("aps")
			.and_then(|aps| aps.as_object())
			.is_some_and(serde_json::Map::is_empty)
	{
		payload.remove("aps");
	}
}

/// The id-only envelope: today's minimal message, which for a data
/// message keeps the background shape and its id, because a silent push
/// must stay silent.
fn trim_minimal(
	message: &mut serde_json::Value,
	credential: &str,
	original: &Message,
	now: leptoskit_db::Timestamp,
) {
	*message = fcm_message(credential, original, Include::Minimal, now);
}

/// What a message carries beside its ids: everything, or the id-only
/// envelope the provider's payload limit can force.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Include {
	All,
	Minimal,
}

/// The HTTP v1 message: the standard visible notification — or, for a
/// data send, no notification block of any kind, because FCM shows a
/// message that has one in the tray instead of handing it to the app —
/// the send's `data` as the
/// string-valued map the API asks for, the notification's id as the
/// collapse key and in `data`, the same collapse id for an iOS app the
/// project also talks to, and the notification's remaining life as the
/// TTL both platforms hold it for, so a provider can't deliver past the
/// send's expiry. The send's [options](crate::MessageOptions) shape the
/// Android notification, the iOS payload, and the priority, each where
/// its platform reads it.
///
/// [`Include::Minimal`] is the id-only envelope, when the full one
/// doesn't fit the provider's limit. The collapse key and headers stay
/// either way: a fallback that dropped them would let a repeat of the
/// same notification stack banners the full payload would have replaced.
/// Collapse is best-effort deduplication, though — a provider that never
/// coalesces, or a device that was offline, can still show two.
fn fcm_message(
	credential: &str,
	message: &Message,
	include: Include,
	now: leptoskit_db::Timestamp,
) -> serde_json::Value {
	let ttl = message
		.expires_at
		.map(|expires_at| (expires_at - now).num_seconds().clamp(0, MAX_TTL_SECS));

	let mut root = serde_json::Map::new();
	root.insert("token".to_owned(), serde_json::json!(credential));

	if !message.options.is_data() {
		root.insert(
			"notification".to_owned(),
			fcm_notification(message, include),
		);
	}

	root.insert(
		"data".to_owned(),
		serde_json::Value::Object(fcm_data_map(message, include)),
	);
	root.insert("android".to_owned(), fcm_android(message, include, ttl));
	// An iOS app served through the same project sees the same alert,
	// collapse id, and expiry as a direct APNs send would give it.
	root.insert("apns".to_owned(), fcm_apns(message, include, ttl));

	serde_json::json!({
		"message": serde_json::Value::Object(root),
	})
}

/// The visible notification block — which a data message carries none
/// of — with the image when there is one and room for it.
fn fcm_notification(message: &Message, include: Include) -> serde_json::Value {
	let mut notification = serde_json::json!({
		"title": message.title,
		"body": message.body,
	});

	if include == Include::All && message.options.image.is_some() {
		notification["image"] = serde_json::json!(message.options.image);
	}

	notification
}

/// The string-valued `data` map: the caller's own object, the link the
/// send names, a data message's badge, and the notification's id — with
/// the envelope's own keys and the platform's reserved words never let
/// through.
fn fcm_data_map(message: &Message, include: Include) -> serde_json::Map<String, serde_json::Value> {
	let mut map = serde_json::Map::new();

	if include == Include::All {
		let parsed: serde_json::Value = serde_json::from_str(&message.data).unwrap_or_default();

		if let Some(object) = parsed.as_object() {
			for (name, value) in object {
				// The envelope's own keys and FCM's reserved words are
				// dropped from the caller's data rather than overwritten
				// or sent: `lkn`, `lklink`, `lkimg`, and `lkbadge` are
				// ours, and `from`, `message_type`, and any key starting
				// `google.` or `gcm.` (`gcm.notification.` among them) are
				// the words the HTTP v1 reference reserves, which FCM
				// answers with a 400 for the whole message. A send is
				// for every platform, so the key still reaches APNs and
				// browsers.
				if matches!(
					name.as_str(),
					"lkn" | "lklink" | "lkimg" | "lkbadge" | "from" | "message_type"
				) || name.starts_with("google.")
					|| name.starts_with("gcm.")
				{
					continue;
				}

				// The data map is string-valued: anything nested goes as
				// its JSON text.
				map.insert(
					name.clone(),
					serde_json::Value::String(match value {
						serde_json::Value::String(text) => text.clone(),
						other => other.to_string(),
					}),
				);
			}
		}

		if let Some(link) = message.options.link.as_deref() {
			map.insert(
				"lklink".to_owned(),
				serde_json::Value::String(link.to_owned()),
			);
		}

		// A data message has no notification to badge: its badge rides as
		// data, for the app to apply itself.
		if message.options.is_data()
			&& let Some(badge) = message.badge
		{
			map.insert(
				"lkbadge".to_owned(),
				serde_json::Value::String(badge.to_string()),
			);
		}
	}

	map.insert(
		"lkn".to_owned(),
		serde_json::Value::String(message.id.to_string()),
	);

	map
}

/// The `android` config: the collapse key and TTL every message carries,
/// the priority the send named, and — when there is room, and only for a
/// visible notification — the notification block the Android options
/// shape. An Android notification block on a data message would make FCM
/// show it in the tray.
fn fcm_android(message: &Message, include: Include, ttl: Option<i64>) -> serde_json::Value {
	let mut android = serde_json::json!({
		"collapse_key": message.collapse_key(),
	});

	if let Some(ttl) = ttl
		&& let Some(object) = android.as_object_mut()
	{
		object.insert(
			"ttl".to_owned(),
			serde_json::Value::String(format!("{ttl}s")),
		);
	}

	// Priority rides in the Android config, and — like the collapse key
	// and the TTL — survives the id-only envelope: it is two bytes of
	// intent, not decoration.
	match message.options.priority {
		Some(crate::options::SendPriority::High) => {
			android["priority"] = serde_json::json!("HIGH");
		}
		Some(crate::options::SendPriority::Normal) => {
			android["priority"] = serde_json::json!("NORMAL");
		}
		None => {}
	}

	if message.options.is_data() {
		return android;
	}

	if include != Include::All {
		// The tag a collapse key stands for is intent, like the key: it
		// stays when the decoration around it goes.
		if let Some(tag) = collapse_tag(message) {
			android["notification"] = serde_json::json!({ "tag": tag });
		}

		return android;
	}

	let mut notification = serde_json::Map::new();

	if let Some(tag) = collapse_tag(message) {
		notification.insert("tag".to_owned(), serde_json::json!(tag));
	}

	if let Some(android_options) = message.options.android.as_ref() {
		if let Some(channel) = android_options.raw_channel_id.as_deref() {
			// A channel the app made itself, before the plugin: named as
			// the device holds it.
			notification.insert("channel_id".to_owned(), serde_json::json!(channel));
		} else if let Some(channel) = android_options.channel_id.as_deref() {
			notification.insert(
				"channel_id".to_owned(),
				serde_json::json!(android_channel(channel)),
			);
		}

		let fields = [
			("color", android_options.color.as_deref()),
			("icon", android_options.icon.as_deref()),
			("tag", android_options.tag.as_deref()),
		];

		for (name, value) in fields {
			if let Some(value) = value {
				notification.insert(name.to_owned(), serde_json::json!(value));
			}
		}

		if let Some(sticky) = android_options.sticky {
			notification.insert("sticky".to_owned(), serde_json::json!(sticky));
		}
	}

	match message.options.sound.as_ref() {
		Some(crate::options::NotificationSound::Default) => {
			notification.insert("sound".to_owned(), serde_json::json!("default"));
		}
		Some(crate::options::NotificationSound::Named(name)) => {
			notification.insert("sound".to_owned(), serde_json::json!(name));
		}
		// Unset and Silent both leave the sound to the channel: a
		// channel is the only silence Android honors per notification.
		None | Some(crate::options::NotificationSound::Silent) => {}
	}

	if let Some(badge) = message.badge {
		notification.insert("notification_count".to_owned(), serde_json::json!(badge));
	}

	if !notification.is_empty() {
		android["notification"] = serde_json::Value::Object(notification);
	}

	android
}

/// The Android tag a send's collapse key stands for, when the send names
/// no tag of its own. FCM's `collapse_key` replaces only a message a
/// device has not been handed yet; the tag is what replaces a banner
/// already in the tray.
fn collapse_tag(message: &Message) -> Option<&str> {
	message
		.options
		.android
		.as_ref()
		.and_then(|android| android.tag.as_deref())
		.is_none()
		.then_some(message.options.collapse_key.as_deref())
		.flatten()
}

/// The prefix the local-notifications plugin gives every Android channel it
/// creates: `configure_channel` with the id `inbox` makes the channel
/// `leptoskit.notifications.inbox`.
const ANDROID_CHANNEL_PREFIX: &str = "leptoskit.notifications.";

/// The channel id Android knows a send's `android.channel_id` by. A send
/// names the id the app gave the plugin, and the device holds that channel
/// under the plugin's prefix, so the prefix is added here: the raw id names
/// no channel, and Firebase posts a message whose channel doesn't exist
/// through its fallback instead. An id that already carries the prefix is
/// sent as it is, so the full name works too.
fn android_channel(id: &str) -> std::borrow::Cow<'_, str> {
	if id.starts_with(ANDROID_CHANNEL_PREFIX) {
		std::borrow::Cow::Borrowed(id)
	} else {
		std::borrow::Cow::Owned(format!("{ANDROID_CHANNEL_PREFIX}{id}"))
	}
}

/// The `apns` block FCM serves an iOS app through the same project: the
/// collapse header every message carries, the expiry and the
/// background-or-priority headers the send implies, and what a direct
/// APNs send would add — for a data message `content-available` alone,
/// and otherwise the Apple options, the badge, and the image under the
/// mutable-content flag. FCM builds the iOS alert from the message's
/// `notification`, so the title and body don't ride here a second time
/// unless a subtitle needs the alert they belong to.
fn fcm_apns(message: &Message, include: Include, ttl: Option<i64>) -> serde_json::Value {
	let data = message.options.is_data();
	let mut headers = serde_json::Map::new();

	headers.insert(
		"apns-collapse-id".to_owned(),
		serde_json::Value::String(message.collapse_key().into_owned()),
	);

	// A data send's iOS half is a background push, as a direct APNs send
	// would be; an explicit priority rides the header too. A plain
	// notification adds neither, exactly as before options existed.
	if data {
		headers.insert("apns-push-type".to_owned(), serde_json::json!("background"));
		headers.insert("apns-priority".to_owned(), serde_json::json!("5"));
	} else if let Some(priority) = message.options.priority {
		headers.insert(
			"apns-priority".to_owned(),
			serde_json::json!(match priority {
				crate::options::SendPriority::High => "10",
				crate::options::SendPriority::Normal => "5",
			}),
		);
	}

	if ttl.is_some() {
		headers.insert(
			"apns-expiration".to_owned(),
			serde_json::Value::String(message.apns_expiration().unwrap_or(0).to_string()),
		);
	}

	let mut aps = if data {
		serde_json::json!({
			"content-available": 1,
		})
	} else {
		serde_json::json!({})
	};

	if include == Include::All && !data {
		apply_apple_options(&mut aps, message);
		apply_badge_and_image(&mut aps, message);
	}

	let mut payload = serde_json::json!({
		"lkn": message.id.to_string(),
	});

	if aps.as_object().is_some_and(|aps| !aps.is_empty()) {
		payload["aps"] = aps;
	}

	if include == Include::All && message.options.image.is_some() && !data {
		payload["fcm_options"] = serde_json::json!({
			"image": message.options.image,
		});
	}

	serde_json::json!({
		"headers": serde_json::Value::Object(headers),
		"payload": payload,
	})
}

/// The Apple options into `aps`: a subtitle on the alert, and the
/// grouping and interruption keys beside it. An `apns.payload` alert
/// takes the place of the notification's title and body, so a subtitle
/// carries them too.
fn apply_apple_options(aps: &mut serde_json::Value, message: &Message) {
	let Some(apple) = message.options.apple.as_ref() else {
		return;
	};

	if let Some(subtitle) = apple.subtitle.as_deref() {
		aps["alert"] = serde_json::json!({
			"title": message.title,
			"body": message.body,
			"subtitle": subtitle,
		});
	}

	if let Some(thread_id) = apple.thread_id.as_deref() {
		aps["thread-id"] = serde_json::json!(thread_id);
	}

	if let Some(category) = apple.category.as_deref() {
		aps["category"] = serde_json::json!(category);
	}

	if let Some(level) = apple.interruption_level {
		aps["interruption-level"] = serde_json::json!(level.as_str());
	}

	if let Some(score) = apple.relevance_score {
		aps["relevance-score"] = serde_json::json!(score);
	}
}

/// The badge and the image flag into `aps`. An image needs a
/// Notification Service Extension in the iOS app to display: the
/// mutable-content flag hands the payload to it.
fn apply_badge_and_image(aps: &mut serde_json::Value, message: &Message) {
	if let Some(badge) = message.badge {
		aps["badge"] = serde_json::json!(badge);
	}

	if message.options.image.is_some() {
		aps["mutable-content"] = serde_json::json!(1);
	}
}

/// The bytes FCM's limit counts: the keys and values of the payload a
/// device receives — the notification, the data, and the Android
/// notification options delivered with them — not the token, the
/// collapse key, the TTL, or the JSON around them. The APNs payload an
/// iOS app of the same project receives counts too, encoded, so one
/// measure covers whichever platform the token belongs to.
fn payload_size(message: &serde_json::Value) -> usize {
	let fields = |value: &serde_json::Value| -> usize {
		value.as_object().map_or(0, |object| {
			object
				.iter()
				.map(|(key, value)| {
					key.len()
						+ match value {
							serde_json::Value::String(text) => text.len(),
							other => other.to_string().len(),
						}
				})
				.sum()
		})
	};
	let root = &message["message"];

	fields(&root["notification"])
		+ fields(&root["data"])
		+ fields(&root["android"]["notification"])
		+ root["apns"]
			.get("payload")
			.map_or(0, |payload| payload.to_string().len())
}

/// Classifies one FCM answer, as the HTTP v1 error reference documents
/// each code. The provider's status strings are read here for
/// classification and never surface anywhere else.
///
/// Only `UNREGISTERED` invalidates the endpoint. `SENDER_ID_MISMATCH` —
/// the token belongs to another project — and the third-party errors —
/// the project's APNs or Web Push credential is missing or wrong — are
/// the operator's setup, and no retry fixes them, so they end the
/// delivery as the operator's configuration with the device enrolled.
fn classify(response: &WireResponse) -> Outcome {
	if (response.status == 401 || response.status == 403)
		&& matches!(
			fcm_error_code(response).as_deref(),
			Some("SENDER_ID_MISMATCH" | "THIRD_PARTY_AUTH_ERROR" | "THIRD_PARTY_PERMISSION_DENIED")
		) {
		return Outcome::Ended {
			reason: DeliveryReason::OperatorConfiguration,
			invalidate: false,
		};
	}

	match response.status {
		200..=299 => Outcome::Accepted,
		404 => {
			// Only the error detail that says the registration token is
			// gone may invalidate the endpoint: a plain 404 on the send
			// URL is the operator's project naming, not a dead device.
			if unregistered(response) {
				return Outcome::Ended {
					reason: DeliveryReason::Unregistered,
					invalidate: true,
				};
			}

			Outcome::AuthRejected { after: None }
		}
		400 => {
			// A registration token the project doesn't know is dead
			// here; a payload-shaped 400 is the send's own, and the
			// endpoint stays.
			if unregistered(response) {
				return Outcome::Ended {
					reason: DeliveryReason::Unregistered,
					invalidate: true,
				};
			}

			Outcome::Ended {
				reason: DeliveryReason::ProviderRejected,
				invalidate: false,
			}
		}
		401 | 403 => {
			// Authorization is the operator's account: the send's retry
			// path refreshes once, and a refusal that survives it stalls
			// the delivery instead of ending it.
			Outcome::AuthRejected { after: None }
		}
		429 => {
			Outcome::Transient {
				after: Some(
					retry_after(response)
						.unwrap_or_default()
						.max(Duration::from_secs(60)),
				),
			}
		}
		500..=599 => {
			Outcome::Transient {
				after: retry_after(response),
			}
		}
		// Anything unrecognized — redirects included, which the boundary
		// refuses — ends the delivery the same clear way.
		_ => {
			Outcome::Ended {
				reason: DeliveryReason::ProviderRejected,
				invalidate: false,
			}
		}
	}
}

/// Only an FCM-specific detail identifies an unregistered token.
fn unregistered(response: &WireResponse) -> bool {
	fcm_error_code(response).as_deref() == Some("UNREGISTERED")
}

/// The FCM-specific error code an answer's details carry, if any: only a
/// detail typed as FCM's own counts.
fn fcm_error_code(response: &WireResponse) -> Option<String> {
	let parsed = serde_json::from_slice::<serde_json::Value>(response.body.trim_ascii()).ok()?;

	parsed["error"]["details"]
		.as_array()?
		.iter()
		.find(|detail| {
			detail["@type"].as_str() == Some("type.googleapis.com/google.firebase.fcm.v1.FcmError")
		})?["errorCode"]
		.as_str()
		.map(str::to_owned)
}

fn retry_after(response: &WireResponse) -> Option<Duration> {
	super::retry_after(response, std::time::SystemTime::now())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_channel_id_is_sent_under_the_plugins_prefix_exactly_once() {
		assert_eq!(android_channel("inbox"), "leptoskit.notifications.inbox");
		assert_eq!(
			android_channel("leptoskit.notifications.inbox"),
			"leptoskit.notifications.inbox"
		);
		// Only the whole prefix counts as already carrying it.
		assert_eq!(
			android_channel("leptoskit.inbox"),
			"leptoskit.notifications.leptoskit.inbox"
		);
	}

	#[test]
	fn quota_minimum_and_server_retry_after_are_honored() {
		let mut response = WireResponse {
			status: 429,
			headers: vec![("retry-after".into(), "1".into())],
			body: vec![],
		};
		assert_eq!(
			classify(&response),
			Outcome::Transient {
				after: Some(Duration::from_secs(60))
			}
		);
		response.status = 503;
		response.headers[0].1 = "60000".into();
		assert_eq!(
			classify(&response),
			Outcome::Transient {
				after: Some(Duration::from_mins(1000))
			}
		);
	}

	fn fcm_answer(status: u16, error: &str, code: &str) -> WireResponse {
		let body = if code.is_empty() {
			serde_json::json!({ "error": { "status": error } })
		} else {
			serde_json::json!({
				"error": {
					"status": error,
					"details": [{
						"@type": "type.googleapis.com/google.firebase.fcm.v1.FcmError",
						"errorCode": code,
					}],
				},
			})
		};

		WireResponse {
			status,
			headers: Vec::new(),
			body: body.to_string().into_bytes(),
		}
	}

	/// Every error code the FCM HTTP v1 reference documents, and what
	/// each comes to. Only `UNREGISTERED` invalidates the endpoint. The
	/// answers that name the sender's setup — another project's token, a
	/// missing APNs or Web Push credential in the Firebase project — end
	/// at once as the operator's configuration, since no retry fixes them;
	/// a refused access token is refreshed and retried; the rest of the
	/// 4xx answers are the request's own.
	#[test]
	fn fcm_answers_classify_as_the_reference_documents_them() {
		let configuration = Outcome::Ended {
			reason: DeliveryReason::OperatorConfiguration,
			invalidate: false,
		};
		let rejected = Outcome::Ended {
			reason: DeliveryReason::ProviderRejected,
			invalidate: false,
		};
		let gone = Outcome::Ended {
			reason: DeliveryReason::Unregistered,
			invalidate: true,
		};
		let auth = Outcome::AuthRejected { after: None };

		for (status, error, code, expected) in [
			(200, "", "", Outcome::Accepted),
			(404, "NOT_FOUND", "UNREGISTERED", gone),
			(400, "INVALID_ARGUMENT", "UNREGISTERED", gone),
			(400, "INVALID_ARGUMENT", "INVALID_ARGUMENT", rejected),
			(400, "INVALID_ARGUMENT", "", rejected),
			(401, "UNAUTHENTICATED", "", auth),
			(
				401,
				"UNAUTHENTICATED",
				"THIRD_PARTY_AUTH_ERROR",
				configuration,
			),
			(
				403,
				"PERMISSION_DENIED",
				"SENDER_ID_MISMATCH",
				configuration,
			),
			(
				403,
				"PERMISSION_DENIED",
				"THIRD_PARTY_PERMISSION_DENIED",
				configuration,
			),
			(403, "PERMISSION_DENIED", "", auth),
			(404, "NOT_FOUND", "", auth),
			(
				429,
				"RESOURCE_EXHAUSTED",
				"QUOTA_EXCEEDED",
				Outcome::Transient {
					after: Some(Duration::from_secs(60)),
				},
			),
			(
				500,
				"INTERNAL",
				"INTERNAL",
				Outcome::Transient { after: None },
			),
			(
				503,
				"UNAVAILABLE",
				"UNAVAILABLE",
				Outcome::Transient { after: None },
			),
			(302, "", "", rejected),
		] {
			assert_eq!(
				classify(&fcm_answer(status, error, code)),
				expected,
				"{status} {error} {code}"
			);
		}
	}

	#[test]
	fn unregistered_requires_an_fcm_typed_detail() {
		let response = |body: &str| {
			WireResponse {
				status: 404,
				headers: Vec::new(),
				body: body.as_bytes().to_vec(),
			}
		};

		assert!(unregistered(&response(
			r#"{"error":{"status":"NOT_FOUND","details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":"UNREGISTERED"}]}}"#
		)));
		assert!(!unregistered(&response(
			r#"{"error":{"status":"NOT_FOUND"}}"#
		)));
		assert!(!unregistered(&response(
			r#"{"error":{"details":[{"errorCode":"UNREGISTERED"}]}}"#
		)));
		assert!(!unregistered(&response(
			r#"{"error":{"details":[{"@type":"type.googleapis.com/google.rpc.BadRequest","errorCode":"UNREGISTERED"}]}}"#
		)));
	}
}
