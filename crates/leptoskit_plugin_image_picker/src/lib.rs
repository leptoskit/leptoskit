//! <!-- {=p_image_pickerSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_image_picker` as a direct dependency and select the
//! `image-picker` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Generated Windows and terminal hosts
//! currently omit runtime plugins, so use explicit registration in a custom
//! host there. The user picks one library image or video; the app receives a
//! temporary URI and metadata under a size limit, never a persistent gallery
//! grant. Camera capture is deferred to a later `camera` plugin.
//!
//! <!-- {/p_image_pickerSummary} -->
//!
//! Request: media kind (`u32`) then max bytes (`u32`). Reply: length-prefixed
//! UTF-8 URI, length-prefixed MIME (empty when unknown), width, height
//! (`u32::MAX` unknown), and size as little-endian `u64` (`u64::MAX` unknown).

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime capability name.
pub const NAME: &str = "image-picker";
/// Present the library picker and return one temporary asset.
pub const PICK: MethodId = MethodId(0);
/// Maximum selected file size the MVP will copy into app cache.
pub const MAX_BYTES: u32 = 16 * 1024 * 1024;
/// Maximum UTF-8 bytes in the temporary URI.
pub const MAX_URI_BYTES: usize = 4096;
/// Maximum UTF-8 bytes in the MIME type.
pub const MAX_MIME_BYTES: usize = 256;

/// Which library media kinds the picker may return.
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

/// Validated picker request; zero allows only empty files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
	media: MediaKind,
	max_bytes: u32,
}

impl Options {
	/// Rejects limits above the plugin's fixed allocation budget.
	pub fn new(media: MediaKind, max_bytes: u32) -> Result<Self, Error> {
		if max_bytes > MAX_BYTES {
			return Err(Error::TooLarge);
		}
		Ok(Self { media, max_bytes })
	}

	/// Images-only request at the default size budget.
	pub fn images() -> Self {
		Self {
			media: MediaKind::Images,
			max_bytes: MAX_BYTES,
		}
	}

	/// Media kinds the picker may return.
	pub fn media(self) -> MediaKind {
		self.media
	}

	/// Maximum selected content bytes.
	pub fn max_bytes(self) -> u32 {
		self.max_bytes
	}

	/// Stable request wire representation.
	pub fn encode(self) -> [u8; 8] {
		let mut bytes = [0_u8; 8];
		bytes[..4].copy_from_slice(&(self.media as u32).to_le_bytes());
		bytes[4..].copy_from_slice(&self.max_bytes.to_le_bytes());
		bytes
	}

	/// Validates a host/guest request payload.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let media = MediaKind::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;
		let max_bytes = input.u32()?;
		input.finish()?;
		Self::new(media, max_bytes)
	}
}

impl Default for Options {
	fn default() -> Self {
		Self::images()
	}
}

/// One temporary library asset with no retained gallery grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
	/// App-owned temporary file URI (`file:` / `blob:`). Not a gallery
	/// bookmark.
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
	pub fn decode(bytes: &[u8], options: Options) -> Result<Self, Error> {
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
			&& size > u64::from(options.max_bytes())
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

/// Selects using the native system picker registered by the host.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub async fn pick(options: Options) -> Result<Asset, Error> {
	let client = leptoskit_bridge::guest::Client::new(NAME)?;
	let reply = client.call(PICK, &options.encode())?.await?;
	Asset::decode(&reply, options)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod web;

#[cfg(test)]
mod tests {
	use leptoskit_bridge::codec::Encoder;

	use super::*;

	#[test]
	fn options_encode_decode_and_reject_oversize() {
		assert_eq!(
			Options::new(MediaKind::All, MAX_BYTES + 1),
			Err(Error::TooLarge)
		);
		for media in [MediaKind::Images, MediaKind::Videos, MediaKind::All] {
			let options = Options::new(media, 1024).unwrap();
			assert_eq!(Options::decode(&options.encode()), Ok(options));
			assert_eq!(options.media(), media);
			assert_eq!(options.max_bytes(), 1024);
		}
		assert_eq!(Options::default().media(), MediaKind::Images);
		assert_eq!(Options::default().max_bytes(), MAX_BYTES);
		assert_eq!(Options::images(), Options::default());
		assert_eq!(MediaKind::from_code(9), None);
		assert!(Options::decode(&[1, 2, 3]).is_err());
		assert!(
			Options::decode(&{
				let mut bad = Options::images().encode().to_vec();
				bad[0] = 9;
				bad
			})
			.is_err()
		);
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
		assert_eq!(
			Asset::decode(&bytes, Options::new(MediaKind::Images, 12).unwrap()),
			Ok(asset.clone())
		);
		assert_eq!(
			Asset::decode(&bytes, Options::new(MediaKind::Images, 11).unwrap()),
			Err(Error::TooLarge)
		);

		for len in 0..bytes.len() {
			assert!(Asset::decode(&bytes[..len], Options::default()).is_err());
		}
		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert!(Asset::decode(&trailing, Options::default()).is_err());
	}

	#[test]
	fn rejects_empty_uri_reserved_unknowns_and_oversized_metadata() {
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
		assert_eq!(
			Asset {
				uri: "a".repeat(MAX_URI_BYTES + 1),
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
				mime_type: Some("m".repeat(MAX_MIME_BYTES + 1)),
				width: None,
				height: None,
				size_bytes: None,
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn decode_rejects_empty_or_oversized_uri_and_mime() {
		let options = Options::default();
		let mut empty_uri = Encoder::with_capacity(32);
		empty_uri.bytes(b"").unwrap();
		empty_uri.bytes(b"image/png").unwrap();
		empty_uri.u32(u32::MAX);
		empty_uri.u32(u32::MAX);
		empty_uri.u32(u32::MAX);
		empty_uri.u32(u32::MAX);
		assert_eq!(
			Asset::decode(&empty_uri.finish(), options),
			Err(Error::InvalidArgument)
		);

		let mut long_uri = Encoder::with_capacity(64);
		long_uri
			.bytes("u".repeat(MAX_URI_BYTES + 1).as_bytes())
			.unwrap();
		long_uri.bytes(b"").unwrap();
		long_uri.u32(1);
		long_uri.u32(1);
		long_uri.u32(0);
		long_uri.u32(0);
		assert_eq!(
			Asset::decode(&long_uri.finish(), options),
			Err(Error::InvalidArgument)
		);

		let mut long_mime = Encoder::with_capacity(64);
		long_mime.bytes(b"file:///ok").unwrap();
		long_mime
			.bytes("m".repeat(MAX_MIME_BYTES + 1).as_bytes())
			.unwrap();
		long_mime.u32(1);
		long_mime.u32(1);
		long_mime.u32(0);
		long_mime.u32(0);
		assert_eq!(
			Asset::decode(&long_mime.finish(), options),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn unknown_dimensions_and_size_round_trip() {
		let asset = Asset {
			uri: "blob:example".into(),
			mime_type: None,
			width: None,
			height: None,
			size_bytes: None,
		};
		assert_eq!(
			Asset::decode(&asset.encode().unwrap(), Options::default()),
			Ok(asset)
		);
	}
}
