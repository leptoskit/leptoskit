//! <!-- {=p_device_motionSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_device_motion` as a direct dependency and select the
//! `device_motion` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Generated Linux hosts register it where it
//! has a Linux Rust implementation; generated Windows and terminal hosts
//! currently omit runtime plugins, so use explicit registration in a custom
//! host there. Guests enable the `guest` feature and reuse
//! `guest::DeviceMotion` rather than resolving the plugin on every call. iOS
//! uses `CoreMotion` device motion, Android fuses accelerometer, gravity,
//! gyroscope and rotation-vector sensors, desktop has no sensor, and the
//! browser uses `DeviceMotionEvent` with `DeviceOrientationEvent` for attitude.
//!
//! <!-- {/p_device_motionSummary} -->

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
pub use leptoskit_sensor::AVAILABLE;
pub use leptoskit_sensor::Interval;
pub use leptoskit_sensor::NEXT;

/// Native plugin registration name.
pub const NAME: &str = "device_motion";
/// Independently grantable methods.
pub const METHODS: &[MethodId] = &[AVAILABLE, NEXT];

/// Bytes in one little-endian f64 on the wire.
const DOUBLE_BYTES: usize = 8;
/// Fields in one fused reading.
const FIELD_COUNT: usize = 13;

/// One fused sample on Android device axes. Screen rotation does not remap the
/// vectors. A phone lying face up reads about +9.8 m/s² on gravity's z.
///
/// Wire layout is thirteen little-endian `f64` values:
/// `timestamp`, user acceleration `x/y/z` (m/s²), gravity `x/y/z` (m/s²),
/// rotation rate `x/y/z` (rad/s), and attitude `alpha/beta/gamma` (radians
/// around Z/X/Y). Apple `CoreMotion` samples are scaled to SI and negated onto
/// Android axes the way the other sensor plugins do. Browser
/// `DeviceMotionEvent` rotation rates in degrees per second are converted to
/// radians (beta→x, gamma→y, alpha→z); attitude degrees become radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
	/// Monotonic platform timestamp in seconds; not comparable across devices
	/// or APIs.
	pub timestamp: f64,
	/// User acceleration without gravity, m/s², native x.
	pub accel_x: f64,
	/// User acceleration without gravity, m/s², native y.
	pub accel_y: f64,
	/// User acceleration without gravity, m/s², native z.
	pub accel_z: f64,
	/// Gravity vector, m/s², native x.
	pub gravity_x: f64,
	/// Gravity vector, m/s², native y.
	pub gravity_y: f64,
	/// Gravity vector, m/s², native z.
	pub gravity_z: f64,
	/// Angular velocity around native x, rad/s.
	pub rotation_x: f64,
	/// Angular velocity around native y, rad/s.
	pub rotation_y: f64,
	/// Angular velocity around native z, rad/s.
	pub rotation_z: f64,
	/// Attitude around Z (yaw / alpha), radians.
	pub attitude_alpha: f64,
	/// Attitude around X (pitch / beta), radians.
	pub attitude_beta: f64,
	/// Attitude around Y (roll / gamma), radians.
	pub attitude_gamma: f64,
}

impl Reading {
	/// Bytes in one encoded reading.
	pub const BYTES: usize = FIELD_COUNT * DOUBLE_BYTES;

	/// Decodes thirteen finite little-endian f64 values and rejects a negative
	/// timestamp.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let [
			timestamp,
			accel_x,
			accel_y,
			accel_z,
			gravity_x,
			gravity_y,
			gravity_z,
			rotation_x,
			rotation_y,
			rotation_z,
			attitude_alpha,
			attitude_beta,
			attitude_gamma,
		] = decode_values(payload)?;

		if timestamp < 0.0 {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			timestamp,
			accel_x,
			accel_y,
			accel_z,
			gravity_x,
			gravity_y,
			gravity_z,
			rotation_x,
			rotation_y,
			rotation_z,
			attitude_alpha,
			attitude_beta,
			attitude_gamma,
		})
	}

	/// Validates and encodes a platform sample without allocation.
	pub fn encode(self) -> Result<[u8; Self::BYTES], Error> {
		let values = [
			self.timestamp,
			self.accel_x,
			self.accel_y,
			self.accel_z,
			self.gravity_x,
			self.gravity_y,
			self.gravity_z,
			self.rotation_x,
			self.rotation_y,
			self.rotation_z,
			self.attitude_alpha,
			self.attitude_beta,
			self.attitude_gamma,
		];

		if self.timestamp < 0.0 || values.iter().any(|value| !value.is_finite()) {
			return Err(Error::InvalidArgument);
		}

		let mut bytes = [0; Self::BYTES];
		for (slot, value) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(values) {
			slot.copy_from_slice(&value.to_le_bytes());
		}
		Ok(bytes)
	}
}

fn decode_values(payload: &[u8]) -> Result<[f64; FIELD_COUNT], Error> {
	if payload.len() != Reading::BYTES {
		return Err(Error::InvalidArgument);
	}

	let mut values = [0.0; FIELD_COUNT];
	let (chunks, _) = payload.as_chunks::<8>();
	for (value, bytes) in values.iter_mut().zip(chunks) {
		*value = f64::from_le_bytes(*bytes);
		if !value.is_finite() {
			return Err(Error::InvalidArgument);
		}
	}
	Ok(values)
}

/// Validates a method without acquiring hardware.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		AVAILABLE if payload.is_empty() => Ok(()),
		AVAILABLE => Err(Error::InvalidArgument),
		NEXT => {
			Interval::decode(payload)?;
			Ok(())
		}
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without device-motion hardware.
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
		validate(method, &payload)?;
		if method == AVAILABLE {
			Ok(Bytes::copy_from_slice(&[0, 0, 0, 0]))
		} else {
			Err(Error::Unavailable)
		}
	}
}

/// Cached guest client. Only one outstanding reading is accepted per plugin.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolved device-motion plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct DeviceMotion(leptoskit_bridge::guest::Client);

	impl DeviceMotion {
		/// Resolves the plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Tests hardware availability, independently of permission status.
		pub async fn available(self) -> Result<bool, Error> {
			leptoskit_sensor::decode_available(&self.0.call(AVAILABLE, &[])?.await?)
		}

		/// Waits for one fused sample. Dropping this call cancels hardware
		/// acquisition. The sampling interval is a hint; this is not a
		/// lossless recording stream.
		pub async fn next(self, interval: Interval) -> Result<Reading, Error> {
			Reading::decode(&self.0.call(NEXT, &interval.encode())?.await?)
		}
	}
}

/// Desktop support is explicitly unavailable where there is no portable sensor
/// API.
#[cfg(not(target_arch = "wasm32"))]
pub mod desktop {
	use super::*;

	/// Reports the absence of a portable desktop sensor backend.
	pub fn available() -> bool {
		false
	}

	/// Returns Unavailable instead of inventing a sensor reading.
	pub fn next(_interval: Interval) -> std::future::Ready<Result<Reading, Error>> {
		std::future::ready(Err(Error::Unavailable))
	}
}

/// Browser adapter using `DeviceMotionEvent` for acceleration and rotation
/// rate, plus `DeviceOrientationEvent` for attitude.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	fn sample() -> Reading {
		Reading {
			timestamp: 1.5,
			accel_x: 0.1,
			accel_y: -0.2,
			accel_z: 0.3,
			gravity_x: 0.0,
			gravity_y: 0.0,
			gravity_z: 9.8,
			rotation_x: 0.01,
			rotation_y: -0.02,
			rotation_z: 0.03,
			attitude_alpha: 1.0,
			attitude_beta: 0.5,
			attitude_gamma: -0.25,
		}
	}

	#[test]
	fn reading_round_trips_and_rejects_invalid_payloads() {
		let reading = sample();
		let bytes = reading.encode().unwrap();
		assert_eq!(bytes.len(), Reading::BYTES);
		assert_eq!(Reading::decode(&bytes).unwrap(), reading);

		assert_eq!(
			Reading::decode(&bytes[..Reading::BYTES - 1]),
			Err(Error::InvalidArgument)
		);
		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert_eq!(Reading::decode(&trailing), Err(Error::InvalidArgument));

		let mut negative = reading;
		negative.timestamp = -1.0;
		assert_eq!(negative.encode(), Err(Error::InvalidArgument));
		assert_eq!(
			Reading::decode(&{
				let mut bad = bytes;
				bad[..8].copy_from_slice(&(-1.0_f64).to_le_bytes());
				bad
			}),
			Err(Error::InvalidArgument)
		);

		let mut nan = reading;
		nan.accel_x = f64::NAN;
		assert_eq!(nan.encode(), Err(Error::InvalidArgument));
		assert_eq!(
			Reading::decode(&{
				let mut bad = bytes;
				bad[8..16].copy_from_slice(&f64::NAN.to_le_bytes());
				bad
			}),
			Err(Error::InvalidArgument)
		);
	}

	#[test]
	fn validate_and_unsupported_never_invent_readings() {
		assert_eq!(validate(AVAILABLE, &[]), Ok(()));
		assert_eq!(validate(AVAILABLE, &[0]), Err(Error::InvalidArgument));
		assert_eq!(
			validate(NEXT, &Interval::new(100).unwrap().encode()),
			Ok(())
		);
		assert_eq!(validate(NEXT, &[0, 0]), Err(Error::InvalidArgument));
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));

		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			plugin.invoke(AVAILABLE, Bytes::new()).unwrap().as_ref(),
			&[0, 0, 0, 0]
		);
		assert_eq!(
			plugin.invoke(
				NEXT,
				Bytes::copy_from_slice(&Interval::new(50).unwrap().encode())
			),
			Err(Error::Unavailable)
		);
		assert_eq!(NAME, "device_motion");
	}

	#[test]
	fn desktop_reports_unavailable_instead_of_inventing_readings() {
		assert!(!desktop::available());
		assert_eq!(
			desktop::next(Interval::new(100).unwrap()).into_inner(),
			Err(Error::Unavailable)
		);
	}
}
