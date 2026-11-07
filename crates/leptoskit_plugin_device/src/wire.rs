//! Strict little-endian field helpers shared by the snapshot and identifier
//! codecs. Every optional value has exactly one valid encoding, so a decoded
//! message re-encodes to the same bytes.

use std::time::Duration;
use std::time::SystemTime;

use leptoskit_bridge::Error;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Appends validated fields.
pub(crate) struct Writer(Encoder);

impl Writer {
	pub(crate) fn new(capacity: usize) -> Self {
		Self(Encoder::with_capacity(capacity))
	}

	pub(crate) fn u32(&mut self, value: u32) {
		self.0.u32(value);
	}

	/// Two little-endian words, low first: the same bytes as a little-endian
	/// u64.
	pub(crate) fn u64(&mut self, value: u64) {
		self.0.u32(value as u32);
		self.0.u32((value >> 32) as u32);
	}

	/// Unknown, no, yes as 0, 1, 2.
	pub(crate) fn tri(&mut self, value: Option<bool>) {
		self.u32(match value {
			None => 0,
			Some(false) => 1,
			Some(true) => 2,
		});
	}

	/// A presence word (0 or 1) followed by the value, which is zero when
	/// absent.
	pub(crate) fn opt_u32(&mut self, value: Option<u32>) {
		self.u32(u32::from(value.is_some()));
		self.u32(value.unwrap_or(0));
	}

	/// A presence word (0 or 1) followed by the value, which is zero when
	/// absent.
	pub(crate) fn opt_u64(&mut self, value: Option<u64>) {
		self.u32(u32::from(value.is_some()));
		self.u64(value.unwrap_or(0));
	}

	/// Milliseconds since the Unix epoch, truncated; times before the epoch
	/// are invalid.
	pub(crate) fn opt_time(&mut self, value: Option<SystemTime>) -> Result<(), Error> {
		let millis = value.map(millis).transpose()?;
		self.opt_u64(millis);
		Ok(())
	}

	/// A length-prefixed UTF-8 string. Empty means unknown, so a known empty
	/// string is invalid.
	pub(crate) fn text(&mut self, value: Option<&str>, max: usize) -> Result<(), Error> {
		let value = value.unwrap_or_default();

		if value.len() > max {
			return Err(Error::TooLarge);
		}

		self.0.bytes(value.as_bytes())
	}

	/// Like [`Writer::text`] but rejects a known empty string.
	pub(crate) fn known(&mut self, value: Option<&str>, max: usize) -> Result<(), Error> {
		if value.is_some_and(str::is_empty) {
			return Err(Error::InvalidArgument);
		}

		self.text(value, max)
	}

	/// A count followed by non-empty, bounded strings.
	pub(crate) fn list(
		&mut self,
		items: &[String],
		max_items: usize,
		max_bytes: usize,
	) -> Result<(), Error> {
		if items.len() > max_items {
			return Err(Error::TooLarge);
		}

		self.u32(u32::try_from(items.len()).map_err(|_| Error::TooLarge)?);

		for item in items {
			if item.is_empty() {
				return Err(Error::InvalidArgument);
			}

			self.text(Some(item), max_bytes)?;
		}

		Ok(())
	}

	pub(crate) fn finish(self) -> leptoskit_bridge::Bytes {
		self.0.finish()
	}
}

/// Reads and validates fields written by [`Writer`].
pub(crate) struct Reader<'a>(Decoder<'a>);

impl<'a> Reader<'a> {
	pub(crate) fn new(bytes: &'a [u8]) -> Self {
		Self(Decoder::new(bytes))
	}

	pub(crate) fn u32(&mut self) -> Result<u32, Error> {
		self.0.u32()
	}

	pub(crate) fn u64(&mut self) -> Result<u64, Error> {
		let low = u64::from(self.0.u32()?);
		let high = u64::from(self.0.u32()?);
		Ok(low | (high << 32))
	}

	pub(crate) fn tri(&mut self) -> Result<Option<bool>, Error> {
		match self.u32()? {
			0 => Ok(None),
			1 => Ok(Some(false)),
			2 => Ok(Some(true)),
			_ => Err(Error::InvalidArgument),
		}
	}

	pub(crate) fn opt_u32(&mut self) -> Result<Option<u32>, Error> {
		let present = self.u32()?;
		let value = self.u32()?;

		match (present, value) {
			(0, 0) => Ok(None),
			(1, value) => Ok(Some(value)),
			_ => Err(Error::InvalidArgument),
		}
	}

	pub(crate) fn opt_u64(&mut self) -> Result<Option<u64>, Error> {
		let present = self.u32()?;
		let value = self.u64()?;

		match (present, value) {
			(0, 0) => Ok(None),
			(1, value) => Ok(Some(value)),
			_ => Err(Error::InvalidArgument),
		}
	}

	pub(crate) fn opt_time(&mut self) -> Result<Option<SystemTime>, Error> {
		self.opt_u64()?
			.map(|millis| {
				SystemTime::UNIX_EPOCH
					.checked_add(Duration::from_millis(millis))
					.ok_or(Error::InvalidArgument)
			})
			.transpose()
	}

	/// Empty decodes as unknown.
	pub(crate) fn text(&mut self, max: usize) -> Result<Option<String>, Error> {
		let value = self.0.string()?;

		if value.len() > max {
			return Err(Error::TooLarge);
		}

		Ok((!value.is_empty()).then(|| value.to_owned()))
	}

	pub(crate) fn list(
		&mut self,
		max_items: usize,
		max_bytes: usize,
	) -> Result<Vec<String>, Error> {
		let count = self.u32()? as usize;

		if count > max_items {
			return Err(Error::TooLarge);
		}

		let mut items = Vec::with_capacity(count);

		for _ in 0..count {
			items.push(self.text(max_bytes)?.ok_or(Error::InvalidArgument)?);
		}

		Ok(items)
	}

	pub(crate) fn finish(self) -> Result<(), Error> {
		self.0.finish()
	}
}

/// Milliseconds since the Unix epoch.
pub(crate) fn millis(time: SystemTime) -> Result<u64, Error> {
	let elapsed = time
		.duration_since(SystemTime::UNIX_EPOCH)
		.map_err(|_| Error::InvalidArgument)?;
	u64::try_from(elapsed.as_millis()).map_err(|_| Error::TooLarge)
}
