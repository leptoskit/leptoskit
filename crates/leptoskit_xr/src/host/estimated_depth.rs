//! Depth read from the camera picture by `leptoskit_depth`'s model, for
//! sessions on phones whose platform measures none.
//!
//! [`ModelEstimator`] is the [`DepthEstimator`] over a
//! [`leptoskit_depth::DepthWorker`]: it hands the runtime's camera frames to
//! the model's thread without copying their pixels, and converts each depth
//! map it finishes into a [`DepthEstimate`] once. [`install_depth_model`]
//! gives every runtime the `xr` plugin makes one, and the plugin calls it
//! itself when `[native.plugin-options.xr] depth-model` names the model in
//! the app's assets.

use std::sync::Arc;
use std::time::Duration;

use leptoskit_depth::DepthModel;
use leptoskit_depth::DepthWorker;
use leptoskit_depth::Estimator;
use leptoskit_depth::Frame;
use leptoskit_depth::Orientation;
use leptoskit_depth::Picture;
use leptoskit_depth::Pixels;
use leptoskit_depth::Plane;

use super::CameraFrame;
use super::DepthEstimate;
use super::DepthEstimator;
use super::install_depth_estimator;
use crate::image::Affine2;
use crate::image::DepthFormat;
use crate::image::DepthImage;
use crate::types::View;

/// What comes back with a picture's depth: the frame it was read from.
#[derive(Debug, Clone, Copy)]
struct Captured {
	view: View,
	display_transform: Affine2,
}

/// A [`DepthEstimator`] that reads depth from camera pictures with a
/// `leptoskit_depth` model, one picture at a time on its own thread and at
/// most once a period (a second by default), so the runtime never waits.
///
/// Clones share the thread and its newest depth, so the runtimes they serve
/// share the model's memory and its one core: [`install_depth_model`] gives
/// each runtime the `xr` plugin makes a clone of one.
#[derive(Debug, Clone)]
pub struct ModelEstimator {
	worker: Arc<DepthWorker<Captured>>,
	converted: Option<(u64, DepthEstimate)>,
}

impl ModelEstimator {
	/// Starts a thread that reads a picture at most once a
	/// [`leptoskit_depth::DEFAULT_PERIOD`] with `estimator`: a
	/// [`DepthModel`], or a stand-in in tests.
	pub fn spawn(estimator: impl Estimator) -> std::io::Result<Self> {
		Self::with_period(estimator, leptoskit_depth::DEFAULT_PERIOD)
	}

	/// Starts a thread that reads a picture at most once each `period`.
	pub fn with_period(estimator: impl Estimator, period: Duration) -> std::io::Result<Self> {
		Ok(Self {
			worker: Arc::new(DepthWorker::with_period(estimator, period)?),
			converted: None,
		})
	}

	/// Why the last picture could not be read, if it could not.
	#[must_use]
	pub fn last_error(&self) -> Option<leptoskit_depth::Error> {
		self.worker.last_error()
	}
}

/// The model's view of a camera frame: the same pixels, shared rather than
/// copied, turned upright the way the frame's display transform shows them.
fn picture(frame: &CameraFrame) -> Picture {
	let rgb = &frame.rgb;
	let intrinsics = frame.intrinsics;

	Picture::new(Pixels::Rgb8(Plane {
		width: rgb.width,
		height: rgb.height,
		row_stride: rgb.row_stride,
		data: Arc::clone(&rgb.data),
	}))
	.with_orientation(Orientation::from_display_transform(
		&frame.display_transform,
	))
	.with_intrinsics(leptoskit_depth::Intrinsics {
		focal: intrinsics.focal,
		principal: intrinsics.principal,
		size: intrinsics.size,
	})
}

impl DepthEstimator for ModelEstimator {
	fn max_side(&self) -> u32 {
		self.worker.max_side()
	}

	fn ready(&self) -> bool {
		self.worker.ready()
	}

	fn submit(&mut self, frame: Arc<CameraFrame>) {
		self.worker.submit(Frame {
			picture: picture(&frame),
			time: frame.time,
			context: Captured {
				view: frame.view,
				display_transform: frame.display_transform,
			},
		});
	}

	/// The newest depth, converted once per estimate: every other call is an
	/// `Arc` clone.
	fn latest(&mut self) -> Option<DepthEstimate> {
		let estimate = self.worker.latest()?;

		if self.converted.as_ref().map(|(sequence, _)| *sequence) != Some(estimate.sequence) {
			let map = &estimate.depth;
			let depth = DepthImage {
				width: map.width,
				height: map.height,
				format: DepthFormat::Meters32,
				data: map.to_le_bytes().into(),
				row_stride: map.width * 4,
				confidence: None,
				display_transform: estimate.context.display_transform,
			};
			self.converted = Some((
				estimate.sequence,
				DepthEstimate {
					depth,
					view: estimate.context.view,
					time: estimate.time,
				},
			));
		}

		self.converted
			.as_ref()
			.map(|(_, estimate)| estimate.clone())
	}

	fn reset(&mut self) {
		self.worker.reset();
		self.converted = None;
	}
}

/// Reads depth with `model` for every runtime the `xr` plugin makes, now
/// and from now on, on one thread they share: see
/// [`install_depth_estimator`]. Sessions that start afterwards offer
/// `DEPTH` wherever the device offers the camera picture.
///
/// `[native.plugin-options.xr] depth-model` calls this once the model has
/// loaded from the app's assets; call it yourself to load the model some
/// other way.
pub fn install_depth_model(model: DepthModel) -> std::io::Result<()> {
	let estimator = ModelEstimator::spawn(model)?;
	install_depth_estimator(move || Box::new(estimator.clone()));
	Ok(())
}

/// Loads the model at `path` in the app's bundled assets on a thread of its
/// own, checks that it is [`leptoskit_depth::MODEL`], and installs it with
/// [`install_depth_model`]. Plugin creation doesn't wait: loading takes half
/// a second to a second and a half. A missing or wrong file is logged and
/// leaves depth off.
#[cfg(feature = "async-host")]
pub(crate) fn load_in_background(
	platform: Option<leptoskit_bridge::PlatformContext>,
	path: String,
) {
	let spawned = std::thread::Builder::new()
		.name("leptoskit-xr-depth-model".into())
		.spawn(move || {
			match load(platform.as_ref(), &path) {
				Ok(()) => {
					super::bundle::info(&format!(
						"estimating depth from the camera with assets/{path}"
					));
				}
				Err(problem) => {
					super::bundle::warn(&format!(
						"depth-model: {problem}. Phones that measure no depth get none."
					));
				}
			}
		});

	if let Err(error) = spawned {
		super::bundle::warn(&format!(
			"depth-model: the thread that loads it can't start ({error}). Phones that measure no \
			 depth get none."
		));
	}
}

#[cfg(feature = "async-host")]
fn load(platform: Option<&leptoskit_bridge::PlatformContext>, path: &str) -> Result<(), String> {
	use leptoskit_depth::MODEL;

	super::bundle::check_path(path)
		.map_err(|reason| format!("{path:?} is not a path in the app's assets folder: {reason}"))?;
	let bytes = super::bundle::read(platform, path)?;
	MODEL.verify(&bytes).map_err(|error| {
		format!(
			"assets/{path} is not the model leptoskit_depth is made for ({error}). Fetch {} from \
			 {} and check its SHA-256",
			MODEL.name, MODEL.url
		)
	})?;
	let model = DepthModel::from_bytes(&bytes)
		.map_err(|error| format!("assets/{path} can't be loaded: {error}"))?;
	drop(bytes);
	install_depth_model(model).map_err(|error| format!("the depth thread can't start: {error}"))
}

#[cfg(test)]
mod tests {
	use std::sync::Mutex;
	use std::sync::mpsc;

	use leptoskit_depth::DepthMap;
	use leptoskit_depth::Error;

	use super::*;
	use crate::host::Intrinsics;
	use crate::image::ImagePlane;
	use crate::math::Fov;
	use crate::math::Pose;
	use crate::math::Projection;
	use crate::math::Vec3;

	/// Answers each picture, once the test lets it, with a 2 × 1 map: its
	/// width in metres, then unknown. Reports which way the picture was
	/// turned and whether its pixels were shared.
	struct Stub {
		gate: Mutex<mpsc::Receiver<()>>,
		seen: mpsc::Sender<(Orientation, [f32; 2], usize)>,
	}

	impl Estimator for Stub {
		fn estimate(&self, picture: &Picture) -> Result<DepthMap, Error> {
			self.gate
				.lock()
				.unwrap()
				.recv()
				.map_err(|_| Error::Inference("closed".into()))?;
			let Pixels::Rgb8(plane) = &picture.pixels else {
				return Err(Error::Picture("not RGB"));
			};
			let _ = self.seen.send((
				picture.orientation,
				picture.lens().focal,
				Arc::strong_count(&plane.data),
			));
			Ok(DepthMap {
				width: 2,
				height: 1,
				depths: vec![plane.width as f32, 0.0],
				intrinsics: picture.lens(),
				scale: 1.0,
				took: Duration::ZERO,
			})
		}

		fn max_side(&self) -> u32 {
			64
		}
	}

	fn frame(width: u32, time: f64) -> Arc<CameraFrame> {
		Arc::new(CameraFrame {
			time,
			view: View {
				pose: Pose::at(Vec3::new(0.0, 1.5, 0.0)),
				projection: Projection::Fov(Fov::symmetric(1.0, 0.75)),
			},
			rgb: ImagePlane {
				width,
				height: 2,
				row_stride: width * 3,
				data: vec![90; width as usize * 6].into(),
			},
			intrinsics: Intrinsics {
				focal: [3.0, 3.0],
				principal: [width as f32 / 2.0, 1.0],
				size: [width, 2],
			},
			// The view's down is the picture's −x: a sensor that turns a
			// quarter clockwise to stand upright.
			display_transform: [0.0, -1.0, 1.0, 0.0, 0.0, 1.0],
		})
	}

	fn wait(estimator: &mut ModelEstimator, time: f64) -> DepthEstimate {
		for _ in 0..1000 {
			if let Some(estimate) = estimator
				.latest()
				.filter(|estimate| estimate.time.to_bits() == time.to_bits())
			{
				return estimate;
			}
			std::thread::sleep(Duration::from_millis(2));
		}
		panic!("no estimate of the frame at {time}");
	}

	#[test]
	fn frames_reach_the_model_turned_and_unshared_and_come_back_as_depth() {
		let (open, gate) = mpsc::channel();
		let (seen, report) = mpsc::channel();
		let stub = Stub {
			gate: Mutex::new(gate),
			seen,
		};
		let mut estimator = ModelEstimator::with_period(stub, Duration::ZERO).unwrap();
		assert_eq!(estimator.max_side(), 64);
		assert!(estimator.ready());
		assert!(estimator.latest().is_none());

		let captured = frame(4, 1.5);
		estimator.submit(Arc::clone(&captured));
		assert!(!estimator.ready());
		open.send(()).unwrap();
		let estimate = wait(&mut estimator, 1.5);

		let (orientation, focal, holders) = report.recv().unwrap();
		assert_eq!(orientation, Orientation::Clockwise);
		assert_eq!(focal, [3.0, 3.0]);
		// The frame and the model's picture held the same pixels.
		assert!(holders >= 2, "{holders}");

		assert_eq!(estimate.view, captured.view);
		let depth = &estimate.depth;
		assert_eq!((depth.width, depth.height, depth.row_stride), (2, 1, 8));
		assert_eq!(depth.format, DepthFormat::Meters32);
		assert_eq!(depth.display_transform, captured.display_transform);
		assert_eq!(depth.sample(0.25, 0.5), Some(4.0));
		assert_eq!(depth.sample(0.75, 0.5), None);

		// The same estimate is converted once and handed out again.
		let again = estimator.latest().unwrap();
		assert!(Arc::ptr_eq(&again.depth.data, &estimate.depth.data));

		// A clone shares the thread and its depth.
		let mut shared = estimator.clone();
		assert_eq!(shared.latest().unwrap().time, 1.5);

		// A reset forgets it.
		estimator.reset();
		assert!(estimator.latest().is_none());
		assert!(shared.latest().is_none());
		estimator.submit(frame(6, 2.5));
		open.send(()).unwrap();
		assert_eq!(wait(&mut estimator, 2.5).depth.sample(0.25, 0.5), Some(6.0));
	}
}
