//! The plugin's requests, end to end against an offline mixer.

use std::path::PathBuf;

use super::*;
use crate::Category;
use crate::FilterKind;

const RATE: u32 = 48_000;

fn context() -> AppContext {
	let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
	AppContext::new(fixtures.clone(), fixtures)
}

fn call(audio: &mut Audio, method: MethodId, payload: Vec<u8>) -> Result<Bytes, Error> {
	audio.invoke(method, Bytes::from(payload))
}

fn load(audio: &mut Audio, name: &str, mode: LoadMode) -> SoundInfo {
	let payload = protocol::encode_load(Source::Data(name), mode).unwrap();
	protocol::decode_sound_info(&call(audio, crate::LOAD, payload).unwrap()).unwrap()
}

fn play(audio: &mut Audio, sound: Sound, play: Play) -> Voice {
	let payload = protocol::encode_play(sound, play).unwrap();
	protocol::decode_voice(&call(audio, crate::PLAY, payload).unwrap()).unwrap()
}

fn events(audio: &mut Audio) -> Vec<Event> {
	protocol::decode_events(&call(audio, crate::TAKE_EVENTS, Vec::new()).unwrap()).unwrap()
}

fn render(mixer: &mut Rendering, seconds: f32) -> Vec<f32> {
	let mut output = vec![0.0; (seconds * RATE as f32) as usize * 2];
	mixer.render(&mut output, 2);
	output
}

fn peak(samples: &[f32]) -> f32 {
	samples
		.iter()
		.fold(0.0, |peak, sample| peak.max(sample.abs()))
}

#[test]
fn a_loaded_effect_plays_and_reports_its_end_once() {
	let (mut audio, mut mixer) = offline(&context(), RATE);
	let info = load(&mut audio, "tone.ogg", LoadMode::Auto);
	assert!(!info.streamed);
	assert_eq!(info.sample_rate, 44_100);
	assert_eq!(info.channels, 2);
	assert!(info.duration().unwrap() > Duration::from_millis(480));

	let first = play(&mut audio, info.sound, Play::new());
	let second = play(&mut audio, info.sound, Play::new().volume(0.5).pan(1.0));
	assert_ne!(first, second);
	let output = render(&mut mixer, 0.25);
	// Two voices of a 0.5 tone, one at half volume: 0.75 at the peaks.
	assert!((peak(&output) - 0.75).abs() < 0.05, "{}", peak(&output));
	assert_eq!(events(&mut audio), []);

	render(&mut mixer, 0.5);
	let mut ended = events(&mut audio);
	ended.sort_by_key(|event| {
		match event {
			Event::Ended(voice) => voice.0,
			_ => 0,
		}
	});
	assert_eq!(ended, [Event::Ended(first), Event::Ended(second)]);
	assert_eq!(events(&mut audio), []);

	// A change for a voice that has ended is accepted and does nothing.
	let late = protocol::encode_volume(Target::Voice(first), 0.1, Duration::ZERO).unwrap();
	assert!(call(&mut audio, crate::SET_VOLUME, late).is_ok());
	let unknown = protocol::encode_volume(Target::Voice(Voice(99)), 0.1, Duration::ZERO).unwrap();
	assert_eq!(
		call(&mut audio, crate::SET_VOLUME, unknown),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn streamed_music_loops_crossfades_and_stops_without_an_end_event() {
	let (mut audio, mut mixer) = offline(&context(), RATE);
	let music = load(&mut audio, "tone.ogg", LoadMode::Stream);
	let other = load(&mut audio, "tone.flac", LoadMode::Stream);
	assert!(music.streamed && other.streamed);

	let first = play(&mut audio, music.sound, Play::new().looping(true));
	render(&mut mixer, 0.1);

	// Three loops of a half-second piece, fed by the decoder thread.
	let mut loud = 0;

	for _ in 0..30 {
		std::thread::sleep(Duration::from_millis(5));
		let block = render(&mut mixer, 0.05);

		if peak(&block) > 0.4 {
			loud += 1;
		}
	}

	assert!(loud >= 28, "only {loud} of 30 blocks had the music");
	let second = play(
		&mut audio,
		other.sound,
		Play::new()
			.looping(true)
			.fade_in(Duration::from_millis(500)),
	);
	let stop = protocol::encode_stop(Target::Voice(first), Duration::from_millis(500));
	call(&mut audio, crate::STOP, stop).unwrap();

	for _ in 0..20 {
		std::thread::sleep(Duration::from_millis(5));
		render(&mut mixer, 0.05);
	}

	assert_eq!(events(&mut audio), []);
	assert_eq!(audio.voices.keys().copied().collect::<Vec<_>>(), [second.0]);
}

#[test]
fn the_master_bus_veils_holds_and_silences_everything() {
	let (mut audio, mut mixer) = offline(&context(), RATE);
	let tone = load(&mut audio, "tone.wav", LoadMode::Memory);
	play(&mut audio, tone.sound, Play::new().looping(true));
	let veil = protocol::encode_filter(
		Target::Master,
		0,
		Some(Filter {
			kind: FilterKind::HighPass,
			cutoff: 8_000.0,
			resonance: 0.707,
		}),
		Duration::from_millis(100),
	)
	.unwrap();
	call(&mut audio, crate::SET_FILTER, veil).unwrap();
	let output = render(&mut mixer, 0.5);
	// A 440 Hz tone under an 8 kHz high-pass is all but gone.
	assert!(peak(&output[RATE as usize..]) < 0.01);

	let hold = protocol::encode_pause(Target::Master, true, Duration::from_millis(20));
	call(&mut audio, crate::PAUSE, hold).unwrap();
	render(&mut mixer, 0.1);
	assert_eq!(peak(&render(&mut mixer, 0.1)), 0.0);
	assert!(audio.master.held);

	let resume = protocol::encode_pause(Target::Master, false, Duration::ZERO);
	call(&mut audio, crate::PAUSE, resume).unwrap();
	let open = protocol::encode_filter(Target::Master, 0, None, Duration::ZERO).unwrap();
	call(&mut audio, crate::SET_FILTER, open).unwrap();
	assert!(peak(&render(&mut mixer, 0.1)[2_000..]) > 0.45);

	let silence = protocol::encode_stop(Target::Master, Duration::from_millis(50));
	call(&mut audio, crate::STOP, silence).unwrap();
	render(&mut mixer, 0.1);
	assert_eq!(peak(&render(&mut mixer, 0.1)), 0.0);
	call(&mut audio, crate::TAKE_EVENTS, Vec::new()).unwrap();
	assert!(audio.voices.is_empty());
}

#[test]
fn unloading_cuts_its_voices_and_bad_requests_are_refused() {
	let (mut audio, mut mixer) = offline(&context(), RATE);
	let tone = load(&mut audio, "tone.mp3", LoadMode::Auto);
	assert_eq!(tone.channels, 1);

	for _ in 0..3 {
		play(&mut audio, tone.sound, Play::new().looping(true));
	}

	call(
		&mut audio,
		crate::UNLOAD,
		tone.sound.0.to_le_bytes().to_vec(),
	)
	.unwrap();
	render(&mut mixer, 0.05);
	assert_eq!(peak(&render(&mut mixer, 0.05)), 0.0);
	assert_eq!(events(&mut audio), []);
	assert!(audio.voices.is_empty());

	let replay = protocol::encode_play(tone.sound, Play::new()).unwrap();
	assert_eq!(
		call(&mut audio, crate::PLAY, replay),
		Err(Error::InvalidArgument)
	);
	let missing = protocol::encode_load(Source::Data("missing.ogg"), LoadMode::Auto).unwrap();
	assert_eq!(
		call(&mut audio, crate::LOAD, missing),
		Err(Error::Unavailable)
	);
	let garbage = protocol::encode_load(Source::Bytes(&[1, 2, 3, 4]), LoadMode::Memory).unwrap();
	assert_eq!(
		call(&mut audio, crate::LOAD, garbage),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		call(&mut audio, MethodId(42), Vec::new()),
		Err(Error::Unavailable)
	);
	// Desktops have no audio session to configure.
	let session = protocol::encode_session(Session {
		category: Category::Playback,
		..Session::default()
	});
	assert!(call(&mut audio, crate::CONFIGURE, session).is_ok());
}

/// The real device, silent: the master bus is at zero before anything
/// plays. Machines without an output device report `Unavailable`.
#[test]
fn a_device_plays_holds_and_stops_or_reports_that_there_is_none() {
	let mut audio = from_context(&context()).unwrap();
	let tone = load(&mut audio, "tone.wav", LoadMode::Memory);
	let mute = protocol::encode_volume(Target::Master, 0.0, Duration::ZERO).unwrap();
	call(&mut audio, crate::SET_VOLUME, mute).unwrap();
	let start = protocol::encode_play(tone.sound, Play::new()).unwrap();

	let voice = match call(&mut audio, crate::PLAY, start) {
		Ok(voice) => protocol::decode_voice(&voice).unwrap(),
		Err(error) => {
			assert_eq!(error, Error::Unavailable);
			return;
		}
	};

	let deadline = std::time::Instant::now() + Duration::from_secs(5);

	while !events(&mut audio).contains(&Event::Ended(voice)) {
		assert!(
			std::time::Instant::now() < deadline,
			"the voice never ended"
		);
		std::thread::sleep(Duration::from_millis(20));
	}

	let looping = play(&mut audio, tone.sound, Play::new().looping(true));
	let hold = protocol::encode_pause(Target::Master, true, Duration::from_millis(10));
	call(&mut audio, crate::PAUSE, hold).unwrap();
	std::thread::sleep(Duration::from_millis(100));
	let resume = protocol::encode_pause(Target::Master, false, Duration::ZERO);
	call(&mut audio, crate::PAUSE, resume).unwrap();
	let stop = protocol::encode_stop(Target::Voice(looping), Duration::from_millis(10));
	call(&mut audio, crate::STOP, stop).unwrap();

	while !audio.voices.is_empty() {
		assert!(
			std::time::Instant::now() < deadline,
			"the voice never stopped"
		);
		std::thread::sleep(Duration::from_millis(20));
		events(&mut audio);
	}
}

#[test]
fn voices_are_capped_and_bytes_load_like_files() {
	let (mut audio, _mixer) = offline(&context(), RATE);
	let bytes = std::fs::read(context().data_dir.join("tone.wav")).unwrap();
	let payload = protocol::encode_load(Source::Bytes(&bytes), LoadMode::Auto).unwrap();
	let info =
		protocol::decode_sound_info(&call(&mut audio, crate::LOAD, payload).unwrap()).unwrap();
	assert_eq!(info.frames, Some(24_000));

	for _ in 0..MAX_VOICES {
		play(&mut audio, info.sound, Play::new());
	}

	let one_more = protocol::encode_play(info.sound, Play::new()).unwrap();
	assert_eq!(call(&mut audio, crate::PLAY, one_more), Err(Error::Busy));
	assert_eq!(audio.name(), NAME);
	assert_eq!(audio.methods(), METHODS);
}
