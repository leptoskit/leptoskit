//! <!-- {=p_app_integritySummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_app_integrity` as a direct dependency and select the
//! `app-integrity` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Generated Linux hosts register it where it
//! has a Linux Rust implementation; generated Windows and terminal hosts
//! currently omit runtime plugins, so use explicit registration in a custom
//! host there. Guests enable the `guest` feature and reuse
//! `guest::AppIntegrity` rather than resolving the plugin on every call. Query
//! whether attestation is available, then request a bounded token for an
//! app-owned challenge. Tokens are opaque bytes for server verification; this
//! MVP returns Unavailable on every host until App Attest / Play Integrity
//! wiring lands. Never invents a passing attestation.
//!
//! <!-- {/p_app_integritySummary} -->
//!
//! Wire v1: status is empty → four-byte [`Capability`]; attest takes a
//! length-prefixed challenge (≤256 bytes) and returns [`Attestation`].

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "app-integrity";
/// Empty request → [`Capability`].
pub const STATUS: MethodId = MethodId(0);
/// Challenge → [`Attestation`].
pub const ATTEST: MethodId = MethodId(1);
/// Supported wire operations.
pub const METHODS: &[MethodId] = &[STATUS, ATTEST];
/// Maximum UTF-8/byte length of a challenge.
pub const MAX_CHALLENGE_BYTES: usize = 256;
/// Maximum attestation token bytes.
pub const MAX_TOKEN_BYTES: usize = 8192;

/// Whether the host can produce an attestation token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Capability {
	/// OS policy currently blocks attestation.
	Restricted = 0,
	/// A challenge may be submitted.
	Available = 1,
	/// This host has no attestation API.
	Unavailable = 2,
}

impl Capability {
	/// Rejects unknown values and non-exact lengths.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match word(bytes)? {
			0 => Ok(Self::Restricted),
			1 => Ok(Self::Available),
			2 => Ok(Self::Unavailable),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Fixed four-byte response.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}
}

/// Challenge bytes supplied by the app / server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
	bytes: Bytes,
}

impl Challenge {
	/// Rejects empty or oversized challenges.
	pub fn new(bytes: impl Into<Bytes>) -> Result<Self, Error> {
		let bytes = bytes.into();
		if bytes.is_empty() || bytes.len() > MAX_CHALLENGE_BYTES {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { bytes })
	}

	/// Challenge payload.
	#[must_use]
	pub fn bytes(&self) -> &[u8] {
		&self.bytes
	}

	/// Length-prefixed challenge.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.bytes.len());
		output.bytes(&self.bytes)?;
		Ok(output.finish())
	}

	/// Decodes through [`Challenge::new`].
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let challenge = input.bytes()?.to_owned();
		input.finish()?;
		Self::new(challenge)
	}
}

/// Attestation outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attestation {
	/// No token is available for this challenge.
	Unavailable,
	/// Opaque platform token for server verification.
	Token(Bytes),
}

impl Attestation {
	/// Encodes kind then optional token bytes.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(8);
		match self {
			Self::Unavailable => output.u32(0),
			Self::Token(token) => {
				if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
					return Err(Error::InvalidArgument);
				}
				output.u32(1);
				output.bytes(token)?;
			}
		}
		Ok(output.finish())
	}

	/// Decodes kind tags; rejects oversized tokens and trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let kind = input.u32()?;
		let value = match kind {
			0 => Self::Unavailable,
			1 => {
				let token = Bytes::copy_from_slice(input.bytes()?);
				if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
					return Err(Error::InvalidArgument);
				}
				Self::Token(token)
			}
			_ => return Err(Error::InvalidArgument),
		};
		input.finish()?;
		Ok(value)
	}
}

fn word(bytes: &[u8]) -> Result<u32, Error> {
	let mut input = Decoder::new(bytes);
	let value = input.u32()?;
	input.finish()?;
	Ok(value)
}

/// Validates method payloads without invoking a host.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		STATUS => Decoder::new(payload).finish(),
		ATTEST => Challenge::decode(payload).map(|_| ()),
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without attestation.
#[derive(Debug, Default)]
pub struct Unsupported;

impl leptoskit_bridge::Plugin for Unsupported {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		validate(method, &payload)?;
		if method == STATUS {
			Ok(Bytes::copy_from_slice(&Capability::Unavailable.encode()))
		} else {
			Attestation::Unavailable.encode()
		}
	}
}

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct AppIntegrity(leptoskit_bridge::guest::Client);

	impl AppIntegrity {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries whether attestation can be requested.
		pub async fn status(self) -> Result<Capability, Error> {
			Capability::decode(&self.0.call(STATUS, &[])?.await?)
		}

		/// Requests an attestation token for `challenge`.
		pub async fn attest(self, challenge: &Challenge) -> Result<Attestation, Error> {
			Attestation::decode(&self.0.call(ATTEST, &challenge.encode()?)?.await?)
		}
	}
}

/// Browsers have no app attestation API in this MVP.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always Unavailable.
	#[must_use]
	pub const fn status() -> Capability {
		Capability::Unavailable
	}

	/// Always Unavailable.
	#[must_use]
	pub fn attest(_challenge: &Challenge) -> Attestation {
		Attestation::Unavailable
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	#[test]
	fn codecs_round_trip_and_reject_bounds() {
		for (value, expected) in [
			(0, Capability::Restricted),
			(1, Capability::Available),
			(2, Capability::Unavailable),
		] {
			assert_eq!(Capability::decode(&u32::to_le_bytes(value)), Ok(expected));
		}
		for bytes in [&[][..], &[1], &[1, 0, 0], &[1, 0, 0, 0, 0], &[3, 0, 0, 0]] {
			assert_eq!(Capability::decode(bytes), Err(Error::InvalidArgument));
		}

		assert_eq!(Challenge::new(&[][..]), Err(Error::InvalidArgument));
		assert_eq!(
			Challenge::new(vec![0; MAX_CHALLENGE_BYTES + 1]),
			Err(Error::InvalidArgument)
		);
		let challenge = Challenge::new(b"nonce".as_slice()).unwrap();
		assert_eq!(challenge.bytes(), b"nonce");
		assert_eq!(
			Challenge::decode(&challenge.encode().unwrap()),
			Ok(challenge)
		);
		assert_eq!(
			Challenge::decode(&[5, 0, 0, 0, b'n', b'o', b'n', b'c', b'e', 0]),
			Err(Error::InvalidArgument)
		);

		assert_eq!(
			Attestation::decode(&Attestation::Unavailable.encode().unwrap()),
			Ok(Attestation::Unavailable)
		);
		let token = Attestation::Token(Bytes::from_static(b"tok"));
		assert_eq!(Attestation::decode(&token.encode().unwrap()), Ok(token));
		assert_eq!(
			Attestation::decode(&u32::to_le_bytes(3)),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Attestation::Token(Bytes::new()).encode(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Attestation::Token(Bytes::from(vec![0; MAX_TOKEN_BYTES + 1])).encode(),
			Err(Error::InvalidArgument)
		);
		let mut empty_token = Encoder::with_capacity(8);
		empty_token.u32(1);
		empty_token.bytes(&[]).unwrap();
		assert_eq!(
			Attestation::decode(&empty_token.finish()),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn validate_and_unsupported_stay_honest() {
		assert_eq!(validate(STATUS, &[]), Ok(()));
		assert_eq!(validate(STATUS, &[0]), Err(Error::InvalidArgument));
		assert_eq!(
			validate(
				ATTEST,
				&Challenge::new(b"x".as_slice()).unwrap().encode().unwrap()
			),
			Ok(())
		);
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));

		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			Capability::decode(&plugin.invoke(STATUS, Bytes::new()).unwrap()).unwrap(),
			Capability::Unavailable
		);
		assert_eq!(
			Attestation::decode(
				&plugin
					.invoke(
						ATTEST,
						Challenge::new(b"x".as_slice()).unwrap().encode().unwrap()
					)
					.unwrap()
			)
			.unwrap(),
			Attestation::Unavailable
		);
	}
}
