//! The batch format clients send to `v1/batch`: binary, and small because
//! what's the same for every event is sent once.
//!
//! A batch is:
//!
//! ```text
//! batch    = "LKE" version:u8 sent_at:uvarint strings contexts entries
//! strings  = count:uvarint (len:uvarint utf8)*        the batch's string table
//! contexts = count:uvarint map*                       device and library properties
//! entries  = count:uvarint entry*
//! entry    = head:u8 uuid:16 [distinct:string-ref] [session:16] context:uvarint body
//! body     = by the head's kind, below
//! map      = count:uvarint (name value)*
//! name     = uvarint: even is a reserved name's code × 2, odd is a string-ref × 2 + 1
//! value    = tag:u8 payload
//! ```
//!
//! Integers are LEB128 (`uvarint`), and signed ones are zigzag-encoded
//! first. An entry's head holds its kind in the low three bits, and flags:
//! `0x08` it has a session, `0x10` its session is the previous entry's,
//! `0x20` its distinct id is the previous entry's. Those make the common
//! batch (one person, one session) cost a byte per event for both.
//!
//! An event's time isn't sent: its UUID v7 holds it, in milliseconds. The
//! batch's `sent_at` is the client's clock when it sent the batch, which
//! the service uses to correct the client's clock.
//!
//! The format is versioned by the byte after `LKE`; the service refuses
//! versions it doesn't know rather than guessing. [`RESERVED`] is
//! append-only: a code, once given, always means the same name.

use std::fmt;

/// What a batch starts with.
pub const MAGIC: &[u8; 3] = b"LKE";

/// The format's version.
pub const VERSION: u8 = 1;

/// The request's `Content-Type`.
pub const CONTENT_TYPE: &str = "application/vnd.leptoskit.events; v=1";

/// Names that have a code, so a batch never spells them out. Append only.
pub const RESERVED: &[&str] = &[
	"$pageview",
	"$pageleave",
	"$screen",
	"$autocapture",
	"$exception",
	"$feature_flag_called",
	"$web_vitals",
	"$identify",
	"$create_alias",
	"$set",
	"$set_once",
	"$unset",
	"$groupidentify",
	"$groups",
	"$current_url",
	"$pathname",
	"$host",
	"$referrer",
	"$referring_domain",
	"$title",
	"$lib",
	"$lib_version",
	"$os",
	"$os_version",
	"$browser",
	"$browser_version",
	"$device_type",
	"$screen_width",
	"$screen_height",
	"$viewport_width",
	"$viewport_height",
	"$locale",
	"$timezone",
	"$app_version",
	"$device_id",
	"$window_id",
	"$session_id",
	"$anon_distinct_id",
	"$lk_release",
	"$lk_route",
	"$lk_component",
	"$lk_test_id",
	"$lk_dropped",
	"utm_source",
	"utm_medium",
	"utm_campaign",
	"utm_term",
	"utm_content",
	"gclid",
	"fbclid",
	"msclkid",
];

/// A property's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
	Null,
	Bool(bool),
	Int(i64),
	Float(f64),
	Text(String),
	/// An instant, in milliseconds since the Unix epoch.
	Time(i64),
	List(Vec<Value>),
	Map(Vec<(String, Value)>),
}

/// Properties, in the order they were given.
pub type Properties = Vec<(String, Value)>;

macro_rules! value_from {
	($($from:ty => $variant:ident as $as:ty),+ $(,)?) => {$(
		impl From<$from> for Value {
			fn from(value: $from) -> Self {
				Self::$variant(<$as>::from(value))
			}
		}
	)+};
}

value_from! {
	bool => Bool as bool,
	i8 => Int as i64,
	i16 => Int as i64,
	i32 => Int as i64,
	i64 => Int as i64,
	u8 => Int as i64,
	u16 => Int as i64,
	u32 => Int as i64,
	f32 => Float as f64,
	f64 => Float as f64,
	String => Text as String,
	&str => Text as String,
	&String => Text as String,
}

impl<T: Into<Value>> From<Option<T>> for Value {
	fn from(value: Option<T>) -> Self {
		value.map_or(Self::Null, Into::into)
	}
}

impl<T: Into<Value>> From<Vec<T>> for Value {
	fn from(values: Vec<T>) -> Self {
		Self::List(values.into_iter().map(Into::into).collect())
	}
}

/// One batch.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Batch {
	/// The client's clock when it sent the batch, in milliseconds since the
	/// Unix epoch.
	pub sent_at: i64,
	/// What's the same for many entries: the library, the device, the app's
	/// version. Entries name one by its index.
	pub contexts: Vec<Properties>,
	pub entries: Vec<Entry>,
}

/// One thing a client recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
	/// A UUID v7, made when the entry was recorded: it's both the entry's
	/// identity, so a retried batch is stored once, and its time.
	pub uuid: [u8; 16],
	/// Who it's about, as the client knows them.
	pub distinct_id: String,
	/// The session's UUID v7.
	pub session_id: Option<[u8; 16]>,
	/// The index of the entry's context in [`Batch::contexts`].
	pub context: u32,
	pub body: Body,
}

/// What an entry records.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
	/// Something that happened.
	Event {
		name: String,
		properties: Properties,
	},
	/// The client learned who the person is: `distinct_id` is now the
	/// identified id, and `anon_distinct_id` the one it used before.
	Identify {
		anon_distinct_id: String,
		set: Properties,
		set_once: Properties,
	},
	/// `alias` is another id for the same person.
	Alias { alias: String },
	/// Changes to the person's properties.
	Set {
		set: Properties,
		set_once: Properties,
		unset: Vec<String>,
	},
	/// A group's properties: `group_key` of the type `group_type`.
	GroupIdentify {
		group_type: String,
		group_key: String,
		set: Properties,
	},
}

impl Body {
	fn kind(&self) -> u8 {
		match self {
			Self::Event { .. } => 0,
			Self::Identify { .. } => 1,
			Self::Alias { .. } => 2,
			Self::Set { .. } => 3,
			Self::GroupIdentify { .. } => 4,
		}
	}
}

const KIND_MASK: u8 = 0x07;
const HAS_SESSION: u8 = 0x08;
const SAME_SESSION: u8 = 0x10;
const SAME_DISTINCT: u8 = 0x20;

const TAG_NULL: u8 = 0;
const TAG_FALSE: u8 = 1;
const TAG_TRUE: u8 = 2;
const TAG_INT: u8 = 3;
const TAG_FLOAT: u8 = 4;
const TAG_TEXT: u8 = 5;
const TAG_TIME: u8 = 6;
const TAG_LIST: u8 = 7;
const TAG_MAP: u8 = 8;

/// Encodes `batch`.
pub fn encode(batch: &Batch) -> Vec<u8> {
	let mut strings = Strings::default();
	let mut body = Vec::with_capacity(64 * batch.entries.len() + 64);

	put_uvarint(&mut body, batch.contexts.len() as u64);
	for context in &batch.contexts {
		put_map(&mut body, &mut strings, context);
	}

	put_uvarint(&mut body, batch.entries.len() as u64);
	let mut previous: Option<&Entry> = None;
	for entry in &batch.entries {
		let same_distinct = previous.is_some_and(|before| before.distinct_id == entry.distinct_id);
		let same_session = entry.session_id.is_some()
			&& previous.is_some_and(|before| before.session_id == entry.session_id);
		let mut head = entry.body.kind();
		if entry.session_id.is_some() {
			head |= HAS_SESSION;
		}
		if same_session {
			head |= SAME_SESSION;
		}
		if same_distinct {
			head |= SAME_DISTINCT;
		}
		body.push(head);
		body.extend_from_slice(&entry.uuid);
		if !same_distinct {
			put_uvarint(&mut body, strings.index(&entry.distinct_id));
		}
		if let (Some(session), false) = (entry.session_id, same_session) {
			body.extend_from_slice(&session);
		}
		put_uvarint(&mut body, u64::from(entry.context));

		match &entry.body {
			Body::Event { name, properties } => {
				put_name(&mut body, &mut strings, name);
				put_map(&mut body, &mut strings, properties);
			}
			Body::Identify {
				anon_distinct_id,
				set,
				set_once,
			} => {
				put_uvarint(&mut body, strings.index(anon_distinct_id));
				put_map(&mut body, &mut strings, set);
				put_map(&mut body, &mut strings, set_once);
			}
			Body::Alias { alias } => put_uvarint(&mut body, strings.index(alias)),
			Body::Set {
				set,
				set_once,
				unset,
			} => {
				put_map(&mut body, &mut strings, set);
				put_map(&mut body, &mut strings, set_once);
				put_uvarint(&mut body, unset.len() as u64);
				for name in unset {
					put_name(&mut body, &mut strings, name);
				}
			}
			Body::GroupIdentify {
				group_type,
				group_key,
				set,
			} => {
				put_name(&mut body, &mut strings, group_type);
				put_uvarint(&mut body, strings.index(group_key));
				put_map(&mut body, &mut strings, set);
			}
		}

		previous = Some(entry);
	}

	let mut out = Vec::with_capacity(body.len() + strings.bytes + 16);
	out.extend_from_slice(MAGIC);
	out.push(VERSION);
	put_uvarint(&mut out, batch.sent_at.max(0) as u64);
	put_uvarint(&mut out, strings.list.len() as u64);
	for string in &strings.list {
		put_uvarint(&mut out, string.len() as u64);
		out.extend_from_slice(string.as_bytes());
	}
	out.extend_from_slice(&body);
	out
}

/// The batch's string table while encoding. Batches hold few distinct
/// strings, so a list searched in order is smaller than a hash map, and as
/// fast.
#[derive(Default)]
struct Strings<'a> {
	list: Vec<&'a str>,
	bytes: usize,
}

impl<'a> Strings<'a> {
	fn index(&mut self, string: &'a str) -> u64 {
		if let Some(index) = self.list.iter().position(|known| *known == string) {
			return index as u64;
		}
		self.list.push(string);
		self.bytes += string.len() + 2;
		(self.list.len() - 1) as u64
	}
}

fn put_uvarint(out: &mut Vec<u8>, mut value: u64) {
	while value >= 0x80 {
		out.push((value as u8) | 0x80);
		value >>= 7;
	}
	out.push(value as u8);
}

fn put_ivarint(out: &mut Vec<u8>, value: i64) {
	put_uvarint(out, ((value << 1) ^ (value >> 63)) as u64);
}

fn put_name<'a>(out: &mut Vec<u8>, strings: &mut Strings<'a>, name: &'a str) {
	match RESERVED.iter().position(|reserved| *reserved == name) {
		Some(code) => put_uvarint(out, code as u64 * 2),
		None => put_uvarint(out, strings.index(name) * 2 + 1),
	}
}

fn put_map<'a>(out: &mut Vec<u8>, strings: &mut Strings<'a>, map: &'a [(String, Value)]) {
	put_uvarint(out, map.len() as u64);
	for (name, value) in map {
		put_name(out, strings, name);
		put_value(out, strings, value);
	}
}

fn put_value<'a>(out: &mut Vec<u8>, strings: &mut Strings<'a>, value: &'a Value) {
	match value {
		Value::Null => out.push(TAG_NULL),
		Value::Bool(false) => out.push(TAG_FALSE),
		Value::Bool(true) => out.push(TAG_TRUE),
		Value::Int(int) => {
			out.push(TAG_INT);
			put_ivarint(out, *int);
		}
		Value::Float(float) => {
			out.push(TAG_FLOAT);
			out.extend_from_slice(&float.to_le_bytes());
		}
		Value::Text(text) => {
			out.push(TAG_TEXT);
			put_uvarint(out, text.len() as u64);
			out.extend_from_slice(text.as_bytes());
		}
		Value::Time(millis) => {
			out.push(TAG_TIME);
			put_ivarint(out, *millis);
		}
		Value::List(items) => {
			out.push(TAG_LIST);
			put_uvarint(out, items.len() as u64);
			for item in items {
				put_value(out, strings, item);
			}
		}
		Value::Map(map) => {
			out.push(TAG_MAP);
			put_map(out, strings, map);
		}
	}
}

/// How much a batch may hold, so a hostile one can't make the service
/// allocate without bound.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
	pub entries: usize,
	pub contexts: usize,
	pub strings: usize,
	/// The longest string, in bytes.
	pub string_bytes: usize,
	/// The most items in one list or map.
	pub items: usize,
	/// How deeply lists and maps nest.
	pub depth: usize,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			entries: 10_000,
			contexts: 256,
			strings: 65_536,
			string_bytes: 1 << 20,
			items: 4_096,
			depth: 8,
		}
	}
}

/// Why a batch couldn't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
	/// Where in the batch, in bytes.
	pub offset: usize,
	pub reason: &'static str,
}

impl fmt::Display for DecodeError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{} at byte {}", self.reason, self.offset)
	}
}

impl std::error::Error for DecodeError {}

/// Reads a batch, within `limits`.
///
/// # Errors
///
/// The first thing wrong with it, and where.
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Batch, DecodeError> {
	let mut reader = Reader {
		bytes,
		at: 0,
		limits,
		strings: Vec::new(),
	};

	if reader.take(3)? != MAGIC {
		return Err(error_at(0, "not a leptoskit event batch"));
	}
	if reader.byte()? != VERSION {
		return Err(error_at(3, "a batch version this service doesn't know"));
	}
	let sent_at =
		i64::try_from(reader.uvarint()?).map_err(|_| reader.error("sent_at is out of range"))?;

	let count = reader.count(limits.strings, "too many strings")?;
	reader.strings.reserve(count);
	for _ in 0..count {
		let string = reader.text()?;
		reader.strings.push(string);
	}

	let count = reader.count(limits.contexts, "too many contexts")?;
	let mut contexts = Vec::with_capacity(count);
	for _ in 0..count {
		contexts.push(reader.map(0)?);
	}

	let count = reader.count(limits.entries, "too many entries")?;
	let mut entries: Vec<Entry> = Vec::with_capacity(count);
	for _ in 0..count {
		let start = reader.at;
		let head = reader.byte()?;
		let uuid: [u8; 16] = reader.take(16)?.try_into().expect("16 bytes");
		let previous = entries.last();
		let distinct_id = if head & SAME_DISTINCT == 0 {
			reader.string_ref()?
		} else {
			previous
				.map(|entry| entry.distinct_id.clone())
				.ok_or_else(|| error_at(start, "the first entry can't repeat a distinct id"))?
		};
		let session_id = match (head & HAS_SESSION != 0, head & SAME_SESSION != 0) {
			(false, false) => None,
			(true, false) => Some(reader.take(16)?.try_into().expect("16 bytes")),
			(true, true) => {
				let session = previous.and_then(|entry| entry.session_id);
				Some(session.ok_or_else(|| error_at(start, "no previous session to repeat"))?)
			}
			(false, true) => {
				return Err(error_at(start, "a repeated session that isn't there"));
			}
		};
		let context =
			u32::try_from(reader.uvarint()?).map_err(|_| reader.error("no such context"))?;
		if context as usize >= contexts.len() && !(contexts.is_empty() && context == 0) {
			return Err(reader.error("no such context"));
		}

		let body = match head & KIND_MASK {
			0 => {
				Body::Event {
					name: reader.name()?,
					properties: reader.map(0)?,
				}
			}
			1 => {
				Body::Identify {
					anon_distinct_id: reader.string_ref()?,
					set: reader.map(0)?,
					set_once: reader.map(0)?,
				}
			}
			2 => {
				Body::Alias {
					alias: reader.string_ref()?,
				}
			}
			3 => {
				let set = reader.map(0)?;
				let set_once = reader.map(0)?;
				let count = reader.count(limits.items, "too many names to unset")?;
				let mut unset = Vec::with_capacity(count);
				for _ in 0..count {
					unset.push(reader.name()?);
				}
				Body::Set {
					set,
					set_once,
					unset,
				}
			}
			4 => {
				Body::GroupIdentify {
					group_type: reader.name()?,
					group_key: reader.string_ref()?,
					set: reader.map(0)?,
				}
			}
			_ => return Err(error_at(start, "an entry kind this service doesn't know")),
		};

		entries.push(Entry {
			uuid,
			distinct_id,
			session_id,
			context,
			body,
		});
	}

	if reader.at != bytes.len() {
		return Err(reader.error("bytes after the batch's end"));
	}

	Ok(Batch {
		sent_at,
		contexts,
		entries,
	})
}

fn error_at(offset: usize, reason: &'static str) -> DecodeError {
	DecodeError { offset, reason }
}

struct Reader<'a> {
	bytes: &'a [u8],
	at: usize,
	limits: &'a Limits,
	strings: Vec<String>,
}

impl<'a> Reader<'a> {
	fn error(&self, reason: &'static str) -> DecodeError {
		error_at(self.at, reason)
	}

	fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
		let end = self
			.at
			.checked_add(len)
			.filter(|end| *end <= self.bytes.len())
			.ok_or_else(|| self.error("the batch ends early"))?;
		let slice = &self.bytes[self.at..end];
		self.at = end;
		Ok(slice)
	}

	fn byte(&mut self) -> Result<u8, DecodeError> {
		Ok(self.take(1)?[0])
	}

	fn uvarint(&mut self) -> Result<u64, DecodeError> {
		let start = self.at;
		let mut value = 0_u64;
		for shift in (0..64).step_by(7) {
			let byte = self.byte()?;
			if shift == 63 && byte > 1 {
				return Err(error_at(start, "an integer longer than 64 bits"));
			}
			value |= u64::from(byte & 0x7F) << shift;
			if byte & 0x80 == 0 {
				return Ok(value);
			}
		}
		Err(error_at(start, "an integer longer than 64 bits"))
	}

	fn ivarint(&mut self) -> Result<i64, DecodeError> {
		let value = self.uvarint()?;
		Ok((value >> 1) as i64 ^ -((value & 1) as i64))
	}

	fn count(&mut self, limit: usize, reason: &'static str) -> Result<usize, DecodeError> {
		let start = self.at;
		let count = self.uvarint()?;
		// Every counted item takes at least a byte, which bounds what a
		// short batch can make the reader reserve.
		let remaining = (self.bytes.len() - self.at) as u64;
		if count > limit as u64 || count > remaining.max(1) {
			return Err(error_at(start, reason));
		}
		Ok(count as usize)
	}

	fn text(&mut self) -> Result<String, DecodeError> {
		let start = self.at;
		let len = self.uvarint()?;
		if len > self.limits.string_bytes as u64 {
			return Err(error_at(start, "a string longer than the limit"));
		}
		let bytes = self.take(len as usize)?;
		String::from_utf8(bytes.to_vec()).map_err(|_| error_at(start, "a string that isn't UTF-8"))
	}

	fn string_ref(&mut self) -> Result<String, DecodeError> {
		let index = self.uvarint()?;
		self.strings
			.get(usize::try_from(index).unwrap_or(usize::MAX))
			.cloned()
			.ok_or_else(|| self.error("no such string"))
	}

	fn name(&mut self) -> Result<String, DecodeError> {
		let code = self.uvarint()?;
		if code % 2 == 0 {
			RESERVED
				.get(usize::try_from(code / 2).unwrap_or(usize::MAX))
				.map(|name| (*name).to_owned())
				.ok_or_else(|| self.error("a reserved name this service doesn't know"))
		} else {
			self.strings
				.get(usize::try_from(code / 2).unwrap_or(usize::MAX))
				.cloned()
				.ok_or_else(|| self.error("no such string"))
		}
	}

	fn map(&mut self, depth: usize) -> Result<Properties, DecodeError> {
		let count = self.count(self.limits.items, "a map with too many items")?;
		let mut map = Vec::with_capacity(count);
		for _ in 0..count {
			let name = self.name()?;
			let value = self.value(depth)?;
			map.push((name, value));
		}
		Ok(map)
	}

	fn value(&mut self, depth: usize) -> Result<Value, DecodeError> {
		let start = self.at;
		Ok(match self.byte()? {
			TAG_NULL => Value::Null,
			TAG_FALSE => Value::Bool(false),
			TAG_TRUE => Value::Bool(true),
			TAG_INT => Value::Int(self.ivarint()?),
			TAG_FLOAT => {
				Value::Float(f64::from_le_bytes(
					self.take(8)?.try_into().expect("8 bytes"),
				))
			}
			TAG_TEXT => Value::Text(self.text()?),
			TAG_TIME => Value::Time(self.ivarint()?),
			TAG_LIST | TAG_MAP if depth >= self.limits.depth => {
				return Err(error_at(start, "lists and maps nested too deeply"));
			}
			TAG_LIST => {
				let count = self.count(self.limits.items, "a list with too many items")?;
				let mut items = Vec::with_capacity(count);
				for _ in 0..count {
					items.push(self.value(depth + 1)?);
				}
				Value::List(items)
			}
			TAG_MAP => Value::Map(self.map(depth + 1)?),
			_ => return Err(error_at(start, "a value type this service doesn't know")),
		})
	}
}

/// The time a UUID v7 holds, in milliseconds since the Unix epoch, or
/// `None` when it isn't a UUID v7.
pub fn uuid_v7_millis(uuid: &[u8; 16]) -> Option<i64> {
	let version = uuid[6] >> 4;
	let variant = uuid[8] & 0xC0;
	if version != 7 || variant != 0x80 {
		return None;
	}
	let mut millis = [0_u8; 8];
	millis[2..].copy_from_slice(&uuid[..6]);
	Some(i64::from_be_bytes(millis))
}

/// A UUID v7 for `millis`, with `random` filling the rest.
pub fn uuid_v7(millis: i64, random: [u8; 10]) -> [u8; 16] {
	let mut uuid = [0_u8; 16];
	uuid[..6].copy_from_slice(&millis.max(0).to_be_bytes()[2..]);
	uuid[6..].copy_from_slice(&random);
	uuid[6] = 0x70 | (uuid[6] & 0x0F);
	uuid[8] = 0x80 | (uuid[8] & 0x3F);
	uuid
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn varints_round_trip_at_their_edges() {
		for value in [
			0,
			1,
			127,
			128,
			16_383,
			16_384,
			u64::from(u32::MAX),
			u64::MAX,
		] {
			let mut out = Vec::new();
			put_uvarint(&mut out, value);
			let limits = Limits::default();
			let mut reader = Reader {
				bytes: &out,
				at: 0,
				limits: &limits,
				strings: Vec::new(),
			};
			assert_eq!(reader.uvarint().unwrap(), value);
			assert_eq!(reader.at, out.len());
		}
		for value in [0, -1, 1, i64::MIN, i64::MAX] {
			let mut out = Vec::new();
			put_ivarint(&mut out, value);
			let limits = Limits::default();
			let mut reader = Reader {
				bytes: &out,
				at: 0,
				limits: &limits,
				strings: Vec::new(),
			};
			assert_eq!(reader.ivarint().unwrap(), value);
		}
	}

	#[test]
	fn an_overlong_varint_is_refused() {
		let bytes = [0xFF; 11];
		let limits = Limits::default();
		let mut reader = Reader {
			bytes: &bytes,
			at: 0,
			limits: &limits,
			strings: Vec::new(),
		};
		assert_eq!(
			reader.uvarint().unwrap_err().reason,
			"an integer longer than 64 bits"
		);
	}

	#[test]
	fn a_uuid_v7_holds_its_time() {
		let uuid = uuid_v7(1_762_646_400_123, [0xAB; 10]);
		assert_eq!(uuid_v7_millis(&uuid), Some(1_762_646_400_123));
		assert_eq!(uuid_v7_millis(&[0; 16]), None);
	}

	#[test]
	fn reserved_names_are_unique() {
		let mut names = RESERVED.to_vec();
		names.sort_unstable();
		names.dedup();
		assert_eq!(names.len(), RESERVED.len());
	}
}
