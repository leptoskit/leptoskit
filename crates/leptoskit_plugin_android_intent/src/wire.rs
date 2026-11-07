//! Little-endian framing shared by the intent codec and its replies.
//!
//! Integers are little-endian. A string is a u32 byte length followed by
//! that many UTF-8 bytes. Every read checks its bound before it checks that
//! the bytes are present, so an oversized field is [`Error::TooLarge`] even
//! when it is also truncated, exactly as the Kotlin decoder reports it.

use leptoskit_bridge::Error;

/// Appends little-endian fields to an owned buffer.
#[derive(Debug, Default)]
pub(crate) struct Writer {
	bytes: Vec<u8>,
}

impl Writer {
	pub(crate) fn u32(&mut self, value: u32) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	pub(crate) fn i32(&mut self, value: i32) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	pub(crate) fn i64(&mut self, value: i64) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	pub(crate) fn f64(&mut self, value: f64) {
		self.bytes.extend_from_slice(&value.to_bits().to_le_bytes());
	}

	pub(crate) fn len(&mut self, value: usize) -> Result<(), Error> {
		self.u32(u32::try_from(value).map_err(|_| Error::TooLarge)?);
		Ok(())
	}

	pub(crate) fn bytes(&mut self, value: &[u8]) -> Result<(), Error> {
		self.len(value.len())?;
		self.bytes.extend_from_slice(value);
		Ok(())
	}

	pub(crate) fn string(&mut self, value: &str) -> Result<(), Error> {
		self.bytes(value.as_bytes())
	}

	/// An absent optional string is the empty string; empty values are
	/// never valid for the fields that use this.
	pub(crate) fn optional(&mut self, value: Option<&str>) -> Result<(), Error> {
		self.string(value.unwrap_or_default())
	}

	pub(crate) fn finish(self) -> Vec<u8> {
		self.bytes
	}
}

/// Reads little-endian fields from a borrowed payload.
#[derive(Debug)]
pub(crate) struct Reader<'a> {
	remaining: &'a [u8],
}

impl<'a> Reader<'a> {
	pub(crate) fn new(input: &'a [u8]) -> Self {
		Self { remaining: input }
	}

	fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
		let bytes = self.remaining.get(..len).ok_or(Error::InvalidArgument)?;
		self.remaining = &self.remaining[len..];
		Ok(bytes)
	}

	fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
		self.take(N)?.try_into().map_err(|_| Error::InvalidArgument)
	}

	pub(crate) fn u32(&mut self) -> Result<u32, Error> {
		Ok(u32::from_le_bytes(self.array()?))
	}

	pub(crate) fn i32(&mut self) -> Result<i32, Error> {
		Ok(i32::from_le_bytes(self.array()?))
	}

	pub(crate) fn i64(&mut self) -> Result<i64, Error> {
		Ok(i64::from_le_bytes(self.array()?))
	}

	/// Only finite doubles cross the wire: NaN payloads and infinities have
	/// no portable meaning for an extra.
	pub(crate) fn f64(&mut self) -> Result<f64, Error> {
		let value = f64::from_bits(u64::from_le_bytes(self.array()?));

		if !value.is_finite() {
			return Err(Error::InvalidArgument);
		}

		Ok(value)
	}

	/// A boolean is exactly 0 or 1 in a u32; other words are not truthy.
	pub(crate) fn bool(&mut self) -> Result<bool, Error> {
		match self.u32()? {
			0 => Ok(false),
			1 => Ok(true),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// A count or length, refused as too large before anything is read.
	pub(crate) fn count(&mut self, max: usize) -> Result<usize, Error> {
		let value = self.u32()? as usize;

		if value > max {
			return Err(Error::TooLarge);
		}

		Ok(value)
	}

	pub(crate) fn bytes(&mut self, max: usize) -> Result<&'a [u8], Error> {
		let len = self.count(max)?;
		self.take(len)
	}

	pub(crate) fn string(&mut self, max: usize) -> Result<&'a str, Error> {
		core::str::from_utf8(self.bytes(max)?).map_err(|_| Error::InvalidArgument)
	}

	/// The empty string means the optional field is absent.
	pub(crate) fn optional(&mut self, max: usize) -> Result<Option<&'a str>, Error> {
		let value = self.string(max)?;
		Ok((!value.is_empty()).then_some(value))
	}

	pub(crate) fn finish(self) -> Result<(), Error> {
		if self.remaining.is_empty() {
			return Ok(());
		}

		Err(Error::InvalidArgument)
	}
}
