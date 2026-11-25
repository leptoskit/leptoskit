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
use crate::protocol::Ink;
use crate::protocol::Print;
use crate::protocol::PrintMode;
use crate::protocol::PrintOverride;
use crate::protocol::Shading;
use crate::protocol::Style;
use crate::raycast;
use crate::upload::*;
use crate::world::World;

/// The most views one target may have.
pub const MAX_VIEWS: usize = 4;
const GLOBALS_STRIDE: u64 = 512;
// Each view's globals fit their stride with at least one vec4 of headroom.
const _: () = assert!(size_of::<GlobalsUniform>() as u64 + 16 <= GLOBALS_STRIDE);
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
	/// Ink outline draw calls, per view.
	pub outlines: u32,
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
	style: StyleUniform,
}

/// A view's style, as the shader's `style`, `shade`, `ink`, `ink_noise`,
/// `print_bands`, and `print_paper` read it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct StyleUniform {
	/// x: 0 PBR, 1 Toon, 2 Print.
	/// Toon: `[1, bands, softness, specular]`.
	/// Print: `[2, side_heavy, side_drop, side_from]`.
	style: [f32; 4],
	/// Toon: `[shade.rgb, hatch]`.
	/// Print: `[side_to, hatch_gain, hatch_max, cross_duty]`.
	shade: [f32; 4],
	/// PBR/Toon: `[line_px, facing, shade_weight, wobble]`.
	/// Print: `[line_px, fade_lo_px, fade_hi_px, wobble]`.
	ink: [f32; 4],
	/// Where the wobble's noise is read, and the world size of one of the
	/// view's pixels at a clip w of 1.
	ink_noise: [f32; 4],
	/// Print: `[t_lit, t_deep, t_cross, rim]`. Zero in PBR and Toon.
	print_bands: [f32; 4],
	/// Print: `[paper.rgb, knockout_px]`. Zero in PBR and Toon.
	print_paper: [f32; 4],
}

/// The smallest radius, in pixels, a part may have on screen and still be
/// outlined: below it a line would swallow the part.
const MIN_OUTLINED_RADIUS: f32 = 4.0;

/// The step the boil is on at `time`: how many times the wobble has been
/// drawn afresh at `rate` a second.
fn boil_step(time: f64, rate: f32) -> u64 {
	if rate > 0.0 && time > 0.0 {
		(time * f64::from(rate)).floor() as u64
	} else {
		0
	}
}

/// Where the wobble's noise is read for `seed` at boil `step`: three
/// offsets from 0 to 256, the same for the same seed and step.
fn noise_offset(seed: u32, step: u64) -> [f32; 3] {
	// SplitMix64.
	let mut state = (u64::from(seed) << 32) ^ step;
	let mut next = || {
		state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
		let mut z = state;
		z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
		z ^ (z >> 31)
	};
	[0; 3].map(|_| (next() >> 40) as f32 / (1_u64 << 24) as f32 * 256.0)
}

/// The world size of one of `view`'s pixels at a clip w of 1, from its
/// projection's vertical scale.
fn pixel_size(view: &ViewParams) -> f32 {
	let scale = view.projection.y_axis.y.abs() * view.viewport[3];

	if scale > 0.0 && scale.is_finite() {
		2.0 / scale
	} else {
		0.0
	}
}

/// World-space unit vector pointing towards the directional light for `view`.
///
/// When [`Light::camera_space`] is `true`, [`Light::direction`] is in camera
/// space (`+x` right, `+y` up, `-z` forward) and is rotated into world space
/// by the view's inverse.
fn light_toward(light: &crate::protocol::Light, view_inverse: &Mat4) -> Vec3 {
	let toward = -Vec3::from(light.direction).normalize_or(Vec3::NEG_Y);
	if light.camera_space {
		(Mat3::from_mat4(*view_inverse) * toward).normalize_or(Vec3::Y)
	} else {
		toward
	}
}

/// The outline's width in `view`'s pixels.
fn line_pixels(ink: &Ink, view: &ViewParams) -> f32 {
	ink.width / 1000.0 * view.viewport[3]
}

/// What a view's globals say of the scene's style at `time`. The default
/// style packs to zeroes but for the shade, which only toon shading reads.
fn style_uniform(style: &Style, time: f64, view: &ViewParams) -> StyleUniform {
	if let Shading::Print(print) = style.shading {
		let (line_px, wobble, noise) = match &style.ink {
			None => (0.0, 0.0, [0.0; 3]),
			Some(ink) => {
				(
					line_pixels(ink, view),
					ink.wobble,
					noise_offset(ink.seed, boil_step(time, ink.boil)),
				)
			}
		};
		let knockout_px = print.knockout_width / 1000.0 * view.viewport[3];
		return StyleUniform {
			style: [2.0, print.side_heavy, print.side_drop, print.side_from],
			shade: [
				print.side_to,
				print.hatch_gain,
				print.hatch_max,
				print.cross_duty,
			],
			ink: [line_px, print.fade_px[0], print.fade_px[1], wobble],
			ink_noise: [noise[0], noise[1], noise[2], pixel_size(view)],
			print_bands: [print.t_lit, print.t_deep, print.t_cross, print.rim],
			print_paper: [print.paper[0], print.paper[1], print.paper[2], knockout_px],
		};
	}

	let (shading, shade) = match style.shading {
		Shading::Pbr => ([0.0; 4], [1.0, 1.0, 1.0, 0.0]),
		Shading::Toon(toon) => {
			(
				[1.0, f32::from(toon.bands), toon.softness, toon.specular],
				[toon.shade[0], toon.shade[1], toon.shade[2], toon.hatch],
			)
		}
		Shading::Print(_) => unreachable!(),
	};

	let (ink, ink_noise) = match &style.ink {
		None => ([0.0; 4], [0.0; 4]),
		Some(ink) => {
			let [x, y, z] = noise_offset(ink.seed, boil_step(time, ink.boil));
			(
				[
					line_pixels(ink, view),
					ink.facing,
					ink.shade_weight,
					ink.wobble,
				],
				[x, y, z, pixel_size(view)],
			)
		}
	};

	StyleUniform {
		style: shading,
		shade,
		ink,
		ink_noise,
		print_bands: [0.0; 4],
		print_paper: [0.0; 4],
	}
}

/// A part's line in relief-print mode: always carries ink RGB for hatching,
/// and multiplies a print override's `ink_weight` with any `SetInk` weight.
fn part_ink_print(
	part: &crate::world::Overrides,
	material: &crate::asset::Material,
	resolved: &PrintOverride,
	blended: bool,
	ink: Option<&Ink>,
) -> [f32; 4] {
	let [red, green, blue] = part
		.ink
		.color
		.unwrap_or_else(|| ink.map_or(Ink::default().color, |ink| ink.color));
	let weight = if ink.is_none() {
		0.0
	} else if let Some(print_weight) = resolved.ink_weight {
		print_weight * part.ink.weight.unwrap_or(1.0)
	} else {
		let default = if material.unlit || blended { 0.0 } else { 1.0 };
		part.ink.weight.unwrap_or(default)
	};
	[red, green, blue, weight]
}

/// A part's line as its instance record carries it: its colour, and its
/// weight, 0 for none. Unlit, blended, and additive parts have none unless
/// the guest set one, and a scene without ink has none at all.
fn part_ink(
	part: &crate::world::Overrides,
	material: &crate::asset::Material,
	blended: bool,
	ink: Option<&Ink>,
) -> [f32; 4] {
	let Some(ink) = ink else {
		return [0.0; 4];
	};
	let default = if material.unlit || blended { 0.0 } else { 1.0 };
	let [red, green, blue] = part.ink.color.unwrap_or(ink.color);
	[red, green, blue, part.ink.weight.unwrap_or(default)]
}

/// Whether a part `radius` pixels across on screen is worth a line `line`
/// pixels wide: big enough that the line would not swallow it.
fn outline_worthy(radius: f32, line: f32) -> bool {
	radius >= (2.0 * line).max(MIN_OUTLINED_RADIUS)
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
/// Per-draw settings for the relief-print paper knockout hull.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KnockoutDraw {
	/// Whether the primitive has open boundary edges (double-sided knockout).
	open_edges: bool,
	/// Whether its material uses `AlphaMode::Mask`.
	masked: bool,
}

struct Item {
	variant: Variant,
	asset: usize,
	material: usize,
	first_index: u32,
	index_count: u32,
	vertex_offset: u64,
	ink_offset: u64,
	skin_offset: Option<u64>,
	/// Whether it draws an ink outline.
	outlined: bool,
	/// Relief-print paper knockout settings when this item draws one.
	knockout: Option<KnockoutDraw>,
	/// Its figure among [`Renderer::figures`], or [`NONE`] without a
	/// knockout style.
	figure: u32,
	/// Its figure's stencil reference (`1..=255`) when that figure draws a
	/// knockout this frame, `0` otherwise.
	stencil_ref: u8,
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
	ink_offset: u64,
	skin_offset: Option<u64>,
	/// Whether any of its instances draws an ink outline; the others have
	/// no weight, so the outline shader drops them.
	outlined: bool,
	/// Relief-print paper knockout settings when any instance draws one.
	knockout: Option<KnockoutDraw>,
	stencil_ref: u8,
	morphed: bool,
	instances: std::ops::Range<u32>,
}

/// Shared resources and toggles passed into each render pass of one frame.
struct FramePass<'a> {
	views: &'a [ViewParams],
	pipelines: &'a Pipelines,
	outlines: Option<&'a [wgpu::RenderPipeline; 4]>,
	knockout_solid: Option<&'a [wgpu::RenderPipeline; 6]>,
	knockout_discard: Option<&'a [wgpu::RenderPipeline; 6]>,
	globals: &'a wgpu::BindGroup,
	background: Option<&'a wgpu::BindGroup>,
	assets: &'a [Arc<GpuAsset>],
	draws: &'a [Draw],
	clear: wgpu::Color,
	shadow_count: u32,
	use_stencil: bool,
	occluded: bool,
}

/// How large things look in one view: its view matrix, its focal length in
/// pixels, and its outline's width in pixels.
#[derive(Clone, Copy, Debug, Default)]
struct ViewScale {
	view: Mat4,
	focal: f32,
	line: f32,
}

/// What every view can see, the first view's eye for sorting, and the scene's
/// ink with how large things look in each view.
struct Sight {
	frustums: Vec<Frustum>,
	eye: Mat4,
	ink: Option<Ink>,
	print: Option<Print>,
	scales: [ViewScale; MAX_VIEWS],
	views: usize,
}

impl Sight {
	fn sees(&self, bounds: &Aabb) -> bool {
		self.frustums.iter().any(|frustum| frustum.sees(bounds))
	}

	/// Whether some view sees `bounds` large enough for a line of `weight`.
	fn worth_outlining(&self, bounds: &Aabb, weight: f32) -> bool {
		if bounds.is_empty() {
			return false;
		}
		let radius = (bounds.max - bounds.min).length() * 0.5;
		let center = bounds.center().extend(1.0);

		self.scales[..self.views].iter().any(|scale| {
			let depth = -(scale.view * center).z;
			// A part wholly behind the view is never worth outlining for it;
			// a view inside the bounds sees the part fill it.
			if depth < -radius {
				false
			} else if depth <= radius {
				true
			} else {
				outline_worthy(radius * scale.focal / depth, scale.line * weight)
			}
		})
	}

	/// Whether some view sees `bounds` at least half a pixel across:
	/// relief-print mode keeps outlining fine parts and only drops
	/// sub-pixel specks (and anything strictly behind a view).
	fn visible_for_print_outline(&self, bounds: &Aabb) -> bool {
		if bounds.is_empty() {
			return false;
		}
		let radius = (bounds.max - bounds.min).length() * 0.5;
		let center = bounds.center().extend(1.0);

		self.scales[..self.views].iter().any(|scale| {
			let depth = -(scale.view * center).z;
			(depth >= -radius && depth <= radius)
				|| (depth > radius && radius * scale.focal / depth >= 0.5)
		})
	}
}

/// One model instance's state for a frame, copied out of the world.
struct ModelState {
	asset: Arc<crate::asset::Asset>,
	weights: Vec<Vec<f32>>,
	overrides: Vec<crate::world::Overrides>,
	print_by_material: Vec<PrintOverride>,
	print_by_node: Vec<PrintOverride>,
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
		let print_active = matches!(world.settings.style.shading, Shading::Print(_));

		Some(Self {
			weights: pose.weights.clone(),
			overrides: instance.overrides.clone(),
			print_by_material: if print_active {
				instance.print_by_material.clone()
			} else {
				Vec::new()
			},
			print_by_node: if print_active {
				instance.print_by_node.clone()
			} else {
				Vec::new()
			},
			hidden: instance.hidden.clone(),
			worlds: instance.world.clone(),
			skins,
			cast_shadow: instance.cast_shadow,
			ground: node.world.w_axis.y,
			asset,
		})
	}
}

/// A mesh node of a model: which one, its asset's slot, its joints, and its
/// figure.
struct Part<'a> {
	model: &'a ModelState,
	index: usize,
	slot: usize,
	joints: u32,
	figure: u32,
}

/// A figure met this frame: whether any of its opaque parts in view draws a
/// paper knockout, and the stencil reference it takes if one does.
#[derive(Clone, Copy, Debug, Default)]
struct Figure {
	knockout: bool,
	stencil_ref: u8,
}

/// The figure `id` prints as part of: the highest model instance reached by
/// following [`Op::Attach`](crate::protocol::Op::Attach) links up from `id`,
/// or `id` itself. A weapon attached to a hand shares its wielder's stencil
/// reference, with no paper seam between them, while a model spawned under
/// another (a figure on a stage) is a figure of its own.
fn figure_root(world: &World, id: crate::protocol::NodeId) -> crate::protocol::NodeId {
	let mut figure = id;
	let mut cursor = id;

	// At most one step per node, so no shape of the tree can hang a frame.
	for _ in 0..world.nodes.len() {
		let Some(parent) = world
			.nodes
			.get(&cursor)
			.filter(|node| node.attached)
			.and_then(|node| node.parent)
		else {
			break;
		};

		if world
			.nodes
			.get(&parent)
			.is_some_and(|node| node.instance.is_some())
		{
			figure = parent;
		}

		cursor = parent;
	}

	figure
}

/// A primitive's instance attributes: its matrix, its part's overrides,
/// and where its joints and morph weights start (`[joints, morph]`).
fn instance_record(
	part: &crate::world::Overrides,
	material: &crate::asset::Material,
	model: Mat4,
	[joints, morph]: [u32; 2],
	ink: [f32; 4],
	units: f32,
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
			additive_and_units(part.blend == Blend::Additive, units),
		],
		ink,
	}
}

/// A primitive's instance attributes in relief-print mode.
#[allow(clippy::too_many_arguments)]
fn print_instance_record(
	part: &crate::world::Overrides,
	material: &crate::asset::Material,
	resolved: &PrintOverride,
	model: Mat4,
	[joints, morph]: [u32; 2],
	ink: [f32; 4],
	extra_texel: u32,
	knockout: bool,
	units: f32,
) -> InstanceRecord {
	let base = [
		material.base_color[0] * part.color[0],
		material.base_color[1] * part.color[1],
		material.base_color[2] * part.color[2],
	];
	let lit = resolved.lit.unwrap_or(base);
	let shade = resolved.shade.unwrap_or(lit.map(|channel| channel * 0.65));
	let deep = resolved.deep.unwrap_or(lit.map(|channel| channel * 0.35));
	let specular = resolved.specular.unwrap_or(0.0);
	let hatch_min = resolved.hatch_min.unwrap_or(0.0);
	// Unlit parts (glows, eyes, screens) take no light anywhere else, so
	// they print flat unless a target asks for bands.
	let mode = resolved.mode.unwrap_or(if material.unlit {
		PrintMode::Flat
	} else {
		PrintMode::Banded
	});
	let flags = u32::from(mode == PrintMode::Flat)
		| (u32::from(resolved.cross_hatch.unwrap_or(false)) << 1)
		| (u32::from(resolved.rim.unwrap_or(false)) << 2)
		| (u32::from(knockout) << 3);
	let print_meta = (flags << 24) | (extra_texel & 0x00ff_ffff);

	InstanceRecord {
		model: model.to_cols_array_2d(),
		tint: [lit[0], lit[1], lit[2], part.color[3] * part.opacity],
		emissive: [shade[0], shade[1], shade[2], specular],
		rim: [deep[0], deep[1], deep[2], hatch_min],
		params: [
			joints,
			morph,
			print_meta,
			additive_and_units(part.blend == Blend::Additive, units),
		],
		ink,
	}
}

/// An instance's `params.w`: bit 0 set when the part is additive, and in the
/// high half, as a half float, `log2` of how long a unit of its mesh's space
/// is in metres (0 when nothing reads it or when the unit is 1 m, avoiding
/// half-float overflow above 65504 and subnormal flush-to-zero on quantized
/// meshes).
fn additive_and_units(additive: bool, units: f32) -> u32 {
	let log_units = if units > 0.0 && units.is_finite() {
		let value = units.clamp(1.0e-12, 1.0e12).log2();
		if value.abs() < half::f16::MIN_POSITIVE.to_f32() {
			0.0
		} else {
			value
		}
	} else {
		0.0
	};
	u32::from(additive) | (u32::from(half::f16::from_f32(log_units).to_bits()) << 16)
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
	key: (u32, u32, wgpu::TextureFormat, u32, bool),
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
	print_extras: HashMap<[u32; 8], u32>,
	/// This frame's figures under a knockout style, in the order met, and
	/// each figure's index by its root node.
	figures: Vec<Figure>,
	figure_index: HashMap<crate::protocol::NodeId, u32>,
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
			print_extras: HashMap::new(),
			figures: Vec::new(),
			figure_index: HashMap::new(),
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
		// The style alone decides, so the attachments and the pipeline set
		// stay put while knockout figures walk in and out of view.
		let use_stencil = matches!(
			self.world.settings.style.shading,
			Shading::Print(print) if print.knockout_width > 0.0
		);
		let samples = self.gpu.samples(target.format, use_stencil);
		let pipelines =
			self.shared
				.pipelines(self.gpu.device(), target.format, samples, use_stencil);
		let outline_count = draws
			.iter()
			.filter(|draw| draw.outlined && !draw.variant.blend)
			.count() as u32;
		let occluded = target.occlusion.is_some();
		let outlines = (outline_count > 0).then(|| {
			self.shared
				.outline_pipelines(self.gpu.device(), &pipelines, occluded)
		});
		let knockout_solid = (use_stencil
			&& !occluded
			&& draws
				.iter()
				.any(|draw| !draw.variant.blend && draw.knockout.is_some_and(|ko| !ko.masked)))
		.then(|| {
			self.shared
				.knockout_pipelines(self.gpu.device(), &pipelines, false)
		});
		let knockout_discard = (use_stencil
			&& draws.iter().any(|draw| {
				!draw.variant.blend && draw.knockout.is_some_and(|ko| occluded || ko.masked)
			}))
		.then(|| {
			self.shared
				.knockout_pipelines(self.gpu.device(), &pipelines, true)
		});
		self.prepare_attachments(width, height, target.format, samples, use_stencil);
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

		let frame = FramePass {
			views: &views,
			pipelines: &pipelines,
			outlines,
			knockout_solid,
			knockout_discard,
			globals: &globals,
			background: background.as_ref(),
			assets: &assets,
			draws: &draws,
			clear,
			shadow_count,
			use_stencil,
			occluded,
		};

		for (view, range) in passes {
			self.record_pass(&mut encoder, view, range, &frame);
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
			outlines: outline_count,
		};
		Ok(self.stats)
	}

	fn record_pass(
		&self,
		encoder: &mut wgpu::CommandEncoder,
		view: &wgpu::TextureView,
		range: std::ops::Range<usize>,
		frame: &FramePass<'_>,
	) {
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
					load: wgpu::LoadOp::Clear(frame.clear),
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
				stencil_ops: frame.use_stencil.then_some(wgpu::Operations {
					load: wgpu::LoadOp::Clear(0),
					store: wgpu::StoreOp::Discard,
				}),
			}),
			..wgpu::RenderPassDescriptor::default()
		});

		for index in range {
			let [x, y, w, h] = frame.views[index].viewport;
			pass.set_viewport(x, y, w, h, 0.0, 1.0);

			if let Some(background) = frame.background {
				pass.set_pipeline(&frame.pipelines.background);
				pass.set_bind_group(0, background, &[]);
				pass.draw(0..3, 0..1);
			}

			let offset = (index as u64 * GLOBALS_STRIDE) as u32;
			pass.set_bind_group(0, frame.globals, &[offset]);
			self.draw_meshes(
				&mut pass,
				frame.pipelines,
				frame.assets,
				frame.draws,
				frame.use_stencil,
				|draw| !draw.variant.blend,
			);

			// After the opaque meshes, so most of the hull fails the depth
			// test early, and before anything blended.
			if let Some(outlines) = frame.outlines {
				self.draw_outlines(
					&mut pass,
					outlines,
					frame.assets,
					frame.draws,
					frame.use_stencil,
				);
			}

			if frame.shadow_count > 0
				&& let Some(buffer) = &self.shadows.buffer
			{
				pass.set_pipeline(&frame.pipelines.shadow);
				pass.set_vertex_buffer(0, buffer.slice(..));
				pass.draw(0..4, 0..frame.shadow_count);
			}

			// After the shadows, which it covers: the knockout writes no
			// depth, so a shadow drawn later would darken the paper.
			if frame.use_stencil {
				self.draw_knockout(
					&mut pass,
					frame.knockout_solid,
					frame.knockout_discard,
					frame.occluded,
					frame.assets,
					frame.draws,
				);
			}
			self.draw_meshes(
				&mut pass,
				frame.pipelines,
				frame.assets,
				frame.draws,
				false,
				|draw| draw.variant.blend,
			);
		}
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

	/// Deduplicates and writes two per-primitive relief-print texels into
	/// `self.texels`: `[spot.rgb, hatch_per_m]` and `[noise.xyz, wobble_freq]`.
	fn push_print_extra(&mut self, spot_hatch: [f32; 4], noise_freq: [f32; 4]) -> u32 {
		let key = [
			spot_hatch[0].to_bits(),
			spot_hatch[1].to_bits(),
			spot_hatch[2].to_bits(),
			spot_hatch[3].to_bits(),
			noise_freq[0].to_bits(),
			noise_freq[1].to_bits(),
			noise_freq[2].to_bits(),
			noise_freq[3].to_bits(),
		];

		if let Some(&index) = self.print_extras.get(&key) {
			return index;
		}

		let index = self.texels.len() as u32;
		self.texels.push(spot_hatch);
		self.texels.push(noise_freq);
		self.print_extras.insert(key, index);
		index
	}

	/// Collects every visible primitive of every instance, with its instance
	/// record, culling what no view sees.
	fn build(&mut self, views: &[ViewParams]) {
		self.items.clear();
		self.texels.clear();
		self.print_extras.clear();
		self.figures.clear();
		self.figure_index.clear();
		self.shadow_records.clear();
		self.assets.clear();
		self.asset_index.clear();
		// Texel 0 is never a valid offset, so a zeroed record reads nothing.
		self.texels.push([0.0; 4]);
		let ink = self.world.settings.style.ink;
		let print = match self.world.settings.style.shading {
			Shading::Print(print) => Some(print),
			_ => None,
		};
		let print_knockout = print.is_some_and(|print| print.knockout_width > 0.0);
		let mut scales = [ViewScale::default(); MAX_VIEWS];

		for (scale, view) in scales.iter_mut().zip(views) {
			*scale = ViewScale {
				view: view.view,
				focal: view.projection.y_axis.y.abs() * view.viewport[3] * 0.5,
				line: ink.map_or(0.0, |ink| line_pixels(&ink, view)),
			};
		}

		let sight = Sight {
			frustums: views
				.iter()
				.map(|view| Frustum::new(&(view.projection * view.view)))
				.collect(),
			eye: views[0].view,
			ink,
			print,
			scales,
			views: views.len(),
		};
		let shadow = self.world.settings.light.shadow;

		let mut ids: Vec<_> = self.world.nodes.keys().copied().collect();
		ids.sort_unstable();
		self.stats.culled = 0;

		for id in ids {
			let Some(model) = ModelState::of(&self.world, id) else {
				continue;
			};
			let figure = if print_knockout {
				let next = self.figures.len() as u32;
				let figure = *self
					.figure_index
					.entry(figure_root(&self.world, id))
					.or_insert(next);

				if figure == next {
					self.figures.push(Figure::default());
				}

				figure
			} else {
				NONE
			};
			let bounds = self.build_model(&model, &sight, figure);

			if model.cast_shadow
				&& let Some(record) = shadow_record(bounds, model.ground, shadow, &sight)
			{
				self.shadow_records.push(record);
			}
		}

		self.assign_stencil_refs();
	}

	/// Gives each figure whose knockout draws this frame a stencil reference,
	/// in the order the figures were met, and every opaque item of that figure
	/// its reference. Figures out of view, or without a knockout, take none.
	/// There are 255 references, so past 255 knockout figures in one frame
	/// they repeat, and two figures sharing one draw no paper between them.
	fn assign_stencil_refs(&mut self) {
		let mut drawn = 0_usize;

		for figure in &mut self.figures {
			if figure.knockout {
				figure.stencil_ref = ((drawn % 255) + 1) as u8;
				drawn += 1;
			}
		}

		for item in &mut self.items {
			if let Some(figure) = self.figures.get(item.figure as usize)
				&& !item.variant.blend
			{
				item.stencil_ref = figure.stencil_ref;
			}
		}
	}

	/// Adds a model's visible parts, returning its bounds in this pose.
	fn build_model(&mut self, model: &ModelState, sight: &Sight, figure: u32) -> Aabb {
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
					figure,
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
		let overrides = &model.overrides[index];
		let blend = material.alpha == AlphaMode::Blend
			|| overrides.color[3] * overrides.opacity < 1.0
			|| overrides.blend == Blend::Additive;
		let masked = matches!(material.alpha, AlphaMode::Mask(_));

		let (outlined, knockout, record) = if let Some(print) = sight.print {
			let mut resolved = model.print_by_material[primitive.material];
			resolved.overlay(&model.print_by_node[index]);
			let mut ink = part_ink_print(overrides, material, &resolved, blend, sight.ink.as_ref());
			let inked = !blend && ink[3] > 0.0 && sight.visible_for_print_outline(&bounds);

			if !blend && !inked {
				ink[3] = 0.0;
			}

			// A scene's ink of no width draws no outline, and alpha-masked
			// cutouts skip the inverted hull so transparent cutout
			// holes are never filled with ink, while keeping their
			// inner lines and hatching.
			let outlined = inked
				&& material.alpha == AlphaMode::Opaque
				&& sight.ink.is_some_and(|ink| ink.width > 0.0);
			let knockout =
				!blend && print.knockout_width > 0.0 && resolved.knockout.unwrap_or(true);
			let spot = resolved.spot.unwrap_or(print.paper);
			let hatch_per_m = resolved.hatch_per_m.unwrap_or(print.hatch_per_m);
			let noise = match sight.ink {
				Some(ink) => {
					let part_seed = model.asset.seeds.get(index).copied().unwrap_or(0) ^ ink.seed;
					let step = resolved.boil_step.map_or_else(
						|| boil_step(self.world.time, ink.boil) % 3,
						|step| u64::from(step % 3),
					);
					noise_offset(part_seed, step)
				}
				None => [0.0; 3],
			};
			let extra = self.push_print_extra(
				[spot[0], spot[1], spot[2], hatch_per_m],
				[noise[0], noise[1], noise[2], print.wobble_frequency],
			);
			let units = model.asset.units.get(index).copied().unwrap_or(1.0);
			let record = print_instance_record(
				overrides,
				material,
				&resolved,
				if skinned {
					Mat4::IDENTITY
				} else {
					model.worlds[index]
				},
				[if skinned { part.joints } else { NONE }, morph],
				ink,
				extra,
				knockout,
				units,
			);
			(outlined, knockout, record)
		} else {
			let mut ink = part_ink(overrides, material, blend, sight.ink.as_ref());
			let inked = !blend && ink[3] > 0.0 && sight.worth_outlining(&bounds, ink[3]);

			if !blend && !inked {
				// Too small for a line: no ink drawn on its surface either.
				ink[3] = 0.0;
			}

			// A scene's ink of no width draws no outline, and alpha-masked
			// cutouts skip the inverted hull so transparent cutout
			// holes are never filled with ink, while keeping their
			// inner lines and hatching.
			let outlined = inked
				&& material.alpha == AlphaMode::Opaque
				&& sight.ink.is_some_and(|ink| ink.width > 0.0);

			let record = instance_record(
				overrides,
				material,
				if skinned {
					Mat4::IDENTITY
				} else {
					model.worlds[index]
				},
				[if skinned { part.joints } else { NONE }, morph],
				ink,
				// Only ink reads the units, so a scene without it packs them as
				// zero, as it always did.
				if sight.ink.is_some() {
					model.asset.units.get(index).copied().unwrap_or(1.0)
				} else {
					0.0
				},
			);
			(outlined, false, record)
		};
		let mirrored = match (&model.skins[index], skinned) {
			(Some((matrices, _)), true) => {
				matrices.first().map_or_else(
					|| model.worlds[index].determinant() < 0.0,
					|matrix| matrix.determinant() < 0.0,
				)
			}
			_ => model.worlds[index].determinant() < 0.0,
		};
		let vertex_size = size_of::<crate::asset::Vertex>() as u64;
		let ink_size = size_of::<crate::asset::InkVertex>() as u64;
		let skin_size = size_of::<crate::asset::SkinVertex>() as u64;

		if knockout && let Some(figure) = self.figures.get_mut(part.figure as usize) {
			figure.knockout = true;
		}

		self.items.push(Item {
			variant: Variant {
				skinned,
				blend,
				double_sided: material.double_sided,
				mirrored,
			},
			asset: part.slot,
			material: primitive.material,
			first_index: primitive.first_index,
			index_count: primitive.index_count,
			vertex_offset: u64::from(primitive.base_vertex) * vertex_size,
			ink_offset: u64::from(primitive.base_vertex) * ink_size,
			outlined,
			knockout: knockout.then_some(KnockoutDraw {
				open_edges: primitive.has_open_edges,
				masked,
			}),
			figure: part.figure,
			// Set once every figure in view is known.
			stencil_ref: 0,
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
			(a.variant, a.stencil_ref, a.asset, a.material, a.first_index)
				.cmp(&(b.variant, b.stencil_ref, b.asset, b.material, b.first_index))
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
				&& last.stencil_ref == item.stencil_ref
				&& last.asset == item.asset
				&& last.material == item.material
				&& last.first_index == item.first_index
				&& last.vertex_offset == item.vertex_offset
				&& !last.morphed
				&& last.skin_offset.is_none()
				&& last.instances.end == index
			{
				last.instances.end += 1;
				last.outlined |= item.outlined;
				last.knockout = last.knockout.or(item.knockout);
				continue;
			}
			draws.push(Draw {
				variant: item.variant,
				asset: item.asset,
				material: item.material,
				first_index: item.first_index,
				index_count: item.index_count,
				vertex_offset: item.vertex_offset,
				ink_offset: item.ink_offset,
				outlined: item.outlined,
				knockout: item.knockout,
				stencil_ref: item.stencil_ref,
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
			let view_inverse = view.view.inverse();
			let camera = view_inverse.w_axis;
			let toward = light_toward(&light, &view_inverse);
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
				style: style_uniform(&settings.style, self.world.time, view),
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
		stencil: bool,
	) {
		let key = (width, height, format, samples, stencil);

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
		let depth_format = if stencil {
			DEPTH_STENCIL_FORMAT
		} else {
			DEPTH_FORMAT
		};
		self.attachments = Some(Attachments {
			key,
			color: (samples > 1).then(|| texture("leptoskit_3d samples", format)),
			depth: texture("leptoskit_3d depth", depth_format),
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
		write_stencil: bool,
		include: impl Fn(&Draw) -> bool,
	) {
		let Some(instances) = &self.instances.buffer else {
			return;
		};
		let mut current: Option<Variant> = None;
		let mut current_ref: Option<u8> = None;

		for draw in draws.iter().filter(|draw| include(draw)) {
			if current != Some(draw.variant) {
				pass.set_pipeline(&pipelines.meshes[&draw.variant]);
				current = Some(draw.variant);
			}
			if write_stencil && current_ref != Some(draw.stencil_ref) {
				pass.set_stencil_reference(u32::from(draw.stencil_ref));
				current_ref = Some(draw.stencil_ref);
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

	/// Draws the ink outline of every opaque draw that has one, with the
	/// same instances and indices as its mesh.
	fn draw_outlines(
		&self,
		pass: &mut wgpu::RenderPass<'_>,
		pipelines: &[wgpu::RenderPipeline; 4],
		assets: &[Arc<GpuAsset>],
		draws: &[Draw],
		write_stencil: bool,
	) {
		let Some(instances) = &self.instances.buffer else {
			return;
		};
		let mut current: Option<(bool, bool)> = None;
		let mut current_ref: Option<u8> = None;

		for draw in draws
			.iter()
			.filter(|draw| draw.outlined && !draw.variant.blend)
		{
			let key = (draw.variant.skinned, draw.variant.mirrored);

			if current != Some(key) {
				let slot = usize::from(key.0) + usize::from(key.1) * 2;
				pass.set_pipeline(&pipelines[slot]);
				current = Some(key);
			}
			if write_stencil && current_ref != Some(draw.stencil_ref) {
				pass.set_stencil_reference(u32::from(draw.stencil_ref));
				current_ref = Some(draw.stencil_ref);
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
			pass.set_vertex_buffer(2, asset.ink.slice(draw.ink_offset..));

			if let (Some(offset), Some(skin)) = (draw.skin_offset, &asset.skin) {
				pass.set_vertex_buffer(3, skin.slice(offset..));
			}

			pass.set_index_buffer(asset.indices.slice(..), wgpu::IndexFormat::Uint32);
			pass.draw_indexed(
				draw.first_index..draw.first_index + draw.index_count,
				0,
				draw.instances.clone(),
			);
		}
	}

	/// Draws each figure's outer relief-print paper knockout hull in the same
	/// render pass, testing stencil `NotEqual` against that figure's reference.
	fn draw_knockout(
		&self,
		pass: &mut wgpu::RenderPass<'_>,
		solid: Option<&[wgpu::RenderPipeline; 6]>,
		discard: Option<&[wgpu::RenderPipeline; 6]>,
		occluded: bool,
		assets: &[Arc<GpuAsset>],
		draws: &[Draw],
	) {
		let Some(instances) = &self.instances.buffer else {
			return;
		};
		let mut current_pipeline: Option<(bool, usize)> = None;
		let mut current_ref: Option<u8> = None;

		for draw in draws.iter().filter(|draw| !draw.variant.blend) {
			let Some(ko) = draw.knockout else {
				continue;
			};
			let use_discard = occluded || ko.masked;
			let cull_slot = if ko.open_edges {
				2
			} else {
				usize::from(draw.variant.mirrored)
			};
			let slot = cull_slot * 2 + usize::from(draw.variant.skinned);

			if current_pipeline != Some((use_discard, slot)) {
				let set = if use_discard { discard } else { solid };
				let Some(pipelines) = set else {
					continue;
				};
				pass.set_pipeline(&pipelines[slot]);
				current_pipeline = Some((use_discard, slot));
			}
			if current_ref != Some(draw.stencil_ref) {
				pass.set_stencil_reference(u32::from(draw.stencil_ref));
				current_ref = Some(draw.stencil_ref);
			}

			let asset = &assets[draw.asset];
			let normals = if ko.open_edges {
				asset.knockout.as_ref().unwrap_or(&asset.ink)
			} else {
				&asset.ink
			};

			pass.set_bind_group(1, &asset.materials[draw.material].bind_group, &[]);
			pass.set_bind_group(
				2,
				asset.morphs.as_ref().unwrap_or(&self.shared.no_morphs),
				&[],
			);
			pass.set_vertex_buffer(0, asset.vertices.slice(draw.vertex_offset..));
			pass.set_vertex_buffer(1, instances.slice(..));
			pass.set_vertex_buffer(2, normals.slice(draw.ink_offset..));

			if let (Some(offset), Some(skin)) = (draw.skin_offset, &asset.skin) {
				pass.set_vertex_buffer(3, skin.slice(offset..));
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

/// The glTF builders the integration tests use, for the tests below that
/// draw.
#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "../tests/support/fixture.rs"]
mod fixture;

#[cfg(test)]
mod tests {
	use super::*;
	use crate::asset::Material;
	use crate::protocol::PartInk;
	use crate::protocol::Toon;
	use crate::world::Overrides;

	fn view(height: f32) -> ViewParams {
		ViewParams {
			view: Mat4::IDENTITY,
			projection: glam::camera::rh::proj::directx::perspective(0.8, 0.75, 0.1, 50.0),
			viewport: [0.0, 0.0, height * 0.75, height],
		}
	}

	#[test]
	fn the_default_style_packs_to_nothing_the_shader_reads() {
		let packed = style_uniform(&Style::default(), 12.5, &view(960.0));
		// Toon shading, hatching, the outline, and the inner line are all off.
		assert_eq!(packed.style, [0.0; 4]);
		assert_eq!(packed.shade[3], 0.0);
		assert_eq!(packed.ink, [0.0; 4]);
		assert_eq!(packed.ink_noise, [0.0; 4]);
		assert_eq!(packed.print_bands, [0.0; 4]);
		assert_eq!(packed.print_paper, [0.0; 4]);
	}

	#[test]
	fn print_style_packs_into_globals_with_headroom() {
		assert!(size_of::<GlobalsUniform>() as u64 + 16 <= GLOBALS_STRIDE);
		let style = Style {
			shading: Shading::Print(Print::default()),
			ink: None,
		};
		let packed = style_uniform(&style, 0.0, &view(1000.0));
		assert_eq!(packed.style[0], 2.0);
		assert_eq!(packed.ink[0], 0.0);
		assert_eq!(packed.ink[1], Print::default().fade_px[0]);
		assert_eq!(packed.ink[2], Print::default().fade_px[1]);
		assert!(
			packed.ink_noise[3] > 0.0,
			"pixel_size stays set without ink"
		);
		assert_eq!(
			packed.print_bands,
			[
				Print::default().t_lit,
				Print::default().t_deep,
				Print::default().t_cross,
				Print::default().rim,
			]
		);
		assert!((packed.print_paper[3] - Print::default().knockout_width).abs() < 1.0e-5);
	}

	#[test]
	fn styles_pack_into_each_views_globals() {
		let style = Style {
			shading: Shading::Toon(Toon {
				bands: 3,
				softness: 0.2,
				shade: [0.4, 0.3, 0.6],
				specular: 1.5,
				hatch: 0.5,
			}),
			ink: Some(Ink {
				width: 2.5,
				facing: 0.7,
				shade_weight: 1.0,
				wobble: 0.3,
				..Ink::default()
			}),
		};
		let tall = style_uniform(&style, 0.0, &view(1000.0));
		assert_eq!(tall.style, [1.0, 3.0, 0.2, 1.5]);
		assert_eq!(tall.shade, [0.4, 0.3, 0.6, 0.5]);
		// 2.5 thousandths of a 1000-pixel view.
		assert_eq!(tall.ink, [2.5, 0.7, 1.0, 0.3]);
		// Half the height, half the pixels: the line keeps its share of the
		// view on every density, and each eye of a stereo pair gets its own.
		let short = style_uniform(&style, 0.0, &view(500.0));
		assert_eq!(short.ink[0], 1.25);
		assert!((short.ink_noise[3] - 2.0 * tall.ink_noise[3]).abs() < 1.0e-9);
		// A pixel of a 1000-pixel view with a 0.8 rad field of view, at a
		// depth of 1, is 2 tan(0.4) / 1000 wide.
		let expected = 2.0 * 0.4_f32.tan() / 1000.0;
		assert!((tall.ink_noise[3] - expected).abs() < 1.0e-8, "{tall:?}");
	}

	#[test]
	fn the_boil_steps_at_its_rate_and_holds_between_steps() {
		assert_eq!(boil_step(10.0, 0.0), 0, "no boil holds still");
		assert_eq!(boil_step(0.99 / 8.0, 8.0), 0);
		assert_eq!(boil_step(1.0 / 8.0, 8.0), 1);
		assert_eq!(boil_step(3.06, 8.0), 24);
		assert_eq!(noise_offset(7, 24), noise_offset(7, 24));
		assert_ne!(noise_offset(7, 24), noise_offset(7, 25));
		assert_ne!(noise_offset(7, 24), noise_offset(8, 24));
		assert!(
			noise_offset(7, 24)
				.into_iter()
				.all(|value| (0.0..256.0).contains(&value))
		);

		let style = Style {
			shading: Shading::Pbr,
			ink: Some(Ink {
				boil: 8.0,
				..Ink::default()
			}),
		};
		let at = |time| style_uniform(&style, time, &view(800.0)).ink_noise;
		assert_eq!(at(1.0), at(1.1), "the same step draws the same line");
		assert_ne!(at(1.0), at(1.0 + 1.0 / 8.0), "the next step draws afresh");
	}

	#[test]
	fn parts_take_the_scenes_ink_unless_unlit_or_blended() {
		let ink = Ink {
			color: [0.1, 0.2, 0.3],
			..Ink::default()
		};
		let lit = Material::default();
		let unlit = Material {
			unlit: true,
			..Material::default()
		};
		let part = Overrides::default();
		assert_eq!(
			part_ink(&part, &lit, false, None),
			[0.0; 4],
			"no ink, no line"
		);
		assert_eq!(
			part_ink(&part, &lit, false, Some(&ink)),
			[0.1, 0.2, 0.3, 1.0]
		);
		assert_eq!(part_ink(&part, &unlit, false, Some(&ink))[3], 0.0);
		assert_eq!(part_ink(&part, &lit, true, Some(&ink))[3], 0.0);

		// A line the guest sets wins, on any part.
		let asked = Overrides {
			ink: PartInk {
				weight: Some(2.0),
				color: Some([0.9, 0.8, 0.7]),
			},
			..Overrides::default()
		};
		assert_eq!(
			part_ink(&asked, &unlit, true, Some(&ink)),
			[0.9, 0.8, 0.7, 2.0]
		);
		let none = Overrides {
			ink: PartInk {
				weight: Some(0.0),
				color: None,
			},
			..Overrides::default()
		};
		assert_eq!(part_ink(&none, &lit, false, Some(&ink))[3], 0.0);
	}

	#[test]
	fn small_parts_are_not_outlined() {
		assert!(outline_worthy(40.0, 3.0));
		assert!(outline_worthy(6.0, 3.0));
		assert!(!outline_worthy(5.9, 3.0), "the line would swallow it");
		assert!(!outline_worthy(3.9, 0.5), "too small to see");
		assert!(outline_worthy(4.0, 0.5));
	}

	#[test]
	fn the_additive_flag_shares_its_word_with_the_units() {
		assert_eq!(additive_and_units(false, 0.0), 0);
		assert_eq!(additive_and_units(true, 0.0), 1);
		assert_eq!(additive_and_units(false, 1.0), 0);
		let packed = additive_and_units(true, 7.845_902e-5);
		assert_eq!(packed & 1, 1);
		let units = half::f16::from_bits((packed >> 16) as u16).to_f32().exp2();
		assert!(
			(units - 7.845_902e-5).abs() / 7.845_902e-5 < 0.01,
			"{units}"
		);
	}

	#[test]
	fn units_packing_handles_large_scales_without_overflow() {
		let large = additive_and_units(false, 100_000.0);
		let large_f16 = half::f16::from_bits((large >> 16) as u16);
		assert!(
			large_f16.is_finite(),
			"100_000.0 overflowed f16 to {large_f16} (bits=0x{:04x})",
			large_f16.to_bits()
		);
		let decoded = large_f16.to_f32().exp2();
		assert!(
			(decoded - 100_000.0).abs() / 100_000.0 < 0.01,
			"100_000.0 decoded to {decoded}"
		);
	}

	#[test]
	fn units_packing_avoids_subnormal_f16_for_small_quantized_models() {
		for scale in [3.792e-5_f32, 4.772e-5, 4.969e-5] {
			let packed = additive_and_units(false, scale);
			let bits = (packed >> 16) as u16;
			let exponent = (bits >> 10) & 0x1f;
			assert!(
				exponent > 0,
				"scale {scale:.3e} packed as subnormal f16 (bits=0x{bits:04x}), which flushes to \
				 0 on FTZ mobile GPUs"
			);
			let decoded = half::f16::from_bits(bits).to_f32().exp2();
			assert!(
				(decoded - scale).abs() / scale < 0.01,
				"scale {scale:.3e} decoded to {decoded:.3e}"
			);
		}
	}

	#[test]
	fn worth_outlining_rejects_tiny_part_behind_view() {
		let v = view(1000.0);
		let mut scales = [ViewScale::default(); MAX_VIEWS];
		scales[0] = ViewScale {
			view: v.view,
			focal: v.projection.y_axis.y.abs() * v.viewport[3] * 0.5,
			line: 2.0,
		};
		let sight = Sight {
			frustums: Vec::new(),
			eye: v.view,
			ink: Some(Ink::default()),
			print: None,
			scales,
			views: 1,
		};
		// A tiny 2 mm cube 5 m behind the camera (z = +5.0 -> depth = -5.0).
		let behind = Aabb {
			min: Vec3::new(-0.001, -0.001, 4.999),
			max: Vec3::new(0.001, 0.001, 5.001),
		};
		assert!(
			!sight.worth_outlining(&behind, 1.0),
			"tiny part behind camera (depth=-5.0) passed depth <= radius"
		);
		// A large box enclosing the camera (depth = 0.0, radius > 1) still
		// counts.
		let enclosing = Aabb {
			min: Vec3::splat(-1.0),
			max: Vec3::splat(1.0),
		};
		assert!(sight.worth_outlining(&enclosing, 1.0));
	}

	#[test]
	fn camera_space_light_rotates_with_the_view() {
		let world_light = crate::protocol::Light {
			direction: [0.0, 0.0, -1.0],
			camera_space: false,
			..crate::protocol::Light::default()
		};
		let camera_light = crate::protocol::Light {
			camera_space: true,
			..world_light
		};
		// Camera rotated +90 deg around Y (looking along -X, so camera +Z is
		// world +X).
		let view_inverse = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
		let world = light_toward(&world_light, &view_inverse);
		assert!((world - Vec3::Z).length() < 1.0e-6, "{world:?}");

		let camera = light_toward(&camera_light, &view_inverse);
		assert!((camera - Vec3::X).length() < 1.0e-6, "{camera:?}");
	}

	/// A print target's `ink_weight` multiplies the part's own line weight
	/// from [`Op::SetInk`](crate::protocol::Op::SetInk), so a part set to no
	/// line keeps none.
	#[test]
	fn a_print_ink_weight_scales_the_parts_own_line() {
		let ink = Ink::default();
		let material = Material::default();
		let resolved = PrintOverride {
			ink_weight: Some(2.0),
			..PrintOverride::default()
		};
		let weighted = |weight: Option<f32>| {
			let part = Overrides {
				ink: PartInk {
					weight,
					color: None,
				},
				..Overrides::default()
			};
			part_ink_print(&part, &material, &resolved, false, Some(&ink))[3]
		};
		assert_eq!(weighted(None), 2.0);
		assert_eq!(weighted(Some(0.5)), 1.0);
		assert_eq!(weighted(Some(0.0)), 0.0, "a part without a line keeps none");
	}

	/// Paper knockout keeps one depth-stencil attachment and one pipeline
	/// set while figures walk in and out of view: what a frame draws with
	/// follows the style, not what the frame happens to see.
	#[cfg(not(target_arch = "wasm32"))]
	#[test]
	fn knockout_attachments_hold_while_figures_leave_the_view() {
		use crate::protocol::AssetId;
		use crate::protocol::NodeId;
		use crate::protocol::Op;
		use crate::protocol::Transform;

		let gpu = match Gpu::headless() {
			Ok(gpu) => gpu,
			Err(error) if error.is_unavailable() => return,
			Err(error) => panic!("{error}"),
		};
		let asset = crate::Asset::from_slice(&fixture::lit_sphere(16, None)).unwrap();
		let mut renderer = Renderer::new(&gpu);
		let world = renderer.world_mut();
		world.apply([
			Op::LoadAsset {
				asset: AssetId(1),
				path: "models/sphere.glb".into(),
			},
			Op::Spawn {
				node: NodeId(1),
				asset: Some(AssetId(1)),
				parent: None,
			},
			Op::SetCamera {
				transform: Transform::from_translation(0.0, 0.0, 3.0),
				fov_y: 0.8,
				near: 0.1,
				far: 50.0,
			},
			Op::SetStyle(Style {
				shading: Shading::Print(Print::default()),
				ink: None,
			}),
		]);
		world.take_loads();
		world.asset_loaded(AssetId(1), Ok(Arc::new(asset)));
		world.take_events();

		// In view, behind the camera, and back.
		let keys: Vec<_> = [0.0, 40.0, 0.0]
			.into_iter()
			.map(|z| {
				renderer.world_mut().apply([Op::SetTransform {
					node: NodeId(1),
					transform: Transform::from_translation(0.0, 0.0, z),
				}]);
				renderer.advance(0.0);
				renderer.render_to_rgba(32, 32).unwrap();
				renderer
					.attachments
					.as_ref()
					.map(|attachments| attachments.key)
			})
			.collect();
		assert!(
			keys[0].is_some_and(|key| key.4),
			"a knockout style draws with a stencil"
		);
		assert_eq!(keys[1], keys[0], "the figure leaving keeps the attachments");
		assert_eq!(keys[2], keys[0], "the figure returning keeps them too");
	}
}
