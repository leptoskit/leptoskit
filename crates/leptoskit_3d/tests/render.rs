//! Renders small scenes on the GPU and checks their pixels. Machines with no
//! GPU adapter (some CI runners) skip these tests.
#![cfg(all(feature = "renderer", not(target_arch = "wasm32")))]
#![recursion_limit = "256"]

mod support;

use std::sync::Arc;
use std::sync::OnceLock;

use leptoskit_3d::glam::Mat3;
use leptoskit_3d::glam::Mat4;
use leptoskit_3d::glam::Vec3;
use leptoskit_3d::*;
use support::fixture;

const SIZE: u32 = 64;

/// One device for every test, as an app has one per process.
fn gpu() -> Option<&'static Gpu> {
	static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
	GPU.get_or_init(|| {
		match Gpu::headless() {
			Ok(gpu) => Some(gpu),
			Err(error) if error.is_unavailable() => {
				eprintln!("skipping: {error}");
				None
			}

			Err(error) => panic!("{error}"),
		}
	})
	.as_ref()
}

struct Image {
	pixels: Vec<u8>,
	width: u32,
}

impl Image {
	fn at(&self, x: u32, y: u32) -> [u8; 4] {
		let i = ((y * self.width + x) * 4) as usize;
		[
			self.pixels[i],
			self.pixels[i + 1],
			self.pixels[i + 2],
			self.pixels[i + 3],
		]
	}
}

fn near(actual: [u8; 4], expected: [u8; 4], tolerance: u8) -> bool {
	actual
		.iter()
		.zip(expected)
		.all(|(a, e)| a.abs_diff(e) <= tolerance)
}

/// What the shader writes for a linear colour on an 8-bit target without
/// sRGB encoding: Khronos PBR Neutral, then sRGB.
fn displayed(color: [f32; 3]) -> [u8; 4] {
	let x = color[0].min(color[1]).min(color[2]);
	let offset = if x < 0.08 { x - 6.25 * x * x } else { 0.04 };
	let mut mapped = color.map(|value| value - offset);
	let peak = mapped[0].max(mapped[1]).max(mapped[2]);

	if peak >= 0.76 {
		let d = 1.0 - 0.76;
		let new_peak = 1.0 - d * d / (peak + d - 0.76);
		mapped = mapped.map(|value| value * new_peak / peak);

		let g = 1.0 - 1.0 / (0.15 * (peak - new_peak) + 1.0);
		mapped = mapped.map(|value| value + (new_peak - value) * g);
	}

	let encode = |value: f32| {
		let value = value.clamp(0.0, 1.0);
		let srgb = if value <= 0.003_130_8 {
			value * 12.92
		} else {
			1.055 * value.powf(1.0 / 2.4) - 0.055
		};

		(srgb * 255.0).round() as u8
	};
	[encode(mapped[0]), encode(mapped[1]), encode(mapped[2]), 255]
}

/// A renderer looking at the origin from 3 m along +z.
fn renderer(gpu: &Gpu) -> Renderer {
	let mut renderer = Renderer::new(gpu);
	renderer.world_mut().apply([Op::SetCamera {
		transform: Transform::from_translation(0.0, 0.0, 3.0),
		fov_y: 0.8,
		near: 0.1,
		far: 50.0,
	}]);
	renderer
}

/// Loads `bytes` as asset `id` and spawns it as `node`.
fn spawn(renderer: &mut Renderer, id: u32, node: u32, bytes: &[u8]) -> Arc<Asset> {
	let asset = Arc::new(Asset::from_slice(bytes).unwrap());
	let world = renderer.world_mut();
	world.apply([
		Op::LoadAsset {
			asset: AssetId(id),
			path: format!("models/{id}.glb"),
		},
		Op::Spawn {
			node: NodeId(node),
			asset: Some(AssetId(id)),
			parent: None,
		},
	]);
	world.take_loads();
	world.asset_loaded(AssetId(id), Ok(asset.clone()));
	world.take_events();
	asset
}

fn draw(renderer: &mut Renderer) -> Image {
	renderer.advance(0.0);
	let pixels = renderer.render_to_rgba(SIZE, SIZE).unwrap();
	Image {
		pixels,
		width: SIZE,
	}
}

#[test]
fn an_unlit_cube_draws_its_colour_and_nothing_else() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(
		&mut renderer,
		1,
		1,
		&fixture::unlit_cube([0.5, 0.25, 0.1, 1.0], false),
	);
	let image = draw(&mut renderer);
	let expected = displayed([0.5, 0.25, 0.1]);
	assert!(
		near(image.at(32, 32), expected, 2),
		"{:?} vs {expected:?}",
		image.at(32, 32)
	);
	assert_eq!(image.at(1, 1), [0; 4], "the background is transparent");
	let stats = renderer.stats();

	assert_eq!((stats.draws, stats.instances, stats.triangles), (1, 1, 12));
}

#[test]
fn the_backdrop_colour_clears_the_target() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	renderer
		.world_mut()
		.apply([Op::SetBackdrop(Backdrop::Color([0.0, 0.0, 1.0, 1.0]))]);
	let image = draw(&mut renderer);
	assert_eq!(image.at(5, 5), [0, 0, 255, 255]);
	// The camera backdrop with no camera image is transparent.
	renderer
		.world_mut()
		.apply([Op::SetBackdrop(Backdrop::Camera)]);
	assert_eq!(draw(&mut renderer).at(5, 5), [0; 4]);
}

#[test]
fn lit_faces_turned_to_the_light_are_brighter() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::lit_cube());
	renderer.world_mut().apply([
		Op::SetCamera {
			transform: Transform::from_translation(0.0, 2.0, 2.5)
				.with_rotation(glam::Quat::from_rotation_x(-0.67).to_array()),
			fov_y: 0.8,
			near: 0.1,
			far: 50.0,
		},
		Op::SetLight(Light {
			direction: [0.0, -1.0, 0.0],
			shadow: 0.0,
			..Light::default()
		}),
	]);
	let image = draw(&mut renderer);
	// The top face fills the upper middle of the view; the front face the
	// lower middle.
	let top = image.at(32, 20);
	let front = image.at(32, 46);
	assert_eq!(top[3], 255);

	assert_eq!(front[3], 255);
	assert!(top[0] > front[0] + 40, "top {top:?} front {front:?}");
	assert!(front[0] > 10, "the environment lights the front: {front:?}");
}

#[test]
fn blended_and_faded_parts_are_translucent() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(
		&mut renderer,
		1,
		1,
		&fixture::unlit_cube([1.0, 1.0, 1.0, 0.5], true),
	);
	let alpha = draw(&mut renderer).at(32, 32)[3];
	assert!((120..=136).contains(&alpha), "{alpha}");

	let mut renderer = self::renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::unlit_cube([1.0; 4], false));
	renderer.world_mut().apply([Op::SetMaterial {
		node: NodeId(1),
		part: None,
		material: MaterialOverride {
			opacity: Some(0.25),
			..MaterialOverride::default()
		},
	}]);
	let alpha = draw(&mut renderer).at(32, 32)[3];
	assert!((56..=72).contains(&alpha), "{alpha}");
}

#[test]
fn material_overrides_tint_glow_and_rim() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::unlit_cube([1.0; 4], false));
	renderer.world_mut().apply([Op::SetMaterial {
		node: NodeId(1),
		part: Some("cube".into()),
		material: MaterialOverride {
			color: Some([0.0, 0.5, 0.0, 1.0]),
			..MaterialOverride::default()
		},
	}]);
	let image = draw(&mut renderer);
	assert!(
		near(image.at(32, 32), displayed([0.0, 0.5, 0.0]), 2),
		"{:?}",
		image.at(32, 32)
	);
	renderer.world_mut().apply([Op::SetMaterial {
		node: NodeId(1),
		part: None,
		material: MaterialOverride {
			emissive: Some([0.0, 0.0, 1.0]),
			emissive_strength: Some(0.5),
			..MaterialOverride::default()
		},
	}]);
	let glowing = draw(&mut renderer).at(32, 32);

	assert!(near(glowing, displayed([0.0, 0.5, 0.5]), 2), "{glowing:?}");
}

#[test]
fn hidden_parts_and_removed_nodes_are_not_drawn() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::unlit_cube([1.0; 4], false));
	renderer.world_mut().apply([Op::SetPartVisible {
		node: NodeId(1),
		part: "cube".into(),
		visible: false,
	}]);
	assert_eq!(draw(&mut renderer).at(32, 32), [0; 4]);
	renderer.world_mut().apply([
		Op::SetPartVisible {
			node: NodeId(1),
			part: "cube".into(),
			visible: true,
		},
		Op::Remove { node: NodeId(1) },
	]);
	assert_eq!(draw(&mut renderer).at(32, 32), [0; 4]);
	assert_eq!(renderer.stats().draws, 0);
}

#[test]
fn repeated_static_meshes_draw_instanced() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 0, &fixture::unlit_cube([1.0; 4], false));
	let mut ops = Vec::new();

	for index in 1..=99_u32 {
		ops.push(Op::Spawn {
			node: NodeId(index),
			asset: Some(AssetId(1)),
			parent: None,
		});
		ops.push(Op::SetTransform {
			node: NodeId(index),
			transform: Transform::from_translation(
				(index % 10) as f32 * 0.2 - 1.0,
				(index / 10) as f32 * 0.2 - 1.0,
				-2.0,
			)
			.with_scale(0.1),
		});
	}

	renderer.world_mut().apply(ops);

	draw(&mut renderer);
	let stats = renderer.stats();
	assert_eq!(stats.instances, 100);
	assert_eq!(stats.draws, 1, "{stats:?}");
}

#[test]
fn what_no_view_sees_is_culled() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::unlit_cube([1.0; 4], false));
	renderer.world_mut().apply([Op::SetTransform {
		node: NodeId(1),
		transform: Transform::from_translation(0.0, 0.0, 10.0),
	}]);
	draw(&mut renderer);
	let stats = renderer.stats();
	assert_eq!((stats.draws, stats.culled), (0, 1));
}

#[test]
fn skinned_meshes_follow_their_animation() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::rig());
	renderer.world_mut().apply([
		Op::SetCamera {
			transform: Transform::from_translation(0.0, 1.0, 4.0),
			fov_y: 0.8,
			near: 0.1,
			far: 50.0,
		},
		Op::SetLight(Light {
			shadow: 0.0,
			..Light::default()
		}),
	]);
	// (-0.65, 0.8) is left of the body at rest and inside it once the arm
	// has turned its top 90° to the left.
	let probe = |renderer: &Renderer| {
		let (view, projection) = (
			glam::camera::rh::view::look_to_mat4(Vec3::new(0.0, 1.0, 4.0), Vec3::NEG_Z, Vec3::Y),
			glam::camera::rh::proj::directx::perspective(0.8, 1.0, 0.1, 50.0),
		);
		let _ = renderer;
		let clip = projection * view * Vec3::new(-0.65, 0.8, 0.0).extend(1.0);
		let ndc = clip.truncate() / clip.w;
		(
			((ndc.x * 0.5 + 0.5) * SIZE as f32) as u32,
			((0.5 - ndc.y * 0.5) * SIZE as f32) as u32,
		)
	};
	let (x, y) = probe(&renderer);
	let image = draw(&mut renderer);
	assert_eq!(image.at(x, y)[3], 0, "nothing there at rest");

	assert_eq!(image.at(32, 40)[3], 255, "the body is drawn");
	renderer.world_mut().apply([Op::Play {
		node: NodeId(1),
		play: Play::new("raise").once(),
	}]);

	for _ in 0..70 {
		renderer.advance(1.0 / 60.0);
	}

	let pixels = renderer.render_to_rgba(SIZE, SIZE).unwrap();
	let image = Image {
		pixels,
		width: SIZE,
	};
	assert_eq!(image.at(x, y)[3], 255, "the raised half is there");
	assert!(
		image.at(x, y)[0] > image.at(x, y)[2],
		"it is the red body: {:?}",
		image.at(x, y)
	);
}

#[test]
fn morph_targets_reshape_meshes() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::rig());
	renderer.world_mut().apply([
		Op::SetCamera {
			transform: Transform::from_translation(0.0, 1.8, 2.0),
			fov_y: 0.8,
			near: 0.1,
			far: 50.0,
		},
		Op::SetPartVisible {
			node: NodeId(1),
			part: "body".into(),
			visible: false,
		},
	]);
	// The head is 0.4 m wide: at 2 m it spans about 12 of 64 pixels; grown,
	// about 18.
	let width = |image: &Image| (0..SIZE).filter(|x| image.at(*x, 32)[3] > 0).count();
	let rest = width(&draw(&mut renderer));
	renderer.world_mut().apply([Op::SetMorphWeight {
		node: NodeId(1),
		part: None,
		target: "grow".into(),
		weight: 1.0,
	}]);
	let grown = width(&draw(&mut renderer));

	assert!(
		grown as f32 > rest as f32 * 1.35,
		"rest {rest} grown {grown}"
	);
	renderer.world_mut().apply([Op::SetMorphWeight {
		node: NodeId(1),
		part: None,
		target: "grow".into(),
		weight: 0.0,
	}]);
	assert_eq!(width(&draw(&mut renderer)), rest);
}

#[test]
fn models_cast_a_soft_shadow_on_their_ground() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::unlit_cube([1.0; 4], false));
	renderer.world_mut().apply([
		Op::SetTransform {
			node: NodeId(1),
			transform: Transform::from_translation(0.0, 0.5, 0.0),
		},
		// Far above, looking straight down, with a narrow view: nearly
		// orthographic, so the shadow on the ground shows around the cube.
		Op::SetCamera {
			transform: Transform::from_translation(0.0, 10.0, 0.0).with_rotation(
				glam::Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2).to_array(),
			),
			fov_y: 0.3,
			near: 0.1,
			far: 50.0,
		},
	]);
	let image = draw(&mut renderer);
	assert_eq!(image.at(32, 32)[3], 255);
	let edge = (0..32)
		.map(|x| image.at(x, 32))
		.find(|pixel| pixel[3] > 0 && pixel[3] < 255);
	assert!(
		edge.is_some_and(|pixel| pixel[0] == 0),
		"a dark, soft edge: {edge:?}"
	);

	assert_eq!(renderer.stats().shadows, 1);
	renderer.world_mut().apply([Op::SetShadow {
		node: NodeId(1),
		cast: false,
	}]);
	draw(&mut renderer);
	assert_eq!(renderer.stats().shadows, 0);
}

fn solid_texture(gpu: &Gpu, format: wgpu::TextureFormat, texel: &[u8]) -> wgpu::TextureView {
	let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
		label: None,
		size: wgpu::Extent3d {
			width: 4,
			height: 4,
			depth_or_array_layers: 1,
		},
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	});
	let bytes = texel.repeat(16);
	gpu.queue().write_texture(
		texture.as_image_copy(),
		&bytes,
		wgpu::TexelCopyBufferLayout {
			offset: 0,
			bytes_per_row: Some(texel.len() as u32 * 4),
			rows_per_image: None,
		},
		texture.size(),
	);
	texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// Renders into a fresh target with these views, background, and
/// occlusion.
fn render_with(
	renderer: &mut Renderer,
	views: &[ViewParams],
	background: Background<'_>,
	occlusion: Option<Occlusion<'_>>,
) -> Image {
	renderer.advance(0.0);
	let texture = renderer.offscreen(SIZE, SIZE).unwrap();
	let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
	let target = RenderTarget {
		views,
		background,
		occlusion,
		..RenderTarget::new(&view, wgpu::TextureFormat::Rgba8Unorm, SIZE, SIZE)
	};
	renderer.render(&target).unwrap();
	Image {
		pixels: renderer.read_back(&texture).unwrap(),
		width: SIZE,
	}
}

#[test]
fn camera_images_draw_behind_the_scene() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(
		&mut renderer,
		1,
		1,
		&fixture::unlit_cube([0.5, 0.25, 0.1, 1.0], false),
	);
	let rgba = solid_texture(gpu, wgpu::TextureFormat::Rgba8UnormSrgb, &[0, 255, 0, 255]);
	let background = Background::Camera(CameraImage {
		planes: CameraPlanes::Rgba(&rgba),
		transform: Mat3::IDENTITY,
	});
	let image = render_with(&mut renderer, &[], background, None);
	assert!(
		near(image.at(2, 2), [0, 255, 0, 255], 1),
		"{:?}",
		image.at(2, 2)
	);
	assert!(
		near(image.at(32, 32), displayed([0.5, 0.25, 0.1]), 2),
		"the scene draws over it"
	);

	// Luma 0.5 with neutral chroma is mid grey in sRGB.
	let y = solid_texture(gpu, wgpu::TextureFormat::R8Unorm, &[128]);
	let cbcr = solid_texture(gpu, wgpu::TextureFormat::Rg8Unorm, &[128, 128]);
	let background = Background::Camera(CameraImage {
		planes: CameraPlanes::YCbCr { y: &y, cbcr: &cbcr },
		transform: Mat3::IDENTITY,
	});
	let image = render_with(&mut renderer, &[], background, None);
	assert!(
		near(image.at(2, 2), [128, 128, 128, 255], 2),
		"{:?}",
		image.at(2, 2)
	);
}

#[test]
fn real_world_depth_hides_what_is_behind_it() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::unlit_cube([1.0; 4], false));
	// The cube's front face is 2.5 m from the camera.
	let near_wall = solid_texture(gpu, wgpu::TextureFormat::R32Float, &1.0_f32.to_le_bytes());
	let far_wall = solid_texture(gpu, wgpu::TextureFormat::R32Float, &10.0_f32.to_le_bytes());
	let unknown = solid_texture(gpu, wgpu::TextureFormat::R32Float, &0.0_f32.to_le_bytes());

	for (depth, visible) in [(&near_wall, false), (&far_wall, true), (&unknown, true)] {
		let occlusion = Occlusion {
			depth,
			transform: Mat3::IDENTITY,
			scale: 1.0,
			tolerance: 0.05,
		};

		let image = render_with(&mut renderer, &[], Background::Scene, Some(occlusion));
		assert_eq!(image.at(32, 32)[3] == 255, visible);
	}
}

#[test]
fn stereo_views_draw_side_by_side_or_into_layers() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::unlit_cube([1.0; 4], false));
	renderer.world_mut().apply([Op::SetTransform {
		node: NodeId(1),
		transform: Transform::IDENTITY.with_scale(0.4),
	}]);
	let projection = glam::camera::rh::proj::directx::perspective(0.8, 0.5, 0.1, 50.0);
	let eye = |x: f32, viewport: [f32; 4]| {
		ViewParams {
			view: glam::camera::rh::view::look_to_mat4(
				Vec3::new(x, 0.0, 3.0),
				Vec3::NEG_Z,
				Vec3::Y,
			),
			projection,
			viewport,
		}
	};
	let views = [
		eye(-0.03, [0.0, 0.0, 32.0, 64.0]),
		eye(0.03, [32.0, 0.0, 32.0, 64.0]),
	];
	let image = render_with(&mut renderer, &views, Background::Scene, None);

	assert_eq!(image.at(16, 32)[3], 255, "the left eye");
	assert_eq!(image.at(48, 32)[3], 255, "the right eye");
	assert_eq!(image.at(31, 32)[3], 0, "between them");
	assert_eq!(renderer.stats().draws, 1);

	let left = renderer.offscreen(32, 64).unwrap();
	let right = renderer.offscreen(32, 64).unwrap();
	let views_of = [
		left.create_view(&wgpu::TextureViewDescriptor::default()),
		right.create_view(&wgpu::TextureViewDescriptor::default()),
	];
	let layers = [&views_of[0], &views_of[1]];
	let full = [
		eye(-0.03, [0.0, 0.0, 32.0, 64.0]),
		eye(0.03, [0.0, 0.0, 32.0, 64.0]),
	];
	let target = RenderTarget {
		color: TargetColor::Layers(&layers),
		format: wgpu::TextureFormat::Rgba8Unorm,
		size: [32, 64],
		views: &full,
		background: Background::Color([1.0, 0.0, 0.0, 1.0]),
		occlusion: None,
	};
	renderer.render(&target).unwrap();

	for texture in [&left, &right] {
		let pixels = renderer.read_back(texture).unwrap();
		let image = Image { pixels, width: 32 };
		assert_eq!(image.at(16, 32)[3], 255);
		assert_eq!(image.at(0, 0), [255, 0, 0, 255]);
	}

	let mismatched = RenderTarget {
		views: &full[..1],
		..target
	};
	assert!(matches!(
		renderer.render(&mismatched),
		Err(Error::Target(_))
	));
}

#[test]
fn srgb_targets_show_the_same_colours() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(
		&mut renderer,
		1,
		1,
		&fixture::unlit_cube([0.5, 0.25, 0.1, 1.0], false),
	);
	renderer.advance(0.0);
	let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
		label: None,
		size: wgpu::Extent3d {
			width: SIZE,
			height: SIZE,
			depth_or_array_layers: 1,
		},
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: wgpu::TextureFormat::Bgra8UnormSrgb,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
		view_formats: &[],
	});
	let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
	renderer
		.render(&RenderTarget::new(
			&view,
			wgpu::TextureFormat::Bgra8UnormSrgb,
			SIZE,
			SIZE,
		))
		.unwrap();
	let image = Image {
		pixels: renderer.read_back(&texture).unwrap(),
		width: SIZE,
	};

	assert!(
		near(image.at(32, 32), displayed([0.5, 0.25, 0.1]), 2),
		"{:?}",
		image.at(32, 32)
	);
}

#[test]
fn renderers_on_one_device_share_uploaded_assets() {
	// A device of its own, so other tests' uploads do not count.
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,

		Err(error) if error.is_unavailable() => return eprintln!("skipping: {error}"),
		Err(error) => panic!("{error}"),
	};

	let gpu = &gpu;
	let asset = Arc::new(Asset::from_slice(&fixture::rig()).unwrap());
	let mut first = renderer(gpu);
	let mut second = renderer(gpu);

	for renderer in [&mut first, &mut second] {
		let world = renderer.world_mut();
		world.apply([
			Op::LoadAsset {
				asset: AssetId(1),
				path: "models/rig.glb".into(),
			},
			Op::Spawn {
				node: NodeId(1),
				asset: Some(AssetId(1)),
				parent: None,
			},
		]);

		world.take_loads();
		world.asset_loaded(AssetId(1), Ok(asset.clone()));
		draw(renderer);
	}

	assert_eq!(gpu.uploaded_assets(), 1, "both scenes draw one copy");
	draw(&mut first);
	draw(&mut second);
	assert_eq!(gpu.uploaded_assets(), 1, "drawing again uploads nothing");
	drop((first, second, asset));
	assert_eq!(gpu.uploaded_assets(), 0, "a copy no scene holds is let go");
}

#[test]
fn degenerate_targets_are_refused_or_skipped() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	let texture = renderer.offscreen(4, 4).unwrap();
	let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
	let empty = RenderTarget::new(&view, wgpu::TextureFormat::Rgba8Unorm, 0, 4);
	assert_eq!(renderer.render(&empty).unwrap(), FrameStats::default());
	let huge = RenderTarget::new(&view, wgpu::TextureFormat::Rgba8Unorm, 1 << 20, 4);
	assert!(matches!(
		renderer.render(&huge),
		Err(Error::TooLarge { .. })
	));

	let views = [ViewParams {
		view: Mat4::IDENTITY,
		projection: Mat4::IDENTITY,
		viewport: [0.0, 0.0, 4.0, 4.0],
	}; MAX_VIEWS + 1];
	let crowded = RenderTarget {
		views: &views,
		..RenderTarget::new(&view, wgpu::TextureFormat::Rgba8Unorm, 4, 4)
	};
	assert!(matches!(renderer.render(&crowded), Err(Error::Target(_))));
}
