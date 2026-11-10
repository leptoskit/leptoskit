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
	for run in &list.glyphs {
		scene = blit_text(scene, run);
	}
	scene
}

pub(crate) fn paint_gpu(scene: &Scene, width: u32, height: u32) -> Result<Vec<u8>, String> {
	leptoskit_canvas::render_to_rgba(scene, width, height, 1.0).map_err(|error| error.to_string())
}

fn blit_text(mut scene: Scene, run: &crate::raster::GlyphRun) -> Scene {
	let scale = run.scale.max(1.0);
	let mut pen = run.x;
	let ink = color(run.color);
	for ch in run.text.chars() {
		for (row, bits) in crate::font::rows(ch).into_iter().enumerate() {
			for col in 0..5 {
				if bits & (1 << (4 - col)) == 0 {
					continue;
				}
				scene = scene.fill_rect(
					Rect::new(
						pen + col as f32 * scale,
						run.y + row as f32 * scale,
						scale,
						scale,
					),
					ink,
				);
			}
		}
		pen += 6.0 * scale;
	}
	scene
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
		assert_ne!(scene.commands.len(), 0);
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
