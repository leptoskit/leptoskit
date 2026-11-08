//! The model: which file it is, loading it, and running it on a picture.

use std::fmt::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use sha2::Digest;
use sha2::Sha256;
use tract_onnx::prelude::*;

use crate::DepthMap;
use crate::Error;
use crate::Picture;
use crate::layout;
use crate::layout::Crop;

/// The Apache-2.0 licence the model is released under. Ship it with the
/// model file.
pub const MODEL_LICENSE: &str = include_str!("../model-license.txt");

/// A model file this crate is built and measured for, pinned by its digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFile {
	/// The file's name.
	pub name: &'static str,
	/// Where to download it. The digest, not the address, is what is
	/// trusted.
	pub url: &'static str,
	/// Its SHA-256, in lowercase hex.
	pub sha256: &'static str,
	/// Its size in bytes.
	pub size: u64,
	/// Where its weights come from.
	pub source: &'static str,
	/// The licence its weights are released under; the text is
	/// [`MODEL_LICENSE`].
	pub license: &'static str,
}

impl ModelFile {
	/// Fails unless `bytes` are this file.
	pub fn verify(&self, bytes: &[u8]) -> Result<(), Error> {
		let actual =
			Sha256::digest(bytes)
				.iter()
				.fold(String::with_capacity(64), |mut hex, byte| {
					let _ = write!(hex, "{byte:02x}");
					hex
				});

		if actual == self.sha256 {
			Ok(())
		} else {
			Err(Error::Checksum {
				expected: self.sha256,
				actual,
			})
		}
	}
}

/// Depth Anything V2 Small, the authors' fine-tune for metric depth indoors,
/// exported to ONNX for a 252 × 448 upright picture with its weights
/// quantised to 8 bits (99 MB to 27 MB, the same accuracy against NYU Depth
/// v2). Only the Small size is under Apache-2.0; do not swap in a larger
/// one, whose licence is non-commercial.
///
/// The file was converted for the Reapers game, and is served from the
/// commit that added it there.
pub const MODEL: ModelFile = ModelFile {
	name: "depth_anything_v2_metric_indoor_small.onnx",
	url: "https://raw.githubusercontent.com/pina-rs/reapers/7a10588e9d7e76ff54370e674dbc6cfb6ae97fb6/reapers_flutter/assets/depth/depth_anything_v2_metric_indoor_small.onnx",
	sha256: "b81c4fae8985bd73d5026ffc1ebf545ff01d73fbf4d534a06c811abfcadb6dbd",
	size: 27_237_510,
	source: "https://huggingface.co/depth-anything/Depth-Anything-V2-Metric-Indoor-Small-hf/tree/8078d68a9c75a972131914f6afd0c1723be0da7f",
	license: "Apache-2.0",
};

/// How much too far the model reads every distance in a picture shown to it
/// through a lens whose focal length is `focal` of its input's pixels.
///
/// The model learnt its metres from pictures about twice as sharp as the
/// 252-pixel-wide ones it is given here, and the smaller a room is drawn the
/// farther away it takes it to be: 3% long at 518 pixels across, 45% long at
/// 252. Against rooms of known depth (NYU Depth v2) the error is a steady
/// factor, varying about 7% from room to room, that follows this curve,
/// fitted for the Reapers game. [`DepthModel::estimate`] divides it out.
#[must_use]
pub fn overreach(focal: f32) -> f32 {
	1.026 * (579.0 / focal.clamp(200.0, 700.0)).powf(0.48)
}

/// A loaded depth model, ready to run on pictures. Loading takes about half
/// a second; keep it.
#[derive(Debug, Clone)]
pub struct DepthModel {
	plan: Arc<TypedRunnableModel>,
	width: usize,
	height: usize,
}

impl DepthModel {
	/// Loads an ONNX depth model from its bytes, such as [`MODEL`] read from
	/// the app's assets. The model must take one fixed-size RGB picture,
	/// `[1, 3, height, width]`, and give one depth picture of the same size.
	pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
		let model = onnx()
			.model_for_read(&mut &*bytes)
			.map_err(|error| Error::Model(format!("{error:#}")))?
			.into_optimized()
			.map_err(|error| Error::Model(format!("{error:#}")))?;
		let (width, height) = input_size(&model)?;
		check_output(&model, width, height)?;
		let plan = model
			.into_runnable()
			.map_err(|error| Error::Model(format!("{error:#}")))?;

		Ok(Self {
			plan,
			width,
			height,
		})
	}

	/// Loads an ONNX depth model from a file: see [`DepthModel::from_bytes`].
	pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Error> {
		let path = path.as_ref();
		let bytes = std::fs::read(path).map_err(|error| {
			Error::Model(format!("{} could not be read: {error}", path.display()))
		})?;

		Self::from_bytes(&bytes)
	}

	/// The upright picture the model reads, width and height in pixels.
	#[must_use]
	pub fn input_size(&self) -> (u32, u32) {
		(self.width as u32, self.height as u32)
	}

	/// How far away everything in `picture` is.
	///
	/// The picture is turned upright, cropped to the model's shape from its
	/// middle, and averaged down to the model's size; the model's answer is
	/// laid back over the picture as it was, at the model's resolution, and
	/// its scale corrected for the lens (see [`overreach`]). Outside the
	/// crop the depth is unknown. About a quarter of a second on one core
	/// of an Apple M4; run it off the thread that draws, as
	/// [`DepthWorker`](crate::DepthWorker) does.
	pub fn estimate(&self, picture: &Picture) -> Result<DepthMap, Error> {
		let started = Instant::now();
		picture.pixels.check()?;
		let (width, height) = picture.size();
		let upright = picture.orientation.upright_size(width, height);
		let crop = Crop::centred(upright, self.width, self.height);
		let output = self.run(&layout::input(picture, crop))?;
		let lens = picture.lens();
		// The focal length across the upright picture, in the input's pixels.
		let across = lens.focal[usize::from(picture.orientation.is_sideways())];
		let scale = overreach(across / crop.step);
		let (grid_width, grid_height, depths) =
			layout::depth_grid(&output, crop, (width, height), picture.orientation, scale);

		Ok(DepthMap {
			width: grid_width,
			height: grid_height,
			depths,
			intrinsics: lens.scaled(grid_width, grid_height),
			scale,
			took: started.elapsed(),
		})
	}

	fn run(&self, input: &[f32]) -> Result<Vec<f32>, Error> {
		let tensor = Tensor::from_shape(&[1, 3, self.height, self.width], input)
			.map_err(|error| Error::Inference(format!("{error:#}")))?;
		let outputs = self
			.plan
			.run(tvec!(tensor.into()))
			.map_err(|error| Error::Inference(format!("{error:#}")))?;
		let output = outputs
			.first()
			.ok_or_else(|| Error::Inference("the model gave no output".into()))?
			.to_plain_array_view::<f32>()
			.map_err(|error| Error::Inference(format!("{error:#}")))?;

		if output.len() == self.width * self.height {
			Ok(output.iter().copied().collect())
		} else {
			Err(Error::Inference(format!(
				"the model gave {} values for a {} × {} picture",
				output.len(),
				self.width,
				self.height
			)))
		}
	}
}

/// The fixed width and height of a model's one `[1, 3, height, width]`
/// input.
fn input_size(model: &TypedModel) -> Result<(usize, usize), Error> {
	let fact = model
		.input_fact(0)
		.map_err(|error| Error::Model(format!("{error:#}")))?;
	let shape = fact
		.shape
		.as_concrete()
		.ok_or_else(|| Error::Model("its input has no fixed size".into()))?;

	match *shape {
		[1, 3, height, width] if height > 0 && width > 0 => Ok((width, height)),
		_ => {
			Err(Error::Model(format!(
				"its input is {shape:?}, not [1, 3, height, width]"
			)))
		}
	}
}

/// Fails unless the model gives one depth for each input pixel.
fn check_output(model: &TypedModel, width: usize, height: usize) -> Result<(), Error> {
	let fact = model
		.output_fact(0)
		.map_err(|error| Error::Model(format!("{error:#}")))?;
	let volume = fact
		.shape
		.as_concrete()
		.map(|shape| shape.iter().product::<usize>());

	if fact.datum_type == f32::datum_type() && volume == Some(width * height) {
		Ok(())
	} else {
		Err(Error::Model(format!(
			"its output is {:?} {:?}, not one depth for each of the {width} × {height} input's \
			 pixels",
			fact.datum_type, fact.shape
		)))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_lens_correction_matches_the_seeker() {
		// The Seeker's camera, upright and cropped to 252 × 448: 5.24 mm on
		// a 5.76 mm-wide sensor, three quarters of which the crop keeps.
		let tan_half = 0.75 * 2.88 / 5.24;
		let focal = 252.0 / (2.0 * tan_half);
		assert!(
			(overreach(focal) - 1.39).abs() < 0.01,
			"{}",
			overreach(focal)
		);
		// Outside the fitted range the curve holds its ends.
		assert_eq!(overreach(10.0), overreach(200.0));
	}

	#[test]
	fn the_pin_checks_bytes() {
		assert!(matches!(
			MODEL.verify(b"not a model"),
			Err(Error::Checksum { .. })
		));
		assert!(MODEL_LICENSE.starts_with("                                 Apache License"));
	}

	#[test]
	fn bytes_that_are_not_a_model_are_refused() {
		assert!(matches!(
			DepthModel::from_bytes(b"nope"),
			Err(Error::Model(_))
		));
	}
}
