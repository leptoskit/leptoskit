//! <!-- {=p_live_photoSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_live_photo` as a direct dependency and select the
//! `live-photo` plugin in `native.plugins`; supported generated hosts discover
//! it from Cargo metadata. Generated Linux hosts register it where it has a
//! Linux Rust implementation; generated Windows and terminal hosts currently
//! omit runtime plugins, so use explicit registration in a custom host there.
//! Guests enable the `guest` feature and reuse `guest::LivePhoto` rather than
//! resolving the plugin on every call. Create a session from an app-owned Live
//! Photo asset URI, bind it with `LivePhoto(session)`, then start or stop
//! playback. iOS and macOS use `PHLivePhotoView`; Android, Windows, Linux,
//! HarmonyOS, and the browser return `Unavailable`. Editing and export remain
//! later scope.
//!
//! <!-- {/p_live_photoSummary} -->
//!
//! Live Photo wire v1: create takes a length-prefixed URI; start-playback takes
//! session + style; stop/status/release take a session id. Style is hint (0) or
//! full (1).

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "live-photo";
/// URI → session id.
pub const CREATE: MethodId = MethodId(0);
/// Session + style → empty reply.
pub const START_PLAYBACK: MethodId = MethodId(1);
/// Session → empty reply.
pub const STOP_PLAYBACK: MethodId = MethodId(2);
/// Session → encoded [`Status`].
pub const STATUS: MethodId = MethodId(3);
/// Session → empty reply; drops the host session.
pub const RELEASE: MethodId = MethodId(4);
/// Supported wire operations.
pub const METHODS: &[MethodId] = &[CREATE, START_PLAYBACK, STOP_PLAYBACK, STATUS, RELEASE];

const WIRE_VERSION: u32 = 1;
/// Maximum UTF-8 bytes for the source URI.
pub const MAX_URI_BYTES: usize = 4096;

/// A host-owned Live Photo session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub u32);

impl SessionId {
	/// Encodes the id as four little-endian bytes.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(4);
		output.u32(self.0);
		output.finish()
	}

	/// Decodes a session id, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let id = input.u32()?;
		input.finish()?;
		Ok(Self(id))
	}
}

/// Playback intensity for [`START_PLAYBACK`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PlaybackStyle {
	/// Short hint playback.
	Hint = 0,
	/// Full Live Photo playback.
	Full = 1,
}

impl PlaybackStyle {
	fn from_code(code: u32) -> Option<Self> {
		match code {
			0 => Some(Self::Hint),
			1 => Some(Self::Full),
			_ => None,
		}
	}
}

/// Create request: app-owned asset URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Create {
	/// Source URI (`file:` / `ph://` / content URI the host understands).
	uri: String,
}

impl Create {
	/// Rejects empty, NUL, and oversized URIs.
	pub fn new(uri: impl Into<String>) -> Result<Self, Error> {
		let uri = uri.into();
		if uri.is_empty() || uri.contains('\0') || uri.len() > MAX_URI_BYTES {
			return Err(Error::InvalidArgument);
		}
		Ok(Self { uri })
	}

	/// Source URI.
	#[must_use]
	pub fn uri(&self) -> &str {
		&self.uri
	}

	/// Encodes a length-prefixed URI.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.uri.len());
		output.bytes(self.uri.as_bytes())?;
		Ok(output.finish())
	}

	/// Decodes through [`Create::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let uri = input.string()?.to_owned();
		input.finish()?;
		Self::new(uri)
	}
}

/// Start playback for one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartPlayback {
	/// Target session.
	pub session: SessionId,
	/// Hint or full playback.
	pub style: PlaybackStyle,
}

impl StartPlayback {
	/// Encodes session id and style code.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(8);
		output.u32(self.session.0);
		output.u32(self.style as u32);
		output.finish()
	}

	/// Decodes session and style, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let session = SessionId(input.u32()?);
		let style = PlaybackStyle::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;
		input.finish()?;
		Ok(Self { session, style })
	}
}

/// A transport that names one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCommand {
	/// Target session.
	pub session: SessionId,
}

impl SessionCommand {
	/// Encodes the session id alone.
	#[must_use]
	pub fn encode(self) -> Bytes {
		self.session.encode()
	}

	/// Decodes a session id, rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		Ok(Self {
			session: SessionId::decode(bytes)?,
		})
	}
}

/// Snapshot of one Live Photo session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
	/// Whether the session is open.
	pub active: bool,
	/// Whether playback is currently running.
	pub playing: bool,
}

impl Status {
	/// Encodes version and the two flags.
	#[must_use]
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(12);
		output.u32(WIRE_VERSION);
		output.u32(u32::from(self.active));
		output.u32(u32::from(self.playing));
		output.finish()
	}

	/// Strictly validates a fixed 12-byte status reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != WIRE_VERSION {
			return Err(Error::InvalidArgument);
		}
		let active = decode_flag(input.u32()?)?;
		let playing = decode_flag(input.u32()?)?;
		input.finish()?;
		Ok(Self { active, playing })
	}
}

fn decode_flag(value: u32) -> Result<bool, Error> {
	match value {
		0 => Ok(false),
		1 => Ok(true),
		_ => Err(Error::InvalidArgument),
	}
}

/// Validates method payloads without invoking a host.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		CREATE => Create::decode(payload).map(|_| ()),
		START_PLAYBACK => StartPlayback::decode(payload).map(|_| ()),
		STOP_PLAYBACK | STATUS | RELEASE => SessionCommand::decode(payload).map(|_| ()),
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without Live Photo playback.
#[derive(Debug, Default)]
pub struct Unsupported;

impl leptoskit_bridge::Plugin for Unsupported {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		validate(method, &payload)?;
		Err(Error::Unavailable)
	}
}

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct LivePhoto(leptoskit_bridge::guest::Client);

	impl LivePhoto {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Opens a session for `uri` and returns its id.
		pub async fn create(self, uri: &str) -> Result<SessionId, Error> {
			SessionId::decode(&self.0.call(CREATE, &Create::new(uri)?.encode()?)?.await?)
		}

		/// Starts hint or full playback.
		pub async fn start_playback(self, request: StartPlayback) -> Result<(), Error> {
			require_empty(&self.0.call(START_PLAYBACK, &request.encode())?.await?)
		}

		/// Stops playback for `session`.
		pub async fn stop_playback(self, session: SessionId) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(STOP_PLAYBACK, &SessionCommand { session }.encode())?
					.await?,
			)
		}

		/// Snapshots active and playing state.
		pub async fn status(self, session: SessionId) -> Result<Status, Error> {
			Status::decode(
				&self
					.0
					.call(STATUS, &SessionCommand { session }.encode())?
					.await?,
			)
		}

		/// Releases the host session; bound `LivePhoto` views show nothing.
		pub async fn release(self, session: SessionId) -> Result<(), Error> {
			require_empty(
				&self
					.0
					.call(RELEASE, &SessionCommand { session }.encode())?
					.await?,
			)
		}
	}

	fn require_empty(bytes: &[u8]) -> Result<(), Error> {
		Decoder::new(bytes).finish()
	}
}

/// Browsers have no Live Photo host in this MVP.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always Unavailable without inventing a session.
	pub fn create(_uri: &str) -> Result<SessionId, Error> {
		Err(Error::Unavailable)
	}

	/// Always Unavailable.
	pub fn start_playback(_request: StartPlayback) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always Unavailable.
	pub fn stop_playback(_session: SessionId) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always Unavailable.
	pub fn status(_session: SessionId) -> Result<Status, Error> {
		Err(Error::Unavailable)
	}

	/// Always Unavailable.
	pub fn release(_session: SessionId) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	#[test]
	fn create_playback_and_session_codecs() {
		let create = Create::new("file:///tmp/a.pvt").unwrap();
		assert_eq!(create.uri(), "file:///tmp/a.pvt");
		assert_eq!(Create::decode(&create.encode().unwrap()), Ok(create));
		assert_eq!(Create::new(""), Err(Error::InvalidArgument));
		assert_eq!(Create::new("a\0b"), Err(Error::InvalidArgument));
		assert_eq!(
			Create::new("x".repeat(MAX_URI_BYTES + 1)),
			Err(Error::InvalidArgument)
		);
		let mut trailing = Create::new("file:///x").unwrap().encode().unwrap().to_vec();
		trailing.push(0);
		assert_eq!(Create::decode(&trailing), Err(Error::InvalidArgument));

		let start = StartPlayback {
			session: SessionId(2),
			style: PlaybackStyle::Full,
		};
		assert_eq!(StartPlayback::decode(&start.encode()), Ok(start));
		assert_eq!(
			StartPlayback::decode(&{
				let mut bad = start.encode().to_vec();
				bad[4] = 9;
				bad
			}),
			Err(Error::InvalidArgument)
		);
		assert_eq!(PlaybackStyle::from_code(0), Some(PlaybackStyle::Hint));
		assert_eq!(PlaybackStyle::from_code(2), None);

		assert_eq!(SessionId::decode(&SessionId(7).encode()), Ok(SessionId(7)));
		assert_eq!(
			SessionCommand::decode(
				&SessionCommand {
					session: SessionId(3)
				}
				.encode()
			),
			Ok(SessionCommand {
				session: SessionId(3)
			})
		);
		let mut trailing = SessionId(1).encode().to_vec();
		trailing.push(0);
		assert_eq!(SessionId::decode(&trailing), Err(Error::InvalidArgument));
	}

	#[test]
	fn status_validate_and_unsupported_never_invent_sessions() {
		let status = Status {
			active: true,
			playing: false,
		};
		assert_eq!(Status::decode(&status.encode()), Ok(status));
		let mut bad_version = status.encode().to_vec();
		bad_version[0] = 9;
		assert_eq!(Status::decode(&bad_version), Err(Error::InvalidArgument));
		let mut bad_flag = status.encode().to_vec();
		bad_flag[4] = 2;
		assert_eq!(Status::decode(&bad_flag), Err(Error::InvalidArgument));
		let mut bad_playing = status.encode().to_vec();
		bad_playing[8] = 3;
		assert_eq!(Status::decode(&bad_playing), Err(Error::InvalidArgument));
		let mut trailing = status.encode().to_vec();
		trailing.push(0);
		assert_eq!(Status::decode(&trailing), Err(Error::InvalidArgument));

		let create = Create::new("file:///tmp/a.pvt").unwrap().encode().unwrap();
		assert_eq!(validate(CREATE, &create), Ok(()));
		assert_eq!(
			validate(
				START_PLAYBACK,
				&StartPlayback {
					session: SessionId(1),
					style: PlaybackStyle::Hint,
				}
				.encode()
			),
			Ok(())
		);
		assert_eq!(
			validate(
				STOP_PLAYBACK,
				&SessionCommand {
					session: SessionId(1)
				}
				.encode()
			),
			Ok(())
		);
		assert_eq!(
			validate(
				STATUS,
				&SessionCommand {
					session: SessionId(1)
				}
				.encode()
			),
			Ok(())
		);
		assert_eq!(
			validate(
				RELEASE,
				&SessionCommand {
					session: SessionId(1)
				}
				.encode()
			),
			Ok(())
		);
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));
		assert_eq!(METHODS.len(), 5);
		assert_eq!(NAME, "live-photo");
		assert_eq!(decode_flag(0), Ok(false));
		assert_eq!(decode_flag(1), Ok(true));
		assert_eq!(decode_flag(2), Err(Error::InvalidArgument));

		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(plugin.invoke(CREATE, create), Err(Error::Unavailable));
	}
}
