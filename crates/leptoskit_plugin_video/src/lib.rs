//! <!-- {=p_videoSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_video` as a direct dependency and select the `video`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Linux hosts register it where it has a Linux Rust
//! implementation; generated Windows and terminal hosts currently omit runtime
//! plugins, so use explicit registration in a custom host there. Guests enable
//! the `guest` feature and reuse `guest::Video` rather than resolving the
//! plugin on every call. iOS and macOS use `AVPlayer` with an `AVPlayerLayer`
//! surface, Android uses Media3 `ExoPlayer` with a `PlayerView`, and the
//! browser uses an HTML `<video>` element; Windows, Linux, and HarmonyOS return
//! `Unavailable`.
//!
//! <!-- {/p_videoSummary} -->
//!
//! Video wire v1: create/replace take a length-prefixed UTF-8 URL; transport
//! methods take a player id plus optional flags or millisecond offsets; status
//! replies with version, playing, current time, and duration. All integers are
//! little-endian; trailing bytes are rejected.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "video";
/// Encoded URL -> player id.
pub const CREATE: MethodId = MethodId(0);
/// Player id and URL -> empty reply.
pub const REPLACE: MethodId = MethodId(1);
/// Player id -> empty reply.
pub const PLAY: MethodId = MethodId(2);
/// Player id -> empty reply.
pub const PAUSE: MethodId = MethodId(3);
/// Player id and seek milliseconds -> empty reply.
pub const SEEK: MethodId = MethodId(4);
/// Player id and muted flag -> empty reply.
pub const SET_MUTED: MethodId = MethodId(5);
/// Player id and loop flag -> empty reply.
pub const SET_LOOP: MethodId = MethodId(6);
/// Player id and rate thousandths -> empty reply.
pub const SET_RATE: MethodId = MethodId(7);
/// Player id -> encoded [`Status`].
pub const STATUS: MethodId = MethodId(8);
/// Player id -> empty reply; drops the host player.
pub const RELEASE: MethodId = MethodId(9);
/// Supported wire operations.
pub const METHODS: &[MethodId] = &[
	CREATE, REPLACE, PLAY, PAUSE, SEEK, SET_MUTED, SET_LOOP, SET_RATE, STATUS, RELEASE,
];

/// Wire protocol version for status replies and create/replace requests.
const WIRE_VERSION: u32 = 1;
/// Longest accepted source URL, in UTF-8 bytes.
pub const MAX_URL_BYTES: usize = 2048;
/// Playback rate thousandths: 0.25x through 2.0x.
pub const RATE_MIN: u32 = 250;
pub const RATE_MAX: u32 = 2000;
/// Normal playback rate (1.0x).
pub const RATE_NORMAL: u32 = 1000;
/// Unknown or unavailable duration / position sentinel.
pub const TIME_UNKNOWN: u32 = u32::MAX;

/// A host-owned video player. Identifiers are never reused within one plugin
/// instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u32);

impl PlayerId {
	/// Encodes the id as four little-endian bytes.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(4);
		output.u32(self.0);
		output.finish()
	}

	/// Decodes a player id, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let id = input.u32()?;
		input.finish()?;
		Ok(Self(id))
	}
}

/// A validated media source URL for create and replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
	url: String,
}

impl Source {
	/// Accepts a non-empty URL of at most [`MAX_URL_BYTES`] UTF-8 bytes.
	/// Schemes are not restricted here; the host may still refuse a URL.
	pub fn url(url: impl Into<String>) -> Result<Self, Error> {
		let url = url.into();
		if url.is_empty() {
			return Err(Error::InvalidArgument);
		}
		if url.len() > MAX_URL_BYTES {
			return Err(Error::TooLarge);
		}
		Ok(Self { url })
	}

	/// The validated URL string.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.url
	}

	/// Encodes version then the length-prefixed URL.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(8 + self.url.len());
		output.u32(WIRE_VERSION);
		output.bytes(self.url.as_bytes())?;
		Ok(output.finish())
	}

	/// Decodes through [`Source::url`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != WIRE_VERSION {
			return Err(Error::InvalidArgument);
		}
		let url = input.string()?;
		if url.len() > MAX_URL_BYTES {
			return Err(Error::TooLarge);
		}
		let source = Self::url(url)?;
		input.finish()?;
		Ok(source)
	}

	/// Encodes a player id then a replace source.
	pub fn encode_replace(player: PlayerId, source: &Self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(12 + source.url.len());
		output.u32(player.0);
		output.u32(WIRE_VERSION);
		output.bytes(source.url.as_bytes())?;
		Ok(output.finish())
	}

	/// Decodes replace as player id then source.
	pub fn decode_replace(bytes: &[u8]) -> Result<(PlayerId, Self), Error> {
		let mut input = Decoder::new(bytes);
		let player = PlayerId(input.u32()?);
		if input.u32()? != WIRE_VERSION {
			return Err(Error::InvalidArgument);
		}
		let url = input.string()?;
		if url.len() > MAX_URL_BYTES {
			return Err(Error::TooLarge);
		}
		let source = Self::url(url)?;
		input.finish()?;
		Ok((player, source))
	}
}

/// A transport or property change that names one player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerCommand {
	/// Target player.
	pub player: PlayerId,
}

impl PlayerCommand {
	/// Encodes the player id alone.
	#[must_use]
	pub fn encode(self) -> Bytes {
		self.player.encode()
	}

	/// Decodes a player id, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		Ok(Self {
			player: PlayerId::decode(bytes)?,
		})
	}
}

/// Seek to a position in whole milliseconds from the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seek {
	/// Target player.
	pub player: PlayerId,
	/// Milliseconds from the start of the media.
	pub position_ms: u32,
}

impl Seek {
	/// Builds a seek; `position_ms` must not be [`TIME_UNKNOWN`].
	pub fn new(player: PlayerId, position_ms: u32) -> Result<Self, Error> {
		if position_ms == TIME_UNKNOWN {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			player,
			position_ms,
		})
	}

	/// Encodes player id and position.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(8);
		output.u32(self.player.0);
		output.u32(self.position_ms);
		output.finish()
	}

	/// Decodes through [`Seek::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let player = PlayerId(input.u32()?);
		let position_ms = input.u32()?;
		input.finish()?;
		Self::new(player, position_ms)
	}
}

/// A boolean player property (muted or loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerFlag {
	/// Target player.
	pub player: PlayerId,
	/// Whether the property is on.
	pub enabled: bool,
}

impl PlayerFlag {
	/// Encodes player id and a 0/1 flag.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(8);
		output.u32(self.player.0);
		output.u32(u32::from(self.enabled));
		output.finish()
	}

	/// Decodes a player id and boolean flag.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let player = PlayerId(input.u32()?);
		let enabled = decode_flag(input.u32()?)?;
		input.finish()?;
		Ok(Self { player, enabled })
	}
}

/// Playback rate in thousandths of real time (1000 = 1.0x).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
	/// Target player.
	pub player: PlayerId,
	/// Rate thousandths in [`RATE_MIN`]..=[`RATE_MAX`].
	pub thousandths: u32,
}

impl Rate {
	/// Validates the portable rate range.
	pub fn new(player: PlayerId, thousandths: u32) -> Result<Self, Error> {
		if !(RATE_MIN..=RATE_MAX).contains(&thousandths) {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			player,
			thousandths,
		})
	}

	/// Encodes player id and rate.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(8);
		output.u32(self.player.0);
		output.u32(self.thousandths);
		output.finish()
	}

	/// Decodes through [`Rate::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let player = PlayerId(input.u32()?);
		let thousandths = input.u32()?;
		input.finish()?;
		Self::new(player, thousandths)
	}
}

/// Snapshot of one player's transport state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
	/// Whether the host currently considers the player playing.
	pub playing: bool,
	/// Current position in milliseconds, or [`TIME_UNKNOWN`].
	pub current_ms: u32,
	/// Media duration in milliseconds, or [`TIME_UNKNOWN`] when not yet known.
	pub duration_ms: u32,
}

impl Status {
	/// Encodes version and the three status fields.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(16);
		output.u32(WIRE_VERSION);
		output.u32(u32::from(self.playing));
		output.u32(self.current_ms);
		output.u32(self.duration_ms);
		output.finish()
	}

	/// Strictly validates a fixed 16-byte status reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != WIRE_VERSION {
			return Err(Error::InvalidArgument);
		}
		let playing = decode_flag(input.u32()?)?;
		let current_ms = input.u32()?;
		let duration_ms = input.u32()?;
		input.finish()?;
		Ok(Self {
			playing,
			current_ms,
			duration_ms,
		})
	}
}

fn decode_flag(value: u32) -> Result<bool, Error> {
	match value {
		0 => Ok(false),
		1 => Ok(true),
		_ => Err(Error::InvalidArgument),
	}
}

/// Cached native guest client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// One resolved video plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Video(leptoskit_bridge::guest::Client);

	impl Video {
		/// Resolves the installed plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Creates a player for `source` and returns its id.
		pub async fn create(self, source: &Source) -> Result<PlayerId, Error> {
			PlayerId::decode(&self.0.call(CREATE, &source.encode()?)?.await?)
		}

		/// Replaces the media on an existing player.
		pub async fn replace(self, player: PlayerId, source: &Source) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(REPLACE, &Source::encode_replace(player, source)?)?
					.await?,
			)
		}

		/// Starts or resumes playback.
		pub async fn play(self, player: PlayerId) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(PLAY, &PlayerCommand { player }.encode())?
					.await?,
			)
		}

		/// Pauses playback, keeping the current position.
		pub async fn pause(self, player: PlayerId) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(PAUSE, &PlayerCommand { player }.encode())?
					.await?,
			)
		}

		/// Seeks to `position_ms` from the start.
		pub async fn seek(self, player: PlayerId, position_ms: u32) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(SEEK, &Seek::new(player, position_ms)?.encode())?
					.await?,
			)
		}

		/// Mutes or unmutes the player.
		pub async fn set_muted(self, player: PlayerId, muted: bool) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(
						SET_MUTED,
						&PlayerFlag {
							player,
							enabled: muted,
						}
						.encode(),
					)?
					.await?,
			)
		}

		/// Enables or disables looping.
		pub async fn set_loop(self, player: PlayerId, looping: bool) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(
						SET_LOOP,
						&PlayerFlag {
							player,
							enabled: looping,
						}
						.encode(),
					)?
					.await?,
			)
		}

		/// Sets playback rate in thousandths of real time.
		pub async fn set_rate(self, player: PlayerId, thousandths: u32) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(SET_RATE, &Rate::new(player, thousandths)?.encode())?
					.await?,
			)
		}

		/// Snapshots playing state, current time, and duration.
		pub async fn status(self, player: PlayerId) -> Result<Status, Error> {
			Status::decode(
				&self
					.0
					.call(STATUS, &PlayerCommand { player }.encode())?
					.await?,
			)
		}

		/// Releases the host player; bound `Video` views show nothing.
		pub async fn release(self, player: PlayerId) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(RELEASE, &PlayerCommand { player }.encode())?
					.await?,
			)
		}
	}

	fn require_empty(bytes: &[u8]) -> Result<(), Error> {
		Decoder::new(bytes).finish()
	}
}

/// Browser Direct module: HTML `<video>` elements keyed by player id.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

#[cfg(test)]
mod tests {
	use super::*;

	const SAMPLE_URL: &str = "https://example.com/clip.mp4";

	#[test]
	fn source_round_trip_and_bounds() {
		let source = Source::url(SAMPLE_URL).unwrap();
		assert_eq!(source.as_str(), SAMPLE_URL);
		assert_eq!(
			Source::decode(&source.encode().unwrap()),
			Ok(source.clone())
		);
		assert_eq!(Source::url(""), Err(Error::InvalidArgument));
		assert_eq!(
			Source::url("x".repeat(MAX_URL_BYTES + 1)),
			Err(Error::TooLarge)
		);
		let encoded = source.encode().unwrap();
		for length in 0..encoded.len() {
			assert!(Source::decode(&encoded[..length]).is_err());
		}
		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert_eq!(Source::decode(&trailing), Err(Error::InvalidArgument));
		let mut bad_version = encoded.to_vec();
		bad_version[0] = 2;
		assert_eq!(Source::decode(&bad_version), Err(Error::InvalidArgument));

		// Oversized wire that bypasses Source::url construction.
		let oversized = "x".repeat(MAX_URL_BYTES + 1);
		let mut wire = Encoder::with_capacity(8 + oversized.len());
		wire.u32(WIRE_VERSION);
		wire.bytes(oversized.as_bytes()).unwrap();
		assert_eq!(Source::decode(&wire.finish()), Err(Error::TooLarge));
	}

	#[test]
	fn replace_round_trip_and_rejects_trailing() {
		let source = Source::url(SAMPLE_URL).unwrap();
		let encoded = Source::encode_replace(PlayerId(7), &source).unwrap();
		assert_eq!(Source::decode_replace(&encoded), Ok((PlayerId(7), source)));
		let mut trailing = encoded.to_vec();
		trailing.push(1);
		assert_eq!(
			Source::decode_replace(&trailing),
			Err(Error::InvalidArgument)
		);
		assert!(Source::decode_replace(&encoded[..encoded.len() - 1]).is_err());

		let mut bad_version = encoded.to_vec();
		bad_version[4] = 9;
		assert_eq!(
			Source::decode_replace(&bad_version),
			Err(Error::InvalidArgument)
		);

		let oversized = "x".repeat(MAX_URL_BYTES + 1);
		let mut wire = Encoder::with_capacity(12 + oversized.len());
		wire.u32(3);
		wire.u32(WIRE_VERSION);
		wire.bytes(oversized.as_bytes()).unwrap();
		assert_eq!(Source::decode_replace(&wire.finish()), Err(Error::TooLarge));
	}

	#[test]
	fn player_command_and_id_codecs() {
		assert_eq!(PlayerId::decode(&PlayerId(42).encode()), Ok(PlayerId(42)));
		assert_eq!(
			PlayerCommand::decode(
				&PlayerCommand {
					player: PlayerId(9)
				}
				.encode()
			),
			Ok(PlayerCommand {
				player: PlayerId(9)
			})
		);
		let mut trailing = PlayerId(1).encode().to_vec();
		trailing.push(0);
		assert_eq!(PlayerId::decode(&trailing), Err(Error::InvalidArgument));
		assert!(PlayerId::decode(&[]).is_err());
	}

	#[test]
	fn seek_flag_and_rate_codecs() {
		let seek = Seek::new(PlayerId(3), 1_500).unwrap();
		assert_eq!(Seek::decode(&seek.encode()), Ok(seek));
		assert_eq!(
			Seek::new(PlayerId(1), TIME_UNKNOWN),
			Err(Error::InvalidArgument)
		);

		let flag = PlayerFlag {
			player: PlayerId(2),
			enabled: true,
		};
		assert_eq!(PlayerFlag::decode(&flag.encode()), Ok(flag));
		let mut bad_flag = flag.encode().to_vec();
		bad_flag[4] = 2;
		assert_eq!(PlayerFlag::decode(&bad_flag), Err(Error::InvalidArgument));

		let rate = Rate::new(PlayerId(4), 1_250).unwrap();
		assert_eq!(Rate::decode(&rate.encode()), Ok(rate));
		assert_eq!(
			Rate::new(PlayerId(1), RATE_MIN - 1),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Rate::new(PlayerId(1), RATE_MAX + 1),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Rate::new(PlayerId(1), RATE_NORMAL).unwrap().thousandths,
			RATE_NORMAL
		);
	}

	#[test]
	fn status_round_trip_and_rejects_bad_flags() {
		let status = Status {
			playing: true,
			current_ms: 250,
			duration_ms: 10_000,
		};
		assert_eq!(Status::decode(&status.encode()), Ok(status));
		let unknown = Status {
			playing: false,
			current_ms: TIME_UNKNOWN,
			duration_ms: TIME_UNKNOWN,
		};
		assert_eq!(Status::decode(&unknown.encode()), Ok(unknown));
		let mut bad = status.encode().to_vec();
		bad[4] = 3;
		assert_eq!(Status::decode(&bad), Err(Error::InvalidArgument));
		let mut trailing = status.encode().to_vec();
		trailing.push(0);
		assert_eq!(Status::decode(&trailing), Err(Error::InvalidArgument));
		assert!(Status::decode(&status.encode()[..12]).is_err());
		let mut bad_version = status.encode().to_vec();
		bad_version[0] = 9;
		assert_eq!(Status::decode(&bad_version), Err(Error::InvalidArgument));
	}

	#[test]
	fn methods_cover_the_mvp_surface() {
		assert_eq!(METHODS.len(), 10);
		assert_eq!(CREATE, MethodId(0));
		assert_eq!(RELEASE, MethodId(9));
		assert_eq!(NAME, "video");
	}

	#[test]
	fn empty_reply_helper_rejects_trailing() {
		assert!(Decoder::new(&[]).finish().is_ok());
		assert_eq!(Decoder::new(&[0]).finish(), Err(Error::InvalidArgument));
	}
}
