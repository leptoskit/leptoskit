//! Renders a glTF model to a PNG, for looking at what the renderer draws.
//!
//! ```sh
//! cargo run -p leptoskit_3d --example render_model -- model.glb out.png [clip] [seconds] [options]
//! ```
//!
//! `HIDE=head_1,head_2` hides the named parts, as an app choosing between a
//! model's variants does.
//!
//! The camera frames the model from the front and a little above, over a
//! light grey background, with the clip (if any) played for `seconds`.
//!
//! Options choose the style, so a look can be tried on a real model:
//!
//! - `--toon` shades in [`Toon::default`]'s bands; `--bands 2`, `--softness
//!   0.2`, `--shade 0.4,0.4,0.6`, `--specular 1`, and `--hatch 0.6` change
//!   them.
//! - `--ink 2` draws an [`Ink::default`] line 2 thousandths of the view's
//!   height wide; `--ink-color 0.03,0.02,0.05`, `--facing 0.7`, `--shade-weight
//!   1`, `--wobble 0.3`, `--boil 8`, and `--seed 7` change it.
//! - `--distance 8` moves the camera back, `--size 360x480` sets the image's
//!   size, `--backdrop 0.1,0.1,0.12` its colour, and `--exposure 1.3` its
//!   exposure.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
	native::main()
}

/// Rendering offscreen needs a blocking device, which the browser lacks.
#[cfg(not(target_arch = "wasm32"))]
mod native {
	use std::collections::HashMap;
	use std::sync::Arc;

	use leptoskit_3d::*;

	type Failure = Box<dyn std::error::Error>;

	/// The command line: positional arguments, and `--name value` options
	/// (`--toon` takes no value).
	struct Arguments {
		positional: Vec<String>,
		options: HashMap<String, String>,
		toon: bool,
	}

	impl Arguments {
		fn parse() -> Self {
			let mut arguments = std::env::args().skip(1);
			let mut parsed = Self {
				positional: Vec::new(),
				options: HashMap::new(),
				toon: false,
			};

			while let Some(argument) = arguments.next() {
				match argument.strip_prefix("--") {
					Some("toon") => parsed.toon = true,
					Some(name) => {
						let value = arguments.next().unwrap_or_default();
						parsed.options.insert(name.to_owned(), value);
					}
					None => parsed.positional.push(argument),
				}
			}

			parsed
		}

		fn number(&self, name: &str) -> Result<Option<f32>, Failure> {
			self.options
				.get(name)
				.map(|value| value.parse::<f32>())
				.transpose()
				.map_err(|error| format!("--{name}: {error}").into())
		}

		fn color(&self, name: &str) -> Result<Option<[f32; 3]>, Failure> {
			let Some(value) = self.options.get(name) else {
				return Ok(None);
			};
			let parts = value
				.split(',')
				.map(str::parse::<f32>)
				.collect::<Result<Vec<_>, _>>()
				.map_err(|error| format!("--{name}: {error}"))?;
			let [red, green, blue] = parts[..] else {
				return Err(format!("--{name} takes three numbers, as 0.1,0.2,0.3").into());
			};
			Ok(Some([red, green, blue]))
		}

		/// The style the options describe.
		fn style(&self) -> Result<Style, Failure> {
			let shading = if self.toon || self.options.contains_key("bands") {
				let mut toon = Toon::default();

				if let Some(bands) = self.number("bands")? {
					toon.bands = bands as u8;
				}

				toon.softness = self.number("softness")?.unwrap_or(toon.softness);
				toon.shade = self.color("shade")?.unwrap_or(toon.shade);
				toon.specular = self.number("specular")?.unwrap_or(toon.specular);
				toon.hatch = self.number("hatch")?.unwrap_or(toon.hatch);
				Shading::Toon(toon)
			} else {
				Shading::Pbr
			};

			let ink = match self.number("ink")? {
				Some(width) => {
					let mut ink = Ink {
						width,
						..Ink::default()
					};
					ink.color = self.color("ink-color")?.unwrap_or(ink.color);
					ink.facing = self.number("facing")?.unwrap_or(ink.facing);
					ink.shade_weight = self.number("shade-weight")?.unwrap_or(ink.shade_weight);
					ink.wobble = self.number("wobble")?.unwrap_or(ink.wobble);
					ink.boil = self.number("boil")?.unwrap_or(ink.boil);

					if let Some(seed) = self.number("seed")? {
						ink.seed = seed as u32;
					}

					Some(ink)
				}
				None => None,
			};

			Ok(Style { shading, ink })
		}

		fn size(&self) -> Result<(u32, u32), Failure> {
			let Some(size) = self.options.get("size") else {
				return Ok((720, 960));
			};
			let (width, height) = size
				.split_once('x')
				.ok_or("--size takes a width and a height, as 720x960")?;
			Ok((width.parse()?, height.parse()?))
		}
	}

	pub fn main() -> Result<(), Failure> {
		let arguments = Arguments::parse();
		let mut positional = arguments.positional.iter();
		let (Some(input), Some(output)) = (positional.next(), positional.next()) else {
			eprintln!("usage: render_model model.glb out.png [clip] [seconds] [options]");
			std::process::exit(2);
		};
		let clip = positional.next();
		let seconds: f32 = positional.next().map_or(Ok(0.5), |value| value.parse())?;
		let style = arguments.style()?;
		let distance = arguments.number("distance")?.unwrap_or(3.6);
		let backdrop = arguments.color("backdrop")?.unwrap_or([0.8, 0.8, 0.82]);
		let exposure = arguments.number("exposure")?.unwrap_or(1.0);
		let (width, height) = arguments.size()?;
		let asset = Arc::new(Asset::from_slice(&std::fs::read(input)?)?);
		let gpu = Gpu::headless()?;

		let mut renderer = Renderer::new(&gpu);
		let world = renderer.world_mut();
		world.apply([
			Op::LoadAsset {
				asset: AssetId(1),
				path: "model.glb".into(),
			},
			Op::Spawn {
				node: NodeId(1),
				asset: Some(AssetId(1)),
				parent: None,
			},
			Op::SetBackdrop(Backdrop::Color([
				backdrop[0],
				backdrop[1],
				backdrop[2],
				1.0,
			])),
			Op::SetExposure(exposure),
			Op::SetStyle(style),
		]);
		world.take_loads();
		world.asset_loaded(AssetId(1), Ok(asset.clone()));
		world.advance(0.0);
		// Frame the model's bounds, found by casting rays would be overkill:
		// a person-sized model sits in a 2.5 m box at the origin. Further
		// cameras back away along the same line, and the default distance
		// places the camera exactly where it always was.
		let eye = glam::Vec3::new(0.0, 1.3, 3.6);
		let aim = glam::Vec3::new(0.0, 1.3 - 3.6 * 0.12_f32.tan(), 0.0);
		let position = eye + (eye - aim) * ((distance - 3.6) / 3.6);
		world.apply([Op::SetCamera {
			transform: Transform::from_translation(position.x, position.y, position.z)
				.with_rotation(glam::Quat::from_rotation_x(-0.12).to_array()),
			fov_y: 0.75,
			near: 0.05,
			far: 50.0,
		}]);

		for part in std::env::var("HIDE")
			.unwrap_or_default()
			.split(',')
			.filter(|part| !part.is_empty())
		{
			world.apply([Op::SetPartVisible {
				node: NodeId(1),
				part: part.into(),
				visible: false,
			}]);
		}

		if let Some(clip) = clip {
			world.apply([Op::Play {
				node: NodeId(1),
				play: Play::new(clip.clone()),
			}]);
		}

		let steps = (seconds * 60.0) as usize;

		for _ in 0..steps.max(1) {
			renderer.advance(1.0 / 60.0);
		}

		let pixels = renderer.render_to_rgba(width, height)?;
		let file = std::fs::File::create(output)?;
		let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
		encoder.set_color(png::ColorType::Rgba);
		encoder.set_depth(png::BitDepth::Eight);
		encoder.write_header()?.write_image_data(&pixels)?;
		let stats = renderer.stats();

		println!(
			"{output}: {} draws, {} outlines, {} instances, {} triangles, clips {:?}",
			stats.draws,
			stats.outlines,
			stats.instances,
			stats.triangles,
			asset.clip_names().take(8).collect::<Vec<_>>()
		);
		Ok(())
	}
}
