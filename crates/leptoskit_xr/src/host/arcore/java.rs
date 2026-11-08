//! The Java calls ARCore needs around it: the calling thread's `JNIEnv`, the
//! camera permission, and installing Google Play Services for AR.

use std::ffi::c_void;

use jni::JNIEnv;
use jni::JavaVM;
use jni::objects::JObject;
use jni::objects::JValue;
use leptoskit_bridge::PlatformContext;

use super::ffi;

/// `PackageManager.PERMISSION_GRANTED`.
const PERMISSION_GRANTED: i32 = 0;

/// Runs `body` with this thread's `JNIEnv` and the app's `Context`, attaching
/// the thread to the VM for the rest of its life if it isn't already: the
/// runtime's threads call ARCore again every frame.
pub(super) fn with_context<T>(
	platform: &PlatformContext,
	body: impl FnOnce(*mut c_void, *mut c_void) -> T,
) -> Option<T> {
	let (vm, context) = platform.android_handles()?;
	// SAFETY: The host lends its process `JavaVM*` for the lifetime of the
	// platform context.
	let vm = unsafe { JavaVM::from_raw(vm.cast()) }.ok()?;
	let env = vm.attach_current_thread_as_daemon().ok()?;

	Some(body(env.get_raw().cast(), context))
}

fn with_env<T>(
	platform: &PlatformContext,
	body: impl FnOnce(&mut JNIEnv<'_>, &JObject<'_>) -> Option<T>,
) -> Option<T> {
	let (vm, context) = platform.android_handles()?;
	// SAFETY: As in `with_context`.
	let vm = unsafe { JavaVM::from_raw(vm.cast()) }.ok()?;
	let mut env = vm.attach_current_thread_as_daemon().ok()?;
	// SAFETY: `context` is the host's global reference, kept alive by the
	// platform context; `JObject` borrows it without deleting it.
	let context = unsafe { JObject::from_raw(context.cast()) };
	let result = env
		.with_local_frame(8, |env| Ok::<_, jni::errors::Error>(body(env, &context)))
		.ok()
		.flatten();

	if env.exception_check().unwrap_or(false) {
		let _ = env.exception_clear();
	}

	result
}

/// Whether the user allowed the camera, or `None` when it can't be told.
pub(super) fn camera_granted(platform: &PlatformContext) -> Option<bool> {
	with_env(platform, |env, context| {
		let permission = env.new_string("android.permission.CAMERA").ok()?;
		let result = env
			.call_method(
				context,
				"checkSelfPermission",
				"(Ljava/lang/String;)I",
				&[JValue::Object(&permission)],
			)
			.ok()?
			.i()
			.ok()?;
		Some(result == PERMISSION_GRANTED)
	})
}

/// Asks Google Play Services for AR to install or update itself, which opens
/// the store over the app. `Some(true)` once it is installed; `Some(false)`
/// while the install is under way, after which the app resumes and asks
/// again. Needs an `Activity` context.
pub(super) fn request_install(platform: &PlatformContext) -> Option<bool> {
	let api = ffi::api()?;
	let is_activity = with_env(platform, |env, context| {
		env.is_instance_of(context, "android/app/Activity").ok()
	})?;

	if !is_activity {
		return None;
	}

	with_context(platform, |env, activity| {
		let mut status = 0;
		// SAFETY: `activity` is the host's `Activity`, on an attached thread.
		let result = unsafe { (api.ArCoreApk_requestInstall)(env, activity, 1, &raw mut status) };
		(result == ffi::SUCCESS).then_some(status == ffi::INSTALL_STATUS_INSTALLED)
	})
	.flatten()
}
