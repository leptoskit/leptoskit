//! How long the model takes to load and to read one picture on this
//! machine.
//!
//! ```sh
//! cargo run --release -p leptoskit_depth --example benchmark -- path/to/model.onnx [runs]
//! ```
//!
//! The model path can also come from `LEPTOSKIT_DEPTH_MODEL`. It reads the
//! kitchen fixture, a 252 × 336 picture, once to warm up and then `runs`
//! times (10 by default), and prints the load time and the median and
//! fastest reads.

use std::time::Duration;
use std::time::Instant;

use leptoskit_depth::DepthModel;
use leptoskit_depth::MODEL;
use leptoskit_depth::Picture;
use leptoskit_depth::Pixels;
use leptoskit_depth::Plane;
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let mut arguments = std::env::args().skip(1);
	let path = arguments
		.next()
		.or_else(|| std::env::var("LEPTOSKIT_DEPTH_MODEL").ok())
		.ok_or("pass the model's path, or set LEPTOSKIT_DEPTH_MODEL")?;
	let runs: usize = arguments.next().map_or(Ok(10), |runs| runs.parse())?;

	let bytes = std::fs::read(&path)?;
	match MODEL.verify(&bytes) {
		Ok(()) => println!("model: {} (pinned)", MODEL.name),
		Err(error) => println!("model: {path} ({error})"),
	}

	let started = Instant::now();
	let model = DepthModel::from_bytes(&bytes)?;
	println!("load:   {:>7.1} ms", millis(started.elapsed()));

	let picture = Picture::new(Pixels::Rgb8(kitchen()?));
	let first = model.estimate(&picture)?;
	println!("first:  {:>7.1} ms", millis(first.took));

	let mut times: Vec<Duration> = (0..runs.max(1))
		.map(|_| model.estimate(&picture).map(|depth| depth.took))
		.collect::<Result<_, _>>()?;
	times.sort();
	println!(
		"median: {:>7.1} ms over {} runs",
		millis(times[times.len() / 2]),
		times.len()
	);
	println!("min:    {:>7.1} ms", millis(times[0]));
	println!(
		"depth at the centre: {:.2} m",
		first.sample(0.5, 0.5).unwrap_or_default()
	);
	Ok(())
}

fn kitchen() -> Result<Plane, Box<dyn std::error::Error>> {
	let bytes = std::fs::read(concat!(
		env!("CARGO_MANIFEST_DIR"),
		"/tests/fixtures/kitchen.jpg"
	))?;
	let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
	let mut decoder = JpegDecoder::new_with_options(bytes.as_slice(), options);
	let rgb = decoder.decode()?;
	let (width, height) = decoder.dimensions().ok_or("the fixture has no size")?;
	Ok(Plane::packed(width as u32, height as u32, 3, rgb))
}

fn millis(duration: Duration) -> f64 {
	duration.as_secs_f64() * 1000.0
}
