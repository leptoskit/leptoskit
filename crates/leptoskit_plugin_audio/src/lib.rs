//! <!-- {=p_audioSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_audio` as a direct dependency and select the `audio`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Linux hosts register it too; generated Windows and
//! terminal hosts currently omit runtime plugins, so use explicit registration
//! in a custom host there. Guests enable the `guest` feature and reuse
//! `guest::Audio` rather than resolving the plugin on every call. iOS, Android,
//! macOS, and Linux play through one Rust engine that decodes with symphonia
//! and outputs with cpal; the browser uses `web::Audio` over Web Audio.
//! HarmonyOS has no audio output yet.
//!
//! <!-- {/p_audioSummary} -->
//!
//! Sounds are loaded once and played as many voices as needed. A voice can
//! loop, fade in, change volume over a ramp, pass through low-pass and
//! high-pass filters whose cutoff glides, pause, and stop with a fade. The
//! master bus has the same volume and filters, so a whole soundtrack can be
//! veiled, held while the app is in the background, or silenced at once.
//!
//! Native hosts run one Rust engine: [`symphonia`](https://docs.rs/symphonia)
//! decodes Ogg Vorbis, WAV, MP3, FLAC and AAC; long music streams from disk
//! while short effects are decoded once and kept in memory; a real-time mixer
//! takes its commands over a lock-free ring buffer and never allocates or
//! locks on the audio thread; and [`cpal`](https://docs.rs/cpal) plays the
//! result through Core Audio, `AAudio`, WASAPI or ALSA. Browser apps use
//! [`web::Audio`] over Web Audio instead.
//!
//! Rust plugins cannot push events to the guest yet, so the end of a voice
//! and audio-session interruptions are queued and read with
//! [`guest::Audio::events`], typically once per frame.
#![cfg_attr(
	all(feature = "host", any(target_os = "android", target_os = "ios")),
	allow(unsafe_code)
)]

use std::time::Duration;

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

pub mod protocol;

#[cfg(all(
	feature = "host",
	not(target_arch = "wasm32"),
	not(target_env = "ohos")
))]
mod host;
#[cfg(all(
	feature = "host",
	not(target_arch = "wasm32"),
	not(target_env = "ohos")
))]
pub use host::Audio;
#[cfg(all(
	feature = "host",
	not(target_arch = "wasm32"),
	not(target_env = "ohos")
))]
pub use host::Rendering;
#[cfg(all(
	feature = "host",
	not(target_arch = "wasm32"),
	not(target_env = "ohos")
))]
pub use host::from_context;
#[cfg(all(
	feature = "host",
	not(target_arch = "wasm32"),
	not(target_env = "ohos")
))]
pub use host::offline;

#[cfg(all(feature = "browser", target_arch = "wasm32", target_os = "unknown"))]
pub mod web;

/// Stable runtime registration name.
pub const NAME: &str = "audio";
/// Sets the audio session: [`Session`]. Empty reply.
pub const CONFIGURE: MethodId = MethodId(0);
/// Loads a [`Source`] in a [`LoadMode`]. Replies with [`SoundInfo`].
pub const LOAD: MethodId = MethodId(1);
/// Releases a [`Sound`], stopping its voices at once. Empty reply.
pub const UNLOAD: MethodId = MethodId(2);
/// Starts a [`Voice`] of a [`Sound`] with [`Play`] options. Replies with the
/// voice.
pub const PLAY: MethodId = MethodId(3);
/// Ramps the volume of a voice or of the master bus. Empty reply.
pub const SET_VOLUME: MethodId = MethodId(4);
/// Fades out and stops one voice or every voice. Empty reply.
pub const STOP: MethodId = MethodId(5);
/// Pauses or resumes one voice or the whole output. Empty reply.
pub const PAUSE: MethodId = MethodId(6);
/// Sets or clears one filter slot of a voice or of the master bus. Empty
/// reply.
pub const SET_FILTER: MethodId = MethodId(7);
/// Takes the queued [`Event`]s, oldest first.
pub const TAKE_EVENTS: MethodId = MethodId(8);
/// Supported wire operations.
pub const METHODS: &[MethodId] = &[
	CONFIGURE,
	LOAD,
	UNLOAD,
	PLAY,
	SET_VOLUME,
	STOP,
	PAUSE,
	SET_FILTER,
	TAKE_EVENTS,
];

/// Filter slots on each voice and on the master bus.
pub const FILTER_SLOTS: u32 = 2;
/// The loudest volume accepted: four times the recording's own level.
pub const MAX_VOLUME: f32 = 4.0;
/// The longest fade, ramp or glide accepted.
pub const MAX_FADE: Duration = Duration::from_secs(600);
/// The lowest filter cutoff accepted, in hertz.
pub const MIN_CUTOFF: f32 = 10.0;
/// The highest filter cutoff accepted, in hertz. Engines also hold the cutoff
/// below half their output rate.
pub const MAX_CUTOFF: f32 = 22_000.0;
/// The resonance (Q) range accepted. 0.707 is a plain filter with no peak.
pub const RESONANCE: std::ops::RangeInclusive<f32> = 0.1..=20.0;
/// The longest asset or file path accepted, in bytes.
pub const MAX_PATH_BYTES: usize = 1024;

/// A loaded sound. Play it as many times as needed; unload it when done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sound(pub u32);

/// One playing instance of a sound. Identifiers are never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Voice(pub u32);

/// What a volume, pause, stop or filter change applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
	/// The master bus, after every voice is mixed: every sound at once.
	Master,
	/// One voice.
	Voice(Voice),
}

impl Target {
	fn wire(self) -> u32 {
		match self {
			Self::Master => 0,
			Self::Voice(voice) => voice.0,
		}
	}

	fn from_wire(value: u32) -> Self {
		match value {
			0 => Self::Master,
			id => Self::Voice(Voice(id)),
		}
	}
}

/// How the app's sound shares the device with other apps and the silent
/// switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum Category {
	/// Sound that belongs to the app's look and feel, such as game music and
	/// effects. It mixes with other apps' audio and, on iOS, is silenced by
	/// the Ring/Silent switch and the screen lock. Android requests no audio
	/// focus.
	#[default]
	Ambient = 0,
	/// Sound the user asked to hear. It plays with the silent switch on and,
	/// unless [`Session::mix_with_others`] is set, interrupts other apps'
	/// audio. Android requests audio focus, so the system ducks the app
	/// under navigation prompts and other transient sounds.
	Playback = 1,
}

/// The audio session the app asks the operating system for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Session {
	/// Ambient or playback.
	pub category: Category,
	/// Playback only: keep other apps' audio playing alongside.
	pub mix_with_others: bool,
	/// Playback only: lower other apps' audio while this app plays.
	pub duck_others: bool,
}

/// Where a sound's encoded bytes come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source<'a> {
	/// A path inside the app's `assets/` folder, such as `music/hunt.ogg`.
	/// Browsers fetch `/assets/<path>`; native hosts read the app bundle's
	/// `assets` folder (Android: the APK's assets).
	Asset(&'a str),
	/// A path inside the app's private data directory.
	Data(&'a str),
	/// A path inside the app's cache directory.
	Cache(&'a str),
	/// The encoded file itself: Ogg, WAV, MP3, FLAC or AAC, at most the
	/// bridge's 8 MiB payload limit.
	Bytes(&'a [u8]),
}

/// Whether a sound is decoded up front or as it plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum LoadMode {
	/// Stream files larger than 512 KiB, decode smaller ones into memory.
	#[default]
	Auto = 0,
	/// Decode everything now: no work while playing, and many voices of it
	/// at once cost almost nothing. For short effects.
	Memory = 1,
	/// Decode while playing, a second ahead. For music, which would take tens
	/// of megabytes once decoded.
	Stream = 2,
}

/// What [`LOAD`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundInfo {
	/// The handle to play it with.
	pub sound: Sound,
	/// The recording's own sample rate, in hertz.
	pub sample_rate: u32,
	/// The recording's channel count.
	pub channels: u32,
	/// Its length in frames at [`Self::sample_rate`], when the container
	/// says.
	pub frames: Option<u64>,
	/// Whether it streams while playing rather than sitting in memory.
	pub streamed: bool,
}

impl SoundInfo {
	/// Its length, when known.
	pub fn duration(&self) -> Option<Duration> {
		let frames = self.frames?;
		Some(Duration::from_secs_f64(
			frames as f64 / f64::from(self.sample_rate.max(1)),
		))
	}
}

/// How a voice starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Play {
	/// Linear gain, 0 to [`MAX_VOLUME`]; 1 is the recording's level.
	pub volume: f32,
	/// Stereo position, -1 (left) to 1 (right).
	pub pan: f32,
	/// Start again from the beginning, without a gap, at the end.
	pub looping: bool,
	/// Rise from silence over this long.
	pub fade_in: Duration,
	/// Start paused, ready to resume.
	pub paused: bool,
}

impl Default for Play {
	fn default() -> Self {
		Self {
			volume: 1.0,
			pan: 0.0,
			looping: false,
			fade_in: Duration::ZERO,
			paused: false,
		}
	}
}

impl Play {
	/// Full volume, centred, once, no fade.
	pub fn new() -> Self {
		Self::default()
	}

	/// Sets the volume.
	#[must_use]
	pub fn volume(mut self, volume: f32) -> Self {
		self.volume = volume;
		self
	}

	/// Sets the stereo position.
	#[must_use]
	pub fn pan(mut self, pan: f32) -> Self {
		self.pan = pan;
		self
	}

	/// Loops the sound.
	#[must_use]
	pub fn looping(mut self, looping: bool) -> Self {
		self.looping = looping;
		self
	}

	/// Fades in from silence.
	#[must_use]
	pub fn fade_in(mut self, fade_in: Duration) -> Self {
		self.fade_in = fade_in;
		self
	}

	/// Starts paused.
	#[must_use]
	pub fn paused(mut self, paused: bool) -> Self {
		self.paused = paused;
		self
	}
}

/// The shape of a filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum FilterKind {
	/// Passes what is below the cutoff: muffles, as if through a wall.
	LowPass = 1,
	/// Passes what is above the cutoff: thins, as if through a telephone.
	HighPass = 2,
}

/// A resonant two-pole filter, the response of a biquad.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Filter {
	/// Low-pass or high-pass.
	pub kind: FilterKind,
	/// Where the filter starts to cut, in hertz.
	pub cutoff: f32,
	/// The peak at the cutoff (Q). 0.707 has none; higher values ring.
	pub resonance: f32,
}

impl Filter {
	/// A low-pass filter with no resonant peak.
	pub fn low_pass(cutoff: f32) -> Self {
		Self {
			kind: FilterKind::LowPass,
			cutoff,
			resonance: std::f32::consts::FRAC_1_SQRT_2,
		}
	}

	/// A high-pass filter with no resonant peak.
	pub fn high_pass(cutoff: f32) -> Self {
		Self {
			kind: FilterKind::HighPass,
			cutoff,
			resonance: std::f32::consts::FRAC_1_SQRT_2,
		}
	}

	/// Sets the resonance.
	#[must_use]
	pub fn resonance(mut self, resonance: f32) -> Self {
		self.resonance = resonance;
		self
	}
}

/// Something that happened since the events were last taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
	/// A voice that does not loop played to its end. Stopped voices do not
	/// report this.
	Ended(Voice),
	/// The operating system took the audio away: a phone call, an alarm, or
	/// another app's playback. Output is silent until [`Self::Resumed`].
	Interrupted,
	/// The interruption ended and output carries on.
	Resumed,
}

/// Validates a volume.
pub fn check_volume(volume: f32) -> Result<f32, Error> {
	if (0.0..=MAX_VOLUME).contains(&volume) {
		Ok(volume)
	} else {
		Err(Error::InvalidArgument)
	}
}

/// Validates a pan position.
pub fn check_pan(pan: f32) -> Result<f32, Error> {
	if (-1.0..=1.0).contains(&pan) {
		Ok(pan)
	} else {
		Err(Error::InvalidArgument)
	}
}

/// Validates a fade, ramp or glide length, in milliseconds.
pub fn check_fade(milliseconds: u32) -> Result<Duration, Error> {
	let fade = Duration::from_millis(u64::from(milliseconds));

	if fade <= MAX_FADE {
		Ok(fade)
	} else {
		Err(Error::InvalidArgument)
	}
}

/// Validates a filter.
pub fn check_filter(filter: Filter) -> Result<Filter, Error> {
	if (MIN_CUTOFF..=MAX_CUTOFF).contains(&filter.cutoff) && RESONANCE.contains(&filter.resonance) {
		Ok(filter)
	} else {
		Err(Error::InvalidArgument)
	}
}

/// Validates a relative path: no root, no `..`, no backslashes or NULs, and
/// at most [`MAX_PATH_BYTES`].
pub fn check_path(path: &str) -> Result<&str, Error> {
	let bad = path.is_empty()
		|| path.len() > MAX_PATH_BYTES
		|| path.starts_with('/')
		|| path.contains(['\\', '\0'])
		|| path
			.split('/')
			.any(|part| part.is_empty() || part == "." || part == "..");

	if bad {
		Err(Error::InvalidArgument)
	} else {
		Ok(path)
	}
}

fn milliseconds(duration: Duration) -> u32 {
	u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

/// Native interpreter client; resolves the plugin only once.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use std::time::Duration;

	use leptoskit_bridge::Error;
	use leptoskit_bridge::MethodId;

	use super::*;

	/// Cached native plugin handle.
	#[derive(Debug, Clone, Copy)]
	pub struct Audio(leptoskit_bridge::guest::Client);

	impl Audio {
		/// Resolves the host-installed plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		async fn call(
			self,
			method: MethodId,
			payload: &[u8],
		) -> Result<leptoskit_bridge::Bytes, Error> {
			self.0.call(method, payload)?.await
		}

		async fn unit(self, method: MethodId, payload: &[u8]) -> Result<(), Error> {
			protocol::decode_unit(&self.call(method, payload).await?)
		}

		/// Asks the operating system for an audio session.
		pub async fn configure(self, session: Session) -> Result<(), Error> {
			self.unit(CONFIGURE, &protocol::encode_session(session))
				.await
		}

		/// Loads a sound.
		pub async fn load(self, source: Source<'_>, mode: LoadMode) -> Result<SoundInfo, Error> {
			protocol::decode_sound_info(
				&self
					.call(LOAD, &protocol::encode_load(source, mode)?)
					.await?,
			)
		}

		/// Releases a sound and stops its voices.
		pub async fn unload(self, sound: Sound) -> Result<(), Error> {
			self.unit(UNLOAD, &sound.0.to_le_bytes()).await
		}

		/// Starts a voice.
		pub async fn play(self, sound: Sound, play: Play) -> Result<Voice, Error> {
			protocol::decode_voice(
				&self
					.call(PLAY, &protocol::encode_play(sound, play)?)
					.await?,
			)
		}

		/// Ramps a voice's volume, or the master volume, over `ramp`.
		pub async fn set_volume(
			self,
			target: Target,
			volume: f32,
			ramp: Duration,
		) -> Result<(), Error> {
			self.unit(SET_VOLUME, &protocol::encode_volume(target, volume, ramp)?)
				.await
		}

		/// Fades out and stops a voice, or every voice with
		/// [`Target::Master`].
		pub async fn stop(self, target: Target, fade: Duration) -> Result<(), Error> {
			self.unit(STOP, &protocol::encode_stop(target, fade)).await
		}

		/// Pauses or resumes a voice, or the whole output with
		/// [`Target::Master`], easing over `fade` so nothing clicks.
		pub async fn pause(
			self,
			target: Target,
			paused: bool,
			fade: Duration,
		) -> Result<(), Error> {
			self.unit(PAUSE, &protocol::encode_pause(target, paused, fade))
				.await
		}

		/// Sets filter `slot` of a voice or the master bus, gliding from the
		/// current cutoff over `glide`; `None` removes it.
		pub async fn set_filter(
			self,
			target: Target,
			slot: u32,
			filter: Option<Filter>,
			glide: Duration,
		) -> Result<(), Error> {
			self.unit(
				SET_FILTER,
				&protocol::encode_filter(target, slot, filter, glide)?,
			)
			.await
		}

		/// Starts `sound` rising over `fade` while `from` falls away over the
		/// same time.
		pub async fn crossfade(
			self,
			from: Option<Voice>,
			sound: Sound,
			play: Play,
			fade: Duration,
		) -> Result<Voice, Error> {
			let voice = self.play(sound, play.fade_in(fade)).await?;

			if let Some(from) = from {
				self.stop(Target::Voice(from), fade).await?;
			}

			Ok(voice)
		}

		/// Takes the events queued since the last call, oldest first.
		pub async fn events(self) -> Result<Vec<Event>, Error> {
			protocol::decode_events(&self.call(TAKE_EVENTS, &[]).await?)
		}
	}
}
