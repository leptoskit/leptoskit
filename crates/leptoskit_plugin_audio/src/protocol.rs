//! The binary wire format between guests and the native engine.
//!
//! Every integer is a little-endian `u32`; floats travel as their `u32` bits;
//! strings and bytes carry a `u32` length prefix. Decoding validates every
//! field and rejects trailing bytes. Target `0` is the master bus.

use std::time::Duration;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

use super::*;

const SOURCE_ASSET: u32 = 0;
const SOURCE_DATA: u32 = 1;
const SOURCE_CACHE: u32 = 2;
const SOURCE_BYTES: u32 = 3;
const EVENT_ENDED: u32 = 0;
const EVENT_INTERRUPTED: u32 = 1;
const EVENT_RESUMED: u32 = 2;
const FILTER_OFF: u32 = 0;

const PLAY_LOOPING: u32 = 1;
const PLAY_PAUSED: u32 = 2;
const SESSION_MIX: u32 = 1;
const SESSION_DUCK: u32 = 2;
/// Unknown frame counts travel as this.
const UNKNOWN_FRAMES: u64 = u64::MAX;

/// A decoded, validated call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Request<'a> {
	/// [`CONFIGURE`].
	Configure(Session),
	/// [`LOAD`].
	Load(Source<'a>, LoadMode),
	/// [`UNLOAD`].
	Unload(Sound),
	/// [`PLAY`].
	Play(Sound, Play),
	/// [`SET_VOLUME`].
	SetVolume(Target, f32, Duration),
	/// [`STOP`].
	Stop(Target, Duration),
	/// [`PAUSE`].
	Pause(Target, bool, Duration),
	/// [`SET_FILTER`].
	SetFilter(Target, u32, Option<Filter>, Duration),
	/// [`TAKE_EVENTS`].
	TakeEvents,
}

/// Decodes and validates a call.
pub fn decode(method: MethodId, payload: &[u8]) -> Result<Request<'_>, Error> {
	let mut input = Decoder::new(payload);

	let request = match method {
		CONFIGURE => Request::Configure(session(&mut input)?),
		LOAD => {
			let kind = input.u32()?;
			let mode = match input.u32()? {
				0 => LoadMode::Auto,
				1 => LoadMode::Memory,
				2 => LoadMode::Stream,
				_ => return Err(Error::InvalidArgument),
			};
			let source = match kind {
				SOURCE_ASSET => Source::Asset(check_path(input.string()?)?),
				SOURCE_DATA => Source::Data(check_path(input.string()?)?),
				SOURCE_CACHE => Source::Cache(check_path(input.string()?)?),
				SOURCE_BYTES => Source::Bytes(non_empty(input.bytes()?)?),
				_ => return Err(Error::InvalidArgument),
			};

			Request::Load(source, mode)
		}
		UNLOAD => Request::Unload(Sound(non_zero(input.u32()?)?)),
		PLAY => {
			let sound = Sound(non_zero(input.u32()?)?);
			let volume = check_volume(float(&mut input)?)?;
			let pan = check_pan(float(&mut input)?)?;
			let fade_in = check_fade(input.u32()?)?;
			let flags = flags(&mut input, PLAY_LOOPING | PLAY_PAUSED)?;
			Request::Play(
				sound,
				Play {
					volume,
					pan,
					looping: flags & PLAY_LOOPING != 0,
					fade_in,
					paused: flags & PLAY_PAUSED != 0,
				},
			)
		}
		SET_VOLUME => {
			let target = Target::from_wire(input.u32()?);
			let volume = check_volume(float(&mut input)?)?;
			Request::SetVolume(target, volume, check_fade(input.u32()?)?)
		}
		STOP => Request::Stop(Target::from_wire(input.u32()?), check_fade(input.u32()?)?),
		PAUSE => {
			let target = Target::from_wire(input.u32()?);
			let paused = match input.u32()? {
				0 => false,
				1 => true,
				_ => return Err(Error::InvalidArgument),
			};

			Request::Pause(target, paused, check_fade(input.u32()?)?)
		}
		SET_FILTER => {
			let target = Target::from_wire(input.u32()?);
			let slot = input.u32()?;

			if slot >= FILTER_SLOTS {
				return Err(Error::InvalidArgument);
			}

			let kind = input.u32()?;
			let cutoff = float(&mut input)?;
			let resonance = float(&mut input)?;
			let glide = check_fade(input.u32()?)?;
			let kind = match kind {
				FILTER_OFF => None,
				1 => Some(FilterKind::LowPass),
				2 => Some(FilterKind::HighPass),
				_ => return Err(Error::InvalidArgument),
			};

			let filter = kind
				.map(|kind| {
					check_filter(Filter {
						kind,
						cutoff,
						resonance,
					})
				})
				.transpose()?;
			Request::SetFilter(target, slot, filter, glide)
		}
		TAKE_EVENTS => Request::TakeEvents,
		_ => return Err(Error::Unavailable),
	};

	input.finish()?;
	Ok(request)
}

fn session(input: &mut Decoder<'_>) -> Result<Session, Error> {
	let category = match input.u32()? {
		0 => Category::Ambient,
		1 => Category::Playback,
		_ => return Err(Error::InvalidArgument),
	};

	let flags = flags(input, SESSION_MIX | SESSION_DUCK)?;

	Ok(Session {
		category,
		mix_with_others: flags & SESSION_MIX != 0,
		duck_others: flags & SESSION_DUCK != 0,
	})
}

fn float(input: &mut Decoder<'_>) -> Result<f32, Error> {
	let value = f32::from_bits(input.u32()?);

	if value.is_finite() {
		Ok(value)
	} else {
		Err(Error::InvalidArgument)
	}
}

fn flags(input: &mut Decoder<'_>, known: u32) -> Result<u32, Error> {
	let flags = input.u32()?;

	if flags & !known == 0 {
		Ok(flags)
	} else {
		Err(Error::InvalidArgument)
	}
}

fn non_zero(value: u32) -> Result<u32, Error> {
	if value == 0 {
		Err(Error::InvalidArgument)
	} else {
		Ok(value)
	}
}

fn non_empty(bytes: &[u8]) -> Result<&[u8], Error> {
	if bytes.is_empty() {
		Err(Error::InvalidArgument)
	} else {
		Ok(bytes)
	}
}

fn float_bits(output: &mut Encoder, value: f32) {
	output.u32(value.to_bits());
}

/// Encodes a [`CONFIGURE`] payload.
pub fn encode_session(session: Session) -> Vec<u8> {
	let mut output = Encoder::with_capacity(8);
	output.u32(session.category as u32);
	output.u32(
		if session.mix_with_others {
			SESSION_MIX
		} else {
			0
		} | if session.duck_others { SESSION_DUCK } else { 0 },
	);
	output.finish().to_vec()
}

/// Encodes a [`LOAD`] payload, validating paths.
pub fn encode_load(source: Source<'_>, mode: LoadMode) -> Result<Vec<u8>, Error> {
	let (kind, bytes) = match source {
		Source::Asset(path) => (SOURCE_ASSET, check_path(path)?.as_bytes()),
		Source::Data(path) => (SOURCE_DATA, check_path(path)?.as_bytes()),
		Source::Cache(path) => (SOURCE_CACHE, check_path(path)?.as_bytes()),
		Source::Bytes(bytes) => (SOURCE_BYTES, non_empty(bytes)?),
	};

	let mut output = Encoder::with_capacity(12 + bytes.len());
	output.u32(kind);
	output.u32(mode as u32);
	output.bytes(bytes)?;
	Ok(output.finish().to_vec())
}

/// Encodes a [`PLAY`] payload, validating the options.
pub fn encode_play(sound: Sound, play: Play) -> Result<Vec<u8>, Error> {
	let mut output = Encoder::with_capacity(20);
	output.u32(non_zero(sound.0)?);
	float_bits(&mut output, check_volume(play.volume)?);
	float_bits(&mut output, check_pan(play.pan)?);
	output.u32(fade(play.fade_in)?);
	output.u32(
		if play.looping { PLAY_LOOPING } else { 0 } | if play.paused { PLAY_PAUSED } else { 0 },
	);
	Ok(output.finish().to_vec())
}

fn fade(duration: Duration) -> Result<u32, Error> {
	if duration <= MAX_FADE {
		Ok(milliseconds(duration))
	} else {
		Err(Error::InvalidArgument)
	}
}

/// Encodes a [`SET_VOLUME`] payload.
pub fn encode_volume(target: Target, volume: f32, ramp: Duration) -> Result<Vec<u8>, Error> {
	let mut output = Encoder::with_capacity(12);
	output.u32(target.wire());
	float_bits(&mut output, check_volume(volume)?);
	output.u32(fade(ramp)?);
	Ok(output.finish().to_vec())
}

/// Encodes a [`STOP`] payload; fades longer than [`MAX_FADE`] are shortened.
pub fn encode_stop(target: Target, fade: Duration) -> Vec<u8> {
	let mut output = Encoder::with_capacity(8);
	output.u32(target.wire());
	output.u32(milliseconds(fade.min(MAX_FADE)));
	output.finish().to_vec()
}

/// Encodes a [`PAUSE`] payload; fades longer than [`MAX_FADE`] are
/// shortened.
pub fn encode_pause(target: Target, paused: bool, fade: Duration) -> Vec<u8> {
	let mut output = Encoder::with_capacity(12);
	output.u32(target.wire());
	output.u32(u32::from(paused));
	output.u32(milliseconds(fade.min(MAX_FADE)));
	output.finish().to_vec()
}

/// Encodes a [`SET_FILTER`] payload.
pub fn encode_filter(
	target: Target,
	slot: u32,
	filter: Option<Filter>,
	glide: Duration,
) -> Result<Vec<u8>, Error> {
	if slot >= FILTER_SLOTS {
		return Err(Error::InvalidArgument);
	}

	let mut output = Encoder::with_capacity(24);
	output.u32(target.wire());
	output.u32(slot);

	if let Some(filter) = filter {
		let filter = check_filter(filter)?;
		output.u32(filter.kind as u32);
		float_bits(&mut output, filter.cutoff);
		float_bits(&mut output, filter.resonance);
	} else {
		output.u32(FILTER_OFF);
		float_bits(&mut output, MAX_CUTOFF);
		float_bits(&mut output, std::f32::consts::FRAC_1_SQRT_2);
	}

	output.u32(fade(glide)?);
	Ok(output.finish().to_vec())
}

/// Encodes a [`LOAD`] reply.
pub fn encode_sound_info(info: SoundInfo) -> Bytes {
	let frames = info.frames.unwrap_or(UNKNOWN_FRAMES);
	let mut output = Encoder::with_capacity(24);
	output.u32(info.sound.0);
	output.u32(info.sample_rate);
	output.u32(info.channels);
	output.u32(frames as u32);
	output.u32((frames >> 32) as u32);
	output.u32(u32::from(info.streamed));

	output.finish()
}

/// Decodes a [`LOAD`] reply.
pub fn decode_sound_info(payload: &[u8]) -> Result<SoundInfo, Error> {
	let mut input = Decoder::new(payload);
	let sound = Sound(non_zero(input.u32()?)?);
	let sample_rate = input.u32()?;
	let channels = input.u32()?;
	let frames = u64::from(input.u32()?) | (u64::from(input.u32()?) << 32);

	let streamed = match input.u32()? {
		0 => false,
		1 => true,
		_ => return Err(Error::InvalidArgument),
	};

	input.finish()?;
	Ok(SoundInfo {
		sound,
		sample_rate,
		channels,
		frames: (frames != UNKNOWN_FRAMES).then_some(frames),
		streamed,
	})
}

/// Encodes a [`PLAY`] reply.
pub fn encode_voice(voice: Voice) -> Bytes {
	Bytes::copy_from_slice(&voice.0.to_le_bytes())
}

/// Decodes a [`PLAY`] reply.
pub fn decode_voice(payload: &[u8]) -> Result<Voice, Error> {
	let mut input = Decoder::new(payload);
	let voice = Voice(non_zero(input.u32()?)?);
	input.finish()?;
	Ok(voice)
}

/// Decodes an empty reply.
pub fn decode_unit(payload: &[u8]) -> Result<(), Error> {
	Decoder::new(payload).finish()
}

/// Encodes a [`TAKE_EVENTS`] reply.
pub fn encode_events(events: impl ExactSizeIterator<Item = Event>) -> Result<Bytes, Error> {
	let count = u32::try_from(events.len()).map_err(|_| Error::TooLarge)?;
	let mut output = Encoder::with_capacity(4 + events.len() * 8);
	output.u32(count);

	for event in events {
		let (kind, voice) = match event {
			Event::Ended(voice) => (EVENT_ENDED, voice.0),
			Event::Interrupted => (EVENT_INTERRUPTED, 0),
			Event::Resumed => (EVENT_RESUMED, 0),
		};

		output.u32(kind);
		output.u32(voice);
	}

	Ok(output.finish())
}

/// Decodes a [`TAKE_EVENTS`] reply.
pub fn decode_events(payload: &[u8]) -> Result<Vec<Event>, Error> {
	let mut input = Decoder::new(payload);
	let count = input.u32()? as usize;

	if count > payload.len() / 8 {
		return Err(Error::InvalidArgument);
	}

	let mut events = Vec::with_capacity(count);

	for _ in 0..count {
		let kind = input.u32()?;
		let voice = input.u32()?;
		events.push(match (kind, voice) {
			(EVENT_ENDED, voice) => Event::Ended(Voice(non_zero(voice)?)),
			(EVENT_INTERRUPTED, 0) => Event::Interrupted,
			(EVENT_RESUMED, 0) => Event::Resumed,
			_ => return Err(Error::InvalidArgument),
		});
	}

	input.finish()?;
	Ok(events)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_request_round_trips() {
		let session = Session {
			category: Category::Playback,
			mix_with_others: true,
			duck_others: false,
		};
		assert_eq!(
			decode(CONFIGURE, &encode_session(session)),
			Ok(Request::Configure(session))
		);

		let load = encode_load(Source::Asset("music/hunt.ogg"), LoadMode::Stream).unwrap();
		assert_eq!(
			decode(LOAD, &load),
			Ok(Request::Load(
				Source::Asset("music/hunt.ogg"),
				LoadMode::Stream
			))
		);
		let bytes = encode_load(Source::Bytes(b"RIFF"), LoadMode::Memory).unwrap();
		assert_eq!(
			decode(LOAD, &bytes),
			Ok(Request::Load(Source::Bytes(b"RIFF"), LoadMode::Memory))
		);

		let play = Play::new()
			.volume(0.5)
			.pan(-0.25)
			.looping(true)
			.fade_in(Duration::from_millis(1500));
		assert_eq!(
			decode(PLAY, &encode_play(Sound(3), play).unwrap()),
			Ok(Request::Play(Sound(3), play))
		);

		let volume = encode_volume(Target::Master, 0.25, Duration::from_millis(180)).unwrap();
		assert_eq!(
			decode(SET_VOLUME, &volume),
			Ok(Request::SetVolume(
				Target::Master,
				0.25,
				Duration::from_millis(180)
			))
		);
		assert_eq!(
			decode(
				STOP,
				&encode_stop(Target::Voice(Voice(9)), Duration::from_secs(2))
			),
			Ok(Request::Stop(
				Target::Voice(Voice(9)),
				Duration::from_secs(2)
			))
		);
		assert_eq!(
			decode(
				PAUSE,
				&encode_pause(Target::Master, true, Duration::from_millis(50))
			),
			Ok(Request::Pause(
				Target::Master,
				true,
				Duration::from_millis(50)
			))
		);

		let veil = Filter::low_pass(800.0).resonance(2.0);
		let filter =
			encode_filter(Target::Master, 1, Some(veil), Duration::from_millis(400)).unwrap();
		assert_eq!(
			decode(SET_FILTER, &filter),
			Ok(Request::SetFilter(
				Target::Master,
				1,
				Some(veil),
				Duration::from_millis(400)
			))
		);
		let off = encode_filter(Target::Voice(Voice(2)), 0, None, Duration::ZERO).unwrap();
		assert_eq!(
			decode(SET_FILTER, &off),
			Ok(Request::SetFilter(
				Target::Voice(Voice(2)),
				0,
				None,
				Duration::ZERO
			))
		);
		assert_eq!(decode(TAKE_EVENTS, &[]), Ok(Request::TakeEvents));
		assert_eq!(decode(MethodId(99), &[]), Err(Error::Unavailable));
	}

	#[test]
	fn invalid_values_and_trailing_bytes_are_rejected() {
		assert_eq!(
			encode_play(Sound(1), Play::new().volume(4.5)),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			encode_play(Sound(1), Play::new().pan(1.5)),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			encode_play(Sound(0), Play::new()),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			encode_filter(Target::Master, 2, None, Duration::ZERO),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			encode_filter(
				Target::Master,
				0,
				Some(Filter::low_pass(5.0)),
				Duration::ZERO
			),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			encode_volume(Target::Master, 1.0, Duration::from_secs(601)),
			Err(Error::InvalidArgument)
		);

		for path in [
			"",
			"/etc/passwd",
			"../secret",
			"a//b",
			"a/./b",
			"a\\b",
			"a/..",
		] {
			assert_eq!(
				encode_load(Source::Asset(path), LoadMode::Auto),
				Err(Error::InvalidArgument),
				"{path}"
			);
		}

		assert_eq!(
			encode_load(Source::Bytes(&[]), LoadMode::Auto),
			Err(Error::InvalidArgument)
		);

		let mut nan = encode_volume(Target::Master, 1.0, Duration::ZERO).unwrap();
		nan[4..8].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
		assert_eq!(decode(SET_VOLUME, &nan), Err(Error::InvalidArgument));

		let mut trailing = encode_stop(Target::Master, Duration::ZERO);
		trailing.push(0);
		assert_eq!(decode(STOP, &trailing), Err(Error::InvalidArgument));

		let mut flags = encode_play(Sound(1), Play::new()).unwrap();
		flags[16] = 4;
		assert_eq!(decode(PLAY, &flags), Err(Error::InvalidArgument));
		assert_eq!(decode(UNLOAD, &[1, 0, 0]), Err(Error::InvalidArgument));
		assert_eq!(decode(TAKE_EVENTS, &[0]), Err(Error::InvalidArgument));
	}

	#[test]
	fn replies_round_trip() {
		let info = SoundInfo {
			sound: Sound(4),
			sample_rate: 44_100,
			channels: 2,
			frames: Some(5_000_000_000),
			streamed: true,
		};
		assert_eq!(decode_sound_info(&encode_sound_info(info)), Ok(info));
		let unknown = SoundInfo {
			frames: None,
			..info
		};
		assert_eq!(decode_sound_info(&encode_sound_info(unknown)), Ok(unknown));
		assert_eq!(
			info.duration().map(|duration| duration.as_secs()),
			Some(113_378)
		);

		assert_eq!(decode_voice(&encode_voice(Voice(7))), Ok(Voice(7)));
		assert_eq!(decode_voice(&[0, 0, 0, 0]), Err(Error::InvalidArgument));

		let events = [Event::Ended(Voice(3)), Event::Interrupted, Event::Resumed];
		let encoded = encode_events(events.into_iter()).unwrap();
		assert_eq!(decode_events(&encoded), Ok(events.to_vec()));
		assert_eq!(decode_events(&[9, 0, 0, 0]), Err(Error::InvalidArgument));
		assert_eq!(decode_unit(&[]), Ok(()));
	}
}
