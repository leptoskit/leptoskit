//! The camera picture on the host: orientation sessions with a camera, and
//! depth estimated from camera frames for sessions whose platform has none.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_xr::host::*;
use leptoskit_xr::image::*;
use leptoskit_xr::*;

/// A 4 × 4 grey picture with neutral chroma.
fn grey() -> CameraPixels {
	CameraPixels::YCbCr {
		y: ImagePlane {
			width: 4,
			height: 4,
			row_stride: 4,
			data: vec![128; 16].into(),
		},
		cbcr: ImagePlane {
			width: 2,
			height: 2,
			row_stride: 4,
			data: vec![128; 8].into(),
		},
	}
}

fn intrinsics() -> Intrinsics {
	Intrinsics {
		focal: [4.0, 4.0],
		principal: [2.0, 2.0],
		size: [4, 4],
	}
}

/// A rotation sensor that always reads the same, a little later each time.
struct Still(f64);

impl OrientationSource for Still {
	fn available(&self) -> bool {
		true
	}

	fn start(&mut self) -> Result<(), Error> {
		Ok(())
	}

	fn latest(&mut self) -> Option<(f64, Quat)> {
		self.0 += 1.0 / 60.0;
		Some((
			self.0,
			Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), std::f32::consts::FRAC_PI_2),
		))
	}

	fn stop(&mut self) {}
}

/// A camera that delivers one picture, and is refused until `allowed`.
struct Lens {
	allowed: bool,
	open: Arc<AtomicUsize>,
}

impl CameraSource for Lens {
	fn available(&self) -> bool {
		true
	}

	fn start(&mut self) -> Result<(), Error> {
		if !self.allowed {
			return Err(Error::PermissionDenied);
		}

		self.open.fetch_add(1, Ordering::SeqCst);
		Ok(())
	}

	fn latest(&mut self) -> Option<Arc<CapturedImage>> {
		Some(Arc::new(CapturedImage {
			time: 1.0,
			width: 4,
			height: 4,
			pixels: grey(),
			sensor_quarter_turns: 1,
			intrinsics: intrinsics(),
		}))
	}

	fn stop(&mut self) {
		self.open.fetch_sub(1, Ordering::SeqCst);
	}
}

fn orientation(allowed: bool) -> (OrientationBackend<Still>, Arc<AtomicUsize>) {
	let open = Arc::new(AtomicUsize::new(0));
	let backend = OrientationBackend::new(Still(0.0)).with_camera(Lens {
		allowed,
		open: Arc::clone(&open),
	});
	(backend, open)
}

#[test]
fn orientation_sessions_show_the_camera_when_they_ask() {
	let (backend, open) = orientation(true);
	let runtime = XrRuntime::new(backend);
	let offered = runtime.capabilities().features(Mode::Orientation).unwrap();
	assert!(offered.contains(Features::CAMERA_IMAGE));
	runtime.set_viewport(Viewport {
		width: 400,
		height: 400,
		quarter_turns: 0,
	});

	let info = runtime
		.start(SessionConfig::new(Mode::Orientation).prefer(Features::CAMERA_IMAGE))
		.unwrap();
	assert!(info.features.contains(Features::CAMERA_IMAGE));
	assert_eq!(open.load(Ordering::SeqCst), 1);

	let frame = runtime.update(0.0).unwrap().unwrap();
	let image = frame.camera_image.as_ref().expect("the camera picture");
	// The sensor is turned a quarter, so the view's top-left is its
	// bottom-left.
	assert_eq!(apply_affine(&image.display_transform, 0.0, 0.0), (0.0, 1.0));
	assert_eq!(frame.camera_intrinsics, Some(intrinsics()));
	// A square view of a square picture shows its whole field: 2·atan(2/4).
	let Projection::Fov(fov) = frame.views[0].projection else {
		panic!("orientation sessions project with angles");
	};
	assert!((fov.up - 0.5_f32.atan()).abs() < 1e-5);

	runtime.stop();
	assert_eq!(open.load(Ordering::SeqCst), 0);
}

#[test]
fn without_the_permission_the_picture_is_left_out_unless_required() {
	let (backend, open) = orientation(false);
	let runtime = XrRuntime::new(backend);
	let info = runtime
		.start(SessionConfig::new(Mode::Orientation).prefer(Features::CAMERA_IMAGE))
		.unwrap();
	assert!(!info.features.contains(Features::CAMERA_IMAGE));
	assert!(runtime.update(0.0).unwrap().unwrap().camera_image.is_none());
	runtime.stop();

	let refused =
		runtime.start(SessionConfig::new(Mode::Orientation).require(Features::CAMERA_IMAGE));
	assert_eq!(refused, Err(Error::PermissionDenied));
	assert_eq!(open.load(Ordering::SeqCst), 0);
	assert!(runtime.session().is_none());
}

#[test]
fn sessions_that_do_not_ask_leave_the_camera_closed() {
	let (backend, open) = orientation(true);
	let runtime = XrRuntime::new(backend);
	runtime
		.start(SessionConfig::new(Mode::Orientation))
		.unwrap();
	assert_eq!(open.load(Ordering::SeqCst), 0);
	assert!(runtime.update(0.0).unwrap().unwrap().camera_image.is_none());
}

/// A flat wall `meters` away, along the camera's forward axis, over a
/// picture shown through `display_transform`.
fn wall(meters: f32, display_transform: Affine2) -> DepthImage {
	let data: Vec<u8> = std::iter::repeat_n(meters.to_le_bytes(), 4)
		.flatten()
		.collect();
	DepthImage {
		width: 2,
		height: 2,
		format: DepthFormat::Meters32,
		data: data.into(),
		row_stride: 8,
		confidence: None,
		display_transform,
	}
}

/// An estimator that reads the first frame it is offered after a reset as a
/// flat wall two metres away, and remembers what it saw.
#[derive(Default)]
struct Wall {
	seen: Arc<Mutex<Vec<(u32, u32, f64)>>>,
	resets: Arc<AtomicUsize>,
	latest: Option<DepthEstimate>,
}

impl DepthEstimator for Wall {
	fn max_side(&self) -> u32 {
		2
	}

	fn ready(&self) -> bool {
		self.latest.is_none()
	}

	fn submit(&mut self, frame: Arc<CameraFrame>) {
		self.seen
			.lock()
			.unwrap()
			.push((frame.rgb.width, frame.rgb.height, frame.time));
		self.latest = Some(DepthEstimate {
			depth: wall(2.0, frame.display_transform),
			view: frame.view,
			time: frame.time,
		});
	}

	fn latest(&mut self) -> Option<DepthEstimate> {
		self.latest.clone()
	}

	fn reset(&mut self) {
		self.resets.fetch_add(1, Ordering::SeqCst);
		self.latest = None;
	}
}

#[test]
fn an_estimator_fills_in_depth_from_the_camera() {
	let (backend, _) = orientation(true);
	let runtime = XrRuntime::new(backend);
	assert!(
		!runtime
			.capabilities()
			.features(Mode::Orientation)
			.unwrap()
			.contains(Features::DEPTH)
	);

	let wall = Wall::default();
	let seen = Arc::clone(&wall.seen);
	runtime.set_depth_estimator(Some(Box::new(wall)));
	assert!(
		runtime
			.capabilities()
			.features(Mode::Orientation)
			.unwrap()
			.contains(Features::DEPTH)
	);

	// Depth alone is asked for; the runtime opens the camera for it but
	// doesn't report the picture as a feature.
	let info = runtime
		.start(SessionConfig::new(Mode::Orientation).require(Features::DEPTH))
		.unwrap();
	assert!(info.features.contains(Features::DEPTH));
	assert!(!info.features.contains(Features::CAMERA_IMAGE));

	let frame = runtime.update(0.0).unwrap().unwrap();
	assert_eq!(seen.lock().unwrap().as_slice(), &[(2, 2, frame.time)]);
	let depth = frame.depth.as_ref().expect("estimated depth");
	assert_eq!(depth.sample_view(0.5, 0.5), Some(2.0));
	// Read from this very frame, so seen from its own view.
	assert_eq!(frame.depth_view, Some(frame.views[0]));
	assert_eq!(frame.depth_transform(), Some(depth.display_transform));
	assert_eq!(frame.to_guest(1, true).depth, Some(DepthSource::Estimated));
}

/// A phone with AR tracking but no depth of its own, like ARCore without
/// the Depth API: the viewer stands 1.5 m up and backs away along +Z,
/// `step` metres each frame, looking straight ahead. With `measures`, it
/// measures a wall one metre ahead itself.
struct Retreat {
	step: f32,
	frames: u32,
	measures: bool,
	features: Features,
}

impl Retreat {
	fn new(step: f32, measures: bool) -> Self {
		Self {
			step,
			frames: 0,
			measures,
			features: Features::NONE,
		}
	}
}

impl Backend for Retreat {
	fn capabilities(&self) -> Capabilities {
		let depth = if self.measures {
			Features::DEPTH
		} else {
			Features::NONE
		};
		Capabilities::NONE.with(
			Mode::PhoneAr,
			Features::CAMERA_IMAGE | Features::HIT_TEST | depth,
		)
	}

	fn prepare(&mut self, _mode: Mode, _install: bool) -> Availability {
		Availability::Ready
	}

	fn start(&mut self, config: &SessionConfig, features: Features) -> Result<Started, Error> {
		self.features = features;
		self.frames = 0;
		Ok(Started::new(config.reference_space, features))
	}

	fn update(&mut self, _time: f64) -> Result<Option<XrFrame>, Error> {
		let pose = Pose::at(Vec3::new(0.0, 1.5, self.step * self.frames as f32));
		self.frames += 1;
		let camera = self.features.contains(Features::CAMERA_IMAGE);
		let measured = self.measures && self.features.contains(Features::DEPTH);

		Ok(Some(XrFrame {
			time: f64::from(self.frames) / 30.0,
			tracking: TrackingState::Normal,
			viewer: pose,
			views: vec![View {
				pose,
				projection: Projection::Fov(Fov::symmetric(1.0, 1.0)),
			}],
			camera_image: camera.then(|| {
				CameraImage {
					width: 4,
					height: 4,
					display_transform: IDENTITY_AFFINE,
					pixels: grey(),
				}
			}),
			camera_intrinsics: camera.then(intrinsics),
			depth: measured.then(|| wall(1.0, IDENTITY_AFFINE)),
			..XrFrame::default()
		}))
	}

	fn stop(&mut self) {}
}

fn ahead(runtime: &XrRuntime) -> Vec<Hit> {
	runtime
		.hit_test(&HitTestRequest::screen(0.5, 0.5).targets(HitTargets::DEPTH))
		.unwrap()
}

#[test]
fn hit_tests_cast_against_estimated_depth_from_where_it_was_seen() {
	let runtime = XrRuntime::new(Retreat::new(0.5, false));
	let wall = Wall::default();
	let (seen, resets) = (Arc::clone(&wall.seen), Arc::clone(&wall.resets));
	runtime.set_depth_estimator(Some(Box::new(wall)));
	let info = runtime
		.start(SessionConfig::new(Mode::PhoneAr).prefer(Features::DEPTH))
		.unwrap();
	assert!(info.features.contains(Features::DEPTH));

	// The first frame is read at z = 0: the wall stands at z = −2.
	let first = runtime.update(0.0).unwrap().unwrap();
	assert_eq!(first.depth_view, Some(first.views[0]));
	let hit = ahead(&runtime)[0];
	assert_eq!(hit.kind, HitKind::EstimatedDepth);
	assert!((hit.distance - 2.0).abs() < 0.03, "{}", hit.distance);

	// Two frames later the viewer has backed a metre away, and the depth
	// is still the first frame's. The wall stays where it was: three metres
	// off now, not the two the old picture would say from here.
	runtime.update(0.0).unwrap();
	let third = runtime.update(0.0).unwrap().unwrap();
	assert_eq!(seen.lock().unwrap().len(), 1);
	assert!((third.viewer.position.z - 1.0).abs() < 1e-6);
	assert_eq!(third.depth_view, Some(first.views[0]));
	let hit = ahead(&runtime)[0];
	assert!((hit.distance - 3.0).abs() < 0.03, "{}", hit.distance);
	assert!((hit.pose.position.z + 2.0).abs() < 0.03);
	// Only having moved, not turned, the old depth lines up with the view.
	assert_eq!(
		third.depth_transform(),
		Some(third.depth.as_ref().unwrap().display_transform)
	);

	// Depth isn't asked for: nothing is hit.
	let planes = runtime
		.hit_test(&HitTestRequest::screen(0.5, 0.5).targets(HitTargets::PLANES))
		.unwrap();
	assert_eq!(planes, []);

	// Stopping forgets the old depth; the next session reads its own.
	runtime.stop();
	assert_eq!(resets.load(Ordering::SeqCst), 1);
	runtime
		.start(SessionConfig::new(Mode::PhoneAr).prefer(Features::DEPTH))
		.unwrap();
	let restarted = runtime.update(0.0).unwrap().unwrap();
	assert_eq!(seen.lock().unwrap().len(), 2);
	assert_eq!(restarted.depth_view, Some(restarted.views[0]));
}

#[test]
fn a_platform_that_measures_depth_keeps_its_own() {
	let runtime = XrRuntime::new(Retreat::new(0.5, true));
	let wall = Wall::default();
	let seen = Arc::clone(&wall.seen);
	runtime.set_depth_estimator(Some(Box::new(wall)));
	runtime
		.start(SessionConfig::new(Mode::PhoneAr).prefer(Features::DEPTH))
		.unwrap();

	runtime.update(0.0).unwrap();
	let frame = runtime.update(0.0).unwrap().unwrap();
	// The estimator is never asked: the platform's depth is newer.
	assert!(seen.lock().unwrap().is_empty());
	assert_eq!(frame.depth_view, None);
	assert_eq!(frame.to_guest(2, true).depth, Some(DepthSource::Measured));
	assert_eq!(
		frame.depth.as_ref().unwrap().sample_view(0.5, 0.5),
		Some(1.0)
	);
	// Measured this frame, so cast from where the viewer is now.
	let hit = ahead(&runtime)[0];
	assert_eq!(hit.kind, HitKind::Depth);
	assert!((hit.distance - 1.0).abs() < 0.03, "{}", hit.distance);
}
