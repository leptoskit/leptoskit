//! The bodies of the recurrence operations, and the work that makes each
//! occurrence's send.
//!
//! **Exactly once.** An occurrence is made by one transaction
//! ([`Occur`]) that does three things together: it moves the recurrence
//! on to its next occurrence, with an `UPDATE` filtered on the occurrence
//! it read; it makes the occurrence's send, through the same write
//! [`send`](fn@crate::send) uses; and it enqueues the job for the
//! occurrence after. Two workers that find one occurrence due are told
//! apart by the database: the loser's `UPDATE` matches no row, it runs
//! again, and finds nothing due. Were the claim ever bypassed, the send's
//! idempotency key — the recurrence's id and the occurrence's instant —
//! answers the second attempt as a duplicate of the first.
//!
//! **Found when due.** Creating, resuming, and every occurrence enqueue a
//! [`recur`](fn@crate::recur) job for the moment the next send is due, in
//! the transaction that sets that moment, so a recurrence that is due
//! always has a job coming. The jobs are unkeyed: a job that finds
//! nothing due does nothing, so an extra one costs one indexed read. The
//! same job also runs every hour, which picks up an occurrence whose own
//! job failed for good.
//!
//! **Catch-up.** A recurrence found due after the service was down sends
//! at most one occurrence, the latest one whose time has come, and only
//! when that is no more than [`CATCH_UP_SECS`] late. Every other missed
//! occurrence is counted as skipped.

use leptoskit_db::Db;
use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use uuid::Uuid;

use super::config::Retention;
use crate::SendSpec;
use crate::SendTarget;
use crate::recurrence::CATCH_UP_SECS;
use crate::recurrence::MAX_RECURRENCE_ERROR_LEN;
use crate::recurrence::MAX_RECURRENCES;
use crate::recurrence::MAX_RECURRENCES_PAGE;
use crate::recurrence::MAX_START_AHEAD_DAYS;
use crate::recurrence::NewRecurrence;
use crate::recurrence::NewRecurrenceRow;
use crate::recurrence::RecurrenceEnd;
use crate::recurrence::RecurrenceEntry;
use crate::recurrence::RecurrenceFilter;
use crate::recurrence::RecurrenceRow;
use crate::recurrence::RecurrenceRule;
use crate::recurrence::RecurrenceState;
use crate::recurrence::SavedRecurrence;

/// How many due recurrences one run of the job makes. More than can be
/// active at once, so one run reaches them all.
const DUE_PAGE: u64 = 2 * MAX_RECURRENCES as u64;

/// How many missed occurrences one transaction steps over. A recurrence
/// further behind than this is brought up to date over several runs.
const MOST_MISSED: u32 = 1_000;

/// A rule ready to answer when its occurrences are: its time of day, its
/// start date, and its time zone, looked up.
pub(crate) struct Schedule {
	rule: RecurrenceRule,
	start: chrono::NaiveDate,
	time: chrono::NaiveTime,
	zone: jiff::tz::TimeZone,
}

impl Schedule {
	/// The schedule of a stored rule.
	///
	/// # Errors
	///
	/// [`rpc::Error::Invalid`] when the rule is one
	/// [`check_rule`](crate::recurrence::check_rule) refuses, has no start
	/// date, or names a time zone this build's database doesn't have.
	pub(crate) fn of(rule: &RecurrenceRule) -> rpc::Result<Self> {
		crate::recurrence::check_rule(rule).map_err(rpc::Error::invalid)?;

		let time = rule.time().map_err(rpc::Error::invalid)?;
		let start = rule
			.start()
			.map_err(rpc::Error::invalid)?
			.ok_or_else(|| rpc::Error::invalid("the rule has no `starts_on`"))?;
		let zone = super::local::zone(&rule.time_zone).ok_or_else(|| unknown_zone(rule))?;

		Ok(Self {
			rule: rule.clone(),
			start,
			time,
			zone,
		})
	}

	/// The instant of the occurrence on `date`: the rule's time of day
	/// there, in the rule's time zone. A time the clocks skip that night
	/// arrives when the gap ends, as far past it as the time was into it;
	/// a time they repeat arrives the first time.
	pub(crate) fn at(&self, date: chrono::NaiveDate) -> rpc::Result<Timestamp> {
		super::local::arrives_in(&self.zone, date.and_time(self.time))
			.ok_or_else(|| rpc::Error::server(format!("{date} {} is out of range", self.rule.at)))
	}

	/// When the send of the occurrence on `date` is made: at its instant,
	/// or, for a rule that goes out at each person's own time, before the
	/// first time zone reaches it.
	pub(crate) fn due(&self, date: chrono::NaiveDate) -> rpc::Result<Timestamp> {
		if self.rule.local_time {
			return Ok(date.and_time(self.time).and_utc()
				- chrono::Duration::seconds(crate::local::EARLIEST_ZONE_SECS));
		}

		self.at(date)
	}

	/// The date it is at `now` in the rule's time zone.
	fn today(&self, now: Timestamp) -> rpc::Result<chrono::NaiveDate> {
		let out_of_range = || rpc::Error::server("the clock is out of range");
		let date = jiff::Timestamp::from_second(now.timestamp())
			.map_err(|_| out_of_range())?
			.to_zoned(self.zone.clone())
			.date();

		chrono::NaiveDate::from_ymd_opt(
			i32::from(date.year()),
			u32::try_from(date.month()).unwrap_or(0),
			u32::try_from(date.day()).unwrap_or(0),
		)
		.ok_or_else(out_of_range)
	}

	/// The first occurrence after `date`, or `None` when the rule has
	/// ended by then.
	fn next_after(&self, date: chrono::NaiveDate) -> Option<chrono::NaiveDate> {
		self.rule.next_date_after(self.start, date)
	}

	/// The first occurrence whose send is still ahead of `now`, or `None`
	/// when the rule has none left.
	pub(crate) fn first_ahead(&self, now: Timestamp) -> rpc::Result<Option<chrono::NaiveDate>> {
		// A send is due at most a day before its date, so nothing earlier
		// than the day before yesterday can still be ahead.
		let mut date = self.today(now)? - chrono::Duration::days(3);

		while let Some(next) = self.next_after(date) {
			if self.due(next)? > now {
				return Ok(Some(next));
			}

			date = next;
		}

		Ok(None)
	}
}

/// The refusal for a rule whose time zone this build doesn't know.
fn unknown_zone(rule: &RecurrenceRule) -> rpc::Error {
	rpc::Error::invalid(format!(
		"time_zone: {}, or it is one this service's time zone database doesn't have",
		crate::local::zone_complaint(&rule.time_zone)
	))
}

/// A stored date.
fn date_of(text: &str) -> rpc::Result<chrono::NaiveDate> {
	chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
		.map_err(|error| rpc::Error::server(format!("a stored date won't parse: {error}")))
}

/// The stored row of recurrence `id`.
///
/// # Errors
///
/// [`rpc::Error::NotFound`] for an unknown id, and whatever the read
/// fails with.
async fn row(db: &impl leptoskit_db::Read, id: Uuid) -> rpc::Result<RecurrenceRow> {
	RecurrenceRow::query()
		.filter(|recurrence| recurrence.id.eq(id))
		.first(db)
		.await?
		.ok_or_else(|| rpc::Error::not_found("no recurrence with this id exists"))
}

/// What a row stores as JSON: its message, its audience, and its rule. A
/// stored value that no longer parses is a bug's shape, not a caller's.
fn stored(row: &RecurrenceRow) -> rpc::Result<(SendSpec, SendTarget, RecurrenceRule)> {
	let broken = |what: &str, error: serde_json::Error| {
		rpc::Error::server(format!("a stored recurrence {what} won't parse: {error}"))
	};

	Ok((
		serde_json::from_str(&row.spec).map_err(|error| broken("message", error))?,
		serde_json::from_str(&row.target).map_err(|error| broken("target", error))?,
		serde_json::from_str(&row.rule).map_err(|error| broken("rule", error))?,
	))
}

/// The state a row stores.
fn state_of(row: &RecurrenceRow) -> RecurrenceState {
	RecurrenceState::from_str_value(&row.state).unwrap_or(RecurrenceState::Ended)
}

/// A stored count, as callers read it.
fn count_of(stored: i64) -> u32 {
	u32::try_from(stored).unwrap_or(u32::MAX)
}

/// The instant of the row's next occurrence. A rule whose time zone a
/// later build lost has none to show; the job says why when it comes due.
fn next_at(row: &RecurrenceRow, rule: &RecurrenceRule) -> Option<Timestamp> {
	let date = date_of(row.next_on.as_deref()?).ok()?;

	Schedule::of(rule).ok()?.at(date).ok()
}

/// The row as callers read it.
fn saved(row: &RecurrenceRow) -> rpc::Result<SavedRecurrence> {
	let (spec, target, rule) = stored(row)?;

	Ok(SavedRecurrence {
		id: row.id,
		state: state_of(row),
		name: row.name.clone(),
		note: row.note.clone(),
		next_at: next_at(row, &rule),
		spec,
		target,
		rule,
		last_at: row.last_at,
		last_send_key: row.last_send_key.clone(),
		sent: count_of(row.sent),
		skipped: count_of(row.skipped),
		error: row.error.clone(),
		created_at: row.created_at,
		created_by: row.created_by.clone(),
		updated_at: row.updated_at,
		updated_by: row.updated_by.clone(),
		ended_at: row.ended_at,
	})
}

/// The row as a list answers it, or `None` for a row that won't parse.
fn entry(row: &RecurrenceRow) -> Option<RecurrenceEntry> {
	let (spec, target, rule) = stored(row).ok()?;

	Some(RecurrenceEntry {
		id: row.id,
		state: state_of(row),
		name: row.name.clone(),
		category: spec.category,
		title: spec.title,
		kind: spec.options.kind,
		target: super::campaign::summary_of(&target),
		next_at: next_at(row, &rule),
		rule,
		last_at: row.last_at,
		sent: count_of(row.sent),
		skipped: count_of(row.skipped),
		updated_at: row.updated_at,
	})
}

/// How many recurrences have not ended.
async fn open(db: &impl leptoskit_db::Read) -> rpc::Result<u64> {
	Ok(RecurrenceRow::query()
		.filter(|recurrence| {
			recurrence.state.in_list([
				RecurrenceState::Active.as_str(),
				RecurrenceState::Paused.as_str(),
			])
		})
		.count(db)
		.await?)
}

/// Enqueues the job that makes whatever is due at `due`.
async fn wake_at(cx: &rpc::MutationCx, due: Timestamp) -> rpc::Result<()> {
	// Unkeyed on purpose: two recurrences due at one instant each enqueue
	// their own, and a keyed enqueue that met the other's inside this
	// transaction would refuse the whole write. A job that finds nothing
	// due does nothing.
	cx.enqueue(crate::Recur {}).at(due).await?;

	Ok(())
}

/// The body of
/// [`create_recurrence`](crate::recurrence::create_recurrence), as of
/// `now`.
///
/// # Errors
///
/// See [`create_recurrence`](crate::recurrence::create_recurrence).
pub(crate) async fn create(
	cx: &rpc::MutationCx,
	new: &NewRecurrence,
	by: &str,
	broadcast: bool,
	now: Timestamp,
) -> rpc::Result<SavedRecurrence> {
	let tx = cx.tx();

	if matches!(new.target, SendTarget::Everyone) && !broadcast {
		return Err(rpc::Error::forbidden(
			"this recurrence goes to everyone, every time: create it with `broadcast: true`, once \
			 you have checked that the person creating it may send to everyone",
		));
	}

	// The rule is stored with its start date, so "every other week" means
	// the same weeks however long the recurrence lives.
	let mut rule = new.rule.clone();
	let zone = super::local::zone(&rule.time_zone).ok_or_else(|| unknown_zone(&rule))?;

	if rule.starts_on.is_none() {
		rule.starts_on = Some(
			jiff::Timestamp::from_second(now.timestamp())
				.map_err(|_| rpc::Error::server("the clock is out of range"))?
				.to_zoned(zone)
				.date()
				.to_string(),
		);
	}

	let schedule = Schedule::of(&rule)?;

	if (schedule.start - schedule.today(now)?).num_days() > MAX_START_AHEAD_DAYS {
		return Err(rpc::Error::invalid(format!(
			"`starts_on` is at most {MAX_START_AHEAD_DAYS} days ahead"
		)));
	}

	let Some(first) = schedule.first_ahead(now)? else {
		return Err(rpc::Error::invalid(
			"this rule has no occurrence ahead: it ends before its first one",
		));
	};

	if open(tx).await? >= u64::from(MAX_RECURRENCES) {
		return Err(rpc::Error::invalid(format!(
			"there are already {MAX_RECURRENCES} recurrences that have not ended: end some before \
			 creating another"
		)));
	}

	// What every occurrence will be asked, asked once now, so a message a
	// send would refuse is refused here and not tomorrow morning.
	let id = Uuid::now_v7();
	let (key, spec) = occurrence(id, &new.spec, &schedule, first)?;
	crate::check_send_shape(&key, &spec, &new.target).map_err(rpc::Error::invalid)?;
	crate::check_timing(&spec).map_err(rpc::Error::invalid)?;

	let due = schedule.due(first)?;

	NewRecurrenceRow {
		id,
		state: RecurrenceState::Active.as_str().to_owned(),
		name: new.name.clone(),
		note: new.note.clone(),
		spec: serde_json::to_string(&new.spec).expect("the spec is plain JSON"),
		target: serde_json::to_string(&new.target).expect("the target is plain JSON"),
		rule: serde_json::to_string(&rule).expect("the rule is plain JSON"),
		next_on: Some(first.to_string()),
		due_at: Some(due),
		sent: 0,
		skipped: 0,
		last_at: None,
		last_send_key: None,
		error: None,
		created_by: by.to_owned(),
		updated_by: by.to_owned(),
		ended_at: None,
		updated_at: now,
	}
	.insert(tx)
	.await?;
	wake_at(cx, due).await?;

	saved(&row(tx, id).await?)
}

/// The body of [`recurrence`](crate::recurrence::recurrence).
///
/// # Errors
///
/// [`rpc::Error::NotFound`] for an unknown id, and whatever the read
/// fails with.
pub(crate) async fn read(db: &impl leptoskit_db::Read, id: Uuid) -> rpc::Result<SavedRecurrence> {
	saved(&row(db, id).await?)
}

/// The body of [`recurrences`](crate::recurrence::recurrences): newest
/// first by id, through the primary key, or through `(state, id)` when
/// one state is asked for.
///
/// # Errors
///
/// [`rpc::Error::Invalid`] for a cursor that isn't one of this list's,
/// and whatever the read fails with.
pub(crate) async fn list(
	db: &impl leptoskit_db::Read,
	page: &rpc::PageArgs,
	filter: RecurrenceFilter,
) -> rpc::Result<rpc::Page<RecurrenceEntry>> {
	let mut page = page.clone();
	page.limit = page.limit.clamp(1, MAX_RECURRENCES_PAGE);

	let mut query = RecurrenceRow::query();

	if let Some(state) = filter.state {
		query = query.filter(|recurrence| recurrence.state.eq(state.as_str()));
	}

	let rows = super::paginate_bounded(
		db,
		query,
		&page,
		5 * MAX_RECURRENCES_PAGE,
		|recurrence| recurrence.id.desc(),
		|recurrence| recurrence.id,
	)
	.await?;

	Ok(rpc::Page {
		items: rows.items.iter().filter_map(entry).collect(),
		end: rows.end,
		has_more: rows.has_more,
	})
}

/// The refusal for a pause or a resume of a recurrence that has ended.
fn over(doing: &str) -> rpc::Error {
	rpc::Error::conflict(format!(
		"this recurrence has ended, and can't be {doing}: create another"
	))
}

/// The body of [`pause_recurrence`](crate::recurrence::pause_recurrence).
///
/// # Errors
///
/// See [`pause_recurrence`](crate::recurrence::pause_recurrence).
pub(crate) async fn pause(
	tx: &leptoskit_db::Tx,
	id: Uuid,
	by: &str,
) -> rpc::Result<SavedRecurrence> {
	RecurrenceRow::update()
		.filter(|recurrence| {
			recurrence.id.eq(id) & recurrence.state.eq(RecurrenceState::Active.as_str())
		})
		.set(
			|recurrence| recurrence.state,
			RecurrenceState::Paused.as_str().to_owned(),
		)
		.set(|recurrence| recurrence.next_on, None::<String>)
		.set(|recurrence| recurrence.due_at, None::<Timestamp>)
		.set(|recurrence| recurrence.updated_by, by.to_owned())
		.set(|recurrence| recurrence.updated_at, leptoskit_db::now())
		.run(tx)
		.await?;

	let now = row(tx, id).await?;

	// Asked again, it answers what is there.
	if state_of(&now) == RecurrenceState::Ended {
		return Err(over("paused"));
	}

	saved(&now)
}

/// The body of
/// [`resume_recurrence`](crate::recurrence::resume_recurrence), as of
/// `now`.
///
/// # Errors
///
/// See [`resume_recurrence`](crate::recurrence::resume_recurrence).
pub(crate) async fn resume(
	cx: &rpc::MutationCx,
	id: Uuid,
	by: &str,
	now: Timestamp,
) -> rpc::Result<SavedRecurrence> {
	let tx = cx.tx();
	let found = row(tx, id).await?;

	match state_of(&found) {
		RecurrenceState::Paused => {}
		// Asked again, it answers what is there.
		RecurrenceState::Active => return saved(&found),
		RecurrenceState::Ended => return Err(over("resumed")),
	}

	let (_, _, rule) = stored(&found)?;
	let schedule = Schedule::of(&rule)?;
	let update = RecurrenceRow::update()
		.filter(|recurrence| {
			recurrence.id.eq(id) & recurrence.state.eq(RecurrenceState::Paused.as_str())
		})
		.set(|recurrence| recurrence.error, None::<String>)
		.set(|recurrence| recurrence.updated_by, by.to_owned())
		.set(|recurrence| recurrence.updated_at, now);

	// The rule may have run out while it was paused: then resuming it is
	// what ends it.
	let changed = match schedule.first_ahead(now)? {
		Some(first) => {
			let due = schedule.due(first)?;
			let changed = update
				.set(
					|recurrence| recurrence.state,
					RecurrenceState::Active.as_str().to_owned(),
				)
				.set(|recurrence| recurrence.next_on, Some(first.to_string()))
				.set(|recurrence| recurrence.due_at, Some(due))
				.run(tx)
				.await?;

			if changed == 1 {
				wake_at(cx, due).await?;
			}

			changed
		}
		None => {
			update
				.set(
					|recurrence| recurrence.state,
					RecurrenceState::Ended.as_str().to_owned(),
				)
				.set(|recurrence| recurrence.ended_at, Some(now))
				.run(tx)
				.await?
		}
	};

	// Someone else resumed or ended it since the read above: run again
	// and answer what is there now.
	if changed == 0 {
		return Err(rpc::Error::Contended);
	}

	saved(&row(tx, id).await?)
}

/// The body of [`end_recurrence`](crate::recurrence::end_recurrence).
///
/// # Errors
///
/// See [`end_recurrence`](crate::recurrence::end_recurrence).
pub(crate) async fn end(tx: &leptoskit_db::Tx, id: Uuid, by: &str) -> rpc::Result<SavedRecurrence> {
	let now = leptoskit_db::now();

	// Ending an ended recurrence changes nothing, and answers it.
	RecurrenceRow::update()
		.filter(|recurrence| {
			recurrence.id.eq(id) & recurrence.state.ne(RecurrenceState::Ended.as_str())
		})
		.set(
			|recurrence| recurrence.state,
			RecurrenceState::Ended.as_str().to_owned(),
		)
		.set(|recurrence| recurrence.next_on, None::<String>)
		.set(|recurrence| recurrence.due_at, None::<Timestamp>)
		.set(|recurrence| recurrence.ended_at, Some(now))
		.set(|recurrence| recurrence.updated_by, by.to_owned())
		.set(|recurrence| recurrence.updated_at, now)
		.run(tx)
		.await?;

	saved(&row(tx, id).await?)
}

/// The key and the message of the occurrence on `date`: the recurrence's
/// message, labelled as the recurrence's, and — for a rule that goes out
/// at each person's own time — carrying that date and time as its local
/// time, with the rule's time zone for a person whose own isn't known.
fn occurrence(
	id: Uuid,
	spec: &SendSpec,
	schedule: &Schedule,
	date: chrono::NaiveDate,
) -> rpc::Result<(String, SendSpec)> {
	let mut spec = spec.clone();
	spec.label = Some(crate::recurrence::recurrence_label(id));

	if schedule.rule.local_time {
		spec.local_time = Some(crate::LocalTime {
			at: format!("{date}T{}", schedule.rule.at),
			default_time_zone: Some(schedule.rule.time_zone.clone()),
		});
	}

	Ok((
		crate::recurrence::occurrence_key(id, schedule.at(date)?),
		spec,
	))
}

/// What became of one recurrence found due.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ran {
	/// Occurrences whose send was made.
	pub sent: u32,
	/// Occurrences stepped over without a send: missed while the service
	/// was down, too late to send, or refused.
	pub skipped: u32,
	/// Recurrences the service paused because an occurrence's send was
	/// refused.
	pub paused: u32,
	/// Recurrences whose rule ended.
	pub ended: u32,
}

impl Ran {
	fn add(&mut self, other: Self) {
		self.sent += other.sent;
		self.skipped += other.skipped;
		self.paused += other.paused;
		self.ended += other.ended;
	}

	/// Whether nothing was due.
	#[must_use]
	pub fn is_empty(&self) -> bool {
		*self == Self::default()
	}
}

/// One due recurrence's turn, as the mutation [`run`] runs for it: the
/// claim, the send, and the next job, in one transaction, retried when
/// another gets in its way. Called in process only, never routed.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Occur {
	id: Uuid,
	now: Timestamp,
}

impl rpc::Operation for Occur {
	type Context = rpc::MutationCx;
	type Output = Ran;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	const NAME: &'static str = "recur";
	const SERVICE: Option<&'static str> = Some(crate::SERVICE);

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<Self::Output>> + Send {
		occur(cx, self.id, self.now)
	}
}

/// A send's refusal that would be the same refusal tomorrow: the message,
/// the audience, or the key's history, not the database.
fn refusal(error: &rpc::Error) -> Option<&str> {
	match error {
		rpc::Error::Invalid(reason)
		| rpc::Error::Forbidden(reason)
		| rpc::Error::NotFound(reason)
		| rpc::Error::Conflict(reason) => Some(reason),
		_ => None,
	}
}

/// `text`, cut to at most `most` bytes at a character boundary.
fn cut(text: &str, most: usize) -> &str {
	let mut end = text.len().min(most);

	while !text.is_char_boundary(end) {
		end -= 1;
	}

	&text[..end]
}

/// What a due recurrence owes as of some moment: the one occurrence it
/// may still send, and what it steps over to get there.
struct Owed {
	/// The latest occurrence whose send is due.
	date: chrono::NaiveDate,
	/// How many occurrences before it were missed. They are never sent.
	missed: u32,
	/// Whether `date`'s send is no more than [`CATCH_UP_SECS`] late. When
	/// it is later than that, it is not sent either.
	on_time: bool,
	/// The occurrence after `date`, when the rule has one.
	following: Option<chrono::NaiveDate>,
}

impl Owed {
	/// What a recurrence whose next occurrence is on `next_on` owes as of
	/// `now`, or `None` when that occurrence's send is not due yet.
	fn of(
		schedule: &Schedule,
		next_on: chrono::NaiveDate,
		now: Timestamp,
	) -> rpc::Result<Option<Self>> {
		let mut date = next_on;

		if schedule.due(date)? > now {
			return Ok(None);
		}

		let mut missed: u32 = 0;

		while missed < MOST_MISSED
			&& let Some(later) = schedule.next_after(date)
			&& schedule.due(later)? <= now
		{
			missed += 1;
			date = later;
		}

		Ok(Some(Self {
			date,
			missed,
			on_time: (now - schedule.due(date)?).num_seconds() <= CATCH_UP_SECS,
			following: schedule.next_after(date),
		}))
	}

	/// How many occurrences this turn does not send.
	fn skipped(&self) -> u32 {
		self.missed + u32::from(!self.on_time)
	}
}

/// One due recurrence's turn: the row as this transaction read it, and
/// the writes that are fenced on it.
struct Turn<'a> {
	cx: &'a rpc::MutationCx,
	found: RecurrenceRow,
	/// The occurrence the row was at when it was read: what every write
	/// of the turn is filtered on.
	next_on: String,
	now: Timestamp,
}

impl Turn<'_> {
	/// An update of the recurrence that matches only while it is still
	/// active and still at the occurrence this turn read. Another turn
	/// that got there first has moved it on, and this one changes nothing.
	fn claim(&self) -> leptoskit_db::Update<RecurrenceRow> {
		let id = self.found.id;
		let next_on = self.next_on.clone();

		RecurrenceRow::update()
			.filter(|recurrence| {
				recurrence.id.eq(id)
					& recurrence.state.eq(RecurrenceState::Active.as_str())
					& recurrence.next_on.eq(Some(next_on))
			})
			.set(|recurrence| recurrence.updated_at, self.now)
	}

	/// Pauses the recurrence because its occurrence can't be sent, with
	/// `reason` stored for a person to read. The occurrence counts as
	/// skipped, and so do the `missed` ones before it.
	///
	/// # Errors
	///
	/// [`rpc::Error::Contended`] when another turn claimed the occurrence
	/// first, and whatever the write fails with.
	async fn pause(&self, reason: &str, missed: u32) -> rpc::Result<Ran> {
		let paused = self
			.claim()
			.set(
				|recurrence| recurrence.state,
				RecurrenceState::Paused.as_str().to_owned(),
			)
			.set(|recurrence| recurrence.next_on, None::<String>)
			.set(|recurrence| recurrence.due_at, None::<Timestamp>)
			.set(
				|recurrence| recurrence.skipped,
				self.found.skipped + i64::from(missed) + 1,
			)
			.set(
				|recurrence| recurrence.error,
				Some(cut(reason, MAX_RECURRENCE_ERROR_LEN).to_owned()),
			)
			.run(self.cx.tx())
			.await?;

		if paused == 0 {
			return Err(rpc::Error::Contended);
		}

		Ok(Ran {
			skipped: missed + 1,
			paused: 1,
			..Ran::default()
		})
	}

	/// The claim that moves the recurrence past what it `owed`: on to the
	/// following occurrence, or to its end when the rule has none left or
	/// `sent` sends are as many as it allows. Answers the update, not yet
	/// run, and when the following occurrence's send is due.
	fn move_on(
		&self,
		schedule: &Schedule,
		owed: &Owed,
		sent: i64,
	) -> rpc::Result<(leptoskit_db::Update<RecurrenceRow>, Option<Timestamp>)> {
		let update = self.claim().set(
			|recurrence| recurrence.skipped,
			self.found.skipped + i64::from(owed.skipped()),
		);
		let more = match schedule.rule.ends {
			Some(RecurrenceEnd::After(count)) => sent < i64::from(count),
			_ => true,
		};

		Ok(match owed.following {
			Some(following) if more => {
				let due = schedule.due(following)?;

				(
					update
						.set(|recurrence| recurrence.next_on, Some(following.to_string()))
						.set(|recurrence| recurrence.due_at, Some(due)),
					Some(due),
				)
			}
			_ => {
				(
					update
						.set(
							|recurrence| recurrence.state,
							RecurrenceState::Ended.as_str().to_owned(),
						)
						.set(|recurrence| recurrence.next_on, None::<String>)
						.set(|recurrence| recurrence.due_at, None::<Timestamp>)
						.set(|recurrence| recurrence.ended_at, Some(self.now)),
					None,
				)
			}
		})
	}

	/// Finishes a turn whose claim was written: the job for the next
	/// occurrence, or the count of a recurrence that ended.
	async fn finish(&self, mut ran: Ran, due: Option<Timestamp>) -> rpc::Result<Ran> {
		match due {
			Some(due) => wake_at(self.cx, due).await?,
			None => ran.ended = 1,
		}

		Ok(ran)
	}

	/// Puts the recurrence back where this turn found it and pauses it,
	/// after the turn's claim was written and its send was then refused.
	///
	/// # Errors
	///
	/// Whatever the write fails with.
	async fn undo(&self, reason: &str, missed: u32) -> rpc::Result<Ran> {
		let id = self.found.id;

		RecurrenceRow::update()
			.filter(|recurrence| recurrence.id.eq(id))
			.set(
				|recurrence| recurrence.state,
				RecurrenceState::Paused.as_str().to_owned(),
			)
			.set(|recurrence| recurrence.next_on, None::<String>)
			.set(|recurrence| recurrence.due_at, None::<Timestamp>)
			.set(|recurrence| recurrence.ended_at, None::<Timestamp>)
			.set(|recurrence| recurrence.sent, self.found.sent)
			.set(
				|recurrence| recurrence.skipped,
				self.found.skipped + i64::from(missed) + 1,
			)
			.set(|recurrence| recurrence.last_at, self.found.last_at)
			.set(
				|recurrence| recurrence.last_send_key,
				self.found.last_send_key.clone(),
			)
			.set(
				|recurrence| recurrence.error,
				Some(cut(reason, MAX_RECURRENCE_ERROR_LEN).to_owned()),
			)
			.run(self.cx.tx())
			.await?;

		Ok(Ran {
			skipped: missed + 1,
			paused: 1,
			..Ran::default()
		})
	}
}

/// The body of [`Occur`]: what recurrence `id` owes as of `now`.
///
/// # Errors
///
/// [`rpc::Error::Contended`] when another run claimed the occurrence
/// first — the mutation runs again and finds nothing due — and whatever a
/// read, a write, or an enqueue fails with. A send that is refused is not
/// an error here: it pauses the recurrence, with the refusal stored.
async fn occur(cx: &rpc::MutationCx, id: Uuid, now: Timestamp) -> rpc::Result<Ran> {
	let Some(found) = RecurrenceRow::query()
		.filter(|recurrence| recurrence.id.eq(id))
		.first(cx.tx())
		.await?
	else {
		return Ok(Ran::default());
	};
	let (Some(next_on), Some(due_at)) = (found.next_on.clone(), found.due_at) else {
		return Ok(Ran::default());
	};

	if state_of(&found) != RecurrenceState::Active || due_at > now {
		return Ok(Ran::default());
	}

	let (spec, target, rule) = stored(&found)?;
	let turn = Turn {
		cx,
		found,
		next_on,
		now,
	};
	// A time zone a later build lost: nothing can say when the occurrence
	// is, so the recurrence waits for a person.
	let schedule = match Schedule::of(&rule) {
		Ok(schedule) => schedule,
		Err(error) => {
			return match refusal(&error) {
				Some(reason) => turn.pause(reason, 0).await,
				None => Err(error),
			};
		}
	};
	let Some(owed) = Owed::of(&schedule, date_of(&turn.next_on)?, now)? else {
		return Ok(Ran::default());
	};
	let ran = Ran {
		skipped: owed.skipped(),
		..Ran::default()
	};

	// Too late to send: the recurrence only moves on.
	if !owed.on_time {
		let (update, due) = turn.move_on(&schedule, &owed, turn.found.sent)?;

		if update.run(cx.tx()).await? == 0 {
			return Err(rpc::Error::Contended);
		}

		return turn.finish(ran, due).await;
	}

	let (key, spec) = occurrence(id, &spec, &schedule, owed.date)?;

	// What `send` checks before its write, checked the same way here.
	if let Err(reason) = crate::check_send_shape(&key, &spec, &target) {
		return turn.pause(&reason, owed.missed).await;
	}

	// The claim: the recurrence moves on from the occurrence this turn
	// read, or this turn lost it to another and makes nothing.
	let sent = turn.found.sent + 1;
	let (update, due) = turn.move_on(&schedule, &owed, sent)?;
	let claimed = update
		.set(|recurrence| recurrence.sent, sent)
		.set(
			|recurrence| recurrence.last_at,
			Some(schedule.at(owed.date)?),
		)
		.set(|recurrence| recurrence.last_send_key, Some(key.clone()))
		.run(cx.tx())
		.await?;

	if claimed == 0 {
		return Err(rpc::Error::Contended);
	}

	// The send itself, through the write `send` makes: its stored-send
	// lookup, its checks, the opt-outs, the fanout or its first job, the
	// wakes, and the pushes.
	if let Err(error) = Box::pin(crate::send_write(cx, key, spec, target)).await {
		return match refusal(&error) {
			Some(reason) => turn.undo(reason, owed.missed).await,
			None => Err(error),
		};
	}

	turn.finish(Ran { sent: 1, ..ran }, due).await
}

/// Makes every occurrence that is due as of `now`: the body of the
/// [`recur`](fn@crate::recur) job, and what a test calls with a `now` of
/// its own instead of waiting. `push` is the service's push
/// configuration, when it has one: with it, each send queues its pushes
/// as a send from the app's server does.
///
/// Each recurrence is its own transaction, so one that fails holds up no
/// other. Running it twice at once, or again after a crash, makes no
/// occurrence twice.
///
/// # Errors
///
/// The first failure of a read or a write, after every other due
/// recurrence had its turn. What was made stays made; running it again
/// picks up what is left.
pub async fn run(db: &Db, push: Option<&super::push::Push>, now: Timestamp) -> rpc::Result<Ran> {
	let due: Vec<Uuid> = RecurrenceRow::query()
		.filter(|recurrence| {
			recurrence.state.eq(RecurrenceState::Active.as_str()) & recurrence.due_at.lte(now)
		})
		.order_by(|recurrence| recurrence.due_at.asc())
		.limit(DUE_PAGE)
		.all(db)
		.await?
		.into_iter()
		.map(|recurrence| recurrence.id)
		.collect();

	// The send's write is the app's own, so it runs as the app, carrying
	// the push configuration the way a request to the service does.
	let mut identity = rpc::server::Identity::anonymous().with_caller(rpc::server::APP_CALLER);

	if let Some(push) = push {
		identity.context = identity.context.with(push.clone());
	}

	let mut ran = Ran::default();
	let mut failed = None;

	for id in due {
		match rpc::server::call_as(db, identity.clone(), None, Occur { id, now }).await {
			Ok(one) => ran.add(one),
			Err(error) => {
				failed.get_or_insert(error);
			}
		}
	}

	match failed {
		Some(error) => Err(error),
		None => Ok(ran),
	}
}

/// Deletes the recurrences that ended more than `retention.sends_days`
/// ago, as of `now`, and answers how many went. An ended recurrence is a
/// record, like a send's, and holds a message and an audience: it is kept
/// as long as a send's record is, and no longer.
///
/// # Errors
///
/// Whatever the write fails with.
pub async fn sweep(db: &Db, retention: &Retention, now: Timestamp) -> rpc::Result<u64> {
	let Some(cutoff) = retention
		.sends_days
		.and_then(|days| now.checked_sub_signed(chrono::Duration::days(i64::from(days))))
	else {
		return Ok(0);
	};

	Ok(RecurrenceRow::query()
		.filter(|recurrence| {
			recurrence.state.eq(RecurrenceState::Ended.as_str()) & recurrence.ended_at.lte(cutoff)
		})
		.delete(db)
		.await?)
}

/// Takes `person` out of every stored recurrence that names them as a
/// recipient, as part of [`forget_person`](crate::forget_person): the
/// stored audience no longer holds their id, and a recurrence left with
/// nobody to send to ends.
///
/// A condition's text is not searched: what a condition says about a
/// person is the app's own text, as it is in a stored send.
///
/// # Errors
///
/// Whatever a read or a write fails with.
pub(crate) async fn forget(tx: &leptoskit_db::Tx, person: Uuid) -> rpc::Result<()> {
	let named = person.to_string();
	let rows = RecurrenceRow::query()
		.filter(|recurrence| recurrence.target.contains(named.as_str()))
		.all(tx)
		.await?;
	let now = leptoskit_db::now();

	for found in rows {
		let Ok(SendTarget::Recipients(recipients)) =
			serde_json::from_str::<SendTarget>(&found.target)
		else {
			continue;
		};
		let left: Vec<Uuid> = recipients
			.into_iter()
			.filter(|recipient| *recipient != person)
			.collect();
		let nobody = left.is_empty();
		let update = RecurrenceRow::update()
			.filter(|recurrence| recurrence.id.eq(found.id))
			.set(
				|recurrence| recurrence.target,
				serde_json::to_string(&SendTarget::Recipients(left))
					.expect("the target is plain JSON"),
			)
			.set(|recurrence| recurrence.updated_at, now);

		if nobody && state_of(&found) != RecurrenceState::Ended {
			update
				.set(
					|recurrence| recurrence.state,
					RecurrenceState::Ended.as_str().to_owned(),
				)
				.set(|recurrence| recurrence.next_on, None::<String>)
				.set(|recurrence| recurrence.due_at, None::<Timestamp>)
				.set(|recurrence| recurrence.ended_at, Some(now))
				.set(
					|recurrence| recurrence.error,
					Some("everyone it was for has been forgotten".to_owned()),
				)
				.run(tx)
				.await?;
		} else {
			update.run(tx).await?;
		}
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::recurrence::Repeat;
	use crate::recurrence::Weekday;

	fn day(text: &str) -> chrono::NaiveDate {
		date_of(text).expect("a date")
	}

	fn instant(text: &str) -> Timestamp {
		chrono::DateTime::parse_from_rfc3339(text)
			.expect("an instant")
			.to_utc()
	}

	fn schedule(repeat: Repeat, at: &str, zone: &str, starts_on: &str) -> Schedule {
		Schedule::of(&RecurrenceRule {
			repeat,
			at: at.into(),
			time_zone: zone.into(),
			starts_on: Some(starts_on.into()),
			ends: None,
			local_time: false,
		})
		.expect("a schedule")
	}

	/// A daily 09:00 in New York is 14:00 UTC in winter and 13:00 UTC in
	/// summer: the wall clock holds, the instant moves.
	#[test]
	fn the_wall_clock_holds_across_a_daylight_saving_change() {
		let daily = schedule(
			Repeat::Daily { every: 1 },
			"09:00",
			"America/New_York",
			"2027-03-01",
		);

		assert_eq!(
			daily.at(day("2027-03-13")).unwrap(),
			instant("2027-03-13T14:00:00Z")
		);
		assert_eq!(
			daily.at(day("2027-03-14")).unwrap(),
			instant("2027-03-14T13:00:00Z"),
			"the clocks went forward that night"
		);
		assert_eq!(
			daily.at(day("2027-11-07")).unwrap(),
			instant("2027-11-07T14:00:00Z"),
			"and back on this one"
		);
	}

	/// 02:30 does not exist in New York on 14 March 2027: the clocks jump
	/// from 02:00 to 03:00. The occurrence goes out at 03:30, once, and
	/// the days either side are at 02:30 as written.
	#[test]
	fn a_skipped_time_goes_out_when_the_gap_ends() {
		let nightly = schedule(
			Repeat::Daily { every: 1 },
			"02:30",
			"America/New_York",
			"2027-03-01",
		);

		assert_eq!(
			nightly.at(day("2027-03-13")).unwrap(),
			instant("2027-03-13T07:30:00Z"),
			"02:30 EST"
		);
		assert_eq!(
			nightly.at(day("2027-03-14")).unwrap(),
			instant("2027-03-14T07:30:00Z"),
			"03:30 EDT, an hour after 02:00 EST"
		);
		assert_eq!(
			nightly.at(day("2027-03-15")).unwrap(),
			instant("2027-03-15T06:30:00Z"),
			"02:30 EDT"
		);
	}

	/// 01:30 happens twice in New York on 7 November 2027: the clocks fall
	/// back from 02:00 to 01:00. The occurrence is the first 01:30, and
	/// the next occurrence is the next day's, not the second 01:30.
	#[test]
	fn a_repeated_time_goes_out_the_first_time_only() {
		let nightly = schedule(
			Repeat::Daily { every: 1 },
			"01:30",
			"America/New_York",
			"2027-11-01",
		);

		assert_eq!(
			nightly.at(day("2027-11-07")).unwrap(),
			instant("2027-11-07T05:30:00Z"),
			"01:30 EDT, the first one"
		);
		assert_eq!(
			nightly.next_after(day("2027-11-07")),
			Some(day("2027-11-08")),
			"one occurrence per date"
		);
		assert_eq!(
			nightly.at(day("2027-11-08")).unwrap(),
			instant("2027-11-08T06:30:00Z"),
			"01:30 EST"
		);
	}

	/// The first occurrence is the first one still ahead: today's when its
	/// time has not come, tomorrow's when it has.
	#[test]
	fn the_first_occurrence_is_the_first_still_ahead() {
		let daily = schedule(
			Repeat::Daily { every: 1 },
			"09:00",
			"Europe/London",
			"2027-01-01",
		);

		assert_eq!(
			daily.first_ahead(instant("2027-06-10T07:59:59Z")).unwrap(),
			Some(day("2027-06-10")),
			"08:59:59 in London, in summer"
		);
		assert_eq!(
			daily.first_ahead(instant("2027-06-10T08:00:00Z")).unwrap(),
			Some(day("2027-06-11")),
			"09:00 exactly has come"
		);

		// A start date ahead is where it begins.
		let later = schedule(
			Repeat::Weekly {
				every: 1,
				on: vec![Weekday::Monday],
			},
			"09:00",
			"UTC",
			"2027-07-01",
		);
		assert_eq!(
			later.first_ahead(instant("2027-06-10T00:00:00Z")).unwrap(),
			Some(day("2027-07-05"))
		);
	}

	/// A rule that goes out at each person's own time is due fourteen
	/// hours before its wall-clock time read as UTC, whatever its own
	/// zone: in time for the first zone the time reaches.
	#[test]
	fn a_local_time_rule_is_due_before_the_first_zone() {
		let mut local = schedule(
			Repeat::Daily { every: 1 },
			"09:00",
			"America/New_York",
			"2027-01-01",
		);
		local.rule.local_time = true;

		assert_eq!(
			local.due(day("2027-06-10")).unwrap(),
			instant("2027-06-09T19:00:00Z")
		);
		assert_eq!(
			local.at(day("2027-06-10")).unwrap(),
			instant("2027-06-10T13:00:00Z"),
			"its instant is still the rule's own zone's"
		);
		assert_eq!(
			local.first_ahead(instant("2027-06-09T19:00:00Z")).unwrap(),
			Some(day("2027-06-11")),
			"the send for the 10th is due, so the 11th is the first ahead"
		);
	}

	#[test]
	fn an_unknown_time_zone_is_refused_by_name() {
		let error = Schedule::of(&RecurrenceRule {
			repeat: Repeat::Daily { every: 1 },
			at: "09:00".into(),
			time_zone: "Mars/Olympus".into(),
			starts_on: Some("2027-01-01".into()),
			ends: None,
			local_time: false,
		})
		.err()
		.expect("refused");

		assert!(
			matches!(&error, rpc::Error::Invalid(reason) if reason.contains("time_zone") && reason.contains("Mars/Olympus")),
			"{error}"
		);
	}
}
