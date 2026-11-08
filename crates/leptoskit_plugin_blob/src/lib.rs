//! <!-- {=p_blobSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_blob` as a direct dependency and select the `blob`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Linux hosts register it where it has a Linux Rust
//! implementation; generated Windows and terminal hosts currently omit runtime
//! plugins, so use explicit registration in a custom host there. Guests enable
//! the `guest` feature and reuse `guest::Blob` rather than resolving the plugin
//! on every call. Create an owned byte blob, read a bounded slice, and release
//! it without copying the whole payload through every control message. This MVP
//! returns Unavailable from every host until a durable resource protocol lands;
//! codecs and validation are complete so later hosts can fill in storage.
//!
//! <!-- {/p_blobSummary} -->
//!
//! Wire v1: create takes length-prefixed bytes → little-endian u32 id; read
//! takes id + offset + length (u32) → length-prefixed slice; release takes id →
//! empty.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

pub const NAME: &str = "blob";
pub const CREATE: MethodId = MethodId(0);
pub const READ: MethodId = MethodId(1);
pub const RELEASE: MethodId = MethodId(2);
pub const METHODS: &[MethodId] = &[CREATE, READ, RELEASE];
pub const MAX_CREATE_BYTES: usize = 65_536;
pub const MAX_READ_BYTES: u32 = 8_192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Create {
	bytes: Bytes,
}
impl Create {
	pub fn new(bytes: impl Into<Bytes>) -> Result<Self, Error> {
		let bytes = bytes.into();
		if bytes.is_empty() || bytes.len() > MAX_CREATE_BYTES {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { bytes })
	}

	#[must_use]
	pub fn bytes(&self) -> &[u8] {
		&self.bytes
	}

	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.bytes.len());
		output.bytes(&self.bytes)?;
		Ok(output.finish())
	}

	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let payload = Bytes::copy_from_slice(input.bytes()?);
		input.finish()?;
		Self::new(payload)
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobId(pub u32);
impl BlobId {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Read {
	id: u32,
	offset: u32,
	length: u32,
}
impl Read {
	pub fn new(id: u32, offset: u32, length: u32) -> Result<Self, Error> {
		if length == 0 || length > MAX_READ_BYTES {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { id, offset, length })
	}

	#[must_use]
	pub fn id(self) -> u32 {
		self.id
	}

	#[must_use]
	pub fn offset(self) -> u32 {
		self.offset
	}

	#[must_use]
	pub fn length(self) -> u32 {
		self.length
	}

	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(12);
		output.u32(self.id);
		output.u32(self.offset);
		output.u32(self.length);
		output.finish()
	}

	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let id = input.u32()?;
		let offset = input.u32()?;
		let length = input.u32()?;
		input.finish()?;
		Self::new(id, offset, length)
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
	bytes: Bytes,
}
impl Slice {
	pub fn new(bytes: impl Into<Bytes>) -> Result<Self, Error> {
		let bytes = bytes.into();
		if bytes.len() > MAX_READ_BYTES as usize {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { bytes })
	}

	#[must_use]
	pub fn bytes(&self) -> &[u8] {
		&self.bytes
	}

	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.bytes.len());
		output.bytes(&self.bytes)?;
		Ok(output.finish())
	}

	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let payload = Bytes::copy_from_slice(input.bytes()?);
		input.finish()?;
		Self::new(payload)
	}
}

pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		CREATE => Create::decode(payload).map(|_| ()),
		READ => Read::decode(payload).map(|_| ()),
		RELEASE => BlobId::decode(payload).map(|_| ()),
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
	pub struct Blob(leptoskit_bridge::guest::Client);
	impl Blob {
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		pub async fn create(self, request: &Create) -> Result<BlobId, Error> {
			BlobId::decode(&self.0.call(CREATE, &request.encode()?)?.await?)
		}

		pub async fn read(self, request: Read) -> Result<Slice, Error> {
			Slice::decode(&self.0.call(READ, &request.encode())?.await?)
		}

		pub async fn release(self, id: BlobId) -> Result<(), Error> {
			let bytes = self.0.call(RELEASE, &id.encode())?.await?;
			Decoder::new(&bytes).finish()
		}
	}
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;
	pub fn create(_request: &Create) -> Result<BlobId, Error> {
		Err(Error::Unavailable)
	}
	pub fn read(_request: Read) -> Result<Slice, Error> {
		Err(Error::Unavailable)
	}
	pub fn release(_id: BlobId) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	#[test]
	fn codecs_round_trip() {
		assert_eq!(Create::new(&[][..]), Err(Error::InvalidArgument));
		assert_eq!(
			Create::new(vec![0; MAX_CREATE_BYTES + 1]),
			Err(Error::InvalidArgument)
		);
		let create = Create::new(b"hi".as_slice()).unwrap();
		assert_eq!(create.bytes(), b"hi");
		assert_eq!(Create::decode(&create.encode().unwrap()), Ok(create));
		let id = BlobId(7);
		assert_eq!(BlobId::decode(&id.encode()), Ok(id));
		assert_eq!(Read::new(1, 0, 0), Err(Error::InvalidArgument));
		assert_eq!(
			Read::new(1, 0, MAX_READ_BYTES + 1),
			Err(Error::InvalidArgument)
		);
		let read = Read::new(1, 2, 3).unwrap();
		assert_eq!((read.id(), read.offset(), read.length()), (1, 2, 3));
		assert_eq!(Read::decode(&read.encode()), Ok(read));
		let slice = Slice::new(b"ab".as_slice()).unwrap();
		assert_eq!(slice.bytes(), b"ab");
		assert_eq!(Slice::decode(&slice.encode().unwrap()), Ok(slice));
		assert_eq!(
			Slice::new(vec![0; MAX_READ_BYTES as usize + 1]),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn unsupported_validates_then_unavailable() {
		assert_eq!(
			validate(
				CREATE,
				&Create::new(b"x".as_slice()).unwrap().encode().unwrap()
			),
			Ok(())
		);
		assert_eq!(
			validate(READ, &Read::new(1, 0, 1).unwrap().encode()),
			Ok(())
		);
		assert_eq!(validate(RELEASE, &BlobId(1).encode()), Ok(()));
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));
		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			plugin.invoke(
				CREATE,
				Create::new(b"x".as_slice()).unwrap().encode().unwrap()
			),
			Err(Error::Unavailable)
		);
	}
}
