//! Typed intent extras: the values an Android `Bundle` can carry without
//! parcelables.

use leptoskit_bridge::Error;

use crate::MAX_ARRAY_LEN;
use crate::MAX_DEPTH;
use crate::MAX_EXTRAS;
use crate::MAX_NAME_BYTES;
use crate::MAX_TOTAL_EXTRAS;
use crate::MAX_VALUE_BYTES;
use crate::validate;
use crate::wire::Reader;
use crate::wire::Writer;

/// One extra value. The variants cover `android_intent_plus`'s `arguments`
/// (scalars, strings, nested bundles, byte arrays and integer or string
/// lists) and its `arrayArguments` (typed arrays).
///
/// There is deliberately no variant for a URI, an intent, a pending intent
/// or any other parcelable: those are how one app hands another a
/// capability, and a guest that can be replaced over the air should not be
/// able to mint one.
#[derive(Debug, Clone, PartialEq)]
pub enum Extra {
	/// `Bundle.putBoolean`.
	Bool(bool),
	/// `Bundle.putInt`.
	Int(i32),
	/// `Bundle.putLong`.
	Long(i64),
	/// `Bundle.putDouble`. Must be finite.
	Double(f64),
	/// `Bundle.putString`. May be empty.
	String(String),
	/// `Bundle.putBundle`, nested at most [`MAX_DEPTH`] bundles deep.
	Bundle(Extras),
	/// `Bundle.putByteArray`.
	Bytes(Vec<u8>),
	/// `Bundle.putBooleanArray`.
	BoolArray(Vec<bool>),
	/// `Bundle.putIntArray`.
	IntArray(Vec<i32>),
	/// `Bundle.putLongArray`.
	LongArray(Vec<i64>),
	/// `Bundle.putDoubleArray`. Every element must be finite.
	DoubleArray(Vec<f64>),
	/// `Bundle.putStringArray`.
	StringArray(Vec<String>),
	/// `Bundle.putIntegerArrayList`, for receivers that read an
	/// `ArrayList<Integer>`.
	IntList(Vec<i32>),
	/// `Bundle.putStringArrayList`, for receivers that read an
	/// `ArrayList<String>`.
	StringList(Vec<String>),
}

impl Extra {
	/// The stable wire tag for this variant.
	#[must_use]
	pub const fn tag(&self) -> u32 {
		match self {
			Self::Bool(_) => 1,
			Self::Int(_) => 2,
			Self::Long(_) => 3,
			Self::Double(_) => 4,
			Self::String(_) => 5,
			Self::Bundle(_) => 6,
			Self::Bytes(_) => 7,
			Self::BoolArray(_) => 8,
			Self::IntArray(_) => 9,
			Self::LongArray(_) => 10,
			Self::DoubleArray(_) => 11,
			Self::StringArray(_) => 12,
			Self::IntList(_) => 13,
			Self::StringList(_) => 14,
		}
	}

	fn encode(&self, output: &mut Writer) -> Result<(), Error> {
		output.u32(self.tag());

		match self {
			Self::Bool(value) => output.u32(u32::from(*value)),
			Self::Int(value) => output.i32(*value),
			Self::Long(value) => output.i64(*value),
			Self::Double(value) => output.f64(*value),
			Self::String(value) => output.string(value)?,
			Self::Bundle(value) => value.encode(output)?,
			Self::Bytes(value) => output.bytes(value)?,
			Self::BoolArray(values) => list(output, values, write_bool)?,
			Self::IntArray(values) | Self::IntList(values) => list(output, values, Writer::i32)?,
			Self::LongArray(values) => list(output, values, Writer::i64)?,
			Self::DoubleArray(values) => list(output, values, Writer::f64)?,
			Self::StringArray(values) | Self::StringList(values) => {
				output.len(values.len())?;

				for value in values {
					output.string(value)?;
				}
			}
		}

		Ok(())
	}

	fn decode(input: &mut Reader<'_>, depth: usize, total: &mut usize) -> Result<Self, Error> {
		let value = match input.u32()? {
			1 => Self::Bool(input.bool()?),
			2 => Self::Int(input.i32()?),
			3 => Self::Long(input.i64()?),
			4 => Self::Double(input.f64()?),
			5 => Self::String(input.string(MAX_VALUE_BYTES)?.to_owned()),
			6 => {
				if depth >= MAX_DEPTH {
					return Err(Error::TooLarge);
				}

				Self::Bundle(Extras::decode_nested(input, depth + 1, total)?)
			}
			7 => Self::Bytes(input.bytes(MAX_VALUE_BYTES)?.to_vec()),
			8 => Self::BoolArray(read_list(input, Reader::bool)?),
			9 => Self::IntArray(read_list(input, Reader::i32)?),
			10 => Self::LongArray(read_list(input, Reader::i64)?),
			11 => Self::DoubleArray(read_list(input, Reader::f64)?),
			12 => Self::StringArray(read_strings(input)?),
			13 => Self::IntList(read_list(input, Reader::i32)?),
			14 => Self::StringList(read_strings(input)?),
			_ => return Err(Error::InvalidArgument),
		};

		Ok(value)
	}
}

fn write_bool(output: &mut Writer, value: bool) {
	output.u32(u32::from(value));
}

fn list<T: Copy>(
	output: &mut Writer,
	values: &[T],
	mut write: impl FnMut(&mut Writer, T),
) -> Result<(), Error> {
	output.len(values.len())?;

	for value in values {
		write(output, *value);
	}

	Ok(())
}

fn read_list<'a, T>(
	input: &mut Reader<'a>,
	mut read: impl FnMut(&mut Reader<'a>) -> Result<T, Error>,
) -> Result<Vec<T>, Error> {
	let len = input.count(MAX_ARRAY_LEN)?;
	let mut values = Vec::with_capacity(len);

	for _ in 0..len {
		values.push(read(input)?);
	}

	Ok(values)
}

fn read_strings(input: &mut Reader<'_>) -> Result<Vec<String>, Error> {
	read_list(input, |input| Ok(input.string(MAX_VALUE_BYTES)?.to_owned()))
}

macro_rules! extra_from {
	($($source:ty => $variant:ident),* $(,)?) => {
		$(
			impl From<$source> for Extra {
				fn from(value: $source) -> Self {
					Self::$variant(value.into())
				}
			}
		)*
	};
}

extra_from! {
	bool => Bool,
	i32 => Int,
	i64 => Long,
	f64 => Double,
	String => String,
	&str => String,
	Extras => Bundle,
	Vec<u8> => Bytes,
	&[u8] => Bytes,
	Vec<bool> => BoolArray,
	Vec<i32> => IntArray,
	Vec<i64> => LongArray,
	Vec<f64> => DoubleArray,
	Vec<String> => StringArray,
}

impl From<Vec<&str>> for Extra {
	fn from(values: Vec<&str>) -> Self {
		Self::StringArray(values.into_iter().map(str::to_owned).collect())
	}
}

/// An ordered bundle of extras with unique keys.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Extras(Vec<(String, Extra)>);

impl Extras {
	/// An empty bundle.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Sets `key`, replacing an earlier value with the same key as
	/// `Bundle.put*` does.
	#[must_use]
	pub fn put(mut self, key: impl Into<String>, value: impl Into<Extra>) -> Self {
		self.insert(key, value);
		self
	}

	/// Sets `key` in place, replacing an earlier value with the same key.
	pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Extra>) {
		let key = key.into();
		let value = value.into();

		if let Some(entry) = self.0.iter_mut().find(|(existing, _)| *existing == key) {
			entry.1 = value;
		} else {
			self.0.push((key, value));
		}
	}

	/// The value stored under `key`.
	#[must_use]
	pub fn get(&self, key: &str) -> Option<&Extra> {
		self.0
			.iter()
			.find_map(|(existing, value)| (existing == key).then_some(value))
	}

	/// Entries in insertion order.
	pub fn iter(&self) -> <&Self as IntoIterator>::IntoIter {
		self.into_iter()
	}

	/// Number of entries at this level, not counting nested bundles.
	#[must_use]
	pub fn len(&self) -> usize {
		self.0.len()
	}

	/// Whether the bundle has no entries.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}

	pub(crate) fn encode(&self, output: &mut Writer) -> Result<(), Error> {
		output.len(self.0.len())?;

		for (key, value) in &self.0 {
			output.string(key)?;
			value.encode(output)?;
		}

		Ok(())
	}

	/// Decodes the intent's top-level bundle.
	pub(crate) fn decode(input: &mut Reader<'_>) -> Result<Self, Error> {
		Self::decode_nested(input, 1, &mut 0)
	}

	fn decode_nested(
		input: &mut Reader<'_>,
		depth: usize,
		total: &mut usize,
	) -> Result<Self, Error> {
		let count = input.count(MAX_EXTRAS)?;
		let mut entries: Vec<(String, Extra)> = Vec::with_capacity(count);

		for _ in 0..count {
			*total += 1;

			if *total > MAX_TOTAL_EXTRAS {
				return Err(Error::TooLarge);
			}

			let key = input.string(MAX_NAME_BYTES)?;
			validate::key(key)?;

			if entries.iter().any(|(existing, _)| existing == key) {
				return Err(Error::InvalidArgument);
			}

			let value = Extra::decode(input, depth, total)?;
			entries.push((key.to_owned(), value));
		}

		Ok(Self(entries))
	}
}

/// Borrows one entry as a key and value pair.
type Entry<'a> = (&'a str, &'a Extra);

fn entry(pair: &(String, Extra)) -> Entry<'_> {
	(pair.0.as_str(), &pair.1)
}

impl<'a> IntoIterator for &'a Extras {
	type IntoIter = core::iter::Map<
		core::slice::Iter<'a, (String, Extra)>,
		fn(&'a (String, Extra)) -> Entry<'a>,
	>;
	type Item = Entry<'a>;

	fn into_iter(self) -> Self::IntoIter {
		self.0
			.iter()
			.map(entry as fn(&'a (String, Extra)) -> Entry<'a>)
	}
}
