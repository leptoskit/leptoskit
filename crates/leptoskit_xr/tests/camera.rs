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

/// An estimator that answers every frame with a flat wall two metres away,
/// and remembers what it saw.
#[derive(Default)]
struct Wall {
	seen: Arc<Mutex<Vec<(u32, u32, f64)>>>,
	latest: Option<DepthImage>,
}

impl DepthEstimator for Wall {
	fn max_side(&self) -> u32 {
		2
	}

	fn submit(&mut self, frame: Arc<CameraFrame>) {
		self.seen
			.lock()
			.unwrap()
			.push((frame.rgb.width, frame.rgb.height, frame.time));
		let meters: Vec<u8> = std::iter::repeat_n(2.0_f32.to_le_bytes(), 4)
			.flatten()
			.collect();
		self.latest = Some(DepthImage {
			width: 2,
			height: 2,
			format: DepthFormat::Meters32,
			data: meters.into(),
			row_stride: 8,
			confidence: None,
			display_transform: frame.display_transform,
		});
	}

	fn latest(&mut self) -> Option<DepthImage> {
		self.latest.clone()
	}

	fn reset(&mut self) {
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
	assert!(frame.to_guest(1, true).depth);
}
