//! Look around by turning: three degrees of freedom from the rotation
//! sensors, for phones without AR, with the camera picture behind the scene
//! where the phone has a [`CameraSource`].

use std::sync::Arc;

use leptoskit_bridge::Error;

use super::Backend;
use super::CameraSource;
use super::CapturedImage;
use super::Started;
use super::Viewport;
use super::XrFrame;
use super::camera::upright_fov;
use super::camera::upright_transform;
use crate::image::CameraImage;
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
///
/// With a [`CameraSource`] ([`OrientationBackend::with_camera`]), sessions
/// that ask for [`Features::CAMERA_IMAGE`] also carry the camera picture,
/// turned upright and cropped to the screen, and render with the camera's
/// field of view so the scene keeps the picture's scale. Without the
/// camera permission such a session starts without the picture, unless it
/// requires it.
pub struct OrientationBackend<S> {
	source: S,
	camera: Option<Box<dyn CameraSource>>,
	filming: bool,
	viewport: Viewport,
	running: bool,
	recentre: Option<Quat>,
	last_time: Option<f64>,
}

impl<S: std::fmt::Debug> std::fmt::Debug for OrientationBackend<S> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("OrientationBackend")
			.field("source", &self.source)
			.field("camera", &self.camera.is_some())
			.field("filming", &self.filming)
			.field("running", &self.running)
			.finish_non_exhaustive()
	}
}

impl<S: OrientationSource> OrientationBackend<S> {
	/// A backend over `source`.
	pub fn new(source: S) -> Self {
		Self {
			source,
			camera: None,
			filming: false,
			viewport: Viewport::default(),
			running: false,
			recentre: None,
			last_time: None,
		}
	}

	/// Also shows `camera`'s picture to sessions that ask for it.
	#[must_use]
	pub fn with_camera(mut self, camera: impl CameraSource + 'static) -> Self {
		self.camera = Some(Box::new(camera));
		self
	}

	fn has_camera(&self) -> bool {
		self.camera
			.as_ref()
			.is_some_and(|camera| camera.available())
	}

	/// Opens or closes the camera for `features`, returning the features
	/// that work.
	fn film(&mut self, config: &SessionConfig, mut features: Features) -> Result<Features, Error> {
		let wanted = features.contains(Features::CAMERA_IMAGE);

		match self.camera.as_mut() {
			Some(camera) if wanted && !self.filming => {
				match camera.start() {
					Ok(()) => self.filming = true,
					Err(error) if config.required.contains(Features::CAMERA_IMAGE) => {
						return Err(error);
					}
					Err(_) => features = features.difference(Features::CAMERA_IMAGE),
				}
			}
			Some(camera) if !wanted && self.filming => {
				camera.stop();
				self.filming = false;
			}
			None if wanted => features = features.difference(Features::CAMERA_IMAGE),
			_ => {}
		}

		Ok(features)
	}

	fn frame(&mut self, time: f64, device: Quat) -> XrFrame {
		let viewer = attitude_to_viewer(device, self.viewport.quarter_turns);
		let recentre = *self
			.recentre
			.get_or_insert_with(|| Quat::from_axis_angle(Vec3::Y, -heading(viewer)));
		let pose = Pose::new(Vec3::ZERO, recentre * viewer);
		let picture = self
			.camera
			.as_mut()
			.filter(|_| self.filming)
			.and_then(|camera| camera.latest());
		let fov = picture.as_ref().map_or_else(
			|| Fov::symmetric(VERTICAL_FOV, self.viewport.aspect()),
			|picture| {
				upright_fov(
					&picture.intrinsics,
					picture.sensor_quarter_turns,
					self.viewport,
				)
			},
		);

		XrFrame {
			time,
			tracking: TrackingState::Normal,
			viewer: pose,
			views: vec![View {
				pose,
				projection: Projection::Fov(fov),
			}],
			camera_intrinsics: picture.as_ref().map(|picture| picture.intrinsics),
			camera_image: picture.map(|picture| self.upright(&picture)),
			..XrFrame::default()
		}
	}

	/// The picture with the transform that turns it upright on the screen.
	fn upright(&self, picture: &Arc<CapturedImage>) -> CameraImage {
		let (display_transform, _) = upright_transform(
			picture.width,
			picture.height,
			picture.sensor_quarter_turns,
			self.viewport,
		);

		CameraImage {
			width: picture.width,
			height: picture.height,
			display_transform,
			pixels: picture.pixels.clone(),
		}
	}
}

impl<S: OrientationSource> Backend for OrientationBackend<S> {
	fn capabilities(&self) -> Capabilities {
		if !self.source.available() {
			return Capabilities::NONE;
		}

		let mut features = Features::HIT_TEST | Features::ANCHORS;

		if self.has_camera() {
			features = features | Features::CAMERA_IMAGE;
		}

		Capabilities::NONE.with(Mode::Orientation, features)
	}

	fn prepare(&mut self, mode: Mode, _install: bool) -> Availability {
		if mode == Mode::Orientation && self.source.available() {
			Availability::Ready
		} else {
			Availability::Unsupported
		}
	}

	fn start(&mut self, config: &SessionConfig, features: Features) -> Result<Started, Error> {
		self.source.start()?;
		let features = match self.film(config, features) {
			Ok(features) => features,
			Err(error) => {
				self.source.stop();
				return Err(error);
			}
		};
		self.running = true;
		self.recentre = None;
		self.last_time = None;
		Ok(Started::new(ReferenceSpace::Local, features))
	}

	fn configure(&mut self, config: &SessionConfig, features: Features) -> Result<Started, Error> {
		let features = self.film(config, features)?;
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

		if self.filming {
			if let Some(camera) = self.camera.as_mut() {
				camera.stop();
			}

			self.filming = false;
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
