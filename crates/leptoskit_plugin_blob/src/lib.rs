//! <!-- {=p_blobSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_blob` as a direct dependency and select the `blob`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Linux hosts register it where it has a Linux Rust
//! implementation; generated Windows and terminal hosts currently omit runtime
//! plugins, so use explicit registration in a custom host there. Guests enable
//! the `guest` feature and reuse `guest::Blob` rather than resolving the plugin
//! on every call. Query availability, create an owned byte blob, read a bounded
//! slice, and release it without copying the whole payload through every
//! control message. This MVP keeps blobs in process memory on Apple, Android,
//! Linux, Windows, and the browser; durable shared-memory / object-URL backends
//! remain later scope.
//!
//! <!-- {/p_blobSummary} -->
//!
//! Wire v1: status is empty → four-byte [`Capability`]; create takes
//! length-prefixed bytes → little-endian u32 id; read takes id + offset +
//! length (u32) → length-prefixed slice (empty means EOF / past end); release
//! takes id → empty. Trailing bytes are rejected.

use std::collections::HashMap;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "blob";
/// Empty → [`Capability`].
pub const STATUS: MethodId = MethodId(0);
/// Bytes → [`BlobId`].
pub const CREATE: MethodId = MethodId(1);
/// [`Read`] → [`Slice`].
pub const READ: MethodId = MethodId(2);
/// [`BlobId`] → empty.
pub const RELEASE: MethodId = MethodId(3);
/// Supported operations.
pub const METHODS: &[MethodId] = &[STATUS, CREATE, READ, RELEASE];
/// Maximum create payload bytes.
pub const MAX_CREATE_BYTES: usize = 65_536;
/// Maximum read length.
pub const MAX_READ_BYTES: u32 = 8_192;
/// Maximum simultaneously live blobs in the process-memory store.
pub const MAX_LIVE_BLOBS: usize = 256;

/// Whether owned blob storage is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Capability {
	/// OS or host policy currently blocks blob ownership.
	Restricted = 0,
	/// Create / read / release are available.
	Available = 1,
	/// This host has no blob store.
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

/// Create request: non-empty bytes up to [`MAX_CREATE_BYTES`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Create {
	bytes: Bytes,
}

impl Create {
	/// Rejects empty and oversized payloads.
	pub fn new(bytes: impl Into<Bytes>) -> Result<Self, Error> {
		let bytes = bytes.into();
		if bytes.is_empty() || bytes.len() > MAX_CREATE_BYTES {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { bytes })
	}

	/// Owned payload bytes.
	#[must_use]
	pub fn bytes(&self) -> &[u8] {
		&self.bytes
	}

	/// Length-prefixed payload.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.bytes.len());
		output.bytes(&self.bytes)?;
		Ok(output.finish())
	}

	/// Decodes through [`Create::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let payload = Bytes::copy_from_slice(input.bytes()?);
		input.finish()?;
		Self::new(payload)
	}
}

/// Opaque process-local blob identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlobId(pub u32);

impl BlobId {
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

/// Read a bounded slice from a live blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Read {
	id: u32,
	offset: u32,
	length: u32,
}

impl Read {
	/// Rejects zero and oversized lengths.
	pub fn new(id: u32, offset: u32, length: u32) -> Result<Self, Error> {
		if length == 0 || length > MAX_READ_BYTES {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { id, offset, length })
	}

	/// Blob id.
	#[must_use]
	pub fn id(self) -> u32 {
		self.id
	}

	/// Byte offset into the blob.
	#[must_use]
	pub fn offset(self) -> u32 {
		self.offset
	}

	/// Requested length (hosts may return fewer bytes, including empty for
	/// EOF).
	#[must_use]
	pub fn length(self) -> u32 {
		self.length
	}

	/// Encodes id, offset, and length.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(12);
		output.u32(self.id);
		output.u32(self.offset);
		output.u32(self.length);
		output.finish()
	}

	/// Decodes through [`Read::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let id = input.u32()?;
		let offset = input.u32()?;
		let length = input.u32()?;
		input.finish()?;
		Self::new(id, offset, length)
	}
}

/// Read reply. Empty bytes mean EOF / offset past the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
	bytes: Bytes,
}

impl Slice {
	/// Rejects slices larger than [`MAX_READ_BYTES`]. Empty is allowed (EOF).
	pub fn new(bytes: impl Into<Bytes>) -> Result<Self, Error> {
		let bytes = bytes.into();
		if bytes.len() > MAX_READ_BYTES as usize {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { bytes })
	}

	/// Returned bytes (may be empty).
	#[must_use]
	pub fn bytes(&self) -> &[u8] {
		&self.bytes
	}

	/// Length-prefixed payload.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.bytes.len());
		output.bytes(&self.bytes)?;
		Ok(output.finish())
	}

	/// Decodes through [`Slice::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let payload = Bytes::copy_from_slice(input.bytes()?);
		input.finish()?;
		Self::new(payload)
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
		CREATE => Create::decode(payload).map(|_| ()),
		READ => Read::decode(payload).map(|_| ()),
		RELEASE => BlobId::decode(payload).map(|_| ()),
		_ => Err(Error::Unavailable),
	}
}

/// Process-memory blob store used by Rust hosts (Linux / Windows) and tests.
#[derive(Debug, Default)]
pub struct Blob {
	next_id: u32,
	entries: HashMap<u32, Bytes>,
}

impl Blob {
	/// Empty store; ids start at 1.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	fn allocate(&mut self, bytes: Bytes) -> Result<BlobId, Error> {
		if self.entries.len() >= MAX_LIVE_BLOBS {
			return Err(Error::Busy);
		}
		let id = self.next_id.checked_add(1).ok_or(Error::Exhausted)?;
		self.next_id = id;
		self.entries.insert(id, bytes);
		Ok(BlobId(id))
	}

	fn read_slice(&self, request: Read) -> Result<Slice, Error> {
		let entry = self
			.entries
			.get(&request.id())
			.ok_or(Error::InvalidArgument)?;
		let offset = request.offset() as usize;
		if offset >= entry.len() {
			return Slice::new(Bytes::new());
		}
		let end = offset
			.saturating_add(request.length() as usize)
			.min(entry.len());
		Slice::new(Bytes::copy_from_slice(&entry[offset..end]))
	}

	fn release_id(&mut self, id: BlobId) -> Result<(), Error> {
		if self.entries.remove(&id.0).is_none() {
			return Err(Error::InvalidArgument);
		}
		Ok(())
	}
}

impl leptoskit_bridge::Plugin for Blob {
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
			CREATE => {
				validate(method, &payload)?;
				let create = Create::decode(&payload)?;
				Ok(Bytes::copy_from_slice(
					&self
						.allocate(Bytes::copy_from_slice(create.bytes()))?
						.encode(),
				))
			}
			READ => {
				validate(method, &payload)?;
				self.read_slice(Read::decode(&payload)?)?.encode()
			}
			RELEASE => {
				validate(method, &payload)?;
				self.release_id(BlobId::decode(&payload)?)?;
				Ok(Bytes::new())
			}
			_ => Err(Error::Unavailable),
		}
	}
}

/// Explicit unavailable handler for hosts that intentionally omit blob storage.
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
	pub struct Blob(leptoskit_bridge::guest::Client);

	impl Blob {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries whether owned blob storage is available.
		pub async fn status(self) -> Result<Capability, Error> {
			Capability::decode(&self.0.call(STATUS, &[])?.await?)
		}

		/// Creates an owned blob and returns its id.
		pub async fn create(self, request: &Create) -> Result<BlobId, Error> {
			BlobId::decode(&self.0.call(CREATE, &request.encode()?)?.await?)
		}

		/// Reads a bounded slice; empty means EOF / past end.
		pub async fn read(self, request: Read) -> Result<Slice, Error> {
			Slice::decode(&self.0.call(READ, &request.encode())?.await?)
		}

		/// Releases a live blob id.
		pub async fn release(self, id: BlobId) -> Result<(), Error> {
			let bytes = self.0.call(RELEASE, &id.encode())?.await?;
			Decoder::new(&bytes).finish()
		}
	}
}

/// Browser Direct API: process-local memory store (no `blob:` object URLs yet).
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use std::cell::RefCell;

	use super::*;

	thread_local! {
		static STORE: RefCell<Blob> = RefCell::new(Blob::new());
	}

	/// Always Available in this MVP.
	#[must_use]
	pub const fn status() -> Capability {
		Capability::Available
	}

	/// Creates an owned blob in page memory.
	pub fn create(request: &Create) -> Result<BlobId, Error> {
		STORE.with(|store| {
			store
				.borrow_mut()
				.allocate(Bytes::copy_from_slice(request.bytes()))
		})
	}

	/// Reads a bounded slice; empty means EOF / past end.
	pub fn read(request: Read) -> Result<Slice, Error> {
		STORE.with(|store| store.borrow().read_slice(request))
	}

	/// Releases a live blob id.
	pub fn release(id: BlobId) -> Result<(), Error> {
		STORE.with(|store| store.borrow_mut().release_id(id))
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

		assert_eq!(Create::new(&[][..]), Err(Error::InvalidArgument));
		assert_eq!(
			Create::new(vec![0; MAX_CREATE_BYTES + 1]),
			Err(Error::InvalidArgument)
		);
		let create = Create::new(b"hi".as_slice()).unwrap();
		assert_eq!(create.bytes(), b"hi");
		assert_eq!(Create::decode(&create.encode().unwrap()), Ok(create));
		assert_eq!(
			Create::decode(&[2, 0, 0, 0, b'h', b'i', 0]),
			Err(Error::InvalidArgument)
		);

		let id = BlobId(7);
		assert_eq!(BlobId::decode(&id.encode()), Ok(id));
		assert_eq!(
			BlobId::decode(&[1, 0, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);

		assert_eq!(Read::new(1, 0, 0), Err(Error::InvalidArgument));
		assert_eq!(
			Read::new(1, 0, MAX_READ_BYTES + 1),
			Err(Error::InvalidArgument)
		);
		let read = Read::new(1, 2, 3).unwrap();
		assert_eq!((read.id(), read.offset(), read.length()), (1, 2, 3));
		assert_eq!(Read::decode(&read.encode()), Ok(read));
		assert_eq!(
			Read::decode(&[1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);

		let empty = Slice::new(Bytes::new()).unwrap();
		assert_eq!(empty.bytes(), b"");
		assert_eq!(Slice::decode(&empty.encode().unwrap()), Ok(empty));
		let slice = Slice::new(b"ab".as_slice()).unwrap();
		assert_eq!(slice.bytes(), b"ab");
		assert_eq!(Slice::decode(&slice.encode().unwrap()), Ok(slice));
		assert_eq!(
			Slice::new(vec![0; MAX_READ_BYTES as usize + 1]),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn memory_store_create_read_release() {
		let mut plugin = Blob::new();
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
		let id = BlobId::decode(
			&plugin
				.invoke(
					CREATE,
					Create::new(b"abcdef".as_slice()).unwrap().encode().unwrap(),
				)
				.unwrap(),
		)
		.unwrap();
		assert_eq!(id, BlobId(1));
		let mid = Slice::decode(
			&plugin
				.invoke(READ, Read::new(id.0, 2, 3).unwrap().encode())
				.unwrap(),
		)
		.unwrap();
		assert_eq!(mid.bytes(), b"cde");
		let eof = Slice::decode(
			&plugin
				.invoke(READ, Read::new(id.0, 100, 1).unwrap().encode())
				.unwrap(),
		)
		.unwrap();
		assert_eq!(eof.bytes(), b"");
		let short = Slice::decode(
			&plugin
				.invoke(READ, Read::new(id.0, 4, 8).unwrap().encode())
				.unwrap(),
		)
		.unwrap();
		assert_eq!(short.bytes(), b"ef");
		assert_eq!(
			plugin.invoke(RELEASE, Bytes::copy_from_slice(&id.encode())),
			Ok(Bytes::new())
		);
		assert_eq!(
			plugin.invoke(READ, Read::new(id.0, 0, 1).unwrap().encode()),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			plugin.invoke(RELEASE, Bytes::copy_from_slice(&id.encode())),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn memory_store_live_cap_and_validate() {
		let mut plugin = Blob::new();
		for _ in 0..MAX_LIVE_BLOBS {
			plugin
				.invoke(
					CREATE,
					Create::new(b"x".as_slice()).unwrap().encode().unwrap(),
				)
				.unwrap();
		}
		assert_eq!(
			plugin.invoke(
				CREATE,
				Create::new(b"x".as_slice()).unwrap().encode().unwrap()
			),
			Err(Error::Busy)
		);
		assert_eq!(validate(STATUS, &[]), Ok(()));
		assert_eq!(validate(STATUS, &[0]), Err(Error::InvalidArgument));
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
		assert_eq!(METHODS.len(), 4);
		assert_eq!(NAME, "blob");
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
			plugin.invoke(
				CREATE,
				Create::new(b"x".as_slice()).unwrap().encode().unwrap()
			),
			Err(Error::Unavailable)
		);
	}
}
