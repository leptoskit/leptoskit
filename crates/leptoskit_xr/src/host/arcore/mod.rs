//! ARCore: motion tracking, planes, the Depth API, light estimation, hit
//! tests, and anchors on Android, through ARCore's NDK C API.
//!
//! # The camera picture
//!
//! ARCore can hand over each camera frame three ways, and the choice matters
//! because the renderer draws through wgpu, on OpenGL ES by default on
//! Android (the Solana Seeker's Vulkan driver reset the GPU when the camera
//! closed):
//!
//! - **An external OES texture** (`AR_TEXTURE_UPDATE_MODE_BIND_TO_TEXTURE_
//!   EXTERNAL_OES`, ARCore's default). Zero-copy, but `ArSession_update` then
//!   needs the renderer's EGL context current on the calling thread, and wgpu
//!   has no `samplerExternalOES`, so drawing it means raw GL inside wgpu's
//!   context.
//! - **An `AHardwareBuffer`** (`EXPOSE_HARDWARE_BUFFER`). Also zero-copy and
//!   decoupled from GL, but its YUV format needs an `EGLImage` bound to an
//!   external texture (the same wgpu gap) or Vulkan's sampler YCbCr conversion,
//!   which wgpu doesn't expose either.
//! - **The CPU image** (`ArFrame_acquireCameraImage`): a YUV 4:2:0 copy the
//!   renderer uploads as luma and chroma textures, which works on GL ES and
//!   Vulkan alike.
//!
//! This backend runs ARCore in hardware-buffer mode, so tracking needs no GL
//! context and can run on any thread, and hands the renderer the CPU image
//! as luma and chroma planes. The copy is about 1.5 bytes per pixel per frame
//! (3 MB at 1080p). Zero-copy import becomes possible once wgpu exposes
//! external textures on GL ES; the hardware buffer is where it will come from.

use std::collections::HashMap;

use leptoskit_bridge::Error;
use leptoskit_bridge::PlatformContext;

use super::Backend;
use super::Started;
use super::Viewport;
use super::XrFrame;
use crate::hit_test;
use crate::math::Pose;
use crate::math::Quat;
use crate::math::Vec3;
use crate::types::*;

mod ffi;
mod java;

use ffi::Api;

/// The clip planes ARCore's projection is asked for; only its angles are
/// kept.
const NEAR: f32 = 0.05;
const FAR: f32 = 100.0;
/// ARCore's pixel intensity of a well-lit room.
const NEUTRAL_INTENSITY: f32 = 0.5;
/// ARCore doesn't estimate a colour temperature; this is daylight.
const NEUTRAL_KELVIN: f32 = 6500.0;

/// ARCore's handles for one session.
struct Session {
	api: &'static Api,
	handle: *mut ffi::Session,
	frame: *mut ffi::Frame,
	light: *mut ffi::LightEstimate,
	planes: *mut ffi::TrackableList,
}

impl Drop for Session {
	fn drop(&mut self) {
		let api = self.api;
		// SAFETY: Releases handles this session made, the session last.
		unsafe {
			(api.ArTrackableList_destroy)(self.planes);
			(api.ArLightEstimate_destroy)(self.light);
			(api.ArFrame_destroy)(self.frame);
			(api.ArSession_pause)(self.handle);
			(api.ArSession_destroy)(self.handle);
		}
	}
}

/// A plane ARCore tracks, with the reference this backend holds so the
/// handle stays the same plane.
struct KnownPlane {
	id: u64,
	trackable: *mut ffi::Trackable,
}

/// ARCore's motion tracking.
pub struct ArCoreBackend {
	platform: Option<PlatformContext>,
	session: Option<Session>,
	features: Features,
	viewport: Viewport,
	last_time: i64,
	planes: HashMap<usize, KnownPlane>,
	next_plane: u64,
	anchors: HashMap<u64, *mut ffi::Anchor>,
	next_anchor: u64,
}

impl std::fmt::Debug for ArCoreBackend {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ArCoreBackend")
			.field("running", &self.session.is_some())
			.field("features", &self.features)
			.finish_non_exhaustive()
	}
}

// SAFETY: ARCore's handles may be used from any thread, one at a time; the
// runtime calls the backend from one thread at a time, and every JNI call
// attaches the calling thread first.
unsafe impl Send for ArCoreBackend {}

impl ArCoreBackend {
	/// A backend for the app whose context the host passed; `None` (no
	/// Android context) makes AR unavailable.
	#[must_use]
	pub fn new(platform: Option<PlatformContext>) -> Self {
		Self {
			platform,
			session: None,
			features: Features::NONE,
			viewport: Viewport::default(),
			last_time: -1,
			planes: HashMap::new(),
			next_plane: 0,
			anchors: HashMap::new(),
			next_anchor: 0,
		}
	}

	fn availability(&self) -> Option<i32> {
		let api = ffi::api()?;
		java::with_context(self.platform.as_ref()?, |env, context| {
			let mut availability = 0;
			// SAFETY: `env` is this thread's attached environment and
			// `context` the app's global reference.
			unsafe { (api.ArCoreApk_checkAvailability)(env, context, &raw mut availability) };
			availability
		})
	}

	fn open(&self, features: Features) -> Result<Session, Error> {
		let api = ffi::api().ok_or(Error::Unavailable)?;
		let platform = self.platform.as_ref().ok_or(Error::Unavailable)?;
		let mut session = std::ptr::null_mut();
		// SAFETY: Creates a session for the app's context on an attached
		// thread.
		let status = java::with_context(platform, |env, context| unsafe {
			(api.ArSession_create)(env, context, &raw mut session)
		})
		.ok_or(Error::Unavailable)?;
		status_to_error(status)?;
		let session = new_session(api, session);
		self.apply(&session, features)?;
		// SAFETY: Resumes the configured session.
		status_to_error(unsafe { (api.ArSession_resume)(session.handle) })?;
		Ok(session)
	}

	/// Configures plane finding, depth, and light estimation for `features`.
	fn apply(&self, session: &Session, features: Features) -> Result<(), Error> {
		let api = session.api;
		let raw = session.handle;
		let planes = match (
			features.contains(Features::PLANES_HORIZONTAL),
			features.contains(Features::PLANES_VERTICAL),
		) {
			(true, true) => ffi::PLANE_FINDING_HORIZONTAL_AND_VERTICAL,
			(true, false) => ffi::PLANE_FINDING_HORIZONTAL,
			(false, true) => ffi::PLANE_FINDING_VERTICAL,
			(false, false) => ffi::PLANE_FINDING_DISABLED,
		};
		let depth = if features.contains(Features::DEPTH) && depth_supported(session) {
			ffi::DEPTH_AUTOMATIC
		} else {
			ffi::DEPTH_DISABLED
		};
		let light = if features.contains(Features::LIGHT_ESTIMATION) {
			ffi::LIGHT_ESTIMATION_AMBIENT_INTENSITY
		} else {
			ffi::LIGHT_ESTIMATION_DISABLED
		};
		let mut config = std::ptr::null_mut();
		// SAFETY: Builds a configuration, applies it, and frees it.
		let status = unsafe {
			(api.ArConfig_create)(raw, &raw mut config);
			(api.ArConfig_setPlaneFindingMode)(raw, config, planes);
			(api.ArConfig_setDepthMode)(raw, config, depth);
			(api.ArConfig_setLightEstimationMode)(raw, config, light);
			(api.ArConfig_setUpdateMode)(raw, config, ffi::UPDATE_LATEST_CAMERA_IMAGE);
			(api.ArConfig_setFocusMode)(raw, config, ffi::FOCUS_AUTO);
			(api.ArConfig_setTextureUpdateMode)(raw, config, ffi::TEXTURE_EXPOSE_HARDWARE_BUFFER);
			let status = (api.ArSession_configure)(raw, config);
			(api.ArConfig_destroy)(config);
			status
		};
		self.set_geometry(session);
		status_to_error(status)
	}

	fn set_geometry(&self, session: &Session) {
		// SAFETY: Tells ARCore the screen's rotation and size.
		unsafe {
			(session.api.ArSession_setDisplayGeometry)(
				session.handle,
				(self.viewport.quarter_turns % 4) as i32,
				self.viewport.width.max(1) as i32,
				self.viewport.height.max(1) as i32,
			);
		}
	}

	fn granted(features: Features, session: &Session) -> Features {
		if features.contains(Features::DEPTH) && !depth_supported(session) {
			features.difference(Features::DEPTH)
		} else {
			features
		}
	}

	fn release_planes(&mut self, api: &Api) {
		for (_, plane) in self.planes.drain() {
			// SAFETY: Releases the reference `planes_of` kept.
			unsafe { (api.ArTrackable_release)(plane.trackable) };
		}
	}

	fn release_anchors(&mut self) {
		let Some(session) = &self.session else {
			return;
		};

		for (_, anchor) in self.anchors.drain() {
			// SAFETY: Detaches and releases anchors this backend made.
			unsafe {
				(session.api.ArAnchor_detach)(session.handle, anchor);
				(session.api.ArAnchor_release)(anchor);
			}
		}
	}
}

fn new_session(api: &'static Api, session: *mut ffi::Session) -> Session {
	let mut frame = std::ptr::null_mut();
	let mut light = std::ptr::null_mut();
	let mut planes = std::ptr::null_mut();
	// SAFETY: Allocates the per-session frame, light estimate, and list.
	unsafe {
		(api.ArFrame_create)(session, &raw mut frame);
		(api.ArLightEstimate_create)(session, &raw mut light);
		(api.ArTrackableList_create)(session, &raw mut planes);
	}

	Session {
		api,
		handle: session,
		frame,
		light,
		planes,
	}
}

fn depth_supported(session: &Session) -> bool {
	let mut supported = 0;
	// SAFETY: A capability query on a live session.
	unsafe {
		(session.api.ArSession_isDepthModeSupported)(
			session.handle,
			ffi::DEPTH_AUTOMATIC,
			&raw mut supported,
		);
	}
	supported != 0
}

fn status_to_error(status: ffi::Status) -> Result<(), Error> {
	match status {
		ffi::SUCCESS => Ok(()),
		ffi::ERROR_CAMERA_PERMISSION_NOT_GRANTED => Err(Error::PermissionDenied),
		ffi::UNAVAILABLE_ARCORE_NOT_INSTALLED
		| ffi::UNAVAILABLE_APK_TOO_OLD
		| ffi::UNAVAILABLE_DEVICE_NOT_COMPATIBLE
		| ffi::UNAVAILABLE_USER_DECLINED_INSTALLATION => Err(Error::Unavailable),
		_ => Err(Error::Native),
	}
}

/// Reads an `ArPose` (`qx, qy, qz, qw, tx, ty, tz`).
fn read_pose(session: &Session, pose: *const ffi::Pose) -> Pose {
	let mut raw = [0.0_f32; 7];
	// SAFETY: `raw` has the seven floats ARCore writes.
	unsafe { (session.api.ArPose_getPoseRaw)(session.handle, pose, raw.as_mut_ptr()) };

	Pose::new(
		Vec3::new(raw[4], raw[5], raw[6]),
		Quat::new(raw[0], raw[1], raw[2], raw[3])
			.normalized()
			.unwrap_or(Quat::IDENTITY),
	)
}

/// An `ArPose` to read into or pass in; destroyed when dropped.
struct ScratchPose<'a>(&'a Session, *mut ffi::Pose);

impl<'a> ScratchPose<'a> {
	fn new(session: &'a Session, pose: Pose) -> Self {
		let Pose {
			position,
			orientation,
		} = pose;
		let raw = [
			orientation.x,
			orientation.y,
			orientation.z,
			orientation.w,
			position.x,
			position.y,
			position.z,
		];
		let mut handle = std::ptr::null_mut();
		// SAFETY: `raw` holds the seven floats ARCore reads.
		unsafe { (session.api.ArPose_create)(session.handle, raw.as_ptr(), &raw mut handle) };
		Self(session, handle)
	}

	fn read(&self) -> Pose {
		read_pose(self.0, self.1)
	}
}

impl Drop for ScratchPose<'_> {
	fn drop(&mut self) {
		// SAFETY: Frees the pose `new` made.
		unsafe { (self.0.api.ArPose_destroy)(self.1) };
	}
}

fn tracking(state: i32, reason: i32) -> TrackingState {
	match state {
		ffi::TRACKING_STATE_TRACKING => TrackingState::Normal,
		ffi::TRACKING_STATE_PAUSED => {
			TrackingState::Limited(match reason {
				ffi::FAILURE_BAD_STATE => LimitedReason::Initializing,
				ffi::FAILURE_INSUFFICIENT_LIGHT => LimitedReason::InsufficientLight,
				ffi::FAILURE_EXCESSIVE_MOTION => LimitedReason::ExcessiveMotion,
				ffi::FAILURE_INSUFFICIENT_FEATURES => LimitedReason::InsufficientFeatures,
				ffi::FAILURE_CAMERA_UNAVAILABLE => LimitedReason::CameraUnavailable,
				_ => LimitedReason::Unknown,
			})
		}
		_ => TrackingState::NotAvailable,
	}
}

mod frame;

impl Backend for ArCoreBackend {
	fn capabilities(&self) -> Capabilities {
		match self.availability() {
			None | Some(ffi::AVAILABILITY_UNSUPPORTED_DEVICE_NOT_CAPABLE) => Capabilities::NONE,
			Some(_) => {
				Capabilities::NONE.with(
					Mode::PhoneAr,
					Features::PLANES
						| Features::DEPTH
						| Features::LIGHT_ESTIMATION
						| Features::HIT_TEST
						| Features::ANCHORS
						| Features::CAMERA_IMAGE,
				)
			}
		}
	}

	fn prepare(&mut self, mode: Mode, install: bool) -> Availability {
		if mode != Mode::PhoneAr {
			return Availability::Unsupported;
		}

		let Some(platform) = self.platform.as_ref() else {
			return Availability::Unsupported;
		};

		match self.availability() {
			None | Some(ffi::AVAILABILITY_UNSUPPORTED_DEVICE_NOT_CAPABLE) => {
				return Availability::Unsupported;
			}
			Some(ffi::AVAILABILITY_UNKNOWN_CHECKING) => return Availability::Checking,
			Some(ffi::AVAILABILITY_SUPPORTED_INSTALLED) => {}
			Some(_) if install && java::request_install(platform) == Some(true) => {}
			Some(_) => return Availability::NeedsInstall,
		}

		if java::camera_granted(platform) == Some(true) {
			Availability::Ready
		} else {
			Availability::NeedsPermission
		}
	}

	fn start(&mut self, config: &SessionConfig, features: Features) -> Result<Started, Error> {
		if java::camera_granted(self.platform.as_ref().ok_or(Error::Unavailable)?) != Some(true) {
			return Err(Error::PermissionDenied);
		}

		let session = self.open(features)?;
		self.features = Self::granted(features, &session);
		self.session = Some(session);
		self.last_time = -1;
		// ARCore's origin is where tracking started; it has no floor origin.
		let space = match config.reference_space {
			ReferenceSpace::Local => ReferenceSpace::Local,
			_ => ReferenceSpace::World,
		};
		Ok(Started::new(space, self.features))
	}

	fn configure(&mut self, config: &SessionConfig, features: Features) -> Result<Started, Error> {
		let session = self.session.as_ref().ok_or(Error::Unavailable)?;
		self.apply(session, features)?;
		self.features = Self::granted(features, session);
		let space = match config.reference_space {
			ReferenceSpace::Local => ReferenceSpace::Local,
			_ => ReferenceSpace::World,
		};
		Ok(Started::new(space, self.features))
	}

	fn update(&mut self, _time: f64) -> Result<Option<XrFrame>, Error> {
		let session = self.session.as_ref().ok_or(Error::Unavailable)?;
		// SAFETY: Advances the session into its frame.
		let status = unsafe { (session.api.ArSession_update)(session.handle, session.frame) };

		match status {
			ffi::SUCCESS => {}
			ffi::ERROR_CAMERA_PERMISSION_NOT_GRANTED => return Err(Error::PermissionDenied),
			_ => return Ok(None),
		}

		let mut time = 0;
		// SAFETY: Reads the frame's capture time.
		unsafe { (session.api.ArFrame_getTimestamp)(session.handle, session.frame, &raw mut time) };

		if time == self.last_time {
			return Ok(None);
		}

		self.last_time = time;
		Ok(Some(self.frame(time)))
	}

	fn hit_test(&mut self, request: &HitTestRequest, frame: &XrFrame) -> Vec<Hit> {
		let Some(ray) = hit_test::request_ray(request.origin, &frame.views) else {
			return Vec::new();
		};
		let mut hits = self.native_hits(ray, request.targets);

		if request.targets.contains(HitTargets::INFINITE_PLANES) {
			hits.extend(hit_test::hit_planes(
				ray,
				&frame.planes,
				HitTargets::INFINITE_PLANES,
			));
		}

		hits
	}

	fn add_anchor(&mut self, pose: Pose, plane: Option<u64>) -> Result<u64, Error> {
		let session = self.session.as_ref().ok_or(Error::Unavailable)?;
		let trackable = plane.and_then(|id| {
			self.planes
				.values()
				.find(|known| known.id == id)
				.map(|known| known.trackable)
		});
		let scratch = ScratchPose::new(session, pose);
		let mut anchor = std::ptr::null_mut();
		// SAFETY: Makes an anchor at `pose`, on the plane when one is given.
		let status = unsafe {
			match trackable {
				Some(trackable) => {
					(session.api.ArTrackable_acquireNewAnchor)(
						session.handle,
						trackable,
						scratch.1,
						&raw mut anchor,
					)
				}
				None => {
					(session.api.ArSession_acquireNewAnchor)(
						session.handle,
						scratch.1,
						&raw mut anchor,
					)
				}
			}
		};
		status_to_error(status)?;
		self.next_anchor += 1;
		self.anchors.insert(self.next_anchor, anchor);
		Ok(self.next_anchor)
	}

	fn remove_anchor(&mut self, id: u64) -> Result<(), Error> {
		let session = self.session.as_ref().ok_or(Error::Unavailable)?;
		let anchor = self.anchors.remove(&id).ok_or(Error::InvalidArgument)?;
		// SAFETY: Detaches and releases an anchor this backend made.
		unsafe {
			(session.api.ArAnchor_detach)(session.handle, anchor);
			(session.api.ArAnchor_release)(anchor);
		}
		Ok(())
	}

	fn set_viewport(&mut self, viewport: Viewport) {
		self.viewport = viewport;

		if let Some(session) = &self.session {
			self.set_geometry(session);
		}
	}

	fn stop(&mut self) {
		self.release_anchors();

		if let Some(session) = self.session.take() {
			self.release_planes(session.api);
		}
	}
}

impl Drop for ArCoreBackend {
	fn drop(&mut self) {
		self.stop();
	}
}

/// Copies one plane of an `ArImage`: its bytes, row stride, and pixel
/// stride.
fn image_plane(
	session: &Session,
	image: *const ffi::Image,
	index: i32,
) -> Option<(&[u8], i32, i32)> {
	let (mut data, mut length, mut row, mut pixel) = (std::ptr::null(), 0, 0, 0);
	// SAFETY: Reads the plane's pointer and strides; the bytes live until
	// the image is released, after the caller copies them.
	unsafe {
		(session.api.ArImage_getPlaneData)(
			session.handle,
			image,
			index,
			&raw mut data,
			&raw mut length,
		);
		(session.api.ArImage_getPlaneRowStride)(session.handle, image, index, &raw mut row);
		(session.api.ArImage_getPlanePixelStride)(session.handle, image, index, &raw mut pixel);
	}

	if data.is_null() || length <= 0 {
		return None;
	}

	// SAFETY: ARCore reported `length` readable bytes at `data`.
	Some((
		unsafe { std::slice::from_raw_parts(data, length as usize) },
		row,
		pixel,
	))
}
