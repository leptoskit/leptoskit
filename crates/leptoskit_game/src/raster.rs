//! Fills the frame on the CPU. The GPU painter draws the same list.

use crate::font;
use crate::project::Quad;

/// A screen-space rectangle in pixels, origin at the top left.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScreenRect {
	pub x: f32,
	pub y: f32,
	pub width: f32,
	pub height: f32,
	pub color: [u8; 4],
}

/// A run of HUD text.
#[derive(Debug, Clone)]
pub(crate) struct GlyphRun {
	pub x: f32,
	pub y: f32,
	pub text: String,
	pub color: [u8; 4],
	pub scale: f32,
}

/// Everything one frame draws, back to front.
#[derive(Debug, Default)]
pub(crate) struct DrawList {
	pub sky: [u8; 4],
	pub quads: Vec<Quad>,
	pub rects: Vec<ScreenRect>,
	pub glyphs: Vec<GlyphRun>,
}

pub(crate) fn paint(list: &DrawList, width: u32, height: u32) -> Vec<u8> {
	let mut pixels = vec![0_u8; width as usize * height as usize * 4];
	fill_rect(
		&mut pixels,
		width,
		height,
		&ScreenRect {
			x: 0.0,
			y: 0.0,
			width: width as f32,
			height: height as f32,
			color: list.sky,
		},
	);
	for quad in &list.quads {
		fill_polygon(&mut pixels, width, height, &quad.points, quad.color);
	}
	for rect in &list.rects {
		fill_rect(&mut pixels, width, height, rect);
	}
	for run in &list.glyphs {
		paint_run(&mut pixels, width, height, run);
	}
	pixels
}

fn fill_rect(pixels: &mut [u8], width: u32, height: u32, rect: &ScreenRect) {
	if rect.width <= 0.0 || rect.height <= 0.0 || rect.color[3] == 0 {
		return;
	}
	let x0 = rect.x.floor().max(0.0) as i32;
	let y0 = rect.y.floor().max(0.0) as i32;
	let x1 = (rect.x + rect.width).ceil().min(width as f32) as i32;
	let y1 = (rect.y + rect.height).ceil().min(height as f32) as i32;
	for y in y0..y1 {
		for x in x0..x1 {
			blend(pixels, width, x, y, rect.color);
		}
	}
}

fn fill_polygon(pixels: &mut [u8], width: u32, height: u32, points: &[(f32, f32)], color: [u8; 4]) {
	if points.len() < 3 || color[3] == 0 {
		return;
	}
	let min_y = points
		.iter()
		.map(|point| point.1)
		.fold(f32::INFINITY, f32::min);
	let max_y = points
		.iter()
		.map(|point| point.1)
		.fold(f32::NEG_INFINITY, f32::max);
	let y0 = min_y.floor().max(0.0) as i32;
	let y1 = max_y.ceil().min(height as f32) as i32;
	for y in y0..y1.max(y0) {
		let scan = y as f32 + 0.5;
		let mut crossings = Vec::new();
		for index in 0..points.len() {
			let (x0, y_start) = points[index];
			let (x1, y_end) = points[(index + 1) % points.len()];
			let crosses = (y_start <= scan && y_end > scan) || (y_end <= scan && y_start > scan);
			if crosses && (y_end - y_start).abs() > f32::EPSILON {
				let t = (scan - y_start) / (y_end - y_start);
				crossings.push(x0 + t * (x1 - x0));
			}
		}
		crossings.sort_by(f32::total_cmp);
		let mut index = 0;
		while index + 1 < crossings.len() {
			fill_span(
				pixels,
				width,
				height,
				crossings[index],
				crossings[index + 1],
				y,
				color,
			);
			index += 2;
		}
	}
}

fn fill_span(
	pixels: &mut [u8],
	width: u32,
	height: u32,
	left: f32,
	right: f32,
	y: i32,
	color: [u8; 4],
) {
	if !(0..height as i32).contains(&y) {
		return;
	}
	let x0 = left.min(right).floor().max(0.0) as i32;
	let x1 = left.max(right).ceil().min(width as f32) as i32;
	for x in x0..x1 {
		blend(pixels, width, x, y, color);
	}
}

fn paint_run(pixels: &mut [u8], width: u32, height: u32, run: &GlyphRun) {
	let scale = run.scale.max(1.0);
	let mut pen = run.x;
	for ch in run.text.chars() {
		for (row, bits) in font::rows(ch).into_iter().enumerate() {
			for col in 0..5 {
				if bits & (1 << (4 - col)) == 0 {
					continue;
				}
				fill_rect(
					pixels,
					width,
					height,
					&ScreenRect {
						x: pen + col as f32 * scale,
						y: run.y + row as f32 * scale,
						width: scale,
						height: scale,
						color: run.color,
					},
				);
			}
		}
		pen += 6.0 * scale;
	}
}

fn blend(pixels: &mut [u8], width: u32, x: i32, y: i32, color: [u8; 4]) {
	if x < 0 || y < 0 || x as u32 >= width {
		return;
	}
	let index = (y as usize * width as usize + x as usize) * 4;
	let Some(pixel) = pixels.get_mut(index..index + 4) else {
		return;
	};
	if color[3] == 255 {
		pixel.copy_from_slice(&color);
		return;
	}
	let alpha = f32::from(color[3]) / 255.0;
	for channel in 0..3 {
		let mixed = f32::from(color[channel]) * alpha + f32::from(pixel[channel]) * (1.0 - alpha);
		pixel[channel] = mixed.round() as u8;
	}
	pixel[3] = 255;
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_rect_and_a_glyph_leave_ink() {
		let list = DrawList {
			sky: [0, 0, 0, 255],
			quads: Vec::new(),
			rects: vec![ScreenRect {
				x: 1.0,
				y: 1.0,
				width: 4.0,
				height: 3.0,
				color: [10, 20, 30, 128],
			}],
			glyphs: vec![GlyphRun {
				x: 10.0,
				y: 2.0,
				text: "R".to_owned(),
				color: [255, 255, 255, 255],
				scale: 1.0,
			}],
		};
		let pixels = paint(&list, 40, 20);
		assert_eq!(pixel(&pixels, 40, 2, 2), [5, 10, 15, 255]);
		assert!(pixels.chunks(4).any(|pixel| pixel == [255, 255, 255, 255]));
		assert_eq!(
			paint(
				&DrawList {
					sky: [1, 2, 3, 255],
					..DrawList::default()
				},
				2,
				2
			)[0..4],
			[1, 2, 3, 255]
		);
	}

	fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
		let index = (y * width + x) as usize * 4;
		pixels[index..index + 4].try_into().unwrap()
	}
}
