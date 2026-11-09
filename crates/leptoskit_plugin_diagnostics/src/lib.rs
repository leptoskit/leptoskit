//! <!-- {=p_diagnosticsSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_diagnostics` as a direct dependency and select the
//! `diagnostics` plugin in `native.plugins`; supported generated hosts discover
//! it from Cargo metadata. Generated Linux hosts register it where it has a
//! Linux Rust implementation; generated Windows and terminal hosts currently
//! omit runtime plugins, so use explicit registration in a custom host there.
//! Guests enable the `guest` feature and reuse `guest::Diagnostics` rather than
//! resolving the plugin on every call. Query availability, start and end named
//! spans, and record one-shot events for startup and interaction timing. This
//! MVP is an in-process no-op allocator on Apple, Android, Linux, Windows, and
//! the browser — spans succeed without choosing a hosted exporter;
//! OpenTelemetry / OS logging backends remain later scope.
//!
//! <!-- {/p_diagnosticsSummary} -->
//!
//! Wire v1: status is empty → four-byte [`Capability`]; start takes a
//! length-prefixed name → little-endian u32 span id; end takes a span id →
//! empty; event takes a length-prefixed name → empty. Trailing bytes are
//! rejected.

use std::collections::HashSet;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "diagnostics";
/// Empty → [`Capability`].
pub const STATUS: MethodId = MethodId(0);
/// [`Name`] → [`SpanId`].
pub const START: MethodId = MethodId(1);
/// [`SpanId`] → empty.
pub const END: MethodId = MethodId(2);
/// [`Name`] → empty.
pub const EVENT: MethodId = MethodId(3);
/// Supported operations.
pub const METHODS: &[MethodId] = &[STATUS, START, END, EVENT];
/// Maximum UTF-8 bytes in a span or event name.
pub const MAX_NAME_BYTES: usize = 128;
/// Maximum simultaneously open spans in the no-op allocator.
pub const MAX_OPEN_SPANS: usize = 256;

/// Whether diagnostic spans are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Capability {
	/// OS or host policy currently blocks diagnostics.
	Restricted = 0,
	/// Start / end / event are available.
	Available = 1,
	/// This host has no diagnostics sink.
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

/// Span or event name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
	name: String,
}

impl Name {
	/// Rejects empty, NUL, and oversized names.
	pub fn new(name: impl Into<String>) -> Result<Self, Error> {
		let name = name.into();
		if name.is_empty() || name.contains('\0') || name.len() > MAX_NAME_BYTES {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { name })
	}

	/// UTF-8 name.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.name
	}

	/// Length-prefixed name bytes.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.name.len());
		output.bytes(self.name.as_bytes())?;
		Ok(output.finish())
	}

	/// Decodes through [`Name::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let name = input.string()?.to_owned();
		input.finish()?;
		Self::new(name)
	}
}

/// Opaque process-local span identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpanId(pub u32);

impl SpanId {
	/// Fixed four-byte id.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		self.0.to_le_bytes()
	}

	/// Decodes one word, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		Ok(Self(word(bytes)?))
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
		START | EVENT => Name::decode(payload).map(|_| ()),
		END => SpanId::decode(payload).map(|_| ()),
		_ => Err(Error::Unavailable),
	}
}

/// In-process no-op span allocator used by Rust hosts and tests.
#[derive(Debug, Default)]
pub struct Diagnostics {
	next_id: u32,
	open: HashSet<u32>,
}

impl Diagnostics {
	/// Empty allocator; ids start at 1.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	fn start_span(&mut self, _name: &Name) -> Result<SpanId, Error> {
		if self.open.len() >= MAX_OPEN_SPANS {
			return Err(Error::Busy);
		}
		let id = self.next_id.checked_add(1).ok_or(Error::Exhausted)?;
		self.next_id = id;
		self.open.insert(id);
		Ok(SpanId(id))
	}

	fn end_span(&mut self, id: SpanId) -> Result<(), Error> {
		if !self.open.remove(&id.0) {
			return Err(Error::InvalidArgument);
		}
		Ok(())
	}
}

impl leptoskit_bridge::Plugin for Diagnostics {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		match method {
			STATUS => {
				validate(method, &payload)?;
				Ok(Bytes::copy_from_slice(&Capability::Available.encode()))
			}
			START => {
				validate(method, &payload)?;
				let name = Name::decode(&payload)?;
				Ok(Bytes::copy_from_slice(&self.start_span(&name)?.encode()))
			}
			END => {
				validate(method, &payload)?;
				self.end_span(SpanId::decode(&payload)?)?;
				Ok(Bytes::new())
			}
			EVENT => {
				validate(method, &payload)?;
				Name::decode(&payload)?;
				Ok(Bytes::new())
			}
			_ => Err(Error::Unavailable),
		}
	}
}

/// Explicit unavailable handler for hosts that intentionally omit diagnostics.
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
			Err(Error::Unavailable)
		}
	}
}

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct Diagnostics(leptoskit_bridge::guest::Client);

	impl Diagnostics {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries whether spans are available.
		pub async fn status(self) -> Result<Capability, Error> {
			Capability::decode(&self.0.call(STATUS, &[])?.await?)
		}

		/// Starts a named span and returns its id.
		pub async fn start(self, name: &Name) -> Result<SpanId, Error> {
			SpanId::decode(&self.0.call(START, &name.encode()?)?.await?)
		}

		/// Ends an open span.
		pub async fn end(self, id: SpanId) -> Result<(), Error> {
			Decoder::new(&self.0.call(END, &id.encode())?.await?).finish()
		}

		/// Records a one-shot named event.
		pub async fn event(self, name: &Name) -> Result<(), Error> {
			Decoder::new(&self.0.call(EVENT, &name.encode()?)?.await?).finish()
		}
	}
}

/// Browser Direct API: process-local no-op allocator.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use std::cell::RefCell;

	use super::*;

	thread_local! {
		static STORE: RefCell<Diagnostics> = RefCell::new(Diagnostics::new());
	}

	/// Always Available in this MVP.
	#[must_use]
	pub const fn status() -> Capability {
		Capability::Available
	}

	/// Starts a named span in page memory.
	pub fn start(name: &Name) -> Result<SpanId, Error> {
		STORE.with(|store| store.borrow_mut().start_span(name))
	}

	/// Ends an open span.
	pub fn end(id: SpanId) -> Result<(), Error> {
		STORE.with(|store| store.borrow_mut().end_span(id))
	}

	/// Records a one-shot named event (no-op after validation).
	pub fn event(name: &Name) -> Result<(), Error> {
		Name::new(name.as_str()).map(|_| ())
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

		assert_eq!(Name::new(""), Err(Error::InvalidArgument));
		assert_eq!(Name::new("a\0b"), Err(Error::InvalidArgument));
		assert_eq!(
			Name::new("x".repeat(MAX_NAME_BYTES + 1)),
			Err(Error::InvalidArgument)
		);
		let name = Name::new("boot").unwrap();
		assert_eq!(name.as_str(), "boot");
		assert_eq!(Name::decode(&name.encode().unwrap()), Ok(name));
		assert_eq!(
			Name::decode(&[4, 0, 0, 0, b'b', b'o', b'o', b't', 0]),
			Err(Error::InvalidArgument)
		);

		let id = SpanId(9);
		assert_eq!(SpanId::decode(&id.encode()), Ok(id));
		assert_eq!(
			SpanId::decode(&[1, 0, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn no_op_allocator_start_end_event() {
		let mut plugin = Diagnostics::new();
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			plugin.invoke(MethodId(9), Bytes::new()),
			Err(Error::Unavailable)
		);
		assert_eq!(
			Capability::decode(&plugin.invoke(STATUS, Bytes::new()).unwrap()).unwrap(),
			Capability::Available
		);
		let id = SpanId::decode(
			&plugin
				.invoke(START, Name::new("boot").unwrap().encode().unwrap())
				.unwrap(),
		)
		.unwrap();
		assert_eq!(id, SpanId(1));
		assert_eq!(
			plugin.invoke(EVENT, Name::new("tap").unwrap().encode().unwrap()),
			Ok(Bytes::new())
		);
		assert_eq!(
			plugin.invoke(END, Bytes::copy_from_slice(&id.encode())),
			Ok(Bytes::new())
		);
		assert_eq!(
			plugin.invoke(END, Bytes::copy_from_slice(&id.encode())),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn no_op_open_cap_and_validate() {
		let mut plugin = Diagnostics::new();
		for _ in 0..MAX_OPEN_SPANS {
			plugin
				.invoke(START, Name::new("s").unwrap().encode().unwrap())
				.unwrap();
		}
		assert_eq!(
			plugin.invoke(START, Name::new("s").unwrap().encode().unwrap()),
			Err(Error::Busy)
		);
		assert_eq!(validate(STATUS, &[]), Ok(()));
		assert_eq!(validate(STATUS, &[0]), Err(Error::InvalidArgument));
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
		assert_eq!(METHODS.len(), 4);
		assert_eq!(NAME, "diagnostics");
	}

	#[test]
	fn unsupported_status_is_unavailable() {
		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			Capability::decode(&plugin.invoke(STATUS, Bytes::new()).unwrap()).unwrap(),
			Capability::Unavailable
		);
		assert_eq!(
			plugin.invoke(START, Name::new("boot").unwrap().encode().unwrap()),
			Err(Error::Unavailable)
		);
	}
}
