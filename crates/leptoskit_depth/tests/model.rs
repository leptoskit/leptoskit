//! The model on photographs of real rooms (see `fixtures/readme.md`): what
//! is nearer must read nearer, a flat wall must read flat, and the metres
//! must be a room's.
//!
//! The model is 27 MB and not in the repository. These tests run it when
//! `LEPTOSKIT_DEPTH_MODEL` names the file (see the crate's readme for how
//! to fetch it), and pass with a note on stderr when it doesn't.

use std::sync::OnceLock;
use std::time::Duration;

use leptoskit_depth::DepthMap;
use leptoskit_depth::DepthModel;
use leptoskit_depth::DepthWorker;
use leptoskit_depth::Frame;
use leptoskit_depth::Intrinsics;
use leptoskit_depth::MODEL;
use leptoskit_depth::Orientation;
use leptoskit_depth::Picture;
use leptoskit_depth::Pixels;
use leptoskit_depth::Plane;
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

fn model() -> Option<&'static DepthModel> {
	static MODEL_FILE: OnceLock<Option<DepthModel>> = OnceLock::new();

	MODEL_FILE
		.get_or_init(|| {
			let Some(path) = std::env::var_os("LEPTOSKIT_DEPTH_MODEL") else {
				eprintln!("LEPTOSKIT_DEPTH_MODEL is not set: the model was not run");
				return None;
			};
			let bytes = std::fs::read(&path).expect("LEPTOSKIT_DEPTH_MODEL names a readable file");
			MODEL
				.verify(&bytes)
				.expect("LEPTOSKIT_DEPTH_MODEL is the pinned model");
			Some(DepthModel::from_bytes(&bytes).expect("the pinned model loads"))
		})
		.as_ref()
}

/// An upright fixture picture as RGB.
fn fixture(name: &str) -> Plane {
	let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
	let bytes = std::fs::read(path).unwrap();
	let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
	let mut decoder = JpegDecoder::new_with_options(bytes.as_slice(), options);
	let rgb = decoder.decode().unwrap();
	let (width, height) = decoder.dimensions().unwrap();
	Plane::packed(width as u32, height as u32, 3, rgb)
}

fn estimate(picture: &Picture) -> DepthMap {
	let depth = model().unwrap().estimate(picture).unwrap();
	eprintln!(
		"{} × {} in {:?}, scale {:.2}",
		depth.width, depth.height, depth.took, depth.scale
	);
	depth
}

/// The median known depth over a band of the picture, `(0, 0)` top-left to
/// `(1, 1)` bottom-right.
fn median(depth: &DepthMap, across: (f32, f32), down: (f32, f32)) -> f32 {
	let mut values: Vec<f32> = (0..depth.height)
		.flat_map(|y| (0..depth.width).map(move |x| (x, y)))
		.filter(|&(x, y)| {
			let u = (x as f32 + 0.5) / depth.width as f32;
			let v = (y as f32 + 0.5) / depth.height as f32;
			(across.0..across.1).contains(&u) && (down.0..down.1).contains(&v)
		})
		.filter_map(|(x, y)| depth.get(x, y))
		.collect();
	assert!(!values.is_empty(), "nothing known in {across:?} × {down:?}");
	values.sort_by(f32::total_cmp);
	values[values.len() / 2]
}

#[test]
fn a_kitchen_island_reads_nearer_than_the_cooker_behind_it() {
	if model().is_none() {
		return;
	}

	let depth = estimate(&Picture::new(Pixels::Rgb8(fixture("kitchen.jpg"))));
	// A 3:4 picture into a 9:16 model: its sides are not read.
	assert!(
		(depth.coverage() - 0.75).abs() < 0.03,
		"{}",
		depth.coverage()
	);
	assert_eq!(depth.sample(0.02, 0.5), None);

	let island = median(&depth, (0.3, 0.7), (0.85, 1.0));
	let cooker = median(&depth, (0.3, 0.7), (0.35, 0.5));
	eprintln!("island {island:.2} m, cooker {cooker:.2} m");
	assert!(
		island < 0.6 * cooker,
		"island {island} m, cooker {cooker} m"
	);
	// A kitchen's metres, not a stadium's or a thumbnail's.
	assert!((0.3..1.5).contains(&island), "island {island} m");
	assert!((1.5..5.0).contains(&cooker), "cooker {cooker} m");
}

#[test]
fn a_corridor_s_floor_reads_nearer_than_its_end() {
	if model().is_none() {
		return;
	}

	let depth = estimate(&Picture::new(Pixels::Rgb8(fixture("corridor.jpg"))));
	assert!(depth.coverage() > 0.99);
	let floor = median(&depth, (0.2, 0.8), (0.88, 1.0));
	let boiler = median(&depth, (0.35, 0.65), (0.5, 0.7));
	eprintln!("floor {floor:.2} m, boiler {boiler:.2} m");
	assert!(floor < 0.75 * boiler, "floor {floor} m, boiler {boiler} m");
	assert!((0.3..4.0).contains(&boiler), "boiler {boiler} m");
}

#[test]
fn a_wall_faced_square_on_reads_flat() {
	if model().is_none() {
		return;
	}

	// Landscape and upright: the model reads a portrait strip from its middle.
	let depth = estimate(&Picture::new(Pixels::Rgb8(fixture("wall.jpg"))));
	let mut values: Vec<f32> = depth.depths.iter().copied().filter(|d| *d > 0.0).collect();
	values.sort_by(f32::total_cmp);
	let quartile = |share: f32| values[(share * (values.len() - 1) as f32) as usize];
	let (low, middle, high) = (quartile(0.25), quartile(0.5), quartile(0.75));
	eprintln!("wall {low:.2} / {middle:.2} / {high:.2} m");
	assert!((high - low) / middle < 0.25, "{low} / {middle} / {high} m");
	assert!((0.5..5.0).contains(&middle), "{middle} m");
}

/// `plane` turned a quarter counterclockwise, as a sensor mounted a quarter
/// turn from upright reads it.
fn sideways(plane: &Plane) -> Plane {
	let (width, height) = (plane.width as usize, plane.height as usize);
	let mut data = Vec::with_capacity(width * height * 3);
	for y in 0..width {
		for x in 0..height {
			// Sensor (x, y) is upright (width - 1 - y, x).
			let at = x * plane.row_stride as usize + (width - 1 - y) * 3;
			data.extend_from_slice(&plane.data[at..at + 3]);
		}
	}
	Plane::packed(height as u32, width as u32, 3, data)
}

#[test]
fn a_sideways_picture_reads_as_it_would_upright() {
	if model().is_none() {
		return;
	}

	let upright = fixture("kitchen.jpg");
	let lens = Intrinsics::from_fov(upright.width, upright.height, 1.2);
	let sensor_lens = Intrinsics {
		focal: lens.focal,
		principal: [lens.principal[1], lens.principal[0]],
		size: [lens.size[1], lens.size[0]],
	};
	let reference = estimate(&Picture::new(Pixels::Rgb8(upright.clone())).with_intrinsics(lens));
	let turned = estimate(
		&Picture::new(Pixels::Rgb8(sideways(&upright)))
			.with_orientation(Orientation::Clockwise)
			.with_intrinsics(sensor_lens),
	);
	assert_eq!(
		(turned.width, turned.height),
		(reference.height, reference.width)
	);
	assert!((turned.scale - reference.scale).abs() < 1e-6);

	// The sensor's (u, v) is the upright picture's (1 − v, u).
	let mut differences = Vec::new();
	for step in 1..20 {
		for across in 1..20 {
			let (u, v) = (across as f32 / 20.0, step as f32 / 20.0);
			if let (Some(a), Some(b)) = (turned.sample(u, v), reference.sample(1.0 - v, u)) {
				differences.push((a - b).abs() / b);
			}
		}
	}
	let mean = differences.iter().sum::<f32>() / differences.len() as f32;
	assert!(
		differences.len() > 200 && mean < 0.03,
		"{} points, {mean}",
		differences.len()
	);
}

#[test]
fn a_longer_lens_reads_the_same_room_farther() {
	if model().is_none() {
		return;
	}

	let plane = fixture("corridor.jpg");
	let wide = Intrinsics::from_fov(plane.width, plane.height, 1.3);
	let narrow = Intrinsics::from_fov(plane.width, plane.height, 0.9);
	let near = estimate(&Picture::new(Pixels::Rgb8(plane.clone())).with_intrinsics(wide));
	let far = estimate(&Picture::new(Pixels::Rgb8(plane)).with_intrinsics(narrow));
	assert!(far.scale < near.scale);
	let ratio = far.get(126, 300).unwrap() / near.get(126, 300).unwrap();
	assert!((ratio - near.scale / far.scale).abs() < 1e-4, "{ratio}");
}

#[test]
fn the_worker_reads_a_picture_off_the_caller_s_thread() {
	let Some(model) = model() else {
		return;
	};

	let worker = DepthWorker::with_period(model.clone(), Duration::ZERO).unwrap();
	assert_eq!(worker.max_side(), 896);
	let frame = Frame {
		picture: Picture::new(Pixels::Rgb8(fixture("corridor.jpg"))),
		time: 12.5,
		context: [0.0_f32; 6],
	};
	assert!(worker.submit(frame));
	let estimate = (0..600)
		.find_map(|_| {
			std::thread::sleep(Duration::from_millis(100));
			worker.latest()
		})
		.expect("an estimate within a minute");
	assert_eq!((estimate.sequence, estimate.time), (1, 12.5));
	assert!(estimate.depth.coverage() > 0.99);
	assert_eq!(worker.last_error(), None);
}
