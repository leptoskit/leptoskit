//! The Web Push transport: RFC 8030 posts of RFC 8291-encrypted
//! envelopes, signed RFC 8292 VAPID, to the browser's push service.
//!
//! The envelope is the documented one — title, body, the notification's
//! id, and the send's `data` — and a client's service worker shows it or
//! not as it chooses; nothing executable rides in it. Encryption is the
//! standard path over ring's primitives: an ephemeral P-256 key per
//! message, ECDH against the subscription's public key, HKDF-SHA256, and
//! AES-128-GCM in the `aes128gcm` framing, with a fresh key and salt
//! every time. VAPID is the same ES256 JWT the other transports sign,
//! its audience the endpoint's origin and its subject the operator's
//! contact. The operator holds the key pair the ecosystem's own tools
//! generate: a 32-byte private key and a 65-byte public key, base64url.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::aead::Aad;
use ring::aead::LessSafeKey;
use ring::aead::NONCE_LEN;
use ring::aead::Nonce;
use ring::agreement::EphemeralPrivateKey;
use ring::agreement::UnparsedPublicKey;
use ring::agreement::agree_ephemeral;
use ring::hkdf::HKDF_SHA256;
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
use crate::service::transport::Transport;
use crate::service::transport::WireRequest;
use crate::service::transport::WireResponse;
use crate::service::transport::host;

/// The VAPID JWT's life: RFC 8292 allows a day; twelve hours leaves
/// room for clock drift and needs no caching.
const VAPID_LIFETIME_SECS: i64 = 12 * 3_600;

/// The largest body a push service must take: the encrypted record's
/// record size, header included.
const BODY_LIMIT: usize = 4_096;

/// The `aes128gcm` record size, which the body's single record carries.
const RECORD_SIZE: u32 = 4_096;

/// The tag AES-128-GCM appends.
const TAG_LEN: usize = 16;

/// The record's last-byte delimiter, past the payload.
const RECORD_DELIMITER: u8 = 0x02;

/// The longest time a push may sit at the service, in seconds: four
/// weeks, the largest TTL the example services take.
const DEFAULT_TTL_SECS: i64 = 28 * 24 * 3_600;

/// Builds a ring-signable P-256 key from the private scalar and the
/// public point, as the VAPID key pair's two base64url halves. Ring's
/// `from_pkcs8` requires the PKCS#8 form that carries both, so the pair
/// is spliced into the DER template ring itself signs from. The pair is
/// cross-checked first: a mistyped half fails here, not at every push
/// service at once.
///
/// # Errors
///
/// A message naming which half is wrong.
fn vapid_pair(private_scalar: &[u8], public_point: &[u8]) -> Result<EcdsaKeyPair, String> {
	if private_scalar.len() != 32 {
		return Err("the VAPID private key isn't 32 bytes".into());
	}

	if public_point.len() != 65 || public_point[0] != 0x04 {
		return Err("the VAPID public key isn't a 65-byte uncompressed point".into());
	}

	// The PKCS#8 v1 EC private key ring's
	// `EC_PUBLIC_KEY_P256_PKCS8_V1_TEMPLATE` matches: PKCS#8 header,
	// algorithm identifiers, the RFC 5915 key with its scalar, and the
	// public point in a bit string. The scalar sits at offset 36, the point
	// at 73, exactly as ring's own template places them.
	let mut pkcs8 = Vec::with_capacity(138);
	pkcs8.extend_from_slice(&[
		0x30, 0x81, 0x87, // SEQUENCE, 135 bytes
		0x02, 0x01, 0x00, // PKCS#8 version 0
		0x30, 0x13, // AlgorithmIdentifier, 19 bytes
		0x06, 0x07, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01, // OID ecPublicKey
		0x06, 0x08, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07, // OID P-256
		0x04, 0x6D, // OCTET STRING, 109 bytes
		0x30, 0x6B, // SEQUENCE, 107 bytes
		0x02, 0x01, 0x01, // RFC 5915 version 1
		0x04, 0x20, // OCTET STRING, 32 bytes: the scalar
	]);
	debug_assert_eq!(pkcs8.len(), 36);
	pkcs8.extend_from_slice(private_scalar);
	pkcs8.extend_from_slice(&[0xA1, 0x44, 0x03, 0x42, 0x00]); // [1] BIT STRING, no unused bits
	debug_assert_eq!(pkcs8.len(), 73);
	pkcs8.extend_from_slice(public_point);
	debug_assert_eq!(pkcs8.len(), 138);

	EcdsaKeyPair::from_pkcs8(
		&ECDSA_P256_SHA256_FIXED_SIGNING,
		&pkcs8,
		&SystemRandom::new(),
	)
	.map_err(|_| "the VAPID private key doesn't match its public key".to_owned())
}

/// The Web Push sender.
pub struct WebPush {
	key: EcdsaKeyPair,
	/// The public key, base64url without padding, as browsers subscribe
	/// with it.
	public_key: String,
	subject: String,
	policy: host::HostPolicy,
	boundary: std::sync::Arc<dyn Boundary>,
}

impl WebPush {
	/// Builds the transport from the operator's configuration, parsing
	/// the VAPID pair now.
	///
	/// # Errors
	///
	/// A message naming what's wrong with the keys or the subject.
	pub fn new(
		config: &WebPushConfig,
		boundary: std::sync::Arc<dyn Boundary>,
	) -> Result<Self, String> {
		let private = base64_url(&config.vapid_private_key)
			.ok_or("Web Push: the VAPID private key isn't base64url")?;
		let public = base64_url(&config.vapid_public_key)
			.ok_or("Web Push: the VAPID public key isn't base64url")?;
		let key = vapid_pair(&private, &public).map_err(|error| format!("Web Push: {error}"))?;

		let subject = config.subject.trim();

		if !(subject.starts_with("mailto:") || subject.starts_with("https://"))
			|| subject.len() <= 8
			|| subject.len() > 254
		{
			return Err("Web Push: the VAPID subject is a mailto: or https: contact".into());
		}

		Ok(Self {
			key,
			public_key: URL_SAFE_NO_PAD.encode(public),
			subject: subject.to_owned(),
			policy: host::HostPolicy::with_extra_hosts(&config.extra_hosts),
			boundary,
		})
	}

	/// The host policy enrollments and attempts are checked against.
	pub(crate) fn policy(&self) -> &host::HostPolicy {
		&self.policy
	}

	/// The `Authorization` header's VAPID value for one endpoint: `vapid
	/// t=<JWT>, k=<public key>`.
	///
	/// # Errors
	///
	/// A message when signing fails.
	fn vapid(&self, origin: &str) -> Result<String, String> {
		let now = leptoskit_db::now().timestamp();
		let header = r#"{"typ":"JWT","alg":"ES256"}"#;
		let claims = format!(
			"{{\"aud\":{},\"exp\":{},\"sub\":{}}}",
			serde_json::to_string(origin).expect("a plain string"),
			now + VAPID_LIFETIME_SECS,
			serde_json::to_string(&self.subject).expect("a plain string"),
		);
		let signed = format!(
			"{}.{}",
			URL_SAFE_NO_PAD.encode(header),
			URL_SAFE_NO_PAD.encode(claims)
		);
		let signature = self
			.key
			.sign(&SystemRandom::new(), signed.as_bytes())
			.map_err(|_| "Web Push: the VAPID key couldn't sign".to_owned())?;

		Ok(format!(
			"vapid t={signed}.{}, k={}",
			URL_SAFE_NO_PAD.encode(signature.as_ref()),
			self.public_key
		))
	}

	/// Checks a stored subscription against the host policy, resolving
	/// and pinning an extra host's addresses for the connection. Shipped
	/// providers answer unpinned.
	async fn route(&self, endpoint: &str) -> Result<(url::Url, Option<Pinned>), String> {
		let url =
			url::Url::parse(endpoint).map_err(|_| "a stored endpoint doesn't parse".to_owned())?;
		let pinning = self.policy.check(&url)?;

		match pinning {
			Some(mut pinned) => {
				pinned.addrs = host::resolve_pinned(&pinned.host, pinned.port).await?.addrs;
				Ok((url, Some(pinned)))
			}
			None => Ok((url, None)),
		}
	}

	/// Posts one encrypted body and classifies the answer.
	async fn post(
		&self,
		url: &url::Url,
		pinned: Option<Pinned>,
		vapid: String,
		body: Vec<u8>,
		ttl: i64,
		message: &Message,
	) -> Outcome {
		let request = WireRequest {
			url: url.clone().into(),
			headers: vec![
				("authorization".to_owned(), vapid),
				("content-encoding".to_owned(), "aes128gcm".to_owned()),
				(
					"content-type".to_owned(),
					"application/octet-stream".to_owned(),
				),
				("ttl".to_owned(), ttl.max(0).to_string()),
				("urgency".to_owned(), "normal".to_owned()),
				// The stable tag: a repeat replaces, not stacks. A topic
				// is 32 bytes, exactly a UUID without its dashes.
				("topic".to_owned(), message.id.simple().to_string()),
			],
			body,
			pinned,
		};
		let answer = self.boundary.post(request).await;

		match answer {
			Ok(response) => classify(&response),
			Err(_) => Outcome::Transient { after: None },
		}
	}
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
			let subscription = crate::push::stored_web_subscription(credential);

			let Ok(subscription) = subscription.as_ref().map(|s| &s.endpoint) else {
				return Outcome::Ended {
					reason: DeliveryReason::EndpointUnavailable,
					invalidate: false,
				};
			};
			let Ok((url, pinned)) = self.route(subscription).await else {
				return Outcome::Ended {
					reason: DeliveryReason::EndpointUnavailable,
					invalidate: false,
				};
			};

			let keys = match crate::push::stored_web_subscription(credential) {
				Ok(subscription) => subscription.keys,
				Err(_) => {
					return Outcome::Ended {
						reason: DeliveryReason::EndpointUnavailable,
						invalidate: false,
					};
				}
			};

			let Ok(vapid) = self.vapid(&format!(
				"{}://{}",
				url.scheme(),
				url.host_str().unwrap_or_default()
			)) else {
				return Outcome::Transient { after: None };
			};

			let ttl = message.expires_at.map_or(DEFAULT_TTL_SECS, |expires_at| {
				(expires_at - leptoskit_db::now())
					.num_seconds()
					.clamp(0, DEFAULT_TTL_SECS)
			});

			// Size is checked on the encoded body: the full envelope
			// first, then the minimal id-only one, and past both a
			// terminal clear result.
			let full = envelope(message, true);
			let encrypted = encrypt(&keys, &full);

			match encrypted {
				Ok(body) if body.len() <= BODY_LIMIT => {
					self.post(&url, pinned, vapid, body, ttl, message).await
				}
				_ => {
					let minimal = envelope(message, false);
					let encrypted = encrypt(&keys, &minimal);

					match encrypted {
						Ok(body) if body.len() <= BODY_LIMIT => {
							self.post(&url, pinned, vapid, body, ttl, message).await
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

/// Encrypts `payload` for a subscription, RFC 8291 with the `aes128gcm`
/// content coding: one record, no padding, the sender's fresh public key
/// in the header as its key id.
///
/// # Errors
///
/// A message for keys that don't decode or a payload past one record.
pub(crate) fn encrypt(keys: &WebPushKeys, payload: &[u8]) -> Result<Vec<u8>, String> {
	let record_limit = RECORD_SIZE as usize - TAG_LEN - 1;

	if payload.len() > record_limit {
		return Err(format!(
			"a push record carries at most {record_limit} bytes"
		));
	}

	let rng = SystemRandom::new();
	let private = EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
		.map_err(|_| "Web Push: no ephemeral key".to_owned())?;
	let public = private
		.compute_public_key()
		.map_err(|_| "Web Push: no ephemeral public key".to_owned())?
		.as_ref()
		.to_vec();
	let mut salt = [0_u8; 16];
	rng.fill(&mut salt)
		.map_err(|_| "Web Push: no salt".to_owned())?;

	let browser = base64_url(&keys.p256dh).ok_or("the p256dh key isn't base64url")?;
	let auth = base64_url(&keys.auth).ok_or("the auth secret isn't base64url")?;
	let browser_public = UnparsedPublicKey::new(&ring::agreement::ECDH_P256, browser.clone());
	let shared = agree_ephemeral(private, &browser_public, <[u8]>::to_vec)
		.map_err(|_| "the subscription's public key isn't a P-256 point")?;

	// The input key material: HKDF over the ECDH secret, salted with the
	// subscription's auth secret, info "WebPush: info", a NUL, and both
	// public keys — the subscription's first, then the sender's.
	let mut key_info = Vec::with_capacity(14 + 65 + 65);
	key_info.extend_from_slice(b"WebPush: info\0");
	key_info.extend_from_slice(&browser);
	key_info.extend_from_slice(&public);
	let ikm = derive(&auth, &shared, &[&key_info], 32);

	// The content key and nonce, HKDF over the IKM, salted fresh per
	// message.
	let mut cek = [0_u8; 16];
	let mut nonce = [0_u8; NONCE_LEN];
	let content = derive(&salt, &ikm, &[b"Content-Encoding: aes128gcm\0"], 16);
	cek.copy_from_slice(&content);
	let nonce_bytes = derive(&salt, &ikm, &[b"Content-Encoding: nonce\0"], 12);
	nonce.copy_from_slice(&nonce_bytes);

	// One record: the payload, the last-record delimiter, and the tag.
	let mut record = Vec::with_capacity(payload.len() + 1 + TAG_LEN);
	record.extend_from_slice(payload);
	record.push(RECORD_DELIMITER);

	let key = LessSafeKey::new(
		ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, &cek)
			.map_err(|_| "Web Push: the content key didn't build".to_owned())?,
	);
	key.seal_in_place_append_tag(
		Nonce::assume_unique_for_key(nonce),
		Aad::empty(),
		&mut record,
	)
	.map_err(|_| "Web Push: the record didn't seal".to_owned())?;

	// The header: salt, record size, key-id length, and the sender's
	// public key as the key id.
	let mut body = Vec::with_capacity(21 + public.len() + record.len());
	body.extend_from_slice(&salt);
	body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
	body.push(u8::try_from(public.len()).map_err(|_| "the public key is too long")?);
	body.extend_from_slice(&public);
	body.extend_from_slice(&record);

	Ok(body)
}

/// HKDF-SHA256: `length` bytes from `salt` and `secret` through `infos`.
/// The lengths asked for here are fixed and valid, so an expansion
/// failure is a bug and stops the run.
fn derive(salt: &[u8], secret: &[u8], infos: &[&[u8]], length: usize) -> Vec<u8> {
	struct Length(usize);

	impl ring::hkdf::KeyType for Length {
		fn len(&self) -> usize {
			self.0
		}
	}

	let salt = Salt::new(HKDF_SHA256, salt);
	let prk = salt.extract(secret);
	let expansion = prk
		.expand(infos, Length(length))
		.expect("these HKDF-SHA256 lengths are valid");
	let mut okm = vec![0_u8; length];
	expansion
		.fill(&mut okm)
		.expect("fill writes exactly the length expanded for");

	okm
}

/// The parts of an `aes128gcm` body's header: the salt, the record size,
/// and the header's remainder split into the sender's public key and the
/// record.
#[cfg(test)]
pub(crate) type Header<'a> = ([u8; 16], u32, &'a [u8], &'a [u8]);

/// Reads a body's `aes128gcm` header: salt, record size, and the
/// sender's public key. The receiver side of the round-trip test.
#[cfg(test)]
pub(crate) fn read_header(body: &[u8]) -> Result<Header<'_>, String> {
	if body.len() < 21 {
		return Err("the body is shorter than an aes128gcm header".into());
	}

	let mut salt = [0_u8; 16];
	salt.copy_from_slice(&body[0..16]);
	let record_size = u32::from_be_bytes([body[16], body[17], body[18], body[19]]);
	let key_id_len = body[20] as usize;
	let end = 21 + key_id_len;

	if body.len() < end {
		return Err("the header's key id runs past the body".into());
	}

	Ok((salt, record_size, &body[21..end], &body[end..]))
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

#[cfg(test)]
mod tests {
	use base64::Engine as _;
	use base64::engine::general_purpose::URL_SAFE_NO_PAD;
	use ring::aead::Aad;
	use ring::aead::LessSafeKey;
	use ring::aead::Nonce;
	use ring::agreement::EphemeralPrivateKey;
	use ring::agreement::UnparsedPublicKey;
	use ring::agreement::agree_ephemeral;
	use ring::rand::SecureRandom as _;
	use ring::rand::SystemRandom;
	use ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING;
	use ring::signature::EcdsaKeyPair;
	use ring::signature::KeyPair;

	use super::BODY_LIMIT;
	use super::RECORD_SIZE;
	use super::vapid_pair;
	use crate::push::WebPushKeys;

	/// A VAPID pair from the ecosystem's raw-scalar format rebuilds into
	/// a ring key that signs and verifies as itself.
	#[test]
	fn a_vapid_pair_rebuilds_and_verifies() {
		let rng = SystemRandom::new();
		let document =
			EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
		let der = document.as_ref();
		// The scalar sits at offset 36 in ring's template, the point at 73.
		let scalar = &der[36..68];
		let point = &der[73..138];
		let rebuilt = vapid_pair(scalar, point).expect("the pair rebuilds");

		assert_eq!(rebuilt.public_key().as_ref(), point, "the point is kept");

		// The rebuilt key signs something a verifier checks with the
		// point: the same ES256 the push services verify VAPID with.
		let signed = rebuilt.sign(&rng, b"a vapid claim").unwrap();
		let verified = ring::signature::UnparsedPublicKey::new(
			&ring::signature::ECDSA_P256_SHA256_FIXED,
			point,
		)
		.verify(b"a vapid claim", signed.as_ref());
		assert!(verified.is_ok(), "the signature verifies");
	}

	/// A scalar without its point, or with a point that isn't the
	/// scalar's, is refused.
	#[test]
	fn a_mismatched_vapid_pair_is_refused() {
		let rng = SystemRandom::new();
		let one = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
		let two = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
		let scalar = &one.as_ref()[36..68];
		let other_point = &two.as_ref()[73..138];

		assert!(vapid_pair(scalar, other_point).is_err());
		assert!(vapid_pair(&scalar[..31], other_point).is_err());
	}

	/// An encrypted body decrypts on the receiver's side of RFC 8291:
	/// the header carries the sender's fresh public key, the keys derive
	/// the same way from the receiver's view, and the record opens to the
	/// payload.
	#[test]
	fn an_encrypted_body_decrypts_on_the_receivers_side() {
		let rng = SystemRandom::new();

		// The receiver's subscription, as a browser would hold it: a
		// private key for decryption and the public point it shares.
		let receiver_private =
			EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng).unwrap();
		let receiver_point = receiver_private.compute_public_key().unwrap();
		let mut auth_secret = [0_u8; 16];
		rng.fill(&mut auth_secret).unwrap();

		let keys = WebPushKeys {
			p256dh: URL_SAFE_NO_PAD.encode(receiver_point.as_ref()),
			auth: URL_SAFE_NO_PAD.encode(auth_secret),
		};
		let payload = b"When I grow up, I want to be a watermelon";
		let body = super::encrypt(&keys, payload).unwrap();
		assert!(body.len() <= BODY_LIMIT, "one record fits the limit");

		let (salt, record_size, sender_point, record) = super::read_header(&body).unwrap();
		assert_eq!(record_size, RECORD_SIZE, "one 4 KiB record");
		assert_eq!(sender_point.len(), 65, "the key id is the sender's point");
		assert_eq!(sender_point[0], 0x04, "an uncompressed point");

		// The receiver derives from their own view: ua_public is theirs,
		// as_public is the sender's from the header, and the ECDH secret
		// is theirs against the sender's point.
		let shared = agree_ephemeral(
			receiver_private,
			&UnparsedPublicKey::new(&ring::agreement::ECDH_P256, sender_point.to_vec()),
			<[u8]>::to_vec,
		)
		.expect("the sender's point is on the curve");
		let mut key_info = Vec::new();
		key_info.extend_from_slice(b"WebPush: info\0");
		key_info.extend_from_slice(receiver_point.as_ref());
		key_info.extend_from_slice(sender_point);
		let ikm = super::derive(&auth_secret, &shared, &[&key_info], 32);
		let cek = super::derive(&salt, &ikm, &[b"Content-Encoding: aes128gcm\0"], 16);
		let nonce = super::derive(&salt, &ikm, &[b"Content-Encoding: nonce\0"], 12);

		let key =
			LessSafeKey::new(ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, &cek).unwrap());
		let mut record = record.to_vec();
		let opened = key
			.open_in_place(
				Nonce::assume_unique_for_key(nonce.try_into().expect("the nonce is 12 bytes")),
				Aad::empty(),
				&mut record,
			)
			.expect("the record opens");
		assert_eq!(&opened[..opened.len() - 1], payload, "the payload opens");
		assert_eq!(opened[opened.len() - 1], 0x02, "the last-record delimiter");
	}

	/// A payload past one record is refused before a byte is encrypted.
	#[test]
	fn a_payload_past_one_record_is_refused() {
		let rng = SystemRandom::new();
		let receiver_private =
			EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng).unwrap();
		let keys = WebPushKeys {
			p256dh: URL_SAFE_NO_PAD.encode(receiver_private.compute_public_key().unwrap().as_ref()),
			auth: URL_SAFE_NO_PAD.encode([7_u8; 16]),
		};

		let too_long = vec![0_u8; RECORD_SIZE as usize];
		assert!(super::encrypt(&keys, &too_long).is_err());
	}
}
