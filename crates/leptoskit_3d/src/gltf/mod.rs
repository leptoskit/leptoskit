//! The glTF 2.0 loader: binary `.glb` and JSON `.gltf` files, with
//! `EXT_meshopt_compression`, `KHR_mesh_quantization`,
//! `KHR_materials_emissive_strength`, `KHR_materials_unlit`, and
//! `KHR_texture_transform`.

mod json;
pub mod meshopt;

use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use glam::Mat4;
use glam::Quat;
use glam::Vec3;

use crate::asset::*;
use crate::sanitize::*;

/// Maximum number of vertex-adjacency smoothing rings applied to welded
/// outline normals via [`LoadOptions::outline_smooth_rings`] or glTF
/// `extras.outlineSmoothRings`.
pub const MAX_OUTLINE_SMOOTH_RINGS: u8 = 8;

/// Options controlling how a glTF asset is prepared at load time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadOptions {
	/// How many rings of vertex-adjacency Laplacian smoothing to apply to
	/// welded outline normals (`0` leaves the default welded hull unchanged;
	/// clamped to [`MAX_OUTLINE_SMOOTH_RINGS`]).
	///
	/// When `None`, the loader reads `extras.outlineSmoothRings` from each
	/// mesh or from the document root, defaulting to `0` when absent.
	pub outline_smooth_rings: Option<u8>,
}

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
	"KHR_texture_transform",
];

/// glTF's accessor component types, which are OpenGL's type enums.
const BYTE: u32 = 5120;
const UNSIGNED_BYTE: u32 = 5121;
const SHORT: u32 = 5122;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;
const FLOAT: u32 = 5126;

/// glTF's sampler enums, also OpenGL's.
const NEAREST: u32 = 9728;
const CLAMP_TO_EDGE: u32 = 33071;
const MIRRORED_REPEAT: u32 = 33648;

/// glTF's primitive modes that draw triangles.
const TRIANGLES: u32 = 4;
const TRIANGLE_STRIP: u32 = 5;
const TRIANGLE_FAN: u32 = 6;

/// A `.glb` starts with a 12-byte header (magic, version, length), and each
/// chunk with an 8-byte one (length, type).
const GLB_HEADER: usize = 12;
const GLB_CHUNK_HEADER: usize = 8;
const GLB_VERSION: usize = 2;

/// Floats in a 4×4 matrix.
const MAT4_FLOATS: usize = 16;

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

	if word(4)? != GLB_VERSION {
		return Err(LoadError::Format(
			"only glTF 2 binaries are supported".into(),
		));
	}

	let length = word(GLB_HEADER - 4)?.min(bytes.len());
	let mut offset = GLB_HEADER;
	let mut json = None;

	let mut bin = None;

	while offset + GLB_CHUNK_HEADER <= length {
		let size = word(offset)?;
		let kind = word(offset + 4)?;
		let start = offset + GLB_CHUNK_HEADER;
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

/// Decodes standard or URL-safe base64, ignoring whitespace and padding.
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
		"MAT4" => MAT4_FLOATS,
		_ => return Err(malformed(format!("unknown accessor type {kind}"))),
	})
}

fn component_size(component_type: u32) -> Result<usize, LoadError> {
	Ok(match component_type {
		BYTE | UNSIGNED_BYTE => 1,
		SHORT | UNSIGNED_SHORT => 2,
		UNSIGNED_INT | FLOAT => 4,
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
		(BYTE, true) => (f32::from(bytes[0] as i8) / f32::from(i8::MAX)).max(-1.0),
		(BYTE, false) => f32::from(bytes[0] as i8),
		(UNSIGNED_BYTE, true) => f32::from(bytes[0]) / f32::from(u8::MAX),
		(UNSIGNED_BYTE, false) => f32::from(bytes[0]),
		(SHORT, true) => {
			(f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / f32::from(i16::MAX)).max(-1.0)
		}
		(SHORT, false) => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])),
		(UNSIGNED_SHORT, true) => {
			f32::from(u16::from_le_bytes([bytes[0], bytes[1]])) / f32::from(u16::MAX)
		}
		(UNSIGNED_SHORT, false) => f32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
		(UNSIGNED_INT, _) => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32,
		_ => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
	}
}

fn integer(bytes: &[u8], component_type: u32) -> u32 {
	match component_type {
		BYTE | UNSIGNED_BYTE => u32::from(bytes[0]),
		SHORT | UNSIGNED_SHORT => u32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
		_ => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
	}
}

impl<'a> Document<'a> {
	/// Parses the JSON, checks it against the limits and the extensions
	/// this loader implements, then reads every buffer and decompresses
	/// every meshopt view once, so accessors read plain bytes.
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

			// The last element ends `element` bytes past its stride, not a
			// whole stride.
			let end = match accessor.count {
				0 => 0,
				count => {
					accessor
						.byte_offset
						.checked_add(stride * (count - 1) + element)
						.ok_or_else(|| malformed("an accessor overflows"))?
				}
			};

			if end > bytes.len() {
				return Err(malformed("an accessor runs past its buffer view"));
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
				let index_offset = sparse.indices.byte_offset + item * index_size;
				let target = indices
					.get(index_offset..index_offset + index_size)
					.map(|bytes| integer(bytes, sparse.indices.component_type) as usize)
					.filter(|target| *target < accessor.count)
					.ok_or_else(|| malformed("a sparse index is out of range"))?;
				let value_offset = sparse.values.byte_offset + item * element;
				let value = values
					.get(value_offset..value_offset + element)
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

		if component_type == FLOAT {
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

/// Loads a model from the bytes of a `.glb` or `.gltf` file with default
/// [`LoadOptions`]. `resolve` reads a file the model refers to by a relative
/// path, such as a `.bin` buffer or a texture beside a `.gltf`; it is never
/// asked for a path outside the model's folder.
pub(crate) fn load(
	bytes: &[u8],
	resolve: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Asset, LoadError> {
	load_with_options(bytes, LoadOptions::default(), resolve)
}

/// Loads a model from `.glb` or `.gltf` bytes with explicit [`LoadOptions`].
pub(crate) fn load_with_options(
	bytes: &[u8],
	options: LoadOptions,
	resolve: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Asset, LoadError> {
	let document = Document::parse(bytes, resolve)?;
	let mut asset = Asset::default();
	load_nodes(&document, &mut asset)?;
	load_materials(&document, &mut asset, resolve)?;
	load_meshes(&document, &mut asset)?;
	load_skins(&document, &mut asset)?;
	load_clips(&document, &mut asset)?;
	asset.units = rest_units(&asset);
	smooth_outline_normals(&document, &mut asset, options);
	build_knockout_vertices(&mut asset);
	Ok(asset)
}

/// How long one unit of each node's mesh space is, in the model's metres,
/// in its rest pose: the scale a quantized mesh's node or skin undoes. Ink
/// drawn on a surface measures its noise and hatching in these units, so it
/// keeps one size whatever the file's units, and stays fixed to the surface
/// however the model moves.
fn rest_units(asset: &Asset) -> Vec<f32> {
	let mut world = vec![Mat4::IDENTITY; asset.nodes.len()];
	let mut stack: Vec<(usize, Mat4)> = asset
		.roots
		.iter()
		.map(|root| (*root, Mat4::IDENTITY))
		.collect();

	while let Some((index, parent)) = stack.pop() {
		let node = &asset.nodes[index];
		world[index] = parent
			* Mat4::from_scale_rotation_translation(node.scale, node.rotation, node.translation);
		stack.extend(node.children.iter().map(|child| (*child, world[index])));
	}

	let scale = |matrix: Mat4| {
		let scale = matrix.determinant().abs().cbrt();
		if scale.is_finite() && scale > 0.0 {
			scale
		} else {
			1.0
		}
	};

	asset
		.nodes
		.iter()
		.enumerate()
		.map(|(index, node)| {
			let bind = node.skin.and_then(|skin| {
				let skin = &asset.skins[skin];
				Some(world[*skin.joints.first()?] * *skin.inverse_bind.first()?)
			});
			scale(bind.unwrap_or(world[index]))
		})
		.collect()
}

/// Standard IEEE CRC-32, used to derive deterministic per-part seeds from
/// node names without adding a crate dependency.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
	let mut crc = 0xFFFF_FFFF_u32;

	for &byte in bytes {
		crc ^= u32::from(byte);

		for _ in 0..8 {
			let mask = (crc & 1).wrapping_neg();
			crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
		}
	}

	!crc
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

		let rotation = if rotation.is_finite() {
			rotation
		} else {
			Quat::IDENTITY
		};

		let name = node.name.clone().unwrap_or_else(|| format!("node{index}"));
		asset.seeds.push(crc32(name.as_bytes()));
		asset.names.entry(name.clone()).or_insert(index);
		asset.nodes.push(AssetNode {
			name,
			parent: parents[index],
			children: node.children.clone(),
			translation,
			rotation,

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
			Some(CLAMP_TO_EDGE) => 0,
			Some(MIRRORED_REPEAT) => 2,
			_ => 1,
		}
	};
	Ok(Some(TextureRef {
		image,
		sampler: SamplerDesc {
			nearest: sampler.and_then(|sampler| sampler.mag_filter) == Some(NEAREST),
			repeat_u: wrap(sampler.and_then(|sampler| sampler.wrap_s)),
			repeat_v: wrap(sampler.and_then(|sampler| sampler.wrap_t)),
		},
	}))
}

/// The texture coordinate set a texture reads: its own, unless its
/// `KHR_texture_transform` names another.
fn coordinate_set(tex_coord: u32, extensions: &json::TextureInfoExtensions) -> u32 {
	extensions
		.texture_transform
		.as_ref()
		.and_then(|transform| transform.tex_coord)
		.unwrap_or(tex_coord)
}

/// A `KHR_texture_transform`, baked into a primitive's texture coordinates
/// as they load. Every texture of a material samples the one set the
/// loader reads, so one transform serves them all: that of the first
/// texture reading the set, looking at the base color texture first, then
/// the emissive, metallic-roughness, normal and occlusion ones. gltfpack
/// gives every texture of a material the same one, the scale and offset
/// that undo its quantization.
#[derive(Clone, Copy, Debug, PartialEq)]
struct UvTransform {
	offset: [f32; 2],
	rotation: f32,
	scale: [f32; 2],
}

impl UvTransform {
	fn of_primitive(document: &Document<'_>, primitive: &json::Primitive) -> Option<Self> {
		let material = document.root.materials.get(primitive.material?)?;
		let pbr = &material.pbr_metallic_roughness;
		let infos = [
			pbr.base_color_texture
				.as_ref()
				.map(|t| (t.tex_coord, &t.extensions)),
			material
				.emissive_texture
				.as_ref()
				.map(|t| (t.tex_coord, &t.extensions)),
			pbr.metallic_roughness_texture
				.as_ref()
				.map(|t| (t.tex_coord, &t.extensions)),
			material
				.normal_texture
				.as_ref()
				.map(|t| (t.tex_coord, &t.extensions)),
			material
				.occlusion_texture
				.as_ref()
				.map(|t| (t.tex_coord, &t.extensions)),
		];

		infos
			.into_iter()
			.flatten()
			.find(|(tex_coord, extensions)| coordinate_set(*tex_coord, extensions) == 0)
			.and_then(|(_, extensions)| extensions.texture_transform.as_ref())
			.map(Self::new)
	}

	fn new(transform: &json::TextureTransform) -> Self {
		let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };

		Self {
			offset: transform.offset.map(|value| finite(value, 0.0)),
			rotation: finite(transform.rotation, 0.0),
			scale: transform.scale.map(|value| finite(value, 1.0)),
		}
	}

	/// The specification's matrix: translation, then rotation, then scale.
	fn apply(self, [u, v]: [f32; 2]) -> [f32; 2] {
		let (sin, cos) = self.rotation.sin_cos();
		let (u, v) = (u * self.scale[0], v * self.scale[1]);

		[
			self.offset[0] + cos * u + sin * v,
			self.offset[1] - sin * u + cos * v,
		]
	}
}

/// Reads the materials, then appends a default one for primitives that
/// name none.
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
					.map(|t| (t.index, coordinate_set(t.tex_coord, &t.extensions))),
			)?,
			metallic: finite(pbr.metallic_factor, 1.0).clamp(0.0, 1.0),
			roughness: finite(pbr.roughness_factor, 1.0).clamp(0.0, 1.0),
			metallic_roughness_texture: slot(
				pbr.metallic_roughness_texture
					.as_ref()
					.map(|t| (t.index, coordinate_set(t.tex_coord, &t.extensions))),
			)?,
			normal_texture: slot(
				material
					.normal_texture
					.as_ref()
					.map(|t| (t.index, coordinate_set(t.tex_coord, &t.extensions))),
			)?,
			normal_scale: material
				.normal_texture
				.as_ref()
				.map_or(1.0, |t| finite(t.scale, 1.0)),
			occlusion_texture: slot(
				material
					.occlusion_texture
					.as_ref()
					.map(|t| (t.index, coordinate_set(t.tex_coord, &t.extensions))),
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
					.map(|t| (t.index, coordinate_set(t.tex_coord, &t.extensions))),
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
	// A strip flips every other triangle's winding, which the list keeps by
	// swapping that triangle's first two corners.
	let strip = |index: usize| {
		let [first, second] = if index % 2 == 0 {
			[index, index + 1]
		} else {
			[index + 1, index]
		};

		[indices[first], indices[second], indices[index + 2]]
	};

	match mode {
		TRIANGLE_STRIP => {
			(0..indices.len().saturating_sub(2))
				.flat_map(strip)
				.collect()
		}
		TRIANGLE_FAN => {
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

/// Reads a primitive's vertex attributes, each checked to hold one element
/// per vertex.
struct Attributes<'a, 'd> {
	document: &'a Document<'d>,
	primitive: &'a json::Primitive,
	count: usize,
}

impl Attributes<'_, '_> {
	fn attribute(&self, name: &str) -> Option<usize> {
		self.primitive.attributes.get(name).copied()
	}

	/// Reads an attribute as `width` floats per vertex, dropping any
	/// components past `width`.
	fn read(&self, index: usize, width: usize) -> Result<Vec<f32>, LoadError> {
		let (values, actual) = self.document.floats(index)?;

		if values.len() / actual.max(1) != self.count || actual < width {
			return Err(malformed("an attribute's count or type does not match"));
		}

		Ok(values
			.chunks_exact(actual)
			.flat_map(|chunk| chunk[..width].to_vec())
			.collect())
	}

	/// Vertex colours as RGBA bytes; white where the primitive has none.
	fn colors(&self) -> Result<Vec<[u8; 4]>, LoadError> {
		let Some(index) = self.attribute("COLOR_0") else {
			return Ok(vec![[u8::MAX; 4]; self.count]);
		};
		let (values, width) = self.document.floats(index)?;

		if values.len() / width.max(1) != self.count || !(3..=4).contains(&width) {
			return Err(malformed("COLOR_0 must be VEC3 or VEC4 per vertex"));
		}

		let byte = |value: f32| (value.clamp(0.0, 1.0) * f32::from(u8::MAX)).round() as u8;

		// A missing alpha is opaque.
		Ok(values
			.chunks_exact(width)
			.map(|chunk| [0, 1, 2, 3].map(|index| byte(chunk.get(index).copied().unwrap_or(1.0))))
			.collect())
	}

	/// Each vertex's ink weight, as [`InkVertex::weight`] stores it: twice
	/// `COLOR_1`'s red channel, so 0.5 is the part's line, 0 none, and 1 a
	/// line twice as heavy. `COLOR_1`'s green, blue, and alpha channels are
	/// reserved for app-specific masks and ignored by the renderer. A
	/// primitive without `COLOR_1` has the part's line throughout.
	fn ink_weights(&self) -> Result<Vec<i8>, LoadError> {
		let Some(index) = self.attribute("COLOR_1") else {
			return Ok(vec![0; self.count]);
		};
		let (values, width) = self.document.floats(index)?;

		if values.len() / width.max(1) != self.count || !(3..=4).contains(&width) {
			return Err(malformed("COLOR_1 must be VEC3 or VEC4 per vertex"));
		}

		Ok(values
			.chunks_exact(width)
			.map(|chunk| ink_weight_byte(chunk[0]))
			.collect())
	}
}

/// `COLOR_1`'s red channel as an ink weight minus one, in signed normalized
/// bytes: 0 is -127 (no line), 0.5 is 0 (the part's line), 1 is 127.
fn ink_weight_byte(red: f32) -> i8 {
	let red = if red.is_finite() {
		red.clamp(0.0, 1.0)
	} else {
		0.5
	};

	((red * 2.0 - 1.0) * 127.0).round() as i8
}

/// A unit vector as signed normalized bytes.
fn snorm(vector: Vec3) -> [i8; 3] {
	vector
		.to_array()
		.map(|value| (value.clamp(-1.0, 1.0) * 127.0).round() as i8)
}

fn find_root(parent: &mut [usize], mut index: usize) -> usize {
	while parent[index] != index {
		parent[index] = parent[parent[index]];
		index = parent[index];
	}
	index
}

fn union_roots(parent: &mut [usize], a: usize, b: usize) {
	let root_a = find_root(parent, a);
	let root_b = find_root(parent, b);
	if root_a != root_b {
		parent[root_b] = root_a;
	}
}

fn f64_point(point: Vec3) -> [f64; 3] {
	[f64::from(point.x), f64::from(point.y), f64::from(point.z)]
}

fn dist2_f64(a: [f64; 3], b: [f64; 3]) -> f64 {
	let dx = a[0] - b[0];
	let dy = a[1] - b[1];
	let dz = a[2] - b[2];
	dx * dx + dy * dy + dz * dz
}

fn boundary_offsets(frac: f64) -> &'static [i64] {
	if frac < 0.26 {
		&[0, -1]
	} else if frac > 0.74 {
		&[0, 1]
	} else {
		&[0]
	}
}

fn weld_boundary_neighbors(
	index: usize,
	key: [i64; 3],
	frac: [f64; 3],
	positions: &[Vec3],
	cells: &HashMap<[i64; 3], Vec<usize>>,
	tol2: f64,
	parent: &mut [usize],
) {
	let dx = boundary_offsets(frac[0]);
	let dy = boundary_offsets(frac[1]);
	let dz = boundary_offsets(frac[2]);

	if dx.len() == 1 && dy.len() == 1 && dz.len() == 1 {
		return;
	}

	let point = f64_point(positions[index]);

	for &ox in dx {
		for &oy in dy {
			for &oz in dz {
				if ox == 0 && oy == 0 && oz == 0 {
					continue;
				}

				let neighbor = [key[0] + ox, key[1] + oy, key[2] + oz];
				let Some(other) = cells.get(&neighbor) else {
					continue;
				};

				if other.len() > MAX_WELD_CELL {
					continue;
				}

				for &other_index in other {
					if dist2_f64(point, f64_point(positions[other_index])) <= tol2 {
						union_roots(parent, index, other_index);
					}
				}
			}
		}
	}
}

/// The most distinct points one weld cell compares pairwise. A cell is four
/// weld tolerances across, so a real mesh holds a handful of points in one;
/// a denser cell, which only a broken or hostile file makes, welds just its
/// exact copies, keeping a load linear in its vertices rather than quadratic.
const MAX_WELD_CELL: usize = 128;

/// The diagonal of the finite points' bounds, measured in `f64` so extreme
/// finite `f32` coordinates cannot overflow, or `None` when no point is
/// finite or the diagonal is not.
fn finite_span(positions: &[Vec3]) -> Option<f64> {
	let mut min = [f64::INFINITY; 3];
	let mut max = [f64::NEG_INFINITY; 3];

	for point in positions.iter().filter(|point| point.is_finite()) {
		let point = f64_point(*point);

		for axis in 0..3 {
			min[axis] = min[axis].min(point[axis]);
			max[axis] = max[axis].max(point[axis]);
		}
	}

	let span = dist2_f64(max, min).sqrt();
	span.is_finite().then_some(span)
}

/// Groups the positions closer than `tol` to one another, transitively:
/// each position's group, as the index of one position in it. Non-finite
/// positions stay alone.
///
/// Exact copies are found by their bits, then the distinct points are hashed
/// into cells four tolerances wide and compared within a cell, and with the
/// neighbouring cell wherever a point lies within a tolerance of a cell's
/// face, so copies straddling a cell boundary still weld.
fn weld_groups(positions: &[Vec3], tol: f64) -> Vec<usize> {
	let count = positions.len();
	let mut parent: Vec<usize> = (0..count).collect();
	let mut first_at: HashMap<[u32; 3], usize> = HashMap::with_capacity(count);
	let mut distinct = Vec::with_capacity(count);
	let mut min = [f64::INFINITY; 3];

	for (index, point) in positions.iter().enumerate() {
		if !point.is_finite() {
			continue;
		}

		// Adding zero turns -0 into 0, which compares equal to it.
		match first_at.entry((*point + Vec3::ZERO).to_array().map(f32::to_bits)) {
			Entry::Occupied(first) => union_roots(&mut parent, *first.get(), index),
			Entry::Vacant(slot) => {
				slot.insert(index);
				distinct.push(index);
				let point = f64_point(*point);

				for axis in 0..3 {
					min[axis] = min[axis].min(point[axis]);
				}
			}
		}
	}

	if distinct.len() > 1 && tol.is_finite() && tol > 0.0 {
		let tol2 = tol * tol;
		let cell = 4.0 * tol;
		let mut cells: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
		let mut coords = vec![([0_i64; 3], [0.0_f64; 3]); count];

		for &index in &distinct {
			let point = f64_point(positions[index]);
			let u = [0, 1, 2].map(|axis| (point[axis] - min[axis]) / cell);
			let floor = u.map(f64::floor);
			let key = floor.map(|value| value as i64);
			coords[index] = (key, [0, 1, 2].map(|axis| u[axis] - floor[axis]));
			cells.entry(key).or_default().push(index);
		}

		for reps in cells.values().filter(|reps| reps.len() <= MAX_WELD_CELL) {
			for a in 0..reps.len() {
				let pa = f64_point(positions[reps[a]]);

				for b in (a + 1)..reps.len() {
					if dist2_f64(pa, f64_point(positions[reps[b]])) <= tol2 {
						union_roots(&mut parent, reps[a], reps[b]);
					}
				}
			}

			for &index in reps {
				let (key, frac) = coords[index];
				weld_boundary_neighbors(index, key, frac, positions, &cells, tol2, &mut parent);
			}
		}
	}

	(0..count)
		.map(|index| find_root(&mut parent, index))
		.collect()
}

/// Outline normals for a mesh: at each position, the mean of the distinct
/// normals its vertices carry there. Vertices split at a hard edge, at a
/// seam, or between primitives share a position, so they get one normal and
/// a line pushed out along it stays closed; a smooth vertex keeps its own.
///
/// The normals are the file's, made before the mesh was quantized. The
/// faces' own are not to be trusted: quantizing tilts the smallest
/// triangles every which way, and a line pushed along them splinters.
/// Copies whose normals cancel, as the two sides of a thin sheet do, keep
/// their own.
pub(crate) fn welded_normals(positions: &[Vec3], normals: &[Vec3]) -> Vec<Vec3> {
	let count = positions.len().min(normals.len());
	let mut welded = normals.to_vec();

	if count < 2 {
		return welded;
	}

	let Some(span) = finite_span(&positions[..count]) else {
		return welded;
	};

	// Positions closer than a hundred-thousandth of the mesh's size weld.
	let groups = weld_groups(&positions[..count], (span * 1.0e-5).max(1.0e-12));

	// Each vertex by its welded group, then by its normal in bytes, so the
	// copies at a position sit together and each distinct normal counts once.
	let mut order: Vec<(usize, [i8; 3], usize)> = (0..count)
		.map(|index| (groups[index], snorm(normals[index]), index))
		.collect();
	order.sort_unstable();

	for copies in order.chunk_by(|a, b| a.0 == b.0) {
		let sum: Vec3 = copies
			.chunk_by(|a, b| a.1 == b.1)
			.map(|same| normals[same[0].2].normalize_or_zero())
			.sum();

		if copies.len() < 2 || sum.length_squared() < 0.01 {
			continue;
		}

		let normal = sum.normalize();

		for (.., index) in copies {
			welded[*index] = normal;
		}
	}

	welded
}

/// Writes the outline normals of the mesh whose vertices start at
/// `first_vertex` and run to the end of the asset's.
fn weld_mesh(asset: &mut Asset, first_vertex: usize) {
	let vertices = &asset.vertices[first_vertex..];
	let positions: Vec<Vec3> = vertices
		.iter()
		.map(|vertex| Vec3::from(vertex.position))
		.collect();
	let normals: Vec<Vec3> = vertices
		.iter()
		.map(|vertex| Vec3::from(vertex.normal))
		.collect();

	for (ink, normal) in asset.ink_vertices[first_vertex..]
		.iter_mut()
		.zip(welded_normals(&positions, &normals))
	{
		ink.normal = snorm(normal);
	}
}

/// Optional vertex-adjacency Laplacian smoothing of welded outline normals.
/// Runs only when `LoadOptions::outline_smooth_rings` or glTF
/// `extras.outlineSmoothRings` resolves to `rings > 0`; at `0` (the default),
/// `asset.ink_vertices` stays byte-for-byte identical to [`weld_mesh`].
fn smooth_outline_normals(document: &Document<'_>, asset: &mut Asset, options: LoadOptions) {
	let root_rings = document
		.root
		.extras
		.as_ref()
		.and_then(|extras| extras.outline_smooth_rings);

	for (mesh_index, mesh) in asset.meshes.iter().enumerate() {
		let mesh_rings = document
			.root
			.meshes
			.get(mesh_index)
			.and_then(|mesh| mesh.extras.as_ref())
			.and_then(|extras| extras.outline_smooth_rings);
		let rings = options
			.outline_smooth_rings
			.or(mesh_rings)
			.or(root_rings)
			.unwrap_or(0)
			.min(MAX_OUTLINE_SMOOTH_RINGS);

		if rings == 0 || mesh.primitives.is_empty() {
			continue;
		}

		let first_vertex = mesh
			.primitives
			.iter()
			.map(|primitive| primitive.base_vertex as usize)
			.min()
			.unwrap_or(0);
		let end_vertex = mesh
			.primitives
			.iter()
			.map(|primitive| primitive.base_vertex as usize + primitive.vertex_count as usize)
			.max()
			.unwrap_or(first_vertex);

		if end_vertex <= first_vertex || end_vertex > asset.vertices.len() {
			continue;
		}

		let unit_scale = asset
			.nodes
			.iter()
			.enumerate()
			.find_map(|(node_idx, node)| {
				(node.mesh == Some(mesh_index))
					.then(|| asset.units.get(node_idx).copied().unwrap_or(1.0))
			})
			.unwrap_or(1.0);

		let mut min = [f64::INFINITY; 3];
		let mut max = [f64::NEG_INFINITY; 3];
		for vertex in &asset.vertices[first_vertex..end_vertex] {
			for axis in 0..3 {
				let v = f64::from(vertex.position[axis]);
				if v.is_finite() {
					min[axis] = min[axis].min(v);
					max[axis] = max[axis].max(v);
				}
			}
		}
		let extent = [
			(max[0] - min[0]).max(0.0),
			(max[1] - min[1]).max(0.0),
			(max[2] - min[2]).max(0.0),
		];
		let diag = (extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2]).sqrt();
		let weld_m = 1.2e-3_f64 / f64::from(unit_scale).max(1.0e-4);
		let cell = if diag.is_finite() && diag > 0.0 {
			(diag * 1.0e-5).max(weld_m).min(diag * 0.01).max(1.0e-12)
		} else {
			1.0
		};

		let local_count = end_vertex - first_vertex;
		let mut order: Vec<([i64; 3], [i8; 3], usize)> = asset.vertices[first_vertex..end_vertex]
			.iter()
			.enumerate()
			.map(|(offset, vertex)| {
				let key = vertex.position.map(|value| {
					let scaled = f64::from(value) / cell;
					if scaled.is_finite() {
						scaled.round() as i64
					} else {
						0
					}
				});
				(key, snorm(Vec3::from(vertex.normal)), offset)
			})
			.collect();
		order.sort_unstable();

		let mut group_of = vec![0_usize; local_count];
		let mut group_normals: Vec<Vec3> = Vec::new();
		let mut group_valid: Vec<bool> = Vec::new();

		for copies in order.chunk_by(|a, b| a.0 == b.0) {
			let group = group_normals.len();
			let sum: Vec3 = copies
				.chunk_by(|a, b| a.1 == b.1)
				.map(|same| {
					Vec3::from(asset.vertices[first_vertex + same[0].2].normal).normalize_or_zero()
				})
				.sum();
			let valid = sum.length_squared() >= 0.01;
			group_normals.push(if valid { sum.normalize() } else { Vec3::ZERO });
			group_valid.push(valid);
			for (.., offset) in copies {
				group_of[*offset] = group;
			}
		}

		let group_count = group_normals.len();
		let mut neighbors: Vec<Vec<usize>> = vec![Vec::new(); group_count];
		for primitive in &mesh.primitives {
			let base = primitive.base_vertex as usize;
			let start = primitive.first_index as usize;
			let end = start + primitive.index_count as usize;
			for tri in asset.indices[start..end].chunks_exact(3) {
				let va = base + tri[0] as usize;
				let vb = base + tri[1] as usize;
				let vc = base + tri[2] as usize;
				if va < first_vertex
					|| vb < first_vertex
					|| vc < first_vertex
					|| va >= end_vertex
					|| vb >= end_vertex
					|| vc >= end_vertex
				{
					continue;
				}
				let ga = group_of[va - first_vertex];
				let gb = group_of[vb - first_vertex];
				let gc = group_of[vc - first_vertex];
				for (u, v) in [(ga, gb), (gb, gc), (gc, ga)] {
					if u != v {
						neighbors[u].push(v);
						neighbors[v].push(u);
					}
				}
			}
		}
		for list in &mut neighbors {
			list.sort_unstable();
			list.dedup();
		}

		let mut current = group_normals;
		let mut next = vec![Vec3::ZERO; group_count];
		for _ in 0..rings {
			for g in 0..group_count {
				if !group_valid[g] {
					next[g] = Vec3::ZERO;
					continue;
				}
				let mut sum = current[g];
				let mut count = 1_u32;
				for &nbr in &neighbors[g] {
					if group_valid[nbr] {
						sum += current[nbr];
						count += 1;
					}
				}
				next[g] = sum / count as f32;
			}
			std::mem::swap(&mut current, &mut next);
		}

		for offset in 0..local_count {
			let g = group_of[offset];
			if group_valid[g] && current[g].length_squared() > 1.0e-6 {
				asset.ink_vertices[first_vertex + offset].normal = snorm(current[g].normalize());
			}
		}
	}
}

/// Detects open boundary edges per mesh and builds `asset.knockout_vertices`
/// when any primitive has open edges (such as a single-sided robe hem or
/// sheet). Closed meshes leave `asset.knockout_vertices` empty and reuse
/// `ink_vertices`.
fn build_knockout_vertices(asset: &mut Asset) {
	for mesh in &mut asset.meshes {
		if mesh.primitives.is_empty() {
			continue;
		}
		let first_vertex = mesh
			.primitives
			.iter()
			.map(|primitive| primitive.base_vertex as usize)
			.min()
			.unwrap_or(0);
		let end_vertex = mesh
			.primitives
			.iter()
			.map(|primitive| primitive.base_vertex as usize + primitive.vertex_count as usize)
			.max()
			.unwrap_or(first_vertex);

		if end_vertex <= first_vertex || end_vertex > asset.vertices.len() {
			continue;
		}

		// Compute extent in f64 so extreme f32 coordinates never overflow to
		// Inf.
		let mut min = [f64::INFINITY; 3];
		let mut max = [f64::NEG_INFINITY; 3];
		for vertex in &asset.vertices[first_vertex..end_vertex] {
			for axis in 0..3 {
				let v = f64::from(vertex.position[axis]);
				if v.is_finite() {
					min[axis] = min[axis].min(v);
					max[axis] = max[axis].max(v);
				}
			}
		}
		let extent = [
			(max[0] - min[0]).max(0.0),
			(max[1] - min[1]).max(0.0),
			(max[2] - min[2]).max(0.0),
		];
		let diag = (extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2]).sqrt();
		let cell = if diag.is_finite() && diag > 0.0 {
			(diag * 1.0e-5).max(1.0e-12)
		} else {
			1.0
		};

		let local_count = end_vertex - first_vertex;
		let mut cells: HashMap<[i64; 3], usize> = HashMap::with_capacity(local_count);
		let mut group_of = vec![0_usize; local_count];
		let mut group_count = 0_usize;

		for (offset, vertex) in asset.vertices[first_vertex..end_vertex].iter().enumerate() {
			let key = vertex.position.map(|value| {
				let scaled = f64::from(value) / cell;
				if scaled.is_finite() {
					scaled.round() as i64
				} else {
					0
				}
			});
			let group = *cells.entry(key).or_insert_with(|| {
				let id = group_count;
				group_count += 1;
				id
			});
			group_of[offset] = group;
		}

		let mut edge_counts: HashMap<(usize, usize), u32> = HashMap::new();
		for primitive in &mesh.primitives {
			let base = primitive.base_vertex as usize;
			let start = primitive.first_index as usize;
			let end = start + primitive.index_count as usize;
			for tri in asset.indices[start..end].chunks_exact(3) {
				let va = base + tri[0] as usize;
				let vb = base + tri[1] as usize;
				let vc = base + tri[2] as usize;
				if va < first_vertex
					|| vb < first_vertex
					|| vc < first_vertex
					|| va >= end_vertex
					|| vb >= end_vertex
					|| vc >= end_vertex
				{
					continue;
				}
				let ga = group_of[va - first_vertex];
				let gb = group_of[vb - first_vertex];
				let gc = group_of[vc - first_vertex];
				if ga == gb || gb == gc || gc == ga {
					continue;
				}
				for (u, v) in [(ga, gb), (gb, gc), (gc, ga)] {
					let edge = if u < v { (u, v) } else { (v, u) };
					*edge_counts.entry(edge).or_insert(0) += 1;
				}
			}
		}

		if !edge_counts.values().any(|&count| count == 1) {
			continue;
		}

		let mut boundary_outward = vec![Vec3::ZERO; group_count];
		for primitive in &mut mesh.primitives {
			let base = primitive.base_vertex as usize;
			let start = primitive.first_index as usize;
			let end = start + primitive.index_count as usize;
			let mut prim_open = false;

			for tri in asset.indices[start..end].chunks_exact(3) {
				let va = base + tri[0] as usize;
				let vb = base + tri[1] as usize;
				let vc = base + tri[2] as usize;
				if va < first_vertex
					|| vb < first_vertex
					|| vc < first_vertex
					|| va >= end_vertex
					|| vb >= end_vertex
					|| vc >= end_vertex
				{
					continue;
				}
				let ga = group_of[va - first_vertex];
				let gb = group_of[vb - first_vertex];
				let gc = group_of[vc - first_vertex];
				if ga == gb || gb == gc || gc == ga {
					continue;
				}
				let verts = [(va, ga), (vb, gb), (vc, gc)];
				for edge_idx in 0..3 {
					let (v0, g0) = verts[edge_idx];
					let (v1, g1) = verts[(edge_idx + 1) % 3];
					let (v2, _) = verts[(edge_idx + 2) % 3];
					let edge = if g0 < g1 { (g0, g1) } else { (g1, g0) };
					if edge_counts.get(&edge).copied() != Some(1) {
						continue;
					}
					prim_open = true;
					let p0 = Vec3::from(asset.vertices[v0].position);
					let p1 = Vec3::from(asset.vertices[v1].position);
					let p2 = Vec3::from(asset.vertices[v2].position);
					let e = p1 - p0;
					let len2 = e.length_squared();
					if len2 > 1.0e-20 {
						let to_opp = p2 - p0;
						let perp = to_opp - e * (to_opp.dot(e) / len2);
						let outward = (-perp).normalize_or_zero();
						boundary_outward[g0] += outward;
						boundary_outward[g1] += outward;
					}
				}
			}

			primitive.has_open_edges = prim_open;
		}

		if asset.knockout_vertices.is_empty() {
			asset.knockout_vertices = asset.ink_vertices.clone();
		}

		for primitive in &mesh.primitives {
			if !primitive.has_open_edges {
				continue;
			}
			let start = primitive.base_vertex as usize;
			let end = start + primitive.vertex_count as usize;
			for v in start..end {
				let out_sum = boundary_outward[group_of[v - first_vertex]];
				if out_sum.length_squared() <= 1.0e-6 {
					continue;
				}
				let out_dir = out_sum.normalize();
				let surf_n = Vec3::from(
					asset.ink_vertices[v]
						.normal
						.map(|channel| f32::from(channel) / 127.0),
				)
				.normalize_or_zero();
				let combined = (out_dir + surf_n * 0.20).normalize_or_zero();
				if combined.length_squared() > 0.0 {
					asset.knockout_vertices[v].normal = snorm(combined);
				}
			}
		}
	}
}

/// Reads one primitive's vertices and indices into the asset's shared
/// arrays. Strips and fans become lists, missing normals are computed, and
/// skin weights are renormalized, so the renderer sees one vertex layout.
fn load_primitive(
	document: &Document<'_>,
	asset: &mut Asset,
	primitive: &json::Primitive,
	target_count: usize,
) -> Result<Option<Primitive>, LoadError> {
	let mode = primitive.mode.unwrap_or(TRIANGLES);

	if !matches!(mode, TRIANGLES | TRIANGLE_STRIP | TRIANGLE_FAN) {
		// Points and lines are not drawn.
		return Ok(None);
	}

	let count = primitive_vertices(document, primitive)?;

	if asset.vertices.len() + count > MAX_ASSET_VERTICES {
		return Err(LoadError::TooLarge("the meshes have too many vertices"));
	}

	let reader = Attributes {
		document,
		primitive,
		count,
	};
	let positions: Vec<Vec3> = reader
		.read(reader.attribute("POSITION").unwrap_or_default(), 3)?
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

	let normals: Vec<Vec3> = match reader.attribute("NORMAL") {
		Some(index) => {
			reader
				.read(index, 3)?
				.as_chunks::<3>()
				.0
				.iter()
				.map(|chunk| Vec3::from_slice(chunk).try_normalize().unwrap_or(Vec3::Y))
				.collect()
		}
		None => computed_normals(&positions, &indices),
	};

	let mut uvs = match reader.attribute("TEXCOORD_0") {
		Some(index) => reader.read(index, 2)?,
		None => vec![0.0; count * 2],
	};

	if let Some(transform) = UvTransform::of_primitive(document, primitive) {
		for uv in uvs.as_chunks_mut::<2>().0 {
			*uv = transform.apply(*uv);
		}
	}

	let colors = reader.colors()?;
	let ink_weights = reader.ink_weights()?;

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
		asset.ink_vertices.push(InkVertex {
			normal: snorm(normals[index]),
			weight: ink_weights[index],
		});
	}

	let skin_base = load_skin(&reader, asset)?;
	let morph = load_morphs(&reader, asset, target_count, &positions, &mut bounds)?;

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
		has_open_edges: false,
	}))
}

/// Reads a primitive's joints and weights into the asset's skin vertices,
/// and returns where they start. Weights are renormalized so they sum to
/// one, since exporters round them and the shader does not divide.
fn load_skin(reader: &Attributes<'_, '_>, asset: &mut Asset) -> Result<Option<u32>, LoadError> {
	let (Some(joints), Some(weights)) =
		(reader.attribute("JOINTS_0"), reader.attribute("WEIGHTS_0"))
	else {
		return Ok(None);
	};
	let (joints, joint_width) = reader.document.integers(joints)?;
	let weights = reader.read(weights, 4)?;

	if joint_width != 4 || joints.len() != reader.count * 4 {
		return Err(malformed("JOINTS_0 must be VEC4 per vertex"));
	}

	let base = asset.skin_vertices.len() as u32;

	for (joints, weights) in joints
		.as_chunks::<4>()
		.0
		.iter()
		.zip(weights.as_chunks::<4>().0)
	{
		asset.skin_vertices.push(SkinVertex {
			joints: [0, 1, 2, 3].map(|slot| joints[slot].min(u32::from(u16::MAX)) as u16),
			weights: skin_weights(weights),
		});
	}

	Ok(Some(base))
}

/// Weights as fractions of `u16::MAX` that sum to one. A vertex whose
/// weights are all zero, or not numbers, follows its first joint.
fn skin_weights(weights: &[f32]) -> [u16; 4] {
	let weight = [0, 1, 2, 3].map(|slot| {
		let value = weights[slot];
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

	weight.map(|value| (value * f32::from(u16::MAX)).round() as u16)
}

/// Reads a primitive's morph targets into the asset's deltas: a position
/// and a normal delta per target and vertex, zero where a target has
/// none. Every primitive of a mesh gets `target_count` targets, so the
/// mesh's weights index each of them alike. The bounds grow to hold each
/// target fully applied.
fn load_morphs(
	reader: &Attributes<'_, '_>,
	asset: &mut Asset,
	target_count: usize,
	positions: &[Vec3],
	bounds: &mut Aabb,
) -> Result<Option<MorphRange>, LoadError> {
	let primitive_targets = &reader.primitive.targets;

	if primitive_targets.is_empty() {
		return Ok(None);
	}

	let count = reader.count;
	let targets = primitive_targets
		.len()
		.min(target_count.max(primitive_targets.len()));

	if targets > MAX_MORPH_TARGETS {
		return Err(LoadError::TooLarge("a mesh has too many morph targets"));
	}

	let base = asset.morph_deltas.len();
	let size = targets * count * 2;

	if base + size > MAX_ASSET_VERTICES * 4 {
		return Err(LoadError::TooLarge("the morph targets are too large"));
	}

	asset.morph_deltas.resize(base + size, [0.0; 4]);

	for (target, attributes) in primitive_targets.iter().enumerate() {
		for (slot, name) in [(0, "POSITION"), (1, "NORMAL")] {
			let Some(&index) = attributes.get(name) else {
				continue;
			};
			let deltas = reader.read(index, 3)?;

			for (vertex, delta) in deltas.as_chunks::<3>().0.iter().enumerate() {
				let offset = base + (target * count + vertex) * 2 + slot;
				asset.morph_deltas[offset] = [delta[0], delta[1], delta[2], 0.0];
			}

			if slot == 0 {
				positions
					.iter()
					.zip(deltas.as_chunks::<3>().0)
					.for_each(|(position, delta)| bounds.grow(*position + Vec3::from_slice(delta)));
			}
		}
	}

	Ok(Some(MorphRange {
		base: base as u32,
		targets: targets as u32,
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
		let first_vertex = asset.vertices.len();

		for primitive in &mesh.primitives {
			if let Some(primitive) = load_primitive(document, asset, primitive, target_count)? {
				primitives.push(primitive);
			}
		}

		weld_mesh(asset, first_vertex);

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

/// Reads the skins, then measures how far each joint reaches.
fn load_skins(document: &Document<'_>, asset: &mut Asset) -> Result<(), LoadError> {
	for skin in &document.root.skins {
		if skin.joints.len() > MAX_JOINTS {
			return Err(LoadError::TooLarge("a skin has too many joints"));
		}

		if skin.joints.iter().any(|joint| *joint >= asset.nodes.len()) {
			return Err(malformed("a skin names a missing joint"));
		}

		asset.skins.push(Skin {
			joint_radius: vec![0.0; skin.joints.len()],
			joints: skin.joints.clone(),
			inverse_bind: inverse_binds(document, skin)?,
		});
	}

	measure_joint_reach(asset);
	Ok(())
}

/// A skin's inverse bind matrices, one per joint; identity where the skin
/// has none.
fn inverse_binds(document: &Document<'_>, skin: &json::Skin) -> Result<Vec<Mat4>, LoadError> {
	let Some(index) = skin.inverse_bind_matrices else {
		return Ok(vec![Mat4::IDENTITY; skin.joints.len()]);
	};
	let (values, width) = document.floats(index)?;

	if width != MAT4_FLOATS || values.len() != skin.joints.len() * MAT4_FLOATS {
		return Err(malformed(
			"a skin's inverse bind matrices do not match its joints",
		));
	}

	Ok(values
		.as_chunks::<MAT4_FLOATS>()
		.0
		.iter()
		.map(|chunk| Mat4::from_cols_slice(chunk))
		.collect())
}

/// Each joint's reach: how far the vertices it moves lie from it in its
/// bind pose. Culling bounds a skinned part by its joints grown by this.
fn measure_joint_reach(asset: &mut Asset) {
	for node in &asset.nodes {
		let (Some(mesh), Some(skin)) = (node.mesh, node.skin) else {
			continue;
		};
		let skin = &mut asset.skins[skin];

		for primitive in &asset.meshes[mesh].primitives {
			let Some(skin_base) = primitive.skin_base else {
				continue;
			};
			let range = |base: u32| base as usize..base as usize + primitive.vertex_count as usize;
			let vertices = &asset.vertices[range(primitive.base_vertex)];
			let influences = &asset.skin_vertices[range(skin_base)];

			for (vertex, influence) in vertices.iter().zip(influences) {
				grow_reach(skin, Vec3::from(vertex.position), influence);
			}
		}
	}
}

fn grow_reach(skin: &mut Skin, position: Vec3, influence: &SkinVertex) {
	for (joint, weight) in influence.joints.iter().zip(influence.weights) {
		let joint = usize::from(*joint);

		if weight == 0 || joint >= skin.joints.len() {
			continue;
		}

		let local = skin.inverse_bind[joint].transform_point3(position);
		skin.joint_radius[joint] = skin.joint_radius[joint].max(local.length());
	}
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

			// Clips often share one sampler input between channels.
			if let Entry::Vacant(entry) = cache.entry(sampler.input) {
				entry.insert(document.floats(sampler.input)?.0);
			}

			let times = cache[&sampler.input].clone();

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

			// A cubic spline key holds an in tangent, a value, and an out
			// tangent.
			let per_key = match interpolation {
				Interpolation::CubicSpline => width * 3,
				Interpolation::Step | Interpolation::Linear => width,
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

	/// One triangle with texture coordinates (0, 0), (1, 0) and (0, 1), and
	/// a material whose base color texture carries `transform`.
	fn transformed_triangle(transform: &str) -> Asset {
		let json = r#"{"asset":{"version":"2.0"},
			"buffers":[{"byteLength":60,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAIA/"}],
			"bufferViews":[{"buffer":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":24}],
			"accessors":[
				{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
				{"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}],
			"textures":[{}],
			"materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0,
				"extensions":{"KHR_texture_transform":TRANSFORM}}}}],
			"meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"material":0}]}],
			"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0}"#
			.replace("TRANSFORM", transform);

		load(json.as_bytes(), &mut |_| Err("none".into())).unwrap()
	}

	fn uvs(asset: &Asset) -> Vec<[f32; 2]> {
		asset.vertices.iter().map(|vertex| vertex.uv).collect()
	}

	#[test]
	fn texture_transforms_are_baked_into_texture_coordinates() {
		let asset = transformed_triangle(r#"{"offset":[0.25,0.5],"scale":[0.5,0.25]}"#);
		assert_eq!(uvs(&asset), [[0.25, 0.5], [0.75, 0.5], [0.25, 0.75]]);

		// A quarter turn carries u onto -v and v onto u.
		let asset = transformed_triangle(r#"{"rotation":1.5707964}"#);
		for (uv, expected) in uvs(&asset)
			.iter()
			.zip([[0.0, 0.0], [0.0, -1.0], [1.0, 0.0]])
		{
			assert!(
				(uv[0] - expected[0]).abs() < 1.0e-6 && (uv[1] - expected[1]).abs() < 1.0e-6,
				"{uv:?}"
			);
		}

		// A transform that reads another set leaves this one alone.
		let asset = transformed_triangle(r#"{"offset":[0.5,0.5],"texCoord":1}"#);
		assert_eq!(uvs(&asset), [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
	}

	#[test]
	fn a_file_requiring_texture_transforms_loads() {
		let json = br#"{"asset":{"version":"2.0"},"extensionsRequired":["KHR_texture_transform"]}"#;
		assert!(load(json, &mut |_| Err("none".into())).is_ok());
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

	/// A cube from -0.5 to 0.5 split at its hard edges, as exporters write
	/// one: four vertices a face, each with its face's normal, 24 in all,
	/// and 12 triangles wound counter-clockwise seen from outside.
	fn split_cube() -> (Vec<Vec3>, Vec<Vec3>, Vec<[u32; 3]>) {
		let mut positions = Vec::new();
		let mut normals = Vec::new();
		let mut triangles = Vec::new();

		for normal in [
			Vec3::X,
			Vec3::NEG_X,
			Vec3::Y,
			Vec3::NEG_Y,
			Vec3::Z,
			Vec3::NEG_Z,
		] {
			// Two axes across the face with u x v = normal, so the corners go
			// round it counter-clockwise.
			let u = normal.any_orthonormal_vector();
			let v = normal.cross(u);
			let base = positions.len() as u32;

			for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
				positions.push((normal + u * a + v * b) * 0.5);
				normals.push(normal);
			}

			triangles.extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
		}

		(positions, normals, triangles)
	}

	/// The split cube as a file whose one mesh has two primitives of three
	/// faces each, both reading the same vertices, with a `COLOR_1` of
	/// `(type, component type, bytes)` when one is given.
	fn split_cube_file(color_1: Option<(&str, u32, Vec<u8>)>) -> Result<Asset, LoadError> {
		let (positions, normals, triangles) = split_cube();
		let mut bin: Vec<u8> = positions
			.iter()
			.chain(&normals)
			.flat_map(Vec3::to_array)
			.flat_map(f32::to_le_bytes)
			.collect();
		bin.extend(
			triangles
				.iter()
				.flatten()
				.flat_map(|index| index.to_le_bytes()),
		);
		let (attribute, view, accessor) = match color_1 {
			Some((kind, component, bytes)) => {
				let offset = bin.len();
				let length = bytes.len();
				bin.extend(bytes);
				(
					r#","COLOR_1":4"#.to_owned(),
					format!(r#",{{"buffer":0,"byteOffset":{offset},"byteLength":{length}}}"#),
					format!(
						r#",{{"bufferView":3,"componentType":{component},"normalized":{},"count":24,"type":"{kind}"}}"#,
						component != 5126
					),
				)
			}
			None => (String::new(), String::new(), String::new()),
		};
		let json = format!(
			r#"{{"asset":{{"version":"2.0"}},
			"buffers":[{{"byteLength":{},"uri":"cube.bin"}}],
			"bufferViews":[{{"buffer":0,"byteLength":288}},{{"buffer":0,"byteOffset":288,"byteLength":288}},
				{{"buffer":0,"byteOffset":576,"byteLength":144}}{view}],
			"accessors":[
				{{"bufferView":0,"componentType":5126,"count":24,"type":"VEC3","min":[-0.5,-0.5,-0.5],"max":[0.5,0.5,0.5]}},
				{{"bufferView":1,"componentType":5126,"count":24,"type":"VEC3"}},
				{{"bufferView":2,"componentType":5125,"count":18,"type":"SCALAR"}},
				{{"bufferView":2,"byteOffset":72,"componentType":5125,"count":18,"type":"SCALAR"}}{accessor}],
			"meshes":[{{"primitives":[
				{{"attributes":{{"POSITION":0,"NORMAL":1{attribute}}},"indices":2}},
				{{"attributes":{{"POSITION":0,"NORMAL":1{attribute}}},"indices":3}}]}}],
			"nodes":[{{"mesh":0}}],"scenes":[{{"nodes":[0]}}],"scene":0}}"#,
			bin.len()
		);

		load(json.as_bytes(), &mut |_| Ok(bin.clone()))
	}

	#[test]
	fn outline_normals_weld_across_hard_edges() {
		let (positions, normals, _) = split_cube();
		let welded = welded_normals(&positions, &normals);

		// Three faces meet at right angles at every corner, so each of the
		// corner's three copies points out along the same diagonal.
		for (point, normal) in positions.iter().zip(&welded) {
			let diagonal = point.signum() / 3.0_f32.sqrt();
			assert!(normal.abs_diff_eq(diagonal, 1.0e-6), "{point} {normal}");
		}

		// Copies with the same normal, as a seam splits, count once; a lone
		// vertex keeps its own; and the two sides of a sheet, which cancel,
		// keep theirs.
		let points = [
			Vec3::ZERO,
			Vec3::ZERO,
			Vec3::ZERO,
			Vec3::X,
			Vec3::Y,
			Vec3::Y,
		];
		let normals = [Vec3::X, Vec3::X, Vec3::Y, Vec3::Z, Vec3::Z, Vec3::NEG_Z];
		let between = Vec3::new(1.0, 1.0, 0.0).normalize();
		let welded = welded_normals(&points, &normals);
		assert!(
			welded[..3]
				.iter()
				.all(|normal| normal.abs_diff_eq(between, 1.0e-6)),
			"{welded:?}"
		);
		assert_eq!(welded[3..], [Vec3::Z, Vec3::Z, Vec3::NEG_Z]);
		assert_eq!(welded_normals(&[], &[]), Vec::<Vec3>::new());
	}

	#[test]
	fn outline_normals_weld_across_a_meshs_primitives() {
		let asset = split_cube_file(None).unwrap();
		assert_eq!(asset.mesh_count(), (1, 2));
		assert_eq!(asset.vertices.len(), 48, "each primitive reads all 24");
		assert_eq!(asset.ink_vertices.len(), asset.vertices.len());
		// 1/sqrt(3) in signed normalized bytes.
		let corner = (127.0 / 3.0_f32.sqrt()).round() as i8;

		// The corner at (+, +, +) has faces +x and +y in the first primitive
		// and +z in the second: welded per primitive, the copies would part.
		for (vertex, ink) in asset.vertices.iter().zip(&asset.ink_vertices) {
			let expected = Vec3::from(vertex.position)
				.signum()
				.to_array()
				.map(|sign| sign as i8 * corner);
			assert_eq!(ink.normal, expected, "{:?}", vertex.position);
			assert_eq!(ink.weight, 0, "with no COLOR_1, the part's own line");
		}
	}

	#[test]
	fn color_1s_red_channel_weighs_the_line() {
		assert_eq!(ink_weight_byte(0.0), -127, "no line");
		assert_eq!(ink_weight_byte(0.5), 0, "the part's line");
		assert_eq!(ink_weight_byte(1.0), 127, "twice as heavy");
		assert_eq!(ink_weight_byte(0.75), 64);
		assert_eq!(ink_weight_byte(3.0), 127);
		assert_eq!(ink_weight_byte(-1.0), -127);
		assert_eq!(ink_weight_byte(f32::NAN), 0);

		// Floats, red rising across the vertices; green, blue and alpha are
		// free for other uses.
		let colors: Vec<u8> = (0..24)
			.flat_map(|vertex| [vertex as f32 / 23.0, 0.3, 0.9, 1.0])
			.flat_map(f32::to_le_bytes)
			.collect();
		let asset = split_cube_file(Some(("VEC4", 5126, colors))).unwrap();
		for (index, ink) in asset.ink_vertices.iter().enumerate() {
			assert_eq!(ink.weight, ink_weight_byte((index % 24) as f32 / 23.0));
		}

		// Normalized bytes, as gltfpack writes them: 0, 128 and 255.
		let colors: Vec<u8> = (0..24_u8)
			.flat_map(|vertex| [[0, 128, 255][usize::from(vertex % 3)], 7, 7])
			.collect();
		let asset = split_cube_file(Some(("VEC3", 5121, colors))).unwrap();
		for (index, ink) in asset.ink_vertices.iter().enumerate() {
			assert_eq!(ink.weight, [-127, 0, 127][index % 24 % 3], "vertex {index}");
		}

		// Anything else is not a colour.
		let colors = [0.5_f32; 48]
			.iter()
			.flat_map(|value| value.to_le_bytes())
			.collect();
		assert!(matches!(
			split_cube_file(Some(("VEC2", 5126, colors))),
			Err(LoadError::Malformed(_))
		));
	}

	#[test]
	fn units_undo_the_rest_poses_scale() {
		let node = |scale: f32, children: Vec<usize>, skin: Option<usize>| {
			AssetNode {
				name: String::new(),
				parent: None,
				children,
				translation: Vec3::new(1.0, 2.0, 3.0),
				rotation: Quat::from_rotation_y(0.7),
				scale: Vec3::splat(scale),
				mesh: None,
				skin,
				weights: None,
			}
		};
		let asset = Asset {
			nodes: vec![
				node(0.5, vec![1, 2], None),
				node(0.25, Vec::new(), None),
				node(1.0, Vec::new(), Some(0)),
				node(4.0, Vec::new(), None),
				node(0.0, Vec::new(), None),
			],
			roots: vec![0, 3, 4],
			skins: vec![Skin {
				joints: vec![3],
				inverse_bind: vec![Mat4::from_scale(Vec3::splat(0.5))],
				joint_radius: vec![0.0],
			}],
			..Asset::default()
		};
		let units = rest_units(&asset);
		let expected = [
			0.5, // A quantized mesh's node scales it down, under its parents.
			0.125,
			// A skin is placed by its joints, not its node: the first
			// joint's rest pose after its inverse bind.
			2.0, 4.0, // A flat node measures nothing, so its units are one.
			1.0,
		];
		for (unit, expected) in units.iter().zip(expected) {
			assert!((unit - expected).abs() < 1.0e-5, "{units:?}");
		}
		assert_eq!(units.len(), 5);
	}

	#[test]
	fn welded_normals_weld_across_cell_boundaries() {
		// Mesh spans [0, 1] -> length = 1.0, tolerance = 1e-5.
		// Test seam copies 2e-8 apart straddling both 0.5e-5 and 4.0e-5 cell
		// boundaries.
		for boundary in [0.5e-5_f32, 4.0e-5_f32] {
			let a = Vec3::new(boundary - 1.0e-8, 0.0, 0.0);
			let b = Vec3::new(boundary + 1.0e-8, 0.0, 0.0);
			let far = Vec3::new(1.0, 0.0, 0.0);
			let welded = welded_normals(&[a, b, far], &[Vec3::X, Vec3::Y, Vec3::Z]);
			assert!(
				welded[0].abs_diff_eq(welded[1], 1.0e-5),
				"vertices 2e-8 apart failed to weld across boundary {boundary:e}: {:?} vs {:?}",
				welded[0],
				welded[1]
			);
			assert!(welded[2].abs_diff_eq(Vec3::Z, 1.0e-5));
		}
	}

	#[test]
	fn welded_normals_do_not_overflow_on_extreme_bounds() {
		// Valid finite f32 positions whose difference overflows f32 to +inf.
		let points = [
			Vec3::new(-1.0e38, 0.0, 0.0),
			Vec3::new(1.0e38, 0.0, 0.0),
			Vec3::new(0.0, 1.0, 0.0),
		];
		let normals = [Vec3::X, Vec3::Y, Vec3::Z];
		let welded = welded_normals(&points, &normals);
		assert_eq!(welded[2], Vec3::Z, "distant vertex got welded: {welded:?}");
	}

	#[test]
	fn welding_a_dense_cell_stays_near_linear() {
		// Two far corners make the weld tolerance 1 cm, in cells 4 cm wide.
		// A lattice of 46 656 distinct points 1 mm apart then fills one cell,
		// each point twice, with a different normal on each copy. Comparing
		// every pair in that cell took minutes.
		let side = 36_usize;
		let mut positions = vec![Vec3::ZERO, Vec3::splat(1000.0 / 3.0_f32.sqrt())];
		let mut normals = vec![Vec3::Z, Vec3::Z];

		for x in 0..side {
			for y in 0..side {
				for z in 0..side {
					let point = Vec3::new(x as f32, y as f32, z as f32) * 1.0e-3 + 0.481;
					positions.extend([point, point]);
					normals.extend([Vec3::X, Vec3::Y]);
				}
			}
		}

		let started = std::time::Instant::now();
		let welded = welded_normals(&positions, &normals);
		let elapsed = started.elapsed();
		let mean = (Vec3::X + Vec3::Y).normalize();
		assert!(
			welded[2..]
				.iter()
				.all(|normal| normal.abs_diff_eq(mean, 1.0e-5)),
			"the copies of each point weld"
		);
		assert!(
			elapsed < std::time::Duration::from_secs(10),
			"welding {} vertices with {} distinct points in one cell took {elapsed:?}",
			positions.len(),
			side * side * side
		);
	}

	#[test]
	fn node_seeds_match_ieee_crc32() {
		assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
		assert_ne!(crc32(b"head_0"), crc32(b"robe_0"));
	}

	/// Builds a minimal faceted bent strip (two flat quads meeting at a 90°
	/// corner, with a second strip along `+x` so vertices sit 1 and 2 rings
	/// away from the corner) with optional root and mesh `outlineSmoothRings`
	/// glTF extras.
	fn bent_faceted_gltf(root_extra: Option<u8>, mesh_extra: Option<u8>) -> Vec<u8> {
		// 3 flat quads along x=0..1..2 (normal +z) and 1 bent down at x=0 into
		// z=0..-1 (normal -x). Vertex 2 at (1, 0, 0) is 1 ring from the 90 deg
		// crease at x=0; vertex 4 at (2, 0, 0) is 2 rings away.
		let positions: [[f32; 3]; 12] = [
			// Quad A: x in [0, 1], z=0, normal +z
			[0.0, 0.0, 0.0],
			[1.0, 0.0, 0.0],
			[1.0, 1.0, 0.0],
			[0.0, 1.0, 0.0],
			// Quad B: x in [1, 2], z=0, normal +z
			[1.0, 0.0, 0.0],
			[2.0, 0.0, 0.0],
			[2.0, 1.0, 0.0],
			[1.0, 1.0, 0.0],
			// Quad C: x=0, z in [-1, 0], normal -x
			[0.0, 0.0, -1.0],
			[0.0, 0.0, 0.0],
			[0.0, 1.0, 0.0],
			[0.0, 1.0, -1.0],
		];
		let normals: [[f32; 3]; 12] = [
			[0.0, 0.0, 1.0],
			[0.0, 0.0, 1.0],
			[0.0, 0.0, 1.0],
			[0.0, 0.0, 1.0],
			[0.0, 0.0, 1.0],
			[0.0, 0.0, 1.0],
			[0.0, 0.0, 1.0],
			[0.0, 0.0, 1.0],
			[-1.0, 0.0, 0.0],
			[-1.0, 0.0, 0.0],
			[-1.0, 0.0, 0.0],
			[-1.0, 0.0, 0.0],
		];
		let indices: [u16; 18] = [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11];

		let mut bin = Vec::new();
		let pos_off = bin.len();
		for p in &positions {
			for v in p {
				bin.extend_from_slice(&v.to_le_bytes());
			}
		}
		let pos_len = bin.len() - pos_off;
		let norm_off = bin.len();
		for n in &normals {
			for v in n {
				bin.extend_from_slice(&v.to_le_bytes());
			}
		}
		let norm_len = bin.len() - norm_off;
		let idx_off = bin.len();
		for i in &indices {
			bin.extend_from_slice(&i.to_le_bytes());
		}
		let idx_len = bin.len() - idx_off;
		while bin.len() % 4 != 0 {
			bin.push(0);
		}

		let root_ext = root_extra
			.map(|rings| format!(r#","extras":{{"outlineSmoothRings":{rings}}}"#))
			.unwrap_or_default();
		let mesh_ext = mesh_extra
			.map(|rings| format!(r#","extras":{{"outlineSmoothRings":{rings}}}"#))
			.unwrap_or_default();
		let json = format!(
			r#"{{"asset":{{"version":"2.0"}},"scenes":[{{"nodes":[0]}}],"nodes":[{{"name":"strip","mesh":0}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"NORMAL":1}},"indices":2}}]{mesh_ext}}}],"buffers":[{{"byteLength":{}}}],"bufferViews":[{{"buffer":0,"byteOffset":{pos_off},"byteLength":{pos_len}}},{{"buffer":0,"byteOffset":{norm_off},"byteLength":{norm_len}}},{{"buffer":0,"byteOffset":{idx_off},"byteLength":{idx_len}}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":12,"type":"VEC3","min":[0,0,-1],"max":[2,1,0]}},{{"bufferView":1,"componentType":5126,"count":12,"type":"VEC3"}},{{"bufferView":2,"componentType":5123,"count":18,"type":"SCALAR"}}]{root_ext}}}"#,
			bin.len()
		);
		let mut json_bytes = json.into_bytes();
		while json_bytes.len() % 4 != 0 {
			json_bytes.push(b' ');
		}
		let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
		let mut glb = Vec::with_capacity(total);
		glb.extend_from_slice(b"glTF");
		glb.extend_from_slice(&2_u32.to_le_bytes());
		glb.extend_from_slice(&(total as u32).to_le_bytes());
		glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
		glb.extend_from_slice(&0x4E4F_534A_u32.to_le_bytes());
		glb.extend_from_slice(&json_bytes);
		glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
		glb.extend_from_slice(&0x004E_4942_u32.to_le_bytes());
		glb.extend_from_slice(&bin);
		glb
	}

	#[test]
	fn outline_smooth_rings_defaults_to_unchanged_and_propagates_across_rings() {
		let bytes = bent_faceted_gltf(None, None);
		let default_asset = Asset::from_slice(&bytes).unwrap();
		let zero_asset = Asset::from_slice_with_options(
			&bytes,
			LoadOptions {
				outline_smooth_rings: Some(0),
			},
		)
		.unwrap();
		assert_eq!(
			default_asset.ink_vertices, zero_asset.ink_vertices,
			"default and 0 rings keep today's hull byte-for-byte unchanged"
		);
		// Vertex 1 (at x=1, 1 ring from crease) is pure +z without smoothing.
		assert_eq!(default_asset.ink_vertices[1].normal, [0, 0, 127]);
		// Vertex 5 (at x=2, 2 rings from crease) is pure +z without smoothing.
		assert_eq!(default_asset.ink_vertices[5].normal, [0, 0, 127]);

		// 1 ring tilts vertex 1 (1 hop from crease) toward -x, while vertex 5
		// (2 hops) stays pure +z.
		let ring1 = Asset::from_slice_with_options(
			&bytes,
			LoadOptions {
				outline_smooth_rings: Some(1),
			},
		)
		.unwrap();
		assert!(
			ring1.ink_vertices[1].normal[0] < -10,
			"1 ring spreads crease to 1-hop neighbor: {:?}",
			ring1.ink_vertices[1].normal
		);
		assert_eq!(
			ring1.ink_vertices[5].normal,
			[0, 0, 127],
			"1 ring does not reach 2-hop vertex"
		);

		// 2 rings reaches vertex 5 (2 hops away) as well.
		let ring2 = Asset::from_slice_with_options(
			&bytes,
			LoadOptions {
				outline_smooth_rings: Some(2),
			},
		)
		.unwrap();
		assert!(
			ring2.ink_vertices[5].normal[0] < -5,
			"2 rings reaches 2-hop vertex: {:?}",
			ring2.ink_vertices[5].normal
		);

		// Clamping at MAX_OUTLINE_SMOOTH_RINGS.
		let capped = Asset::from_slice_with_options(
			&bytes,
			LoadOptions {
				outline_smooth_rings: Some(99),
			},
		)
		.unwrap();
		let max_rings = Asset::from_slice_with_options(
			&bytes,
			LoadOptions {
				outline_smooth_rings: Some(MAX_OUTLINE_SMOOTH_RINGS),
			},
		)
		.unwrap();
		assert_eq!(capped.ink_vertices, max_rings.ink_vertices);

		// glTF extras: mesh beats root, explicit LoadOptions beats both.
		let with_root = Asset::from_slice(&bent_faceted_gltf(Some(2), None)).unwrap();
		assert_eq!(with_root.ink_vertices, ring2.ink_vertices);

		let with_mesh = Asset::from_slice(&bent_faceted_gltf(Some(2), Some(1))).unwrap();
		assert_eq!(with_mesh.ink_vertices, ring1.ink_vertices);

		let override_zero = Asset::from_slice_with_options(
			&bent_faceted_gltf(Some(2), Some(2)),
			LoadOptions {
				outline_smooth_rings: Some(0),
			},
		)
		.unwrap();
		assert_eq!(override_zero.ink_vertices, default_asset.ink_vertices);
	}
}
