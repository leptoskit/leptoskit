//! The camera picture the model reads: its pixels, which way is up, and the
//! lens that took it.

use std::sync::Arc;

use crate::Error;

/// How wide a phone's main camera sees across its longer side, in radians,
/// when a picture comes without [`Intrinsics`]: 68°, a 26 mm-equivalent lens
/// on a 4:3 sensor, which is what most phones' main cameras are.
pub const ASSUMED_LONG_SIDE_FOV: f32 = 68.0 * std::f32::consts::PI / 180.0;

/// One plane of pixels in CPU memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plane {
	/// Width in pixels.
	pub width: u32,
	/// Height in pixels.
	pub height: u32,
	/// Bytes from one row to the next.
	pub row_stride: u32,
	/// The pixels, `row_stride × height` bytes or more.
	pub data: Arc<[u8]>,
}

impl Plane {
	/// A plane whose rows follow one another with no padding.
	#[must_use]
	pub fn packed(
		width: u32,
		height: u32,
		bytes_per_pixel: u32,
		data: impl Into<Arc<[u8]>>,
	) -> Self {
		Self {
			width,
			height,
			row_stride: width * bytes_per_pixel,
			data: data.into(),
		}
	}

	/// Whether `data` holds every row.
	#[must_use]
	pub fn is_complete(&self, bytes_per_pixel: u32) -> bool {
		u64::from(self.row_stride) >= u64::from(self.width) * u64::from(bytes_per_pixel)
			&& self.data.len() as u64 >= u64::from(self.row_stride) * u64::from(self.height)
	}

	/// The byte at column `x`, row `y`, `channel` bytes into the pixel.
	fn byte(&self, x: u32, y: u32, bytes_per_pixel: u32, channel: u32) -> f32 {
		let at = y as usize * self.row_stride as usize + (x * bytes_per_pixel + channel) as usize;
		f32::from(self.data[at])
	}
}

/// The picture's pixels, in the layouts cameras and AR platforms hand over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pixels {
	/// sRGB, three bytes a pixel: red, green, blue.
	Rgb8(Plane),
	/// sRGB, four bytes a pixel: red, green, blue, alpha.
	Rgba8(Plane),
	/// sRGB, four bytes a pixel: blue, green, red, alpha (iOS's
	/// `kCVPixelFormatType_32BGRA`).
	Bgra8(Plane),
	/// Full-resolution luma and half-resolution interleaved chroma, Cb then
	/// Cr (NV12, as ARKit captures and ARCore's CPU image can be read), in
	/// BT.601 full range.
	YCbCr {
		/// Luma, one byte per pixel.
		y: Plane,
		/// Chroma, two bytes per pixel at half the width and height.
		cbcr: Plane,
	},
}

impl Pixels {
	/// The picture's width and height in pixels.
	#[must_use]
	pub fn size(&self) -> (u32, u32) {
		match self {
			Self::Rgb8(plane)
			| Self::Rgba8(plane)
			| Self::Bgra8(plane)
			| Self::YCbCr { y: plane, .. } => (plane.width, plane.height),
		}
	}

	/// Fails when a plane is shorter than its size and stride say, or the
	/// picture is empty.
	pub(crate) fn check(&self) -> Result<(), Error> {
		let complete = match self {
			Self::Rgb8(plane) => plane.is_complete(3),
			Self::Rgba8(plane) | Self::Bgra8(plane) => plane.is_complete(4),
			Self::YCbCr { y, cbcr } => {
				y.is_complete(1)
					&& cbcr.is_complete(2)
					&& cbcr.width >= y.width.div_ceil(2)
					&& cbcr.height >= y.height.div_ceil(2)
			}
		};
		let (width, height) = self.size();

		if width == 0 || height == 0 {
			Err(Error::Picture("it has no pixels"))
		} else if complete {
			Ok(())
		} else {
			Err(Error::Picture(
				"a plane is shorter than its size and stride say",
			))
		}
	}

	/// Red, green, and blue from 0 to 255 at column `x`, row `y`, which
	/// [`Pixels::check`] has made safe to read.
	pub(crate) fn rgb(&self, x: u32, y: u32) -> [f32; 3] {
		match self {
			Self::Rgb8(plane) => [0, 1, 2].map(|channel| plane.byte(x, y, 3, channel)),
			Self::Rgba8(plane) => [0, 1, 2].map(|channel| plane.byte(x, y, 4, channel)),
			Self::Bgra8(plane) => [2, 1, 0].map(|channel| plane.byte(x, y, 4, channel)),
			Self::YCbCr { y: luma, cbcr } => {
				let brightness = luma.byte(x, y, 1, 0);
				let cb = cbcr.byte(x / 2, y / 2, 2, 0) - 128.0;
				let cr = cbcr.byte(x / 2, y / 2, 2, 1) - 128.0;
				[
					brightness + 1.402 * cr,
					brightness - 0.344_136 * cb - 0.714_136 * cr,
					brightness + 1.772 * cb,
				]
				.map(|value| value.clamp(0.0, 255.0))
			}
		}
	}
}

/// Which way a picture must turn to stand upright, with the floor at the
/// bottom.
///
/// The model was trained on upright pictures, so a picture is turned before
/// it reads it, and the depth is turned back: a [`DepthMap`](crate::DepthMap)
/// always lies the way its picture did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
	/// Already upright.
	#[default]
	Upright,
	/// Upright after a quarter turn clockwise, as a phone's back camera reads
	/// with the phone held in portrait (Android's sensor orientation of 90°).
	Clockwise,
	/// Upside down.
	UpsideDown,
	/// Upright after a quarter turn counterclockwise (a sensor orientation of
	/// 270°, as most front cameras read in portrait).
	Counterclockwise,
}

impl Orientation {
	/// The orientation that clockwise quarter turns make upright, any number
	/// of whole turns included.
	#[must_use]
	pub fn from_quarter_turns(turns: u32) -> Self {
		match turns % 4 {
			0 => Self::Upright,
			1 => Self::Clockwise,
			2 => Self::UpsideDown,
			_ => Self::Counterclockwise,
		}
	}

	/// The clockwise quarter turns that make the picture upright, 0 to 3.
	#[must_use]
	pub fn quarter_turns(self) -> u32 {
		match self {
			Self::Upright => 0,
			Self::Clockwise => 1,
			Self::UpsideDown => 2,
			Self::Counterclockwise => 3,
		}
	}

	/// The orientation in which `(dx, dy)`, a direction in the picture's
	/// pixels with +y down, points at the floor. The nearest quarter turn
	/// wins.
	#[must_use]
	pub fn from_down(dx: f32, dy: f32) -> Self {
		if dy.abs() >= dx.abs() {
			if dy >= 0.0 {
				Self::Upright
			} else {
				Self::UpsideDown
			}
		} else if dx > 0.0 {
			Self::Clockwise
		} else {
			Self::Counterclockwise
		}
	}

	/// The orientation of a camera picture shown upright on a screen through
	/// `transform`, an affine `[a, b, c, d, tx, ty]` from a point on the
	/// view, `(0, 0)` top-left to `(1, 1)` bottom-right, to the point in the
	/// picture to sample, as Core Graphics writes it and `leptoskit_xr`'s
	/// display transforms are. The view's down is the picture's `(c, d)`.
	#[must_use]
	pub fn from_display_transform(transform: &[f32; 6]) -> Self {
		Self::from_down(transform[2], transform[3])
	}

	/// Whether the picture's width runs up and down once it is upright.
	#[must_use]
	pub fn is_sideways(self) -> bool {
		matches!(self, Self::Clockwise | Self::Counterclockwise)
	}

	/// The size of a `width` by `height` picture once it is upright.
	#[must_use]
	pub fn upright_size(self, width: u32, height: u32) -> (u32, u32) {
		if self.is_sideways() {
			(height, width)
		} else {
			(width, height)
		}
	}

	/// Where a point of the upright picture lies in the picture as it is, in
	/// continuous pixel coordinates (`(0, 0)` is the top-left corner of the
	/// top-left pixel). `width` and `height` are the picture's own.
	#[must_use]
	pub fn to_picture(self, width: f32, height: f32, (x, y): (f32, f32)) -> (f32, f32) {
		match self {
			Self::Upright => (x, y),
			Self::Clockwise => (y, height - x),
			Self::UpsideDown => (width - x, height - y),
			Self::Counterclockwise => (width - y, x),
		}
	}

	/// Where a point of the picture as it is lies once it is upright: the
	/// inverse of [`Orientation::to_picture`].
	#[must_use]
	pub fn to_upright(self, width: f32, height: f32, (x, y): (f32, f32)) -> (f32, f32) {
		match self {
			Self::Upright => (x, y),
			Self::Clockwise => (height - y, x),
			Self::UpsideDown => (width - x, height - y),
			Self::Counterclockwise => (y, width - x),
		}
	}
}

/// A pinhole camera's projection, in pixels of the picture it describes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Intrinsics {
	/// Focal lengths along x and y, in pixels.
	pub focal: [f32; 2],
	/// Where the optical axis meets the picture, in pixels from its top-left
	/// corner.
	pub principal: [f32; 2],
	/// The picture's width and height in pixels.
	pub size: [u32; 2],
}

impl Intrinsics {
	/// A centred lens that sees `long_side_fov` radians across the longer
	/// side of a `width` by `height` picture, with square pixels.
	#[must_use]
	pub fn from_fov(width: u32, height: u32, long_side_fov: f32) -> Self {
		let long = width.max(height).max(1) as f32;
		let focal = long / 2.0 / (long_side_fov / 2.0).tan();

		Self {
			focal: [focal, focal],
			principal: [width as f32 / 2.0, height as f32 / 2.0],
			size: [width, height],
		}
	}

	/// What a phone's main camera most likely is, for a picture that came
	/// without intrinsics: see [`ASSUMED_LONG_SIDE_FOV`].
	#[must_use]
	pub fn assumed(width: u32, height: u32) -> Self {
		Self::from_fov(width, height, ASSUMED_LONG_SIDE_FOV)
	}

	/// The same camera described for the picture resized to `width` by
	/// `height`.
	#[must_use]
	pub fn scaled(self, width: u32, height: u32) -> Self {
		let sx = width as f32 / self.size[0].max(1) as f32;
		let sy = height as f32 / self.size[1].max(1) as f32;

		Self {
			focal: [self.focal[0] * sx, self.focal[1] * sy],
			principal: [self.principal[0] * sx, self.principal[1] * sy],
			size: [width, height],
		}
	}
}

/// One camera picture to read depth from.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
	/// The pixels, as the camera read them.
	pub pixels: Pixels,
	/// Which way the pixels must turn to stand upright.
	pub orientation: Orientation,
	/// The lens, in these pixels. Without it the model's metres are scaled
	/// for [`Intrinsics::assumed`], and can be a fifth out.
	pub intrinsics: Option<Intrinsics>,
}

impl Picture {
	/// An upright picture of unknown lens.
	#[must_use]
	pub fn new(pixels: Pixels) -> Self {
		Self {
			pixels,
			orientation: Orientation::Upright,
			intrinsics: None,
		}
	}

	/// The same picture, turned `orientation` from upright.
	#[must_use]
	pub fn with_orientation(mut self, orientation: Orientation) -> Self {
		self.orientation = orientation;
		self
	}

	/// The same picture, taken through `intrinsics`.
	#[must_use]
	pub fn with_intrinsics(mut self, intrinsics: Intrinsics) -> Self {
		self.intrinsics = Some(intrinsics);
		self
	}

	/// Width and height in pixels, as the camera read them.
	#[must_use]
	pub fn size(&self) -> (u32, u32) {
		self.pixels.size()
	}

	/// The lens, given or assumed, described for this picture's own size.
	#[must_use]
	pub fn lens(&self) -> Intrinsics {
		let (width, height) = self.size();

		match self.intrinsics {
			Some(intrinsics) if intrinsics.size == [width, height] => intrinsics,
			Some(intrinsics) => intrinsics.scaled(width, height),
			None => Intrinsics::assumed(width, height),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn turning_upright_and_back_lands_where_it_started() {
		for turns in 0..4 {
			let orientation = Orientation::from_quarter_turns(turns);
			assert_eq!(orientation.quarter_turns(), turns);
			let (width, height) = (640.0, 480.0);
			let point = (123.5, 45.25);
			let upright = orientation.to_upright(width, height, point);
			let back = orientation.to_picture(width, height, upright);
			assert!((back.0 - point.0).abs() < 1e-4 && (back.1 - point.1).abs() < 1e-4);
		}
	}

	#[test]
	fn a_clockwise_picture_keeps_its_left_edge_at_the_top() {
		// A landscape sensor reading a portrait scene: its left edge is the
		// scene's top, so the upright picture's top-left corner is the
		// sensor's bottom-left.
		let corner = Orientation::Clockwise.to_picture(640.0, 480.0, (0.0, 0.0));
		assert_eq!(corner, (0.0, 480.0));
		assert_eq!(Orientation::Clockwise.upright_size(640, 480), (480, 640));
	}

	#[test]
	fn the_floor_s_direction_names_the_turn() {
		assert_eq!(Orientation::from_down(0.0, 1.0), Orientation::Upright);
		assert_eq!(Orientation::from_down(1.0, 0.1), Orientation::Clockwise);
		assert_eq!(Orientation::from_down(0.1, -1.0), Orientation::UpsideDown);
		assert_eq!(
			Orientation::from_down(-1.0, 0.0),
			Orientation::Counterclockwise
		);

		// A quarter-turned display transform (view down is the picture's +x)
		// is a clockwise picture: the same affine `leptoskit_xr` writes for a
		// 90° sensor on a portrait screen.
		let transform = [0.0, -1.0, 1.0, 0.0, 0.0, 1.0];
		assert_eq!(
			Orientation::from_display_transform(&transform),
			Orientation::Clockwise
		);
	}

	#[test]
	fn every_layout_reads_the_same_colour() {
		let rgb = Pixels::Rgb8(Plane::packed(1, 1, 3, vec![200, 100, 50]));
		let rgba = Pixels::Rgba8(Plane::packed(1, 1, 4, vec![200, 100, 50, 255]));
		let bgra = Pixels::Bgra8(Plane::packed(1, 1, 4, vec![50, 100, 200, 255]));
		for pixels in [&rgb, &rgba, &bgra] {
			pixels.check().unwrap();
			assert_eq!(pixels.rgb(0, 0), [200.0, 100.0, 50.0]);
		}

		// Grey has no chroma.
		let grey = Pixels::YCbCr {
			y: Plane::packed(2, 2, 1, vec![90; 4]),
			cbcr: Plane::packed(1, 1, 2, vec![128, 128]),
		};
		grey.check().unwrap();
		assert_eq!(grey.rgb(1, 1), [90.0; 3]);
	}

	#[test]
	fn short_planes_are_refused() {
		let short = Pixels::Rgb8(Plane::packed(2, 2, 3, vec![0; 11]));
		assert!(short.check().is_err());
		let no_chroma = Pixels::YCbCr {
			y: Plane::packed(4, 4, 1, vec![0; 16]),
			cbcr: Plane::packed(1, 1, 2, vec![0; 2]),
		};
		assert!(no_chroma.check().is_err());
		assert!(
			Pixels::Rgb8(Plane::packed(0, 0, 3, Vec::new()))
				.check()
				.is_err()
		);
	}

	#[test]
	fn a_lens_follows_its_picture_s_size() {
		let lens = Intrinsics::from_fov(800, 600, std::f32::consts::FRAC_PI_2);
		assert!((lens.focal[0] - 400.0).abs() < 1e-3);
		let picture = Picture::new(Pixels::Rgb8(Plane::packed(400, 300, 3, vec![0; 360_000])))
			.with_intrinsics(lens);
		assert!((picture.lens().focal[0] - 200.0).abs() < 1e-3);
		assert_eq!(picture.lens().principal, [200.0, 150.0]);
	}
}
