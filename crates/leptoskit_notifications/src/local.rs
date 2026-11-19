//! What a send resolves per person: the language its text is in, and the
//! moment a wall-clock time arrives where they are.
//!
//! **Translated variants.** A [`SendSpec`] carries its text once, and may
//! carry [`Variant`]s keyed by locale that replace it: the title, the body,
//! the Apple subtitle, and the titles of the web action buttons. Each
//! person still gets exactly one inbox row, written with the text of the
//! variant their locale matches, and the push for that row carries the
//! same text.
//!
//! **Local time.** A send may name a wall-clock time ([`LocalTime`]),
//! `09:00 on 1 December`, instead of an instant. Each person's row becomes
//! visible, and is pushed, when that time arrives in their time zone.
//!
//! Both resolve per person by one rule: a `locale` or `time_zone`
//! [attribute](crate::set_person_attributes) the app set wins, then the
//! person's most recently seen live push endpoint, then the send's own
//! default: the spec's base text, and [`LocalTime::default_time_zone`].
//!
//! The checks here run on both sides, like every shared check. What needs a
//! time zone database (whether a zone exists, and when a wall-clock time
//! arrives in it) is the service's alone.

use std::borrow::Cow;
use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::MAX_BODY_LEN;
use crate::MAX_SCHEDULE_SECS;
use crate::MAX_TITLE_LEN;
use crate::MessageOptions;
use crate::SendSpec;
use crate::options::MAX_ACTION_TITLE_LEN;
use crate::options::MAX_OPTIONS_LEN;
use crate::options::MAX_SUBTITLE_LEN;

/// The most variants one send carries. The whole spec, variants included,
/// is bounded by [`MAX_SPEC_LEN`](crate::MAX_SPEC_LEN) as well, which a
/// send with long bodies reaches first.
pub const MAX_VARIANTS: usize = 32;

/// The attribute a person's language is read from, when the app set one:
/// a BCP 47 tag such as `pt-BR`.
pub const LOCALE_ATTRIBUTE: &str = "locale";

/// The attribute a person's time zone is read from, when the app set one:
/// an IANA name such as `America/Sao_Paulo`.
pub const TIME_ZONE_ATTRIBUTE: &str = "time_zone";

/// The time zone a local-time send falls back to when it names none.
pub const DEFAULT_TIME_ZONE: &str = "UTC";

/// How far ahead of UTC the earliest time zone is, in seconds: fourteen
/// hours. A wall-clock time arrives there first.
pub const EARLIEST_ZONE_SECS: i64 = 14 * 60 * 60;

/// How far behind UTC the latest time zone is, in seconds: twelve hours. A
/// wall-clock time arrives there last, which is what bounds a local-time
/// send's schedule and its expiry.
pub const LATEST_ZONE_SECS: i64 = 12 * 60 * 60;

/// The text of a send in one language: what replaces the spec's own text
/// for a person whose locale matches the variant's key. Every field is
/// optional, and one left out keeps the spec's text, so a variant can
/// translate a body and keep a product name as the title.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Variant {
	/// Replaces [`SendSpec::title`]: 1 to [`MAX_TITLE_LEN`] bytes.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,
	/// Replaces [`SendSpec::body`]: at most [`MAX_BODY_LEN`] bytes.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub body: Option<String>,
	/// Replaces `options.apple.subtitle`, which the send must set: a
	/// variant translates text the send has, it never adds any.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub subtitle: Option<String>,
	/// Replaces the titles of `options.web.actions`, by action id. An id
	/// the send's own actions don't have is refused.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub actions: BTreeMap<String, String>,
}

impl Variant {
	/// Whether the variant translates nothing at all.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		self.title.is_none()
			&& self.body.is_none()
			&& self.subtitle.is_none()
			&& self.actions.is_empty()
	}
}

/// A wall-clock time a send becomes visible at, in each person's own time
/// zone: `2026-12-01T09:00` is nine in the morning on 1 December wherever
/// the person is, so the send reaches Auckland most of a day before it
/// reaches Honolulu.
///
/// A person whose local time has already passed when the send reaches them
/// gets it at once. A time the clocks skip (a spring-forward gap) arrives
/// when the gap ends, and a time the clocks repeat (an autumn fold)
/// arrives the first time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LocalTime {
	/// The date and time on the wall, with no offset: `2026-12-01T09:00`
	/// or `2026-12-01T09:00:00`.
	pub at: String,
	/// The IANA time zone for people the service knows none for, such as
	/// `Europe/London`. `None` is [`DEFAULT_TIME_ZONE`], UTC.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub default_time_zone: Option<String>,
}

impl LocalTime {
	/// The wall-clock time `at` names.
	///
	/// # Errors
	///
	/// A message naming the shape `at` must have.
	pub fn wall_clock(&self) -> Result<chrono::NaiveDateTime, String> {
		["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"]
			.into_iter()
			.find_map(|format| chrono::NaiveDateTime::parse_from_str(&self.at, format).ok())
			// A leap second has no instant in every zone's arithmetic; a
			// schedule never needs one.
			.filter(|wall| chrono::Timelike::nanosecond(wall) < 1_000_000_000)
			.ok_or_else(|| {
				format!(
					"local_time.at is a date and a time on the wall, with no offset, like \
					 `2026-12-01T09:00`: `{}` isn't one",
					self.at
				)
			})
	}

	/// The time zone people fall back to: the one the send names, or
	/// [`DEFAULT_TIME_ZONE`].
	#[must_use]
	pub fn default_zone(&self) -> &str {
		self.default_time_zone
			.as_deref()
			.map(str::trim)
			.filter(|zone| !zone.is_empty())
			.unwrap_or(DEFAULT_TIME_ZONE)
	}

	/// The first instant the wall-clock time arrives anywhere:
	/// [`EARLIEST_ZONE_SECS`] ahead of UTC.
	///
	/// # Errors
	///
	/// As [`wall_clock`](Self::wall_clock), and for a time too far from
	/// the present to shift.
	pub fn earliest(&self) -> Result<leptoskit_db::Timestamp, String> {
		self.shifted(-EARLIEST_ZONE_SECS)
	}

	/// The last instant the wall-clock time arrives anywhere:
	/// [`LATEST_ZONE_SECS`] behind UTC. Every person's row is visible by
	/// then, so this is what a local-time send's schedule limit and its
	/// expiry are measured against.
	///
	/// # Errors
	///
	/// As [`wall_clock`](Self::wall_clock), and for a time too far from
	/// the present to shift.
	pub fn latest(&self) -> Result<leptoskit_db::Timestamp, String> {
		self.shifted(LATEST_ZONE_SECS)
	}

	/// The wall-clock time read as UTC, moved by `seconds`.
	fn shifted(&self, seconds: i64) -> Result<leptoskit_db::Timestamp, String> {
		self.wall_clock()?
			.checked_add_signed(chrono::Duration::seconds(seconds))
			.map(|shifted| shifted.and_utc())
			.ok_or_else(|| "local_time.at is too far from the present".to_owned())
	}
}

/// A send's text as one person reads it: the spec's own, or a variant's
/// over it.
#[derive(Clone, Debug, PartialEq)]
pub struct Localized<'a> {
	/// The title the person's row and push carry.
	pub title: &'a str,
	/// The body the person's row and push carry.
	pub body: &'a str,
	/// The options the person's row stores and their push is shaped by:
	/// the spec's, with the subtitle and action titles a variant
	/// translated.
	pub options: Cow<'a, MessageOptions>,
}

impl SendSpec {
	/// The key of the variant `locale` matches, as the spec spells it, or
	/// `None` when the spec's own text is what that locale reads.
	///
	/// The match is [RFC 4647](https://www.rfc-editor.org/rfc/rfc4647#section-3.4)'s
	/// lookup, case-insensitive: the whole tag first, then the tag with its
	/// last subtag dropped, and again until only the language is left. So
	/// `pt-BR` reads a `pt-BR` variant, then a `pt` one; `zh-Hant-TW` tries
	/// `zh-Hant` on the way to `zh`. It never widens: a person whose locale
	/// is `pt` does not read a `pt-BR` variant. A locale that isn't a BCP
	/// 47 tag matches nothing.
	#[must_use]
	pub fn variant_for(&self, locale: &str) -> Option<&str> {
		if self.variants.is_empty() {
			return None;
		}

		let canonical = crate::push::canonical_locale(locale).ok()?;
		let mut candidate = canonical.as_str();

		loop {
			let found = self.variants.keys().find(|key| {
				crate::push::canonical_locale(key)
					.is_ok_and(|key| key.eq_ignore_ascii_case(candidate))
			});

			if let Some(key) = found {
				return Some(key);
			}

			let (shorter, _) = candidate.rsplit_once('-')?;
			// A one-character subtag introduces an extension or private
			// use: it goes with what followed it.
			candidate = match shorter.rsplit_once('-') {
				Some((rest, singleton)) if singleton.len() == 1 => rest,
				_ => shorter,
			};
		}
	}

	/// The send's text under the variant `key` names, or its own text for
	/// `None` and for a key the spec doesn't carry.
	#[must_use]
	pub fn localized(&self, key: Option<&str>) -> Localized<'_> {
		let Some(variant) = key.and_then(|key| self.variants.get(key)) else {
			return Localized {
				title: &self.title,
				body: &self.body,
				options: Cow::Borrowed(&self.options),
			};
		};

		let mut options = Cow::Borrowed(&self.options);

		if let Some(subtitle) = variant.subtitle.as_ref()
			&& let Some(apple) = options.to_mut().apple.as_mut()
		{
			apple.subtitle = Some(subtitle.clone());
		}

		if !variant.actions.is_empty()
			&& let Some(web) = options.to_mut().web.as_mut()
		{
			for action in &mut web.actions {
				if let Some(title) = variant.actions.get(&action.action) {
					action.title.clone_from(title);
				}
			}
		}

		Localized {
			title: variant.title.as_deref().unwrap_or(&self.title),
			body: variant.body.as_deref().unwrap_or(&self.body),
			options,
		}
	}
}

/// Checks a send's variants: at most [`MAX_VARIANTS`], each keyed by a
/// BCP 47 tag no other key repeats, each translating something, and each
/// translation within the bound of the text it replaces. A variant
/// translates only what the send has: a subtitle when the send sets one,
/// and the actions the send names.
///
/// # Errors
///
/// A message naming the variant and what's wrong with it.
pub fn check_variants(spec: &SendSpec) -> Result<(), String> {
	if spec.variants.is_empty() {
		return Ok(());
	}

	if spec.is_data() {
		return Err(
			"variants translate what a notification shows, and a data message shows nothing".into(),
		);
	}

	if spec.variants.len() > MAX_VARIANTS {
		return Err(format!("a send carries at most {MAX_VARIANTS} variants"));
	}

	let mut seen: BTreeMap<String, &str> = BTreeMap::new();

	for (key, variant) in &spec.variants {
		let canonical = crate::push::canonical_locale(key).map_err(|_| {
			format!("variants: `{key}` isn't a BCP 47 language tag such as `pt-BR`")
		})?;

		if let Some(other) = seen.insert(canonical.to_ascii_lowercase(), key) {
			return Err(format!(
				"variants: `{other}` and `{key}` are the same locale, `{canonical}`"
			));
		}

		if variant.is_empty() {
			return Err(format!(
				"variants.{key} translates nothing: give it a title, a body, a subtitle, or \
				 action titles"
			));
		}

		if let Some(title) = variant.title.as_deref()
			&& (title.trim().is_empty() || title.len() > MAX_TITLE_LEN || !printable(title))
		{
			return Err(format!(
				"variants.{key}.title is 1 to {MAX_TITLE_LEN} bytes, with no control characters"
			));
		}

		if let Some(body) = variant.body.as_deref()
			&& (body.len() > MAX_BODY_LEN || !printable(body))
		{
			return Err(format!(
				"variants.{key}.body is at most {MAX_BODY_LEN} bytes, with no control characters"
			));
		}

		if let Some(subtitle) = variant.subtitle.as_deref() {
			if spec
				.options
				.apple
				.as_ref()
				.is_none_or(|apple| apple.subtitle.is_none())
			{
				return Err(format!(
					"variants.{key}.subtitle translates options.apple.subtitle, which this send \
					 doesn't set"
				));
			}

			if subtitle.is_empty() || subtitle.len() > MAX_SUBTITLE_LEN || !printable(subtitle) {
				return Err(format!(
					"variants.{key}.subtitle is 1 to {MAX_SUBTITLE_LEN} bytes of printable text"
				));
			}
		}

		for (action, title) in &variant.actions {
			if !spec
				.options
				.web
				.as_ref()
				.is_some_and(|web| web.actions.iter().any(|known| known.action == *action))
			{
				return Err(format!(
					"variants.{key}.actions names `{action}`, which isn't one of \
					 options.web.actions"
				));
			}

			if title.is_empty() || title.len() > MAX_ACTION_TITLE_LEN || !printable(title) {
				return Err(format!(
					"variants.{key}.actions.{action} is 1 to {MAX_ACTION_TITLE_LEN} bytes of \
					 printable text"
				));
			}
		}

		// Each person's row stores the options as their variant shaped
		// them, so each variant's are held to the column on their own.
		if crate::stored_len(spec.localized(Some(key)).options.as_ref()) > MAX_OPTIONS_LEN {
			return Err(format!(
				"variants.{key}: with its subtitle and action titles, the options pass \
				 {MAX_OPTIONS_LEN} bytes once stored as JSON"
			));
		}
	}

	Ok(())
}

/// Checks what can never change about a send's local time: a wall-clock
/// time that parses, a default time zone shaped like an IANA name, no
/// `visible_at` beside it (a send has one schedule), and an expiry after
/// the last zone's turn ([`LocalTime::latest`]), so no person's row can
/// expire before it shows.
///
/// Whether the default zone exists is the service's to say: it holds the
/// time zone database.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_local_time(spec: &SendSpec) -> Result<(), String> {
	let Some(local) = spec.local_time.as_ref() else {
		return Ok(());
	};

	if spec.visible_at.is_some() {
		return Err(
			"a send is scheduled by `visible_at` or by `local_time`, not both: drop one".into(),
		);
	}

	let latest = local.latest()?;
	local.earliest()?;

	if let Some(zone) = local.default_time_zone.as_deref() {
		crate::push::check_time_zone(zone)
			.map_err(|_| format!("local_time.default_time_zone: {}", zone_complaint(zone)))?;
	}

	if let Some(expires_at) = spec.expires_at
		&& expires_at <= latest
	{
		return Err(format!(
			"an expiry must be after the local time has arrived everywhere: `{}` reaches the last \
			 time zone at {}",
			local.at,
			latest.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
		));
	}

	Ok(())
}

/// What a zone that isn't one is told.
pub(crate) fn zone_complaint(zone: &str) -> String {
	format!("`{zone}` isn't an IANA time zone name such as `Europe/London`")
}

/// Checks what the clock says about a new send's local time: the last
/// zone's turn within [`MAX_SCHEDULE_SECS`] of now. A wall-clock time
/// already past everywhere is not refused: it means now, as a past
/// `visible_at` does.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_local_timing(local: &LocalTime) -> Result<(), String> {
	check_local_timing_at(local, chrono::Utc::now())
}

/// [`check_local_timing`], with the clock as an argument.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_local_timing_at(
	local: &LocalTime,
	now: leptoskit_db::Timestamp,
) -> Result<(), String> {
	if (local.latest()? - now).num_seconds() > MAX_SCHEDULE_SECS {
		return Err(format!(
			"a notification may be scheduled at most {MAX_SCHEDULE_SECS} seconds ahead, counted \
			 to the last time zone its local time reaches"
		));
	}

	Ok(())
}

/// Printable text: no control characters.
fn printable(value: &str) -> bool {
	!value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use super::LocalTime;
	use super::MAX_VARIANTS;
	use super::Variant;
	use super::check_local_time;
	use super::check_local_timing_at;
	use super::check_variants;
	use crate::MessageOptions;
	use crate::SendSpec;
	use crate::options::AppleOptions;
	use crate::options::MessageKind;
	use crate::options::WebOptions;
	use crate::options::WebPushAction;

	fn spec() -> SendSpec {
		SendSpec {
			category: "news".into(),
			title: "Hello".into(),
			body: "A body".into(),
			..SendSpec::default()
		}
	}

	fn titled(title: &str) -> Variant {
		Variant {
			title: Some(title.into()),
			..Variant::default()
		}
	}

	fn with(variants: &[(&str, Variant)]) -> SendSpec {
		SendSpec {
			variants: variants
				.iter()
				.map(|(key, variant)| ((*key).to_owned(), variant.clone()))
				.collect(),
			..spec()
		}
	}

	fn local(at: &str) -> LocalTime {
		LocalTime {
			at: at.into(),
			default_time_zone: None,
		}
	}

	fn instant(rfc3339: &str) -> leptoskit_db::Timestamp {
		chrono::DateTime::parse_from_rfc3339(rfc3339)
			.unwrap()
			.with_timezone(&chrono::Utc)
	}

	#[test]
	fn a_locale_reads_its_own_variant_then_its_language_then_the_default() {
		let spec = with(&[
			("pt", titled("Olá")),
			("pt-BR", titled("Oi")),
			("zh-Hant", titled("你好")),
			("fr", titled("Bonjour")),
		]);

		assert_eq!(spec.variant_for("pt-BR"), Some("pt-BR"), "the exact tag");
		assert_eq!(
			spec.variant_for("PT_br"),
			Some("pt-BR"),
			"whatever its case"
		);
		assert_eq!(spec.variant_for("pt-PT"), Some("pt"), "then the language");
		assert_eq!(spec.variant_for("pt"), Some("pt"));
		assert_eq!(
			spec.variant_for("zh-Hant-TW"),
			Some("zh-Hant"),
			"one subtag at a time"
		);
		assert_eq!(spec.variant_for("zh-Hans-CN"), None, "never sideways");
		assert_eq!(spec.variant_for("fr-CA-u-ca-gregory"), Some("fr"));
		assert_eq!(spec.variant_for("de"), None, "then the send's own text");
		assert_eq!(spec.variant_for("not a tag"), None);
		assert_eq!(spec.localized(Some("pt-BR")).title, "Oi");
		assert_eq!(
			spec.localized(Some("pt-BR")).body,
			"A body",
			"what a variant leaves"
		);
		assert_eq!(spec.localized(None).title, "Hello");
	}

	#[test]
	fn a_language_never_widens_to_a_region() {
		let spec = with(&[("pt-BR", titled("Oi"))]);

		assert_eq!(spec.variant_for("pt"), None);
		assert_eq!(spec.variant_for("pt-PT"), None);
	}

	#[test]
	fn a_variant_key_matches_however_the_spec_spells_it() {
		let spec = with(&[("EN_gb", titled("Colour"))]);

		assert_eq!(spec.variant_for("en-GB"), Some("EN_gb"));
		check_variants(&spec).expect("a tag in any case is a tag");
	}

	#[test]
	fn a_variant_translates_the_subtitle_and_the_action_titles() {
		let mut spec = spec();
		spec.options = MessageOptions {
			apple: Some(AppleOptions {
				subtitle: Some("Today".into()),
				..AppleOptions::default()
			}),
			web: Some(WebOptions {
				actions: vec![
					WebPushAction {
						action: "open".into(),
						title: "Open".into(),
					},
					WebPushAction {
						action: "later".into(),
						title: "Later".into(),
					},
				],
				..WebOptions::default()
			}),
			..MessageOptions::default()
		};
		spec.variants.insert(
			"fr".into(),
			Variant {
				subtitle: Some("Aujourd'hui".into()),
				actions: BTreeMap::from([("open".to_owned(), "Ouvrir".to_owned())]),
				..Variant::default()
			},
		);
		check_variants(&spec).expect("it translates what the send has");

		let french = spec.localized(Some("fr"));
		assert_eq!(french.title, "Hello");
		assert_eq!(
			french.options.apple.as_ref().unwrap().subtitle.as_deref(),
			Some("Aujourd'hui")
		);
		let actions = &french.options.web.as_ref().unwrap().actions;
		assert_eq!(actions[0].title, "Ouvrir");
		assert_eq!(
			actions[1].title, "Later",
			"an untranslated action keeps its title"
		);
		assert_eq!(
			spec.localized(None).options.as_ref(),
			&spec.options,
			"the send's own text is untouched"
		);
	}

	#[test]
	fn bad_variants_are_refused_by_name() {
		let refused = |spec: &SendSpec| check_variants(spec).expect_err("refused");

		assert!(refused(&with(&[("not a tag", titled("x"))])).contains("`not a tag`"));
		assert!(
			refused(&with(&[("en-GB", titled("a")), ("en_gb", titled("b"))]))
				.contains("the same locale")
		);
		assert!(refused(&with(&[("fr", Variant::default())])).contains("translates nothing"));
		assert!(refused(&with(&[("fr", titled(" "))])).contains("variants.fr.title"));
		assert!(refused(&with(&[("fr", titled(&"x".repeat(201)))])).contains("variants.fr.title"));
		assert!(
			refused(&with(&[(
				"fr",
				Variant {
					body: Some("x".repeat(2_001)),
					..Variant::default()
				}
			)]))
			.contains("variants.fr.body")
		);
		assert!(
			refused(&with(&[(
				"fr",
				Variant {
					subtitle: Some("Sous-titre".into()),
					..Variant::default()
				}
			)]))
			.contains("doesn't set")
		);
		assert!(
			refused(&with(&[(
				"fr",
				Variant {
					actions: BTreeMap::from([("open".to_owned(), "Ouvrir".to_owned())]),
					..Variant::default()
				}
			)]))
			.contains("isn't one of options.web.actions")
		);

		let letters = "abcdefghijklmnopqrstuvwxyz";
		let many: Vec<(String, Variant)> = letters
			.chars()
			.flat_map(|first| {
				letters
					.chars()
					.map(move |second| format!("q{first}{second}"))
			})
			.take(MAX_VARIANTS + 1)
			.map(|key| (key, titled("x")))
			.collect();
		let many: Vec<(&str, Variant)> = many
			.iter()
			.map(|(key, variant)| (key.as_str(), variant.clone()))
			.collect();
		assert!(refused(&with(&many)).contains("at most 32 variants"));

		let mut silent = with(&[("fr", titled("Bonjour"))]);
		silent.options.kind = MessageKind::Data;
		assert!(refused(&silent).contains("a data message shows nothing"));
	}

	#[test]
	fn variants_count_toward_the_stored_spec() {
		let long = Variant {
			body: Some("x".repeat(2_000)),
			..Variant::default()
		};
		let spec = with(&[
			("de", long.clone()),
			("es", long.clone()),
			("fr", long.clone()),
			("it", long.clone()),
			("pt", long),
		]);

		check_variants(&spec).expect("each variant is within its own bounds");
		let refused = crate::check_spec(&spec).expect_err("together they pass the column");
		assert!(refused.contains("8192 bytes"), "{refused}");
		assert!(refused.contains("variants"), "{refused}");
	}

	#[test]
	fn a_spec_without_variants_or_local_time_stores_as_it_always_did() {
		let stored = serde_json::to_string(&spec()).unwrap();

		assert!(!stored.contains("variants"), "{stored}");
		assert!(!stored.contains("local_time"), "{stored}");

		let parsed: SendSpec =
			serde_json::from_str(r#"{"category":"news","title":"Hello"}"#).unwrap();
		assert!(parsed.variants.is_empty());
		assert!(parsed.local_time.is_none());
	}

	#[test]
	fn a_wall_clock_time_parses_with_or_without_seconds_and_never_with_an_offset() {
		assert_eq!(
			local("2026-12-01T09:00").wall_clock().unwrap(),
			local("2026-12-01T09:00:00").wall_clock().unwrap()
		);

		for bad in [
			"2026-12-01T09:00:00Z",
			"2026-12-01T09:00+01:00",
			"2026-12-01 09:00",
			"2026-12-01",
			"09:00",
			"2026-02-30T09:00",
			"2026-12-01T25:00",
			"2026-12-01T23:59:60",
			"",
		] {
			let refused = local(bad).wall_clock().expect_err(bad);
			assert!(refused.contains("local_time.at"), "{refused}");
		}
	}

	#[test]
	fn a_local_time_spans_twenty_six_hours_of_instants() {
		let local = local("2026-12-01T09:00");

		assert_eq!(local.earliest().unwrap(), instant("2026-11-30T19:00:00Z"));
		assert_eq!(local.latest().unwrap(), instant("2026-12-01T21:00:00Z"));
		assert_eq!(local.default_zone(), "UTC");
	}

	#[test]
	fn a_local_time_send_has_one_schedule_and_an_expiry_after_every_zone() {
		let mut send = spec();
		send.local_time = Some(local("2026-12-01T09:00"));
		check_local_time(&send).expect("a local time alone is a schedule");

		send.visible_at = Some(instant("2026-12-01T09:00:00Z"));
		assert!(check_local_time(&send).unwrap_err().contains("not both"));
		send.visible_at = None;

		// After nine in UTC, but before nine has reached UTC-12.
		send.expires_at = Some(instant("2026-12-01T12:00:00Z"));
		let refused = check_local_time(&send).unwrap_err();
		assert!(refused.contains("arrived everywhere"), "{refused}");
		assert!(refused.contains("2026-12-01T21:00:00Z"), "{refused}");

		send.expires_at = Some(instant("2026-12-01T21:00:00Z"));
		assert!(
			check_local_time(&send).is_err(),
			"the last turn itself is too early"
		);
		send.expires_at = Some(instant("2026-12-01T21:00:01Z"));
		check_local_time(&send).expect("after the last zone's turn");

		send.local_time = Some(LocalTime {
			at: "2026-12-01T09:00".into(),
			default_time_zone: Some("Not A Zone".into()),
		});
		let refused = check_local_time(&send).unwrap_err();
		assert!(
			refused.contains("local_time.default_time_zone"),
			"{refused}"
		);
	}

	#[test]
	fn the_last_zone_bounds_how_far_ahead_a_local_time_schedules() {
		let now = instant("2026-01-01T00:00:00Z");
		// 366 days ahead of now is 2027-01-02T00:00Z; the last zone's turn
		// is twelve hours after the wall-clock time read as UTC.
		check_local_timing_at(&local("2027-01-01T12:00"), now).expect("exactly at the limit");
		let refused = check_local_timing_at(&local("2027-01-01T12:00:01"), now).unwrap_err();
		assert!(refused.contains("last time zone"), "{refused}");
		check_local_timing_at(&local("2020-01-01T09:00"), now)
			.expect("a time already past everywhere means now");
	}
}
