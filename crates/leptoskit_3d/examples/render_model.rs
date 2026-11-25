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
//! - `--print` shades in [`Print::default`]'s relief-print bands; `--t-lit`,
//!   `--t-deep`, `--t-cross`, `--rim`, `--paper r,g,b`, `--knockout`,
//!   `--hatch-per-m`, `--hatch-gain`, `--hatch-max`, `--cross-duty`, `--fade-px
//!   lo,hi`, `--side-heavy`, `--side-drop`, `--side-from`, `--side-to`,
//!   `--wobble-frequency`, and `--palette file.json` configure it.
//! - `--ink 2` draws an [`Ink::default`] line 2 thousandths of the view's
//!   height wide; `--ink-color 0.03,0.02,0.05`, `--facing 0.7`, `--shade-weight
//!   1`, `--wobble 0.3`, `--boil 8`, and `--seed 7` change it.
//! - `--light x,y,z`, `--camera-light`, `--no-shadow`, `--distance 8`, `--aim-y
//!   1.0`, `--pitch 7`, `--fov 36`, `--yaw 25`, `--smooth-rings 2`, `--attach
//!   path.glb:bone`, `--attach-hide a,b`, `--size 360x480`, `--backdrop
//!   0.1,0.1,0.12`, and `--exposure 1.3` configure the scene.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
	native::main()
}

/// Rendering offscreen needs a blocking device, which the browser lacks.
#[cfg(not(target_arch = "wasm32"))]
mod native {
	use std::collections::BTreeMap;
	use std::collections::HashMap;
	use std::sync::Arc;

	use leptoskit_3d::*;
	use serde::Deserialize;

	type Failure = Box<dyn std::error::Error>;

	#[derive(Deserialize, Default)]
	#[serde(default)]
	struct PaletteOverride {
		lit: Option<[f32; 3]>,
		shade: Option<[f32; 3]>,
		deep: Option<[f32; 3]>,
		spot: Option<[f32; 3]>,
		mode: Option<String>,
		hatch_min: Option<f32>,
		hatch_per_m: Option<f32>,
		cross_hatch: Option<bool>,
		specular: Option<f32>,
		rim: Option<bool>,
		ink_weight: Option<f32>,
		knockout: Option<bool>,
		boil_step: Option<u8>,
	}

	impl PaletteOverride {
		fn into_print_override(self) -> Result<PrintOverride, Failure> {
			let mode = match self.mode.as_deref() {
				None => None,
				Some("banded") => Some(PrintMode::Banded),
				Some("flat") => Some(PrintMode::Flat),
				Some(other) => return Err(format!("unknown print mode {other:?}").into()),
			};
			Ok(PrintOverride {
				lit: self.lit,
				shade: self.shade,
				deep: self.deep,
				spot: self.spot,
				mode,
				hatch_min: self.hatch_min,
				hatch_per_m: self.hatch_per_m,
				cross_hatch: self.cross_hatch,
				specular: self.specular,
				rim: self.rim,
				ink_weight: self.ink_weight,
				knockout: self.knockout,
				boil_step: self.boil_step,
			})
		}
	}

	#[derive(Deserialize, Default)]
	#[serde(default)]
	struct PaletteFile {
		all: Option<PaletteOverride>,
		materials: BTreeMap<String, PaletteOverride>,
		parts: BTreeMap<String, PaletteOverride>,
	}

	#[derive(Clone, Copy, PartialEq, Eq)]
	enum ShadingMode {
		Default,
		Toon,
		Print,
	}

	/// The command line: positional arguments, boolean flags, and `--name
	/// value` options.
	struct Arguments {
		positional: Vec<String>,
		options: HashMap<String, String>,
		shading: ShadingMode,
		camera_light: bool,
		no_shadow: bool,
	}

	impl Arguments {
		fn parse() -> Self {
			let mut arguments = std::env::args().skip(1);
			let mut parsed = Self {
				positional: Vec::new(),
				options: HashMap::new(),
				shading: ShadingMode::Default,
				camera_light: false,
				no_shadow: false,
			};

			while let Some(argument) = arguments.next() {
				match argument.strip_prefix("--") {
					Some("toon") => parsed.shading = ShadingMode::Toon,
					Some("print") => parsed.shading = ShadingMode::Print,
					Some("camera-light") => parsed.camera_light = true,
					Some("no-shadow") => parsed.no_shadow = true,
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

		fn integer<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>, Failure>
		where
			T::Err: std::fmt::Display,
		{
			self.options
				.get(name)
				.map(|value| value.parse::<T>())
				.transpose()
				.map_err(|error| format!("--{name}: {error}").into())
		}

		fn pair(&self, name: &str) -> Result<Option<[f32; 2]>, Failure> {
			let Some(value) = self.options.get(name) else {
				return Ok(None);
			};
			let parts = value
				.split(',')
				.map(str::parse::<f32>)
				.collect::<Result<Vec<_>, _>>()
				.map_err(|error| format!("--{name}: {error}"))?;
			let [first, second] = parts[..] else {
				return Err(format!("--{name} takes two numbers, as 2.5,4.0").into());
			};
			Ok(Some([first, second]))
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
			let shading = if self.shading == ShadingMode::Print {
				let mut print = Print::default();
				print.t_lit = self.number("t-lit")?.unwrap_or(print.t_lit);
				print.t_deep = self.number("t-deep")?.unwrap_or(print.t_deep);
				print.t_cross = self.number("t-cross")?.unwrap_or(print.t_cross);
				print.rim = self.number("rim")?.unwrap_or(print.rim);
				print.paper = self.color("paper")?.unwrap_or(print.paper);
				print.knockout_width = self.number("knockout")?.unwrap_or(print.knockout_width);
				print.hatch_per_m = self.number("hatch-per-m")?.unwrap_or(print.hatch_per_m);
				print.hatch_gain = self.number("hatch-gain")?.unwrap_or(print.hatch_gain);
				print.hatch_max = self.number("hatch-max")?.unwrap_or(print.hatch_max);
				print.cross_duty = self.number("cross-duty")?.unwrap_or(print.cross_duty);
				print.fade_px = self.pair("fade-px")?.unwrap_or(print.fade_px);
				print.side_heavy = self.number("side-heavy")?.unwrap_or(print.side_heavy);
				print.side_drop = self.number("side-drop")?.unwrap_or(print.side_drop);
				print.side_from = self.number("side-from")?.unwrap_or(print.side_from);
				print.side_to = self.number("side-to")?.unwrap_or(print.side_to);
				print.wobble_frequency = self
					.number("wobble-frequency")?
					.unwrap_or(print.wobble_frequency);
				Shading::Print(print)
			} else if self.shading == ShadingMode::Toon || self.options.contains_key("bands") {
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

					if let Some(seed) = self.integer::<u32>("seed")? {
						ink.seed = seed;
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

	fn apply_palette(world: &mut World, node: NodeId, palette: &[(PrintTarget, PrintOverride)]) {
		for (target, print) in palette {
			world.apply([Op::SetPrint {
				node,
				target: target.clone(),
				print: *print,
			}]);
		}
	}

	fn load_palette_ops(
		path: Option<&String>,
	) -> Result<Vec<(PrintTarget, PrintOverride)>, Failure> {
		let Some(path) = path else {
			return Ok(Vec::new());
		};
		let parsed: PaletteFile = serde_json::from_str(&std::fs::read_to_string(path)?)?;
		let mut ops = Vec::new();
		if let Some(all) = parsed.all {
			ops.push((PrintTarget::All, all.into_print_override()?));
		}
		for (material, entry) in parsed.materials {
			ops.push((
				PrintTarget::Material(material),
				entry.into_print_override()?,
			));
		}
		for (part, entry) in parsed.parts {
			ops.push((PrintTarget::Part(part), entry.into_print_override()?));
		}
		Ok(ops)
	}

	fn load_attachment(
		world: &mut World,
		arguments: &Arguments,
		load_options: LoadOptions,
		palette_ops: &[(PrintTarget, PrintOverride)],
	) -> Result<(), Failure> {
		let Some(attach_spec) = arguments.options.get("attach") else {
			return Ok(());
		};
		let (attach_path, bone) = attach_spec
			.rsplit_once(':')
			.ok_or("--attach takes path.glb:bone")?;
		let attached = Arc::new(Asset::from_slice_with_options(
			&std::fs::read(attach_path)?,
			load_options,
		)?);
		world.apply([
			Op::LoadAsset {
				asset: AssetId(2),
				path: "attach.glb".into(),
			},
			Op::Spawn {
				node: NodeId(2),
				asset: Some(AssetId(2)),
				parent: None,
			},
			Op::Attach {
				node: NodeId(2),
				target: NodeId(1),
				bone: bone.into(),
			},
		]);
		if arguments.no_shadow {
			world.apply([Op::SetShadow {
				node: NodeId(2),
				cast: false,
			}]);
		}
		world.take_loads();
		world.asset_loaded(AssetId(2), Ok(attached));
		apply_palette(world, NodeId(2), palette_ops);

		for part in arguments
			.options
			.get("attach-hide")
			.map(String::as_str)
			.unwrap_or_default()
			.split(',')
			.filter(|part| !part.is_empty())
		{
			world.apply([Op::SetPartVisible {
				node: NodeId(2),
				part: part.into(),
				visible: false,
			}]);
		}
		Ok(())
	}

	fn camera_from_args(arguments: &Arguments, distance: f32) -> Result<(Transform, f32), Failure> {
		let custom_camera = arguments.options.contains_key("aim-y")
			|| arguments.options.contains_key("pitch")
			|| arguments.options.contains_key("fov");
		if custom_camera {
			let aim_y = arguments.number("aim-y")?.unwrap_or(0.95);
			let pitch = arguments.number("pitch")?.unwrap_or(6.88).to_radians();
			let fov_y = arguments.number("fov")?.unwrap_or(42.97).to_radians();
			let position =
				glam::Vec3::new(0.0, aim_y + distance * pitch.sin(), distance * pitch.cos());
			Ok((
				Transform::from_translation(position.x, position.y, position.z)
					.with_rotation(glam::Quat::from_rotation_x(-pitch).to_array()),
				fov_y,
			))
		} else {
			// Frame the model's bounds, found by casting rays would be
			// overkill: a person-sized model sits in a 2.5 m box at
			// the origin. Further cameras back away along the same
			// line, and the default distance places the camera
			// exactly where it always was.
			let eye = glam::Vec3::new(0.0, 1.3, 3.6);
			let aim = glam::Vec3::new(0.0, 1.3 - 3.6 * 0.12_f32.tan(), 0.0);
			let position = eye + (eye - aim) * ((distance - 3.6) / 3.6);
			Ok((
				Transform::from_translation(position.x, position.y, position.z)
					.with_rotation(glam::Quat::from_rotation_x(-0.12).to_array()),
				0.75,
			))
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
		let load_options = LoadOptions {
			outline_smooth_rings: arguments.integer::<u8>("smooth-rings")?,
		};
		let palette_ops = load_palette_ops(arguments.options.get("palette"))?;
		let asset = Arc::new(Asset::from_slice_with_options(
			&std::fs::read(input)?,
			load_options,
		)?);
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

		if arguments.options.contains_key("light") || arguments.camera_light || arguments.no_shadow
		{
			let mut light = Light::default();
			if let Some(direction) = arguments.color("light")? {
				light.direction = direction;
			}
			light.camera_space = arguments.camera_light;
			if arguments.no_shadow {
				light.shadow = 0.0;
			}
			world.apply([Op::SetLight(light)]);
		}
		if arguments.no_shadow {
			world.apply([Op::SetShadow {
				node: NodeId(1),
				cast: false,
			}]);
		}
		if let Some(yaw_deg) = arguments.number("yaw")? {
			world.apply([Op::SetTransform {
				node: NodeId(1),
				transform: Transform::IDENTITY.with_yaw(yaw_deg.to_radians()),
			}]);
		}

		world.take_loads();
		world.asset_loaded(AssetId(1), Ok(asset.clone()));
		apply_palette(world, NodeId(1), &palette_ops);
		load_attachment(world, &arguments, load_options, &palette_ops)?;
		world.advance(0.0);

		let (cam_transform, fov_y) = camera_from_args(&arguments, distance)?;
		world.apply([Op::SetCamera {
			transform: cam_transform,
			fov_y,
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
