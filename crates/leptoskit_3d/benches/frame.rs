//! How long a frame takes the CPU with six animated characters: advancing
//! their clips, and building and encoding the frame. GPU time is not
//! counted: each sample starts with the GPU idle.
//!
//! The skinned characters are generated (10 240 vertices, 40 joints, two
//! morph targets, every joint animated). With `LEPTOSKIT_3D_MODELS` set to
//! a folder holding Reapers' `reaper_*.glb` files, it also measures six of
//! those (node-animated, morphing robes, 64 clips each), dressed as the game
//! shows them. Without a GPU adapter the frame benchmarks are skipped.
#![recursion_limit = "256"]
fn main() {
	#[cfg(not(target_arch = "wasm32"))]
	divan::main();
}

#[cfg(not(target_arch = "wasm32"))]
#[path = "../tests/support/fixture.rs"]
mod fixture;

/// The device the frames draw on blocks, which the browser cannot.
#[cfg(not(target_arch = "wasm32"))]
mod native {
	use std::sync::Arc;
	use std::sync::OnceLock;

	use leptoskit_3d::wgpu;
	use leptoskit_3d::*;

	use super::fixture;

	fn gpu() -> Option<&'static Gpu> {
		static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
		GPU.get_or_init(|| {
			Gpu::headless()
				.inspect_err(|error| eprintln!("skipping: {error}"))
				.ok()
		})
		.as_ref()
	}

	/// Six instances of `assets` (cycled) side by side, each playing `clip`
	/// with `hide` hidden.
	fn scene(assets: &[Arc<Asset>], clip: &str, hide: &[&str]) -> World {
		let mut world = World::new();
		let mut ops = vec![Op::SetCamera {
			transform: Transform::from_translation(0.0, 1.2, 8.0),
			fov_y: 0.9,
			near: 0.1,
			far: 100.0,
		}];
		for (index, _) in assets.iter().enumerate() {
			ops.push(Op::LoadAsset {
				asset: AssetId(index as u32),
				path: format!("models/{index}.glb"),
			});
		}
		for node in 0..6_u32 {
			let asset = AssetId(node % assets.len() as u32);
			ops.push(Op::Spawn {
				node: NodeId(node),
				asset: Some(asset),
				parent: None,
			});
			ops.push(Op::SetTransform {
				node: NodeId(node),
				// Two rows of three, all in view of a portrait phone screen.
				transform: Transform::from_translation(
					(node % 3) as f32 * 1.1 - 1.1,
					0.0,
					-((node / 3) as f32) * 2.0,
				),
			});
			for part in hide {
				ops.push(Op::SetPartVisible {
					node: NodeId(node),
					part: (*part).into(),
					visible: false,
				});
			}
			ops.push(Op::Play {
				node: NodeId(node),
				play: Play::new(clip),
			});
		}
		world.apply(ops);
		world.take_loads();
		for (index, asset) in assets.iter().enumerate() {
			world.asset_loaded(AssetId(index as u32), Ok(asset.clone()));
		}
		world.take_events();
		world.advance(0.0);
		world
	}

	fn skinned() -> World {
		let asset = Arc::new(Asset::from_slice(&fixture::character(40)).unwrap());
		scene(&[asset], "sway", &[])
	}

	fn reapers() -> Option<World> {
		let folder = std::env::var("LEPTOSKIT_3D_MODELS").ok()?;
		let mut assets = Vec::new();
		for name in [
			"chooser",
			"ferryman",
			"harvester",
			"hollow",
			"mourner",
			"weigher",
		] {
			let bytes = std::fs::read(format!("{folder}/reaper_{name}.glb")).ok()?;
			assets.push(Arc::new(Asset::from_slice(&bytes).ok()?));
		}
		let hide = [
			"humanoid_form",
			"head_1",
			"head_2",
			"mantle_1",
			"mantle_2",
			"robe_1",
			"robe_2",
			"robe_3",
		];
		Some(scene(&assets, "idle", &hide))
	}

	#[divan::bench]
	fn advance_six_skinned_characters(bencher: divan::Bencher<'_, '_>) {
		let mut world = skinned();
		bencher.bench_local(|| world.advance(1.0 / 60.0));
	}

	#[divan::bench]
	fn advance_six_reapers(bencher: divan::Bencher<'_, '_>) {
		let Some(mut world) = reapers() else {
			eprintln!("skipping: LEPTOSKIT_3D_MODELS is not set");
			return;
		};
		bencher.bench_local(|| world.advance(1.0 / 60.0));
	}

	fn frame(bencher: divan::Bencher<'_, '_>, world: World) {
		let Some(gpu) = gpu() else { return };
		let mut renderer = Renderer::with_world(gpu, world);
		let (width, height) = (1179, 2556);
		let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
			label: None,
			size: wgpu::Extent3d {
				width,
				height,
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Bgra8Unorm,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			view_formats: &[],
		});
		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		let target = RenderTarget::new(&view, wgpu::TextureFormat::Bgra8Unorm, width, height);
		renderer.render(&target).unwrap();
		bencher
			.with_inputs(|| {
				gpu.device()
					.poll(wgpu::PollType::wait_indefinitely())
					.unwrap();
			})
			.bench_local_values(|()| {
				renderer.advance(1.0 / 60.0);
				renderer.render(&target).unwrap()
			});
		let stats = renderer.stats();
		eprintln!("{stats:?}");
	}

	#[divan::bench]
	fn frame_six_skinned_characters(bencher: divan::Bencher<'_, '_>) {
		frame(bencher, skinned());
	}

	#[divan::bench]
	fn frame_six_reapers(bencher: divan::Bencher<'_, '_>) {
		let Some(world) = reapers() else {
			eprintln!("skipping: LEPTOSKIT_3D_MODELS is not set");
			return;
		};
		frame(bencher, world);
	}

	#[divan::bench]
	fn load_a_reaper(bencher: divan::Bencher<'_, '_>) {
		let Some(bytes) = std::env::var("LEPTOSKIT_3D_MODELS")
			.ok()
			.and_then(|folder| std::fs::read(format!("{folder}/reaper_chooser.glb")).ok())
		else {
			eprintln!("skipping: LEPTOSKIT_3D_MODELS is not set");
			return;
		};
		bencher.bench_local(|| Asset::from_slice(&bytes).unwrap());
	}
}
