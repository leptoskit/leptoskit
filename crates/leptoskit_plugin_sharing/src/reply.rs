//! What the platform reported about the user's choice.
use leptoskit_bridge::Bytes;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

use crate::Error;
use crate::MAX_RAW_BYTES;
use crate::WIRE_VERSION;

/// Reply framing: version, status, and the length of the raw identifier.
const FRAMING_BYTES: usize = 12;

/// How the share sheet ended. It never confirms that content was delivered:
/// a chosen app can still discard it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareResult {
	/// The user picked a destination. The value is the platform's raw
	/// identifier for it: the iOS activity type (such as
	/// `com.apple.UIKit.activity.Mail`), the macOS sharing service's title, or
	/// the Android component (`package/class`) the chooser started.
	Success(String),
	/// The user closed the sheet without choosing a destination.
	Dismissed,
	/// The platform shared the content but cannot say what the user chose,
	/// as with the browser Web Share API.
	Unavailable,
}

impl ShareResult {
	const fn code(&self) -> u32 {
		match self {
			Self::Success(_) => 0,
			Self::Dismissed => 1,
			Self::Unavailable => 2,
		}
	}

	/// The raw destination identifier, for [`ShareResult::Success`] only.
	pub fn raw(&self) -> Option<&str> {
		match self {
			Self::Success(raw) => Some(raw),
			Self::Dismissed | Self::Unavailable => None,
		}
	}

	/// Wire: little-endian version, status (0 success, 1 dismissed, 2
	/// unavailable), then the length-prefixed UTF-8 raw identifier, which is
	/// present exactly for success.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let raw = self.raw().unwrap_or_default();
		validate_raw(self, raw)?;
		let mut encoder = Encoder::with_capacity(FRAMING_BYTES + raw.len());
		encoder.u32(WIRE_VERSION);
		encoder.u32(self.code());
		encoder.bytes(raw.as_bytes())?;
		Ok(encoder.finish())
	}

	/// Validates a native reply: known version and status, a bounded raw
	/// identifier exactly when the status is success, and no trailing data.
	pub fn decode(reply: &[u8]) -> Result<Self, Error> {
		if reply.len() > FRAMING_BYTES + MAX_RAW_BYTES {
			return Err(Error::TooLarge);
		}

		let mut decoder = Decoder::new(reply);

		if decoder.u32()? != WIRE_VERSION {
			return Err(Error::InvalidArgument);
		}

		let status = decoder.u32()?;
		let raw = decoder.string()?;
		decoder.finish()?;
		let result = match status {
			0 => Self::Success(raw.to_owned()),
			1 => Self::Dismissed,
			2 => Self::Unavailable,
			_ => return Err(Error::InvalidArgument),
		};
		validate_raw(&result, raw)?;
		Ok(result)
	}
}

/// Success names a nonempty identifier without control characters; the other
/// statuses carry none.
fn validate_raw(result: &ShareResult, raw: &str) -> Result<(), Error> {
	if raw.len() > MAX_RAW_BYTES {
		return Err(Error::TooLarge);
	}

	let valid = match result {
		ShareResult::Success(_) => !raw.is_empty() && !raw.chars().any(char::is_control),
		ShareResult::Dismissed | ShareResult::Unavailable => raw.is_empty(),
	};

	if valid {
		Ok(())
	} else {
		Err(Error::InvalidArgument)
	}
}
