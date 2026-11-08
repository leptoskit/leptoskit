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

	/// A LEB128 integer of at most five bytes: seven bits a byte, low bits
	/// first, the high bit set on every byte but the last.
	fn vbyte(&mut self) -> Result<u32> {
		let mut result = 0;

		for shift in (0..u32::BITS).step_by(VBYTE_BITS) {
			let group = self.byte()?;
			result |= u32::from(group & !VBYTE_MORE) << shift;

			if group & VBYTE_MORE == 0 {
				break;
			}
		}

		Ok(result)
	}
}

const VBYTE_MORE: u8 = 0x80;
const VBYTE_BITS: usize = 7;

const VERTEX_HEADER: u8 = 0xa0;
const MAX_VERTEX_STRIDE: usize = 256;
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
		1 => decode_packed(cursor, output, 2)?,
		2 => decode_packed(cursor, output, 4)?,
		_ => {
			for value in output.iter_mut() {
				*value = cursor.byte()?;
			}
		}
	}

	Ok(())
}

/// Sixteen `bits`-bit values packed high bits first; the all-ones value
/// escapes to a full byte after the packed ones.
fn decode_packed(
	cursor: &mut Cursor<'_>,
	output: &mut [u8; BYTE_GROUP],
	bits: usize,
) -> Result<()> {
	let packed = BYTE_GROUP * bits / u8::BITS as usize;
	let start = cursor.position;

	if cursor.remaining() < packed {
		return Err(MeshoptError("the data ends early"));
	}

	cursor.position += packed;
	let sentinel = (1_u8 << bits) - 1;

	for (index, value) in output.iter_mut().enumerate() {
		let bit = index * bits;
		let byte = cursor.bytes[start + bit / u8::BITS as usize];
		let encoded = (byte >> (u8::BITS as usize - bits - bit % u8::BITS as usize)) & sentinel;
		*value = if encoded == sentinel {
			cursor.byte()?
		} else {
			encoded
		};
	}

	Ok(())
}

/// Decodes vertex codec version 0. Vertices come in blocks; within a block
/// each byte of the stride is a column of deltas from the previous vertex,
/// zigzagged and packed in groups of sixteen at 0, 2, 4, or 8 bits as a
/// two-bit header per group says. The first vertex's bytes are deltas from
/// the tail at the end of the data.
fn decode_vertices(output: &mut [u8], count: usize, stride: usize, source: &[u8]) -> Result<()> {
	if stride == 0 || stride > MAX_VERTEX_STRIDE || stride % 4 != 0 {
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

	let mut last = [0_u8; MAX_VERTEX_STRIDE];
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

/// Entries in each of the decoder's FIFOs.
const FIFO_SIZE: usize = 16;
const FIFO_MASK: usize = FIFO_SIZE - 1;
/// Bytes of auxiliary codes at the end of the index data.
const AUX_BYTES: usize = 16;
/// A vertex code that reads an index explicitly from the data.
const EXPLICIT: u8 = 15;
/// Vertex codes that step the last explicit index back or forward by one.
const STEP_BACK: u8 = 13;
const STEP_FORWARD: u8 = 14;
/// Triangle codes: below this one reuses an edge, below `FREE` takes its
/// vertices' codes from the auxiliary bytes, and the rest read them from
/// the data.
const AUX_CODES: u8 = 0xf0;
const FREE: u8 = 0xfe;
const LOW_NIBBLE: u8 = 0x0f;

/// The decoder's two FIFOs of recent edges and vertices.
struct Fifos {
	edges: [[u32; 2]; FIFO_SIZE],
	edge_offset: usize,
	vertices: [u32; FIFO_SIZE],
	vertex_offset: usize,
}

impl Fifos {
	fn push_edge(&mut self, from: u32, to: u32) {
		self.edges[self.edge_offset] = [from, to];
		self.edge_offset = (self.edge_offset + 1) & FIFO_MASK;
	}

	fn push_vertex(&mut self, vertex: u32, advance: bool) {
		self.vertices[self.vertex_offset] = vertex;
		self.vertex_offset = (self.vertex_offset + usize::from(advance)) & FIFO_MASK;
	}

	fn edge(&self, back: usize) -> [u32; 2] {
		self.edges[(self.edge_offset.wrapping_sub(1 + back)) & FIFO_MASK]
	}

	fn vertex(&self, back: usize) -> u32 {
		self.vertices[(self.vertex_offset.wrapping_sub(back)) & FIFO_MASK]
	}

	/// Remembers a whole triangle's edges.
	fn push_edges(&mut self, [first, second, third]: [u32; 3]) {
		self.push_edge(second, first);
		self.push_edge(third, second);
		self.push_edge(first, third);
	}
}

/// The index codec's state between triangles: the FIFOs, the next vertex
/// never seen before, and the last index encoded explicitly.
struct TriangleDecoder<'a> {
	data: Cursor<'a>,
	aux: &'a [u8],
	fifos: Fifos,
	next: u32,
	last: u32,
	/// Version 1 spends vertex codes 13 and 14 on steps; version 0 reads
	/// every code below 15 from the FIFO.
	fifo_codes: u8,
}

impl TriangleDecoder<'_> {
	fn decode(&mut self, code: u8) -> Result<[u32; 3]> {
		match code {
			0..AUX_CODES => self.edge_triangle(code),
			AUX_CODES..FREE => Ok(self.aux_triangle(code)),
			_ => self.free_triangle(code),
		}
	}

	/// A triangle on a recent edge; its third vertex is new, recent, or
	/// encoded explicitly.
	fn edge_triangle(&mut self, code: u8) -> Result<[u32; 3]> {
		let [first, second] = self.fifos.edge(usize::from(code >> 4));
		let third_code = code & LOW_NIBBLE;
		let fresh = third_code == 0;
		let third = if third_code >= self.fifo_codes {
			self.explicit(third_code)?
		} else if fresh {
			self.next
		} else {
			self.fifos.vertex(usize::from(third_code) + 1)
		};

		self.next += u32::from(fresh);
		self.fifos
			.push_vertex(third, fresh || third_code >= self.fifo_codes);
		self.fifos.push_edge(third, second);
		self.fifos.push_edge(first, third);
		Ok([first, second, third])
	}

	/// A triangle starting at a new vertex, whose other two are new or
	/// recent as an auxiliary byte says.
	fn aux_triangle(&mut self, code: u8) -> [u32; 3] {
		let codes = self.aux[usize::from(code & LOW_NIBBLE)];
		let (second_code, third_code) = (codes >> 4, codes & LOW_NIBBLE);
		let first = self.take_next();
		let second = self.recent_or_next(second_code);
		let third = self.recent_or_next(third_code);
		self.fifos.push_vertex(first, true);
		self.fifos.push_vertex(second, second_code == 0);
		self.fifos.push_vertex(third, third_code == 0);

		self.fifos.push_edges([first, second, third]);
		[first, second, third]
	}

	/// A triangle whose vertices' codes follow in the data: each new,
	/// recent, or explicit. A zero byte restarts the new vertices at zero.
	fn free_triangle(&mut self, code: u8) -> Result<[u32; 3]> {
		let codes = self.data.byte()?;
		let (second_code, third_code) = (codes >> 4, codes & LOW_NIBBLE);

		if codes == 0 {
			self.next = 0;
		}

		let first_explicit = code != FREE;
		let first = if first_explicit { 0 } else { self.take_next() };
		let second = self.recent_or_next(second_code);
		let third = self.recent_or_next(third_code);

		// Explicit indices follow in the order of the vertices.
		let first = if first_explicit {
			self.explicit(EXPLICIT)?
		} else {
			first
		};
		let second = if second_code == EXPLICIT {
			self.explicit(EXPLICIT)?
		} else {
			second
		};

		let third = if third_code == EXPLICIT {
			self.explicit(EXPLICIT)?
		} else {
			third
		};

		self.fifos.push_vertex(first, true);
		self.fifos
			.push_vertex(second, second_code == 0 || second_code == EXPLICIT);
		self.fifos
			.push_vertex(third, third_code == 0 || third_code == EXPLICIT);
		self.fifos.push_edges([first, second, third]);
		Ok([first, second, third])
	}

	fn take_next(&mut self) -> u32 {
		self.next += 1;
		self.next - 1
	}

	/// Code 0 is the next new vertex; others count back through the FIFO.
	fn recent_or_next(&mut self, code: u8) -> u32 {
		if code == 0 {
			self.take_next()
		} else {
			self.fifos.vertex(usize::from(code))
		}
	}

	/// An index relative to the last explicit one: read from the data, or
	/// one step from it.
	fn explicit(&mut self, code: u8) -> Result<u32> {
		self.last = match code {
			STEP_BACK => self.last.wrapping_sub(1),
			STEP_FORWARD => self.last.wrapping_add(1),
			_ => self.last.wrapping_add(unzigzag32(self.data.vbyte()?)),
		};

		Ok(self.last)
	}
}

/// Decodes index codec versions 0 and 1: a code byte per triangle up
/// front, then the variable-length data they read from, then sixteen
/// auxiliary codes. Most triangles share an edge with a recent one, which
/// is what makes the codes small.
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

	if source.len() < 1 + triangles + AUX_BYTES {
		return Err(MeshoptError("the index data is too short"));
	}

	if source[0] & 0xf0 != INDEX_HEADER {
		return Err(MeshoptError("the index data has the wrong header"));
	}

	let version = source[0] & LOW_NIBBLE;

	if version > 1 {
		return Err(MeshoptError(
			"only versions 0 and 1 of the index codec are supported",
		));
	}

	let codes = &source[1..=triangles];
	let mut decoder = TriangleDecoder {
		data: Cursor {
			bytes: &source[..source.len() - AUX_BYTES],
			position: 1 + triangles,
		},
		aux: &source[source.len() - AUX_BYTES..],
		fifos: Fifos {
			edges: [[u32::MAX; 2]; FIFO_SIZE],
			edge_offset: 0,
			vertices: [u32::MAX; FIFO_SIZE],
			vertex_offset: 0,
		},
		next: 0,
		last: 0,

		fifo_codes: if version >= 1 { STEP_BACK } else { EXPLICIT },
	};

	for (triangle, &code) in codes.iter().enumerate() {
		let indices = decoder.decode(code)?;

		for (corner, index) in indices.into_iter().enumerate() {
			write_index(output, triangle * 3 + corner, size, index);
		}
	}

	if decoder.data.remaining() != 0 {
		return Err(MeshoptError("the index data has bytes left over"));
	}

	Ok(())
}

/// Decodes an index sequence: each index is a delta from one of the last
/// two, the low bit saying which.
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

/// Undoes an attribute filter in place. Each filter needs a stride that
/// fits its encoding, which a malformed file may not have.
fn apply_filter(data: &mut [u8], count: usize, stride: usize, filter: Filter) -> Result<()> {
	match (filter, stride) {
		(Filter::None, _) => {}
		(Filter::Octahedral, 4) => {
			data.as_chunks_mut::<4>()
				.0
				.iter_mut()
				.take(count)
				.for_each(octahedral_bytes);
		}
		(Filter::Octahedral, 8) => {
			data.as_chunks_mut::<8>()
				.0
				.iter_mut()
				.take(count)
				.for_each(octahedral_shorts);
		}
		(Filter::Octahedral, _) => {
			return Err(MeshoptError(
				"the octahedral filter needs a stride of 4 or 8",
			));
		}
		(Filter::Quaternion, 8) => {
			data.as_chunks_mut::<8>()
				.0
				.iter_mut()
				.take(count)
				.for_each(quaternion);
		}
		(Filter::Quaternion, _) => {
			return Err(MeshoptError("the quaternion filter needs a stride of 8"));
		}

		(Filter::Exponential, _) if stride % 4 == 0 => {
			data.as_chunks_mut::<4>()
				.0
				.iter_mut()
				.take(count * stride / 4)
				.for_each(exponential);
		}

		(Filter::Exponential, _) => {
			return Err(MeshoptError(
				"the exponential filter needs a stride that divides by 4",
			));
		}
	}

	Ok(())
}

fn read_i16(bytes: &[u8], index: usize) -> i16 {
	i16::from_le_bytes([bytes[index * 2], bytes[index * 2 + 1]])
}

fn write_i16(bytes: &mut [u8], index: usize, value: i16) {
	bytes[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
}

/// An octahedral unit vector in three signed bytes, the fourth untouched.
fn octahedral_bytes(element: &mut [u8; 4]) {
	let values = [element[0] as i8, element[1] as i8, element[2] as i8];
	let decoded = octahedral(values.map(f32::from), f32::from(i8::MAX));

	for (byte, value) in element.iter_mut().zip(decoded) {
		*byte = value as i8 as u8;
	}
}

/// An octahedral unit vector in three signed shorts, the fourth untouched.
fn octahedral_shorts(element: &mut [u8; 8]) {
	let values = [0, 1, 2].map(|index| f32::from(read_i16(element, index)));
	let decoded = octahedral(values, f32::from(i16::MAX));

	for (index, value) in decoded.into_iter().enumerate() {
		write_i16(element, index, value as i16);
	}
}

/// A unit quaternion stored as its three smallest components; the last
/// short's low two bits say which component was dropped, and its other
/// bits the scale.
fn quaternion(element: &mut [u8; 8]) {
	let [first, second, third, last] = [0, 1, 2, 3].map(|index| read_i16(element, index));
	let scale = std::f32::consts::FRAC_1_SQRT_2 / f32::from(last | 3);
	let kept = [first, second, third].map(|value| f32::from(value) * scale);
	let dropped = (1.0 - kept.iter().map(|value| value * value).sum::<f32>())
		.max(0.0)
		.sqrt();
	let max = f32::from(i16::MAX);
	let dropped_slot = (last & 3) as usize;
	let mut quaternion = [0_i16; 4];

	for (offset, value) in kept.into_iter().enumerate() {
		quaternion[(dropped_slot + offset + 1) & 3] = round(value * max) as i16;
	}

	quaternion[dropped_slot] = round(dropped * max) as i16;

	for (index, value) in quaternion.into_iter().enumerate() {
		write_i16(element, index, value);
	}
}

/// A float as a 24-bit signed mantissa and an 8-bit signed exponent.
fn exponential(word: &mut [u8; 4]) {
	let value = u32::from_le_bytes(*word);
	let mantissa = ((value << 8) as i32) >> 8;
	let exponent = (value as i32) >> 24;
	let decoded = mantissa as f32 * 2.0_f32.powi(exponent);
	*word = decoded.to_bits().to_le_bytes();
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
