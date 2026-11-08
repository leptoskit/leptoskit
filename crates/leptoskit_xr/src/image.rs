//! The pictures a frame carries on the host: the camera image the renderer
//! draws behind the scene, and the depth picture it uses for occlusion and
//! hit tests. Neither crosses to the guest.

use std::sync::Arc;

/// A 2D affine transform `[a, b, c, d, tx, ty]`, mapping `(u, v)` to
/// `(a·u + c·v + tx, b·u + d·v + ty)`, as Core Graphics writes it.
pub type Affine2 = [f32; 6];

/// The transform that changes nothing.
pub const IDENTITY_AFFINE: Affine2 = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Applies an [`Affine2`] to a point.
#[must_use]
pub fn apply_affine(transform: &Affine2, u: f32, v: f32) -> (f32, f32) {
	let [xx, xy, yx, yy, tx, ty] = *transform;

	(xx * u + yx * v + tx, xy * u + yy * v + ty)
}

/// One plane of pixels in CPU memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlane {
	/// Width in pixels.
	pub width: u32,
	/// Height in pixels.
	pub height: u32,
	/// Bytes from one row to the next.
	pub row_stride: u32,
	/// The pixels, `row_stride × height` bytes or more.
	pub data: Arc<[u8]>,
}

impl ImagePlane {
	/// Whether `data` holds every row.
	#[must_use]
	pub fn is_complete(&self, bytes_per_pixel: u32) -> bool {
		self.row_stride >= self.width.saturating_mul(bytes_per_pixel)
			&& self.data.len() as u64 >= u64::from(self.row_stride) * u64::from(self.height)
	}
}

/// The camera picture's pixels.
#[derive(Debug, Clone)]
pub enum CameraPixels {
	/// Full-resolution luma and half-resolution interleaved chroma (NV12, as
	/// ARKit captures and ARCore's CPU image can be read). The renderer
	/// uploads them as `R8Unorm` and `Rg8Unorm` textures and converts with
	/// BT.601 full-range coefficients.
	YCbCr {
		/// Luma, one byte per pixel.
		y: ImagePlane,
		/// Chroma, two bytes per pixel: Cb then Cr.
		cbcr: ImagePlane,
	},
	/// sRGB RGBA, four bytes per pixel.
	Rgba(ImagePlane),
	/// A platform image the renderer imports without copying.
	Native(NativeImage),
}

/// A camera image that stays in the platform's own memory, for a renderer
/// that imports it without a copy.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum NativeImage {}

/// The camera picture for one frame.
#[derive(Debug, Clone)]
pub struct CameraImage {
	/// Width of the captured image in pixels.
	pub width: u32,
	/// Height of the captured image in pixels.
	pub height: u32,
	/// Maps a point on the view, `(0, 0)` top-left to `(1, 1)` bottom-right,
	/// to the point in the image to sample. It undoes the sensor's
	/// orientation and crops the image to the view's aspect ratio.
	pub display_transform: Affine2,
	/// The pixels.
	pub pixels: CameraPixels,
}

/// How a depth picture stores distances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepthFormat {
	/// 32-bit floats in metres (ARKit).
	Meters32,
	/// 16-bit unsigned millimetres (ARCore).
	Millimeters16,
}

impl DepthFormat {
	/// Bytes per pixel.
	#[must_use]
	pub fn bytes(self) -> u32 {
		match self {
			Self::Meters32 => 4,
			Self::Millimeters16 => 2,
		}
	}
}

/// How far the real world is from the camera, pixel by pixel.
#[derive(Debug, Clone)]
pub struct DepthImage {
	/// Width in pixels.
	pub width: u32,
	/// Height in pixels.
	pub height: u32,
	/// How distances are stored.
	pub format: DepthFormat,
	/// The distances, along the camera's forward axis (not along each ray),
	/// in rows of `row_stride` bytes. Zero means unknown.
	pub data: Arc<[u8]>,
	/// Bytes from one row to the next.
	pub row_stride: u32,
	/// How sure the platform is of each pixel, one byte per pixel at the same
	/// size, where 255 is certain. ARKit's three levels map to 0, 128 and 255.
	pub confidence: Option<ImagePlane>,
	/// Maps a point on the view, `(0, 0)` top-left to `(1, 1)` bottom-right,
	/// to the point in this picture to sample.
	pub display_transform: Affine2,
}

impl DepthImage {
	/// The distance in metres at a point in the picture, `(0, 0)` top-left
	/// to `(1, 1)` bottom-right, or `None` where it is unknown or outside.
	#[must_use]
	pub fn sample(&self, u: f32, v: f32) -> Option<f32> {
		if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
			return None;
		}

		let x = ((u * self.width as f32) as u32).min(self.width.checked_sub(1)?);
		let y = ((v * self.height as f32) as u32).min(self.height.checked_sub(1)?);
		let offset = (y as usize)
			.checked_mul(self.row_stride as usize)?
			.checked_add(x as usize * self.format.bytes() as usize)?;
		let meters = match self.format {
			DepthFormat::Meters32 => {
				f32::from_le_bytes(self.data.get(offset..offset + 4)?.try_into().ok()?)
			}
			DepthFormat::Millimeters16 => {
				f32::from(u16::from_le_bytes(
					self.data.get(offset..offset + 2)?.try_into().ok()?,
				)) / 1000.0
			}
		};

		(meters.is_finite() && meters > 0.0).then_some(meters)
	}

	/// The distance in metres at a point on the view, `(0, 0)` top-left to
	/// `(1, 1)` bottom-right.
	#[must_use]
	pub fn sample_view(&self, x: f32, y: f32) -> Option<f32> {
		let (u, v) = apply_affine(&self.display_transform, x, y);
		self.sample(u, v)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn depth_samples_both_formats_and_rejects_unknowns() {
		let meters: Vec<u8> = [1.5_f32, 0.0, 2.0, 3.0]
			.iter()
			.flat_map(|value| value.to_le_bytes())
			.collect();
		let image = DepthImage {
			width: 2,
			height: 2,
			format: DepthFormat::Meters32,
			data: meters.into(),
			row_stride: 8,
			confidence: None,
			display_transform: IDENTITY_AFFINE,
		};
		assert_eq!(image.sample(0.1, 0.1), Some(1.5));
		assert_eq!(image.sample(0.9, 0.1), None);
		assert_eq!(image.sample(0.9, 0.9), Some(3.0));
		assert_eq!(image.sample(1.5, 0.0), None);

		let millimeters: Vec<u8> = [1250_u16].iter().flat_map(|v| v.to_le_bytes()).collect();
		let image = DepthImage {
			width: 1,
			height: 1,
			format: DepthFormat::Millimeters16,
			data: millimeters.into(),
			row_stride: 2,
			confidence: None,
			// A view rotated a quarter turn from the sensor still lands inside.
			display_transform: [0.0, 1.0, -1.0, 0.0, 1.0, 0.0],
		};
		assert_eq!(image.sample_view(0.5, 0.5), Some(1.25));
	}
}
