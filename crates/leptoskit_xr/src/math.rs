//! Small, dependency-free geometry: vectors, rotations, poses, rays, and
//! projections.
//!
//! Every backend reports the same convention, which ARKit, ARCore, OpenXR and
//! WebXR already share: right-handed, metres, +Y up, and a viewer looking down
//! its own −Z axis. Matrices are column-major `[f32; 16]`, as wgpu and WGSL
//! read them, and projections map depth to wgpu's 0..1 clip range.

use std::ops::Add;
use std::ops::Mul;
use std::ops::Neg;
use std::ops::Sub;

/// A point or direction in metres.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
	/// Right.
	pub x: f32,
	/// Up.
	pub y: f32,
	/// Backwards: a viewer looks down −Z.
	pub z: f32,
}

impl Vec3 {
	/// The direction a viewer looks.
	pub const FORWARD: Self = Self::new(0.0, 0.0, -1.0);
	/// World up.
	pub const Y: Self = Self::new(0.0, 1.0, 0.0);
	/// The origin.
	pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);

	/// A vector from its components.
	#[must_use]
	pub const fn new(x: f32, y: f32, z: f32) -> Self {
		Self { x, y, z }
	}

	/// The dot product.
	#[must_use]
	pub fn dot(self, other: Self) -> f32 {
		self.x * other.x + self.y * other.y + self.z * other.z
	}

	/// The cross product.
	#[must_use]
	pub fn cross(self, other: Self) -> Self {
		Self::new(
			self.y * other.z - self.z * other.y,
			self.z * other.x - self.x * other.z,
			self.x * other.y - self.y * other.x,
		)
	}

	/// The length.
	#[must_use]
	pub fn length(self) -> f32 {
		self.dot(self).sqrt()
	}

	/// The same direction with length one, or `None` for a zero or non-finite
	/// vector.
	#[must_use]
	pub fn normalized(self) -> Option<Self> {
		let length = self.length();

		(length.is_finite() && length > f32::EPSILON).then(|| self * (1.0 / length))
	}

	/// Whether every component is finite.
	#[must_use]
	pub fn is_finite(self) -> bool {
		self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
	}
}

impl Add for Vec3 {
	type Output = Self;

	fn add(self, other: Self) -> Self {
		Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
	}
}

impl Sub for Vec3 {
	type Output = Self;

	fn sub(self, other: Self) -> Self {
		Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
	}
}

impl Mul<f32> for Vec3 {
	type Output = Self;

	fn mul(self, scale: f32) -> Self {
		Self::new(self.x * scale, self.y * scale, self.z * scale)
	}
}

impl Neg for Vec3 {
	type Output = Self;

	fn neg(self) -> Self {
		Self::new(-self.x, -self.y, -self.z)
	}
}

/// A rotation as a unit quaternion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
	/// Vector part, x.
	pub x: f32,
	/// Vector part, y.
	pub y: f32,
	/// Vector part, z.
	pub z: f32,
	/// Scalar part.
	pub w: f32,
}

impl Default for Quat {
	fn default() -> Self {
		Self::IDENTITY
	}
}

impl Quat {
	/// No rotation.
	pub const IDENTITY: Self = Self::new(0.0, 0.0, 0.0, 1.0);

	/// A quaternion from its components; see [`Quat::normalized`].
	#[must_use]
	pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
		Self { x, y, z, w }
	}

	/// A rotation of `angle` radians about `axis`, counter-clockwise when
	/// looking down the axis towards the origin.
	#[must_use]
	pub fn from_axis_angle(axis: Vec3, angle: f32) -> Self {
		let Some(axis) = axis.normalized() else {
			return Self::IDENTITY;
		};
		let (sin, cos) = (angle * 0.5).sin_cos();

		Self::new(axis.x * sin, axis.y * sin, axis.z * sin, cos)
	}

	/// The rotation that turns +Y onto `normal`, taking the shortest arc.
	#[must_use]
	pub fn from_up(normal: Vec3) -> Self {
		let Some(normal) = normal.normalized() else {
			return Self::IDENTITY;
		};
		let dot = Vec3::Y.dot(normal);

		if dot < -0.999_999 {
			return Self::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), std::f32::consts::PI);
		}

		let axis = Vec3::Y.cross(normal);

		Self::new(axis.x, axis.y, axis.z, 1.0 + dot)
			.normalized()
			.unwrap_or(Self::IDENTITY)
	}

	/// The inverse of a unit quaternion.
	#[must_use]
	pub fn conjugate(self) -> Self {
		Self::new(-self.x, -self.y, -self.z, self.w)
	}

	/// The same rotation with norm one, or `None` when it has no direction.
	#[must_use]
	pub fn normalized(self) -> Option<Self> {
		let norm = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();

		(norm.is_finite() && norm > f32::EPSILON).then(|| {
			let scale = 1.0 / norm;
			Self::new(
				self.x * scale,
				self.y * scale,
				self.z * scale,
				self.w * scale,
			)
		})
	}

	/// Rotates a vector.
	#[must_use]
	pub fn rotate(self, vector: Vec3) -> Vec3 {
		let axis = Vec3::new(self.x, self.y, self.z);
		let twice = axis.cross(vector) * 2.0;

		vector + twice * self.w + axis.cross(twice)
	}

	/// Whether every component is finite.
	#[must_use]
	pub fn is_finite(self) -> bool {
		self.x.is_finite() && self.y.is_finite() && self.z.is_finite() && self.w.is_finite()
	}
}

impl Mul for Quat {
	type Output = Self;

	/// `self * other` rotates by `other` first, then by `self`.
	fn mul(self, other: Self) -> Self {
		Self::new(
			self.w * other.x + self.x * other.w + self.y * other.z - self.z * other.y,
			self.w * other.y - self.x * other.z + self.y * other.w + self.z * other.x,
			self.w * other.z + self.x * other.y - self.y * other.x + self.z * other.w,
			self.w * other.w - self.x * other.x - self.y * other.y - self.z * other.z,
		)
	}
}

/// A rigid transform: where something is and which way it faces.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pose {
	/// Position in metres.
	pub position: Vec3,
	/// Orientation.
	pub orientation: Quat,
}

impl Pose {
	/// At the origin, unrotated.
	pub const IDENTITY: Self = Self {
		position: Vec3::ZERO,
		orientation: Quat::IDENTITY,
	};

	/// A pose from its parts.
	#[must_use]
	pub const fn new(position: Vec3, orientation: Quat) -> Self {
		Self {
			position,
			orientation,
		}
	}

	/// An unrotated pose at `position`.
	#[must_use]
	pub const fn at(position: Vec3) -> Self {
		Self::new(position, Quat::IDENTITY)
	}

	/// Maps a point from this pose's space into its parent's.
	#[must_use]
	pub fn transform_point(self, point: Vec3) -> Vec3 {
		self.position + self.orientation.rotate(point)
	}

	/// Maps a direction from this pose's space into its parent's.
	#[must_use]
	pub fn transform_vector(self, vector: Vec3) -> Vec3 {
		self.orientation.rotate(vector)
	}

	/// The pose that undoes this one.
	#[must_use]
	pub fn inverse(self) -> Self {
		let orientation = self.orientation.conjugate();

		Self::new(orientation.rotate(-self.position), orientation)
	}

	/// `self.then(child)` places `child`, given in this pose's space, in the
	/// parent's space.
	#[must_use]
	pub fn then(self, child: Self) -> Self {
		Self::new(
			self.transform_point(child.position),
			self.orientation * child.orientation,
		)
	}

	/// The direction this pose looks: its −Z axis.
	#[must_use]
	pub fn forward(self) -> Vec3 {
		self.orientation.rotate(Vec3::FORWARD)
	}

	/// The pose as a column-major matrix, mapping this space to its parent's.
	#[must_use]
	pub fn to_matrix(self) -> [f32; 16] {
		let Quat { x, y, z, w } = self.orientation;
		let Vec3 {
			x: px,
			y: py,
			z: pz,
		} = self.position;

		[
			1.0 - 2.0 * (y * y + z * z),
			2.0 * (x * y + z * w),
			2.0 * (x * z - y * w),
			0.0,
			2.0 * (x * y - z * w),
			1.0 - 2.0 * (x * x + z * z),
			2.0 * (y * z + x * w),
			0.0,
			2.0 * (x * z + y * w),
			2.0 * (y * z - x * w),
			1.0 - 2.0 * (x * x + y * y),
			0.0,
			px,
			py,
			pz,
			1.0,
		]
	}

	/// Reads a rigid column-major matrix, such as ARKit's and WebXR's
	/// transforms. Scale and shear are discarded.
	#[must_use]
	pub fn from_matrix(m: &[f32; 16]) -> Self {
		let position = Vec3::new(m[12], m[13], m[14]);
		let orientation =
			quat_from_rotation([m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]);

		Self::new(position, orientation)
	}

	/// Whether every component is finite.
	#[must_use]
	pub fn is_finite(self) -> bool {
		self.position.is_finite() && self.orientation.is_finite()
	}
}

/// A rotation matrix's columns as a unit quaternion (Shepperd's method).
fn quat_from_rotation(x: [f32; 3], y: [f32; 3], z: [f32; 3]) -> Quat {
	let trace = x[0] + y[1] + z[2];
	let quat = if trace > 0.0 {
		let s = (trace + 1.0).sqrt() * 2.0;
		Quat::new(
			(y[2] - z[1]) / s,
			(z[0] - x[2]) / s,
			(x[1] - y[0]) / s,
			s / 4.0,
		)
	} else if x[0] > y[1] && x[0] > z[2] {
		let s = (1.0 + x[0] - y[1] - z[2]).sqrt() * 2.0;
		Quat::new(
			s / 4.0,
			(y[0] + x[1]) / s,
			(z[0] + x[2]) / s,
			(y[2] - z[1]) / s,
		)
	} else if y[1] > z[2] {
		let s = (1.0 + y[1] - x[0] - z[2]).sqrt() * 2.0;
		Quat::new(
			(y[0] + x[1]) / s,
			s / 4.0,
			(z[1] + y[2]) / s,
			(z[0] - x[2]) / s,
		)
	} else {
		let s = (1.0 + z[2] - x[0] - y[1]).sqrt() * 2.0;
		Quat::new(
			(z[0] + x[2]) / s,
			(z[1] + y[2]) / s,
			s / 4.0,
			(x[1] - y[0]) / s,
		)
	};

	quat.normalized().unwrap_or(Quat::IDENTITY)
}

/// A half-line: where it starts and which way it goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
	/// The start, in metres.
	pub origin: Vec3,
	/// The direction, of length one.
	pub direction: Vec3,
}

impl Ray {
	/// A ray from `origin` towards `direction`, or `None` for a zero
	/// direction.
	#[must_use]
	pub fn new(origin: Vec3, direction: Vec3) -> Option<Self> {
		Some(Self {
			origin,
			direction: direction.normalized()?,
		})
	}

	/// The point `distance` metres along the ray.
	#[must_use]
	pub fn at(self, distance: f32) -> Vec3 {
		self.origin + self.direction * distance
	}
}

/// A view's field of view as four angles from its forward axis, in radians.
/// `left` and `down` are negative for a view centred on its axis, as OpenXR
/// reports them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fov {
	/// The angle to the left edge.
	pub left: f32,
	/// The angle to the right edge.
	pub right: f32,
	/// The angle to the top edge.
	pub up: f32,
	/// The angle to the bottom edge.
	pub down: f32,
}

impl Fov {
	/// A symmetric field of view from a vertical angle and an aspect ratio
	/// (width over height).
	#[must_use]
	pub fn symmetric(vertical: f32, aspect: f32) -> Self {
		let up = vertical * 0.5;
		let right = (up.tan() * aspect).atan();

		Self {
			left: -right,
			right,
			up,
			down: -up,
		}
	}

	/// The angles a perspective projection matrix covers, such as the ones
	/// ARKit, ARCore, and WebXR compute for the screen. `None` for a matrix
	/// that isn't a perspective projection.
	#[must_use]
	pub fn from_matrix(m: &[f32; 16]) -> Option<Self> {
		// x_ndc = m0·(x / −z) − m8, so the edges at ±1 are at
		// x / −z = (m8 ± 1) / m0; likewise for y with m5 and m9.
		if m[0].abs() < f32::EPSILON || m[5].abs() < f32::EPSILON || (m[11] + 1.0).abs() > 1e-3 {
			return None;
		}

		let fov = Self {
			left: ((m[8] - 1.0) / m[0]).atan(),
			right: ((m[8] + 1.0) / m[0]).atan(),
			up: ((m[9] + 1.0) / m[5]).atan(),
			down: ((m[9] - 1.0) / m[5]).atan(),
		};

		[fov.left, fov.right, fov.up, fov.down]
			.iter()
			.all(|angle| angle.is_finite())
			.then_some(fov)
	}

	/// A perspective projection with depth mapped to 0..1 between `near` and
	/// `far`.
	#[must_use]
	pub fn projection(self, near: f32, far: f32) -> [f32; 16] {
		let left = self.left.tan();
		let right = self.right.tan();
		let up = self.up.tan();
		let down = self.down.tan();
		let width = right - left;
		let height = up - down;
		let depth = far - near;

		[
			2.0 / width,
			0.0,
			0.0,
			0.0,
			0.0,
			2.0 / height,
			0.0,
			0.0,
			(right + left) / width,
			(up + down) / height,
			-far / depth,
			-1.0,
			0.0,
			0.0,
			-(far * near) / depth,
			0.0,
		]
	}
}

/// How a view projects the world: angles a renderer turns into a matrix
/// (headsets), or a matrix the platform computed for the screen (phones, the
/// browser).
#[derive(Debug, Clone, Copy, PartialEq)]
// A matrix is 64 bytes and angles 16; boxing it would cost an allocation per
// view per frame to save 48 bytes.
#[allow(variant_size_differences)]
pub enum Projection {
	/// Angles from the view's forward axis.
	Fov(Fov),
	/// A column-major projection matrix with 0..1 depth.
	Matrix([f32; 16]),
}

impl Projection {
	/// The column-major projection matrix for this view. A platform matrix is
	/// returned as is; its clip planes are the ones the backend was asked
	/// for.
	#[must_use]
	pub fn matrix(self, near: f32, far: f32) -> [f32; 16] {
		match self {
			Self::Fov(fov) => fov.projection(near, far),
			Self::Matrix(matrix) => matrix,
		}
	}

	/// The direction, in the view's own space, through a point on the view:
	/// `(0, 0)` is its top-left corner and `(1, 1)` its bottom-right.
	#[must_use]
	pub fn direction(self, x: f32, y: f32) -> Vec3 {
		let ndc_x = x * 2.0 - 1.0;
		let ndc_y = 1.0 - y * 2.0;

		match self {
			Self::Fov(fov) => {
				let horizontal = fov.left.tan() + (fov.right.tan() - fov.left.tan()) * x;
				let vertical = fov.up.tan() + (fov.down.tan() - fov.up.tan()) * y;
				Vec3::new(horizontal, vertical, -1.0)
			}
			Self::Matrix(m) => {
				// For a perspective matrix, x_clip = m0·x + m8·z and w = −z,
				// so on the z = −1 plane x = (ndc + m8) / m0.
				Vec3::new((ndc_x + m[8]) / m[0], (ndc_y + m[9]) / m[5], -1.0)
			}
		}
	}

	/// Whether every number is finite.
	#[must_use]
	pub fn is_finite(self) -> bool {
		match self {
			Self::Fov(fov) => {
				[fov.left, fov.right, fov.up, fov.down]
					.iter()
					.all(|angle| angle.is_finite())
			}
			Self::Matrix(matrix) => matrix.iter().all(|value| value.is_finite()),
		}
	}
}

/// Turns a device rotation in a +Z-up world into the viewer's orientation in
/// this crate's +Y-up world, looking out of the back of the phone, for an
/// interface turned `quarter_turns` clockwise from the device's natural
/// orientation.
#[must_use]
pub fn attitude_to_viewer(device: Quat, quarter_turns: u32) -> Quat {
	// The +Z-up world is the +Y-up world turned a quarter about +X.
	let world = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -std::f32::consts::FRAC_PI_2);
	// Turning the interface clockwise turns its frame about the screen's
	// normal, the opposite way.
	let screen = Quat::from_axis_angle(
		Vec3::new(0.0, 0.0, 1.0),
		-((quarter_turns % 4) as f32) * std::f32::consts::FRAC_PI_2,
	);

	(world * device * screen)
		.normalized()
		.unwrap_or(Quat::IDENTITY)
}

/// The heading of a viewer orientation: the rotation about +Y that points
/// −Z the way it looks, ignoring pitch and roll.
pub fn heading(orientation: Quat) -> f32 {
	let forward = orientation.rotate(Vec3::FORWARD);

	if forward.x.abs() < 1e-4 && forward.z.abs() < 1e-4 {
		return 0.0;
	}

	(-forward.x).atan2(-forward.z)
}

/// A device rotation from the W3C `deviceorientation` angles in degrees:
/// `alpha` about +Z, then `beta` about the new +X, then `gamma` about the new
/// +Y, in the same device and +Z-up world frames as
/// [`attitude_to_viewer`] takes.
#[must_use]
pub fn device_orientation(alpha: f32, beta: f32, gamma: f32) -> Quat {
	let z = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), alpha.to_radians());
	let x = Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), beta.to_radians());
	let y = Quat::from_axis_angle(Vec3::Y, gamma.to_radians());

	(z * x * y).normalized().unwrap_or(Quat::IDENTITY)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn close(a: Vec3, b: Vec3) -> bool {
		(a - b).length() < 1e-4
	}

	#[test]
	fn a_quarter_turn_about_y_turns_forward_to_the_left() {
		let turn = Quat::from_axis_angle(Vec3::Y, std::f32::consts::FRAC_PI_2);
		assert!(close(turn.rotate(Vec3::FORWARD), Vec3::new(-1.0, 0.0, 0.0)));
	}

	#[test]
	fn poses_compose_and_invert() {
		let parent = Pose::new(
			Vec3::new(1.0, 2.0, 3.0),
			Quat::from_axis_angle(Vec3::Y, 0.7),
		);
		let child = Pose::new(
			Vec3::new(-0.5, 0.25, 2.0),
			Quat::from_axis_angle(Vec3::new(1.0, 0.0, 0.0), -0.3),
		);
		let point = Vec3::new(0.3, -0.2, 0.9);
		let composed = parent.then(child);
		assert!(close(
			composed.transform_point(point),
			parent.transform_point(child.transform_point(point))
		));
		let round_trip = parent.inverse().then(parent);
		assert!(close(round_trip.position, Vec3::ZERO));
		assert!(close(round_trip.transform_vector(Vec3::Y), Vec3::Y));
	}

	#[test]
	fn matrices_round_trip_through_poses() {
		let pose = Pose::new(
			Vec3::new(0.5, 1.5, -2.0),
			Quat::from_axis_angle(Vec3::new(1.0, 1.0, 0.0), 2.4),
		);
		let back = Pose::from_matrix(&pose.to_matrix());
		let point = Vec3::new(1.0, -1.0, 0.5);
		assert!(close(
			back.transform_point(point),
			pose.transform_point(point)
		));
	}

	#[test]
	fn from_up_turns_y_onto_the_normal() {
		for normal in [
			Vec3::Y,
			-Vec3::Y,
			Vec3::new(1.0, 0.0, 0.0),
			Vec3::new(0.0, 0.0, 1.0),
		] {
			assert!(close(Quat::from_up(normal).rotate(Vec3::Y), normal));
		}
	}

	#[test]
	fn a_projection_puts_the_near_plane_at_zero_and_the_far_plane_at_one() {
		let projection = Fov::symmetric(1.0, 1.5).projection(0.1, 100.0);
		let depth = |z: f32| (projection[10] * z + projection[14]) / -z;
		assert!(depth(-0.1).abs() < 1e-5);
		assert!((depth(-100.0) - 1.0).abs() < 1e-4);
	}

	#[test]
	fn an_upright_phone_in_the_browser_looks_at_the_horizon() {
		// `beta` = 90: the phone stands upright in portrait.
		let viewer = attitude_to_viewer(device_orientation(0.0, 90.0, 0.0), 0);
		assert!(close(viewer.rotate(Vec3::Y), Vec3::Y));
		assert!(viewer.rotate(Vec3::FORWARD).y.abs() < 1e-4);
	}

	#[test]
	fn angles_survive_a_round_trip_through_a_matrix() {
		let fov = Fov {
			left: -0.6,
			right: 0.5,
			up: 0.45,
			down: -0.55,
		};
		let back = Fov::from_matrix(&fov.projection(0.05, 30.0)).unwrap();
		for (a, b) in [
			(fov.left, back.left),
			(fov.right, back.right),
			(fov.up, back.up),
			(fov.down, back.down),
		] {
			assert!((a - b).abs() < 1e-5);
		}
		assert_eq!(Fov::from_matrix(&Pose::IDENTITY.to_matrix()), None);
	}

	#[test]
	fn screen_directions_agree_between_angles_and_matrices() {
		let fov = Fov {
			left: -0.6,
			right: 0.5,
			up: 0.45,
			down: -0.55,
		};
		let from_matrix = Projection::Matrix(fov.projection(0.1, 10.0));

		for (x, y) in [(0.0, 0.0), (0.5, 0.5), (1.0, 0.25), (0.2, 0.9)] {
			assert!(close(
				Projection::Fov(fov).direction(x, y),
				from_matrix.direction(x, y)
			));
		}
	}
}
