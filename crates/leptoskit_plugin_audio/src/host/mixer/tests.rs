//! The mixer, rendered offline into buffers.

use super::super::dsp::tests::rms;
use super::super::dsp::tests::sine;
use super::*;

const RATE: u32 = 48_000;

fn tone(frequency: f32, seconds: f32) -> Arc<Pcm> {
	let mono = sine(frequency, RATE as f32, (seconds * RATE as f32) as usize);
	Arc::new(Pcm::from_mono(RATE, &mono))
}

fn start(volume: f32) -> Start {
	Start {
		source_rate: RATE,
		volume,
		pan: 0.0,
		looping: false,
		fade_in: 0.0,
		paused: false,
	}
}

fn memory(pcm: &Arc<Pcm>) -> Feed {
	Feed::Memory {
		pcm: Arc::clone(pcm),
		frame: 0,
	}
}

/// Renders `frames` stereo frames.
fn render(mixer: &mut Mixer, frames: usize) -> Vec<f32> {
	let mut output = vec![0.0; frames * 2];
	mixer.render(&mut output, 2);
	output
}

fn left(stereo: &[f32]) -> Vec<f32> {
	stereo.iter().step_by(2).copied().collect()
}

fn largest_step(samples: &[f32]) -> f32 {
	samples
		.windows(2)
		.map(|pair| (pair[1] - pair[0]).abs())
		.fold(0.0, f32::max)
}

fn send(handle: &mut Handle, command: Command) {
	handle.commands.push(command).unwrap();
}

#[test]
fn a_voice_plays_its_sound_and_reports_its_end() {
	let (mut mixer, mut handle) = Mixer::new(RATE);
	let sound = tone(440.0, 0.1);
	send(
		&mut handle,
		Command::Play(VoiceState::new(1, memory(&sound), start(1.0), RATE)),
	);

	let output = left(&render(&mut mixer, 4_800 + 64));
	// Two frames late, otherwise exactly the recording.
	for (played, recorded) in output[2..4_802].iter().zip(sound.left()) {
		assert!((played - recorded).abs() < 1e-6);
	}

	let retired = handle.retired.pop().unwrap();
	assert_eq!(retired.voice.id, 1);
	assert!(retired.ended);
	assert!(output[4_806..].iter().all(|sample| *sample == 0.0));
}

#[test]
fn a_loop_joins_end_to_start_without_a_seam() {
	let (mut mixer, mut handle) = Mixer::new(RATE);
	// Exactly 100 cycles of 1 kHz, so the loop point is mid-waveform
	// continuous: any seam would show as a step.
	let sound = tone(1_000.0, 0.1);
	send(
		&mut handle,
		Command::Play(VoiceState::new(
			1,
			memory(&sound),
			Start {
				looping: true,
				..start(1.0)
			},
			RATE,
		)),
	);

	let output = left(&render(&mut mixer, 48_000));
	let natural = largest_step(&sine(1_000.0, RATE as f32, 1_000));
	assert!(largest_step(&output[2..]) <= natural + 1e-4);
	assert!(handle.retired.pop().is_err());

	// And a resampled loop is just as smooth.
	let (mut mixer, mut handle) = Mixer::new(44_100);
	send(
		&mut handle,
		Command::Play(VoiceState::new(
			1,
			memory(&sound),
			Start {
				looping: true,
				..start(1.0)
			},
			44_100,
		)),
	);
	let output = left(&render(&mut mixer, 44_100));
	let natural = largest_step(&sine(1_000.0, 44_100.0, 1_000));
	assert!(largest_step(&output[4..]) <= natural * 1.02);
}

#[test]
fn volume_changes_ramp_without_clicks() {
	let (mut mixer, mut handle) = Mixer::new(RATE);
	let dc = Arc::new(Pcm::from_mono(RATE, &vec![0.5; 96_000]));
	send(
		&mut handle,
		Command::Play(VoiceState::new(1, memory(&dc), start(1.0), RATE)),
	);
	render(&mut mixer, 4_800);
	// An instant change still takes the shortest ramp.
	send(
		&mut handle,
		Command::Volume {
			target: 1,
			volume: 0.0,
			frames: frames(0.0, RATE as f32),
		},
	);
	let output = left(&render(&mut mixer, 4_800));
	assert!(largest_step(&output) <= 0.5 / 240.0 + 1e-6);
	assert!(output[240..].iter().all(|sample| sample.abs() < 1e-6));

	send(
		&mut handle,
		Command::Volume {
			target: 0,
			volume: 0.5,
			frames: 0,
		},
	);
	send(
		&mut handle,
		Command::Volume {
			target: 1,
			volume: 1.0,
			frames: 4_800,
		},
	);
	let output = left(&render(&mut mixer, 9_600));
	assert!(largest_step(&output) <= 0.25 / 4_800.0 + 1e-6);
	assert!((output[9_000] - 0.25).abs() < 1e-6);
}

#[test]
fn a_crossfade_keeps_the_level_and_stops_the_old_voice_without_an_end_event() {
	let (mut mixer, mut handle) = Mixer::new(RATE);
	let dc = Arc::new(Pcm::from_mono(RATE, &vec![0.5; 4_800]));
	let looping = Start {
		looping: true,
		..start(1.0)
	};
	send(
		&mut handle,
		Command::Play(VoiceState::new(1, memory(&dc), looping, RATE)),
	);
	render(&mut mixer, 480);
	send(
		&mut handle,
		Command::Stop {
			target: 1,
			frames: 9_600,
		},
	);
	send(
		&mut handle,
		Command::Play(VoiceState::new(
			2,
			memory(&dc),
			Start {
				fade_in: 0.2,
				..looping
			},
			RATE,
		)),
	);
	let output = left(&render(&mut mixer, 19_200));

	// A linear crossfade of equal material keeps the sum level.
	assert!(output[4..].iter().all(|sample| (sample - 0.5).abs() < 1e-3));
	let retired = handle.retired.pop().unwrap();
	assert_eq!(retired.voice.id, 1);
	assert!(!retired.ended);
}

#[test]
fn pausing_eases_out_holds_its_place_and_eases_back() {
	let (mut mixer, mut handle) = Mixer::new(RATE);
	let ramp: Vec<f32> = (0..48_000).map(|frame| frame as f32 / 48_000.0).collect();
	let sound = Arc::new(Pcm::from_mono(RATE, &ramp));
	send(
		&mut handle,
		Command::Play(VoiceState::new(1, memory(&sound), start(1.0), RATE)),
	);
	render(&mut mixer, 1_000);
	send(
		&mut handle,
		Command::Pause {
			target: 1,
			paused: true,
			frames: 480,
		},
	);
	let fading = left(&render(&mut mixer, 1_000));
	assert!(largest_step(&fading) < 0.01);

	assert!(fading[480..].iter().all(|sample| *sample == 0.0));

	send(
		&mut handle,
		Command::Pause {
			target: 1,
			paused: false,
			frames: 480,
		},
	);
	let resumed = left(&render(&mut mixer, 1_000));
	// It picks up where the fade-out left it, not where time has moved to.
	assert!((resumed[999] - 2_477.0 / 48_000.0).abs() < 1e-5);
	assert!(largest_step(&resumed) < 0.01);
}

#[test]
fn a_paused_voice_that_is_stopped_ends_silently() {
	let (mut mixer, mut handle) = Mixer::new(RATE);
	let sound = tone(440.0, 1.0);
	send(
		&mut handle,
		Command::Play(VoiceState::new(
			1,
			memory(&sound),
			Start {
				paused: true,
				looping: true,
				..start(1.0)
			},
			RATE,
		)),
	);
	render(&mut mixer, 480);
	send(
		&mut handle,
		Command::Stop {
			target: 1,
			frames: 480,
		},
	);
	let output = render(&mut mixer, 1_000);
	assert!(output.iter().all(|sample| *sample == 0.0));
	assert!(!handle.retired.pop().unwrap().ended);
}

#[test]
fn holding_the_master_freezes_every_voice() {
	let (mut mixer, mut handle) = Mixer::new(RATE);
	let sound = tone(440.0, 1.0);
	send(
		&mut handle,
		Command::Play(VoiceState::new(1, memory(&sound), start(1.0), RATE)),
	);
	send(
		&mut handle,
		Command::Pause {
			target: 0,
			paused: true,
			frames: 240,
		},
	);
	let output = left(&render(&mut mixer, 48_000 * 2));
	assert!(output[240..].iter().all(|sample| *sample == 0.0));
	// Two seconds held and nothing ended.
	assert!(handle.retired.pop().is_err());
	send(
		&mut handle,
		Command::Pause {
			target: 0,
			paused: false,
			frames: 240,
		},
	);

	assert!(rms(&left(&render(&mut mixer, 4_800))[240..]) > 0.6);
}

#[test]
fn the_master_veil_muffles_everything() {
	let (mut mixer, mut handle) = Mixer::new(RATE);
	let bright = tone(5_000.0, 1.0);
	send(
		&mut handle,
		Command::Play(VoiceState::new(1, memory(&bright), start(1.0), RATE)),
	);
	send(
		&mut handle,
		Command::Filter {
			target: 0,
			slot: 0,
			filter: Some(Filter::low_pass(500.0)),
			frames: 0,
		},
	);
	let output = left(&render(&mut mixer, 24_000));
	assert!(rms(&output[4_800..]) < 0.012 * std::f32::consts::FRAC_1_SQRT_2);
}

#[test]
fn stopping_everything_fades_and_mono_or_surround_outputs_are_filled() {
	let (mut mixer, mut handle) = Mixer::new(RATE);
	let dc = Arc::new(Pcm::from_mono(RATE, &vec![0.5; 48_000]));

	for id in 1..=3 {
		send(
			&mut handle,
			Command::Play(VoiceState::new(
				id,
				memory(&dc),
				Start {
					pan: -1.0,
					..start(0.25)
				},
				RATE,
			)),
		);
	}

	send(
		&mut handle,
		Command::Stop {
			target: 0,
			frames: 480,
		},
	);

	let mut surround = vec![1.0; 600 * 6];
	mixer.render(&mut surround, 6);
	// Hard left: the right channel and the extra channels are silent.
	assert!((surround[12] - 3.0 * 0.125).abs() < 0.01);
	assert!(
		surround
			.chunks(6)
			.all(|frame| frame[1..].iter().all(|s| *s == 0.0))
	);
	assert_eq!(surround[490 * 6], 0.0);

	for _ in 0..3 {
		assert!(!handle.retired.pop().unwrap().ended);
	}

	let mut mono = vec![1.0; 10];
	mixer.render(&mut mono, 1);
	assert!(mono.iter().all(|sample| *sample == 0.0));
}

#[test]
fn a_starved_stream_is_silent_until_its_frames_arrive_and_ends_when_finished() {
	let (mut mixer, mut handle) = Mixer::new(RATE);
	let (mut producer, consumer) = RingBuffer::new(1_024);
	let finished = Arc::new(AtomicBool::new(false));
	let feed = Feed::Stream {
		frames: consumer,
		finished: Arc::clone(&finished),
	};
	send(
		&mut handle,
		Command::Play(VoiceState::new(1, feed, start(1.0), RATE)),
	);
	assert!(render(&mut mixer, 100).iter().all(|sample| *sample == 0.0));

	for _ in 0..100 {
		producer.push(0.5).unwrap();
		producer.push(-0.5).unwrap();
	}

	// The silence it was given while starved is still two frames ahead.
	let output = render(&mut mixer, 100);
	assert!((output[4] - 0.5).abs() < 1e-6 && (output[5] + 0.5).abs() < 1e-6);
	assert!(handle.retired.pop().is_err());
	finished.store(true, Ordering::Release);
	render(&mut mixer, 100);
	assert!(handle.retired.pop().unwrap().ended);
}
