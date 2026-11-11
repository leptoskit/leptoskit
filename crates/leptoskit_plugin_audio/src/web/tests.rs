//! The Web Audio engine in a real browser, muted.

use wasm_bindgen_test::*;

use super::*;

wasm_bindgen_test_configure!(run_in_browser);

const TONE: &[u8] = include_bytes!("../../tests/fixtures/tone.wav");

#[wasm_bindgen_test]
async fn a_sound_decodes_plays_ramps_filters_and_stops() {
	exercise_sound(Audio::new().unwrap()).await;
}

#[wasm_bindgen_test]
async fn a_sound_preserves_its_length_at_both_output_rates() {
	for sample_rate in [44_100, 48_000] {
		let options = web_sys::AudioContextOptions::new();
		options.set_sample_rate(sample_rate as f32);
		let context = AudioContext::new_with_context_options(&options).unwrap();
		assert_eq!(context.sample_rate() as u32, sample_rate);
		exercise_sound(Audio::with_context(context).unwrap()).await;
	}
}

async fn exercise_sound(audio: Audio) {
	let info = audio
		.load(Source::Bytes(TONE), LoadMode::Memory)
		.await
		.unwrap();
	assert_eq!(info.channels, 2);
	assert_eq!(info.sample_rate, audio.0.context.sample_rate() as u32);
	let frames = info.frames.expect("a buffered sound has a known length");
	// The fixture is half a second; Web Audio resamples it to the context's
	// rate and may round its output length by one frame.
	let expected_frames = u64::from(info.sample_rate) / 2;
	assert!(
		frames.abs_diff(expected_frames) <= 1,
		"half-second clip at {} Hz: {frames} frames, expected {expected_frames} within one frame",
		info.sample_rate
	);

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

	if let Some(unlock) = audio.0.unlock.borrow_mut().take()
		&& let Some(document) = page()
	{
		listen(&document, &unlock, false);
	}
	JsFuture::from(audio.0.context.close().unwrap())
		.await
		.unwrap();
}
