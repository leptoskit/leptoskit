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
fn additive_parts_add_their_light_to_what_is_behind() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	renderer
		.world_mut()
		.apply([Op::SetBackdrop(Backdrop::Color([0.0, 0.0, 0.5, 1.0]))]);
	spawn(
		&mut renderer,
		1,
		1,
		&fixture::unlit_cube([0.5, 0.0, 0.0, 1.0], false),
	);
	let backdrop = draw(&mut renderer).at(2, 2);
	// Drawn normally, the red cube covers the blue.
	let covered = draw(&mut renderer).at(32, 32);
	assert!(covered[2] < 8, "{covered:?}");

	renderer.world_mut().apply([Op::SetBlend {
		node: NodeId(1),
		part: None,
		blend: Blend::Additive,
	}]);
	let added = draw(&mut renderer).at(32, 32);
	assert!(
		near(added, [covered[0], 0, backdrop[2], 255], 2),
		"{added:?}"
	);

	// Its opacity scales the light it adds.
	renderer.world_mut().apply([Op::SetMaterial {
		node: NodeId(1),
		part: None,
		material: MaterialOverride {
			opacity: Some(0.5),
			..MaterialOverride::default()
		},
	}]);
	let dimmer = draw(&mut renderer).at(32, 32);
	assert!(dimmer[0] < added[0] && dimmer[0] > 8, "{dimmer:?}");
	assert_eq!(dimmer[2], backdrop[2]);

	// Over nothing it writes light with no alpha, for a premultiplied
	// compositor to add; back to normal, it covers again.
	let mut renderer = self::renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::unlit_cube([1.0; 4], false));
	renderer.world_mut().apply([Op::SetBlend {
		node: NodeId(1),
		part: Some("cube".into()),
		blend: Blend::Additive,
	}]);
	let light = draw(&mut renderer).at(32, 32);
	assert!(light[0] > 200 && light[3] == 0, "{light:?}");
	renderer.world_mut().apply([Op::SetBlend {
		node: NodeId(1),
		part: None,
		blend: Blend::Normal,
	}]);
	assert_eq!(draw(&mut renderer).at(32, 32)[3], 255);
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

/// Pure red ink, `pixels` wide on the test's 64-pixel view, with no wobble.
fn red_ink(pixels: f32) -> Ink {
	Ink {
		color: [1.0, 0.0, 0.0],
		width: pixels / SIZE as f32 * 1000.0,
		wobble: 0.0,
		..Ink::default()
	}
}

fn style(shading: Shading, ink: Option<Ink>) -> Op {
	Op::SetStyle(Style { shading, ink })
}

/// Whether a pixel is mostly red ink: covered at least half way, with red
/// well above green and blue. The line's antialiased edges count.
fn is_ink(pixel: [u8; 4]) -> bool {
	pixel[3] >= 128
		&& pixel[0] >= pixel[1].saturating_add(60)
		&& pixel[0] >= pixel[2].saturating_add(60)
}

/// The runs of ink along row `y`, as their first column and length.
fn ink_runs(image: &Image, y: u32) -> Vec<(u32, u32)> {
	let mut runs: Vec<(u32, u32)> = Vec::new();

	for x in 0..image.width {
		if !is_ink(image.at(x, y)) {
			continue;
		}

		match runs.last_mut() {
			Some((start, length)) if *start + *length == x => *length += 1,
			_ => runs.push((x, 1)),
		}
	}

	runs
}

fn ink_count(image: &Image) -> usize {
	image
		.pixels
		.as_chunks::<4>()
		.0
		.iter()
		.filter(|pixel| is_ink(**pixel))
		.count()
}

/// The lit sphere, twice its size, lit from +x (the right of the view)
/// with no shadow under it.
fn lit_sphere(gpu: &Gpu, ink: Option<f32>) -> Renderer {
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::lit_sphere(48, ink));
	renderer.world_mut().apply([
		Op::SetTransform {
			node: NodeId(1),
			transform: Transform::IDENTITY.with_scale(2.0),
		},
		Op::SetLight(Light {
			direction: [-1.0, 0.0, 0.0],
			shadow: 0.0,
			..Light::default()
		}),
	]);
	renderer
}

fn flat_toon(bands: u8) -> Toon {
	Toon {
		bands,
		specular: 0.0,
		..Toon::default()
	}
}

#[test]
fn the_default_style_draws_as_before() {
	let Some(gpu) = gpu() else { return };

	for bytes in [
		fixture::lit_cube(),
		fixture::rig(),
		fixture::lit_sphere(48, Some(1.0)),
	] {
		let mut renderer = renderer(gpu);
		spawn(&mut renderer, 1, 1, &bytes);
		renderer.world_mut().apply([Op::SetCamera {
			transform: Transform::from_translation(0.3, 1.0, 3.5),
			fov_y: 0.8,
			near: 0.1,
			far: 50.0,
		}]);
		let before = draw(&mut renderer).pixels;
		renderer.world_mut().apply([Op::SetStyle(Style::default())]);
		assert_eq!(draw(&mut renderer).pixels, before);
		assert_eq!(renderer.stats().outlines, 0);

		// A style set and taken away leaves nothing behind.
		renderer
			.world_mut()
			.apply([Op::SetStyle(Style::hand_drawn())]);
		assert_ne!(draw(&mut renderer).pixels, before);
		renderer.world_mut().apply([Op::SetStyle(Style::default())]);
		assert_eq!(draw(&mut renderer).pixels, before);
	}
}

#[test]
fn toon_shading_bands_the_light() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = lit_sphere(gpu, None);
	let plateaus = |image: &Image| {
		let row: Vec<[u8; 4]> = (0..SIZE)
			.map(|x| image.at(x, 32))
			.filter(|pixel| pixel[3] == 255)
			.collect();
		let mut levels: Vec<[u8; 4]> = Vec::new();
		let mut steps = 0;

		for pair in row.windows(2) {
			if !near(pair[0], pair[1], 2) {
				steps += 1;
			}
		}

		for pixel in &row {
			if !levels.iter().any(|level| near(*level, *pixel, 2)) {
				levels.push(*pixel);
			}
		}

		(levels.len(), steps, row.len())
	};
	let smooth = draw(&mut renderer);
	let (levels, ..) = plateaus(&smooth);
	assert!(
		levels > 10,
		"physically based light is a gradient: {levels}"
	);

	for bands in [2, 3, 4] {
		renderer
			.world_mut()
			.apply([style(Shading::Toon(flat_toon(bands)), None)]);
		let image = draw(&mut renderer);
		let (levels, steps, width) = plateaus(&image);
		assert!(width > 40, "{width}");
		// One level a band, and between them edges a pixel or two wide.
		assert!(
			levels >= usize::from(bands)
				&& levels <= usize::from(bands) + 2 * (usize::from(bands) - 1),
			"{bands} bands: {levels} levels"
		);
		assert!(
			steps <= 2 * (usize::from(bands) - 1),
			"{bands} bands: {steps} steps"
		);
		// The lit side is the brighter.
		assert!(image.at(52, 32)[0] > image.at(12, 32)[0] + 20);
	}
}

#[test]
fn toon_shadow_takes_the_shade_tint() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::lit_cube());
	// The light comes from behind, so the face the camera sees is in shade.
	renderer.world_mut().apply([Op::SetLight(Light {
		direction: [0.0, 0.0, 1.0],
		shadow: 0.0,
		..Light::default()
	})]);
	let grey = draw(&mut renderer).at(32, 32);
	assert!(
		grey[0].abs_diff(grey[2]) <= 6,
		"physically based shade is grey: {grey:?}"
	);
	renderer.world_mut().apply([style(
		Shading::Toon(Toon {
			shade: [0.2, 0.3, 0.9],
			..flat_toon(3)
		}),
		None,
	)]);
	let tinted = draw(&mut renderer).at(32, 32);
	assert!(tinted[2] > tinted[0] + 40, "{tinted:?}");
	assert!(tinted[2] > tinted[1] + 20, "{tinted:?}");
}

#[test]
fn outlines_ring_the_silhouette_in_ink() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = lit_sphere(gpu, None);
	renderer
		.world_mut()
		.apply([style(Shading::Pbr, Some(red_ink(3.0)))]);
	let image = draw(&mut renderer);
	assert!(!is_ink(image.at(32, 32)), "the middle is the sphere");
	assert_eq!(image.at(1, 1), [0; 4], "outside the line is clear");
	let runs = ink_runs(&image, 32);
	assert_eq!(runs.len(), 2, "{runs:?}");

	for (_, length) in &runs {
		assert!((2..=4).contains(length), "a 3-pixel line: {runs:?}");
	}

	// The line is outside the sphere's own silhouette: the sphere, with
	// no ink, ends where the line begins.
	renderer.world_mut().apply([Op::SetStyle(Style::default())]);
	let plain = draw(&mut renderer);
	let (left, _) = runs[0];
	let (right, length) = runs[1];
	assert_eq!(plain.at(left, 32)[3], 0, "{runs:?}");
	assert_eq!(plain.at(right + length - 1, 32)[3], 0, "{runs:?}");
	let stats = renderer.stats();
	assert_eq!(stats.outlines, 0);
	renderer
		.world_mut()
		.apply([style(Shading::Pbr, Some(red_ink(3.0)))]);
	draw(&mut renderer);
	assert_eq!(renderer.stats().outlines, 1);
	assert_eq!(
		renderer.stats().draws,
		1,
		"the line is not a draw of its own"
	);

	// Ink of no width draws no outline.
	renderer
		.world_mut()
		.apply([style(Shading::Pbr, Some(red_ink(0.0)))]);
	assert_eq!(ink_count(&draw(&mut renderer)), 0);
	assert_eq!(renderer.stats().outlines, 0);
}

#[test]
fn outline_width_holds_in_screen_space() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = lit_sphere(gpu, None);
	renderer
		.world_mut()
		.apply([style(Shading::Pbr, Some(red_ink(3.0)))]);
	let near_runs = ink_runs(&draw(&mut renderer), 32);
	renderer.world_mut().apply([Op::SetTransform {
		node: NodeId(1),
		transform: Transform::from_translation(0.0, 0.0, -3.0).with_scale(2.0),
	}]);
	let far_runs = ink_runs(&draw(&mut renderer), 32);
	assert_eq!(near_runs.len(), 2);
	assert_eq!(far_runs.len(), 2);
	assert!(
		far_runs[0].0 > near_runs[0].0 + 5,
		"the sphere is smaller: {far_runs:?}"
	);

	for (near_run, far_run) in near_runs.iter().zip(&far_runs) {
		assert!(
			near_run.1.abs_diff(far_run.1) <= 1,
			"{near_runs:?} {far_runs:?}"
		);
	}
}

#[test]
fn heavier_lines_fall_on_the_shadow_side() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = lit_sphere(gpu, None);
	renderer.world_mut().apply([style(
		Shading::Pbr,
		Some(Ink {
			shade_weight: 2.0,
			..red_ink(2.0)
		}),
	)]);
	let runs = ink_runs(&draw(&mut renderer), 32);
	let [(_, shaded), (_, lit)] = runs[..] else {
		panic!("{runs:?}")
	};
	assert!(shaded >= lit + 2, "{runs:?}");
}

#[test]
fn hard_edged_meshes_outline_without_gaps() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::unlit_cube([1.0; 4], false));
	// Corner on: the (1, 1, 1) diagonal points at the camera.
	let rotation = glam::Quat::from_rotation_arc(Vec3::ONE.normalize(), Vec3::Z);
	renderer.world_mut().apply([
		Op::SetTransform {
			node: NodeId(1),
			transform: Transform::IDENTITY.with_rotation(rotation.to_array()),
		},
		style(Shading::Pbr, Some(red_ink(3.0))),
		Op::SetInk {
			node: NodeId(1),
			part: None,
			ink: PartInk {
				weight: Some(1.0),
				color: None,
			},
		},
	]);
	let image = draw(&mut renderer);
	let mut touching = Vec::new();

	for y in 1..SIZE - 1 {
		for x in 1..SIZE - 1 {
			let pixel = image.at(x, y);

			if pixel[3] != 255 || pixel[1] < 150 {
				continue;
			}

			let clear = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
				.into_iter()
				.any(|(x, y)| image.at(x, y)[3] < 128);

			if clear {
				touching.push((x, y));
			}
		}
	}

	assert!(
		touching.is_empty(),
		"the cube meets the background at {touching:?}"
	);
	assert!(ink_count(&image) > 100);
}

#[test]
fn skinned_and_morphed_parts_carry_their_outline() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::character(4));
	renderer.world_mut().apply([
		Op::SetCamera {
			transform: Transform::from_translation(0.0, 1.0, 3.0),
			fov_y: 0.8,
			near: 0.1,
			far: 50.0,
		},
		style(Shading::Pbr, Some(red_ink(2.0))),
	]);
	let weigh = |renderer: &mut Renderer, weight: f32| {
		renderer.world_mut().apply([Op::SetMorphWeight {
			node: NodeId(1),
			part: None,
			target: "wide".into(),
			weight,
		}]);
		ink_runs(&draw(renderer), 32)
	};
	let thin = weigh(&mut renderer, 0.0);
	let wide = weigh(&mut renderer, 1.0);
	assert_eq!(thin.len(), 2, "{thin:?}");
	assert_eq!(wide.len(), 2, "{wide:?}");
	assert!(wide[0].0 + 2 <= thin[0].0, "{thin:?} {wide:?}");
	assert!(wide[1].0 >= thin[1].0 + 2, "{thin:?} {wide:?}");

	for (thin, wide) in thin.iter().zip(&wide) {
		assert!(thin.1.abs_diff(wide.1) <= 1, "the line keeps its width");
	}

	// Swaying, the line goes with the skin.
	renderer.world_mut().apply([Op::Play {
		node: NodeId(1),
		play: Play::new("sway"),
	}]);
	renderer.advance(0.2);
	let image = draw(&mut renderer);
	assert!(ink_count(&image) > 50);
	assert_eq!(renderer.stats().outlines, 1);
}

#[test]
fn blended_additive_unlit_and_unweighted_parts_draw_no_line() {
	let Some(gpu) = gpu() else { return };
	let weight = |weight| {
		Op::SetInk {
			node: NodeId(1),
			part: None,
			ink: PartInk {
				weight: Some(weight),
				color: None,
			},
		}
	};
	let cases: [(Vec<u8>, Vec<Op>, bool); 7] = [
		(fixture::lit_cube(), Vec::new(), true),
		(fixture::lit_cube(), vec![weight(0.0)], false),
		(fixture::unlit_cube([1.0; 4], false), Vec::new(), false),
		(
			fixture::unlit_cube([1.0; 4], false),
			vec![weight(1.0)],
			true,
		),
		(
			fixture::unlit_cube([1.0, 1.0, 1.0, 0.5], true),
			Vec::new(),
			false,
		),
		(
			fixture::lit_cube(),
			vec![Op::SetMaterial {
				node: NodeId(1),
				part: None,
				material: MaterialOverride {
					opacity: Some(0.5),
					..MaterialOverride::default()
				},
			}],
			false,
		),
		(
			fixture::lit_cube(),
			vec![Op::SetBlend {
				node: NodeId(1),
				part: None,
				blend: Blend::Additive,
			}],
			false,
		),
	];

	for (index, (bytes, ops, lined)) in cases.into_iter().enumerate() {
		let mut renderer = renderer(gpu);
		spawn(&mut renderer, 1, 1, &bytes);
		renderer
			.world_mut()
			.apply([style(Shading::Pbr, Some(red_ink(3.0)))]);
		renderer.world_mut().apply(ops);
		let image = draw(&mut renderer);
		assert_eq!(ink_count(&image) > 0, lined, "case {index}");
		assert_eq!(renderer.stats().outlines, u32::from(lined), "case {index}");
	}
}

#[test]
fn per_vertex_weights_taper_the_line() {
	let Some(gpu) = gpu() else { return };
	let mut widths = Vec::new();

	for red in [0.0, 0.5, 1.0] {
		let mut renderer = lit_sphere(gpu, Some(red));
		renderer
			.world_mut()
			.apply([style(Shading::Pbr, Some(red_ink(2.0)))]);
		let runs = ink_runs(&draw(&mut renderer), 32);
		widths.push(runs.iter().map(|(_, length)| *length).sum::<u32>());
	}

	assert_eq!(widths[0], 0, "a red of 0 draws no line: {widths:?}");
	assert!(
		(4..=7).contains(&widths[1]),
		"0.5 is the part's line: {widths:?}"
	);
	assert!(
		widths[2] >= widths[1] + 3,
		"1 is twice as heavy: {widths:?}"
	);
}

#[test]
fn parts_too_small_to_see_are_not_outlined() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::lit_cube());
	renderer.world_mut().apply([
		style(Shading::Pbr, Some(red_ink(2.0))),
		Op::SetTransform {
			node: NodeId(1),
			transform: Transform::IDENTITY.with_scale(0.02),
		},
	]);
	draw(&mut renderer);
	let stats = renderer.stats();
	assert_eq!((stats.draws, stats.outlines), (1, 0));
	renderer.world_mut().apply([Op::SetTransform {
		node: NodeId(1),
		transform: Transform::IDENTITY.with_scale(0.5),
	}]);
	draw(&mut renderer);
	assert_eq!(renderer.stats().outlines, 1);
}

#[test]
fn real_world_depth_hides_lines_too() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::lit_cube());
	renderer
		.world_mut()
		.apply([style(Shading::Pbr, Some(red_ink(3.0)))]);
	let near_wall = solid_texture(gpu, wgpu::TextureFormat::R32Float, &1.0_f32.to_le_bytes());
	let far_wall = solid_texture(gpu, wgpu::TextureFormat::R32Float, &10.0_f32.to_le_bytes());

	for (depth, visible) in [(&near_wall, false), (&far_wall, true)] {
		let occlusion = Occlusion {
			depth,
			transform: Mat3::IDENTITY,
			scale: 1.0,
			tolerance: 0.05,
		};
		let image = render_with(&mut renderer, &[], Background::Scene, Some(occlusion));
		assert_eq!(ink_count(&image) > 0, visible);
	}
}

#[test]
fn the_boil_steps_and_holds() {
	let Some(gpu) = gpu() else { return };
	let boiling = style(
		Shading::Pbr,
		Some(Ink {
			wobble: 1.0,
			boil: 4.0,
			seed: 3,
			..red_ink(4.0)
		}),
	);
	let mut renderer = lit_sphere(gpu, None);
	renderer.world_mut().apply([boiling.clone()]);

	for _ in 0..4 {
		renderer.advance(0.25);
	}

	// One second in: step 4 of a boil at 4 a second.
	let first = draw(&mut renderer).pixels;
	assert_eq!(
		draw(&mut renderer).pixels,
		first,
		"the same time, the same line"
	);
	assert!(!renderer.world().needs_frame(), "a boil asks for no frames");
	renderer.advance(0.2);
	assert_eq!(draw(&mut renderer).pixels, first, "held within a step");
	renderer.advance(0.1);
	let next = draw(&mut renderer).pixels;
	assert_ne!(next, first, "drawn afresh at the next step");

	// Another scene at the same time draws the same line.
	let mut again = lit_sphere(gpu, None);
	again.world_mut().apply([boiling]);

	for _ in 0..4 {
		again.advance(0.25);
	}

	assert_eq!(draw(&mut again).pixels, first);
}

#[test]
fn wobble_varies_the_line_along_the_surface() {
	let Some(gpu) = gpu() else { return };
	let widths = |wobble: f32| {
		let mut renderer = lit_sphere(gpu, None);
		renderer.world_mut().apply([style(
			Shading::Pbr,
			Some(Ink {
				wobble,
				..red_ink(4.0)
			}),
		)]);
		let image = draw(&mut renderer);
		(8..56)
			.filter_map(|y| ink_runs(&image, y).first().map(|(_, length)| *length))
			.collect::<Vec<_>>()
	};
	let even = widths(0.0);
	let wobbly = widths(1.0);
	let spread = |widths: &[u32]| widths.iter().max().unwrap() - widths.iter().min().unwrap();
	assert!(spread(&wobbly) > spread(&even), "{even:?} {wobbly:?}");
}

#[test]
fn inner_lines_and_hatching_ink_the_surface() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = lit_sphere(gpu, None);
	let toon = Shading::Toon(flat_toon(3));
	renderer
		.world_mut()
		.apply([style(toon, Some(red_ink(2.0)))]);
	let plain = ink_count(&draw(&mut renderer));

	// The facing line inks the sphere's rim inside its outline.
	renderer.world_mut().apply([style(
		toon,
		Some(Ink {
			facing: 1.0,
			..red_ink(2.0)
		}),
	)]);
	let image = draw(&mut renderer);
	assert!(
		ink_count(&image) > plain + 20,
		"{plain} {}",
		ink_count(&image)
	);
	assert!(!is_ink(image.at(32, 32)));

	// Hatching inks the darkest band, on the left, and nothing on the
	// right. Its lines are fixed to the surface, 60 to a metre of the
	// model, and fade out where they would crowd closer than a few pixels:
	// at 64 pixels the sphere is too small to hatch, so this draws it at
	// 512.
	let mut hatched = |hatch: f32| {
		renderer.world_mut().apply([style(
			Shading::Toon(Toon {
				hatch,
				..flat_toon(3)
			}),
			Some(red_ink(2.0)),
		)]);
		renderer.advance(0.0);
		Image {
			pixels: renderer.render_to_rgba(512, 512).unwrap(),
			width: 512,
		}
	};
	let count = |image: &Image, columns: std::ops::Range<u32>| {
		columns
			.flat_map(|x| (160..352).map(move |y| (x, y)))
			.filter(|(x, y)| is_ink(image.at(*x, *y)))
			.count()
	};
	let clean = hatched(0.0);
	let image = hatched(1.0);
	assert_eq!(count(&clean, 112..224), 0);
	assert!(
		count(&image, 112..224) > 1000,
		"{}",
		count(&image, 112..224)
	);
	assert_eq!(count(&image, 320..400), 0, "the lit side is clear");

	// Lines come and go along a row in the shade: they are lines, not a
	// tint.
	let row: Vec<bool> = (112..224).map(|x| is_ink(image.at(x, 256))).collect();
	let changes = row.windows(2).filter(|pair| pair[0] != pair[1]).count();
	assert!(changes >= 8, "{changes}");
}

#[test]
fn stereo_views_ink_each_eye() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = lit_sphere(gpu, None);
	renderer.world_mut().apply([
		Op::SetTransform {
			node: NodeId(1),
			transform: Transform::IDENTITY.with_scale(0.8),
		},
		style(Shading::Pbr, Some(red_ink(3.0))),
	]);
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
	let runs = ink_runs(&image, 32);
	assert_eq!(runs.len(), 4, "two edges in each eye: {runs:?}");

	for (_, length) in &runs {
		assert!((2..=4).contains(length), "{runs:?}");
	}

	assert_eq!(
		renderer.stats().outlines,
		1,
		"one outline draw for both eyes"
	);
}

/// A single lit quad at z = 0 facing -z (back-facing to a camera at +z),
/// double-sided, with optional `COLOR_1` red weight and alpha mode.
fn back_facing_quad(color: [f32; 4], alpha_mode: &str, ink_red: Option<f32>) -> Vec<u8> {
	let mut builder = fixture::Builder::default();
	// Wound clockwise when viewed from +z (so outward normal is -z, front face
	// away from camera).
	let positions: Vec<f32> = vec![
		-0.5, -0.5, 0.0, -0.5, 0.5, 0.0, 0.5, 0.5, 0.0, 0.5, -0.5, 0.0,
	];
	let normals: Vec<f32> = vec![
		0.0, 0.0, -1.0, 0.0, 0.0, -1.0, 0.0, 0.0, -1.0, 0.0, 0.0, -1.0,
	];
	let indices: Vec<u16> = vec![0, 1, 2, 0, 2, 3];
	let position = builder.floats(&positions, 3, true);
	let normal = builder.floats(&normals, 3, false);
	let index = builder.u16s(&indices, 1);
	let color_1 = ink_red.map_or(String::new(), |red| {
		let values: Vec<f32> = (0..4).flat_map(|_| [red, 0.0, 0.0, 1.0]).collect();
		format!(r#","COLOR_1":{}"#, builder.floats(&values, 4, false))
	});
	let json = format!(
		r#""asset":{{"version":"2.0"}},
"scenes":[{{"nodes":[0]}}],
"nodes":[{{"name":"quad","mesh":0}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":{position},"NORMAL":{normal}{color_1}}},"indices":{index},"material":0}}]}}],
"materials":[{{"name":"mat","pbrMetallicRoughness":{{"baseColorFactor":{color:?},"metallicFactor":0,"roughnessFactor":0.7}},"alphaMode":"{alpha_mode}","alphaCutoff":0.5,"doubleSided":true}}]"#
	);
	builder.glb(&json)
}

#[test]
fn double_sided_zero_weight_does_not_paint_solid_ink() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(
		&mut renderer,
		1,
		1,
		&back_facing_quad([0.5, 0.5, 0.5, 1.0], "OPAQUE", Some(0.0)),
	);
	renderer
		.world_mut()
		.apply([style(Shading::Pbr, Some(red_ink(3.0)))]);
	let image = draw(&mut renderer);
	assert!(
		!is_ink(image.at(32, 32)),
		"center of zero-weight double-sided quad painted ink: {:?}",
		image.at(32, 32)
	);
}

#[test]
fn alpha_mask_cutout_is_not_filled_by_outline() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	// Fully cut-out masked quad (alpha = 0.0 < cutoff 0.5): fs_mesh discards
	// all pixels, so nothing should be visible, including no ink hull.
	spawn(
		&mut renderer,
		1,
		1,
		&back_facing_quad([0.5, 0.5, 0.5, 0.0], "MASK", None),
	);
	renderer
		.world_mut()
		.apply([style(Shading::Pbr, Some(red_ink(3.0)))]);
	let image = draw(&mut renderer);
	assert_eq!(
		ink_count(&image),
		0,
		"cut-out MASK quad painted {} solid ink pixels; center={:?}",
		ink_count(&image),
		image.at(32, 32)
	);
	assert_eq!(renderer.stats().outlines, 0);
}

#[test]
fn mirrored_negative_scale_keeps_outline() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = lit_sphere(gpu, None);
	renderer.world_mut().apply([
		Op::SetTransform {
			node: NodeId(1),
			transform: Transform {
				scale: [-2.0, 2.0, 2.0],
				..Transform::IDENTITY
			},
		},
		style(Shading::Pbr, Some(red_ink(3.0))),
	]);
	let image = draw(&mut renderer);
	let runs = ink_runs(&image, 32);
	assert_eq!(
		runs.len(),
		2,
		"mirrored sphere lost its outline: runs={runs:?}, total ink={}",
		ink_count(&image)
	);
	assert!(!is_ink(image.at(32, 32)), "center must be the lit sphere");
}

#[test]
fn fading_part_ink_weight_does_not_clamp_to_one_pixel() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = lit_sphere(gpu, None);
	renderer.world_mut().apply([
		style(Shading::Pbr, Some(red_ink(3.0))),
		Op::SetInk {
			node: NodeId(1),
			part: None,
			ink: PartInk {
				weight: Some(0.001),
				color: None,
			},
		},
	]);
	let image = draw(&mut renderer);
	// Requested width is 3.0 * 0.001 = 0.003 px: weights below 1 taper the
	// floor smoothly to 0 rather than clamping to a full 1 px ring.
	assert_eq!(
		ink_count(&image),
		0,
		"weight=0.001 (0.003 px) still drew {} ink pixels; row 32 runs={:?}",
		ink_count(&image),
		ink_runs(&image, 32)
	);
}

#[test]
fn toon_softness_widens_the_band_transition() {
	let Some(gpu) = gpu() else { return };
	let distinct_levels = |softness: f32| {
		let mut renderer = lit_sphere(gpu, None);
		renderer.world_mut().apply([style(
			Shading::Toon(Toon {
				softness,
				..flat_toon(2)
			}),
			None,
		)]);
		let image = draw(&mut renderer);
		let mut levels: Vec<[u8; 4]> = Vec::new();

		for x in 0..SIZE {
			let pixel = image.at(x, 32);

			if pixel[3] == 255 && !levels.iter().any(|level| near(*level, pixel, 2)) {
				levels.push(pixel);
			}
		}

		levels.len()
	};
	let crisp = distinct_levels(0.0);
	let soft = distinct_levels(1.0);
	assert!(crisp <= 4, "crisp 2-band toon should be flat: {crisp}");
	assert!(
		soft >= 10,
		"softness=1.0 should blend smoothly across the band edge: crisp={crisp} soft={soft}"
	);
}

#[test]
fn toon_specular_draws_a_hard_highlight_spot() {
	let Some(gpu) = gpu() else { return };
	let peak_red = |specular: f32| {
		let mut renderer = lit_sphere(gpu, None);
		renderer.world_mut().apply([
			Op::SetLight(Light {
				direction: [-0.3, -0.3, -1.0],
				shadow: 0.0,
				..Light::default()
			}),
			Op::SetMaterial {
				node: NodeId(1),
				part: None,
				material: MaterialOverride {
					color: Some([0.2, 0.2, 0.2, 1.0]),
					..MaterialOverride::default()
				},
			},
			style(
				Shading::Toon(Toon {
					bands: 2,
					specular,
					..Toon::default()
				}),
				None,
			),
		]);
		let image = draw(&mut renderer);
		image
			.pixels
			.as_chunks::<4>()
			.0
			.iter()
			.filter(|pixel| pixel[3] == 255)
			.map(|pixel| pixel[0])
			.max()
			.unwrap_or(0)
	};
	let plain = peak_red(0.0);
	let shiny = peak_red(2.0);
	assert!(
		shiny > plain + 40,
		"toon specular=2.0 should brighten the highlight spot: plain={plain} shiny={shiny}"
	);
}

#[test]
fn toon_rim_is_hard_edged_rather_than_soft() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = lit_sphere(gpu, None);
	renderer.world_mut().apply([
		style(Shading::Toon(flat_toon(2)), None),
		Op::SetMaterial {
			node: NodeId(1),
			part: None,
			material: MaterialOverride {
				color: Some([0.2, 0.2, 0.2, 1.0]),
				rim: Some(Rim {
					color: [0.0, 1.0, 0.0],
					strength: 1.5,
					power: 1.0,
				}),
				..MaterialOverride::default()
			},
		},
	]);
	let image = draw(&mut renderer);
	let opaque: Vec<u32> = (0..SIZE).filter(|&x| image.at(x, 32)[3] == 255).collect();
	let left = *opaque.first().unwrap();
	let right = *opaque.last().unwrap();
	let half = (right - left) / 2;
	let center = u32::midpoint(left, right);
	// With power = 1.0, hard toon rim starts at facing = 0.5.
	// At ~60% of the radius from center, facing is ~0.2 (< 0.5): a soft rim
	// adds 1.5 * 0.2 = 0.3 linear green (> +70 sRGB), while hard toon rim adds
	// 0.
	let interior = image.at(center + half * 3 / 5, 32);
	let edge = image.at(right - 1, 32);
	assert!(
		edge[1] > edge[0].saturating_add(60),
		"silhouette should carry the bright green rim: edge={edge:?}"
	);
	assert!(
		interior[1] <= interior[0].saturating_add(15),
		"interior below rim threshold must not get soft rim glow: interior={interior:?}"
	);
}

/// A sphere with the same world-space radius (0.5) as `fixture::lit_sphere`,
/// stored in quantized-style mesh coordinates multiplied by `1 / scale` with
/// node scale `scale`, so `rest_units` equals `scale`.
fn quantized_scale_sphere(segments: usize, scale: f32) -> Vec<u8> {
	let mut builder = fixture::Builder::default();
	let rings = segments / 2;
	let mut positions = Vec::new();
	let mut normals = Vec::new();
	let mut indices = Vec::new();
	let inv_scale = 1.0 / scale;

	for ring in 0..=rings {
		let (sin_theta, cos_theta) = (ring as f32 / rings as f32 * std::f32::consts::PI).sin_cos();

		for step in 0..=segments {
			let (sin_phi, cos_phi) =
				(step as f32 / segments as f32 * std::f32::consts::TAU).sin_cos();
			let normal = [sin_theta * cos_phi, cos_theta, sin_theta * sin_phi];
			positions.extend(normal.map(|value| value * 0.5 * inv_scale));
			normals.extend(normal);
		}
	}

	for ring in 0..rings {
		for step in 0..segments {
			let a = (ring * (segments + 1) + step) as u16;
			let b = a + 1;
			let c = a + segments as u16 + 1;
			let d = c + 1;
			indices.extend_from_slice(&[a, b, c, b, d, c]);
		}
	}

	let position = builder.floats(&positions, 3, true);
	let normal = builder.floats(&normals, 3, false);
	let index = builder.u16s(&indices, 1);
	let json = format!(
		r#""asset":{{"version":"2.0"}},
"scenes":[{{"nodes":[0]}}],
"nodes":[{{"name":"sphere","mesh":0,"scale":[{scale},{scale},{scale}]}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":{position},"NORMAL":{normal}}},"indices":{index},"material":0}}]}}],
"materials":[{{"name":"grey","pbrMetallicRoughness":{{"baseColorFactor":[0.5,0.5,0.5,1],"metallicFactor":0,"roughnessFactor":0.7}}}}]"#
	);
	builder.glb(&json)
}

#[test]
fn quantized_scale_mesh_hatches_at_rest_pose_density() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	// 5e-5 matches gltfpack-quantized models (below f16::MIN_POSITIVE).
	spawn(&mut renderer, 1, 1, &quantized_scale_sphere(48, 5.0e-5));
	renderer.world_mut().apply([
		Op::SetTransform {
			node: NodeId(1),
			transform: Transform::IDENTITY.with_scale(2.0),
		},
		Op::SetLight(Light {
			direction: [-1.0, 0.0, 0.0],
			shadow: 0.0,
			..Light::default()
		}),
		style(
			Shading::Toon(Toon {
				hatch: 1.0,
				..flat_toon(3)
			}),
			Some(red_ink(2.0)),
		),
	]);
	renderer.advance(0.0);
	let image = Image {
		pixels: renderer.render_to_rgba(512, 512).unwrap(),
		width: 512,
	};
	let shaded_ink = (112..224)
		.flat_map(|x| (160..352).map(move |y| (x, y)))
		.filter(|(x, y)| is_ink(image.at(*x, *y)))
		.count();
	assert!(
		shaded_ink > 1000,
		"quantized sphere should hatch in rest-pose metres, got only {shaded_ink} ink pixels"
	);
	let row: Vec<bool> = (112..224).map(|x| is_ink(image.at(x, 256))).collect();
	let changes = row.windows(2).filter(|pair| pair[0] != pair[1]).count();
	assert!(changes >= 8, "expected distinct hatch lines, got {changes}");
}

/// Exact sRGB encoding of a linear colour without exposure or tonemapping, as
/// relief-print surfaces and ink lines output on an `Rgba8Unorm` target.
fn exact_srgb(color: [f32; 3]) -> [u8; 4] {
	let encode = |value: f32| {
		let value = value.clamp(0.0, 1.0);
		let srgb = if value <= 0.003_130_8 {
			value * 12.92
		} else {
			1.055 * value.powf(1.0 / 2.4) - 0.055
		};
		(srgb * 255.0).round() as u8
	};
	[encode(color[0]), encode(color[1]), encode(color[2]), 255]
}

fn print_no_knockout() -> Print {
	Print {
		knockout_width: 0.0,
		..Print::default()
	}
}

#[test]
fn print_bands_output_exact_un_tonemapped_colours_and_flat_mode() {
	let Some(gpu) = gpu() else { return };
	let lit = [0.85, 0.70, 0.15];
	let shade = [0.15, 0.55, 0.20];
	let deep = [0.12, 0.10, 0.60];
	let mut renderer = lit_sphere(gpu, None);
	renderer.world_mut().apply([
		Op::SetExposure(4.0),
		Op::SetLight(Light {
			direction: [-1.0, 0.0, 0.0],
			intensity: 10.0,
			shadow: 0.0,
			..Light::default()
		}),
		style(
			Shading::Print(Print {
				t_lit: 0.30,
				t_deep: -0.10,
				rim: 0.0,
				..print_no_knockout()
			}),
			None,
		),
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(lit),
				shade: Some(shade),
				deep: Some(deep),
				..PrintOverride::default()
			},
		},
	]);

	let image = draw(&mut renderer);
	assert!(
		near(image.at(48, 32), exact_srgb(lit), 2),
		"lit band: {:?} vs {:?}",
		image.at(48, 32),
		exact_srgb(lit)
	);
	assert!(
		near(image.at(32, 32), exact_srgb(shade), 2),
		"shade band: {:?} vs {:?}",
		image.at(32, 32),
		exact_srgb(shade)
	);
	assert!(
		near(image.at(16, 32), exact_srgb(deep), 2),
		"deep band: {:?} vs {:?}",
		image.at(16, 32),
		exact_srgb(deep)
	);

	// Flat mode outputs exact `lit` everywhere regardless of N·L.
	renderer.world_mut().apply([Op::SetPrint {
		node: NodeId(1),
		target: PrintTarget::All,
		print: PrintOverride {
			lit: Some(lit),
			shade: Some(shade),
			deep: Some(deep),
			mode: Some(PrintMode::Flat),
			..PrintOverride::default()
		},
	}]);
	let flat = draw(&mut renderer);
	for x in [16, 32, 48] {
		assert!(
			near(flat.at(x, 32), exact_srgb(lit), 2),
			"flat at x={x}: {:?} vs {:?}",
			flat.at(x, 32),
			exact_srgb(lit)
		);
	}
}

#[test]
fn print_hatching_cross_hatching_and_fade_to_mean_tone() {
	let Some(gpu) = gpu() else { return };
	let lit = [0.90, 0.90, 0.90];
	let shade = [0.50, 0.50, 0.50];
	let deep = [0.05, 0.05, 0.80];
	let ink_color = [0.95, 0.05, 0.05];
	let mut renderer = lit_sphere(gpu, None);
	let render_at = |renderer: &mut Renderer, size: u32| {
		renderer.advance(0.0);
		Image {
			pixels: renderer.render_to_rgba(size, size).unwrap(),
			width: size,
		}
	};

	renderer.world_mut().apply([
		style(
			Shading::Print(Print {
				t_lit: 0.30,
				t_deep: 0.05,
				t_cross: -0.40,
				rim: 0.0,
				hatch_per_m: 32.0,
				hatch_gain: 0.40,
				hatch_max: 0.55,
				cross_duty: 0.35,
				fade_px: [2.5, 4.0],
				..print_no_knockout()
			}),
			Some(Ink {
				color: ink_color,
				width: 0.0,
				..Ink::default()
			}),
		),
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(lit),
				shade: Some(shade),
				deep: Some(deep),
				hatch_min: Some(0.25),
				cross_hatch: Some(false),
				..PrintOverride::default()
			},
		},
	]);

	let primary = render_at(&mut renderer, 256);
	let red_count = |image: &Image, cols: std::ops::Range<u32>, rows: std::ops::Range<u32>| {
		cols.flat_map(|x| rows.clone().map(move |y| (x, y)))
			.filter(|(x, y)| {
				let p = image.at(*x, *y);
				u16::from(p[0]) > u16::from(p[2]) + 40
			})
			.count()
	};
	// Lit side (right) stays clear of hatching.
	assert_eq!(red_count(&primary, 165..210, 96..160), 0);
	// Deep side (left) has crisp alternating hatch stripes.
	let primary_deep_red = red_count(&primary, 48..116, 96..160);
	assert!(primary_deep_red > 500, "{primary_deep_red}");
	let row: Vec<bool> = (52..116)
		.map(|x| {
			let p = primary.at(x, 128);
			p[0] > p[2]
		})
		.collect();
	let transitions = row.windows(2).filter(|pair| pair[0] != pair[1]).count();
	assert!(transitions >= 6, "expected discrete stripes: {transitions}");

	// Enabling cross-hatch adds secondary strokes in the deepest shadow (N·L <
	// t_cross).
	renderer.world_mut().apply([Op::SetPrint {
		node: NodeId(1),
		target: PrintTarget::All,
		print: PrintOverride {
			lit: Some(lit),
			shade: Some(shade),
			deep: Some(deep),
			hatch_min: Some(0.25),
			cross_hatch: Some(true),
			..PrintOverride::default()
		},
	}]);
	let crossed = render_at(&mut renderer, 256);
	let crossed_deep_red = red_count(&crossed, 48..92, 96..160);
	let primary_deepest_red = red_count(&primary, 48..92, 96..160);
	assert!(
		crossed_deep_red > primary_deepest_red + 100,
		"cross-hatch adds ink in deepest band: {crossed_deep_red} vs {primary_deepest_red}"
	);

	// Very fine pitch (< fade_lo px) fades cleanly to the mean tone instead of
	// moiré or 0.
	renderer.world_mut().apply([Op::SetPrint {
		node: NodeId(1),
		target: PrintTarget::All,
		print: PrintOverride {
			lit: Some(lit),
			shade: Some(shade),
			deep: Some(deep),
			hatch_min: Some(0.50),
			hatch_per_m: Some(1000.0),
			cross_hatch: Some(false),
			..PrintOverride::default()
		},
	}]);
	let faded = draw(&mut renderer);
	let samples: Vec<[u8; 4]> = (14..22).map(|x| faded.at(x, 32)).collect();
	let min_r = samples.iter().map(|p| p[0]).min().unwrap();
	let max_r = samples.iter().map(|p| p[0]).max().unwrap();
	// Uniform mean blend of red ink into blue deep (noticeably redder than pure
	// deep [62,62,231]).
	let pure_deep = exact_srgb(deep);
	assert!(
		min_r > pure_deep[0] + 60,
		"faded hatch keeps mean ink tone instead of vanishing: {samples:?} vs pure {pure_deep:?}"
	);
	assert!(
		max_r - min_r <= 20,
		"faded hatch stays smooth without moiré stripes: {samples:?}"
	);
}

#[test]
fn print_paper_specular_and_backlit_spot_rim() {
	let Some(gpu) = gpu() else { return };
	let lit = [0.20, 0.25, 0.30];
	let shade = [0.10, 0.12, 0.15];
	let deep = [0.05, 0.05, 0.08];
	let paper = [0.95, 0.90, 0.65];
	let spot = [0.95, 0.15, 0.85];
	let mut renderer = lit_sphere(gpu, None);
	renderer.world_mut().apply([
		Op::SetLight(Light {
			// Side-key light (matching cel.toml's grazing key angle) so the right turning flank
			// has N·V < 0.5 and N·H > 0.90, while the left silhouette has N·V < rim and N·L < 0.05.
			direction: [-0.95, 0.0, 0.30],
			shadow: 0.0,
			..Light::default()
		}),
		style(
			Shading::Print(Print {
				t_lit: 0.30,
				t_deep: -0.10,
				rim: 0.45,
				paper,
				..print_no_knockout()
			}),
			None,
		),
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(lit),
				shade: Some(shade),
				deep: Some(deep),
				spot: Some(spot),
				specular: Some(0.90),
				rim: Some(true),
				..PrintOverride::default()
			},
		},
	]);

	let image = draw(&mut renderer);
	let expected_paper = exact_srgb(paper);
	let expected_spot = exact_srgb(spot);
	let expected_deep = exact_srgb(deep);

	// Center faces camera (N·V ~ 1.0 >= 0.5): must NOT get turning-edge paper
	// specular or rim.
	assert!(
		near(image.at(32, 32), expected_deep, 2),
		"center should be deep (not paper or spot): {:?} vs {expected_deep:?}",
		image.at(32, 32)
	);

	// Scan equator row for paper specular on right turning flank and spot rim
	// on left shadow flank.
	let right_paper = (33..60)
		.filter(|&x| near(image.at(x, 32), expected_paper, 4))
		.count();
	let left_spot = (4..31)
		.filter(|&x| near(image.at(x, 32), expected_spot, 4))
		.count();
	assert!(
		right_paper >= 2,
		"expected paper specular on turning edge, got {right_paper}"
	);
	assert!(
		left_spot >= 2,
		"expected spot rim on shadow silhouette, got {left_spot}"
	);
}

#[test]
fn print_overrides_part_beats_material_beats_all_on_screen() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::multi_material_parts());
	let blue = [0.05, 0.15, 0.85];
	let green = [0.10, 0.80, 0.15];
	let red = [0.85, 0.10, 0.10];

	renderer.world_mut().apply([
		Op::SetLight(Light {
			direction: [0.0, 0.0, -1.0],
			shadow: 0.0,
			..Light::default()
		}),
		style(Shading::Print(print_no_knockout()), None),
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(blue),
				mode: Some(PrintMode::Flat),
				..PrintOverride::default()
			},
		},
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::Material("cloth".into()),
			print: PrintOverride {
				lit: Some(green),
				..PrintOverride::default()
			},
		},
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::Part("right".into()),
			print: PrintOverride {
				lit: Some(red),
				..PrintOverride::default()
			},
		},
	]);

	let image = draw(&mut renderer);
	// Left-top is `left` with `cloth.001` -> Material("cloth") beats All ->
	// green.
	assert!(
		near(image.at(20, 24), exact_srgb(green), 2),
		"left/cloth.001: {:?} vs {:?}",
		image.at(20, 24),
		exact_srgb(green)
	);
	// Left-bottom is `left` with `gold` -> falls back to All -> blue.
	assert!(
		near(image.at(20, 42), exact_srgb(blue), 2),
		"left/gold: {:?} vs {:?}",
		image.at(20, 42),
		exact_srgb(blue)
	);
	// Right is `right` with `cloth.002` -> Part("right") beats
	// Material("cloth") -> red.
	assert!(
		near(image.at(44, 32), exact_srgb(red), 2),
		"right/cloth.002: {:?} vs {:?}",
		image.at(44, 32),
		exact_srgb(red)
	);
}

#[test]
fn camera_space_light_follows_the_camera() {
	let Some(gpu) = gpu() else { return };
	let lit = [0.90, 0.85, 0.20];
	let deep = [0.10, 0.10, 0.50];
	let mut renderer = lit_sphere(gpu, None);
	renderer.world_mut().apply([
		style(
			Shading::Print(Print {
				t_lit: 0.25,
				t_deep: -0.25,
				rim: 0.0,
				..print_no_knockout()
			}),
			None,
		),
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(lit),
				shade: Some([0.4, 0.4, 0.4]),
				deep: Some(deep),
				..PrintOverride::default()
			},
		},
		// Light travels -X in camera space (from camera's right).
		Op::SetLight(Light {
			direction: [-1.0, 0.0, 0.0],
			shadow: 0.0,
			camera_space: true,
			..Light::default()
		}),
	]);

	let front = draw(&mut renderer);
	assert!(near(front.at(48, 32), exact_srgb(lit), 2));
	assert!(near(front.at(16, 32), exact_srgb(deep), 2));

	// Orbit camera 180 deg to the opposite side: camera-space light stays on
	// screen right.
	renderer.world_mut().apply([Op::SetCamera {
		transform: Transform::from_translation(0.0, 0.0, -3.0).with_yaw(std::f32::consts::PI),
		fov_y: 0.8,
		near: 0.1,
		far: 50.0,
	}]);
	let back = draw(&mut renderer);
	assert!(
		near(back.at(48, 32), exact_srgb(lit), 2),
		"camera-space light stayed on right: {:?}",
		back.at(48, 32)
	);
	assert!(
		near(back.at(16, 32), exact_srgb(deep), 2),
		"camera-space shadow stayed on left: {:?}",
		back.at(16, 32)
	);

	// Flip to world-space light: now screen right and left swap.
	renderer.world_mut().apply([Op::SetLight(Light {
		direction: [-1.0, 0.0, 0.0],
		shadow: 0.0,
		camera_space: false,
		..Light::default()
	})]);
	let world = draw(&mut renderer);
	assert!(
		near(world.at(16, 32), exact_srgb(lit), 2),
		"world-space light moved to screen left: {:?}",
		world.at(16, 32)
	);
	assert!(
		near(world.at(48, 32), exact_srgb(deep), 2),
		"world-space shadow moved to screen right: {:?}",
		world.at(48, 32)
	);
}

#[test]
fn print_outline_thins_on_lit_side_and_thickens_on_shadow_side() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = lit_sphere(gpu, None);
	let render_128 = |renderer: &mut Renderer| {
		renderer.advance(0.0);
		Image {
			pixels: renderer.render_to_rgba(128, 128).unwrap(),
			width: 128,
		}
	};

	renderer.world_mut().apply([
		Op::SetLight(Light {
			// Light shines from the right (-X travel, toward +X).
			direction: [-1.0, 0.0, 0.0],
			shadow: 0.0,
			..Light::default()
		}),
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some([0.2, 0.3, 0.8]),
				shade: Some([0.1, 0.2, 0.6]),
				deep: Some([0.05, 0.1, 0.4]),
				..PrintOverride::default()
			},
		},
		style(
			Shading::Print(Print {
				side_heavy: 1.40,
				side_drop: 0.90,
				side_from: -0.35,
				side_to: 0.75,
				..print_no_knockout()
			}),
			Some(Ink {
				wobble: 0.0,
				..red_ink(55.0)
			}),
		),
	]);

	let image = render_128(&mut renderer);
	let runs = ink_runs(&image, 64);
	assert_eq!(runs.len(), 2, "two silhouette edges at equator: {runs:?}");
	let shadow_width = runs[0].1;
	let lit_width = runs[1].1;
	assert!(
		shadow_width > lit_width + 3,
		"shadow edge ({shadow_width}px) must be noticeably thicker than lit edge ({lit_width}px): \
		 {runs:?}"
	);

	// Zero side_drop restores symmetric edges.
	renderer.world_mut().apply([style(
		Shading::Print(Print {
			side_heavy: 1.0,
			side_drop: 0.0,
			..print_no_knockout()
		}),
		Some(Ink {
			wobble: 0.0,
			..red_ink(55.0)
		}),
	)]);
	let symmetric = render_128(&mut renderer);
	let sym_runs = ink_runs(&symmetric, 64);
	assert_eq!(sym_runs.len(), 2, "{sym_runs:?}");
	assert!(
		sym_runs[0].1.abs_diff(sym_runs[1].1) <= 1,
		"zero side_drop yields symmetric edges: {sym_runs:?}"
	);
}

#[test]
fn print_outline_wobble_frequency_and_per_figure_three_slot_boil() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = renderer(gpu);
	let sphere = fixture::lit_sphere(32, None);
	spawn(&mut renderer, 1, 1, &sphere);
	renderer.world_mut().apply([
		Op::SetBackdrop(Backdrop::Color([0.05, 0.05, 0.10, 1.0])),
		Op::SetLight(Light {
			direction: [0.0, 0.0, -1.0],
			shadow: 0.0,
			..Light::default()
		}),
		Op::SetTransform {
			node: NodeId(1),
			transform: Transform::from_translation(-0.65, 0.0, 0.0).with_scale(0.45),
		},
		Op::Spawn {
			node: NodeId(2),
			asset: Some(AssetId(1)),
			parent: None,
		},
		Op::SetTransform {
			node: NodeId(2),
			transform: Transform::from_translation(0.65, 0.0, 0.0).with_scale(0.45),
		},
		style(
			Shading::Print(Print {
				wobble_frequency: 5.0,
				..print_no_knockout()
			}),
			Some(Ink {
				wobble: 0.45,
				boil: 0.0,
				..red_ink(55.0)
			}),
		),
	]);

	let half = |image: &Image, cols: std::ops::Range<u32>| -> Vec<[u8; 4]> {
		(0..128)
			.flat_map(|y| cols.clone().map(move |x| image.at(x, y)))
			.collect()
	};

	let mut frames = Vec::new();
	for slot in 0..=3_u8 {
		renderer.world_mut().apply([Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some([0.1, 0.3, 0.8]),
				shade: Some([0.1, 0.2, 0.6]),
				deep: Some([0.05, 0.1, 0.4]),
				boil_step: Some(slot),
				..PrintOverride::default()
			},
		}]);
		renderer.advance(0.0);
		frames.push(Image {
			pixels: renderer.render_to_rgba(128, 128).unwrap(),
			width: 128,
		});
	}

	// Boiling figure (left half) differs across slots 0, 1, 2 and wraps back at
	// 3 == 0.
	let left: Vec<_> = frames.iter().map(|img| half(img, 0..64)).collect();
	assert_ne!(left[0], left[1], "slot 0 and 1 must differ");
	assert_ne!(left[1], left[2], "slot 1 and 2 must differ");
	assert_ne!(left[0], left[2], "slot 0 and 2 must differ");
	assert_eq!(left[0], left[3], "slot 3 wraps back to slot 0");

	// Static sibling (right half) is byte-for-byte identical across all frames.
	let right: Vec<_> = frames.iter().map(|img| half(img, 64..128)).collect();
	assert_eq!(right[0], right[1], "static sibling must not boil");
	assert_eq!(right[0], right[2], "static sibling must not boil");
	assert_eq!(right[0], right[3], "static sibling must not boil");

	// Changing wobble_frequency alters the wobbled outline profile.
	renderer.world_mut().apply([style(
		Shading::Print(Print {
			wobble_frequency: 1.0,
			..print_no_knockout()
		}),
		Some(Ink {
			wobble: 0.45,
			boil: 0.0,
			..red_ink(55.0)
		}),
	)]);
	renderer.advance(0.0);
	let low_freq = Image {
		pixels: renderer.render_to_rgba(128, 128).unwrap(),
		width: 128,
	};
	assert_ne!(
		half(&low_freq, 0..64),
		left[0],
		"wobble_frequency must affect outline wobble"
	);
}

#[test]
fn print_mode_keeps_outlining_small_parts_above_half_pixel() {
	let Some(gpu) = gpu() else { return };
	let mut renderer = lit_sphere(gpu, None);
	// Scale 0.06 at z = 3 projects to ~1.5 px radius (< 4 px Toon cull, >= 0.5
	// px Print floor).
	renderer.world_mut().apply([Op::SetTransform {
		node: NodeId(1),
		transform: Transform::IDENTITY.with_scale(0.06),
	}]);

	renderer
		.world_mut()
		.apply([style(Shading::Toon(flat_toon(3)), Some(red_ink(3.0)))]);
	let _ = draw(&mut renderer);
	assert_eq!(
		renderer.stats().outlines,
		0,
		"Toon culls small parts under MIN_OUTLINED_RADIUS"
	);

	renderer.world_mut().apply([style(
		Shading::Print(print_no_knockout()),
		Some(red_ink(3.0)),
	)]);
	let print_small = draw(&mut renderer);
	assert_eq!(
		renderer.stats().outlines,
		1,
		"Print keeps outlining small parts above 0.5 px"
	);
	assert!(ink_count(&print_small) > 0, "small part has visible ink");

	// Sub-pixel speck (~0.12 px radius) is still dropped.
	renderer.world_mut().apply([Op::SetTransform {
		node: NodeId(1),
		transform: Transform::IDENTITY.with_scale(0.005),
	}]);
	let _ = draw(&mut renderer);
	assert_eq!(renderer.stats().outlines, 0, "sub-pixel speck is dropped");
}

const PAPER_COLOR: [f32; 3] = [0.95, 0.90, 0.20];

fn is_paper(pixel: [u8; 4]) -> bool {
	near(pixel, exact_srgb(PAPER_COLOR), 12)
}

fn paper_count(image: &Image) -> usize {
	(0..image.width)
		.flat_map(|y| (0..image.width).map(move |x| (x, y)))
		.filter(|(x, y)| is_paper(image.at(*x, *y)))
		.count()
}

fn render_size(renderer: &mut Renderer, size: u32) -> Image {
	renderer.advance(0.0);
	Image {
		pixels: renderer.render_to_rgba(size, size).unwrap(),
		width: size,
	}
}

#[test]
fn print_paper_knockout_frames_figure_outside_key_line() {
	let Some(gpu) = gpu() else { return };
	let blue = [0.10, 0.20, 0.85];
	let mut renderer = lit_sphere(gpu, None);
	renderer.world_mut().apply([
		Op::SetBackdrop(Backdrop::Color([0.04, 0.04, 0.08, 1.0])),
		Op::SetLight(Light {
			direction: [0.0, 0.0, -1.0],
			shadow: 0.0,
			..Light::default()
		}),
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(blue),
				mode: Some(PrintMode::Flat),
				..PrintOverride::default()
			},
		},
		style(
			Shading::Print(Print {
				paper: PAPER_COLOR,
				knockout_width: 45.0,
				..Print::default()
			}),
			Some(Ink {
				wobble: 0.0,
				..red_ink(35.0)
			}),
		),
	]);

	let image = render_size(&mut renderer, 128);
	// Center is untouched blue fill.
	assert!(near(image.at(64, 64), exact_srgb(blue), 4));
	// Scanning right from center: blue -> red key ink -> yellow paper knockout
	// -> backdrop.
	let mut saw_ink = false;
	let mut saw_paper_after_ink = false;
	for x in 64..128 {
		let p = image.at(x, 64);
		if is_ink(p) {
			assert!(
				!saw_paper_after_ink,
				"key ink must sit inside paper knockout at x={x}"
			);
			saw_ink = true;
		} else if is_paper(p) {
			assert!(saw_ink, "paper knockout must sit outside key ink at x={x}");
			saw_paper_after_ink = true;
		}
	}
	assert!(saw_ink && saw_paper_after_ink);

	// Same ordering scanning left from center.
	let mut saw_ink_l = false;
	let mut saw_paper_l = false;
	for x in (0..=64).rev() {
		let p = image.at(x, 64);
		if is_ink(p) {
			assert!(!saw_paper_l, "left ink inside paper at x={x}");
			saw_ink_l = true;
		} else if is_paper(p) {
			assert!(saw_ink_l, "left paper outside ink at x={x}");
			saw_paper_l = true;
		}
	}
	assert!(saw_ink_l && saw_paper_l);
}

#[test]
fn print_paper_knockout_separates_overlapping_figures() {
	let Some(gpu) = gpu() else { return };
	let blue = [0.10, 0.25, 0.85];
	let green = [0.10, 0.75, 0.20];
	let mut renderer = renderer(gpu);
	let sphere = fixture::lit_sphere(32, None);
	spawn(&mut renderer, 1, 1, &sphere);
	renderer.world_mut().apply([
		Op::SetBackdrop(Backdrop::Color([0.04, 0.04, 0.08, 1.0])),
		Op::SetLight(Light {
			direction: [0.0, 0.0, -1.0],
			shadow: 0.0,
			..Light::default()
		}),
		// Near figure on the left-front.
		Op::SetTransform {
			node: NodeId(1),
			transform: Transform::from_translation(-0.22, 0.0, 0.45).with_scale(0.55),
		},
		// Far figure overlapping directly behind the near figure.
		Op::Spawn {
			node: NodeId(2),
			asset: Some(AssetId(1)),
			parent: None,
		},
		Op::SetTransform {
			node: NodeId(2),
			transform: Transform::from_translation(0.25, 0.0, -0.75).with_scale(1.05),
		},
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(blue),
				mode: Some(PrintMode::Flat),
				..PrintOverride::default()
			},
		},
		Op::SetPrint {
			node: NodeId(2),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(green),
				mode: Some(PrintMode::Flat),
				..PrintOverride::default()
			},
		},
		style(
			Shading::Print(Print {
				paper: PAPER_COLOR,
				knockout_width: 55.0,
				..Print::default()
			}),
			Some(Ink {
				wobble: 0.0,
				..red_ink(25.0)
			}),
		),
	]);

	let image = render_size(&mut renderer, 128);
	// Scan across the equator from the near blue figure into the far green
	// figure: per-figure stencil refs must put a paper knockout border
	// between them.
	let mut seen_blue = false;
	let mut paper_between = 0_usize;
	let mut seen_green_after_paper = false;
	for x in 20..115 {
		let p = image.at(x, 64);
		if near(p, exact_srgb(blue), 12) {
			seen_blue = true;
		} else if seen_blue && is_paper(p) && !seen_green_after_paper {
			paper_between += 1;
		} else if seen_blue && paper_between > 0 && near(p, exact_srgb(green), 12) {
			seen_green_after_paper = true;
		}
	}
	assert!(
		seen_blue && paper_between >= 3 && seen_green_after_paper,
		"expected paper separation between near blue and far green figures \
		 (seen_blue={seen_blue}, paper_between={paper_between}, \
		 seen_green={seen_green_after_paper})"
	);
}

#[test]
fn print_attached_child_shares_stencil_ref_with_no_interior_seam() {
	let Some(gpu) = gpu() else { return };
	let blue = [0.10, 0.25, 0.85];
	let green = [0.10, 0.75, 0.20];
	let mut renderer = renderer(gpu);
	let sphere = fixture::lit_sphere(32, None);
	spawn(&mut renderer, 1, 1, &sphere);
	renderer.world_mut().apply([
		Op::SetBackdrop(Backdrop::Color([0.04, 0.04, 0.08, 1.0])),
		Op::SetLight(Light {
			direction: [0.0, 0.0, -1.0],
			shadow: 0.0,
			..Light::default()
		}),
		Op::SetTransform {
			node: NodeId(1),
			transform: Transform::IDENTITY.with_scale(0.90),
		},
		// Child weapon/part attached to NodeId(1) crossing right in front of its torso.
		Op::Spawn {
			node: NodeId(2),
			asset: Some(AssetId(1)),
			parent: Some(NodeId(1)),
		},
		Op::SetTransform {
			node: NodeId(2),
			transform: Transform::from_translation(0.0, 0.0, 0.55).with_scale(0.35),
		},
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(blue),
				mode: Some(PrintMode::Flat),
				..PrintOverride::default()
			},
		},
		Op::SetPrint {
			node: NodeId(2),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(green),
				mode: Some(PrintMode::Flat),
				..PrintOverride::default()
			},
		},
		style(
			Shading::Print(Print {
				paper: PAPER_COLOR,
				knockout_width: 60.0,
				..Print::default()
			}),
			None,
		),
	]);

	let image = render_size(&mut renderer, 128);
	// Child sits on top at center; parent surrounds it inside radius 22 px.
	assert!(near(image.at(64, 64), exact_srgb(green), 4));
	assert!(near(image.at(64, 48), exact_srgb(blue), 4));
	// Strictly inside the parent figure's circular disc (radius <= 19 px),
	// the attached child must produce ZERO interior paper seam pixels.
	let interior_paper = (0..128)
		.flat_map(|y| (0..128).map(move |x| (x, y)))
		.filter(|&(x, y)| {
			let dx = x as i32 - 64;
			let dy = y as i32 - 64;
			dx * dx + dy * dy <= 19 * 19 && is_paper(image.at(x, y))
		})
		.count();
	assert_eq!(
		interior_paper, 0,
		"attached child must not draw an interior paper seam over its parent"
	);
	// Outside the combined silhouette, the figure still gets a paper knockout.
	assert!(paper_count(&image) > 100, "outer silhouette has knockout");
}

#[test]
fn print_paper_knockout_frames_open_single_sided_sheet() {
	let Some(gpu) = gpu() else { return };
	let blue = [0.10, 0.25, 0.85];
	let mut renderer = renderer(gpu);
	spawn(&mut renderer, 1, 1, &fixture::open_sheet());
	renderer.world_mut().apply([
		Op::SetBackdrop(Backdrop::Color([0.04, 0.04, 0.08, 1.0])),
		Op::SetLight(Light {
			direction: [0.0, 0.0, -1.0],
			shadow: 0.0,
			..Light::default()
		}),
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(blue),
				mode: Some(PrintMode::Flat),
				..PrintOverride::default()
			},
		},
		style(
			Shading::Print(Print {
				paper: PAPER_COLOR,
				knockout_width: 30.0,
				..Print::default()
			}),
			None,
		),
	]);

	let check_sheet = |image: &Image, label: &str| {
		assert!(
			near(image.at(128, 128), exact_srgb(blue), 4),
			"{label}: center must stay blue, got {:?}",
			image.at(128, 128)
		);
		let left = (0..128).filter(|&x| is_paper(image.at(x, 128))).count();
		let right = (128..256).filter(|&x| is_paper(image.at(x, 128))).count();
		let top = (0..128).filter(|&y| is_paper(image.at(128, y))).count();
		let bottom = (128..256).filter(|&y| is_paper(image.at(128, y))).count();
		assert!(
			left >= 3 && right >= 3 && top >= 3 && bottom >= 3,
			"{label}: all 4 open edges must be framed in paper (left={left}, right={right}, \
			 top={top}, bottom={bottom})"
		);
	};

	check_sheet(&render_size(&mut renderer, 256), "front view");

	// View the same single-sided sheet from the back: all 4 edges stay framed.
	renderer.world_mut().apply([Op::SetCamera {
		transform: Transform::from_translation(0.0, 0.0, -3.0).with_yaw(std::f32::consts::PI),
		fov_y: 0.8,
		near: 0.1,
		far: 50.0,
	}]);
	check_sheet(&render_size(&mut renderer, 256), "back view");
}

#[test]
fn print_knockout_opt_out_draws_no_halo() {
	let Some(gpu) = gpu() else { return };
	let blue = [0.10, 0.25, 0.85];
	let mut renderer = renderer(gpu);
	let sphere = fixture::lit_sphere(32, None);
	spawn(&mut renderer, 1, 1, &sphere);
	renderer.world_mut().apply([
		Op::SetBackdrop(Backdrop::Color([0.04, 0.04, 0.08, 1.0])),
		Op::SetTransform {
			node: NodeId(1),
			transform: Transform::from_translation(-0.65, 0.0, 0.0).with_scale(0.45),
		},
		Op::Spawn {
			node: NodeId(2),
			asset: Some(AssetId(1)),
			parent: None,
		},
		Op::SetTransform {
			node: NodeId(2),
			transform: Transform::from_translation(0.65, 0.0, 0.0).with_scale(0.45),
		},
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(blue),
				mode: Some(PrintMode::Flat),
				knockout: Some(true),
				..PrintOverride::default()
			},
		},
		Op::SetPrint {
			node: NodeId(2),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(blue),
				mode: Some(PrintMode::Flat),
				knockout: Some(false),
				..PrintOverride::default()
			},
		},
		style(
			Shading::Print(Print {
				paper: PAPER_COLOR,
				knockout_width: 55.0,
				..Print::default()
			}),
			None,
		),
	]);

	let image = render_size(&mut renderer, 128);
	let left_paper = (0..128)
		.flat_map(|y| (0..64).map(move |x| (x, y)))
		.filter(|(x, y)| is_paper(image.at(*x, *y)))
		.count();
	let right_paper = (0..128)
		.flat_map(|y| (64..128).map(move |x| (x, y)))
		.filter(|(x, y)| is_paper(image.at(*x, *y)))
		.count();
	assert!(left_paper > 100, "opted-in figure has paper: {left_paper}");
	assert_eq!(right_paper, 0, "opted-out figure has zero paper");
}

#[test]
fn print_knockout_zero_width_keeps_depth32_and_toggles_cleanly() {
	let Some(gpu) = gpu() else { return };
	let blue = [0.10, 0.25, 0.85];
	let mut renderer = lit_sphere(gpu, None);
	renderer.world_mut().apply([
		Op::SetBackdrop(Backdrop::Color([0.04, 0.04, 0.08, 1.0])),
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(blue),
				mode: Some(PrintMode::Flat),
				..PrintOverride::default()
			},
		},
		style(
			Shading::Print(Print {
				paper: PAPER_COLOR,
				knockout_width: 0.0,
				..Print::default()
			}),
			None,
		),
	]);

	let before = draw(&mut renderer);
	assert_eq!(paper_count(&before), 0);

	// Toggle stencil knockout on, then back off: Depth32Float path stays
	// identical.
	renderer.world_mut().apply([style(
		Shading::Print(Print {
			paper: PAPER_COLOR,
			knockout_width: 55.0,
			..Print::default()
		}),
		None,
	)]);
	let with_knockout = draw(&mut renderer);
	assert!(paper_count(&with_knockout) > 40);

	renderer.world_mut().apply([style(
		Shading::Print(Print {
			paper: PAPER_COLOR,
			knockout_width: 0.0,
			..Print::default()
		}),
		None,
	)]);
	let after = draw(&mut renderer);
	assert_eq!(before.pixels, after.pixels);
}

#[test]
fn print_paper_knockout_respects_real_world_depth_occlusion() {
	let Some(gpu) = gpu() else { return };
	let blue = [0.10, 0.25, 0.85];
	let mut renderer = lit_sphere(gpu, None);
	renderer.world_mut().apply([
		Op::SetBackdrop(Backdrop::Color([0.04, 0.04, 0.08, 1.0])),
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(blue),
				mode: Some(PrintMode::Flat),
				..PrintOverride::default()
			},
		},
		style(
			Shading::Print(Print {
				paper: PAPER_COLOR,
				knockout_width: 60.0,
				..Print::default()
			}),
			None,
		),
	]);
	let near_wall = solid_texture(gpu, wgpu::TextureFormat::R32Float, &1.0_f32.to_le_bytes());
	let far_wall = solid_texture(gpu, wgpu::TextureFormat::R32Float, &10.0_f32.to_le_bytes());

	for (depth, visible) in [(&near_wall, false), (&far_wall, true)] {
		let occlusion = Occlusion {
			depth,
			transform: Mat3::IDENTITY,
			scale: 1.0,
			tolerance: 0.05,
		};
		let image = render_with(&mut renderer, &[], Background::Scene, Some(occlusion));
		assert_eq!(
			paper_count(&image) > 0,
			visible,
			"knockout visibility mismatch for visible={visible}"
		);
	}
}

#[test]
fn print_paper_knockout_handles_alpha_mask_and_mirrored_transform() {
	let Some(gpu) = gpu() else { return };
	let blue = [0.10, 0.25, 0.85];

	// (a) AlphaMode::Mask cutout half must NOT be filled with paper knockout.
	let mut masked = renderer(gpu);
	spawn(&mut masked, 1, 1, &fixture::masked_quads());
	masked.world_mut().apply([
		Op::SetBackdrop(Backdrop::Color([0.04, 0.04, 0.08, 1.0])),
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(blue),
				mode: Some(PrintMode::Flat),
				..PrintOverride::default()
			},
		},
		style(
			Shading::Print(Print {
				paper: PAPER_COLOR,
				knockout_width: 55.0,
				..Print::default()
			}),
			None,
		),
	]);
	let mask_img = render_size(&mut masked, 128);
	// Center of the right cutout quad (~x=86, y=64) must stay dark backdrop,
	// never filled with paper.
	let right_cutout_paper = (44..84)
		.flat_map(|y| (74..104).map(move |x| (x, y)))
		.filter(|(x, y)| is_paper(mask_img.at(*x, *y)))
		.count();
	assert_eq!(
		right_cutout_paper, 0,
		"AlphaMode::Mask cutout must not be filled with paper knockout"
	);
	let left_paper = (0..128)
		.flat_map(|y| (0..64).map(move |x| (x, y)))
		.filter(|(x, y)| is_paper(mask_img.at(*x, *y)))
		.count();
	assert!(
		left_paper > 40,
		"opaque half of masked mesh still gets paper knockout: {left_paper}"
	);

	// (b) Negative-determinant (mirrored) transform keeps an outer paper ring
	// instead of filling the front of the mesh with solid paper.
	let mut mirrored = renderer(gpu);
	spawn(&mut mirrored, 1, 1, &fixture::double_sided_sphere(32));
	mirrored.world_mut().apply([
		Op::SetBackdrop(Backdrop::Color([0.04, 0.04, 0.08, 1.0])),
		Op::SetTransform {
			node: NodeId(1),
			transform: Transform {
				scale: [-0.8, 0.8, 0.8],
				..Transform::IDENTITY
			},
		},
		Op::SetPrint {
			node: NodeId(1),
			target: PrintTarget::All,
			print: PrintOverride {
				lit: Some(blue),
				mode: Some(PrintMode::Flat),
				..PrintOverride::default()
			},
		},
		style(
			Shading::Print(Print {
				paper: PAPER_COLOR,
				knockout_width: 50.0,
				..Print::default()
			}),
			None,
		),
	]);
	let mir_img = render_size(&mut mirrored, 128);
	assert!(
		near(mir_img.at(64, 64), exact_srgb(blue), 4),
		"mirrored sphere center must stay blue, got {:?}",
		mir_img.at(64, 64)
	);
	let left_ring = (0..64).filter(|&x| is_paper(mir_img.at(x, 64))).count();
	let right_ring = (64..128).filter(|&x| is_paper(mir_img.at(x, 64))).count();
	assert!(
		left_ring >= 3 && right_ring >= 3,
		"mirrored sphere must have outer paper ring (left={left_ring}, right={right_ring})"
	);
}

#[test]
fn outline_smooth_rings_loads_and_renders_closed_smoothed_hull() {
	let Some(gpu) = gpu() else { return };
	// Build a hard-edged 3-band faceted cube (3 rings along Y so middle rings
	// sit 1 hop from the top/bottom 90 deg caps) and compare rings=0 vs
	// rings=2.
	let mut builder = fixture::Builder::default();
	let (mut positions, mut normals, mut indices) = (Vec::new(), Vec::new(), Vec::new());
	let bands: [(f32, f32, f32, f32); 3] = [
		(-0.5, -0.2, 0.5, 0.5),
		(-0.2, 0.2, 0.5, 0.5),
		(0.2, 0.5, 0.5, 0.35),
	];
	let mut push_quad = |quad: [[f32; 3]; 4], normal: [f32; 3]| {
		let base = (positions.len() / 3) as u16;
		for p in quad {
			positions.extend_from_slice(&p);
			normals.extend_from_slice(&normal);
		}
		indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
	};
	for &(y0, y1, r0, r1) in &bands {
		let side_dirs: [([f32; 2], [f32; 2], [f32; 3]); 4] = [
			([-1.0, 1.0], [1.0, 1.0], [0.0, 0.0, 1.0]),
			([1.0, 1.0], [1.0, -1.0], [1.0, 0.0, 0.0]),
			([1.0, -1.0], [-1.0, -1.0], [0.0, 0.0, -1.0]),
			([-1.0, -1.0], [-1.0, 1.0], [-1.0, 0.0, 0.0]),
		];
		for (a, b, n) in side_dirs {
			push_quad(
				[
					[a[0] * r0, y0, a[1] * r0],
					[b[0] * r0, y0, b[1] * r0],
					[b[0] * r1, y1, b[1] * r1],
					[a[0] * r1, y1, a[1] * r1],
				],
				n,
			);
		}
	}
	// Top and bottom caps.
	push_quad(
		[
			[-0.35, 0.5, 0.35],
			[0.35, 0.5, 0.35],
			[0.35, 0.5, -0.35],
			[-0.35, 0.5, -0.35],
		],
		[0.0, 1.0, 0.0],
	);
	push_quad(
		[
			[-0.5, -0.5, -0.5],
			[0.5, -0.5, -0.5],
			[0.5, -0.5, 0.5],
			[-0.5, -0.5, 0.5],
		],
		[0.0, -1.0, 0.0],
	);
	let p = builder.floats(&positions, 3, true);
	let n = builder.floats(&normals, 3, false);
	let i = builder.u16s(&indices, 1);
	let glb = builder.glb(&format!(
		r#""asset":{{"version":"2.0"}},"scenes":[{{"nodes":[0]}}],"nodes":[{{"name":"bevel","mesh":0}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":{p},"NORMAL":{n}}},"indices":{i},"material":0}}]}}],"materials":[{{"name":"clay","pbrMetallicRoughness":{{"baseColorFactor":[0.5,0.5,0.5,1]}}}}]"#
	));

	let render_with_rings = |rings: Option<u8>| {
		let asset = Arc::new(
			Asset::from_slice_with_options(
				&glb,
				LoadOptions {
					outline_smooth_rings: rings,
				},
			)
			.unwrap(),
		);
		let mut r = renderer(gpu);
		let world = r.world_mut();
		world.apply([
			Op::LoadAsset {
				asset: AssetId(1),
				path: "bevel.glb".into(),
			},
			Op::Spawn {
				node: NodeId(1),
				asset: Some(AssetId(1)),
				parent: None,
			},
			Op::SetTransform {
				node: NodeId(1),
				transform: Transform::IDENTITY.with_yaw(0.45),
			},
			Op::SetBackdrop(Backdrop::Color([0.04, 0.04, 0.08, 1.0])),
			Op::SetPrint {
				node: NodeId(1),
				target: PrintTarget::All,
				print: PrintOverride {
					lit: Some([0.1, 0.25, 0.85]),
					mode: Some(PrintMode::Flat),
					..PrintOverride::default()
				},
			},
			style(
				Shading::Print(print_no_knockout()),
				Some(Ink {
					wobble: 0.0,
					..red_ink(30.0)
				}),
			),
		]);
		world.take_loads();
		world.asset_loaded(AssetId(1), Ok(asset));
		world.take_events();
		render_size(&mut r, 128)
	};

	let plain = render_with_rings(None);
	let zero = render_with_rings(Some(0));
	let smooth = render_with_rings(Some(2));
	assert_eq!(plain.pixels, zero.pixels, "None and Some(0) match exactly");
	assert!(ink_count(&smooth) > 200, "smoothed hull draws full outline");
	assert_ne!(
		plain.pixels, smooth.pixels,
		"2 smoothing rings visibly adjust faceted hull corners"
	);
}
