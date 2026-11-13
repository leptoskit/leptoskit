//! The frame loop: tick, commit the menu, paint, write a PNG.

use std::path::Path;
use std::path::PathBuf;

use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeSurface;

use crate::Error;
use crate::overlay;
use crate::project::Camera;
use crate::project::SKY;
use crate::raster::DrawList;
use crate::scene;
use crate::sim::Game;
use crate::sim::Sim;
use crate::world::World;

/// The longest edge of a frame, in pixels.
const MAX_EDGE: u32 = 4096;

/// Which painter draws the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Painter {
	/// wgpu, then the CPU painter if the adapter is missing.
	#[default]
	Auto,
	/// wgpu only.
	Gpu,
	/// The CPU painter.
	Cpu,
}

/// How many frames to run, and where to put the last one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
	/// Fixed steps to run. Each step is one frame.
	pub frames: u32,
	/// Frame width in pixels.
	pub width: u32,
	/// Frame height in pixels.
	pub height: u32,
	/// Where to write the last frame. `None` keeps it in memory only.
	pub out: Option<PathBuf>,
	/// Actions held for every tick.
	pub hold: Vec<String>,
	/// A control label to press halfway through the run.
	pub tap: Option<String>,
	/// Which painter to use.
	pub painter: Painter,
}

impl Default for Options {
	fn default() -> Self {
		Self {
			frames: 12,
			width: 480,
			height: 320,
			out: Some(PathBuf::from("artifacts/frame.png")),
			hold: Vec::new(),
			tap: None,
			painter: Painter::Auto,
		}
	}
}

impl Options {
	/// Parses the game binary's arguments.
	pub fn parse<I, S>(args: I) -> Result<Self, Error>
	where
		I: IntoIterator<Item = S>,
		S: AsRef<str>,
	{
		let mut options = Self::default();
		let mut args = args.into_iter();
		while let Some(arg) = args.next() {
			match arg.as_ref() {
				"--frames" => options.frames = flag_u32("--frames", args.next())?,
				"--width" => options.width = flag_u32("--width", args.next())?,
				"--height" => options.height = flag_u32("--height", args.next())?,
				"--out" => options.out = Some(PathBuf::from(flag_value("--out", args.next())?)),
				"--no-out" => options.out = None,
				"--hold" => options.hold.push(flag_value("--hold", args.next())?),
				"--tap" => options.tap = Some(flag_value("--tap", args.next())?),
				"--painter" => {
					options.painter = parse_painter(&flag_value("--painter", args.next())?)?;
				}
				other => return Err(Error::Argument(other.to_owned())),
			}
		}
		Ok(options)
	}
}

fn flag_value(flag: &'static str, value: Option<impl AsRef<str>>) -> Result<String, Error> {
	value
		.map(|value| value.as_ref().to_owned())
		.ok_or(Error::Missing(flag))
}

fn flag_u32(flag: &'static str, value: Option<impl AsRef<str>>) -> Result<u32, Error> {
	let value = flag_value(flag, value)?;
	value.parse().map_err(|_| Error::Number { flag, value })
}

fn parse_painter(value: &str) -> Result<Painter, Error> {
	match value {
		"auto" => Ok(Painter::Auto),
		"gpu" => Ok(Painter::Gpu),
		"cpu" => Ok(Painter::Cpu),
		_ => Err(Error::Painter(value.to_owned())),
	}
}

/// What a run produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
	/// [`Game::ID`](crate::Game::ID).
	pub id: &'static str,
	/// Frames that ran.
	pub frames: u32,
	/// Completed ticks.
	pub tick: u64,
	/// `gpu` or `cpu`.
	pub painter: &'static str,
	/// The file that was written, when one was requested.
	pub out: Option<PathBuf>,
	/// Number publications, in name order.
	pub numbers: Vec<(String, f64)>,
	/// Mean time of one frame, in microseconds, including the tick and the
	/// paint.
	pub frame_us: u64,
}

/// Reads the process arguments, runs `game`, and writes the last frame.
pub fn run<G, H>(hud: H) -> Result<Report, Error>
where
	G: Game,
	H: FnMut(&World) -> leptoskit_native::Element,
{
	run_with::<G, H>(Options::parse(std::env::args().skip(1))?, hud)
}

/// Runs `game` with `options`. `hud` is the menu for the world after each tick.
pub fn run_with<G, H>(options: Options, mut hud: H) -> Result<Report, Error>
where
	G: Game,
	H: FnMut(&World) -> leptoskit_native::Element,
{
	validate(&options)?;
	let mut sim = Sim::<G>::new();
	sim.set_held(&options.hold);
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(options.width as f32, options.height as f32);
	let framed = crate::project::camera_frame(&Camera::default(), options.width, options.height);
	let mut gpu = scene::GpuPainter::new();
	let mut pixels = Vec::new();
	let tap_at = options.tap.as_ref().map(|_| options.frames / 2);
	let started = std::time::Instant::now();
	let mut painter_name = "cpu";
	for index in 0..options.frames {
		sim.step();
		surface.set_owned_root(hud(sim.world()));
		if let Some(label) = options.tap.as_deref()
			&& tap_at == Some(index)
		{
			overlay::tap(&surface, label)?;
		}
		let list = compose(sim.world(), &surface, &framed);
		painter_name = paint_frame(
			&mut gpu,
			&mut pixels,
			&list,
			options.width,
			options.height,
			options.painter,
		)?;
	}
	let frame_us = (started.elapsed().as_micros() / u128::from(options.frames)) as u64;
	if let Some(path) = &options.out {
		write_png(path, options.width, options.height, &pixels)?;
	}
	let report = Report {
		id: G::ID,
		frames: options.frames,
		tick: sim.world().tick(),
		painter: painter_name,
		out: options.out,
		numbers: sim.world().numbers(),
		frame_us,
	};
	print_report(&report);
	Ok(report)
}

fn validate(options: &Options) -> Result<(), Error> {
	if options.frames == 0 || options.width == 0 || options.height == 0 {
		return Err(Error::Empty);
	}
	if options.width > MAX_EDGE || options.height > MAX_EDGE {
		return Err(Error::TooLarge);
	}
	if options.tap.is_some() && options.frames < 2 {
		return Err(Error::TapFrames);
	}
	Ok(())
}

fn compose(
	world: &World,
	surface: &NativeSurface<HeadlessBackend>,
	framed: &crate::project::Projection,
) -> DrawList {
	let overlay = overlay::read(surface);
	DrawList {
		sky: SKY,
		quads: crate::project::quads_in(world, framed),
		rects: overlay.rects,
		glyphs: overlay.glyphs,
	}
}

fn paint_frame(
	gpu: &mut scene::GpuPainter,
	pixels: &mut Vec<u8>,
	list: &DrawList,
	width: u32,
	height: u32,
	painter: Painter,
) -> Result<&'static str, Error> {
	paint_with(
		pixels,
		list,
		width,
		height,
		painter,
		|scene, width, height| gpu.paint(scene, width, height),
	)
}

fn paint_with(
	pixels: &mut Vec<u8>,
	list: &DrawList,
	width: u32,
	height: u32,
	painter: Painter,
	mut gpu: impl FnMut(&leptoskit_canvas::Scene, u32, u32) -> Result<Vec<u8>, String>,
) -> Result<&'static str, Error> {
	if painter == Painter::Cpu {
		crate::raster::paint_into(pixels, list, width, height);
		return Ok("cpu");
	}
	let scene = scene::to_scene(list, width, height);
	match gpu(&scene, width, height) {
		Ok(mut rgba) => {
			crate::raster::stamp_glyphs(&mut rgba, list, width, height);
			*pixels = rgba;
			Ok("gpu")
		}
		Err(error) if painter == Painter::Gpu => Err(Error::Gpu(error)),
		Err(_) => {
			crate::raster::paint_into(pixels, list, width, height);
			Ok("cpu")
		}
	}
}

fn print_report(report: &Report) {
	println!("{}", report.id);
	println!("painter: {}", report.painter);
	println!("frames: {}", report.frames);
	println!("tick: {}", report.tick);
	println!("frame_us: {}", report.frame_us);
	for (name, value) in &report.numbers {
		println!("{name}: {value}");
	}
	if let Some(path) = &report.out {
		println!("wrote: {}", path.display());
	}
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), Error> {
	if let Some(parent) = path.parent()
		&& !parent.as_os_str().is_empty()
	{
		std::fs::create_dir_all(parent).map_err(|source| {
			Error::Write {
				path: path.to_path_buf(),
				source,
			}
		})?;
	}
	let file = std::fs::File::create(path).map_err(|source| {
		Error::Write {
			path: path.to_path_buf(),
			source,
		}
	})?;
	let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	encoder
		.write_header()
		.and_then(|mut writer| writer.write_image_data(rgba))
		.map_err(|error| {
			Error::Encode {
				path: path.to_path_buf(),
				message: error.to_string(),
			}
		})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::project::SKY;
	use crate::raster::DrawList;

	struct Still;

	impl Game for Still {
		const ID: &'static str = "still";

		fn setup(world: &mut World) {
			let _ = world.cube("box").at(0.0, 0.5, 0.0).color("#e7b15a");
		}

		fn tick(world: &mut World, input: &crate::Input) {
			if input.held("move") {
				world.position_mut("box").unwrap()[0] += world.dt();
			}
			let x = world.position("box").unwrap()[0];
			world.publish("x", x);
		}
	}

	fn hud(_: &World) -> leptoskit_native::Element {
		leptoskit_native::components::View().class("w-full h-full")
	}

	#[test]
	fn arguments_and_a_cpu_run() {
		let options = Options::parse([
			"--frames",
			"2",
			"--width",
			"32",
			"--height",
			"24",
			"--hold",
			"move",
			"--painter",
			"cpu",
			"--no-out",
		])
		.unwrap();
		assert_eq!(options.frames, 2);
		assert_eq!(options.painter, Painter::Cpu);
		assert!(options.out.is_none());
		let report = run_with::<Still, _>(options, hud).unwrap();
		assert_eq!(report.id, "still");
		assert_eq!(report.painter, "cpu");
		assert_eq!(report.tick, 2);
		assert!(report.numbers[0].1 > 0.0);
		assert!(report.frame_us < 100_000, "{}", report.frame_us);

		let path = std::env::temp_dir().join(format!("leptoskit-game-{}.png", std::process::id()));
		let written = run_with::<Still, _>(
			Options {
				frames: 1,
				width: 16,
				height: 16,
				out: Some(path.clone()),
				hold: Vec::new(),
				tap: None,
				painter: Painter::Cpu,
			},
			hud,
		)
		.unwrap();
		assert_eq!(written.out.as_deref(), Some(path.as_path()));
		assert!(path.is_file());
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn rejected_arguments_and_frames() {
		assert!(matches!(
			Options::parse(["--frames"]),
			Err(Error::Missing(_))
		));
		assert!(matches!(
			Options::parse(["--frames", "nope"]),
			Err(Error::Number { .. })
		));
		assert!(matches!(
			Options::parse(["--painter", "ink"]),
			Err(Error::Painter(_))
		));
		assert!(matches!(
			Options::parse(["--nope"]),
			Err(Error::Argument(_))
		));
		let cpu = Options {
			out: None,
			painter: Painter::Cpu,
			..Options::default()
		};
		assert!(matches!(
			run_with::<Still, _>(
				Options {
					frames: 0,
					..cpu.clone()
				},
				hud
			),
			Err(Error::Empty)
		));
		assert!(matches!(
			run_with::<Still, _>(
				Options {
					height: 0,
					..cpu.clone()
				},
				hud
			),
			Err(Error::Empty)
		));
		let bad =
			std::env::temp_dir().join(format!("leptoskit-game-bad-{}.png", std::process::id()));
		assert!(matches!(
			write_png(&bad, 2, 2, &[0, 1, 2]),
			Err(Error::Encode { .. })
		));
		assert!(matches!(
			run_with::<Still, _>(
				Options {
					width: MAX_EDGE + 1,
					out: None,
					painter: Painter::Cpu,
					..Options::default()
				},
				hud
			),
			Err(Error::TooLarge)
		));
		assert!(matches!(
			run_with::<Still, _>(
				Options {
					frames: 1,
					tap: Some("Restart".to_owned()),
					out: None,
					painter: Painter::Cpu,
					..Options::default()
				},
				hud
			),
			Err(Error::TapFrames)
		));
	}

	#[test]
	fn auto_falls_back_when_the_gpu_refuses() {
		let list = DrawList {
			sky: SKY,
			..DrawList::default()
		};
		let mut pixels = Vec::new();
		let cpu = paint_with(&mut pixels, &list, 4, 4, Painter::Auto, |_, _, _| {
			Err("no adapter".to_owned())
		})
		.unwrap();
		assert_eq!(cpu, "cpu");
		assert_eq!(pixels.len(), 4 * 4 * 4);
		let gpu = paint_with(&mut pixels, &list, 2, 2, Painter::Gpu, |_, _, _| {
			Ok(vec![9; 16])
		})
		.unwrap();
		assert_eq!(gpu, "gpu");
		assert_eq!(pixels.len(), 16);
		assert!(matches!(
			paint_with(&mut pixels, &list, 2, 2, Painter::Gpu, |_, _, _| {
				Err("missing".to_owned())
			}),
			Err(Error::Gpu(_))
		));
	}
}
