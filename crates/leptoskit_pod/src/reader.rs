//! Allocation-free Pod decoder over a borrowed buffer.

use crate::Error;
use crate::Result;

/// Reads little-endian Pod fields from `remaining`.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
	remaining: &'a [u8],
}

impl<'a> Reader<'a> {
	/// Starts at the beginning of `input`.
	pub fn new(input: &'a [u8]) -> Self {
		Self { remaining: input }
	}

	/// Bytes not yet consumed.
	pub fn remaining(&self) -> &'a [u8] {
		self.remaining
	}

	fn take(&mut self, len: usize) -> Result<&'a [u8]> {
		let bytes = self.remaining.get(..len).ok_or(Error::Truncated)?;
		self.remaining = &self.remaining[len..];
		Ok(bytes)
	}

	/// Reads a `u8`.
	pub fn u8(&mut self) -> Result<u8> {
		Ok(self.take(1)?[0])
	}

	/// Reads a little-endian `u16`.
	pub fn u16(&mut self) -> Result<u16> {
		let bytes = self.take(2)?;
		Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
	}

	/// Reads a little-endian `u32`.
	pub fn u32(&mut self) -> Result<u32> {
		let bytes = self.take(4)?;
		Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
	}

	/// Reads a little-endian `u64`.
	pub fn u64(&mut self) -> Result<u64> {
		let bytes = self.take(8)?;
		Ok(u64::from_le_bytes(
			bytes.try_into().map_err(|_| Error::Truncated)?,
		))
	}

	/// Reads an `i8`.
	pub fn i8(&mut self) -> Result<i8> {
		Ok(self.u8()? as i8)
	}

	/// Reads an `i16`.
	pub fn i16(&mut self) -> Result<i16> {
		Ok(self.u16()? as i16)
	}

	/// Reads an `i32`.
	pub fn i32(&mut self) -> Result<i32> {
		Ok(self.u32()? as i32)
	}

	/// Reads an `i64`.
	pub fn i64(&mut self) -> Result<i64> {
		Ok(self.u64()? as i64)
	}

	/// Reads a bool (`0` / `1` only).
	pub fn bool(&mut self) -> Result<bool> {
		match self.u8()? {
			0 => Ok(false),
			1 => Ok(true),
			other => Err(Error::InvalidTag(u16::from(other))),
		}
	}

	/// Borrows a length-prefixed byte slice.
	pub fn bytes(&mut self) -> Result<&'a [u8]> {
		let len = self.u32()? as usize;
		self.take(len)
	}

	/// Borrows a UTF-8 string.
	pub fn string(&mut self) -> Result<&'a str> {
		std::str::from_utf8(self.bytes()?).map_err(|_| Error::InvalidUtf8)
	}

	/// Reads a vector element count.
	pub fn vec_len(&mut self) -> Result<usize> {
		Ok(self.u32()? as usize)
	}

	/// Option tag → whether a payload follows.
	pub fn option_tag(&mut self) -> Result<bool> {
		match self.u8()? {
			0 => Ok(false),
			1 => Ok(true),
			other => Err(Error::InvalidTag(u16::from(other))),
		}
	}

	/// Result tag → `true` when Ok payload follows, `false` when Err.
	pub fn result_tag(&mut self) -> Result<bool> {
		match self.u8()? {
			0 => Ok(true),
			1 => Ok(false),
			other => Err(Error::InvalidTag(u16::from(other))),
		}
	}

	/// Struct version + flags.
	pub fn struct_header(&mut self) -> Result<(u8, u8)> {
		Ok((self.u8()?, self.u8()?))
	}

	/// Enum variant discriminator.
	pub fn variant(&mut self) -> Result<u16> {
		self.u16()
	}

	/// Requires every byte to have been consumed.
	pub fn finish(self) -> Result<()> {
		if self.remaining.is_empty() {
			Ok(())
		} else {
			Err(Error::Trailing)
		}
	}
}
