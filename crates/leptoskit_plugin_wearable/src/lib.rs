//! Direct on-watch live and passive biometric sensors for Wear OS (including
//! Samsung Galaxy Watch), watchOS, and HarmonyOS wearables.
//!
//! Add `leptoskit_plugin_wearable` as a direct dependency and select `wearable`
//! in `native.plugins`. Guests enable the `guest` feature and reuse
//! `guest::Wearable`.

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Automatic native registration name.
pub const NAME: &str = "wearable";
/// Query platform capabilities and available biometric sensors, empty request.
pub const CAPABILITIES: MethodId = MethodId(0);
/// Current body-sensor permission state, empty request; never prompts.
pub const PERMISSION: MethodId = MethodId(1);
/// Request body-sensor / activity permissions for a [`SensorMask`].
pub const REQUEST_PERMISSION: MethodId = MethodId(2);
/// Start OS-scheduled passive background monitoring for a [`SensorMask`].
pub const START_PASSIVE: MethodId = MethodId(3);
/// Stop passive background monitoring, empty request.
pub const STOP_PASSIVE: MethodId = MethodId(4);
/// Start a continuous live wrist session ([`LiveOptions`]).
pub const START_LIVE: MethodId = MethodId(5);
/// Stop the active live wrist session, empty request.
pub const STOP_LIVE: MethodId = MethodId(6);
/// Drain the next bounded [`Batch`] of wrist readings, empty request.
pub const NEXT_BATCH: MethodId = MethodId(7);
/// Complete native method table.
pub const METHODS: &[MethodId] = &[
	CAPABILITIES,
	PERMISSION,
	REQUEST_PERMISSION,
	START_PASSIVE,
	STOP_PASSIVE,
	START_LIVE,
	STOP_LIVE,
	NEXT_BATCH,
];

/// Maximum readings returned in a single [`Batch`].
pub const MAX_BATCH: usize = 128;

/// Which wearable OS is serving this plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum PlatformKind {
	/// Not a watch host or unsupported device.
	Unsupported = 0,
	/// Android Wear OS (Pixel Watch, Samsung Galaxy Watch, etc.).
	WearOs = 1,
	/// Apple watchOS.
	WatchOs = 2,
	/// Huawei HarmonyOS Wearable.
	HarmonyOs = 3,
}

impl PlatformKind {
	fn from_code(code: u32) -> Result<Self, Error> {
		match code {
			0 => Ok(Self::Unsupported),
			1 => Ok(Self::WearOs),
			2 => Ok(Self::WatchOs),
			3 => Ok(Self::HarmonyOs),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Bitmask of wrist biometric and wear-state sensors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SensorMask(u32);

impl SensorMask {
	/// Every defined sensor bit.
	pub const ALL: Self = Self((1 << 12) - 1);
	/// Bioelectrical impedance body composition (e.g. Samsung Galaxy Watch
	/// BIA).
	pub const BODY_COMPOSITION: Self = Self(1 << 10);
	/// Active energy burned (kcal).
	pub const CALORIES: Self = Self(1 << 6);
	/// Distance travelled (m).
	pub const DISTANCE: Self = Self(1 << 7);
	/// Raw single-lead ECG waveform when exposed by the platform.
	pub const ECG: Self = Self(1 << 11);
	/// No sensors.
	pub const EMPTY: Self = Self(0);
	/// Floors climbed.
	pub const FLOORS: Self = Self(1 << 5);
	/// Optical heart rate (BPM).
	pub const HEART_RATE: Self = Self(1 << 0);
	/// Heart rate variability / inter-beat intervals (IBI / RMSSD).
	pub const HRV_IBI: Self = Self(1 << 1);
	/// Wrist / skin temperature (°C).
	pub const SKIN_TEMPERATURE: Self = Self(1 << 3);
	/// Blood oxygen saturation (`SpO2` %).
	pub const SPO2: Self = Self(1 << 2);
	/// Step count / cadence.
	pub const STEPS: Self = Self(1 << 4);
	/// Stress score.
	pub const STRESS: Self = Self(1 << 9);
	/// On-wrist / off-body detection.
	pub const WEAR_STATE: Self = Self(1 << 8);

	/// Raw wire mask.
	#[must_use]
	pub const fn bits(self) -> u32 {
		self.0
	}

	/// Decodes a validated mask.
	pub fn from_bits(bits: u32) -> Result<Self, Error> {
		(bits & !Self::ALL.0 == 0)
			.then_some(Self(bits))
			.ok_or(Error::InvalidArgument)
	}

	/// Bitwise union.
	#[must_use]
	pub const fn union(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}

	/// Whether all bits of `other` are set.
	#[must_use]
	pub const fn contains(self, other: Self) -> bool {
		(self.0 & other.0) == other.0
	}

	/// Whether no bits are set.
	#[must_use]
	pub const fn is_empty(self) -> bool {
		self.0 == 0
	}
}

/// Device-reported wearable sensor capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Capabilities {
	/// Which watch OS hosts the plugin.
	pub platform: PlatformKind,
	/// Sensors this watch hardware and OS can read.
	pub sensors: SensorMask,
	/// Supports low-power OS passive background delivery.
	pub passive_monitoring: bool,
	/// Supports continuous live sessions / workout builders.
	pub live_sessions: bool,
	/// Samsung Health Sensor SDK is available on this Galaxy Watch.
	pub samsung_sensor_sdk: bool,
	/// Huawei Health / Sensor Service Kit is available.
	pub huawei_sensor_kit: bool,
	/// Supports low-latency off-body / on-wrist detection.
	pub off_body_detection: bool,
}

const CAP_PASSIVE: u32 = 1 << 0;
const CAP_LIVE: u32 = 1 << 1;
const CAP_SAMSUNG: u32 = 1 << 2;
const CAP_HUAWEI: u32 = 1 << 3;
const CAP_OFF_BODY: u32 = 1 << 4;
const CAP_ALL_FLAGS: u32 = CAP_PASSIVE | CAP_LIVE | CAP_SAMSUNG | CAP_HUAWEI | CAP_OFF_BODY;

impl Capabilities {
	/// Encodes capabilities to the wire format.
	#[must_use]
	pub fn encode(&self) -> Vec<u8> {
		let mut flags = 0_u32;
		if self.passive_monitoring {
			flags |= CAP_PASSIVE;
		}
		if self.live_sessions {
			flags |= CAP_LIVE;
		}
		if self.samsung_sensor_sdk {
			flags |= CAP_SAMSUNG;
		}
		if self.huawei_sensor_kit {
			flags |= CAP_HUAWEI;
		}
		if self.off_body_detection {
			flags |= CAP_OFF_BODY;
		}
		let mut out = Vec::with_capacity(12);
		out.extend_from_slice(&(self.platform as u32).to_le_bytes());
		out.extend_from_slice(&self.sensors.bits().to_le_bytes());
		out.extend_from_slice(&flags.to_le_bytes());
		out
	}

	/// Decodes a capabilities reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut r = Reader::new(payload);
		let platform = PlatformKind::from_code(r.u32()?)?;
		let sensors = SensorMask::from_bits(r.u32()?)?;
		let flags = r.u32()?;
		r.finish()?;
		if flags & !CAP_ALL_FLAGS != 0 {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			platform,
			sensors,
			passive_monitoring: flags & CAP_PASSIVE != 0,
			live_sessions: flags & CAP_LIVE != 0,
			samsung_sensor_sdk: flags & CAP_SAMSUNG != 0,
			huawei_sensor_kit: flags & CAP_HUAWEI != 0,
			off_body_detection: flags & CAP_OFF_BODY != 0,
		})
	}
}

/// Body-sensor permission status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Permission {
	/// Not yet decided or platform hides read status (`HealthKit`).
	Unknown = 0,
	/// Granted for foreground / active use.
	GrantedForeground = 1,
	/// Granted for background / passive wrist monitoring.
	GrantedBackground = 2,
	/// Explicitly denied.
	Denied = 3,
	/// Not supported on this device.
	Unsupported = 4,
}

impl Permission {
	/// Decodes a permission reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut r = Reader::new(payload);
		let code = r.u32()?;
		r.finish()?;
		match code {
			0 => Ok(Self::Unknown),
			1 => Ok(Self::GrantedForeground),
			2 => Ok(Self::GrantedBackground),
			3 => Ok(Self::Denied),
			4 => Ok(Self::Unsupported),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Configuration for a continuous live wrist sensor session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveOptions {
	/// Sensors to stream.
	pub sensors: SensorMask,
	/// Target sampling interval in milliseconds (minimum 200 ms).
	pub interval_ms: u32,
	/// Optional platform workout activity code (0 = general monitoring).
	pub workout_activity: u32,
}

impl LiveOptions {
	/// Encodes a live-session start request.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		if self.sensors.is_empty() || self.interval_ms < 200 {
			return Err(Error::InvalidArgument);
		}
		let mut out = Vec::with_capacity(12);
		out.extend_from_slice(&self.sensors.bits().to_le_bytes());
		out.extend_from_slice(&self.interval_ms.to_le_bytes());
		out.extend_from_slice(&self.workout_activity.to_le_bytes());
		Ok(out)
	}
}

/// Accuracy of a wrist heart-rate measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Accuracy {
	/// Unknown or not reported.
	Unknown = 0,
	/// Sensor is not in contact with skin.
	NoContact = 1,
	/// Low confidence reading.
	Low = 2,
	/// Medium confidence reading.
	Medium = 3,
	/// High confidence reading.
	High = 4,
}

impl Accuracy {
	fn from_code(code: u32) -> Result<Self, Error> {
		match code {
			0 => Ok(Self::Unknown),
			1 => Ok(Self::NoContact),
			2 => Ok(Self::Low),
			3 => Ok(Self::Medium),
			4 => Ok(Self::High),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// One typed biometric or wear-state reading delivered from a watch.
#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
	/// Optical heart rate in beats per minute.
	HeartRate {
		/// Unix milliseconds.
		time_ms: i64,
		/// Beats per minute.
		bpm: f64,
		/// Confidence reported by the wrist sensor.
		accuracy: Accuracy,
	},
	/// Heart-rate variability (RMSSD and/or raw inter-beat intervals).
	HrvIbi {
		/// Unix milliseconds.
		time_ms: i64,
		/// RMSSD in milliseconds when computed by the watch.
		rmssd_ms: Option<f64>,
		/// Raw inter-beat intervals in milliseconds (e.g. Samsung IBI / PPG).
		rr_intervals_ms: Vec<f32>,
	},
	/// Blood oxygen saturation (`SpO2`).
	SpO2 {
		/// Unix milliseconds.
		time_ms: i64,
		/// Percent (0..=100).
		percent: f64,
	},
	/// Wrist or skin temperature.
	SkinTemperature {
		/// Unix milliseconds.
		time_ms: i64,
		/// Degrees Celsius.
		celsius: f64,
	},
	/// Steps accumulated over `start_ms..=end_ms`.
	Steps {
		/// Interval start in Unix milliseconds.
		start_ms: i64,
		/// Interval end in Unix milliseconds.
		end_ms: i64,
		/// Step count.
		count: u64,
	},
	/// Floors climbed over `start_ms..=end_ms`.
	Floors {
		/// Interval start in Unix milliseconds.
		start_ms: i64,
		/// Interval end in Unix milliseconds.
		end_ms: i64,
		/// Floors climbed.
		floors: f64,
	},
	/// Active energy burned over `start_ms..=end_ms`.
	Calories {
		/// Interval start in Unix milliseconds.
		start_ms: i64,
		/// Interval end in Unix milliseconds.
		end_ms: i64,
		/// Kilocalories.
		kilocalories: f64,
	},
	/// Distance travelled over `start_ms..=end_ms`.
	Distance {
		/// Interval start in Unix milliseconds.
		start_ms: i64,
		/// Interval end in Unix milliseconds.
		end_ms: i64,
		/// Meters.
		meters: f64,
	},
	/// On-wrist / off-body transition.
	WearState {
		/// Unix milliseconds.
		time_ms: i64,
		/// True while worn on the wrist.
		on_wrist: bool,
	},
	/// Watch-computed stress score (typically 0..=100).
	Stress {
		/// Unix milliseconds.
		time_ms: i64,
		/// Stress score.
		score: f64,
	},
	/// Body composition reading (e.g. Galaxy Watch BIA).
	BodyComposition {
		/// Unix milliseconds.
		time_ms: i64,
		/// Body fat percentage when measured.
		body_fat_percent: Option<f64>,
		/// Skeletal muscle mass in kg when measured.
		skeletal_muscle_kg: Option<f64>,
		/// Body water percentage when measured.
		water_percent: Option<f64>,
	},
}

const KIND_HR: u32 = 0;
const KIND_HRV: u32 = 1;
const KIND_SPO2: u32 = 2;
const KIND_SKIN_TEMP: u32 = 3;
const KIND_STEPS: u32 = 4;
const KIND_FLOORS: u32 = 5;
const KIND_CALORIES: u32 = 6;
const KIND_DISTANCE: u32 = 7;
const KIND_WEAR: u32 = 8;
const KIND_STRESS: u32 = 9;
const KIND_BIA: u32 = 10;

/// One bounded batch of wrist readings drained from the native ring buffer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Batch {
	/// Readings in arrival order.
	pub readings: Vec<Reading>,
	/// Readings dropped by the native buffer before the guest drained it.
	pub dropped: u32,
}

impl Batch {
	/// Encodes a batch to the wire format.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		if self.readings.len() > MAX_BATCH {
			return Err(Error::TooLarge);
		}
		let mut w = Writer::default();
		w.u32(self.dropped);
		w.u32(self.readings.len() as u32);
		for reading in &self.readings {
			encode_reading(&mut w, reading)?;
		}
		Ok(w.finish())
	}

	/// Decodes a batch from wire bytes.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut r = Reader::new(payload);
		let dropped = r.u32()?;
		let count = r.u32()? as usize;
		if count > MAX_BATCH || count.saturating_mul(12) > r.remaining() {
			return Err(Error::InvalidArgument);
		}
		let mut readings = Vec::with_capacity(count);
		for _ in 0..count {
			readings.push(decode_reading(&mut r)?);
		}
		r.finish()?;
		Ok(Self { readings, dropped })
	}
}

fn encode_reading(w: &mut Writer, reading: &Reading) -> Result<(), Error> {
	match reading {
		Reading::HeartRate {
			time_ms,
			bpm,
			accuracy,
		} => {
			w.u32(KIND_HR)
				.i64(*time_ms)
				.f64(*bpm)?
				.u32(*accuracy as u32);
		}
		Reading::HrvIbi {
			time_ms,
			rmssd_ms,
			rr_intervals_ms,
		} => {
			if rr_intervals_ms.len() > 256 {
				return Err(Error::TooLarge);
			}
			w.u32(KIND_HRV)
				.i64(*time_ms)
				.opt_f64(*rmssd_ms)?
				.u32(rr_intervals_ms.len() as u32);
			for &rr in rr_intervals_ms {
				w.f32(rr)?;
			}
		}
		Reading::SpO2 { time_ms, percent } => {
			w.u32(KIND_SPO2).i64(*time_ms).f64(*percent)?;
		}
		Reading::SkinTemperature { time_ms, celsius } => {
			w.u32(KIND_SKIN_TEMP).i64(*time_ms).f64(*celsius)?;
		}
		Reading::Steps {
			start_ms,
			end_ms,
			count,
		} => {
			if end_ms < start_ms {
				return Err(Error::InvalidArgument);
			}
			w.u32(KIND_STEPS)
				.i64(*start_ms)
				.i64(*end_ms)
				.i64(i64::try_from(*count).map_err(|_| Error::InvalidArgument)?);
		}
		Reading::Floors {
			start_ms,
			end_ms,
			floors,
		} => {
			if end_ms < start_ms {
				return Err(Error::InvalidArgument);
			}
			w.u32(KIND_FLOORS)
				.i64(*start_ms)
				.i64(*end_ms)
				.f64(*floors)?;
		}
		Reading::Calories {
			start_ms,
			end_ms,
			kilocalories,
		} => {
			if end_ms < start_ms {
				return Err(Error::InvalidArgument);
			}
			w.u32(KIND_CALORIES)
				.i64(*start_ms)
				.i64(*end_ms)
				.f64(*kilocalories)?;
		}
		Reading::Distance {
			start_ms,
			end_ms,
			meters,
		} => {
			if end_ms < start_ms {
				return Err(Error::InvalidArgument);
			}
			w.u32(KIND_DISTANCE)
				.i64(*start_ms)
				.i64(*end_ms)
				.f64(*meters)?;
		}
		Reading::WearState { time_ms, on_wrist } => {
			w.u32(KIND_WEAR).i64(*time_ms).u32(u32::from(*on_wrist));
		}
		Reading::Stress { time_ms, score } => {
			w.u32(KIND_STRESS).i64(*time_ms).f64(*score)?;
		}
		Reading::BodyComposition {
			time_ms,
			body_fat_percent,
			skeletal_muscle_kg,
			water_percent,
		} => {
			w.u32(KIND_BIA)
				.i64(*time_ms)
				.opt_f64(*body_fat_percent)?
				.opt_f64(*skeletal_muscle_kg)?
				.opt_f64(*water_percent)?;
		}
	}
	Ok(())
}

fn decode_reading(r: &mut Reader<'_>) -> Result<Reading, Error> {
	match r.u32()? {
		KIND_HR => {
			Ok(Reading::HeartRate {
				time_ms: r.i64()?,
				bpm: r.f64()?,
				accuracy: Accuracy::from_code(r.u32()?)?,
			})
		}
		KIND_HRV => {
			let time_ms = r.i64()?;
			let rmssd_ms = r.opt_f64()?;
			let count = r.u32()? as usize;
			if count > 256 || count.saturating_mul(4) > r.remaining() {
				return Err(Error::InvalidArgument);
			}
			let mut rr_intervals_ms = Vec::with_capacity(count);
			for _ in 0..count {
				rr_intervals_ms.push(r.f32()?);
			}
			Ok(Reading::HrvIbi {
				time_ms,
				rmssd_ms,
				rr_intervals_ms,
			})
		}
		KIND_SPO2 => {
			Ok(Reading::SpO2 {
				time_ms: r.i64()?,
				percent: r.f64()?,
			})
		}
		KIND_SKIN_TEMP => {
			Ok(Reading::SkinTemperature {
				time_ms: r.i64()?,
				celsius: r.f64()?,
			})
		}
		KIND_STEPS => {
			let start_ms = r.i64()?;
			let end_ms = r.i64()?;
			let count = u64::try_from(r.i64()?).map_err(|_| Error::InvalidArgument)?;
			if end_ms < start_ms {
				return Err(Error::InvalidArgument);
			}
			Ok(Reading::Steps {
				start_ms,
				end_ms,
				count,
			})
		}
		KIND_FLOORS => {
			let start_ms = r.i64()?;
			let end_ms = r.i64()?;
			let floors = r.f64()?;
			if end_ms < start_ms {
				return Err(Error::InvalidArgument);
			}
			Ok(Reading::Floors {
				start_ms,
				end_ms,
				floors,
			})
		}
		KIND_CALORIES => {
			let start_ms = r.i64()?;
			let end_ms = r.i64()?;
			let kilocalories = r.f64()?;
			if end_ms < start_ms {
				return Err(Error::InvalidArgument);
			}
			Ok(Reading::Calories {
				start_ms,
				end_ms,
				kilocalories,
			})
		}
		KIND_DISTANCE => {
			let start_ms = r.i64()?;
			let end_ms = r.i64()?;
			let meters = r.f64()?;
			if end_ms < start_ms {
				return Err(Error::InvalidArgument);
			}
			Ok(Reading::Distance {
				start_ms,
				end_ms,
				meters,
			})
		}
		KIND_WEAR => {
			let time_ms = r.i64()?;
			let flag = r.u32()?;
			let on_wrist = match flag {
				0 => false,
				1 => true,
				_ => return Err(Error::InvalidArgument),
			};
			Ok(Reading::WearState { time_ms, on_wrist })
		}
		KIND_STRESS => {
			Ok(Reading::Stress {
				time_ms: r.i64()?,
				score: r.f64()?,
			})
		}
		KIND_BIA => {
			Ok(Reading::BodyComposition {
				time_ms: r.i64()?,
				body_fat_percent: r.opt_f64()?,
				skeletal_muscle_kg: r.opt_f64()?,
				water_percent: r.opt_f64()?,
			})
		}
		_ => Err(Error::InvalidArgument),
	}
}

/// Encodes a [`SensorMask`] request for permission or passive monitoring.
pub fn mask_request(mask: SensorMask) -> Result<Vec<u8>, Error> {
	if mask.is_empty() {
		return Err(Error::InvalidArgument);
	}
	Ok(mask.bits().to_le_bytes().to_vec())
}

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
	fn u32(&mut self, value: u32) -> &mut Self {
		self.0.extend_from_slice(&value.to_le_bytes());
		self
	}

	fn i64(&mut self, value: i64) -> &mut Self {
		self.0.extend_from_slice(&value.to_le_bytes());
		self
	}

	fn f32(&mut self, value: f32) -> Result<&mut Self, Error> {
		if !value.is_finite() {
			return Err(Error::InvalidArgument);
		}
		self.0.extend_from_slice(&value.to_le_bytes());
		Ok(self)
	}

	fn f64(&mut self, value: f64) -> Result<&mut Self, Error> {
		if !value.is_finite() {
			return Err(Error::InvalidArgument);
		}
		self.0.extend_from_slice(&value.to_le_bytes());
		Ok(self)
	}

	fn opt_f64(&mut self, value: Option<f64>) -> Result<&mut Self, Error> {
		match value {
			Some(v) => {
				self.u32(1);
				self.f64(v)
			}
			None => Ok(self.u32(0)),
		}
	}

	fn finish(self) -> Vec<u8> {
		self.0
	}
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
	fn new(bytes: &'a [u8]) -> Self {
		Self(bytes)
	}

	fn remaining(&self) -> usize {
		self.0.len()
	}

	fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
		let (head, tail) = self.0.split_at_checked(N).ok_or(Error::InvalidArgument)?;
		self.0 = tail;
		head.try_into().map_err(|_| Error::InvalidArgument)
	}

	fn u32(&mut self) -> Result<u32, Error> {
		self.take().map(u32::from_le_bytes)
	}

	fn i64(&mut self) -> Result<i64, Error> {
		self.take().map(i64::from_le_bytes)
	}

	fn f32(&mut self) -> Result<f32, Error> {
		let value = f32::from_le_bytes(self.take()?);
		value
			.is_finite()
			.then_some(value)
			.ok_or(Error::InvalidArgument)
	}

	fn f64(&mut self) -> Result<f64, Error> {
		let value = f64::from_le_bytes(self.take()?);
		value
			.is_finite()
			.then_some(value)
			.ok_or(Error::InvalidArgument)
	}

	fn opt_f64(&mut self) -> Result<Option<f64>, Error> {
		match self.u32()? {
			0 => Ok(None),
			1 => self.f64().map(Some),
			_ => Err(Error::InvalidArgument),
		}
	}

	fn finish(self) -> Result<(), Error> {
		self.0
			.is_empty()
			.then_some(())
			.ok_or(Error::InvalidArgument)
	}
}

/// Native guest client. Requests never start until polled.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Cached bridge handle.
	#[derive(Debug, Clone, Copy)]
	pub struct Wearable(leptoskit_bridge::guest::Client);

	impl Wearable {
		/// Resolves the selected plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reports watch OS kind and supported wrist sensors.
		pub async fn capabilities(self) -> Result<Capabilities, Error> {
			Capabilities::decode(&self.0.call(CAPABILITIES, &[])?.await?)
		}

		/// Current body-sensor permission state.
		pub async fn permission(self) -> Result<Permission, Error> {
			Permission::decode(&self.0.call(PERMISSION, &[])?.await?)
		}

		/// Requests permissions for `mask`.
		pub async fn request_permission(self, mask: SensorMask) -> Result<Permission, Error> {
			let req = mask_request(mask)?;
			Permission::decode(&self.0.call(REQUEST_PERMISSION, &req)?.await?)
		}

		/// Starts OS-scheduled passive background monitoring.
		pub async fn start_passive(self, mask: SensorMask) -> Result<(), Error> {
			let req = mask_request(mask)?;
			self.0.call(START_PASSIVE, &req)?.await?;
			Ok(())
		}

		/// Stops passive background monitoring.
		pub async fn stop_passive(self) -> Result<(), Error> {
			self.0.call(STOP_PASSIVE, &[])?.await?;
			Ok(())
		}

		/// Starts a live continuous wrist sensor session.
		pub async fn start_live(self, options: LiveOptions) -> Result<(), Error> {
			let req = options.encode()?;
			self.0.call(START_LIVE, &req)?.await?;
			Ok(())
		}

		/// Stops the active live wrist sensor session.
		pub async fn stop_live(self) -> Result<(), Error> {
			self.0.call(STOP_LIVE, &[])?.await?;
			Ok(())
		}

		/// Drains the next batch of wrist readings.
		pub async fn next_batch(self) -> Result<Batch, Error> {
			Batch::decode(&self.0.call(NEXT_BATCH, &[])?.await?)
		}
	}
}

/// Browsers have no direct wrist biometric sensors.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Reports unsupported capabilities.
	pub fn capabilities() -> Capabilities {
		Capabilities {
			platform: PlatformKind::Unsupported,
			sensors: SensorMask::EMPTY,
			passive_monitoring: false,
			live_sessions: false,
			samsung_sensor_sdk: false,
			huawei_sensor_kit: false,
			off_body_detection: false,
		}
	}
}

/// Desktops have no direct wrist biometric sensors.
#[cfg(not(target_arch = "wasm32"))]
pub mod desktop {
	use super::*;

	/// Reports unsupported capabilities.
	pub fn capabilities() -> Capabilities {
		Capabilities {
			platform: PlatformKind::Unsupported,
			sensors: SensorMask::EMPTY,
			passive_monitoring: false,
			live_sessions: false,
			samsung_sensor_sdk: false,
			huawei_sensor_kit: false,
			off_body_detection: false,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn methods_are_dense_and_in_order() {
		for (index, method) in METHODS.iter().enumerate() {
			assert_eq!(method.0 as usize, index);
		}
	}

	#[test]
	fn capabilities_and_batches_round_trip() {
		let caps = Capabilities {
			platform: PlatformKind::WearOs,
			sensors: SensorMask::HEART_RATE
				.union(SensorMask::STEPS)
				.union(SensorMask::SPO2)
				.union(SensorMask::BODY_COMPOSITION),
			passive_monitoring: true,
			live_sessions: true,
			samsung_sensor_sdk: true,
			huawei_sensor_kit: false,
			off_body_detection: true,
		};
		assert_eq!(Capabilities::decode(&caps.encode()), Ok(caps));

		let batch = Batch {
			dropped: 2,
			readings: vec![
				Reading::HeartRate {
					time_ms: 1_700_000_000_000,
					bpm: 72.5,
					accuracy: Accuracy::High,
				},
				Reading::HrvIbi {
					time_ms: 1_700_000_001_000,
					rmssd_ms: Some(42.3),
					rr_intervals_ms: vec![810.0, 825.5, 795.0],
				},
				Reading::SpO2 {
					time_ms: 1_700_000_002_000,
					percent: 98.0,
				},
				Reading::SkinTemperature {
					time_ms: 1_700_000_003_000,
					celsius: 36.2,
				},
				Reading::Steps {
					start_ms: 1_700_000_000_000,
					end_ms: 1_700_000_060_000,
					count: 120,
				},
				Reading::WearState {
					time_ms: 1_700_000_004_000,
					on_wrist: true,
				},
				Reading::Stress {
					time_ms: 1_700_000_005_000,
					score: 28.0,
				},
				Reading::BodyComposition {
					time_ms: 1_700_000_006_000,
					body_fat_percent: Some(18.5),
					skeletal_muscle_kg: Some(34.2),
					water_percent: Some(59.0),
				},
			],
		};
		let bytes = batch.encode().unwrap();
		assert_eq!(Batch::decode(&bytes), Ok(batch));
	}

	#[test]
	fn invalid_masks_and_options_are_rejected() {
		assert_eq!(SensorMask::from_bits(1 << 20), Err(Error::InvalidArgument));
		assert_eq!(mask_request(SensorMask::EMPTY), Err(Error::InvalidArgument));
		assert_eq!(
			LiveOptions {
				sensors: SensorMask::HEART_RATE,
				interval_ms: 100,
				workout_activity: 0,
			}
			.encode(),
			Err(Error::InvalidArgument)
		);
	}
}
