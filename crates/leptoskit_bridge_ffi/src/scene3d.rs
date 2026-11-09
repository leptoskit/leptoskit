//! 3D scenes in native surfaces: the C ABI the Swift and Kotlin renderers
//! use to show a `Scene3d` element.
//!
//! The guest's ops reach the scene through the `leptoskit.scene3d` plugin
//! ([`lk_app_enable_scene3d`]). A view attaches to a `CAMetalLayer` or an
//! `ANativeWindow`, draws on the platform's display link, and calls the
//! waker the host sets when the scene changes while the display link is
//! stopped.
//!
//! Without the `scene3d` feature every function is present, so the
//! renderers always link, but nothing attaches: [`lk_scene3d_available`]
//! reports 0 and attaching returns null.

use std::cell::RefCell;
use std::ffi::c_void;

use crate::Buffer;

thread_local! {
	static ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Records why a `lk_scene3d_*` call failed, for [`lk_scene3d_error`], and
/// logs it.
fn fail(message: impl Into<String>) {
	let message = message.into();
	log::error!(target: leptoskit_log::target::GPU, "{message}");
	ERROR.with_borrow_mut(|error| *error = message);
}

/// Borrows a UTF-8 string from C.
///
/// # Safety
/// `bytes` addresses `len` readable bytes, or is null with `len` 0.
unsafe fn text<'a>(bytes: *const u8, len: usize) -> Option<&'a str> {
	if len == 0 {
		return Some("");
	}

	if bytes.is_null() || len > 64 * 1024 {
		return None;
	}

	// SAFETY: The caller lends `len` readable bytes.
	std::str::from_utf8(unsafe { std::slice::from_raw_parts(bytes, len) }).ok()
}

/// The last error a `lk_scene3d_*` call on this thread reported, as UTF-8.
/// Release it with `lk_buffer_release`.
#[unsafe(no_mangle)]
pub extern "C" fn lk_scene3d_error() -> Buffer {
	Buffer::new(ERROR.with_borrow(|error| error.clone().into_bytes().into()))
}

/// Called with a context pointer when a view's scene wants a frame. It may
/// run on any thread and must only schedule work (start a display link),
/// never draw or call back into these functions.
pub type WakeFn = unsafe extern "C" fn(context: *mut c_void);

/// Reads an asset for the host: writes its bytes with
/// [`lk_scene3d_sink_write`] and returns 1, or returns 0 when it is missing.
/// Runs on the asset loader's thread.
pub type ReadFn = unsafe extern "C" fn(
	context: *mut c_void,
	path: *const u8,
	path_len: usize,
	sink: *mut c_void,
) -> i32;

/// Appends bytes to an asset a [`ReadFn`] is reading.
///
/// # Safety
/// `sink` is the pointer the read callback received, during that call;
/// `bytes` addresses `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_sink_write(sink: *mut c_void, bytes: *const u8, len: usize) {
	if sink.is_null() || (bytes.is_null() && len > 0) {
		return;
	}

	// SAFETY: The read callback lends the sink, a `Vec<u8>`, for its call.
	let sink = unsafe { &mut *sink.cast::<Vec<u8>>() };

	if len > 0 {
		// SAFETY: The caller lends `len` readable bytes.
		sink.extend_from_slice(unsafe { std::slice::from_raw_parts(bytes, len) });
	}
}

#[cfg(feature = "scene3d")]
mod enabled {
	use std::ffi::c_void;
	use std::ptr;
	use std::ptr::NonNull;

	use leptoskit_3d::Backend;
	use leptoskit_3d::SceneId;
	use leptoskit_3d::host::Directory;
	use leptoskit_3d::host::Hub;
	use leptoskit_3d::host::View;
	use leptoskit_3d::wgpu;

	use super::ReadFn;
	use super::WakeFn;
	use super::fail;

	/// A context and function the host lends for as long as the view or the
	/// reader lives, callable from any thread.
	struct Callback<F> {
		context: *mut c_void,
		function: F,
	}

	// SAFETY: The host contract makes the context and function valid from any
	// thread until the view is detached (or, for the reader, forever).
	unsafe impl<F> Send for Callback<F> {}
	// SAFETY: As above; the functions take no exclusive access.
	unsafe impl<F> Sync for Callback<F> {}

	impl Callback<ReadFn> {
		/// Reads `path` through the host's reader.
		fn read(&self, path: &str) -> Result<Vec<u8>, String> {
			let mut bytes = Vec::new();
			// SAFETY: The host lends the reader for the process's life; the
			// sink is this call's vector.
			let found = unsafe {
				(self.function)(
					self.context,
					path.as_ptr(),
					path.len(),
					ptr::from_mut(&mut bytes).cast(),
				)
			};

			if found == 1 {
				Ok(bytes)
			} else {
				Err(format!("{path} is not among the app's assets"))
			}
		}
	}

	impl Callback<WakeFn> {
		fn wake(&self) {
			// SAFETY: The host keeps the context valid until it detaches the
			// view, which drops this waker first.
			unsafe { (self.function)(self.context) };
		}
	}

	pub fn available() -> i32 {
		1
	}

	pub fn set_backend(name: &str) -> i32 {
		match Backend::from_name(name) {
			Some(backend) if Hub::global().set_backend(backend) => 1,
			Some(_) => {
				fail("the backend can only change before the first view attaches");
				0
			}
			None => {
				fail(format!("{name:?} is not a graphics backend"));
				-1
			}
		}
	}

	pub fn set_asset_directory(path: &str) -> i32 {
		Hub::global().set_asset_source(Directory(path.into()));
		1
	}

	pub fn set_asset_reader(context: *mut c_void, read: ReadFn) -> i32 {
		let reader = Callback {
			context,
			function: read,
		};
		Hub::global().set_asset_source(move |path: &str| reader.read(path));
		1
	}

	/// A view the host owns through an opaque pointer.
	pub struct ViewHandle(View);

	pub fn attach(
		scene: u32,
		target: wgpu::SurfaceTargetUnsafe,
		width: u32,
		height: u32,
	) -> *mut ViewHandle {
		let hub = Hub::global();
		// SAFETY: The host keeps the layer or window alive until it detaches
		// the view, which drops the surface first.
		let surface = match unsafe { hub.instance().create_surface_unsafe(target) } {
			Ok(surface) => surface,
			Err(error) => {
				fail(format!("scene {scene}: cannot create a surface: {error}"));
				return ptr::null_mut();
			}
		};

		match hub.attach(SceneId(scene), surface, width, height) {
			Ok(view) => {
				log::info!(
					target: leptoskit_log::target::GPU,
					"scene {scene}: created a {width}×{height} surface"
				);
				Box::into_raw(Box::new(ViewHandle(view)))
			}
			Err(error) => {
				fail(format!(
					"scene {scene}: cannot draw on a {width}×{height} surface: {error}"
				));
				ptr::null_mut()
			}
		}
	}

	pub fn layer_target(layer: *mut c_void) -> Option<wgpu::SurfaceTargetUnsafe> {
		#[cfg(target_vendor = "apple")]
		{
			(!layer.is_null()).then_some(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer))
		}
		#[cfg(not(target_vendor = "apple"))]
		{
			let _ = layer;
			None
		}
	}

	pub fn window_target(window: *mut c_void) -> Option<wgpu::SurfaceTargetUnsafe> {
		#[cfg(target_os = "android")]
		{
			use wgpu::rwh::AndroidDisplayHandle;
			use wgpu::rwh::AndroidNdkWindowHandle;
			use wgpu::rwh::RawDisplayHandle;
			use wgpu::rwh::RawWindowHandle;
			let window = NonNull::new(window)?;
			Some(wgpu::SurfaceTargetUnsafe::RawHandle {
				raw_display_handle: Some(RawDisplayHandle::Android(AndroidDisplayHandle::new())),
				raw_window_handle: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(window)),
			})
		}
		#[cfg(not(target_os = "android"))]
		{
			let _ = (window, NonNull::<c_void>::dangling());
			None
		}
	}

	pub fn set_waker(view: &mut ViewHandle, context: *mut c_void, wake: WakeFn) {
		let callback = Callback {
			context,
			function: wake,
		};
		view.0.set_waker(move || callback.wake());
	}

	pub fn resize(view: &mut ViewHandle, width: u32, height: u32) {
		log::debug!(
			target: leptoskit_log::target::GPU,
			"resized a surface to {width}×{height}"
		);
		view.0.resize(width, height);
	}

	pub fn set_rotation(view: &mut ViewHandle, quarter_turns: u32) {
		view.0.set_rotation(quarter_turns);
	}

	pub fn shows_camera(view: &ViewHandle) -> i32 {
		i32::from(view.0.shows_camera())
	}

	pub fn frame(view: &mut ViewHandle, time: f64) -> i32 {
		match view.0.frame(time) {
			Ok(more) => i32::from(more),
			Err(error) => {
				fail(format!("cannot draw a frame: {error}"));
				-1
			}
		}
	}

	pub fn needs_frame(view: &ViewHandle) -> i32 {
		i32::from(view.0.needs_frame())
	}
}

#[cfg(not(feature = "scene3d"))]
mod enabled {
	use std::ffi::c_void;

	use super::ReadFn;
	use super::WakeFn;
	use super::fail;

	const DISABLED: &str = "this app was built without the bridge's scene3d feature";

	pub fn available() -> i32 {
		0
	}

	pub fn set_backend(_: &str) -> i32 {
		fail(DISABLED);
		-1
	}

	pub fn set_asset_directory(_: &str) -> i32 {
		fail(DISABLED);
		-1
	}

	pub fn set_asset_reader(_: *mut c_void, _: ReadFn) -> i32 {
		fail(DISABLED);
		-1
	}

	/// Never constructed without the feature.
	pub enum ViewHandle {}

	pub fn layer_target(_: *mut c_void) -> Option<()> {
		fail(DISABLED);
		None
	}

	pub fn window_target(_: *mut c_void) -> Option<()> {
		fail(DISABLED);
		None
	}

	pub fn attach(_: u32, (): (), _: u32, _: u32) -> *mut ViewHandle {
		std::ptr::null_mut()
	}

	pub fn set_waker(view: &mut ViewHandle, _: *mut c_void, _: WakeFn) {
		match *view {}
	}

	pub fn resize(view: &mut ViewHandle, _: u32, _: u32) {
		match *view {}
	}

	pub fn set_rotation(view: &mut ViewHandle, _: u32) {
		match *view {}
	}

	pub fn shows_camera(view: &ViewHandle) -> i32 {
		match *view {}
	}

	pub fn frame(view: &mut ViewHandle, _: f64) -> i32 {
		match *view {}
	}

	pub fn needs_frame(view: &ViewHandle) -> i32 {
		match *view {}
	}
}

pub use enabled::ViewHandle;

/// 1 when this build can show 3D scenes, 0 when it was built without them.
#[unsafe(no_mangle)]
pub extern "C" fn lk_scene3d_available() -> i32 {
	enabled::available()
}

/// Chooses the graphics API by name (`auto`, `metal`, `vulkan`, `gl`).
/// Returns 1 when chosen, 0 when too late (a view has attached), -1 for an
/// unknown name or a build without 3D.
///
/// # Safety
/// `name` addresses `len` readable UTF-8 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_set_backend(name: *const u8, len: usize) -> i32 {
	// SAFETY: The caller lends the name.
	if let Some(name) = unsafe { text(name, len) } {
		enabled::set_backend(name)
	} else {
		fail("the backend name is not UTF-8");
		-1
	}
}

/// Reads assets from a folder, such as the app bundle's `assets`. Returns 1,
/// or -1 in a build without 3D.
///
/// # Safety
/// `path` addresses `len` readable UTF-8 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_set_asset_directory(path: *const u8, len: usize) -> i32 {
	// SAFETY: The caller lends the path.
	match unsafe { text(path, len) } {
		Some(path) if !path.is_empty() => enabled::set_asset_directory(path),
		_ => {
			fail("the asset directory is not a UTF-8 path");
			-1
		}
	}
}

/// Reads assets through `read`, such as Android's `AssetManager`. Returns 1,
/// or -1 in a build without 3D.
///
/// # Safety
/// `context` and `read` stay valid, from any thread, for the rest of the
/// process.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_set_asset_reader(
	context: *mut c_void,
	read: Option<ReadFn>,
) -> i32 {
	if let Some(read) = read {
		enabled::set_asset_reader(context, read)
	} else {
		fail("an asset reader is required");
		-1
	}
}

/// Shows `scene` in a `CAMetalLayer`, `width` by `height` pixels. Returns the
/// view, or null (see [`lk_scene3d_error`]).
///
/// # Safety
/// `layer` is a live `CAMetalLayer` that outlives the view; call only on the
/// main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_view_attach_layer(
	scene: u32,
	layer: *mut c_void,
	width: u32,
	height: u32,
) -> *mut ViewHandle {
	if let Some(target) = enabled::layer_target(layer) {
		enabled::attach(scene, target, width, height)
	} else {
		fail("a CAMetalLayer is required on Apple platforms");
		std::ptr::null_mut()
	}
}

/// Shows `scene` in an `ANativeWindow`, `width` by `height` pixels. Returns
/// the view, or null (see [`lk_scene3d_error`]).
///
/// # Safety
/// `window` is an acquired `ANativeWindow` that the caller releases only
/// after detaching the view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_view_attach_window(
	scene: u32,
	window: *mut c_void,
	width: u32,
	height: u32,
) -> *mut ViewHandle {
	if let Some(target) = enabled::window_target(window) {
		enabled::attach(scene, target, width, height)
	} else {
		fail("an ANativeWindow is required on Android");
		std::ptr::null_mut()
	}
}

/// Calls `wake(context)` whenever the view's scene changes, so the host can
/// start its display link.
///
/// # Safety
/// `view` is live; `context` and `wake` stay valid, from any thread, until
/// the view is detached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_view_set_waker(
	view: *mut ViewHandle,
	context: *mut c_void,
	wake: Option<WakeFn>,
) {
	// SAFETY: The caller lends a live view.
	if let (Some(view), Some(wake)) = (unsafe { view.as_mut() }, wake) {
		enabled::set_waker(view, context, wake);
	}
}

/// Resizes the view's drawing surface, in pixels.
///
/// # Safety
/// `view` is live; call on the thread that draws it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_view_resize(view: *mut ViewHandle, width: u32, height: u32) {
	// SAFETY: The caller lends a live view.
	if let Some(view) = unsafe { view.as_mut() } {
		enabled::resize(view, width, height);
	}
}

/// Tells the view how far the interface is turned from the device's natural
/// orientation, in clockwise quarter turns: Android's `Display.getRotation()`,
/// or on iOS 0 for portrait, 1 landscape right, 2 upside down, 3 landscape
/// left. A scene drawn over the camera needs it to turn the picture upright.
///
/// # Safety
/// `view` is live; call on the thread that draws it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_view_set_rotation(view: *mut ViewHandle, quarter_turns: u32) {
	// SAFETY: The caller lends a live view.
	if let Some(view) = unsafe { view.as_mut() } {
		enabled::set_rotation(view, quarter_turns);
	}
}

/// 1 when the view's last frame drew a live camera picture, which covers the
/// whole view, 0 otherwise. An Android host composites such a view below the
/// window's views instead of above them.
///
/// # Safety
/// `view` is null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_view_shows_camera(view: *const ViewHandle) -> i32 {
	// SAFETY: The caller lends a live view or null.
	unsafe { view.as_ref() }.map_or(0, enabled::shows_camera)
}

/// Draws a frame for the display link's timestamp, in seconds. Returns 1
/// while the scene wants more frames, 0 when it is still, -1 on failure.
///
/// # Safety
/// `view` is live; call on the thread that draws it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_view_frame(view: *mut ViewHandle, time: f64) -> i32 {
	// SAFETY: The caller lends a live view.
	match unsafe { view.as_mut() } {
		Some(view) if time.is_finite() => enabled::frame(view, time),
		_ => -1,
	}
}

/// 1 when the view's scene has changed or animates, so the display link
/// should run.
///
/// # Safety
/// `view` is null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_view_needs_frame(view: *const ViewHandle) -> i32 {
	// SAFETY: The caller lends a live view or null.
	unsafe { view.as_ref() }.map_or(0, enabled::needs_frame)
}

/// Stops showing the view and lets its surface go.
///
/// # Safety
/// `view` is null or a view from an attach call, detached once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lk_scene3d_view_detach(view: *mut ViewHandle) {
	if !view.is_null() {
		log::info!(target: leptoskit_log::target::GPU, "released a surface");
		// SAFETY: The caller transfers the view back exactly once.
		drop(unsafe { Box::from_raw(view) });
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sinks_collect_what_readers_write() {
		let mut bytes: Vec<u8> = Vec::new();
		let sink: *mut c_void = std::ptr::from_mut(&mut bytes).cast();
		// SAFETY: The sink is a live vector; the bytes are a live slice.
		unsafe {
			lk_scene3d_sink_write(sink, b"glTF".as_ptr(), 4);
			lk_scene3d_sink_write(sink, b"!".as_ptr(), 1);
			lk_scene3d_sink_write(sink, std::ptr::null(), 0);
			lk_scene3d_sink_write(std::ptr::null_mut(), b"x".as_ptr(), 1);
		}
		assert_eq!(bytes, b"glTF!");
	}

	#[test]
	fn bad_arguments_fail_without_attaching() {
		// SAFETY: Null pointers and empty names are the cases under test.
		unsafe {
			assert!(lk_scene3d_view_attach_layer(1, std::ptr::null_mut(), 10, 10).is_null());
			assert!(lk_scene3d_view_attach_window(1, std::ptr::null_mut(), 10, 10).is_null());
			assert_eq!(lk_scene3d_view_frame(std::ptr::null_mut(), 0.0), -1);
			assert_eq!(lk_scene3d_view_needs_frame(std::ptr::null()), 0);
			assert_eq!(lk_scene3d_view_shows_camera(std::ptr::null()), 0);
			lk_scene3d_view_set_rotation(std::ptr::null_mut(), 1);
			lk_scene3d_view_detach(std::ptr::null_mut());
			lk_scene3d_view_resize(std::ptr::null_mut(), 1, 1);
			assert_eq!(lk_scene3d_set_asset_directory(std::ptr::null(), 0), -1);
			assert_eq!(lk_scene3d_set_asset_reader(std::ptr::null_mut(), None), -1);
			assert_eq!(lk_scene3d_set_backend(b"glide".as_ptr(), 5), -1);
		}
		let error = lk_scene3d_error();
		assert!(error.len > 0);
		// SAFETY: The buffer was just returned and is released once.
		unsafe { crate::lk_buffer_release(error) };
	}

	#[cfg(feature = "scene3d")]
	#[test]
	fn the_feature_reports_itself_and_parses_backends() {
		assert_eq!(lk_scene3d_available(), 1);
		// SAFETY: Live name bytes.
		assert_eq!(unsafe { lk_scene3d_set_backend(b"auto".as_ptr(), 4) }, 1);
	}
}
