//! Sensor-first wearable application API.
//!
//! Trackers, collectors, and ambient health apps spend almost all of their
//! lifetime with the watch screen off. Instead of starting from a phone-style
//! view tree, [`WearableApp`] starts from the biometric and motion streams the
//! watch records, its power/sampling policy, and how batches leave the wrist
//! (directly over Wi-Fi/LTE, through a paired phone companion, or both).
//!
//! For the brief moments the wearer raises their wrist, [`WearableApp`] either
//! synthesizes a budget-checked minimal status [`WatchApp`] (interactive page,
//! tile or glance, and face complication) from a typed [`WearableStatus`], or
//! presents an explicit custom [`WatchApp`].

use std::collections::BTreeSet;

use leptoskit_native::components::Button;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;

use crate::ComplicationFamily;
use crate::RefreshPolicy;
use crate::WatchApp;
use crate::WatchComplication;
use crate::WatchError;
use crate::WatchGlance;
use crate::WatchKind;
use crate::WatchNavigation;
use crate::WatchPage;
use crate::WatchTile;

/// A biometric, motion, environment, or device stream a wearable can record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u32)]
pub enum WearableStream {
	/// Instantaneous or passive heart rate (BPM).
	HeartRate = 0,
	/// Resting heart rate (BPM).
	RestingHeartRate = 1,
	/// Heart rate variability (RMSSD / SDNN / IBI).
	HeartRateVariability = 2,
	/// Blood oxygen saturation (`SpO2` %).
	OxygenSaturation = 3,
	/// Wrist or skin temperature (°C or baseline delta).
	SkinTemperature = 4,
	/// Breaths per minute.
	RespiratoryRate = 5,
	/// Step count per interval or boot counter.
	Steps = 6,
	/// Floors or flights climbed.
	FloorsClimbed = 7,
	/// Active energy burned (kcal).
	ActiveCalories = 8,
	/// Distance travelled (meters).
	Distance = 9,
	/// Sleep sessions and stages.
	Sleep = 10,
	/// Exercise / workout sessions.
	Workout = 11,
	/// On-wrist / off-body wear detection.
	WearState = 12,
	/// Body composition (BIA on supported watches such as Galaxy Watch).
	BodyComposition = 13,
	/// Stress score when exposed by the wearable OS.
	Stress = 14,
	/// Raw electrocardiogram waveform when exposed.
	Ecg = 15,
	/// Tri-axial accelerometer samples.
	Accelerometer = 16,
	/// Tri-axial gyroscope samples.
	Gyroscope = 17,
	/// Barometric pressure / elevation samples.
	Barometer = 18,
	/// GPS / GNSS location fixes.
	Location = 19,
	/// Battery level and charging state.
	Battery = 20,
	/// Thermal state for power-budgeted collection.
	Thermal = 21,
	/// Paired BLE GATT heart-rate / fitness straps or rings.
	BleSensors = 22,
}

impl WearableStream {
	/// Every stream in wire order.
	pub const ALL: [Self; 23] = [
		Self::HeartRate,
		Self::RestingHeartRate,
		Self::HeartRateVariability,
		Self::OxygenSaturation,
		Self::SkinTemperature,
		Self::RespiratoryRate,
		Self::Steps,
		Self::FloorsClimbed,
		Self::ActiveCalories,
		Self::Distance,
		Self::Sleep,
		Self::Workout,
		Self::WearState,
		Self::BodyComposition,
		Self::Stress,
		Self::Ecg,
		Self::Accelerometer,
		Self::Gyroscope,
		Self::Barometer,
		Self::Location,
		Self::Battery,
		Self::Thermal,
		Self::BleSensors,
	];

	/// Stable wire / bit index.
	#[must_use]
	pub const fn bit(self) -> u32 {
		1 << (self as u32)
	}

	/// Stable lowercase identifier for diagnostics and telemetry.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::HeartRate => "heart_rate",
			Self::RestingHeartRate => "resting_heart_rate",
			Self::HeartRateVariability => "hrv",
			Self::OxygenSaturation => "spo2",
			Self::SkinTemperature => "skin_temperature",
			Self::RespiratoryRate => "respiratory_rate",
			Self::Steps => "steps",
			Self::FloorsClimbed => "floors_climbed",
			Self::ActiveCalories => "active_calories",
			Self::Distance => "distance",
			Self::Sleep => "sleep",
			Self::Workout => "workout",
			Self::WearState => "wear_state",
			Self::BodyComposition => "body_composition",
			Self::Stress => "stress",
			Self::Ecg => "ecg",
			Self::Accelerometer => "accelerometer",
			Self::Gyroscope => "gyroscope",
			Self::Barometer => "barometer",
			Self::Location => "location",
			Self::Battery => "battery",
			Self::Thermal => "thermal",
			Self::BleSensors => "ble_sensors",
		}
	}

	/// Whether this stream carries sensitive health / biometric data.
	#[must_use]
	pub const fn is_biometric(self) -> bool {
		matches!(
			self,
			Self::HeartRate
				| Self::RestingHeartRate
				| Self::HeartRateVariability
				| Self::OxygenSaturation
				| Self::SkinTemperature
				| Self::RespiratoryRate
				| Self::Steps
				| Self::FloorsClimbed
				| Self::ActiveCalories
				| Self::Distance
				| Self::Sleep
				| Self::Workout
				| Self::WearState
				| Self::BodyComposition
				| Self::Stress
				| Self::Ecg
		)
	}
}

/// A compact bitset of [`WearableStream`] subscriptions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct WearableStreams(u32);

impl WearableStreams {
	/// Every defined stream.
	pub const ALL: Self = Self((1 << WearableStream::ALL.len()) - 1);
	/// Empty set.
	pub const EMPTY: Self = Self(0);

	/// Builds a set from a slice of streams.
	#[must_use]
	pub fn of(streams: &[WearableStream]) -> Self {
		streams.iter().copied().collect()
	}

	/// Raw wire mask.
	#[must_use]
	pub const fn bits(self) -> u32 {
		self.0
	}

	/// Inserts `stream`.
	pub fn insert(&mut self, stream: WearableStream) {
		self.0 |= stream.bit();
	}

	/// Whether `stream` is included.
	#[must_use]
	pub const fn contains(self, stream: WearableStream) -> bool {
		self.0 & stream.bit() != 0
	}

	/// Whether no streams are selected.
	#[must_use]
	pub const fn is_empty(self) -> bool {
		self.0 == 0
	}

	/// Number of selected streams.
	#[must_use]
	pub const fn len(self) -> usize {
		self.0.count_ones() as usize
	}

	/// Iterates selected streams in wire order.
	pub fn iter(self) -> impl Iterator<Item = WearableStream> {
		WearableStream::ALL
			.into_iter()
			.filter(move |stream| self.contains(*stream))
	}
}

impl FromIterator<WearableStream> for WearableStreams {
	fn from_iter<I: IntoIterator<Item = WearableStream>>(iter: I) -> Self {
		let mut set = Self::EMPTY;
		for stream in iter {
			set.insert(stream);
		}
		set
	}
}

/// How aggressively the wearable samples sensors in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SamplingPolicy {
	/// OS-scheduled low-power passive background delivery
	/// (`PassiveMonitoringClient` on Wear OS, `HKObserverQuery` /
	/// background delivery on watchOS, periodic health/sensor wakes on
	/// HarmonyOS). Best default for all-day tracking.
	Passive,
	/// Periodic wakes every `interval_secs` seconds to sample and flush
	/// batches.
	Balanced {
		/// Seconds between scheduled collection passes.
		interval_secs: u32,
	},
	/// Active continuous wrist session (`MeasureClient` + health foreground
	/// service on Wear OS, `HKWorkoutSession` / `WKExtendedRuntimeSession` on
	/// watchOS, `HEALTH` continuous background task on HarmonyOS).
	Continuous,
	/// High-frequency workout session with optional GPS and live biometrics.
	Workout {
		/// Activity name (e.g. `"running"`, `"cycling"`, `"walking"`).
		activity: String,
	},
}

/// How collected readings leave the watch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DeliveryMode {
	/// Upload directly from the watch over Wi-Fi / LTE.
	Direct,
	/// Ferry readings through the paired phone (`WatchConnectivity`, Wear Data
	/// Layer, or Huawei Wear Engine).
	Companion,
	/// Upload directly when network is available, and fall back to the paired
	/// phone companion when offline or during initial pairing.
	#[default]
	DirectWithCompanionFallback,
}

/// Typed, low-boilerplate glanceable status rendered onto watch surfaces.
///
/// Collector-heavy wearable apps update this struct instead of managing a
/// complex view hierarchy; [`WearableApp::render_status_app`] turns it into a
/// budget-checked page, tile/glance, and face complication.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WearableStatus {
	/// Short status title (e.g. `"Recording"`, `"Paused"`, `"Pair watch"`).
	pub headline: String,
	/// Whether collection is currently running.
	pub recording: bool,
	/// Whether the watch is paired with a destination or companion phone.
	pub paired: bool,
	/// Latest measured heart rate in beats per minute, when known.
	pub heart_rate_bpm: Option<u16>,
	/// Steps counted today, when known.
	pub steps_today: Option<u64>,
	/// Number of spooled readings waiting to be delivered.
	pub queued_readings: u64,
	/// Watch battery percentage (0..=100), when known.
	pub battery_percent: Option<u8>,
	/// Optional short one-line notice (e.g. `"Via phone"`, `"Needs sensor
	/// ok"`).
	pub notice: Option<String>,
}

/// How a [`WearableApp`] draws its on-watch UI.
#[derive(Debug, Clone)]
pub enum WearableSurfaceMode {
	/// Automatically synthesize a budget-safe status page, tile/glance, and
	/// circular face complication from [`WearableStatus`].
	AutoStatus,
	/// Present an explicit app-authored [`WatchApp`] instead of the generated
	/// status surfaces.
	Custom(WatchApp),
}

/// Sensor-first wearable application definition.
///
/// Declare the streams, sampling policy, and delivery mode once; then either
/// use [`WearableApp::auto_status_surface`] for zero-boilerplate glanceable UI
/// or [`WearableApp::custom_surface`] for bespoke watch pages and tiles.
#[derive(Debug, Clone)]
pub struct WearableApp {
	id: String,
	kind: WatchKind,
	streams: WearableStreams,
	policy: SamplingPolicy,
	delivery: DeliveryMode,
	surface: WearableSurfaceMode,
}

impl WearableApp {
	/// Starts a new sensor-first wearable app for `kind` with passive sampling,
	/// direct-with-companion delivery, and an auto-generated status surface.
	#[must_use]
	pub fn new(id: impl Into<String>, kind: WatchKind) -> Self {
		Self {
			id: id.into(),
			kind,
			streams: WearableStreams::EMPTY,
			policy: SamplingPolicy::Passive,
			delivery: DeliveryMode::DirectWithCompanionFallback,
			surface: WearableSurfaceMode::AutoStatus,
		}
	}

	/// Adds a slice of streams to collect.
	#[must_use]
	pub fn streams(mut self, streams: &[WearableStream]) -> Self {
		for &stream in streams {
			self.streams.insert(stream);
		}
		self
	}

	/// Adds one stream to collect.
	#[must_use]
	pub fn stream(mut self, stream: WearableStream) -> Self {
		self.streams.insert(stream);
		self
	}

	/// Sets the background sampling policy.
	#[must_use]
	pub fn policy(mut self, policy: SamplingPolicy) -> Self {
		self.policy = policy;
		self
	}

	/// Sets how collected batches leave the wrist.
	#[must_use]
	pub fn delivery(mut self, delivery: DeliveryMode) -> Self {
		self.delivery = delivery;
		self
	}

	/// Uses the built-in minimal glanceable status page, tile/glance, and
	/// complication rendered from [`WearableStatus`].
	#[must_use]
	pub fn auto_status_surface(mut self) -> Self {
		self.surface = WearableSurfaceMode::AutoStatus;
		self
	}

	/// Uses a custom app-authored [`WatchApp`] for on-watch UI surfaces.
	#[must_use]
	pub fn custom_surface(mut self, app: WatchApp) -> Self {
		self.surface = WearableSurfaceMode::Custom(app);
		self
	}

	/// App id.
	#[must_use]
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Target watch operating system.
	#[must_use]
	pub fn kind(&self) -> WatchKind {
		self.kind
	}

	/// Declared sensor and biometric streams.
	#[must_use]
	pub fn declared_streams(&self) -> WearableStreams {
		self.streams
	}

	/// Configured sampling policy.
	#[must_use]
	pub fn sampling_policy(&self) -> &SamplingPolicy {
		&self.policy
	}

	/// Configured batch delivery mode.
	#[must_use]
	pub fn delivery_mode(&self) -> DeliveryMode {
		self.delivery
	}

	/// Configured UI surface mode.
	#[must_use]
	pub fn surface_mode(&self) -> &WearableSurfaceMode {
		&self.surface
	}

	/// Validates id, streams, sampling policy, and any custom watch surfaces.
	pub fn validate(&self) -> Result<(), WatchError> {
		crate::page::validate_id(&self.id)?;

		if self.streams.is_empty() {
			return Err(WatchError::NoStreams);
		}

		if let SamplingPolicy::Balanced { interval_secs: 0 } = self.policy {
			return Err(WatchError::InvalidInterval);
		}

		if let WearableSurfaceMode::Custom(app) = &self.surface {
			app.validate()?;
			if app.kind() != self.kind {
				return Err(WatchError::Unavailable(format!(
					"wearable app targets {} but custom WatchApp targets {}",
					self.kind.display_name(),
					app.kind().display_name()
				)));
			}
		}

		Ok(())
	}

	/// Platform permissions or usage-description keys needed for the declared
	/// streams and sampling policy on [`Self::kind`].
	#[must_use]
	pub fn required_permissions(&self) -> Vec<&'static str> {
		let mut perms = BTreeSet::new();
		let has_biometrics = self.streams.iter().any(WearableStream::is_biometric);
		let has_steps = self.streams.contains(WearableStream::Steps)
			|| self.streams.contains(WearableStream::FloorsClimbed)
			|| self.streams.contains(WearableStream::Distance);
		let has_location = self.streams.contains(WearableStream::Location);
		let has_ble = self.streams.contains(WearableStream::BleSensors);
		let continuous = matches!(
			self.policy,
			SamplingPolicy::Continuous | SamplingPolicy::Workout { .. }
		);

		match self.kind {
			WatchKind::WearOs => {
				if has_biometrics {
					perms.insert("android.permission.BODY_SENSORS");
					perms.insert("android.permission.BODY_SENSORS_BACKGROUND");
				}
				if has_steps {
					perms.insert("android.permission.ACTIVITY_RECOGNITION");
				}
				if has_location {
					perms.insert("android.permission.ACCESS_FINE_LOCATION");
					perms.insert("android.permission.ACCESS_BACKGROUND_LOCATION");
				}
				if has_ble {
					perms.insert("android.permission.BLUETOOTH_SCAN");
					perms.insert("android.permission.BLUETOOTH_CONNECT");
				}
				if continuous {
					perms.insert("android.permission.FOREGROUND_SERVICE");
					perms.insert("android.permission.FOREGROUND_SERVICE_HEALTH");
				}
				if self.streams.contains(WearableStream::Accelerometer)
					|| self.streams.contains(WearableStream::Gyroscope)
				{
					perms.insert("android.permission.HIGH_SAMPLING_RATE_SENSORS");
				}
			}
			WatchKind::WatchOs => {
				if has_biometrics {
					perms.insert("NSHealthShareUsageDescription");
					if continuous {
						perms.insert("NSHealthUpdateUsageDescription");
					}
				}
				if has_steps
					|| self.streams.contains(WearableStream::Accelerometer)
					|| self.streams.contains(WearableStream::Gyroscope)
				{
					perms.insert("NSMotionUsageDescription");
				}
				if has_location {
					perms.insert("NSLocationWhenInUseUsageDescription");
					perms.insert("NSLocationAlwaysAndWhenInUseUsageDescription");
				}
				if has_ble {
					perms.insert("NSBluetoothAlwaysUsageDescription");
				}
			}
			WatchKind::HarmonyOs => {
				if has_biometrics {
					perms.insert("ohos.permission.READ_HEALTH_DATA");
				}
				if has_steps {
					perms.insert("ohos.permission.ACTIVITY_MOTION");
				}
				if self.streams.contains(WearableStream::Accelerometer) {
					perms.insert("ohos.permission.ACCELEROMETER");
				}
				if self.streams.contains(WearableStream::Gyroscope) {
					perms.insert("ohos.permission.GYROSCOPE");
				}
				if has_location {
					perms.insert("ohos.permission.APPROXIMATELY_LOCATION");
					perms.insert("ohos.permission.LOCATION");
					perms.insert("ohos.permission.LOCATION_IN_BACKGROUND");
				}
				if has_ble {
					perms.insert("ohos.permission.ACCESS_BLUETOOTH");
				}
				if continuous || has_biometrics {
					perms.insert("ohos.permission.KEEP_BACKGROUND_RUNNING");
				}
			}
		}

		perms.into_iter().collect()
	}

	/// Renders the on-watch [`WatchApp`] for `status`.
	///
	/// For [`WearableSurfaceMode::AutoStatus`], builds a validated, budget-safe
	/// status page, a tile (Wear OS / HarmonyOS) or glance (watchOS), and a
	/// circular face complication. For [`WearableSurfaceMode::Custom`], returns
	/// a validated clone of the custom app.
	pub fn render_status_app(&self, status: &WearableStatus) -> Result<WatchApp, WatchError> {
		self.validate()?;

		if let WearableSurfaceMode::Custom(app) = &self.surface {
			return Ok(app.clone());
		}

		let headline = clamp_chars(
			if status.headline.trim().is_empty() {
				if status.recording {
					"Recording"
				} else {
					"Paused"
				}
			} else {
				status.headline.trim()
			},
			24,
		);
		let hr_label = status
			.heart_rate_bpm
			.map_or_else(|| "-- bpm".to_owned(), |bpm| format!("{bpm} bpm"));
		let steps_label = status
			.steps_today
			.map_or_else(|| "-- steps".to_owned(), |steps| format!("{steps} steps"));
		let queue_label = if status.paired {
			if status.queued_readings == 0 {
				"Synced".to_owned()
			} else {
				format!("{} queued", status.queued_readings)
			}
		} else {
			"Not paired".to_owned()
		};
		let action_label = if status.recording { "Pause" } else { "Start" };

		let mut page_root = View()
			.class("items-center justify-center gap-1 p-2")
			.child(Text(headline.clone()).class("text-sm font-bold"))
			.child(Text(hr_label.clone()).class("text-xl font-bold"))
			.child(Text(steps_label.clone()).class("text-xs"))
			.child(Text(queue_label).class("text-xs"));
		if let Some(notice) = status
			.notice
			.as_deref()
			.map(str::trim)
			.filter(|s| !s.is_empty())
		{
			page_root = page_root.child(Text(clamp_chars(notice, 28)).class("text-xs"));
		}
		page_root = page_root.child(Button(action_label));

		let mut app = WatchApp::new(self.id.clone(), self.kind)
			.navigation(WatchNavigation::Single)
			.page(WatchPage::new("status", page_root).title(headline.clone()));

		let compact_metric = status
			.heart_rate_bpm
			.map(|bpm| format!("{bpm} bpm"))
			.or_else(|| status.steps_today.map(compact_steps))
			.unwrap_or_else(|| {
				if status.recording {
					"ON".into()
				} else {
					"OFF".into()
				}
			});

		match self.kind {
			WatchKind::WearOs | WatchKind::HarmonyOs => {
				let tile_tree = View()
					.class("items-center justify-center gap-1")
					.child(Text(headline.clone()).class("text-xs"))
					.child(Text(hr_label).class("text-lg font-bold"))
					.child(Text(steps_label).class("text-xs"));
				app = app.tile(
					WatchTile::new("status-tile", tile_tree).refresh(RefreshPolicy::QuarterHour),
				);
			}
			WatchKind::WatchOs => {
				let preview = Text(clamp_chars(&compact_metric, 14));
				let body = View()
					.class("items-center justify-center gap-1")
					.child(Text(clamp_chars(&headline, 16)))
					.child(Text(clamp_chars(&hr_label, 12)));
				app = app.glance(WatchGlance::new("status-glance", preview, body));
			}
		}

		let comp_text = clamp_chars(
			&status
				.heart_rate_bpm
				.map(|bpm| format!("{bpm}"))
				.or_else(|| status.steps_today.map(compact_steps))
				.unwrap_or_else(|| {
					if status.recording {
						"REC".into()
					} else {
						"--".into()
					}
				}),
			10,
		);
		app = app.complication(WatchComplication::new(
			"status-complication",
			ComplicationFamily::CircularSmall,
			Text(comp_text),
		));

		app.validate()?;
		Ok(app)
	}
}

fn compact_steps(steps: u64) -> String {
	if steps >= 1_000 {
		format!("{:.1}k", steps as f64 / 1_000.0)
	} else {
		steps.to_string()
	}
}

fn clamp_chars(input: &str, max_chars: usize) -> String {
	input.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn wearable_app_validates_streams_and_renders_budget_safe_surfaces() {
		for kind in [WatchKind::WearOs, WatchKind::WatchOs, WatchKind::HarmonyOs] {
			let app = WearableApp::new("quant-watch", kind)
				.streams(&[
					WearableStream::HeartRate,
					WearableStream::Steps,
					WearableStream::OxygenSaturation,
					WearableStream::Location,
					WearableStream::BleSensors,
				])
				.policy(SamplingPolicy::Passive)
				.delivery(DeliveryMode::DirectWithCompanionFallback);

			let status = WearableStatus {
				headline: "Recording".into(),
				recording: true,
				paired: true,
				heart_rate_bpm: Some(68),
				steps_today: Some(8_432),
				queued_readings: 12,
				battery_percent: Some(81),
				notice: Some("Direct sync".into()),
			};

			let rendered = app.render_status_app(&status).unwrap();
			assert_eq!(rendered.kind(), kind);
			assert_eq!(rendered.pages().len(), 1);
			assert_eq!(rendered.ambients().len(), 2);
			assert_ne!(app.required_permissions(), [] as [&str; 0]);
		}
	}

	#[test]
	fn wearable_app_rejects_empty_streams_and_zero_interval() {
		let empty = WearableApp::new("empty", WatchKind::WearOs);
		assert_eq!(empty.validate(), Err(WatchError::NoStreams));

		let zero = WearableApp::new("zero", WatchKind::WatchOs)
			.stream(WearableStream::HeartRate)
			.policy(SamplingPolicy::Balanced { interval_secs: 0 });
		assert_eq!(zero.validate(), Err(WatchError::InvalidInterval));
	}

	#[test]
	fn required_permissions_match_each_watch_os() {
		let wear = WearableApp::new("w", WatchKind::WearOs)
			.streams(&[WearableStream::HeartRate, WearableStream::Steps])
			.policy(SamplingPolicy::Continuous);
		let wear_perms = wear.required_permissions();
		assert!(wear_perms.contains(&"android.permission.BODY_SENSORS"));
		assert!(wear_perms.contains(&"android.permission.FOREGROUND_SERVICE_HEALTH"));

		let apple = WearableApp::new("a", WatchKind::WatchOs)
			.streams(&[WearableStream::HeartRate, WearableStream::BleSensors]);
		let apple_perms = apple.required_permissions();
		assert!(apple_perms.contains(&"NSHealthShareUsageDescription"));
		assert!(apple_perms.contains(&"NSBluetoothAlwaysUsageDescription"));

		let huawei = WearableApp::new("h", WatchKind::HarmonyOs)
			.streams(&[WearableStream::HeartRate, WearableStream::Steps]);
		let huawei_perms = huawei.required_permissions();
		assert!(huawei_perms.contains(&"ohos.permission.READ_HEALTH_DATA"));
		assert!(huawei_perms.contains(&"ohos.permission.ACTIVITY_MOTION"));
		assert!(huawei_perms.contains(&"ohos.permission.KEEP_BACKGROUND_RUNNING"));
	}
}
