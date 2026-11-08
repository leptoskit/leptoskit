//! The glTF 2.0 loader: binary `.glb` and JSON `.gltf` files, with
//! `EXT_meshopt_compression`, `KHR_mesh_quantization`,
//! `KHR_materials_emissive_strength`, and `KHR_materials_unlit`.

mod json;
pub mod meshopt;

use std::borrow::Cow;
use std::collections::HashMap;

use glam::Mat4;
use glam::Quat;
use glam::Vec3;

use crate::asset::*;
use crate::sanitize::*;

/// Why a model could not load.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
	/// The file is not glTF, or its JSON does not parse.
	#[error("the file is not valid glTF: {0}")]
	Format(String),
	/// The file needs an extension the loader does not implement.
	#[error("the file requires the unsupported extension {0}")]
	Extension(String),
	/// A part of the file refers to something that is not there, or holds
	/// data of the wrong shape.
	#[error("the file is malformed: {0}")]
	Malformed(String),
	/// The file is larger than the loader's limits allow.
	#[error("the file is too large: {0}")]
	TooLarge(&'static str),
	/// A buffer or image the file names could not be read.
	#[error("{0}")]
	Resource(String),
	/// Compressed geometry did not decode.
	#[error(transparent)]
	Meshopt(#[from] meshopt::MeshoptError),
	/// An image did not decode.
	#[error("an image did not decode: {0}")]
	Image(String),
}

fn malformed(message: impl Into<String>) -> LoadError {
	LoadError::Malformed(message.into())
}

const SUPPORTED_EXTENSIONS: &[&str] = &[
	"EXT_meshopt_compression",
	"KHR_mesh_quantization",
	"KHR_materials_emissive_strength",
	"KHR_materials_unlit",
];

/// Splits a `.glb` into its JSON and binary chunks; any other bytes are
/// taken to be JSON.
fn container(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>), LoadError> {
	if !bytes.starts_with(b"glTF") {
		return Ok((bytes, None));
	}
	let word = |offset: usize| -> Result<usize, LoadError> {
		bytes
			.get(offset..offset + 4)
			.map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]) as usize)
			.ok_or_else(|| LoadError::Format("the binary header ends early".into()))
	};
	if word(4)? != 2 {
		return Err(LoadError::Format(
			"only glTF 2 binaries are supported".into(),
		));
	}
	let length = word(8)?.min(bytes.len());
	let mut offset = 12;
	let mut json = None;
	let mut bin = None;
	while offset + 8 <= length {
		let size = word(offset)?;
		let kind = word(offset + 4)?;
		let start = offset + 8;
		let end = start
			.checked_add(size)
			.filter(|end| *end <= length)
			.ok_or_else(|| LoadError::Format("a chunk runs past the end of the file".into()))?;
		match kind {
			0x4E4F_534A if json.is_none() => json = Some(&bytes[start..end]),
			0x004E_4942 if bin.is_none() => bin = Some(&bytes[start..end]),
			_ => {}
		}
		offset = end.next_multiple_of(4);
	}
	let json = json.ok_or_else(|| LoadError::Format("the binary has no JSON chunk".into()))?;
	Ok((json, bin))
}

fn decode_base64(text: &str) -> Result<Vec<u8>, LoadError> {
	fn value(byte: u8) -> Option<u32> {
		Some(u32::from(match byte {
			b'A'..=b'Z' => byte - b'A',
			b'a'..=b'z' => byte - b'a' + 26,
			b'0'..=b'9' => byte - b'0' + 52,
			b'+' | b'-' => 62,
			b'/' | b'_' => 63,
			_ => return None,
		}))
	}
	let digits: Vec<u8> = text
		.bytes()
		.filter(|byte| !byte.is_ascii_whitespace() && *byte != b'=')
		.collect();
	let mut output = Vec::with_capacity(digits.len() * 3 / 4);
	for chunk in digits.chunks(4) {
		let mut word = 0;
		for (index, byte) in chunk.iter().enumerate() {
			word |= value(*byte).ok_or_else(|| malformed("a data URI is not base64"))?
				<< (18 - index * 6);
		}
		let bytes = word.to_be_bytes();
		output.extend_from_slice(&bytes[1..chunk.len()]);
	}
	Ok(output)
}

/// Reads a buffer or image URI: an embedded `data:` URI, or a file beside the
/// model through `resolve`.
fn read_uri(
	uri: &str,
	resolve: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, LoadError> {
	if let Some(data) = uri.strip_prefix("data:") {
		let (header, payload) = data
			.split_once(',')
			.ok_or_else(|| malformed("a data URI has no comma"))?;
		if !header.ends_with(";base64") {
			return Err(malformed("only base64 data URIs are supported"));
		}
		return decode_base64(payload);
	}
	let path = percent_decode(uri);
	if !is_safe_asset_path(&path) {
		return Err(LoadError::Resource(format!(
			"the file refers to {path:?}, which is outside its folder"
		)));
	}
	resolve(&path).map_err(LoadError::Resource)
}

fn percent_decode(uri: &str) -> String {
	let bytes = uri.as_bytes();
	let mut output = Vec::with_capacity(bytes.len());
	let mut index = 0;
	while index < bytes.len() {
		if bytes[index] == b'%'
			&& let Some(hex) = uri.get(index + 1..index + 3)
			&& let Ok(byte) = u8::from_str_radix(hex, 16)
		{
			output.push(byte);
			index += 3;
		} else {
			output.push(bytes[index]);
			index += 1;
		}
	}
	String::from_utf8_lossy(&output).into_owned()
}

struct Document<'a> {
	root: json::Root,
	buffers: Vec<Cow<'a, [u8]>>,
	views: Vec<Option<Vec<u8>>>,
}

fn components(kind: &str) -> Result<usize, LoadError> {
	Ok(match kind {
		"SCALAR" => 1,
		"VEC2" => 2,
		"VEC3" => 3,
		"VEC4" | "MAT2" => 4,
		"MAT3" => 9,
		"MAT4" => 16,
		_ => return Err(malformed(format!("unknown accessor type {kind}"))),
	})
}

fn component_size(component_type: u32) -> Result<usize, LoadError> {
	Ok(match component_type {
		5120 | 5121 => 1,
		5122 | 5123 => 2,
		5125 | 5126 => 4,
		_ => {
			return Err(malformed(format!(
				"unknown component type {component_type}"
			)));
		}
	})
}

/// Reads one component as a float, normalizing integers when asked.
fn component(bytes: &[u8], component_type: u32, normalized: bool) -> f32 {
	match (component_type, normalized) {
		(5120, true) => (f32::from(bytes[0] as i8) / 127.0).max(-1.0),
		(5120, false) => f32::from(bytes[0] as i8),
		(5121, true) => f32::from(bytes[0]) / 255.0,
		(5121, false) => f32::from(bytes[0]),
		(5122, true) => (f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32767.0).max(-1.0),
		(5122, false) => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])),
		(5123, true) => f32::from(u16::from_le_bytes([bytes[0], bytes[1]])) / 65535.0,
		(5123, false) => f32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
		(5125, _) => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32,
		_ => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
	}
}

fn integer(bytes: &[u8], component_type: u32) -> u32 {
	match component_type {
		5120 | 5121 => u32::from(bytes[0]),
		5122 | 5123 => u32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
		_ => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
	}
}

impl<'a> Document<'a> {
	fn parse(
		bytes: &'a [u8],
		resolve: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
	) -> Result<Self, LoadError> {
		if bytes.len() > MAX_ASSET_BYTES {
			return Err(LoadError::TooLarge("the file is larger than 128 MiB"));
		}
		let (json, bin) = container(bytes)?;
		let root: json::Root =
			serde_json::from_slice(json).map_err(|error| LoadError::Format(error.to_string()))?;
		if !root.asset.version.starts_with('2') {
			return Err(LoadError::Format("only glTF 2.0 is supported".into()));
		}
		if let Some(extension) = root
			.extensions_required
			.iter()
			.find(|extension| !SUPPORTED_EXTENSIONS.contains(&extension.as_str()))
		{
			return Err(LoadError::Extension(extension.clone()));
		}
		if root.nodes.len() > MAX_ASSET_NODES {
			return Err(LoadError::TooLarge("the file has too many nodes"));
		}
		if root.animations.len() > MAX_CLIPS {
			return Err(LoadError::TooLarge("the file has too many animations"));
		}
		let mut buffers = Vec::with_capacity(root.buffers.len());
		let mut total = 0_usize;
		for (index, buffer) in root.buffers.iter().enumerate() {
			let data: Cow<'a, [u8]> = match (&buffer.uri, bin) {
				(Some(uri), _) => Cow::Owned(read_uri(uri, resolve)?),
				(None, Some(bin)) if index == 0 => Cow::Borrowed(bin),
				// A buffer with neither is a meshopt fallback, which the
				// compressed views replace.
				(None, _) => Cow::Borrowed(&[]),
			};
			total = total.saturating_add(data.len());
			if total > MAX_ASSET_BYTES {
				return Err(LoadError::TooLarge("the buffers are larger than 128 MiB"));
			}
			buffers.push(data);
		}
		let views = (0..root.buffer_views.len()).map(|_| None).collect();
		let mut document = Self {
			root,
			buffers,
			views,
		};
		document.decompress()?;
		Ok(document)
	}

	/// Decodes every meshopt-compressed buffer view up front.
	fn decompress(&mut self) -> Result<(), LoadError> {
		let mut total = 0_usize;
		for (index, view) in self.root.buffer_views.iter().enumerate() {
			let Some(compressed) = view
				.extensions
				.as_ref()
				.and_then(|ext| ext.meshopt.as_ref())
			else {
				continue;
			};
			let source = self
				.buffers
				.get(compressed.buffer)
				.and_then(|buffer| {
					buffer.get(
						compressed.byte_offset
							..compressed.byte_offset.checked_add(compressed.byte_length)?,
					)
				})
				.ok_or_else(|| malformed("a compressed view runs past its buffer"))?;
			let size = compressed
				.count
				.checked_mul(compressed.byte_stride)
				.ok_or(LoadError::TooLarge("a compressed view is too large"))?;
			total = total.saturating_add(size);
			if total > MAX_ASSET_BYTES * 2 {
				return Err(LoadError::TooLarge("the decompressed views are too large"));
			}
			let mode = match compressed.mode.as_str() {
				"ATTRIBUTES" => meshopt::Mode::Attributes,
				"TRIANGLES" => meshopt::Mode::Triangles,
				"INDICES" => meshopt::Mode::Indices,
				_ => return Err(malformed("an unknown meshopt mode")),
			};
			let filter = match compressed.filter.as_deref().unwrap_or("NONE") {
				"NONE" => meshopt::Filter::None,
				"OCTAHEDRAL" => meshopt::Filter::Octahedral,
				"QUATERNION" => meshopt::Filter::Quaternion,
				"EXPONENTIAL" => meshopt::Filter::Exponential,
				_ => return Err(malformed("an unknown meshopt filter")),
			};
			let decoded = meshopt::decode(
				source,
				compressed.count,
				compressed.byte_stride,
				mode,
				filter,
			)?;
			self.views[index] = Some(decoded);
		}
		Ok(())
	}

	fn view(&self, index: usize) -> Result<(&[u8], Option<usize>), LoadError> {
		let view = self
			.root
			.buffer_views
			.get(index)
			.ok_or_else(|| malformed("an accessor names a missing buffer view"))?;
		if let Some(decoded) = &self.views[index] {
			return Ok((decoded, view.byte_stride));
		}
		let buffer = self
			.buffers
			.get(view.buffer)
			.ok_or_else(|| malformed("a buffer view names a missing buffer"))?;
		let end = view
			.byte_offset
			.checked_add(view.byte_length)
			.ok_or_else(|| malformed("a buffer view overflows"))?;
		let bytes = buffer
			.get(view.byte_offset..end)
			.ok_or_else(|| malformed("a buffer view runs past its buffer"))?;
		Ok((bytes, view.byte_stride))
	}

	fn accessor(&self, index: usize) -> Result<&json::Accessor, LoadError> {
		self.root
			.accessors
			.get(index)
			.ok_or_else(|| malformed("a missing accessor"))
	}

	/// Calls `each` with every element's bytes, in order, sparse values
	/// included.
	fn elements(
		&self,
		index: usize,
		mut each: impl FnMut(usize, &[u8]),
	) -> Result<(usize, &json::Accessor), LoadError> {
		let accessor = self.accessor(index)?;
		let width = components(&accessor.kind)?;
		let size = component_size(accessor.component_type)?;
		let element = width * size;
		if accessor.count > MAX_ASSET_INDICES {
			return Err(LoadError::TooLarge("an accessor has too many elements"));
		}
		if let Some(view) = accessor.buffer_view {
			let (bytes, stride) = self.view(view)?;
			let stride = stride.unwrap_or(element).max(element);
			if accessor.count > 0 {
				let last = accessor
					.byte_offset
					.checked_add(stride * (accessor.count - 1) + element)
					.ok_or_else(|| malformed("an accessor overflows"))?;
				if last > bytes.len() {
					return Err(malformed("an accessor runs past its buffer view"));
				}
			}
			for item in 0..accessor.count {
				let start = accessor.byte_offset + item * stride;
				each(item, &bytes[start..start + element]);
			}
		} else {
			let zeros = vec![0; element];
			for item in 0..accessor.count {
				each(item, &zeros);
			}
		}
		if let Some(sparse) = &accessor.sparse {
			if sparse.count > accessor.count {
				return Err(malformed("a sparse accessor replaces more than it holds"));
			}
			let (indices, _) = self.view(sparse.indices.buffer_view)?;
			let (values, _) = self.view(sparse.values.buffer_view)?;
			let index_size = component_size(sparse.indices.component_type)?;
			for item in 0..sparse.count {
				let at = sparse.indices.byte_offset + item * index_size;
				let target = indices
					.get(at..at + index_size)
					.map(|bytes| integer(bytes, sparse.indices.component_type) as usize)
					.filter(|target| *target < accessor.count)
					.ok_or_else(|| malformed("a sparse index is out of range"))?;
				let at = sparse.values.byte_offset + item * element;
				let value = values
					.get(at..at + element)
					.ok_or_else(|| malformed("a sparse value runs past its view"))?;
				each(target, value);
			}
		}
		Ok((width, accessor))
	}

	/// Reads an accessor as floats, `width` per element.
	fn floats(&self, index: usize) -> Result<(Vec<f32>, usize), LoadError> {
		let accessor = self.accessor(index)?;
		let width = components(&accessor.kind)?;
		let size = component_size(accessor.component_type)?;
		let (component_type, normalized) = (accessor.component_type, accessor.normalized);
		let mut values = vec![0.0; accessor.count * width];
		self.elements(index, |item, bytes| {
			for (slot, chunk) in values[item * width..(item + 1) * width]
				.iter_mut()
				.zip(bytes.chunks_exact(size))
			{
				*slot = component(chunk, component_type, normalized);
			}
		})?;
		Ok((values, width))
	}

	/// Reads an accessor of unsigned integers, `width` per element.
	fn integers(&self, index: usize) -> Result<(Vec<u32>, usize), LoadError> {
		let accessor = self.accessor(index)?;
		let width = components(&accessor.kind)?;
		let size = component_size(accessor.component_type)?;
		let component_type = accessor.component_type;
		if component_type == 5126 {
			return Err(malformed("indices or joints must be integers"));
		}
		let mut values = vec![0; accessor.count * width];
		self.elements(index, |item, bytes| {
			for (slot, chunk) in values[item * width..(item + 1) * width]
				.iter_mut()
				.zip(bytes.chunks_exact(size))
			{
				*slot = integer(chunk, component_type);
			}
		})?;
		Ok((values, width))
	}

	fn count(&self, index: usize) -> Result<usize, LoadError> {
		Ok(self.accessor(index)?.count)
	}
}

/// Loads a model from the bytes of a `.glb` or `.gltf` file. `resolve` reads
/// a file the model refers to by a relative path, such as a `.bin` buffer or
/// a texture beside a `.gltf`; it is never asked for a path outside the
/// model's folder.
pub(crate) fn load(
	bytes: &[u8],
	resolve: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Asset, LoadError> {
	let document = Document::parse(bytes, resolve)?;
	let mut asset = Asset::default();
	load_nodes(&document, &mut asset)?;
	load_materials(&document, &mut asset, resolve)?;
	load_meshes(&document, &mut asset)?;
	load_skins(&document, &mut asset)?;
	load_clips(&document, &mut asset)?;
	Ok(asset)
}

fn load_nodes(document: &Document<'_>, asset: &mut Asset) -> Result<(), LoadError> {
	let root = &document.root;
	let mut parents = vec![None; root.nodes.len()];
	for (index, node) in root.nodes.iter().enumerate() {
		for &child in &node.children {
			if child >= root.nodes.len() || child == index {
				return Err(malformed("a node has a missing child"));
			}
			if parents[child].replace(index).is_some() {
				return Err(malformed("a node has two parents"));
			}
		}
	}
	for (index, node) in root.nodes.iter().enumerate() {
		let (scale, rotation, translation) = match node.matrix {
			Some(matrix) => Mat4::from_cols_array(&matrix).to_scale_rotation_translation(),
			None => {
				(
					Vec3::from(node.scale.unwrap_or([1.0; 3])),
					Quat::from_array(node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0])).normalize(),
					Vec3::from(node.translation.unwrap_or([0.0; 3])),
				)
			}
		};
		if node.mesh.is_some_and(|mesh| mesh >= root.meshes.len())
			|| node.skin.is_some_and(|skin| skin >= root.skins.len())
		{
			return Err(malformed("a node names a missing mesh or skin"));
		}
		let name = node.name.clone().unwrap_or_else(|| format!("node{index}"));
		asset.names.entry(name.clone()).or_insert(index);
		asset.nodes.push(AssetNode {
			name,
			parent: parents[index],
			children: node.children.clone(),
			translation,
			rotation: if rotation.is_finite() {
				rotation
			} else {
				Quat::IDENTITY
			},
			scale,
			mesh: node.mesh,
			skin: node.skin,
			weights: node.weights.clone(),
		});
	}
	// A cycle leaves nodes with parents but never reached from a root.
	let scene = root
		.scene
		.and_then(|scene| root.scenes.get(scene))
		.or(root.scenes.first());
	asset.roots = match scene {
		Some(scene) => scene.nodes.clone(),
		None => {
			(0..root.nodes.len())
				.filter(|index| parents[*index].is_none())
				.collect()
		}
	};
	if asset
		.roots
		.iter()
		.any(|root| *root >= asset.nodes.len() || parents[*root].is_some())
	{
		return Err(malformed("a scene's root is missing or has a parent"));
	}
	let mut reached = vec![false; asset.nodes.len()];
	let mut stack = asset.roots.clone();
	while let Some(index) = stack.pop() {
		if std::mem::replace(&mut reached[index], true) {
			return Err(malformed("the node tree has a cycle"));
		}
		stack.extend(asset.nodes[index].children.iter().copied());
	}
	Ok(())
}

fn texture_ref(
	document: &Document<'_>,
	index: usize,
	tex_coord: u32,
) -> Result<Option<TextureRef>, LoadError> {
	if tex_coord != 0 {
		// Only the first texture coordinate set is read.
		return Ok(None);
	}
	let texture = document
		.root
		.textures
		.get(index)
		.ok_or_else(|| malformed("a material names a missing texture"))?;
	let Some(image) = texture.source else {
		return Ok(None);
	};
	if image >= document.root.images.len() {
		return Err(malformed("a texture names a missing image"));
	}
	let sampler = texture
		.sampler
		.and_then(|sampler| document.root.samplers.get(sampler));
	let wrap = |mode: Option<u32>| {
		match mode {
			Some(33071) => 0,
			Some(33648) => 2,
			_ => 1,
		}
	};
	Ok(Some(TextureRef {
		image,
		sampler: SamplerDesc {
			nearest: sampler.and_then(|sampler| sampler.mag_filter) == Some(9728),
			repeat_u: wrap(sampler.and_then(|sampler| sampler.wrap_s)),
			repeat_v: wrap(sampler.and_then(|sampler| sampler.wrap_t)),
		},
	}))
}

fn load_materials(
	document: &Document<'_>,
	asset: &mut Asset,
	resolve: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), LoadError> {
	let mut used = vec![false; document.root.images.len()];
	for material in &document.root.materials {
		let pbr = &material.pbr_metallic_roughness;
		let mut slot = |info: Option<(usize, u32)>| -> Result<Option<TextureRef>, LoadError> {
			let Some((index, tex_coord)) = info else {
				return Ok(None);
			};
			let texture = texture_ref(document, index, tex_coord)?;
			if let Some(texture) = texture {
				used[texture.image] = true;
			}
			Ok(texture)
		};
		let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
		asset.materials.push(Material {
			name: material.name.clone().unwrap_or_default(),
			base_color: pbr
				.base_color_factor
				.map(|value| finite(value, 1.0).clamp(0.0, 1.0)),
			base_color_texture: slot(
				pbr.base_color_texture
					.as_ref()
					.map(|t| (t.index, t.tex_coord)),
			)?,
			metallic: finite(pbr.metallic_factor, 1.0).clamp(0.0, 1.0),
			roughness: finite(pbr.roughness_factor, 1.0).clamp(0.0, 1.0),
			metallic_roughness_texture: slot(
				pbr.metallic_roughness_texture
					.as_ref()
					.map(|t| (t.index, t.tex_coord)),
			)?,
			normal_texture: slot(
				material
					.normal_texture
					.as_ref()
					.map(|t| (t.index, t.tex_coord)),
			)?,
			normal_scale: material
				.normal_texture
				.as_ref()
				.map_or(1.0, |t| finite(t.scale, 1.0)),
			occlusion_texture: slot(
				material
					.occlusion_texture
					.as_ref()
					.map(|t| (t.index, t.tex_coord)),
			)?,
			occlusion_strength: material
				.occlusion_texture
				.as_ref()
				.map_or(1.0, |t| finite(t.strength, 1.0).clamp(0.0, 1.0)),
			emissive: material
				.emissive_factor
				.map(|value| finite(value, 0.0).clamp(0.0, 1.0)),
			emissive_texture: slot(
				material
					.emissive_texture
					.as_ref()
					.map(|t| (t.index, t.tex_coord)),
			)?,
			emissive_strength: material
				.extensions
				.emissive_strength
				.as_ref()
				.map_or(1.0, |strength| {
					finite(strength.emissive_strength, 1.0).clamp(0.0, MAX_INTENSITY)
				}),
			alpha: match material.alpha_mode.as_str() {
				"MASK" => AlphaMode::Mask(finite(material.alpha_cutoff, 0.5).clamp(0.0, 1.0)),
				"BLEND" => AlphaMode::Blend,
				_ => AlphaMode::Opaque,
			},
			double_sided: material.double_sided,
			unlit: material.extensions.unlit.is_some(),
		});
	}
	// The default material, for primitives that name none.
	asset.materials.push(Material::default());
	for (index, image) in document.root.images.iter().enumerate() {
		asset.images.push(if used[index] {
			load_image(document, image, resolve)?
		} else {
			Image {
				width: 1,
				height: 1,
				pixels: vec![255; 4],
			}
		});
	}
	Ok(())
}

fn load_image(
	document: &Document<'_>,
	image: &json::Image,
	resolve: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Image, LoadError> {
	let owned;
	let bytes: &[u8] = match (&image.uri, image.buffer_view) {
		(_, Some(view)) => document.view(view)?.0,
		(Some(uri), None) => {
			owned = read_uri(uri, resolve)?;
			&owned
		}
		(None, None) => return Err(malformed("an image has neither a URI nor a buffer view")),
	};
	crate::image::decode(bytes).map_err(LoadError::Image)
}

/// The vertex count of a primitive: its `POSITION` accessor's.
fn primitive_vertices(
	document: &Document<'_>,
	primitive: &json::Primitive,
) -> Result<usize, LoadError> {
	let position = primitive
		.attributes
		.get("POSITION")
		.ok_or_else(|| malformed("a primitive has no positions"))?;
	document.count(*position)
}

/// Turns strips and fans into a triangle list.
fn triangle_list(mode: u32, indices: Vec<u32>) -> Vec<u32> {
	match mode {
		5 => {
			(0..indices.len().saturating_sub(2))
				.flat_map(|index| {
					if index % 2 == 0 {
						[indices[index], indices[index + 1], indices[index + 2]]
					} else {
						[indices[index + 1], indices[index], indices[index + 2]]
					}
				})
				.collect()
		}
		6 => {
			(1..indices.len().saturating_sub(1))
				.flat_map(|index| [indices[0], indices[index], indices[index + 1]])
				.collect()
		}
		_ => {
			let mut indices = indices;
			indices.truncate(indices.len() / 3 * 3);
			indices
		}
	}
}

/// Smooth normals weighted by triangle area, for meshes that have none.
fn computed_normals(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
	let mut normals = vec![Vec3::ZERO; positions.len()];
	for triangle in indices.as_chunks::<3>().0 {
		let [a, b, c] = [triangle[0], triangle[1], triangle[2]].map(|index| index as usize);
		let normal = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
		for index in [a, b, c] {
			normals[index] += normal;
		}
	}
	normals
		.into_iter()
		.map(|normal| normal.try_normalize().unwrap_or(Vec3::Y))
		.collect()
}

#[allow(clippy::too_many_lines)]
fn load_primitive(
	document: &Document<'_>,
	asset: &mut Asset,
	primitive: &json::Primitive,
	target_count: usize,
) -> Result<Option<Primitive>, LoadError> {
	let mode = primitive.mode.unwrap_or(4);
	if !matches!(mode, 4..=6) {
		// Points and lines are not drawn.
		return Ok(None);
	}
	let count = primitive_vertices(document, primitive)?;
	if asset.vertices.len() + count > MAX_ASSET_VERTICES {
		return Err(LoadError::TooLarge("the meshes have too many vertices"));
	}
	let attribute = |name: &str| primitive.attributes.get(name).copied();
	let read = |index: usize, width: usize| -> Result<Vec<f32>, LoadError> {
		let (values, actual) = document.floats(index)?;
		if values.len() / actual.max(1) != count || actual < width {
			return Err(malformed("an attribute's count or type does not match"));
		}
		Ok(values
			.chunks_exact(actual)
			.flat_map(|chunk| chunk[..width].to_vec())
			.collect())
	};
	let positions: Vec<Vec3> = read(attribute("POSITION").unwrap_or_default(), 3)?
		.as_chunks::<3>()
		.0
		.iter()
		.map(|chunk| {
			let point = Vec3::from_slice(chunk);
			if point.is_finite() { point } else { Vec3::ZERO }
		})
		.collect();
	let indices = match primitive.indices {
		Some(index) => document.integers(index)?.0,
		None => (0..count as u32).collect(),
	};
	if indices.iter().any(|index| *index as usize >= count) {
		return Err(malformed("an index is past the end of its vertices"));
	}
	let indices = triangle_list(mode, indices);
	if asset.indices.len() + indices.len() > MAX_ASSET_INDICES {
		return Err(LoadError::TooLarge("the meshes have too many indices"));
	}
	let normals: Vec<Vec3> = match attribute("NORMAL") {
		Some(index) => {
			read(index, 3)?
				.as_chunks::<3>()
				.0
				.iter()
				.map(|chunk| Vec3::from_slice(chunk).try_normalize().unwrap_or(Vec3::Y))
				.collect()
		}
		None => computed_normals(&positions, &indices),
	};
	let uvs = match attribute("TEXCOORD_0") {
		Some(index) => read(index, 2)?,
		None => vec![0.0; count * 2],
	};
	let colors: Vec<[u8; 4]> = match attribute("COLOR_0") {
		Some(index) => {
			let (values, width) = document.floats(index)?;
			if values.len() / width.max(1) != count || !(3..=4).contains(&width) {
				return Err(malformed("COLOR_0 must be VEC3 or VEC4 per vertex"));
			}
			values
				.chunks_exact(width)
				.map(|chunk| {
					let channel = |index: usize| {
						(chunk.get(index).copied().unwrap_or(1.0).clamp(0.0, 1.0) * 255.0).round()
							as u8
					};
					[channel(0), channel(1), channel(2), channel(3)]
				})
				.collect()
		}
		None => vec![[255; 4]; count],
	};
	let mut bounds = Aabb::EMPTY;
	for point in &positions {
		bounds.grow(*point);
	}
	let base_vertex = asset.vertices.len() as u32;
	for index in 0..count {
		asset.vertices.push(Vertex {
			position: positions[index].to_array(),
			normal: normals[index].to_array(),
			uv: [uvs[index * 2], uvs[index * 2 + 1]],
			color: colors[index],
		});
	}
	let skin_base = match (attribute("JOINTS_0"), attribute("WEIGHTS_0")) {
		(Some(joints), Some(weights)) => {
			let (joints, joint_width) = document.integers(joints)?;
			let weights = read(weights, 4)?;
			if joint_width != 4 || joints.len() != count * 4 {
				return Err(malformed("JOINTS_0 must be VEC4 per vertex"));
			}
			let base = asset.skin_vertices.len() as u32;
			for index in 0..count {
				let mut weight: [f32; 4] = weights[index * 4..index * 4 + 4]
					.try_into()
					.unwrap_or_default();
				weight = weight.map(|value| {
					if value.is_finite() {
						value.max(0.0)
					} else {
						0.0
					}
				});
				let sum: f32 = weight.iter().sum();
				let weight = if sum > 0.0 {
					weight.map(|value| value / sum)
				} else {
					[1.0, 0.0, 0.0, 0.0]
				};
				asset.skin_vertices.push(SkinVertex {
					joints: [0, 1, 2, 3]
						.map(|slot| joints[index * 4 + slot].min(u32::from(u16::MAX)) as u16),
					weights: weight.map(|value| (value * 65535.0).round() as u16),
				});
			}
			Some(base)
		}
		_ => None,
	};
	let morph = if primitive.targets.is_empty() {
		None
	} else {
		let targets = primitive
			.targets
			.len()
			.min(target_count.max(primitive.targets.len()));
		if targets > MAX_MORPH_TARGETS {
			return Err(LoadError::TooLarge("a mesh has too many morph targets"));
		}
		let base = asset.morph_deltas.len() as u32;
		let size = targets * count * 2;
		if asset.morph_deltas.len() + size > MAX_ASSET_VERTICES * 4 {
			return Err(LoadError::TooLarge("the morph targets are too large"));
		}
		asset
			.morph_deltas
			.resize(asset.morph_deltas.len() + size, [0.0; 4]);
		for (target, attributes) in primitive.targets.iter().enumerate() {
			for (slot, name) in [(0, "POSITION"), (1, "NORMAL")] {
				let Some(&index) = attributes.get(name) else {
					continue;
				};
				let deltas = read(index, 3)?;
				for vertex in 0..count {
					let at = base as usize + (target * count + vertex) * 2 + slot;
					let delta = &deltas[vertex * 3..vertex * 3 + 3];
					asset.morph_deltas[at] = [delta[0], delta[1], delta[2], 0.0];
					if slot == 0 {
						bounds.grow(positions[vertex] + Vec3::from_slice(delta));
					}
				}
			}
		}
		Some(MorphRange {
			base,
			targets: targets as u32,
		})
	};
	let first_index = asset.indices.len() as u32;
	asset.indices.extend_from_slice(&indices);
	let default_material = asset.materials.len() - 1;
	Ok(Some(Primitive {
		base_vertex,
		vertex_count: count as u32,
		first_index,
		index_count: indices.len() as u32,
		material: primitive
			.material
			.filter(|material| *material < default_material)
			.unwrap_or(default_material),
		bounds,
		skin_base,
		morph,
	}))
}

fn load_meshes(document: &Document<'_>, asset: &mut Asset) -> Result<(), LoadError> {
	for mesh in &document.root.meshes {
		let target_count = mesh
			.primitives
			.iter()
			.map(|primitive| primitive.targets.len())
			.max()
			.unwrap_or(0);
		let mut primitives = Vec::with_capacity(mesh.primitives.len());
		for primitive in &mesh.primitives {
			if let Some(primitive) = load_primitive(document, asset, primitive, target_count)? {
				primitives.push(primitive);
			}
		}
		let mut target_names = mesh
			.extras
			.as_ref()
			.map(|extras| extras.target_names.clone())
			.unwrap_or_default();
		target_names.resize_with(target_count, String::new);
		let mut weights = mesh.weights.clone().unwrap_or_default();
		weights.resize(target_count, 0.0);
		asset.meshes.push(Mesh {
			primitives,
			target_names,
			weights,
		});
	}
	Ok(())
}

fn load_skins(document: &Document<'_>, asset: &mut Asset) -> Result<(), LoadError> {
	for skin in &document.root.skins {
		if skin.joints.len() > MAX_JOINTS {
			return Err(LoadError::TooLarge("a skin has too many joints"));
		}
		if skin.joints.iter().any(|joint| *joint >= asset.nodes.len()) {
			return Err(malformed("a skin names a missing joint"));
		}
		let inverse_bind = match skin.inverse_bind_matrices {
			Some(index) => {
				let (values, width) = document.floats(index)?;
				if width != 16 || values.len() != skin.joints.len() * 16 {
					return Err(malformed(
						"a skin's inverse bind matrices do not match its joints",
					));
				}
				values
					.as_chunks::<16>()
					.0
					.iter()
					.map(|chunk| Mat4::from_cols_slice(chunk))
					.collect()
			}
			None => vec![Mat4::IDENTITY; skin.joints.len()],
		};
		asset.skins.push(Skin {
			joint_radius: vec![0.0; skin.joints.len()],
			joints: skin.joints.clone(),
			inverse_bind,
		});
	}
	// Each joint's reach: how far the vertices it moves lie from it.
	for node in &asset.nodes {
		let (Some(mesh), Some(skin)) = (node.mesh, node.skin) else {
			continue;
		};
		let skin = &mut asset.skins[skin];
		for primitive in &asset.meshes[mesh].primitives {
			let Some(skin_base) = primitive.skin_base else {
				continue;
			};
			for vertex in 0..primitive.vertex_count as usize {
				let position =
					Vec3::from(asset.vertices[primitive.base_vertex as usize + vertex].position);
				let influence = asset.skin_vertices[skin_base as usize + vertex];
				for slot in 0..4 {
					let joint = usize::from(influence.joints[slot]);
					if influence.weights[slot] == 0 || joint >= skin.joints.len() {
						continue;
					}
					let local = skin.inverse_bind[joint].transform_point3(position);
					skin.joint_radius[joint] = skin.joint_radius[joint].max(local.length());
				}
			}
		}
	}
	Ok(())
}

fn load_clips(document: &Document<'_>, asset: &mut Asset) -> Result<(), LoadError> {
	let mut keyframes = 0_usize;
	let mut cache: HashMap<usize, Vec<f32>> = HashMap::new();
	for (index, animation) in document.root.animations.iter().enumerate() {
		let mut channels = Vec::with_capacity(animation.channels.len());
		let mut duration = 0.0_f32;
		for channel in &animation.channels {
			let Some(node) = channel.target.node else {
				continue;
			};
			if node >= asset.nodes.len() {
				return Err(malformed("an animation targets a missing node"));
			}
			let path = match channel.target.path.as_str() {
				"translation" => ChannelPath::Translation,
				"rotation" => ChannelPath::Rotation,
				"scale" => ChannelPath::Scale,
				"weights" => ChannelPath::Weights,
				_ => continue,
			};
			let sampler = animation
				.samplers
				.get(channel.sampler)
				.ok_or_else(|| malformed("an animation channel names a missing sampler"))?;
			let interpolation = match sampler.interpolation.as_deref() {
				Some("STEP") => Interpolation::Step,
				Some("CUBICSPLINE") => Interpolation::CubicSpline,
				_ => Interpolation::Linear,
			};
			let times = if let Some(times) = cache.get(&sampler.input) {
				times.clone()
			} else {
				let (times, _) = document.floats(sampler.input)?;
				cache.insert(sampler.input, times.clone());
				times
			};
			if times.iter().any(|time| !time.is_finite())
				|| times.windows(2).any(|pair| pair[1] < pair[0])
			{
				return Err(malformed(
					"an animation's times are not finite and increasing",
				));
			}
			let (values, _) = document.floats(sampler.output)?;
			let width = match path {
				ChannelPath::Rotation => 4,
				ChannelPath::Weights => {
					let mesh = asset.nodes[node].mesh.ok_or_else(|| {
						malformed("a weights animation targets a node with no mesh")
					})?;
					asset.meshes[mesh].weights.len().max(1)
				}
				ChannelPath::Translation | ChannelPath::Scale => 3,
			};
			let per_key = if interpolation == Interpolation::CubicSpline {
				width * 3
			} else {
				width
			};
			if values.len() != times.len() * per_key {
				return Err(malformed("an animation's outputs do not match its inputs"));
			}
			keyframes += times.len();
			if keyframes > MAX_KEYFRAMES {
				return Err(LoadError::TooLarge(
					"the animations have too many keyframes",
				));
			}
			duration = duration.max(times.last().copied().unwrap_or(0.0));
			channels.push(Channel {
				node,
				path,
				interpolation,
				times,
				values,
				width,
			});
		}
		asset.clips.push(Clip {
			name: animation
				.name
				.clone()
				.unwrap_or_else(|| format!("animation{index}")),
			duration,
			channels,
		});
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Checks every primitive's decoded positions against the bounds its
	/// file declares, returning how many it checked.
	fn check_positions(name: &str, document: &Document<'_>) -> usize {
		let mut checked = 0;
		for primitive in document
			.root
			.meshes
			.iter()
			.flat_map(|mesh| &mesh.primitives)
		{
			let position = primitive.attributes["POSITION"];
			let accessor = document.accessor(position).unwrap();
			let (Some(min), Some(max)) = (&accessor.min, &accessor.max) else {
				continue;
			};
			let (values, width) = document.floats(position).unwrap();
			for (axis, value) in values
				.chunks_exact(width)
				.flat_map(|point| point[..3].iter().enumerate())
			{
				// The exponential filter keeps a shared exponent and a
				// mantissa, so it rounds relative to the largest value.
				let slack = min[axis].abs().max(max[axis].abs()).max(1.0) / 4096.0;
				assert!(
					*value >= min[axis] - slack && *value <= max[axis] + slack,
					"{name}: {value} is outside {min:?}..{max:?} on axis {axis}"
				);
			}
			let spread = (0..3).map(|axis| max[axis] - min[axis]).sum::<f32>();
			assert!(spread > 0.0, "{name}: a mesh is flat in every axis");
			checked += 1;
		}
		checked
	}

	/// Checks that every rotation key decoded to a unit quaternion.
	fn check_rotations(name: &str, asset: &Asset) {
		let rotations = asset
			.clips
			.iter()
			.flat_map(|clip| &clip.channels)
			.filter(|channel| channel.path == ChannelPath::Rotation);
		for quaternion in rotations.flat_map(|channel| channel.values.as_chunks::<4>().0) {
			let length = quaternion
				.iter()
				.map(|value| value * value)
				.sum::<f32>()
				.sqrt();
			assert!(
				(length - 1.0).abs() < 2.0e-3,
				"{name}: a rotation has length {length}"
			);
		}
	}

	/// Loads every `.glb` in `LEPTOSKIT_3D_MODELS` and checks the decoded
	/// positions against the bounds each file declares, and that rotations
	/// decode to unit quaternions. Without the variable it does nothing: the
	/// models it was written against are not part of this repository.
	#[test]
	fn models_in_a_folder_decode_within_their_declared_bounds() {
		let Ok(folder) = std::env::var("LEPTOSKIT_3D_MODELS") else {
			eprintln!("skipping: LEPTOSKIT_3D_MODELS is not set");
			return;
		};
		let mut paths: Vec<_> = std::fs::read_dir(&folder)
			.unwrap()
			.map(|entry| entry.unwrap().path())
			.filter(|path| path.extension().is_some_and(|extension| extension == "glb"))
			.collect();
		paths.sort();
		assert!(!paths.is_empty(), "no .glb files in {folder}");
		for path in paths {
			let name = path.file_name().unwrap().to_string_lossy().into_owned();
			let bytes = std::fs::read(&path).unwrap();
			let start = std::time::Instant::now();
			let asset = load(&bytes, &mut |_| Err("none".into()))
				.unwrap_or_else(|error| panic!("{name}: {error}"));
			let elapsed = start.elapsed();
			let document = Document::parse(&bytes, &mut |_| Err("none".into())).unwrap();
			let checked = check_positions(&name, &document);
			check_rotations(&name, &asset);
			let (meshes, primitives) = asset.mesh_count();
			eprintln!(
				"{name}: {} vertices, {} triangles, {meshes} meshes, {primitives} primitives, {} \
				 clips, {} morph targets, {checked} bounds checked, {:.1} ms",
				asset.vertex_count(),
				asset.triangle_count(),
				asset.clips.len(),
				asset.morph_target_names().len(),
				elapsed.as_secs_f64() * 1000.0,
			);
		}
	}

	#[test]
	fn base64_decodes_with_and_without_padding() {
		assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
		assert_eq!(decode_base64("aGVsbG8").unwrap(), b"hello");
		assert!(decode_base64("a*").is_err());
	}

	#[test]
	fn percent_escapes_decode() {
		assert_eq!(percent_decode("a%20b.bin"), "a b.bin");
		assert_eq!(percent_decode("bad%zz"), "bad%zz");
	}

	#[test]
	fn strips_and_fans_become_lists() {
		assert_eq!(triangle_list(5, vec![0, 1, 2, 3]), [0, 1, 2, 2, 1, 3]);
		assert_eq!(triangle_list(6, vec![0, 1, 2, 3]), [0, 1, 2, 0, 2, 3]);
		assert_eq!(triangle_list(4, vec![0, 1, 2, 3]), [0, 1, 2]);
	}

	#[test]
	fn normalized_components_follow_the_specification() {
		assert_eq!(component(&[0x81], 5120, true), -1.0);
		assert_eq!(component(&[0x80], 5120, true), -1.0);
		assert_eq!(component(&[255], 5121, true), 1.0);
		assert_eq!(component(&[0xff, 0x7f], 5122, true), 1.0);
		assert_eq!(component(&[0xff, 0xff], 5123, true), 1.0);
		assert_eq!(component(&[7], 5121, false), 7.0);
	}

	#[test]
	fn unsupported_required_extensions_are_refused() {
		let json =
			br#"{"asset":{"version":"2.0"},"extensionsRequired":["KHR_draco_mesh_compression"]}"#;
		let error = load(json, &mut |_| Err("none".into())).unwrap_err();
		assert!(
			matches!(error, LoadError::Extension(name) if name == "KHR_draco_mesh_compression")
		);
	}

	#[test]
	fn paths_outside_the_folder_are_never_resolved() {
		let json =
			br#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":"../secret.bin"}]}"#;
		let mut asked = false;
		let error = load(json, &mut |_| {
			asked = true;
			Ok(vec![0; 4])
		})
		.unwrap_err();
		assert!(!asked);
		assert!(matches!(error, LoadError::Resource(_)));
	}
}
