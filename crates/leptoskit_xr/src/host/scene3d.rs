//! Drawing a `leptoskit_3d` scene into an XR session.
//!
//! A frame's views become the renderer's views, its camera picture becomes
//! the background, and its real-world depth hides the parts of the scene that
//! are behind real things. On a phone `color` is the surface's texture view;
//! on a headset it is the two eye layers the swapchain hands over.
//!
//! With the `scene3d` feature, [`SessionFeed`] hands a runtime's phone
//! sessions to `leptoskit_3d`'s native views: a `Scene3d` whose backdrop is
//! the camera draws over the session's newest frame, from the session's
//! viewpoint, while one runs.

use glam::Mat3;
use glam::Mat4;
use leptoskit_3d::Background;
use leptoskit_3d::CameraImage;
use leptoskit_3d::CameraPlanes;
use leptoskit_3d::Occlusion;
use leptoskit_3d::RenderTarget;
use leptoskit_3d::TargetColor;
use leptoskit_3d::ViewParams;

use crate::host::XrFrame;
use crate::host::render::CameraViews;
use crate::host::render::FrameTextures;
use crate::host::render::affine_matrix;
use crate::host::render::view_matrices;

/// The nearest a real thing can be and still be drawn in front of, in metres.
const NEAR: f32 = 0.05;
/// The farthest the scene is drawn, in metres.
const FAR: f32 = 100.0;
/// How far behind real depth a surface may sit and still show, in metres:
/// enough to absorb the depth sensor's noise at arm's length.
const OCCLUSION_TOLERANCE: f32 = 0.05;

/// Draws `frame` into `color` with `renderer`, using the camera and depth
/// pictures `textures` uploaded for it.
pub fn draw(
	renderer: &mut leptoskit_3d::Renderer,
	frame: &XrFrame,
	textures: &FrameTextures,
	color: TargetColor<'_>,
	format: wgpu::TextureFormat,
	size: [u32; 2],
) -> Result<(), leptoskit_3d::Error> {
	let views: Vec<ViewParams> = frame
		.views
		.iter()
		.map(|view| {
			let (view_matrix, projection) = view_matrices(view, NEAR, FAR);
			ViewParams {
				view: Mat4::from_cols_array(&view_matrix),
				projection: Mat4::from_cols_array(&projection),
				viewport: [0.0, 0.0, size[0] as f32, size[1] as f32],
			}
		})
		.collect();

	let background = match textures.camera() {
		Some((CameraViews::YCbCr { y, cbcr }, transform)) => {
			Background::Camera(CameraImage {
				planes: CameraPlanes::YCbCr { y, cbcr },
				transform: Mat3::from_cols_array(&affine_matrix(&transform)),
			})
		}
		Some((CameraViews::Rgba(rgba), transform)) => {
			Background::Camera(CameraImage {
				planes: CameraPlanes::Rgba(rgba),
				transform: Mat3::from_cols_array(&affine_matrix(&transform)),
			})
		}
		None => Background::Scene,
	};

	let occlusion = textures.depth().map(|(depth, transform)| {
		Occlusion {
			depth,
			transform: Mat3::from_cols_array(&affine_matrix(&transform)),
			scale: 1.0,
			tolerance: OCCLUSION_TOLERANCE,
		}
	});

	renderer.render(&RenderTarget {
		color,
		format,
		size,
		views: &views,
		background,
		occlusion,
	})?;

	Ok(())
}

#[cfg(feature = "scene3d")]
pub use feed::SessionFeed;

#[cfg(feature = "scene3d")]
mod feed {
	use std::sync::Arc;

	use leptoskit_3d::Error;
	use leptoskit_3d::Renderer;
	use leptoskit_3d::TargetColor;
	use leptoskit_3d::host::CameraFeed;
	use leptoskit_3d::host::CameraPainter;
	use leptoskit_3d::host::CameraTarget;
	use leptoskit_3d::host::Hub;

	use crate::host::Viewport;
	use crate::host::XrRuntime;
	use crate::host::render::FrameTextures;
	use crate::types::Mode;

	/// An [`XrRuntime`]'s sessions as the camera feed `leptoskit_3d`'s native
	/// views draw scenes with the camera backdrop over.
	///
	/// Only phone sessions (AR and orientation) draw into a view: a headset's
	/// frames go to its own swapchain. A view's size and rotation become the
	/// session's viewport, so the backend crops the picture to fill the view
	/// and projects for it.
	#[derive(Debug)]
	pub struct SessionFeed {
		runtime: Arc<XrRuntime>,
	}

	impl SessionFeed {
		/// The feed for `runtime`'s sessions.
		#[must_use]
		pub fn new(runtime: Arc<XrRuntime>) -> Self {
			Self { runtime }
		}

		/// Makes `runtime`'s sessions `hub`'s camera feed, and wakes `hub`'s
		/// views whenever a session starts, changes, or stops.
		pub fn install(runtime: &Arc<XrRuntime>, hub: &'static Hub) {
			hub.set_camera_feed(Some(Arc::new(Self::new(Arc::clone(runtime)))));
			runtime.set_listener(Some(Arc::new(move || hub.wake_views())));
		}
	}

	/// Whether `runtime` runs a session a phone's view shows.
	fn showing(runtime: &XrRuntime) -> bool {
		runtime
			.session()
			.is_some_and(|session| matches!(session.mode, Mode::PhoneAr | Mode::Orientation))
	}

	impl CameraFeed for SessionFeed {
		fn live(&self) -> bool {
			showing(&self.runtime)
		}

		fn painter(&self) -> Box<dyn CameraPainter> {
			Box::new(SessionPainter {
				runtime: Arc::clone(&self.runtime),
				textures: FrameTextures::new(),
				uploaded: None,
				viewport: None,
			})
		}
	}

	/// One view's textures for a [`SessionFeed`].
	struct SessionPainter {
		runtime: Arc<XrRuntime>,
		textures: FrameTextures,
		/// The capture time, as bits, of the frame whose pictures `textures`
		/// hold.
		uploaded: Option<u64>,
		viewport: Option<Viewport>,
	}

	impl CameraPainter for SessionPainter {
		fn draw(
			&mut self,
			renderer: &mut Renderer,
			target: &CameraTarget<'_>,
		) -> Option<Result<(), Error>> {
			let viewport = Viewport {
				width: target.size[0],
				height: target.size[1],
				quarter_turns: target.quarter_turns,
			};

			// Told before a session starts too, so its first frame fits.
			if self.viewport != Some(viewport) {
				self.runtime.set_viewport(viewport);
				self.viewport = Some(viewport);
			}

			if !showing(&self.runtime) {
				return None;
			}

			// The frame is shared with the runtime; its pictures are copied
			// into this view's textures once, and it is not kept,
			// so the platform gets its camera buffers back.
			let frame = self.runtime.latest_frame()?;

			if self.uploaded != Some(frame.time.to_bits()) {
				let gpu = renderer.gpu();
				self.textures.upload(gpu.device(), gpu.queue(), &frame);
				self.uploaded = Some(frame.time.to_bits());
			}

			Some(super::draw(
				renderer,
				&frame,
				&self.textures,
				TargetColor::View(target.color),
				target.format,
				target.size,
			))
		}
	}
}
