//! Data types, permissions and availability.

use leptoskit_bridge::Error;

use crate::wire::Reader;

/// A kind of health record the plugin reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u32)]
pub enum DataType {
	/// Heart rate in beats per minute, as a series of timed measurements.
	HeartRate = 0,
	/// Resting heart rate in beats per minute.
	RestingHeartRate = 1,
	/// Heart rate variability as RMSSD in milliseconds. Health Connect only.
	HeartRateVariabilityRmssd = 2,
	/// Heart rate variability as SDNN in milliseconds. `HealthKit` only. SDNN
	/// and RMSSD are different measures and are never converted.
	HeartRateVariabilitySdnn = 3,
	/// Steps counted over an interval.
	Steps = 4,
	/// Sleep, with its stages.
	Sleep = 5,
	/// Blood oxygen saturation in percent.
	OxygenSaturation = 6,
	/// Breaths per minute.
	RespiratoryRate = 7,
	/// An exercise session: its activity and when it started and ended.
	Exercise = 8,
}

impl DataType {
	/// Every data type, in wire order.
	pub const ALL: [Self; 9] = [
		Self::HeartRate,
		Self::RestingHeartRate,
		Self::HeartRateVariabilityRmssd,
		Self::HeartRateVariabilitySdnn,
		Self::Steps,
		Self::Sleep,
		Self::OxygenSaturation,
		Self::RespiratoryRate,
		Self::Exercise,
	];

	/// The wire code.
	pub const fn code(self) -> u32 {
		self as u32
	}

	/// Decodes a wire code.
	pub fn from_code(code: u32) -> Result<Self, Error> {
		Self::ALL
			.get(code as usize)
			.copied()
			.ok_or(Error::InvalidArgument)
	}

	const fn bit(self) -> u32 {
		1 << self as u32
	}
}

/// A set of data types.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DataTypes(u32);

impl DataTypes {
	/// Every data type.
	pub const ALL: Self = Self((1 << DataType::ALL.len()) - 1);
	/// No data types.
	pub const EMPTY: Self = Self(0);

	/// The set of the given types.
	pub fn of(types: &[DataType]) -> Self {
		types.iter().copied().collect()
	}

	/// The wire mask: bit `n` is the type with code `n`.
	pub const fn bits(self) -> u32 {
		self.0
	}

	/// Decodes a wire mask, rejecting bits no data type uses.
	pub fn from_bits(bits: u32) -> Result<Self, Error> {
		(bits & !Self::ALL.0 == 0)
			.then_some(Self(bits))
			.ok_or(Error::InvalidArgument)
	}

	/// Whether the set holds `data_type`.
	pub const fn contains(self, data_type: DataType) -> bool {
		self.0 & data_type.bit() != 0
	}

	/// Adds `data_type`.
	pub fn insert(&mut self, data_type: DataType) {
		self.0 |= data_type.bit();
	}

	/// Whether the set is empty.
	pub const fn is_empty(self) -> bool {
		self.0 == 0
	}

	/// The types in the set, in wire order.
	pub fn iter(self) -> impl Iterator<Item = DataType> {
		DataType::ALL
			.into_iter()
			.filter(move |data_type| self.contains(*data_type))
	}
}

impl FromIterator<DataType> for DataTypes {
	fn from_iter<I: IntoIterator<Item = DataType>>(types: I) -> Self {
		let mut set = Self::EMPTY;

		for data_type in types {
			set.insert(data_type);
		}

		set
	}
}

/// Whether this device can provide health data at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
	/// Health Connect or `HealthKit` is ready.
	Available,
	/// Android 13 or earlier without the Health Connect app installed.
	NotInstalled,
	/// The Health Connect app must be updated first.
	NeedsUpdate,
	/// The platform has no health store: the web, desktops, macOS, older
	/// Android versions, and devices without `HealthKit`.
	Unsupported,
}

impl Availability {
	/// Decodes the availability reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		match payload {
			[0, 0, 0, 0] => Ok(Self::Available),
			[1, 0, 0, 0] => Ok(Self::NotInstalled),
			[2, 0, 0, 0] => Ok(Self::NeedsUpdate),
			[3, 0, 0, 0] => Ok(Self::Unsupported),
			_ => Err(Error::InvalidArgument),
		}
	}
}

const BACKGROUND: u32 = 1;
const HISTORY: u32 = 2;
const EXACT: u32 = 4;

/// What the platform's health store can do on this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Features {
	/// The data types this platform records.
	pub types: DataTypes,
	/// Whether Health Connect can grant reads while the app is in the
	/// background. Always false on iOS, which has no such permission.
	pub background: bool,
	/// Whether Health Connect can grant reads of data older than 30 days
	/// before the first grant. Always false on iOS, which has no such limit.
	pub history: bool,
}

impl Features {
	/// Decodes the features reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut reader = Reader::new(payload);
		let types = DataTypes::from_bits(reader.u32()?)?;
		let flags = reader.u32()?;
		reader.finish()?;
		(flags & !(BACKGROUND | HISTORY) == 0)
			.then_some(Self {
				types,
				background: flags & BACKGROUND != 0,
				history: flags & HISTORY != 0,
			})
			.ok_or(Error::InvalidArgument)
	}
}

/// Android permissions to ask for alongside the data types.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Extra {
	/// `READ_HEALTH_DATA_IN_BACKGROUND`: read while the app isn't in the
	/// foreground. The app must also declare it in its own manifest.
	pub background: bool,
	/// `READ_HEALTH_DATA_HISTORY`: read data from before the 30 days ahead of
	/// the first grant. The app must also declare it in its own manifest.
	pub history: bool,
}

impl Extra {
	/// The wire flags: bit 0 background, bit 1 history.
	pub const fn bits(self) -> u32 {
		(if self.background { BACKGROUND } else { 0 }) | if self.history { HISTORY } else { 0 }
	}
}

/// What the user allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grants {
	/// The data types the app may read. When [`Grants::exact`] is false
	/// these are the types the user has answered for, whatever the answer.
	pub read: DataTypes,
	/// Whether the platform reports read access itself. `HealthKit` never
	/// does: it hides denied reads, which then return no samples.
	pub exact: bool,
	/// Whether background reads are allowed (Android).
	pub background: bool,
	/// Whether reads of older history are allowed (Android).
	pub history: bool,
}

impl Grants {
	/// Decodes the grants reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut reader = Reader::new(payload);
		let read = DataTypes::from_bits(reader.u32()?)?;
		let flags = reader.u32()?;
		reader.finish()?;
		(flags & !(BACKGROUND | HISTORY | EXACT) == 0)
			.then_some(Self {
				read,
				exact: flags & EXACT != 0,
				background: flags & BACKGROUND != 0,
				history: flags & HISTORY != 0,
			})
			.ok_or(Error::InvalidArgument)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn data_type_codes_and_masks_round_trip() {
		for (code, data_type) in DataType::ALL.into_iter().enumerate() {
			assert_eq!(DataType::from_code(code as u32), Ok(data_type));
			assert_eq!(data_type.code(), code as u32);
		}

		assert_eq!(DataType::from_code(9), Err(Error::InvalidArgument));
		let set = DataTypes::of(&[DataType::Sleep, DataType::HeartRate, DataType::Sleep]);
		assert_eq!(set.bits(), 0b10_0001);
		assert_eq!(
			set.iter().collect::<Vec<_>>(),
			[DataType::HeartRate, DataType::Sleep]
		);
		assert_eq!(DataTypes::from_bits(0x1FF), Ok(DataTypes::ALL));
		assert_eq!(DataTypes::from_bits(0x200), Err(Error::InvalidArgument));
		assert!(DataTypes::EMPTY.is_empty());
	}

	#[test]
	fn availability_states_decode_exactly() {
		for (byte, expected) in [
			(0, Availability::Available),
			(1, Availability::NotInstalled),
			(2, Availability::NeedsUpdate),
			(3, Availability::Unsupported),
		] {
			assert_eq!(Availability::decode(&[byte, 0, 0, 0]), Ok(expected));
		}

		for invalid in [&[4, 0, 0, 0][..], &[0], &[0, 0, 0, 0, 0], &[0, 1, 0, 0]] {
			assert_eq!(Availability::decode(invalid), Err(Error::InvalidArgument));
		}
	}

	#[test]
	fn grants_and_features_reject_unknown_flags_and_types() {
		let grants = [0b1_0000, 0, 0, 0, 0b111, 0, 0, 0];
		assert_eq!(
			Grants::decode(&grants),
			Ok(Grants {
				read: DataTypes::of(&[DataType::Steps]),
				exact: true,
				background: true,
				history: true,
			})
		);
		assert_eq!(
			Grants::decode(&[0, 0, 0, 0, 8, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Grants::decode(&[0, 4, 0, 0, 0, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Features::decode(&[0xF7, 1, 0, 0, 2, 0, 0, 0]),
			Ok(Features {
				types: DataTypes(0x1F7),
				background: false,
				history: true,
			})
		);
		assert_eq!(
			Features::decode(&[0, 0, 0, 0, 4, 0, 0, 0]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(Features::decode(&[0; 7]), Err(Error::InvalidArgument));
		assert_eq!(
			Extra {
				background: true,
				history: true
			}
			.bits(),
			3
		);
	}
}
