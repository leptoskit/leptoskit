//! The Web Push transport: RFC 8030 posts of RFC 8291-encrypted
//! envelopes, signed RFC 8292 VAPID, to the browser's push service.
//!
//! The protocol itself is the maintained [`web-push`] crate's — Mozilla's
//! `ece` underneath: it encrypts the payload `aes128gcm` with a fresh
//! ephemeral key and salt per message, builds the VAPID ES256 JWT with
//! the endpoint's origin as its audience, and shapes the request's TTL,
//! urgency, topic, content encoding, and signed authorization headers.
//! This transport chooses the envelope and hands the built request, still
//! as bytes, to the injected [`Boundary`], which owns every socket rule:
//! no redirects, no proxies, DNS-validated pinning for operator-approved
//! extra hosts.
//!
//! The envelope is the documented one — title, body, the notification's
//! id, and the send's `data` — and a client's service worker shows it or
//! not as it chooses; nothing executable rides in it. The operator holds
//! the key pair the ecosystem's own tools generate: a 32-byte private
//! scalar and a 65-byte public point, base64url, cross-checked against
//! each other here so a mistyped half fails at startup, not at every
//! push service at once.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use web_push::ContentEncoding;
use web_push::SubscriptionInfo;
use web_push::Urgency;
use web_push::VapidSignatureBuilder;
use web_push::WebPushMessageBuilder;
use web_push::request_builder::build_request as build_http_request;

use crate::push::DeliveryReason;
use crate::push::WebPushKeys;
use crate::push::base64_url;
use crate::service::config::WebPushConfig;
use crate::service::transport::Boundary;
use crate::service::transport::Message;
use crate::service::transport::Outcome;
use crate::service::transport::Pinned;
use crate::service::transport::Transport;
use crate::service::transport::WireRequest;
use crate::service::transport::WireResponse;
use crate::service::transport::host;

/// The largest body a push service must take: the encrypted record's
/// record size, header included.
const BODY_LIMIT: usize = 4_096;

/// The longest time a push may sit at the service, in seconds: four
/// weeks, the largest TTL the example services take.
const DEFAULT_TTL_SECS: i64 = 28 * 24 * 3_600;

/// The Web Push sender.
pub struct WebPush {
	/// The VAPID private scalar, unpadded base64url: the exact form the
	/// signing library takes.
	private_scalar: String,
	subject: String,
	policy: host::HostPolicy,
	boundary: std::sync::Arc<dyn Boundary>,
}

impl WebPush {
	/// Builds the transport from the operator's configuration, checking
	/// the VAPID pair now: the private scalar must be 32 bytes of
	/// base64url, the public point 65, and the point must be the scalar's
	/// own. An operator-approved extra host that isn't a plain host name
	/// is refused here too, named, rather than silently dropped.
	///
	/// # Errors
	///
	/// A message naming what's wrong with the keys, the subject, or an
	/// extra host.
	pub fn new(
		config: &WebPushConfig,
		boundary: std::sync::Arc<dyn Boundary>,
	) -> Result<Self, String> {
		let private = base64_url(&config.vapid_private_key)
			.ok_or("Web Push: the VAPID private key isn't base64url")?;
		let public = base64_url(&config.vapid_public_key)
			.ok_or("Web Push: the VAPID public key isn't base64url")?;

		if private.len() != 32 {
			return Err("Web Push: the VAPID private key isn't 32 bytes".into());
		}

		if public.len() != 65 || public[0] != 0x04 {
			return Err("Web Push: the VAPID public key isn't a 65-byte uncompressed point".into());
		}

		// The pair is cross-checked through the same key the sends sign
		// with: the scalar's own public point must be the configured one,
		// so a mistyped half fails here, once, instead of at every push
		// service at once.
		let derived = VapidSignatureBuilder::from_base64_no_sub(&URL_SAFE_NO_PAD.encode(&private))
			.map_err(|error| format!("Web Push: the VAPID pair won't build: {error}"))?;

		if derived.get_public_key() != public {
			return Err("Web Push: the VAPID private key doesn't match its public key".into());
		}

		let subject = config.subject.trim();

		if !(subject.starts_with("mailto:") || subject.starts_with("https://"))
			|| subject.len() <= 8
			|| subject.len() > 254
		{
			return Err("Web Push: the VAPID subject is a mailto: or https: contact".into());
		}

		Ok(Self {
			private_scalar: URL_SAFE_NO_PAD.encode(&private),
			subject: subject.to_owned(),
			policy: host::HostPolicy::with_extra_hosts(&config.extra_hosts)?,
			boundary,
		})
	}

	/// The host policy enrollments and attempts are checked against.
	pub(crate) fn policy(&self) -> &host::HostPolicy {
		&self.policy
	}

	/// Checks a stored subscription against the host policy, resolving
	/// and pinning an extra host's addresses for the connection. Shipped
	/// providers answer unpinned. A resolver failure or timeout is
	/// transient — the delivery retries — while a policy refusal is
	/// permanent.
	async fn route(&self, endpoint: &str) -> Result<Option<Pinned>, RouteFailure> {
		let url = url::Url::parse(endpoint).map_err(|_| RouteFailure::Permanent)?;
		let pinning = self
			.policy
			.check(&url)
			.map_err(|_| RouteFailure::Permanent)?;

		match pinning {
			Some(pinned) => {
				let resolved = host::resolve_pinned(&pinned.host, pinned.port)
					.await
					.map_err(|_| RouteFailure::Transient)?;
				Ok(Some(resolved))
			}
			None => Ok(None),
		}
	}

	/// Builds one fully signed and encrypted request for `keys` and
	/// `message`, as the wire shape the boundary carries. `with_data` is
	/// false for the minimal id-only envelope. The library builds the
	/// request's whole shape — URL, TTL, urgency, topic, the encrypted
	/// body, and the signed authorization headers — and this is the
	/// mechanical half of turning it into the boundary's type.
	fn build_wire(
		&self,
		endpoint: &str,
		keys: &WebPushKeys,
		message: &Message,
		with_data: bool,
		ttl: i64,
	) -> Result<WireRequest, web_push::WebPushError> {
		let subscription = SubscriptionInfo::new(endpoint, &keys.p256dh, &keys.auth);
		let mut builder = WebPushMessageBuilder::new(&subscription);
		let content = envelope(message, with_data);

		builder.set_payload(ContentEncoding::Aes128Gcm, &content);
		builder.set_ttl(ttl.clamp(0, DEFAULT_TTL_SECS) as u32);
		builder.set_urgency(Urgency::Normal);
		// The stable tag: a repeat replaces, not stacks, wherever the
		// service coalesces — best-effort, like every provider's. A topic
		// is 32 base64url characters, exactly a UUID without its dashes.
		builder.set_topic(message.id.simple().to_string());

		let mut vapid = VapidSignatureBuilder::from_base64(&self.private_scalar, &subscription)?;
		vapid.add_claim("sub", self.subject.as_str());
		builder.set_vapid_signature(vapid.build()?);

		let built = build_http_request::<Vec<u8>>(builder.build()?);
		let mut headers = Vec::new();

		for (name, value) in built.headers() {
			headers.push((
				name.as_str().to_ascii_lowercase(),
				value.to_str().unwrap_or_default().to_owned(),
			));
		}

		Ok(WireRequest {
			url: built.uri().to_string(),
			headers,
			body: built.into_body(),
			pinned: None,
		})
	}

	/// Posts one built request, pinned to `pinned`'s validated addresses,
	/// and classifies the answer.
	async fn post(&self, mut request: WireRequest, pinned: Option<Pinned>) -> Outcome {
		request.pinned = pinned;
		let answer = self.boundary.post(request).await;

		match answer {
			Ok(response) => classify(&response),
			Err(_) => Outcome::Transient { after: None },
		}
	}
}

/// Why a stored endpoint couldn't be routed: permanently — its host is
/// outside the policy — or transiently, because the resolver failed or
/// timed out and the delivery should retry. The reason is a sanitized
/// code on the delivery, never the endpoint's own text.
enum RouteFailure {
	Permanent,
	Transient,
}

impl Transport for WebPush {
	fn attempt<'a>(
		&'a self,
		credential: &'a str,
		message: &'a Message,
	) -> Pin<Box<dyn Future<Output = Outcome> + Send + 'a>> {
		let send = async move {
			// Defense in depth: the policy held at enrollment, and it
			// holds again at the moment of the attempt, against a
			// subscription stored under an earlier policy.
			let Ok(subscription) = crate::push::stored_web_subscription(credential) else {
				return Outcome::Ended {
					reason: DeliveryReason::EndpointUnavailable,
					invalidate: false,
				};
			};

			let pinned = match self.route(&subscription.endpoint).await {
				Ok(routed) => routed,
				Err(RouteFailure::Permanent) => {
					return Outcome::Ended {
						reason: DeliveryReason::EndpointUnavailable,
						invalidate: false,
					};
				}
				Err(RouteFailure::Transient) => {
					// DNS is a network dependency like any other: it
					// comes back, and the delivery retries.
					return Outcome::Transient { after: None };
				}
			};

			let ttl = message.expires_at.map_or(DEFAULT_TTL_SECS, |expires_at| {
				(expires_at - leptoskit_db::now())
					.num_seconds()
					.clamp(0, DEFAULT_TTL_SECS)
			});

			// Size is checked on the encrypted body: the full envelope
			// first, then the minimal id-only one, and past both a
			// terminal clear result. The library refuses a payload past
			// its record itself, and a bound on the encoded bytes guards
			// whatever it let through.
			match self.build_wire(
				&subscription.endpoint,
				&subscription.keys,
				message,
				true,
				ttl,
			) {
				Ok(request) if request.body.len() <= BODY_LIMIT => self.post(request, pinned).await,
				_ => {
					match self.build_wire(
						&subscription.endpoint,
						&subscription.keys,
						message,
						false,
						ttl,
					) {
						Ok(request) if request.body.len() <= BODY_LIMIT => {
							self.post(request, pinned).await
						}
						_ => {
							Outcome::Ended {
								reason: DeliveryReason::Oversized,
								invalidate: false,
							}
						}
					}
				}
			}
		};

		Box::pin(send)
	}
}

/// The push envelope a client's service worker reads: the visible text,
/// the notification's id, and — when it fits — the send's `data`. No
/// executable anything rides in it.
fn envelope(message: &Message, with_data: bool) -> Vec<u8> {
	let mut payload = serde_json::json!({
		"title": message.title,
		"body": message.body,
		"lkn": message.id.to_string(),
	});

	if with_data {
		let data: serde_json::Value = serde_json::from_str(&message.data).unwrap_or_default();

		if let Some(object) = data.as_object()
			&& !object.is_empty()
		{
			payload["lk"] = data;
		}
	}

	serde_json::to_vec(&payload).unwrap_or_default()
}

/// Classifies one push-service answer.
fn classify(response: &WireResponse) -> Outcome {
	match response.status {
		200..=299 => Outcome::Accepted,
		404 | 410 => {
			Outcome::Ended {
				reason: DeliveryReason::Unregistered,
				invalidate: true,
			}
		}
		413 => {
			Outcome::Ended {
				reason: DeliveryReason::Oversized,
				invalidate: false,
			}
		}
		429 => {
			Outcome::Transient {
				after: retry_after(response),
			}
		}
		500..=599 => Outcome::Transient { after: None },
		_ => {
			Outcome::Ended {
				reason: DeliveryReason::ProviderRejected,
				invalidate: false,
			}
		}
	}
}

/// A `Retry-After` header, seconds or HTTP-date, as a delay — never
/// longer than the engine's own ceiling.
fn retry_after(response: &WireResponse) -> Option<Duration> {
	let value = response
		.headers
		.iter()
		.find(|(name, _)| name == "retry-after")?
		.1
		.trim();

	let seconds = value.parse::<i64>().ok().or_else(|| {
		chrono::DateTime::parse_from_rfc2822(value)
			.ok()
			.map(|date| date.timestamp() - leptoskit_db::now().timestamp())
	})?;

	if seconds <= 0 {
		return Some(Duration::from_secs(1));
	}

	Some(Duration::from_secs(
		seconds.min(crate::push::MAX_RETRY_DELAY_SECS) as u64,
	))
}
