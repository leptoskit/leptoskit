//! Targeting conditions: the boolean grammar a send can address an
//! audience with, written the way Firebase Cloud Messaging writes it, so
//! a migration from FCM's conditions is a paste.
//!
//! ```text
//! 'TopicA' in topics && ('TopicB' in topics || !('TopicC' in topics))
//! ```
//!
//! A condition names topics and combines them with `&&`, `||`, `!`, and
//! parentheses. Precedence is `!` first, then `&&`, then `||`, so
//! `'a' in topics || 'b' in topics && 'c' in topics` means
//! `'a' in topics || ('b' in topics && 'c' in topics)`.
//!
//! One shape is refused outright: a condition that is already true when
//! the person holds no topics at all, such as `!('a' in topics)`. There
//! is no "everyone" audience yet, and a condition that starts matching
//! before a person subscribes to anything would quietly become one, so
//! [`Condition::parse`] checks the parsed tree with every membership
//! false and refuses a condition that holds.
//!
//! The bounds follow FCM's: at most [`MAX_CONDITION_TOPICS`] distinct
//! topics, nesting at most [`MAX_CONDITION_DEPTH`] levels deep, and at
//! most [`MAX_CONDITION_LEN`] bytes of text. Topic names inside the
//! quotes are topic names anywhere: 1 to
//! [`MAX_TOPIC_LEN`](crate::MAX_TOPIC_LEN) bytes of the characters FCM
//! accepts.

/// The longest condition, in bytes.
pub const MAX_CONDITION_LEN: usize = 1_024;

/// The most distinct topics one condition may name, as FCM allows.
pub const MAX_CONDITION_TOPICS: usize = 5;

/// How deeply a condition may nest: a membership is depth 1, and every
/// `!`, `&&`, and `||` over it adds one.
pub const MAX_CONDITION_DEPTH: usize = 8;

/// A parsed condition: a boolean over the topics a person holds. Built by
/// [`Condition::parse`], evaluated by [`Condition::matches`] against the
/// set the moment asks about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
	/// `'name' in topics`: true when the person holds the topic.
	Topic(String),
	/// `left && right`.
	And(Box<Condition>, Box<Condition>),
	/// `left || right`.
	Or(Box<Condition>, Box<Condition>),
	/// `!inner`.
	Not(Box<Condition>),
}

/// Why a condition can't be used: what's wrong with it, and the byte
/// offset in the condition text where the parser gave up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConditionError {
	/// What's wrong, in plain language.
	pub message: String,
	/// Where in the condition, in bytes from its start.
	pub offset: usize,
}

impl std::fmt::Display for ConditionError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{} (at byte {})", self.message, self.offset)
	}
}

impl std::error::Error for ConditionError {}

impl Condition {
	/// Parses and checks a condition: the grammar, the topic names, the
	/// bounds, and the anchoring rule above. The same text always parses
	/// to the same tree, and a condition that passed once can be stored
	/// and parsed again by the service's fanout.
	///
	/// # Errors
	///
	/// [`ConditionError`], with the byte offset the parser gave up at:
	/// text past the length bound, a topic name outside FCM's charset, a
	/// missing `in topics`, unbalanced parentheses, a stray token, more
	/// than [`MAX_CONDITION_TOPICS`] distinct topics, nesting past
	/// [`MAX_CONDITION_DEPTH`], or a condition that holds when nobody
	/// holds any topic.
	pub fn parse(text: &str) -> Result<Self, ConditionError> {
		if text.len() > MAX_CONDITION_LEN {
			return Err(ConditionError {
				message: format!("a condition is at most {MAX_CONDITION_LEN} bytes"),
				offset: 0,
			});
		}

		let mut parser = Parser {
			text: text.as_bytes(),
			pos: 0,
			topics: Vec::new(),
		};
		let parsed = parser.expression()?;
		parser.skip_whitespace();
		let end = parser.pos;

		if end != parser.text.len() {
			return Err(ConditionError {
				message: format!(
					"unexpected {} after the condition",
					name_of(parser.text[end])
				),
				offset: end,
			});
		}

		if parser.topics.len() > MAX_CONDITION_TOPICS {
			return Err(ConditionError {
				message: format!(
					"a condition names at most {MAX_CONDITION_TOPICS} distinct topics"
				),
				offset: 0,
			});
		}

		// Anchored: a person with no topics must not match, or the
		// condition would be an everyone audience in disguise.
		if parsed.matches(|_| false) {
			return Err(ConditionError {
				message: "a condition must not match a person who holds no topics: name at least \
				          one topic it requires"
					.into(),
				offset: 0,
			});
		}

		Ok(parsed)
	}

	/// The distinct topics the condition names, in order of first
	/// appearance. At most [`MAX_CONDITION_TOPICS`] once parsed.
	#[must_use]
	pub fn topics(&self) -> Vec<&str> {
		let mut topics = Vec::new();
		collect_topics(self, &mut topics);
		topics
	}

	/// Whether the condition holds for a person whose memberships `holds`
	/// answers: called with the topic names the condition names, never
	/// any other.
	#[must_use]
	pub fn matches(&self, holds: impl Fn(&str) -> bool) -> bool {
		self.matches_with(&holds)
	}

	/// [`Condition::matches`] over a trait object, so the recursion
	/// doesn't build a fresh closure type per level.
	fn matches_with(&self, holds: &dyn Fn(&str) -> bool) -> bool {
		match self {
			Condition::Topic(topic) => holds(topic),
			Condition::And(left, right) => left.matches_with(holds) && right.matches_with(holds),
			Condition::Or(left, right) => left.matches_with(holds) || right.matches_with(holds),
			Condition::Not(inner) => !inner.matches_with(holds),
		}
	}

	/// The tree's depth: a membership is 1, and every operator over one
	/// adds a level.
	fn depth(&self) -> usize {
		match self {
			Condition::Topic(_) => 1,
			Condition::And(left, right) | Condition::Or(left, right) => {
				left.depth().max(right.depth()) + 1
			}
			Condition::Not(inner) => inner.depth() + 1,
		}
	}
}

/// Collects the distinct topics a tree names, in order.
fn collect_topics<'a>(condition: &'a Condition, topics: &mut Vec<&'a str>) {
	match condition {
		Condition::Topic(topic) => {
			if !topics.contains(&topic.as_str()) {
				topics.push(topic.as_str());
			}
		}
		Condition::And(left, right) | Condition::Or(left, right) => {
			collect_topics(left, topics);
			collect_topics(right, topics);
		}
		Condition::Not(inner) => collect_topics(inner, topics),
	}
}

/// Checks a condition before a send names it, the way
/// [`check_send`](crate::check_send) checks the rest: parse it fully, and say
/// what's wrong.
///
/// # Errors
///
/// [`ConditionError`], as [`Condition::parse`].
pub fn check_condition(text: &str) -> Result<(), ConditionError> {
	Condition::parse(text).map(|_| ())
}

/// The recursive-descent parser. Tracks the position in the text and the
/// distinct topics seen, so the topic bound can name its offset.
struct Parser<'a> {
	text: &'a [u8],
	pos: usize,
	topics: Vec<(&'a str, usize)>,
}

impl Parser<'_> {
	/// `or`, the loosest level: `and` separated by `||`.
	fn expression(&mut self) -> Result<Condition, ConditionError> {
		let start = self.pos;
		let mut left = self.conjunction()?;

		while self.operator(b"||") {
			let right = self.conjunction()?;
			left = Condition::Or(Box::new(left), Box::new(right));
			Self::bounded(&left, start)?;
		}

		Ok(left)
	}

	/// `and`: `unary` separated by `&&`.
	fn conjunction(&mut self) -> Result<Condition, ConditionError> {
		let start = self.pos;
		let mut left = self.unary()?;

		while self.operator(b"&&") {
			let right = self.unary()?;
			left = Condition::And(Box::new(left), Box::new(right));
			Self::bounded(&left, start)?;
		}

		Ok(left)
	}

	/// `!`, a parenthesized expression, or a membership.
	fn unary(&mut self) -> Result<Condition, ConditionError> {
		self.skip_whitespace();

		let start = self.pos;

		if self.eat(b"!") {
			let inner = self.unary()?;
			let not = Condition::Not(Box::new(inner));
			Self::bounded(&not, start)?;

			return Ok(not);
		}

		if self.eat(b"(") {
			let inner = self.expression()?;
			self.skip_whitespace();

			if !self.eat(b")") {
				return Err(ConditionError {
					message: "an unclosed `(`: expected `)` here".into(),
					offset: self.pos,
				});
			}

			return Ok(inner);
		}

		self.membership()
	}

	/// `'name' in topics`.
	fn membership(&mut self) -> Result<Condition, ConditionError> {
		if !self.eat(b"'") {
			return Err(ConditionError {
				message: format!("expected a topic in quotes, found {}", self.here()),
				offset: self.pos,
			});
		}

		let name_start = self.pos;

		while self
			.peek()
			.is_some_and(|byte| byte != b'\'' && !byte.is_ascii_whitespace())
		{
			self.pos += 1;
		}

		let name_end = self.pos;

		if !self.eat(b"'") {
			return Err(ConditionError {
				message: "an unclosed topic name: expected `'` here".into(),
				offset: self.pos,
			});
		}

		let name = std::str::from_utf8(&self.text[name_start..name_end]).map_err(|_| {
			ConditionError {
				message: "a topic name is ASCII".into(),
				offset: name_start,
			}
		})?;

		crate::check_topic(name).map_err(|problem| {
			ConditionError {
				message: format!("the topic name `{name}` isn't one: {problem}"),
				offset: name_start,
			}
		})?;

		self.skip_whitespace();
		self.expect(b"in")?;
		self.skip_whitespace();
		self.expect(b"topics")?;

		if !self.topics.iter().any(|(seen, _)| *seen == name) {
			if self.topics.len() >= MAX_CONDITION_TOPICS {
				return Err(ConditionError {
					message: format!(
						"a condition names at most {MAX_CONDITION_TOPICS} distinct topics, and \
						 `{name}` is one too many"
					),
					offset: name_start,
				});
			}

			self.topics.push((name, name_start));
		}

		Ok(Condition::Topic(name.to_owned()))
	}

	/// Consumes `bytes` when the parser is on it, answering whether it
	/// did. Whitespace before it is allowed.
	fn operator(&mut self, bytes: &[u8]) -> bool {
		self.skip_whitespace();
		self.eat(bytes)
	}

	/// Consumes the word `bytes` or fails at the parser's position.
	fn expect(&mut self, bytes: &[u8]) -> Result<(), ConditionError> {
		if !self.eat(bytes) {
			return Err(ConditionError {
				message: format!("expected `{}`", String::from_utf8_lossy(bytes)),
				offset: self.pos,
			});
		}

		// The word must end where a word ends, so `in` doesn't match the
		// start of `input`.
		if self.peek().is_some_and(|byte| byte.is_ascii_alphanumeric()) {
			return Err(ConditionError {
				message: format!(
					"expected `{}` standing alone",
					String::from_utf8_lossy(bytes)
				),
				offset: self.pos,
			});
		}

		Ok(())
	}

	/// Whether the parser is on `bytes`, which it then consumes.
	fn eat(&mut self, bytes: &[u8]) -> bool {
		if self.text[self.pos..].starts_with(bytes) {
			self.pos += bytes.len();
			true
		} else {
			false
		}
	}

	fn peek(&self) -> Option<u8> {
		self.text.get(self.pos).copied()
	}

	fn skip_whitespace(&mut self) {
		while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
			self.pos += 1;
		}
	}

	/// Refuses a tree that nests past the bound, at the offset where the
	/// too-deep expression started.
	fn bounded(condition: &Condition, offset: usize) -> Result<(), ConditionError> {
		if condition.depth() > MAX_CONDITION_DEPTH {
			return Err(ConditionError {
				message: format!("a condition nests at most {MAX_CONDITION_DEPTH} levels deep"),
				offset,
			});
		}

		Ok(())
	}

	/// What sits at the parser's position, for an error that names it.
	fn here(&self) -> String {
		self.peek().map_or_else(
			|| "the end of the condition".to_owned(),
			|byte| format!("`{}`", char::from(byte)),
		)
	}
}

/// The readable name of the byte a stray token starts with.
fn name_of(byte: u8) -> String {
	match byte {
		b'\'' => "a topic".to_owned(),
		b'!' => "`!`".to_owned(),
		b'(' => "`(`".to_owned(),
		b')' => "`)`".to_owned(),
		b'&' => "`&&`".to_owned(),
		b'|' => "`||`".to_owned(),
		other if other.is_ascii_whitespace() => "whitespace".to_owned(),
		other => format!("`{}`", char::from(other)),
	}
}

#[cfg(test)]
mod tests {
	use super::Condition;
	use super::ConditionError;
	use super::MAX_CONDITION_DEPTH;
	use super::MAX_CONDITION_LEN;

	/// The topics a person holds, as a set.
	fn holding<'a>(topics: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
		move |topic| topics.contains(&topic)
	}

	/// FCM's documented example: subscribed to `TopicA`, and either `TopicB`
	/// or not `TopicC`.
	#[test]
	fn fcm_documented_example_parses_and_evaluates() {
		let condition = "'TopicA' in topics && ('TopicB' in topics || !('TopicC' in topics))";
		let parsed = Condition::parse(condition).expect("FCM's own example parses");

		assert_eq!(
			parsed.topics(),
			vec!["TopicA", "TopicB", "TopicC"],
			"in order of appearance"
		);

		assert!(parsed.matches(holding(&["TopicA"])));
		assert!(parsed.matches(holding(&["TopicA", "TopicB"])));
		assert!(
			!parsed.matches(holding(&["TopicA", "TopicC"])),
			"TopicB absent and TopicC present"
		);
		assert!(!parsed.matches(holding(&["TopicB", "TopicC"])), "no TopicA");
		assert!(!parsed.matches(holding(&[])), "anchored: nobody matches");
	}

	/// The example FCM marks as reserved for the future is not quietly
	/// accepted: a bare topic with no `in topics` is a parse error.
	#[test]
	fn fcm_examples_with_two_conditions_parse() {
		let both = Condition::parse("'TopicA' in topics && 'TopicB' in topics").unwrap();
		assert!(both.matches(holding(&["TopicA", "TopicB"])));
		assert!(!both.matches(holding(&["TopicA"])));

		let either_c =
			Condition::parse("('TopicA' in topics || 'TopicB' in topics) && 'TopicC' in topics")
				.unwrap();
		assert!(either_c.matches(holding(&["TopicB", "TopicC"])));
		assert!(!either_c.matches(holding(&["TopicA"])));
	}

	/// Precedence: `!` binds tighter than `&&`, which binds tighter than
	/// `||`, and parentheses say otherwise.
	#[test]
	fn not_binds_tighter_than_and_which_binds_tighter_than_or() {
		let parsed = Condition::parse("!'a' in topics && 'b' in topics || 'c' in topics").unwrap();
		let expected = Condition::Or(
			Box::new(Condition::And(
				Box::new(Condition::Not(Box::new(Condition::Topic("a".into())))),
				Box::new(Condition::Topic("b".into())),
			)),
			Box::new(Condition::Topic("c".into())),
		);
		assert_eq!(parsed, expected);

		// `!` over a parenthesized group is one negation of the whole.
		let grouped =
			Condition::parse("!('a' in topics || 'b' in topics) && 'c' in topics").unwrap();
		assert!(grouped.matches(holding(&["c"])));
		assert!(!grouped.matches(holding(&["a", "c"])));
		assert!(!grouped.matches(holding(&[])), "still anchored by `c`");
	}

	/// Whitespace is free where a token boundary already exists.
	#[test]
	fn whitespace_is_free() {
		let parsed = Condition::parse("  'a'  in   topics\n&&\t'b' in topics  ").unwrap();
		assert!(parsed.matches(holding(&["a", "b"])));
	}

	/// Every way a condition can be wrong, each with the offset the
	/// parser gave up at.
	#[test]
	fn errors_carry_byte_offsets() {
		let cases: &[(&str, usize)] = &[
			("", 0),
			("   ", 3),
			("'a' in topics &&", 16),
			("'a' in topic", 7),
			("'a' in", 6),
			("'a'", 3),
			("a in topics", 0),
			("'a' in topics 'b' in topics", 14),
			("('a' in topics", 14),
			("'a' in topics)", 13),
			("'a in topics", 2),
			("'a&' in topics", 1),
			("!(", 2),
			("!'a' in topics &&", 17),
		];

		for (text, offset) in cases {
			let error = Condition::parse(text).expect_err(text);
			assert_eq!(
				error.offset, *offset,
				"{text:?}: the error points where the parser gave up: {error}"
			);
		}
	}

	/// The bounds: length, distinct topics, and nesting depth.
	#[test]
	fn the_bounds_hold() {
		let long = format!("'{}' in topics", "a".repeat(MAX_CONDITION_LEN));
		let error = Condition::parse(&long).expect_err("past the length bound");
		assert_eq!(error.offset, 0);

		let five = (1..=super::MAX_CONDITION_TOPICS)
			.map(|n| format!("'t{n}' in topics"))
			.collect::<Vec<_>>()
			.join(" && ");
		Condition::parse(&five).expect("five distinct topics fit");

		let six = (1..=super::MAX_CONDITION_TOPICS + 1)
			.map(|n| format!("'t{n}' in topics"))
			.collect::<Vec<_>>()
			.join(" && ");
		let error = Condition::parse(&six).expect_err("six distinct topics don't");
		assert!(
			error.message.contains("at most 5"),
			"the error names the bound: {error}"
		);

		// Depth: a membership is 1, every operator adds a level. Six
		// negations under a conjunction sit exactly at eight levels; a
		// seventh makes nine. The plain negation stack can't test the
		// boundary because nothing in it is anchored.
		let deep_ok = format!(
			"{}'a' in topics && 'b' in topics",
			"!".repeat(MAX_CONDITION_DEPTH - 2)
		);
		Condition::parse(&deep_ok).expect("eight levels fit");

		let deep_bad = format!(
			"{}'a' in topics && 'b' in topics",
			"!".repeat(MAX_CONDITION_DEPTH - 1)
		);
		let error = Condition::parse(&deep_bad).expect_err("nine levels don't");
		assert!(error.message.contains("nest"), "names the bound: {error}");

		let wide_bad = format!("{}'a' in topics", "(".repeat(MAX_CONDITION_DEPTH));
		let error = Condition::parse(&wide_bad).expect_err("parentheses nest too");
		assert!(
			error.message.contains("unclosed") || error.message.contains("nest"),
			"{error}"
		);
	}

	/// A condition that already holds for a person with no topics is an
	/// everyone audience in disguise, and is refused wherever the
	/// negation hides.
	#[test]
	fn unanchored_conditions_are_refused() {
		for text in [
			"!'a' in topics",
			"!('a' in topics && 'b' in topics)",
			"'a' in topics || !'b' in topics",
			"!('a' in topics) || !('b' in topics)",
		] {
			let error = Condition::parse(text).expect_err(text);
			assert!(
				error.message.contains("no topics"),
				"{text}: the refusal says why: {error}"
			);
		}

		// Requiring any topic, even beside negations, anchors it.
		Condition::parse("!'a' in topics && 'b' in topics").expect("anchored by `b`");
		Condition::parse("!'a' in topics && !('b' in topics || !'c' in topics)").unwrap();
	}

	/// A topic name outside the charset is refused where it starts.
	#[test]
	fn topic_names_follow_fcms_charset() {
		for bad in [
			"' '",
			"'a b' in topics",
			"'a/b' in topics",
			"'a#b' in topics",
		] {
			assert!(Condition::parse(bad).is_err(), "{bad} is not a topic name");
		}

		let longest = format!("'{}' in topics", "a".repeat(128));
		Condition::parse(&longest).expect("128 bytes fit");

		let over = format!("'{}' in topics", "a".repeat(129));
		assert!(Condition::parse(&over).is_err(), "129 bytes don't");
	}

	/// The error prints as a sentence with its offset.
	#[test]
	fn the_error_reads_plainly() {
		let error = ConditionError {
			message: "an unclosed topic name".into(),
			offset: 7,
		};
		let printed = error.to_string();
		assert!(printed.contains("byte 7"), "{printed}");
	}
}
