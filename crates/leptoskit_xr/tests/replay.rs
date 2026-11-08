//! A recording played through the runtime: determinism, the guest's frame
//! rate, plane revisions, hit tests, anchors, and stopping.

mod support;

use std::task::Poll;

use leptoskit_xr::host::Backend;
use leptoskit_xr::host::Recorder;
use leptoskit_xr::host::Recording;
use leptoskit_xr::host::ReplayBackend;
use leptoskit_xr::host::XrRuntime;
use leptoskit_xr::protocol::NextFrame;
use leptoskit_xr::protocol::PlaneCache;
use leptoskit_xr::*;
use support::*;

fn runtime(recording: Recording, rate: u32) -> std::sync::Arc<XrRuntime> {
	let runtime = XrRuntime::new(ReplayBackend::new(recording));
	runtime
		.start(
			SessionConfig::new(Mode::PhoneAr)
				.prefer(Features::PLANES | Features::DEPTH)
				.frame_rate(rate),
		)
		.unwrap();
	runtime
}

/// Updates the runtime once per recorded frame, from time `start`.
fn play(runtime: &XrRuntime, start: f64, frames: usize) {
	for index in 0..frames {
		runtime.update(start + index as f64 / RATE).unwrap();
	}
}

fn next(runtime: &std::sync::Arc<XrRuntime>, request: NextFrame) -> Poll<Result<Frame, Error>> {
	poll_once(runtime.next_frame(request))
}

#[test]
fn playback_is_the_same_on_every_run() {
	let times: Vec<f64> = (0..200).map(|i| 3.0 + f64::from(i) * 0.007).collect();
	let run = || {
		let mut backend = ReplayBackend::new(walk(60)).looping(true);
		Backend::start(
			&mut backend,
			&SessionConfig::new(Mode::PhoneAr),
			Features::NONE,
		)
		.unwrap();
		times
			.iter()
			.filter_map(|&time| Backend::update(&mut backend, time).unwrap())
			.map(|frame| (frame.time, frame.viewer))
			.collect::<Vec<_>>()
	};

	let first = run();
	assert_eq!(first, run());
	// The recording lasts 59 frames; looping repeats it with a later time.
	assert!(first.windows(2).all(|pair| pair[0].0 < pair[1].0));
	assert!(first.len() > 60);
}

#[test]
fn recordings_survive_the_file_format() {
	let recording = walk(40);
	let bytes = recording.to_bytes();
	let read = Recording::from_bytes(&bytes).unwrap();
	assert_eq!(read.frames.len(), 40);
	assert_eq!(read.frames[35].frame, recording.frames[35].frame);
	assert_eq!(read.capabilities, recording.capabilities);
	assert!(Recording::from_bytes(&bytes[..bytes.len() - 1]).is_err());
	let mut wrong = bytes.clone();

	wrong[0] = b'X';
	assert!(Recording::from_bytes(&wrong).is_err());
}

#[test]
fn a_recorder_captures_what_the_runtime_produced() {
	let runtime = runtime(walk(20), 60);
	let mut recorder = Recorder::new(runtime.capabilities());

	for index in 0..20 {
		if let Some(frame) = runtime.update(f64::from(index) / RATE).unwrap() {
			recorder.record(&frame);
		}
	}

	let recording = recorder.finish();
	assert_eq!(recording.frames.len(), 20);
	assert_eq!(recording.frames[FLOOR_FOUND].frame.planes, vec![floor(2.0)]);
}

#[test]
fn the_guest_sees_frames_at_its_own_rate() {
	let runtime = runtime(walk(61), 20);
	let mut received = Vec::new();
	let mut after = 0;

	for index in 0..61 {
		runtime.update(f64::from(index) / RATE).unwrap();

		if let Poll::Ready(Ok(frame)) = next(
			&runtime,
			NextFrame {
				after,
				planes_revision: 0,
			},
		) {
			after = frame.sequence;
			received.push(frame.time);
		}
	}

	// One second of frames at 20 a second, counting the first.
	assert_eq!(received.len(), 21, "{received:?}");
	assert!(received.windows(2).all(|pair| pair[1] - pair[0] > 0.04));
	// Asking again without a newer frame waits.
	assert!(
		next(
			&runtime,
			NextFrame {
				after,
				planes_revision: 0
			}
		)
		.is_pending()
	);
}

#[test]
fn planes_are_sent_only_when_they_change() {
	let runtime = runtime(walk(40), 60);
	let mut cache = PlaneCache::default();
	let mut after = 0;
	let mut revisions = Vec::new();

	for index in 0..40 {
		runtime.update(f64::from(index) / RATE).unwrap();
		let request = NextFrame {
			after,
			planes_revision: cache.revision(),
		};
		let Poll::Ready(Ok(mut frame)) = next(&runtime, request) else {
			panic!("frame {index} was not published");
		};
		let sent = frame.planes.len();
		after = frame.sequence;
		cache.merge(&mut frame);

		revisions.push((frame.planes_revision, sent, frame.planes.len()));
	}

	// Before the floor: revision 0 and nothing. On discovery and on growth
	// the planes are sent once; in between the cache fills them in.
	assert_eq!(revisions[0], (0, 0, 0));
	assert_eq!(revisions[FLOOR_FOUND], (1, 1, 1));
	assert_eq!(revisions[FLOOR_FOUND + 1], (1, 0, 1));
	assert_eq!(revisions[FLOOR_GROWS], (2, 1, 1));
	assert_eq!(revisions[FLOOR_GROWS + 5], (2, 0, 1));
}

#[test]
fn hit_tests_find_the_floor_under_a_tap() {
	let runtime = runtime(walk(40), 60);
	play(&runtime, 0.0, FLOOR_FOUND);
	assert_eq!(
		runtime.hit_test(&HitTestRequest::screen(0.5, 0.5)).unwrap(),
		[]
	);
	play(&runtime, 1.0, 40);
	let hits = runtime.hit_test(&HitTestRequest::screen(0.5, 0.5)).unwrap();
	assert_eq!(hits.len(), 1);
	assert_eq!(hits[0].plane, Some(1));
	assert!(hits[0].pose.position.y.abs() < 1e-4);

	// The camera is 1.4 m up, looking 45° down.
	assert!((hits[0].distance - 1.4 * std::f32::consts::SQRT_2).abs() < 1e-3);
}

#[test]
fn runtime_anchors_ride_along_in_every_frame() {
	let runtime = runtime(walk(10), 60);
	let pose = Pose::at(Vec3::new(1.0, 0.0, -2.0));
	let id = runtime.add_anchor(pose, None).unwrap();
	let frame = runtime.update(0.0).unwrap().unwrap();
	assert_eq!(frame.anchors.len(), 1);
	assert_eq!(frame.anchors[0].id, id);
	assert_eq!(frame.anchors[0].pose, pose);
	runtime.remove_anchor(id).unwrap();

	assert_eq!(runtime.remove_anchor(id), Err(Error::InvalidArgument));
	let frame = runtime.update(1.0 / RATE).unwrap().unwrap();
	assert_eq!(frame.anchors, []);
}

#[test]
fn stopping_cancels_waiting_frames_and_frees_the_session() {
	let runtime = runtime(walk(10), 60);
	let waiting = runtime.next_frame(NextFrame::default());
	assert_eq!(
		runtime.start(SessionConfig::new(Mode::PhoneAr)),
		Err(Error::Busy)
	);
	runtime.stop();
	assert_eq!(poll_once(waiting), Poll::Ready(Err(Error::Cancelled)));
	assert!(runtime.update(0.0).unwrap().is_none());
	// A request from the old session can't see the new one's frames.
	let stale = runtime.next_frame(NextFrame::default());
	runtime.start(SessionConfig::new(Mode::PhoneAr)).unwrap();

	runtime.update(0.0).unwrap();
	assert_eq!(poll_once(stale), Poll::Ready(Err(Error::Cancelled)));
}

#[test]
fn sessions_need_a_supported_mode_and_their_required_features() {
	let runtime = XrRuntime::new(ReplayBackend::new(walk(5)));
	assert_eq!(
		runtime.start(SessionConfig::new(Mode::ImmersiveVr)),
		Err(Error::Unavailable)
	);
	assert_eq!(
		runtime.start(SessionConfig::new(Mode::PhoneAr).require(Features::DEPTH)),
		Err(Error::Unavailable)
	);
	assert_eq!(
		runtime.start(SessionConfig::new(Mode::PhoneAr).frame_rate(0)),
		Err(Error::InvalidArgument)
	);
	let info = runtime
		.start(SessionConfig::new(Mode::PhoneAr).prefer(Features::DEPTH | Features::PLANES))
		.unwrap();
	assert_eq!(info.features, Features::PLANES);
	assert_eq!(
		runtime.configure(SessionConfig::new(Mode::Orientation)),
		Err(Error::InvalidArgument)
	);
}
