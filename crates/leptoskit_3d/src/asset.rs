//! A loaded model: meshes, materials, the node tree, skins, and animation
//! clips, decoded once and shared by every instance in every scene.

use std::collections::HashMap;

use glam::Mat4;
use glam::Quat;
use glam::Vec3;

/// One vertex as the GPU reads it: 36 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Vertex {
	pub position: [f32; 3],
	pub normal: [f32; 3],
	pub uv: [f32; 2],
	/// Linear RGBA, from `COLOR_0`; white when the mesh has none.
	pub color: [u8; 4],
}

/// The joints and weights of a skinned vertex: 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SkinVertex {
	pub joints: [u16; 4],
	/// Weights as unsigned normalized 16-bit values, summing to one.
	pub weights: [u16; 4],
}

/// An axis-aligned box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Aabb {
	pub min: Vec3,
	pub max: Vec3,
}

impl Aabb {
	pub const EMPTY: Self = Self {
		min: Vec3::splat(f32::INFINITY),
		max: Vec3::splat(f32::NEG_INFINITY),
	};

	pub fn is_empty(&self) -> bool {
		self.min.x > self.max.x
	}

	pub fn grow(&mut self, point: Vec3) {
		self.min = self.min.min(point);
		self.max = self.max.max(point);
	}

	pub fn union(self, other: Self) -> Self {
		Self {
			min: self.min.min(other.min),
			max: self.max.max(other.max),
		}
	}

	pub fn center(&self) -> Vec3 {
		(self.min + self.max) * 0.5
	}

	/// The box around this one after `matrix` moves it.
	pub fn transformed(&self, matrix: &Mat4) -> Self {
		if self.is_empty() {
			return *self;
		}

		let center = matrix.transform_point3(self.center());
		let extent = (self.max - self.min) * 0.5;
		let x = matrix.x_axis.truncate().abs() * extent.x;
		let y = matrix.y_axis.truncate().abs() * extent.y;
		let z = matrix.z_axis.truncate().abs() * extent.z;
		let half = x + y + z;

		Self {
			min: center - half,
			max: center + half,
		}
	}
}

/// Where a primitive's morph targets live in [`Asset::morph_deltas`]. Target
/// `t` of vertex `v` has its position delta at index
/// `base + (t * vertex_count + v) * 2`, and its normal delta right after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MorphRange {
	pub base: u32,
	pub targets: u32,
}

/// A drawable piece of a mesh with one material. Its indices count from its
/// own first vertex.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Primitive {
	pub base_vertex: u32,
	pub vertex_count: u32,
	pub first_index: u32,
	pub index_count: u32,
	pub material: usize,
	pub bounds: Aabb,
	/// Where its joints and weights start in [`Asset::skin_vertices`].
	pub skin_base: Option<u32>,
	pub morph: Option<MorphRange>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Mesh {
	pub primitives: Vec<Primitive>,
	pub target_names: Vec<String>,
	pub weights: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AssetNode {
	pub name: String,
	pub parent: Option<usize>,
	pub children: Vec<usize>,
	pub translation: Vec3,
	pub rotation: Quat,
	pub scale: Vec3,
	pub mesh: Option<usize>,
	pub skin: Option<usize>,
	pub weights: Option<Vec<f32>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Skin {
	pub joints: Vec<usize>,
	pub inverse_bind: Vec<Mat4>,
	/// How far any vertex a joint moves lies from it, in the joint's space,
	/// so a posed skin's bounds follow from its joints alone.
	pub joint_radius: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChannelPath {
	Translation,
	Rotation,
	Scale,
	Weights,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Interpolation {
	Step,
	Linear,
	CubicSpline,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Channel {
	pub node: usize,
	pub path: ChannelPath,
	pub interpolation: Interpolation,
	pub times: Vec<f32>,
	/// `width` floats per key, or three times that for cubic splines (in
	/// tangent, value, out tangent).
	pub values: Vec<f32>,
	pub width: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Clip {
	pub name: String,
	pub duration: f32,
	pub channels: Vec<Channel>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AlphaMode {
	Opaque,
	Mask(f32),
	Blend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub(crate) struct SamplerDesc {
	pub nearest: bool,
	pub repeat_u: u8,
	pub repeat_v: u8,
}

/// A texture slot: which decoded image, sampled how.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TextureRef {
	pub image: usize,
	pub sampler: SamplerDesc,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Material {
	pub name: String,
	pub base_color: [f32; 4],
	pub base_color_texture: Option<TextureRef>,
	pub metallic: f32,
	pub roughness: f32,
	pub metallic_roughness_texture: Option<TextureRef>,
	pub normal_texture: Option<TextureRef>,
	pub normal_scale: f32,
	pub occlusion_texture: Option<TextureRef>,
	pub occlusion_strength: f32,
	pub emissive: [f32; 3],
	pub emissive_texture: Option<TextureRef>,
	pub emissive_strength: f32,
	pub alpha: AlphaMode,
	pub double_sided: bool,
	pub unlit: bool,
}

impl Default for Material {
	fn default() -> Self {
		Self {
			name: String::new(),
			base_color: [1.0; 4],
			base_color_texture: None,
			metallic: 1.0,
			roughness: 1.0,
			metallic_roughness_texture: None,
			normal_texture: None,
			normal_scale: 1.0,
			occlusion_texture: None,
			occlusion_strength: 1.0,
			emissive: [0.0; 3],
			emissive_texture: None,
			emissive_strength: 1.0,
			alpha: AlphaMode::Opaque,
			double_sided: false,
			unlit: false,
		}
	}
}

/// A decoded image, RGBA with 8 bits a channel.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Image {
	pub width: u32,
	pub height: u32,
	pub pixels: Vec<u8>,
}

/// A model loaded from a glTF file, decoded and ready to draw.
///
/// Load one with [`Asset::from_slice`]. It holds no GPU objects: a renderer
/// uploads it once per device, and every instance of it shares those
/// buffers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Asset {
	pub(crate) nodes: Vec<AssetNode>,
	pub(crate) roots: Vec<usize>,
	pub(crate) meshes: Vec<Mesh>,
	pub(crate) materials: Vec<Material>,
	pub(crate) images: Vec<Image>,
	pub(crate) skins: Vec<Skin>,
	pub(crate) clips: Vec<Clip>,
	pub(crate) vertices: Vec<Vertex>,
	pub(crate) skin_vertices: Vec<SkinVertex>,
	pub(crate) indices: Vec<u32>,
	pub(crate) morph_deltas: Vec<[f32; 4]>,
	pub(crate) names: HashMap<String, usize>,
}

impl Asset {
	/// The animation clips' names, in file order.
	pub fn clip_names(&self) -> impl Iterator<Item = &str> {
		self.clips.iter().map(|clip| clip.name.as_str())
	}

	/// The nodes' names, in file order. These are the parts a guest can show,
	/// hide, restyle, and attach to.
	pub fn node_names(&self) -> impl Iterator<Item = &str> {
		self.nodes.iter().map(|node| node.name.as_str())
	}

	/// The morph targets' names across every mesh, each once.
	pub fn morph_target_names(&self) -> Vec<&str> {
		let mut names: Vec<&str> = self
			.meshes
			.iter()
			.flat_map(|mesh| mesh.target_names.iter().map(String::as_str))
			.collect();
		names.sort_unstable();
		names.dedup();
		names
	}

	/// How many vertices its meshes hold.
	pub fn vertex_count(&self) -> usize {
		self.vertices.len()
	}

	/// How many triangles its meshes hold.
	pub fn triangle_count(&self) -> usize {
		self.indices.len() / 3
	}

	/// How many meshes it has, and how many primitives they hold.
	pub fn mesh_count(&self) -> (usize, usize) {
		(
			self.meshes.len(),
			self.meshes.iter().map(|mesh| mesh.primitives.len()).sum(),
		)
	}

	/// How many skins it has.
	pub fn skin_count(&self) -> usize {
		self.skins.len()
	}

	/// A clip's length in seconds.
	pub fn clip_duration(&self, name: &str) -> Option<f32> {
		self.clips
			.iter()
			.find(|clip| clip.name == name)
			.map(|clip| clip.duration)
	}

	/// The node with `name`, the first one when several share it.
	pub(crate) fn node_named(&self, name: &str) -> Option<usize> {
		self.names.get(name).copied()
	}
}
