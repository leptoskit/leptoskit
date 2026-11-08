//! What a wgpu renderer needs from a frame: the camera and depth pictures as
//! textures, and each view's matrices.
//!
//! [`FrameTextures`] uploads the pictures a backend left in CPU memory. It
//! works on every wgpu backend, OpenGL ES included, which is what Android
//! renders with by default. ARKit's `CVPixelBuffer` is also copied for now;
//! wrapping it in Metal textures through a `CVMetalTextureCache` is the
//! zero-copy path (see `docs/agents/xr.md`).
//!
//! With the `openxr` feature, [`openxr_vulkan`] creates the Vulkan device a
//! headset session needs and renders into its swapchains.

use super::XrFrame;
use crate::image::*;
use crate::types::View;

#[cfg(all(
	feature = "openxr",
	any(
		target_os = "android",
		target_os = "windows",
		all(target_os = "linux", not(target_env = "ohos"))
	)
))]
#[allow(unsafe_code)]
pub mod openxr_vulkan;

/// A view's matrices, column-major: world to view, and view to clip with
/// depth from 0 at `near` to 1 at `far`.
#[must_use]
pub fn view_matrices(view: &View, near: f32, far: f32) -> ([f32; 16], [f32; 16]) {
	(view.view_matrix(), view.projection_matrix(near, far))
}

/// An [`Affine2`] as the column-major 3 × 3 matrix renderers multiply
/// `[x, y, 1]` by.
#[must_use]
pub fn affine_matrix(transform: &Affine2) -> [f32; 9] {
	let [xx, xy, yx, yy, tx, ty] = *transform;
	[xx, xy, 0.0, yx, yy, 0.0, tx, ty, 1.0]
}

/// One texture this module keeps, recreated when the picture's size or
/// format changes.
#[derive(Debug)]
struct Slot {
	texture: wgpu::Texture,
	view: wgpu::TextureView,
}

impl Slot {
	fn ensure<'a>(
		slot: &'a mut Option<Self>,
		device: &wgpu::Device,
		label: &'static str,
		format: wgpu::TextureFormat,
		size: (u32, u32),
	) -> &'a Self {
		let fits = slot.as_ref().is_some_and(|slot| {
			slot.texture.format() == format
				&& slot.texture.width() == size.0
				&& slot.texture.height() == size.1
		});

		if !fits {
			let texture = device.create_texture(&wgpu::TextureDescriptor {
				label: Some(label),
				size: wgpu::Extent3d {
					width: size.0.max(1),
					height: size.1.max(1),
					depth_or_array_layers: 1,
				},
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format,
				usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
				view_formats: &[],
			});
			let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
			*slot = Some(Self { texture, view });
		}

		slot.as_ref().expect("set above")
	}

	fn write(&self, queue: &wgpu::Queue, data: &[u8], row_stride: u32) {
		queue.write_texture(
			wgpu::TexelCopyTextureInfo {
				texture: &self.texture,
				mip_level: 0,
				origin: wgpu::Origin3d::ZERO,
				aspect: wgpu::TextureAspect::All,
			},
			data,
			wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(row_stride),
				rows_per_image: None,
			},
			self.texture.size(),
		);
	}
}

/// The camera picture as textures.
#[derive(Debug, Clone, Copy)]
pub enum CameraViews<'a> {
	/// Luma (`R8Unorm`) and chroma (`Rg8Unorm`), full-range BT.601.
	YCbCr {
		/// The luma plane.
		y: &'a wgpu::TextureView,
		/// The chroma plane.
		cbcr: &'a wgpu::TextureView,
	},
	/// sRGB RGBA (`Rgba8UnormSrgb`).
	Rgba(&'a wgpu::TextureView),
}

/// Keeps a frame's pictures on the GPU, reusing the textures from frame to
/// frame.
#[derive(Debug, Default)]
pub struct FrameTextures {
	luma: Option<Slot>,
	chroma: Option<Slot>,
	rgba: Option<Slot>,
	depth: Option<Slot>,
	camera: Option<(bool, Affine2)>,
	depth_transform: Option<Affine2>,
}

impl FrameTextures {
	/// No textures yet.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Uploads `frame`'s camera and depth pictures. Missing pictures leave
	/// [`FrameTextures::camera`] or [`FrameTextures::depth`] empty.
	pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &XrFrame) {
		self.camera = frame
			.camera_image
			.as_ref()
			.and_then(|image| self.upload_camera(device, queue, image));
		self.depth_transform = frame
			.depth
			.as_ref()
			.and_then(|depth| self.upload_depth(device, queue, depth));
	}

	fn upload_camera(
		&mut self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		image: &CameraImage,
	) -> Option<(bool, Affine2)> {
		match &image.pixels {
			CameraPixels::YCbCr { y, cbcr } if y.is_complete(1) && cbcr.is_complete(2) => {
				let size = (y.width, y.height);
				Slot::ensure(
					&mut self.luma,
					device,
					"xr camera luma",
					wgpu::TextureFormat::R8Unorm,
					size,
				)
				.write(queue, &y.data, y.row_stride);
				let size = (cbcr.width, cbcr.height);
				Slot::ensure(
					&mut self.chroma,
					device,
					"xr camera chroma",
					wgpu::TextureFormat::Rg8Unorm,
					size,
				)
				.write(queue, &cbcr.data, cbcr.row_stride);
				Some((true, image.display_transform))
			}

			CameraPixels::Rgba(rgba) if rgba.is_complete(4) => {
				let size = (rgba.width, rgba.height);
				Slot::ensure(
					&mut self.rgba,
					device,
					"xr camera",
					wgpu::TextureFormat::Rgba8UnormSrgb,
					size,
				)
				.write(queue, &rgba.data, rgba.row_stride);
				Some((false, image.display_transform))
			}

			#[cfg(all(target_os = "ios", target_arch = "aarch64"))]
			CameraPixels::Native(NativeImage::PixelBuffer(buffer)) => {
				let (y, cbcr) = buffer.planes()?;
				self.upload_camera(
					device,
					queue,
					&CameraImage {
						pixels: CameraPixels::YCbCr { y, cbcr },
						..image.clone()
					},
				)
			}
			_ => None,
		}
	}

	/// Uploads depth as `R32Float` metres, converting millimetres.
	fn upload_depth(
		&mut self,
		device: &wgpu::Device,
		queue: &wgpu::Queue,
		depth: &DepthImage,
	) -> Option<Affine2> {
		let plane = ImagePlane {
			width: depth.width,
			height: depth.height,
			row_stride: depth.row_stride,
			data: depth.data.clone(),
		};

		if !plane.is_complete(depth.format.bytes()) {
			return None;
		}

		let slot = Slot::ensure(
			&mut self.depth,
			device,
			"xr depth",
			wgpu::TextureFormat::R32Float,
			(depth.width, depth.height),
		);

		match depth.format {
			DepthFormat::Meters32 => slot.write(queue, &depth.data, depth.row_stride),
			DepthFormat::Millimeters16 => {
				let meters: Vec<u8> = (0..depth.height as usize)
					.flat_map(|row| {
						let start = row * depth.row_stride as usize;
						depth.data[start..start + depth.width as usize * 2]
							.as_chunks::<2>()
							.0
							.iter()
							.flat_map(|raw| {
								(f32::from(u16::from_le_bytes(*raw)) / 1000.0).to_le_bytes()
							})
							.collect::<Vec<u8>>()
					})
					.collect();
				slot.write(queue, &meters, depth.width * 4);
			}
		}

		Some(depth.display_transform)
	}

	/// The camera picture uploaded last, and the transform from the view to
	/// its texture coordinates.
	#[must_use]
	pub fn camera(&self) -> Option<(CameraViews<'_>, Affine2)> {
		let (ycbcr, transform) = self.camera?;
		let views = if ycbcr {
			CameraViews::YCbCr {
				y: &self.luma.as_ref()?.view,
				cbcr: &self.chroma.as_ref()?.view,
			}
		} else {
			CameraViews::Rgba(&self.rgba.as_ref()?.view)
		};

		Some((views, transform))
	}

	/// The depth picture uploaded last (`R32Float` metres, zero unknown), and
	/// the transform from the view to its texture coordinates.
	#[must_use]
	pub fn depth(&self) -> Option<(&wgpu::TextureView, Affine2)> {
		Some((&self.depth.as_ref()?.view, self.depth_transform?))
	}
}
