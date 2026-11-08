//! The parts of the glTF 2.0 JSON the loader reads. Unknown fields are
//! ignored; the loader checks `extensionsRequired` itself.

use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Root {
	pub asset: AssetInfo,
	pub extensions_required: Vec<String>,
	pub scene: Option<usize>,
	pub scenes: Vec<Scene>,
	pub nodes: Vec<Node>,
	pub meshes: Vec<Mesh>,
	pub accessors: Vec<Accessor>,
	pub buffer_views: Vec<BufferView>,
	pub buffers: Vec<Buffer>,
	pub materials: Vec<Material>,
	pub textures: Vec<Texture>,
	pub images: Vec<Image>,
	pub samplers: Vec<Sampler>,
	pub skins: Vec<Skin>,
	pub animations: Vec<Animation>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct AssetInfo {
	pub version: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Scene {
	pub nodes: Vec<usize>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Node {
	pub name: Option<String>,
	pub children: Vec<usize>,
	pub mesh: Option<usize>,
	pub skin: Option<usize>,
	pub matrix: Option<[f32; 16]>,
	pub translation: Option<[f32; 3]>,
	pub rotation: Option<[f32; 4]>,
	pub scale: Option<[f32; 3]>,
	pub weights: Option<Vec<f32>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Mesh {
	pub name: Option<String>,
	pub primitives: Vec<Primitive>,
	pub weights: Option<Vec<f32>>,
	pub extras: Option<MeshExtras>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct MeshExtras {
	pub target_names: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Primitive {
	pub attributes: BTreeMap<String, usize>,
	pub indices: Option<usize>,
	pub material: Option<usize>,
	pub mode: Option<u32>,
	pub targets: Vec<BTreeMap<String, usize>>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Accessor {
	pub buffer_view: Option<usize>,
	pub byte_offset: usize,
	pub component_type: u32,
	pub normalized: bool,
	pub count: usize,
	#[serde(rename = "type")]
	pub kind: String,
	pub sparse: Option<Sparse>,
	pub min: Option<Vec<f32>>,
	pub max: Option<Vec<f32>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Sparse {
	pub count: usize,
	pub indices: SparseIndices,
	pub values: SparseValues,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SparseIndices {
	pub buffer_view: usize,
	pub byte_offset: usize,
	pub component_type: u32,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SparseValues {
	pub buffer_view: usize,
	pub byte_offset: usize,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BufferView {
	pub buffer: usize,
	pub byte_offset: usize,
	pub byte_length: usize,
	pub byte_stride: Option<usize>,
	pub extensions: Option<BufferViewExtensions>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct BufferViewExtensions {
	#[serde(rename = "EXT_meshopt_compression")]
	pub meshopt: Option<Meshopt>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Meshopt {
	pub buffer: usize,
	pub byte_offset: usize,
	pub byte_length: usize,
	pub byte_stride: usize,
	pub count: usize,
	pub mode: String,
	pub filter: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Buffer {
	pub byte_length: usize,
	pub uri: Option<String>,
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Material {
	pub name: Option<String>,
	pub pbr_metallic_roughness: Pbr,
	pub normal_texture: Option<NormalTextureInfo>,
	pub occlusion_texture: Option<OcclusionTextureInfo>,
	pub emissive_texture: Option<TextureInfo>,
	pub emissive_factor: [f32; 3],
	pub alpha_mode: String,
	pub alpha_cutoff: f32,
	pub double_sided: bool,
	pub extensions: MaterialExtensions,
}

impl Default for Material {
	fn default() -> Self {
		Self {
			name: None,
			pbr_metallic_roughness: Pbr::default(),
			normal_texture: None,
			occlusion_texture: None,
			emissive_texture: None,
			emissive_factor: [0.0; 3],
			alpha_mode: "OPAQUE".into(),
			alpha_cutoff: 0.5,
			double_sided: false,
			extensions: MaterialExtensions::default(),
		}
	}
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct MaterialExtensions {
	#[serde(rename = "KHR_materials_emissive_strength")]
	pub emissive_strength: Option<EmissiveStrength>,
	#[serde(rename = "KHR_materials_unlit")]
	pub unlit: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EmissiveStrength {
	pub emissive_strength: f32,
}

impl Default for EmissiveStrength {
	fn default() -> Self {
		Self {
			emissive_strength: 1.0,
		}
	}
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Pbr {
	pub base_color_factor: [f32; 4],
	pub base_color_texture: Option<TextureInfo>,
	pub metallic_factor: f32,
	pub roughness_factor: f32,
	pub metallic_roughness_texture: Option<TextureInfo>,
}

impl Default for Pbr {
	fn default() -> Self {
		Self {
			base_color_factor: [1.0; 4],
			base_color_texture: None,
			metallic_factor: 1.0,
			roughness_factor: 1.0,
			metallic_roughness_texture: None,
		}
	}
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TextureInfo {
	pub index: usize,
	pub tex_coord: u32,
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NormalTextureInfo {
	pub index: usize,
	pub tex_coord: u32,
	pub scale: f32,
}

impl Default for NormalTextureInfo {
	fn default() -> Self {
		Self {
			index: 0,
			tex_coord: 0,
			scale: 1.0,
		}
	}
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OcclusionTextureInfo {
	pub index: usize,
	pub tex_coord: u32,
	pub strength: f32,
}

impl Default for OcclusionTextureInfo {
	fn default() -> Self {
		Self {
			index: 0,
			tex_coord: 0,
			strength: 1.0,
		}
	}
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Texture {
	pub source: Option<usize>,
	pub sampler: Option<usize>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Image {
	pub uri: Option<String>,
	pub mime_type: Option<String>,
	pub buffer_view: Option<usize>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Sampler {
	pub mag_filter: Option<u32>,
	pub min_filter: Option<u32>,
	pub wrap_s: Option<u32>,
	pub wrap_t: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Skin {
	pub inverse_bind_matrices: Option<usize>,
	pub joints: Vec<usize>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Animation {
	pub name: Option<String>,
	pub channels: Vec<Channel>,
	pub samplers: Vec<AnimationSampler>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Channel {
	pub sampler: usize,
	pub target: ChannelTarget,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ChannelTarget {
	pub node: Option<usize>,
	pub path: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct AnimationSampler {
	pub input: usize,
	pub output: usize,
	pub interpolation: Option<String>,
}
