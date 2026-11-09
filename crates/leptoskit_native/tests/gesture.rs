//! The gesture arena: recognizers, composition, and pointer lifetime.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use leptoskit_native::Gesture;
use leptoskit_native::GestureState;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeSurface;
use leptoskit_native::PointerDecision;
use leptoskit_native::PointerKind;
use leptoskit_native::PointerPhase;
use leptoskit_native::PointerRoute;
use leptoskit_native::PointerSample;
use leptoskit_native::components::*;

fn mount(root: &leptoskit_native::Element) -> NativeSurface<HeadlessBackend> {
	let mut surface = NativeSurface::new(HeadlessBackend::default());
	surface.set_viewport(200.0, 200.0);
	surface.set_root(root);
	surface
}

fn sample(phase: PointerPhase, x: f32, y: f32, time_ms: u64) -> PointerSample {
	PointerSample::new(1, phase, x, y, time_ms)
}

fn down(x: f32, y: f32) -> PointerSample {
	sample(PointerPhase::Down, x, y, 0)
}

struct Log {
	events: Arc<Mutex<Vec<GestureState>>>,
	xs: Arc<Mutex<Vec<f32>>>,
	scales: Arc<Mutex<f32>>,
	rotations: Arc<Mutex<f32>>,
	speed: Arc<Mutex<f32>>,
}

impl Log {
	fn new() -> Self {
		Self {
			events: Arc::new(Mutex::new(Vec::new())),
			xs: Arc::new(Mutex::new(Vec::new())),
			scales: Arc::new(Mutex::new(1.0)),
			rotations: Arc::new(Mutex::new(0.0)),
			speed: Arc::new(Mutex::new(0.0)),
		}
	}

	fn states(&self) -> Vec<GestureState> {
		std::mem::take(&mut self.events.lock().expect("log"))
	}

	fn positions(&self) -> Vec<f32> {
		std::mem::take(&mut self.xs.lock().expect("log"))
	}

	fn scale(&self) -> f32 {
		*self.scales.lock().expect("log")
	}

	fn rotation(&self) -> f32 {
		*self.rotations.lock().expect("log")
	}

	fn speed(&self) -> f32 {
		*self.speed.lock().expect("log")
	}

	fn record(&self) -> impl Fn(leptoskit_native::GestureEvent) + Send + Sync + 'static {
		let events = Arc::clone(&self.events);
		let xs = Arc::clone(&self.xs);
		let scales = Arc::clone(&self.scales);
		let rotations = Arc::clone(&self.rotations);
		let speed = Arc::clone(&self.speed);
		move |event| {
			events.lock().expect("log").push(event.state);
			xs.lock().expect("log").push(event.translation_x);
			*scales.lock().expect("log") = event.scale;
			*rotations.lock().expect("log") = event.rotation;
			*speed.lock().expect("log") = event.velocity_x.hypot(event.velocity_y);
		}
	}
}

#[test]
fn tap_completes_inside_the_slop_and_a_drag_fails_it() {
	let log = Log::new();
	let record = log.record();
	let surface = mount(
		&View()
			.class("w-40 h-40")
			.gesture(Gesture::tap().on_begin(record).on_end(log.record())),
	);
	assert_eq!(surface.pointer(down(10.0, 10.0)).route, PointerRoute::Track);
	assert_eq!(
		surface
			.pointer(sample(PointerPhase::Up, 12.0, 11.0, 40))
			.route,
		PointerRoute::Platform
	);
	assert_eq!(log.states(), vec![GestureState::Began, GestureState::End]);

	let log = Log::new();
	let surface = mount(
		&View()
			.class("w-40 h-40")
			.gesture(Gesture::tap().on_end(log.record())),
	);
	surface.pointer(down(10.0, 10.0));
	surface.pointer(sample(PointerPhase::Move, 40.0, 10.0, 20));
	surface.pointer(sample(PointerPhase::Up, 40.0, 10.0, 30));
	assert_eq!(log.states().len(), 0);
}

#[test]
fn tap_fails_when_its_max_duration_passes() {
	let log = Log::new();
	let surface = mount(
		&View().class("w-40 h-40").gesture(
			Gesture::tap()
				.max_duration(Duration::from_millis(100))
				.on_end(log.record()),
		),
	);
	let decision = surface.pointer(down(10.0, 10.0));
	assert_eq!(decision.deadline_ms, Some(100));
	assert_eq!(surface.advance_gestures(150).route, PointerRoute::Platform);
	surface.pointer(sample(PointerPhase::Up, 10.0, 10.0, 160));
	assert_eq!(log.states().len(), 0);
}

#[test]
fn long_press_wins_on_the_clock_and_loses_when_the_finger_drifts() {
	let log = Log::new();
	let surface = mount(
		&View().class("w-40 h-40").gesture(
			Gesture::long_press()
				.min_duration(Duration::from_millis(200))
				.on_begin(log.record())
				.on_update(log.record())
				.on_end(log.record()),
		),
	);
	assert_eq!(surface.pointer(down(8.0, 8.0)).deadline_ms, Some(200));
	assert_eq!(surface.advance_gestures(100).route, PointerRoute::Track);
	assert_eq!(surface.advance_gestures(200).route, PointerRoute::Claim);
	surface.pointer(sample(PointerPhase::Move, 12.0, 8.0, 240));
	surface.pointer(sample(PointerPhase::Up, 12.0, 8.0, 260));
	assert_eq!(
		log.states(),
		vec![GestureState::Began, GestureState::Active, GestureState::End]
	);

	let log = Log::new();
	let surface = mount(
		&View()
			.class("w-40 h-40")
			.gesture(Gesture::long_press().on_begin(log.record())),
	);
	surface.pointer(down(8.0, 8.0));
	surface.pointer(sample(PointerPhase::Move, 40.0, 8.0, 100));
	assert_eq!(surface.advance_gestures(600).route, PointerRoute::Platform);
	assert_eq!(log.states().len(), 0);
}

#[test]
fn pan_reports_translation_and_velocity_and_claims_the_pointer() {
	let log = Log::new();
	let surface = mount(
		&View().class("w-40 h-40").gesture(
			Gesture::pan()
				.on_begin(log.record())
				.on_update(log.record())
				.on_end(log.record()),
		),
	);
	assert_eq!(surface.pointer(down(10.0, 10.0)).route, PointerRoute::Track);
	let claimed = surface.pointer(sample(PointerPhase::Move, 30.0, 16.0, 16));
	assert_eq!(claimed.route, PointerRoute::Claim);
	surface.pointer(sample(PointerPhase::Move, 50.0, 16.0, 32));
	surface.pointer(sample(PointerPhase::Up, 50.0, 16.0, 48));
	let states = log.states();
	assert_eq!(
		states,
		vec![
			GestureState::Began,
			GestureState::Active,
			GestureState::Active,
			GestureState::End
		]
	);
	assert!(log.speed() > 0.0);
	assert!(log.positions().into_iter().sum::<f32>() > 0.0);
}

#[test]
fn axis_locked_pan_rejects_the_other_direction() {
	let horizontal = Log::new();
	let vertical = Log::new();
	let surface = mount(
		&View().class("w-full h-full").child(
			View().class("w-40 h-40").gesture(
				Gesture::pan()
					.horizontal()
					.on_begin(horizontal.record())
					.exclusive(Gesture::pan().vertical().on_begin(vertical.record())),
			),
		),
	);
	surface.pointer(down(10.0, 10.0));
	surface.pointer(sample(PointerPhase::Move, 10.0, 40.0, 30));
	assert_eq!(horizontal.states().len(), 0);
	assert_eq!(vertical.states(), vec![GestureState::Began]);
}

#[test]
fn exclusive_pan_beats_a_tap_and_a_still_tap_waits_for_the_pan_to_fail() {
	let pans = Log::new();
	let taps = Log::new();
	let surface = mount(
		&View().class("w-40 h-40").gesture(
			Gesture::pan()
				.on_begin(pans.record())
				.exclusive(Gesture::tap().on_end(taps.record())),
		),
	);
	surface.pointer(down(10.0, 10.0));
	surface.pointer(sample(PointerPhase::Move, 40.0, 10.0, 20));
	surface.pointer(sample(PointerPhase::Up, 40.0, 10.0, 30));
	assert_eq!(pans.states(), vec![GestureState::Began]);
	assert_eq!(taps.states().len(), 0);

	let pans = Log::new();
	let taps = Log::new();
	let surface = mount(
		&View().class("w-40 h-40").gesture(
			Gesture::pan()
				.on_begin(pans.record())
				.exclusive(Gesture::tap().on_end(taps.record())),
		),
	);
	surface.pointer(down(10.0, 10.0));
	surface.pointer(sample(PointerPhase::Up, 11.0, 10.0, 30));
	assert_eq!(pans.states().len(), 0);
	assert_eq!(taps.states(), vec![GestureState::End]);
}

#[test]
fn race_gives_a_long_press_priority_over_a_tap_when_it_wins_first() {
	let presses = Log::new();
	let taps = Log::new();
	let surface = mount(
		&View().class("w-40 h-40").gesture(
			Gesture::long_press()
				.min_duration(Duration::from_millis(50))
				.on_begin(presses.record())
				.on_end(presses.record())
				.race(Gesture::tap().on_end(taps.record())),
		),
	);
	surface.pointer(down(10.0, 10.0));
	assert_eq!(surface.advance_gestures(50).route, PointerRoute::Claim);
	surface.pointer(sample(PointerPhase::Up, 10.0, 10.0, 80));
	assert_eq!(
		presses.states(),
		vec![GestureState::Began, GestureState::End]
	);
	assert_eq!(taps.states().len(), 0);
}

#[test]
fn pinch_and_rotation_run_together() {
	let pinches = Log::new();
	let turns = Log::new();
	let surface = mount(
		&View().class("w-40 h-40").gesture(
			Gesture::pinch()
				.on_begin(pinches.record())
				.on_update(pinches.record())
				.simultaneous(
					Gesture::rotation()
						.on_begin(turns.record())
						.on_update(turns.record()),
				),
		),
	);
	surface.pointer(PointerSample::new(1, PointerPhase::Down, 20.0, 20.0, 0));
	let second = PointerSample::new(2, PointerPhase::Down, 40.0, 20.0, 10);
	assert_eq!(surface.pointer(second).route, PointerRoute::Claim);
	surface.pointer(PointerSample::new(2, PointerPhase::Move, 60.0, 20.0, 20));
	assert!(pinches.scale() > 1.5);
	surface.pointer(PointerSample::new(2, PointerPhase::Move, 20.0, 60.0, 30));
	assert!(turns.rotation().abs() > 1.0);
	assert!(pinches.states().contains(&GestureState::Began));
	assert!(turns.states().contains(&GestureState::Began));
	surface.pointer(PointerSample::new(2, PointerPhase::Up, 40.0, 40.0, 40));
	surface.pointer(PointerSample::new(1, PointerPhase::Up, 20.0, 20.0, 50));
	assert_eq!(surface.pointer(down(10.0, 10.0)).route, PointerRoute::Track);
}

#[test]
fn fling_wins_only_above_its_speed() {
	let fast = Log::new();
	let surface = mount(
		&View()
			.class("w-40 h-40")
			.gesture(Gesture::fling().min_velocity(100.0).on_end(fast.record())),
	);
	surface.pointer(down(10.0, 10.0));
	surface.pointer(sample(PointerPhase::Move, 30.0, 10.0, 10));
	surface.pointer(sample(PointerPhase::Up, 50.0, 10.0, 20));
	assert_eq!(fast.states(), vec![GestureState::End]);
	assert!(fast.speed() > 100.0);

	let slow = Log::new();
	let surface = mount(
		&View()
			.class("w-40 h-40")
			.gesture(Gesture::fling().on_end(slow.record())),
	);
	surface.pointer(down(10.0, 10.0));
	surface.pointer(sample(PointerPhase::Move, 12.0, 10.0, 400));
	surface.pointer(sample(PointerPhase::Up, 14.0, 10.0, 800));
	assert_eq!(slow.states().len(), 0);
}

#[test]
fn the_deepest_control_wins_a_drag_unless_the_pan_blocks_it() {
	let parent = Log::new();
	let surface = mount(
		&View()
			.class("w-full h-full")
			.gesture(Gesture::pan().on_begin(parent.record()))
			.child(
				ScrollView()
					.class("w-32 h-32")
					.child(View().class("w-20 h-20")),
			),
	);
	surface.pointer(down(10.0, 10.0));
	let routed = surface.pointer(sample(PointerPhase::Move, 40.0, 10.0, 20));
	assert_eq!(routed.route, PointerRoute::Platform);
	assert_eq!(parent.states().len(), 0);

	let parent = Log::new();
	let surface = mount(
		&View()
			.class("w-full h-full")
			.gesture(Gesture::pan().blocks_native().on_begin(parent.record()))
			.child(
				ScrollView()
					.class("w-32 h-32")
					.child(View().class("w-20 h-20")),
			),
	);
	surface.pointer(down(10.0, 10.0));
	let routed = surface.pointer(sample(PointerPhase::Move, 40.0, 10.0, 20));
	assert_eq!(routed.route, PointerRoute::Claim);
	assert_eq!(parent.states(), vec![GestureState::Began]);
}

#[test]
fn a_button_keeps_a_tap_and_loses_a_drag_to_its_parent() {
	let taps = Log::new();
	let surface = mount(
		&View()
			.class("w-full h-full")
			.gesture(Gesture::tap().on_end(taps.record()))
			.child(Button("Go").class("w-24 h-10")),
	);
	let decision = surface.pointer(down(8.0, 8.0));
	assert_eq!(decision.route, PointerRoute::Track);
	assert_eq!(
		surface
			.pointer(sample(PointerPhase::Up, 8.0, 8.0, 30))
			.route,
		PointerRoute::Platform
	);
	assert_eq!(taps.states().len(), 0);

	let pans = Log::new();
	let surface = mount(
		&View()
			.class("w-full h-full")
			.gesture(Gesture::pan().on_begin(pans.record()))
			.child(Button("Go").class("w-24 h-10")),
	);
	surface.pointer(down(8.0, 8.0));
	assert_eq!(
		surface
			.pointer(sample(PointerPhase::Move, 40.0, 8.0, 20))
			.route,
		PointerRoute::Claim
	);
	assert_eq!(pans.states(), vec![GestureState::Began]);
}

#[test]
fn a_tap_on_a_button_still_leaves_the_pointer_with_the_platform() {
	let taps = Log::new();
	let surface = mount(
		&Button("Go")
			.class("w-24 h-10")
			.gesture(Gesture::tap().on_end(taps.record())),
	);
	surface.pointer(down(4.0, 4.0));
	assert_eq!(
		surface
			.pointer(sample(PointerPhase::Up, 4.0, 4.0, 20))
			.route,
		PointerRoute::Platform
	);
	assert_eq!(taps.states(), vec![GestureState::End]);
}

#[test]
fn cancel_and_removal_end_an_active_gesture_so_the_next_one_can_start() {
	let log = Log::new();
	let surface = mount(
		&View().class("w-40 h-40").gesture(
			Gesture::pan()
				.on_begin(log.record())
				.on_cancel(log.record()),
		),
	);
	surface.pointer(down(10.0, 10.0));
	surface.pointer(sample(PointerPhase::Move, 40.0, 10.0, 20));
	surface.pointer(sample(PointerPhase::Cancel, 40.0, 10.0, 30));
	assert_eq!(
		log.states(),
		vec![GestureState::Began, GestureState::Cancelled]
	);
	assert_eq!(surface.pointer(down(10.0, 10.0)).route, PointerRoute::Track);

	let log = Log::new();
	let mut surface = mount(
		&View().class("w-40 h-40").gesture(
			Gesture::pan()
				.on_begin(log.record())
				.on_cancel(log.record()),
		),
	);
	surface.pointer(down(10.0, 10.0));
	surface.pointer(sample(PointerPhase::Move, 40.0, 10.0, 20));
	surface.set_root(&View().class("w-40 h-40"));
	assert_eq!(
		log.states(),
		vec![GestureState::Began, GestureState::Cancelled]
	);
	assert_eq!(
		surface
			.pointer(
				PointerSample::new(7, PointerPhase::Down, 10.0, 10.0, 80).kind(PointerKind::Mouse)
			)
			.route,
		PointerRoute::Platform
	);
}

#[test]
fn a_repeated_pointer_id_cancels_the_gesture_it_replaces() {
	let log = Log::new();
	let surface = mount(
		&View().class("w-40 h-40").gesture(
			Gesture::pan()
				.on_begin(log.record())
				.on_cancel(log.record())
				.on_end(log.record()),
		),
	);
	surface.pointer(down(10.0, 10.0));
	surface.pointer(sample(PointerPhase::Move, 40.0, 10.0, 20));
	surface.pointer(PointerSample::new(1, PointerPhase::Down, 12.0, 12.0, 30));
	surface.pointer(sample(PointerPhase::Up, 12.0, 12.0, 40));
	assert_eq!(
		log.states(),
		vec![GestureState::Began, GestureState::Cancelled]
	);
}

#[test]
fn samples_outside_the_tree_or_with_bad_numbers_are_ignored() {
	let log = Log::new();
	let surface = mount(
		&View()
			.class("w-20 h-20")
			.gesture(Gesture::tap().on_end(log.record())),
	);
	assert_eq!(
		surface.pointer(down(180.0, 180.0)).route,
		PointerRoute::Platform
	);
	assert_eq!(
		surface
			.pointer(sample(PointerPhase::Move, 10.0, 10.0, 10))
			.route,
		PointerRoute::Platform
	);
	assert_eq!(
		surface
			.pointer(PointerSample::new(1, PointerPhase::Down, f32::NAN, 10.0, 0))
			.route,
		PointerRoute::Platform
	);
	surface.pointer(down(4.0, 4.0));
	assert_eq!(
		surface
			.pointer(PointerSample::new(
				1,
				PointerPhase::Move,
				f32::INFINITY,
				4.0,
				10
			))
			.route,
		PointerRoute::Track
	);
	assert_eq!(log.states().len(), 0);
	let hidden = Log::new();
	let surface = mount(
		&View()
			.class("w-40 h-40")
			.with_props(|props| props.hidden = Some(true))
			.gesture(Gesture::tap().on_end(hidden.record())),
	);
	assert_eq!(
		surface.pointer(down(4.0, 4.0)).route,
		PointerRoute::Platform
	);
}

#[test]
fn disabled_ancestors_and_a_full_pointer_table_do_not_stick() {
	let log = Log::new();
	let surface = mount(
		&View().class("w-full h-full").enabled(false).child(
			View()
				.class("w-20 h-20")
				.gesture(Gesture::tap().on_end(log.record())),
		),
	);
	assert_eq!(
		surface.pointer(down(4.0, 4.0)).route,
		PointerRoute::Platform
	);

	let log = Log::new();
	let surface = mount(
		&View()
			.class("w-full h-full")
			.gesture(Gesture::tap().on_end(log.record())),
	);
	for id in 0..16 {
		let decision = surface.pointer(PointerSample::new(id, PointerPhase::Down, 4.0, 4.0, id));
		assert_eq!(decision.route, PointerRoute::Track);
	}
	assert_eq!(
		surface
			.pointer(PointerSample::new(16, PointerPhase::Down, 4.0, 4.0, 16))
			.route,
		PointerRoute::Track
	);
	for id in 0..16 {
		surface.pointer(PointerSample::new(id, PointerPhase::Up, 4.0, 4.0, 100 + id));
	}
	assert_eq!(log.states().len(), 16);
}

#[test]
fn slider_native_drag_beats_a_parent_pan_and_an_explicit_native_is_click_like() {
	let pans = Log::new();
	let surface = mount(
		&View()
			.class("w-full h-full")
			.gesture(Gesture::pan().on_begin(pans.record()))
			.child(
				Slider(SliderState::new(0.0, 10.0, 0.0, None).expect("slider")).class("w-32 h-8"),
			),
	);
	surface.pointer(down(8.0, 4.0));
	assert_eq!(
		surface
			.pointer(sample(PointerPhase::Move, 40.0, 4.0, 20))
			.route,
		PointerRoute::Platform
	);
	assert_eq!(pans.states().len(), 0);

	let taps = Log::new();
	let surface = mount(
		&View()
			.class("w-40 h-40")
			.gesture(Gesture::native().exclusive(Gesture::tap().on_end(taps.record()))),
	);
	surface.pointer(down(8.0, 8.0));
	surface.pointer(sample(PointerPhase::Up, 8.0, 8.0, 20));
	assert_eq!(taps.states().len(), 0);
}

#[test]
fn mismatched_builder_settings_leave_the_recognizer_usable() {
	let log = Log::new();
	let gesture = Gesture::pan()
		.max_duration(Duration::from_millis(10))
		.min_duration(Duration::from_millis(10))
		.min_velocity(10.0)
		.horizontal()
		.on_begin(log.record());
	let surface = mount(&View().class("w-40 h-40").gesture(gesture));
	surface.pointer(down(5.0, 5.0));
	surface.pointer(sample(PointerPhase::Move, 40.0, 5.0, 20));
	assert_eq!(log.states(), vec![GestureState::Began]);
	let _ = PointerDecision::platform();
	let _ = Gesture::pinch()
		.on_cancel(|_| {})
		.blocks_native()
		.max_distance(f32::NAN);
	let _ = format!(
		"{:?}",
		Gesture::rotation().simultaneous(Gesture::fling().race(Gesture::native()))
	);
}

#[test]
fn a_second_gesture_call_runs_alongside_the_first() {
	let taps = Log::new();
	let pans = Log::new();
	let surface = mount(
		&View()
			.class("w-40 h-40")
			.gesture(Gesture::tap().on_end(taps.record()))
			.gesture(Gesture::pan().on_begin(pans.record())),
	);
	surface.pointer(down(6.0, 6.0));
	surface.pointer(sample(PointerPhase::Move, 40.0, 6.0, 20));
	surface.pointer(sample(PointerPhase::Up, 40.0, 6.0, 30));
	assert_eq!(pans.states(), vec![GestureState::Began]);
	assert_eq!(taps.states().len(), 0);
}

#[test]
fn a_pen_sample_is_tracked_and_a_zero_dt_move_has_no_velocity() {
	let log = Log::new();
	let surface = mount(
		&View()
			.class("w-40 h-40")
			.gesture(Gesture::pan().on_end(log.record())),
	);
	surface
		.pointer(PointerSample::new(3, PointerPhase::Down, 10.0, 10.0, 5).kind(PointerKind::Pen));
	surface.pointer(PointerSample::new(3, PointerPhase::Move, 40.0, 10.0, 5));
	surface.pointer(PointerSample::new(3, PointerPhase::Up, 40.0, 10.0, 5));
	assert_eq!(log.speed(), 0.0);
	assert_eq!(log.states(), vec![GestureState::End]);
}
