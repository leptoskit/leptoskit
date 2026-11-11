//! A bounded window of a payload around the byte a decoder failed at.

use std::fmt;
use std::ops::Range;

/// The most text an [`Excerpt`] keeps, in bytes, not counting the markers
/// [`Excerpt`]'s `Display` adds around it.
pub const MAX_EXCERPT_BYTES: usize = 512;

/// Where an excerpt's failure was, in its `Display`.
const HERE: &str = "<<HERE>>";

/// A window of at most [`MAX_EXCERPT_BYTES`] of a payload around one
/// position, cut on character boundaries.
///
/// The text is the payload's own when it is UTF-8; other bytes read as
/// U+FFFD, and control characters, newlines included, as spaces, so the
/// window stays on one line of the log.
///
/// ```
/// use leptoskit_log::Excerpt;
///
/// let payload = br#"{"style":{"opacity":{"value":1}}}"#;
/// let excerpt = Excerpt::around(payload, 20);
/// assert_eq!(
/// 	excerpt.to_string(),
/// 	r#"bytes 0..33 of 33: {"style":{"opacity":<<HERE>>{"value":1}}}"#
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excerpt {
	/// The payload bytes the window was cut from. Invalid UTF-8 can read as
	/// more text than fits, and is then cut shorter than this range.
	pub range: Range<usize>,
	/// The payload's whole length.
	pub total: usize,
	/// The text before the position.
	pub before: String,
	/// The text from the position on.
	pub after: String,
}

impl Excerpt {
	/// The window of `payload` around byte `at`, which is clamped to the
	/// payload. Up to half the room goes before `at` and the rest after, so
	/// the failure stays in view.
	pub fn around(payload: &[u8], at: usize) -> Self {
		let half = MAX_EXCERPT_BYTES / 2;
		let at = boundary_before(payload, at.min(payload.len()));
		let start = boundary_after(payload, at.saturating_sub(half)).min(at);
		let before = keep_end(readable(&payload[start..at]), half);
		let room = MAX_EXCERPT_BYTES - before.len();
		let end = boundary_before(payload, at.saturating_add(room).min(payload.len())).max(at);
		let after = keep_start(readable(&payload[at..end]), room);

		Self {
			range: start..end,
			total: payload.len(),
			before,
			after,
		}
	}

	/// The window around a JSON decoder's position, given as its 1-based
	/// `line` and `column`, as `serde_json::Error` reports them.
	pub fn at_line_column(payload: &[u8], line: usize, column: usize) -> Self {
		Self::around(payload, offset(payload, line, column))
	}

	/// The excerpt's text, without the marker `Display` puts at the
	/// position.
	pub fn text(&self) -> String {
		format!("{}{}", self.before, self.after)
	}
}

impl fmt::Display for Excerpt {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		let Range { start, end } = self.range;
		let (open, close) = (
			if start > 0 { "…" } else { "" },
			if end < self.total { "…" } else { "" },
		);

		write!(
			formatter,
			"bytes {start}..{end} of {}: {open}{}{HERE}{}{close}",
			self.total, self.before, self.after
		)
	}
}

/// The byte offset of a 1-based line and column. A column past its line, or a
/// line past the payload, is clamped to the end.
fn offset(payload: &[u8], line: usize, column: usize) -> usize {
	let start = if line <= 1 {
		0
	} else {
		payload
			.iter()
			.enumerate()
			.filter(|(_, byte)| **byte == b'\n')
			.nth(line - 2)
			.map_or(payload.len(), |(index, _)| index + 1)
	};

	start
		.saturating_add(column.saturating_sub(1))
		.min(payload.len())
}

fn is_continuation(byte: u8) -> bool {
	byte & 0xC0 == 0x80
}

/// `at`, moved back to the start of the character it falls in. Invalid
/// UTF-8 moves at most three bytes.
fn boundary_before(payload: &[u8], mut at: usize) -> usize {
	for _ in 0..3 {
		if at == 0 || at >= payload.len() || !is_continuation(payload[at]) {
			break;
		}

		at -= 1;
	}

	at
}

/// `at`, moved on past the character it falls in. Invalid UTF-8 moves at
/// most three bytes.
fn boundary_after(payload: &[u8], mut at: usize) -> usize {
	for _ in 0..3 {
		if at >= payload.len() || !is_continuation(payload[at]) {
			break;
		}

		at += 1;
	}

	at
}

/// The bytes as one line of text.
fn readable(bytes: &[u8]) -> String {
	String::from_utf8_lossy(bytes)
		.chars()
		.map(|character| {
			if character.is_control() {
				' '
			} else {
				character
			}
		})
		.collect()
}

/// The last characters of `text` that fit in `limit` bytes.
fn keep_end(text: String, limit: usize) -> String {
	if text.len() <= limit {
		return text;
	}

	let mut start = text.len() - limit;

	while !text.is_char_boundary(start) {
		start += 1;
	}

	text[start..].to_owned()
}

/// The first characters of `text` that fit in `limit` bytes.
fn keep_start(mut text: String, limit: usize) -> String {
	if text.len() <= limit {
		return text;
	}

	let mut end = limit;

	while !text.is_char_boundary(end) {
		end -= 1;
	}

	text.truncate(end);
	text
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A small deterministic generator, so the property tests need no crate
	/// and fail the same way every run.
	struct Lcg(u64);

	impl Lcg {
		fn next(&mut self) -> u64 {
			self.0 = self
				.0
				.wrapping_mul(6_364_136_223_846_793_005)
				.wrapping_add(1_442_695_040_888_963_407);
			self.0 >> 33
		}

		fn below(&mut self, limit: usize) -> usize {
			(self.next() % limit.max(1) as u64) as usize
		}
	}

	const ALPHABET: &[&str] = &[
		"a", "{", "\"", ":", "1", " ", "é", "ü", "€", "中", "🦀", "\n", "\t", "\u{0}",
	];

	fn text(random: &mut Lcg, characters: usize) -> String {
		(0..characters)
			.map(|_| ALPHABET[random.below(ALPHABET.len())])
			.collect()
	}

	#[test]
	fn every_excerpt_is_bounded_and_whole_characters() {
		let mut random = Lcg(0x5eed);

		for _ in 0..2_000 {
			let payload: Vec<u8> = if random.below(3) == 0 {
				(0..random.below(4_000))
					.map(|_| random.next() as u8)
					.collect()
			} else {
				let characters = random.below(2_000);
				text(&mut random, characters).into_bytes()
			};
			let at = random.below(payload.len() + 64);
			let excerpt = Excerpt::around(&payload, at);

			assert!(
				excerpt.before.len() + excerpt.after.len() <= MAX_EXCERPT_BYTES,
				"{} bytes from {} at {at}",
				excerpt.before.len() + excerpt.after.len(),
				payload.len()
			);
			assert!(excerpt.to_string().len() <= MAX_EXCERPT_BYTES + 64);
			assert!(excerpt.range.start <= excerpt.range.end);
			assert!(excerpt.range.end <= payload.len());
			assert_eq!(excerpt.total, payload.len());
			assert!(!excerpt.text().chars().any(char::is_control));
		}
	}

	#[test]
	fn a_utf8_payload_is_quoted_exactly_around_its_position() {
		let mut random = Lcg(0xfeed);

		for _ in 0..2_000 {
			let characters = random.below(1_500);
			let payload = text(&mut random, characters);
			let at = random.below(payload.len() + 1);
			let excerpt = Excerpt::around(payload.as_bytes(), at);
			let readable = readable(payload.as_bytes());

			// The window is a run of the payload's own characters, cut where
			// characters start, with the position between the halves.
			assert!(readable.contains(&excerpt.text()), "{excerpt:?}");
			assert!(payload.is_char_boundary(excerpt.range.start));
			assert!(payload.is_char_boundary(excerpt.range.end));
			let position = boundary_before(payload.as_bytes(), at);
			assert!(
				readable[..position].ends_with(&excerpt.before),
				"{excerpt:?} at {at}"
			);
			assert!(readable[position..].starts_with(&excerpt.after));
		}
	}

	#[test]
	fn a_long_payload_shows_its_failure_with_room_on_both_sides() {
		let payload = format!(
			r#"{{"pad":"{}","opacity":{{"value":1}},"tail":"{}"}}"#,
			"x".repeat(2_000),
			"y".repeat(2_000)
		);
		let at = payload.find(r#"{"value"#).unwrap();
		let excerpt = Excerpt::around(payload.as_bytes(), at);

		assert!(excerpt.before.ends_with(r#""opacity":"#));
		assert!(excerpt.after.starts_with(r#"{"value":1}"#));
		assert_eq!(excerpt.before.len(), MAX_EXCERPT_BYTES / 2);
		assert_eq!(excerpt.after.len(), MAX_EXCERPT_BYTES / 2);
		assert!(excerpt.to_string().starts_with(&format!(
			"bytes {}..{} of {}: …",
			at - 256,
			at + 256,
			payload.len()
		)));
		assert!(excerpt.to_string().ends_with('…'));
	}

	#[test]
	fn line_and_column_positions_point_at_the_failing_value() {
		// The shape of the Seeker's failure: a map where a number belongs.
		let payload = "{\"kind\":\"text\",\n\"opacity\":{\"value\":1}}";
		let (line, column) = line_column(payload, "{\"value\"");
		let excerpt = Excerpt::at_line_column(payload.as_bytes(), line, column);
		assert!(excerpt.after.starts_with("{\"value\""), "{excerpt}");
	}

	/// The 1-based line and column of `needle` in `payload`, as a JSON
	/// decoder reports the start of a value.
	fn line_column(payload: &str, needle: &str) -> (usize, usize) {
		let at = payload.find(needle).unwrap();
		let line = payload[..at].matches('\n').count() + 1;
		let column = at - payload[..at].rfind('\n').map_or(0, |newline| newline + 1) + 1;
		(line, column)
	}

	#[test]
	fn positions_past_the_end_are_clamped() {
		assert_eq!(offset(b"ab\ncd", 9, 9), 5);
		assert_eq!(offset(b"ab\ncd", 2, 9), 5);
		assert_eq!(offset(b"ab\ncd", 2, 2), 4);
		assert_eq!(offset(b"", 1, 1), 0);
		let excerpt = Excerpt::around(b"", 10);
		assert_eq!(excerpt.to_string(), "bytes 0..0 of 0: <<HERE>>");
	}
}
