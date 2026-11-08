//! Append Pod fields into a growable buffer.

use crate::Error;
use crate::Result;

/// Appends little-endian Pod fields.
#[derive(Debug, Default, Clone)]
pub struct Writer {
	bytes: Vec<u8>,
}

impl Writer {
	/// Empty writer.
	pub fn new() -> Self {
		Self::default()
	}

	/// Reserves capacity for a known encoded size.
	pub fn with_capacity(capacity: usize) -> Self {
		Self {
			bytes: Vec::with_capacity(capacity),
		}
	}

	/// Current length of the buffer.
	pub fn len(&self) -> usize {
		self.bytes.len()
	}

	/// Whether the buffer is empty.
	pub fn is_empty(&self) -> bool {
		self.bytes.is_empty()
	}

	/// Appends raw bytes with no length prefix.
	pub fn raw(&mut self, value: &[u8]) {
		self.bytes.extend_from_slice(value);
	}

	/// Appends a `u8`.
	pub fn u8(&mut self, value: u8) {
		self.bytes.push(value);
	}

	/// Appends a little-endian `u16`.
	pub fn u16(&mut self, value: u16) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	/// Appends a little-endian `u32`.
	pub fn u32(&mut self, value: u32) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	/// Appends a little-endian `u64`.
	pub fn u64(&mut self, value: u64) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	/// Appends a little-endian `i8`.
	pub fn i8(&mut self, value: i8) {
		self.u8(value as u8);
	}

	/// Appends a little-endian `i16`.
	pub fn i16(&mut self, value: i16) {
		self.u16(value as u16);
	}

	/// Appends a little-endian `i32`.
	pub fn i32(&mut self, value: i32) {
		self.u32(value as u32);
	}

	/// Appends a little-endian `i64`.
	pub fn i64(&mut self, value: i64) {
		self.u64(value as u64);
	}

	/// Appends a bool as `0` / `1`.
	pub fn bool(&mut self, value: bool) {
		self.u8(u8::from(value));
	}

	/// Appends a `u32` length then the bytes.
	pub fn bytes(&mut self, value: &[u8]) -> Result<()> {
		let len = u32::try_from(value.len()).map_err(|_| Error::TooLarge)?;
		self.u32(len);
		self.bytes.extend_from_slice(value);
		Ok(())
	}

	/// Appends a UTF-8 string as length-prefixed bytes.
	pub fn string(&mut self, value: &str) -> Result<()> {
		self.bytes(value.as_bytes())
	}

	/// Appends a vector element count (`u32`).
	pub fn vec_len(&mut self, len: usize) -> Result<()> {
		let len = u32::try_from(len).map_err(|_| Error::TooLarge)?;
		self.u32(len);
		Ok(())
	}

	/// Option tag: `0` = none, `1` = some.
	pub fn option_tag(&mut self, some: bool) {
		self.u8(u8::from(some));
	}

	/// Result tag: `0` = ok, `1` = err.
	pub fn result_tag(&mut self, ok: bool) {
		self.u8(u8::from(!ok));
	}

	/// Struct layout version (little-endian `u16`).
	pub fn struct_header(&mut self, version: u16) {
		self.u16(version);
	}

	/// Enum variant discriminator.
	pub fn variant(&mut self, disc: u16) {
		self.u16(disc);
	}

	/// Borrows the encoded bytes.
	pub fn as_slice(&self) -> &[u8] {
		&self.bytes
	}

	/// Transfers ownership of the buffer.
	pub fn finish(self) -> Vec<u8> {
		self.bytes
	}
}
