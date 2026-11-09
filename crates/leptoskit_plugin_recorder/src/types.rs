//! Permission, start options, recovery reports and recording status.

use leptoskit_bridge::Error;

use crate::wire::Reader;
use crate::wire::Writer;

/// The microphone permission state, as far as the platform reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
	/// The platform has not answered yet; a request could still prompt.
	Unknown = 0,
	/// The user allowed microphone access.
	Granted = 1,
	/// The user denied microphone access, or the platform withholds it.
	Denied = 2,
}

impl Permission {
	/// Decodes the permission reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		match payload {
			[0, 0, 0, 0] => Ok(Self::Unknown),
			[1, 0, 0, 0] => Ok(Self::Granted),
			[2, 0, 0, 0] => Ok(Self::Denied),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Encodes the permission reply, for handlers and tests.
	pub fn encode(self) -> Vec<u8> {
		Writer::default().u32(self as u32).finish()
	}
}

/// Longest accepted segment, in seconds.
pub const MAX_SEGMENT_SECS: u32 = 3600;
/// Shortest accepted segment, in seconds.
pub const MIN_SEGMENT_SECS: u32 = 5;
/// The segment length used when the app does not name one, in seconds.
pub const DEFAULT_SEGMENT_SECS: u32 = 300;
/// Highest accepted sample rate, in hertz.
pub const MAX_SAMPLE_RATE: u32 = 48_000;
/// Lowest accepted sample rate, in hertz.
pub const MIN_SAMPLE_RATE: u32 = 8_000;
/// The sample rate used when the app does not name one, in hertz.
pub const DEFAULT_SAMPLE_RATE: u32 = 16_000;
/// Highest accepted AAC bitrate, in bits per second.
pub const MAX_BITRATE: u32 = 320_000;
/// Lowest accepted AAC bitrate, in bits per second.
pub const MIN_BITRATE: u32 = 8_000;
/// The bitrate used when the app does not name one, in bits per second.
pub const DEFAULT_BITRATE: u32 = 24_000;
/// The loudest level the activity threshold can name, in dBFS: full scale.
pub const MAX_ACTIVITY_THRESHOLD_DBFS: f64 = 0.0;
/// The quietest level the activity threshold can name, in dBFS.
pub const MIN_ACTIVITY_THRESHOLD_DBFS: f64 = -100.0;
/// The threshold used when the app does not name one, in dBFS.
pub const DEFAULT_ACTIVITY_THRESHOLD_DBFS: f64 = -45.0;
/// Longest accepted directory path, in bytes.
pub const MAX_DIR_BYTES: usize = 1024;

/// Configuration for one continuous recording session.
///
/// Segments are AAC-LC, mono, `.m4a`, rotated at each boundary with no gap
/// between segments. The app owns the directory: it must sit inside the app's
/// own data or cache directories, and the plugin never deletes a delivered
/// segment, so the app decides when to upload or remove one.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
	dir: String,
	segment_secs: u32,
	sample_rate: u32,
	bitrate: u32,
	activity_threshold_dbfs: f64,
}

impl Options {
	/// Constructs a session for `dir` with every default: 300-second
	/// segments, 16 kHz mono, 24 kbps AAC, and a −45 dBFS activity threshold.
	pub fn new(dir: impl Into<String>) -> Result<Self, Error> {
		let dir = dir.into();
		validate_dir(&dir)?;
		Ok(Self {
			dir,
			segment_secs: DEFAULT_SEGMENT_SECS,
			sample_rate: DEFAULT_SAMPLE_RATE,
			bitrate: DEFAULT_BITRATE,
			activity_threshold_dbfs: DEFAULT_ACTIVITY_THRESHOLD_DBFS,
		})
	}

	/// Names the segment length in seconds; values outside
	/// `MIN_SEGMENT_SECS..=MAX_SEGMENT_SECS` are clamped, not rejected,
	/// because a lifetime-logging app may compute it.
	#[must_use]
	pub fn with_segment_secs(mut self, segment_secs: u32) -> Self {
		self.segment_secs = segment_secs.clamp(MIN_SEGMENT_SECS, MAX_SEGMENT_SECS);
		self
	}

	/// Names the sample rate in hertz; 8 kHz to 48 kHz.
	pub fn with_sample_rate(mut self, sample_rate: u32) -> Result<Self, Error> {
		(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE)
			.contains(&sample_rate)
			.then_some(())
			.ok_or(Error::InvalidArgument)?;
		self.sample_rate = sample_rate;
		Ok(self)
	}

	/// Names the AAC bitrate in bits per second; 8 kbps to 320 kbps.
	pub fn with_bitrate(mut self, bitrate: u32) -> Result<Self, Error> {
		(MIN_BITRATE..=MAX_BITRATE)
			.contains(&bitrate)
			.then_some(())
			.ok_or(Error::InvalidArgument)?;
		self.bitrate = bitrate;
		Ok(self)
	}

	/// Names the activity threshold in dBFS; −100 to 0. Frames whose RMS
	/// stays above it count as active in each segment's `active_ms`.
	pub fn with_activity_threshold_dbfs(mut self, threshold: f64) -> Result<Self, Error> {
		(MIN_ACTIVITY_THRESHOLD_DBFS..=MAX_ACTIVITY_THRESHOLD_DBFS)
			.contains(&threshold)
			.then_some(())
			.ok_or(Error::InvalidArgument)?;
		self.activity_threshold_dbfs = threshold;
		Ok(self)
	}

	/// The app-private directory segments are written to.
	#[must_use]
	pub fn dir(&self) -> &str {
		&self.dir
	}

	/// The segment length in seconds.
	#[must_use]
	pub const fn segment_secs(&self) -> u32 {
		self.segment_secs
	}

	/// The sample rate in hertz.
	#[must_use]
	pub const fn sample_rate(&self) -> u32 {
		self.sample_rate
	}

	/// The AAC bitrate in bits per second.
	#[must_use]
	pub const fn bitrate(&self) -> u32 {
		self.bitrate
	}

	/// The activity threshold in dBFS.
	#[must_use]
	pub const fn activity_threshold_dbfs(&self) -> f64 {
		self.activity_threshold_dbfs
	}

	/// Encodes the start request.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		Ok(Writer::default()
			.str(&self.dir)?
			.u32(self.segment_secs)
			.u32(self.sample_rate)
			.u32(self.bitrate)
			.f64(self.activity_threshold_dbfs)?
			.finish())
	}

	/// Decodes the start request, applying the same clamps and bounds the
	/// constructor does, and rejecting trailing bytes.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_DIR_BYTES + 32 {
			return Err(Error::InvalidArgument);
		}

		let mut reader = Reader::new(payload);
		let dir = reader.str()?.to_owned();
		let segment_secs = reader.u32()?;
		let sample_rate = reader.u32()?;
		let bitrate = reader.u32()?;
		let threshold = reader.f64()?;
		reader.finish()?;

		Self::new(dir)?
			.with_segment_secs(segment_secs)
			.with_sample_rate(sample_rate)?
			.with_bitrate(bitrate)?
			.with_activity_threshold_dbfs(threshold)
	}
}

/// An absolute path inside the app's sandbox, without traversal or control
/// characters. The native handler still checks the directory against the
/// app's real data and cache directories before writing.
fn validate_dir(dir: &str) -> Result<(), Error> {
	if dir.is_empty()
		|| dir.len() > MAX_DIR_BYTES
		|| !dir.starts_with('/')
		|| dir.chars().any(char::is_control)
		|| dir.split('/').any(|component| component == "..")
	{
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// What a start or recovery pass found left over from an earlier session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Recovery {
	/// Finalised files re-queued for delivery as recovered segments.
	pub recovered: u32,
	/// Truncated files that could not be read and were deleted. Each is
	/// reported once, here, when it is deleted.
	pub discarded: u32,
}

impl Recovery {
	/// Decodes the recovery report.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut reader = Reader::new(payload);
		let recovered = reader.u32()?;
		let discarded = reader.u32()?;
		reader.finish()?;
		Ok(Self {
			recovered,
			discarded,
		})
	}

	/// Encodes the recovery report, for handlers and tests.
	pub fn encode(self) -> Vec<u8> {
		Writer::default()
			.u32(self.recovered)
			.u32(self.discarded)
			.finish()
	}
}

/// What the recorder is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
	/// Not recording; segments may still be waiting for delivery.
	Idle = 0,
	/// Recording, rotating segments as they fill.
	Recording = 1,
	/// iOS interrupted the session, with a call or Siri; the plugin tries to
	/// resume when the interruption ends.
	Interrupted = 2,
	/// Android silenced the app's capture because another app holds the
	/// microphone; segments keep rotating with silence.
	Silenced = 3,
	/// Recording failed; `reason` says why.
	Failed = 4,
}

impl State {
	/// Decodes a state code.
	pub fn from_code(code: u32) -> Result<Self, Error> {
		Ok(match code {
			0 => Self::Idle,
			1 => Self::Recording,
			2 => Self::Interrupted,
			3 => Self::Silenced,
			4 => Self::Failed,
			_ => return Err(Error::InvalidArgument),
		})
	}
}

/// Longest failure reason accepted from a handler, in bytes.
pub const MAX_REASON_BYTES: usize = 512;

/// A recording session's current state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
	state: State,
	current_segment_started_at_ms: i64,
	reason: Option<String>,
}

impl Status {
	/// Encodes a status reply, for handlers and tests.
	pub fn encode(
		state: State,
		current_segment_started_at_ms: i64,
		reason: Option<&str>,
	) -> Result<Vec<u8>, Error> {
		if reason.is_some_and(|reason| reason.is_empty() || reason.len() > MAX_REASON_BYTES) {
			return Err(Error::InvalidArgument);
		}

		Ok(Writer::default()
			.u32(state as u32)
			.i64(current_segment_started_at_ms)
			.optional(reason)?
			.finish())
	}

	/// Decodes the status reply. A failed state always carries a reason; any
	/// other state never does. A current segment start time exists only
	/// while a segment is open.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut reader = Reader::new(payload);
		let state = State::from_code(reader.u32()?)?;
		let started_at = reader.i64()?;
		let reason = reader.optional()?.map(str::to_owned);
		reader.finish()?;

		let open = matches!(state, State::Recording | State::Silenced);
		if open == (started_at == 0) || started_at < 0 {
			return Err(Error::InvalidArgument);
		}

		match (reason, state) {
			(Some(reason), State::Failed)
				if !reason.is_empty() && reason.len() <= MAX_REASON_BYTES =>
			{
				Ok(Self {
					state,
					current_segment_started_at_ms: started_at,
					reason: Some(reason),
				})
			}
			(None, _) => {
				Ok(Self {
					state,
					current_segment_started_at_ms: started_at,
					reason: None,
				})
			}
			_ => Err(Error::InvalidArgument),
		}
	}

	/// What the recorder is doing.
	#[must_use]
	pub const fn state(&self) -> State {
		self.state
	}

	/// When the current segment started, in Unix milliseconds, when one is
	/// open.
	#[must_use]
	pub const fn current_segment_started_at_ms(&self) -> Option<i64> {
		if self.current_segment_started_at_ms > 0 {
			Some(self.current_segment_started_at_ms)
		} else {
			None
		}
	}

	/// Why recording failed; present only in the failed state.
	#[must_use]
	pub fn reason(&self) -> Option<&str> {
		self.reason.as_deref()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn permission_states_decode_exactly() {
		assert_eq!(Permission::decode(&[0, 0, 0, 0]), Ok(Permission::Unknown));
		assert_eq!(Permission::decode(&[1, 0, 0, 0]), Ok(Permission::Granted));
		assert_eq!(Permission::decode(&[2, 0, 0, 0]), Ok(Permission::Denied));
		assert_eq!(
			Permission::decode(&[3, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(Permission::decode(&[1, 0, 0]), Err(Error::InvalidArgument));
		assert_eq!(
			Permission::decode(&[1, 0, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(Permission::encode(Permission::Denied), vec![2, 0, 0, 0]);
	}

	#[test]
	fn options_carry_defaults_and_round_trip() {
		let options = Options::new("/var/mobile/Containers/…/recorder").unwrap();
		assert_eq!(options.segment_secs(), DEFAULT_SEGMENT_SECS);
		assert_eq!(options.sample_rate(), DEFAULT_SAMPLE_RATE);
		assert_eq!(options.bitrate(), DEFAULT_BITRATE);
		assert_eq!(
			options.activity_threshold_dbfs(),
			DEFAULT_ACTIVITY_THRESHOLD_DBFS
		);

		let bytes = options.encode().unwrap();
		assert_eq!(Options::decode(&bytes).unwrap(), options);

		let tuned = Options::new("/data/recorder")
			.unwrap()
			.with_segment_secs(60)
			.with_sample_rate(48_000)
			.unwrap()
			.with_bitrate(128_000)
			.unwrap()
			.with_activity_threshold_dbfs(-60.0)
			.unwrap();
		let bytes = tuned.encode().unwrap();
		assert_eq!(Options::decode(&bytes).unwrap(), tuned);
	}

	#[test]
	fn segment_secs_clamps_and_other_fields_reject() {
		let clamped = Options::new("/d").unwrap().with_segment_secs(1);
		assert_eq!(clamped.segment_secs(), MIN_SEGMENT_SECS);
		let clamped = Options::new("/d").unwrap().with_segment_secs(9_999);
		assert_eq!(clamped.segment_secs(), MAX_SEGMENT_SECS);

		for invalid in [7_999, 4_000, 4_800, 0] {
			assert!(
				Options::new("/d")
					.unwrap()
					.with_sample_rate(invalid)
					.is_err(),
				"sample rate {invalid} accepted"
			);
		}
		for invalid in [7_999, 320_001, 0] {
			assert!(
				Options::new("/d").unwrap().with_bitrate(invalid).is_err(),
				"bitrate {invalid} accepted"
			);
		}
		for invalid in [-100.5, 0.5, f64::NAN, -120.0] {
			assert!(
				Options::new("/d")
					.unwrap()
					.with_activity_threshold_dbfs(invalid)
					.is_err(),
				"threshold {invalid} accepted"
			);
		}
	}

	#[test]
	fn directories_must_be_absolute_sandbox_paths() {
		assert!(Options::new("").is_err());
		assert!(Options::new("recorder").is_err());
		assert!(Options::new("/a/../b").is_err());
		assert!(Options::new("/a\0b").is_err());
		assert!(Options::new(format!("/{}", "x".repeat(MAX_DIR_BYTES))).is_err());
		assert!(Options::new("/tmp/recorder").is_ok());

		let payload = Options::new("/ok").unwrap().encode().unwrap();
		for length in 0..payload.len() {
			assert!(Options::decode(&payload[..length]).is_err());
		}
		let mut trailing = payload.clone();
		trailing.push(0);
		assert!(Options::decode(&trailing).is_err());
	}

	#[test]
	fn recovery_reports_round_trip() {
		let report = Recovery {
			recovered: 3,
			discarded: 1,
		};
		assert_eq!(report.encode(), vec![3, 0, 0, 0, 1, 0, 0, 0]);
		assert_eq!(Recovery::decode(&report.encode()), Ok(report));
		assert_eq!(Recovery::decode(&[0; 7]), Err(Error::InvalidArgument));
		assert_eq!(Recovery::decode(&[0; 9]), Err(Error::InvalidArgument));
	}

	#[test]
	fn status_pairs_states_with_open_segments_and_reasons() {
		let bytes = Status::encode(State::Recording, 1_700_000_000_000, None).unwrap();
		let status = Status::decode(&bytes).unwrap();
		assert_eq!(status.state(), State::Recording);
		assert_eq!(
			status.current_segment_started_at_ms(),
			Some(1_700_000_000_000)
		);
		assert_eq!(status.reason(), None);

		let bytes = Status::encode(State::Failed, 0, Some("engine stopped")).unwrap();
		let status = Status::decode(&bytes).unwrap();
		assert_eq!(status.state(), State::Failed);
		assert_eq!(status.current_segment_started_at_ms(), None);
		assert_eq!(status.reason(), Some("engine stopped"));

		// A closed state cannot name an open segment and vice versa.
		assert!(Status::decode(&Status::encode(State::Idle, 5, None).unwrap()).is_err());
		assert!(Status::decode(&Status::encode(State::Interrupted, 5, None).unwrap()).is_err());
		assert!(Status::decode(&Status::encode(State::Recording, 0, None).unwrap()).is_err());
		assert!(Status::decode(&Status::encode(State::Silenced, 0, None).unwrap()).is_err());
		// Only the failed state carries a reason, and it is never empty or
		// oversized.
		assert!(Status::decode(&Status::encode(State::Recording, 5, Some("x")).unwrap()).is_err());
		assert!(Status::encode(State::Failed, 0, Some("")).is_err());
		assert!(Status::encode(State::Failed, 0, Some(&"x".repeat(MAX_REASON_BYTES + 1))).is_err());
		assert!(Status::decode(&[5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
	}

	#[test]
	fn states_decode_their_codes() {
		for (code, expected) in [
			(0, State::Idle),
			(1, State::Recording),
			(2, State::Interrupted),
			(3, State::Silenced),
			(4, State::Failed),
		] {
			assert_eq!(State::from_code(code), Ok(expected));
		}
		assert_eq!(State::from_code(5), Err(Error::InvalidArgument));
	}
}
