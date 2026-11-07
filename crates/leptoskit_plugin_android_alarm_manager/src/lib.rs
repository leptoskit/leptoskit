//! <!-- {=p_android_alarm_managerSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_android_alarm_manager` as a direct dependency and
//! select the `android_alarm_manager` plugin in `native.plugins`; the generated
//! Android host discovers it from Cargo metadata and declares its alarm and
//! boot receivers and its `SCHEDULE_EXACT_ALARM` and `RECEIVE_BOOT_COMPLETED`
//! permissions. Guests enable the `guest` feature and keep one
//! `guest::AndroidAlarmManager`. It schedules one-shot and repeating Android
//! `AlarmManager` alarms by integer ID, with exact, wakeup, allow-while-idle,
//! alarm-clock and reschedule-on-reboot options, and keeps scheduled and fired
//! alarms in an on-device store that survives process death. Guests wait for or
//! drain fired alarms, and an alarm marked `headless` starts the guest without
//! UI when the app isn't running. iOS, macOS, Windows, Linux, HarmonyOS and the
//! browser return `Unavailable`: they have no equivalent that runs app code at
//! a set time.
//!
//! <!-- {/p_android_alarm_managerSummary} -->
//!
//! # Wire format
//!
//! Integers are little-endian. Byte strings are a `u32` length followed by the
//! bytes. Booleans are one byte, 0 or 1. Times are `i64` milliseconds since the
//! Unix epoch; durations are `i64` milliseconds. Every decoder rejects
//! truncation, trailing bytes, unknown codes and values outside their range.

use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest;

/// Stable native registration name.
pub const NAME: &str = "android_alarm_manager";
/// Empty payload; replies with a [`Status`].
pub const STATUS: MethodId = MethodId(0);
/// One [`Request`]; replies with [`Scheduled`]. Replaces an alarm with the
/// same ID.
pub const SCHEDULE: MethodId = MethodId(1);
/// One `i32` alarm ID; replies with one boolean: whether it was scheduled.
pub const CANCEL: MethodId = MethodId(2);
/// Empty payload; replies with the [`ScheduledAlarm`] list.
pub const LIST: MethodId = MethodId(3);
/// Empty payload; opens Android's exact-alarm setting and replies with one
/// boolean: whether exact alarms are allowed afterwards.
pub const REQUEST_EXACT_PERMISSION: MethodId = MethodId(4);
/// One `u64` cursor; acknowledges fired alarms up to it, then waits for the
/// oldest one after it and replies with that [`FiredAlarm`].
pub const NEXT: MethodId = MethodId(5);
/// A `u64` cursor and a `u32` page size; acknowledges fired alarms up to the
/// cursor and replies at once with up to that many later ones.
pub const DRAIN: MethodId = MethodId(6);
/// Empty payload; replies with a boolean and, when it is 1, the
/// [`FiredAlarm`] that started this headless guest.
pub const LAUNCH: MethodId = MethodId(7);
/// One `u64` sequence number; removes that fired alarm and ends the headless
/// run that it started.
pub const FINISH: MethodId = MethodId(8);
/// Independently grantable operations.
pub const METHODS: &[MethodId] = &[
	STATUS,
	SCHEDULE,
	CANCEL,
	LIST,
	REQUEST_EXACT_PERMISSION,
	NEXT,
	DRAIN,
	LAUNCH,
	FINISH,
];

/// Largest opaque parameter payload an alarm carries back when it fires.
pub const MAX_PARAMS_BYTES: usize = 1024;
/// Most alarms the device store keeps scheduled at once.
pub const MAX_ALARMS: usize = 128;
/// Most fired alarms the device store keeps for the guest; the oldest is
/// dropped, and counted in [`Status::dropped`], when another fires.
pub const MAX_FIRED: usize = 64;
/// Android runs repeating alarms at most once a minute.
pub const MIN_PERIOD: Duration = Duration::from_mins(1);
/// Longest repeat interval: 366 days.
pub const MAX_PERIOD: Duration = Duration::from_hours(366 * 24);
/// Longest one-shot delay: 3,660 days.
pub const MAX_DELAY: Duration = Duration::from_hours(3660 * 24);
/// 9999-12-31T23:59:59.999Z, the last instant Android's date APIs format.
pub const MAX_UNIX_MILLIS: i64 = 253_402_300_799_999;
/// How long a headless guest may run for one alarm before the host stops it.
pub const HEADLESS_BUDGET: Duration = Duration::from_secs(8);

const SCHEDULE_HEADER_BYTES: usize = 4 + 1 + 4 + 8 + 8 + 4;
const FIRED_HEADER_BYTES: usize = 8 + 4 + 8 + 8 + 8 + 4;
const ENTRY_HEADER_BYTES: usize = 4 + 8 + 8 + 4 + 1 + 4;
const STATUS_BYTES: usize = 4 + 1 + 1 + 4 + 8;
/// `Schedule::Periodic` without a start time encodes this instead.
const FROM_NOW: i64 = -1;

/// Milliseconds since the Unix epoch, from 0 to [`MAX_UNIX_MILLIS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnixMillis(i64);

impl UnixMillis {
	/// Accepts 0 to [`MAX_UNIX_MILLIS`].
	pub fn new(millis: i64) -> Result<Self, Error> {
		if (0..=MAX_UNIX_MILLIS).contains(&millis) {
			Ok(Self(millis))
		} else {
			Err(Error::InvalidArgument)
		}
	}

	/// Converts a wall-clock time, truncating below a millisecond. Times
	/// before 1970 or after [`MAX_UNIX_MILLIS`] are rejected.
	pub fn from_system_time(time: SystemTime) -> Result<Self, Error> {
		let since = time
			.duration_since(UNIX_EPOCH)
			.map_err(|_| Error::InvalidArgument)?;
		Self::new(i64::try_from(since.as_millis()).map_err(|_| Error::InvalidArgument)?)
	}

	/// The milliseconds since the Unix epoch.
	#[must_use]
	pub fn get(self) -> i64 {
		self.0
	}

	/// The same instant as a [`SystemTime`].
	#[must_use]
	pub fn to_system_time(self) -> SystemTime {
		UNIX_EPOCH + Duration::from_millis(self.0.unsigned_abs())
	}
}

/// How an alarm asks Android to deliver it. Every flag defaults to off, which
/// is an inexact alarm that waits for the device to wake for another reason.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[allow(clippy::struct_excessive_bools)]
pub struct Options {
	/// Fire at the requested time rather than in a battery-friendly window.
	/// On Android 12 and later this needs the exact-alarm permission.
	pub exact: bool,
	/// Wake the device from sleep to fire (`RTC_WAKEUP`) instead of waiting
	/// for it to wake for another reason (`RTC`).
	pub wakeup: bool,
	/// Fire even while the device dozes. Android limits how often such alarms
	/// run, to roughly one every nine minutes per app.
	pub allow_while_idle: bool,
	/// Schedule with `setAlarmClock`: always exact, wakes the device, and
	/// shows an alarm icon in the status bar. Needs the exact-alarm
	/// permission on Android 12 and later.
	pub alarm_clock: bool,
	/// Keep the alarm across a device restart: the plugin schedules it again
	/// when Android reports `BOOT_COMPLETED`.
	pub reschedule_on_reboot: bool,
	/// When the app isn't running as the alarm fires, start its guest without
	/// any UI so it can handle the alarm with
	/// `guest::AndroidAlarmManager::handle_launch`.
	pub headless: bool,
	/// When exact timing isn't allowed, schedule an inexact alarm instead of
	/// failing with [`Error::PermissionDenied`]. Requires `exact` or
	/// `alarm_clock`.
	pub inexact_fallback: bool,
}

impl Options {
	const ALARM_CLOCK: u32 = 1 << 3;
	const ALLOW_WHILE_IDLE: u32 = 1 << 2;
	const EXACT: u32 = 1;
	const HEADLESS: u32 = 1 << 5;
	const INEXACT_FALLBACK: u32 = 1 << 6;
	const KNOWN: u32 = (1 << 7) - 1;
	const RESCHEDULE_ON_REBOOT: u32 = 1 << 4;
	const WAKEUP: u32 = 1 << 1;

	/// The flags as wire bits: exact=1, wakeup=2, allow-while-idle=4,
	/// alarm-clock=8, reschedule-on-reboot=16, headless=32,
	/// inexact-fallback=64.
	#[must_use]
	pub fn bits(self) -> u32 {
		[
			(self.exact, Self::EXACT),
			(self.wakeup, Self::WAKEUP),
			(self.allow_while_idle, Self::ALLOW_WHILE_IDLE),
			(self.alarm_clock, Self::ALARM_CLOCK),
			(self.reschedule_on_reboot, Self::RESCHEDULE_ON_REBOOT),
			(self.headless, Self::HEADLESS),
			(self.inexact_fallback, Self::INEXACT_FALLBACK),
		]
		.into_iter()
		.filter(|(set, _)| *set)
		.fold(0, |bits, (_, bit)| bits | bit)
	}

	/// Reads wire bits, rejecting unknown bits and a fallback without exact
	/// timing to fall back from.
	pub fn from_bits(bits: u32) -> Result<Self, Error> {
		if bits & !Self::KNOWN != 0 {
			return Err(Error::InvalidArgument);
		}

		let options = Self {
			exact: bits & Self::EXACT != 0,
			wakeup: bits & Self::WAKEUP != 0,
			allow_while_idle: bits & Self::ALLOW_WHILE_IDLE != 0,
			alarm_clock: bits & Self::ALARM_CLOCK != 0,
			reschedule_on_reboot: bits & Self::RESCHEDULE_ON_REBOOT != 0,
			headless: bits & Self::HEADLESS != 0,
			inexact_fallback: bits & Self::INEXACT_FALLBACK != 0,
		};
		options.validate()?;
		Ok(options)
	}

	/// Rejects `inexact_fallback` without `exact` or `alarm_clock`.
	pub fn validate(self) -> Result<(), Error> {
		if self.inexact_fallback && !self.exact && !self.alarm_clock {
			return Err(Error::InvalidArgument);
		}

		Ok(())
	}
}

/// When an alarm fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Schedule {
	/// Once, this long after the host receives the request. Up to
	/// [`MAX_DELAY`], in whole milliseconds.
	After(Duration),
	/// Once, at this time. A time in the past fires as soon as possible.
	At(UnixMillis),
	/// Repeatedly, every `interval` ([`MIN_PERIOD`] to [`MAX_PERIOD`], in
	/// whole milliseconds), first at `start`, or one interval from now.
	Periodic {
		/// Time between firings.
		interval: Duration,
		/// The first firing; `None` means one interval after the request.
		start: Option<UnixMillis>,
	},
}

impl Schedule {
	const AFTER: u8 = 0;
	const AT: u8 = 1;
	const PERIODIC: u8 = 2;

	fn validate(self) -> Result<(), Error> {
		match self {
			Self::After(delay) if delay <= MAX_DELAY && whole_millis(delay) => Ok(()),
			Self::At(_) => Ok(()),
			Self::Periodic { interval, .. }
				if (MIN_PERIOD..=MAX_PERIOD).contains(&interval) && whole_millis(interval) =>
			{
				Ok(())
			}
			_ => Err(Error::InvalidArgument),
		}
	}
}

fn whole_millis(duration: Duration) -> bool {
	duration.subsec_nanos() % 1_000_000 == 0
}

fn millis(duration: Duration) -> Result<i64, Error> {
	i64::try_from(duration.as_millis()).map_err(|_| Error::InvalidArgument)
}

/// One alarm to schedule.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Request {
	/// Scheduling the same ID again replaces the earlier alarm.
	pub id: i32,
	/// When it fires.
	pub schedule: Schedule,
	/// How Android delivers it.
	pub options: Options,
	/// Opaque bytes handed back in every [`FiredAlarm`]; up to
	/// [`MAX_PARAMS_BYTES`].
	pub params: Vec<u8>,
}

impl Request {
	/// A one-shot alarm `delay` from now.
	#[must_use]
	pub fn one_shot(id: i32, delay: Duration) -> Self {
		Self::new(id, Schedule::After(delay))
	}

	/// A one-shot alarm at `at`.
	#[must_use]
	pub fn one_shot_at(id: i32, at: UnixMillis) -> Self {
		Self::new(id, Schedule::At(at))
	}

	/// A repeating alarm, first one `interval` from now.
	#[must_use]
	pub fn periodic(id: i32, interval: Duration) -> Self {
		Self::new(
			id,
			Schedule::Periodic {
				interval,
				start: None,
			},
		)
	}

	fn new(id: i32, schedule: Schedule) -> Self {
		Self {
			id,
			schedule,
			options: Options::default(),
			params: Vec::new(),
		}
	}

	/// Sets the delivery options.
	#[must_use]
	pub fn with_options(mut self, options: Options) -> Self {
		self.options = options;
		self
	}

	/// Sets the bytes handed back when the alarm fires.
	#[must_use]
	pub fn with_params(mut self, params: impl Into<Vec<u8>>) -> Self {
		self.params = params.into();
		self
	}

	/// Sets the first firing of a periodic alarm; other schedules are
	/// unchanged.
	#[must_use]
	pub fn starting_at(mut self, at: UnixMillis) -> Self {
		if let Schedule::Periodic { start, .. } = &mut self.schedule {
			*start = Some(at);
		}

		self
	}

	/// Checks every bound the host checks.
	pub fn validate(&self) -> Result<(), Error> {
		self.schedule.validate()?;
		self.options.validate()?;

		if self.params.len() > MAX_PARAMS_BYTES {
			return Err(Error::TooLarge);
		}

		Ok(())
	}

	/// `i32` id, `u8` kind (0 after, 1 at, 2 periodic), `u32` option bits,
	/// `i64` time (delay, instant, or first firing with -1 for one interval
	/// from now), `i64` interval (0 unless periodic), then the params.
	pub fn encode(&self) -> Result<Bytes, Error> {
		self.validate()?;
		let (kind, time, interval) = match self.schedule {
			Schedule::After(delay) => (Schedule::AFTER, millis(delay)?, 0),
			Schedule::At(at) => (Schedule::AT, at.get(), 0),
			Schedule::Periodic { interval, start } => {
				(
					Schedule::PERIODIC,
					start.map_or(FROM_NOW, UnixMillis::get),
					millis(interval)?,
				)
			}
		};
		let mut output = Writer::with_capacity(SCHEDULE_HEADER_BYTES + self.params.len());
		output.i32(self.id);
		output.u8(kind);
		output.u32(self.options.bits());
		output.i64(time);
		output.i64(interval);
		output.bytes(&self.params);
		Ok(output.finish())
	}

	/// Strictly decodes [`Request::encode`].
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > SCHEDULE_HEADER_BYTES + MAX_PARAMS_BYTES {
			return Err(Error::TooLarge);
		}

		let mut input = Reader::new(payload);
		let id = input.i32()?;
		let kind = input.u8()?;
		let options = Options::from_bits(input.u32()?)?;
		let time = input.i64()?;
		let interval = input.i64()?;
		let params = input.bytes(MAX_PARAMS_BYTES)?.to_vec();
		input.finish()?;
		let schedule = match kind {
			Schedule::AFTER if interval == 0 => Schedule::After(duration(time)?),
			Schedule::AT if interval == 0 => Schedule::At(UnixMillis::new(time)?),
			Schedule::PERIODIC => {
				Schedule::Periodic {
					interval: duration(interval)?,
					start: if time == FROM_NOW {
						None
					} else {
						Some(UnixMillis::new(time)?)
					},
				}
			}
			_ => return Err(Error::InvalidArgument),
		};
		let request = Self {
			id,
			schedule,
			options,
			params,
		};
		request.validate()?;
		Ok(request)
	}
}

fn duration(millis: i64) -> Result<Duration, Error> {
	u64::try_from(millis)
		.map(Duration::from_millis)
		.map_err(|_| Error::InvalidArgument)
}

/// How Android ended up timing an alarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Timing {
	/// `setExact` or `setExactAndAllowWhileIdle`.
	Exact = 0,
	/// `set` or `setAndAllowWhileIdle`: Android picks a window around the
	/// requested time. Also what `inexact_fallback` falls back to.
	Inexact = 1,
	/// `setAlarmClock`.
	AlarmClock = 2,
}

impl Timing {
	fn from_u8(value: u8) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::Exact),
			1 => Ok(Self::Inexact),
			2 => Ok(Self::AlarmClock),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// The host's reply to [`SCHEDULE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Scheduled {
	/// How the alarm was handed to Android.
	pub timing: Timing,
	/// When it first fires, as the host computed it.
	pub first: UnixMillis,
}

impl Scheduled {
	/// `u8` timing, then the `i64` first firing.
	#[must_use]
	pub fn encode(self) -> [u8; 9] {
		let mut output = [0; 9];
		output[0] = self.timing as u8;
		output[1..].copy_from_slice(&self.first.get().to_le_bytes());
		output
	}

	/// Strictly decodes [`Scheduled::encode`].
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut input = Reader::new(payload);
		let timing = Timing::from_u8(input.u8()?)?;
		let first = UnixMillis::new(input.i64()?)?;
		input.finish()?;
		Ok(Self { timing, first })
	}
}

/// An alarm the device store holds as scheduled.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScheduledAlarm {
	/// The app's ID for it.
	pub id: i32,
	/// The repeat interval, or `None` for a one-shot alarm.
	pub interval: Option<Duration>,
	/// When it fires next.
	pub next: UnixMillis,
	/// The options it was scheduled with.
	pub options: Options,
	/// How Android is timing it.
	pub timing: Timing,
	/// Its opaque parameters.
	pub params: Vec<u8>,
}

impl ScheduledAlarm {
	fn encode_into(&self, output: &mut Writer) -> Result<(), Error> {
		if self.params.len() > MAX_PARAMS_BYTES {
			return Err(Error::TooLarge);
		}

		output.i32(self.id);
		output.i64(self.next.get());
		output.i64(optional_interval(self.interval)?);
		output.u32(self.options.bits());
		output.u8(self.timing as u8);
		output.bytes(&self.params);
		Ok(())
	}

	fn decode_from(input: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			id: input.i32()?,
			next: UnixMillis::new(input.i64()?)?,
			interval: interval(input.i64()?)?,
			options: Options::from_bits(input.u32()?)?,
			timing: Timing::from_u8(input.u8()?)?,
			params: input.bytes(MAX_PARAMS_BYTES)?.to_vec(),
		})
	}

	/// A `u32` count (up to [`MAX_ALARMS`]), then per alarm: `i32` id, `i64`
	/// next firing, `i64` interval (0 for one-shot), `u32` option bits, `u8`
	/// timing and its params.
	pub fn encode_list(alarms: &[Self]) -> Result<Bytes, Error> {
		if alarms.len() > MAX_ALARMS {
			return Err(Error::TooLarge);
		}

		let mut output = Writer::with_capacity(
			4 + alarms
				.iter()
				.map(|alarm| ENTRY_HEADER_BYTES + alarm.params.len())
				.sum::<usize>(),
		);
		output.u32(count(alarms.len()));

		for alarm in alarms {
			alarm.encode_into(&mut output)?;
		}

		Ok(output.finish())
	}

	/// Strictly decodes [`ScheduledAlarm::encode_list`], rejecting repeated
	/// IDs.
	pub fn decode_list(payload: &[u8]) -> Result<Vec<Self>, Error> {
		let mut input = Reader::new(payload);
		let count = input.u32()? as usize;

		if count > MAX_ALARMS {
			return Err(Error::InvalidArgument);
		}

		let mut alarms: Vec<Self> = Vec::with_capacity(count);

		for _ in 0..count {
			let alarm = Self::decode_from(&mut input)?;

			if alarms.iter().any(|other| other.id == alarm.id) {
				return Err(Error::InvalidArgument);
			}

			alarms.push(alarm);
		}

		input.finish()?;
		Ok(alarms)
	}
}

fn optional_interval(interval: Option<Duration>) -> Result<i64, Error> {
	match interval {
		None => Ok(0),
		Some(interval)
			if (MIN_PERIOD..=MAX_PERIOD).contains(&interval) && whole_millis(interval) =>
		{
			millis(interval)
		}
		Some(_) => Err(Error::InvalidArgument),
	}
}

fn interval(millis: i64) -> Result<Option<Duration>, Error> {
	if millis == 0 {
		return Ok(None);
	}

	let interval = duration(millis)?;

	if (MIN_PERIOD..=MAX_PERIOD).contains(&interval) {
		Ok(Some(interval))
	} else {
		Err(Error::InvalidArgument)
	}
}

fn count(len: usize) -> u32 {
	u32::try_from(len).unwrap_or(u32::MAX)
}

/// An alarm that fired, kept on the device until a guest acknowledges it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FiredAlarm {
	/// Increases with every firing on this device, from 1; never reused.
	pub sequence: u64,
	/// The app's ID for the alarm.
	pub id: i32,
	/// The repeat interval, or `None` for a one-shot alarm.
	pub interval: Option<Duration>,
	/// When it was due.
	pub scheduled: UnixMillis,
	/// When Android delivered it.
	pub fired: UnixMillis,
	/// The params it was scheduled with.
	pub params: Vec<u8>,
}

impl FiredAlarm {
	/// `u64` sequence, `i32` id, `i64` interval (0 for one-shot), `i64` due
	/// time, `i64` delivery time, then the params.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Writer::with_capacity(FIRED_HEADER_BYTES + self.params.len());
		self.encode_into(&mut output)?;
		Ok(output.finish())
	}

	fn encode_into(&self, output: &mut Writer) -> Result<(), Error> {
		if self.sequence == 0 {
			return Err(Error::InvalidArgument);
		}

		if self.params.len() > MAX_PARAMS_BYTES {
			return Err(Error::TooLarge);
		}

		output.u64(self.sequence);
		output.i32(self.id);
		output.i64(optional_interval(self.interval)?);
		output.i64(self.scheduled.get());
		output.i64(self.fired.get());
		output.bytes(&self.params);
		Ok(())
	}

	/// Strictly decodes [`FiredAlarm::encode`].
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut input = Reader::new(payload);
		let alarm = Self::decode_from(&mut input)?;
		input.finish()?;
		Ok(alarm)
	}

	fn decode_from(input: &mut Reader<'_>) -> Result<Self, Error> {
		let sequence = input.u64()?;

		if sequence == 0 {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			sequence,
			id: input.i32()?,
			interval: interval(input.i64()?)?,
			scheduled: UnixMillis::new(input.i64()?)?,
			fired: UnixMillis::new(input.i64()?)?,
			params: input.bytes(MAX_PARAMS_BYTES)?.to_vec(),
		})
	}

	/// A `u32` count, then each fired alarm in increasing sequence order.
	pub fn encode_page(alarms: &[Self]) -> Result<Bytes, Error> {
		if alarms.len() > MAX_FIRED {
			return Err(Error::TooLarge);
		}

		let mut output = Writer::with_capacity(
			4 + alarms
				.iter()
				.map(|alarm| FIRED_HEADER_BYTES + alarm.params.len())
				.sum::<usize>(),
		);
		output.u32(count(alarms.len()));

		for alarm in alarms {
			alarm.encode_into(&mut output)?;
		}

		Ok(output.finish())
	}

	/// Strictly decodes a [`DRAIN`] reply for `cursor` and `max`: at most
	/// `max` alarms, each after the cursor and after the one before it.
	pub fn decode_page(payload: &[u8], cursor: u64, max: u32) -> Result<Vec<Self>, Error> {
		let mut input = Reader::new(payload);
		let count = input.u32()?;

		if count > max || count as usize > MAX_FIRED {
			return Err(Error::InvalidArgument);
		}

		let mut alarms = Vec::with_capacity(count as usize);
		let mut last = cursor;

		for _ in 0..count {
			let alarm = Self::decode_from(&mut input)?;

			if alarm.sequence <= last {
				return Err(Error::InvalidArgument);
			}

			last = alarm.sequence;
			alarms.push(alarm);
		}

		input.finish()?;
		Ok(alarms)
	}

	/// The [`LAUNCH`] reply: 0, or 1 followed by the alarm.
	pub fn encode_launch(alarm: Option<&Self>) -> Result<Bytes, Error> {
		let Some(alarm) = alarm else {
			return Ok(Bytes::from(vec![0]));
		};
		let mut output = Writer::with_capacity(1 + FIRED_HEADER_BYTES + alarm.params.len());
		output.u8(1);
		alarm.encode_into(&mut output)?;
		Ok(output.finish())
	}

	/// Strictly decodes [`FiredAlarm::encode_launch`].
	pub fn decode_launch(payload: &[u8]) -> Result<Option<Self>, Error> {
		let mut input = Reader::new(payload);
		let alarm = if boolean(input.u8()?)? {
			Some(Self::decode_from(&mut input)?)
		} else {
			None
		};
		input.finish()?;
		Ok(alarm)
	}
}

/// What the host reports about alarms on this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Status {
	/// Android's API level (`Build.VERSION.SDK_INT`).
	pub sdk: u32,
	/// Whether exact and alarm-clock alarms may be scheduled now. Always true
	/// before Android 12.
	pub can_schedule_exact: bool,
	/// Whether this host can start the guest headlessly for an alarm. False
	/// when the app takes over-the-air updates: the headless guest must be
	/// the same code the app runs, and the update store belongs to the app.
	pub headless: bool,
	/// Fired alarms waiting for a guest to acknowledge them.
	pub pending: u32,
	/// Fired alarms dropped, oldest first, because more than [`MAX_FIRED`]
	/// were waiting.
	pub dropped: u64,
}

impl Status {
	/// `u32` SDK, `u8` exact, `u8` headless, `u32` pending, `u64` dropped.
	#[must_use]
	pub fn encode(self) -> [u8; STATUS_BYTES] {
		let mut output = Writer::with_capacity(STATUS_BYTES);
		output.u32(self.sdk);
		output.u8(self.can_schedule_exact.into());
		output.u8(self.headless.into());
		output.u32(self.pending);
		output.u64(self.dropped);
		output.bytes.try_into().unwrap_or([0; STATUS_BYTES])
	}

	/// Strictly decodes [`Status::encode`].
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut input = Reader::new(payload);
		let status = Self {
			sdk: input.u32()?,
			can_schedule_exact: boolean(input.u8()?)?,
			headless: boolean(input.u8()?)?,
			pending: input.u32()?,
			dropped: input.u64()?,
		};
		input.finish()?;

		if status.pending as usize > MAX_FIRED {
			return Err(Error::InvalidArgument);
		}

		Ok(status)
	}
}

/// The [`CANCEL`] request.
#[must_use]
pub fn encode_cancel(id: i32) -> [u8; 4] {
	id.to_le_bytes()
}

/// Strictly decodes the [`CANCEL`] request.
pub fn decode_cancel(payload: &[u8]) -> Result<i32, Error> {
	let mut input = Reader::new(payload);
	let id = input.i32()?;
	input.finish()?;
	Ok(id)
}

/// The [`NEXT`] request: the cursor of the last alarm the guest received.
#[must_use]
pub fn encode_next(cursor: u64) -> [u8; 8] {
	cursor.to_le_bytes()
}

/// Strictly decodes the [`NEXT`] request.
pub fn decode_next(payload: &[u8]) -> Result<u64, Error> {
	let mut input = Reader::new(payload);
	let cursor = input.u64()?;
	input.finish()?;
	Ok(cursor)
}

/// The [`DRAIN`] request; `max` is 0 to acknowledge without reading, up to
/// [`MAX_FIRED`].
pub fn encode_drain(cursor: u64, max: u32) -> Result<[u8; 12], Error> {
	if max as usize > MAX_FIRED {
		return Err(Error::InvalidArgument);
	}

	let mut output = [0; 12];
	output[..8].copy_from_slice(&cursor.to_le_bytes());
	output[8..].copy_from_slice(&max.to_le_bytes());
	Ok(output)
}

/// Strictly decodes the [`DRAIN`] request.
pub fn decode_drain(payload: &[u8]) -> Result<(u64, u32), Error> {
	let mut input = Reader::new(payload);
	let cursor = input.u64()?;
	let max = input.u32()?;
	input.finish()?;

	if max as usize > MAX_FIRED {
		return Err(Error::InvalidArgument);
	}

	Ok((cursor, max))
}

/// The [`FINISH`] request.
pub fn encode_finish(sequence: u64) -> Result<[u8; 8], Error> {
	if sequence == 0 {
		return Err(Error::InvalidArgument);
	}

	Ok(sequence.to_le_bytes())
}

/// Strictly decodes the [`FINISH`] request.
pub fn decode_finish(payload: &[u8]) -> Result<u64, Error> {
	let mut input = Reader::new(payload);
	let sequence = input.u64()?;
	input.finish()?;

	if sequence == 0 {
		return Err(Error::InvalidArgument);
	}

	Ok(sequence)
}

/// Strictly decodes a one-byte boolean reply.
pub fn decode_flag(payload: &[u8]) -> Result<bool, Error> {
	match payload {
		[value] => boolean(*value),
		_ => Err(Error::InvalidArgument),
	}
}

fn boolean(value: u8) -> Result<bool, Error> {
	match value {
		0 => Ok(false),
		1 => Ok(true),
		_ => Err(Error::InvalidArgument),
	}
}

struct Writer {
	bytes: Vec<u8>,
}

impl Writer {
	fn with_capacity(capacity: usize) -> Self {
		Self {
			bytes: Vec::with_capacity(capacity),
		}
	}

	fn u8(&mut self, value: u8) {
		self.bytes.push(value);
	}

	fn u32(&mut self, value: u32) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	fn i32(&mut self, value: i32) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	fn i64(&mut self, value: i64) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	fn u64(&mut self, value: u64) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	/// Callers bound `value` to [`MAX_PARAMS_BYTES`] first.
	fn bytes(&mut self, value: &[u8]) {
		self.u32(count(value.len()));
		self.bytes.extend_from_slice(value);
	}

	fn finish(self) -> Bytes {
		self.bytes.into()
	}
}

struct Reader<'a> {
	remaining: &'a [u8],
}

impl<'a> Reader<'a> {
	fn new(input: &'a [u8]) -> Self {
		Self { remaining: input }
	}

	fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
		let (head, tail) = self
			.remaining
			.split_first_chunk::<N>()
			.ok_or(Error::InvalidArgument)?;
		self.remaining = tail;
		Ok(*head)
	}

	fn u8(&mut self) -> Result<u8, Error> {
		Ok(self.take::<1>()?[0])
	}

	fn u32(&mut self) -> Result<u32, Error> {
		self.take().map(u32::from_le_bytes)
	}

	fn i32(&mut self) -> Result<i32, Error> {
		self.take().map(i32::from_le_bytes)
	}

	fn i64(&mut self) -> Result<i64, Error> {
		self.take().map(i64::from_le_bytes)
	}

	fn u64(&mut self) -> Result<u64, Error> {
		self.take().map(u64::from_le_bytes)
	}

	fn bytes(&mut self, max: usize) -> Result<&'a [u8], Error> {
		let len = self.u32()? as usize;

		if len > max {
			return Err(Error::InvalidArgument);
		}

		let (head, tail) = self
			.remaining
			.split_at_checked(len)
			.ok_or(Error::InvalidArgument)?;
		self.remaining = tail;
		Ok(head)
	}

	fn finish(self) -> Result<(), Error> {
		if self.remaining.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}
}

/// Browsers have no alarm service that runs while the page is closed. Every
/// operation is unavailable.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use std::future::Ready;
	use std::future::ready;

	use super::*;

	/// Always unavailable in a browser.
	pub fn status() -> Ready<Result<Status, Error>> {
		ready(Err(Error::Unavailable))
	}

	/// Always unavailable in a browser.
	pub fn schedule(_request: &Request) -> Ready<Result<Scheduled, Error>> {
		ready(Err(Error::Unavailable))
	}

	/// Always unavailable in a browser.
	pub fn cancel(_id: i32) -> Ready<Result<bool, Error>> {
		ready(Err(Error::Unavailable))
	}

	/// Always unavailable in a browser.
	pub fn scheduled() -> Ready<Result<Vec<ScheduledAlarm>, Error>> {
		ready(Err(Error::Unavailable))
	}

	/// Always unavailable in a browser.
	pub fn drain() -> Ready<Result<Vec<FiredAlarm>, Error>> {
		ready(Err(Error::Unavailable))
	}
}

/// Explicit unavailable handler for custom hosts other than Android: it
/// checks each payload like the Android host, then fails with
/// [`Error::Unavailable`].
#[derive(Debug, Default)]
pub struct Unsupported;

impl leptoskit_bridge::Plugin for Unsupported {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		match method {
			STATUS | LIST | REQUEST_EXACT_PERMISSION | LAUNCH if !payload.is_empty() => {
				return Err(Error::InvalidArgument);
			}
			SCHEDULE => {
				Request::decode(&payload)?;
			}
			CANCEL => {
				decode_cancel(&payload)?;
			}
			NEXT => {
				decode_next(&payload)?;
			}
			DRAIN => {
				decode_drain(&payload)?;
			}
			FINISH => {
				decode_finish(&payload)?;
			}
			_ => {}
		}

		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests;
