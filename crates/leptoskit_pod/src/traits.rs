//! Encode / decode traits for Pod types.

use crate::Error;
use crate::Reader;
use crate::Result;
use crate::Writer;

/// Writes `Self` as Pod bytes.
pub trait PodEncode {
	/// Appends this value to `w`.
	fn encode(&self, w: &mut Writer) -> Result<()>;

	/// Encodes into a new buffer.
	fn to_pod_bytes(&self) -> Result<Vec<u8>> {
		let mut w = Writer::new();
		self.encode(&mut w)?;
		Ok(w.finish())
	}
}

/// Reads `Self` from Pod bytes.
pub trait PodDecode<'a>: Sized {
	/// Reads one value from `r`.
	fn decode(r: &mut Reader<'a>) -> Result<Self>;

	/// Decodes the whole buffer; rejects trailing bytes.
	fn from_pod_bytes(bytes: &'a [u8]) -> Result<Self> {
		let mut r = Reader::new(bytes);
		let value = Self::decode(&mut r)?;
		r.finish()?;
		Ok(value)
	}
}

macro_rules! impl_fixed {
	($($ty:ty => $write:ident, $read:ident),+ $(,)?) => {
		$(
			impl PodEncode for $ty {
				fn encode(&self, w: &mut Writer) -> Result<()> {
					w.$write(*self);
					Ok(())
				}
			}
			impl<'a> PodDecode<'a> for $ty {
				fn decode(r: &mut Reader<'a>) -> Result<Self> {
					r.$read()
				}
			}
		)+
	};
}

impl_fixed! {
	u8 => u8, u8,
	u16 => u16, u16,
	u32 => u32, u32,
	u64 => u64, u64,
	i8 => i8, i8,
	i16 => i16, i16,
	i32 => i32, i32,
	i64 => i64, i64,
	bool => bool, bool,
}

impl PodEncode for usize {
	fn encode(&self, w: &mut Writer) -> Result<()> {
		u64::try_from(*self).map_err(|_| Error::TooLarge)?.encode(w)
	}
}

impl<'a> PodDecode<'a> for usize {
	fn decode(r: &mut Reader<'a>) -> Result<Self> {
		let value = u64::decode(r)?;
		usize::try_from(value).map_err(|_| Error::TooLarge)
	}
}

impl PodEncode for String {
	fn encode(&self, w: &mut Writer) -> Result<()> {
		w.string(self)
	}
}

impl<'a> PodDecode<'a> for String {
	fn decode(r: &mut Reader<'a>) -> Result<Self> {
		Ok(r.string()?.to_owned())
	}
}

impl PodEncode for str {
	fn encode(&self, w: &mut Writer) -> Result<()> {
		w.string(self)
	}
}

impl PodEncode for [u8] {
	fn encode(&self, w: &mut Writer) -> Result<()> {
		w.bytes(self)
	}
}

/// Opaque length-prefixed bytes (not `Vec<u8>` element-wise).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ByteBuf(pub Vec<u8>);

impl PodEncode for ByteBuf {
	fn encode(&self, w: &mut Writer) -> Result<()> {
		w.bytes(&self.0)
	}
}

impl<'a> PodDecode<'a> for ByteBuf {
	fn decode(r: &mut Reader<'a>) -> Result<Self> {
		Ok(Self(r.bytes()?.to_vec()))
	}
}

impl<T: PodEncode> PodEncode for Option<T> {
	fn encode(&self, w: &mut Writer) -> Result<()> {
		match self {
			None => {
				w.option_tag(false);
				Ok(())
			}
			Some(value) => {
				w.option_tag(true);
				value.encode(w)
			}
		}
	}
}

impl<'a, T: PodDecode<'a>> PodDecode<'a> for Option<T> {
	fn decode(r: &mut Reader<'a>) -> Result<Self> {
		if r.option_tag()? {
			Ok(Some(T::decode(r)?))
		} else {
			Ok(None)
		}
	}
}

impl<T: PodEncode, E: PodEncode> PodEncode for std::result::Result<T, E> {
	fn encode(&self, w: &mut Writer) -> Result<()> {
		match self {
			Ok(value) => {
				w.result_tag(true);
				value.encode(w)
			}
			Err(error) => {
				w.result_tag(false);
				error.encode(w)
			}
		}
	}
}

impl<'a, T: PodDecode<'a>, E: PodDecode<'a>> PodDecode<'a> for std::result::Result<T, E> {
	fn decode(r: &mut Reader<'a>) -> Result<Self> {
		if r.result_tag()? {
			Ok(Ok(T::decode(r)?))
		} else {
			Ok(Err(E::decode(r)?))
		}
	}
}

impl<T: PodEncode> PodEncode for Vec<T> {
	fn encode(&self, w: &mut Writer) -> Result<()> {
		w.vec_len(self.len())?;
		for item in self {
			item.encode(w)?;
		}
		Ok(())
	}
}

impl<'a, T: PodDecode<'a>> PodDecode<'a> for Vec<T> {
	fn decode(r: &mut Reader<'a>) -> Result<Self> {
		let len = r.vec_len()?;
		let mut out = Vec::new();
		for _ in 0..len {
			out.push(T::decode(r)?);
		}
		Ok(out)
	}
}

impl PodEncode for () {
	fn encode(&self, _w: &mut Writer) -> Result<()> {
		Ok(())
	}
}

impl<'a> PodDecode<'a> for () {
	fn decode(_r: &mut Reader<'a>) -> Result<Self> {
		Ok(())
	}
}

/// Dense named-field struct: `[version:u16][fields…]`.
pub fn encode_struct(
	w: &mut Writer,
	version: u16,
	encode_fields: impl FnOnce(&mut Writer) -> Result<()>,
) -> Result<()> {
	w.struct_header(version);
	encode_fields(w)
}

/// Encodes a dense struct at layout version `0`.
pub fn encode_struct_v0(
	w: &mut Writer,
	encode_fields: impl FnOnce(&mut Writer) -> Result<()>,
) -> Result<()> {
	encode_struct(w, 0, encode_fields)
}

/// Decodes a dense struct; requires matching version.
///
/// For inbound RPC / jobs that may arrive at an older version, call
/// [`crate::climb`] first, then decode at the current version.
pub fn decode_struct_version<'a, T>(
	r: &mut Reader<'a>,
	expected_version: u16,
	decode_fields: impl FnOnce(&mut Reader<'a>) -> Result<T>,
) -> Result<T> {
	let version = r.struct_header()?;
	if version != expected_version {
		return Err(Error::UnknownVersion(version));
	}
	decode_fields(r)
}
