//! The camera picture on the CPU: a camera that runs without AR tracking,
//! and the seam a depth estimator reads frames through.
//!
//! - [`CameraSource`] is a camera with no tracking of its own, such as
//!   Android's Camera2 on a phone without ARCore. An [`OrientationBackend`]
//!   with one draws the camera picture behind its scene, turned upright by
//!   [`upright_transform`].
//! - [`CameraFrame`] is one picture made small for reading on the CPU, with its
//!   time, the viewer's pose, and its [`Intrinsics`]. A [`DepthEstimator`]
//!   installed with [`XrRuntime::set_depth_estimator`] turns such frames into
//!   depth pictures for sessions whose platform has no depth of its own.
//!
//! None of these pictures reach the guest. A crate that estimates depth
//! reaches the runtimes the generated hosts make through
//! [`install_depth_estimator`].
//!
//! [`OrientationBackend`]: super::OrientationBackend
//! [`XrRuntime::set_depth_estimator`]: super::XrRuntime::set_depth_estimator

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::Weak;

use leptoskit_bridge::Error;

use super::Viewport;
use super::XrFrame;
use super::XrRuntime;
use crate::image::*;
use crate::math::Fov;
use crate::math::Pose;

/// A pinhole camera's projection, in pixels of the image it describes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Intrinsics {
	/// Focal lengths along x and y, in pixels.
	pub focal: [f32; 2],
	/// Where the optical axis meets the image, in pixels from its top-left.
	pub principal: [f32; 2],
	/// The image's width and height in pixels.
	pub size: [u32; 2],
}

impl Intrinsics {
	/// The same camera described for the image resized to `width` by
	/// `height`.
	#[must_use]
	pub fn scaled(self, width: u32, height: u32) -> Self {
		let sx = width as f32 / self.size[0].max(1) as f32;
		let sy = height as f32 / self.size[1].max(1) as f32;

		Self {
			focal: [self.focal[0] * sx, self.focal[1] * sy],
			principal: [self.principal[0] * sx, self.principal[1] * sy],
			size: [width, height],
		}
	}
}

/// One picture from a camera without tracking: see [`CameraSource`].
#[derive(Debug, Clone)]
pub struct CapturedImage {
	/// When it was captured, in seconds on the sensor's monotonic clock.
	pub time: f64,
	/// Width in pixels, as the sensor reads it.
	pub width: u32,
	/// Height in pixels, as the sensor reads it.
	pub height: u32,
	/// The pixels, in the sensor's orientation.
	pub pixels: CameraPixels,
	/// Clockwise quarter turns that make the picture upright on the device
	/// held in its natural orientation (Android's `SENSOR_ORIENTATION` / 90).
	pub sensor_quarter_turns: u32,
	/// How the camera projects, in this picture's pixels.
	pub intrinsics: Intrinsics,
}

/// A camera with no tracking of its own, for orientation sessions that show
/// the camera picture behind their scene.
pub trait CameraSource: Send {
	/// Whether the device has a camera this source can open.
	fn available(&self) -> bool;
	/// Opens the camera. Fails with `PermissionDenied` until the user allows
	/// the camera.
	fn start(&mut self) -> Result<(), Error>;
	/// The newest picture, or `None` before the first.
	fn latest(&mut self) -> Option<Arc<CapturedImage>>;
	/// Closes the camera.
	fn stop(&mut self);
}

/// The transform from a view, `(0, 0)` top-left to `(1, 1)` bottom-right,
/// to a camera picture `width` by `height` that must turn
/// `sensor_quarter_turns` clockwise to stand upright: it turns the picture
/// for the interface's rotation and crops it to fill the view. Also returns
/// the share of the turned picture's width and height the view shows.
#[must_use]
pub fn upright_transform(
	width: u32,
	height: u32,
	sensor_quarter_turns: u32,
	viewport: Viewport,
) -> (Affine2, [f32; 2]) {
	let turns = (sensor_quarter_turns + 4 - viewport.quarter_turns % 4) % 4;
	let (shown_width, shown_height) = if turns % 2 == 1 {
		(height, width)
	} else {
		(width, height)
	};
	let picture = shown_width.max(1) as f32 / shown_height.max(1) as f32;
	let view = viewport.aspect();
	let [sx, sy] = if view > picture {
		[1.0, picture / view]
	} else {
		[view / picture, 1.0]
	};
	// From the view to the upright picture: the centre stays put.
	let fill = [sx, 0.0, 0.0, sy, 0.5 - 0.5 * sx, 0.5 - 0.5 * sy];
	// From the upright picture to the sensor's.
	let turn = match turns {
		1 => [0.0, -1.0, 1.0, 0.0, 0.0, 1.0],
		2 => [-1.0, 0.0, 0.0, -1.0, 1.0, 1.0],
		3 => [0.0, 1.0, -1.0, 0.0, 1.0, 0.0],
		_ => IDENTITY_AFFINE,
	};

	(compose_affine(&turn, &fill), [sx, sy])
}

/// The field of view a view shows of a camera picture turned upright and
/// cropped as [`upright_transform`] does, so a scene drawn over it matches
/// the picture's scale.
#[must_use]
pub fn upright_fov(intrinsics: &Intrinsics, sensor_quarter_turns: u32, viewport: Viewport) -> Fov {
	let turns = (sensor_quarter_turns + 4 - viewport.quarter_turns % 4) % 4;
	let [width, height] = intrinsics.size;
	let (_, [_, shown]) = upright_transform(width, height, sensor_quarter_turns, viewport);
	// The view's vertical is the sensor's horizontal when the picture turns
	// a quarter.
	let (pixels, focal) = if turns % 2 == 1 {
		(width as f32, intrinsics.focal[0])
	} else {
		(height as f32, intrinsics.focal[1])
	};
	let vertical = 2.0 * (shown * pixels * 0.5 / focal.max(f32::EPSILON)).atan();

	Fov::symmetric(vertical, viewport.aspect())
}

/// `outer` after `inner`.
fn compose_affine(outer: &Affine2, inner: &Affine2) -> Affine2 {
	let [oxx, oxy, oyx, oyy, otx, oty] = *outer;
	let [ixx, ixy, iyx, iyy, itx, ity] = *inner;

	[
		oxx * ixx + oyx * ixy,
		oxy * ixx + oyy * ixy,
		oxx * iyx + oyx * iyy,
		oxy * iyx + oyy * iyy,
		oxx * itx + oyx * ity + otx,
		oxy * itx + oyy * ity + oty,
	]
}

/// One camera picture made small for reading on the CPU, such as by a depth
/// estimator.
#[derive(Debug, Clone)]
pub struct CameraFrame {
	/// When it was captured, in seconds on the clock of [`XrFrame::time`].
	pub time: f64,
	/// Where the viewer was when it was captured, in the session's world.
	pub pose: Pose,
	/// sRGB pixels, three bytes each, in rows from the top as the sensor
	/// reads them (not turned upright).
	pub rgb: ImagePlane,
	/// How the camera projects, in `rgb`'s pixels.
	pub intrinsics: Intrinsics,
	/// Maps a point on the view, `(0, 0)` top-left to `(1, 1)` bottom-right,
	/// to the point in `rgb` to sample: the camera picture's own display
	/// transform. A depth picture made from this frame uses it too.
	pub display_transform: Affine2,
}

impl CameraFrame {
	/// `frame`'s camera picture, shrunk so its longer side is at most
	/// `max_side` pixels. `None` when the frame has no camera picture, no
	/// intrinsics, or pixels that can't be read on the CPU.
	#[must_use]
	pub fn from_frame(frame: &XrFrame, max_side: u32) -> Option<Self> {
		let image = frame.camera_image.as_ref()?;
		let intrinsics = frame.camera_intrinsics?;
		let longest = image.width.max(image.height).max(1);
		let scale = (max_side.max(1) as f32 / longest as f32).min(1.0);
		let width = ((image.width as f32 * scale).round() as u32).max(1);
		let height = ((image.height as f32 * scale).round() as u32).max(1);
		let rgb = reduce(&image.pixels, image.width, image.height, width, height)?;

		Some(Self {
			time: frame.time,
			pose: frame.viewer,
			rgb,
			intrinsics: intrinsics.scaled(width, height),
			display_transform: image.display_transform,
		})
	}
}

/// Turns camera pictures into depth for sessions whose platform has none,
/// such as a phone without a depth sensor or ARCore's Depth API.
///
/// The runtime offers it frames on the thread that drives the session, so
/// every method must return at once: a model that takes longer than a frame
/// runs on its own thread, takes a new frame only when [`ready`], and hands
/// back the newest depth it has finished.
///
/// [`ready`]: DepthEstimator::ready
pub trait DepthEstimator: Send {
	/// The longest side, in pixels, of the frames it wants.
	fn max_side(&self) -> u32;

	/// Whether it would take a new frame now. The runtime shrinks a camera
	/// picture only when it would.
	fn ready(&self) -> bool {
		true
	}

	/// Hands over a frame.
	fn submit(&mut self, frame: Arc<CameraFrame>);

	/// The newest depth picture it has finished, or `None` before the first.
	/// Its distances are metres (or millimetres) along the camera's forward
	/// axis, and its display transform is that of the frame it came from.
	fn latest(&mut self) -> Option<DepthImage>;

	/// Forgets everything: the session stopped.
	fn reset(&mut self) {}
}

/// Makes one [`DepthEstimator`] per runtime.
type EstimatorFactory = Arc<dyn Fn() -> Box<dyn DepthEstimator> + Send + Sync>;

/// The installed estimator factory, and the runtimes the `xr` plugin made.
static ESTIMATION: Mutex<(Option<EstimatorFactory>, Vec<Weak<XrRuntime>>)> =
	Mutex::new((None, Vec::new()));

/// Gives every runtime the `xr` plugin makes, now and from now on, an
/// estimator from `make`. Generated hosts make the plugin and its runtime
/// themselves, so this is how a depth crate plugs in: call it once at start,
/// before or after the plugin is made. Sessions started afterwards offer
/// `DEPTH` wherever the device offers the camera picture.
pub fn install_depth_estimator(make: impl Fn() -> Box<dyn DepthEstimator> + Send + Sync + 'static) {
	let make: EstimatorFactory = Arc::new(make);
	let mut estimation = ESTIMATION.lock().unwrap_or_else(PoisonError::into_inner);
	estimation.1.retain(|runtime| runtime.strong_count() > 0);

	for runtime in estimation.1.iter().filter_map(Weak::upgrade) {
		runtime.set_depth_estimator(Some(make()));
	}

	estimation.0 = Some(make);
}

/// Registers a runtime the `xr` plugin made, giving it the installed
/// estimator, if any.
#[cfg_attr(not(feature = "async-host"), allow(dead_code))]
pub(crate) fn adopt(runtime: &Arc<XrRuntime>) {
	let mut estimation = ESTIMATION.lock().unwrap_or_else(PoisonError::into_inner);
	estimation.1.retain(|runtime| runtime.strong_count() > 0);

	if let Some(make) = &estimation.0 {
		runtime.set_depth_estimator(Some(make()));
	}

	estimation.1.push(Arc::downgrade(runtime));
}

/// Shrinks a camera picture to `width` by `height` sRGB pixels, averaging
/// the luma under each one.
fn reduce(
	pixels: &CameraPixels,
	source_width: u32,
	source_height: u32,
	width: u32,
	height: u32,
) -> Option<ImagePlane> {
	let footprint = |index: u32, size: u32, source: u32| {
		let (index, size, source) = (u64::from(index), u64::from(size), u64::from(source));
		let start = (index * source / size) as u32;
		let end =
			(((index + 1) * source).div_ceil(size) as u32).clamp(start + 1, source.max(1) as u32);
		start..end
	};
	let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);

	match pixels {
		CameraPixels::YCbCr { y, cbcr } if y.is_complete(1) && cbcr.is_complete(2) => {
			for row in 0..height {
				let rows = footprint(row, height, source_height.min(y.height));

				for column in 0..width {
					let columns = footprint(column, width, source_width.min(y.width));
					let (mut sum, mut count) = (0_u32, 0_u32);

					for source_row in rows.clone() {
						let line = source_row as usize * y.row_stride as usize;

						for source_column in columns.clone() {
							sum += u32::from(*y.data.get(line + source_column as usize)?);
							count += 1;
						}
					}

					let luma = sum as f32 / count.max(1) as f32;
					let chroma_x = (u32::midpoint(columns.start, columns.end) / 2)
						.min(cbcr.width.saturating_sub(1));
					let chroma_y = (u32::midpoint(rows.start, rows.end) / 2)
						.min(cbcr.height.saturating_sub(1));
					let at = chroma_y as usize * cbcr.row_stride as usize + chroma_x as usize * 2;
					let cb = f32::from(*cbcr.data.get(at)?) - 128.0;
					let cr = f32::from(*cbcr.data.get(at + 1)?) - 128.0;
					let channel = |value: f32| value.round().clamp(0.0, 255.0) as u8;
					rgb.extend([
						channel(luma + 1.402 * cr),
						channel(luma - 0.344_136 * cb - 0.714_136 * cr),
						channel(luma + 1.772 * cb),
					]);
				}
			}
		}
		CameraPixels::Rgba(rgba) if rgba.is_complete(4) => {
			for row in 0..height {
				let source_row =
					(u64::from(row) * u64::from(rgba.height) / u64::from(height)) as usize;

				for column in 0..width {
					let source_column =
						(u64::from(column) * u64::from(rgba.width) / u64::from(width)) as usize;
					let at = source_row * rgba.row_stride as usize + source_column * 4;
					rgb.extend_from_slice(rgba.data.get(at..at + 3)?);
				}
			}
		}
		#[cfg(all(target_os = "ios", target_arch = "aarch64"))]
		CameraPixels::Native(NativeImage::PixelBuffer(buffer)) => {
			let (y, cbcr) = buffer.planes()?;
			return reduce(
				&CameraPixels::YCbCr { y, cbcr },
				source_width,
				source_height,
				width,
				height,
			);
		}
		_ => return None,
	}

	Some(ImagePlane {
		width,
		height,
		row_stride: width * 3,
		data: rgb.into(),
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	fn close(a: (f32, f32), b: (f32, f32)) -> bool {
		(a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5
	}

	fn portrait() -> Viewport {
		Viewport {
			width: 1080,
			height: 1920,
			quarter_turns: 0,
		}
	}

	#[test]
	fn a_landscape_sensor_turns_upright_for_a_portrait_screen() {
		// A 4:3 sensor mounted a quarter turn clockwise, as most back cameras
		// are, shown on a 9:16 portrait screen.
		let (transform, shown) = upright_transform(640, 480, 1, portrait());
		// The upright picture is 3:4; the screen is narrower, so its sides
		// are cropped.
		assert!((shown[0] - (9.0 / 16.0) / (3.0 / 4.0)).abs() < 1e-5);
		assert!((shown[1] - 1.0).abs() < 1e-6);
		// The screen's top-left shows the sensor's bottom-left region, and its
		// top edge runs up the sensor's left edge.
		let crop = (1.0 - shown[0]) / 2.0;
		assert!(close(apply_affine(&transform, 0.0, 0.0), (0.0, 1.0 - crop)));
		assert!(close(apply_affine(&transform, 1.0, 0.0), (0.0, crop)));
		assert!(close(apply_affine(&transform, 0.5, 0.5), (0.5, 0.5)));
	}

	#[test]
	fn turning_the_interface_with_the_sensor_shows_it_as_it_is() {
		let viewport = Viewport {
			width: 1920,
			height: 1080,
			quarter_turns: 1,
		};
		let (transform, shown) = upright_transform(1920, 1080, 1, viewport);
		assert_eq!(shown, [1.0, 1.0]);
		assert!(close(apply_affine(&transform, 0.25, 0.75), (0.25, 0.75)));
	}

	#[test]
	fn every_turn_maps_the_view_s_corners_onto_the_picture_s() {
		for sensor in 0..4 {
			for screen in 0..4 {
				let viewport = Viewport {
					width: 1000,
					height: 1000,
					quarter_turns: screen,
				};
				let (transform, _) = upright_transform(1000, 1000, sensor, viewport);
				for corner in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
					let (u, v) = apply_affine(&transform, corner.0, corner.1);
					assert!(
						[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
							.iter()
							.any(|&expected| close((u, v), expected)),
						"sensor {sensor}, screen {screen}: {corner:?} → {u}, {v}"
					);
				}
			}
		}
	}

	#[test]
	fn the_field_of_view_follows_the_crop() {
		// 640 × 480 with a 500-pixel focal length, turned a quarter onto a
		// portrait screen: the screen's height spans the sensor's width.
		let intrinsics = Intrinsics {
			focal: [500.0, 500.0],
			principal: [320.0, 240.0],
			size: [640, 480],
		};
		let fov = upright_fov(&intrinsics, 1, portrait());
		let expected = (320.0_f32 / 500.0).atan();
		assert!((fov.up - expected).abs() < 1e-5);
		assert!((fov.right - (expected.tan() * 1080.0 / 1920.0).atan()).abs() < 1e-5);
	}

	#[test]
	fn frames_shrink_to_their_longest_side_in_rgb() {
		// A 4 × 2 picture: white on the left half, black on the right, with
		// neutral chroma.
		let y = ImagePlane {
			width: 4,
			height: 2,
			row_stride: 4,
			data: vec![255, 255, 0, 0, 255, 255, 0, 0].into(),
		};
		let cbcr = ImagePlane {
			width: 2,
			height: 1,
			row_stride: 4,
			data: vec![128; 4].into(),
		};
		let frame = XrFrame {
			time: 2.0,
			camera_image: Some(CameraImage {
				width: 4,
				height: 2,
				display_transform: IDENTITY_AFFINE,
				pixels: CameraPixels::YCbCr { y, cbcr },
			}),
			camera_intrinsics: Some(Intrinsics {
				focal: [4.0, 4.0],
				principal: [2.0, 1.0],
				size: [4, 2],
			}),
			..XrFrame::default()
		};
		let small = CameraFrame::from_frame(&frame, 2).unwrap();
		assert_eq!((small.rgb.width, small.rgb.height), (2, 1));
		assert_eq!(&*small.rgb.data, &[255, 255, 255, 0, 0, 0]);
		assert_eq!(small.intrinsics.focal, [2.0, 2.0]);
		assert!((small.time - 2.0).abs() < f64::EPSILON);
		assert!(CameraFrame::from_frame(&XrFrame::default(), 2).is_none());
	}
}
