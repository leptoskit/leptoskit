//! <!-- {=p_thermalSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_thermal` as a direct dependency and select the
//! `thermal` plugin in `native.plugins`; generated iOS, Android, and macOS
//! hosts discover it from Cargo metadata. Guests enable the `guest` feature and
//! reuse `guest::Thermal` rather than resolving the plugin on every call. A
//! platform that cannot report its thermal state answers `Unknown` instead of
//! pretending the device is cool, and Android's finer throttling status is kept
//! alongside the shared four-level state.
//!
//! <!-- {/p_thermalSummary} -->
//!
//! Snapshot messages are exactly two little-endian u32 fields (8 bytes): the
//! shared state (unknown/nominal/fair/serious/critical = 0/1/2/3/4) and
//! Android's `PowerManager` thermal status (`0..=6`, `u32::MAX` where the
//! platform has none).

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime plugin name.
pub const NAME: &str = "thermal";
/// Two little-endian u32 fields.
pub const SNAPSHOT_BYTES: usize = 8;
/// Empty input -> an 8-byte [`Snapshot`].
pub const SNAPSHOT: MethodId = MethodId(0);
/// Previous snapshot -> the next different snapshot. One waiter per plugin;
/// dropping the guest future cancels the native subscription.
pub const WAIT_FOR_CHANGE: MethodId = MethodId(1);

/// The wire value for a field the platform cannot report.
const UNKNOWN: u32 = u32::MAX;

/// How hot the device is, on the four levels Apple's `ProcessInfo` defines.
/// Android's seven throttling levels are folded into these; the original
/// status stays in [`Snapshot::android_status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum State {
	/// The platform cannot report a thermal state.
	Unknown = 0,
	/// Within normal limits.
	Nominal = 1,
	/// Slightly elevated. Android: light or moderate throttling.
	Fair = 2,
	/// High; the system is reducing performance. Android: severe throttling.
	Serious = 3,
	/// Critical; the app should shed work so the device can cool. Android:
	/// critical, emergency, or shutdown.
	Critical = 4,
}

impl State {
	/// Decodes a wire code, refusing codes past the last state.
	pub fn from_code(code: u32) -> Result<Self, Error> {
		match code {
			0 => Ok(Self::Unknown),
			1 => Ok(Self::Nominal),
			2 => Ok(Self::Fair),
			3 => Ok(Self::Serious),
			4 => Ok(Self::Critical),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Whether the system is already throttling the device, so background
	/// work should wait.
	pub fn is_throttled(self) -> bool {
		self >= Self::Serious
	}
}

/// Android's `PowerManager.THERMAL_STATUS_*` levels, kept as reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum AndroidStatus {
	/// `THERMAL_STATUS_NONE`: not under throttling.
	None = 0,
	/// `THERMAL_STATUS_LIGHT`: light throttling, user experience unaffected.
	Light = 1,
	/// `THERMAL_STATUS_MODERATE`: moderate throttling.
	Moderate = 2,
	/// `THERMAL_STATUS_SEVERE`: severe throttling, user experience affected.
	Severe = 3,
	/// `THERMAL_STATUS_CRITICAL`: the platform has done everything it can.
	Critical = 4,
	/// `THERMAL_STATUS_EMERGENCY`: key components are shutting down.
	Emergency = 5,
	/// `THERMAL_STATUS_SHUTDOWN`: the device is about to shut down.
	Shutdown = 6,
}

impl AndroidStatus {
	/// Decodes `PowerManager`'s status code.
	pub fn from_code(code: u32) -> Result<Self, Error> {
		match code {
			0 => Ok(Self::None),
			1 => Ok(Self::Light),
			2 => Ok(Self::Moderate),
			3 => Ok(Self::Severe),
			4 => Ok(Self::Critical),
			5 => Ok(Self::Emergency),
			6 => Ok(Self::Shutdown),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// The shared state this status folds into. The Kotlin host uses the
	/// same table.
	pub fn state(self) -> State {
		match self {
			Self::None => State::Nominal,
			Self::Light | Self::Moderate => State::Fair,
			Self::Severe => State::Serious,
			Self::Critical | Self::Emergency | Self::Shutdown => State::Critical,
		}
	}
}

/// One platform thermal reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Snapshot {
	/// The shared four-level state.
	pub state: State,
	/// Android's own status; `None` on every other platform and on Android
	/// before API 29.
	pub android_status: Option<AndroidStatus>,
}

impl Snapshot {
	/// A reading from a platform that reports nothing.
	pub const UNKNOWN: Self = Self {
		state: State::Unknown,
		android_status: None,
	};

	/// The reading an Android status produces.
	pub fn from_android(status: AndroidStatus) -> Self {
		Self {
			state: status.state(),
			android_status: Some(status),
		}
	}

	/// Encodes a fixed-size binary reply.
	pub fn encode(self) -> Bytes {
		let mut output = Encoder::with_capacity(SNAPSHOT_BYTES);
		output.u32(self.state as u32);
		output.u32(self.android_status.map_or(UNKNOWN, |status| status as u32));
		output.finish()
	}

	/// Decodes and validates both fields, rejects trailing bytes, and refuses
	/// an Android status that contradicts the shared state.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let state = State::from_code(input.u32()?)?;

		let android_status = match input.u32()? {
			UNKNOWN => None,
			code => Some(AndroidStatus::from_code(code)?),
		};

		input.finish()?;

		if android_status.is_some_and(|status| status.state() != state) {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			state,
			android_status,
		})
	}
}

/// Cached interpreter client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// One resolved host plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Thermal(leptoskit_bridge::guest::Client);

	impl Thermal {
		/// Resolves the installed thermal plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads and validates the platform's current thermal state.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}

		/// Waits until the platform reports a state other than `previous`.
		/// The host installs its observer before sampling, so a change
		/// between calls is not missed. Intermediate states may be coalesced;
		/// this is a latest-state subscription. A second concurrent waiter
		/// returns `Busy`, a platform that cannot observe changes returns
		/// `Unavailable`, and dropping the future cancels it.
		pub async fn wait_for_change(self, previous: Snapshot) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(WAIT_FOR_CHANGE, &previous.encode())?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Every Android status, in `PowerManager` order.
	const ANDROID: [AndroidStatus; 7] = [
		AndroidStatus::None,
		AndroidStatus::Light,
		AndroidStatus::Moderate,
		AndroidStatus::Severe,
		AndroidStatus::Critical,
		AndroidStatus::Emergency,
		AndroidStatus::Shutdown,
	];

	#[test]
	fn snapshots_have_a_fixed_little_endian_layout() {
		let apple = Snapshot {
			state: State::Serious,
			android_status: None,
		};
		assert_eq!(apple.encode().as_ref(), &[3, 0, 0, 0, 255, 255, 255, 255]);
		assert_eq!(Snapshot::decode(&apple.encode()), Ok(apple));

		let android = Snapshot::from_android(AndroidStatus::Moderate);
		assert_eq!(android.encode().as_ref(), &[2, 0, 0, 0, 2, 0, 0, 0]);
		assert_eq!(Snapshot::decode(&android.encode()), Ok(android));

		assert_eq!(
			Snapshot::decode(&Snapshot::UNKNOWN.encode()),
			Ok(Snapshot::UNKNOWN)
		);
	}

	#[test]
	fn every_state_survives_the_wire() {
		for state in [
			State::Unknown,
			State::Nominal,
			State::Fair,
			State::Serious,
			State::Critical,
		] {
			let reading = Snapshot {
				state,
				android_status: None,
			};
			assert_eq!(Snapshot::decode(&reading.encode()), Ok(reading));
		}

		for status in ANDROID {
			let reading = Snapshot::from_android(status);
			assert_eq!(Snapshot::decode(&reading.encode()), Ok(reading));
		}
	}

	#[test]
	fn android_statuses_fold_into_the_shared_levels() {
		let folded: Vec<State> = ANDROID.iter().map(|status| status.state()).collect();
		assert_eq!(
			folded,
			[
				State::Nominal,
				State::Fair,
				State::Fair,
				State::Serious,
				State::Critical,
				State::Critical,
				State::Critical,
			]
		);

		assert!(!State::Unknown.is_throttled());
		assert!(!State::Fair.is_throttled());
		assert!(State::Serious.is_throttled());
		assert!(State::Critical.is_throttled());
	}

	#[test]
	fn invalid_fields_and_lengths_are_rejected() {
		let valid = Snapshot::from_android(AndroidStatus::None).encode();

		for (field, value) in [(0, 5_u32), (1, 7), (1, u32::MAX - 1)] {
			let mut bytes = valid.to_vec();
			bytes[field * 4..field * 4 + 4].copy_from_slice(&value.to_le_bytes());
			assert_eq!(Snapshot::decode(&bytes), Err(Error::InvalidArgument));
		}

		// Severe throttling cannot be reported as a nominal device.
		let mut contradiction = valid.to_vec();
		contradiction[4..8].copy_from_slice(&(AndroidStatus::Severe as u32).to_le_bytes());
		assert_eq!(
			Snapshot::decode(&contradiction),
			Err(Error::InvalidArgument)
		);

		assert_eq!(
			Snapshot::decode(&valid[..SNAPSHOT_BYTES - 1]),
			Err(Error::InvalidArgument)
		);
		let mut trailing = valid.to_vec();
		trailing.push(0);
		assert_eq!(Snapshot::decode(&trailing), Err(Error::InvalidArgument));
		assert_eq!(Snapshot::decode(&[]), Err(Error::InvalidArgument));
	}

	#[test]
	fn codes_round_trip_through_their_decoders() {
		for status in ANDROID {
			assert_eq!(AndroidStatus::from_code(status as u32), Ok(status));
		}

		assert_eq!(AndroidStatus::from_code(7), Err(Error::InvalidArgument));
		assert_eq!(State::from_code(5), Err(Error::InvalidArgument));
		assert_eq!(NAME, "thermal");
	}
}
