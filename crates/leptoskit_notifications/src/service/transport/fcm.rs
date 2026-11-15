//! The FCM transport: HTTP v1 `messages:send` against the project the
//! operator's service account names, authorized by an OAuth 2.0 access
//! token exchanged for that account's ES256-signed JWT.
//!
//! The access token is short-lived by design, so it's exchanged on
//! demand and cached only until two minutes before its `expires_in`
//! runs out. There is no static server key anywhere in this path: the
//! legacy key API doesn't exist in HTTP v1, and a long-lived credential
//! would defeat the point of the account model. The project comes from
//! the service account file, and the send URL from that project — an
//! enrollment carries only the registration token, so a caller can't
//! point a push at a project of their choosing.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::SystemRandom;

use crate::push::DeliveryReason;
use crate::service::config::FcmConfig;
use crate::service::transport::Boundary;
use crate::service::transport::Message;
use crate::service::transport::Outcome;
use crate::service::transport::Transport;
use crate::service::transport::WireRequest;
use crate::service::transport::WireResponse;
use crate::service::transport::apns::signing_key;

/// The token endpoint the service account's JWT is exchanged at.
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// The scope the exchange asks for: sending messages, and nothing else.
const SEND_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

/// The send endpoint's fixed host and path shape.
const SEND_HOST: &str = "https://fcm.googleapis.com/v1/projects";

/// The whole encoded message's limit, in bytes.
const PAYLOAD_LIMIT: usize = 4_096;

/// How early of an access token's lapse a cached one is retired.
const TOKEN_REFRESH_MARGIN_SECS: i64 = 120;

/// The service account, as the operator's JSON file holds it.
pub(crate) struct Account {
	/// The account's email: the JWT's `iss`.
	pub email: String,
	/// The project pushes go out under: the send URL names it.
	pub project: String,
	/// The account's `.p8`-style private key, a PEM string in the file.
	pub key_p8: String,
}

/// Reads the service account out of its JSON file.
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
		key_p8: private_key,
	})
}

/// The FCM sender.
pub struct Fcm {
	account: Account,
	key: ring::signature::EcdsaKeyPair,
	boundary: std::sync::Arc<dyn Boundary>,
	/// The cached access token, and the epoch second it stops being
	/// trusted.
	token: Mutex<Option<(String, i64)>>,
}

impl Fcm {
	/// Builds the transport from the operator's configuration, parsing
	/// the account and its key now.
	///
	/// # Errors
	///
	/// A message naming what's wrong with the account or its key.
	pub fn new(config: &FcmConfig, boundary: std::sync::Arc<dyn Boundary>) -> Result<Self, String> {
		let account = service_account(&config.service_account_json)?;
		let key = signing_key(&account.key_p8).map_err(|error| format!("FCM: {error}"))?;

		Ok(Self {
			account,
			key,
			boundary,
			token: Mutex::new(None),
		})
	}

	/// The access token: cached until near its lapse, exchanged fresh
	/// when it's gone. The exchange is a POST the boundary carries; the
	/// JWT it asserts is signed here, ES256, claims `iss` (the account),
	/// `scope`, `aud` (the token endpoint), `iat`, and `exp`.
	async fn access_token(&self) -> Result<String, String> {
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

		let header = r#"{"alg":"ES256","typ":"JWT"}"#;
		let claims = format!(
			"{{\"iss\":{},\"scope\":\"{SEND_SCOPE}\",\"aud\":\"{TOKEN_ENDPOINT}\",\"iat\":{now},\"\
			 exp\":{}}}",
			serde_json::to_string(&self.account.email).expect("a plain string"),
			now + 3_600
		);
		let assertion = format!(
			"{}.{}",
			URL_SAFE_NO_PAD.encode(header),
			URL_SAFE_NO_PAD.encode(claims)
		);
		let signature = self
			.key
			.sign(&SystemRandom::new(), assertion.as_bytes())
			.map_err(|_| "FCM: the service account key couldn't sign".to_owned())?;
		let assertion = format!("{assertion}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()));

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
			.map_err(|_| "FCM: the token exchange didn't answer".to_owned())?;

		if response.status != 200 {
			return Err("FCM: the token exchange was refused".to_owned());
		}

		let answer: serde_json::Value = serde_json::from_slice(&response.body)
			.map_err(|_| "FCM: the token exchange didn't answer JSON".to_owned())?;
		let access_token = answer
			.get("access_token")
			.and_then(serde_json::Value::as_str)
			.ok_or("FCM: the token exchange answered no access_token")?;
		let expires_in = answer
			.get("expires_in")
			.and_then(serde_json::Value::as_i64)
			.unwrap_or(3_600)
			.max(0);

		*self
			.token
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner) = Some((
			access_token.to_owned(),
			now + expires_in.saturating_sub(TOKEN_REFRESH_MARGIN_SECS),
		));

		Ok(access_token.to_owned())
	}

	/// Posts one encoded message and classifies the answer.
	async fn post(&self, access_token: &str, body: Vec<u8>) -> Outcome {
		let request = WireRequest {
			url: format!("{SEND_HOST}/{}/messages:send", self.account.project),
			headers: vec![
				("authorization".to_owned(), format!("Bearer {access_token}")),
				("content-type".to_owned(), "application/json".to_owned()),
			],
			body,
			pinned: None,
		};
		let answer = self.boundary.post(request).await;

		match answer {
			Ok(response) => classify(&response),
			Err(_) => Outcome::Transient { after: None },
		}
	}
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

			// An exchange failure is the operator's account
			// configuration, not the endpoint's: retry, and the send's
			// status shows it stuck.
			let Ok(access_token) = self.access_token().await else {
				return Outcome::Transient { after: None };
			};

			self.post(&access_token, body).await
		};

		Box::pin(send)
	}
}

/// The HTTP v1 message: the standard visible notification, the send's
/// `data` as the string-valued map the API asks for, the notification's
/// id as the collapse key and in `data`, and the same collapse id for an
/// iOS app the project also talks to.
///
/// `with_data` is false for the minimal id-only envelope, when the full
/// one doesn't fit the provider's limit.
fn fcm_message(credential: &str, message: &Message, with_data: bool) -> serde_json::Value {
	let data: serde_json::Value = serde_json::from_str(&message.data).unwrap_or_default();
	let mut map = serde_json::Map::new();

	if let Some(object) = data.as_object() {
		for (name, value) in object {
			// The data map is string-valued: anything nested goes as its
			// JSON text.
			map.insert(
				name.clone(),
				serde_json::Value::String(match value {
					serde_json::Value::String(text) => text.clone(),
					other => other.to_string(),
				}),
			);
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
		"headers": { "apns-collapse-id": message.id.to_string() },
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

	if !with_data {
		// The minimal envelope keeps the visible notification and the id,
		// and drops the send's own data everywhere.
		if let Some(object) = android.as_object_mut() {
			object.remove("collapse_key");
		}

		if let Some(object) = apns.as_object_mut() {
			object.remove("headers");
		}

		if let Some(object) = apns
			.get_mut("payload")
			.and_then(|payload| payload.as_object_mut())
		{
			object.remove("lkn");
		}

		map = serde_json::Map::new();
		map.insert(
			"lkn".to_owned(),
			serde_json::Value::String(message.id.to_string()),
		);
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
			// alert and collapse id as a direct APNs send would give it.
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
			Outcome::Ended {
				reason: DeliveryReason::Unregistered,
				invalidate: true,
			}
		}
		400 => {
			// A registration token the project doesn't know is dead
			// here; a payload-shaped 400 is the send's own, and the
			// endpoint stays.
			if status(response).as_deref() == Some("UNREGISTERED") {
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
		429 => {
			Outcome::Transient {
				after: retry_after(response),
			}
		}
		500..=599 => Outcome::Transient { after: None },
		// An authorization failure is the operator's account setup, and
		// anything unrecognized — redirects included, which the boundary
		// refuses — ends the delivery the same clear way.
		_ => {
			Outcome::Ended {
				reason: DeliveryReason::ProviderRejected,
				invalidate: false,
			}
		}
	}
}

/// A provider answer's status string, when its body is the error object
/// FCM answers with.
fn status(response: &WireResponse) -> Option<String> {
	let parsed = serde_json::from_slice::<serde_json::Value>(response.body.trim_ascii()).ok()?;

	parsed
		.get("error")?
		.get("status")?
		.as_str()
		.map(str::to_owned)
}

/// A `Retry-After` header as a delay, capped at the engine's ceiling.
fn retry_after(response: &WireResponse) -> Option<Duration> {
	let value = response
		.headers
		.iter()
		.find(|(name, _)| name == "retry-after")?
		.1
		.trim();

	let seconds = value.parse::<i64>().ok()?;

	if seconds <= 0 {
		return Some(Duration::from_secs(1));
	}

	Some(Duration::from_secs(
		seconds.min(crate::push::MAX_RETRY_DELAY_SECS) as u64,
	))
}
