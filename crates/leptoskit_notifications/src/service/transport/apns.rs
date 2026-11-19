//! The APNs transport: HTTP/2 POSTs to `/3/device/<token>` with a
//! provider token signed ES256 from the operator's `.p8` key.
//!
//! The provider token is a JWT — header `{"alg":"ES256","kid":<key id>}`,
//! claims `{"iss":<team id>,"iat":now}` — valid an hour, so it's signed
//! on demand and cached until ten minutes before it lapses. The topic and
//! the sandbox or production host are the operator's configuration: a
//! caller's enrollment carries only the device token, so nothing a
//! caller sends can steer where a push really goes.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::SystemRandom;
use ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING;
use ring::signature::EcdsaKeyPair;

use crate::push::DeliveryReason;
use crate::service::config::ApnsConfig;
use crate::service::transport::Boundary;
use crate::service::transport::Message;
use crate::service::transport::Outcome;
use crate::service::transport::Shaped;
use crate::service::transport::Transport;
use crate::service::transport::WireRequest;
use crate::service::transport::WireResponse;

/// The production host.
const HOST: &str = "https://api.push.apple.com";

/// The sandbox host, for development builds.
const SANDBOX_HOST: &str = "https://api.sandbox.push.apple.com";

/// The provider token's whole life, as APNs grants it.
const TOKEN_LIFETIME_SECS: i64 = 3_600;

/// How early of the token's lapse a cached one is retired.
const TOKEN_REFRESH_MARGIN_SECS: i64 = 600;

/// The provider's payload limit, in bytes.
const PAYLOAD_LIMIT: usize = 4_096;

/// A PEM body between its armor lines, without the line breaks.
pub(crate) fn pem_body(pem: &str) -> Option<Vec<u8>> {
	let mut lines = pem
		.lines()
		.skip_while(|line| !line.starts_with("-----BEGIN"));
	let header = lines.next()?;

	if !header.ends_with("PRIVATE KEY-----") {
		return None;
	}

	let mut der = Vec::new();

	for line in lines {
		if line.starts_with("-----END") {
			return Some(der);
		}

		let decoded = base64::engine::general_purpose::STANDARD
			.decode(line.trim())
			.ok()?;
		der.extend_from_slice(&decoded);
	}

	None
}

/// The `.p8` signing key parsed once, so a broken key fails the service
/// at startup rather than at the first delivery.
///
/// # Errors
///
/// A message naming what's wrong with the key.
pub(crate) fn signing_key(key_p8: &str) -> Result<EcdsaKeyPair, String> {
	let der = pem_body(key_p8).ok_or("the key isn't a .p8 PEM file")?;
	EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &SystemRandom::new())
		.map_err(|_| "the key isn't a PKCS#8 P-256 private key".to_owned())
}

/// The APNs sender.
pub struct Apns {
	key: EcdsaKeyPair,
	key_id: String,
	team_id: String,
	topic: String,
	host: &'static str,
	boundary: std::sync::Arc<dyn Boundary>,
	/// The cached provider token, and the epoch second it stops being
	/// trusted.
	token: Mutex<Option<(String, i64)>>,
}

impl Apns {
	/// Builds the transport from the operator's configuration, parsing
	/// the key now.
	///
	/// # Errors
	///
	/// A message naming what's wrong with the key, the ids, the topic, or
	/// the environment.
	pub fn new(
		config: &ApnsConfig,
		boundary: std::sync::Arc<dyn Boundary>,
	) -> Result<Self, String> {
		let key = signing_key(&config.key_p8).map_err(|error| format!("APNs: {error}"))?;
		let key_id = config.key_id.trim();

		if key_id.is_empty() || key_id.len() > 64 {
			return Err("APNs: the key id is 1 to 64 characters".into());
		}

		let team_id = config.team_id.trim();

		if team_id.is_empty() || team_id.len() > 64 {
			return Err("APNs: the team id is 1 to 64 characters".into());
		}

		let topic = config.topic.trim();

		if topic.is_empty() || topic.len() > 250 {
			return Err("APNs: the topic is 1 to 250 characters".into());
		}

		let host = match config.environment {
			crate::service::config::ApnsEnvironment::Production => HOST,
			crate::service::config::ApnsEnvironment::Sandbox => SANDBOX_HOST,
		};

		Ok(Self {
			key,
			key_id: key_id.to_owned(),
			team_id: team_id.to_owned(),
			topic: topic.to_owned(),
			host,
			boundary,
			token: Mutex::new(None),
		})
	}

	/// The provider token: cached until near its lapse, signed fresh when
	/// it's gone.
	fn provider_token(&self) -> Result<String, String> {
		let now = leptoskit_db::now().timestamp();
		let cached = self
			.token
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
			.clone();

		if let Some((token, until)) = cached
			&& now < until
		{
			return Ok(token);
		}

		let header = format!(
			"{{\"alg\":\"ES256\",\"kid\":{}}}",
			serde_json::to_string(&self.key_id).expect("a plain string")
		);
		let claims = format!(
			"{{\"iss\":{},\"iat\":{now}}}",
			serde_json::to_string(&self.team_id).expect("a plain string")
		);
		let signed = format!(
			"{}.{}",
			URL_SAFE_NO_PAD.encode(header),
			URL_SAFE_NO_PAD.encode(claims)
		);
		let signature = self
			.key
			.sign(&SystemRandom::new(), signed.as_bytes())
			.map_err(|_| "APNs: the key couldn't sign a provider token".to_owned())?;
		let token = format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()));

		*self
			.token
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner) = Some((
			token.clone(),
			now + TOKEN_LIFETIME_SECS - TOKEN_REFRESH_MARGIN_SECS,
		));

		Ok(token)
	}

	/// Drops the cached provider token when it is the one APNs refused, so
	/// the next attempt signs a fresh one. A token another attempt already
	/// replaced stays: every delivery that saw the same refusal clears it
	/// once, not once each, and Apple asks for a new token no more than
	/// every twenty minutes.
	fn drop_token(&self, refused: &str) {
		let mut cached = self
			.token
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);

		if cached.as_ref().is_some_and(|(token, _)| token == refused) {
			*cached = None;
		}
	}

	/// Posts one shaped payload and classifies the answer.
	async fn post(&self, credential: &str, token: &str, shaped: Shaped) -> Outcome {
		let mut headers = vec![
			("authorization".to_owned(), format!("Bearer {token}")),
			("apns-topic".to_owned(), self.topic.clone()),
		];
		headers.extend(shaped.headers);
		let payload = shaped.body;

		let request = WireRequest {
			url: format!("{}/3/device/{credential}", self.host),
			headers,
			body: payload,
			pinned: None,
		};
		let answer = self.boundary.post(request).await;

		match answer {
			Ok(response) => {
				let outcome = classify(&response);

				// A refused provider token is this server's: the retry
				// signs a new one rather than presenting it again.
				if matches!(outcome, Outcome::AuthRejected { .. }) {
					self.drop_token(token);
				}

				outcome
			}
			Err(_) => Outcome::Transient { after: None },
		}
	}
}

impl Transport for Apns {
	fn attempt<'a>(
		&'a self,
		credential: &'a str,
		message: &'a Message,
	) -> Pin<Box<dyn Future<Output = Outcome> + Send + 'a>> {
		let send = async move {
			// A signing failure is the operator's configuration, not the
			// endpoint's: the delivery retries, and the send's status
			// shows it stuck.
			let Ok(token) = self.provider_token() else {
				return Outcome::Transient { after: None };
			};

			// Size is checked on the encoded payload, trimmed in the
			// shared order; past the id-only envelope it is a terminal
			// clear result with no request.
			let shaped = shape(message);

			if !shaped.fits {
				return Outcome::Ended {
					reason: DeliveryReason::Oversized,
					invalidate: false,
				};
			}

			self.post(credential, &token, shaped).await
		};

		Box::pin(send)
	}
}

/// The headers every push of `message` carries, whoever it goes to: the
/// push type and priority, the notification's id as the request id and
/// the collapse id, and the expiry when the send names one.
fn headers(message: &Message) -> Vec<(String, String)> {
	// A silent data message is a background push: Apple requires the
	// background push type, and priority 5 with it. A visible
	// notification keeps the alert type and — as before options
	// existed — priority 10 unless the send said otherwise.
	let background = message.options.is_data();
	let priority = if background {
		"5"
	} else {
		match message.options.priority {
			Some(crate::options::SendPriority::Normal) => "5",
			Some(crate::options::SendPriority::High) | None => "10",
		}
	};
	let mut headers = vec![
		(
			"apns-push-type".to_owned(),
			if background { "background" } else { "alert" }.to_owned(),
		),
		("apns-priority".to_owned(), priority.to_owned()),
		("apns-id".to_owned(), message.id.to_string()),
		// The stable collapse id: a repeat replaces, not stacks. The
		// send's own key when it chose one, the notification's id
		// otherwise.
		(
			"apns-collapse-id".to_owned(),
			message.collapse_key().into_owned(),
		),
	];

	if let Some(expiration) = message.apns_expiration() {
		headers.push(("apns-expiration".to_owned(), expiration.to_string()));
	}

	headers
}

/// What APNs is sent for `message`: the encoded payload, trimmed in the
/// shared order — platform overrides and the image, then sound, badge,
/// and link, then the id-only envelope — until it fits the limit, which
/// Apple counts on the encoded JSON.
pub(crate) fn shape(message: &Message) -> Shaped {
	let mut payload = apns_payload(message);
	let mut trims = 0;

	for trim in [Trim::PlatformAndImage, Trim::SoundBadgeLink, Trim::Minimal] {
		if serde_json::to_vec(&payload).unwrap_or_default().len() <= PAYLOAD_LIMIT {
			break;
		}

		trim.apply(&mut payload, message);
		trims += 1;
	}

	let body = serde_json::to_vec(&payload).unwrap_or_default();

	Shaped {
		headers: headers(message),
		bytes: body.len(),
		limit: PAYLOAD_LIMIT,
		trims,
		fits: body.len() <= PAYLOAD_LIMIT,
		body,
	}
}

/// One step of the documented degradation order, applied to the payload
/// in place: first the platform overrides and the image, then the sound,
/// the badge, and the link, then — the id-only envelope — the send's
/// `data` and everything else optional.
enum Trim {
	PlatformAndImage,
	SoundBadgeLink,
	Minimal,
}

impl Trim {
	/// What this step drops.
	fn apply(self, payload: &mut serde_json::Value, message: &Message) {
		match self {
			Self::PlatformAndImage => {
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

					if let Some(alert) =
						aps.get_mut("alert").and_then(|alert| alert.as_object_mut())
					{
						alert.remove("subtitle");
					}
				}

				if let Some(object) = payload.as_object_mut() {
					object.remove("lkimg");
				}
			}
			Self::SoundBadgeLink => {
				if let Some(aps) = payload.get_mut("aps").and_then(|aps| aps.as_object_mut()) {
					aps.remove("sound");
					aps.remove("badge");
				}

				if let Some(object) = payload.as_object_mut() {
					object.remove("lklink");
					object.remove("lkbadge");
				}
			}
			Self::Minimal => {
				// The id-only envelope: today's minimal payload, which
				// for a data message keeps the background shape and its
				// id, because a silent push must stay silent.
				*payload = apns_minimal_payload(message);
			}
		}
	}
}

/// The full payload: the visible alert — or a background message's
/// `content-available` — the notification's id for routing and
/// deduplication, the send's `data`, and every option that shapes the
/// push: badge, sound, link, and the Apple overrides.
fn apns_payload(message: &Message) -> serde_json::Value {
	let mut payload = apns_minimal_payload(message);

	if let Some(object) = data_object(message)
		&& !object.is_empty()
	{
		payload["lk"] = serde_json::Value::Object(object);
	}

	// Apple's background push takes `content-available` alone in `aps`,
	// with nothing that would interact with the person: a badge rides
	// beside it as data for the app to apply, and what only shapes a
	// banner doesn't ride at all.
	if message.options.is_data() {
		if let Some(badge) = message.badge {
			payload["lkbadge"] = serde_json::json!(badge);
		}

		return payload;
	}

	let mut aps = payload["aps"].take();

	// An image needs a Notification Service Extension in the app to
	// display: the mutable-content flag hands the payload to it, with the
	// URL beside.
	if let Some(image) = message.options.image.as_deref() {
		aps["mutable-content"] = serde_json::json!(1);
		payload["lkimg"] = serde_json::json!(image);
	}

	if let Some(apple) = message.options.apple.as_ref()
		&& let Some(subtitle) = apple.subtitle.as_deref()
	{
		aps["alert"]["subtitle"] = serde_json::json!(subtitle);
	}

	if let Some(apple) = message.options.apple.as_ref() {
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

	if let Some(badge) = message.badge {
		aps["badge"] = serde_json::json!(badge);
	}

	match message.options.sound.as_ref() {
		// Unset and Silent both leave the sound to the system: Apple
		// platforms are silent without a `sound` key, so an explicit
		// silence needs nothing.
		None | Some(crate::options::NotificationSound::Silent) => {}
		Some(crate::options::NotificationSound::Default) => {
			aps["sound"] = serde_json::json!("default");
		}
		Some(crate::options::NotificationSound::Named(name)) => {
			aps["sound"] = serde_json::json!(name);
		}
	}

	if let Some(link) = message.options.link.as_deref() {
		payload["lklink"] = serde_json::json!(link);
	}

	payload["aps"] = aps;
	payload
}

/// The minimal payload: the visible alert and the notification's id —
/// or, for a data message, the background shape that shows nothing and
/// the id.
fn apns_minimal_payload(message: &Message) -> serde_json::Value {
	if message.options.is_data() {
		serde_json::json!({
			"aps": {
				"content-available": 1,
			},
			"lkn": message.id.to_string(),
		})
	} else {
		serde_json::json!({
			"aps": {
				"alert": {
					"title": message.title,
					"body": message.body,
				},
			},
			"lkn": message.id.to_string(),
		})
	}
}

/// The send's `data` as an object, when it parses as one.
fn data_object(message: &Message) -> Option<serde_json::Map<String, serde_json::Value>> {
	serde_json::from_str::<serde_json::Value>(&message.data)
		.ok()
		.and_then(|data| data.as_object().cloned())
}

/// Classifies one APNs answer, as Apple's "Handling notification
/// responses from APNs" documents each status and reason. The provider's
/// reason strings are read here for classification and never surface
/// anywhere else.
///
/// Only `410` — the token is no longer active for the topic, or has
/// expired — invalidates the endpoint. `BadDeviceToken` and
/// `DeviceTokenNotForTopic` are what every token gets when the operator's
/// environment or topic is wrong, so they end the delivery as the
/// operator's configuration: one misconfigured deploy must not mark every
/// device dead. A provider token APNs calls stale or invalid is retried
/// under a freshly signed one; the other `403`s are the key's setup.
fn classify(response: &WireResponse) -> Outcome {
	let operator = Outcome::Ended {
		reason: DeliveryReason::OperatorConfiguration,
		invalidate: false,
	};
	let rejected = Outcome::Ended {
		reason: DeliveryReason::ProviderRejected,
		invalidate: false,
	};
	let oversized = Outcome::Ended {
		reason: DeliveryReason::Oversized,
		invalidate: false,
	};
	let reason = reason(response);

	match response.status {
		200..=299 => Outcome::Accepted,
		400 => {
			match reason.as_deref() {
				Some(
					"BadDeviceToken"
					| "DeviceTokenNotForTopic"
					| "BadTopic"
					| "MissingTopic"
					| "TopicDisallowed",
				) => operator,
				Some("PayloadTooLarge") => oversized,
				// The connection sat idle too long: the network's, not the
				// request's.
				Some("IdleTimeout") => Outcome::Transient { after: None },
				_ => rejected,
			}
		}
		403 => {
			match reason.as_deref() {
				Some(
					"ExpiredProviderToken"
					| "InvalidProviderToken"
					| "MissingProviderToken"
					| "UnrelatedKeyIdInToken",
				) => Outcome::AuthRejected { after: None },
				_ => operator,
			}
		}
		410 => {
			Outcome::Ended {
				reason: DeliveryReason::Unregistered,
				invalidate: true,
			}
		}
		413 => oversized,
		429 => {
			Outcome::Transient {
				after: retry_after(response),
			}
		}
		500..=599 => Outcome::Transient { after: None },
		// A bad path or method is the request's own. Redirects are refused
		// by the boundary, so a provider answering one isn't taking the
		// message. Either way the delivery ends clearly.
		_ => rejected,
	}
}

/// A provider answer's reason string, when its body is the small JSON
/// object APNs answers with.
fn reason(response: &WireResponse) -> Option<String> {
	serde_json::from_slice::<serde_json::Value>(&response.body)
		.ok()?
		.get("reason")?
		.as_str()
		.map(str::to_owned)
}

fn retry_after(response: &WireResponse) -> Option<Duration> {
	super::retry_after(response, std::time::SystemTime::now())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn answer(status: u16, reason: &str) -> WireResponse {
		WireResponse {
			status,
			headers: Vec::new(),
			body: if reason.is_empty() {
				Vec::new()
			} else {
				serde_json::json!({ "reason": reason })
					.to_string()
					.into_bytes()
			},
		}
	}

	fn ended(reason: DeliveryReason, invalidate: bool) -> Outcome {
		Outcome::Ended { reason, invalidate }
	}

	/// Every status and reason string Apple documents for a push request,
	/// and what each comes to. Only `410` — the token is no longer active
	/// for the topic — invalidates the endpoint. `BadDeviceToken` and
	/// `DeviceTokenNotForTopic` are what every token gets when the
	/// operator's environment or topic is wrong, so they end as the
	/// operator's configuration and leave the device enrolled; a refused
	/// provider token is retried under a freshly signed one.
	#[test]
	fn apns_answers_classify_as_apple_documents_them() {
		let configuration = ended(DeliveryReason::OperatorConfiguration, false);
		let rejected = ended(DeliveryReason::ProviderRejected, false);
		let retry_token = Outcome::AuthRejected { after: None };
		let transient = Outcome::Transient { after: None };

		for (status, reason, expected) in [
			(200, "", Outcome::Accepted),
			(400, "BadDeviceToken", configuration),
			(400, "DeviceTokenNotForTopic", configuration),
			(400, "BadTopic", configuration),
			(400, "MissingTopic", configuration),
			(400, "TopicDisallowed", configuration),
			(400, "BadCollapseId", rejected),
			(400, "BadExpirationDate", rejected),
			(400, "BadMessageId", rejected),
			(400, "BadPriority", rejected),
			(400, "DuplicateHeaders", rejected),
			(400, "InvalidPushType", rejected),
			(400, "MissingDeviceToken", rejected),
			(400, "PayloadEmpty", rejected),
			(400, "IdleTimeout", transient),
			(400, "", rejected),
			(403, "ExpiredProviderToken", retry_token),
			(403, "InvalidProviderToken", retry_token),
			(403, "MissingProviderToken", retry_token),
			(403, "UnrelatedKeyIdInToken", retry_token),
			(403, "BadEnvironmentKeyIdInToken", configuration),
			(403, "Forbidden", configuration),
			(403, "BadCertificate", configuration),
			(403, "BadCertificateEnvironment", configuration),
			(403, "", configuration),
			(404, "BadPath", rejected),
			(405, "MethodNotAllowed", rejected),
			(
				410,
				"Unregistered",
				ended(DeliveryReason::Unregistered, true),
			),
			(
				410,
				"ExpiredToken",
				ended(DeliveryReason::Unregistered, true),
			),
			(
				413,
				"PayloadTooLarge",
				ended(DeliveryReason::Oversized, false),
			),
			(429, "TooManyRequests", transient),
			(429, "TooManyProviderTokenUpdates", transient),
			(500, "InternalServerError", transient),
			(503, "ServiceUnavailable", transient),
			(503, "Shutdown", transient),
			(302, "", rejected),
		] {
			assert_eq!(
				classify(&answer(status, reason)),
				expected,
				"{status} {reason}"
			);
		}
	}
}
