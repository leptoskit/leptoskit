//! Decodes a model's PNG and JPEG textures to RGBA with 8 bits a channel.

use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use crate::asset::Image;
use crate::sanitize::MAX_TEXTURE_SIZE;

/// Decodes a PNG or JPEG, refusing anything wider or taller than
/// [`MAX_TEXTURE_SIZE`].
pub(crate) fn decode(bytes: &[u8]) -> Result<Image, String> {
	if bytes.starts_with(b"\x89PNG") {
		png_rgba(bytes)
	} else if bytes.starts_with(&[0xff, 0xd8]) {
		jpeg_rgba(bytes)
	} else {
		Err("only PNG and JPEG textures are supported".into())
	}
}

fn png_rgba(bytes: &[u8]) -> Result<Image, String> {
	let limit = MAX_TEXTURE_SIZE as usize;
	let mut decoder = png::Decoder::new_with_limits(
		bytes,
		png::Limits {
			bytes: limit * limit * 8,
		},
	);
	decoder.set_transformations(png::Transformations::normalize_to_color8());
	let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
	let (width, height) = (reader.info().width, reader.info().height);
	if width == 0 || height == 0 || width > MAX_TEXTURE_SIZE || height > MAX_TEXTURE_SIZE {
		return Err(format!("a {width}×{height} texture is outside the limits"));
	}
	let mut buffer = vec![0; reader.output_buffer_size()];
	let info = reader
		.next_frame(&mut buffer)
		.map_err(|error| error.to_string())?;
	buffer.truncate(info.buffer_size());
	let pixels = match info.color_type {
		png::ColorType::Rgba => buffer,
		png::ColorType::Rgb => {
			buffer
				.as_chunks::<3>()
				.0
				.iter()
				.flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
				.collect()
		}
		png::ColorType::GrayscaleAlpha => {
			buffer
				.as_chunks::<2>()
				.0
				.iter()
				.flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
				.collect()
		}
		png::ColorType::Grayscale => {
			buffer
				.iter()
				.flat_map(|value| [*value, *value, *value, 255])
				.collect()
		}
		png::ColorType::Indexed => return Err("an indexed PNG did not expand".into()),
	};
	Ok(Image {
		width,
		height,
		pixels,
	})
}

fn jpeg_rgba(bytes: &[u8]) -> Result<Image, String> {
	let limit = MAX_TEXTURE_SIZE as usize;
	let options = DecoderOptions::default()
		.set_max_width(limit)
		.set_max_height(limit)
		.jpeg_set_out_colorspace(ColorSpace::RGBA);
	let mut decoder = JpegDecoder::new_with_options(bytes, options);
	let pixels = decoder.decode().map_err(|error| error.to_string())?;
	let info = decoder.info().ok_or("the JPEG has no header")?;
	let (width, height) = (u32::from(info.width), u32::from(info.height));
	if pixels.len() != (width * height * 4) as usize {
		return Err("the JPEG decoded to the wrong size".into());
	}
	Ok(Image {
		width,
		height,
		pixels,
	})
}
