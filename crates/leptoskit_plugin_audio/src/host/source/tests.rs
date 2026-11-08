//! Decoding the committed fixtures, whole and streamed.

use super::*;

pub fn fixture(name: &str) -> Encoded {
	Encoded::File(
		PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("tests/fixtures")
			.join(name),
	)
}

/// Every fixture is the same half-second, 440 Hz stereo tone at 0.5.
fn check_tone(pcm: &Pcm, rate: u32) {
	assert_eq!(pcm.rate(), rate);
	let frames = pcm.frames();
	assert!(
		(rate as usize / 2 - 1_200..=rate as usize / 2 + 2_400).contains(&frames),
		"{frames} frames"
	);
	let middle: Vec<f32> = (frames / 4..frames * 3 / 4)
		.map(|frame| pcm.frame(frame)[0])
		.collect();
	let peak = middle
		.iter()
		.fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
	assert!((0.45..0.56).contains(&peak), "peak {peak}");
	// A 440 Hz tone crosses zero 880 times a second.
	let crossings = middle
		.windows(2)
		.filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
		.count() as f32;
	let expected = 440.0 * middle.len() as f32 / rate as f32;

	assert!(
		(crossings - expected).abs() <= 2.0,
		"{crossings} vs {expected}"
	);
}

#[test]
fn every_format_decodes_to_the_same_tone() {
	for (name, rate) in [
		("tone.wav", 48_000),
		("tone.ogg", 44_100),
		("tone.mp3", 44_100),
		("tone.flac", 48_000),
		("tone.m4a", 44_100),
	] {
		let decoder =
			Decoder::open(&fixture(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
		assert_eq!(
			decoder.shape().channels,
			if name == "tone.mp3" { 1 } else { 2 },
			"{name}"
		);
		check_tone(&decoder.decode_all().unwrap(), rate);
	}
}

#[test]
fn bytes_decode_like_files_and_garbage_is_refused() {
	let bytes: Arc<[u8]> =
		std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.ogg"))
			.unwrap()
			.into();
	let pcm = Decoder::open(&Encoded::Bytes(bytes))
		.unwrap()
		.decode_all()
		.unwrap();
	check_tone(&pcm, 44_100);

	let garbage: Arc<[u8]> = vec![7; 4_096].into();
	assert_eq!(
		Decoder::open(&Encoded::Bytes(garbage)).unwrap_err(),
		Error::InvalidArgument
	);
	assert_eq!(
		Decoder::open(&fixture("missing.ogg")).unwrap_err(),
		Error::Unavailable
	);
}

#[test]
fn a_stream_primes_then_keeps_a_looping_voice_fed_across_the_loop() {
	let streamer = Streamer::spawn().unwrap();
	let (feed, shape) = streamer.feed(&fixture("tone.ogg"), true).unwrap();
	assert_eq!(shape.rate, 44_100);
	let Feed::Stream {
		mut frames,
		finished,
	} = feed
	else {
		panic!("not a stream");
	};
	assert!(frames.slots() >= 22_050, "primed {}", frames.slots());

	// Read three seconds, six times the sound's length.
	let mut read = Vec::new();
	let deadline = std::time::Instant::now() + Duration::from_secs(10);

	while read.len() < 44_100 * 2 * 3 {
		assert!(std::time::Instant::now() < deadline, "the stream stalled");

		if let Ok(chunk) = frames.read_chunk(frames.slots()) {
			let (first, second) = chunk.as_slices();
			read.extend_from_slice(first);
			read.extend_from_slice(second);
			chunk.commit_all();
		}

		std::thread::sleep(Duration::from_millis(2));
	}

	assert!(!finished.load(Ordering::Acquire));
	assert!(read.iter().all(|sample| sample.abs() <= 0.56));
	drop(frames);
}

#[test]
fn a_stream_that_does_not_loop_finishes() {
	let streamer = Streamer::spawn().unwrap();
	let (feed, _) = streamer.feed(&fixture("tone.wav"), false).unwrap();
	let Feed::Stream {
		mut frames,
		finished,
	} = feed
	else {
		panic!("not a stream");
	};
	let mut count = 0;
	let deadline = std::time::Instant::now() + Duration::from_secs(10);

	while !(finished.load(Ordering::Acquire) && frames.is_empty()) {
		assert!(
			std::time::Instant::now() < deadline,
			"the stream never finished"
		);

		while frames.pop().is_ok() {
			count += 1;
		}

		std::thread::sleep(Duration::from_millis(2));
	}

	assert_eq!(count, 48_000);
}
