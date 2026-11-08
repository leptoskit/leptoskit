//! The real depth model behind a session: a phone with no depth of its own
//! looks down a corridor, and hit tests reach the room through the depth the
//! model reads from the camera picture.
//!
//! The model is 27 MB and not in the repository. This runs it when
//! `LEPTOSKIT_DEPTH_MODEL` names the file (see `leptoskit_depth`'s readme
//! for how to fetch it), and passes with a note on stderr when it doesn't.

use std::time::Duration;
use std::time::Instant;

use leptoskit_depth::DepthModel;
use leptoskit_depth::MODEL;
use leptoskit_xr::host::*;
use leptoskit_xr::image::*;
use leptoskit_xr::*;
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

/// The focal length the corridor picture is taken to have, in its pixels.
const FOCAL: f32 = 300.0;

fn model() -> Option<DepthModel> {
	let Some(path) = std::env::var_os("LEPTOSKIT_DEPTH_MODEL") else {
		eprintln!("LEPTOSKIT_DEPTH_MODEL is not set: the model was not run");
		return None;
	};
	let bytes = std::fs::read(&path).expect("LEPTOSKIT_DEPTH_MODEL names a readable file");
	MODEL
		.verify(&bytes)
		.expect("LEPTOSKIT_DEPTH_MODEL is the pinned model");
	Some(DepthModel::from_bytes(&bytes).expect("the pinned model loads"))
}

/// `leptoskit_depth`'s corridor fixture, upright, as an RGBA camera picture.
fn corridor() -> ImagePlane {
	let path = concat!(
		env!("CARGO_MANIFEST_DIR"),
		"/../leptoskit_depth/tests/fixtures/corridor.jpg"
	);
	let bytes = std::fs::read(path).unwrap();
	let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
	let mut decoder = JpegDecoder::new_with_options(bytes.as_slice(), options);
	let rgb = decoder.decode().unwrap();
	let (width, height) = decoder.dimensions().unwrap();
	let rgba: Vec<u8> = rgb
		.as_chunks::<3>()
		.0
		.iter()
		.flat_map(|&[red, green, blue]| [red, green, blue, 255])
		.collect();
	ImagePlane {
		width: width as u32,
		height: height as u32,
		row_stride: width as u32 * 4,
		data: rgba.into(),
	}
}

/// A phone with tracking and a camera but no depth, standing still and
/// looking at the corridor; its view is the picture, uncropped.
struct Corridor {
	picture: ImagePlane,
	features: Features,
	frames: u32,
}

impl Corridor {
	fn intrinsics(&self) -> Intrinsics {
		let (width, height) = (self.picture.width, self.picture.height);
		Intrinsics {
			focal: [FOCAL, FOCAL],
			principal: [width as f32 / 2.0, height as f32 / 2.0],
			size: [width, height],
		}
	}
}

impl Backend for Corridor {
	fn capabilities(&self) -> Capabilities {
		Capabilities::NONE.with(Mode::PhoneAr, Features::CAMERA_IMAGE | Features::HIT_TEST)
	}

	fn prepare(&mut self, _mode: Mode, _install: bool) -> Availability {
		Availability::Ready
	}

	fn start(&mut self, config: &SessionConfig, features: Features) -> Result<Started, Error> {
		self.features = features;
		Ok(Started::new(config.reference_space, features))
	}

	fn update(&mut self, _time: f64) -> Result<Option<XrFrame>, Error> {
		self.frames += 1;
		let intrinsics = self.intrinsics();
		let [width, height] = intrinsics.size.map(|side| side as f32);
		let pose = Pose::at(Vec3::new(0.0, 1.5, 0.0));
		let camera = self.features.contains(Features::CAMERA_IMAGE);

		Ok(Some(XrFrame {
			time: f64::from(self.frames) / 30.0,
			tracking: TrackingState::Normal,
			viewer: pose,
			views: vec![View {
				pose,
				projection: Projection::Fov(Fov {
					left: -(width / 2.0 / FOCAL).atan(),
					right: (width / 2.0 / FOCAL).atan(),
					up: (height / 2.0 / FOCAL).atan(),
					down: -(height / 2.0 / FOCAL).atan(),
				}),
			}],
			camera_image: camera.then(|| {
				CameraImage {
					width: self.picture.width,
					height: self.picture.height,
					display_transform: IDENTITY_AFFINE,
					pixels: CameraPixels::Rgba(self.picture.clone()),
				}
			}),
			camera_intrinsics: camera.then_some(intrinsics),
			..XrFrame::default()
		}))
	}

	fn stop(&mut self) {}
}

/// The median distance to what the rays through `points` hit.
fn median_hit(runtime: &XrRuntime, points: &[(f32, f32)]) -> f32 {
	let mut distances: Vec<f32> = points
		.iter()
		.filter_map(|&(x, y)| {
			runtime
				.hit_test(&HitTestRequest::screen(x, y).targets(HitTargets::DEPTH))
				.unwrap()
				.first()
				.map(|hit| hit.distance)
		})
		.collect();
	assert_eq!(distances.len(), points.len(), "every ray hits something");
	distances.sort_by(f32::total_cmp);
	distances[distances.len() / 2]
}

#[test]
fn a_phone_without_depth_reaches_the_room_through_the_model() {
	let Some(model) = model() else {
		return;
	};

	let runtime = XrRuntime::new(Corridor {
		picture: corridor(),
		features: Features::NONE,
		frames: 0,
	});
	let estimator = ModelEstimator::spawn(model).unwrap();
	runtime.set_depth_estimator(Some(Box::new(estimator.clone())));
	let info = runtime
		.start(SessionConfig::new(Mode::PhoneAr).require(Features::DEPTH))
		.unwrap();
	assert!(info.features.contains(Features::DEPTH));

	let started = Instant::now();
	let frame = loop {
		let frame = runtime.update(0.0).unwrap().unwrap();
		if frame.depth.is_some() {
			break frame;
		}
		assert!(
			started.elapsed() < Duration::from_secs(120),
			"no depth within two minutes: {:?}",
			estimator.last_error()
		);
		std::thread::sleep(Duration::from_millis(20));
	};
	eprintln!("depth after {:?}", started.elapsed());
	assert_eq!(frame.depth_view, Some(frame.views[0]));
	assert!(frame.to_guest(frame.time as u64, false).depth);

	// The stone floor at the bottom of the picture is nearer than the boiler
	// at the end of the corridor, and both are a corridor's metres away.
	let floor = median_hit(
		&runtime,
		&[
			(0.3, 0.94),
			(0.4, 0.94),
			(0.5, 0.94),
			(0.6, 0.94),
			(0.7, 0.94),
		],
	);
	let boiler = median_hit(
		&runtime,
		&[
			(0.4, 0.55),
			(0.5, 0.55),
			(0.6, 0.55),
			(0.5, 0.65),
			(0.5, 0.6),
		],
	);
	eprintln!("floor {floor:.2} m, boiler {boiler:.2} m along their rays");
	assert!(floor < boiler, "floor {floor} m, boiler {boiler} m");
	assert!((0.3..6.0).contains(&boiler), "boiler {boiler} m");

	runtime.stop();
	assert!(runtime.update(0.0).unwrap().is_none());
}
