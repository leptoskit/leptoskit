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

	/// Posts one payload and classifies the answer.
	async fn post(
		&self,
		credential: &str,
		token: &str,
		payload: Vec<u8>,
		message: &Message,
	) -> Outcome {
		let mut headers = vec![
			("authorization".to_owned(), format!("Bearer {token}")),
			("apns-topic".to_owned(), self.topic.clone()),
			("apns-push-type".to_owned(), "alert".to_owned()),
			("apns-priority".to_owned(), "10".to_owned()),
			("apns-id".to_owned(), message.id.to_string()),
			// The stable collapse id: a repeat replaces, not stacks.
			("apns-collapse-id".to_owned(), message.id.to_string()),
		];

		if let Some(expires_at) = message.expires_at {
			headers.push((
				"apns-expiration".to_owned(),
				expires_at.timestamp().max(0).to_string(),
			));
		}

		let request = WireRequest {
			url: format!("{}/3/device/{credential}", self.host),
			headers,
			body: payload,
			pinned: None,
		};
		let answer = self.boundary.post(request).await;

		match answer {
			Ok(response) => classify(&response),
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

			// Size is checked on the encoded payload: the full one first,
			// then the minimal id-only envelope, and past both a terminal
			// clear result.
			let full = serde_json::to_vec(&apns_payload(message)).unwrap_or_else(|_| Vec::new());

			if full.len() > PAYLOAD_LIMIT {
				let minimal =
					serde_json::to_vec(&apns_minimal_payload(message)).unwrap_or_default();

				if minimal.len() > PAYLOAD_LIMIT {
					return Outcome::Ended {
						reason: DeliveryReason::Oversized,
						invalidate: false,
					};
				}

				return self.post(credential, &token, minimal, message).await;
			}

			self.post(credential, &token, full, message).await
		};

		Box::pin(send)
	}
}

/// The full payload: the visible alert, the notification's id for
/// routing and deduplication, and the send's `data`.
fn apns_payload(message: &Message) -> serde_json::Value {
	let mut payload = apns_minimal_payload(message);
	let data: serde_json::Value = serde_json::from_str(&message.data).unwrap_or_default();

	if let Some(object) = data.as_object()
		&& !object.is_empty()
	{
		payload["lk"] = data;
	}

	payload
}

/// The minimal payload: the visible alert and the notification's id.
fn apns_minimal_payload(message: &Message) -> serde_json::Value {
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

/// Classifies one APNs answer. The provider's reason strings are read
/// here for classification and never surface anywhere else.
fn classify(response: &WireResponse) -> Outcome {
	match response.status {
		200..=299 => Outcome::Accepted,
		400 | 413 => {
			if response.status == 413 || reason(response).as_deref() == Some("PayloadTooLarge") {
				return Outcome::Ended {
					reason: DeliveryReason::Oversized,
					invalidate: false,
				};
			}

			// A token that means nothing to this topic or environment is
			// dead as far as this server is concerned.
			if matches!(
				reason(response).as_deref(),
				Some("BadDeviceToken" | "DeviceTokenNotForTopic")
			) {
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
		410 => {
			Outcome::Ended {
				reason: DeliveryReason::Unregistered,
				invalidate: true,
			}
		}
		429 => {
			Outcome::Transient {
				after: retry_after(response),
			}
		}
		500..=599 => Outcome::Transient { after: None },
		// Invalid provider credentials or a topic mismatch are the
		// operator's configuration: the endpoint is fine, so nothing is
		// invalidated. Redirects are refused by the boundary, so a
		// provider answering one isn't taking the message. Either way the
		// delivery ends clearly.
		_ => {
			Outcome::Ended {
				reason: DeliveryReason::ProviderRejected,
				invalidate: false,
			}
		}
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
