//! A recorded walk through a room, shared by the integration tests.

use std::future::Future;
use std::pin::pin;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;

use leptoskit_xr::host::RecordedFrame;
use leptoskit_xr::host::Recording;
use leptoskit_xr::*;

/// Frames per second in the recording.
pub const RATE: f64 = 60.0;
/// The frame at which the floor is found.
pub const FLOOR_FOUND: usize = 10;
/// The frame at which the floor grows.
pub const FLOOR_GROWS: usize = 30;

/// The floor, as the recording finds it, `size` metres square.
pub fn floor(size: f32) -> Plane {
	Plane {
		id: 1,
		orientation: PlaneOrientation::HorizontalUp,
		label: PlaneLabel::Floor,
		pose: Pose::at(Vec3::new(0.0, 0.0, -1.5)),
		extent: [size, size],
		polygon: Vec::new(),
	}
}

/// A phone walking forward at 1.4 m eye height, looking 45° down, for
/// `count` frames at [`RATE`].
pub fn walk(count: usize) -> Recording {
	let down = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -std::f32::consts::FRAC_PI_4);
	let frames = (0..count)
		.map(|index| {
			let viewer = Pose::new(Vec3::new(0.0, 1.4, -0.01 * index as f32), down);
			let planes = match index {
				i if i >= FLOOR_GROWS => vec![floor(4.0)],
				i if i >= FLOOR_FOUND => vec![floor(2.0)],
				_ => Vec::new(),
			};

			RecordedFrame {
				frame: Frame {
					sequence: 0,
					time: 100.0 + index as f64 / RATE,
					tracking: TrackingState::Normal,
					viewer,
					views: vec![View {
						pose: viewer,
						projection: Projection::Fov(Fov::symmetric(1.05, 0.5)),
					}],
					planes,
					planes_revision: 0,
					anchors: Vec::new(),
					light: Some(LightEstimate {
						intensity: 1.0,
						color_temperature: 6500.0,
						main_light_direction: None,
					}),
					inputs: Vec::new(),
					depth: None,
				},
				depth: None,
			}
		})
		.collect();

	Recording {
		capabilities: Capabilities::NONE.with(
			Mode::PhoneAr,
			Features::PLANES | Features::HIT_TEST | Features::ANCHORS | Features::LIGHT_ESTIMATION,
		),
		frames,
	}
}

/// Polls a future once with a waker that does nothing.
#[allow(
	dead_code,
	reason = "each test binary uses a different part of the support module"
)]
pub fn poll_once<F: Future>(future: F) -> Poll<F::Output> {
	let mut context = Context::from_waker(Waker::noop());
	pin!(future).poll(&mut context)
}
