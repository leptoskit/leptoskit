//! Decoders for `EXT_meshopt_compression`: the vertex and index codecs of
//! [meshoptimizer](https://github.com/zeux/meshoptimizer) (MIT) and the
//! extension's filters, written from the extension's specification so the
//! loader needs no C++ toolchain on any platform.
//!
//! Every read is bounds-checked: malformed input is an error, never a panic
//! or an out-of-bounds read.

use std::fmt;

/// Why a compressed buffer view could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshoptError(pub &'static str);

impl fmt::Display for MeshoptError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(formatter, "meshopt: {}", self.0)
	}
}

impl std::error::Error for MeshoptError {}

type Result<T> = std::result::Result<T, MeshoptError>;

/// How a buffer view was compressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
	/// Vertex attributes.
	Attributes,
	/// A triangle list's indices.
	Triangles,
	/// Any other indices.
	Indices,
}

/// The filter applied after decoding attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
	/// None.
	None,
	/// Octahedral unit vectors.
	Octahedral,
	/// Unit quaternions.
	Quaternion,
	/// Floats with a shared exponent.
	Exponential,
}

/// Decodes `count` elements of `stride` bytes compressed with `mode`, then
/// applies `filter`.
pub fn decode(
	source: &[u8],
	count: usize,
	stride: usize,
	mode: Mode,
	filter: Filter,
) -> Result<Vec<u8>> {
	let size = count
		.checked_mul(stride)
		.ok_or(MeshoptError("the decoded size overflows"))?;
	let mut output = vec![0; size];

	match mode {
		Mode::Attributes => {
			decode_vertices(&mut output, count, stride, source)?;
			apply_filter(&mut output, count, stride, filter)?;
		}
		Mode::Triangles => {
			if filter != Filter::None {
				return Err(MeshoptError("only attributes take a filter"));
			}

			decode_triangles(&mut output, count, stride, source)?;
		}
		Mode::Indices => {
			if filter != Filter::None {
				return Err(MeshoptError("only attributes take a filter"));
			}

			decode_index_sequence(&mut output, count, stride, source)?;
		}
	}

	Ok(output)
}

struct Cursor<'a> {
	bytes: &'a [u8],
	position: usize,
}

impl Cursor<'_> {
	fn byte(&mut self) -> Result<u8> {
		let byte = *self
			.bytes
			.get(self.position)
			.ok_or(MeshoptError("the data ends early"))?;
		self.position += 1;
		Ok(byte)
	}

	fn remaining(&self) -> usize {
		self.bytes.len() - self.position
	}

	/// A LEB128 integer of at most five bytes.
	fn vbyte(&mut self) -> Result<u32> {
		let lead = self.byte()?;

		if lead < 128 {
			return Ok(u32::from(lead));
		}

		let mut result = u32::from(lead & 127);
		let mut shift = 7;

		for _ in 0..4 {
			let group = self.byte()?;
			result |= u32::from(group & 127) << shift;

			shift += 7;

			if group < 128 {
				break;
			}
		}

		Ok(result)
	}
}

const VERTEX_HEADER: u8 = 0xa0;
const VERTEX_BLOCK_BYTES: usize = 8192;
const VERTEX_BLOCK_MAX: usize = 256;
const BYTE_GROUP: usize = 16;
const TAIL_MAX: usize = 32;

fn unzigzag8(value: u8) -> u8 {
	(value >> 1) ^ 0_u8.wrapping_sub(value & 1)
}

fn unzigzag32(value: u32) -> u32 {
	(value >> 1) ^ 0_u32.wrapping_sub(value & 1)
}

/// Decodes one group of sixteen bytes, each stored in `bits` bits, with the
/// all-ones value escaping to a full byte that follows the group.
fn decode_group(cursor: &mut Cursor<'_>, output: &mut [u8; BYTE_GROUP], mode: u8) -> Result<()> {
	match mode {
		0 => output.fill(0),
		1 | 2 => {
			let bits = if mode == 1 { 2 } else { 4 };
			let packed = BYTE_GROUP * bits / 8;
			let start = cursor.position;
			if cursor.remaining() < packed {
				return Err(MeshoptError("the data ends early"));
			}

			cursor.position += packed;
			let sentinel = (1_u8 << bits) - 1;
			for (index, value) in output.iter_mut().enumerate() {
				let byte = cursor.bytes[start + index * bits / 8];
				let shift = 8 - bits - (index * bits) % 8;
				let encoded = (byte >> shift) & sentinel;
				*value = if encoded == sentinel {
					cursor.byte()?
				} else {
					encoded
				};
			}
		}
		_ => {
			for value in output.iter_mut() {
				*value = cursor.byte()?;
			}
		}
	}

	Ok(())
}

fn decode_vertices(output: &mut [u8], count: usize, stride: usize, source: &[u8]) -> Result<()> {
	if stride == 0 || stride > 256 || stride % 4 != 0 {
		return Err(MeshoptError(
			"a vertex stride must be a multiple of 4 up to 256",
		));
	}

	if source.len() < 1 + stride {
		return Err(MeshoptError("the vertex data is too short"));
	}

	let header = source[0];

	if header & 0xf0 != VERTEX_HEADER {
		return Err(MeshoptError("the vertex data has the wrong header"));
	}

	if header & 0x0f != 0 {
		return Err(MeshoptError(
			"only version 0 of the vertex codec is supported",
		));
	}

	let tail = stride.max(TAIL_MAX);

	if source.len() < 1 + tail {
		return Err(MeshoptError("the vertex data is too short"));
	}

	let mut last = [0_u8; 256];
	last[..stride].copy_from_slice(&source[source.len() - stride..]);
	let mut cursor = Cursor {
		bytes: &source[..source.len() - tail],
		position: 1,
	};
	let block = ((VERTEX_BLOCK_BYTES / stride) & !(BYTE_GROUP - 1)).min(VERTEX_BLOCK_MAX);
	let mut encoded = [0_u8; VERTEX_BLOCK_MAX];
	let mut offset = 0;

	while offset < count {
		let size = block.min(count - offset);
		let groups = size.div_ceil(BYTE_GROUP);

		for byte in 0..stride {
			let header_size = groups.div_ceil(4);

			if cursor.remaining() < header_size {
				return Err(MeshoptError("the vertex data ends early"));
			}

			let headers = cursor.position;
			cursor.position += header_size;

			for group in 0..groups {
				let mode = (cursor.bytes[headers + group / 4] >> ((group % 4) * 2)) & 3;
				let slot: &mut [u8; BYTE_GROUP] = (&mut encoded
					[group * BYTE_GROUP..(group + 1) * BYTE_GROUP])
					.try_into()
					.map_err(|_| MeshoptError("a group is misaligned"))?;
				decode_group(&mut cursor, slot, mode)?;
			}

			let mut previous = last[byte];

			for (vertex, value) in encoded[..size].iter().enumerate() {
				previous = previous.wrapping_add(unzigzag8(*value));
				output[(offset + vertex) * stride + byte] = previous;
			}

			last[byte] = previous;
		}

		offset += size;
	}

	if cursor.remaining() != 0 {
		return Err(MeshoptError("the vertex data has bytes left over"));
	}

	Ok(())
}

const INDEX_HEADER: u8 = 0xe0;
const SEQUENCE_HEADER: u8 = 0xd0;

fn write_index(output: &mut [u8], index: usize, size: usize, value: u32) {
	let start = index * size;

	if size == 2 {
		output[start..start + 2].copy_from_slice(&(value as u16).to_le_bytes());
	} else {
		output[start..start + 4].copy_from_slice(&value.to_le_bytes());
	}
}

/// The decoder's two FIFOs of recent edges and vertices.
struct Fifos {
	edges: [[u32; 2]; 16],
	edge_offset: usize,
	vertices: [u32; 16],
	vertex_offset: usize,
}

impl Fifos {
	fn push_edge(&mut self, a: u32, b: u32) {
		self.edges[self.edge_offset] = [a, b];
		self.edge_offset = (self.edge_offset + 1) & 15;
	}

	fn push_vertex(&mut self, vertex: u32, advance: bool) {
		self.vertices[self.vertex_offset] = vertex;
		self.vertex_offset = (self.vertex_offset + usize::from(advance)) & 15;
	}

	fn edge(&self, back: usize) -> [u32; 2] {
		self.edges[(self.edge_offset.wrapping_sub(1 + back)) & 15]
	}

	fn vertex(&self, back: usize) -> u32 {
		self.vertices[(self.vertex_offset.wrapping_sub(back)) & 15]
	}
}

#[allow(clippy::too_many_lines)]
fn decode_triangles(output: &mut [u8], count: usize, size: usize, source: &[u8]) -> Result<()> {
	if count % 3 != 0 {
		return Err(MeshoptError(
			"a triangle list's index count must divide by 3",
		));
	}

	if size != 2 && size != 4 {
		return Err(MeshoptError("indices must be 2 or 4 bytes"));
	}

	let triangles = count / 3;

	if source.len() < 1 + triangles + 16 {
		return Err(MeshoptError("the index data is too short"));
	}

	if source[0] & 0xf0 != INDEX_HEADER {
		return Err(MeshoptError("the index data has the wrong header"));
	}

	let version = source[0] & 0x0f;

	if version > 1 {
		return Err(MeshoptError(
			"only versions 0 and 1 of the index codec are supported",
		));
	}

	let codes = &source[1..=triangles];
	let aux = &source[source.len() - 16..];
	let mut data = Cursor {
		bytes: &source[..source.len() - 16],
		position: 1 + triangles,
	};
	let mut fifos = Fifos {
		edges: [[u32::MAX; 2]; 16],
		edge_offset: 0,
		vertices: [u32::MAX; 16],
		vertex_offset: 0,
	};
	let fec_max = if version >= 1 { 13 } else { 15 };
	let mut next = 0_u32;
	let mut last = 0_u32;
	let decode_index = |data: &mut Cursor<'_>, last: &mut u32| -> Result<u32> {
		let value = last.wrapping_add(unzigzag32(data.vbyte()?));
		*last = value;
		Ok(value)
	};

	for (triangle, &code) in codes.iter().enumerate() {
		let (a, b, c);

		if code < 0xf0 {
			let fe = usize::from(code >> 4);
			[a, b] = fifos.edge(fe);
			let fec = code & 15;

			if fec < fec_max {
				c = if fec == 0 {
					next
				} else {
					fifos.vertex(usize::from(fec) + 1)
				};

				let fresh = fec == 0;
				next += u32::from(fresh);
				fifos.push_vertex(c, fresh);
			} else {
				c = if fec == 15 {
					decode_index(&mut data, &mut last)?
				} else {
					// 13 and 14 step the last free index by -1 and +1.
					last = last.wrapping_add(if fec == 13 { u32::MAX } else { 1 });
					last
				};

				fifos.push_vertex(c, true);
			}

			fifos.push_edge(c, b);
			fifos.push_edge(a, c);
		} else if code < 0xfe {
			let codeaux = aux[usize::from(code & 15)];
			let feb = usize::from(codeaux >> 4);
			let fec = usize::from(codeaux & 15);
			a = next;
			next += 1;
			b = if feb == 0 { next } else { fifos.vertex(feb) };
			next += u32::from(feb == 0);
			c = if fec == 0 { next } else { fifos.vertex(fec) };
			next += u32::from(fec == 0);
			fifos.push_vertex(a, true);
			fifos.push_vertex(b, feb == 0);
			fifos.push_vertex(c, fec == 0);
			fifos.push_edge(b, a);
			fifos.push_edge(c, b);
			fifos.push_edge(a, c);
		} else {
			let codeaux = data.byte()?;
			let fea = if code == 0xfe { 0 } else { 15 };
			let feb = usize::from(codeaux >> 4);
			let fec = usize::from(codeaux & 15);

			if codeaux == 0 {
				next = 0;
			}
			let mut va = if fea == 0 {
				next += 1;
				next - 1
			} else {
				0
			};
			let mut vb = if feb == 0 {
				next += 1;
				next - 1
			} else {
				fifos.vertex(feb)
			};
			let mut vc = if fec == 0 {
				next += 1;
				next - 1
			} else {
				fifos.vertex(fec)
			};

			if fea == 15 {
				va = decode_index(&mut data, &mut last)?;
			}

			if feb == 15 {
				vb = decode_index(&mut data, &mut last)?;
			}

			if fec == 15 {
				vc = decode_index(&mut data, &mut last)?;
			}

			(a, b, c) = (va, vb, vc);
			fifos.push_vertex(a, true);
			fifos.push_vertex(b, feb == 0 || feb == 15);
			fifos.push_vertex(c, fec == 0 || fec == 15);
			fifos.push_edge(b, a);
			fifos.push_edge(c, b);
			fifos.push_edge(a, c);
		}

		write_index(output, triangle * 3, size, a);
		write_index(output, triangle * 3 + 1, size, b);
		write_index(output, triangle * 3 + 2, size, c);
	}

	if data.remaining() != 0 {
		return Err(MeshoptError("the index data has bytes left over"));
	}

	Ok(())
}

fn decode_index_sequence(
	output: &mut [u8],
	count: usize,
	size: usize,
	source: &[u8],
) -> Result<()> {
	if size != 2 && size != 4 {
		return Err(MeshoptError("indices must be 2 or 4 bytes"));
	}

	if source.len() < 1 + 4 {
		return Err(MeshoptError("the index data is too short"));
	}

	if source[0] & 0xf0 != SEQUENCE_HEADER || source[0] & 0x0f > 1 {
		return Err(MeshoptError("the index sequence has the wrong header"));
	}

	let mut data = Cursor {
		bytes: &source[..source.len() - 4],
		position: 1,
	};
	let mut last = [0_u32; 2];

	for index in 0..count {
		let value = data.vbyte()?;
		let current = (value & 1) as usize;
		let decoded = last[current].wrapping_add(unzigzag32(value >> 1));
		last[current] = decoded;
		write_index(output, index, size, decoded);
	}

	if data.remaining() != 0 {
		return Err(MeshoptError("the index data has bytes left over"));
	}

	Ok(())
}

fn round(value: f32) -> i32 {
	(value + if value >= 0.0 { 0.5 } else { -0.5 }) as i32
}

fn apply_filter(data: &mut [u8], count: usize, stride: usize, filter: Filter) -> Result<()> {
	match filter {
		Filter::None => {}

		Filter::Octahedral if stride == 4 => {
			for element in data.as_chunks_mut::<4>().0.iter_mut().take(count) {
				let values = [element[0] as i8, element[1] as i8, element[2] as i8];
				let [x, y, z] = octahedral(values.map(f32::from), 127.0);
				element[0] = x as i8 as u8;
				element[1] = y as i8 as u8;
				element[2] = z as i8 as u8;
			}
		}

		Filter::Octahedral if stride == 8 => {
			for element in data.as_chunks_mut::<8>().0.iter_mut().take(count) {
				let values = [0, 1, 2].map(|index| {
					f32::from(i16::from_le_bytes([
						element[index * 2],
						element[index * 2 + 1],
					]))
				});
				let decoded = octahedral(values, 32767.0);

				for (index, value) in decoded.into_iter().enumerate() {
					element[index * 2..index * 2 + 2]
						.copy_from_slice(&(value as i16).to_le_bytes());
				}
			}
		}

		Filter::Octahedral => {
			return Err(MeshoptError(
				"the octahedral filter needs a stride of 4 or 8",
			));
		}

		Filter::Quaternion if stride == 8 => {
			for element in data.as_chunks_mut::<8>().0.iter_mut().take(count) {
				let read =
					|index: usize| i16::from_le_bytes([element[index * 2], element[index * 2 + 1]]);
				let (x, y, z, w) = (read(0), read(1), read(2), read(3));
				let scale = std::f32::consts::FRAC_1_SQRT_2 / f32::from(w | 3);
				let [x, y, z] = [x, y, z].map(|value| f32::from(value) * scale);
				let ww = 1.0 - x * x - y * y - z * z;
				let w_value = ww.max(0.0).sqrt();
				let [xf, yf, zf, wf] =
					[x, y, z, w_value].map(|value| round(value * 32767.0) as i16);
				let qc = (w & 3) as usize;
				let mut out = [0_i16; 4];
				out[(qc + 1) & 3] = xf;
				out[(qc + 2) & 3] = yf;
				out[(qc + 3) & 3] = zf;
				out[qc] = wf;

				for (index, value) in out.into_iter().enumerate() {
					element[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
				}
			}
		}

		Filter::Quaternion => {
			return Err(MeshoptError("the quaternion filter needs a stride of 8"));
		}

		Filter::Exponential if stride % 4 == 0 => {
			for word in data
				.as_chunks_mut::<4>()
				.0
				.iter_mut()
				.take(count * stride / 4)
			{
				let value = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
				let mantissa = ((value << 8) as i32) >> 8;
				let exponent = (value as i32) >> 24;
				let decoded = mantissa as f32 * 2.0_f32.powi(exponent);
				word.copy_from_slice(&decoded.to_bits().to_le_bytes());
			}
		}

		Filter::Exponential => {
			return Err(MeshoptError(
				"the exponential filter needs a stride that divides by 4",
			));
		}
	}

	Ok(())
}

/// Rebuilds a unit vector from two octahedral coordinates; the third
/// component holds the scale `one` stands for.
fn octahedral([x, y, one]: [f32; 3], max: f32) -> [i32; 3] {
	let (mut x, mut y) = (x, y);
	let z = one - x.abs() - y.abs();
	let t = z.min(0.0);
	x += if x >= 0.0 { t } else { -t };
	y += if y >= 0.0 { t } else { -t };
	let length = (x * x + y * y + z * z).sqrt();
	let scale = if length > 0.0 { max / length } else { 0.0 };
	[round(x * scale), round(y * scale), round(z * scale)]
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn zigzag_round_trips_small_values() {
		assert_eq!(unzigzag8(0), 0);
		assert_eq!(unzigzag8(1), 0xff);
		assert_eq!(unzigzag8(2), 1);
		assert_eq!(unzigzag32(3), u32::MAX - 1);
		assert_eq!(unzigzag32(4), 2);
	}

	#[test]
	fn vbyte_reads_one_and_several_bytes() {
		let mut cursor = Cursor {
			bytes: &[0x05, 0x80 | 0x05, 0x01],
			position: 0,
		};
		assert_eq!(cursor.vbyte(), Ok(5));
		assert_eq!(cursor.vbyte(), Ok(5 | (1 << 7)));
		assert!(cursor.vbyte().is_err());
	}

	#[test]
	fn the_exponential_filter_scales_its_mantissa() {
		let encoded = ((-2_i32 as u32) << 24) | 6;
		let mut data = encoded.to_le_bytes().to_vec();
		apply_filter(&mut data, 1, 4, Filter::Exponential).unwrap();
		assert_eq!(f32::from_le_bytes(data[..4].try_into().unwrap()), 1.5);
	}

	#[test]
	fn the_octahedral_filter_rebuilds_unit_vectors() {
		// (0, 0, 127) stands for +z.
		let mut data = vec![0, 0, 127, 0];
		apply_filter(&mut data, 1, 4, Filter::Octahedral).unwrap();
		assert_eq!(data, [0, 0, 127, 0]);
	}

	#[test]
	fn truncated_and_mislabelled_data_are_errors() {
		assert!(decode(&[], 3, 4, Mode::Attributes, Filter::None).is_err());
		assert!(decode(&[0xb0; 64], 3, 4, Mode::Attributes, Filter::None).is_err());
		assert!(decode(&[0xe1; 4], 3, 2, Mode::Triangles, Filter::None).is_err());
		assert!(decode(&[0xd1; 8], 3, 2, Mode::Indices, Filter::None).is_err());
		assert!(decode(&[0xa0; 64], 3, 6, Mode::Attributes, Filter::None).is_err());
		assert!(decode(&[0xe1; 64], 3, 2, Mode::Triangles, Filter::Octahedral).is_err());
		assert!(decode(&[0xe1; 64], 4, 2, Mode::Triangles, Filter::None).is_err());
	}

	#[test]
	fn random_bytes_never_panic() {
		let mut state = 0x2545_f491_u32;
		let mut next = move || {
			state ^= state << 13;
			state ^= state >> 17;
			state ^= state << 5;
			state
		};

		for round in 0..2000 {
			let length = (next() % 200) as usize;
			let mut bytes: Vec<u8> = (0..length).map(|_| next() as u8).collect();

			if let Some(first) = bytes.first_mut() {
				*first = [0xa0, 0xe0, 0xe1, 0xd1][round % 4];
			}

			let count = (next() % 64) as usize;
			let _ = decode(&bytes, count, 4, Mode::Attributes, Filter::Exponential);

			let _ = decode(&bytes, count * 3, 2, Mode::Triangles, Filter::None);
			let _ = decode(&bytes, count, 4, Mode::Indices, Filter::None);
			let _ = decode(&bytes, count, 8, Mode::Attributes, Filter::Quaternion);
		}
	}
}
