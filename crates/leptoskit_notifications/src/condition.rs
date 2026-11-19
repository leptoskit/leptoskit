//! Targeting conditions: the boolean grammar a send can address an
//! audience with, written the way Firebase Cloud Messaging writes its
//! topic half, so a migration from FCM's conditions is a paste.
//!
//! ```text
//! 'TopicA' in topics && ('TopicB' in topics || !('TopicC' in topics))
//! user.plan == 'pro' && 'news.product' in topics
//! ```
//!
//! A condition names topics and person attributes, and combines them
//! with `&&`, `||`, `!`, and parentheses. Precedence is `!` first, then
//! `&&`, then `||`, so `'a' in topics || 'b' in topics && user.beta ==
//! 'yes'` means `'a' in topics || ('b' in topics && user.beta ==
//! 'yes')`.
//!
//! A topic membership is `'name' in topics`. An attribute predicate is
//! one of:
//!
//! - `user.<key> == 'value'`, `user.<key> != 'value'`
//! - `user.<key> in ['a', 'b']`
//! - `has(user.<key>)`
//! - `user.<key> < '10'` with `<`, `<=`, `>`, `>=`: numeric, true only when the
//!   held value and the literal both parse as finite numbers
//!
//! An attribute the person doesn't hold answers false to every
//! predicate, `!=` and `in` included, so nobody matches a predicate
//! merely for lacking the attribute.
//!
//! A device predicate asks about one of the person's enrolled devices, with
//! the same operators: `device.<key>` for one of [`DeviceKey`]'s keys —
//! `device.platform` (one of [`PushPlatform::as_str`]'s names),
//! `device.locale`, `device.language` (the locale's primary subtag, so
//! `device.language == 'fr'` matches `fr-CA`), `device.app_version`, and
//! `device.time_zone`. Locale and language compare without regard to
//! case. `device.app_version` compares as a version under every
//! comparison operator — see [`compare_versions`] for the rule. Any other
//! key is a parse error that names the ones there are.
//!
//! A device predicate is judged per device: a send's audience is everyone
//! the condition holds for with no device at all, or on at least one of
//! their live endpoints, and the push goes only to the endpoints it holds
//! for ([`Condition::matches_on`]). With no device, a device predicate
//! answers exactly as an absent attribute does: false, `!=` and `in`
//! included.
//!
//! One shape is refused outright: a condition that is already true when
//! the person holds no topics, no attributes, and no device at all, such
//! as `!('a' in topics)`. That is an "everyone" audience in disguise — the
//! registry has a plain target for that ([`SendTarget::Everyone`]) — so
//! [`Condition::parse`] checks the parsed tree with every membership
//! false, every attribute absent, and no device, and refuses a condition
//! that holds.
//!
//! The bounds follow FCM's: at most [`MAX_CONDITION_TOPICS`] distinct
//! topics, at most [`MAX_CONDITION_ATTRIBUTES`] attribute and device
//! predicates together, nesting at most [`MAX_CONDITION_DEPTH`] levels
//! deep, and at most [`MAX_CONDITION_LEN`] bytes of text. Topic names
//! inside the quotes are topic names anywhere: 1 to
//! [`MAX_TOPIC_LEN`](crate::MAX_TOPIC_LEN) bytes of the characters FCM
//! accepts. Attribute keys are `[a-z0-9_]` — 1 to
//! [`MAX_ATTRIBUTE_KEY_LEN`](crate::MAX_ATTRIBUTE_KEY_LEN) bytes — and
//! values are 1 to
//! [`MAX_ATTRIBUTE_VALUE_LEN`](crate::MAX_ATTRIBUTE_VALUE_LEN) bytes.
//!
//! [`PushPlatform::as_str`]: crate::push::PushPlatform::as_str

/// The longest condition, in bytes.
pub const MAX_CONDITION_LEN: usize = 1_024;

/// The most distinct topics one condition may name, as FCM allows.
pub const MAX_CONDITION_TOPICS: usize = 5;

/// The most attribute and device predicates one condition may use,
/// together.
pub const MAX_CONDITION_ATTRIBUTES: usize = 10;

/// How deeply a condition may nest: a membership is depth 1, and every
/// `!`, `&&`, and `||` over it adds one.
pub const MAX_CONDITION_DEPTH: usize = 8;

/// A parsed condition: a boolean over the topics a person holds, the
/// attributes the app set for them, and one of their devices. Built by
/// [`Condition::parse`], evaluated by [`Condition::matches`],
/// [`Condition::matches_over`], and [`Condition::matches_on`] against the
/// person the moment asks about.
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
	/// `'name' in topics`: true when the person holds the topic.
	Topic(String),
	/// `user.<key> …`: true when the attribute the app set answers the
	/// test. False whenever the person holds no such attribute.
	Attribute { key: String, test: Test },
	/// `device.<key> …`: true when the device being judged reports a
	/// value that answers the test. False with no device, and when the
	/// device reported nothing for the key.
	Device { key: DeviceKey, test: Test },
	/// `left && right`.
	And(Box<Condition>, Box<Condition>),
	/// `left || right`.
	Or(Box<Condition>, Box<Condition>),
	/// `!inner`.
	Not(Box<Condition>),
}

/// What an attribute or device predicate asks of the value it reads.
#[derive(Clone, Debug, PartialEq)]
pub enum Test {
	/// `== 'value'`.
	Equals(String),
	/// `!= 'value'`.
	NotEquals(String),
	/// `in ['a', 'b']`.
	AnyOf(Vec<String>),
	/// `has(user.<key>)` or `has(device.<key>)`.
	Has,
	/// A numeric comparison: true only when the held value parses as a
	/// finite number. The literal was checked finite at parse time.
	Numeric { op: NumericOp, bound: f64 },
	/// A version comparison, `device.app_version`'s ordering: true only
	/// when the held value reads as a version under [`compare_versions`]'s
	/// rule. The literal was checked to read as one at parse time.
	Version { op: NumericOp, bound: String },
}

/// The ordering operators a predicate can compare with: numerically for
/// an attribute, as versions for `device.app_version`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericOp {
	Less,
	LessEq,
	Greater,
	GreaterEq,
}

impl Test {
	/// Whether the test holds for the attribute value a person holds, or
	/// `None`: absent answers false to every test.
	#[must_use]
	pub fn holds(&self, value: Option<&str>) -> bool {
		let Some(value) = value else {
			return false;
		};

		match self {
			Test::Has => true,
			Test::Equals(want) => value == want,
			Test::NotEquals(want) => value != want,
			Test::AnyOf(wanted) => wanted.iter().any(|want| want == value),
			Test::Numeric { op, bound } => {
				value
					.parse::<f64>()
					.ok()
					.filter(|number| number.is_finite())
					.is_some_and(|number| op.holds(number, *bound))
			}
			Test::Version { op, bound } => {
				compare_versions(value, bound).is_some_and(|ordering| op.orders(ordering))
			}
		}
	}
}

impl NumericOp {
	/// Whether `number op bound` holds. Both sides are finite.
	#[must_use]
	pub fn holds(self, number: f64, bound: f64) -> bool {
		match self {
			NumericOp::Less => number < bound,
			NumericOp::LessEq => number <= bound,
			NumericOp::Greater => number > bound,
			NumericOp::GreaterEq => number >= bound,
		}
	}

	/// Whether `held op bound` holds, given how the held value orders
	/// against the bound.
	#[must_use]
	pub fn orders(self, ordering: std::cmp::Ordering) -> bool {
		match self {
			NumericOp::Less => ordering.is_lt(),
			NumericOp::LessEq => ordering.is_le(),
			NumericOp::Greater => ordering.is_gt(),
			NumericOp::GreaterEq => ordering.is_ge(),
		}
	}
}

/// What a `device.` predicate can ask about. Each is something an
/// enrolled endpoint reports about itself (see
/// [`DeviceDetails`](crate::push::DeviceDetails)), or its platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKey {
	/// `device.platform`: the push service the endpoint speaks to, as
	/// [`PushPlatform::as_str`](crate::push::PushPlatform::as_str) names
	/// it — `apns` (iOS and macOS), `fcm` (Android), or `web_push`.
	Platform,
	/// `device.locale`: the full BCP 47 tag, such as `en-GB`. Compared
	/// without regard to case.
	Locale,
	/// `device.language`: the locale's primary subtag, lower case — `fr`
	/// for `fr-CA`. Compared without regard to case.
	Language,
	/// `device.app_version`: compared as a version (see
	/// [`compare_versions`]).
	AppVersion,
	/// `device.time_zone`: the IANA name, such as `Europe/London`.
	TimeZone,
}

impl DeviceKey {
	/// Every key a condition can name, in the order errors list them.
	pub const ALL: [DeviceKey; 5] = [
		DeviceKey::Platform,
		DeviceKey::Locale,
		DeviceKey::Language,
		DeviceKey::AppVersion,
		DeviceKey::TimeZone,
	];

	/// The key's name after `device.`.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			DeviceKey::Platform => "platform",
			DeviceKey::Locale => "locale",
			DeviceKey::Language => "language",
			DeviceKey::AppVersion => "app_version",
			DeviceKey::TimeZone => "time_zone",
		}
	}

	/// The key a name after `device.` stands for, or `None` for anything
	/// else.
	#[must_use]
	pub fn from_name(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|key| key.as_str() == name)
	}

	/// The value `device` reports for this key, or `None` when it reported
	/// nothing.
	#[must_use]
	pub fn of<'a>(self, device: &Device<'a>) -> Option<&'a str> {
		match self {
			DeviceKey::Platform => Some(device.platform.as_str()),
			DeviceKey::Locale => device.locale,
			DeviceKey::Language => device.locale.and_then(crate::push::language_of),
			DeviceKey::AppVersion => device.app_version,
			DeviceKey::TimeZone => device.time_zone,
		}
	}

	/// Whether `test` holds for what `device` reports, or for no device at
	/// all: no device, and a device that reported nothing for the key,
	/// answer false to every test, the way an absent attribute does.
	fn holds(self, test: &Test, device: Option<&Device<'_>>) -> bool {
		let Some(value) = device.and_then(|device| self.of(device)) else {
			return false;
		};

		match (self, test) {
			(_, Test::Has) => true,
			(DeviceKey::AppVersion, Test::Equals(want)) => same_version(value, want),
			(DeviceKey::AppVersion, Test::NotEquals(want)) => !same_version(value, want),
			(DeviceKey::AppVersion, Test::AnyOf(wanted)) => {
				wanted.iter().any(|want| same_version(value, want))
			}
			(DeviceKey::Locale | DeviceKey::Language, Test::Equals(want)) => {
				value.eq_ignore_ascii_case(want)
			}
			(DeviceKey::Locale | DeviceKey::Language, Test::NotEquals(want)) => {
				!value.eq_ignore_ascii_case(want)
			}
			(DeviceKey::Locale | DeviceKey::Language, Test::AnyOf(wanted)) => {
				wanted.iter().any(|want| value.eq_ignore_ascii_case(want))
			}
			(_, test) => test.holds(Some(value)),
		}
	}
}

/// The `device.` keys, as a sentence's list: what an unknown key's error
/// names.
fn device_keys_listed() -> String {
	let names: Vec<String> = DeviceKey::ALL
		.iter()
		.map(|key| format!("`device.{}`", key.as_str()))
		.collect();
	let (last, rest) = names.split_last().expect("there are keys");

	format!("{}, and {last}", rest.join(", "))
}

/// One device, as a condition's `device.` predicates read it: its
/// platform and what it reported about itself. Borrowed, so a stored
/// endpoint can be judged without copying its details.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device<'a> {
	/// The push service the device's endpoint speaks to.
	pub platform: crate::push::PushPlatform,
	/// Its preferred locale, a BCP 47 tag, when it reported one.
	pub locale: Option<&'a str>,
	/// Its IANA time zone name, when it reported one.
	pub time_zone: Option<&'a str>,
	/// The app's version on it, when it reported one.
	pub app_version: Option<&'a str>,
}

impl<'a> Device<'a> {
	/// A device on `platform` that reported `details`.
	#[must_use]
	pub fn new(
		platform: crate::push::PushPlatform,
		details: &'a crate::push::DeviceDetails,
	) -> Self {
		Self {
			platform,
			locale: details.locale.as_deref(),
			time_zone: details.time_zone.as_deref(),
			app_version: details.app_version.as_deref(),
		}
	}
}

/// How two app versions order, or `None` when either doesn't read as a
/// version. The rule, exactly:
///
/// 1. Everything from the first `-` or `+` onward is ignored: `2.1.0-beta` and
///    `2.1.0+812` read as `2.1.0`.
/// 2. What's left is split on `.`, and every part must be one or more ASCII
///    digits — otherwise the text isn't a version. So the empty string, `beta`,
///    `v2.1`, `2..1`, `2.1.`, and ` 2.1` aren't versions.
/// 3. The parts compare as whole numbers, left to right, and a missing part
///    counts as 0. So `2.10 > 2.9`, `2.1 == 2.1.0`, and `2.01 == 2.1`.
///
/// A condition's `device.app_version` uses this for `<`, `<=`, `>`, `>=`
/// — which answer false when the held value isn't a version — and for
/// `==`, `!=`, and `in`, which fall back to comparing the text exactly
/// when either side isn't a version.
#[must_use]
pub fn compare_versions(left: &str, right: &str) -> Option<std::cmp::Ordering> {
	let left = version_parts(left)?;
	let right = version_parts(right)?;

	for index in 0..left.len().max(right.len()) {
		let one = left.get(index).copied().unwrap_or("");
		let two = right.get(index).copied().unwrap_or("");
		// Leading zeros are gone, so a longer run of digits is a larger
		// number, and equal lengths order as their digits do — however
		// many digits there are.
		let ordering = one.len().cmp(&two.len()).then_with(|| one.cmp(two));

		if ordering.is_ne() {
			return Some(ordering);
		}
	}

	Some(std::cmp::Ordering::Equal)
}

/// A version's numeric parts with their leading zeros dropped — `0`
/// becomes the empty string, which is how a missing part reads too — or
/// `None` when the text isn't a version.
fn version_parts(text: &str) -> Option<Vec<&str>> {
	let core = text.split(['-', '+']).next().unwrap_or("");

	if core.is_empty() {
		return None;
	}

	core.split('.')
		.map(|part| {
			(!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
				.then(|| part.trim_start_matches('0'))
		})
		.collect()
}

/// `==` for app versions: equal as versions when both read as one,
/// otherwise equal as text.
fn same_version(held: &str, want: &str) -> bool {
	compare_versions(held, want).map_or(held == want, std::cmp::Ordering::is_eq)
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
	/// Parses and checks a condition: the grammar, the topic names and
	/// attribute keys, the bounds, and the anchoring rule above. The same
	/// text always parses to the same tree, and a condition that passed
	/// once can be stored and parsed again by the service's fanout.
	///
	/// # Errors
	///
	/// [`ConditionError`], with the byte offset the parser gave up at:
	/// text past the length bound, a topic name outside FCM's charset, a
	/// malformed attribute predicate, a missing `in topics`, unbalanced
	/// parentheses, a stray token, more than [`MAX_CONDITION_TOPICS`]
	/// distinct topics or [`MAX_CONDITION_ATTRIBUTES`] attribute
	/// predicates, nesting past [`MAX_CONDITION_DEPTH`], or a condition
	/// that holds when nobody holds any topic or attribute.
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
			predicates: 0,
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

		// Anchored: a person with no topics, no attributes, and no device
		// must not match, or the condition would be an everyone audience
		// in disguise — the registry's own target says that plainly.
		if parsed.matches(|_| false) {
			return Err(ConditionError {
				message: "a condition must not match a person who holds no topics and no \
				          attributes on no device: name at least one topic, attribute, or device \
				          detail it requires"
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

	/// The distinct attribute keys the condition's predicates read, in
	/// order of first appearance. At most [`MAX_CONDITION_ATTRIBUTES`]
	/// once parsed, each key once however many predicates read it.
	#[must_use]
	pub fn attribute_keys(&self) -> Vec<&str> {
		let mut keys = Vec::new();
		collect_keys(self, &mut keys);
		keys
	}

	/// Whether any predicate asks about a device. A condition that doesn't
	/// is judged once per person, and every endpoint of a matching person
	/// gets the push; one that does is judged per device as well.
	#[must_use]
	pub fn mentions_device(&self) -> bool {
		match self {
			Condition::Device { .. } => true,
			Condition::Topic(_) | Condition::Attribute { .. } => false,
			Condition::And(left, right) | Condition::Or(left, right) => {
				left.mentions_device() || right.mentions_device()
			}
			Condition::Not(inner) => inner.mentions_device(),
		}
	}

	/// Whether the condition holds for a person whose memberships `holds`
	/// answers: called with the topic names the condition names, never
	/// any other. The person is taken to hold no attributes at all, and
	/// no device, so every attribute and device predicate answers false.
	#[must_use]
	pub fn matches(&self, holds: impl Fn(&str) -> bool) -> bool {
		self.matches_over(holds, |_| None)
	}

	/// Whether the condition holds for a person whose memberships `holds`
	/// answers and whose attributes `attribute` answers: called with the
	/// topic names and attribute keys the condition names, never any
	/// other, with `None` for an attribute the person doesn't hold. No
	/// device is judged, so every device predicate answers false.
	#[must_use]
	pub fn matches_over<'k>(
		&'k self,
		holds: impl Fn(&str) -> bool,
		attribute: impl Fn(&'k str) -> Option<&'k str>,
	) -> bool {
		self.matches_on(holds, attribute, None)
	}

	/// [`Condition::matches_over`], judged on one of the person's devices:
	/// `device` is what its `device.` predicates read, and `None` judges the
	/// person with no device at all, where every device predicate answers
	/// false — the way an absent attribute does.
	///
	/// A send asks this once with no device and once per live endpoint of
	/// the person: they are in the audience when any of those holds, and
	/// the push goes to the endpoints it holds for.
	#[must_use]
	pub fn matches_on<'k>(
		&'k self,
		holds: impl Fn(&str) -> bool,
		attribute: impl Fn(&'k str) -> Option<&'k str>,
		device: Option<Device<'_>>,
	) -> bool {
		self.matches_with(&holds, &attribute, device.as_ref())
	}

	/// [`Condition::matches_on`] over trait objects, so the recursion
	/// doesn't build a fresh closure type per level.
	fn matches_with<'k>(
		&'k self,
		holds: &dyn Fn(&str) -> bool,
		attribute: &dyn Fn(&'k str) -> Option<&'k str>,
		device: Option<&Device<'_>>,
	) -> bool {
		match self {
			Condition::Topic(topic) => holds(topic),
			Condition::Attribute { key, test } => test.holds(attribute(key)),
			Condition::Device { key, test } => key.holds(test, device),
			Condition::And(left, right) => {
				left.matches_with(holds, attribute, device)
					&& right.matches_with(holds, attribute, device)
			}
			Condition::Or(left, right) => {
				left.matches_with(holds, attribute, device)
					|| right.matches_with(holds, attribute, device)
			}
			Condition::Not(inner) => !inner.matches_with(holds, attribute, device),
		}
	}

	/// Whether the condition is false for every person who holds none of
	/// the topics it names, whatever attributes and devices they hold. Only
	/// such a condition can be walked over the topic index's subscribers
	/// alone; anything else may match a person the index has never seen, so
	/// its audience is the person registry — the one a condition of device
	/// predicates alone walks, like everyone.
	#[must_use]
	pub fn topic_anchored(&self) -> bool {
		matches!(self.sans_topics(), Tri::False)
	}

	/// The condition's truth for a person who holds none of the named
	/// topics, with their attributes unknown: definitely [`Tri::False`],
	/// or [`Tri::Unknown`] — true for some attributes, or definitely
	/// true, which only an everyone audience in disguise manages, and
	/// [`Condition::parse`] refuses those.
	fn sans_topics(&self) -> Tri {
		match self {
			Condition::Topic(_) => Tri::False,
			// An attribute or device predicate may hold whatever the
			// person holds; `!false` would be true for every topic-less
			// person, which is the disguise parse refuses — all stay
			// unknown.
			Condition::Attribute { .. } | Condition::Device { .. } | Condition::Not(_) => {
				Tri::Unknown
			}
			Condition::And(left, right) => {
				match (left.sans_topics(), right.sans_topics()) {
					(Tri::False, _) | (_, Tri::False) => Tri::False,
					(Tri::Unknown, Tri::Unknown) => Tri::Unknown,
				}
			}
			Condition::Or(left, right) => {
				match (left.sans_topics(), right.sans_topics()) {
					(Tri::False, Tri::False) => Tri::False,
					_ => Tri::Unknown,
				}
			}
		}
	}

	/// The tree's depth: a membership is 1, and every operator over one
	/// adds a level.
	fn depth(&self) -> usize {
		match self {
			Condition::Topic(_) | Condition::Attribute { .. } | Condition::Device { .. } => 1,
			Condition::And(left, right) | Condition::Or(left, right) => {
				left.depth().max(right.depth()) + 1
			}
			Condition::Not(inner) => inner.depth() + 1,
		}
	}
}

/// What a condition definitely is for a topic-less person: false, or
/// unknown — true under some attributes.
enum Tri {
	False,
	Unknown,
}

/// Collects the distinct topics a tree names, in order.
fn collect_topics<'a>(condition: &'a Condition, topics: &mut Vec<&'a str>) {
	match condition {
		Condition::Topic(topic) => {
			if !topics.contains(&topic.as_str()) {
				topics.push(topic.as_str());
			}
		}
		Condition::Attribute { .. } | Condition::Device { .. } => {}
		Condition::And(left, right) | Condition::Or(left, right) => {
			collect_topics(left, topics);
			collect_topics(right, topics);
		}
		Condition::Not(inner) => collect_topics(inner, topics),
	}
}

/// Collects the distinct attribute keys a tree's predicates read, in
/// order.
fn collect_keys<'a>(condition: &'a Condition, keys: &mut Vec<&'a str>) {
	match condition {
		Condition::Topic(_) | Condition::Device { .. } => {}
		Condition::Attribute { key, .. } => {
			if !keys.contains(&key.as_str()) {
				keys.push(key.as_str());
			}
		}
		Condition::And(left, right) | Condition::Or(left, right) => {
			collect_keys(left, keys);
			collect_keys(right, keys);
		}
		Condition::Not(inner) => collect_keys(inner, keys),
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

/// The recursive-descent parser. Tracks the position in the text, the
/// distinct topics and attribute predicates seen, so the bounds can name
/// their offsets.
struct Parser<'a> {
	text: &'a [u8],
	pos: usize,
	topics: Vec<(&'a str, usize)>,
	predicates: usize,
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

	/// `!`, a parenthesized expression, or a predicate.
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

		self.predicate(start)
	}

	/// A topic membership or an attribute predicate.
	fn predicate(&mut self, start: usize) -> Result<Condition, ConditionError> {
		if self.peek() == Some(b'\'') {
			return self.membership();
		}

		if self.eat_word(b"has") {
			return self.has_predicate(start);
		}

		if self.eat_word(b"user") {
			return self.attribute_predicate(start);
		}

		if self.eat_word(b"device") {
			return self.device_predicate(start);
		}

		Err(ConditionError {
			message: format!(
				"expected a topic in quotes, `user.<key>`, `device.<key>`, or `has(…)`, found {}",
				self.here()
			),
			offset: self.pos,
		})
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

	/// `has(user.<key>)` or `has(device.<key>)`.
	fn has_predicate(&mut self, start: usize) -> Result<Condition, ConditionError> {
		self.skip_whitespace();

		if !self.eat(b"(") {
			return Err(ConditionError {
				message: "expected `(` after `has`".into(),
				offset: self.pos,
			});
		}

		self.skip_whitespace();

		let device = if self.eat_word(b"user") {
			false
		} else if self.eat_word(b"device") {
			true
		} else {
			return Err(ConditionError {
				message: format!(
					"expected `user.<key>` or `device.<key>` inside `has(…)`, found {}",
					self.here()
				),
				offset: self.pos,
			});
		};

		if !self.eat(b".") {
			return Err(ConditionError {
				message: format!(
					"expected `.` after `{}`",
					if device { "device" } else { "user" }
				),
				offset: self.pos,
			});
		}

		let predicate = if device {
			Condition::Device {
				key: self.device_key()?,
				test: Test::Has,
			}
		} else {
			Condition::Attribute {
				key: self.attribute_key()?,
				test: Test::Has,
			}
		};
		self.skip_whitespace();

		if !self.eat(b")") {
			return Err(ConditionError {
				message: "an unclosed `has(…)`: expected `)` here".into(),
				offset: self.pos,
			});
		}

		self.count_predicate(start)?;

		Ok(predicate)
	}

	/// `device.<key> == 'v'`, with every operator the grammar allows:
	/// `device.app_version`'s orderings compare as versions, and
	/// `device.platform`'s values must name a platform.
	fn device_predicate(&mut self, start: usize) -> Result<Condition, ConditionError> {
		if !self.eat(b".") {
			return Err(ConditionError {
				message: "expected `.` after `device`".into(),
				offset: self.pos,
			});
		}

		let key = self.device_key()?;
		self.skip_whitespace();

		let ordering = |parser: &mut Self, op: NumericOp| {
			if key == DeviceKey::AppVersion {
				parser.version(op)
			} else {
				parser.numeric(op)
			}
		};

		let test = if self.eat(b"==") {
			Test::Equals(self.device_value(key)?)
		} else if self.eat(b"!=") {
			Test::NotEquals(self.device_value(key)?)
		} else if self.eat(b"<=") {
			ordering(self, NumericOp::LessEq)?
		} else if self.eat(b">=") {
			ordering(self, NumericOp::GreaterEq)?
		} else if self.eat(b"<") {
			ordering(self, NumericOp::Less)?
		} else if self.eat(b">") {
			ordering(self, NumericOp::Greater)?
		} else if self.eat_word(b"in") {
			let values = self.values()?;

			for (value, offset) in &values {
				check_device_value(key, value, *offset)?;
			}

			Test::AnyOf(values.into_iter().map(|(value, _)| value).collect())
		} else {
			return Err(ConditionError {
				message: "expected an operator after `device.<key>`: `==`, `!=`, `in`, `<`, `<=`, \
				          `>`, or `>=`"
					.into(),
				offset: self.pos,
			});
		};

		self.count_predicate(start)?;

		Ok(Condition::Device { key, test })
	}

	/// The `<key>` of a `device.<key>` reference: one of [`DeviceKey`]'s
	/// names, and nothing else.
	fn device_key(&mut self) -> Result<DeviceKey, ConditionError> {
		let start = self.pos;

		while self
			.peek()
			.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
		{
			self.pos += 1;
		}

		let name = std::str::from_utf8(&self.text[start..self.pos]).expect("the name is ASCII");

		DeviceKey::from_name(name).ok_or_else(|| {
			ConditionError {
				message: format!(
					"`device.{name}` isn't a device detail: a condition can ask about {}",
					device_keys_listed()
				),
				offset: start,
			}
		})
	}

	/// A quoted value a `device.<key>` predicate compares with, checked
	/// against the key: a platform must be one there is.
	fn device_value(&mut self, key: DeviceKey) -> Result<String, ConditionError> {
		let (value, offset) = self.value()?;
		check_device_value(key, &value, offset)?;

		Ok(value)
	}

	/// A version comparison's quoted literal, which must read as a version
	/// under [`compare_versions`]'s rule: a bound that isn't one would make
	/// the comparison false for everyone.
	fn version(&mut self, op: NumericOp) -> Result<Test, ConditionError> {
		let (literal, literal_start) = self.value()?;

		if compare_versions(&literal, &literal).is_none() {
			return Err(ConditionError {
				message: format!(
					"`device.app_version` compares with a version in quotes — dot-separated \
					 numbers like '2.1.0' — and `{literal}` isn't one"
				),
				offset: literal_start,
			});
		}

		Ok(Test::Version { op, bound: literal })
	}

	/// `user.<key> == 'v'`, with every operator the grammar allows.
	fn attribute_predicate(&mut self, start: usize) -> Result<Condition, ConditionError> {
		if !self.eat(b".") {
			return Err(ConditionError {
				message: "expected `.` after `user`".into(),
				offset: self.pos,
			});
		}

		let key = self.attribute_key()?;
		self.skip_whitespace();

		let test = if self.eat(b"==") {
			Test::Equals(self.value()?.0)
		} else if self.eat(b"!=") {
			Test::NotEquals(self.value()?.0)
		} else if self.eat(b"<=") {
			self.numeric(NumericOp::LessEq)?
		} else if self.eat(b">=") {
			self.numeric(NumericOp::GreaterEq)?
		} else if self.eat(b"<") {
			self.numeric(NumericOp::Less)?
		} else if self.eat(b">") {
			self.numeric(NumericOp::Greater)?
		} else if self.eat_word(b"in") {
			self.list()?
		} else {
			return Err(ConditionError {
				message: "expected an operator after `user.<key>`: `==`, `!=`, `in`, `<`, `<=`, \
				          `>`, or `>=`"
					.into(),
				offset: self.pos,
			});
		};

		self.count_predicate(start)?;

		Ok(Condition::Attribute { key, test })
	}

	/// The `<key>` of a `user.<key>` reference: `[a-z0-9_]{1,64}`.
	fn attribute_key(&mut self) -> Result<String, ConditionError> {
		let start = self.pos;

		while self
			.peek()
			.is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
		{
			self.pos += 1;
		}

		let end = self.pos;
		let len = end - start;

		if len == 0 {
			return Err(ConditionError {
				message: "an attribute key is 1 to 64 bytes of `a-z`, `0-9`, or `_`, as in \
				          `user.plan`"
					.into(),
				offset: start,
			});
		}

		if len > crate::MAX_ATTRIBUTE_KEY_LEN {
			return Err(ConditionError {
				message: format!(
					"an attribute key is at most {} bytes",
					crate::MAX_ATTRIBUTE_KEY_LEN
				),
				offset: start,
			});
		}

		Ok(std::str::from_utf8(&self.text[start..end])
			.expect("the key is ASCII")
			.to_owned())
	}

	/// A quoted value: `'…'`, at most
	/// [`MAX_ATTRIBUTE_VALUE_LEN`](crate::MAX_ATTRIBUTE_VALUE_LEN) bytes,
	/// no whitespace inside. Answers the value and where it starts.
	fn value(&mut self) -> Result<(String, usize), ConditionError> {
		self.skip_whitespace();

		if !self.eat(b"'") {
			return Err(ConditionError {
				message: format!("expected a value in quotes, found {}", self.here()),
				offset: self.pos,
			});
		}

		let value_start = self.pos;

		while self
			.peek()
			.is_some_and(|byte| byte != b'\'' && !byte.is_ascii_whitespace())
		{
			self.pos += 1;
		}

		let value_end = self.pos;

		if !self.eat(b"'") {
			return Err(ConditionError {
				message: "an unclosed value: expected `'` here".into(),
				offset: self.pos,
			});
		}

		if value_end - value_start > crate::MAX_ATTRIBUTE_VALUE_LEN {
			return Err(ConditionError {
				message: format!(
					"an attribute value is at most {} bytes",
					crate::MAX_ATTRIBUTE_VALUE_LEN
				),
				offset: value_start,
			});
		}

		let value = std::str::from_utf8(&self.text[value_start..value_end]).map_err(|_| {
			ConditionError {
				message: "an attribute value is one UTF-8 string".into(),
				offset: value_start,
			}
		})?;

		Ok((value.to_owned(), value_start))
	}

	/// A numeric comparison's quoted literal, which must be a finite
	/// number: `NaN` and `inf` are not numbers here.
	fn numeric(&mut self, op: NumericOp) -> Result<Test, ConditionError> {
		let (literal, literal_start) = self.value()?;
		let bound = literal.parse::<f64>().map_err(|_| {
			ConditionError {
				message: format!(
					"a numeric comparison's value is a finite number in quotes, and `{literal}` \
					 isn't one"
				),
				offset: literal_start,
			}
		})?;

		if !bound.is_finite() {
			return Err(ConditionError {
				message: format!(
					"a numeric comparison's value is a finite number, and `{literal}` isn't"
				),
				offset: literal_start,
			});
		}

		Ok(Test::Numeric { op, bound })
	}

	/// `in ['a', 'b']`: one or more quoted values in brackets.
	fn list(&mut self) -> Result<Test, ConditionError> {
		Ok(Test::AnyOf(
			self.values()?.into_iter().map(|(value, _)| value).collect(),
		))
	}

	/// The bracketed values of an `in`, each with where it starts.
	fn values(&mut self) -> Result<Vec<(String, usize)>, ConditionError> {
		self.skip_whitespace();

		if !self.eat(b"[") {
			return Err(ConditionError {
				message: format!("expected `[` after `in`, found {}", self.here()),
				offset: self.pos,
			});
		}

		let mut values = vec![self.value()?];

		loop {
			self.skip_whitespace();

			if self.eat(b"]") {
				break;
			}

			if !self.eat(b",") {
				return Err(ConditionError {
					message: "expected `,` or `]` between the values of `in`".into(),
					offset: self.pos,
				});
			}

			self.skip_whitespace();
			values.push(self.value()?);
		}

		Ok(values)
	}

	/// Counts one more attribute predicate against the bound.
	fn count_predicate(&mut self, offset: usize) -> Result<(), ConditionError> {
		self.predicates += 1;

		if self.predicates > MAX_CONDITION_ATTRIBUTES {
			return Err(ConditionError {
				message: format!(
					"a condition uses at most {MAX_CONDITION_ATTRIBUTES} attribute predicates"
				),
				offset,
			});
		}

		Ok(())
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

	/// Whether the parser is on the word `bytes`, which it then consumes:
	/// a word ends at a character that can't continue one, so the `user`
	/// of `user.plan` matches and the `user` of `username` doesn't.
	fn eat_word(&mut self, bytes: &[u8]) -> bool {
		if !self.eat(bytes) {
			return false;
		}

		if self
			.peek()
			.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
		{
			// Not a whole word: give the bytes back.
			self.pos -= bytes.len();

			return false;
		}

		true
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

/// Checks a value a `device.<key>` predicate compares with for equality:
/// `device.platform` takes only the names a platform has, because any
/// other value would quietly match nobody.
///
/// # Errors
///
/// A [`ConditionError`] at `offset`, naming the platforms there are.
fn check_device_value(key: DeviceKey, value: &str, offset: usize) -> Result<(), ConditionError> {
	if key == DeviceKey::Platform && crate::push::PushPlatform::from_str_value(value).is_none() {
		return Err(ConditionError {
			message: format!(
				"`device.platform` is one of 'apns' (iOS and macOS), 'fcm' (Android), or \
				 'web_push', and `{value}` isn't one"
			),
			offset,
		});
	}

	Ok(())
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
		b'u' => "an attribute".to_owned(),
		b'd' => "a device predicate".to_owned(),
		b'h' => "`has`".to_owned(),
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

	/// The attributes a person holds, as a lookup.
	fn attrs<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&'a str) -> Option<&'a str> + 'a {
		move |key| {
			pairs
				.iter()
				.find(|(held, _)| *held == key)
				.map(|(_, value)| *value)
		}
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

		let attributes = Condition::parse("user.plan  ==\t'pro' &&\nhas( user.beta )").unwrap();
		assert!(attributes.matches_over(holding(&[]), attrs(&[("plan", "pro"), ("beta", "1")])));
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
			("(!", 2),
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

	/// The attribute predicates' errors, at the bytes they gave up at.
	#[test]
	fn attribute_errors_carry_byte_offsets() {
		let cases: &[(&str, usize)] = &[
			("user", 4),
			("user.", 5),
			("user.Plan == 'pro'", 5),
			("user.plan", 9),
			("user.plan ==", 12),
			("user.plan == ", 13),
			("user.plan == 'pro", 17),
			("user.plan ~ 'pro'", 10),
			("user.plan in 'pro'", 13),
			("user.plan in", 12),
			("user.plan in [", 14),
			("user.plan in ['pro'", 19),
			("user.plan in []", 14),
			("user.plan in ['pro' 'free']", 20),
			("user.plan < 'ten'", 13),
			("user.plan < 'NaN'", 13),
			("user.plan < 'inf'", 13),
			("user.plan < '1e400'", 13),
			("has(user.plan", 13),
			("has user.plan", 4),
			("has(user.plan) &&", 17),
			("user.plan == 'pro' &&", 21),
			("username == 'x'", 0),
		];

		for (text, offset) in cases {
			let error = Condition::parse(text).expect_err(text);
			assert_eq!(
				error.offset, *offset,
				"{text:?}: the error points where the parser gave up: {error}"
			);
		}
	}

	/// Every attribute predicate parses, matches, and refuses as its
	/// semantics say.
	#[test]
	fn attribute_predicates_parse_and_evaluate() {
		let equals = Condition::parse("user.plan == 'pro'").unwrap();
		assert!(equals.matches_over(holding(&[]), attrs(&[("plan", "pro")])));
		assert!(!equals.matches_over(holding(&[]), attrs(&[("plan", "free")])));
		assert!(!equals.matches_over(holding(&[]), attrs(&[])), "absent");

		let not_equals = Condition::parse("user.plan != 'pro'").unwrap();
		assert!(not_equals.matches_over(holding(&[]), attrs(&[("plan", "free")])));
		assert!(!not_equals.matches_over(holding(&[]), attrs(&[("plan", "pro")])));
		assert!(
			!not_equals.matches_over(holding(&[]), attrs(&[])),
			"absent answers false, even to `!=`"
		);

		let any_of = Condition::parse("user.country in ['de', 'at', 'ch']").unwrap();
		assert!(any_of.matches_over(holding(&[]), attrs(&[("country", "at")])));
		assert!(!any_of.matches_over(holding(&[]), attrs(&[("country", "fr")])));
		assert!(!any_of.matches_over(holding(&[]), attrs(&[])));

		let has = Condition::parse("has(user.beta)").unwrap();
		assert!(has.matches_over(holding(&[]), attrs(&[("beta", "yes")])));
		assert!(!has.matches_over(holding(&[]), attrs(&[])));
		assert!(
			has.matches_over(holding(&[]), attrs(&[("beta", "")])),
			"any value"
		);

		// Absent attributes answer false everywhere, including under a
		// negation, so a predicate never matches for lack.
		let not_has = Condition::parse("!has(user.beta) && 'a' in topics").unwrap();
		assert!(not_has.matches_over(holding(&["a"]), attrs(&[])));
		assert!(!not_has.matches_over(holding(&["a"]), attrs(&[("beta", "no")])));
	}

	/// Numeric predicates compare as numbers, and only as numbers: a
	/// value that doesn't parse as a finite number answers false, and
	/// literals that aren't finite numbers are refused at parse time.
	#[test]
	fn numeric_predicates_compare_as_numbers() {
		for (text, held, matches) in [
			("user.spend >= '1000'", Some("1000"), true),
			("user.spend >= '1000'", Some("999.99"), false),
			("user.spend > '1000'", Some("1000"), false),
			("user.spend > '1000'", Some("1000.5"), true),
			("user.spend < '10'", Some("9"), true),
			("user.spend <= '10'", Some("10"), true),
			("user.spend > '9'", Some("10"), true),
			// String order would say "10" < "9": numbers, not text.
			("user.spend > '9'", Some("10"), true),
			("user.spend < '10'", Some("2"), true),
			// A non-numeric held value answers false, both ways.
			("user.spend >= '1000'", Some("lots"), false),
			("user.spend < '1000'", Some("lots"), false),
			("user.spend >= '1000'", Some(""), false),
			// An absent attribute answers false, like every predicate.
			("user.spend >= '0'", None, false),
		] {
			let parsed = Condition::parse(text).unwrap();
			let spend: Vec<(&str, &str)> = held.map_or(Vec::new(), |spend| vec![("spend", spend)]);
			let lookup = attrs(&spend);

			assert_eq!(
				parsed.matches_over(holding(&[]), lookup),
				matches,
				"{text} against spend={held:?}"
			);
		}

		// Literals that aren't finite numbers are refused, NaN and inf
		// included — Rust's parser accepts both as numbers.
		for bad in [
			"user.spend > 'NaN'",
			"user.spend > 'inf'",
			"user.spend > '-infinity'",
			"user.spend > '1e400'",
			"user.spend > 'lots'",
			"user.spend > ''",
		] {
			let error = Condition::parse(bad).expect_err(bad);
			assert!(
				error.message.contains("finite"),
				"{bad}: the refusal names the rule: {error}"
			);
		}

		// Negative and exponent-form literals are finite numbers.
		Condition::parse("user.delta >= '-1.5'").unwrap();
		Condition::parse("user.delta < '1e3'").unwrap();
	}

	/// Attributes mix with topics under the same operators, and the tree
	/// carries both.
	#[test]
	fn attributes_mix_with_topics() {
		let mixed = Condition::parse("'news.product' in topics && user.plan == 'pro'").unwrap();
		assert_eq!(mixed.topics(), vec!["news.product"]);
		assert_eq!(mixed.attribute_keys(), vec!["plan"]);
		assert!(mixed.matches_over(holding(&["news.product"]), attrs(&[("plan", "pro")])));
		assert!(!mixed.matches_over(holding(&["news.product"]), attrs(&[("plan", "free")])));
		assert!(!mixed.matches_over(holding(&[]), attrs(&[("plan", "pro")])));

		// The same key in two predicates is one key and two predicates.
		let twice = Condition::parse("user.plan == 'pro' || user.plan == 'enterprise'").unwrap();
		assert_eq!(twice.attribute_keys(), vec!["plan"]);

		// Precedence holds with attributes in the tree.
		let precedence =
			Condition::parse("user.beta == 'yes' || 'a' in topics && user.plan == 'pro'").unwrap();
		let expected = Condition::Or(
			Box::new(Condition::Attribute {
				key: "beta".into(),
				test: super::Test::Equals("yes".into()),
			}),
			Box::new(Condition::And(
				Box::new(Condition::Topic("a".into())),
				Box::new(Condition::Attribute {
					key: "plan".into(),
					test: super::Test::Equals("pro".into()),
				}),
			)),
		);
		assert_eq!(precedence, expected);
	}

	/// A condition anchored only by an attribute parses; one that holds
	/// for a person with no topics and no attributes is still refused.
	#[test]
	fn attribute_anchored_conditions_parse_and_everyone_disguises_refuse() {
		// Anchored by an attribute: nobody without attributes matches.
		let anchored = Condition::parse("user.plan == 'pro' && !('beta' in topics)").unwrap();
		assert!(!anchored.matches(holding(&[])), "no attributes: false");
		assert!(anchored.matches_over(holding(&[]), attrs(&[("plan", "pro")])));
		assert!(!anchored.matches_over(holding(&["beta"]), attrs(&[("plan", "pro")])));

		// `!=` alone is anchored too: absent answers false.
		Condition::parse("user.plan != 'pro'").expect("absent answers false");

		for text in [
			"!'a' in topics",
			"!('a' in topics && 'b' in topics)",
			"'a' in topics || !'b' in topics",
			"!('a' in topics) || !('b' in topics)",
			"!has(user.beta)",
			"!(user.plan == 'pro')",
			"user.plan == 'pro' || !('a' in topics)",
			"!('a' in topics) || user.plan == 'pro'",
		] {
			let error = Condition::parse(text).expect_err(text);
			assert!(
				error.message.contains("no topics"),
				"{text}: the refusal says why: {error}"
			);
		}

		// Requiring any topic or attribute, even beside negations, anchors it.
		Condition::parse("!'a' in topics && 'b' in topics").expect("anchored by `b`");
		Condition::parse("!'a' in topics && !('b' in topics || !'c' in topics)").unwrap();
		Condition::parse("user.plan == 'pro' || 'a' in topics").expect("anchored by both sides");
	}

	/// A condition that needs a topic to match is topic-anchored: the
	/// fanout can walk the topic index alone. One that can match without
	/// any of its topics walks the registry instead.
	#[test]
	fn topic_anchored_conditions_name_their_walk() {
		for text in [
			"'a' in topics",
			"'a' in topics && 'b' in topics",
			"'a' in topics && user.plan == 'pro'",
			"user.plan == 'pro' && 'a' in topics",
			"'a' in topics && !('b' in topics)",
			"user.plan == 'pro' && 'a' in topics && user.country == 'de'",
		] {
			let parsed = Condition::parse(text).expect(text);
			assert!(parsed.topic_anchored(), "{text}: false without its topics");
		}

		for text in [
			"user.plan == 'pro'",
			"user.plan == 'pro' || 'a' in topics",
			"'a' in topics || user.plan == 'pro'",
			"has(user.beta) || 'a' in topics",
			"user.plan == 'pro' && !('beta' in topics)",
			"user.plan == 'pro' && ('a' in topics || user.country == 'de')",
			"!('a' in topics) && user.plan == 'pro'",
			"user.plan == 'pro' || !('a' in topics) && user.beta == 'yes'",
		] {
			let parsed = Condition::parse(text).expect(text);
			assert!(
				!parsed.topic_anchored(),
				"{text}: may match without any of its topics"
			);
		}
	}

	/// The bounds: length, distinct topics, attribute predicates, value
	/// length, and nesting depth.
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

		// Ten attribute predicates fit when the tree stays inside the
		// depth bound — a balanced shape, not one long chain, which the
		// depth bound below refuses on its own; the eleventh doesn't fit
		// however it's written.
		let ten = "((user.k1 == 'v' && user.k2 == 'v') && (user.k3 == 'v' && user.k4 == 'v')) && \
		           ((user.k5 == 'v' && user.k6 == 'v') && (user.k7 == 'v' && user.k8 == 'v')) && \
		           user.k9 == 'v' && user.k10 == 'v'";
		Condition::parse(ten).expect("ten attribute predicates fit");

		let eleven = format!("{ten} && user.k11 == 'v'");
		let error = Condition::parse(&eleven).expect_err("eleven don't");
		assert!(
			error.message.contains("at most 10"),
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

		// Values are bounded, wherever they sit.
		let long_value = format!("user.plan == '{}'", "v".repeat(257));
		let error = Condition::parse(&long_value).expect_err("values are bounded");
		assert!(error.message.contains("at most 256"), "{error}");

		let longest_key = format!("user.{} == 'v'", "k".repeat(64));
		Condition::parse(&longest_key).expect("64-byte keys fit");

		let over_key = format!("user.{} == 'v'", "k".repeat(65));
		let error = Condition::parse(&over_key).expect_err("65-byte keys don't");
		assert!(error.message.contains("at most 64"), "{error}");
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

	/// A device on `platform` with the given details.
	fn device(
		platform: crate::push::PushPlatform,
		details: &crate::push::DeviceDetails,
	) -> super::Device<'_> {
		super::Device::new(platform, details)
	}

	/// Details with every field set.
	fn details(locale: &str, time_zone: &str, app_version: &str) -> crate::push::DeviceDetails {
		crate::push::DeviceDetails {
			locale: Some(locale.into()),
			time_zone: Some(time_zone.into()),
			app_version: Some(app_version.into()),
		}
	}

	/// Whether `text` holds on `device`, for a person with no topics and no
	/// attributes.
	fn on(text: &str, device: Option<super::Device<'_>>) -> bool {
		Condition::parse(text)
			.unwrap_or_else(|error| panic!("{text}: {error}"))
			.matches_on(holding(&[]), attrs(&[]), device)
	}

	/// Every device key, under every operator a `user.` predicate has.
	#[test]
	fn device_predicates_parse_and_evaluate_per_key_and_operator() {
		use crate::push::PushPlatform;

		let london = details("en-GB", "Europe/London", "2.1.0");
		let quebec = details("fr-CA", "America/Toronto", "1.9");
		let phone = device(PushPlatform::Apns, &london);
		let android = device(PushPlatform::Fcm, &quebec);

		for (text, on_phone, on_android) in [
			// platform: ==, !=, in, has.
			("device.platform == 'apns'", true, false),
			("device.platform != 'apns'", false, true),
			("device.platform in ['fcm', 'web_push']", false, true),
			("has(device.platform)", true, true),
			// locale: the full tag, without regard to case.
			("device.locale == 'en-GB'", true, false),
			("device.locale == 'en-gb'", true, false),
			("device.locale != 'en-GB'", false, true),
			("device.locale in ['fr-CA', 'fr-FR']", false, true),
			("has(device.locale)", true, true),
			// language: the primary subtag.
			("device.language == 'fr'", false, true),
			("device.language == 'FR'", false, true),
			("device.language != 'fr'", true, false),
			("device.language in ['en', 'de']", true, false),
			("has(device.language)", true, true),
			// time zone: the IANA name.
			("device.time_zone == 'Europe/London'", true, false),
			("device.time_zone != 'Europe/London'", false, true),
			(
				"device.time_zone in ['America/Toronto', 'America/Montreal']",
				false,
				true,
			),
			("has(device.time_zone)", true, true),
			// app version: as versions.
			("device.app_version == '2.1'", true, false),
			("device.app_version != '2.1'", false, true),
			("device.app_version in ['1.9.0', '3']", false, true),
			("device.app_version < '2.1'", false, true),
			("device.app_version <= '2.1'", true, true),
			("device.app_version > '2.0.9'", true, false),
			("device.app_version >= '1.9'", true, true),
			("has(device.app_version)", true, true),
		] {
			assert_eq!(on(text, Some(phone)), on_phone, "{text} on the phone");
			assert_eq!(on(text, Some(android)), on_android, "{text} on android");
		}
	}

	/// With no device — and on a device that reported nothing for a key —
	/// every device predicate answers the way an absent attribute does:
	/// false, `!=` and `in` included.
	#[test]
	fn no_device_answers_like_an_absent_attribute() {
		use crate::push::PushPlatform;

		let silent = crate::push::DeviceDetails::default();
		let browser = device(PushPlatform::WebPush, &silent);

		for text in [
			"device.platform == 'apns'",
			"device.platform != 'apns'",
			"device.platform in ['apns', 'fcm']",
			"has(device.platform)",
			"device.locale != 'en-GB'",
			"device.language in ['en']",
			"device.time_zone != 'UTC'",
			"device.app_version != '1'",
			"device.app_version < '9'",
			"has(device.app_version)",
		] {
			assert!(!on(text, None), "{text} with no device");

			let user = text.replace("device.", "user.");
			if !text.contains("platform") {
				// The same predicate over an absent attribute agrees.
				assert!(
					!Condition::parse(&user)
						.unwrap()
						.matches_over(holding(&[]), attrs(&[])),
					"{user}"
				);
			}
		}

		// A browser that reported nothing still has its platform, and
		// nothing else.
		assert!(on("device.platform == 'web_push'", Some(browser)));
		assert!(!on("has(device.locale)", Some(browser)));
		assert!(!on("device.locale != 'en-GB'", Some(browser)));
		assert!(!on("device.app_version != '1'", Some(browser)));

		// `matches` and `matches_over` judge no device.
		let parsed = Condition::parse("'a' in topics && !(device.platform == 'apns')").unwrap();
		assert!(parsed.matches(holding(&["a"])));
		assert!(parsed.matches_over(holding(&["a"]), attrs(&[])));
	}

	/// The version rule, at its edges: numeric parts, missing parts as 0,
	/// suffixes from `-` or `+` ignored, and anything else not a version —
	/// which makes an ordering false and an equality compare text.
	#[test]
	fn versions_compare_by_the_documented_rule() {
		use std::cmp::Ordering;

		use super::compare_versions;

		for (left, right, ordering) in [
			("2.10", "2.9", Some(Ordering::Greater)),
			("2.9", "2.10", Some(Ordering::Less)),
			("2.1", "2.1.0", Some(Ordering::Equal)),
			("2.1.0.0", "2.1", Some(Ordering::Equal)),
			("2.01", "2.1", Some(Ordering::Equal)),
			("2.1.0-beta", "2.1.0", Some(Ordering::Equal)),
			("2.1.0-beta", "2.1.1", Some(Ordering::Less)),
			("2.1.0+812", "2.1", Some(Ordering::Equal)),
			("2.1-rc.1+5", "2.1", Some(Ordering::Equal)),
			("10", "9.99.99", Some(Ordering::Greater)),
			("0", "0.0.0", Some(Ordering::Equal)),
			(
				"18446744073709551616.1",
				"18446744073709551615.9",
				Some(Ordering::Greater),
			),
			("", "1", None),
			("1", "", None),
			("beta", "1", None),
			("v2.1", "2.1", None),
			("2..1", "2.1", None),
			("2.1.", "2.1", None),
			(" 2.1", "2.1", None),
			("2.1 ", "2.1", None),
			("-beta", "1", None),
			("+1", "1", None),
			("2.x", "2", None),
		] {
			assert_eq!(
				compare_versions(left, right),
				ordering,
				"{left:?} against {right:?}"
			);
		}

		// Held values against a condition, every operator.
		for (held, text, matches) in [
			("2.10", "device.app_version > '2.9'", true),
			("2.1.0", "device.app_version == '2.1'", true),
			("2.1", "device.app_version == '2.1.0'", true),
			("2.1.0-beta", "device.app_version == '2.1'", true),
			("2.1.0-beta", "device.app_version < '2.1'", false),
			("2.1.0-beta", "device.app_version >= '2.1'", true),
			("2.0.9", "device.app_version < '2.1'", true),
			("2.1", "device.app_version != '2.1.0'", false),
			// Not versions: orderings are false both ways; equality
			// compares the text.
			("", "device.app_version < '9'", false),
			("", "device.app_version >= '0'", false),
			("garbage", "device.app_version < '9'", false),
			("garbage", "device.app_version > '0'", false),
			("garbage", "device.app_version == 'garbage'", true),
			("garbage", "device.app_version != 'garbage'", false),
			("garbage", "device.app_version != '2.1'", true),
			("nightly", "device.app_version in ['2.1', 'nightly']", true),
			("2.1.0", "device.app_version in ['2.0', '2.1']", true),
		] {
			let reported = crate::push::DeviceDetails {
				app_version: Some(held.into()),
				..crate::push::DeviceDetails::default()
			};
			assert_eq!(
				on(
					text,
					Some(device(crate::push::PushPlatform::Fcm, &reported))
				),
				matches,
				"{text} against {held:?}"
			);
		}

		// An ordering's literal must be a version, the way a numeric
		// comparison's must be a finite number.
		for bad in [
			"device.app_version < 'latest'",
			"device.app_version >= ''",
			"device.app_version > 'v2'",
			"device.app_version <= '2..1'",
		] {
			let error = Condition::parse(bad).expect_err(bad);
			assert!(error.message.contains("version"), "{bad}: {error}");
			assert_eq!(error.offset, bad.find('\'').unwrap() + 1, "{bad}: {error}");
		}

		// Equality takes any text: app versions are free text.
		Condition::parse("device.app_version == 'nightly'").expect("free text");
	}

	/// An unknown key is refused where it starts, with a message naming
	/// every key there is; a platform must be one there is.
	#[test]
	fn unknown_device_keys_and_platforms_are_refused() {
		for (text, offset) in [
			("device.os == 'ios'", 7),
			("device.Platform == 'apns'", 7),
			("device. == 'x'", 7),
			("has(device.model)", 11),
			("'a' in topics && device.region == 'GB'", 24),
		] {
			let error = Condition::parse(text).expect_err(text);
			assert_eq!(error.offset, offset, "{text}: {error}");
			for key in super::DeviceKey::ALL {
				assert!(
					error
						.message
						.contains(&format!("`device.{}`", key.as_str())),
					"{text}: the refusal names {}: {error}",
					key.as_str()
				);
			}
		}

		for (text, offset) in [
			("device.platform == 'ios'", 20),
			("device.platform != 'android'", 20),
			("device.platform in ['apns', 'web']", 29),
		] {
			let error = Condition::parse(text).expect_err(text);
			assert_eq!(error.offset, offset, "{text}: {error}");
			assert!(error.message.contains("'apns'"), "{text}: {error}");
		}

		// The other device errors carry offsets like the attribute ones.
		for (text, offset) in [
			("device", 6),
			("device.", 7),
			("device.locale", 13),
			("device.locale ~ 'en'", 14),
			("device.locale in 'en'", 17),
			("has(device.locale", 17),
			("has(device locale)", 10),
			("devices.locale == 'en'", 0),
		] {
			let error = Condition::parse(text).expect_err(text);
			assert_eq!(
				error.offset, offset,
				"{text:?}: the error points where the parser gave up: {error}"
			);
		}
	}

	/// Device predicates anchor a condition: with no device every one is
	/// false, so a condition of device predicates alone is not an everyone
	/// audience in disguise — but one that holds with no device is. It walks
	/// the registry, like everyone, unless a topic anchors it.
	#[test]
	fn device_predicates_anchor_and_name_their_walk() {
		let only_device = Condition::parse("device.platform == 'apns'").unwrap();
		assert!(only_device.mentions_device());
		assert!(!only_device.topic_anchored(), "walks the registry");

		let mixed = Condition::parse("'news' in topics && device.language == 'fr'").unwrap();
		assert!(mixed.mentions_device());
		assert!(mixed.topic_anchored(), "the topic anchors it");

		Condition::parse("user.plan == 'pro' && !(device.platform == 'apns')")
			.expect("anchored by the attribute");
		Condition::parse("device.app_version < '2.1' || 'a' in topics").expect("anchored");

		for text in [
			"!(device.platform == 'apns')",
			"!has(device.locale)",
			"device.platform == 'apns' || !('a' in topics)",
		] {
			let error = Condition::parse(text).expect_err(text);
			assert!(error.message.contains("no device"), "{text}: {error}");
		}

		assert!(
			!Condition::parse("user.plan == 'pro'")
				.unwrap()
				.mentions_device()
		);
		assert!(!Condition::parse("'a' in topics").unwrap().mentions_device());
		assert!(
			Condition::parse("'a' in topics && !(has(device.locale))")
				.unwrap()
				.mentions_device()
		);

		// Device predicates share the attribute bound.
		let eleven = (1..=11)
			.map(|n| {
				if n % 2 == 0 {
					format!("device.app_version != '{n}'")
				} else {
					format!("user.k{n} == 'v'")
				}
			})
			.collect::<Vec<_>>();
		let balanced = format!(
			"(({} && {}) && ({} && {})) && (({} && {}) && ({} && {})) && {} && {} && {}",
			eleven[0],
			eleven[1],
			eleven[2],
			eleven[3],
			eleven[4],
			eleven[5],
			eleven[6],
			eleven[7],
			eleven[8],
			eleven[9],
			eleven[10]
		);
		let error = Condition::parse(&balanced).expect_err("eleven predicates");
		assert!(error.message.contains("at most 10"), "{error}");
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
