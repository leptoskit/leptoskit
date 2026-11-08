//! An asset's GPU copy: its vertex, skin, and index buffers, its morph
//! targets packed into a texture, its textures, and its materials. One copy
//! per device serves every instance in every scene.

use std::collections::HashMap;

use wgpu::util::DeviceExt as _;

use crate::asset::AlphaMode;
use crate::asset::Asset;
use crate::asset::Image;
use crate::asset::SamplerDesc;
use crate::asset::TextureRef;
use crate::gpu::Shared;

pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Texels per row of the morph texture; the shader's `MORPH_WIDTH`.
pub(crate) const MORPH_WIDTH: u32 = 2048;
/// Texels per row of the per-frame data texture; the shader's `DATA_WIDTH`.
pub(crate) const DATA_WIDTH: u32 = 1024;

/// One draw's per-instance attributes: 128 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct InstanceRecord {
	pub model: [[f32; 4]; 4],
	pub tint: [f32; 4],
	pub emissive: [f32; 4],
	pub rim: [f32; 4],
	pub params: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialUniform {
	base_color: [f32; 4],
	emissive: [f32; 4],
	surface: [f32; 4],
	flags: [u32; 4],
}

/// A material's bind group.
pub(crate) struct GpuMaterial {
	pub bind_group: wgpu::BindGroup,
}

pub(crate) struct GpuAsset {
	pub vertices: wgpu::Buffer,
	pub skin: Option<wgpu::Buffer>,
	pub indices: wgpu::Buffer,
	pub morphs: Option<wgpu::BindGroup>,
	pub materials: Vec<GpuMaterial>,
}

/// Halves an RGBA8 image with a box filter.
fn half_size(image: &Image) -> Image {
	let width = (image.width / 2).max(1);
	let height = (image.height / 2).max(1);
	let mut pixels = vec![0; (width * height * 4) as usize];

	for y in 0..height {
		for x in 0..width {
			for channel in 0..4 {
				let mut sum = 0_u32;

				for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
					let sx = (x * 2 + dx).min(image.width - 1);
					let sy = (y * 2 + dy).min(image.height - 1);
					sum +=
						u32::from(image.pixels[((sy * image.width + sx) * 4 + channel) as usize]);
				}

				pixels[((y * width + x) * 4 + channel) as usize] = (sum / 4) as u8;
			}
		}
	}

	Image {
		width,
		height,
		pixels,
	}
}

fn upload_image(device: &wgpu::Device, queue: &wgpu::Queue, image: &Image) -> wgpu::TextureView {
	let mips = 32 - image.width.max(image.height).leading_zeros();
	let texture = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("leptoskit_3d texture"),
		size: wgpu::Extent3d {
			width: image.width,
			height: image.height,
			depth_or_array_layers: 1,
		},
		mip_level_count: mips,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: wgpu::TextureFormat::Rgba8Unorm,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	});
	let mut level = image.clone();

	for mip in 0..mips {
		queue.write_texture(
			wgpu::TexelCopyTextureInfo {
				texture: &texture,
				mip_level: mip,
				origin: wgpu::Origin3d::ZERO,
				aspect: wgpu::TextureAspect::All,
			},
			&level.pixels,
			wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(level.width * 4),
				rows_per_image: None,
			},
			wgpu::Extent3d {
				width: level.width,
				height: level.height,
				depth_or_array_layers: 1,
			},
		);

		if mip + 1 < mips {
			level = half_size(&level);
		}
	}

	texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn address(mode: u8) -> wgpu::AddressMode {
	match mode {
		0 => wgpu::AddressMode::ClampToEdge,
		2 => wgpu::AddressMode::MirrorRepeat,
		_ => wgpu::AddressMode::Repeat,
	}
}

/// Packs morph deltas into one texel per vertex and target: position xyz
/// and normal xyz as six halves.
fn pack_morphs(deltas: &[[f32; 4]]) -> Vec<[u32; 4]> {
	let pack = |a: f32, b: f32| {
		u32::from(half::f16::from_f32(a).to_bits())
			| (u32::from(half::f16::from_f32(b).to_bits()) << 16)
	};
	deltas
		.as_chunks::<2>()
		.0
		.iter()
		.map(|pair| {
			let [position, normal] = [pair[0], pair[1]];
			[
				pack(position[0], position[1]),
				pack(position[2], normal[0]),
				pack(normal[1], normal[2]),
				0,
			]
		})
		.collect()
}

fn upload_morphs(
	device: &wgpu::Device,
	queue: &wgpu::Queue,
	shared: &Shared,
	asset: &Asset,
) -> Option<wgpu::BindGroup> {
	if asset.morph_deltas.is_empty() {
		return None;
	}

	let mut texels = pack_morphs(&asset.morph_deltas);
	let rows = (texels.len() as u32).div_ceil(MORPH_WIDTH);

	if rows > device.limits().max_texture_dimension_2d {
		// The device cannot hold this many targets; the meshes draw unmorphed.
		return None;
	}

	texels.resize((rows * MORPH_WIDTH) as usize, [0; 4]);
	let texture = device.create_texture_with_data(
		queue,
		&wgpu::TextureDescriptor {
			label: Some("leptoskit_3d morph targets"),
			size: wgpu::Extent3d {
				width: MORPH_WIDTH,
				height: rows,
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Rgba32Uint,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		},
		wgpu::util::TextureDataOrder::LayerMajor,
		bytemuck::cast_slice(&texels),
	);
	let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
	Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
		label: Some("leptoskit_3d morph targets"),
		layout: &shared.morph_layout,
		entries: &[wgpu::BindGroupEntry {
			binding: 0,
			resource: wgpu::BindingResource::TextureView(&view),
		}],
	}))
}

impl GpuAsset {
	#[allow(clippy::too_many_lines)]
	pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, shared: &Shared, asset: &Asset) -> Self {
		let buffer = |label, contents: &[u8], usage| {
			device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
				label: Some(label),
				// wgpu refuses empty buffers.
				contents: if contents.is_empty() {
					&[0; 16]
				} else {
					contents
				},

				usage,
			})
		};
		let vertices = buffer(
			"leptoskit_3d vertices",
			bytemuck::cast_slice(&asset.vertices),
			wgpu::BufferUsages::VERTEX,
		);
		let skin = (!asset.skin_vertices.is_empty()).then(|| {
			buffer(
				"leptoskit_3d skin",
				bytemuck::cast_slice(&asset.skin_vertices),
				wgpu::BufferUsages::VERTEX,
			)
		});
		let indices = buffer(
			"leptoskit_3d indices",
			bytemuck::cast_slice(&asset.indices),
			wgpu::BufferUsages::INDEX,
		);
		let morphs = upload_morphs(device, queue, shared, asset);

		let mut textures: HashMap<usize, wgpu::TextureView> = HashMap::new();
		let mut samplers: HashMap<SamplerDesc, wgpu::Sampler> = HashMap::new();
		let mut view = |slot: Option<TextureRef>,
		                fallback: &wgpu::TextureView|
		 -> wgpu::TextureView {
			match slot {
				Some(slot) => {
					textures
						.entry(slot.image)
						.or_insert_with(|| upload_image(device, queue, &asset.images[slot.image]))
						.clone()
				}
				None => fallback.clone(),
			}
		};
		let mut materials = Vec::with_capacity(asset.materials.len());

		for material in &asset.materials {
			let slots = [
				material.base_color_texture,
				material.metallic_roughness_texture,
				material.normal_texture,
				material.occlusion_texture,
				material.emissive_texture,
			];

			let mask = slots
				.iter()
				.enumerate()
				.filter(|(_, slot)| slot.is_some())
				.fold(0_u32, |mask, (index, _)| mask | (1 << index));

			let (mode, cutoff) = match material.alpha {
				AlphaMode::Opaque => (0, 0.0),
				AlphaMode::Mask(cutoff) => (1, cutoff),
				AlphaMode::Blend => (2, 0.0),
			};

			let uniform = MaterialUniform {
				base_color: material.base_color,
				emissive: [
					material.emissive[0],
					material.emissive[1],
					material.emissive[2],
					material.emissive_strength,
				],
				surface: [
					material.metallic,
					material.roughness,
					material.normal_scale,
					material.occlusion_strength,
				],
				flags: [mode, u32::from(material.unlit), mask, f32::to_bits(cutoff)],
			};
			let uniform = buffer(
				"leptoskit_3d material",
				bytemuck::bytes_of(&uniform),
				wgpu::BufferUsages::UNIFORM,
			);
			let sampler_desc = slots
				.iter()
				.flatten()
				.next()
				.map(|slot| slot.sampler)
				.unwrap_or_default();
			let sampler = samplers
				.entry(sampler_desc)
				.or_insert_with(|| {
					let filter = if sampler_desc.nearest {
						wgpu::FilterMode::Nearest
					} else {
						wgpu::FilterMode::Linear
					};

					device.create_sampler(&wgpu::SamplerDescriptor {
						label: Some("leptoskit_3d material"),
						address_mode_u: address(sampler_desc.repeat_u),
						address_mode_v: address(sampler_desc.repeat_v),
						mag_filter: filter,
						min_filter: filter,
						mipmap_filter: wgpu::MipmapFilterMode::Linear,
						..wgpu::SamplerDescriptor::default()
					})
				})
				.clone();
			let views = [
				view(slots[0], &shared.white),
				view(slots[1], &shared.white),
				view(slots[2], &shared.flat_normal),
				view(slots[3], &shared.white),
				view(slots[4], &shared.white),
			];
			let mut entries = vec![wgpu::BindGroupEntry {
				binding: 0,
				resource: uniform.as_entire_binding(),
			}];

			for (index, view) in views.iter().enumerate() {
				entries.push(wgpu::BindGroupEntry {
					binding: index as u32 + 1,
					resource: wgpu::BindingResource::TextureView(view),
				});
			}

			entries.push(wgpu::BindGroupEntry {
				binding: 6,
				resource: wgpu::BindingResource::Sampler(&sampler),
			});
			materials.push(GpuMaterial {
				bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
					label: Some("leptoskit_3d material"),
					layout: &shared.material_layout,
					entries: &entries,
				}),
			});
		}

		Self {
			vertices,
			skin,
			indices,
			morphs,
			materials,
		}
	}
}

/// A GPU buffer that grows to fit what it is given.
pub(crate) struct Growable {
	label: &'static str,
	usage: wgpu::BufferUsages,
	pub buffer: Option<wgpu::Buffer>,
}

impl Growable {
	pub fn new(label: &'static str, usage: wgpu::BufferUsages) -> Self {
		Self {
			label,
			usage: usage | wgpu::BufferUsages::COPY_DST,
			buffer: None,
		}
	}

	/// Uploads `data`, returning whether the buffer had to be replaced.
	pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) -> bool {
		if data.is_empty() {
			return false;
		}

		let size = data.len() as u64;
		let replaced = self
			.buffer
			.as_ref()
			.is_none_or(|buffer| buffer.size() < size);

		if replaced {
			self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
				label: Some(self.label),
				size: size.next_power_of_two().max(256),
				usage: self.usage,
				mapped_at_creation: false,
			}));
		}

		if let Some(buffer) = &self.buffer {
			queue.write_buffer(buffer, 0, data);
		}

		replaced
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn morph_deltas_pack_into_halves() {
		let packed = pack_morphs(&[[1.0, -2.0, 0.5, 0.0], [0.0, 1.0, -1.0, 0.0]]);
		let unpack = |word: u32| {
			[
				half::f16::from_bits(word as u16).to_f32(),
				half::f16::from_bits((word >> 16) as u16).to_f32(),
			]
		};
		assert_eq!(unpack(packed[0][0]), [1.0, -2.0]);
		assert_eq!(unpack(packed[0][1]), [0.5, 0.0]);
		assert_eq!(unpack(packed[0][2]), [1.0, -1.0]);
	}

	#[test]
	fn images_halve_down_to_one_texel() {
		let image = Image {
			width: 3,
			height: 2,
			pixels: (0..24).map(|value| value as u8 * 10).collect(),
		};
		let half = half_size(&image);
		assert_eq!((half.width, half.height), (1, 1));
		let single = half_size(&half);
		assert_eq!((single.width, single.height), (1, 1));
	}
}
