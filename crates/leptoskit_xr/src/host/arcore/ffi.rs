//! The parts of ARCore's C API (`arcore_c_api.h`) this backend calls.
//!
//! `libarcore_sdk_c.so` comes with the `com.google.ar:core` library the
//! plugin's metadata adds to the app. It is opened on first use, so an app
//! on a device without Google Play Services for AR still starts, and AR
//! reports itself unavailable.

#![allow(non_snake_case, non_camel_case_types)]

use std::ffi::c_void;
use std::sync::OnceLock;

/// Opaque ARCore handles.
pub(super) type Session = c_void;
pub(super) type Config = c_void;
pub(super) type Frame = c_void;
pub(super) type Camera = c_void;
pub(super) type Pose = c_void;
pub(super) type Trackable = c_void;
pub(super) type TrackableList = c_void;
pub(super) type Anchor = c_void;
pub(super) type HitResult = c_void;

pub(super) type HitResultList = c_void;
pub(super) type LightEstimate = c_void;
pub(super) type Image = c_void;

/// `ArStatus`.
pub(super) type Status = i32;
pub(super) const SUCCESS: Status = 0;
pub(super) const ERROR_CAMERA_PERMISSION_NOT_GRANTED: Status = -9;
pub(super) const UNAVAILABLE_ARCORE_NOT_INSTALLED: Status = -100;
pub(super) const UNAVAILABLE_DEVICE_NOT_COMPATIBLE: Status = -101;
pub(super) const UNAVAILABLE_APK_TOO_OLD: Status = -103;
pub(super) const UNAVAILABLE_USER_DECLINED_INSTALLATION: Status = -105;

/// `ArAvailability`.
pub(super) const AVAILABILITY_UNKNOWN_CHECKING: i32 = 1;
pub(super) const AVAILABILITY_UNSUPPORTED_DEVICE_NOT_CAPABLE: i32 = 100;
pub(super) const AVAILABILITY_SUPPORTED_INSTALLED: i32 = 203;
/// `ArInstallStatus`.
pub(super) const INSTALL_STATUS_INSTALLED: i32 = 0;

/// `ArTrackingState`.
pub(super) const TRACKING_STATE_TRACKING: i32 = 0;
pub(super) const TRACKING_STATE_PAUSED: i32 = 1;
/// `ArTrackingFailureReason`.
pub(super) const FAILURE_BAD_STATE: i32 = 1;
pub(super) const FAILURE_INSUFFICIENT_LIGHT: i32 = 2;
pub(super) const FAILURE_EXCESSIVE_MOTION: i32 = 3;
pub(super) const FAILURE_INSUFFICIENT_FEATURES: i32 = 4;
pub(super) const FAILURE_CAMERA_UNAVAILABLE: i32 = 5;

/// `ArTrackableType`.
pub(super) const TRACKABLE_PLANE: i32 = 0x4152_0101;
pub(super) const TRACKABLE_DEPTH_POINT: i32 = 0x4152_0111;

/// `ArPlaneType`.
pub(super) const PLANE_HORIZONTAL_UPWARD_FACING: i32 = 0;
pub(super) const PLANE_HORIZONTAL_DOWNWARD_FACING: i32 = 1;

/// `ArPlaneFindingMode`.
pub(super) const PLANE_FINDING_DISABLED: i32 = 0;
pub(super) const PLANE_FINDING_HORIZONTAL: i32 = 1;
pub(super) const PLANE_FINDING_VERTICAL: i32 = 2;
pub(super) const PLANE_FINDING_HORIZONTAL_AND_VERTICAL: i32 = 3;
/// `ArLightEstimationMode`.
pub(super) const LIGHT_ESTIMATION_DISABLED: i32 = 0;
pub(super) const LIGHT_ESTIMATION_AMBIENT_INTENSITY: i32 = 1;
/// `ArLightEstimateState`.
pub(super) const LIGHT_ESTIMATE_VALID: i32 = 1;
/// `ArDepthMode`.
pub(super) const DEPTH_DISABLED: i32 = 0;
pub(super) const DEPTH_AUTOMATIC: i32 = 1;
/// `ArUpdateMode`: `ArSession_update` returns the newest image at once.
pub(super) const UPDATE_LATEST_CAMERA_IMAGE: i32 = 1;
/// `ArFocusMode`.
pub(super) const FOCUS_AUTO: i32 = 1;
/// `ArTextureUpdateMode`: the camera image is exposed as an
/// `AHardwareBuffer`, so `ArSession_update` needs no OpenGL context.
pub(super) const TEXTURE_EXPOSE_HARDWARE_BUFFER: i32 = 1;

/// `ArCoordinates2dType`.
pub(super) const COORDINATES_IMAGE_NORMALIZED: i32 = 3;
pub(super) const COORDINATES_VIEW_NORMALIZED: i32 = 8;

macro_rules! api {
	($($name:ident: fn($($argument:ty),* $(,)?) $(-> $ret:ty)?;)*) => {
		/// ARCore's functions, resolved from `libarcore_sdk_c.so`.
		pub(super) struct Api {
			_library: libloading::Library,
			$(pub(super) $name: unsafe extern "C" fn($($argument),*) $(-> $ret)?,)*
		}

		impl Api {
			fn load() -> Option<Self> {
				// SAFETY: Opens Google's ARCore client library and resolves
				// functions whose C signatures `arcore_c_api.h` declares as
				// written here.
				unsafe {
					let library = libloading::Library::new("libarcore_sdk_c.so").ok()?;
					$(
						let $name = *library
							.get::<unsafe extern "C" fn($($argument),*) $(-> $ret)?>(
								concat!(stringify!($name), "\0").as_bytes(),
							)
							.ok()?;
					)*
					Some(Self { _library: library, $($name,)* })
				}
			}
		}
	};
}

api! {
	ArCoreApk_checkAvailability: fn(*mut c_void, *mut c_void, *mut i32);
	ArCoreApk_requestInstall: fn(*mut c_void, *mut c_void, i32, *mut i32) -> Status;
	ArSession_create: fn(*mut c_void, *mut c_void, *mut *mut Session) -> Status;
	ArSession_destroy: fn(*mut Session);
	ArSession_configure: fn(*mut Session, *const Config) -> Status;
	ArSession_resume: fn(*mut Session) -> Status;
	ArSession_pause: fn(*mut Session) -> Status;
	ArSession_update: fn(*mut Session, *mut Frame) -> Status;
	ArSession_setDisplayGeometry: fn(*mut Session, i32, i32, i32);
	ArSession_isDepthModeSupported: fn(*const Session, i32, *mut i32);
	ArSession_getAllTrackables: fn(*const Session, i32, *mut TrackableList);
	ArSession_acquireNewAnchor: fn(*mut Session, *const Pose, *mut *mut Anchor) -> Status;
	ArConfig_create: fn(*const Session, *mut *mut Config);
	ArConfig_destroy: fn(*mut Config);
	ArConfig_setPlaneFindingMode: fn(*const Session, *mut Config, i32);
	ArConfig_setLightEstimationMode: fn(*const Session, *mut Config, i32);
	ArConfig_setDepthMode: fn(*const Session, *mut Config, i32);
	ArConfig_setUpdateMode: fn(*const Session, *mut Config, i32);
	ArConfig_setFocusMode: fn(*const Session, *mut Config, i32);
	ArConfig_setTextureUpdateMode: fn(*const Session, *mut Config, i32);
	ArFrame_create: fn(*const Session, *mut *mut Frame);
	ArFrame_destroy: fn(*mut Frame);
	ArFrame_getTimestamp: fn(*const Session, *const Frame, *mut i64);
	ArFrame_acquireCamera: fn(*const Session, *const Frame, *mut *mut Camera);
	ArFrame_transformCoordinates2d: fn(*const Session, *const Frame, i32, i32, *const f32, i32, *mut f32);
	ArFrame_getLightEstimate: fn(*const Session, *const Frame, *mut LightEstimate);
	ArFrame_acquireDepthImage16Bits: fn(*const Session, *const Frame, *mut *mut Image) -> Status;
	ArFrame_acquireCameraImage: fn(*mut Session, *mut Frame, *mut *mut Image) -> Status;
	ArFrame_hitTestRay: fn(*const Session, *const Frame, *const f32, *const f32, *mut HitResultList);
	ArCamera_release: fn(*mut Camera);
	ArCamera_getTrackingState: fn(*const Session, *const Camera, *mut i32);
	ArCamera_getTrackingFailureReason: fn(*const Session, *const Camera, *mut i32);
	ArCamera_getDisplayOrientedPose: fn(*const Session, *const Camera, *mut Pose);
	ArCamera_getProjectionMatrix: fn(*const Session, *const Camera, f32, f32, *mut f32);
	ArPose_create: fn(*const Session, *const f32, *mut *mut Pose) -> Status;
	ArPose_destroy: fn(*mut Pose);
	ArPose_getPoseRaw: fn(*const Session, *const Pose, *mut f32);
	ArTrackableList_create: fn(*const Session, *mut *mut TrackableList);
	ArTrackableList_destroy: fn(*mut TrackableList);
	ArTrackableList_getSize: fn(*const Session, *const TrackableList, *mut i32);
	ArTrackableList_acquireItem: fn(*const Session, *const TrackableList, i32, *mut *mut Trackable);
	ArTrackable_release: fn(*mut Trackable);
	ArTrackable_getType: fn(*const Session, *const Trackable, *mut i32);
	ArTrackable_getTrackingState: fn(*const Session, *const Trackable, *mut i32);
	ArTrackable_acquireNewAnchor: fn(*mut Session, *mut Trackable, *mut Pose, *mut *mut Anchor) -> Status;
	ArPlane_acquireSubsumedBy: fn(*const Session, *const Trackable, *mut *mut Trackable);
	ArPlane_getType: fn(*const Session, *const Trackable, *mut i32);
	ArPlane_getCenterPose: fn(*const Session, *const Trackable, *mut Pose);
	ArPlane_getExtentX: fn(*const Session, *const Trackable, *mut f32);
	ArPlane_getExtentZ: fn(*const Session, *const Trackable, *mut f32);
	ArPlane_getPolygonSize: fn(*const Session, *const Trackable, *mut i32);
	ArPlane_getPolygon: fn(*const Session, *const Trackable, *mut f32);
	ArAnchor_getPose: fn(*const Session, *const Anchor, *mut Pose);
	ArAnchor_getTrackingState: fn(*const Session, *const Anchor, *mut i32);
	ArAnchor_detach: fn(*mut Session, *mut Anchor);
	ArAnchor_release: fn(*mut Anchor);
	ArHitResultList_create: fn(*const Session, *mut *mut HitResultList);
	ArHitResultList_destroy: fn(*mut HitResultList);
	ArHitResultList_getSize: fn(*const Session, *const HitResultList, *mut i32);
	ArHitResultList_getItem: fn(*const Session, *const HitResultList, i32, *mut HitResult);
	ArHitResult_create: fn(*const Session, *mut *mut HitResult);
	ArHitResult_destroy: fn(*mut HitResult);
	ArHitResult_getDistance: fn(*const Session, *const HitResult, *mut f32);
	ArHitResult_getHitPose: fn(*const Session, *const HitResult, *mut Pose);
	ArHitResult_acquireTrackable: fn(*const Session, *const HitResult, *mut *mut Trackable);
	ArLightEstimate_create: fn(*const Session, *mut *mut LightEstimate);
	ArLightEstimate_destroy: fn(*mut LightEstimate);
	ArLightEstimate_getState: fn(*const Session, *const LightEstimate, *mut i32);
	ArLightEstimate_getPixelIntensity: fn(*const Session, *const LightEstimate, *mut f32);
	ArImage_release: fn(*mut Image);
	ArImage_getWidth: fn(*const Session, *const Image, *mut i32);
	ArImage_getHeight: fn(*const Session, *const Image, *mut i32);
	ArImage_getPlaneRowStride: fn(*const Session, *const Image, i32, *mut i32);
	ArImage_getPlanePixelStride: fn(*const Session, *const Image, i32, *mut i32);
	ArImage_getPlaneData: fn(*const Session, *const Image, i32, *mut *const u8, *mut i32);
}

/// ARCore, or `None` when the library is missing.
pub(super) fn api() -> Option<&'static Api> {
	static API: OnceLock<Option<Api>> = OnceLock::new();
	API.get_or_init(Api::load).as_ref()
}
