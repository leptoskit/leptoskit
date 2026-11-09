//! The device every view shares: one per process, with the environment,
//! shaders, pipelines, and uploaded assets that all of its renderers reuse.
// The browser has one thread, where wgpu's types are neither `Send` nor
// `Sync`; the `Arc`s share them between renderers, not threads.
#![cfg_attr(target_arch = "wasm32", allow(clippy::arc_with_non_send_sync))]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::Weak;

use crate::asset::Asset;
use crate::environment;
use crate::upload::GpuAsset;

/// The graphics API to draw with.
///
/// [`Backend::Auto`] picks Metal on Apple platforms, Direct3D 12 on
/// Windows, Vulkan on Linux, OpenGL ES on Android, and WebGPU in the browser,
/// falling back to WebGL2 where the browser has no WebGPU. Android defaults
/// to OpenGL ES because some Vulkan drivers (the Solana Seeker's among them)
/// reset the GPU when the camera closes; apps that have tested their devices
/// can choose [`Backend::Vulkan`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Backend {
	/// The platform's default, as above.
	#[default]
	Auto,
	/// Metal (Apple platforms).
	Metal,
	/// Vulkan (Android, Linux, Windows).
	Vulkan,
	/// OpenGL ES (Android) or WebGL2 (the browser).
	Gl,
	/// Direct3D 12 (Windows).
	Dx12,
	/// WebGPU (the browser).
	WebGpu,
}

impl Backend {
	/// Parses a backend's name as an app's configuration writes it:
	/// `auto`, `metal`, `vulkan`, `gl` (or `gles`, `webgl`), `dx12`, or
	/// `webgpu`.
	pub fn from_name(name: &str) -> Option<Self> {
		Some(match name.to_ascii_lowercase().as_str() {
			"auto" | "" => Self::Auto,
			"metal" => Self::Metal,
			"vulkan" => Self::Vulkan,
			"gl" | "gles" | "opengl" | "webgl" | "webgl2" => Self::Gl,
			"dx12" | "d3d12" => Self::Dx12,
			"webgpu" => Self::WebGpu,
			_ => return None,
		})
	}

	/// The wgpu backends this choice allows on the current platform.
	pub fn backends(self) -> wgpu::Backends {
		match self {
			Self::Auto => AUTO_BACKENDS,
			Self::Metal => wgpu::Backends::METAL,
			Self::Vulkan => wgpu::Backends::VULKAN,
			Self::Gl => wgpu::Backends::GL,
			Self::Dx12 => wgpu::Backends::DX12,
			Self::WebGpu => wgpu::Backends::BROWSER_WEBGPU,
		}
	}
}

/// Why the GPU could not draw.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// The system has no GPU adapter wgpu can use with the chosen backend.
	#[error("no GPU adapter is available: {0}")]
	NoAdapter(String),
	/// The adapter refused to create a device.
	#[error("the GPU device could not be created: {0}")]
	Device(String),
	/// The target is bigger than the GPU's largest texture.
	#[error("the target is {width}×{height} pixels, larger than the GPU limit of {limit}")]
	TooLarge { width: u32, height: u32, limit: u32 },
	/// Pixels can only be read back from 8-bit RGBA or BGRA formats.
	#[error("pixels cannot be read back from the {0:?} format")]
	UnsupportedFormat(wgpu::TextureFormat),
	/// Reading pixels back from the GPU failed.
	#[error("reading the pixels back failed: {0}")]
	Readback(String),
	/// A target's views do not match its layers.
	#[error("the target is malformed: {0}")]
	Target(&'static str),
}

impl Error {
	/// Whether the error means there is no usable GPU, rather than a
	/// problem with the scene or target.
	pub fn is_unavailable(&self) -> bool {
		matches!(self, Self::NoAdapter(_) | Self::Device(_))
	}
}

/// A GPU device and everything its renderers share.
///
/// Create one per process and pass it to every [`Renderer`](crate::Renderer):
/// it bakes the environment once, compiles each pipeline once per target
/// format, and uploads each asset once however many scenes show it. Cloning
/// it is cheap.
#[derive(Clone)]
pub struct Gpu(Arc<GpuInner>);

struct GpuInner {
	instance: wgpu::Instance,
	adapter: wgpu::Adapter,
	device: wgpu::Device,
	queue: wgpu::Queue,
	shared: OnceLock<Arc<Shared>>,
}

impl std::fmt::Debug for Gpu {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Gpu")
			.field("adapter", &self.0.adapter.get_info())
			.finish_non_exhaustive()
	}
}

/// The limits the renderer asks for: WebGL2's where the backend is OpenGL
/// ES or WebGL2, the downlevel defaults elsewhere, each raised to what the
/// adapter's resolution allows.
fn limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
	let base = if adapter.get_info().backend == wgpu::Backend::Gl {
		wgpu::Limits::downlevel_webgl2_defaults()
	} else {
		wgpu::Limits::downlevel_defaults()
	};

	base.using_resolution(adapter.limits())
}

impl Gpu {
	/// Finds an adapter for `backend` (compatible with `surface`, when one is
	/// given) and creates a device on it.
	pub async fn request(
		backend: Backend,
		surface: Option<&wgpu::Surface<'_>>,
	) -> Result<Self, Error> {
		let instance = Self::instance(backend).await;
		Self::request_on(instance, surface).await
	}

	/// An instance for `backend`. In the browser it detects WebGPU and
	/// falls back to WebGL2, which is why it is `async`.
	#[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
	pub async fn instance(backend: Backend) -> wgpu::Instance {
		let descriptor = wgpu::InstanceDescriptor {
			backends: backend.backends(),
			..wgpu::InstanceDescriptor::new_without_display_handle_from_env()
		};
		#[cfg(target_arch = "wasm32")]
		{
			wgpu::util::new_instance_with_webgpu_detection(descriptor).await
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			wgpu::Instance::new(descriptor)
		}
	}

	/// Creates a device on an instance the caller made, such as the one a
	/// surface was created from.
	pub async fn request_on(
		instance: wgpu::Instance,
		surface: Option<&wgpu::Surface<'_>>,
	) -> Result<Self, Error> {
		let adapter = instance
			.request_adapter(&wgpu::RequestAdapterOptions {
				compatible_surface: surface,
				..wgpu::RequestAdapterOptions::default()
			})
			.await
			.map_err(|error| Error::NoAdapter(error.to_string()))?;
		let (device, queue) = adapter
			.request_device(&wgpu::DeviceDescriptor {
				label: Some("leptoskit_3d"),
				required_limits: limits(&adapter),
				..wgpu::DeviceDescriptor::default()
			})
			.await
			.map_err(|error| Error::Device(error.to_string()))?;
		Ok(Self::from_parts(instance, adapter, device, queue))
	}

	/// Wraps a device the host already has, such as one an XR runtime chose.
	/// The device must allow the limits of
	/// `wgpu::Limits::downlevel_webgl2_defaults()`.
	pub fn from_parts(
		instance: wgpu::Instance,
		adapter: wgpu::Adapter,
		device: wgpu::Device,
		queue: wgpu::Queue,
	) -> Self {
		Self(Arc::new(GpuInner {
			instance,
			adapter,
			device,
			queue,
			shared: OnceLock::new(),
		}))
	}

	/// A device with no window, for drawing offscreen.
	#[cfg(not(target_arch = "wasm32"))]
	pub fn headless() -> Result<Self, Error> {
		pollster::block_on(async {
			let instance = wgpu::Instance::new(
				wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
			);
			Self::request_on(instance, None).await
		})
	}

	/// The wgpu instance, for creating surfaces.
	pub fn wgpu_instance(&self) -> &wgpu::Instance {
		&self.0.instance
	}

	/// The adapter the device runs on.
	pub fn adapter(&self) -> &wgpu::Adapter {
		&self.0.adapter
	}

	/// The device.
	pub fn device(&self) -> &wgpu::Device {
		&self.0.device
	}

	/// The device's queue.
	pub fn queue(&self) -> &wgpu::Queue {
		&self.0.queue
	}

	/// Whether `other` is this same device.
	pub fn same_device(&self, other: &Self) -> bool {
		Arc::ptr_eq(&self.0, &other.0)
	}

	/// The graphics API the device runs on.
	pub fn backend(&self) -> wgpu::Backend {
		self.0.adapter.get_info().backend
	}

	/// Everything the device's renderers share, created on first use.
	pub(crate) fn shared(&self) -> &Arc<Shared> {
		self.0
			.shared
			.get_or_init(|| Arc::new(Shared::new(&self.0.device, &self.0.queue)))
	}

	/// How many assets the device holds uploaded for its renderers.
	pub fn uploaded_assets(&self) -> usize {
		self.0.shared.get().map_or(0, |shared| shared.asset_count())
	}

	/// The largest multisample count up to 4 that `format` renders with on
	/// this adapter.
	pub(crate) fn samples(&self, format: wgpu::TextureFormat) -> u32 {
		let flags = self.0.adapter.get_texture_format_features(format).flags;
		let depth = self
			.0
			.adapter
			.get_texture_format_features(crate::upload::DEPTH_FORMAT)
			.flags;

		if flags.sample_count_supported(4) && depth.sample_count_supported(4) {
			4
		} else {
			1
		}
	}
}

/// What `Backend::Auto` allows: WebGPU with a WebGL2 fallback in the
/// browser, OpenGL ES on Android (some Vulkan drivers reset the GPU when the
/// camera closes), and each other platform's native API.
const AUTO_BACKENDS: wgpu::Backends = if cfg!(target_arch = "wasm32") {
	wgpu::Backends::BROWSER_WEBGPU.union(wgpu::Backends::GL)
} else if cfg!(target_vendor = "apple") {
	wgpu::Backends::METAL
} else if cfg!(windows) {
	wgpu::Backends::DX12
} else if cfg!(target_os = "android") {
	wgpu::Backends::GL
} else {
	wgpu::Backends::VULKAN
};

/// A render pipeline's variant: how the mesh is skinned and blended, and
/// whether back faces are culled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Variant {
	pub skinned: bool,
	pub blend: bool,
	pub double_sided: bool,
}

impl Variant {
	/// Every variant.
	fn all() -> impl Iterator<Item = Self> {
		let flags = [false, true];
		flags.into_iter().flat_map(move |skinned| {
			flags.into_iter().flat_map(move |blend| {
				flags.map(move |double_sided| {
					Self {
						skinned,
						blend,
						double_sided,
					}
				})
			})
		})
	}
}

fn buffer_layout<T>(
	step_mode: wgpu::VertexStepMode,
	attributes: &[wgpu::VertexAttribute],
) -> wgpu::VertexBufferLayout<'_> {
	wgpu::VertexBufferLayout {
		array_stride: size_of::<T>() as u64,
		step_mode,
		attributes,
	}
}

fn depth_state(write: bool, compare: wgpu::CompareFunction) -> wgpu::DepthStencilState {
	wgpu::DepthStencilState {
		format: crate::upload::DEPTH_FORMAT,
		depth_write_enabled: Some(write),
		depth_compare: Some(compare),
		stencil: wgpu::StencilState::default(),
		bias: wgpu::DepthBiasState::default(),
	}
}

fn color_target(format: wgpu::TextureFormat, blend: bool) -> [Option<wgpu::ColorTargetState>; 1] {
	[Some(wgpu::ColorTargetState {
		format,
		blend: blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
		write_mask: wgpu::ColorWrites::ALL,
	})]
}

/// Every pipeline for one target format and sample count.
pub(crate) struct Pipelines {
	pub meshes: HashMap<Variant, wgpu::RenderPipeline>,
	pub shadow: wgpu::RenderPipeline,
	pub background: wgpu::RenderPipeline,
}

/// What a device's renderers share.
pub(crate) struct Shared {
	pub globals_layout: wgpu::BindGroupLayout,
	pub material_layout: wgpu::BindGroupLayout,
	pub morph_layout: wgpu::BindGroupLayout,
	pub background_layout: wgpu::BindGroupLayout,
	mesh_shader: wgpu::ShaderModule,
	background_shader: wgpu::ShaderModule,
	mesh_layout: wgpu::PipelineLayout,
	shadow_layout: wgpu::PipelineLayout,
	background_pipeline_layout: wgpu::PipelineLayout,
	pub environment: wgpu::TextureView,
	pub irradiance: [[f32; 4]; 9],
	pub linear_sampler: wgpu::Sampler,
	pub white: wgpu::TextureView,
	pub flat_normal: wgpu::TextureView,
	pub no_occlusion: wgpu::TextureView,
	pub no_morphs: wgpu::BindGroup,
	pipelines: Mutex<HashMap<(wgpu::TextureFormat, u32), Arc<Pipelines>>>,
	assets: Mutex<UploadedAssets>,
}

/// Uploaded copies by the address of the asset they copy, with a weak
/// reference that tells when no scene holds it any more.
type UploadedAssets = HashMap<usize, (Weak<Asset>, Arc<GpuAsset>)>;

fn texture_entry(
	binding: u32,
	visibility: wgpu::ShaderStages,
	sample_type: wgpu::TextureSampleType,
	view_dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
	wgpu::BindGroupLayoutEntry {
		binding,
		visibility,
		ty: wgpu::BindingType::Texture {
			sample_type,
			view_dimension,
			multisampled: false,
		},
		count: None,
	}
}

fn uniform_entry(
	binding: u32,
	visibility: wgpu::ShaderStages,
	dynamic: bool,
) -> wgpu::BindGroupLayoutEntry {
	wgpu::BindGroupLayoutEntry {
		binding,
		visibility,
		ty: wgpu::BindingType::Buffer {
			ty: wgpu::BufferBindingType::Uniform,
			has_dynamic_offset: dynamic,
			min_binding_size: None,
		},
		count: None,
	}
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
	wgpu::BindGroupLayoutEntry {
		binding,
		visibility: wgpu::ShaderStages::FRAGMENT,
		ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
		count: None,
	}
}

/// A 1×1 texture of one texel.
fn pixel(
	device: &wgpu::Device,
	queue: &wgpu::Queue,
	label: &str,
	format: wgpu::TextureFormat,
	bytes: &[u8],
) -> wgpu::TextureView {
	let texture = device.create_texture(&wgpu::TextureDescriptor {
		label: Some(label),
		size: wgpu::Extent3d {
			width: 1,
			height: 1,
			depth_or_array_layers: 1,
		},
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	});
	queue.write_texture(
		texture.as_image_copy(),
		bytes,
		wgpu::TexelCopyBufferLayout {
			offset: 0,
			bytes_per_row: Some(bytes.len() as u32),
			rows_per_image: None,
		},
		wgpu::Extent3d {
			width: 1,
			height: 1,
			depth_or_array_layers: 1,
		},
	);
	texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn bake_environment(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
	let texture = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("leptoskit_3d environment"),
		size: wgpu::Extent3d {
			width: environment::SIZE,
			height: environment::SIZE,
			depth_or_array_layers: 6,
		},
		mip_level_count: environment::MIPS,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: wgpu::TextureFormat::Rgba16Float,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
		view_formats: &[],
	});

	for (mip, texels) in environment::bake_cubemap().into_iter().enumerate() {
		let size = (environment::SIZE >> mip).max(1);
		let bytes: Vec<u8> = texels
			.iter()
			.flat_map(|texel| texel.map(|value| half::f16::from_f32(value).to_bits()))
			.flat_map(u16::to_le_bytes)
			.collect();
		queue.write_texture(
			wgpu::TexelCopyTextureInfo {
				texture: &texture,
				mip_level: mip as u32,
				origin: wgpu::Origin3d::ZERO,
				aspect: wgpu::TextureAspect::All,
			},
			&bytes,
			wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(size * 8),
				rows_per_image: Some(size),
			},
			wgpu::Extent3d {
				width: size,
				height: size,
				depth_or_array_layers: 6,
			},
		);
	}

	texture.create_view(&wgpu::TextureViewDescriptor {
		label: Some("leptoskit_3d environment"),
		dimension: Some(wgpu::TextureViewDimension::Cube),
		..wgpu::TextureViewDescriptor::default()
	})
}

impl Shared {
	#[allow(clippy::too_many_lines)]
	fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
		use wgpu::ShaderStages as S;
		use wgpu::TextureSampleType as T;
		use wgpu::TextureViewDimension as D;
		let unfiltered = T::Float { filterable: false };
		let filtered = T::Float { filterable: true };
		let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("leptoskit_3d globals"),
			entries: &[
				uniform_entry(0, S::VERTEX | S::FRAGMENT, true),
				texture_entry(1, S::FRAGMENT, filtered, D::Cube),
				sampler_entry(2),
				texture_entry(3, S::VERTEX, unfiltered, D::D2),
				texture_entry(4, S::FRAGMENT, unfiltered, D::D2),
			],
		});
		let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("leptoskit_3d material"),
			entries: &[
				uniform_entry(0, S::FRAGMENT, false),
				texture_entry(1, S::FRAGMENT, filtered, D::D2),
				texture_entry(2, S::FRAGMENT, filtered, D::D2),
				texture_entry(3, S::FRAGMENT, filtered, D::D2),
				texture_entry(4, S::FRAGMENT, filtered, D::D2),
				texture_entry(5, S::FRAGMENT, filtered, D::D2),
				sampler_entry(6),
			],
		});
		let morph_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("leptoskit_3d morphs"),
			entries: &[texture_entry(0, S::VERTEX, T::Uint, D::D2)],
		});
		let background_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("leptoskit_3d background"),
			entries: &[
				uniform_entry(0, S::FRAGMENT, false),
				texture_entry(1, S::FRAGMENT, filtered, D::D2),
				texture_entry(2, S::FRAGMENT, filtered, D::D2),
				sampler_entry(3),
			],
		});
		let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("leptoskit_3d meshes"),
			bind_group_layouts: &[
				Some(&globals_layout),
				Some(&material_layout),
				Some(&morph_layout),
			],
			immediate_size: 0,
		});
		let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("leptoskit_3d shadows"),
			bind_group_layouts: &[Some(&globals_layout)],
			immediate_size: 0,
		});

		let background_pipeline_layout =
			device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
				label: Some("leptoskit_3d background"),
				bind_group_layouts: &[Some(&background_layout)],
				immediate_size: 0,
			});
		let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("leptoskit_3d meshes"),
			source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
		});
		let background_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("leptoskit_3d background"),
			source: wgpu::ShaderSource::Wgsl(include_str!("background.wgsl").into()),
		});
		let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("leptoskit_3d linear"),
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			mipmap_filter: wgpu::MipmapFilterMode::Linear,
			..wgpu::SamplerDescriptor::default()
		});
		let no_morph_texture = pixel(
			device,
			queue,
			"leptoskit_3d no morphs",
			wgpu::TextureFormat::Rgba32Uint,
			&[0; 16],
		);
		let no_morphs = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("leptoskit_3d no morphs"),
			layout: &morph_layout,
			entries: &[wgpu::BindGroupEntry {
				binding: 0,
				resource: wgpu::BindingResource::TextureView(&no_morph_texture),
			}],
		});
		Self {
			environment: bake_environment(device, queue),
			irradiance: environment::irradiance_sh(),
			white: pixel(
				device,
				queue,
				"leptoskit_3d white",
				wgpu::TextureFormat::Rgba8Unorm,
				&[255; 4],
			),
			flat_normal: pixel(
				device,
				queue,
				"leptoskit_3d flat normal",
				wgpu::TextureFormat::Rgba8Unorm,
				&[128, 128, 255, 255],
			),
			no_occlusion: pixel(
				device,
				queue,
				"leptoskit_3d no occlusion",
				wgpu::TextureFormat::R32Float,
				&0.0_f32.to_le_bytes(),
			),
			no_morphs,
			globals_layout,
			material_layout,
			morph_layout,
			background_layout,
			mesh_shader,
			background_shader,
			mesh_layout,
			shadow_layout,
			background_pipeline_layout,
			linear_sampler,
			pipelines: Mutex::new(HashMap::new()),
			assets: Mutex::new(HashMap::new()),
		}
	}

	/// The pipelines for `format` at `samples`, compiled on first use.
	pub fn pipelines(
		&self,
		device: &wgpu::Device,
		format: wgpu::TextureFormat,
		samples: u32,
	) -> Arc<Pipelines> {
		let mut pipelines = self
			.pipelines
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		pipelines
			.entry((format, samples))
			.or_insert_with(|| Arc::new(self.compile(device, format, samples)))
			.clone()
	}

	/// The device's copy of `asset`, uploaded on first use and kept while
	/// any scene holds the asset.
	pub fn asset(
		&self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		asset: &Arc<Asset>,
	) -> Arc<GpuAsset> {
		let key = Arc::as_ptr(asset) as usize;
		let mut assets = self
			.assets
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);

		if let Some((weak, uploaded)) = assets.get(&key)
			&& weak.upgrade().is_some_and(|live| Arc::ptr_eq(&live, asset))
		{
			return uploaded.clone();
		}
		// Drop copies of assets no scene holds any more.
		assets.retain(|_, (weak, _)| weak.strong_count() > 0);
		let uploaded = Arc::new(GpuAsset::new(device, queue, self, asset));
		assets.insert(key, (Arc::downgrade(asset), uploaded.clone()));
		uploaded
	}

	/// How many assets the device holds uploaded.
	pub fn asset_count(&self) -> usize {
		let assets = self
			.assets
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		assets
			.values()
			.filter(|(weak, _)| weak.strong_count() > 0)
			.count()
	}

	/// Compiles every pipeline for one target format and sample count. Each
	/// mesh variant has its own pipeline, so a draw picks one by its skin
	/// and material rather than branching in the shader.
	fn compile(
		&self,
		device: &wgpu::Device,
		format: wgpu::TextureFormat,
		samples: u32,
	) -> Pipelines {
		let multisample = wgpu::MultisampleState {
			count: samples,
			..wgpu::MultisampleState::default()
		};

		Pipelines {
			meshes: Variant::all()
				.map(|variant| {
					(
						variant,
						self.mesh_pipeline(device, format, multisample, variant),
					)
				})
				.collect(),
			shadow: self.shadow_pipeline(device, format, multisample),
			background: self.background_pipeline(device, format, multisample),
		}
	}

	fn mesh_pipeline(
		&self,
		device: &wgpu::Device,
		format: wgpu::TextureFormat,
		multisample: wgpu::MultisampleState,
		variant: Variant,
	) -> wgpu::RenderPipeline {
		let vertex =
			wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Unorm8x4];
		let instance = wgpu::vertex_attr_array![
			4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4,
			8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 11 => Uint32x4,
		];
		let skin = wgpu::vertex_attr_array![12 => Uint16x4, 13 => Unorm16x4];
		let mut buffers = vec![
			Some(buffer_layout::<crate::asset::Vertex>(
				wgpu::VertexStepMode::Vertex,
				&vertex,
			)),
			Some(buffer_layout::<crate::upload::InstanceRecord>(
				wgpu::VertexStepMode::Instance,
				&instance,
			)),
		];

		if variant.skinned {
			buffers.push(Some(buffer_layout::<crate::asset::SkinVertex>(
				wgpu::VertexStepMode::Vertex,
				&skin,
			)));
		}

		let targets = color_target(format, variant.blend);

		device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("leptoskit_3d mesh"),
			layout: Some(&self.mesh_layout),
			vertex: wgpu::VertexState {
				module: &self.mesh_shader,
				entry_point: Some(if variant.skinned {
					"vs_skinned"
				} else {
					"vs_static"
				}),

				compilation_options: wgpu::PipelineCompilationOptions::default(),
				buffers: &buffers,
			},
			primitive: wgpu::PrimitiveState {
				topology: wgpu::PrimitiveTopology::TriangleList,
				front_face: wgpu::FrontFace::Ccw,
				cull_mode: (!variant.double_sided).then_some(wgpu::Face::Back),
				..wgpu::PrimitiveState::default()
			},
			// Blended draws test depth but do not write it, so what is
			// behind them still blends in.
			depth_stencil: Some(depth_state(
				!variant.blend,
				wgpu::CompareFunction::LessEqual,
			)),
			multisample,
			fragment: Some(wgpu::FragmentState {
				module: &self.mesh_shader,
				entry_point: Some("fs_mesh"),
				compilation_options: wgpu::PipelineCompilationOptions::default(),
				targets: &targets,
			}),
			multiview_mask: None,
			cache: None,
		})
	}

	/// The soft disc under each model: one instanced quad per shadow.
	fn shadow_pipeline(
		&self,
		device: &wgpu::Device,
		format: wgpu::TextureFormat,
		multisample: wgpu::MultisampleState,
	) -> wgpu::RenderPipeline {
		let attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];
		let targets = color_target(format, true);

		device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("leptoskit_3d shadow"),
			layout: Some(&self.shadow_layout),
			vertex: wgpu::VertexState {
				module: &self.mesh_shader,
				entry_point: Some("vs_shadow"),
				compilation_options: wgpu::PipelineCompilationOptions::default(),
				buffers: &[Some(buffer_layout::<[[f32; 4]; 2]>(
					wgpu::VertexStepMode::Instance,
					&attributes,
				))],
			},
			primitive: wgpu::PrimitiveState {
				topology: wgpu::PrimitiveTopology::TriangleStrip,
				..wgpu::PrimitiveState::default()
			},
			depth_stencil: Some(depth_state(false, wgpu::CompareFunction::LessEqual)),
			multisample,
			fragment: Some(wgpu::FragmentState {
				module: &self.mesh_shader,
				entry_point: Some("fs_shadow"),
				compilation_options: wgpu::PipelineCompilationOptions::default(),
				targets: &targets,
			}),
			multiview_mask: None,
			cache: None,
		})
	}

	/// The full-screen background: a colour or the camera image, drawn
	/// behind everything.
	fn background_pipeline(
		&self,
		device: &wgpu::Device,
		format: wgpu::TextureFormat,
		multisample: wgpu::MultisampleState,
	) -> wgpu::RenderPipeline {
		let targets = color_target(format, false);

		device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("leptoskit_3d background"),
			layout: Some(&self.background_pipeline_layout),
			vertex: wgpu::VertexState {
				module: &self.background_shader,
				entry_point: Some("vs_background"),
				compilation_options: wgpu::PipelineCompilationOptions::default(),
				buffers: &[],
			},
			primitive: wgpu::PrimitiveState::default(),
			depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Always)),
			multisample,
			fragment: Some(wgpu::FragmentState {
				module: &self.background_shader,
				entry_point: Some("fs_background"),
				compilation_options: wgpu::PipelineCompilationOptions::default(),
				targets: &targets,
			}),
			multiview_mask: None,
			cache: None,
		})
	}
}

/// How a surface on `backend` composites over what is behind its view,
/// chosen from the modes it lists: premultiplied wherever it can, so a scene
/// over [`Backdrop::Transparent`](crate::protocol::Backdrop::Transparent)
/// shows the page or window behind it.
///
/// A browser's WebGPU canvas always takes `premultiplied`: the WebGPU
/// specification gives a canvas two alpha modes, and every browser has both.
/// wgpu lists only `Opaque` for it, which would draw every transparent scene
/// over black.
#[cfg(any(
	test,
	all(feature = "host", not(target_arch = "wasm32")),
	all(feature = "web", target_arch = "wasm32")
))]
pub(crate) fn surface_alpha_mode(
	backend: wgpu::Backend,
	listed: &[wgpu::CompositeAlphaMode],
) -> wgpu::CompositeAlphaMode {
	if backend == wgpu::Backend::BrowserWebGpu {
		return wgpu::CompositeAlphaMode::PreMultiplied;
	}

	[
		wgpu::CompositeAlphaMode::PreMultiplied,
		wgpu::CompositeAlphaMode::Inherit,
	]
	.into_iter()
	.find(|mode| listed.contains(mode))
	.unwrap_or(wgpu::CompositeAlphaMode::Auto)
}

#[cfg(test)]
mod tests {
	use wgpu::Backend;
	use wgpu::CompositeAlphaMode;

	use super::surface_alpha_mode;

	#[test]
	fn browser_canvases_composite_premultiplied_though_wgpu_lists_opaque() {
		assert_eq!(
			surface_alpha_mode(Backend::BrowserWebGpu, &[CompositeAlphaMode::Opaque]),
			CompositeAlphaMode::PreMultiplied
		);
	}

	#[test]
	fn other_surfaces_take_the_best_mode_they_list() {
		assert_eq!(
			surface_alpha_mode(
				Backend::Metal,
				&[
					CompositeAlphaMode::Opaque,
					CompositeAlphaMode::PreMultiplied
				]
			),
			CompositeAlphaMode::PreMultiplied
		);
		assert_eq!(
			surface_alpha_mode(
				Backend::Vulkan,
				&[CompositeAlphaMode::Opaque, CompositeAlphaMode::Inherit]
			),
			CompositeAlphaMode::Inherit
		);
		// WebGL2 lists only opaque, and wgpu refuses anything it does not list.
		assert_eq!(
			surface_alpha_mode(Backend::Gl, &[CompositeAlphaMode::Opaque]),
			CompositeAlphaMode::Auto
		);
	}
}
