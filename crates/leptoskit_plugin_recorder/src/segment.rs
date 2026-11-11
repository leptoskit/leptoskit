//! Finished segments as the native handlers deliver them.

use leptoskit_bridge::Error;

use crate::wire::Reader;
use crate::wire::Writer;

/// The quietest level a segment reports, in dBFS, so digital silence has a
/// finite value.
pub const FLOOR_DBFS: f64 = -120.0;
/// Longest accepted segment path, in bytes.
pub const MAX_PATH_BYTES: usize = 4096;

/// Why a segment ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndedBy {
	/// The segment reached its configured length and the next one began.
	Rotation = 0,
	/// The app stopped recording; the segment is the last one.
	Stop = 1,
	/// The platform interrupted the session, with a call or Siri.
	Interruption = 2,
	/// A crash or kill left this finalised file behind; a later start or
	/// `recover` delivered it.
	Recovered = 3,
}

impl EndedBy {
	/// Decodes an end code.
	pub fn from_code(code: u32) -> Result<Self, Error> {
		Ok(match code {
			0 => Self::Rotation,
			1 => Self::Stop,
			2 => Self::Interruption,
			3 => Self::Recovered,
			_ => return Err(Error::InvalidArgument),
		})
	}

	/// The wire code.
	pub const fn code(self) -> u32 {
		self as u32
	}
}

/// One finished audio segment, AAC-LC mono in an `.m4a` file.
///
/// The plugin never deletes a delivered segment and never decides whether one
/// is silent: the app owns both choices. `peak_dbfs` and `active_ms` describe
/// 30-millisecond frames; `active_ms` counts the frames whose RMS stayed
/// above the configured threshold, so an app can skip uploading quiet
/// segments without decoding them.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
	/// The segment file's absolute path, inside the configured directory.
	pub path: String,
	/// When the segment started, in Unix milliseconds.
	pub started_at_ms: i64,
	/// How long the segment recorded, in milliseconds.
	pub duration_ms: i64,
	/// The segment file's size in bytes.
	pub bytes: i64,
	/// The loudest frame's peak level in dBFS, from `FLOOR_DBFS` to 0.
	pub peak_dbfs: f64,
	/// How many milliseconds of the segment were above the activity
	/// threshold.
	pub active_ms: i64,
	/// Why the segment ended.
	pub ended_by: EndedBy,
}

impl Segment {
	/// Encodes a next-segment reply carrying this segment, for handlers and
	/// tests.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		validate(self)?;
		Ok(Writer::default()
			.u32(1)
			.str(&self.path)?
			.i64(self.started_at_ms)
			.i64(self.duration_ms)
			.i64(self.bytes)
			.f64(self.peak_dbfs)?
			.i64(self.active_ms)
			.u32(self.ended_by.code())
			.finish())
	}

	/// Encodes the empty next-segment reply: nothing is waiting and nothing
	/// is recording.
	pub fn encode_absent() -> Vec<u8> {
		Writer::default().u32(0).finish()
	}

	/// Decodes one next-segment reply: either a finished segment or nothing,
	/// when no segment is waiting and nothing is recording.
	pub fn decode(payload: &[u8]) -> Result<Option<Self>, Error> {
		let mut reader = Reader::new(payload);
		let present = match reader.u32()? {
			0 => false,
			1 => true,
			_ => return Err(Error::InvalidArgument),
		};

		if !present {
			reader.finish()?;
			return Ok(None);
		}

		let segment = Self {
			path: reader.str()?.to_owned(),
			started_at_ms: reader.i64()?,
			duration_ms: reader.i64()?,
			bytes: reader.i64()?,
			peak_dbfs: reader.f64()?,
			active_ms: reader.i64()?,
			ended_by: EndedBy::from_code(reader.u32()?)?,
		};
		reader.finish()?;
		validate(&segment)?;
		Ok(Some(segment))
	}
}

/// The invariants every segment reply satisfies.
fn validate(segment: &Segment) -> Result<(), Error> {
	if segment.path.is_empty()
		|| segment.path.len() > MAX_PATH_BYTES
		|| segment.started_at_ms <= 0
		|| segment.duration_ms < 0
		|| segment.bytes <= 0
		|| !(FLOOR_DBFS..=0.0).contains(&segment.peak_dbfs)
		|| !(0..=segment.duration_ms).contains(&segment.active_ms)
	{
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn segment() -> Segment {
		Segment {
			path: "/var/recorder/seg-1700000000000.m4a".to_owned(),
			started_at_ms: 1_700_000_000_000,
			duration_ms: 300_000,
			bytes: 900_000,
			peak_dbfs: -12.5,
			active_ms: 42_000,
			ended_by: EndedBy::Rotation,
		}
	}

	#[test]
	fn segments_round_trip_with_their_metrics() {
		let bytes = segment().encode().unwrap();
		assert_eq!(Segment::decode(&bytes).unwrap(), Some(segment()));

		let mut ended = segment();
		ended.ended_by = EndedBy::Recovered;
		ended.active_ms = 0;
		let bytes = ended.encode().unwrap();
		assert_eq!(Segment::decode(&bytes).unwrap(), Some(ended));
	}

	#[test]
	fn an_absent_segment_is_present_zero_and_nothing_else() {
		assert_eq!(Segment::decode(&[0, 0, 0, 0]), Ok(None));
		assert_eq!(
			Segment::decode(&[0, 0, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(Segment::decode(&[]), Err(Error::InvalidArgument));
		assert_eq!(Segment::decode(&[2, 0, 0, 0]), Err(Error::InvalidArgument));
	}

	#[test]
	fn segment_invariants_are_enforced() {
		let path = segment().path;
		let raw = |started: i64, duration: i64, bytes: i64, peak: f64, active: i64, ended: u32| {
			Writer::default()
				.u32(1)
				.str(&path)
				.unwrap()
				.i64(started)
				.i64(duration)
				.i64(bytes)
				.f64(peak)
				.unwrap()
				.i64(active)
				.u32(ended)
				.finish()
		};

		// Every field past its bound is rejected, in every direction.
		for invalid in [
			raw(0, 1, 1, -10.0, 0, 0),
			raw(-5, 1, 1, -10.0, 0, 0),
			raw(1, -1, 1, -10.0, 0, 0),
			raw(1, 1, 0, -10.0, 0, 0),
			raw(1, 1, 1, 0.5, 0, 0),
			raw(1, 1, 1, FLOOR_DBFS - 0.1, 0, 0),
			raw(1, 1_000, 1, -10.0, 1_001, 0),
			raw(1, 1_000, 1, -10.0, -1, 0),
			raw(1, 1, 1, -10.0, 0, 4),
		] {
			assert!(Segment::decode(&invalid).is_err());
		}

		// A zero-duration segment is still a segment, and so is silence.
		assert!(Segment::decode(&raw(1, 0, 1, FLOOR_DBFS, 0, 0)).is_ok());

		// An empty path is rejected too.
		let empty = Writer::default()
			.u32(1)
			.str("")
			.unwrap()
			.i64(1)
			.i64(1)
			.i64(1)
			.f64(-10.0)
			.unwrap()
			.i64(0)
			.u32(0)
			.finish();
		assert!(Segment::decode(&empty).is_err());

		// NaN peak bits never decode.
		let peak_at = 8 + path.len() + 24;
		let mut nan = raw(1, 1, 1, -10.0, 0, 0);
		nan[peak_at..peak_at + 8].copy_from_slice(&f64::NAN.to_le_bytes());
		assert!(Segment::decode(&nan).is_err());

		// Encoding rejects the same violations before they reach the wire.
		assert!(
			Segment {
				path: String::new(),
				..segment()
			}
			.encode()
			.is_err()
		);
		assert!(
			Segment {
				active_ms: 300_001,
				..segment()
			}
			.encode()
			.is_err()
		);
	}

	#[test]
	fn ended_by_codes_are_dense() {
		for (code, expected) in [
			(0, EndedBy::Rotation),
			(1, EndedBy::Stop),
			(2, EndedBy::Interruption),
			(3, EndedBy::Recovered),
		] {
			assert_eq!(EndedBy::from_code(code), Ok(expected));
			assert_eq!(expected.code(), code);
		}
		assert_eq!(EndedBy::from_code(4), Err(Error::InvalidArgument));
	}
}
