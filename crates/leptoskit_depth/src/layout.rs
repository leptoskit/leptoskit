//! Cutting a picture down to the model's input, and laying its output back
//! over the picture.

use crate::Orientation;
use crate::Picture;

/// The colour statistics the model was trained with, per channel.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const SPREAD: [f32; 3] = [0.229, 0.224, 0.225];
/// The most samples taken along each side of the block of the picture one
/// input pixel stands for.
const MAX_TAPS: u32 = 4;

/// Where the model's input lies in the upright picture: the largest centred
/// rectangle of the input's shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Crop {
	/// The rectangle's top-left corner, in upright picture pixels.
	pub left: f32,
	pub top: f32,
	/// Upright picture pixels per input pixel, the same both ways.
	pub step: f32,
	/// The input's width and height in pixels.
	pub width: usize,
	pub height: usize,
}

impl Crop {
	/// The crop of an upright picture `upright` in size for an input
	/// `width` by `height`.
	pub fn centred(upright: (u32, u32), width: usize, height: usize) -> Self {
		let (upright_width, upright_height) = (upright.0 as f32, upright.1 as f32);
		let shape = width as f32 / height as f32;
		let crop_width = if upright_width / upright_height > shape {
			upright_height * shape
		} else {
			upright_width
		};
		let step = crop_width / width as f32;

		Self {
			left: (upright_width - crop_width) / 2.0,
			top: (upright_height - height as f32 * step) / 2.0,
			step,
			width,
			height,
		}
	}

	/// How many samples to take along each side of an input pixel's block:
	/// enough that fine detail averages out instead of turning to noise.
	fn taps(self) -> u32 {
		(self.step.ceil() as u32).clamp(1, MAX_TAPS)
	}
}

/// The model's input: the crop of the upright picture, each pixel the mean
/// of its block, normalised and laid out as three planes (red, green, blue)
/// of rows from the top.
pub(crate) fn input(picture: &Picture, crop: Crop) -> Vec<f32> {
	let area = crop.width * crop.height;
	let mut planes = vec![0.0; area * 3];

	for row in 0..crop.height {
		for column in 0..crop.width {
			let rgb = block_mean(picture, crop, column, row);
			let at = row * crop.width + column;

			for channel in 0..3 {
				planes[channel * area + at] =
					(rgb[channel] / 255.0 - MEAN[channel]) / SPREAD[channel];
			}
		}
	}

	planes
}

/// The mean colour, from 0 to 255, of the block of the picture under input
/// pixel `(column, row)`.
fn block_mean(picture: &Picture, crop: Crop, column: usize, row: usize) -> [f32; 3] {
	let (width, height) = picture.size();
	let (picture_width, picture_height) = (width as f32, height as f32);
	let taps = crop.taps();
	let mut sum = [0.0; 3];

	for tap in 0..taps * taps {
		let offset_x = ((tap % taps) as f32 + 0.5) / taps as f32;
		let offset_y = ((tap / taps) as f32 + 0.5) / taps as f32;
		let upright = (
			crop.left + (column as f32 + offset_x) * crop.step,
			crop.top + (row as f32 + offset_y) * crop.step,
		);
		let (x, y) = picture
			.orientation
			.to_picture(picture_width, picture_height, upright);
		let rgb = picture.pixels.rgb(pixel(x, width), pixel(y, height));

		for channel in 0..3 {
			sum[channel] += rgb[channel];
		}
	}

	sum.map(|total| total / (taps * taps) as f32)
}

/// The pixel a continuous coordinate falls in, kept inside `size`.
fn pixel(coordinate: f32, size: u32) -> u32 {
	(coordinate.floor().max(0.0) as u32).min(size - 1)
}

/// The model's output, `crop.width` by `crop.height` values in the upright
/// crop, laid back over the picture as it is: a grid at the input's
/// resolution covering the whole picture, each value divided by `divide`,
/// and zero outside the crop. Returns the grid's width, height, and values.
pub(crate) fn depth_grid(
	output: &[f32],
	crop: Crop,
	picture: (u32, u32),
	orientation: Orientation,
	divide: f32,
) -> (u32, u32, Vec<f32>) {
	let (picture_width, picture_height) = (picture.0 as f32, picture.1 as f32);
	let width = ((picture_width / crop.step).round() as u32).max(1);
	let height = ((picture_height / crop.step).round() as u32).max(1);
	let (step_x, step_y) = (picture_width / width as f32, picture_height / height as f32);
	let mut depths = Vec::with_capacity(width as usize * height as usize);

	for row in 0..height {
		for column in 0..width {
			let point = ((column as f32 + 0.5) * step_x, (row as f32 + 0.5) * step_y);
			let (x, y) = orientation.to_upright(picture_width, picture_height, point);
			let input = (
				(x - crop.left) / crop.step - 0.5,
				(y - crop.top) / crop.step - 0.5,
			);
			let depth = bilinear(output, crop.width, crop.height, input)
				.map(|metres| metres / divide)
				.filter(|metres| metres.is_finite() && *metres > 0.0);

			depths.push(depth.unwrap_or(0.0));
		}
	}

	(width, height, depths)
}

/// `values`, `width` by `height`, read between pixel centres at `(x, y)`,
/// or `None` outside the pixels' edges.
fn bilinear(values: &[f32], width: usize, height: usize, (x, y): (f32, f32)) -> Option<f32> {
	let inside = |at: f32, size: usize| (-0.5..=size as f32 - 0.5).contains(&at);

	if !inside(x, width) || !inside(y, height) {
		return None;
	}

	let x = x.clamp(0.0, (width - 1) as f32);
	let y = y.clamp(0.0, (height - 1) as f32);
	let (left, top) = (x.floor() as usize, y.floor() as usize);
	let (right, bottom) = ((left + 1).min(width - 1), (top + 1).min(height - 1));
	let (across, down) = (x - left as f32, y - top as f32);
	let at = |column: usize, row: usize| values[row * width + column];
	let upper = at(left, top) * (1.0 - across) + at(right, top) * across;
	let lower = at(left, bottom) * (1.0 - across) + at(right, bottom) * across;

	Some(upper * (1.0 - down) + lower * down)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::Pixels;
	use crate::Plane;

	#[test]
	fn a_wide_picture_loses_its_sides_and_a_tall_one_its_ends() {
		// 3:4 upright into a 9:16 input: the sides go.
		let crop = Crop::centred((1080, 1440), 252, 448);
		assert!((crop.step - 1440.0 / 448.0).abs() < 1e-4);
		assert!((crop.left - (1080.0 - 252.0 * crop.step) / 2.0).abs() < 1e-3);
		assert!(crop.top.abs() < 1e-3);

		// 9:21 upright: the top and bottom go.
		let crop = Crop::centred((900, 2100), 252, 448);
		assert!((crop.step - 900.0 / 252.0).abs() < 1e-4);
		assert!(crop.left.abs() < 1e-3 && crop.top > 0.0);
	}

	#[test]
	fn a_flat_colour_becomes_the_trained_normalisation() {
		let pixels = Pixels::Rgb8(Plane::packed(8, 16, 3, [255_u8, 0, 128].repeat(8 * 16)));
		let picture = Picture::new(pixels);
		let crop = Crop::centred((8, 16), 4, 8);
		let input = input(&picture, crop);
		let area = 32;
		assert!((input[0] - (1.0 - MEAN[0]) / SPREAD[0]).abs() < 1e-5);
		assert!((input[area] - (0.0 - MEAN[1]) / SPREAD[1]).abs() < 1e-5);
		assert!((input[2 * area + 31] - (128.0 / 255.0 - MEAN[2]) / SPREAD[2]).abs() < 1e-5);
	}

	/// A picture whose red is its column and green its row, so a pixel read
	/// says where it came from.
	fn coordinates(width: u32, height: u32) -> Pixels {
		let data: Vec<u8> = (0..height)
			.flat_map(|row| (0..width).flat_map(move |column| [column as u8, row as u8, 0]))
			.collect();
		Pixels::Rgb8(Plane::packed(width, height, 3, data))
	}

	#[test]
	fn a_sideways_picture_is_read_upright() {
		// 16 × 8 sensor, upright after a clockwise quarter turn: 8 × 16.
		let picture = Picture::new(coordinates(16, 8)).with_orientation(Orientation::Clockwise);
		let crop = Crop::centred((8, 16), 8, 16);
		let input = input(&picture, crop);
		let red = |column: usize, row: usize| input[row * 8 + column] * SPREAD[0] + MEAN[0];
		let green = |column: usize, row: usize| input[128 + row * 8 + column] * SPREAD[1] + MEAN[1];
		// The upright top-left is the sensor's bottom-left: column 0, row 7.
		assert!((red(0, 0) * 255.0).abs() < 1e-3);
		assert!((green(0, 0) * 255.0 - 7.0).abs() < 1e-3);
		// Down the upright picture runs along the sensor's columns.
		assert!((red(0, 15) * 255.0 - 15.0).abs() < 1e-3);
		// Across it runs up the sensor's rows.
		assert!((green(7, 0) * 255.0).abs() < 1e-3);
	}

	#[test]
	fn the_output_lies_back_over_the_picture_as_it_was() {
		// An upright 2 × 4 output whose value is its row plus one, from a
		// clockwise 4 × 2 sensor: rows of the upright picture are the
		// sensor's columns.
		let output = [1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0];
		let crop = Crop::centred((2, 4), 2, 4);
		let (width, height, depths) =
			depth_grid(&output, crop, (4, 2), Orientation::Clockwise, 1.0);
		assert_eq!((width, height), (4, 2));
		assert_eq!(depths, [1.0, 2.0, 3.0, 4.0, 1.0, 2.0, 3.0, 4.0]);
	}

	#[test]
	fn outside_the_crop_is_unknown_and_the_scale_divides_out() {
		// A square upright picture into a 1:2 input keeps its middle half.
		let output = [2.0; 8];
		let crop = Crop::centred((8, 8), 2, 4);
		let (width, height, depths) = depth_grid(&output, crop, (8, 8), Orientation::Upright, 2.0);
		assert_eq!((width, height), (4, 4));
		for row in depths.chunks(4) {
			assert_eq!(row, [0.0, 1.0, 1.0, 0.0]);
		}
	}
}
