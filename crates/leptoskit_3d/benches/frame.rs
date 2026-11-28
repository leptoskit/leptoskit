//! How long a frame takes the CPU with six animated characters: advancing
//! their clips, and building and encoding the frame. GPU time is not
//! counted: each sample starts with the GPU idle.
//!
//! The skinned characters are generated (10 240 vertices, 40 joints, two
//! morph targets, every joint animated). With `LEPTOSKIT_3D_MODELS` set to
//! a folder holding Reapers' `reaper_*.glb` files, it also measures six of
//! those (node-animated, morphing robes, 64 clips each), dressed as the game
//! shows them, plain and in the hand-drawn style, and one harvester with
//! every part shown (about 140 000 triangles) drawn and waited for, so its
//! time is the GPU's too, with and without ink: the ink line's cost.
//! Without a GPU adapter the frame benchmarks are skipped.
//!
//! ```sh
//! LEPTOSKIT_3D_MODELS=path/to/models cargo bench -p leptoskit_3d --bench frame
//! ```
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

	/// A phone-sized target: an iPhone 15's screen.
	fn target_texture(gpu: &Gpu) -> wgpu::Texture {
		gpu.device().create_texture(&wgpu::TextureDescriptor {
			label: None,
			size: wgpu::Extent3d {
				width: WIDTH,
				height: HEIGHT,
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Bgra8Unorm,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			view_formats: &[],
		})
	}

	const WIDTH: u32 = 1179;
	const HEIGHT: u32 = 2556;

	fn frame(bencher: divan::Bencher<'_, '_>, world: World) {
		let Some(gpu) = gpu() else { return };
		let mut renderer = Renderer::with_world(gpu, world);
		let texture = target_texture(gpu);
		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		let target = RenderTarget::new(&view, wgpu::TextureFormat::Bgra8Unorm, WIDTH, HEIGHT);
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
	fn frame_six_reapers_hand_drawn(bencher: divan::Bencher<'_, '_>) {
		let Some(mut world) = reapers() else {
			eprintln!("skipping: LEPTOSKIT_3D_MODELS is not set");

			return;
		};
		world.apply([Op::SetStyle(Style::hand_drawn())]);
		frame(bencher, world);
	}

	/// One reaper with every part shown, playing its idle clip, drawn and
	/// waited for, `inked` or not.
	#[divan::bench(args = [false, true])]
	fn frame_one_reaper_on_the_gpu(bencher: divan::Bencher<'_, '_>, inked: bool) {
		let Some(gpu) = gpu() else { return };
		let Some(bytes) = std::env::var("LEPTOSKIT_3D_MODELS")
			.ok()
			.and_then(|folder| std::fs::read(format!("{folder}/reaper_harvester.glb")).ok())
		else {
			eprintln!("skipping: LEPTOSKIT_3D_MODELS is not set");

			return;
		};
		let asset = Arc::new(Asset::from_slice(&bytes).unwrap());
		let mut world = scene(&[asset], "idle", &[]);
		// Only the first of the six, close up.
		world.apply((1..6).map(|node| Op::Remove { node: NodeId(node) }));
		world.apply([
			Op::SetTransform {
				node: NodeId(0),
				transform: Transform::IDENTITY,
			},
			Op::SetCamera {
				transform: Transform::from_translation(0.0, 1.1, 3.2),
				fov_y: 0.8,
				near: 0.1,
				far: 100.0,
			},
		]);

		if inked {
			world.apply([Op::SetStyle(Style::hand_drawn())]);
		}

		let mut renderer = Renderer::with_world(gpu, world);
		let texture = target_texture(gpu);
		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		let target = RenderTarget::new(&view, wgpu::TextureFormat::Bgra8Unorm, WIDTH, HEIGHT);
		renderer.render(&target).unwrap();
		let wait = || {
			gpu.device()
				.poll(wgpu::PollType::wait_indefinitely())
				.unwrap();
		};
		wait();
		bencher.bench_local(|| {
			renderer.advance(1.0 / 60.0);
			let stats = renderer.render(&target).unwrap();
			wait();
			stats
		});

		let stats = renderer.stats();
		eprintln!("{stats:?}");
	}

	/// One reaper with every part shown, playing its idle clip, drawn and
	/// waited for in relief-print mode, with stencil paper `knockout` off or
	/// on.
	#[divan::bench(args = [false, true])]
	fn frame_one_reaper_print_on_the_gpu(bencher: divan::Bencher<'_, '_>, knockout: bool) {
		let Some(gpu) = gpu() else { return };
		let Some(bytes) = std::env::var("LEPTOSKIT_3D_MODELS")
			.ok()
			.and_then(|folder| std::fs::read(format!("{folder}/reaper_harvester.glb")).ok())
		else {
			eprintln!("skipping: LEPTOSKIT_3D_MODELS is not set");

			return;
		};
		let asset = Arc::new(
			Asset::from_slice_with_options(
				&bytes,
				LoadOptions {
					outline_smooth_rings: Some(2),
				},
			)
			.unwrap(),
		);
		let mut world = scene(&[asset], "idle", &[]);
		world.apply((1..6).map(|node| Op::Remove { node: NodeId(node) }));
		let mut print = Print::default();
		if !knockout {
			print.knockout_width = 0.0;
		}
		world.apply([
			Op::SetTransform {
				node: NodeId(0),
				transform: Transform::IDENTITY,
			},
			Op::SetCamera {
				transform: Transform::from_translation(0.0, 1.1, 3.2),
				fov_y: 0.8,
				near: 0.1,
				far: 100.0,
			},
			Op::SetLight(Light {
				direction: [0.86, -0.40, -0.32],
				camera_space: true,
				shadow: 0.0,
				..Light::default()
			}),
			Op::SetStyle(Style {
				shading: Shading::Print(print),
				ink: Some(Ink::default()),
			}),
			Op::SetPrint {
				node: NodeId(0),
				target: PrintTarget::All,
				print: PrintOverride {
					lit: Some([0.45, 0.36, 0.10]),
					shade: Some([0.16, 0.12, 0.04]),
					deep: Some([0.04, 0.03, 0.02]),
					spot: Some([0.90, 0.62, 0.08]),
					hatch_min: Some(0.24),
					cross_hatch: Some(true),
					rim: Some(true),
					..PrintOverride::default()
				},
			},
		]);

		let mut renderer = Renderer::with_world(gpu, world);
		let texture = target_texture(gpu);
		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		let target = RenderTarget::new(&view, wgpu::TextureFormat::Bgra8Unorm, WIDTH, HEIGHT);
		renderer.render(&target).unwrap();
		let wait = || {
			gpu.device()
				.poll(wgpu::PollType::wait_indefinitely())
				.unwrap();
		};
		wait();
		bencher.bench_local(|| {
			renderer.advance(1.0 / 60.0);
			let stats = renderer.render(&target).unwrap();
			wait();
			stats
		});

		let stats = renderer.stats();
		eprintln!("{stats:?}");
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
