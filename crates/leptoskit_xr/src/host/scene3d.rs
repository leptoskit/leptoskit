//! Drawing a `leptoskit_3d` scene into an XR session.
//!
//! A frame's views become the renderer's views, its camera picture becomes
//! the background, and its real-world depth hides the parts of the scene that
//! are behind real things. On a phone `color` is the surface's texture view;
//! on a headset it is the two eye layers the swapchain hands over.

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
