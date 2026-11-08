//! The ARKit methods that take or return SIMD types, which the generated
//! bindings leave out.
//!
//! ARKit returns `simd_float4x4` and `simd_float3` in vector registers: on
//! arm64 they are homogeneous vector aggregates, returned in `v0`–`v3`. Rust
//! lays out an array of `float32x4_t` the same way and passes it the same
//! way, so each method is called through `objc_msgSend` cast to its exact C
//! signature. Message verification can't check these encodings, so they are
//! never sent through `msg_send!`.

use std::arch::aarch64::float32x4_t;
use std::ffi::c_void;

use objc2::runtime::AnyObject;
use objc2::runtime::Sel;

use crate::math::Pose;
use crate::math::Vec3;

/// `simd_float4x4`: four column vectors.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Float4x4([float32x4_t; 4]);

/// `simd_float3x3`: three column vectors, each padded to four lanes.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Float3x3([float32x4_t; 3]);

impl Float3x3 {
	/// The columns, without their padding lanes.
	pub(super) fn columns(self) -> [[f32; 3]; 3] {
		self.0.map(|column| {
			let [x, y, z, _] = lanes(column);
			[x, y, z]
		})
	}
}

/// `simd_float3`, padded to four lanes as ARKit stores it.
pub(super) type Float3 = float32x4_t;

fn lanes(vector: float32x4_t) -> [f32; 4] {
	// SAFETY: `float32x4_t` is four `f32` lanes in order.
	unsafe { std::mem::transmute::<float32x4_t, [f32; 4]>(vector) }
}

fn vector(values: [f32; 4]) -> float32x4_t {
	// SAFETY: As in `lanes`, the other way.
	unsafe { std::mem::transmute::<[f32; 4], float32x4_t>(values) }
}

impl Float4x4 {
	/// The matrix as a column-major array.
	pub(super) fn to_array(self) -> [f32; 16] {
		let mut matrix = [0.0; 16];

		for (column, values) in matrix.as_chunks_mut::<4>().0.iter_mut().zip(self.0) {
			column.copy_from_slice(&lanes(values));
		}

		matrix
	}

	/// A matrix from a column-major array.
	pub(super) fn from_array(matrix: &[f32; 16]) -> Self {
		let column = |index: usize| {
			vector([
				matrix[index * 4],
				matrix[index * 4 + 1],
				matrix[index * 4 + 2],
				matrix[index * 4 + 3],
			])
		};

		Self([column(0), column(1), column(2), column(3)])
	}

	/// The rigid transform the matrix holds.
	pub(super) fn pose(self) -> Pose {
		Pose::from_matrix(&self.to_array())
	}
}

/// A `simd_float3` as a vector.
pub(super) fn vec3(value: Float3) -> Vec3 {
	let [x, y, z, _] = lanes(value);
	Vec3::new(x, y, z)
}

/// A vector as a `simd_float3`.
pub(super) fn float3(value: Vec3) -> Float3 {
	vector([value.x, value.y, value.z, 0.0])
}

/// `objc_msgSend`, cast to `R (*)(id, SEL, A...)`.
macro_rules! send {
	($receiver:expr, $selector:expr $(, $argument:expr => $kind:ty)* ; $ret:ty) => {{
		let send: unsafe extern "C-unwind" fn(*const AnyObject, Sel $(, $kind)*) -> $ret =
			std::mem::transmute::<unsafe extern "C-unwind" fn(), _>(objc2::ffi::objc_msgSend);
		send($receiver, $selector $(, $argument)*)
	}};
}

/// Sends a message returning a `simd_float4x4`.
///
/// # Safety
///
/// `receiver` must be live and respond to `selector` with no arguments and
/// that return type.
pub(super) unsafe fn matrix(receiver: *const AnyObject, selector: Sel) -> Float4x4 {
	// SAFETY: The caller guarantees the method's signature.
	unsafe { send!(receiver, selector; Float4x4) }
}

/// Sends a message returning a `simd_float3`.
///
/// # Safety
///
/// As [`matrix`], for a `simd_float3` return.
pub(super) unsafe fn float3_of(receiver: *const AnyObject, selector: Sel) -> Float3 {
	// SAFETY: The caller guarantees the method's signature.
	unsafe { send!(receiver, selector; Float3) }
}

/// Sends a message returning a pointer.
///
/// # Safety
///
/// As [`matrix`], for a pointer return.
pub(super) unsafe fn pointer(receiver: *const AnyObject, selector: Sel) -> *const c_void {
	// SAFETY: The caller guarantees the method's signature.
	unsafe { send!(receiver, selector; *const c_void) }
}

/// `-[ARCamera viewMatrixForOrientation:]`.
///
/// # Safety
///
/// `camera` must be a live `ARCamera`.
pub(super) unsafe fn view_matrix(camera: *const AnyObject, orientation: isize) -> Float4x4 {
	// SAFETY: The signature is ARKit's.
	unsafe { send!(camera, objc2::sel!(viewMatrixForOrientation:), orientation => isize; Float4x4) }
}

/// `-[ARCamera intrinsics]`: focal lengths and principal point, in pixels of
/// the captured image.
///
/// # Safety
///
/// `camera` must be a live `ARCamera`.
pub(super) unsafe fn intrinsics(camera: *const AnyObject) -> Float3x3 {
	// SAFETY: The signature is ARKit's.
	unsafe { send!(camera, objc2::sel!(intrinsics); Float3x3) }
}

/// `-[ARCamera projectionMatrixForOrientation:viewportSize:zNear:zFar:]`.
///
/// # Safety
///
/// `camera` must be a live `ARCamera`.
pub(super) unsafe fn projection_matrix(
	camera: *const AnyObject,
	orientation: isize,
	size: (f64, f64),
	near: f64,
	far: f64,
) -> Float4x4 {
	/// `CGSize`: two doubles, passed in `d0` and `d1`.
	#[repr(C)]
	#[derive(Clone, Copy)]
	struct Size(f64, f64);

	// SAFETY: The signature is ARKit's.
	unsafe {
		send!(
			camera,
			objc2::sel!(projectionMatrixForOrientation:viewportSize:zNear:zFar:),
			orientation => isize,
			Size(size.0, size.1) => Size,
			near => f64,
			far => f64;
			Float4x4
		)
	}
}

/// `-[ARAnchor initWithTransform:]` on an allocated anchor; returns it
/// retained.
///
/// # Safety
///
/// `allocated` must be a fresh `+[ARAnchor alloc]`.
pub(super) unsafe fn init_anchor(allocated: *mut AnyObject, transform: Float4x4) -> *mut AnyObject {
	// SAFETY: The signature is ARKit's.
	unsafe {
		send!(allocated.cast_const(), objc2::sel!(initWithTransform:), transform => Float4x4; *mut AnyObject)
	}
}

/// `-[ARRaycastQuery initWithOrigin:direction:allowingTarget:alignment:]` on
/// an allocated query; returns it retained.
///
/// # Safety
///
/// `allocated` must be a fresh `+[ARRaycastQuery alloc]`.
pub(super) unsafe fn init_raycast(
	allocated: *mut AnyObject,
	origin: Float3,
	direction: Float3,
	target: isize,
	alignment: isize,
) -> *mut AnyObject {
	// SAFETY: The signature is ARKit's.
	unsafe {
		send!(
			allocated.cast_const(),
			objc2::sel!(initWithOrigin:direction:allowingTarget:alignment:),
			origin => Float3,
			direction => Float3,
			target => isize,
			alignment => isize;
			*mut AnyObject
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::math::Quat;

	#[test]
	fn matrices_keep_their_columns() {
		let pose = Pose::new(
			Vec3::new(1.0, 2.0, 3.0),
			Quat::from_axis_angle(Vec3::Y, 0.3),
		);
		let matrix = pose.to_matrix();
		assert_eq!(Float4x4::from_array(&matrix).to_array(), matrix);
		assert_eq!(
			vec3(float3(Vec3::new(4.0, 5.0, 6.0))),
			Vec3::new(4.0, 5.0, 6.0)
		);
	}
}
