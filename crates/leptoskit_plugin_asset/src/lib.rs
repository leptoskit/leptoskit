//! <!-- {=p_assetSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_asset` as a direct dependency and select the `asset`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Linux hosts register it where it has a Linux Rust
//! implementation; generated Windows and terminal hosts currently omit runtime
//! plugins, so use explicit registration in a custom host there. Guests enable
//! the `guest` feature and reuse `guest::Asset` rather than resolving the
//! plugin on every call. Query whether packaged-asset resolution is available,
//! then resolve a logical asset name to a local URI. This MVP returns
//! Unavailable on every host until packaged-asset ownership wiring lands. Never
//! invents a file URI for a missing asset.
//!
//! <!-- {/p_assetSummary} -->
//!
//! Wire v1: status is empty → four-byte [`Capability`]; resolve takes a
//! length-prefixed name and returns [`Resolved`].

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "asset";
/// Empty → [`Capability`].
pub const STATUS: MethodId = MethodId(0);
/// Name → [`Resolved`].
pub const RESOLVE: MethodId = MethodId(1);
/// Supported operations.
pub const METHODS: &[MethodId] = &[STATUS, RESOLVE];
/// Maximum UTF-8 bytes in an asset name.
pub const MAX_NAME_BYTES: usize = 256;
/// Maximum UTF-8 bytes in a resolved URI.
pub const MAX_URI_BYTES: usize = 1024;

/// Whether packaged-asset resolution is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Capability {
	Restricted = 0,
	Available = 1,
	Unavailable = 2,
}

impl Capability {
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match word(bytes)? {
			0 => Ok(Self::Restricted),
			1 => Ok(Self::Available),
			2 => Ok(Self::Unavailable),
			_ => Err(Error::InvalidArgument),
		}
	}

	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}
}

/// Logical packaged-asset name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
	name: String,
}

impl Name {
	pub fn new(name: impl Into<String>) -> Result<Self, Error> {
		let name = name.into();
		if name.is_empty() || name.contains('\0') || name.len() > MAX_NAME_BYTES {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { name })
	}

	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.name
	}

	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.name.len());
		output.bytes(self.name.as_bytes())?;
		Ok(output.finish())
	}

	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let name = input.string()?.to_owned();
		input.finish()?;
		Self::new(name)
	}
}

/// Resolve outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
	Unavailable,
	Uri(String),
}

impl Resolved {
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(8);
		match self {
			Self::Unavailable => output.u32(0),
			Self::Uri(uri) => {
				if uri.is_empty() || uri.contains('\0') || uri.len() > MAX_URI_BYTES {
					return Err(Error::InvalidArgument);
				}
				output.u32(1);
				output.bytes(uri.as_bytes())?;
			}
		}
		Ok(output.finish())
	}

	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let kind = input.u32()?;
		let value = match kind {
			0 => Self::Unavailable,
			1 => {
				let uri = input.string()?.to_owned();
				if uri.is_empty() || uri.contains('\0') || uri.len() > MAX_URI_BYTES {
					return Err(Error::InvalidArgument);
				}
				Self::Uri(uri)
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

pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		STATUS => Decoder::new(payload).finish(),
		RESOLVE => Name::decode(payload).map(|_| ()),
		_ => Err(Error::Unavailable),
	}
}

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
			Resolved::Unavailable.encode()
		}
	}
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	#[derive(Debug, Clone, Copy)]
	pub struct Asset(leptoskit_bridge::guest::Client);
	impl Asset {
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		pub async fn status(self) -> Result<Capability, Error> {
			Capability::decode(&self.0.call(STATUS, &[])?.await?)
		}

		pub async fn resolve(self, name: &Name) -> Result<Resolved, Error> {
			Resolved::decode(&self.0.call(RESOLVE, &name.encode()?)?.await?)
		}
	}
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;
	#[must_use]
	pub const fn status() -> Capability {
		Capability::Unavailable
	}
	#[must_use]
	pub fn resolve(_name: &Name) -> Resolved {
		Resolved::Unavailable
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	#[test]
	fn codecs_and_bounds() {
		for (value, expected) in [
			(0, Capability::Restricted),
			(1, Capability::Available),
			(2, Capability::Unavailable),
		] {
			assert_eq!(Capability::decode(&u32::to_le_bytes(value)), Ok(expected));
		}
		assert_eq!(
			Capability::decode(&[3, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(Name::new(""), Err(Error::InvalidArgument));
		assert_eq!(Name::new("a\0b"), Err(Error::InvalidArgument));
		assert_eq!(
			Name::new("x".repeat(MAX_NAME_BYTES + 1)),
			Err(Error::InvalidArgument)
		);
		let name = Name::new("icon.png").unwrap();
		assert_eq!(name.as_str(), "icon.png");
		assert_eq!(Name::decode(&name.encode().unwrap()), Ok(name));
		assert_eq!(
			Resolved::decode(&Resolved::Unavailable.encode().unwrap()),
			Ok(Resolved::Unavailable)
		);
		let uri = Resolved::Uri("file:///tmp/icon.png".into());
		assert_eq!(Resolved::decode(&uri.encode().unwrap()), Ok(uri));
		assert_eq!(
			Resolved::Uri(String::new()).encode(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Resolved::decode(&u32::to_le_bytes(3)),
			Err(Error::InvalidArgument)
		);
		let mut empty_uri = Encoder::with_capacity(8);
		empty_uri.u32(1);
		empty_uri.bytes(&[]).unwrap();
		assert_eq!(
			Resolved::decode(&empty_uri.finish()),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn unsupported_honest() {
		assert_eq!(validate(STATUS, &[]), Ok(()));
		assert_eq!(
			validate(RESOLVE, &Name::new("a").unwrap().encode().unwrap()),
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
			Resolved::decode(
				&plugin
					.invoke(RESOLVE, Name::new("a").unwrap().encode().unwrap())
					.unwrap()
			)
			.unwrap(),
			Resolved::Unavailable
		);
	}
}
