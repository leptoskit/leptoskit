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

/// The whole encoded message's limit, in bytes.
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
			headers: vec![
				("authorization".to_owned(), format!("Bearer {access_token}")),
				("content-type".to_owned(), "application/json".to_owned()),
			],
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
		credential: &'a str,
		message: &'a Message,
	) -> Pin<Box<dyn Future<Output = Outcome> + Send + 'a>> {
		let send = async move {
			// Size is checked on the encoded message before any network
			// work: the full one first, then the minimal id-only envelope,
			// and past both a terminal clear result with nothing sent.
			let full =
				serde_json::to_vec(&fcm_message(credential, message, true)).unwrap_or_default();
			let body = if full.len() > PAYLOAD_LIMIT {
				let minimal = serde_json::to_vec(&fcm_message(credential, message, false))
					.unwrap_or_default();

				if minimal.len() > PAYLOAD_LIMIT {
					return Outcome::Ended {
						reason: DeliveryReason::Oversized,
						invalidate: false,
					};
				}

				minimal
			} else {
				full
			};

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

			if status == 401 {
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
		};

		Box::pin(send)
	}
}

/// The HTTP v1 message: the standard visible notification, the send's
/// `data` as the string-valued map the API asks for, the notification's
/// id as the collapse key and in `data`, the same collapse id for an iOS
/// app the project also talks to, and the notification's remaining life
/// as the TTL both platforms hold it for, so a provider can't deliver
/// past the send's expiry.
///
/// `with_data` is false for the minimal id-only envelope, when the full
/// one doesn't fit the provider's limit. The collapse key and headers
/// stay either way: a fallback that dropped them would let a repeat of
/// the same notification stack banners the full payload would have
/// replaced. Collapse is best-effort deduplication, though — a provider
/// that never coalesces, or a device that was offline, can still show
/// two.
fn fcm_message(credential: &str, message: &Message, with_data: bool) -> serde_json::Value {
	let ttl = message.expires_at.map(|expires_at| {
		(expires_at - leptoskit_db::now())
			.num_seconds()
			.clamp(0, MAX_TTL_SECS)
	});

	let mut map = serde_json::Map::new();

	if with_data {
		let data: serde_json::Value = serde_json::from_str(&message.data).unwrap_or_default();

		if let Some(object) = data.as_object() {
			for (name, value) in object {
				// The envelope's own id key and FCM's reserved words are
				// dropped from the caller's data rather than overwritten
				// or sent: `lkn` is ours, and `from`, `google.*`, and
				// `gcm.*` are words the platform reserves.
				if name == "lkn"
					|| name == "from"
					|| name.starts_with("google.")
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
	}

	map.insert(
		"lkn".to_owned(),
		serde_json::Value::String(message.id.to_string()),
	);

	let mut android = serde_json::json!({
		"collapse_key": message.id.to_string(),
	});
	let mut apns = serde_json::json!({
		"headers": {
			"apns-collapse-id": message.id.to_string(),
		},
		"payload": {
			"aps": {
				"alert": {
					"title": message.title,
					"body": message.body,
				},
			},
			"lkn": message.id.to_string(),
		},
	});

	if let Some(ttl) = ttl {
		if let Some(object) = android.as_object_mut() {
			object.insert(
				"ttl".to_owned(),
				serde_json::Value::String(format!("{ttl}s")),
			);
		}

		if let Some(headers) = apns
			.get_mut("headers")
			.and_then(|headers| headers.as_object_mut())
		{
			headers.insert(
				"apns-expiration".to_owned(),
				serde_json::Value::String(
					message
						.expires_at
						.map_or(0, |expires_at| expires_at.timestamp().max(0))
						.to_string(),
				),
			);
		}
	}

	serde_json::json!({
		"message": {
			"token": credential,
			"notification": {
				"title": message.title,
				"body": message.body,
			},
			"data": map,
			"android": android,
			// An iOS app served through the same project sees the same
			// alert, collapse id, and expiry as a direct APNs send would
			// give it.
			"apns": apns,
		},
	})
}

/// Classifies one FCM answer. The provider's status strings are read here
/// for classification and never surface anywhere else.
fn classify(response: &WireResponse) -> Outcome {
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
	let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(response.body.trim_ascii()) else {
		return false;
	};

	parsed["error"]["details"]
		.as_array()
		.is_some_and(|details| {
			details.iter().any(|detail| {
				detail["@type"].as_str()
					== Some("type.googleapis.com/google.firebase.fcm.v1.FcmError")
					&& detail["errorCode"].as_str() == Some("UNREGISTERED")
			})
		})
}

fn retry_after(response: &WireResponse) -> Option<Duration> {
	super::retry_after(response, std::time::SystemTime::now())
}

#[cfg(test)]
mod tests {
	use super::*;

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
