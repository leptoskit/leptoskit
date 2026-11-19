//! Recurring sends: a message that goes out again on a schedule.
//!
//! [`send`](fn@crate::send) happens once. A reminder every morning, a
//! digest every Monday, or a statement on the first of each month is the
//! same message sent again and again, and someone has to remember to send
//! it. A recurrence is the service remembering: a [`NewRecurrence`] holds
//! the message, who it is for, and a [`RecurrenceRule`], and the service
//! makes one send per occurrence until the rule ends or the recurrence is
//! [ended](end_recurrence).
//!
//! What a recurrence promises:
//!
//! - **One send per occurrence, exactly once.** The service advances the
//!   recurrence and makes the occurrence's send in one transaction, and the
//!   send's idempotency key is made from the recurrence's id and the
//!   occurrence's instant ([`occurrence_key`]). Two workers that find the same
//!   occurrence due make one send between them.
//! - **Each occurrence is an ordinary send.** It is checked, counted, pushed,
//!   and listed like any other, skips the people who opted out of its category,
//!   and carries the label [`recurrence_label`] makes, so
//!   [`list_sends`](fn@crate::list_sends) shows which recurrence it came from.
//! - **No backlog.** After the service was down, a recurrence sends at most its
//!   latest missed occurrence, and only when that occurrence is no more than
//!   [`CATCH_UP_SECS`] late. Everything older is counted as
//!   [skipped](SavedRecurrence::skipped) and never sent.
//!
//! The rule is small and typed: [`Repeat`] says every so many days, every
//! so many weeks on chosen weekdays, or every so many months on a day of
//! the month; `at` is a wall-clock time in a named time zone. There are no
//! cron strings.
//!
//! Every operation here is for the app's server only.

use leptoskit_db::table;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::MessageKind;
use crate::SendSpec;
use crate::SendTarget;
use crate::TargetSummary;
use crate::campaign::MAX_CAMPAIGN_NAME_LEN;
use crate::campaign::MAX_CAMPAIGN_NOTE_LEN;

/// The most recurrences that have not ended, active and paused together.
/// End some to make room.
pub const MAX_RECURRENCES: u32 = 100;

/// The most recurrences one page of [`recurrences`] answers.
pub const MAX_RECURRENCES_PAGE: u32 = 50;

/// The longest gap [`Repeat::Daily`] may leave between occurrences, in
/// days. The shortest is one day: a recurrence has at most one occurrence
/// on any date.
pub const MAX_EVERY_DAYS: u32 = 365;

/// The longest gap [`Repeat::Weekly`] may leave, in weeks.
pub const MAX_EVERY_WEEKS: u32 = 52;

/// The longest gap [`Repeat::Monthly`] may leave, in months.
pub const MAX_EVERY_MONTHS: u32 = 12;

/// The most occurrences [`RecurrenceEnd::After`] may name.
pub const MAX_OCCURRENCES: u32 = 10_000;

/// How far ahead of the day it is created a rule's
/// [`starts_on`](RecurrenceRule::starts_on) may be, in days.
pub const MAX_START_AHEAD_DAYS: i64 = 366;

/// How late an occurrence may be sent, in seconds: a day. An occurrence
/// the service could not make within this long of its time — because it
/// was down — is skipped, not sent late.
pub const CATCH_UP_SECS: i64 = 24 * 60 * 60;

/// The longest reason a refused occurrence leaves on its recurrence, in
/// bytes.
pub const MAX_RECURRENCE_ERROR_LEN: usize = 256;

/// A day of the week.
#[derive(
	Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
	Monday,
	Tuesday,
	Wednesday,
	Thursday,
	Friday,
	Saturday,
	Sunday,
}

impl Weekday {
	/// The day of the week `date` falls on.
	#[must_use]
	pub fn of(date: chrono::NaiveDate) -> Self {
		use chrono::Datelike as _;

		match date.weekday() {
			chrono::Weekday::Mon => Self::Monday,
			chrono::Weekday::Tue => Self::Tuesday,
			chrono::Weekday::Wed => Self::Wednesday,
			chrono::Weekday::Thu => Self::Thursday,
			chrono::Weekday::Fri => Self::Friday,
			chrono::Weekday::Sat => Self::Saturday,
			chrono::Weekday::Sun => Self::Sunday,
		}
	}
}

fn one() -> u32 {
	1
}

/// How often a recurrence repeats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
	/// Every `every` days, counted from the rule's
	/// [`starts_on`](RecurrenceRule::starts_on): 1 is every day, 2 every
	/// other day. 1 to [`MAX_EVERY_DAYS`].
	Daily {
		/// How many days from one occurrence to the next. 1 when left out.
		#[serde(default = "one")]
		every: u32,
	},
	/// On the weekdays in `on`, every `every` weeks. Weeks start on Monday
	/// and are counted from the week
	/// [`starts_on`](RecurrenceRule::starts_on) is in.
	Weekly {
		/// How many weeks from one week of occurrences to the next. 1 when
		/// left out; 1 to [`MAX_EVERY_WEEKS`].
		#[serde(default = "one")]
		every: u32,
		/// The weekdays it goes out on: at least one, each once.
		on: Vec<Weekday>,
	},
	/// On day `day` of the month, every `every` months, counted from the
	/// month [`starts_on`](RecurrenceRule::starts_on) is in. In a month
	/// with fewer days it goes out on the month's last day: day 31 is 30
	/// April, and 28 or 29 February.
	Monthly {
		/// How many months from one occurrence to the next. 1 when left
		/// out; 1 to [`MAX_EVERY_MONTHS`].
		#[serde(default = "one")]
		every: u32,
		/// The day of the month, 1 to 31.
		day: u8,
	},
}

/// When a recurrence stops by itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceEnd {
	/// The last date an occurrence may fall on, as `YYYY-MM-DD` in the
	/// rule's time zone.
	On(String),
	/// After this many sends, 1 to [`MAX_OCCURRENCES`]. A skipped
	/// occurrence is not a send and does not count.
	After(u32),
}

#[expect(
	clippy::trivially_copy_pass_by_ref,
	reason = "serde's `skip_serializing_if` passes a reference"
)]
fn is_false(value: &bool) -> bool {
	!*value
}

/// When a recurrence's occurrences fall.
///
/// An occurrence is a date the [`repeat`](Self::repeat) names, at the
/// wall-clock time [`at`](Self::at) in [`time_zone`](Self::time_zone).
/// Two things about wall-clock times:
///
/// - **A time the clocks skip** — 02:30 on the night they jump from 02:00 to
///   03:00 — goes out when the gap ends, as far past it as the time was into
///   it: at 03:30.
/// - **A time the clocks repeat** — 01:30 on the night they fall back from
///   02:00 to 01:00 — goes out the first time, and not again an hour later.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecurrenceRule {
	/// How often.
	pub repeat: Repeat,
	/// The wall-clock time of day, as `HH:MM` on a 24-hour clock.
	pub at: String,
	/// The IANA time zone `at` is read in, such as `Europe/London`.
	pub time_zone: String,
	/// The first date an occurrence may fall on, as `YYYY-MM-DD` in the
	/// rule's time zone, and where "every other week" and "every third
	/// month" are counted from. The day the recurrence is created when
	/// left out; the service fills it in, so a saved rule always has one.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub starts_on: Option<String>,
	/// When it stops by itself. It goes on until
	/// [ended](end_recurrence) when left out.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub ends: Option<RecurrenceEnd>,
	/// Whether each person gets it when `at` arrives in their own time
	/// zone, the way [`SendSpec::local_time`] does for one send, instead
	/// of everyone at the one instant `at` names in the rule's time zone.
	/// The rule's time zone is then the one a person with no known time
	/// zone is read in. Each occurrence's send is made fourteen hours
	/// before `at` read as UTC, so that it is ready for the first time
	/// zone the time reaches.
	#[serde(default, skip_serializing_if = "is_false")]
	pub local_time: bool,
}

/// Reads a `YYYY-MM-DD` date.
fn date(text: &str) -> Option<chrono::NaiveDate> {
	// `parse_from_str` would take `2027-1-5`; the stored form is exact.
	if text.len() != 10 {
		return None;
	}

	chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

/// The last day of the month `date` is in.
fn last_day(date: chrono::NaiveDate) -> u32 {
	use chrono::Datelike as _;

	let (year, month) = if date.month() == 12 {
		(date.year() + 1, 1)
	} else {
		(date.year(), date.month() + 1)
	};

	chrono::NaiveDate::from_ymd_opt(year, month, 1)
		.and_then(|first| first.pred_opt())
		.map_or(28, |last| last.day())
}

impl RecurrenceRule {
	/// The time of day [`at`](Self::at) names.
	///
	/// # Errors
	///
	/// A message naming what's wrong with it.
	pub fn time(&self) -> Result<chrono::NaiveTime, String> {
		let complaint = || "`at` is a time of day as `HH:MM`, from `00:00` to `23:59`".to_owned();

		if self.at.len() != 5 {
			return Err(complaint());
		}

		chrono::NaiveTime::parse_from_str(&self.at, "%H:%M").map_err(|_| complaint())
	}

	/// The date [`starts_on`](Self::starts_on) names, when it names one.
	///
	/// # Errors
	///
	/// A message naming what's wrong with it.
	pub fn start(&self) -> Result<Option<chrono::NaiveDate>, String> {
		self.starts_on
			.as_deref()
			.map(|text| {
				date(text).ok_or_else(|| "`starts_on` is a date as `YYYY-MM-DD`".to_owned())
			})
			.transpose()
	}

	/// The last date an occurrence may fall on, when the rule ends on one.
	///
	/// # Errors
	///
	/// A message naming what's wrong with it.
	pub fn last_date(&self) -> Result<Option<chrono::NaiveDate>, String> {
		match &self.ends {
			Some(RecurrenceEnd::On(text)) => {
				date(text)
					.map(Some)
					.ok_or_else(|| "`ends.on` is a date as `YYYY-MM-DD`".to_owned())
			}
			_ => Ok(None),
		}
	}

	/// Whether an occurrence falls on `date`, for a rule that starts on
	/// `start`: on or after `start`, on or before the date the rule ends
	/// on, and a date the [`repeat`](Self::repeat) names.
	#[must_use]
	pub fn falls_on(&self, start: chrono::NaiveDate, date: chrono::NaiveDate) -> bool {
		use chrono::Datelike as _;

		if date < start {
			return false;
		}

		if let Ok(Some(last)) = self.last_date()
			&& date > last
		{
			return false;
		}

		match &self.repeat {
			Repeat::Daily { every } => (date - start).num_days() % i64::from((*every).max(1)) == 0,
			Repeat::Weekly { every, on } => {
				let first_monday = start
					- chrono::Duration::days(i64::from(start.weekday().num_days_from_monday()));
				let weeks = (date - first_monday).num_days() / 7;

				weeks % i64::from((*every).max(1)) == 0 && on.contains(&Weekday::of(date))
			}
			Repeat::Monthly { every, day } => {
				let months = i64::from(date.year() - start.year()) * 12 + i64::from(date.month())
					- i64::from(start.month());

				months % i64::from((*every).max(1)) == 0
					&& date.day() == u32::from(*day).min(last_day(date))
			}
		}
	}

	/// The first date after `date` an occurrence falls on, for a rule that
	/// starts on `start`, or `None` when the rule has ended by then.
	#[must_use]
	pub fn next_date_after(
		&self,
		start: chrono::NaiveDate,
		date: chrono::NaiveDate,
	) -> Option<chrono::NaiveDate> {
		// Never look before the start: a rule that starts next month has
		// nothing this month.
		let mut candidate = date.max(start.pred_opt()?);
		let last = self.last_date().ok().flatten();

		// The longest any rule leaves between two occurrences is a little
		// over a year: 365 days, 52 weeks and six days, or 12 months.
		for _ in 0..400 {
			candidate = candidate.succ_opt()?;

			if last.is_some_and(|last| candidate > last) {
				return None;
			}

			if self.falls_on(start, candidate) {
				return Some(candidate);
			}
		}

		None
	}
}

/// Checks a rule's shape: what it repeats on, its time of day, the shape
/// of its time zone's name, and its dates. Whether the time zone exists is
/// the service's to say, because it holds the time zone database.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_rule(rule: &RecurrenceRule) -> Result<(), String> {
	match &rule.repeat {
		Repeat::Daily { every } => {
			if *every == 0 || *every > MAX_EVERY_DAYS {
				return Err(format!(
					"`repeat.daily.every` is 1 to {MAX_EVERY_DAYS} days: a recurrence goes out at \
					 most once a day"
				));
			}
		}
		Repeat::Weekly { every, on } => {
			if *every == 0 || *every > MAX_EVERY_WEEKS {
				return Err(format!(
					"`repeat.weekly.every` is 1 to {MAX_EVERY_WEEKS} weeks"
				));
			}

			let mut days = on.clone();
			days.sort_unstable();
			days.dedup();

			if on.is_empty() || days.len() != on.len() {
				return Err(
					"`repeat.weekly.on` names at least one weekday, and each one once".into(),
				);
			}
		}
		Repeat::Monthly { every, day } => {
			if *every == 0 || *every > MAX_EVERY_MONTHS {
				return Err(format!(
					"`repeat.monthly.every` is 1 to {MAX_EVERY_MONTHS} months"
				));
			}

			if *day == 0 || *day > 31 {
				return Err("`repeat.monthly.day` is a day of the month, 1 to 31".into());
			}
		}
	}

	rule.time()?;

	if crate::push::check_time_zone(&rule.time_zone).is_err() {
		return Err(format!(
			"time_zone: {}",
			crate::local::zone_complaint(&rule.time_zone)
		));
	}

	let start = rule.start()?;
	let last = rule.last_date()?;

	if let (Some(start), Some(last)) = (start, last)
		&& last < start
	{
		return Err("`ends.on` is on or after `starts_on`".into());
	}

	if let Some(RecurrenceEnd::After(count)) = rule.ends
		&& (count == 0 || count > MAX_OCCURRENCES)
	{
		return Err(format!("`ends.after` is 1 to {MAX_OCCURRENCES} sends"));
	}

	Ok(())
}

/// What a recurrence is made from: the message, who it is for, when it
/// repeats, and a name and a note for the people who look after it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NewRecurrence {
	/// What the people looking after it call it. Never sent to a device.
	pub name: String,
	/// The message every occurrence sends. It carries no schedule and no
	/// label of its own: `visible_at`, `expires_at`, `local_time`, and
	/// `label` must be unset. The rule is the schedule, and the service
	/// labels each occurrence's send.
	pub spec: SendSpec,
	/// Who every occurrence is for. A topic, a condition, and everyone
	/// are read again at each occurrence, so each reaches the audience as
	/// it stands that day.
	pub target: SendTarget,
	/// When it repeats.
	pub rule: RecurrenceRule,
	/// A note for the people looking after it. Never sent to a device.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub note: Option<String>,
}

/// Where a recurrence stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceState {
	/// Its occurrences are sent as they come.
	#[default]
	Active,
	/// Nothing is sent until it is [resumed](resume_recurrence). An
	/// occurrence that comes while it is paused is not sent later.
	Paused,
	/// Over, for good: its rule ended, or it was [ended](end_recurrence).
	Ended,
}

impl RecurrenceState {
	/// The name the state is stored under.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Active => "active",
			Self::Paused => "paused",
			Self::Ended => "ended",
		}
	}

	/// The state a stored name stands for.
	#[must_use]
	pub fn from_str_value(name: &str) -> Option<Self> {
		match name {
			"active" => Some(Self::Active),
			"paused" => Some(Self::Paused),
			"ended" => Some(Self::Ended),
			_ => None,
		}
	}
}

/// A recurrence as the service holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct SavedRecurrence {
	/// The recurrence's id.
	pub id: uuid::Uuid,
	/// Where it stands.
	pub state: RecurrenceState,
	/// Its name.
	pub name: String,
	/// Its note.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub note: Option<String>,
	/// The message every occurrence sends.
	pub spec: SendSpec,
	/// Who every occurrence is for.
	pub target: SendTarget,
	/// When it repeats, with [`starts_on`](RecurrenceRule::starts_on)
	/// filled in.
	pub rule: RecurrenceRule,
	/// The instant of the next occurrence: `at` on its date, in the
	/// rule's time zone. `None` while paused and once ended.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub next_at: Option<leptoskit_db::Timestamp>,
	/// The instant of the last occurrence that was sent.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub last_at: Option<leptoskit_db::Timestamp>,
	/// The idempotency key of the last occurrence's send: what
	/// [`send_report`](fn@crate::send_report) takes.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub last_send_key: Option<String>,
	/// How many occurrences were sent.
	pub sent: u32,
	/// How many occurrences were not sent: missed while the service was
	/// down, more than [`CATCH_UP_SECS`] late, or refused.
	pub skipped: u32,
	/// Why the service paused it, when it did: an occurrence's send was
	/// refused, and this is the refusal. Cleared when it is resumed.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
	/// When it was created.
	pub created_at: leptoskit_db::Timestamp,
	/// Who created it.
	pub created_by: String,
	/// When it last changed: a pause, a resume, an end, or an occurrence.
	pub updated_at: leptoskit_db::Timestamp,
	/// Who last paused, resumed, or ended it, or created it.
	pub updated_by: String,
	/// When it ended.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub ended_at: Option<leptoskit_db::Timestamp>,
}

/// One recurrence in a list: what a row of a table shows, without the
/// whole message and audience.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RecurrenceEntry {
	/// The recurrence's id.
	pub id: uuid::Uuid,
	/// Where it stands.
	pub state: RecurrenceState,
	/// Its name.
	pub name: String,
	/// The message's category.
	pub category: String,
	/// The message's title.
	pub title: String,
	/// Whether it is a notification or a silent data message.
	#[serde(default)]
	pub kind: MessageKind,
	/// Who it is for.
	pub target: TargetSummary,
	/// When it repeats.
	pub rule: RecurrenceRule,
	/// The instant of the next occurrence.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub next_at: Option<leptoskit_db::Timestamp>,
	/// The instant of the last occurrence that was sent.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub last_at: Option<leptoskit_db::Timestamp>,
	/// How many occurrences were sent.
	pub sent: u32,
	/// How many were not.
	pub skipped: u32,
	/// When it last changed.
	pub updated_at: leptoskit_db::Timestamp,
}

/// Which recurrences [`recurrences`] lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RecurrenceFilter {
	/// Only recurrences in this state. Every state when left out.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub state: Option<RecurrenceState>,
}

impl RecurrenceFilter {
	/// Every recurrence.
	#[must_use]
	pub fn all() -> Self {
		Self::default()
	}

	/// Only the recurrences in `state`.
	#[must_use]
	pub fn in_state(mut self, state: RecurrenceState) -> Self {
		self.state = Some(state);
		self
	}
}

/// One recurrence's stored row. [`SavedRecurrence`] is what callers read.
///
/// `(state, due_at)` is what the [`recur`](fn@crate::recur) job finds the
/// due recurrences through; `(state, id)` is the list of one state and
/// the count of the ones that have not ended.
#[table(name = "recurrences", index(state, due_at), index(state, id))]
pub struct RecurrenceRow {
	/// A UUID v7: recurrences list newest first by it.
	#[db(key)]
	pub id: uuid::Uuid,
	/// A [`RecurrenceState`]'s name.
	#[db(one_of("active", "paused", "ended"))]
	pub state: String,
	/// The recurrence's name.
	#[db(min_len = 1, max_len = 120)]
	pub name: String,
	/// The recurrence's note.
	#[db(max_len = 1_000)]
	pub note: Option<String>,
	/// The message, as JSON.
	#[db(max_len = 8_192)]
	pub spec: String,
	/// The audience, as JSON.
	#[db(max_len = 40_000)]
	pub target: String,
	/// The rule, as JSON, with its start date filled in.
	#[db(max_len = 512)]
	pub rule: String,
	/// The date of the next occurrence, as `YYYY-MM-DD` in the rule's time
	/// zone. `NULL` while paused and once ended.
	#[db(max_len = 10)]
	pub next_on: Option<String>,
	/// When the next occurrence's send is made. `NULL` with `next_on`.
	pub due_at: Option<leptoskit_db::Timestamp>,
	/// How many occurrences were sent.
	pub sent: i64,
	/// How many occurrences were not.
	pub skipped: i64,
	/// The instant of the last occurrence that was sent.
	pub last_at: Option<leptoskit_db::Timestamp>,
	/// The idempotency key of the last occurrence's send.
	#[db(max_len = 255)]
	pub last_send_key: Option<String>,
	/// Why the service paused it.
	#[db(max_len = 256)]
	pub error: Option<String>,
	/// Who created it.
	#[db(min_len = 1, max_len = 128)]
	pub created_by: String,
	/// Who last paused, resumed, or ended it.
	#[db(min_len = 1, max_len = 128)]
	pub updated_by: String,
	/// When it ended.
	pub ended_at: Option<leptoskit_db::Timestamp>,
	/// When it was created.
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
	/// When it last changed.
	pub updated_at: leptoskit_db::Timestamp,
}

/// Checks everything about a new recurrence that needs no clock and no
/// time zone database: its name and note, by a campaign's bounds; its
/// [rule](check_rule); that its message carries no schedule and no label
/// of its own; and that the message and the audience are ones
/// [`send`](fn@crate::send) accepts.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_recurrence(recurrence: &NewRecurrence) -> Result<(), String> {
	if recurrence.name.trim().is_empty() || recurrence.name.len() > MAX_CAMPAIGN_NAME_LEN {
		return Err(format!(
			"a recurrence's name is 1 to {MAX_CAMPAIGN_NAME_LEN} bytes"
		));
	}

	if recurrence.name.chars().any(char::is_control) {
		return Err("a recurrence's name has no control characters".into());
	}

	if let Some(note) = &recurrence.note
		&& note.len() > MAX_CAMPAIGN_NOTE_LEN
	{
		return Err(format!(
			"a recurrence's note is at most {MAX_CAMPAIGN_NOTE_LEN} bytes"
		));
	}

	check_rule(&recurrence.rule)?;

	let spec = &recurrence.spec;

	if spec.visible_at.is_some() || spec.local_time.is_some() {
		return Err(
			"a recurrence's message has no `visible_at` and no `local_time`: its rule says when \
			 each occurrence goes out, and `rule.local_time` sends it at each person's own time"
				.into(),
		);
	}

	if spec.expires_at.is_some() {
		return Err(
			"a recurrence's message has no `expires_at`: one instant can't end every occurrence"
				.into(),
		);
	}

	if spec.label.is_some() {
		return Err(
			"a recurrence's message has no `label`: the service labels each occurrence's send \
			 with the recurrence's own"
				.into(),
		);
	}

	crate::check_spec(spec)?;
	crate::check_target(&recurrence.target)
}

/// The idempotency key of the send one occurrence is made as: the
/// recurrence's id and the occurrence's instant, in Unix seconds. One
/// occurrence is one send, whoever finds it due and however often.
#[must_use]
pub fn occurrence_key(id: uuid::Uuid, at: leptoskit_db::Timestamp) -> String {
	format!("recurrence:{id}:{}", at.timestamp())
}

/// The [label](SendSpec::label) every send of recurrence `id` carries:
/// what ties a row of [`list_sends`](fn@crate::list_sends) to its
/// recurrence.
#[must_use]
pub fn recurrence_label(id: uuid::Uuid) -> String {
	format!("recurrence.{id}")
}

/// Refuses every caller but the app's server.
#[cfg(feature = "service")]
fn app_only(identity: &rpc::server::Identity, does: &str) -> rpc::Result<()> {
	if identity.caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(format!(
			"only the app {does}: call this from its server, with the service's token"
		)));
	}

	Ok(())
}

/// Saves a recurrence and starts it: its first occurrence is the first
/// one whose send is still ahead, so nothing goes out as it is created.
///
/// - Everything [`send`](fn@crate::send) checks about the message and the
///   audience is checked now, and again at every occurrence.
/// - A recurrence for [`SendTarget::Everyone`] is refused unless `broadcast` is
///   `true`, as [`launch_campaign`](crate::campaign::launch_campaign) refuses
///   it: the flag is the caller saying it has checked that the person creating
///   it may send to everyone, again and again.
/// - At most [`MAX_RECURRENCES`] recurrences may be active or paused at once.
/// - A recurrence can't be changed. To change the message, the audience, or the
///   rule, [end](end_recurrence) it and create another.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, or for
/// an everyone audience without `broadcast`; and [`rpc::Error::Invalid`]
/// for a recurrence [`check_recurrence`] refuses, a time zone the service
/// doesn't know, a rule with no occurrence ahead, a `by`
/// [`check_actor`](crate::campaign::check_actor) refuses, or when
/// [`MAX_RECURRENCES`] already exist.
#[rpc::mutation(service = "notifications")]
pub async fn create_recurrence(
	cx: &rpc::MutationCx,
	recurrence: NewRecurrence,
	by: String,
	#[serde(default)] broadcast: bool,
) -> rpc::Result<SavedRecurrence> {
	app_only(cx.identity(), "creates recurrences")?;
	crate::campaign::check_actor(&by).map_err(rpc::Error::invalid)?;
	check_recurrence(&recurrence).map_err(rpc::Error::invalid)?;

	crate::service::recurrence::create(cx, &recurrence, &by, broadcast, leptoskit_db::now()).await
}

/// One recurrence, whole.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::NotFound`] for an unknown id.
#[rpc::query(service = "notifications")]
pub async fn recurrence(cx: &rpc::QueryCx, id: uuid::Uuid) -> rpc::Result<SavedRecurrence> {
	app_only(cx.identity(), "reads recurrences")?;

	crate::service::recurrence::read(cx.db(), id).await
}

/// The recurrences, newest first, a page of at most
/// [`MAX_RECURRENCES_PAGE`] at a time, each as a [`RecurrenceEntry`].
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a cursor that isn't one of this list's.
#[rpc::query(service = "notifications")]
pub async fn recurrences(
	cx: &rpc::QueryCx,
	page: rpc::PageArgs,
	#[serde(default)] filter: RecurrenceFilter,
) -> rpc::Result<rpc::Page<RecurrenceEntry>> {
	app_only(cx.identity(), "lists recurrences")?;

	crate::service::recurrence::list(cx.db(), &page, filter).await
}

/// Pauses a recurrence: nothing more is sent until it is
/// [resumed](resume_recurrence). A send already made goes on. Pausing a
/// paused recurrence answers it as it is.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::Invalid`] for a `by` the check refuses,
/// [`rpc::Error::NotFound`] for an unknown id, and
/// [`rpc::Error::Conflict`] for a recurrence that has ended.
#[rpc::mutation(service = "notifications")]
pub async fn pause_recurrence(
	cx: &rpc::MutationCx,
	id: uuid::Uuid,
	by: String,
) -> rpc::Result<SavedRecurrence> {
	app_only(cx.identity(), "pauses recurrences")?;
	crate::campaign::check_actor(&by).map_err(rpc::Error::invalid)?;

	crate::service::recurrence::pause(cx.tx(), id, &by).await
}

/// Resumes a paused recurrence from the first occurrence whose send is
/// still ahead. Occurrences that came while it was paused are not sent,
/// not even the latest. Resuming an active recurrence answers it as it is.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::Invalid`] for a `by` the check refuses,
/// [`rpc::Error::NotFound`] for an unknown id, and
/// [`rpc::Error::Conflict`] for a recurrence that has ended.
#[rpc::mutation(service = "notifications")]
pub async fn resume_recurrence(
	cx: &rpc::MutationCx,
	id: uuid::Uuid,
	by: String,
) -> rpc::Result<SavedRecurrence> {
	app_only(cx.identity(), "resumes recurrences")?;
	crate::campaign::check_actor(&by).map_err(rpc::Error::invalid)?;

	crate::service::recurrence::resume(cx, id, &by, leptoskit_db::now()).await
}

/// Ends a recurrence for good: nothing more is sent, and it can't be
/// resumed. A send already made goes on; cancel it by its key if that is
/// what you mean. Ending an ended recurrence answers it as it is.
///
/// Only the app may call it.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server,
/// [`rpc::Error::Invalid`] for a `by` the check refuses, and
/// [`rpc::Error::NotFound`] for an unknown id.
#[rpc::mutation(service = "notifications")]
pub async fn end_recurrence(
	cx: &rpc::MutationCx,
	id: uuid::Uuid,
	by: String,
) -> rpc::Result<SavedRecurrence> {
	app_only(cx.identity(), "ends recurrences")?;
	crate::campaign::check_actor(&by).map_err(rpc::Error::invalid)?;

	crate::service::recurrence::end(cx.tx(), id, &by).await
}

#[cfg(test)]
mod tests {
	use super::*;

	fn day(text: &str) -> chrono::NaiveDate {
		date(text).expect("a date")
	}

	fn rule(repeat: Repeat) -> RecurrenceRule {
		RecurrenceRule {
			repeat,
			at: "09:00".into(),
			time_zone: "Europe/London".into(),
			starts_on: None,
			ends: None,
			local_time: false,
		}
	}

	/// The dates a rule falls on from `start`, the first `count` of them.
	fn dates(rule: &RecurrenceRule, start: &str, count: usize) -> Vec<String> {
		let start = day(start);
		let mut at = start.pred_opt().unwrap();
		let mut found = Vec::new();

		while found.len() < count {
			let Some(next) = rule.next_date_after(start, at) else {
				break;
			};

			found.push(next.to_string());
			at = next;
		}

		found
	}

	#[test]
	fn every_n_days_counts_from_the_start() {
		assert_eq!(
			dates(&rule(Repeat::Daily { every: 1 }), "2027-02-27", 4),
			["2027-02-27", "2027-02-28", "2027-03-01", "2027-03-02"]
		);
		assert_eq!(
			dates(&rule(Repeat::Daily { every: 3 }), "2027-12-29", 3),
			["2027-12-29", "2028-01-01", "2028-01-04"]
		);
		assert_eq!(
			dates(&rule(Repeat::Daily { every: 365 }), "2027-01-01", 2),
			["2027-01-01", "2028-01-01"]
		);
	}

	#[test]
	fn weekly_goes_out_on_its_weekdays_every_n_weeks() {
		// 2027-03-03 is a Wednesday.
		let mondays_and_fridays = rule(Repeat::Weekly {
			every: 1,
			on: vec![Weekday::Friday, Weekday::Monday],
		});
		assert_eq!(
			dates(&mondays_and_fridays, "2027-03-03", 4),
			["2027-03-05", "2027-03-08", "2027-03-12", "2027-03-15"]
		);

		// Every other week, counted from the week the start is in: that
		// week's Friday, then the Monday and Friday two weeks on.
		let fortnightly = rule(Repeat::Weekly {
			every: 2,
			on: vec![Weekday::Monday, Weekday::Friday],
		});
		assert_eq!(
			dates(&fortnightly, "2027-03-03", 3),
			["2027-03-05", "2027-03-15", "2027-03-19"]
		);

		// A start on a Sunday is the last day of its week, not the first
		// of the next.
		let sundays = rule(Repeat::Weekly {
			every: 2,
			on: vec![Weekday::Sunday],
		});
		assert_eq!(
			dates(&sundays, "2027-03-07", 2),
			["2027-03-07", "2027-03-21"]
		);
	}

	#[test]
	fn day_31_is_the_last_day_of_a_shorter_month() {
		let end_of_month = rule(Repeat::Monthly { every: 1, day: 31 });
		assert_eq!(
			dates(&end_of_month, "2027-01-01", 5),
			[
				"2027-01-31",
				"2027-02-28",
				"2027-03-31",
				"2027-04-30",
				"2027-05-31"
			]
		);
		// A leap year's February has a 29th.
		assert_eq!(dates(&end_of_month, "2028-02-01", 1), ["2028-02-29"]);
		// Day 30 is the 28th in February too, and day 29 only in a leap
		// year.
		assert_eq!(
			dates(
				&rule(Repeat::Monthly { every: 1, day: 30 }),
				"2027-02-01",
				2
			),
			["2027-02-28", "2027-03-30"]
		);
		assert_eq!(
			dates(
				&rule(Repeat::Monthly { every: 12, day: 29 }),
				"2027-02-01",
				2
			),
			["2027-02-28", "2028-02-29"]
		);
		// Every third month, across a year's end.
		assert_eq!(
			dates(
				&rule(Repeat::Monthly { every: 3, day: 15 }),
				"2027-11-20",
				2
			),
			["2028-02-15", "2028-05-15"]
		);
	}

	#[test]
	fn a_rule_ends_on_its_last_date() {
		let mut until = rule(Repeat::Daily { every: 1 });
		until.ends = Some(RecurrenceEnd::On("2027-03-02".into()));
		assert_eq!(dates(&until, "2027-03-01", 5), ["2027-03-01", "2027-03-02"]);
		assert_eq!(
			until.next_date_after(day("2027-03-01"), day("2027-03-02")),
			None
		);
	}

	#[test]
	fn a_rule_is_checked_by_name() {
		let refused = |change: &dyn Fn(&mut RecurrenceRule), names: &str| {
			let mut rule = rule(Repeat::Daily { every: 1 });
			change(&mut rule);
			let error = check_rule(&rule).unwrap_err();
			assert!(error.contains(names), "`{names}` in `{error}`");
		};

		assert_eq!(check_rule(&rule(Repeat::Daily { every: 1 })), Ok(()));
		refused(
			&|rule| rule.repeat = Repeat::Daily { every: 0 },
			"once a day",
		);
		refused(
			&|rule| rule.repeat = Repeat::Daily { every: 366 },
			"repeat.daily.every",
		);
		refused(
			&|rule| {
				rule.repeat = Repeat::Weekly {
					every: 1,
					on: Vec::new(),
				};
			},
			"repeat.weekly.on",
		);
		refused(
			&|rule| {
				rule.repeat = Repeat::Weekly {
					every: 1,
					on: vec![Weekday::Monday, Weekday::Monday],
				};
			},
			"each one once",
		);
		refused(
			&|rule| {
				rule.repeat = Repeat::Weekly {
					every: 53,
					on: vec![Weekday::Monday],
				};
			},
			"repeat.weekly.every",
		);
		refused(
			&|rule| rule.repeat = Repeat::Monthly { every: 1, day: 0 },
			"repeat.monthly.day",
		);
		refused(
			&|rule| rule.repeat = Repeat::Monthly { every: 1, day: 32 },
			"repeat.monthly.day",
		);
		refused(
			&|rule| rule.repeat = Repeat::Monthly { every: 13, day: 1 },
			"repeat.monthly.every",
		);
		refused(&|rule| rule.at = "9:00".into(), "HH:MM");
		refused(&|rule| rule.at = "24:00".into(), "HH:MM");
		refused(&|rule| rule.at = "09:00:30".into(), "HH:MM");
		refused(&|rule| rule.time_zone = "+02:00".into(), "time_zone");
		refused(&|rule| rule.time_zone = String::new(), "time_zone");
		refused(
			&|rule| rule.starts_on = Some("2027-2-1".into()),
			"starts_on",
		);
		refused(
			&|rule| rule.ends = Some(RecurrenceEnd::On("2027-02-30".into())),
			"ends.on",
		);
		refused(
			&|rule| {
				rule.starts_on = Some("2027-03-01".into());
				rule.ends = Some(RecurrenceEnd::On("2027-02-28".into()));
			},
			"on or after",
		);
		refused(
			&|rule| rule.ends = Some(RecurrenceEnd::After(0)),
			"ends.after",
		);
		refused(
			&|rule| rule.ends = Some(RecurrenceEnd::After(MAX_OCCURRENCES + 1)),
			"ends.after",
		);
	}

	fn new() -> NewRecurrence {
		NewRecurrence {
			name: "Morning reminder".into(),
			spec: SendSpec {
				category: "reminders".into(),
				title: "Time to check in".into(),
				..SendSpec::default()
			},
			target: SendTarget::Topic("reminders.daily".into()),
			rule: rule(Repeat::Daily { every: 1 }),
			note: None,
		}
	}

	#[test]
	fn a_recurrence_is_checked_by_name() {
		assert_eq!(check_recurrence(&new()), Ok(()));

		let refused = |change: &dyn Fn(&mut NewRecurrence), names: &str| {
			let mut recurrence = new();
			change(&mut recurrence);
			let error = check_recurrence(&recurrence).unwrap_err();
			assert!(error.contains(names), "`{names}` in `{error}`");
		};

		refused(&|new| new.name = " ".into(), "name");
		refused(
			&|new| new.name = "n".repeat(MAX_CAMPAIGN_NAME_LEN + 1),
			"name",
		);
		refused(&|new| new.name = "a\nb".into(), "control");
		refused(
			&|new| new.note = Some("n".repeat(MAX_CAMPAIGN_NOTE_LEN + 1)),
			"note",
		);
		refused(
			&|new| new.spec.visible_at = Some(leptoskit_db::now()),
			"visible_at",
		);
		refused(
			&|new| {
				new.spec.local_time = Some(crate::LocalTime {
					at: "2027-03-01T09:00".into(),
					default_time_zone: None,
				});
			},
			"local_time",
		);
		refused(
			&|new| new.spec.expires_at = Some(leptoskit_db::now()),
			"expires_at",
		);
		refused(&|new| new.spec.label = Some("mine".into()), "label");
		refused(&|new| new.spec.title = String::new(), "title");
		refused(
			&|new| new.target = SendTarget::Recipients(Vec::new()),
			"recipients",
		);
		refused(
			&|new| new.target = SendTarget::Condition("user.plan ==".into()),
			"",
		);
		refused(&|new| new.rule.at = "noon".into(), "HH:MM");
	}

	#[test]
	fn an_occurrence_key_and_a_label_are_valid() {
		let at = chrono::DateTime::from_timestamp(4_102_444_800, 0).unwrap();
		let key = occurrence_key(uuid::Uuid::max(), at);
		assert_eq!(
			key,
			"recurrence:ffffffff-ffff-ffff-ffff-ffffffffffff:4102444800"
		);
		assert_eq!(crate::check_key(&key), Ok(()));

		let label = recurrence_label(uuid::Uuid::max());
		assert_eq!(label, "recurrence.ffffffff-ffff-ffff-ffff-ffffffffffff");
		assert_eq!(crate::check_label(&label), Ok(()));
	}

	#[test]
	fn a_rule_reads_back_as_it_was_written() {
		let rule: RecurrenceRule = serde_json::from_str(
			r#"{"repeat":{"weekly":{"on":["monday","thursday"]}},"at":"08:30","time_zone":"America/New_York","ends":{"after":12}}"#,
		)
		.unwrap();
		assert_eq!(
			rule.repeat,
			Repeat::Weekly {
				every: 1,
				on: vec![Weekday::Monday, Weekday::Thursday]
			}
		);
		assert_eq!(rule.ends, Some(RecurrenceEnd::After(12)));
		assert!(!rule.local_time);
		assert_eq!(
			serde_json::to_string(&rule).unwrap(),
			r#"{"repeat":{"weekly":{"every":1,"on":["monday","thursday"]}},"at":"08:30","time_zone":"America/New_York","ends":{"after":12}}"#
		);
	}
}
