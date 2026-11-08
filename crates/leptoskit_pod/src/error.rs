//! Errors from encoding or decoding LK Pod bytes.

use thiserror::Error;

/// A failure while reading or writing Pod bytes.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
	/// The buffer ended before a field was complete.
	#[error("truncated pod buffer")]
	Truncated,
	/// Bytes remained after a complete value was read.
	#[error("trailing bytes after pod value")]
	Trailing,
	/// A length or count could not fit in `u32` / `usize`.
	#[error("pod value too large")]
	TooLarge,
	/// A string was not valid UTF-8.
	#[error("invalid utf-8 in pod string")]
	InvalidUtf8,
	/// An option, result, or enum tag was outside the allowed set.
	#[error("invalid pod tag {0}")]
	InvalidTag(u16),
	/// The transport envelope magic or version was wrong.
	#[error("invalid pod envelope")]
	InvalidEnvelope,
	/// A struct discriminator is not known to this decoder.
	#[error("unknown pod struct discriminator {0}")]
	UnknownDiscriminator(u16),
	/// A struct version is not known to this decoder.
	#[error("unknown pod struct version {0}")]
	UnknownVersion(u16),
}

/// Result alias for codec operations.
pub type Result<T> = std::result::Result<T, Error>;
