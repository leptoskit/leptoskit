//! A headset session on Vulkan, shared by OpenXR and wgpu.
//!
//! OpenXR decides the Vulkan instance and device: `XR_KHR_vulkan_enable2`
//! creates both with the extensions the runtime needs on top of wgpu's. wgpu
//! then adopts them through `wgpu-hal`, and each swapchain image becomes a
//! wgpu texture with one array layer per eye.

use std::ffi::CStr;
use std::ffi::c_char;
use std::sync::Arc;
use std::sync::Mutex;

use ash::vk;
use ash::vk::Handle as _;
use leptoskit_bridge::Error;
use openxr as xr;
use wgpu::hal;

use crate::host::openxr::HeadsetSession;
use crate::host::openxr::SessionFactory;
use crate::host::openxr::SharedHeadset;
use crate::host::openxr::System;
use crate::types::*;

/// The Vulkan version asked of the runtime.
const VULKAN_VERSION: u32 = vk::make_api_version(0, 1, 1, 0);
/// The swapchain's colour format; sRGB so the compositor blends correctly.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn native<T>(_: T) -> Error {
	Error::Native
}

/// The wgpu device that draws into the headset.
#[derive(Debug, Clone)]
pub struct Gpu {
	/// The instance.
	pub instance: wgpu::Instance,
	/// The headset's GPU.
	pub adapter: wgpu::Adapter,
	/// The device OpenXR shares.
	pub device: wgpu::Device,
	/// Its queue.
	pub queue: wgpu::Queue,
}

/// A running headset session with its swapchain, for the renderer.
pub struct VulkanHeadset {
	/// The session, which the backend also updates each frame.
	pub session: Arc<Mutex<HeadsetSession<xr::Vulkan>>>,
	swapchain: xr::Swapchain<xr::Vulkan>,
	textures: Vec<wgpu::Texture>,
	size: [u32; 2],
}

impl std::fmt::Debug for VulkanHeadset {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("VulkanHeadset")
			.field("size", &self.size)
			.finish_non_exhaustive()
	}
}

impl VulkanHeadset {
	/// Each eye's size in pixels.
	pub fn size(&self) -> [u32; 2] {
		self.size
	}

	/// Renders the frame the last update began: `draw` gets one texture view
	/// per eye ([`FORMAT`], [`VulkanHeadset::size`]), in the order of the
	/// frame's views, and the views are shown with the session's passthrough
	/// beneath them. Does nothing when the runtime says not to render, and
	/// returns whether a frame was under way. The session is not locked while
	/// `draw` runs, so it may lock the runtime or a scene.
	pub fn render(&mut self, draw: impl FnOnce(&[wgpu::TextureView])) -> Result<bool, Error> {
		let lock = || {
			self.session
				.lock()
				.unwrap_or_else(std::sync::PoisonError::into_inner)
		};
		let Some(state) = lock().frame_state() else {
			return Ok(false);
		};

		if !state.should_render {
			return lock().end_frame(&[]).map(|()| true);
		}

		let index = self.swapchain.acquire_image().map_err(native)? as usize;
		self.swapchain
			.wait_image(xr::Duration::INFINITE)
			.map_err(native)?;
		let eyes: Vec<wgpu::TextureView> = (0..2)
			.map(|layer| {
				self.textures[index].create_view(&wgpu::TextureViewDescriptor {
					dimension: Some(wgpu::TextureViewDimension::D2),
					base_array_layer: layer,
					array_layer_count: Some(1),
					..wgpu::TextureViewDescriptor::default()
				})
			})
			.collect();
		draw(&eyes);
		self.swapchain.release_image().map_err(native)?;
		let mut session = self
			.session
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		self.submit(&mut session, state.predicted_display_time)
			.map(|()| true)
	}

	fn submit(
		&self,
		session: &mut HeadsetSession<xr::Vulkan>,
		time: xr::Time,
	) -> Result<(), Error> {
		let (_, views) = session
			.session()
			.locate_views(
				super::super::openxr::VIEW_CONFIGURATION,
				time,
				session.space(),
			)
			.map_err(native)?;
		let rect = xr::Rect2Di {
			offset: xr::Offset2Di { x: 0, y: 0 },
			extent: xr::Extent2Di {
				width: self.size[0] as i32,
				height: self.size[1] as i32,
			},
		};
		let projection_views: Vec<_> = views
			.iter()
			.enumerate()
			.map(|(eye, view)| {
				xr::CompositionLayerProjectionView::new()
					.pose(view.pose)
					.fov(view.fov)
					.sub_image(
						xr::SwapchainSubImage::new()
							.swapchain(&self.swapchain)
							.image_array_index(eye as u32)
							.image_rect(rect),
					)
			})
			.collect();
		session.end_projection(&projection_views)
	}
}

/// A [`SessionFactory`] for the OpenXR backend that makes the Vulkan device
/// and session, and hands each new headset and its GPU to `ready`, where the
/// renderer keeps them.
pub fn factory(ready: impl FnMut(Gpu, VulkanHeadset) + Send + 'static) -> SessionFactory {
	let mut ready = ready;

	Box::new(
		move |system: &Arc<System>, config: &SessionConfig, features: Features| {
			let (gpu, vulkan) = create_device(system)?;
			// SAFETY: The handles come from the device just made for this
			// system, and the queue family is the one it was made with.
			let (session, waiter, stream) = unsafe {
				system
					.instance
					.create_session::<xr::Vulkan>(system.system, &vulkan.session_info())
			}
			.map_err(native)?;
			let headset = HeadsetSession::new(
				Arc::clone(system),
				session.clone(),
				waiter,
				stream,
				config,
				features,
			)?;
			let headset = Arc::new(Mutex::new(headset));
			let (swapchain, textures, size) = swapchain(system, &session, &gpu)?;
			ready(
				gpu,
				VulkanHeadset {
					session: Arc::clone(&headset),
					swapchain,
					textures,
					size,
				},
			);
			Ok(headset as SharedHeadset)
		},
	)
}

/// The raw handles OpenXR made.
struct VulkanHandles {
	instance: vk::Instance,
	physical: vk::PhysicalDevice,
	device: vk::Device,
	family: u32,
}

impl VulkanHandles {
	fn session_info(&self) -> xr::vulkan::SessionCreateInfo {
		xr::vulkan::SessionCreateInfo {
			instance: self.instance.as_raw() as _,
			physical_device: self.physical.as_raw() as _,
			device: self.device.as_raw() as _,
			queue_family_index: self.family,
			queue_index: 0,
		}
	}
}

/// The loader's `vkGetInstanceProcAddr`, as OpenXR's headers spell its type.
fn proc_addr(entry: &ash::Entry) -> xr::sys::platform::VkGetInstanceProcAddr {
	// SAFETY: The same C function pointer; the two types differ only in how
	// they name the instance handle.
	unsafe {
		std::mem::transmute::<vk::PFN_vkGetInstanceProcAddr, xr::sys::platform::VkGetInstanceProcAddr>(
			entry.static_fn().get_instance_proc_addr,
		)
	}
}

/// A swapchain, its images as wgpu textures, and each eye's size.
type Swapchain = (xr::Swapchain<xr::Vulkan>, Vec<wgpu::Texture>, [u32; 2]);

fn names(extensions: &[&'static CStr]) -> Vec<*const c_char> {
	extensions.iter().map(|name| name.as_ptr()).collect()
}

/// Creates the Vulkan instance and device through OpenXR and adopts them in
/// wgpu.
fn create_device(system: &System) -> Result<(Gpu, VulkanHandles), Error> {
	let requirements = system
		.instance
		.graphics_requirements::<xr::Vulkan>(system.system)
		.map_err(native)?;

	if requirements.min_api_version_supported > xr::Version::new(1, 1, 0) {
		return Err(Error::Unavailable);
	}

	// SAFETY: Loads the system's Vulkan loader.
	let entry = unsafe { ash::Entry::load() }.map_err(|_| Error::Unavailable)?;
	let flags = wgpu::InstanceFlags::empty();
	let extensions =
		hal::vulkan::Instance::desired_extensions(&entry, VULKAN_VERSION, flags).map_err(native)?;
	let (raw_instance, instance) = create_instance(system, &entry, &extensions)?;
	let loader = entry.clone();
	// SAFETY: `instance` is the Vulkan instance OpenXR made for this system.
	let hal_instance = unsafe {
		hal::vulkan::Instance::from_raw(
			entry,
			raw_instance.clone(),
			VULKAN_VERSION,
			android_sdk_version(),
			None,
			extensions,
			flags,
			wgpu::MemoryBudgetThresholds::default(),
			false,
			None,
		)
	}
	.map_err(native)?;
	// SAFETY: Asks OpenXR which physical device drives the headset.
	let physical = vk::PhysicalDevice::from_raw(
		unsafe {
			system
				.instance
				.vulkan_graphics_device(system.system, instance.as_raw() as _)
		}
		.map_err(native)? as u64,
	);

	let adapter = hal_instance
		.expose_adapter(physical)
		.ok_or(Error::Unavailable)?;
	let (open, handles) = open_device(system, &loader, &raw_instance, &adapter, physical)?;
	// SAFETY: wgpu adopts the instance, adapter, and device made above, which
	// live as long as the wgpu objects that own them now.
	let gpu = unsafe {
		let instance = wgpu::Instance::from_hal::<hal::api::Vulkan>(hal_instance);
		let adapter = instance.create_adapter_from_hal(adapter);
		let (device, queue) = adapter
			.create_device_from_hal(open, &wgpu::DeviceDescriptor::default())
			.map_err(native)?;
		Gpu {
			instance,
			adapter,
			device,
			queue,
		}
	};

	Ok((gpu, handles))
}

fn create_instance(
	system: &System,
	entry: &ash::Entry,
	extensions: &[&'static CStr],
) -> Result<(ash::Instance, vk::Instance), Error> {
	let pointers = names(extensions);
	let application = vk::ApplicationInfo::default().api_version(VULKAN_VERSION);
	let info = vk::InstanceCreateInfo::default()
		.application_info(&application)
		.enabled_extension_names(&pointers);
	// SAFETY: OpenXR creates the instance from `info`, through the loader's
	// own `vkGetInstanceProcAddr`.
	let raw = unsafe {
		system.instance.create_vulkan_instance(
			system.system,
			proc_addr(entry),
			std::ptr::from_ref(&info).cast(),
		)
	}
	.map_err(native)?
	.map_err(|_| Error::Native)?;
	let instance = vk::Instance::from_raw(raw as u64);
	// SAFETY: `instance` was just created by the same loader.
	let loaded = unsafe { ash::Instance::load(entry.static_fn(), instance) };
	Ok((loaded, instance))
}

fn open_device(
	system: &System,
	entry: &ash::Entry,
	instance: &ash::Instance,
	adapter: &hal::ExposedAdapter<hal::api::Vulkan>,
	physical: vk::PhysicalDevice,
) -> Result<(hal::OpenDevice<hal::api::Vulkan>, VulkanHandles), Error> {
	let features = wgpu::Features::MULTIVIEW;
	let extensions = adapter.adapter.required_device_extensions(features);
	let mut physical_features = adapter
		.adapter
		.physical_device_features(&extensions, features);
	// SAFETY: A query on the headset's physical device.
	let family = unsafe { instance.get_physical_device_queue_family_properties(physical) }
		.iter()
		.position(|family| family.queue_flags.contains(vk::QueueFlags::GRAPHICS))
		.ok_or(Error::Unavailable)? as u32;
	let priorities = [1.0];
	let queues = [vk::DeviceQueueCreateInfo::default()
		.queue_family_index(family)
		.queue_priorities(&priorities)];
	let pointers = names(&extensions);
	let info = physical_features.add_to_device_create(
		vk::DeviceCreateInfo::default()
			.queue_create_infos(&queues)
			.enabled_extension_names(&pointers),
	);
	// SAFETY: OpenXR creates the device from `info` on the headset's GPU.
	let raw = unsafe {
		system.instance.create_vulkan_device(
			system.system,
			proc_addr(entry),
			physical.as_raw() as _,
			std::ptr::from_ref(&info).cast(),
		)
	}
	.map_err(native)?
	.map_err(|_| Error::Native)?;
	let device = vk::Device::from_raw(raw as u64);
	// SAFETY: `device` was just created on `instance`.
	let loaded = unsafe { ash::Device::load(instance.fp_v1_0(), device) };
	// SAFETY: wgpu-hal adopts the device with the extensions and queue it
	// was made with.
	let open = unsafe {
		adapter.adapter.device_from_raw(
			loaded,
			None,
			&extensions,
			features,
			&wgpu::Limits::default(),
			&wgpu::MemoryHints::Performance,
			family,
			0,
		)
	}
	.map_err(native)?;

	Ok((
		open,
		VulkanHandles {
			instance: instance.handle(),
			physical,
			device,
			family,
		},
	))
}

/// One swapchain with an array layer per eye, at the headset's recommended
/// size, its images adopted as wgpu textures.
fn swapchain(
	system: &System,
	session: &xr::Session<xr::Vulkan>,
	gpu: &Gpu,
) -> Result<Swapchain, Error> {
	let views = system
		.instance
		.enumerate_view_configuration_views(system.system, super::super::openxr::VIEW_CONFIGURATION)
		.map_err(native)?;
	let view = views.first().ok_or(Error::Unavailable)?;
	let size = [
		view.recommended_image_rect_width,
		view.recommended_image_rect_height,
	];
	let swapchain = session
		.create_swapchain(&xr::SwapchainCreateInfo {
			create_flags: xr::SwapchainCreateFlags::EMPTY,
			usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT
				| xr::SwapchainUsageFlags::SAMPLED,
			format: vk::Format::R8G8B8A8_SRGB.as_raw() as _,
			sample_count: 1,
			width: size[0],
			height: size[1],
			face_count: 1,
			array_size: 2,
			mip_count: 1,
		})
		.map_err(native)?;
	let descriptor = wgpu::TextureDescriptor {
		label: Some("xr swapchain"),
		size: wgpu::Extent3d {
			width: size[0],
			height: size[1],
			depth_or_array_layers: 2,
		},
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: FORMAT,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
		view_formats: &[],
	};
	let textures = swapchain
		.enumerate_images()
		.map_err(native)?
		.into_iter()
		.map(|image| adopt(gpu, image, &descriptor))
		.collect::<Result<_, Error>>()?;

	Ok((swapchain, textures, size))
}

fn adopt(
	gpu: &Gpu,
	image: u64,
	descriptor: &wgpu::TextureDescriptor<'_>,
) -> Result<wgpu::Texture, Error> {
	let hal_descriptor = hal::TextureDescriptor {
		label: descriptor.label,
		size: descriptor.size,
		mip_level_count: 1,
		sample_count: 1,
		dimension: descriptor.dimension,
		format: descriptor.format,
		usage: wgpu::TextureUses::COLOR_TARGET | wgpu::TextureUses::RESOURCE,
		memory_flags: hal::MemoryFlags::empty(),
		view_formats: Vec::new(),
	};
	// SAFETY: The image belongs to the swapchain, which outlives the
	// texture; wgpu doesn't free an image it was handed without memory.
	unsafe {
		let device = gpu
			.device
			.as_hal::<hal::api::Vulkan>()
			.ok_or(Error::Unavailable)?;
		let texture = device.texture_from_raw(
			vk::Image::from_raw(image),
			&hal_descriptor,
			None,
			hal::vulkan::TextureMemory::External,
		);
		drop(device);
		Ok(gpu.device.create_texture_from_hal::<hal::api::Vulkan>(
			texture,
			descriptor,
			wgpu::TextureUses::UNINITIALIZED,
		))
	}
}

/// Android's API level, which wgpu reads to work around old drivers.
fn android_sdk_version() -> u32 {
	#[cfg(target_os = "android")]
	{
		let mut value = [0_u8; 92];
		// SAFETY: `__system_property_get` writes at most 92 bytes.
		let length = unsafe {
			unsafe extern "C" {
				fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
			}
			__system_property_get(c"ro.build.version.sdk".as_ptr(), value.as_mut_ptr().cast())
		};
		std::str::from_utf8(&value[..length.max(0) as usize])
			.ok()
			.and_then(|text| text.parse().ok())
			.unwrap_or(0)
	}
	#[cfg(not(target_os = "android"))]
	0
}
