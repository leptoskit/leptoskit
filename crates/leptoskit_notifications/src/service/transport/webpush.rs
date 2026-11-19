//! The Web Push transport: RFC 8030 posts of RFC 8291-encrypted
//! envelopes, signed RFC 8292 VAPID, to the browser's push service.
//!
//! The protocol is implemented here, directly over ring's
//! constant-time primitives — the same ones the APNs ES256 provider
//! token and the FCM RS256 assertion already use, replacing the
//! `web-push` crate whose `jwt-simple` and `ece` chain carried the
//! `rsa` and `crypto-bigint` advisories. Each message draws a fresh
//! ephemeral P-256 key and salt (RFC 8291's two-stage HKDF from the
//! ECDH secret and the subscription's auth secret, one padded record
//! under an RFC 8188 header, sealed AES-128-GCM) and a VAPID ES256
//! JWT whose audience is the endpoint's origin (RFC 8292). This
//! transport builds the whole request and hands it, still as bytes,
//! to the injected [`Boundary`], which owns every socket rule: no
//! redirects, no proxies, DNS-validated pinning for operator-approved
//! extra hosts.
//!
//! The envelope is the documented one — title, body, the notification's
//! id, and the send's `data` — and a client's service worker shows it;
//! nothing executable rides in it. A silent data message is never sent:
//! browsers require every push to show a notification. The operator holds
//! the key pair the ecosystem's own tools generate: a 32-byte private
//! scalar and a 65-byte public point, base64url, cross-checked against
//! each other here so a mistyped half fails at startup, not at every
//! push service at once.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::aead::AES_128_GCM;
use ring::aead::Aad;
use ring::aead::LessSafeKey;
use ring::aead::Nonce;
use ring::aead::UnboundKey;
use ring::agreement::ECDH_P256;
use ring::agreement::EphemeralPrivateKey;
use ring::agreement::UnparsedPublicKey;
use ring::agreement::agree_ephemeral;
use ring::hkdf::HKDF_SHA256;
use ring::hkdf::KeyType;
use ring::hkdf::Salt;
use ring::rand::SecureRandom as _;
use ring::rand::SystemRandom;
use ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING;
use ring::signature::EcdsaKeyPair;

use crate::push::DeliveryReason;
use crate::push::WebPushKeys;
use crate::push::base64_url;
use crate::service::config::WebPushConfig;
use crate::service::transport::Boundary;
use crate::service::transport::Message;
use crate::service::transport::Outcome;
use crate::service::transport::Pinned;
use crate::service::transport::Shaped;
use crate::service::transport::Target;
use crate::service::transport::Transport;
use crate::service::transport::WireRequest;
use crate::service::transport::WireResponse;
use crate::service::transport::host;

/// The largest body a push service must take: the encrypted record's
/// record size, header included.
const BODY_LIMIT: usize = 4_096;

/// The record size every header declares: one record for the whole
/// payload, with the delimiter and auth tag's room reserved.
const RECORD_SIZE: u32 = 4_096;

/// The longest time a push may sit at the service, in seconds: four
/// weeks, the largest TTL the example services take.
const DEFAULT_TTL_SECS: i64 = 28 * 24 * 3_600;

/// How long a VAPID token claims to live: half a day, inside the day
/// RFC 8292 caps a token's `exp` at.
const VAPID_LIFETIME_SECS: i64 = 12 * 3_600;

/// The Web Push sender.
pub struct WebPush {
	/// The VAPID signing pair, parsed once. Secret material: never
	/// printed, and the pair is checked against itself at build.
	key: EcdsaKeyPair,
	/// The pair's public point, unpadded base64url: the exact form the
	/// Authorization header's `k=` half carries.
	public: String,
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
		// with: ring refuses a pair whose halves don't belong together,
		// so a mistyped half fails here, once, instead of at every push
		// service at once.
		let key = EcdsaKeyPair::from_private_key_and_public_key(
			&ECDSA_P256_SHA256_FIXED_SIGNING,
			&private,
			&public,
			&SystemRandom::new(),
		)
		.map_err(|_| "Web Push: the VAPID private key doesn't match its public key".to_owned())?;

		let subject = config.subject.trim();

		if !(subject.starts_with("mailto:") || subject.starts_with("https://"))
			|| subject.len() <= 8
			|| subject.len() > 254
		{
			return Err("Web Push: the VAPID subject is a mailto: or https: contact".into());
		}

		Ok(Self {
			key,
			public: URL_SAFE_NO_PAD.encode(&public),
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

	/// Builds one fully encrypted and signed request for `keys` and
	/// `message`, as the wire shape the boundary carries. `include` is
	/// the degradation stage of the payload's optional extras, from all
	/// of them down to the id-only envelope. Everything on the wire is
	/// built here — the endpoint's URL, the TTL, urgency, and topic
	/// headers, the RFC 8291-encrypted body, and the RFC 8292-signed
	/// authorization — over key material drawn fresh for this one
	/// request.
	///
	/// # Errors
	///
	/// [`BuildFailure`], which names no key material.
	fn build_wire(
		&self,
		endpoint: &str,
		keys: &WebPushKeys,
		shaped: &Shaped,
	) -> Result<WireRequest, BuildFailure> {
		let ua_public = base64_url(&keys.p256dh).ok_or(BuildFailure::Keys)?;
		let auth = base64_url(&keys.auth).ok_or(BuildFailure::Keys)?;

		if ua_public.len() != 65 || ua_public[0] != 0x04 || auth.is_empty() {
			return Err(BuildFailure::Keys);
		}

		let audience = url::Url::parse(endpoint)
			.ok()
			.map(|url| url.origin().ascii_serialization())
			.ok_or(BuildFailure::Keys)?;
		let authorization = self.vapid_header(&audience)?;
		let body = encrypt_payload(&ua_public, &auth, &shaped.body)?;
		let mut headers = shaped.headers.clone();
		headers.push(("authorization".to_owned(), authorization));

		Ok(WireRequest {
			url: endpoint.to_owned(),
			headers,
			body,
			pinned: None,
		})
	}

	/// RFC 8292: the `Authorization` header, `vapid t=<ES256 JWT>,
	/// k=<the application server's public point>` — the JWT's audience
	/// the endpoint's origin, its subject the operator's contact, and
	/// its expiry ahead but never past the RFC's day.
	fn vapid_header(&self, audience: &str) -> Result<String, BuildFailure> {
		let claims = format!(
			"{{\"aud\":{},\"exp\":{},\"sub\":{}}}",
			serde_json::to_string(audience).expect("a plain string"),
			leptoskit_db::now().timestamp() + VAPID_LIFETIME_SECS,
			serde_json::to_string(&self.subject).expect("a plain string"),
		);
		let signed = format!(
			"{}.{}",
			URL_SAFE_NO_PAD.encode(r#"{"typ":"JWT","alg":"ES256"}"#),
			URL_SAFE_NO_PAD.encode(claims),
		);
		let signature = self
			.key
			.sign(&SystemRandom::new(), signed.as_bytes())
			.map_err(|_| BuildFailure::Random)?;

		Ok(format!(
			"vapid t={signed}.{}, k={}",
			URL_SAFE_NO_PAD.encode(signature.as_ref()),
			self.public
		))
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

/// Why a request couldn't be built: the stored subscription's keys
/// won't parse or aren't a point on the curve, which ends the delivery as
/// an unusable subscription, or the system RNG refused the ephemeral key
/// or the VAPID signature, which retries. The reason names no key
/// material.
enum BuildFailure {
	Keys,
	Random,
}

impl Transport for WebPush {
	fn attempt<'a>(
		&'a self,
		target: Target<'a>,
		message: &'a Message,
	) -> Pin<Box<dyn Future<Output = Outcome> + Send + 'a>> {
		let send = async move {
			let credential = target.credential;

			// Browsers require every push to show a notification — Chrome
			// shows its own generic one, Safari revokes the subscription,
			// Edge takes no silent subscriptions — and the worker has no
			// page to hand data to: a silent data message never goes to
			// one, and no attempt is spent deciding so.
			if message.options.is_data() {
				return Outcome::Ended {
					reason: DeliveryReason::PlatformUnsupported,
					invalidate: false,
				};
			}

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

			// Size is checked on the encrypted body, trimmed in the
			// shared order; a body past the record size would overflow
			// the single record its header declares, so nothing oversized
			// is ever sent.
			let shaped = shape(message, leptoskit_db::now());

			match self.build_wire(&subscription.endpoint, &subscription.keys, &shaped) {
				Ok(request) if shaped.fits && request.body.len() <= BODY_LIMIT => {
					self.post(request, pinned).await
				}
				Ok(_) => {
					Outcome::Ended {
						reason: DeliveryReason::Oversized,
						invalidate: false,
					}
				}
				// A `p256dh` that isn't a point on the curve fails
				// whatever the payload: saying `oversized` would send the
				// operator to the message.
				Err(BuildFailure::Keys) => {
					Outcome::Ended {
						reason: DeliveryReason::SubscriptionUnusable,
						invalidate: false,
					}
				}
				// The system RNG refused: it comes back.
				Err(BuildFailure::Random) => Outcome::Transient { after: None },
			}
		};

		Box::pin(send)
	}
}

/// The ephemeral key material of one message: a fresh P-256 key pair
/// and a fresh salt, neither ever reused — RFC 8291's requirement that
/// a push message's sender key be discardable, and the agreement's own
/// signature keeps it single-use.
struct Seeds {
	private: EphemeralPrivateKey,
	salt: [u8; 16],
}

impl Seeds {
	/// Fresh from the system RNG.
	fn fresh(rng: &SystemRandom) -> Result<Self, ring::error::Unspecified> {
		let mut salt = [0_u8; 16];
		rng.fill(&mut salt)?;
		let private = EphemeralPrivateKey::generate(&ECDH_P256, rng)?;

		Ok(Self { private, salt })
	}
}

/// RFC 8291 encryption of one payload: the ephemeral agreement against
/// the subscription's `p256dh` point, the two-stage HKDF over the
/// subscription's `auth` secret, and one sealed `aes128gcm` record
/// under its RFC 8188 header. A fresh ephemeral key and salt are drawn
/// for every call, and the agreement consumes the key.
///
/// # Errors
///
/// [`BuildFailure::Keys`] when the subscription's point isn't on the
/// curve, [`BuildFailure::Random`] when the system RNG refuses — never one
/// carrying key material.
fn encrypt_payload(
	ua_public: &[u8],
	auth_secret: &[u8],
	payload: &[u8],
) -> Result<Vec<u8>, BuildFailure> {
	let seeds = Seeds::fresh(&SystemRandom::new()).map_err(|_| BuildFailure::Random)?;
	let sender_public = seeds
		.private
		.compute_public_key()
		.map_err(|_| BuildFailure::Random)?;

	agree_ephemeral(
		seeds.private,
		&UnparsedPublicKey::new(&ECDH_P256, ua_public),
		|shared| {
			seal_record(
				shared,
				sender_public.as_ref(),
				ua_public,
				auth_secret,
				&seeds.salt,
				payload,
			)
		},
	)
	.map_err(|_| BuildFailure::Keys)
}

/// The half of RFC 8291 below the agreement: the two HKDF stages, the
/// content key and nonce, the RFC 8188 header (`salt || rs || idlen ||
/// keyid`, the key id the sender's public point), and the single
/// record — the payload, the `0x02` last-record delimiter, and no
/// padding past it — sealed AES-128-GCM. Split from
/// [`encrypt_payload`] so the RFC's Appendix A vector can drive the
/// derivation with its own fixed key material in the tests.
fn seal_record(
	shared: &[u8],
	sender_public: &[u8],
	ua_public: &[u8],
	auth_secret: &[u8],
	salt: &[u8; 16],
	payload: &[u8],
) -> Vec<u8> {
	// IKM = HKDF-Expand(HKDF-Extract(auth_secret, shared),
	// "WebPush: info\0" || ua_public || as_public, 32)
	let ikm = hkdf(
		auth_secret,
		shared,
		&[b"WebPush: info\0", ua_public, sender_public],
		32,
	);
	// CEK and NONCE from HKDF-Extract(salt, IKM), RFC 8188's infos.
	let cek = hkdf(salt, &ikm, &[b"Content-Encoding: aes128gcm\0"], 16);
	let nonce = hkdf(salt, &ikm, &[b"Content-Encoding: nonce\0"], 12);

	let mut body = Vec::with_capacity(86 + payload.len() + 1 + 16);
	body.extend_from_slice(salt);
	body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
	// idlen: the key id is the sender's 65-byte uncompressed point.
	body.push(65);
	body.extend_from_slice(sender_public);

	let mut record = Vec::with_capacity(payload.len() + 1 + 16);
	record.extend_from_slice(payload);
	record.push(0x02);
	LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &cek).expect("a 16-byte content key"))
		.seal_in_place_append_tag(
			Nonce::assume_unique_for_key(nonce.try_into().expect("a 12-byte nonce")),
			Aad::empty(),
			&mut record,
		)
		.expect("the record fits its reserved tag room");
	body.extend_from_slice(&record);
	body
}

/// HKDF-SHA256 — extract then expand — for the fixed, valid lengths
/// RFC 8291 names.
fn hkdf(salt: &[u8], secret: &[u8], infos: &[&[u8]], length: usize) -> Vec<u8> {
	struct Length(usize);

	impl KeyType for Length {
		fn len(&self) -> usize {
			self.0
		}
	}

	let okm = Salt::new(HKDF_SHA256, salt).extract(secret);
	let okm = okm.expand(infos, Length(length)).expect("a valid length");
	let mut out = vec![0_u8; length];
	okm.fill(&mut out).expect("a valid fill");
	out
}

/// What a payload carries beside its ids: everything, one step of the
/// documented degradation order down, or the id-only envelope the
/// record size can force.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Include {
	/// The send's options and its `data`.
	All,
	/// The web presentation options and the image dropped.
	NoPlatform,
	/// The silent flag, the badge, and the link dropped too.
	NoSimpleOptions,
	/// The id-only envelope: the visible text and the notification's id.
	Minimal,
}

/// How long the encrypted body of a `plaintext`-byte envelope is: the
/// RFC 8188 header — a 16-byte salt, the 4-byte record size, the key id's
/// length, and the sender's 65-byte public point — then the one record:
/// the envelope, its 1-byte delimiter, and the 16-byte AES-GCM tag.
const fn encrypted_len(plaintext: usize) -> usize {
	16 + 4 + 1 + 65 + plaintext + 1 + 16
}

/// What a browser's push service is sent for `message`, as of `now` —
/// which the remaining life the `ttl` header names is measured from: the
/// envelope the worker reads, before it is encrypted for one browser,
/// trimmed in the shared order — the web options and the image, then the
/// silent flag, badge, and link, then the id-only envelope — until its
/// encrypted length fits the record.
pub(crate) fn shape(message: &Message, now: leptoskit_db::Timestamp) -> Shaped {
	let ttl = message.expires_at.map_or(DEFAULT_TTL_SECS, |expires_at| {
		(expires_at - now).num_seconds().clamp(0, DEFAULT_TTL_SECS)
	});
	// Urgency tells the push service how hard to try: a high-priority
	// send wakes the radio, an ordinary one may wait for the device to
	// wake on its own. Unset is the service's ordinary, as before.
	let urgency = match message.options.priority {
		Some(crate::options::SendPriority::High) => "high",
		Some(crate::options::SendPriority::Normal) | None => "normal",
	};
	let headers = vec![
		("ttl".to_owned(), ttl.clamp(0, DEFAULT_TTL_SECS).to_string()),
		("urgency".to_owned(), urgency.to_owned()),
		// The stable tag: a repeat replaces, not stacks, wherever the
		// service coalesces — best-effort, like every provider's. A topic
		// is 32 base64url characters, exactly a UUID without its dashes.
		// A collapse key the send chose is held to the same alphabet and
		// length by its own check.
		(
			"topic".to_owned(),
			message
				.options
				.collapse_key
				.clone()
				.unwrap_or_else(|| message.id.simple().to_string()),
		),
		("content-encoding".to_owned(), "aes128gcm".to_owned()),
		(
			"content-type".to_owned(),
			"application/octet-stream".to_owned(),
		),
	];
	let stages = [
		Include::All,
		Include::NoPlatform,
		Include::NoSimpleOptions,
		Include::Minimal,
	];
	let mut trims = 0;
	let mut body = envelope(message, stages[0]);

	while encrypted_len(body.len()) > BODY_LIMIT && trims + 1 < stages.len() {
		trims += 1;
		body = envelope(message, stages[trims]);
	}

	let bytes = encrypted_len(body.len());

	Shaped {
		headers,
		body,
		bytes,
		limit: BODY_LIMIT,
		trims,
		fits: bytes <= BODY_LIMIT,
	}
}

/// The push envelope a client's service worker reads: the visible text,
/// the notification's id, and — when it fits — the send's `data` and the
/// options that shape the banner: an image, a badge count, a click link,
/// a silent flag, and the web presentation options. No executable
/// anything rides in it.
fn envelope(message: &Message, include: Include) -> Vec<u8> {
	let mut payload = serde_json::json!({
		"title": message.title,
		"body": message.body,
		"lkn": message.id.to_string(),
	});

	// The tag the worker shows the banner under, when the send chose a
	// collapse key: at most 32 characters, and what makes a later push
	// replace a banner already shown, so it stays in the smallest
	// envelope too.
	if let Some(key) = message.options.collapse_key.as_deref() {
		payload["lktag"] = serde_json::json!(key);
	}

	if include != Include::Minimal
		&& let Some(object) = serde_json::from_str::<serde_json::Value>(&message.data)
			.ok()
			.and_then(|data| data.as_object().cloned())
			.filter(|object| !object.is_empty())
	{
		payload["lk"] = serde_json::Value::Object(object);
	}

	if include == Include::All
		&& let Some(image) = message.options.image.as_deref()
	{
		payload["image"] = serde_json::json!(image);
	}

	if include != Include::Minimal {
		if let Some(badge) = message.badge {
			payload["lkbadge"] = serde_json::json!(badge);
		}

		if let Some(link) = message.options.link.as_deref() {
			payload["lklink"] = serde_json::json!(link);
		}

		if matches!(
			message.options.sound.as_ref(),
			Some(crate::options::NotificationSound::Silent)
		) {
			payload["silent"] = serde_json::json!(true);
		}
	}

	if include == Include::All
		&& let Some(web) = message.options.web.as_ref()
	{
		let mut presentation = serde_json::Map::new();

		if let Some(icon) = web.icon.as_deref() {
			presentation.insert("icon".to_owned(), serde_json::json!(icon));
		}

		if let Some(badge) = web.badge_icon.as_deref() {
			presentation.insert("badge".to_owned(), serde_json::json!(badge));
		}

		if !web.actions.is_empty() {
			presentation.insert(
				"actions".to_owned(),
				serde_json::Value::Array(
					web.actions
						.iter()
						.map(|action| {
							serde_json::json!({
								"action": action.action,
								"title": action.title,
							})
						})
						.collect(),
				),
			);
		}

		if let Some(require_interaction) = web.require_interaction {
			presentation.insert(
				"require_interaction".to_owned(),
				serde_json::json!(require_interaction),
			);
		}

		if let Some(dir) = web.dir {
			presentation.insert("dir".to_owned(), serde_json::json!(dir.as_str()));
		}

		if let Some(lang) = web.lang.as_deref() {
			presentation.insert("lang".to_owned(), serde_json::json!(lang));
		}

		if !presentation.is_empty() {
			payload["lkw"] = serde_json::Value::Object(presentation);
		}
	}

	serde_json::to_vec(&payload).unwrap_or_default()
}

/// Classifies one push-service answer. RFC 8030 §7.3 signals an expired
/// subscription with 404, and the shipped services answer 410 for one
/// that's gone: only those invalidate. RFC 8292 §4.2 refuses a missing or
/// invalid VAPID signature with 401 or 403 — and Apple answers 403
/// `VapidPkHashMismatch` once the operator's key pair is no longer the one
/// the subscription was made for — which is the operator's to fix, so
/// the subscription stays.
fn classify(response: &WireResponse) -> Outcome {
	match response.status {
		200..=299 => Outcome::Accepted,
		401 | 403 => {
			Outcome::Ended {
				reason: DeliveryReason::OperatorConfiguration,
				invalidate: false,
			}
		}
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

fn retry_after(response: &WireResponse) -> Option<Duration> {
	super::retry_after(response, std::time::SystemTime::now())
}

#[cfg(test)]
mod tests {
	use super::seal_record;
	use crate::push::base64_url;

	/// Every answer RFC 8030 and RFC 8292 define, as the shipped push
	/// services give them, and what each comes to. A subscription the
	/// push service no longer knows (RFC 8030 §7.3's 404, the services'
	/// 410) invalidates the endpoint; a refused VAPID signature (RFC 8292
	/// §4.2's 401 or 403, Apple's `VapidPkHashMismatch` after a key
	/// rotation) is the operator's configuration and leaves it enrolled.
	#[test]
	fn push_service_answers_classify_as_the_rfcs_define_them() {
		use super::DeliveryReason;
		use super::Outcome;
		use super::WireResponse;
		use super::classify;

		let ended = |reason, invalidate| Outcome::Ended { reason, invalidate };

		for (status, expected) in [
			(201, Outcome::Accepted),
			(400, ended(DeliveryReason::ProviderRejected, false)),
			(401, ended(DeliveryReason::OperatorConfiguration, false)),
			(403, ended(DeliveryReason::OperatorConfiguration, false)),
			(404, ended(DeliveryReason::Unregistered, true)),
			(410, ended(DeliveryReason::Unregistered, true)),
			(413, ended(DeliveryReason::Oversized, false)),
			(429, Outcome::Transient { after: None }),
			(500, Outcome::Transient { after: None }),
			(503, Outcome::Transient { after: None }),
			(302, ended(DeliveryReason::ProviderRejected, false)),
		] {
			let response = WireResponse {
				status,
				headers: Vec::new(),
				body: Vec::new(),
			};
			assert_eq!(classify(&response), expected, "{status}");
		}
	}

	/// The RFC 8291 §5 and Appendix A vector, byte for byte: the fixed
	/// plaintext, receiver keys, auth secret, sender point, and salt
	/// drive the sealing core, and the body it answers equals the RFC's
	/// published 144-octet encrypted content exactly. The agreement
	/// above the core is driven with the RFC's own published ECDH
	/// secret — ring exposes no fixed-scalar agreement — and the fresh
	/// per-message key path is proven end to end by the independent
	/// receiver tests in `tests/push.rs`.
	#[test]
	fn appendix_a_encrypts_byte_for_byte() {
		let shared = base64_url("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs").unwrap();
		let sender_public =
			base64_url("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8")
				.unwrap();
		let ua_public = base64_url(
			"BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
		)
		.unwrap();
		let auth_secret = base64_url("BTBZMqHH6r4Tts7J_aSIgg").unwrap();
		let salt: [u8; 16] = base64_url("DGv6ra1nlYgDCS1FRnbzlw")
			.unwrap()
			.try_into()
			.unwrap();
		let plaintext = b"When I grow up, I want to be a watermelon";
		let expected = base64_url(
			"DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN",
		)
		.unwrap();

		let body = seal_record(
			&shared,
			&sender_public,
			&ua_public,
			&auth_secret,
			&salt,
			plaintext,
		);

		assert_eq!(
			body, expected,
			"the RFC 8291 Appendix A body, byte for byte"
		);
	}

	/// The length a preview predicts for an encrypted body is the length
	/// encryption gives it, for an empty envelope, a small one, and one
	/// that exactly fills the record.
	#[test]
	fn the_predicted_encrypted_length_is_the_real_one() {
		let ua_public = base64_url(
			"BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
		)
		.unwrap();
		let auth_secret = base64_url("BTBZMqHH6r4Tts7J_aSIgg").unwrap();

		for length in [0, 1, 41, 1_000, super::BODY_LIMIT - super::encrypted_len(0)] {
			let Ok(body) = super::encrypt_payload(&ua_public, &auth_secret, &vec![b'x'; length])
			else {
				panic!("a {length}-byte envelope encrypts");
			};

			assert_eq!(body.len(), super::encrypted_len(length), "{length} bytes");
		}

		assert_eq!(
			super::encrypted_len(super::BODY_LIMIT - super::encrypted_len(0)),
			super::BODY_LIMIT,
			"the largest envelope fills the record exactly"
		);
	}
}
