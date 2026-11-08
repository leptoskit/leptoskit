//! Field rules shared by the encoder, the decoder and the Kotlin plugin.
//!
//! Each rule checks a value whose length the reader already bounded, so a
//! rule only ever reports [`Error::InvalidArgument`] or, for the URI
//! permission policy, [`Error::PermissionDenied`].

use leptoskit_bridge::Error;

use crate::GRANT_URI_PERMISSION_FLAGS;

/// Actions, categories and other free-form identifiers: printable ASCII
/// with no whitespace, which every platform constant satisfies.
pub(crate) fn name(value: &str) -> Result<(), Error> {
	if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// An Android application ID: dot-separated segments that each start with a
/// letter and continue with letters, digits or underscores. A single
/// segment such as `android` names a platform package.
pub(crate) fn package(value: &str) -> Result<(), Error> {
	if value.is_empty() || !value.split('.').all(package_segment) {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

fn package_segment(segment: &str) -> bool {
	let mut bytes = segment.bytes();

	bytes
		.next()
		.is_some_and(|first| first.is_ascii_alphabetic())
		&& bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// A Java class name. A leading `.` makes it relative to the intent's
/// package, as `ComponentName.createRelative` and the manifest allow.
pub(crate) fn class(value: &str) -> Result<(), Error> {
	let absolute = value.strip_prefix('.').unwrap_or(value);

	if absolute.is_empty() || !absolute.split('.').all(class_segment) {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

fn class_segment(segment: &str) -> bool {
	let mut bytes = segment.bytes();
	let identifier = |byte: u8| byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$';

	bytes.next().is_some_and(identifier)
		&& bytes.all(|byte| identifier(byte) || byte.is_ascii_digit())
}

/// A lowercase `type/subtype` MIME type. Android matches MIME types
/// case-sensitively, so an uppercase type would silently match nothing.
/// `*` is the wildcard Android's resolver accepts.
pub(crate) fn mime(value: &str) -> Result<(), Error> {
	let Some((kind, subtype)) = value.split_once('/') else {
		return Err(Error::InvalidArgument);
	};

	if !mime_token(kind) || !mime_token(subtype) {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

fn mime_token(token: &str) -> bool {
	!token.is_empty()
		&& token.bytes().all(|byte| {
			byte.is_ascii_lowercase()
				|| byte.is_ascii_digit()
				|| matches!(
					byte,
					b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-' | b'*'
				)
		})
}

/// An absolute URI with a valid scheme and a non-empty remainder, carried
/// as printable ASCII with well-formed escapes that never decode to a
/// control character. `file:` URIs are refused: Android throws
/// `FileUriExposedException` for them, and they would name private paths.
pub(crate) fn data(value: &str) -> Result<(), Error> {
	encoded(value)?;
	let (scheme, rest) = value.split_once(':').ok_or(Error::InvalidArgument)?;

	if !scheme_syntax(scheme) || rest.is_empty() {
		return Err(Error::InvalidArgument);
	}

	if scheme.eq_ignore_ascii_case("file") {
		return Err(Error::PermissionDenied);
	}

	Ok(())
}

fn scheme_syntax(scheme: &str) -> bool {
	let mut bytes = scheme.bytes();

	bytes
		.next()
		.is_some_and(|first| first.is_ascii_alphabetic())
		&& bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

/// Printable ASCII without backslashes, with every `%` starting a two-digit
/// escape that does not decode to a control character.
pub(crate) fn encoded(value: &str) -> Result<(), Error> {
	let bytes = value.as_bytes();

	if bytes.is_empty()
		|| !bytes
			.iter()
			.all(|byte| byte.is_ascii_graphic() && *byte != b'\\')
	{
		return Err(Error::InvalidArgument);
	}

	for (index, byte) in bytes.iter().enumerate() {
		if *byte == b'%' {
			escape(bytes, index)?;
		}
	}

	Ok(())
}

fn escape(bytes: &[u8], index: usize) -> Result<(), Error> {
	let digits = bytes
		.get(index + 1..index + 3)
		.ok_or(Error::InvalidArgument)?;

	if !digits.iter().all(u8::is_ascii_hexdigit)
		|| matches!(digits[0], b'0' | b'1')
		|| digits.eq_ignore_ascii_case(b"7f")
	{
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// An `intent:` URI in the form `Intent.toUri(URI_INTENT_SCHEME)` writes,
/// with its `#Intent;…;end` fragment.
pub(crate) fn intent_uri(value: &str) -> Result<(), Error> {
	encoded(value)?;

	if !value.starts_with("intent:") || !value.contains("#Intent;") || !value.ends_with(";end") {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// Extra keys are any non-empty text without control characters.
pub(crate) fn key(value: &str) -> Result<(), Error> {
	if value.is_empty() {
		return Err(Error::InvalidArgument);
	}

	plain_text(value)
}

/// Chooser titles and keys must not carry control characters.
pub(crate) fn plain_text(value: &str) -> Result<(), Error> {
	if value.chars().any(char::is_control) {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// Refuses the flags that would hand another app access to URIs this app
/// can read, such as files behind its own `FileProvider`.
pub(crate) fn flags(value: u32) -> Result<(), Error> {
	if value & GRANT_URI_PERMISSION_FLAGS != 0 {
		return Err(Error::PermissionDenied);
	}

	Ok(())
}
