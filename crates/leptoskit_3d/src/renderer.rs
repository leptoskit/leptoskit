//! The forward renderer: culls, sorts, and draws a [`World`] into any wgpu
//! texture, for one view or several (stereo), over a clear colour or a
//! camera image.

use std::collections::HashMap;
use std::sync::Arc;

use glam::Mat3;
use glam::Mat4;
use glam::Vec3;
use glam::Vec4;

use crate::asset::Aabb;
use crate::asset::AlphaMode;
use crate::gpu::Error;
use crate::gpu::Gpu;
use crate::gpu::Pipelines;
use crate::gpu::Shared;
use crate::gpu::Variant;
use crate::protocol::Backdrop;
use crate::protocol::Blend;
use crate::raycast;
use crate::upload::*;
use crate::world::World;

/// The most views one target may have.
pub const MAX_VIEWS: usize = 4;
const GLOBALS_STRIDE: u64 = 512;
const NONE: u32 = u32::MAX;

/// One view of the scene: where it looks from, how it projects, and the
/// part of the target it fills.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewParams {
	/// World to view space, right-handed, looking along -z.
	pub view: Mat4,
	/// View to clip space, with depth from 0 to 1.
	pub projection: Mat4,
	/// `[x, y, width, height]` in the target's pixels, from its top-left.
	pub viewport: [f32; 4],
}

/// Where a target's views draw.
#[derive(Clone, Copy, Debug)]
pub enum TargetColor<'a> {
	/// One texture; each view draws into its own viewport of it, such as
	/// the halves of a side-by-side stereo image.
	View(&'a wgpu::TextureView),
	/// One texture (or array layer) per view, in the views' order, such as
	/// the layers of an `OpenXR` swapchain image.
	Layers(&'a [&'a wgpu::TextureView]),
}

/// A camera frame's pixels.
#[derive(Clone, Copy, Debug)]
pub enum CameraPlanes<'a> {
	/// Luma in one texture's red channel and chroma in another's red and
	/// green, full-range BT.601, as `ARKit` and `ARCore` deliver frames.
	YCbCr {
		/// The luma plane.
		y: &'a wgpu::TextureView,
		/// The chroma plane, usually half the luma's size.
		cbcr: &'a wgpu::TextureView,
	},
	/// One RGBA texture, read as linear light: use a view in an sRGB format
	/// for sRGB-encoded pixels.
	Rgba(&'a wgpu::TextureView),
}

/// A camera image drawn behind the scene.
#[derive(Clone, Copy, Debug)]
pub struct CameraImage<'a> {
	/// The image.
	pub planes: CameraPlanes<'a>,
	/// Maps a point of the viewport (`x` and `y` from 0 to 1, y down, as
	/// `[x, y, 1]`) to the image's texture coordinates: the display
	/// transform an AR session reports for the view's orientation and
	/// aspect.
	pub transform: Mat3,
}

/// What a target draws behind the scene.
// A camera image is larger than a colour, but a target is borrowed for one
// frame, never stored, so boxing it would only add an allocation.
#[allow(variant_size_differences)]
#[derive(Clone, Copy, Debug, Default)]
pub enum Background<'a> {
	/// What the scene's ops chose. A scene that asks for the camera draws
	/// transparent where the host passes no camera image.
	#[default]
	Scene,
	/// A clear colour, linear RGBA with straight alpha.
	Color([f32; 4]),
	/// A camera image.
	Camera(CameraImage<'a>),
}

/// Real-world depth that hides the parts of the scene behind it, for
/// augmented reality.
#[derive(Clone, Copy, Debug)]
pub struct Occlusion<'a> {
	/// Distance from the camera per texel, in a single-channel float
	/// texture (`R32Float` or `R16Float`). Zero means unknown.
	pub depth: &'a wgpu::TextureView,
	/// Maps a point of the viewport (`[x, y, 1]`, y down, from 0 to 1) to
	/// the depth texture's coordinates.
	pub transform: Mat3,
	/// Metres per unit of the texture's values; 1 for metres, 0.001 for
	/// millimetres.
	pub scale: f32,
	/// How far behind the real surface the scene may be before it hides,
	/// in metres.
	pub tolerance: f32,
}

/// A texture to draw into, with the views to draw and what goes behind
/// them.
#[derive(Clone, Copy, Debug)]
pub struct RenderTarget<'a> {
	/// The texture views, single-sampled, in [`RenderTarget::format`].
	pub color: TargetColor<'a>,
	/// Their format.
	pub format: wgpu::TextureFormat,
	/// Their size in pixels.
	pub size: [u32; 2],
	/// The views; empty draws one view from the scene's camera over the
	/// whole target.
	pub views: &'a [ViewParams],
	/// What draws behind the scene.
	pub background: Background<'a>,
	/// Real-world depth, for occlusion.
	pub occlusion: Option<Occlusion<'a>>,
}

impl<'a> RenderTarget<'a> {
	/// The whole of `view`, `width` by `height` pixels in `format`, from the
	/// scene's camera, over the scene's background.
	pub fn new(
		view: &'a wgpu::TextureView,
		format: wgpu::TextureFormat,
		width: u32,
		height: u32,
	) -> Self {
		Self {
			color: TargetColor::View(view),
			format,
			size: [width, height],
			views: &[],
			background: Background::Scene,
			occlusion: None,
		}
	}
}

/// What one frame drew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
	/// Draw calls, per view.
	pub draws: u32,
	/// Mesh instances drawn, per view.
	pub instances: u32,
	/// Triangles drawn, per view.
	pub triangles: u64,
	/// Mesh instances skipped because no view could see them.
	pub culled: u32,
	/// Soft shadows drawn.
	pub shadows: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GlobalsUniform {
	view_projection: [[f32; 4]; 4],
	view: [[f32; 4]; 4],
	camera: [f32; 4],
	light_direction: [f32; 4],
	light_color: [f32; 4],
	sh: [[f32; 4]; 9],
	params: [f32; 4],
	viewport: [f32; 4],
	occlusion_x: [f32; 4],
	occlusion_y: [f32; 4],
	occlusion_params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct BackgroundUniform {
	transform_x: [f32; 4],
	transform_y: [f32; 4],
	mode: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct ShadowRecord {
	center: [f32; 4],
	radii: [f32; 4],
}

/// The six planes of a view's frustum, each `ax + by + cz + d >= 0` inside.
struct Frustum([Vec4; 6]);

impl Frustum {
	fn new(view_projection: &Mat4) -> Self {
		let rows = [
			view_projection.row(0),
			view_projection.row(1),
			view_projection.row(2),
			view_projection.row(3),
		];
		Self([
			rows[3] + rows[0],
			rows[3] - rows[0],
			rows[3] + rows[1],
			rows[3] - rows[1],
			rows[2],
			rows[3] - rows[2],
		])
	}

	fn sees(&self, bounds: &Aabb) -> bool {
		self.0.iter().all(|plane| {
			let normal = plane.truncate();
			let corner = Vec3::select(normal.cmpge(Vec3::ZERO), bounds.max, bounds.min);
			normal.dot(corner) + plane.w >= 0.0
		})
	}
}

/// A row of the 3×3 as `[a, b, c, 0]`, for the shader's `dot(row.xyz, p)`.
fn row(matrix: &Mat3, index: usize) -> [f32; 4] {
	let row = matrix.row(index);
	[row.x, row.y, row.z, 0.0]
}

fn linear_to_srgb(value: f32) -> f32 {
	if value <= 0.003_130_8 {
		value * 12.92
	} else {
		1.055 * value.powf(1.0 / 2.4) - 0.055
	}
}

/// A mesh primitive ready to draw, before it is merged with others like it.
struct Item {
	variant: Variant,
	asset: usize,
	material: usize,
	first_index: u32,
	index_count: u32,
	vertex_offset: u64,
	skin_offset: Option<u64>,
	morphed: bool,
	/// Whether it can share an instanced draw with identical items.
	batchable: bool,
	depth: f32,
	record: InstanceRecord,
}

struct Draw {
	variant: Variant,
	asset: usize,
	material: usize,
	first_index: u32,
	index_count: u32,
	vertex_offset: u64,
	skin_offset: Option<u64>,
	morphed: bool,
	instances: std::ops::Range<u32>,
}

/// What every view can see, and the first view's eye for sorting.
struct Sight {
	frustums: Vec<Frustum>,
	eye: Mat4,
}

impl Sight {
	fn sees(&self, bounds: &Aabb) -> bool {
		self.frustums.iter().any(|frustum| frustum.sees(bounds))
	}
}

/// One model instance's state for a frame, copied out of the world.
struct ModelState {
	asset: Arc<crate::asset::Asset>,
	weights: Vec<Vec<f32>>,
	overrides: Vec<crate::world::Overrides>,
	hidden: Vec<bool>,
	worlds: Vec<Mat4>,
	/// Per node: a skinned part's joint matrices and posed bounds.
	skins: Vec<Option<(Vec<Mat4>, Aabb)>>,
	cast_shadow: bool,
	/// The height of the instance's origin, where its shadow falls.
	ground: f32,
}

impl ModelState {
	/// The state of the node `id`, if it is a visible, loaded model.
	fn of(world: &World, id: crate::protocol::NodeId) -> Option<Self> {
		let node = &world.nodes[&id];
		let instance = node.instance.as_ref().filter(|_| !node.hidden)?;
		let asset = instance.asset.clone()?;
		let pose = instance.pose.as_ref()?;
		let skins = (0..asset.nodes.len())
			.map(|index| {
				raycast::joint_matrices(&asset, instance, index)
					.zip(raycast::skinned_bounds(&asset, instance, index))
			})
			.collect();

		Some(Self {
			weights: pose.weights.clone(),
			overrides: instance.overrides.clone(),
			hidden: instance.hidden.clone(),
			worlds: instance.world.clone(),
			skins,
			cast_shadow: instance.cast_shadow,
			ground: node.world.w_axis.y,
			asset,
		})
	}
}

/// A mesh node of a model: which one, its asset's slot, and its joints.
struct Part<'a> {
	model: &'a ModelState,
	index: usize,
	slot: usize,
	joints: u32,
}

/// A primitive's instance attributes: its matrix, its part's overrides,
/// and where its joints and morph weights start (`[joints, morph]`).
fn instance_record(
	part: &crate::world::Overrides,
	material: &crate::asset::Material,
	model: Mat4,
	[joints, morph]: [u32; 2],
) -> InstanceRecord {
	let emissive = match (part.emissive, part.emissive_strength) {
		(None, None) => [0.0, 0.0, 0.0, -1.0],
		(color, strength) => {
			let color = color.unwrap_or(material.emissive);
			let strength = strength.unwrap_or(material.emissive_strength);
			[color[0], color[1], color[2], strength]
		}
	};

	let rim = part.rim;

	InstanceRecord {
		model: model.to_cols_array_2d(),
		tint: [
			part.color[0],
			part.color[1],
			part.color[2],
			part.color[3] * part.opacity,
		],
		emissive,
		rim: [rim.color[0], rim.color[1], rim.color[2], rim.strength],
		params: [
			joints,
			morph,
			rim.power.to_bits(),
			u32::from(part.blend == Blend::Additive),
		],
	}
}

/// How much wider than its footprint a model's shadow is, so the soft edge
/// shows past the model.
const SHADOW_SPREAD: f32 = 1.3;
/// The smallest half-width a shadow has, in metres.
const SHADOW_MIN_RADIUS: f32 = 0.05;

/// The soft shadow under a model with `bounds` standing at `ground`, if it
/// casts one any view sees. It fades as the model rises.
fn shadow_record(bounds: Aabb, ground: f32, opacity: f32, sight: &Sight) -> Option<ShadowRecord> {
	if opacity <= 0.0 || bounds.is_empty() {
		return None;
	}

	let extent = (bounds.max - bounds.min) * 0.5;
	let radius_x = extent.x.max(SHADOW_MIN_RADIUS) * SHADOW_SPREAD;
	let radius_z = extent.z.max(SHADOW_MIN_RADIUS) * SHADOW_SPREAD;
	let height = (bounds.min.y - ground).max(0.0);
	let fade = (1.0 - height / (2.0 * radius_x.max(radius_z))).clamp(0.0, 1.0);
	let center = bounds.center();
	let area = Aabb {
		min: Vec3::new(center.x - radius_x, ground, center.z - radius_z),
		max: Vec3::new(center.x + radius_x, ground, center.z + radius_z),
	};

	(fade > 0.0 && sight.sees(&area)).then_some(ShadowRecord {
		center: [center.x, ground, center.z, opacity * fade],
		radii: [radius_x, radius_z, 0.0, 0.0],
	})
}

struct Attachments {
	key: (u32, u32, wgpu::TextureFormat, u32),
	color: Option<wgpu::TextureView>,
	depth: wgpu::TextureView,
}

/// Draws a [`World`] with wgpu.
///
/// It owns its scene: apply ops with [`Renderer::world_mut`], move time on
/// with [`Renderer::advance`], and draw with [`Renderer::render`]. Its
/// buffers and attachments are kept between frames, so keep one renderer
/// per scene. Every renderer on one [`Gpu`] shares its pipelines, its
/// environment, and its uploaded assets.
pub struct Renderer {
	gpu: Gpu,
	shared: Arc<Shared>,
	world: World,
	instances: Growable,
	shadows: Growable,
	globals: wgpu::Buffer,
	data: Option<(wgpu::Texture, wgpu::TextureView, u32)>,
	attachments: Option<Attachments>,
	items: Vec<Item>,
	texels: Vec<[f32; 4]>,
	shadow_records: Vec<ShadowRecord>,
	/// The uploaded assets this frame draws, which [`Item::asset`] indexes.
	assets: Vec<Arc<GpuAsset>>,
	asset_index: HashMap<usize, usize>,
	stats: FrameStats,
}

impl Renderer {
	/// A renderer with an empty scene, drawing on `gpu`.
	pub fn new(gpu: &Gpu) -> Self {
		Self::with_world(gpu, World::new())
	}

	/// A renderer drawing `world` on `gpu`.
	pub fn with_world(gpu: &Gpu, world: World) -> Self {
		let shared = gpu.shared().clone();
		let globals = gpu.device().create_buffer(&wgpu::BufferDescriptor {
			label: Some("leptoskit_3d globals"),
			size: GLOBALS_STRIDE * MAX_VIEWS as u64,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		Self {
			gpu: gpu.clone(),
			shared,
			world,
			instances: Growable::new("leptoskit_3d instances", wgpu::BufferUsages::VERTEX),
			shadows: Growable::new("leptoskit_3d shadows", wgpu::BufferUsages::VERTEX),
			globals,
			data: None,
			attachments: None,
			items: Vec::new(),
			texels: Vec::new(),
			shadow_records: Vec::new(),
			assets: Vec::new(),
			asset_index: HashMap::new(),
			stats: FrameStats::default(),
		}
	}

	/// The scene.
	pub fn world(&self) -> &World {
		&self.world
	}

	/// The scene, to apply ops and hand it assets.
	pub fn world_mut(&mut self) -> &mut World {
		&mut self.world
	}

	/// The device it draws on.
	pub fn gpu(&self) -> &Gpu {
		&self.gpu
	}

	/// Gives the scene back, to draw it on another device. Its assets upload
	/// again to the new renderer's device.
	pub fn into_world(self) -> World {
		self.world
	}

	/// What the last frame drew.
	pub fn stats(&self) -> FrameStats {
		self.stats
	}

	/// Moves the scene's time forward by `elapsed` seconds.
	pub fn advance(&mut self, elapsed: f32) {
		self.world.advance(elapsed);
	}

	/// Draws the scene into `target`, replacing what it held.
	pub fn render(&mut self, target: &RenderTarget<'_>) -> Result<FrameStats, Error> {
		let [width, height] = target.size;

		if width == 0 || height == 0 {
			return Ok(FrameStats::default());
		}

		let limit = self.gpu.device().limits().max_texture_dimension_2d;

		if width > limit || height > limit {
			return Err(Error::TooLarge {
				width,
				height,
				limit,
			});
		}

		let views = self.views(target)?;

		if let TargetColor::Layers(layers) = target.color
			&& layers.len() != views.len()
		{
			return Err(Error::Target("a layered target needs one layer per view"));
		}
		self.world
			.set_aspect(views[0].viewport[2] / views[0].viewport[3].max(1.0));
		self.build(&views);
		let draws = self.batch();
		self.upload_data();
		let shadow_count = self.upload_shadows(&views);

		self.write_globals(target, &views);
		let samples = self.gpu.samples(target.format);
		let pipelines = self
			.shared
			.pipelines(self.gpu.device(), target.format, samples);
		self.prepare_attachments(width, height, target.format, samples);
		let globals = self.globals_bind_group(target.occlusion.as_ref());
		let background = self.background_bind_group(target);
		let assets = std::mem::take(&mut self.assets);
		let mut encoder =
			self.gpu
				.device()
				.create_command_encoder(&wgpu::CommandEncoderDescriptor {
					label: Some("leptoskit_3d"),
				});
		let clear = self.clear_color(target);

		let passes: Vec<(&wgpu::TextureView, std::ops::Range<usize>)> = match target.color {
			TargetColor::View(view) => vec![(view, 0..views.len())],
			TargetColor::Layers(layers) => {
				layers
					.iter()
					.enumerate()
					.map(|(index, view)| (*view, index..index + 1))
					.collect()
			}
		};

		for (view, range) in passes {
			let attachments = self
				.attachments
				.as_ref()
				.expect("attachments were prepared");

			let (color_view, resolve) = match &attachments.color {
				Some(msaa) => (msaa, Some(view)),
				None => (view, None),
			};

			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("leptoskit_3d"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: color_view,
					depth_slice: None,
					resolve_target: resolve,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(clear),
						store: if resolve.is_some() {
							wgpu::StoreOp::Discard
						} else {
							wgpu::StoreOp::Store
						},
					},
				})],
				depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
					view: &attachments.depth,
					depth_ops: Some(wgpu::Operations {
						load: wgpu::LoadOp::Clear(1.0),
						store: wgpu::StoreOp::Discard,
					}),
					stencil_ops: None,
				}),
				..wgpu::RenderPassDescriptor::default()
			});

			for index in range {
				let [x, y, w, h] = views[index].viewport;
				pass.set_viewport(x, y, w, h, 0.0, 1.0);

				if let Some(background) = &background {
					pass.set_pipeline(&pipelines.background);
					pass.set_bind_group(0, background, &[]);
					pass.draw(0..3, 0..1);
				}

				let offset = (index as u64 * GLOBALS_STRIDE) as u32;
				pass.set_bind_group(0, &globals, &[offset]);
				self.draw_meshes(&mut pass, &pipelines, &assets, &draws, |draw| {
					!draw.variant.blend
				});

				if shadow_count > 0
					&& let Some(buffer) = &self.shadows.buffer
				{
					pass.set_pipeline(&pipelines.shadow);
					pass.set_vertex_buffer(0, buffer.slice(..));
					pass.draw(0..4, 0..shadow_count);
				}
				self.draw_meshes(&mut pass, &pipelines, &assets, &draws, |draw| {
					draw.variant.blend
				});
			}
		}

		self.gpu.queue().submit([encoder.finish()]);
		self.assets = assets;
		let instances = draws.iter().map(|draw| draw.instances.len() as u32).sum();
		self.stats = FrameStats {
			draws: draws.len() as u32,
			instances,
			triangles: draws
				.iter()
				.map(|draw| u64::from(draw.index_count / 3) * draw.instances.len() as u64)
				.sum(),
			culled: self.stats.culled,
			shadows: shadow_count,
		};
		Ok(self.stats)
	}

	fn views(&self, target: &RenderTarget<'_>) -> Result<Vec<ViewParams>, Error> {
		if target.views.len() > MAX_VIEWS {
			return Err(Error::Target("a target has more views than the limit"));
		}

		if !target.views.is_empty() {
			return Ok(target.views.to_vec());
		}

		let [width, height] = target.size;
		let (view, projection) = self.world.camera.matrices(width as f32 / height as f32);
		Ok(vec![ViewParams {
			view,
			projection,
			viewport: [0.0, 0.0, width as f32, height as f32],
		}])
	}
}

impl Renderer {
	/// The index of `asset` among this frame's assets, uploading it first if
	/// the device has no copy.
	fn asset_slot(&mut self, asset: &Arc<crate::asset::Asset>) -> usize {
		let key = Arc::as_ptr(asset) as usize;

		if let Some(index) = self.asset_index.get(&key) {
			return *index;
		}

		let uploaded = self
			.shared
			.asset(self.gpu.device(), self.gpu.queue(), asset);
		self.assets.push(uploaded);
		self.asset_index.insert(key, self.assets.len() - 1);
		self.assets.len() - 1
	}

	/// Writes `matrices` as three rows each, returning the first texel.
	fn push_joints(&mut self, matrices: &[Mat4]) -> u32 {
		let first = self.texels.len() as u32;

		for matrix in matrices {
			for index in 0..3 {
				self.texels.push(matrix.row(index).to_array());
			}
		}

		first
	}

	/// Writes a primitive's active morph weights after a header texel,
	/// returning the header's texel, or [`NONE`] when no weight is set.
	fn push_morphs(&mut self, base: u32, vertex_count: u32, weights: &[f32]) -> u32 {
		let active: Vec<(usize, f32)> = weights
			.iter()
			.copied()
			.enumerate()
			.filter(|(_, weight)| *weight != 0.0)
			.collect();

		if active.is_empty() {
			return NONE;
		}

		let header = self.texels.len() as u32;
		self.texels
			.push([base as f32, vertex_count as f32, active.len() as f32, 0.0]);

		for pair in active.chunks(2) {
			let second = pair.get(1).copied().unwrap_or((0, 0.0));
			self.texels
				.push([pair[0].0 as f32, pair[0].1, second.0 as f32, second.1]);
		}

		header
	}

	/// Collects every visible primitive of every instance, with its instance
	/// record, culling what no view sees.
	fn build(&mut self, views: &[ViewParams]) {
		self.items.clear();
		self.texels.clear();
		self.shadow_records.clear();
		self.assets.clear();
		self.asset_index.clear();
		// Texel 0 is never a valid offset, so a zeroed record reads nothing.
		self.texels.push([0.0; 4]);
		let sight = Sight {
			frustums: views
				.iter()
				.map(|view| Frustum::new(&(view.projection * view.view)))
				.collect(),
			eye: views[0].view,
		};
		let shadow = self.world.settings.light.shadow;

		let mut ids: Vec<_> = self.world.nodes.keys().copied().collect();
		ids.sort_unstable();
		self.stats.culled = 0;

		for id in ids {
			let Some(model) = ModelState::of(&self.world, id) else {
				continue;
			};
			let bounds = self.build_model(&model, &sight);

			if model.cast_shadow
				&& let Some(record) = shadow_record(bounds, model.ground, shadow, &sight)
			{
				self.shadow_records.push(record);
			}
		}
	}

	/// Adds a model's visible parts, returning its bounds in this pose.
	fn build_model(&mut self, model: &ModelState, sight: &Sight) -> Aabb {
		let slot = self.asset_slot(&model.asset);
		let mut bounds = Aabb::EMPTY;

		for (index, node) in model.asset.nodes.iter().enumerate() {
			let Some(mesh) = node.mesh else {
				continue;
			};

			if model.hidden[index] || model.overrides[index].opacity <= 0.0 {
				continue;
			}

			// A skinned part's joints, written once for all its primitives.
			let joints = match &model.skins[index] {
				Some((matrices, _)) => self.push_joints(matrices),
				None => NONE,
			};

			for primitive in &model.asset.meshes[mesh].primitives {
				let part = Part {
					model,
					index,
					slot,
					joints,
				};
				bounds = bounds.union(self.build_primitive(&part, primitive, sight));
			}
		}

		bounds
	}

	/// Adds one primitive's draw unless no view sees it, returning its bounds.
	fn build_primitive(
		&mut self,
		part: &Part<'_>,
		primitive: &crate::asset::Primitive,
		sight: &Sight,
	) -> Aabb {
		let model = part.model;
		let index = part.index;
		let skinned = primitive.skin_base.is_some() && model.skins[index].is_some();

		let bounds = match (&model.skins[index], skinned) {
			(Some((_, bounds)), true) => *bounds,
			_ => primitive.bounds.transformed(&model.worlds[index]),
		};

		let weights = &model.weights[index];
		// Weights outside 0 to 1 can push vertices past the bounds the loader
		// measured, so such parts are never culled.
		let unusual = weights.iter().any(|weight| !(0.0..=1.0).contains(weight));

		if !unusual && !sight.sees(&bounds) {
			self.stats.culled += 1;

			return bounds;
		}

		let morph = match primitive.morph {
			Some(range) if self.assets[part.slot].morphs.is_some() => {
				let active = &weights[..weights.len().min(range.targets as usize)];
				self.push_morphs(range.base / 2, primitive.vertex_count, active)
			}

			_ => NONE,
		};

		let material = &model.asset.materials[primitive.material];
		let record = instance_record(
			&model.overrides[index],
			material,
			if skinned {
				Mat4::IDENTITY
			} else {
				model.worlds[index]
			},
			[if skinned { part.joints } else { NONE }, morph],
		);
		let vertex_size = size_of::<crate::asset::Vertex>() as u64;
		let skin_size = size_of::<crate::asset::SkinVertex>() as u64;
		self.items.push(Item {
			variant: Variant {
				skinned,
				blend: material.alpha == AlphaMode::Blend
					|| record.tint[3] < 1.0
					|| record.params[3] != 0,
				double_sided: material.double_sided,
			},
			asset: part.slot,
			material: primitive.material,
			first_index: primitive.first_index,
			index_count: primitive.index_count,
			vertex_offset: u64::from(primitive.base_vertex) * vertex_size,
			skin_offset: primitive
				.skin_base
				.filter(|_| skinned)
				.map(|base| u64::from(base) * skin_size),
			morphed: morph != NONE,
			batchable: !skinned && morph == NONE,
			depth: -(sight.eye * bounds.center().extend(1.0)).z,
			record,
		});

		bounds
	}

	/// Sorts the items and merges identical opaque ones into instanced
	/// draws, writing the instance records in draw order.
	fn batch(&mut self) -> Vec<Draw> {
		let (mut opaque, mut transparent): (Vec<Item>, Vec<Item>) =
			self.items.drain(..).partition(|item| !item.variant.blend);
		// Opaque front to back within each pipeline and material, so the depth
		// test rejects hidden fragments early.
		opaque.sort_by(|a, b| {
			(a.variant, a.asset, a.material, a.first_index)
				.cmp(&(b.variant, b.asset, b.material, b.first_index))
				.then(a.depth.total_cmp(&b.depth))
		});
		transparent.sort_by(|a, b| b.depth.total_cmp(&a.depth));
		let mut records = Vec::with_capacity(opaque.len() + transparent.len());
		let mut draws: Vec<Draw> = Vec::new();

		for item in opaque.into_iter().chain(transparent) {
			let index = records.len() as u32;

			records.push(item.record);

			if let Some(last) = draws.last_mut()
				&& item.batchable
				&& !last.variant.blend
				&& last.variant == item.variant
				&& last.asset == item.asset
				&& last.material == item.material
				&& last.first_index == item.first_index
				&& last.vertex_offset == item.vertex_offset
				&& !last.morphed
				&& last.skin_offset.is_none()
				&& last.instances.end == index
			{
				last.instances.end += 1;
				continue;
			}
			draws.push(Draw {
				variant: item.variant,
				asset: item.asset,
				material: item.material,
				first_index: item.first_index,
				index_count: item.index_count,
				vertex_offset: item.vertex_offset,
				skin_offset: item.skin_offset,
				morphed: item.morphed,
				instances: index..index + 1,
			});
		}

		self.instances.upload(
			self.gpu.device(),
			self.gpu.queue(),
			bytemuck::cast_slice(&records),
		);
		draws
	}

	/// Uploads this frame's joint matrices and morph weights.
	fn upload_data(&mut self) {
		let rows = (self.texels.len() as u32).div_ceil(DATA_WIDTH).max(1);
		self.texels.resize((rows * DATA_WIDTH) as usize, [0.0; 4]);
		let replace = self
			.data
			.as_ref()
			.is_none_or(|(_, _, capacity)| *capacity < rows);

		if replace {
			let capacity = rows.next_power_of_two();
			let texture = self.gpu.device().create_texture(&wgpu::TextureDescriptor {
				label: Some("leptoskit_3d frame data"),
				size: wgpu::Extent3d {
					width: DATA_WIDTH,
					height: capacity,
					depth_or_array_layers: 1,
				},
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: wgpu::TextureFormat::Rgba32Float,
				usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
				view_formats: &[],
			});
			let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
			self.data = Some((texture, view, capacity));
		}

		if let Some((texture, ..)) = &self.data {
			self.gpu.queue().write_texture(
				texture.as_image_copy(),
				bytemuck::cast_slice(&self.texels),
				wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(DATA_WIDTH * 16),
					rows_per_image: None,
				},
				wgpu::Extent3d {
					width: DATA_WIDTH,
					height: rows,
					depth_or_array_layers: 1,
				},
			);
		}
	}

	fn upload_shadows(&mut self, _views: &[ViewParams]) -> u32 {
		let records = std::mem::take(&mut self.shadow_records);
		self.shadows.upload(
			self.gpu.device(),
			self.gpu.queue(),
			bytemuck::cast_slice(&records),
		);
		let count = records.len() as u32;
		self.shadow_records = records;
		count
	}

	fn write_globals(&self, target: &RenderTarget<'_>, views: &[ViewParams]) {
		let settings = &self.world.settings;
		let light = settings.light;
		let toward = -Vec3::from(light.direction).normalize_or(Vec3::NEG_Y);

		let (occlusion_x, occlusion_y, occlusion_params, occluding) = match &target.occlusion {
			Some(occlusion) => {
				(
					row(&occlusion.transform, 0),
					row(&occlusion.transform, 1),
					[occlusion.scale, occlusion.tolerance, 0.0, 0.0],
					1.0,
				)
			}
			None => ([0.0; 4], [0.0; 4], [0.0; 4], 0.0),
		};

		let mut bytes = vec![0_u8; GLOBALS_STRIDE as usize * views.len()];

		for (index, view) in views.iter().enumerate() {
			let camera = view.view.inverse().w_axis;
			let uniform = GlobalsUniform {
				view_projection: (view.projection * view.view).to_cols_array_2d(),
				view: view.view.to_cols_array_2d(),
				camera: camera.to_array(),
				light_direction: [toward.x, toward.y, toward.z, light.intensity],
				light_color: [
					light.color[0],
					light.color[1],
					light.color[2],
					light.ambient,
				],
				sh: self.shared.irradiance,
				params: [
					settings.exposure,
					if target.format.is_srgb() { 0.0 } else { 1.0 },
					occluding,
					(crate::environment::MIPS - 1) as f32,
				],
				viewport: view.viewport,
				occlusion_x,
				occlusion_y,
				occlusion_params,
			};

			let start = index * GLOBALS_STRIDE as usize;
			bytes[start..start + size_of::<GlobalsUniform>()]
				.copy_from_slice(bytemuck::bytes_of(&uniform));
		}

		self.gpu.queue().write_buffer(&self.globals, 0, &bytes);
	}

	fn prepare_attachments(
		&mut self,
		width: u32,
		height: u32,
		format: wgpu::TextureFormat,
		samples: u32,
	) {
		let key = (width, height, format, samples);

		if self
			.attachments
			.as_ref()
			.is_some_and(|attachments| attachments.key == key)
		{
			return;
		}
		let device = self.gpu.device();
		let texture = |label, format| {
			device
				.create_texture(&wgpu::TextureDescriptor {
					label: Some(label),
					size: wgpu::Extent3d {
						width,
						height,
						depth_or_array_layers: 1,
					},
					mip_level_count: 1,
					sample_count: samples,
					dimension: wgpu::TextureDimension::D2,
					format,
					usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
					view_formats: &[],
				})
				.create_view(&wgpu::TextureViewDescriptor::default())
		};
		self.attachments = Some(Attachments {
			key,
			color: (samples > 1).then(|| texture("leptoskit_3d samples", format)),
			depth: texture("leptoskit_3d depth", DEPTH_FORMAT),
		});
	}

	fn globals_bind_group(&self, occlusion: Option<&Occlusion<'_>>) -> wgpu::BindGroup {
		let data = &self.data.as_ref().expect("the data texture was uploaded").1;
		let occlusion = occlusion.map_or(&self.shared.no_occlusion, |occlusion| occlusion.depth);
		self.gpu
			.device()
			.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("leptoskit_3d globals"),
				layout: &self.shared.globals_layout,
				entries: &[
					wgpu::BindGroupEntry {
						binding: 0,
						resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
							buffer: &self.globals,
							offset: 0,
							size: wgpu::BufferSize::new(size_of::<GlobalsUniform>() as u64),
						}),
					},
					wgpu::BindGroupEntry {
						binding: 1,
						resource: wgpu::BindingResource::TextureView(&self.shared.environment),
					},
					wgpu::BindGroupEntry {
						binding: 2,
						resource: wgpu::BindingResource::Sampler(&self.shared.linear_sampler),
					},
					wgpu::BindGroupEntry {
						binding: 3,
						resource: wgpu::BindingResource::TextureView(data),
					},
					wgpu::BindGroupEntry {
						binding: 4,
						resource: wgpu::BindingResource::TextureView(occlusion),
					},
				],
			})
	}

	fn background_bind_group(&self, target: &RenderTarget<'_>) -> Option<wgpu::BindGroup> {
		let Background::Camera(image) = target.background else {
			return None;
		};

		let (mode, first, second) = match image.planes {
			CameraPlanes::YCbCr { y, cbcr } => (1.0, y, cbcr),
			CameraPlanes::Rgba(rgba) => (0.0, rgba, rgba),
		};

		let uniform = BackgroundUniform {
			transform_x: row(&image.transform, 0),
			transform_y: row(&image.transform, 1),
			mode: [
				mode,
				if target.format.is_srgb() { 0.0 } else { 1.0 },
				0.0,
				0.0,
			],
		};
		let device = self.gpu.device();
		let buffer = wgpu::util::DeviceExt::create_buffer_init(
			device,
			&wgpu::util::BufferInitDescriptor {
				label: Some("leptoskit_3d background"),
				contents: bytemuck::bytes_of(&uniform),
				usage: wgpu::BufferUsages::UNIFORM,
			},
		);
		Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("leptoskit_3d background"),
			layout: &self.shared.background_layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: buffer.as_entire_binding(),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::TextureView(first),
				},
				wgpu::BindGroupEntry {
					binding: 2,
					resource: wgpu::BindingResource::TextureView(second),
				},
				wgpu::BindGroupEntry {
					binding: 3,
					resource: wgpu::BindingResource::Sampler(&self.shared.linear_sampler),
				},
			],
		}))
	}

	/// The colour the target clears to, premultiplied and encoded for its
	/// format.
	fn clear_color(&self, target: &RenderTarget<'_>) -> wgpu::Color {
		let color = match target.background {
			Background::Color(color) => color,
			Background::Camera(_) => [0.0, 0.0, 0.0, 1.0],
			Background::Scene => {
				match self.world.settings.backdrop {
					Backdrop::Color(color) => color,
					Backdrop::Transparent | Backdrop::Camera => [0.0; 4],
				}
			}
		};

		let encode = |value: f32| {
			let value = value.clamp(0.0, 1.0);
			f64::from(if target.format.is_srgb() {
				value
			} else {
				linear_to_srgb(value)
			})
		};
		let alpha = f64::from(color[3].clamp(0.0, 1.0));
		wgpu::Color {
			r: encode(color[0]) * alpha,
			g: encode(color[1]) * alpha,
			b: encode(color[2]) * alpha,
			a: alpha,
		}
	}

	fn draw_meshes(
		&self,
		pass: &mut wgpu::RenderPass<'_>,
		pipelines: &Pipelines,
		assets: &[Arc<GpuAsset>],
		draws: &[Draw],
		include: impl Fn(&Draw) -> bool,
	) {
		let Some(instances) = &self.instances.buffer else {
			return;
		};
		let mut current: Option<Variant> = None;

		for draw in draws.iter().filter(|draw| include(draw)) {
			if current != Some(draw.variant) {
				pass.set_pipeline(&pipelines.meshes[&draw.variant]);
				current = Some(draw.variant);
			}

			let asset = &assets[draw.asset];

			pass.set_bind_group(1, &asset.materials[draw.material].bind_group, &[]);
			pass.set_bind_group(
				2,
				asset.morphs.as_ref().unwrap_or(&self.shared.no_morphs),
				&[],
			);
			pass.set_vertex_buffer(0, asset.vertices.slice(draw.vertex_offset..));
			pass.set_vertex_buffer(1, instances.slice(..));

			if let (Some(offset), Some(skin)) = (draw.skin_offset, &asset.skin) {
				pass.set_vertex_buffer(2, skin.slice(offset..));
			}

			pass.set_index_buffer(asset.indices.slice(..), wgpu::IndexFormat::Uint32);
			pass.draw_indexed(
				draw.first_index..draw.first_index + draw.index_count,
				0,
				draw.instances.clone(),
			);
		}
	}

	/// Draws the scene offscreen from its camera and reads it back as RGBA
	/// bytes, four per pixel, row by row, with premultiplied alpha.
	#[cfg(not(target_arch = "wasm32"))]
	pub fn render_to_rgba(&mut self, width: u32, height: u32) -> Result<Vec<u8>, Error> {
		let texture = self.offscreen(width, height)?;
		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		self.render(&RenderTarget::new(
			&view,
			wgpu::TextureFormat::Rgba8Unorm,
			width,
			height,
		))?;
		self.read_back(&texture)
	}

	/// A texture to draw into and read back: `Rgba8Unorm`, `width` by
	/// `height`.
	pub fn offscreen(&self, width: u32, height: u32) -> Result<wgpu::Texture, Error> {
		let limit = self.gpu.device().limits().max_texture_dimension_2d;

		if width == 0 || height == 0 || width > limit || height > limit {
			return Err(Error::TooLarge {
				width,
				height,
				limit,
			});
		}

		Ok(self.gpu.device().create_texture(&wgpu::TextureDescriptor {
			label: Some("leptoskit_3d offscreen"),
			size: wgpu::Extent3d {
				width,
				height,
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba8Unorm,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT
				| wgpu::TextureUsages::COPY_SRC
				| wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		}))
	}

	/// Reads an 8-bit RGBA or BGRA texture's first layer back as RGBA bytes.
	#[cfg(not(target_arch = "wasm32"))]
	pub fn read_back(&self, texture: &wgpu::Texture) -> Result<Vec<u8>, Error> {
		use wgpu::TextureFormat as F;

		let swap = match texture.format() {
			F::Rgba8Unorm | F::Rgba8UnormSrgb => false,
			F::Bgra8Unorm | F::Bgra8UnormSrgb => true,
			format => return Err(Error::UnsupportedFormat(format)),
		};

		let (width, height) = (texture.width(), texture.height());
		let device = self.gpu.device();
		let row = width as usize * 4;
		let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
		let buffer = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("leptoskit_3d readback"),
			size: (padded * height as usize) as u64,
			usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
			label: Some("leptoskit_3d readback"),
		});
		encoder.copy_texture_to_buffer(
			texture.as_image_copy(),
			wgpu::TexelCopyBufferInfo {
				buffer: &buffer,
				layout: wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(padded as u32),
					rows_per_image: None,
				},
			},
			wgpu::Extent3d {
				width,
				height,
				depth_or_array_layers: 1,
			},
		);

		self.gpu.queue().submit([encoder.finish()]);
		let (sender, receiver) = std::sync::mpsc::channel();
		buffer.map_async(wgpu::MapMode::Read, .., move |result| {
			let _ = sender.send(result);
		});
		device
			.poll(wgpu::PollType::wait_indefinitely())
			.map_err(|error| Error::Readback(error.to_string()))?;
		receiver
			.recv()
			.map_err(|error| Error::Readback(error.to_string()))?
			.map_err(|error| Error::Readback(error.to_string()))?;
		let mapped = buffer
			.get_mapped_range(..)
			.map_err(|error| Error::Readback(error.to_string()))?;
		let mut pixels = Vec::with_capacity(row * height as usize);

		for line in mapped.chunks(padded) {
			pixels.extend_from_slice(&line[..row]);
		}

		drop(mapped);
		buffer.unmap();

		if swap {
			for pixel in pixels.as_chunks_mut::<4>().0 {
				pixel.swap(0, 2);
			}
		}

		Ok(pixels)
	}
}
