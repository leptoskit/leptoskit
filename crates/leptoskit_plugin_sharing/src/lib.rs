//! <!-- {=p_sharingSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_sharing` as a direct dependency and select the
//! `sharing` plugin in `native.plugins`; supported generated hosts discover it
//! from Cargo metadata. Generated Linux hosts register it where it has a Linux
//! Rust implementation; generated Windows and terminal hosts currently omit
//! runtime plugins, so use explicit registration in a custom host there. The
//! app supplies text, a web URL, or files from its own files directory or
//! memory; the user picks the destination, and the reply says which one
//! where the platform reports it. It never confirms delivery.
//!
//! <!-- {/p_sharingSummary} -->
//!
//! A share carries text or a web URL, optionally with an email subject and a
//! sheet title, or one or more files with optional text. [`share`] answers
//! with a [`ShareResult`]: the destination the user picked, a dismissal, or
//! `Unavailable` when the platform cannot tell. It never confirms delivery.
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

mod file;
mod reply;

pub use file::File;
pub use file::FileSource;
pub use file::validate_mime;
pub use file::validate_name;
pub use file::validate_path;
pub use reply::ShareResult;

/// Runtime capability name.
pub const NAME: &str = "sharing";
/// Present native share UI; replies with an encoded [`ShareResult`].
pub const SHARE: MethodId = MethodId(0);
/// Version both the request and the reply start with.
pub const WIRE_VERSION: u32 = 1;
/// Maximum UTF-8 text size, before native string conversion.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Maximum ASCII encoded URL length.
pub const MAX_URL_BYTES: usize = 8192;
/// Maximum UTF-8 email subject size.
pub const MAX_SUBJECT_BYTES: usize = 1024;
/// Maximum UTF-8 sheet title size.
pub const MAX_TITLE_BYTES: usize = 1024;
/// Maximum number of files in one share.
pub const MAX_FILES: usize = 32;
/// Maximum UTF-8 size of a path inside the app's files directory.
pub const MAX_PATH_BYTES: usize = 4096;
/// Maximum UTF-8 size of a file's display name.
pub const MAX_FILE_NAME_BYTES: usize = 255;
/// Maximum size of a MIME type.
pub const MAX_MIME_BYTES: usize = 255;
/// Maximum combined size of the in-memory files in one share.
pub const MAX_INLINE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum UTF-8 size of a reply's raw destination identifier.
pub const MAX_RAW_BYTES: usize = 1024;
/// Largest anchor coordinate or extent, in points.
pub const MAX_ANCHOR_COORDINATE: f32 = 1_000_000.0;
/// Number of [`AppleActivity`] codes.
const ACTIVITY_COUNT: usize = 20;
/// Upper bound on one encoded request, which every native decoder checks first.
pub const MAX_REQUEST_BYTES: usize = 4 // version
	+ 4 + MAX_TEXT_BYTES
	+ 4 + MAX_URL_BYTES
	+ 4 + MAX_SUBJECT_BYTES
	+ 4 + MAX_TITLE_BYTES
	+ 4 + 16 // anchor flag and rectangle
	+ 4 + 4 * ACTIVITY_COUNT
	+ 4 + MAX_FILES * (4 + 4 + MAX_PATH_BYTES + 4 + MAX_MIME_BYTES + 4 + MAX_FILE_NAME_BYTES)
	+ MAX_INLINE_BYTES;

/// Where an iPad popover or the macOS sharing picker points, in points within
/// the app's root view, origin top left. Other platforms ignore it. iPhone
/// presents a sheet that needs no anchor; iPad and macOS use the middle of
/// the view without one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
	x: f32,
	y: f32,
	width: f32,
	height: f32,
}

impl Anchor {
	/// A finite, non-negative rectangle no larger than
	/// [`MAX_ANCHOR_COORDINATE`] in any value. Native hosts refuse an origin
	/// outside the root view and clip the size to it.
	pub fn new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, Error> {
		if [x, y, width, height]
			.iter()
			.all(|value| (0.0..=MAX_ANCHOR_COORDINATE).contains(value))
		{
			Ok(Self {
				x,
				y,
				width,
				height,
			})
		} else {
			Err(Error::InvalidArgument)
		}
	}

	/// The left edge.
	pub fn x(&self) -> f32 {
		self.x
	}

	/// The top edge.
	pub fn y(&self) -> f32 {
		self.y
	}

	/// The width.
	pub fn width(&self) -> f32 {
		self.width
	}

	/// The height.
	pub fn height(&self) -> f32 {
		self.height
	}
}

/// iOS share-sheet activities an app can leave out, as in `share_plus`'s
/// `CupertinoActivityType`. Only iOS honours exclusions; macOS's sharing
/// picker and other platforms ignore them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
#[allow(missing_docs)]
pub enum AppleActivity {
	PostToFacebook = 0,
	PostToTwitter = 1,
	PostToWeibo = 2,
	Message = 3,
	Mail = 4,
	Print = 5,
	CopyToPasteboard = 6,
	AssignToContact = 7,
	SaveToCameraRoll = 8,
	AddToReadingList = 9,
	PostToFlickr = 10,
	PostToVimeo = 11,
	PostToTencentWeibo = 12,
	AirDrop = 13,
	OpenInIBooks = 14,
	MarkupAsPdf = 15,
	SharePlay = 16,
	CollaborationInviteWithLink = 17,
	CollaborationCopyLink = 18,
	AddToHomeScreen = 19,
}

impl AppleActivity {
	/// Every activity, in wire-code order.
	pub const ALL: [Self; ACTIVITY_COUNT] = [
		Self::PostToFacebook,
		Self::PostToTwitter,
		Self::PostToWeibo,
		Self::Message,
		Self::Mail,
		Self::Print,
		Self::CopyToPasteboard,
		Self::AssignToContact,
		Self::SaveToCameraRoll,
		Self::AddToReadingList,
		Self::PostToFlickr,
		Self::PostToVimeo,
		Self::PostToTencentWeibo,
		Self::AirDrop,
		Self::OpenInIBooks,
		Self::MarkupAsPdf,
		Self::SharePlay,
		Self::CollaborationInviteWithLink,
		Self::CollaborationCopyLink,
		Self::AddToHomeScreen,
	];

	/// The wire code.
	pub const fn code(self) -> u32 {
		self as u32
	}

	/// The activity for a wire code.
	pub fn from_code(code: u32) -> Option<Self> {
		Self::ALL.get(usize::try_from(code).ok()?).copied()
	}
}

/// Validated share content with private representation, shared across native
/// and web.
///
/// It holds text or a web URL (never both), or files with optional text, plus
/// an optional subject, title, popover anchor and excluded iOS activities.
#[derive(Debug, Clone, PartialEq)]
pub struct Content {
	text: Option<String>,
	url: Option<String>,
	subject: Option<String>,
	title: Option<String>,
	anchor: Option<Anchor>,
	excluded: Vec<AppleActivity>,
	files: Vec<File>,
}

impl Content {
	const EMPTY: Self = Self {
		text: None,
		url: None,
		subject: None,
		title: None,
		anchor: None,
		excluded: Vec::new(),
		files: Vec::new(),
	};

	/// Preserves nonempty UTF-8 text, including newlines. NUL is rejected.
	pub fn text(value: impl Into<String>) -> Result<Self, Error> {
		Self::EMPTY.with_text(value)
	}

	/// Accepts only absolute HTTP(S) URLs without credentials, controls,
	/// whitespace, backslashes, or malformed percent escapes. Percent-encode
	/// Unicode before calling. Native URL parsers may reject a stricter subset,
	/// such as encoded hostnames or abbreviated IPv4 addresses on Android.
	pub fn url(value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();
		validate_url(&value)?;
		Ok(Self {
			url: Some(value),
			..Self::EMPTY
		})
	}

	/// One to [`MAX_FILES`] files, whose in-memory contents together hold at
	/// most [`MAX_INLINE_BYTES`]. Add text with [`Content::with_text`].
	pub fn files(files: impl IntoIterator<Item = File>) -> Result<Self, Error> {
		let files: Vec<_> = files.into_iter().take(MAX_FILES + 1).collect();

		if files.len() > MAX_FILES {
			return Err(Error::TooLarge);
		}

		if files.is_empty() {
			return Err(Error::InvalidArgument);
		}

		if files.iter().map(File::inline_len).sum::<usize>() > MAX_INLINE_BYTES {
			return Err(Error::TooLarge);
		}

		Ok(Self {
			files,
			..Self::EMPTY
		})
	}

	/// Sets the text: the whole message, or a caption alongside files. A URL
	/// share cannot also carry text; put the URL in the text instead.
	pub fn with_text(mut self, value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();

		if value.len() > MAX_TEXT_BYTES {
			return Err(Error::TooLarge);
		}

		if self.url.is_some() || value.is_empty() || value.contains('\0') {
			return Err(Error::InvalidArgument);
		}

		self.text = Some(value);
		Ok(self)
	}

	/// The email subject, where a destination uses one (Android
	/// `EXTRA_SUBJECT`, iOS and macOS mail subject).
	pub fn with_subject(mut self, value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();
		validate_label(&value, MAX_SUBJECT_BYTES)?;
		self.subject = Some(value);
		Ok(self)
	}

	/// The share sheet's title: Android's chooser title and `EXTRA_TITLE`, the
	/// iOS preview title, and the browser's `title`.
	pub fn with_title(mut self, value: impl Into<String>) -> Result<Self, Error> {
		let value = value.into();
		validate_label(&value, MAX_TITLE_BYTES)?;
		self.title = Some(value);
		Ok(self)
	}

	/// Points an iPad popover or the macOS picker at this rectangle.
	#[must_use]
	pub fn with_anchor(mut self, anchor: Anchor) -> Self {
		self.anchor = Some(anchor);
		self
	}

	/// Leaves an iOS activity out of the sheet. Repeats are ignored.
	#[must_use]
	pub fn excluding(mut self, activity: AppleActivity) -> Self {
		if !self.excluded.contains(&activity) {
			self.excluded.push(activity);
		}

		self
	}

	/// The text, when there is one.
	pub fn shared_text(&self) -> Option<&str> {
		self.text.as_deref()
	}

	/// The validated web URL, when this is a URL share.
	pub fn shared_url(&self) -> Option<&str> {
		self.url.as_deref()
	}

	/// The email subject.
	pub fn subject(&self) -> Option<&str> {
		self.subject.as_deref()
	}

	/// The sheet title.
	pub fn title(&self) -> Option<&str> {
		self.title.as_deref()
	}

	/// The popover anchor.
	pub fn anchor(&self) -> Option<Anchor> {
		self.anchor
	}

	/// The excluded iOS activities, in the order they were added.
	pub fn excluded(&self) -> &[AppleActivity] {
		&self.excluded
	}

	/// The shared files.
	pub fn shared_files(&self) -> &[File] {
		&self.files
	}

	/// Wire: little-endian [`WIRE_VERSION`]; length-prefixed text, URL,
	/// subject and title (length 0 when absent); an anchor flag and four
	/// `f32` bit patterns; a count of excluded activity codes; then a count of
	/// files, each a source code (0 app path, 1 bytes), its length-prefixed
	/// path or bytes, MIME type and display name.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut encoder = Encoder::with_capacity(self.encoded_len());
		encoder.u32(WIRE_VERSION);

		for value in [&self.text, &self.url, &self.subject, &self.title] {
			encoder.bytes(value.as_deref().unwrap_or_default().as_bytes())?;
		}

		match self.anchor {
			Some(anchor) => {
				encoder.u32(1);

				for value in [anchor.x, anchor.y, anchor.width, anchor.height] {
					encoder.u32(value.to_bits());
				}
			}
			None => encoder.u32(0),
		}

		encoder.u32(count(self.excluded.len())?);

		for activity in &self.excluded {
			encoder.u32(activity.code());
		}

		encoder.u32(count(self.files.len())?);

		for file in &self.files {
			encoder.u32(file.source().code());
			encoder.bytes(file.source().payload())?;
			encoder.bytes(file.mime().unwrap_or_default().as_bytes())?;
			encoder.bytes(file.name().unwrap_or_default().as_bytes())?;
		}

		Ok(encoder.finish())
	}

	fn encoded_len(&self) -> usize {
		let labels = [&self.text, &self.url, &self.subject, &self.title]
			.iter()
			.map(|value| 4 + value.as_deref().map_or(0, str::len))
			.sum::<usize>();
		let files = self
			.files
			.iter()
			.map(|file| {
				16 + file.source().payload().len()
					+ file.mime().map_or(0, str::len)
					+ file.name().map_or(0, str::len)
			})
			.sum::<usize>();

		4 + labels + 20 + 4 + 4 * self.excluded.len() + 4 + files
	}

	/// Validates untrusted guest payloads with every constructor rule;
	/// rejects unknown versions, kinds and codes, and trailing data.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_REQUEST_BYTES {
			return Err(Error::TooLarge);
		}

		let mut decoder = Decoder::new(payload);

		if decoder.u32()? != WIRE_VERSION {
			return Err(Error::InvalidArgument);
		}

		let text = optional(decoder.string()?);
		let url = optional(decoder.string()?);
		let subject = optional(decoder.string()?);
		let title = optional(decoder.string()?);
		let anchor = decode_anchor(&mut decoder)?;
		let excluded = decode_excluded(&mut decoder)?;
		let files = decode_files(&mut decoder)?;
		decoder.finish()?;

		let mut content = match (url, text, files.is_empty()) {
			(Some(url), None, true) => Self::url(url)?,
			(None, Some(text), true) => Self::text(text)?,
			(None, text, false) => {
				let content = Self::files(files)?;

				match text {
					Some(text) => content.with_text(text)?,
					None => content,
				}
			}
			// Nothing to share, or a URL alongside text or files.
			_ => return Err(Error::InvalidArgument),
		};

		if let Some(subject) = subject {
			content = content.with_subject(subject)?;
		}

		if let Some(title) = title {
			content = content.with_title(title)?;
		}

		content.anchor = anchor;
		content.excluded = excluded;
		Ok(content)
	}
}

fn optional(value: &str) -> Option<&str> {
	(!value.is_empty()).then_some(value)
}

fn count(len: usize) -> Result<u32, Error> {
	u32::try_from(len).map_err(|_| Error::TooLarge)
}

fn decode_anchor(decoder: &mut Decoder<'_>) -> Result<Option<Anchor>, Error> {
	match decoder.u32()? {
		0 => Ok(None),
		1 => {
			let mut values = [0.0; 4];

			for value in &mut values {
				*value = f32::from_bits(decoder.u32()?);
			}

			let [x, y, width, height] = values;
			Anchor::new(x, y, width, height).map(Some)
		}
		_ => Err(Error::InvalidArgument),
	}
}

fn decode_excluded(decoder: &mut Decoder<'_>) -> Result<Vec<AppleActivity>, Error> {
	let count = decoder.u32()? as usize;

	if count > ACTIVITY_COUNT {
		return Err(Error::TooLarge);
	}

	let mut excluded = Vec::with_capacity(count);

	for _ in 0..count {
		let activity = AppleActivity::from_code(decoder.u32()?).ok_or(Error::InvalidArgument)?;

		if excluded.contains(&activity) {
			return Err(Error::InvalidArgument);
		}

		excluded.push(activity);
	}

	Ok(excluded)
}

fn decode_files(decoder: &mut Decoder<'_>) -> Result<Vec<File>, Error> {
	let count = decoder.u32()? as usize;

	if count > MAX_FILES {
		return Err(Error::TooLarge);
	}

	let mut files = Vec::with_capacity(count);

	for _ in 0..count {
		let source = decoder.u32()?;
		let payload = decoder.bytes()?;
		let mime = optional(decoder.string()?);
		let name = optional(decoder.string()?);
		files.push(File::from_wire(source, payload, mime, name)?);
	}

	Ok(files)
}

/// A subject or title: nonempty UTF-8 without control characters.
fn validate_label(value: &str, limit: usize) -> Result<(), Error> {
	if value.len() > limit {
		return Err(Error::TooLarge);
	}

	if value.is_empty() || value.chars().any(char::is_control) {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

fn validate_url(value: &str) -> Result<(), Error> {
	if value.len() > MAX_URL_BYTES {
		return Err(Error::TooLarge);
	}

	require_authority(value)?;
	require_graphic(value)?;
	require_escapes(value)?;
	let parsed = url::Url::parse(value).map_err(|_| Error::InvalidArgument)?;

	if parsed.host_str().is_none() || !parsed.username().is_empty() || parsed.password().is_some() {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// The authority is everything between the scheme and the first path, query, or
/// fragment. An empty or credentialed one is not a shareable destination.
fn require_authority(value: &str) -> Result<(), Error> {
	let authority = value
		.strip_prefix("https://")
		.or_else(|| value.strip_prefix("http://"))
		.ok_or(Error::InvalidArgument)?
		.split(['/', '?', '#'])
		.next()
		.ok_or(Error::InvalidArgument)?;

	if authority.is_empty() || authority.contains('@') {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// Whitespace, controls, and backslashes never belong in a shared URL.
fn require_graphic(value: &str) -> Result<(), Error> {
	if value
		.bytes()
		.any(|byte| !byte.is_ascii_graphic() || byte == b'\\')
	{
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// A percent sign must introduce two hexadecimal digits.
fn require_escapes(value: &str) -> Result<(), Error> {
	for (index, byte) in value.bytes().enumerate() {
		if byte != b'%' {
			continue;
		}

		let pair = value.as_bytes().get(index + 1..index + 3);

		if !pair.is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit)) {
			return Err(Error::InvalidArgument);
		}
	}

	Ok(())
}

/// Opens the installed native share UI and reports how it ended; never
/// confirms message delivery.
#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub async fn share(content: &Content) -> Result<ShareResult, Error> {
	let reply = leptoskit_bridge::guest::Client::new(NAME)?
		.call(SHARE, &content.encode()?)?
		.await?;

	ShareResult::decode(&reply)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod web;

#[cfg(test)]
mod tests;
