//! Turns a draw list into a canvas scene and asks wgpu to paint it.

use leptoskit_canvas::Color;
use leptoskit_canvas::FillRule;
use leptoskit_canvas::Path;
use leptoskit_canvas::Rect;
use leptoskit_canvas::Scene;

use crate::raster::DrawList;
use crate::raster::ScreenRect;

pub(crate) fn to_scene(list: &DrawList, width: u32, height: u32) -> Scene {
	let mut scene = Scene::new().fill_rect(
		Rect::new(0.0, 0.0, width as f32, height as f32),
		color(list.sky),
	);
	for quad in &list.quads {
		let mut path = Path::new().move_to(quad.points[0].0, quad.points[0].1);
		for point in &quad.points[1..] {
			path = path.line_to(point.0, point.1);
		}
		scene = scene.fill_path(path.close(), FillRule::NonZero, color(quad.color));
	}
	for rect in &list.rects {
		scene = scene.fill_rect(rect_of(rect), color(rect.color));
	}
	scene
}

/// One wgpu device and renderer for every frame of a run.
///
/// [`leptoskit_canvas::render_to_rgba`] builds a device per call. A game keeps
/// this instead, and remembers a missing adapter so later frames do not ask
/// again.
pub(crate) struct GpuPainter {
	renderer: Option<leptoskit_canvas::Renderer>,
	missed: bool,
}

impl GpuPainter {
	pub(crate) fn new() -> Self {
		Self {
			renderer: None,
			missed: false,
		}
	}

	/// Draws `scene`. HUD text is not in the scene; the caller stamps it after.
	pub(crate) fn paint(
		&mut self,
		scene: &Scene,
		width: u32,
		height: u32,
	) -> Result<Vec<u8>, String> {
		if self.missed {
			return Err("no GPU adapter is available".to_owned());
		}
		if self.renderer.is_none() {
			let (device, queue) = leptoskit_canvas::headless_device().map_err(|error| {
				self.missed = true;
				error.to_string()
			})?;
			self.renderer = Some(leptoskit_canvas::Renderer::new(
				&device,
				&queue,
				leptoskit_canvas::wgpu::TextureFormat::Rgba8Unorm,
			));
		}
		let Some(renderer) = self.renderer.as_mut() else {
			return Err("the renderer was not installed".to_owned());
		};
		renderer
			.render_to_rgba(scene, width, height, 1.0)
			.map_err(|error| error.to_string())
	}
}

#[cfg(test)]
fn paint_gpu(scene: &Scene, width: u32, height: u32) -> Result<Vec<u8>, String> {
	GpuPainter::new().paint(scene, width, height)
}

fn rect_of(rect: &ScreenRect) -> Rect {
	Rect::new(rect.x, rect.y, rect.width, rect.height)
}

fn color(bytes: [u8; 4]) -> Color {
	Color::rgba8(bytes[0], bytes[1], bytes[2], bytes[3])
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::project::SKY;
	use crate::raster::DrawList;

	#[test]
	fn a_draw_list_becomes_a_scene_with_a_sky() {
		let scene = to_scene(
			&DrawList {
				sky: SKY,
				..DrawList::default()
			},
			8,
			4,
		);
		assert_eq!(scene.commands.len(), 1);
	}

	#[test]
	fn hud_text_is_not_a_draw_per_pixel() {
		let scene = to_scene(
			&DrawList {
				sky: SKY,
				glyphs: vec![crate::raster::GlyphRun {
					x: 4.0,
					y: 4.0,
					text: "Restart".to_owned(),
					color: [255, 255, 255, 255],
					scale: 2.0,
				}],
				..DrawList::default()
			},
			64,
			32,
		);
		assert_eq!(scene.commands.len(), 1);
	}

	#[test]
	fn a_missing_adapter_is_not_requested_again() {
		let mut painter = GpuPainter::new();
		painter.missed = true;
		let first = painter.paint(&Scene::new(), 4, 4).unwrap_err();
		let second = painter.paint(&Scene::new(), 4, 4).unwrap_err();
		assert_eq!(first, second);
		assert!(first.contains("adapter"));
	}

	#[test]
	fn the_gpu_paints_or_reports_a_missing_adapter() {
		let scene = to_scene(
			&DrawList {
				sky: SKY,
				..DrawList::default()
			},
			8,
			4,
		);
		match paint_gpu(&scene, 8, 4) {
			Ok(pixels) => assert_eq!(pixels.len(), 8 * 4 * 4),
			Err(error) => assert_ne!(error.len(), 0),
		}
	}
}
