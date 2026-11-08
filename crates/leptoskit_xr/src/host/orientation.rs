//! Look around by turning: three degrees of freedom from the rotation
//! sensors, for phones without AR.

use leptoskit_bridge::Error;

use super::Backend;
use super::Started;
use super::Viewport;
use super::XrFrame;
use crate::math::Fov;
use crate::math::Pose;
use crate::math::Projection;
use crate::math::Quat;
use crate::math::Vec3;
use crate::math::attitude_to_viewer;
use crate::math::heading;
use crate::types::*;

/// The vertical field of view an orientation session renders with: a
/// phone camera's, so a scene looks the same size as in AR.
const VERTICAL_FOV: f32 = 1.05;

/// A device's rotation, as the platform's rotation-vector or attitude sensor
/// reports it.
pub trait OrientationSource: Send {
	/// Whether the device has the sensor.
	fn available(&self) -> bool;
	/// Starts reporting.
	fn start(&mut self) -> Result<(), Error>;
	/// The newest reading: the time in seconds on the sensor's monotonic
	/// clock, and the rotation from the device's frame to a world with +Z up
	/// and an arbitrary heading. The device's frame has +X to the right of
	/// its natural orientation, +Y to its top, and +Z out of the screen, as
	/// Android's rotation vector and Core Motion's attitude both use.
	fn latest(&mut self) -> Option<(f64, Quat)>;
	/// Stops reporting.
	fn stop(&mut self);
}

/// Orientation-only sessions from an [`OrientationSource`].
///
/// The world's −Z is the way the device looked when the session started, so
/// a scene placed in front appears in front whichever way the user faced.
#[derive(Debug)]
pub struct OrientationBackend<S> {
	source: S,
	viewport: Viewport,
	running: bool,
	recentre: Option<Quat>,
	last_time: Option<f64>,
}

impl<S: OrientationSource> OrientationBackend<S> {
	/// A backend over `source`.
	pub fn new(source: S) -> Self {
		Self {
			source,
			viewport: Viewport::default(),
			running: false,
			recentre: None,
			last_time: None,
		}
	}

	fn frame(&mut self, time: f64, device: Quat) -> XrFrame {
		let viewer = attitude_to_viewer(device, self.viewport.quarter_turns);
		let recentre = *self
			.recentre
			.get_or_insert_with(|| Quat::from_axis_angle(Vec3::Y, -heading(viewer)));
		let pose = Pose::new(Vec3::ZERO, recentre * viewer);

		XrFrame {
			time,
			tracking: TrackingState::Normal,
			viewer: pose,
			views: vec![View {
				pose,
				projection: Projection::Fov(Fov::symmetric(VERTICAL_FOV, self.viewport.aspect())),
			}],
			..XrFrame::default()
		}
	}
}

impl<S: OrientationSource> Backend for OrientationBackend<S> {
	fn capabilities(&self) -> Capabilities {
		if self.source.available() {
			Capabilities::NONE.with(Mode::Orientation, Features::HIT_TEST | Features::ANCHORS)
		} else {
			Capabilities::NONE
		}
	}

	fn prepare(&mut self, mode: Mode, _install: bool) -> Availability {
		if mode == Mode::Orientation && self.source.available() {
			Availability::Ready
		} else {
			Availability::Unsupported
		}
	}

	fn start(&mut self, _config: &SessionConfig, features: Features) -> Result<Started, Error> {
		self.source.start()?;
		self.running = true;
		self.recentre = None;
		self.last_time = None;
		Ok(Started::new(ReferenceSpace::Local, features))
	}

	fn configure(&mut self, _config: &SessionConfig, features: Features) -> Result<Started, Error> {
		Ok(Started::new(ReferenceSpace::Local, features))
	}

	fn update(&mut self, _time: f64) -> Result<Option<XrFrame>, Error> {
		if !self.running {
			return Err(Error::Unavailable);
		}

		let Some((time, device)) = self.source.latest() else {
			return Ok(None);
		};

		if self.last_time == Some(time) {
			return Ok(None);
		}

		self.last_time = Some(time);
		Ok(Some(self.frame(time, device)))
	}

	fn set_viewport(&mut self, viewport: Viewport) {
		self.viewport = viewport;
	}

	fn stop(&mut self) {
		if self.running {
			self.source.stop();
			self.running = false;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn close(a: Vec3, b: Vec3) -> bool {
		(a - b).length() < 1e-4
	}

	#[test]
	fn a_phone_held_upright_looks_at_the_horizon() {
		// Upright in portrait: the device's +Y (its top) points up the world's
		// +Z, so the device is turned a quarter about its +X.
		let upright = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), std::f32::consts::FRAC_PI_2);
		let viewer = attitude_to_viewer(upright, 0);
		assert!(close(viewer.rotate(Vec3::Y), Vec3::Y));
		assert!(viewer.rotate(Vec3::FORWARD).y.abs() < 1e-4);
	}

	#[test]
	fn a_phone_lying_face_up_looks_at_the_floor() {
		let viewer = attitude_to_viewer(Quat::IDENTITY, 0);
		assert!(close(viewer.rotate(Vec3::FORWARD), -Vec3::Y));
	}

	#[test]
	fn landscape_keeps_the_interface_upright() {
		// Upright in landscape, the device turned a quarter counter-clockwise
		// so its +X points up, and the interface turned a quarter clockwise
		// to stay upright (Android's `ROTATION_90`).
		let upright = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), std::f32::consts::FRAC_PI_2);
		let landscape =
			upright * Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), std::f32::consts::FRAC_PI_2);
		let viewer = attitude_to_viewer(landscape, 1);
		assert!(close(viewer.rotate(Vec3::Y), Vec3::Y));
	}

	struct Fixed(Vec<(f64, Quat)>);

	impl OrientationSource for Fixed {
		fn available(&self) -> bool {
			true
		}

		fn start(&mut self) -> Result<(), Error> {
			Ok(())
		}

		fn latest(&mut self) -> Option<(f64, Quat)> {
			if self.0.len() > 1 {
				Some(self.0.remove(0))
			} else {
				self.0.first().copied()
			}
		}

		fn stop(&mut self) {}
	}

	#[test]
	fn sessions_start_facing_forward_and_follow_turns() {
		let upright = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), std::f32::consts::FRAC_PI_2);
		let east = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), -1.0) * upright;
		let turned = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), 0.5) * east;
		let mut backend = OrientationBackend::new(Fixed(vec![(1.0, east), (2.0, turned)]));
		backend
			.start(&SessionConfig::new(Mode::Orientation), Features::NONE)
			.unwrap();
		let first = backend.update(0.0).unwrap().unwrap();
		assert!(close(first.viewer.forward(), Vec3::FORWARD));
		let second = backend.update(0.0).unwrap().unwrap();
		let expected = Quat::from_axis_angle(Vec3::Y, 0.5).rotate(Vec3::FORWARD);
		assert!(close(second.viewer.forward(), expected));
		assert!(backend.update(0.0).unwrap().is_none());
	}
}
