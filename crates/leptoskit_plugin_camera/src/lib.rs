//! <!-- {=p_cameraSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_camera` as a direct dependency and select the `camera`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Linux hosts register it where it has a Linux Rust
//! implementation; generated Windows and terminal hosts currently omit runtime
//! plugins, so use explicit registration in a custom host there. Guests enable
//! the `guest` feature and reuse `guest::Camera` rather than resolving the
//! plugin on every call. Ask for camera access through the `permissions`
//! plugin; this plugin returns `PermissionDenied` or `Unavailable` when capture
//! lacks a grant. iOS and macOS use `AVCaptureSession` with an
//! `AVCaptureVideoPreviewLayer` surface, Android uses `CameraX` with a
//! `PreviewView`, and the browser uses `getUserMedia` into an HTML `<video>`
//! element; Windows, Linux, and HarmonyOS return `Unavailable`. Video recording
//! is not in this MVP.
//!
//! <!-- {/p_cameraSummary} -->
//!
//! Camera wire v1: create takes a facing code; torch and take-picture name a
//! session; status replies with facing, torch on/available, and active; take
//! picture replies with a bounded temporary asset (URI, MIME, width, height,
//! size) matching `image_picker` Asset framing. All integers are little-endian;
//! trailing bytes are rejected.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "camera";
/// Facing code -> session id.
pub const CREATE: MethodId = MethodId(0);
/// Session id and torch flag -> empty reply (`Unavailable` when unsupported).
pub const SET_TORCH: MethodId = MethodId(1);
/// Session id and max bytes -> encoded [`Asset`].
pub const TAKE_PICTURE: MethodId = MethodId(2);
/// Session id -> encoded [`Status`].
pub const STATUS: MethodId = MethodId(3);
/// Session id -> empty reply; drops the host session.
pub const RELEASE: MethodId = MethodId(4);
/// Supported wire operations.
pub const METHODS: &[MethodId] = &[CREATE, SET_TORCH, TAKE_PICTURE, STATUS, RELEASE];

/// Wire protocol version for status replies.
const WIRE_VERSION: u32 = 1;
/// Maximum still-capture file size copied into app cache.
pub const MAX_BYTES: u32 = 16 * 1024 * 1024;
/// Maximum UTF-8 bytes in the temporary URI.
pub const MAX_URI_BYTES: usize = 4096;
/// Maximum UTF-8 bytes in the MIME type.
pub const MAX_MIME_BYTES: usize = 256;

/// Which camera faces the lens toward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Facing {
	/// Rear / world-facing camera.
	Back = 0,
	/// User-facing camera.
	Front = 1,
}

impl Facing {
	fn from_code(code: u32) -> Option<Self> {
		match code {
			0 => Some(Self::Back),
			1 => Some(Self::Front),
			_ => None,
		}
	}
}

/// A host-owned camera session. Identifiers are never reused within one plugin
/// instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub u32);

impl SessionId {
	/// Encodes the id as four little-endian bytes.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(4);
		output.u32(self.0);
		output.finish()
	}

	/// Decodes a session id, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let id = input.u32()?;
		input.finish()?;
		Ok(Self(id))
	}
}

/// Create request: which lens to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Create {
	/// Lens facing.
	pub facing: Facing,
}

impl Create {
	/// Encodes the facing code.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(4);
		output.u32(self.facing as u32);
		output.finish()
	}

	/// Decodes a facing code, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let facing = Facing::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;
		input.finish()?;
		Ok(Self { facing })
	}
}

/// A transport that names one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCommand {
	/// Target session.
	pub session: SessionId,
}

impl SessionCommand {
	/// Encodes the session id alone.
	#[must_use]
	pub fn encode(self) -> Bytes {
		self.session.encode()
	}

	/// Decodes a session id, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		Ok(Self {
			session: SessionId::decode(bytes)?,
		})
	}
}

/// Torch on/off for one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Torch {
	/// Target session.
	pub session: SessionId,
	/// Whether the torch should be on.
	pub enabled: bool,
}

impl Torch {
	/// Encodes session id and a 0/1 flag.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(8);
		output.u32(self.session.0);
		output.u32(u32::from(self.enabled));
		output.finish()
	}

	/// Decodes a session id and boolean flag.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let session = SessionId(input.u32()?);
		let enabled = decode_flag(input.u32()?)?;
		input.finish()?;
		Ok(Self { session, enabled })
	}
}

/// Still-capture request with a size budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TakePicture {
	/// Target session.
	pub session: SessionId,
	/// Maximum file size the host may write into app cache.
	pub max_bytes: u32,
}

impl TakePicture {
	/// Rejects limits above the plugin's fixed allocation budget.
	pub fn new(session: SessionId, max_bytes: u32) -> Result<Self, Error> {
		if max_bytes > MAX_BYTES {
			return Err(Error::TooLarge);
		}
		Ok(Self { session, max_bytes })
	}

	/// Default size budget for `session`.
	#[must_use]
	pub fn default_budget(session: SessionId) -> Self {
		Self {
			session,
			max_bytes: MAX_BYTES,
		}
	}

	/// Encodes session id and max bytes.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(8);
		output.u32(self.session.0);
		output.u32(self.max_bytes);
		output.finish()
	}

	/// Decodes through [`TakePicture::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let session = SessionId(input.u32()?);
		let max_bytes = input.u32()?;
		input.finish()?;
		Self::new(session, max_bytes)
	}
}

/// Snapshot of one camera session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
	/// Lens facing for this session.
	pub facing: Facing,
	/// Whether the torch is currently on.
	pub torch_on: bool,
	/// Whether the host can toggle the torch for this session.
	pub torch_available: bool,
	/// Whether the session is open and may capture.
	pub active: bool,
}

impl Status {
	/// Encodes version and the four status fields.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(20);
		output.u32(WIRE_VERSION);
		output.u32(self.facing as u32);
		output.u32(u32::from(self.torch_on));
		output.u32(u32::from(self.torch_available));
		output.u32(u32::from(self.active));
		output.finish()
	}

	/// Strictly validates a fixed 20-byte status reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != WIRE_VERSION {
			return Err(Error::InvalidArgument);
		}
		let facing = Facing::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;
		let torch_on = decode_flag(input.u32()?)?;
		let torch_available = decode_flag(input.u32()?)?;
		let active = decode_flag(input.u32()?)?;
		input.finish()?;
		Ok(Self {
			facing,
			torch_on,
			torch_available,
			active,
		})
	}
}

/// One temporary still capture with no retained gallery grant.
///
/// Framing matches `leptoskit_plugin_image_picker::Asset` so apps can share
/// decode helpers and size budgets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
	/// App-owned temporary file URI (`file:` / `blob:`).
	pub uri: String,
	/// Provider MIME type, or None when unknown.
	pub mime_type: Option<String>,
	/// Pixel width when known.
	pub width: Option<u32>,
	/// Pixel height when known.
	pub height: Option<u32>,
	/// File size in bytes when known.
	pub size_bytes: Option<u64>,
}

impl Asset {
	/// Encodes a reply the native hosts also emit.
	pub fn encode(&self) -> Result<Bytes, Error> {
		if self.uri.is_empty()
			|| self.uri.len() > MAX_URI_BYTES
			|| self.mime_type.as_ref().is_some_and(String::is_empty)
			|| self
				.mime_type
				.as_ref()
				.is_some_and(|mime| mime.len() > MAX_MIME_BYTES)
		{
			return Err(Error::InvalidArgument);
		}
		if self.size_bytes == Some(u64::MAX) {
			return Err(Error::InvalidArgument);
		}
		if self.width == Some(u32::MAX) || self.height == Some(u32::MAX) {
			return Err(Error::InvalidArgument);
		}

		let mut output = Encoder::with_capacity(64);
		output.bytes(self.uri.as_bytes())?;
		output.bytes(self.mime_type.as_deref().unwrap_or_default().as_bytes())?;
		output.u32(self.width.unwrap_or(u32::MAX));
		output.u32(self.height.unwrap_or(u32::MAX));
		let size = self.size_bytes.unwrap_or(u64::MAX).to_le_bytes();
		output.u32(u32::from_le_bytes(size[..4].try_into().unwrap()));
		output.u32(u32::from_le_bytes(size[4..].try_into().unwrap()));
		Ok(output.finish())
	}

	/// Validates framing and bounds before exposing a native reply.
	pub fn decode(bytes: &[u8], max_bytes: u32) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let uri = input.string()?;
		let mime = input.string()?;
		if uri.is_empty() || uri.len() > MAX_URI_BYTES || mime.len() > MAX_MIME_BYTES {
			return Err(Error::InvalidArgument);
		}
		let width = optional_u32(input.u32()?);
		let height = optional_u32(input.u32()?);
		let low = u64::from(input.u32()?);
		let high = u64::from(input.u32()?);
		let size = low | (high << 32);
		let size_bytes = (size != u64::MAX).then_some(size);
		if let Some(size) = size_bytes
			&& size > u64::from(max_bytes)
		{
			return Err(Error::TooLarge);
		}
		input.finish()?;
		Ok(Self {
			uri: uri.to_owned(),
			mime_type: (!mime.is_empty()).then(|| mime.to_owned()),
			width,
			height,
			size_bytes,
		})
	}
}

fn optional_u32(value: u32) -> Option<u32> {
	(value != u32::MAX).then_some(value)
}

fn decode_flag(value: u32) -> Result<bool, Error> {
	match value {
		0 => Ok(false),
		1 => Ok(true),
		_ => Err(Error::InvalidArgument),
	}
}

/// Cached native guest client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// One resolved camera plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Camera(leptoskit_bridge::guest::Client);

	impl Camera {
		/// Resolves the installed plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Opens a camera session for `facing` and returns its id.
		pub async fn create(self, facing: Facing) -> Result<SessionId, Error> {
			SessionId::decode(&self.0.call(CREATE, &Create { facing }.encode())?.await?)
		}

		/// Turns the torch on or off; hosts without a torch return
		/// `Unavailable`.
		pub async fn set_torch(self, session: SessionId, enabled: bool) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(SET_TORCH, &Torch { session, enabled }.encode())?
					.await?,
			)
		}

		/// Captures one still into a bounded temporary URI.
		pub async fn take_picture(self, request: TakePicture) -> Result<Asset, Error> {
			let reply = self.0.call(TAKE_PICTURE, &request.encode())?.await?;
			Asset::decode(&reply, request.max_bytes)
		}

		/// Snapshots facing, torch, and active state.
		pub async fn status(self, session: SessionId) -> Result<Status, Error> {
			Status::decode(
				&self
					.0
					.call(STATUS, &SessionCommand { session }.encode())?
					.await?,
			)
		}

		/// Releases the host session; bound `Camera` views show nothing.
		pub async fn release(self, session: SessionId) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(RELEASE, &SessionCommand { session }.encode())?
					.await?,
			)
		}
	}

	fn require_empty(bytes: &[u8]) -> Result<(), Error> {
		Decoder::new(bytes).finish()
	}
}

/// Browser Direct module: `getUserMedia` sessions keyed by session id.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

// A guest build leaves the browser module out, and with it every use of
// `web-sys`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "guest"))]
use web_sys as _;

#[cfg(test)]
mod tests {
	use leptoskit_bridge::codec::Encoder;

	use super::*;

	#[test]
	fn facing_create_and_session_codecs() {
		assert_eq!(Facing::from_code(0), Some(Facing::Back));
		assert_eq!(Facing::from_code(1), Some(Facing::Front));
		assert_eq!(Facing::from_code(2), None);
		let create = Create {
			facing: Facing::Front,
		};
		assert_eq!(Create::decode(&create.encode()), Ok(create));
		assert_eq!(
			Create::decode(&{
				let mut bad = create.encode().to_vec();
				bad[0] = 9;
				bad
			}),
			Err(Error::InvalidArgument)
		);
		let mut trailing = create.encode().to_vec();
		trailing.push(0);
		assert_eq!(Create::decode(&trailing), Err(Error::InvalidArgument));
		assert!(Create::decode(&[]).is_err());

		assert_eq!(
			SessionId::decode(&SessionId(42).encode()),
			Ok(SessionId(42))
		);
		assert_eq!(
			SessionCommand::decode(
				&SessionCommand {
					session: SessionId(9)
				}
				.encode()
			),
			Ok(SessionCommand {
				session: SessionId(9)
			})
		);
		let mut trailing = SessionId(1).encode().to_vec();
		trailing.push(0);
		assert_eq!(SessionId::decode(&trailing), Err(Error::InvalidArgument));
	}

	#[test]
	fn torch_and_take_picture_codecs() {
		let torch = Torch {
			session: SessionId(2),
			enabled: true,
		};
		assert_eq!(Torch::decode(&torch.encode()), Ok(torch));
		let mut bad_flag = torch.encode().to_vec();
		bad_flag[4] = 2;
		assert_eq!(Torch::decode(&bad_flag), Err(Error::InvalidArgument));
		let mut trailing = torch.encode().to_vec();
		trailing.push(1);
		assert_eq!(Torch::decode(&trailing), Err(Error::InvalidArgument));
		assert!(Torch::decode(&torch.encode()[..4]).is_err());

		let take = TakePicture::new(SessionId(3), 1_024).unwrap();
		assert_eq!(TakePicture::decode(&take.encode()), Ok(take));
		assert_eq!(
			TakePicture::new(SessionId(1), MAX_BYTES + 1),
			Err(Error::TooLarge)
		);
		assert_eq!(
			TakePicture::default_budget(SessionId(7)).max_bytes,
			MAX_BYTES
		);
		assert_eq!(
			TakePicture::decode(&{
				let mut over = TakePicture::default_budget(SessionId(1)).encode().to_vec();
				over[4..8].copy_from_slice(&(MAX_BYTES + 1).to_le_bytes());
				over
			}),
			Err(Error::TooLarge)
		);
		assert!(TakePicture::decode(&take.encode()[..4]).is_err());
	}

	#[test]
	fn status_round_trip_and_rejects_bad_fields() {
		let status = Status {
			facing: Facing::Back,
			torch_on: true,
			torch_available: true,
			active: true,
		};
		assert_eq!(Status::decode(&status.encode()), Ok(status));
		let off = Status {
			facing: Facing::Front,
			torch_on: false,
			torch_available: false,
			active: false,
		};
		assert_eq!(Status::decode(&off.encode()), Ok(off));
		let mut bad_facing = status.encode().to_vec();
		bad_facing[4] = 9;
		assert_eq!(Status::decode(&bad_facing), Err(Error::InvalidArgument));
		let mut bad_flag = status.encode().to_vec();
		bad_flag[8] = 3;
		assert_eq!(Status::decode(&bad_flag), Err(Error::InvalidArgument));
		let mut bad_available = status.encode().to_vec();
		bad_available[12] = 2;
		assert_eq!(Status::decode(&bad_available), Err(Error::InvalidArgument));
		let mut bad_active = status.encode().to_vec();
		bad_active[16] = 4;
		assert_eq!(Status::decode(&bad_active), Err(Error::InvalidArgument));
		let mut trailing = status.encode().to_vec();
		trailing.push(0);
		assert_eq!(Status::decode(&trailing), Err(Error::InvalidArgument));
		assert!(Status::decode(&status.encode()[..16]).is_err());
		let mut bad_version = status.encode().to_vec();
		bad_version[0] = 9;
		assert_eq!(Status::decode(&bad_version), Err(Error::InvalidArgument));
	}

	#[test]
	fn asset_round_trips_and_enforces_caller_limit() {
		let asset = Asset {
			uri: "file:///tmp/photo.jpg".into(),
			mime_type: Some("image/jpeg".into()),
			width: Some(100),
			height: Some(80),
			size_bytes: Some(12),
		};
		let bytes = asset.encode().unwrap();
		assert_eq!(Asset::decode(&bytes, MAX_BYTES), Ok(asset.clone()));
		assert_eq!(Asset::decode(&bytes, 11), Err(Error::TooLarge));

		assert_eq!(
			Asset {
				uri: String::new(),
				mime_type: None,
				width: None,
				height: None,
				size_bytes: None,
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Asset {
				uri: "x".repeat(MAX_URI_BYTES + 1),
				mime_type: None,
				width: None,
				height: None,
				size_bytes: None,
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Asset {
				uri: "file:///a".into(),
				mime_type: Some(String::new()),
				width: None,
				height: None,
				size_bytes: None,
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Asset {
				uri: "file:///a".into(),
				mime_type: Some("m".repeat(MAX_MIME_BYTES + 1)),
				width: None,
				height: None,
				size_bytes: None,
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Asset {
				uri: "file:///a".into(),
				mime_type: None,
				width: Some(u32::MAX),
				height: None,
				size_bytes: None,
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Asset {
				uri: "file:///a".into(),
				mime_type: None,
				width: None,
				height: Some(u32::MAX),
				size_bytes: None,
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Asset {
				uri: "file:///a".into(),
				mime_type: None,
				width: None,
				height: None,
				size_bytes: Some(u64::MAX),
			}
			.encode(),
			Err(Error::InvalidArgument)
		);

		let unknown = Asset {
			uri: "blob:photo".into(),
			mime_type: None,
			width: None,
			height: None,
			size_bytes: None,
		};
		assert_eq!(
			Asset::decode(&unknown.encode().unwrap(), MAX_BYTES),
			Ok(unknown)
		);

		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert_eq!(
			Asset::decode(&trailing, MAX_BYTES),
			Err(Error::InvalidArgument)
		);
		assert!(Asset::decode(&bytes[..bytes.len() - 1], MAX_BYTES).is_err());

		// Oversized URI on the wire.
		let oversized = "x".repeat(MAX_URI_BYTES + 1);
		let mut wire = Encoder::with_capacity(8 + oversized.len());
		wire.bytes(oversized.as_bytes()).unwrap();
		wire.bytes(b"").unwrap();
		wire.u32(u32::MAX);
		wire.u32(u32::MAX);
		wire.u32(u32::MAX);
		wire.u32(u32::MAX);
		assert_eq!(
			Asset::decode(&wire.finish(), MAX_BYTES),
			Err(Error::InvalidArgument)
		);

		// Empty URI on the wire.
		let mut empty = Encoder::with_capacity(16);
		empty.bytes(b"").unwrap();
		empty.bytes(b"").unwrap();
		empty.u32(u32::MAX);
		empty.u32(u32::MAX);
		empty.u32(u32::MAX);
		empty.u32(u32::MAX);
		assert_eq!(
			Asset::decode(&empty.finish(), MAX_BYTES),
			Err(Error::InvalidArgument)
		);

		// Oversized MIME on the wire.
		let mime = "m".repeat(MAX_MIME_BYTES + 1);
		let mut mime_wire = Encoder::with_capacity(8 + mime.len());
		mime_wire.bytes(b"file:///a").unwrap();
		mime_wire.bytes(mime.as_bytes()).unwrap();
		mime_wire.u32(u32::MAX);
		mime_wire.u32(u32::MAX);
		mime_wire.u32(u32::MAX);
		mime_wire.u32(u32::MAX);
		assert_eq!(
			Asset::decode(&mime_wire.finish(), MAX_BYTES),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn methods_cover_the_mvp_surface() {
		assert_eq!(METHODS.len(), 5);
		assert_eq!(CREATE, MethodId(0));
		assert_eq!(SET_TORCH, MethodId(1));
		assert_eq!(TAKE_PICTURE, MethodId(2));
		assert_eq!(STATUS, MethodId(3));
		assert_eq!(RELEASE, MethodId(4));
		assert_eq!(NAME, "camera");
		assert_eq!(MAX_BYTES, 16 * 1024 * 1024);
		assert_eq!(MAX_URI_BYTES, 4096);
		assert_eq!(MAX_MIME_BYTES, 256);
	}

	#[test]
	fn empty_reply_helper_rejects_trailing() {
		assert!(Decoder::new(&[]).finish().is_ok());
		assert_eq!(Decoder::new(&[0]).finish(), Err(Error::InvalidArgument));
	}

	#[test]
	fn optional_u32_sentinel() {
		assert_eq!(optional_u32(0), Some(0));
		assert_eq!(optional_u32(u32::MAX), None);
		assert_eq!(decode_flag(0), Ok(false));
		assert_eq!(decode_flag(1), Ok(true));
		assert_eq!(decode_flag(2), Err(Error::InvalidArgument));
	}
}
