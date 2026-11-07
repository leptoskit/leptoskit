//! Files a guest shares: a path inside the app's own files directory, or bytes
//! the native host writes to a private share cache.
use crate::Error;
use crate::MAX_FILE_NAME_BYTES;
use crate::MAX_INLINE_BYTES;
use crate::MAX_MIME_BYTES;
use crate::MAX_PATH_BYTES;

/// Reserved prefix of the filesystem plugin's in-progress writes; never a
/// finished file a guest could mean to share.
const TEMP_PREFIX: &str = ".leptoskit-tmp-";

/// Where a shared file's contents come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileSource {
	/// A path relative to the app's files directory: the same root the
	/// `filesystem` plugin reads and writes (`<data>/files`). The native host
	/// resolves it there, refuses anything that leaves that directory (parent
	/// segments, symlinks), and copies the regular file into a private share
	/// cache before presenting it.
	App(String),
	/// Contents the guest supplies. The native host writes them to a private
	/// share cache under the file's name.
	Bytes(Vec<u8>),
}

impl FileSource {
	pub(crate) const fn code(&self) -> u32 {
		match self {
			Self::App(_) => 0,
			Self::Bytes(_) => 1,
		}
	}

	pub(crate) fn payload(&self) -> &[u8] {
		match self {
			Self::App(path) => path.as_bytes(),
			Self::Bytes(bytes) => bytes,
		}
	}
}

/// One shared file, with an optional MIME type and display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
	source: FileSource,
	mime: Option<String>,
	name: Option<String>,
}

impl File {
	/// A regular file inside the app's files directory, named by a relative
	/// path such as `exports/report.pdf`. Absolute paths, `.`/`..` segments,
	/// empty segments, alternate separators and reserved names are refused,
	/// with the same rules as the `filesystem` plugin.
	pub fn app(path: impl Into<String>) -> Result<Self, Error> {
		let path = path.into();
		validate_path(&path)?;
		Ok(Self {
			source: FileSource::App(path),
			mime: None,
			name: None,
		})
	}

	/// In-memory contents shared under `name`, such as `photo.png`. All the
	/// in-memory files of one share together hold at most
	/// [`MAX_INLINE_BYTES`].
	pub fn bytes(name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Result<Self, Error> {
		let bytes = bytes.into();

		if bytes.len() > MAX_INLINE_BYTES {
			return Err(Error::TooLarge);
		}

		let name = name.into();
		validate_name(&name)?;
		Ok(Self {
			source: FileSource::Bytes(bytes),
			mime: None,
			name: Some(name),
		})
	}

	/// The MIME type receivers see, such as `image/png` or `image/*`.
	/// Parameters (`; charset=…`) are refused.
	pub fn with_mime(mut self, mime: impl Into<String>) -> Result<Self, Error> {
		let mime = mime.into();
		validate_mime(&mime)?;
		self.mime = Some(mime);
		Ok(self)
	}

	/// The file name receivers see, replacing the path's last segment.
	pub fn with_name(mut self, name: impl Into<String>) -> Result<Self, Error> {
		let name = name.into();
		validate_name(&name)?;
		self.name = Some(name);
		Ok(self)
	}

	/// Where the contents come from.
	pub fn source(&self) -> &FileSource {
		&self.source
	}

	/// The MIME type, when one was given.
	pub fn mime(&self) -> Option<&str> {
		self.mime.as_deref()
	}

	/// The display name, when one was given. In-memory files always have one.
	pub fn name(&self) -> Option<&str> {
		self.name.as_deref()
	}

	pub(crate) fn inline_len(&self) -> usize {
		match &self.source {
			FileSource::App(_) => 0,
			FileSource::Bytes(bytes) => bytes.len(),
		}
	}

	/// Rebuilds a decoded file, applying every constructor rule.
	pub(crate) fn from_wire(
		source: u32,
		payload: &[u8],
		mime: Option<&str>,
		name: Option<&str>,
	) -> Result<Self, Error> {
		let file = match source {
			0 => {
				let path = std::str::from_utf8(payload).map_err(|_| Error::InvalidArgument)?;
				let file = Self::app(path)?;

				match name {
					Some(name) => file.with_name(name)?,
					None => file,
				}
			}
			1 => Self::bytes(name.ok_or(Error::InvalidArgument)?, payload)?,
			_ => return Err(Error::InvalidArgument),
		};

		match mime {
			Some(mime) => file.with_mime(mime),
			None => Ok(file),
		}
	}
}

/// Validates a portable relative path inside the app's files directory, with
/// the `filesystem` plugin's rules. Native hosts check it again and resolve
/// it against the real directory.
pub fn validate_path(path: &str) -> Result<(), Error> {
	if path.len() > MAX_PATH_BYTES {
		return Err(Error::TooLarge);
	}

	if path.is_empty() || path.split('/').any(|part| !valid_component(part)) {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// A display name is one path segment of at most [`MAX_FILE_NAME_BYTES`].
pub fn validate_name(name: &str) -> Result<(), Error> {
	if name.len() > MAX_FILE_NAME_BYTES {
		return Err(Error::TooLarge);
	}

	if name.contains('/') || !valid_component(name) {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// One segment: nonempty, no dot segment, no control or reserved character, no
/// trailing dot or space, no Windows device name, no reserved temporary name.
fn valid_component(part: &str) -> bool {
	let basename = part
		.split('.')
		.next()
		.unwrap_or_default()
		.to_ascii_uppercase();
	let device = matches!(basename.as_str(), "CON" | "PRN" | "AUX" | "NUL")
		|| (basename.len() == 4
			&& (basename.starts_with("COM") || basename.starts_with("LPT"))
			&& matches!(basename.as_bytes()[3], b'1'..=b'9'));

	!(part.is_empty()
		|| part == "."
		|| part == ".."
		|| part.ends_with(['.', ' '])
		|| part.starts_with(TEMP_PREFIX)
		|| device
		|| part
			.chars()
			.any(|ch| ch.is_control() || ['\\', ':', '<', '>', '"', '|', '?', '*'].contains(&ch)))
}

/// `type/subtype` with RFC 6838 name characters, `*` as a whole subtype, or
/// `*/*`. No parameters, no whitespace.
pub fn validate_mime(mime: &str) -> Result<(), Error> {
	if mime.len() > MAX_MIME_BYTES {
		return Err(Error::TooLarge);
	}

	let Some((kind, subtype)) = mime.split_once('/') else {
		return Err(Error::InvalidArgument);
	};
	let valid = match (kind, subtype) {
		("*", "*") => true,
		("*", _) => false,
		(kind, "*") => mime_name(kind),
		(kind, subtype) => mime_name(kind) && mime_name(subtype),
	};

	if valid {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}

/// An RFC 6838 `restricted-name`: an alphanumeric first character, then at
/// most 126 of the allowed punctuation or alphanumerics.
fn mime_name(name: &str) -> bool {
	let mut bytes = name.bytes();

	name.len() <= 127
		&& bytes
			.next()
			.is_some_and(|byte| byte.is_ascii_alphanumeric())
		&& bytes.all(|byte| byte.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&byte))
}
