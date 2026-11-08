//! <!-- {=p_diagnosticsSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_diagnostics` as a direct dependency and select the
//! `diagnostics` plugin in `native.plugins`; supported generated hosts discover
//! it from Cargo metadata. Generated Linux hosts register it where it has a
//! Linux Rust implementation; generated Windows and terminal hosts currently
//! omit runtime plugins, so use explicit registration in a custom host there.
//! Guests enable the `guest` feature and reuse `guest::Diagnostics` rather than
//! resolving the plugin on every call. Start and end named spans, and record
//! one-shot events, for startup and interaction timing. This MVP returns
//! Unavailable from every host until a diagnostics backend is chosen; wire
//! validation is complete so exporters can attach later.
//!
//! <!-- {/p_diagnosticsSummary} -->
//!
//! Wire v1: start takes a length-prefixed name → little-endian u32 span id; end
//! takes a span id → empty; event takes a length-prefixed name → empty.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

pub const NAME: &str = "diagnostics";
pub const START: MethodId = MethodId(0);
pub const END: MethodId = MethodId(1);
pub const EVENT: MethodId = MethodId(2);
pub const METHODS: &[MethodId] = &[START, END, EVENT];
pub const MAX_NAME_BYTES: usize = 128;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanId(pub u32);
impl SpanId {
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		self.0.to_le_bytes()
	}

	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let id = input.u32()?;
		input.finish()?;
		Ok(Self(id))
	}
}

pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		START | EVENT => Name::decode(payload).map(|_| ()),
		END => SpanId::decode(payload).map(|_| ()),
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
		Err(Error::Unavailable)
	}
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	#[derive(Debug, Clone, Copy)]
	pub struct Diagnostics(leptoskit_bridge::guest::Client);
	impl Diagnostics {
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		pub async fn start(self, name: &Name) -> Result<SpanId, Error> {
			SpanId::decode(&self.0.call(START, &name.encode()?)?.await?)
		}

		pub async fn end(self, id: SpanId) -> Result<(), Error> {
			Decoder::new(&self.0.call(END, &id.encode())?.await?).finish()
		}

		pub async fn event(self, name: &Name) -> Result<(), Error> {
			Decoder::new(&self.0.call(EVENT, &name.encode()?)?.await?).finish()
		}
	}
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;
	pub fn start(_name: &Name) -> Result<SpanId, Error> {
		Err(Error::Unavailable)
	}
	pub fn end(_id: SpanId) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
	pub fn event(_name: &Name) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	#[test]
	fn codecs_round_trip() {
		assert_eq!(Name::new(""), Err(Error::InvalidArgument));
		assert_eq!(Name::new("a\0b"), Err(Error::InvalidArgument));
		assert_eq!(
			Name::new("x".repeat(MAX_NAME_BYTES + 1)),
			Err(Error::InvalidArgument)
		);
		let name = Name::new("boot").unwrap();
		assert_eq!(name.as_str(), "boot");
		assert_eq!(Name::decode(&name.encode().unwrap()), Ok(name));
		let id = SpanId(9);
		assert_eq!(SpanId::decode(&id.encode()), Ok(id));
	}

	#[test]
	fn unsupported_validates_then_unavailable() {
		assert_eq!(
			validate(START, &Name::new("boot").unwrap().encode().unwrap()),
			Ok(())
		);
		assert_eq!(validate(END, &SpanId(1).encode()), Ok(()));
		assert_eq!(
			validate(EVENT, &Name::new("tap").unwrap().encode().unwrap()),
			Ok(())
		);
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));
		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			plugin.invoke(START, Name::new("boot").unwrap().encode().unwrap()),
			Err(Error::Unavailable)
		);
	}
}
