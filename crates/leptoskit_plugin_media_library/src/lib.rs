//! <!-- {=p_media_librarySummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_media_library` as a direct dependency and select the
//! `media-library` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Generated Linux hosts register it where it
//! has a Linux Rust implementation; generated Windows and terminal hosts
//! currently omit runtime plugins, so use explicit registration in a custom
//! host there. Guests enable the `guest` feature and reuse
//! `guest::MediaLibrary`. Status never prompts. List returns at most 64 assets
//! with id, media kind, URI, dimensions, and creation time. Save copies an
//! app-owned file URI into the system library. Change observation, albums, and
//! limited-library nuance remain later scope. Browsers report Unavailable.
//!
//! <!-- {/p_media_librarySummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Native registration name.
pub const NAME: &str = "media-library";
/// Empty request; exact four-byte [`Status`]. Never prompts.
pub const STATUS: MethodId = MethodId(0);
/// Empty request; may prompt, then returns [`Status`].
pub const REQUEST_PERMISSION: MethodId = MethodId(1);
/// [`ListOptions`] request; returns a bounded asset list.
pub const LIST_ASSETS: MethodId = MethodId(2);
/// [`SaveRequest`] request; returns one saved [`Asset`].
pub const SAVE: MethodId = MethodId(3);
/// Independently grantable methods.
pub const METHODS: &[MethodId] = &[STATUS, REQUEST_PERMISSION, LIST_ASSETS, SAVE];

/// Hard cap on assets returned by one list.
pub const MAX_LIST: u32 = 64;
/// Maximum UTF-8 bytes for an asset id.
pub const MAX_ID_BYTES: usize = 256;
/// Maximum UTF-8 bytes for a URI.
pub const MAX_URI_BYTES: usize = 4096;
/// Maximum file bytes accepted by save (copy budget).
pub const MAX_SAVE_BYTES: u32 = 16 * 1024 * 1024;

/// Library authorization, distinct from whether any assets exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Status {
	/// The platform cannot distinguish more specific non-granted states.
	Unknown = 0,
	/// No choice has been made; an explicit request can prompt.
	Prompt = 1,
	/// Access denied by the user or OS.
	Denied = 2,
	/// The OS has granted library access.
	Granted = 3,
	/// A platform restriction prevents the user granting access.
	Restricted = 4,
}

impl Status {
	/// Rejects unknown values and non-exact response lengths.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match word(bytes)? {
			0 => Ok(Self::Unknown),
			1 => Ok(Self::Prompt),
			2 => Ok(Self::Denied),
			3 => Ok(Self::Granted),
			4 => Ok(Self::Restricted),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Fixed four-byte response.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}
}

/// Which media kinds a list may return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MediaKind {
	/// Still images only.
	Images = 0,
	/// Videos only.
	Videos = 1,
	/// Images or videos.
	All = 2,
}

impl MediaKind {
	fn from_code(code: u32) -> Option<Self> {
		match code {
			0 => Some(Self::Images),
			1 => Some(Self::Videos),
			2 => Some(Self::All),
			_ => None,
		}
	}
}

/// Validated list request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListOptions {
	kind: MediaKind,
	limit: u32,
}

impl ListOptions {
	/// Rejects zero or oversized limits.
	pub fn new(kind: MediaKind, limit: u32) -> Result<Self, Error> {
		if limit == 0 || limit > MAX_LIST {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { kind, limit })
	}

	/// Media kinds to include.
	#[must_use]
	pub fn kind(self) -> MediaKind {
		self.kind
	}

	/// Maximum assets to return.
	#[must_use]
	pub fn limit(self) -> u32 {
		self.limit
	}

	/// Wire: kind `u32` + limit `u32`.
	#[must_use]
	pub fn encode(self) -> [u8; 8] {
		let mut bytes = [0_u8; 8];
		bytes[..4].copy_from_slice(&(self.kind as u32).to_le_bytes());
		bytes[4..].copy_from_slice(&self.limit.to_le_bytes());
		bytes
	}

	/// Validates a guest/native request.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let kind = MediaKind::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;
		let limit = input.u32()?;
		input.finish()?;
		Self::new(kind, limit)
	}
}

/// One library asset with no change-observation subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
	id: String,
	kind: MediaKind,
	uri: String,
	width: Option<u32>,
	height: Option<u32>,
	creation_time_ms: Option<u64>,
}

impl Asset {
	/// Builds a validated asset. Id and URI are required and non-empty.
	/// `MediaKind::All` is rejected for a concrete asset.
	pub fn new(
		id: impl Into<String>,
		kind: MediaKind,
		uri: impl Into<String>,
		width: Option<u32>,
		height: Option<u32>,
		creation_time_ms: Option<u64>,
	) -> Result<Self, Error> {
		if matches!(kind, MediaKind::All) {
			return Err(Error::InvalidArgument);
		}
		if width == Some(u32::MAX) || height == Some(u32::MAX) {
			return Err(Error::InvalidArgument);
		}
		if creation_time_ms == Some(u64::MAX) {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			id: required_field(id.into(), MAX_ID_BYTES)?,
			kind,
			uri: required_field(uri.into(), MAX_URI_BYTES)?,
			width,
			height,
			creation_time_ms,
		})
	}

	/// Platform asset identifier.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Image or video.
	#[must_use]
	pub fn kind(&self) -> MediaKind {
		self.kind
	}

	/// Local or content URI for this asset.
	#[must_use]
	pub fn uri(&self) -> &str {
		&self.uri
	}

	/// Pixel width when known.
	#[must_use]
	pub fn width(&self) -> Option<u32> {
		self.width
	}

	/// Pixel height when known.
	#[must_use]
	pub fn height(&self) -> Option<u32> {
		self.height
	}

	/// Creation time in Unix milliseconds when known.
	#[must_use]
	pub fn creation_time_ms(&self) -> Option<u64> {
		self.creation_time_ms
	}

	/// Wire frame for one asset.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(64 + self.id.len() + self.uri.len());
		output.bytes(self.id.as_bytes())?;
		output.u32(self.kind as u32);
		output.bytes(self.uri.as_bytes())?;
		output.u32(self.width.unwrap_or(u32::MAX));
		output.u32(self.height.unwrap_or(u32::MAX));
		write_u64(&mut output, self.creation_time_ms.unwrap_or(u64::MAX));
		Ok(output.finish())
	}

	/// Validates an untrusted asset frame.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() > max_asset_frame() {
			return Err(Error::TooLarge);
		}
		let mut input = Decoder::new(bytes);
		let asset = decode_asset_fields(&mut input)?;
		input.finish()?;
		Ok(asset)
	}
}

/// Save an app-owned file URI into the system library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRequest {
	uri: String,
	kind: MediaKind,
}

impl SaveRequest {
	/// Accepts a non-empty app file URI and a concrete image/video kind.
	pub fn new(uri: impl Into<String>, kind: MediaKind) -> Result<Self, Error> {
		if matches!(kind, MediaKind::All) {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			uri: required_field(uri.into(), MAX_URI_BYTES)?,
			kind,
		})
	}

	/// Source URI inside the app files root.
	#[must_use]
	pub fn uri(&self) -> &str {
		&self.uri
	}

	/// Declared media kind for the save.
	#[must_use]
	pub fn kind(&self) -> MediaKind {
		self.kind
	}

	/// Wire: kind + length-prefixed URI.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(8 + self.uri.len());
		output.u32(self.kind as u32);
		output.bytes(self.uri.as_bytes())?;
		Ok(output.finish())
	}

	/// Validates a guest/native save request.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() > MAX_URI_BYTES + 8 {
			return Err(Error::TooLarge);
		}
		let mut input = Decoder::new(bytes);
		let kind = MediaKind::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;
		let uri = input.string()?.to_owned();
		input.finish()?;
		Self::new(uri, kind)
	}
}

/// Encodes a list of assets with a leading count.
pub fn encode_assets(assets: &[Asset]) -> Result<Bytes, Error> {
	if assets.len() > MAX_LIST as usize {
		return Err(Error::TooLarge);
	}
	let mut output = Encoder::with_capacity(4);
	output.u32(assets.len() as u32);
	for asset in assets {
		let frame = asset.encode()?;
		output.bytes(&frame)?;
	}
	Ok(output.finish())
}

/// Decodes a bounded asset list.
pub fn decode_assets(bytes: &[u8]) -> Result<Vec<Asset>, Error> {
	if bytes.len() > 4 + max_asset_frame() * MAX_LIST as usize {
		return Err(Error::TooLarge);
	}
	let mut input = Decoder::new(bytes);
	let count = input.u32()?;
	if count > MAX_LIST {
		return Err(Error::TooLarge);
	}
	let mut assets = Vec::with_capacity(count as usize);
	for _ in 0..count {
		let frame = input.bytes()?;
		assets.push(Asset::decode(frame)?);
	}
	input.finish()?;
	Ok(assets)
}

fn decode_asset_fields(input: &mut Decoder<'_>) -> Result<Asset, Error> {
	let id = input.string()?.to_owned();
	let kind = MediaKind::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;
	let uri = input.string()?.to_owned();
	let width = optional_u32(input.u32()?);
	let height = optional_u32(input.u32()?);
	let creation = optional_u64(read_u64(input)?);
	Asset::new(id, kind, uri, width, height, creation)
}

fn required_field(value: String, max: usize) -> Result<String, Error> {
	if value.is_empty() || value.contains('\0') {
		return Err(Error::InvalidArgument);
	}
	if value.len() > max {
		return Err(Error::InvalidArgument);
	}
	Ok(value)
}

fn optional_u32(value: u32) -> Option<u32> {
	(value != u32::MAX).then_some(value)
}

fn optional_u64(value: u64) -> Option<u64> {
	(value != u64::MAX).then_some(value)
}

fn write_u64(output: &mut Encoder, value: u64) {
	let bytes = value.to_le_bytes();
	output.u32(u32::from_le_bytes(bytes[..4].try_into().unwrap()));
	output.u32(u32::from_le_bytes(bytes[4..].try_into().unwrap()));
}

fn read_u64(input: &mut Decoder<'_>) -> Result<u64, Error> {
	let low = u64::from(input.u32()?);
	let high = u64::from(input.u32()?);
	Ok(low | (high << 32))
}

fn max_asset_frame() -> usize {
	8 + MAX_ID_BYTES + 4 + 8 + MAX_URI_BYTES + 8 + 8
}

fn word(bytes: &[u8]) -> Result<u32, Error> {
	Ok(u32::from_le_bytes(
		bytes.try_into().map_err(|_| Error::InvalidArgument)?,
	))
}

/// Validates a method without touching the library.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		STATUS | REQUEST_PERMISSION if payload.is_empty() => Ok(()),
		STATUS | REQUEST_PERMISSION => Err(Error::InvalidArgument),
		LIST_ASSETS => {
			ListOptions::decode(payload)?;
			Ok(())
		}
		SAVE => {
			SaveRequest::decode(payload)?;
			Ok(())
		}
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without a photo library.
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
		match method {
			STATUS | REQUEST_PERMISSION => {
				Ok(Bytes::copy_from_slice(&Status::Unknown.encode()))
			}
			_ => Err(Error::Unavailable),
		}
	}
}

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct MediaLibrary(leptoskit_bridge::guest::Client);

	impl MediaLibrary {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries authorization without prompting.
		pub async fn status(self) -> Result<Status, Error> {
			Status::decode(&self.0.call(STATUS, &[])?.await?)
		}

		/// May present a permission prompt, then returns status.
		pub async fn request_permission(self) -> Result<Status, Error> {
			Status::decode(&self.0.call(REQUEST_PERMISSION, &[])?.await?)
		}

		/// Lists up to `options.limit` assets. Denied access returns an error,
		/// never invented rows.
		pub async fn list_assets(self, options: ListOptions) -> Result<Vec<Asset>, Error> {
			decode_assets(&self.0.call(LIST_ASSETS, &options.encode())?.await?)
		}

		/// Copies an app-owned file into the system library.
		pub async fn save(self, request: &SaveRequest) -> Result<Asset, Error> {
			Asset::decode(&self.0.call(SAVE, &request.encode()?)?.await?)
		}
	}
}

/// Browsers have no system photo library API in this MVP.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always Unknown; there is no browser grant path here.
	#[must_use]
	pub const fn status() -> Status {
		Status::Unknown
	}

	/// Always Unavailable without inventing assets.
	pub fn request_permission() -> Result<Status, Error> {
		Err(Error::Unavailable)
	}

	/// Always Unavailable.
	pub fn list_assets(_options: ListOptions) -> Result<Vec<Asset>, Error> {
		Err(Error::Unavailable)
	}

	/// Always Unavailable.
	pub fn save(_request: &SaveRequest) -> Result<Asset, Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	fn sample_asset() -> Asset {
		Asset::new(
			"id-1",
			MediaKind::Images,
			"file:///tmp/a.jpg",
			Some(100),
			Some(200),
			Some(1_700_000_000_000),
		)
		.unwrap()
	}

	#[test]
	fn status_rejects_unknown_values_truncation_and_trailing_bytes() {
		for (value, expected) in [
			(0, Status::Unknown),
			(1, Status::Prompt),
			(2, Status::Denied),
			(3, Status::Granted),
			(4, Status::Restricted),
		] {
			assert_eq!(Status::decode(&u32::to_le_bytes(value)), Ok(expected));
		}
		for bytes in [&[][..], &[1], &[1, 0, 0], &[1, 0, 0, 0, 0], &[5, 0, 0, 0]] {
			assert_eq!(Status::decode(bytes), Err(Error::InvalidArgument));
		}
	}

	#[test]
	fn list_options_and_save_round_trip_and_reject_bounds() {
		let options = ListOptions::new(MediaKind::All, 8).unwrap();
		assert_eq!(options.kind(), MediaKind::All);
		assert_eq!(options.limit(), 8);
		assert_eq!(ListOptions::decode(&options.encode()).unwrap(), options);
		assert_eq!(
			ListOptions::new(MediaKind::Images, 0),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			ListOptions::new(MediaKind::Images, MAX_LIST + 1),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			ListOptions::decode(&[9, 0, 0, 0, 1, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);

		let save = SaveRequest::new("file:///tmp/a.jpg", MediaKind::Images).unwrap();
		assert_eq!(save.uri(), "file:///tmp/a.jpg");
		assert_eq!(save.kind(), MediaKind::Images);
		assert_eq!(SaveRequest::decode(&save.encode().unwrap()).unwrap(), save);
		assert_eq!(
			SaveRequest::new("file:///tmp/a.jpg", MediaKind::All),
			Err(Error::InvalidArgument)
		);
		assert_eq!(SaveRequest::new("", MediaKind::Videos), Err(Error::InvalidArgument));
		assert_eq!(
			SaveRequest::new("a\0b", MediaKind::Videos),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			SaveRequest::new("x".repeat(MAX_URI_BYTES + 1), MediaKind::Images),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			SaveRequest::decode(&vec![0_u8; MAX_URI_BYTES + 9]),
			Err(Error::TooLarge)
		);
	}

	#[test]
	fn asset_and_list_codecs_reject_trailers_caps_and_oversize() {
		let asset = sample_asset();
		assert_eq!(Asset::decode(&asset.encode().unwrap()).unwrap(), asset);
		assert_eq!(asset.id(), "id-1");
		assert_eq!(asset.kind(), MediaKind::Images);
		assert_eq!(asset.uri(), "file:///tmp/a.jpg");
		assert_eq!(asset.width(), Some(100));
		assert_eq!(asset.height(), Some(200));
		assert_eq!(asset.creation_time_ms(), Some(1_700_000_000_000));

		let minimal =
			Asset::new("a", MediaKind::Videos, "file:///v", None, None, None).unwrap();
		assert_eq!(Asset::decode(&minimal.encode().unwrap()).unwrap(), minimal);

		assert_eq!(
			Asset::new("a", MediaKind::All, "file:///x", None, None, None),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Asset::new("a\0", MediaKind::Images, "file:///x", None, None, None),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Asset::new("a", MediaKind::Images, "file:///x", Some(u32::MAX), None, None),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Asset::new("a", MediaKind::Images, "file:///x", None, Some(u32::MAX), None),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Asset::new("a", MediaKind::Images, "file:///x", None, None, Some(u64::MAX)),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Asset::new("x".repeat(MAX_ID_BYTES + 1), MediaKind::Images, "file:///x", None, None, None),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Asset::decode(&vec![0_u8; max_asset_frame() + 1]),
			Err(Error::TooLarge)
		);

		let list = vec![asset.clone(), minimal];
		let encoded = encode_assets(&list).unwrap();
		assert_eq!(decode_assets(&encoded).unwrap(), list);
		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert!(decode_assets(&trailing).is_err());
		for end in 0..encoded.len() {
			assert!(decode_assets(&encoded[..end]).is_err());
		}
		let too_many = (0..=MAX_LIST)
			.map(|index| {
				Asset::new(index.to_string(), MediaKind::Images, "file:///x", None, None, None)
					.unwrap()
			})
			.collect::<Vec<_>>();
		assert_eq!(encode_assets(&too_many), Err(Error::TooLarge));
		assert_eq!(
			decode_assets(&vec![0_u8; 5 + max_asset_frame() * MAX_LIST as usize]),
			Err(Error::TooLarge)
		);
		let over_count = (MAX_LIST + 1).to_le_bytes().to_vec();
		assert_eq!(decode_assets(&over_count), Err(Error::TooLarge));
	}

	#[test]
	fn validate_and_unsupported_never_invent_assets() {
		assert_eq!(validate(STATUS, &[]), Ok(()));
		assert_eq!(validate(STATUS, &[0]), Err(Error::InvalidArgument));
		let options = ListOptions::new(MediaKind::Images, 2).unwrap().encode();
		assert_eq!(validate(LIST_ASSETS, &options), Ok(()));
		let save = SaveRequest::new("file:///tmp/a.jpg", MediaKind::Images)
			.unwrap()
			.encode()
			.unwrap();
		assert_eq!(validate(SAVE, &save), Ok(()));
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));

		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			Status::decode(&plugin.invoke(STATUS, Bytes::new()).unwrap()).unwrap(),
			Status::Unknown
		);
		assert_eq!(plugin.invoke(LIST_ASSETS, Bytes::copy_from_slice(&options)), Err(Error::Unavailable));
		assert_eq!(plugin.invoke(SAVE, save), Err(Error::Unavailable));
	}
}
