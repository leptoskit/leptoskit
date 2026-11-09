//! Little-endian framing shared with the Swift and Kotlin handlers.
//!
//! Integers and floats are little-endian. A string is a `u32` byte length and
//! its UTF-8 bytes; an absent optional string is the length `u32::MAX`.

use leptoskit_bridge::Error;

/// The length that marks an absent optional string.
pub(crate) const ABSENT: u32 = u32::MAX;

/// Appends request fields.
#[derive(Debug, Default)]
pub(crate) struct Writer(Vec<u8>);

impl Writer {
	pub(crate) fn u32(&mut self, value: u32) -> &mut Self {
		self.0.extend_from_slice(&value.to_le_bytes());
		self
	}

	pub(crate) fn i64(&mut self, value: i64) -> &mut Self {
		self.0.extend_from_slice(&value.to_le_bytes());
		self
	}

	/// A finite float; NaN and infinities are never valid level values.
	pub(crate) fn f64(&mut self, value: f64) -> Result<&mut Self, Error> {
		value
			.is_finite()
			.then_some(())
			.ok_or(Error::InvalidArgument)?;
		self.0.extend_from_slice(&value.to_le_bytes());
		Ok(self)
	}

	pub(crate) fn str(&mut self, value: &str) -> Result<&mut Self, Error> {
		let length = u32::try_from(value.len())
			.ok()
			.filter(|length| *length != ABSENT)
			.ok_or(Error::TooLarge)?;
		self.u32(length);
		self.0.extend_from_slice(value.as_bytes());
		Ok(self)
	}

	pub(crate) fn optional(&mut self, value: Option<&str>) -> Result<&mut Self, Error> {
		match value {
			Some(value) => self.str(value),
			None => Ok(self.u32(ABSENT)),
		}
	}

	pub(crate) fn finish(&mut self) -> Vec<u8> {
		std::mem::take(&mut self.0)
	}
}

/// Reads reply fields, rejecting truncation, invalid UTF-8 and trailing
/// bytes.
#[derive(Debug)]
pub(crate) struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
	pub(crate) fn new(bytes: &'a [u8]) -> Self {
		Self(bytes)
	}

	fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
		let (head, tail) = self.0.split_at_checked(N).ok_or(Error::InvalidArgument)?;
		self.0 = tail;
		head.try_into().map_err(|_| Error::InvalidArgument)
	}

	pub(crate) fn u32(&mut self) -> Result<u32, Error> {
		self.take().map(u32::from_le_bytes)
	}

	pub(crate) fn i64(&mut self) -> Result<i64, Error> {
		self.take().map(i64::from_le_bytes)
	}

	/// A finite float; NaN and infinities are never valid level values.
	pub(crate) fn f64(&mut self) -> Result<f64, Error> {
		let value = f64::from_le_bytes(self.take()?);
		value
			.is_finite()
			.then_some(value)
			.ok_or(Error::InvalidArgument)
	}

	fn bytes(&mut self, length: u32) -> Result<&'a str, Error> {
		let (head, tail) = self
			.0
			.split_at_checked(length as usize)
			.ok_or(Error::InvalidArgument)?;
		self.0 = tail;
		std::str::from_utf8(head).map_err(|_| Error::InvalidArgument)
	}

	pub(crate) fn str(&mut self) -> Result<&'a str, Error> {
		match self.u32()? {
			ABSENT => Err(Error::InvalidArgument),
			length => self.bytes(length),
		}
	}

	pub(crate) fn optional(&mut self) -> Result<Option<&'a str>, Error> {
		match self.u32()? {
			ABSENT => Ok(None),
			length => self.bytes(length).map(Some),
		}
	}

	pub(crate) fn finish(self) -> Result<(), Error> {
		self.0
			.is_empty()
			.then_some(())
			.ok_or(Error::InvalidArgument)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn strings_round_trip_and_absent_is_distinct_from_empty() {
		let bytes = Writer::default()
			.str("")
			.unwrap()
			.optional(None)
			.unwrap()
			.optional(Some("ü"))
			.unwrap()
			.finish();
		let mut reader = Reader::new(&bytes);
		assert_eq!(reader.str(), Ok(""));
		assert_eq!(reader.optional(), Ok(None));
		assert_eq!(reader.optional(), Ok(Some("ü")));
		assert_eq!(reader.finish(), Ok(()));
	}

	#[test]
	fn truncation_invalid_text_and_trailing_bytes_are_rejected() {
		assert_eq!(Reader::new(&[1, 0, 0]).u32(), Err(Error::InvalidArgument));
		assert_eq!(
			Reader::new(&[2, 0, 0, 0, b'a']).str(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Reader::new(&[1, 0, 0, 0, 0xFF]).str(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Reader::new(&ABSENT.to_le_bytes()).str(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(Reader::new(&[0]).finish(), Err(Error::InvalidArgument));
	}

	#[test]
	fn floats_reject_non_finite_bits_in_both_directions() {
		let bytes = Writer::default().f64(-45.5).unwrap().finish();
		let mut reader = Reader::new(&bytes);
		assert_eq!(reader.f64(), Ok(-45.5));
		assert_eq!(reader.finish(), Ok(()));

		assert_eq!(
			Writer::default().f64(f64::NAN).map(|_| ()),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Reader::new(&f64::NAN.to_le_bytes()).f64(),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Reader::new(&f64::INFINITY.to_le_bytes()).f64(),
			Err(Error::InvalidArgument)
		);
	}
}
