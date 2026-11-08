//! The app's bundled `assets/` folder, and the device's log: what loading the
//! depth model named by `[native.plugin-options.xr] depth-model` needs.
//!
//! `leptoskit build` copies the app's `assets/` folder into every native app:
//! the iOS bundle's `assets/`, the APK's `assets/assets/`, and an `assets/`
//! folder beside the Windows and Linux executable.

use leptoskit_bridge::PlatformContext;

/// The most bytes read from one asset: 64 MiB, more than twice the depth
/// model.
const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;

/// Fails unless `path` names a file inside the assets folder: relative,
/// separated by `/`, without empty, `.`, or `..` segments.
pub(crate) fn check_path(path: &str) -> Result<(), &'static str> {
	if path.is_empty() {
		Err("it names no file")
	} else if path.contains(['\\', '\0']) {
		Err("separate folders with `/`")
	} else if path
		.split('/')
		.any(|segment| matches!(segment, "" | "." | ".."))
	{
		Err("it must be relative to the assets folder, without empty, `.` or `..` segments")
	} else {
		Ok(())
	}
}

/// Reads the whole asset at `path`, which [`check_path`] accepted. Android
/// reads it from the APK through the app's `AssetManager`, which needs
/// `platform`; everyone else from the folder beside the executable.
pub(crate) fn read(platform: Option<&PlatformContext>, path: &str) -> Result<Vec<u8>, String> {
	#[cfg(target_os = "android")]
	{
		android::read(platform, path)
	}
	#[cfg(not(target_os = "android"))]
	{
		let _ = platform;
		let file = folder()?.join(path);
		let size = std::fs::metadata(&file)
			.map_err(|error| format!("{} can't be read: {error}", file.display()))?
			.len();

		if size > MAX_ASSET_BYTES as u64 {
			return Err(format!(
				"{} is {size} bytes, more than the {MAX_ASSET_BYTES} an asset may be",
				file.display()
			));
		}

		std::fs::read(&file).map_err(|error| format!("{} can't be read: {error}", file.display()))
	}
}

/// The bundled assets folder: `Contents/Resources/assets` in a macOS app,
/// and `assets` beside the executable everywhere else, which on iOS is the
/// app bundle's own.
#[cfg(not(target_os = "android"))]
fn folder() -> Result<std::path::PathBuf, String> {
	let executable = std::env::current_exe()
		.map_err(|error| format!("the app's executable can't be found: {error}"))?;
	let directory = executable
		.parent()
		.ok_or("the app's executable has no folder")?;

	Ok(match directory.parent() {
		Some(contents) if cfg!(target_os = "macos") && directory.ends_with("Contents/MacOS") => {
			contents.join("Resources/assets")
		}
		_ => directory.join("assets"),
	})
}

/// Writes an informational line to the device's log: logcat on Android
/// (tag `leptoskit-xr`), standard error elsewhere, which Xcode shows.
pub(crate) fn info(message: &str) {
	write(4, message);
}

/// Writes a warning to the device's log, as [`info`] does.
pub(crate) fn warn(message: &str) {
	write(5, message);
}

/// `priority` is Android's: 4 for information, 5 for a warning.
fn write(priority: i32, message: &str) {
	#[cfg(target_os = "android")]
	android::log(priority, message);
	#[cfg(not(target_os = "android"))]
	{
		let level = if priority >= 5 { "warning: " } else { "" };
		eprintln!("leptoskit-xr: {level}{message}");
	}
}

#[cfg(target_os = "android")]
#[allow(unsafe_code)]
mod android {
	use std::ffi::CString;
	use std::ffi::c_char;

	use jni::JNIEnv;
	use jni::JavaVM;
	use jni::objects::JObject;
	use jni::objects::JValue;
	use jni::objects::JValueGen;
	use leptoskit_bridge::PlatformContext;

	use super::MAX_ASSET_BYTES;

	/// Bytes read from the asset's stream at a time.
	const CHUNK: i32 = 1024 * 1024;

	#[link(name = "log")]
	unsafe extern "C" {
		fn __android_log_write(priority: i32, tag: *const c_char, text: *const c_char) -> i32;
	}

	pub(super) fn log(priority: i32, message: &str) {
		let Ok(text) = CString::new(message.replace('\0', " ")) else {
			return;
		};
		// SAFETY: Both pointers are NUL-terminated strings that outlive the
		// call.
		unsafe {
			__android_log_write(priority, c"leptoskit-xr".as_ptr(), text.as_ptr());
		}
	}

	/// Reads `assets/<path>` from the APK, where `leptoskit build` packs the
	/// app's `assets/` folder, on the calling thread, which is attached to
	/// the VM for the read.
	pub(super) fn read(platform: Option<&PlatformContext>, path: &str) -> Result<Vec<u8>, String> {
		let (vm, context) = platform
			.and_then(PlatformContext::android_handles)
			.ok_or("the app's Android context is missing")?;
		// SAFETY: The host lends its process `JavaVM*` for the lifetime of the
		// platform context, which the caller holds.
		let vm = unsafe { JavaVM::from_raw(vm.cast()) }
			.map_err(|error| format!("the Java VM can't be reached: {error}"))?;
		let mut env = vm
			.attach_current_thread()
			.map_err(|error| format!("the thread can't attach to the Java VM: {error}"))?;
		// SAFETY: `context` is the host's global reference, kept alive by the
		// platform context; `JObject` borrows it without deleting it.
		let context = unsafe { JObject::from_raw(context.cast()) };
		let name = format!("assets/{path}");

		env.with_local_frame(16, |env| {
			Ok::<_, jni::errors::Error>(read_asset(env, &context, &name))
		})
		.map_err(|error| format!("the APK's {name} can't be read: {error}"))?
	}

	/// Reads through `AssetManager.open`. Every JNI step can throw; a failure
	/// clears the Java exception before returning.
	fn read_asset(
		env: &mut JNIEnv<'_>,
		context: &JObject<'_>,
		name: &str,
	) -> Result<Vec<u8>, String> {
		let failed = |env: &mut JNIEnv<'_>, what: &str| {
			let _ = env.exception_clear();
			format!("the APK's {name} can't be read: {what}")
		};
		let assets = env
			.call_method(
				context,
				"getAssets",
				"()Landroid/content/res/AssetManager;",
				&[],
			)
			.and_then(JValueGen::l)
			.map_err(|_| failed(env, "the app has no asset manager"))?;
		let java_name = env
			.new_string(name)
			.map_err(|_| failed(env, "its name can't be passed to Java"))?;
		let stream = env
			.call_method(
				&assets,
				"open",
				"(Ljava/lang/String;)Ljava/io/InputStream;",
				&[JValue::Object(&java_name)],
			)
			.and_then(JValueGen::l)
			.map_err(|_| failed(env, "it is not in the APK"))?;
		let chunk = env
			.new_byte_array(CHUNK)
			.map_err(|_| failed(env, "no memory for a buffer"))?;
		let mut buffer = vec![0_i8; CHUNK as usize];
		let mut bytes = Vec::new();

		let result = loop {
			let Ok(read) = env
				.call_method(&stream, "read", "([B)I", &[JValue::Object(&chunk)])
				.and_then(JValueGen::i)
			else {
				break Err(failed(env, "reading it failed"));
			};

			if read < 0 {
				break Ok(());
			}

			let read = read as usize;

			if bytes.len() + read > MAX_ASSET_BYTES {
				break Err(format!(
					"the APK's {name} is more than the {MAX_ASSET_BYTES} bytes an asset may be"
				));
			}

			if env
				.get_byte_array_region(&chunk, 0, &mut buffer[..read])
				.is_err()
			{
				break Err(failed(env, "copying it out of Java failed"));
			}

			bytes.extend(buffer[..read].iter().map(|&byte| byte as u8));
		};

		let _ = env.call_method(&stream, "close", "()V", &[]);
		let _ = env.exception_clear();
		result.map(|()| bytes)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_paths_inside_the_folder_are_read() {
		assert_eq!(
			check_path("depth/depth_anything_v2_metric_indoor_small.onnx"),
			Ok(())
		);

		for path in [
			"",
			"/depth/model.onnx",
			"depth//model.onnx",
			"./model.onnx",
			"../model.onnx",
			"depth\\model.onnx",
			"model\0.onnx",
		] {
			assert!(check_path(path).is_err(), "{path:?}");
		}
	}

	#[test]
	fn a_missing_asset_says_where_it_was_looked_for() {
		let error = read(None, "depth/missing.onnx").unwrap_err();
		assert!(error.contains("assets/depth/missing.onnx"), "{error}");
	}
}
