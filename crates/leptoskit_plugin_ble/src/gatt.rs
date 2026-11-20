//! Standard Bluetooth SIG GATT wearable characteristic decoders.
//!
//! Pure Rust, `#![forbid(unsafe_code)]`, usable from both Wasm guests and
//! native services.

use leptoskit_bridge::Error;

/// Heart Rate service (`0x180D`).
pub const SERVICE_HEART_RATE: &str = "0000180d-0000-1000-8000-00805f9b34fb";
/// Heart Rate Measurement characteristic (`0x2A37`).
pub const CHAR_HEART_RATE_MEASUREMENT: &str = "00002a37-0000-1000-8000-00805f9b34fb";
/// Pulse Oximeter service (`0x1822`).
pub const SERVICE_PULSE_OXIMETER: &str = "00001822-0000-1000-8000-00805f9b34fb";
/// PLX Spot-Check Measurement characteristic (`0x2A5E`).
pub const CHAR_PLX_SPOT_CHECK: &str = "00002a5e-0000-1000-8000-00805f9b34fb";
/// PLX Continuous Measurement characteristic (`0x2A5F`).
pub const CHAR_PLX_CONTINUOUS: &str = "00002a5f-0000-1000-8000-00805f9b34fb";
/// Health Thermometer service (`0x1809`).
pub const SERVICE_HEALTH_THERMOMETER: &str = "00001809-0000-1000-8000-00805f9b34fb";
/// Temperature Measurement characteristic (`0x2A1C`).
pub const CHAR_TEMPERATURE_MEASUREMENT: &str = "00002a1c-0000-1000-8000-00805f9b34fb";
/// Battery service (`0x180F`).
pub const SERVICE_BATTERY: &str = "0000180f-0000-1000-8000-00805f9b34fb";
/// Battery Level characteristic (`0x2A19`).
pub const CHAR_BATTERY_LEVEL: &str = "00002a19-0000-1000-8000-00805f9b34fb";
/// Running Speed & Cadence service (`0x1814`).
pub const SERVICE_RUNNING_SPEED_CADENCE: &str = "00001814-0000-1000-8000-00805f9b34fb";
/// RSC Measurement characteristic (`0x2A53`).
pub const CHAR_RSC_MEASUREMENT: &str = "00002a53-0000-1000-8000-00805f9b34fb";
/// Cycling Power service (`0x1818`).
pub const SERVICE_CYCLING_POWER: &str = "00001818-0000-1000-8000-00805f9b34fb";
/// Cycling Power Measurement characteristic (`0x2A63`).
pub const CHAR_CYCLING_POWER_MEASUREMENT: &str = "00002a63-0000-1000-8000-00805f9b34fb";

/// Decoded Heart Rate Measurement (`0x2A37`).
#[derive(Debug, Clone, PartialEq)]
pub struct HeartRateMeasurement {
	/// Beats per minute.
	pub bpm: u16,
	/// Skin contact detected, when the sensor supports contact detection.
	pub sensor_contact: Option<bool>,
	/// Energy expended in kilojoules, when present.
	pub energy_expended_kj: Option<u16>,
	/// Inter-beat (RR) intervals in milliseconds (from 1/1024 s ticks).
	pub rr_intervals_ms: Vec<f32>,
}

impl HeartRateMeasurement {
	/// Decodes a `0x2A37` GATT payload.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() < 2 {
			return Err(Error::InvalidArgument);
		}
		let flags = bytes[0];
		let mut pos = 1;
		let bpm = if flags & 0x01 != 0 {
			if bytes.len() < pos + 2 {
				return Err(Error::InvalidArgument);
			}
			let v = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]);
			pos += 2;
			v
		} else {
			let v = u16::from(bytes[pos]);
			pos += 1;
			v
		};

		let contact_bits = (flags >> 1) & 0x03;
		let sensor_contact = match contact_bits {
			2 => Some(false),
			3 => Some(true),
			_ => None,
		};

		let energy_expended_kj = if flags & 0x08 != 0 {
			if bytes.len() < pos + 2 {
				return Err(Error::InvalidArgument);
			}
			let v = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]);
			pos += 2;
			Some(v)
		} else {
			None
		};

		let mut rr_intervals_ms = Vec::new();
		if flags & 0x10 != 0 {
			let rem = &bytes[pos..];
			let (pairs, rem_tail) = rem.as_chunks::<2>();
			if !rem_tail.is_empty() {
				return Err(Error::InvalidArgument);
			}
			for pair in pairs {
				let ticks = u16::from_le_bytes(*pair);
				rr_intervals_ms.push(f32::from(ticks) * (1000.0 / 1024.0));
			}
		}

		Ok(Self {
			bpm,
			sensor_contact,
			energy_expended_kj,
			rr_intervals_ms,
		})
	}

	/// Computes RMSSD in milliseconds when at least two RR intervals are
	/// present.
	#[must_use]
	pub fn rmssd_ms(&self) -> Option<f64> {
		if self.rr_intervals_ms.len() < 2 {
			return None;
		}
		let mut sum_sq = 0.0_f64;
		for pair in self.rr_intervals_ms.windows(2) {
			let diff = f64::from(pair[1] - pair[0]);
			sum_sq += diff * diff;
		}
		let mean = sum_sq / (self.rr_intervals_ms.len() - 1) as f64;
		Some(mean.sqrt())
	}
}

/// Decoded Pulse Oximeter Measurement (`0x2A5E` / `0x2A5F`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PulseOximeterMeasurement {
	/// Blood oxygen saturation (`SpO2` %).
	pub spo2_percent: f64,
	/// Pulse rate in beats per minute.
	pub pulse_bpm: f64,
}

impl PulseOximeterMeasurement {
	/// Decodes a `0x2A5E` or `0x2A5F` payload.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() < 5 {
			return Err(Error::InvalidArgument);
		}
		let spo2_percent = decode_sfloat(u16::from_le_bytes([bytes[1], bytes[2]]))?;
		let pulse_bpm = decode_sfloat(u16::from_le_bytes([bytes[3], bytes[4]]))?;
		Ok(Self {
			spo2_percent,
			pulse_bpm,
		})
	}
}

/// Decoded Health Thermometer Measurement (`0x2A1C`), normalized to Celsius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemperatureMeasurement {
	/// Temperature in degrees Celsius.
	pub celsius: f64,
}

impl TemperatureMeasurement {
	/// Decodes a `0x2A1C` payload and converts Fahrenheit to Celsius when
	/// flagged.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() < 5 {
			return Err(Error::InvalidArgument);
		}
		let fahrenheit = bytes[0] & 0x01 != 0;
		let raw = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
		let value = decode_ieee_float(raw)?;
		let celsius = if fahrenheit {
			(value - 32.0) * (5.0 / 9.0)
		} else {
			value
		};
		Ok(Self { celsius })
	}
}

/// Decoded Battery Level (`0x2A19`).
pub fn decode_battery_level(bytes: &[u8]) -> Result<u8, Error> {
	let [level] = *bytes else {
		return Err(Error::InvalidArgument);
	};
	(level <= 100)
		.then_some(level)
		.ok_or(Error::InvalidArgument)
}

/// Decoded Running Speed & Cadence (`0x2A53`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunningSpeedCadence {
	/// Instantaneous speed in meters per second.
	pub speed_mps: f64,
	/// Instantaneous cadence in strides per minute.
	pub cadence_spm: u8,
	/// Whether the sensor indicates running (vs walking).
	pub running: bool,
}

impl RunningSpeedCadence {
	/// Decodes a `0x2A53` payload.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() < 4 {
			return Err(Error::InvalidArgument);
		}
		let flags = bytes[0];
		let speed_raw = u16::from_le_bytes([bytes[1], bytes[2]]);
		let cadence_spm = bytes[3];
		Ok(Self {
			speed_mps: f64::from(speed_raw) / 256.0,
			cadence_spm,
			running: flags & 0x04 != 0,
		})
	}
}

/// Decoded Cycling Power (`0x2A63`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CyclingPower {
	/// Instantaneous power in watts.
	pub watts: i16,
}

impl CyclingPower {
	/// Decodes a `0x2A63` payload.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.len() < 4 {
			return Err(Error::InvalidArgument);
		}
		let watts = i16::from_le_bytes([bytes[2], bytes[3]]);
		Ok(Self { watts })
	}
}

/// Decodes an IEEE-11073 16-bit SFLOAT (4-bit signed exponent, 12-bit signed
/// mantissa).
fn decode_sfloat(raw: u16) -> Result<f64, Error> {
	if matches!(raw, 0x07FE..=0x0802) {
		return Err(Error::InvalidArgument);
	}
	let mut mantissa = i32::from(raw & 0x0FFF);
	if mantissa >= 0x0800 {
		mantissa -= 0x1000;
	}
	let mut exponent = i32::from((raw >> 12) & 0x0F);
	if exponent >= 0x08 {
		exponent -= 0x10;
	}
	let value = f64::from(mantissa) * 10.0_f64.powi(exponent);
	value
		.is_finite()
		.then_some(value)
		.ok_or(Error::InvalidArgument)
}

/// Decodes an IEEE-11073 32-bit FLOAT (8-bit signed exponent, 24-bit signed
/// mantissa).
fn decode_ieee_float(raw: u32) -> Result<f64, Error> {
	let exponent = ((raw >> 24) as u8).cast_signed();
	let mut mantissa = (raw & 0x00FF_FFFF) as i32;
	if mantissa >= 0x0080_0000 {
		mantissa -= 0x0100_0000;
	}
	if matches!(mantissa, 0x007F_FFFF | 0x007F_FFFE) || mantissa == -0x0080_0000 {
		return Err(Error::InvalidArgument);
	}
	let value = f64::from(mantissa) * 10.0_f64.powi(i32::from(exponent));
	value
		.is_finite()
		.then_some(value)
		.ok_or(Error::InvalidArgument)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn decodes_heart_rate_with_rr_intervals_and_rmssd() {
		// 8-bit HR (72 bpm), sensor contact supported+detected (0b11 << 1), RR
		// present (0x10). RR ticks: 819 (~800ms), 860 (~840ms)
		let rr1 = 819_u16.to_le_bytes();
		let rr2 = 860_u16.to_le_bytes();
		let payload = [0x16, 72, rr1[0], rr1[1], rr2[0], rr2[1]];
		let hr = HeartRateMeasurement::decode(&payload).unwrap();
		assert_eq!(hr.bpm, 72);
		assert_eq!(hr.sensor_contact, Some(true));
		assert_eq!(hr.rr_intervals_ms.len(), 2);
		let rmssd = hr.rmssd_ms().unwrap();
		assert!((rmssd - 40.04).abs() < 0.2, "rmssd was {rmssd}");
	}

	#[test]
	fn decodes_spo2_temperature_battery_rsc_and_power() {
		// SpO2 = 98 (0x0062), pulse = 64 (0x0040)
		let plx = PulseOximeterMeasurement::decode(&[0x00, 0x62, 0x00, 0x40, 0x00]).unwrap();
		assert!((plx.spo2_percent - 98.0).abs() < 0.01);
		assert!((plx.pulse_bpm - 64.0).abs() < 0.01);

		// 36.5 C = 365 * 10^-1 (exponent = -1 = 0xFF, mantissa = 365 =
		// 0x00016D)
		let temp = TemperatureMeasurement::decode(&[0x00, 0x6D, 0x01, 0x00, 0xFF]).unwrap();
		assert!((temp.celsius - 36.5).abs() < 0.01);

		assert_eq!(decode_battery_level(&[87]), Ok(87));
		assert_eq!(decode_battery_level(&[101]), Err(Error::InvalidArgument));

		// RSC: running flag 0x04, speed 3.0 m/s = 768 (0x0300), cadence 172 spm
		let rsc = RunningSpeedCadence::decode(&[0x04, 0x00, 0x03, 172]).unwrap();
		assert!((rsc.speed_mps - 3.0).abs() < 0.01);
		assert_eq!(rsc.cadence_spm, 172);
		assert!(rsc.running);

		let pwr = CyclingPower::decode(&[0x00, 0x00, 0xFA, 0x00]).unwrap();
		assert_eq!(pwr.watts, 250);
	}
}
