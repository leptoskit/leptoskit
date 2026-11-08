//! Cached native alarm-manager client for Wasm guests.
use std::cell::Cell;
use std::future::Future;

use super::*;

/// Resolves the host plugin once, then uses numeric method identifiers.
///
/// It remembers the last fired alarm it handed out and acknowledges it on
/// the next [`next`](Self::next), [`drain`](Self::drain) or
/// [`acknowledge`](Self::acknowledge) call, so an alarm stays on the device
/// until the guest has really received it. Keep one per guest: a second one
/// starts its own cursor and receives the alarms the first has not
/// acknowledged yet.
#[derive(Debug)]
pub struct AndroidAlarmManager {
	client: leptoskit_bridge::guest::Client,
	cursor: Cell<u64>,
}

impl AndroidAlarmManager {
	/// Resolves the installed host plugin. Fails with
	/// [`Error::Unavailable`] everywhere but Android.
	pub fn new() -> Result<Self, Error> {
		leptoskit_bridge::guest::Client::new(NAME).map(|client| {
			Self {
				client,
				cursor: Cell::new(0),
			}
		})
	}

	/// Reports the API level, exact-alarm permission, whether headless
	/// launches are possible, and the fired-alarm queue.
	pub async fn status(&self) -> Result<Status, Error> {
		Status::decode(&self.client.call(STATUS, &[])?.await?)
	}

	/// Whether exact and alarm-clock alarms may be scheduled now. Always true
	/// before Android 12.
	pub async fn can_schedule_exact_alarms(&self) -> Result<bool, Error> {
		Ok(self.status().await?.can_schedule_exact)
	}

	/// Opens Android's "Alarms & reminders" setting for this app and reports
	/// whether exact alarms are allowed once the person comes back. Before
	/// Android 12, and when already allowed, it returns true without opening
	/// anything. Fails with [`Error::Unavailable`] when no activity is in the
	/// foreground, as in a headless launch.
	pub async fn request_exact_alarm_permission(&self) -> Result<bool, Error> {
		decode_flag(&self.client.call(REQUEST_EXACT_PERMISSION, &[])?.await?)
	}

	/// Schedules `request`, replacing any alarm with the same ID. Exact and
	/// alarm-clock alarms fail with [`Error::PermissionDenied`] when Android
	/// does not allow exact alarms, unless `inexact_fallback` is set.
	pub async fn schedule(&self, request: &Request) -> Result<Scheduled, Error> {
		Scheduled::decode(&self.client.call(SCHEDULE, &request.encode()?)?.await?)
	}

	/// Schedules a one-shot alarm `delay` from now.
	pub async fn one_shot(
		&self,
		id: i32,
		delay: Duration,
		options: Options,
	) -> Result<Scheduled, Error> {
		self.schedule(&Request::one_shot(id, delay).with_options(options))
			.await
	}

	/// Schedules a one-shot alarm at `at`.
	pub async fn one_shot_at(
		&self,
		id: i32,
		at: UnixMillis,
		options: Options,
	) -> Result<Scheduled, Error> {
		self.schedule(&Request::one_shot_at(id, at).with_options(options))
			.await
	}

	/// Schedules an alarm that repeats every `interval`, first one interval
	/// from now.
	pub async fn periodic(
		&self,
		id: i32,
		interval: Duration,
		options: Options,
	) -> Result<Scheduled, Error> {
		self.schedule(&Request::periodic(id, interval).with_options(options))
			.await
	}

	/// Cancels the alarm with this ID and forgets it, including for reboot.
	/// Returns whether it was scheduled. Fired alarms already waiting stay.
	pub async fn cancel(&self, id: i32) -> Result<bool, Error> {
		decode_flag(&self.client.call(CANCEL, &encode_cancel(id))?.await?)
	}

	/// The alarms the device store holds as scheduled.
	pub async fn scheduled(&self) -> Result<Vec<ScheduledAlarm>, Error> {
		ScheduledAlarm::decode_list(&self.client.call(LIST, &[])?.await?)
	}

	/// Waits for the next fired alarm, including ones that fired while the
	/// app wasn't running. Dropping the future loses nothing: the alarm stays
	/// on the device until a later call acknowledges it.
	pub async fn next(&self) -> Result<FiredAlarm, Error> {
		let cursor = self.cursor.get();
		let alarm = FiredAlarm::decode(&self.client.call(NEXT, &encode_next(cursor))?.await?)?;

		if alarm.sequence <= cursor {
			return Err(Error::InvalidArgument);
		}

		self.cursor.set(alarm.sequence);
		Ok(alarm)
	}

	/// Takes every fired alarm waiting now, oldest first, without waiting
	/// for more.
	pub async fn drain(&self) -> Result<Vec<FiredAlarm>, Error> {
		let cursor = self.cursor.get();
		let max = MAX_FIRED as u32;
		let reply = self
			.client
			.call(DRAIN, &encode_drain(cursor, max)?)?
			.await?;
		let alarms = FiredAlarm::decode_page(&reply, cursor, max)?;

		if let Some(last) = alarms.last() {
			self.cursor.set(last.sequence);
		}

		Ok(alarms)
	}

	/// Removes the alarms this client has handed out from the device now,
	/// rather than on the next call.
	pub async fn acknowledge(&self) -> Result<(), Error> {
		let reply = self
			.client
			.call(DRAIN, &encode_drain(self.cursor.get(), 0)?)?
			.await?;
		FiredAlarm::decode_page(&reply, self.cursor.get(), 0).map(drop)
	}

	/// The alarm that started this guest headlessly, or `None` when the
	/// guest started normally. That alarm isn't returned by
	/// [`next`](Self::next) or [`drain`](Self::drain) in this run.
	pub async fn launch(&self) -> Result<Option<FiredAlarm>, Error> {
		FiredAlarm::decode_launch(&self.client.call(LAUNCH, &[])?.await?)
	}

	/// Removes the launch alarm from the device and lets the host stop this
	/// headless guest. Make it the last thing the handler does: the host
	/// tears the guest down right after.
	pub async fn finish(&self, alarm: &FiredAlarm) -> Result<(), Error> {
		let reply = self
			.client
			.call(FINISH, &encode_finish(alarm.sequence)?)?
			.await?;

		if reply.is_empty() {
			Ok(())
		} else {
			Err(Error::InvalidArgument)
		}
	}

	/// Runs `handler` when this guest was started headlessly for an alarm,
	/// then calls [`finish`](Self::finish). Returns whether it did. Call it
	/// early in startup; a guest that never finishes is stopped after
	/// [`HEADLESS_BUDGET`], and its alarm stays waiting for the next
	/// [`next`](Self::next) or [`drain`](Self::drain).
	pub async fn handle_launch<F, Fut>(&self, handler: F) -> Result<bool, Error>
	where
		F: FnOnce(FiredAlarm) -> Fut,
		Fut: Future<Output = ()>,
	{
		let Some(alarm) = self.launch().await? else {
			return Ok(false);
		};
		let sequence = alarm.sequence;
		handler(alarm).await;
		self.client.call(FINISH, &encode_finish(sequence)?)?.await?;
		Ok(true)
	}
}
