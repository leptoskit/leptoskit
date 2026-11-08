//! <!-- {=p_backgroundSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_background` as a direct dependency and select the
//! `background` plugin in `native.plugins`; supported generated hosts discover
//! it from Cargo metadata. Android runs a foreground service with a persistent
//! notification and wakes the app's `AppService` on a schedule, with a
//! `WorkManager` fallback and an opt-in boot receiver; iOS
//! registers `BGTaskScheduler` tasks that wake the `AppService`. iOS gives no
//! guarantee of continuous execution, and macOS, desktop, and the browser
//! report `Unavailable`.
//!
//! <!-- {/p_backgroundSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Stable native registration name.
pub const NAME: &str = "background";
/// Starts the foreground service / schedules background tasks. Payload is an
/// encoded [`Options`].
pub const START: MethodId = MethodId(0);
/// Stops the foreground service and cancels scheduled background tasks.
pub const STOP: MethodId = MethodId(1);
/// Reports the current running state, last wake, and battery optimisation.
pub const STATUS: MethodId = MethodId(2);
/// Asks the user to exempt the app from battery optimisation. Returns the
/// resulting [`BatteryOptimization`] state.
pub const REQUEST_IGNORE_BATTERY_OPTIMIZATIONS: MethodId = MethodId(3);
/// Exported independently grantable operations.
pub const METHODS: &[MethodId] = &[START, STOP, STATUS, REQUEST_IGNORE_BATTERY_OPTIMIZATIONS];

/// Android foreground service types an app may hold while in the background.
///
/// Each bit maps to an Android `FOREGROUND_SERVICE_*` permission and the
/// matching `foregroundServiceType`. Android 14 (API 34) restricts which types
/// may be started from the background: `dataSync` and `health` are generally
/// disallowed unless the app meets a specific exemption, while
/// `connectedDevice` and `location` carry additional runtime requirements.
/// Declaring a type does not grant the underlying permission, and the system
/// may still kill a service it judges abusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ForegroundServiceType {
	/// `dataSync`: transferring data over the network.
	DataSync = 1 << 0,
	/// `health`: exercise, fitness, and health tracking.
	Health = 1 << 1,
	/// `microphone`: recording audio from a microphone.
	Microphone = 1 << 2,
	/// `location`: ongoing location access.
	Location = 1 << 3,
	/// `connectedDevice`: interacting with external devices.
	ConnectedDevice = 1 << 4,
}

/// A validated bitmask of [`ForegroundServiceType`] values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ForegroundServiceTypes(u32);
impl ForegroundServiceTypes {
	const ALL: u32 = (ForegroundServiceType::DataSync as u32)
		| (ForegroundServiceType::Health as u32)
		| (ForegroundServiceType::Microphone as u32)
		| (ForegroundServiceType::Location as u32)
		| (ForegroundServiceType::ConnectedDevice as u32);
	/// `connectedDevice` only.
	pub const CONNECTED_DEVICE: Self = Self(ForegroundServiceType::ConnectedDevice as u32);
	/// `dataSync` only.
	pub const DATA_SYNC: Self = Self(ForegroundServiceType::DataSync as u32);
	/// `health` only.
	pub const HEALTH: Self = Self(ForegroundServiceType::Health as u32);
	/// `location` only.
	pub const LOCATION: Self = Self(ForegroundServiceType::Location as u32);
	/// `microphone` only.
	pub const MICROPHONE: Self = Self(ForegroundServiceType::Microphone as u32);
	/// No foreground service types.
	pub const NONE: Self = Self(0);

	/// Builds a mask from raw bits, rejecting unknown bits.
	pub fn from_bits(bits: u32) -> Result<Self, Error> {
		if bits & !Self::ALL == 0 {
			Ok(Self(bits))
		} else {
			Err(Error::InvalidArgument)
		}
	}

	/// The raw bitmask, for the native handler.
	#[must_use]
	pub const fn bits(self) -> u32 {
		self.0
	}

	/// Whether the mask is empty.
	#[must_use]
	pub const fn is_empty(self) -> bool {
		self.0 == 0
	}

	/// Whether the mask contains `kind`.
	#[must_use]
	pub const fn contains(self, kind: ForegroundServiceType) -> bool {
		self.0 & kind as u32 != 0
	}
}

/// Whether the app is exempt from Android battery optimisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum BatteryOptimization {
	/// The platform cannot report the state, or the app is not on Android.
	Unknown = 0,
	/// The app is exempt from battery optimisation.
	Exempted = 1,
	/// The app is not exempt; background work may be deferred or killed.
	NotExempted = 2,
}
impl BatteryOptimization {
	/// Validates the exact four-byte response.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match u32::from_le_bytes(bytes.try_into().map_err(|_| Error::InvalidArgument)?) {
			0 => Ok(Self::Unknown),
			1 => Ok(Self::Exempted),
			2 => Ok(Self::NotExempted),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Maximum lengths the wire format carries, shared with the native handlers.
pub const MAX_TITLE: usize = 128;
pub const MAX_TEXT: usize = 1024;
/// Smallest and largest wake interval the plugin accepts, in seconds.
pub const MIN_WAKE_INTERVAL: u32 = 1;
pub const MAX_WAKE_INTERVAL: u32 = 86_400;
/// The wake interval used when the app does not name one.
pub const DEFAULT_WAKE_INTERVAL: u32 = 60;

/// Configuration for starting background execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
	notification_title: String,
	notification_text: String,
	types: ForegroundServiceTypes,
	wake_interval_secs: u32,
}
impl Options {
	/// Constructs validated configuration with bounded, control-free text.
	pub fn new(
		notification_title: impl Into<String>,
		notification_text: impl Into<String>,
		types: ForegroundServiceTypes,
		wake_interval_secs: u32,
	) -> Result<Self, Error> {
		let notification_title = notification_title.into();
		let notification_text = notification_text.into();
		validate_text(&notification_title, MAX_TITLE, true)?;
		validate_text(&notification_text, MAX_TEXT, false)?;

		if !(MIN_WAKE_INTERVAL..=MAX_WAKE_INTERVAL).contains(&wake_interval_secs) {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			notification_title,
			notification_text,
			types,
			wake_interval_secs,
		})
	}

	/// The notification title shown while the service runs.
	#[must_use]
	pub fn notification_title(&self) -> &str {
		&self.notification_title
	}

	/// The notification text shown while the service runs.
	#[must_use]
	pub fn notification_text(&self) -> &str {
		&self.notification_text
	}

	/// The requested foreground service types.
	#[must_use]
	pub const fn types(&self) -> ForegroundServiceTypes {
		self.types
	}

	/// The wake interval in seconds.
	#[must_use]
	pub const fn wake_interval_secs(&self) -> u32 {
		self.wake_interval_secs
	}

	/// Encodes the start request.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(
			12 + self.notification_title.len() + self.notification_text.len(),
		);
		output.bytes(self.notification_title.as_bytes())?;
		output.bytes(self.notification_text.as_bytes())?;
		output.u32(self.types.bits());
		output.u32(self.wake_interval_secs);
		Ok(output.finish())
	}

	/// Validates the complete start request, including trailing bytes.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_TITLE + MAX_TEXT + 16 {
			return Err(Error::InvalidArgument);
		}

		let mut input = Decoder::new(payload);
		let title = input.string()?;
		let text = input.string()?;
		let types = ForegroundServiceTypes::from_bits(input.u32()?)?;
		let interval = input.u32()?;
		input.finish()?;
		Self::new(title, text, types, interval)
	}
}

/// Current background execution state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
	running: bool,
	last_wake_secs: u32,
	battery_optimization: BatteryOptimization,
}
impl Status {
	/// Decodes the exact status reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() != 12 {
			return Err(Error::InvalidArgument);
		}

		let mut input = Decoder::new(payload);
		let running = match input.u32()? {
			0 => false,
			1 => true,
			_ => return Err(Error::InvalidArgument),
		};
		let last_wake_secs = input.u32()?;
		let battery_optimization = BatteryOptimization::decode(&input.u32()?.to_le_bytes())?;
		input.finish()?;
		Ok(Self {
			running,
			last_wake_secs,
			battery_optimization,
		})
	}

	/// Whether the foreground service / background tasks are running.
	#[must_use]
	pub const fn running(self) -> bool {
		self.running
	}

	/// Unix seconds of the last background wake, or `None` if never woken.
	#[must_use]
	pub const fn last_wake_secs(self) -> Option<u32> {
		if self.last_wake_secs == 0 {
			None
		} else {
			Some(self.last_wake_secs)
		}
	}

	/// The battery optimisation exemption state.
	#[must_use]
	pub const fn battery_optimization(self) -> BatteryOptimization {
		self.battery_optimization
	}
}

fn validate_text(value: &str, maximum: usize, required: bool) -> Result<(), Error> {
	if value.len() > maximum
		|| (required && value.trim().is_empty())
		|| value.chars().any(char::is_control)
	{
		Err(Error::InvalidArgument)
	} else {
		Ok(())
	}
}

/// Validates a method call without activating any system resource.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		START => {
			Options::decode(payload)?;
			Ok(())
		}
		STOP | STATUS | REQUEST_IGNORE_BATTERY_OPTIMIZATIONS => {
			if payload.is_empty() {
				Ok(())
			} else {
				Err(Error::InvalidArgument)
			}
		}
		_ => Err(Error::Unavailable),
	}
}

#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest;
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn options_roundtrip_unicode_and_reject_ambiguous_input() {
		let types = ForegroundServiceTypes::from_bits(
			(ForegroundServiceType::DataSync as u32) | (ForegroundServiceType::Health as u32),
		)
		.unwrap();
		let options = Options::new("Quant is running ✓", "Draining the spool", types, 60).unwrap();
		let bytes = options.encode().unwrap();
		assert_eq!(Options::decode(&bytes).unwrap(), options);

		for length in 0..bytes.len() {
			assert!(Options::decode(&bytes[..length]).is_err());
		}

		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert!(Options::decode(&trailing).is_err());

		assert!(Options::new("\t", "", types, 60).is_err());
		assert!(Options::new("Title", "bad\0text", types, 60).is_err());
		assert!(Options::new("x".repeat(MAX_TITLE + 1), "", types, 60).is_err());
		assert!(Options::new("Title", "text", types, 0).is_err());
		assert!(Options::new("Title", "text", types, MAX_WAKE_INTERVAL + 1).is_err());
	}

	#[test]
	fn service_types_reject_unknown_bits() {
		assert_eq!(
			ForegroundServiceTypes::from_bits(1 << 5),
			Err(Error::InvalidArgument)
		);
		let all = ForegroundServiceTypes::from_bits(
			(ForegroundServiceType::DataSync as u32)
				| (ForegroundServiceType::Health as u32)
				| (ForegroundServiceType::Microphone as u32)
				| (ForegroundServiceType::Location as u32)
				| (ForegroundServiceType::ConnectedDevice as u32),
		)
		.unwrap();
		assert!(!all.is_empty());
		assert!(all.contains(ForegroundServiceType::Location));
		assert!(all.contains(ForegroundServiceType::DataSync));
		assert!(ForegroundServiceTypes::NONE.is_empty());
	}

	#[test]
	fn status_is_strict_and_preserves_content() {
		let mut status = Encoder::default();
		status.u32(1);
		status.u32(1_700_000_000);
		status.u32(1);
		let status = Status::decode(&status.finish()).unwrap();
		assert!(status.running());
		assert_eq!(status.last_wake_secs(), Some(1_700_000_000));
		assert_eq!(status.battery_optimization(), BatteryOptimization::Exempted);

		for value in [&[][..], &[0u8; 11], &[0u8; 13]] {
			assert!(Status::decode(value).is_err());
		}

		let mut bad_running = Encoder::default();
		bad_running.u32(2);
		bad_running.u32(0);
		bad_running.u32(0);
		assert!(Status::decode(&bad_running.finish()).is_err());
	}

	#[test]
	fn battery_optimization_decodes_exactly() {
		assert_eq!(
			BatteryOptimization::decode(&[0, 0, 0, 0]),
			Ok(BatteryOptimization::Unknown)
		);
		assert_eq!(
			BatteryOptimization::decode(&[1, 0, 0, 0]),
			Ok(BatteryOptimization::Exempted)
		);
		assert_eq!(
			BatteryOptimization::decode(&[2, 0, 0, 0]),
			Ok(BatteryOptimization::NotExempted)
		);
		assert!(BatteryOptimization::decode(&[3, 0, 0, 0]).is_err());
		assert!(BatteryOptimization::decode(&[]).is_err());
	}

	#[test]
	fn validate_accepts_exact_method_payloads() {
		let options = Options::new("Title", "Text", ForegroundServiceTypes::NONE, 60).unwrap();
		assert_eq!(validate(START, &options.encode().unwrap()), Ok(()));
		assert_eq!(validate(STOP, &[]), Ok(()));
		assert_eq!(validate(STATUS, &[]), Ok(()));
		assert_eq!(validate(REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, &[]), Ok(()));
		assert_eq!(validate(STOP, &[0]), Err(Error::InvalidArgument));
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));
	}
}
