//! Bidirectional phone ↔ watch companion sync over `WatchConnectivity`, Wear
//! OS Data Layer, and Huawei Wear Engine.
//!
//! Add `leptoskit_plugin_watch_sync` as a direct dependency and select
//! `watch-sync` in `native.plugins`. Guests enable the `guest` feature and
//! reuse `guest::WatchSync`.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Automatic native registration name.
pub const NAME: &str = "watch-sync";
/// Query current companion pairing and reachability status, empty request.
pub const STATUS: MethodId = MethodId(0);
/// Send an immediate message to the reachable peer ([`Message`]).
pub const SEND_MESSAGE: MethodId = MethodId(1);
/// Queue a durable background transfer ([`Transfer`]) surviving suspension.
pub const QUEUE_TRANSFER: MethodId = MethodId(2);
/// Wait for and return the next [`IncomingEnvelope`] from the paired peer.
pub const NEXT_INCOMING: MethodId = MethodId(3);
/// Acknowledge receipt of an incoming transfer id (UTF-8 string).
pub const ACK: MethodId = MethodId(4);
/// Wait until companion [`SyncStatus`] changes from the supplied snapshot.
pub const WATCH_STATUS: MethodId = MethodId(5);
/// Complete native method table.
pub const METHODS: &[MethodId] = &[
	STATUS,
	SEND_MESSAGE,
	QUEUE_TRANSFER,
	NEXT_INCOMING,
	ACK,
	WATCH_STATUS,
];

/// Maximum payload for an immediate message (64 KiB).
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// Maximum payload for a durable transfer (256 KiB).
pub const MAX_TRANSFER_BYTES: usize = 256 * 1024;
const ABSENT: u32 = u32::MAX;

/// Which platform companion transport backs the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum SyncTransport {
	/// No companion transport on this device.
	None = 0,
	/// Apple `WatchConnectivity` (`WCSession`).
	WatchConnectivity = 1,
	/// Android / Wear OS `Wearable` Data & Message Layer.
	WearDataLayer = 2,
	/// Huawei HarmonyOS Wear Engine.
	HuaweiWearEngine = 3,
}

impl SyncTransport {
	fn from_code(code: u32) -> Result<Self, Error> {
		match code {
			0 => Ok(Self::None),
			1 => Ok(Self::WatchConnectivity),
			2 => Ok(Self::WearDataLayer),
			3 => Ok(Self::HuaweiWearEngine),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Companion link status between watch and phone.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct SyncStatus {
	/// Which transport is active.
	pub transport: SyncTransport,
	/// Whether the hardware/OS supports companion sync.
	pub supported: bool,
	/// Whether a peer watch/phone is paired.
	pub paired: bool,
	/// Whether the peer is currently reachable for immediate messages.
	pub reachable: bool,
	/// Whether the companion app is installed on the peer.
	pub companion_installed: bool,
	/// Display name of the paired peer, when known.
	pub peer_name: Option<String>,
}

const FLAG_SUPPORTED: u32 = 1 << 0;
const FLAG_PAIRED: u32 = 1 << 1;
const FLAG_REACHABLE: u32 = 1 << 2;
const FLAG_INSTALLED: u32 = 1 << 3;
const ALL_FLAGS: u32 = FLAG_SUPPORTED | FLAG_PAIRED | FLAG_REACHABLE | FLAG_INSTALLED;

impl SyncStatus {
	/// Encodes status to wire bytes.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		let mut flags = 0_u32;
		if self.supported {
			flags |= FLAG_SUPPORTED;
		}
		if self.paired {
			flags |= FLAG_PAIRED;
		}
		if self.reachable {
			flags |= FLAG_REACHABLE;
		}
		if self.companion_installed {
			flags |= FLAG_INSTALLED;
		}
		let mut w = Writer::default();
		w.u32(self.transport as u32)
			.u32(flags)
			.opt_str(self.peer_name.as_deref())?;
		Ok(w.finish())
	}

	/// Decodes status from wire bytes.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut r = Reader::new(payload);
		let transport = SyncTransport::from_code(r.u32()?)?;
		let flags = r.u32()?;
		if flags & !ALL_FLAGS != 0 {
			return Err(Error::InvalidArgument);
		}
		let peer_name = r.opt_str()?.map(str::to_owned);
		r.finish()?;
		Ok(Self {
			transport,
			supported: flags & FLAG_SUPPORTED != 0,
			paired: flags & FLAG_PAIRED != 0,
			reachable: flags & FLAG_REACHABLE != 0,
			companion_installed: flags & FLAG_INSTALLED != 0,
			peer_name,
		})
	}
}

/// An immediate control message to the reachable peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message<'a> {
	/// Logical route/topic (e.g. `"/quant/pair"`, `"/quant/reading"`).
	pub topic: &'a str,
	/// Opaque payload bytes.
	pub payload: &'a [u8],
}

impl Message<'_> {
	/// Encodes a message request.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		if self.topic.is_empty() || self.payload.len() > MAX_MESSAGE_BYTES {
			return Err(Error::InvalidArgument);
		}
		let mut w = Writer::default();
		w.str(self.topic)?.bytes(self.payload)?;
		Ok(w.finish())
	}
}

/// A durable background transfer queued until the peer reconnects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer<'a> {
	/// Caller-chosen unique transfer id for deduplication and `ACK`.
	pub id: &'a str,
	/// Logical route/topic.
	pub topic: &'a str,
	/// Opaque payload bytes.
	pub payload: &'a [u8],
}

impl Transfer<'_> {
	/// Encodes a durable transfer request.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		if self.id.is_empty() || self.topic.is_empty() || self.payload.len() > MAX_TRANSFER_BYTES {
			return Err(Error::InvalidArgument);
		}
		let mut w = Writer::default();
		w.str(self.id)?.str(self.topic)?.bytes(self.payload)?;
		Ok(w.finish())
	}
}

/// An envelope delivered from the paired watch or phone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingEnvelope {
	/// Transfer or message identifier (empty for fire-and-forget messages).
	pub id: String,
	/// Logical topic.
	pub topic: String,
	/// Unix milliseconds when received locally.
	pub received_ms: i64,
	/// Opaque payload bytes.
	pub payload: Vec<u8>,
}

impl IncomingEnvelope {
	/// Encodes an incoming envelope to wire bytes.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		if self.topic.is_empty() || self.payload.len() > MAX_TRANSFER_BYTES {
			return Err(Error::InvalidArgument);
		}
		let mut w = Writer::default();
		w.str(&self.id)?
			.str(&self.topic)?
			.i64(self.received_ms)
			.bytes(&self.payload)?;
		Ok(w.finish())
	}

	/// Decodes an incoming envelope from wire bytes.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut r = Reader::new(payload);
		let id = r.str()?.to_owned();
		let topic = r.str()?.to_owned();
		if topic.is_empty() {
			return Err(Error::InvalidArgument);
		}
		let received_ms = r.i64()?;
		let data = r.bytes()?;
		if data.len() > MAX_TRANSFER_BYTES {
			return Err(Error::TooLarge);
		}
		r.finish()?;
		Ok(Self {
			id,
			topic,
			received_ms,
			payload: data.to_vec(),
		})
	}
}

/// Encodes an `ACK` request for transfer `id`.
pub fn ack_request(id: &str) -> Result<Vec<u8>, Error> {
	if id.is_empty() {
		return Err(Error::InvalidArgument);
	}
	let mut w = Writer::default();
	w.str(id)?;
	Ok(w.finish())
}

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
	fn u32(&mut self, value: u32) -> &mut Self {
		self.0.extend_from_slice(&value.to_le_bytes());
		self
	}

	fn i64(&mut self, value: i64) -> &mut Self {
		self.0.extend_from_slice(&value.to_le_bytes());
		self
	}

	fn bytes(&mut self, value: &[u8]) -> Result<&mut Self, Error> {
		let len = u32::try_from(value.len())
			.ok()
			.filter(|l| *l != ABSENT)
			.ok_or(Error::TooLarge)?;
		self.u32(len);
		self.0.extend_from_slice(value);
		Ok(self)
	}

	fn str(&mut self, value: &str) -> Result<&mut Self, Error> {
		self.bytes(value.as_bytes())
	}

	fn opt_str(&mut self, value: Option<&str>) -> Result<&mut Self, Error> {
		match value {
			Some(s) => self.str(s),
			None => Ok(self.u32(ABSENT)),
		}
	}

	fn finish(self) -> Vec<u8> {
		self.0
	}
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
	fn new(bytes: &'a [u8]) -> Self {
		Self(bytes)
	}

	fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
		let (head, tail) = self.0.split_at_checked(N).ok_or(Error::InvalidArgument)?;
		self.0 = tail;
		head.try_into().map_err(|_| Error::InvalidArgument)
	}

	fn u32(&mut self) -> Result<u32, Error> {
		self.take().map(u32::from_le_bytes)
	}

	fn i64(&mut self) -> Result<i64, Error> {
		self.take().map(i64::from_le_bytes)
	}

	fn bytes(&mut self) -> Result<&'a [u8], Error> {
		let len = self.u32()?;
		if len == ABSENT {
			return Err(Error::InvalidArgument);
		}
		let (head, tail) = self
			.0
			.split_at_checked(len as usize)
			.ok_or(Error::InvalidArgument)?;
		self.0 = tail;
		Ok(head)
	}

	fn str(&mut self) -> Result<&'a str, Error> {
		std::str::from_utf8(self.bytes()?).map_err(|_| Error::InvalidArgument)
	}

	fn opt_str(&mut self) -> Result<Option<&'a str>, Error> {
		let len = self.u32()?;
		if len == ABSENT {
			return Ok(None);
		}
		let (head, tail) = self
			.0
			.split_at_checked(len as usize)
			.ok_or(Error::InvalidArgument)?;
		self.0 = tail;
		std::str::from_utf8(head)
			.map(Some)
			.map_err(|_| Error::InvalidArgument)
	}

	fn finish(self) -> Result<(), Error> {
		self.0
			.is_empty()
			.then_some(())
			.ok_or(Error::InvalidArgument)
	}
}

/// Native guest client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Cached bridge handle.
	#[derive(Debug, Clone, Copy)]
	pub struct WatchSync(leptoskit_bridge::guest::Client);

	impl WatchSync {
		/// Resolves the selected plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Current pairing and reachability status.
		pub async fn status(self) -> Result<SyncStatus, Error> {
			SyncStatus::decode(&self.0.call(STATUS, &[])?.await?)
		}

		/// Sends an immediate message to the reachable peer.
		pub async fn send_message(self, topic: &str, payload: &[u8]) -> Result<(), Error> {
			let req = Message { topic, payload }.encode()?;
			self.0.call(SEND_MESSAGE, &req)?.await?;
			Ok(())
		}

		/// Queues a durable background transfer to the paired peer.
		pub async fn queue_transfer(
			self,
			id: &str,
			topic: &str,
			payload: &[u8],
		) -> Result<(), Error> {
			let req = Transfer { id, topic, payload }.encode()?;
			self.0.call(QUEUE_TRANSFER, &req)?.await?;
			Ok(())
		}

		/// Waits for the next incoming message or durable transfer.
		pub async fn next_incoming(self) -> Result<IncomingEnvelope, Error> {
			IncomingEnvelope::decode(&self.0.call(NEXT_INCOMING, &[])?.await?)
		}

		/// Acknowledges an incoming durable transfer.
		pub async fn ack(self, id: &str) -> Result<(), Error> {
			let req = ack_request(id)?;
			self.0.call(ACK, &req)?.await?;
			Ok(())
		}

		/// Waits until status differs from `last`.
		pub async fn watch_status(self, last: &SyncStatus) -> Result<SyncStatus, Error> {
			let req = last.encode()?;
			SyncStatus::decode(&self.0.call(WATCH_STATUS, &req)?.await?)
		}
	}
}

/// Browsers have no watch companion link.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Reports unsupported status.
	pub fn status() -> SyncStatus {
		SyncStatus {
			transport: SyncTransport::None,
			supported: false,
			paired: false,
			reachable: false,
			companion_installed: false,
			peer_name: None,
		}
	}
}

/// Desktops have no watch companion link.
#[cfg(not(target_arch = "wasm32"))]
pub mod desktop {
	use super::*;

	/// Reports unsupported status.
	pub fn status() -> SyncStatus {
		SyncStatus {
			transport: SyncTransport::None,
			supported: false,
			paired: false,
			reachable: false,
			companion_installed: false,
			peer_name: None,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn methods_are_dense_and_in_order() {
		for (index, method) in METHODS.iter().enumerate() {
			assert_eq!(method.0 as usize, index);
		}
	}

	#[test]
	fn status_and_envelopes_round_trip() {
		let status = SyncStatus {
			transport: SyncTransport::WatchConnectivity,
			supported: true,
			paired: true,
			reachable: true,
			companion_installed: true,
			peer_name: Some("Apple Watch Ultra".into()),
		};
		assert_eq!(SyncStatus::decode(&status.encode().unwrap()), Ok(status));

		let envelope = IncomingEnvelope {
			id: "tx-1".into(),
			topic: "/quant/reading".into(),
			received_ms: 1_700_000_000_000,
			payload: vec![1, 2, 3, 4],
		};
		assert_eq!(
			IncomingEnvelope::decode(&envelope.encode().unwrap()),
			Ok(envelope)
		);
	}
}
