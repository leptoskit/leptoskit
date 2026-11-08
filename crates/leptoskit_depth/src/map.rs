//! What the model says: a depth picture in metres over the camera picture.

use std::time::Duration;

use crate::Intrinsics;

/// How far away the real world is, pixel by pixel, over a camera picture.
///
/// It lies the way its picture did (not turned upright) and covers the whole
/// picture, at the model's resolution: a point `(u, v)` of the picture,
/// `(0, 0)` top-left to `(1, 1)` bottom-right, is the same point here, so the
/// picture's display transform serves both. Where the model did not look
/// (the picture's sides or ends, cropped to the model's shape) the depth is
/// unknown.
#[derive(Debug, Clone, PartialEq)]
pub struct DepthMap {
	/// Width in pixels.
	pub width: u32,
	/// Height in pixels.
	pub height: u32,
	/// Metres along the camera's forward axis (not along each ray), in rows
	/// from the top. Zero means unknown.
	pub depths: Vec<f32>,
	/// The camera's lens, described for this grid's pixels.
	pub intrinsics: Intrinsics,
	/// The factor the model's own metres were divided by for the lens: see
	/// [`overreach`](crate::overreach).
	pub scale: f32,
	/// How long reading the picture took.
	pub took: Duration,
}

impl DepthMap {
	/// The depth in metres at pixel `(x, y)`, or `None` where it is unknown
	/// or outside.
	#[must_use]
	pub fn get(&self, x: u32, y: u32) -> Option<f32> {
		if x >= self.width || y >= self.height {
			return None;
		}

		let depth = self.depths[y as usize * self.width as usize + x as usize];
		(depth.is_finite() && depth > 0.0).then_some(depth)
	}

	/// The depth in metres at a point, `(0, 0)` top-left to `(1, 1)`
	/// bottom-right, from the pixel it falls in, or `None` where it is
	/// unknown or outside.
	#[must_use]
	pub fn sample(&self, u: f32, v: f32) -> Option<f32> {
		if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
			return None;
		}

		let x = ((u * self.width as f32) as u32).min(self.width.saturating_sub(1));
		let y = ((v * self.height as f32) as u32).min(self.height.saturating_sub(1));
		self.get(x, y)
	}

	/// The point the map shows at `(u, v)`, in metres in the camera's own
	/// space: +x along the picture's rows, +y down its columns, +z the way
	/// the camera looks (the pinhole convention).
	#[must_use]
	pub fn point(&self, u: f32, v: f32) -> Option<[f32; 3]> {
		let depth = self.sample(u, v)?;
		let [focal_x, focal_y] = self.intrinsics.focal;
		let [centre_x, centre_y] = self.intrinsics.principal;
		let x = (u * self.width as f32 - centre_x) / focal_x;
		let y = (v * self.height as f32 - centre_y) / focal_y;

		Some([x * depth, y * depth, depth])
	}

	/// How far from the camera the point at `(u, v)` is, along its own ray
	/// rather than the forward axis.
	#[must_use]
	pub fn distance(&self, u: f32, v: f32) -> Option<f32> {
		let [right, down, ahead] = self.point(u, v)?;
		Some((right * right + down * down + ahead * ahead).sqrt())
	}

	/// The share of the picture whose depth is known, from 0 to 1.
	#[must_use]
	pub fn coverage(&self) -> f32 {
		let known = self.depths.iter().filter(|depth| **depth > 0.0).count();
		known as f32 / self.depths.len().max(1) as f32
	}

	/// The depths as little-endian 32-bit floats in rows of `width × 4`
	/// bytes, the layout of `leptoskit_xr`'s `DepthFormat::Meters32`.
	#[must_use]
	pub fn to_le_bytes(&self) -> Vec<u8> {
		self.depths
			.iter()
			.flat_map(|depth| depth.to_le_bytes())
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn wall(depth: f32) -> DepthMap {
		DepthMap {
			width: 4,
			height: 2,
			depths: vec![depth, depth, depth, depth, 0.0, depth, depth, depth],
			intrinsics: Intrinsics::from_fov(4, 2, std::f32::consts::FRAC_PI_2),
			scale: 1.0,
			took: Duration::ZERO,
		}
	}

	#[test]
	fn unknown_and_outside_read_as_none() {
		let map = wall(2.0);
		assert_eq!(map.get(1, 0), Some(2.0));
		assert_eq!(map.get(0, 1), None);
		assert_eq!(map.get(4, 0), None);
		assert_eq!(map.sample(0.9, 0.9), Some(2.0));
		assert_eq!(map.sample(0.1, 0.9), None);
		assert_eq!(map.sample(1.1, 0.5), None);
		assert!((map.coverage() - 7.0 / 8.0).abs() < 1e-6);
	}

	#[test]
	fn the_ray_to_a_wall_s_edge_is_longer_than_its_depth() {
		// A 90° lens: the picture's right edge is 45° off axis.
		let map = wall(2.0);
		let [x, y, z] = map.point(1.0, 0.5).unwrap();
		assert!((x - 2.0).abs() < 1e-5 && y.abs() < 1e-5 && (z - 2.0).abs() < 1e-5);
		assert!((map.distance(1.0, 0.5).unwrap() - 2.0 * 2.0_f32.sqrt()).abs() < 1e-5);
		assert!((map.distance(0.5, 0.5).unwrap() - 2.0).abs() < 1e-5);
	}

	#[test]
	fn bytes_are_little_endian_metres() {
		let bytes = wall(1.5).to_le_bytes();
		assert_eq!(bytes.len(), 32);
		assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 1.5);
		assert_eq!(f32::from_le_bytes(bytes[16..20].try_into().unwrap()), 0.0);
	}
}
