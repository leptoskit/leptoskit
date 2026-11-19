//! Resolving a send per person: which [variant](crate::local::Variant)
//! their row is written in, and the instant a
//! [local time](crate::local::LocalTime) arrives where they are.
//!
//! One rule decides both, per person: the `locale` or `time_zone`
//! attribute the app set wins, then the person's most recently seen live
//! endpoint that says, then the send's default. A whole batch is resolved
//! with two reads — the batch's attributes, and its live endpoints — and a
//! send with neither variants nor a local time is resolved with none.
//!
//! Time zones are IANA names looked up in the database this build
//! carries. A name it doesn't know — a device whose tzdata is newer than
//! the service's, an attribute holding something else — is treated as
//! absent and the rule moves on: it never fails a send.

use std::collections::BTreeSet;
use std::collections::HashMap;

use leptoskit_rpc as rpc;
use uuid::Uuid;

use crate::PersonAttribute;
use crate::local::LOCALE_ATTRIBUTE;
use crate::local::LocalTime;
use crate::local::TIME_ZONE_ATTRIBUTE;
use crate::push::PushEndpoint;

/// The time zone `name` names, when this build's database knows it. The
/// lookup ignores case. `Etc/Unknown`, the name a device reports when it
/// doesn't know its own zone, is not one.
pub(crate) fn zone(name: &str) -> Option<jiff::tz::TimeZone> {
	let name = name.trim();

	if name.is_empty() || name.len() > crate::push::MAX_TIME_ZONE_LEN {
		return None;
	}

	jiff::tz::TimeZone::get(name)
		.ok()
		.filter(|zone| !zone.is_unknown())
}

/// The instant the wall-clock time `wall` arrives in `zone`.
///
/// A time the zone's clocks skip — the hour a spring-forward removes —
/// arrives when the gap ends, as far past it as the time was into it:
/// 02:30 on a night that jumps from 02:00 to 03:00 arrives at 03:30. A
/// time the clocks repeat — the hour an autumn fold replays — arrives the
/// first time.
pub(crate) fn arrives_in(
	zone: &jiff::tz::TimeZone,
	wall: chrono::NaiveDateTime,
) -> Option<leptoskit_db::Timestamp> {
	use chrono::Datelike as _;
	use chrono::Timelike as _;

	let civil = jiff::civil::DateTime::new(
		i16::try_from(wall.year()).ok()?,
		i8::try_from(wall.month()).ok()?,
		i8::try_from(wall.day()).ok()?,
		i8::try_from(wall.hour()).ok()?,
		i8::try_from(wall.minute()).ok()?,
		i8::try_from(wall.second()).ok()?,
		0,
	)
	.ok()?;
	let instant = zone.to_ambiguous_timestamp(civil).compatible().ok()?;

	chrono::DateTime::from_timestamp(instant.as_second(), 0)
}

/// Checks the part of a send's local time only the service can: that its
/// default time zone is one the time zone database knows. A new send and a
/// dry run are held to it; a send already accepted is not asked again.
///
/// # Errors
///
/// [`rpc::Error::Invalid`], naming the zone.
pub(crate) fn check_default_zone(spec: &crate::SendSpec) -> rpc::Result<()> {
	let Some(local) = spec.local_time.as_ref() else {
		return Ok(());
	};

	if zone(local.default_zone()).is_none() {
		return Err(rpc::Error::invalid(format!(
			"local_time.default_time_zone: {}, or it is one this service's time zone database \
			 doesn't have",
			crate::local::zone_complaint(local.default_zone())
		)));
	}

	Ok(())
}

/// What one of a person's endpoints says about them, as the rule reads
/// it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Seen<'a> {
	pub(crate) locale: Option<&'a str>,
	pub(crate) time_zone: Option<&'a str>,
}

/// The person's locale: the attribute when it is a BCP 47 tag, then the
/// first of `devices` — most recently seen first — that reports one.
/// `None` reads the send's own text.
pub(crate) fn locale_of(attribute: Option<&str>, devices: &[Seen<'_>]) -> Option<String> {
	let tag = |value: &str| crate::push::canonical_locale(value).ok();

	attribute.and_then(tag).or_else(|| {
		devices
			.iter()
			.find_map(|device| device.locale.and_then(tag))
	})
}

/// When a send's wall-clock time arrives, zone by zone: each zone is
/// looked up and converted once however many people are in it.
pub(crate) struct Arrivals {
	wall: chrono::NaiveDateTime,
	earliest: leptoskit_db::Timestamp,
	latest: leptoskit_db::Timestamp,
	fallback: leptoskit_db::Timestamp,
	zones: HashMap<String, Option<leptoskit_db::Timestamp>>,
}

impl Arrivals {
	/// The arrivals of `local`'s wall-clock time. Its default zone falls
	/// back to UTC when this build doesn't know it — a send accepted by a
	/// build that did still goes out.
	///
	/// # Errors
	///
	/// [`rpc::Error::server`] when a stored local time no longer parses,
	/// which is a bug's shape: it was checked when the send was made.
	pub(crate) fn of(local: &LocalTime) -> rpc::Result<Self> {
		let broken = |error: String| {
			rpc::Error::server(format!("a stored local time no longer parses: {error}"))
		};
		let wall = local.wall_clock().map_err(broken)?;
		let mut arrivals = Self {
			wall,
			earliest: local.earliest().map_err(broken)?,
			latest: local.latest().map_err(broken)?,
			fallback: wall.and_utc(),
			zones: HashMap::new(),
		};

		if let Some(fallback) = arrivals.in_zone(local.default_zone()) {
			arrivals.fallback = fallback;
		}

		Ok(arrivals)
	}

	/// When the wall-clock time arrives in the zone `name` names, or
	/// `None` when this build's database has no such zone. Held inside
	/// the span every real zone falls in, so the send's schedule limit
	/// and expiry check hold for every row.
	fn in_zone(&mut self, name: &str) -> Option<leptoskit_db::Timestamp> {
		if let Some(known) = self.zones.get(name) {
			return *known;
		}

		let arrives = zone(name)
			.and_then(|zone| arrives_in(&zone, self.wall))
			.map(|instant| instant.clamp(self.earliest, self.latest));
		self.zones.insert(name.to_owned(), arrives);

		arrives
	}

	/// When the wall-clock time arrives for one person: in the zone their
	/// attribute names when the database knows it, then in the first known
	/// zone among `devices` — most recently seen first — then in the
	/// send's default zone.
	pub(crate) fn for_person(
		&mut self,
		attribute: Option<&str>,
		devices: &[Seen<'_>],
	) -> leptoskit_db::Timestamp {
		attribute
			.and_then(|name| self.in_zone(name))
			.or_else(|| {
				devices
					.iter()
					.find_map(|device| device.time_zone.and_then(|name| self.in_zone(name)))
			})
			.unwrap_or(self.fallback)
	}
}

/// What a send resolved to for one batch of people.
#[derive(Debug)]
pub(crate) struct Resolved {
	/// The moment the batch was resolved at: a row due by then is visible
	/// as it is written.
	now: leptoskit_db::Timestamp,
	/// When a row is visible for a person the map below doesn't name: the
	/// send's one instant, never before `now`.
	shared: leptoskit_db::Timestamp,
	/// The variant key and, for a local-time send, the arrival of each
	/// person the send resolved one for.
	people: HashMap<Uuid, (Option<String>, Option<leptoskit_db::Timestamp>)>,
}

impl Resolved {
	/// A send with no variants and no local time: every row is the spec's
	/// own text at the spec's own instant.
	fn shared(spec: &crate::SendSpec, now: leptoskit_db::Timestamp) -> Self {
		Self {
			now,
			// An earlier time is now: the row says when it really became
			// visible, so the inbox and the push's delivery window agree
			// on it.
			shared: spec
				.visible_at
				.map_or(now, |visible_at| visible_at.max(now)),
			people: HashMap::new(),
		}
	}

	/// When `person`'s row becomes visible: their local time's arrival —
	/// now, when it has already passed — or the send's own instant.
	pub(crate) fn visible_at(&self, person: Uuid) -> leptoskit_db::Timestamp {
		self.people
			.get(&person)
			.and_then(|(_, arrives)| *arrives)
			.map_or(self.shared, |arrives| arrives.max(self.now))
	}

	/// The instant `person`'s push waits for, when their local time is
	/// still ahead: what their deliveries are queued for, and what their
	/// wake is enqueued at. `None` for a row visible as it is written, and
	/// for every row of a send with no local time, whose one schedule the
	/// send's own jobs carry.
	pub(crate) fn waits_until(&self, person: Uuid) -> Option<leptoskit_db::Timestamp> {
		self.people
			.get(&person)
			.and_then(|(_, arrives)| *arrives)
			.filter(|arrives| *arrives > self.now)
	}

	/// The key of the variant `person`'s row is written in, or `None` for
	/// the send's own text.
	pub(crate) fn variant(&self, person: Uuid) -> Option<&str> {
		self.people
			.get(&person)
			.and_then(|(variant, _)| variant.as_deref())
	}
}

/// Resolves `spec` for `people` at `now`: nothing to read for a send with
/// no variants and no local time, and otherwise one read of the batch's
/// `locale` and `time_zone` attributes and one of its live endpoints.
///
/// # Errors
///
/// Whatever a read fails with, and [`rpc::Error::server`] for a stored
/// local time that no longer parses.
pub(crate) async fn resolve(
	db: &impl leptoskit_db::Read,
	spec: &crate::SendSpec,
	people: &BTreeSet<Uuid>,
	now: leptoskit_db::Timestamp,
) -> rpc::Result<Resolved> {
	let mut resolved = Resolved::shared(spec, now);
	let translated = !spec.variants.is_empty();
	let mut arrivals = spec.local_time.as_ref().map(Arrivals::of).transpose()?;

	if people.is_empty() || (!translated && arrivals.is_none()) {
		return Ok(resolved);
	}

	let known = known(db, people, translated, arrivals.is_some()).await?;
	// Each distinct locale is matched against the variants once.
	let mut matches: HashMap<String, Option<String>> = HashMap::new();

	for person in people {
		let attributes = known.attributes.get(person);
		let devices: Vec<Seen<'_>> = known
			.endpoints
			.get(person)
			.into_iter()
			.flatten()
			.map(|endpoint| {
				Seen {
					locale: endpoint.locale.as_deref(),
					time_zone: endpoint.time_zone.as_deref(),
				}
			})
			.collect();
		let attribute = |key: &str| attributes.and_then(|held| held.get(key).map(String::as_str));

		let variant = if translated {
			locale_of(attribute(LOCALE_ATTRIBUTE), &devices).and_then(|locale| {
				matches
					.entry(locale)
					.or_insert_with_key(|locale| spec.variant_for(locale).map(str::to_owned))
					.clone()
			})
		} else {
			None
		};
		let arrives = arrivals
			.as_mut()
			.map(|arrivals| arrivals.for_person(attribute(TIME_ZONE_ATTRIBUTE), &devices));

		resolved.people.insert(*person, (variant, arrives));
	}

	Ok(resolved)
}

/// The locale [`send_test`](crate::send_test) reads its spec's variants
/// with for `person`: the same rule, for one person.
///
/// # Errors
///
/// Whatever a read fails with.
pub(crate) async fn locale_for(
	db: &impl leptoskit_db::Read,
	person: Uuid,
) -> rpc::Result<Option<String>> {
	let people = BTreeSet::from([person]);
	let known = known(db, &people, true, false).await?;
	let devices: Vec<Seen<'_>> = known
		.endpoints
		.get(&person)
		.into_iter()
		.flatten()
		.map(|endpoint| {
			Seen {
				locale: endpoint.locale.as_deref(),
				time_zone: None,
			}
		})
		.collect();

	Ok(locale_of(
		known
			.attributes
			.get(&person)
			.and_then(|held| held.get(LOCALE_ATTRIBUTE).map(String::as_str)),
		&devices,
	))
}

/// What the service knows about a batch of people, for the rule to read.
struct Known {
	/// The `locale` and `time_zone` attributes the app set, by person.
	attributes: HashMap<Uuid, HashMap<String, String>>,
	/// Each person's live endpoints, most recently seen first.
	endpoints: HashMap<Uuid, Vec<PushEndpoint>>,
}

/// Reads what the rule needs about `people`, each read once for the whole
/// batch.
///
/// # Errors
///
/// Whatever a read fails with.
async fn known(
	db: &impl leptoskit_db::Read,
	people: &BTreeSet<Uuid>,
	locale: bool,
	time_zone: bool,
) -> rpc::Result<Known> {
	let keys: Vec<String> = [
		locale.then_some(LOCALE_ATTRIBUTE),
		time_zone.then_some(TIME_ZONE_ATTRIBUTE),
	]
	.into_iter()
	.flatten()
	.map(str::to_owned)
	.collect();
	let mut attributes: HashMap<Uuid, HashMap<String, String>> = HashMap::new();

	for row in PersonAttribute::query()
		.filter(|attribute| {
			attribute.person.in_list(people.iter().copied()) & attribute.key.in_list(keys.clone())
		})
		.all(db)
		.await?
	{
		attributes
			.entry(row.person)
			.or_default()
			.insert(row.key, row.value);
	}

	let mut endpoints = super::push::live_endpoints(db, people.iter().copied()).await?;

	for held in endpoints.values_mut() {
		// Most recently seen first; an endpoint enrolled before `seen_at`
		// existed was last seen when it was created. The id — a UUID v7 —
		// settles a tie the same way every time.
		held.sort_by_key(|endpoint| {
			std::cmp::Reverse((endpoint.seen_at.unwrap_or(endpoint.created_at), endpoint.id))
		});
	}

	Ok(Known {
		attributes,
		endpoints,
	})
}

#[cfg(test)]
mod tests {
	use super::Arrivals;
	use super::Seen;
	use super::arrives_in;
	use super::locale_of;
	use super::zone;
	use crate::local::LocalTime;

	fn instant(rfc3339: &str) -> leptoskit_db::Timestamp {
		chrono::DateTime::parse_from_rfc3339(rfc3339)
			.unwrap()
			.with_timezone(&chrono::Utc)
	}

	fn wall(at: &str) -> chrono::NaiveDateTime {
		LocalTime {
			at: at.into(),
			default_time_zone: None,
		}
		.wall_clock()
		.unwrap()
	}

	fn arrivals(at: &str, default_time_zone: Option<&str>) -> Arrivals {
		Arrivals::of(&LocalTime {
			at: at.into(),
			default_time_zone: default_time_zone.map(str::to_owned),
		})
		.unwrap()
	}

	fn in_zone(time_zone: &'static str) -> Seen<'static> {
		Seen {
			locale: None,
			time_zone: Some(time_zone),
		}
	}

	#[test]
	fn nine_in_the_morning_arrives_zone_by_zone() {
		let nine = wall("2026-12-01T09:00");
		let at = |name: &str| arrives_in(&zone(name).unwrap(), nine).unwrap();

		assert_eq!(at("UTC"), instant("2026-12-01T09:00:00Z"));
		assert_eq!(at("Asia/Tokyo"), instant("2026-12-01T00:00:00Z"));
		assert_eq!(at("Europe/London"), instant("2026-12-01T09:00:00Z"));
		assert_eq!(at("America/New_York"), instant("2026-12-01T14:00:00Z"));
		assert_eq!(at("Asia/Kolkata"), instant("2026-12-01T03:30:00Z"));
		assert_eq!(at("Pacific/Kiritimati"), instant("2026-11-30T19:00:00Z"));
		assert_eq!(at("Etc/GMT+12"), instant("2026-12-01T21:00:00Z"));
		assert_eq!(
			at("europe/london"),
			at("Europe/London"),
			"names ignore case"
		);
	}

	#[test]
	fn a_skipped_time_arrives_after_the_gap_and_a_repeated_one_the_first_time() {
		let new_york = zone("America/New_York").unwrap();

		// 8 March 2026: 02:00 EST becomes 03:00 EDT, so 02:30 never
		// happens. It arrives as 03:30 EDT.
		assert_eq!(
			arrives_in(&new_york, wall("2026-03-08T02:30")).unwrap(),
			instant("2026-03-08T07:30:00Z")
		);
		// 1 November 2026: 02:00 EDT becomes 01:00 EST, so 01:30 happens
		// twice. It arrives the first time, in EDT.
		assert_eq!(
			arrives_in(&new_york, wall("2026-11-01T01:30")).unwrap(),
			instant("2026-11-01T05:30:00Z")
		);
		// An ordinary time either side of the change is what it says.
		assert_eq!(
			arrives_in(&new_york, wall("2026-03-08T01:30")).unwrap(),
			instant("2026-03-08T06:30:00Z")
		);
		assert_eq!(
			arrives_in(&new_york, wall("2026-03-08T03:30")).unwrap(),
			instant("2026-03-08T07:30:00Z")
		);
	}

	#[test]
	fn an_unknown_zone_is_no_zone() {
		for name in [
			"Mars/Olympus_Mons",
			"Etc/Unknown",
			"",
			"   ",
			"+05:00",
			"EST5EDT,M3.2.0,M11.1.0",
			"not a zone",
		] {
			assert!(zone(name).is_none(), "{name:?}");
		}

		assert!(zone("Europe/London").is_some());
		assert!(zone(" Europe/London ").is_some(), "trimmed");
	}

	#[test]
	fn the_attribute_wins_then_the_most_recent_device_then_the_default() {
		let mut arrivals = arrivals("2026-12-01T09:00", Some("Europe/Paris"));
		let tokyo = instant("2026-12-01T00:00:00Z");
		let new_york = instant("2026-12-01T14:00:00Z");
		let paris = instant("2026-12-01T08:00:00Z");
		let devices = [in_zone("America/New_York"), in_zone("Asia/Tokyo")];

		assert_eq!(
			arrivals.for_person(Some("Asia/Tokyo"), &devices),
			tokyo,
			"the app's attribute wins"
		);
		assert_eq!(
			arrivals.for_person(None, &devices),
			new_york,
			"then the most recently seen device"
		);
		assert_eq!(
			arrivals.for_person(None, &[]),
			paris,
			"then the send's default"
		);
	}

	#[test]
	fn an_unknown_zone_falls_to_the_next_step_and_never_fails() {
		let mut arrivals = arrivals("2026-12-01T09:00", Some("Europe/Paris"));
		let tokyo = instant("2026-12-01T00:00:00Z");
		let paris = instant("2026-12-01T08:00:00Z");

		assert_eq!(
			arrivals.for_person(Some("Mars/Olympus_Mons"), &[in_zone("Asia/Tokyo")]),
			tokyo,
			"an attribute the database doesn't know falls to the device"
		);
		assert_eq!(
			arrivals.for_person(None, &[in_zone("Mars/Olympus_Mons"), in_zone("Asia/Tokyo")]),
			tokyo,
			"a device it doesn't know falls to the next most recent"
		);
		assert_eq!(
			arrivals.for_person(Some("nowhere"), &[in_zone("Etc/Unknown"), Seen::default()]),
			paris,
			"and with nothing known, the send's default"
		);
	}

	#[test]
	fn a_default_zone_this_build_lost_falls_back_to_utc() {
		let mut arrivals = arrivals("2026-12-01T09:00", Some("Mars/Olympus_Mons"));

		assert_eq!(
			arrivals.for_person(None, &[]),
			instant("2026-12-01T09:00:00Z")
		);

		let mut unnamed = self::arrivals("2026-12-01T09:00", None);
		assert_eq!(
			unnamed.for_person(None, &[]),
			instant("2026-12-01T09:00:00Z")
		);
	}

	#[test]
	fn the_locale_attribute_wins_then_the_most_recent_device() {
		let devices = [
			Seen {
				locale: None,
				time_zone: None,
			},
			Seen {
				locale: Some("fr-CA"),
				time_zone: None,
			},
			Seen {
				locale: Some("de"),
				time_zone: None,
			},
		];

		assert_eq!(locale_of(Some("pt_br"), &devices).as_deref(), Some("pt-BR"));
		assert_eq!(
			locale_of(None, &devices).as_deref(),
			Some("fr-CA"),
			"the most recently seen device that says"
		);
		assert_eq!(
			locale_of(Some("not a tag"), &devices).as_deref(),
			Some("fr-CA"),
			"an attribute that isn't a tag falls to the devices"
		);
		assert_eq!(locale_of(None, &[]), None);
	}
}
