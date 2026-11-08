//! The host side: backends that talk to the platform, and the runtime that
//! turns their frames into the guest's bounded frame stream.
//!
//! Tracking and rendering stay here, in native code: the Wasm guest receives
//! poses, planes, and inputs, never the camera or depth pictures.

use std::sync::Arc;

use leptoskit_bridge::Error;

use crate::hit_test;
use crate::image::CameraImage;
use crate::image::DepthImage;
use crate::math::Pose;
use crate::types::*;

mod composite;
mod orientation;
mod replay;
mod runtime;

#[cfg(feature = "async-host")]
mod plugin;

pub use composite::Composite;
pub use orientation::OrientationBackend;
pub use orientation::OrientationSource;
pub use orientation::attitude_to_viewer;
#[cfg(feature = "async-host")]
pub use plugin::Options;
#[cfg(feature = "async-host")]
pub use plugin::XrPlugin;
#[cfg(feature = "async-host")]
pub use plugin::from_context;
pub use replay::RecordedFrame;
pub use replay::Recorder;
pub use replay::Recording;
pub use replay::ReplayBackend;
pub use runtime::Driver;
pub use runtime::NextGuestFrame;
pub use runtime::XrRuntime;

/// The screen a phone backend projects for: its size in pixels and how far
/// the interface is turned from the device's natural orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
	/// Width in pixels, as the interface is shown.
	pub width: u32,
	/// Height in pixels, as the interface is shown.
	pub height: u32,
	/// Clockwise quarter turns of the interface from the device's natural
	/// orientation (Android's `Surface.ROTATION_*`), 0 to 3.
	pub quarter_turns: u32,
}

impl Default for Viewport {
	/// A portrait phone screen, until the host reports the real one.
	fn default() -> Self {
		Self {
			width: 1080,
			height: 1920,
			quarter_turns: 0,
		}
	}
}

impl Viewport {
	/// Width over height.
	#[must_use]
	pub fn aspect(self) -> f32 {
		self.width.max(1) as f32 / self.height.max(1) as f32
	}
}

/// One frame as the host sees it: the guest's [`Frame`] plus the camera and
/// depth pictures the renderer uses.
#[derive(Debug, Clone, Default)]
pub struct XrFrame {
	/// When it was captured, in seconds on the platform's monotonic clock.
	pub time: f64,
	/// How well the device knows where it is.
	pub tracking: TrackingState,
	/// Where the viewer is.
	pub viewer: Pose,
	/// What the renderer draws: one view for a phone, two for a headset.
	pub views: Vec<View>,
	/// The camera picture to draw behind the scene, on phones.
	pub camera_image: Option<CameraImage>,
	/// The depth picture, for occlusion and hit tests.
	pub depth: Option<DepthImage>,
	/// Every plane found so far.
	pub planes: Arc<[Plane]>,
	/// Increases when the planes change; the runtime keeps it.
	pub planes_revision: u64,
	/// The real light, when estimated.
	pub light: Option<LightEstimate>,
	/// Pinned points.
	pub anchors: Vec<Anchor>,
	/// Controllers, hands, touches, and gaze.
	pub inputs: Vec<InputSource>,
}

impl XrFrame {
	/// The guest's view of this frame, with the planes left out when
	/// `include_planes` is false.
	#[must_use]
	pub fn to_guest(&self, sequence: u64, include_planes: bool) -> Frame {
		let mut planes = if include_planes {
			self.planes.to_vec()
		} else {
			Vec::new()
		};
		// Keep the largest planes when there are too many.
		planes.sort_by(|a, b| (b.extent[0] * b.extent[1]).total_cmp(&(a.extent[0] * a.extent[1])));
		planes.truncate(Frame::MAX_PLANES);

		for plane in &mut planes {
			plane.polygon = thin_polygon(&plane.polygon);
		}

		Frame {
			sequence,
			time: self.time,
			tracking: self.tracking,
			viewer: self.viewer,
			views: self.views.iter().take(Frame::MAX_VIEWS).copied().collect(),
			planes,
			planes_revision: self.planes_revision,
			anchors: self
				.anchors
				.iter()
				.take(Frame::MAX_ANCHORS)
				.copied()
				.collect(),
			light: self.light,
			inputs: self
				.inputs
				.iter()
				.take(Frame::MAX_INPUTS)
				.cloned()
				.collect(),
			depth: self.depth.is_some(),
		}
	}
}

/// One way of reaching the real world: ARKit, ARCore, OpenXR, the rotation
/// sensors, or a recording.
///
/// The runtime owns a backend behind a lock and calls it from one thread at a
/// time: the render thread when a renderer drives the session, or the
/// runtime's own [`Driver`] thread otherwise.
pub trait Backend: Send {
	/// What this device can do. Cheap; called often.
	fn capabilities(&self) -> Capabilities;

	/// Whether `mode` can start now. With `install`, opens the store for a
	/// missing AR service.
	fn prepare(&mut self, mode: Mode, install: bool) -> Availability;

	/// Starts a session with `features`, which the runtime already checked
	/// against [`Backend::capabilities`].
	fn start(&mut self, config: &SessionConfig, features: Features) -> Result<(), Error>;

	/// Changes the running session's features or reference space. The
	/// default stops and starts again.
	fn configure(&mut self, config: &SessionConfig, features: Features) -> Result<(), Error> {
		self.stop();
		self.start(config, features)
	}

	/// The newest frame, or `None` when nothing changed since the last call.
	/// `time` is the caller's monotonic clock in seconds; recordings play
	/// against it, live backends report their own capture time.
	fn update(&mut self, time: f64) -> Result<Option<XrFrame>, Error>;

	/// Casts a ray against the real world. The default tests the latest
	/// frame's planes and depth in software.
	fn hit_test(&mut self, request: &HitTestRequest, frame: &XrFrame) -> Vec<Hit> {
		hit_test::hit_test(request, &frame.views, &frame.planes, frame.depth.as_ref())
	}

	/// Pins a point with the platform's own anchors, which follow its map
	/// corrections. `Unavailable` makes the runtime keep a fixed anchor
	/// itself.
	fn add_anchor(&mut self, _pose: Pose, _plane: Option<u64>) -> Result<u64, Error> {
		Err(Error::Unavailable)
	}

	/// Releases an anchor `add_anchor` made.
	fn remove_anchor(&mut self, _id: u64) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Tells a phone backend the size and rotation of the screen it projects
	/// for. Headsets ignore it.
	fn set_viewport(&mut self, _viewport: Viewport) {}

	/// Ends the session and releases the camera and sensors.
	fn stop(&mut self);
}
