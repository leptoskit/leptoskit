//! Reading one ARCore frame: the camera, planes, anchors, light, depth, and
//! the camera picture, and hit tests against it.

use std::sync::Arc;

use super::ArCoreBackend;
use super::KnownPlane;
use super::ScratchPose;
use super::Session;
use super::ffi;
use super::image_plane;
use super::tracking;
use crate::host::Intrinsics;
use crate::host::XrFrame;
use crate::image::*;
use crate::math::Fov;
use crate::math::Pose;
use crate::math::Projection;
use crate::math::Ray;
use crate::types::*;

impl ArCoreBackend {
	pub(super) fn frame(&mut self, time: i64) -> XrFrame {
		let session = self.session.as_ref().expect("update checked the session");
		let (viewer, projection, state) = camera(session);
		let display_transform = display_transform(session);
		let features = self.features;
		let depth = features
			.contains(Features::DEPTH)
			.then(|| depth(session, display_transform))
			.flatten();
		let camera_image = features
			.contains(Features::CAMERA_IMAGE)
			.then(|| camera_image(session, display_transform))
			.flatten();
		let camera_intrinsics = camera_image.as_ref().and_then(|_| intrinsics(session));
		let light = features
			.contains(Features::LIGHT_ESTIMATION)
			.then(|| light(session))
			.flatten();
		let anchors = self.anchors_of();

		let planes = self.planes_of();

		XrFrame {
			time: time as f64 / 1e9,
			tracking: state,
			viewer,
			views: vec![View {
				pose: viewer,
				projection,
			}],
			camera_image,
			camera_intrinsics,
			depth,
			planes: planes.into(),
			planes_revision: 0,
			light,
			anchors,
			inputs: Vec::new(),
		}
	}

	fn planes_of(&mut self) -> Vec<Plane> {
		let Some(session) = self.session.as_ref() else {
			return Vec::new();
		};
		let api = session.api;
		let mut size = 0;
		// SAFETY: Fills the session's list with every plane and counts them.
		unsafe {
			(api.ArSession_getAllTrackables)(session.handle, ffi::TRACKABLE_PLANE, session.planes);
			(api.ArTrackableList_getSize)(session.handle, session.planes, &raw mut size);
		}
		let mut planes = Vec::new();

		for index in 0..size {
			let mut trackable = std::ptr::null_mut();
			// SAFETY: Acquires one plane from the list.
			unsafe {
				(api.ArTrackableList_acquireItem)(
					session.handle,
					session.planes,
					index,
					&raw mut trackable,
				);
			}

			if let Some(plane) = plane(session, trackable) {
				let id = remember(&mut self.planes, &mut self.next_plane, api, trackable);
				planes.push(Plane { id, ..plane });
			} else {
				// SAFETY: Releases a plane that isn't reported.
				unsafe { (api.ArTrackable_release)(trackable) };
			}
		}

		planes
	}

	fn anchors_of(&self) -> Vec<Anchor> {
		let Some(session) = self.session.as_ref() else {
			return Vec::new();
		};
		let scratch = ScratchPose::new(session, Pose::IDENTITY);

		self.anchors
			.iter()
			.map(|(&id, &anchor)| {
				let mut state = 0;
				// SAFETY: Reads a live anchor's pose and tracking state.
				unsafe {
					(session.api.ArAnchor_getPose)(session.handle, anchor, scratch.1);
					(session.api.ArAnchor_getTrackingState)(session.handle, anchor, &raw mut state);
				}

				Anchor {
					id,
					pose: scratch.read(),
					tracking: tracking(state, 0),
				}
			})
			.collect()
	}

	/// ARCore's own hit test, which finds planes inside their outlines,
	/// depth points, and feature points.
	pub(super) fn native_hits(&self, ray: Ray, targets: HitTargets) -> Vec<Hit> {
		let Some(session) = self.session.as_ref() else {
			return Vec::new();
		};
		let api = session.api;
		let (origin, direction) = (
			[ray.origin.x, ray.origin.y, ray.origin.z],
			[ray.direction.x, ray.direction.y, ray.direction.z],
		);
		let (mut list, mut result, mut size) = (std::ptr::null_mut(), std::ptr::null_mut(), 0);
		// SAFETY: Casts the ray against the current frame into a new list.
		unsafe {
			(api.ArHitResultList_create)(session.handle, &raw mut list);
			(api.ArHitResult_create)(session.handle, &raw mut result);
			(api.ArFrame_hitTestRay)(
				session.handle,
				session.frame,
				origin.as_ptr(),
				direction.as_ptr(),
				list,
			);
			(api.ArHitResultList_getSize)(session.handle, list, &raw mut size);
		}
		let hits = (0..size)
			.filter_map(|index| {
				// SAFETY: Copies one result out of the list.
				unsafe { (api.ArHitResultList_getItem)(session.handle, list, index, result) };
				self.hit(session, result, targets)
			})
			.collect();
		// SAFETY: Frees what this function made.
		unsafe {
			(api.ArHitResult_destroy)(result);
			(api.ArHitResultList_destroy)(list);
		}

		hits
	}

	fn hit(
		&self,
		session: &Session,
		result: *const ffi::HitResult,
		targets: HitTargets,
	) -> Option<Hit> {
		let api = session.api;
		let scratch = ScratchPose::new(session, Pose::IDENTITY);
		let (mut distance, mut trackable, mut kind) = (0.0, std::ptr::null_mut(), 0);
		// SAFETY: Reads the result's distance, pose, and what it hit.
		unsafe {
			(api.ArHitResult_getDistance)(session.handle, result, &raw mut distance);
			(api.ArHitResult_getHitPose)(session.handle, result, scratch.1);
			(api.ArHitResult_acquireTrackable)(session.handle, result, &raw mut trackable);
			(api.ArTrackable_getType)(session.handle, trackable, &raw mut kind);
		}
		let plane = self.planes.get(&(trackable as usize)).map(|known| known.id);
		// SAFETY: Releases the trackable just acquired.
		unsafe { (api.ArTrackable_release)(trackable) };

		let (kind, wanted) = match kind {
			ffi::TRACKABLE_PLANE => (HitKind::Plane, HitTargets::PLANES),
			ffi::TRACKABLE_DEPTH_POINT => (HitKind::Depth, HitTargets::DEPTH),
			_ => (HitKind::Estimated, HitTargets::ESTIMATED),
		};

		targets.contains(wanted).then(|| {
			Hit {
				pose: scratch.read(),
				distance,
				kind,
				plane: plane.filter(|_| kind == HitKind::Plane),
			}
		})
	}
}

/// Gives a plane handle its stable id, keeping one reference to it so the
/// handle can't come back as a different plane.
fn remember(
	known: &mut std::collections::HashMap<usize, KnownPlane>,
	next: &mut u64,
	api: &ffi::Api,
	trackable: *mut ffi::Trackable,
) -> u64 {
	if let Some(plane) = known.get(&(trackable as usize)) {
		// SAFETY: Releases the extra reference; the kept one remains.
		unsafe { (api.ArTrackable_release)(trackable) };

		return plane.id;
	}

	*next += 1;
	known.insert(
		trackable as usize,
		KnownPlane {
			id: *next,
			trackable,
		},
	);
	*next
}

/// One tracked, unmerged plane, without its id.
fn plane(session: &Session, trackable: *mut ffi::Trackable) -> Option<Plane> {
	let api = session.api;
	let (mut state, mut subsumed, mut kind) = (0, std::ptr::null_mut(), 0);
	// SAFETY: Reads the plane's state, whether it was merged, and its type.
	unsafe {
		(api.ArTrackable_getTrackingState)(session.handle, trackable, &raw mut state);
		(api.ArPlane_acquireSubsumedBy)(session.handle, trackable, &raw mut subsumed);
		(api.ArPlane_getType)(session.handle, trackable, &raw mut kind);
	}

	if !subsumed.is_null() {
		// SAFETY: Releases the plane that took this one over.
		unsafe { (api.ArTrackable_release)(subsumed) };

		return None;
	}

	if state != ffi::TRACKING_STATE_TRACKING {
		return None;
	}

	let scratch = ScratchPose::new(session, Pose::IDENTITY);
	let (mut width, mut depth, mut count) = (0.0, 0.0, 0);
	// SAFETY: Reads the plane's centre, extent, and outline length.
	unsafe {
		(api.ArPlane_getCenterPose)(session.handle, trackable, scratch.1);
		(api.ArPlane_getExtentX)(session.handle, trackable, &raw mut width);
		(api.ArPlane_getExtentZ)(session.handle, trackable, &raw mut depth);
		(api.ArPlane_getPolygonSize)(session.handle, trackable, &raw mut count);
	}
	let mut outline = vec![0.0_f32; count.max(0) as usize];
	// SAFETY: `outline` has the `count` floats ARCore writes.
	unsafe { (api.ArPlane_getPolygon)(session.handle, trackable, outline.as_mut_ptr()) };
	let points: Vec<[f32; 2]> = outline.as_chunks::<2>().0.to_vec();

	Some(Plane {
		id: 0,
		orientation: match kind {
			ffi::PLANE_HORIZONTAL_UPWARD_FACING => PlaneOrientation::HorizontalUp,
			ffi::PLANE_HORIZONTAL_DOWNWARD_FACING => PlaneOrientation::HorizontalDown,
			_ => PlaneOrientation::Vertical,
		},

		label: PlaneLabel::Unknown,
		pose: scratch.read(),
		extent: [width, depth],
		polygon: thin_polygon(&points),
	})
}

/// The camera's pose for the screen's orientation, its projection, and how
/// well it tracks.
fn camera(session: &Session) -> (Pose, Projection, TrackingState) {
	let api = session.api;
	let scratch = ScratchPose::new(session, Pose::IDENTITY);
	let (mut camera, mut state, mut reason) = (std::ptr::null_mut(), 0, 0);
	let mut matrix = [0.0_f32; 16];
	// SAFETY: Reads the frame's camera and releases it.
	unsafe {
		(api.ArFrame_acquireCamera)(session.handle, session.frame, &raw mut camera);
		(api.ArCamera_getTrackingState)(session.handle, camera, &raw mut state);
		(api.ArCamera_getTrackingFailureReason)(session.handle, camera, &raw mut reason);
		(api.ArCamera_getDisplayOrientedPose)(session.handle, camera, scratch.1);
		(api.ArCamera_getProjectionMatrix)(
			session.handle,
			camera,
			super::NEAR,
			super::FAR,
			matrix.as_mut_ptr(),
		);
		(api.ArCamera_release)(camera);
	}
	let projection = Fov::from_matrix(&matrix).map_or(Projection::Matrix(matrix), Projection::Fov);

	(scratch.read(), projection, tracking(state, reason))
}

/// How the camera projects onto the CPU image.
fn intrinsics(session: &Session) -> Option<Intrinsics> {
	let api = session.api;
	let (mut camera, mut intrinsics) = (std::ptr::null_mut(), std::ptr::null_mut());
	let (mut fx, mut fy, mut cx, mut cy, mut width, mut height) = (0.0, 0.0, 0.0, 0.0, 0, 0);
	// SAFETY: Reads the frame's camera into an intrinsics object, then frees
	// both.
	unsafe {
		(api.ArCameraIntrinsics_create)(session.handle, &raw mut intrinsics);
		(api.ArFrame_acquireCamera)(session.handle, session.frame, &raw mut camera);
		(api.ArCamera_getImageIntrinsics)(session.handle, camera, intrinsics);
		(api.ArCameraIntrinsics_getFocalLength)(
			session.handle,
			intrinsics,
			&raw mut fx,
			&raw mut fy,
		);
		(api.ArCameraIntrinsics_getPrincipalPoint)(
			session.handle,
			intrinsics,
			&raw mut cx,
			&raw mut cy,
		);
		(api.ArCameraIntrinsics_getImageDimensions)(
			session.handle,
			intrinsics,
			&raw mut width,
			&raw mut height,
		);
		(api.ArCamera_release)(camera);
		(api.ArCameraIntrinsics_destroy)(intrinsics);
	}

	(fx > 0.0 && fy > 0.0 && width > 0 && height > 0).then_some(Intrinsics {
		focal: [fx, fy],
		principal: [cx, cy],
		size: [width as u32, height as u32],
	})
}

/// Maps the view to the camera image, from where ARCore puts three corners
/// of the view in the image.
fn display_transform(session: &Session) -> Affine2 {
	let view = [0.0_f32, 0.0, 1.0, 0.0, 0.0, 1.0];
	let mut image = [0.0_f32; 6];
	// SAFETY: Both arrays hold three points.
	unsafe {
		(session.api.ArFrame_transformCoordinates2d)(
			session.handle,
			session.frame,
			ffi::COORDINATES_VIEW_NORMALIZED,
			3,
			view.as_ptr(),
			ffi::COORDINATES_IMAGE_NORMALIZED,
			image.as_mut_ptr(),
		);
	}
	let [ox, oy, rx, ry, dx, dy] = image;

	[rx - ox, ry - oy, dx - ox, dy - oy, ox, oy]
}

fn light(session: &Session) -> Option<LightEstimate> {
	let api = session.api;
	let (mut state, mut intensity) = (0, 0.0);
	// SAFETY: Reads the frame's light estimate into the session's.
	unsafe {
		(api.ArFrame_getLightEstimate)(session.handle, session.frame, session.light);
		(api.ArLightEstimate_getState)(session.handle, session.light, &raw mut state);
		(api.ArLightEstimate_getPixelIntensity)(session.handle, session.light, &raw mut intensity);
	}

	(state == ffi::LIGHT_ESTIMATE_VALID).then(|| {
		LightEstimate {
			intensity: intensity / super::NEUTRAL_INTENSITY,
			color_temperature: super::NEUTRAL_KELVIN,
			main_light_direction: None,
		}
	})
}

/// An `ArImage`, released when dropped.
struct AcquiredImage<'a>(&'a Session, *mut ffi::Image);

impl AcquiredImage<'_> {
	fn size(&self) -> (u32, u32) {
		let (mut width, mut height) = (0, 0);
		// SAFETY: Reads the image's size.
		unsafe {
			(self.0.api.ArImage_getWidth)(self.0.handle, self.1, &raw mut width);
			(self.0.api.ArImage_getHeight)(self.0.handle, self.1, &raw mut height);
		}
		(width.max(0) as u32, height.max(0) as u32)
	}
}

impl Drop for AcquiredImage<'_> {
	fn drop(&mut self) {
		// SAFETY: Releases the image this value acquired.
		unsafe { (self.0.api.ArImage_release)(self.1) };
	}
}

/// The Depth API's 16-bit millimetres.
fn depth(session: &Session, display_transform: Affine2) -> Option<DepthImage> {
	let mut image = std::ptr::null_mut();
	// SAFETY: Acquires the frame's depth image when one is ready.
	let status = unsafe {
		(session.api.ArFrame_acquireDepthImage16Bits)(session.handle, session.frame, &raw mut image)
	};

	if status != ffi::SUCCESS {
		return None;
	}

	let image = AcquiredImage(session, image);
	let (width, height) = image.size();
	let (data, row_stride, _) = image_plane(session, image.1, 0)?;

	Some(DepthImage {
		width,
		height,
		format: DepthFormat::Millimeters16,
		data: Arc::from(data),
		row_stride: row_stride as u32,
		confidence: None,
		display_transform,
	})
}

/// The CPU camera image, as luma and interleaved chroma.
fn camera_image(session: &Session, display_transform: Affine2) -> Option<CameraImage> {
	let mut image = std::ptr::null_mut();
	// SAFETY: Acquires the frame's CPU image when one is ready.
	let status = unsafe {
		(session.api.ArFrame_acquireCameraImage)(session.handle, session.frame, &raw mut image)
	};

	if status != ffi::SUCCESS {
		return None;
	}

	let image = AcquiredImage(session, image);
	let (width, height) = image.size();
	let (luma, luma_stride, _) = image_plane(session, image.1, 0)?;
	let (cb, chroma_stride, chroma_step) = image_plane(session, image.1, 1)?;
	let (cr, ..) = image_plane(session, image.1, 2)?;
	let pixels = ycbcr_from_yuv420(
		width,
		height,
		luma,
		luma_stride as u32,
		cb,
		cr,
		chroma_stride as u32,
		chroma_step as u32,
	)?;

	Some(CameraImage {
		width,
		height,
		display_transform,
		pixels,
	})
}
