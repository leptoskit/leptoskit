//! Renders a glTF model to a PNG, for looking at what the renderer draws.
//!
//! ```sh
//! cargo run -p leptoskit_3d --example render_model -- model.glb out.png [clip] [seconds]
//! ```
//!
//! `HIDE=head_1,head_2` hides the named parts, as an app choosing between a
//! model's variants does.
//!
//! The camera frames the model from the front and a little above, over a
//! light grey background, with the clip (if any) played for `seconds`.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
	native::main()
}

/// Rendering offscreen needs a blocking device, which the browser lacks.
#[cfg(not(target_arch = "wasm32"))]
mod native {
	use std::sync::Arc;

	use leptoskit_3d::*;

	pub fn main() -> Result<(), Box<dyn std::error::Error>> {
		let mut arguments = std::env::args().skip(1);
		let (Some(input), Some(output)) = (arguments.next(), arguments.next()) else {
			eprintln!("usage: render_model model.glb out.png [clip] [seconds]");
			std::process::exit(2);
		};
		let clip = arguments.next();
		let seconds: f32 = arguments.next().map_or(Ok(0.5), |value| value.parse())?;
		let asset = Arc::new(Asset::from_slice(&std::fs::read(&input)?)?);
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
			Op::SetBackdrop(Backdrop::Color([0.8, 0.8, 0.82, 1.0])),
		]);
		world.take_loads();
		world.asset_loaded(AssetId(1), Ok(asset.clone()));
		world.advance(0.0);
		// Frame the model's bounds, found by casting rays would be overkill:
		// a person-sized model sits in a 2.5 m box at the origin.
		world.apply([Op::SetCamera {
			transform: Transform::from_translation(0.0, 1.3, 3.6)
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

		if let Some(clip) = &clip {
			world.apply([Op::Play {
				node: NodeId(1),
				play: Play::new(clip.clone()),
			}]);
		}

		let steps = (seconds * 60.0) as usize;

		for _ in 0..steps.max(1) {
			renderer.advance(1.0 / 60.0);
		}

		let (width, height) = (720, 960);
		let pixels = renderer.render_to_rgba(width, height)?;
		let file = std::fs::File::create(&output)?;
		let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
		encoder.set_color(png::ColorType::Rgba);
		encoder.set_depth(png::BitDepth::Eight);
		encoder.write_header()?.write_image_data(&pixels)?;
		let stats = renderer.stats();

		println!(
			"{output}: {} draws, {} instances, {} triangles, clips {:?}",
			stats.draws,
			stats.instances,
			stats.triangles,
			asset.clip_names().take(8).collect::<Vec<_>>()
		);
		Ok(())
	}
}
