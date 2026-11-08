//! The Web Audio engine in a real browser, muted.

use wasm_bindgen_test::*;

use super::*;

wasm_bindgen_test_configure!(run_in_browser);

const TONE: &[u8] = include_bytes!("../../tests/fixtures/tone.wav");

#[wasm_bindgen_test]
async fn a_sound_decodes_plays_ramps_filters_and_stops() {
	let audio = Audio::new().unwrap();
	let info = audio
		.load(Source::Bytes(TONE), LoadMode::Memory)
		.await
		.unwrap();
	assert_eq!(info.channels, 2);
	assert_eq!(info.frames, Some(24_000));

	let voice = audio
		.play(info.sound, Play::new().volume(0.0).looping(true))
		.unwrap();
	audio
		.set_volume(Target::Voice(voice), 0.0, Duration::from_millis(20))
		.unwrap();
	audio
		.set_filter(
			Target::Master,
			0,
			Some(Filter::low_pass(500.0)),
			Duration::from_millis(50),
		)
		.unwrap();
	audio
		.pause(Target::Voice(voice), true, Duration::ZERO)
		.unwrap();
	audio
		.pause(Target::Voice(voice), false, Duration::ZERO)
		.unwrap();
	audio.stop(Target::Master, Duration::ZERO).unwrap();
	assert_eq!(audio.events(), []);
	assert_eq!(
		audio.set_volume(Target::Voice(Voice(99)), 0.5, Duration::ZERO),
		Err(Error::InvalidArgument)
	);

	assert_eq!(
		audio
			.load(Source::Bytes(&[1, 2, 3]), LoadMode::Memory)
			.await
			.unwrap_err(),
		Error::InvalidArgument
	);
	assert_eq!(
		audio
			.load(Source::Data("x.ogg"), LoadMode::Auto)
			.await
			.unwrap_err(),
		Error::Unavailable
	);
}
