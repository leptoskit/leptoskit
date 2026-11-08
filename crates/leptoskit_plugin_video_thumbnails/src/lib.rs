//! <!-- {=p_video_thumbnailsSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_video_thumbnails` as a direct dependency and select the
//! `video-thumbnails` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Generated Linux hosts register it where it
//! has a Linux Rust implementation; generated Windows and terminal hosts
//! currently omit runtime plugins, so use explicit registration in a custom
//! host there. Guests enable the `guest` feature and reuse
//! `guest::VideoThumbnails` rather than resolving the plugin on every call.
//! Extract one JPEG frame from an app-owned video URI at a time offset. iOS and
//! macOS use `AVAssetImageGenerator`; Android uses `MediaMetadataRetriever`;
//! Windows, Linux, HarmonyOS, and the browser return `Unavailable`. Multi-frame
//! extraction and custom sizes remain later scope.
//!
//! <!-- {/p_video_thumbnailsSummary} -->
//!
//! Wire v1: get-thumbnail takes `time_ms` (u64), quality 1–100 (u32), and a
//! length-prefixed URI; the reply is width, height, and a length-prefixed temp
//! JPEG URI. All integers are little-endian; trailing bytes are rejected.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "video-thumbnails";
/// [`GetThumbnail`] → encoded [`Thumbnail`].
pub const GET_THUMBNAIL: MethodId = MethodId(0);
/// Supported wire operations.
pub const METHODS: &[MethodId] = &[GET_THUMBNAIL];

/// Maximum UTF-8 bytes for a URI.
pub const MAX_URI_BYTES: usize = 4096;
/// JPEG quality inclusive range.
pub const MIN_QUALITY: u32 = 1;
/// JPEG quality inclusive range.
pub const MAX_QUALITY: u32 = 100;

/// Request to extract one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetThumbnail {
	/// Source video URI inside the app files root (or a readable file URI).
	uri: String,
	/// Presentation time in milliseconds from the start of the video.
	time_ms: u64,
	/// JPEG quality from 1 to 100.
	quality: u32,
}

impl GetThumbnail {
	/// Rejects empty/NUL/oversized URIs, `u64::MAX` time, and out-of-range quality.
	pub fn new(uri: impl Into<String>, time_ms: u64, quality: u32) -> Result<Self, Error> {
		let uri = uri.into();
		if uri.is_empty() || uri.contains('\0') || uri.len() > MAX_URI_BYTES {
			return Err(Error::InvalidArgument);
		}
		if time_ms == u64::MAX {
			return Err(Error::InvalidArgument);
		}
		if !(MIN_QUALITY..=MAX_QUALITY).contains(&quality) {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			uri,
			time_ms,
			quality,
		})
	}

	/// Source video URI.
	#[must_use]
	pub fn uri(&self) -> &str {
		&self.uri
	}

	/// Presentation time in milliseconds.
	#[must_use]
	pub fn time_ms(&self) -> u64 {
		self.time_ms
	}

	/// JPEG quality 1–100.
	#[must_use]
	pub fn quality(&self) -> u32 {
		self.quality
	}

	/// Encodes time, quality, then length-prefixed URI.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(16 + self.uri.len());
		write_u64(&mut output, self.time_ms);
		output.u32(self.quality);
		output.bytes(self.uri.as_bytes())?;
		Ok(output.finish())
	}

	/// Decodes through [`GetThumbnail::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let time_ms = read_u64(&mut input)?;
		let quality = input.u32()?;
		let uri = input.string()?.to_owned();
		input.finish()?;
		Self::new(uri, time_ms, quality)
	}
}

/// Extracted JPEG thumbnail written to a temporary app-owned URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
	/// Temporary JPEG file URI.
	uri: String,
	/// Pixel width.
	width: u32,
	/// Pixel height.
	height: u32,
}

impl Thumbnail {
	/// Rejects empty/NUL/oversized URIs and `u32::MAX` dimensions.
	pub fn new(uri: impl Into<String>, width: u32, height: u32) -> Result<Self, Error> {
		let uri = uri.into();
		if uri.is_empty() || uri.contains('\0') || uri.len() > MAX_URI_BYTES {
			return Err(Error::InvalidArgument);
		}
		if width == u32::MAX || height == u32::MAX || width == 0 || height == 0 {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			uri,
			width,
			height,
		})
	}

	/// Temporary JPEG URI.
	#[must_use]
	pub fn uri(&self) -> &str {
		&self.uri
	}

	/// Pixel width.
	#[must_use]
	pub fn width(&self) -> u32 {
		self.width
	}

	/// Pixel height.
	#[must_use]
	pub fn height(&self) -> u32 {
		self.height
	}

	/// Encodes width, height, then length-prefixed URI.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(12 + self.uri.len());
		output.u32(self.width);
		output.u32(self.height);
		output.bytes(self.uri.as_bytes())?;
		Ok(output.finish())
	}

	/// Decodes through [`Thumbnail::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let width = input.u32()?;
		let height = input.u32()?;
		let uri = input.string()?.to_owned();
		input.finish()?;
		Self::new(uri, width, height)
	}
}

fn write_u64(output: &mut Encoder, value: u64) {
	let bits = value.to_le_bytes();
	output.u32(u32::from_le_bytes(bits[..4].try_into().unwrap()));
	output.u32(u32::from_le_bytes(bits[4..].try_into().unwrap()));
}

fn read_u64(input: &mut Decoder<'_>) -> Result<u64, Error> {
	let low = u64::from(input.u32()?);
	let high = u64::from(input.u32()?);
	Ok(low | (high << 32))
}

/// Validates method payloads without invoking a host.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		GET_THUMBNAIL => GetThumbnail::decode(payload).map(|_| ()),
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without frame extraction.
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

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct VideoThumbnails(leptoskit_bridge::guest::Client);

	impl VideoThumbnails {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Extracts one JPEG frame for `request`.
		pub async fn get_thumbnail(self, request: &GetThumbnail) -> Result<Thumbnail, Error> {
			Thumbnail::decode(&self.0.call(GET_THUMBNAIL, &request.encode()?)?.await?)
		}
	}
}

/// Browsers have no first-party video frame extractor in this MVP.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always Unavailable without inventing a thumbnail.
	pub fn get_thumbnail(_request: &GetThumbnail) -> Result<Thumbnail, Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	#[test]
	fn request_and_thumbnail_codecs_reject_bounds_and_trailers() {
		let request = GetThumbnail::new("file:///tmp/a.mp4", 1_500, 80).unwrap();
		assert_eq!(request.uri(), "file:///tmp/a.mp4");
		assert_eq!(request.time_ms(), 1_500);
		assert_eq!(request.quality(), 80);
		assert_eq!(
			GetThumbnail::decode(&request.encode().unwrap()),
			Ok(request)
		);
		assert_eq!(
			GetThumbnail::new("", 0, 50),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			GetThumbnail::new("a\0b", 0, 50),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			GetThumbnail::new("x".repeat(MAX_URI_BYTES + 1), 0, 50),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			GetThumbnail::new("file:///a", u64::MAX, 50),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			GetThumbnail::new("file:///a", 0, 0),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			GetThumbnail::new("file:///a", 0, 101),
			Err(Error::InvalidArgument)
		);
		let mut trailing = GetThumbnail::new("file:///a", 0, 50)
			.unwrap()
			.encode()
			.unwrap()
			.to_vec();
		trailing.push(0);
		assert_eq!(
			GetThumbnail::decode(&trailing),
			Err(Error::InvalidArgument)
		);

		let thumb = Thumbnail::new("file:///tmp/t.jpg", 320, 180).unwrap();
		assert_eq!(thumb.uri(), "file:///tmp/t.jpg");
		assert_eq!(thumb.width(), 320);
		assert_eq!(thumb.height(), 180);
		assert_eq!(Thumbnail::decode(&thumb.encode().unwrap()), Ok(thumb));
		assert_eq!(
			Thumbnail::new("", 1, 1),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Thumbnail::new("file:///a", 0, 1),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Thumbnail::new("file:///a", u32::MAX, 1),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Thumbnail::new("file:///a", 1, u32::MAX),
			Err(Error::InvalidArgument)
		);
		let mut trailing = Thumbnail::new("file:///a", 1, 1)
			.unwrap()
			.encode()
			.unwrap()
			.to_vec();
		trailing.push(0);
		assert_eq!(Thumbnail::decode(&trailing), Err(Error::InvalidArgument));
	}

	#[test]
	fn validate_and_unsupported_never_invent_thumbnails() {
		let request = GetThumbnail::new("file:///tmp/a.mp4", 0, 90)
			.unwrap()
			.encode()
			.unwrap();
		assert_eq!(validate(GET_THUMBNAIL, &request), Ok(()));
		assert_eq!(validate(GET_THUMBNAIL, &[]), Err(Error::InvalidArgument));
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));
		assert_eq!(METHODS.len(), 1);
		assert_eq!(NAME, "video-thumbnails");

		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			plugin.invoke(GET_THUMBNAIL, request),
			Err(Error::Unavailable)
		);
	}
}
