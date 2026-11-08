//! <!-- {=p_age_rangeSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_age_range` as a direct dependency and select the
//! `age-range` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Generated Linux hosts register it where it
//! has a Linux Rust implementation; generated Windows and terminal hosts
//! currently omit runtime plugins, so use explicit registration in a custom
//! host there. Guests enable the `guest` feature and reuse
//! `guest::AgeRange` rather than resolving the plugin on every call.
//! Query whether the host can ask, then request a declared age band around a
//! threshold. Responses are Declined, Unavailable, or Shared with inclusive
//! lower/upper bounds (`upper == 0` means open-ended). iOS uses Declared Age
//! Range where the OS exposes it; Android, desktop, and the browser return
//! `Unavailable` in this MVP. Never invents an age band when the OS declines
//! or withholds one.
//!
//! <!-- {/p_age_rangeSummary} -->
//!
//! Wire v1: status is empty → four-byte [`Capability`]; request takes one
//! little-endian threshold word and returns a [`Response`]. Trailing bytes are
//! rejected.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "age-range";
/// Empty request → [`Capability`].
pub const STATUS: MethodId = MethodId(0);
/// Threshold seconds-as-years word → [`Response`].
pub const REQUEST: MethodId = MethodId(1);
/// Supported wire operations.
pub const METHODS: &[MethodId] = &[STATUS, REQUEST];

/// Minimum allowed age threshold (years).
pub const MIN_THRESHOLD: u32 = 1;
/// Maximum allowed age threshold (years).
pub const MAX_THRESHOLD: u32 = 120;

/// Whether the host can present an age-range prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Capability {
	/// OS policy currently blocks asking.
	Restricted = 0,
	/// A request may be presented; the OS still chooses the outcome.
	Available = 1,
	/// This host has no declared-age API.
	Unavailable = 2,
}

impl Capability {
	/// Rejects unknown values and non-exact response lengths.
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

/// Outcome of an age-range request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
	/// The user declined to share an age band.
	Declined,
	/// The platform cannot share a band for this request.
	Unavailable,
	/// Inclusive lower bound; `upper == 0` means open-ended.
	Shared { lower: u32, upper: u32 },
}

impl Response {
	/// Encodes kind, then lower/upper when shared.
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(12);
		match self {
			Self::Declined => output.u32(0),
			Self::Unavailable => output.u32(1),
			Self::Shared { lower, upper } => {
				output.u32(2);
				output.u32(lower);
				output.u32(upper);
			}
		}
		output.finish()
	}

	/// Decodes through the fixed kind tags, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let kind = input.u32()?;
		let response = match kind {
			0 => Self::Declined,
			1 => Self::Unavailable,
			2 => {
				let lower = input.u32()?;
				let upper = input.u32()?;
				if upper != 0 && upper < lower {
					return Err(Error::InvalidArgument);
				}
				Self::Shared { lower, upper }
			}
			_ => return Err(Error::InvalidArgument),
		};
		input.finish()?;
		Ok(response)
	}
}

/// Request a declared age band around `threshold` years.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
	threshold: u32,
}

impl Request {
	/// Rejects out-of-range thresholds.
	pub fn new(threshold: u32) -> Result<Self, Error> {
		if !(MIN_THRESHOLD..=MAX_THRESHOLD).contains(&threshold) {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { threshold })
	}

	/// Threshold in years.
	#[must_use]
	pub fn threshold(self) -> u32 {
		self.threshold
	}

	/// Encodes the threshold word.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		self.threshold.to_le_bytes()
	}

	/// Decodes through [`Request::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		Self::new(word(bytes)?)
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
		REQUEST => Request::decode(payload).map(|_| ()),
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without a declared-age API.
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
			Ok(Response::Unavailable.encode())
		}
	}
}

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct AgeRange(leptoskit_bridge::guest::Client);

	impl AgeRange {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries whether a request can be presented.
		pub async fn status(self) -> Result<Capability, Error> {
			Capability::decode(&self.0.call(STATUS, &[])?.await?)
		}

		/// Requests a declared age band around `request.threshold()`.
		pub async fn request(self, request: Request) -> Result<Response, Error> {
			Response::decode(&self.0.call(REQUEST, &request.encode())?.await?)
		}
	}
}

/// Browsers have no declared-age API in this MVP.
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
	pub fn request(_request: Request) -> Response {
		Response::Unavailable
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	#[test]
	fn codecs_reject_bounds_and_round_trip() {
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

		assert_eq!(Request::new(0), Err(Error::InvalidArgument));
		assert_eq!(Request::new(121), Err(Error::InvalidArgument));
		let request = Request::new(18).unwrap();
		assert_eq!(request.threshold(), 18);
		assert_eq!(Request::decode(&request.encode()), Ok(request));

		for response in [
			Response::Declined,
			Response::Unavailable,
			Response::Shared {
				lower: 13,
				upper: 15,
			},
			Response::Shared {
				lower: 18,
				upper: 0,
			},
		] {
			assert_eq!(Response::decode(&response.encode()), Ok(response));
		}
		assert_eq!(
			Response::decode(&{
				let mut bad = Encoder::with_capacity(12);
				bad.u32(2);
				bad.u32(20);
				bad.u32(10);
				bad.finish()
			}),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Response::decode(&u32::to_le_bytes(3)),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn validate_and_unsupported_keep_outcomes_honest() {
		assert_eq!(validate(STATUS, &[]), Ok(()));
		assert_eq!(validate(STATUS, &[0]), Err(Error::InvalidArgument));
		assert_eq!(
			validate(REQUEST, &Request::new(18).unwrap().encode()),
			Ok(())
		);
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));
		assert_eq!(METHODS.len(), 2);
		assert_eq!(NAME, "age-range");

		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			Capability::decode(&plugin.invoke(STATUS, Bytes::new()).unwrap()).unwrap(),
			Capability::Unavailable
		);
		assert_eq!(
			Response::decode(
				&plugin
					.invoke(
						REQUEST,
						Bytes::copy_from_slice(&Request::new(18).unwrap().encode())
					)
					.unwrap()
			)
			.unwrap(),
			Response::Unavailable
		);
	}
}
