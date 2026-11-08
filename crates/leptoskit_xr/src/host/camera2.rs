//! The back camera through the NDK's Camera2 (`libcamera2ndk.so`) and an
//! `AImageReader` (`libmediandk.so`), for orientation sessions on phones
//! without ARCore, such as the Solana Seeker.
//!
//! Both libraries are opened at run time, like ARCore, so the app starts
//! on any device. The camera fills a small queue of YUV 4:2:0 images, which
//! [`CameraSource::latest`] polls without a listener or looper and copies
//! into luma and interleaved chroma for the renderer, as ARCore's CPU image
//! is.

#![allow(non_snake_case)]

use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::c_char;
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::OnceLock;

use leptoskit_bridge::Error;

use super::CameraSource;
use super::CapturedImage;
use super::Intrinsics;
use crate::image::ycbcr_from_yuv420;

/// The most pixels a stream may have: enough for a sharp background on a
/// phone, small enough to copy every frame.
const MAX_PIXELS: u32 = 1280 * 960;
/// `ACAMERA_OK` and `AMEDIA_OK`.
const OK: i32 = 0;
/// `ACAMERA_ERROR_PERMISSION_DENIED`.
const PERMISSION_DENIED: i32 = -10013;
/// `AIMAGE_FORMAT_YUV_420_888`.
const YUV_420_888: i32 = 0x23;
/// `TEMPLATE_PREVIEW`.
const TEMPLATE_PREVIEW: i32 = 1;
/// Images the reader holds: one being read, one being filled.
const MAX_IMAGES: i32 = 2;

/// Camera metadata tags (`NdkCameraMetadataTags.h`): a section's index
/// shifted 16 bits, plus the tag's.
const LENS_FACING: u32 = (8 << 16) + 5;
const LENS_INTRINSIC_CALIBRATION: u32 = (8 << 16) + 10;
const LENS_INFO_AVAILABLE_FOCAL_LENGTHS: u32 = (9 << 16) + 2;
const SCALER_AVAILABLE_STREAM_CONFIGURATIONS: u32 = (13 << 16) + 10;
const SENSOR_ORIENTATION: u32 = (14 << 16) + 14;
const SENSOR_INFO_ACTIVE_ARRAY_SIZE: u32 = 15 << 16;
const SENSOR_INFO_PHYSICAL_SIZE: u32 = (15 << 16) + 5;
const SENSOR_INFO_PIXEL_ARRAY_SIZE: u32 = (15 << 16) + 6;
/// `ACAMERA_LENS_FACING_BACK`.
const FACING_BACK: u8 = 1;
/// `ACAMERA_SCALER_AVAILABLE_STREAM_CONFIGURATIONS_OUTPUT`.
const STREAM_OUTPUT: i32 = 0;

/// `ACameraMetadata_const_entry`.
#[repr(C)]
struct Entry {
	tag: u32,
	kind: u8,
	count: u32,
	data: *const c_void,
}

/// `ACameraIdList`.
#[repr(C)]
struct IdList {
	count: i32,
	ids: *const *const c_char,
}

type DeviceState = unsafe extern "C" fn(*mut c_void, *mut c_void);
type DeviceError = unsafe extern "C" fn(*mut c_void, *mut c_void, i32);
type SessionState = unsafe extern "C" fn(*mut c_void, *mut c_void);

/// `ACameraDevice_StateCallbacks`.
#[repr(C)]
struct DeviceCallbacks {
	context: *mut c_void,
	disconnected: DeviceState,
	error: DeviceError,
}

/// `ACameraCaptureSession_stateCallbacks`.
#[repr(C)]
struct SessionCallbacks {
	context: *mut c_void,
	closed: SessionState,
	ready: SessionState,
	active: SessionState,
}

unsafe extern "C" fn ignore_state(_: *mut c_void, _: *mut c_void) {}
unsafe extern "C" fn ignore_error(_: *mut c_void, _: *mut c_void, _: i32) {}

macro_rules! library {
	($api:ident, $file:literal, $($name:ident: fn($($argument:ty),* $(,)?) $(-> $ret:ty)?;)*) => {
		struct $api {
			_library: libloading::Library,
			$($name: unsafe extern "C" fn($($argument),*) $(-> $ret)?,)*
		}

		impl $api {
			fn load() -> Option<Self> {
				// SAFETY: Opens an NDK library and resolves functions whose C
				// signatures its headers declare as written here.
				unsafe {
					let library = libloading::Library::new($file).ok()?;
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

library! {
	Camera, "libcamera2ndk.so",
	ACameraManager_create: fn() -> *mut c_void;
	ACameraManager_delete: fn(*mut c_void);
	ACameraManager_getCameraIdList: fn(*mut c_void, *mut *mut IdList) -> i32;
	ACameraManager_deleteCameraIdList: fn(*mut IdList);
	ACameraManager_getCameraCharacteristics: fn(*mut c_void, *const c_char, *mut *mut c_void) -> i32;
	ACameraManager_openCamera: fn(*mut c_void, *const c_char, *mut DeviceCallbacks, *mut *mut c_void) -> i32;
	ACameraMetadata_free: fn(*mut c_void);
	ACameraMetadata_getConstEntry: fn(*const c_void, u32, *mut Entry) -> i32;
	ACameraDevice_close: fn(*mut c_void) -> i32;
	ACameraDevice_createCaptureRequest: fn(*const c_void, i32, *mut *mut c_void) -> i32;
	ACameraDevice_createCaptureSession: fn(*mut c_void, *const c_void, *const SessionCallbacks, *mut *mut c_void) -> i32;
	ACaptureRequest_free: fn(*mut c_void);
	ACaptureRequest_addTarget: fn(*mut c_void, *const c_void) -> i32;
	ACameraOutputTarget_create: fn(*mut c_void, *mut *mut c_void) -> i32;
	ACameraOutputTarget_free: fn(*mut c_void);
	ACaptureSessionOutputContainer_create: fn(*mut *mut c_void) -> i32;
	ACaptureSessionOutputContainer_free: fn(*mut c_void);
	ACaptureSessionOutputContainer_add: fn(*mut c_void, *const c_void) -> i32;
	ACaptureSessionOutput_create: fn(*mut c_void, *mut *mut c_void) -> i32;
	ACaptureSessionOutput_free: fn(*mut c_void);
	ACameraCaptureSession_setRepeatingRequest: fn(*mut c_void, *mut c_void, i32, *mut *mut c_void, *mut i32) -> i32;
	ACameraCaptureSession_stopRepeating: fn(*mut c_void) -> i32;
	ACameraCaptureSession_close: fn(*mut c_void);
}

library! {
	Media, "libmediandk.so",
	AImageReader_new: fn(i32, i32, i32, i32, *mut *mut c_void) -> i32;
	AImageReader_delete: fn(*mut c_void);
	AImageReader_getWindow: fn(*mut c_void, *mut *mut c_void) -> i32;
	AImageReader_acquireLatestImage: fn(*mut c_void, *mut *mut c_void) -> i32;
	AImage_delete: fn(*mut c_void);
	AImage_getTimestamp: fn(*const c_void, *mut i64) -> i32;
	AImage_getWidth: fn(*const c_void, *mut i32) -> i32;
	AImage_getHeight: fn(*const c_void, *mut i32) -> i32;
	AImage_getPlaneRowStride: fn(*const c_void, i32, *mut i32) -> i32;
	AImage_getPlanePixelStride: fn(*const c_void, i32, *mut i32) -> i32;
	AImage_getPlaneData: fn(*const c_void, i32, *mut *mut u8, *mut i32) -> i32;
}

/// Both libraries, or `None` on a device without them (before Android 7).
fn apis() -> Option<(&'static Camera, &'static Media)> {
	static APIS: OnceLock<Option<(Camera, Media)>> = OnceLock::new();
	let (camera, media) = APIS
		.get_or_init(|| Some((Camera::load()?, Media::load()?)))
		.as_ref()?;
	Some((camera, media))
}

/// What the back camera is and how to stream from it, read once.
#[derive(Debug, Clone)]
struct Choice {
	id: CString,
	size: (u32, u32),
	sensor_quarter_turns: u32,
	intrinsics: Intrinsics,
}

/// The camera's open handles, freed in reverse order, and the manager that
/// opened them last.
struct Open {
	camera: &'static Camera,
	media: &'static Media,
	reader: *mut c_void,
	device: *mut c_void,
	request: *mut c_void,
	target: *mut c_void,
	output: *mut c_void,
	container: *mut c_void,
	session: *mut c_void,
	callbacks: Box<DeviceCallbacks>,
	session_callbacks: Box<SessionCallbacks>,
	manager: Manager,
}

impl Drop for Open {
	fn drop(&mut self) {
		let (camera, media) = (self.camera, self.media);
		// SAFETY: Stops and frees what `open` made, null handles skipped,
		// the camera before the reader whose window it fills.
		unsafe {
			if !self.session.is_null() {
				(camera.ACameraCaptureSession_stopRepeating)(self.session);
				(camera.ACameraCaptureSession_close)(self.session);
			}
			if !self.device.is_null() {
				(camera.ACameraDevice_close)(self.device);
			}
			if !self.request.is_null() {
				(camera.ACaptureRequest_free)(self.request);
			}
			if !self.target.is_null() {
				(camera.ACameraOutputTarget_free)(self.target);
			}
			if !self.output.is_null() {
				(camera.ACaptureSessionOutput_free)(self.output);
			}
			if !self.container.is_null() {
				(camera.ACaptureSessionOutputContainer_free)(self.container);
			}
			if !self.reader.is_null() {
				(media.AImageReader_delete)(self.reader);
			}
		}
	}
}

/// The back camera, for [`OrientationBackend`](super::OrientationBackend).
pub struct Camera2Source {
	choice: Option<Choice>,
	open: Option<Open>,
	latest: Option<Arc<CapturedImage>>,
}

impl std::fmt::Debug for Camera2Source {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Camera2Source")
			.field("choice", &self.choice)
			.field("open", &self.open.is_some())
			.finish_non_exhaustive()
	}
}

// SAFETY: The NDK's camera and image reader handles may be used from any
// thread; the runtime calls the source from one thread at a time.
unsafe impl Send for Camera2Source {}

impl Default for Camera2Source {
	fn default() -> Self {
		Self::new()
	}
}

impl Camera2Source {
	/// The device's first back camera that streams YUV, if it has one.
	#[must_use]
	pub fn new() -> Self {
		Self {
			choice: apis().and_then(|(camera, _)| choose(camera)),
			open: None,
			latest: None,
		}
	}

	fn connect(choice: &Choice) -> Result<Open, Error> {
		let (camera, media) = apis().ok_or(Error::Unavailable)?;
		let mut open = Open {
			camera,
			media,
			reader: std::ptr::null_mut(),
			device: std::ptr::null_mut(),
			request: std::ptr::null_mut(),
			target: std::ptr::null_mut(),
			output: std::ptr::null_mut(),
			container: std::ptr::null_mut(),
			session: std::ptr::null_mut(),
			callbacks: Box::new(DeviceCallbacks {
				context: std::ptr::null_mut(),
				disconnected: ignore_state,
				error: ignore_error,
			}),
			session_callbacks: Box::new(SessionCallbacks {
				context: std::ptr::null_mut(),
				closed: ignore_state,
				ready: ignore_state,
				active: ignore_state,
			}),
			manager: Manager::new(camera).ok_or(Error::Native)?,
		};
		let check = |status: i32| {
			match status {
				OK => Ok(()),
				PERMISSION_DENIED => Err(Error::PermissionDenied),
				_ => Err(Error::Native),
			}
		};
		let (width, height) = choice.size;
		let mut window = std::ptr::null_mut();

		// SAFETY: Each call fills a handle `open` frees when dropped; the
		// reader's window lives as long as the reader.
		unsafe {
			check((media.AImageReader_new)(
				width as i32,
				height as i32,
				YUV_420_888,
				MAX_IMAGES,
				&raw mut open.reader,
			))?;
			check((media.AImageReader_getWindow)(open.reader, &raw mut window))?;
			check((camera.ACameraManager_openCamera)(
				open.manager.0,
				choice.id.as_ptr(),
				&raw mut *open.callbacks,
				&raw mut open.device,
			))?;
			check((camera.ACaptureSessionOutputContainer_create)(
				&raw mut open.container,
			))?;
			check((camera.ACaptureSessionOutput_create)(
				window,
				&raw mut open.output,
			))?;
			check((camera.ACaptureSessionOutputContainer_add)(
				open.container,
				open.output,
			))?;
			check((camera.ACameraOutputTarget_create)(
				window,
				&raw mut open.target,
			))?;
			check((camera.ACameraDevice_createCaptureRequest)(
				open.device,
				TEMPLATE_PREVIEW,
				&raw mut open.request,
			))?;
			check((camera.ACaptureRequest_addTarget)(
				open.request,
				open.target,
			))?;
			check((camera.ACameraDevice_createCaptureSession)(
				open.device,
				open.container,
				&raw const *open.session_callbacks,
				&raw mut open.session,
			))?;
			let mut request = open.request;
			check((camera.ACameraCaptureSession_setRepeatingRequest)(
				open.session,
				std::ptr::null_mut(),
				1,
				&raw mut request,
				std::ptr::null_mut(),
			))?;
		}

		Ok(open)
	}

	/// Copies the newest image the reader holds, if a new one came.
	fn read(open: &Open, choice: &Choice) -> Option<CapturedImage> {
		let media = open.media;
		let mut image = std::ptr::null_mut();
		// SAFETY: Takes the newest image, or reports there is none.
		if unsafe { (media.AImageReader_acquireLatestImage)(open.reader, &raw mut image) } != OK
			|| image.is_null()
		{
			return None;
		}

		let image = Image(media, image);
		let (mut time, mut width, mut height) = (0, 0, 0);
		// SAFETY: Reads a live image's capture time and size.
		unsafe {
			(media.AImage_getTimestamp)(image.1, &raw mut time);
			(media.AImage_getWidth)(image.1, &raw mut width);
			(media.AImage_getHeight)(image.1, &raw mut height);
		}
		let (luma, luma_stride, _) = image.plane(0)?;
		let (cb, chroma_stride, chroma_step) = image.plane(1)?;
		let (cr, ..) = image.plane(2)?;
		let (width, height) = (width.max(0) as u32, height.max(0) as u32);
		let pixels = ycbcr_from_yuv420(
			width,
			height,
			luma,
			luma_stride,
			cb,
			cr,
			chroma_stride,
			chroma_step,
		)?;

		Some(CapturedImage {
			time: time as f64 / 1e9,
			width,
			height,
			pixels,
			sensor_quarter_turns: choice.sensor_quarter_turns,
			intrinsics: choice.intrinsics.scaled(width, height),
		})
	}
}

impl CameraSource for Camera2Source {
	fn available(&self) -> bool {
		self.choice.is_some()
	}

	fn start(&mut self) -> Result<(), Error> {
		if self.open.is_some() {
			return Ok(());
		}

		let choice = self.choice.clone().ok_or(Error::Unavailable)?;
		self.open = Some(Self::connect(&choice)?);
		self.latest = None;
		Ok(())
	}

	fn latest(&mut self) -> Option<Arc<CapturedImage>> {
		if let (Some(open), Some(choice)) = (&self.open, &self.choice)
			&& let Some(image) = Self::read(open, choice)
		{
			self.latest = Some(Arc::new(image));
		}

		self.latest.clone()
	}

	fn stop(&mut self) {
		self.open = None;
		self.latest = None;
	}
}

/// An `AImage`, deleted when dropped.
struct Image(&'static Media, *mut c_void);

impl Image {
	/// One plane's bytes, row stride, and pixel stride; the bytes live until
	/// the image is deleted.
	fn plane(&self, index: i32) -> Option<(&[u8], u32, u32)> {
		let (mut data, mut length, mut row, mut pixel) = (std::ptr::null_mut(), 0, 0, 0);
		// SAFETY: Reads the plane's pointer and strides from a live image.
		unsafe {
			(self.0.AImage_getPlaneData)(self.1, index, &raw mut data, &raw mut length);
			(self.0.AImage_getPlaneRowStride)(self.1, index, &raw mut row);
			(self.0.AImage_getPlanePixelStride)(self.1, index, &raw mut pixel);
		}

		if data.is_null() || length <= 0 || row <= 0 || pixel <= 0 {
			return None;
		}

		// SAFETY: The image reported `length` readable bytes at `data`.
		Some((
			unsafe { std::slice::from_raw_parts(data, length as usize) },
			row as u32,
			pixel as u32,
		))
	}
}

impl Drop for Image {
	fn drop(&mut self) {
		// SAFETY: Returns the image to its reader.
		unsafe { (self.0.AImage_delete)(self.1) };
	}
}

/// An `ACameraManager`, deleted when dropped.
struct Manager(*mut c_void, &'static Camera);

impl Manager {
	fn new(camera: &'static Camera) -> Option<Self> {
		// SAFETY: Makes a manager; `Drop` deletes it.
		let manager = unsafe { (camera.ACameraManager_create)() };
		(!manager.is_null()).then_some(Self(manager, camera))
	}
}

impl Drop for Manager {
	fn drop(&mut self) {
		// SAFETY: Deletes the manager `new` made.
		unsafe { (self.1.ACameraManager_delete)(self.0) };
	}
}

/// A camera's characteristics, freed when dropped.
struct Metadata(*mut c_void, &'static Camera);

impl Metadata {
	fn entry(&self, tag: u32) -> Option<Entry> {
		let mut entry = Entry {
			tag: 0,
			kind: 0,
			count: 0,
			data: std::ptr::null(),
		};
		// SAFETY: Reads one entry, which lives as long as the metadata.
		let status = unsafe { (self.1.ACameraMetadata_getConstEntry)(self.0, tag, &raw mut entry) };
		(status == OK && !entry.data.is_null() && entry.count > 0).then_some(entry)
	}

	fn bytes(&self, tag: u32) -> &[u8] {
		self.entry(tag).map_or(&[], |entry| {
			// SAFETY: A byte entry holds `count` bytes.
			unsafe { std::slice::from_raw_parts(entry.data.cast(), entry.count as usize) }
		})
	}

	fn ints(&self, tag: u32) -> &[i32] {
		self.entry(tag).map_or(&[], |entry| {
			// SAFETY: An `int32` entry holds `count` of them.
			unsafe { std::slice::from_raw_parts(entry.data.cast(), entry.count as usize) }
		})
	}

	fn floats(&self, tag: u32) -> &[f32] {
		self.entry(tag).map_or(&[], |entry| {
			// SAFETY: A `float` entry holds `count` of them.
			unsafe { std::slice::from_raw_parts(entry.data.cast(), entry.count as usize) }
		})
	}
}

impl Drop for Metadata {
	fn drop(&mut self) {
		// SAFETY: Frees the characteristics the manager returned.
		unsafe { (self.1.ACameraMetadata_free)(self.0) };
	}
}

/// The first back camera that streams YUV, with the stream size and
/// projection this source uses.
fn choose(camera: &'static Camera) -> Option<Choice> {
	let manager = Manager::new(camera)?;
	let mut list = std::ptr::null_mut();
	// SAFETY: Lists the camera ids; the list is deleted below.
	if unsafe { (camera.ACameraManager_getCameraIdList)(manager.0, &raw mut list) } != OK
		|| list.is_null()
	{
		return None;
	}

	// SAFETY: The list holds `count` C strings.
	let ids: Vec<CString> = unsafe {
		let list = &*list;
		(0..list.count.max(0) as usize)
			.filter_map(|index| {
				let id = *list.ids.add(index);
				(!id.is_null()).then(|| CStr::from_ptr(id).to_owned())
			})
			.collect()
	};
	// SAFETY: Deletes the list read above.
	unsafe { (camera.ACameraManager_deleteCameraIdList)(list) };

	ids.into_iter().find_map(|id| {
		let mut raw = std::ptr::null_mut();
		// SAFETY: Reads this camera's characteristics; `Metadata` frees them.
		let status = unsafe {
			(camera.ACameraManager_getCameraCharacteristics)(manager.0, id.as_ptr(), &raw mut raw)
		};

		if status != OK || raw.is_null() {
			return None;
		}

		let metadata = Metadata(raw, camera);

		if metadata.bytes(LENS_FACING).first() != Some(&FACING_BACK) {
			return None;
		}

		describe(&metadata, id)
	})
}

/// The stream size and projection for a back camera.
fn describe(metadata: &Metadata, id: CString) -> Option<Choice> {
	let active = metadata.ints(SENSOR_INFO_ACTIVE_ARRAY_SIZE);
	// The NDK gives the active array as left, top, width, height.
	let active = match active {
		[_, _, width, height, ..] if *width > 0 && *height > 0 => (*width as f32, *height as f32),
		_ => return None,
	};
	let sizes: Vec<(u32, u32)> = metadata
		.ints(SCALER_AVAILABLE_STREAM_CONFIGURATIONS)
		.as_chunks::<4>()
		.0
		.iter()
		.filter(|[format, width, height, direction]| {
			*format == YUV_420_888 && *direction == STREAM_OUTPUT && *width > 0 && *height > 0
		})
		.map(|[_, width, height, _]| (*width as u32, *height as u32))
		.collect();
	let size = stream_size(&sizes, active.0 / active.1)?;
	let focal = focal_length(metadata, active.0)?;
	// A stream is the centre of the active array cropped to its aspect and
	// scaled, so the focal length scales with it.
	let scale = (size.0 as f32 / active.0).max(size.1 as f32 / active.1);
	let sensor_quarter_turns = metadata
		.ints(SENSOR_ORIENTATION)
		.first()
		.map_or(1, |degrees| (degrees.rem_euclid(360) / 90) as u32);

	Some(Choice {
		id,
		size,
		sensor_quarter_turns,
		intrinsics: Intrinsics {
			focal: [focal * scale, focal * scale],
			principal: [size.0 as f32 / 2.0, size.1 as f32 / 2.0],
			size: [size.0, size.1],
		},
	})
}

/// The focal length in pixels of the active array: the lens calibration
/// where the device reports one, or the lens's focal length over the
/// sensor's pixel pitch.
fn focal_length(metadata: &Metadata, active_width: f32) -> Option<f32> {
	if let [fx, ..] = metadata.floats(LENS_INTRINSIC_CALIBRATION)
		&& *fx > 0.0
	{
		return Some(*fx);
	}

	let millimetres = *metadata.floats(LENS_INFO_AVAILABLE_FOCAL_LENGTHS).first()?;
	let sensor_width = *metadata.floats(SENSOR_INFO_PHYSICAL_SIZE).first()?;
	let pixels = metadata
		.ints(SENSOR_INFO_PIXEL_ARRAY_SIZE)
		.first()
		.map_or(active_width, |width| *width as f32);

	(millimetres > 0.0 && sensor_width > 0.0).then(|| millimetres * pixels / sensor_width)
}

/// The largest stream of at most [`MAX_PIXELS`] closest to the sensor's
/// aspect ratio, or the smallest stream when every one is larger.
fn stream_size(sizes: &[(u32, u32)], aspect: f32) -> Option<(u32, u32)> {
	// Aspects within 2 % of each other count as the same.
	let skew =
		|width: u32, height: u32| ((width as f32 / height as f32 - aspect).abs() * 50.0) as u32;

	sizes
		.iter()
		.copied()
		.filter(|(width, height)| width * height <= MAX_PIXELS)
		.min_by_key(|&(width, height)| (skew(width, height), std::cmp::Reverse(width * height)))
		.or_else(|| {
			sizes
				.iter()
				.copied()
				.min_by_key(|(width, height)| width * height)
		})
}
