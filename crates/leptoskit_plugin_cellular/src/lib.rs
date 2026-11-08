//! <!-- {=p_cellularSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_cellular` as a direct dependency and select the
//! `cellular` plugin in `native.plugins`; supported generated hosts discover it
//! from Cargo metadata. Generated Linux hosts register it where it has a Linux
//! Rust implementation; generated Windows and terminal hosts currently omit
//! runtime plugins, so use explicit registration in a custom host there. Guests
//! enable the `guest` feature and reuse `guest::Cellular` rather than resolving
//! the plugin on every call. Carrier name, MCC/MNC, radio generation, and
//! `VoIP` allowance stay Unknown, Unavailable, or Restricted when the platform
//! does not expose them — values are never invented.
//!
//! <!-- {/p_cellularSummary} -->
//!
//! Wire snapshot fields are little-endian: radio generation (`u32`), `VoIP`
//! flag (`u32`), MCC status + value, MNC status + value, carrier status, then a
//! length-prefixed UTF-8 carrier name (empty unless the carrier status is
//! Ready). Status codes are Unknown / Unavailable / Restricted / Ready.
//! Generation uses Unknown / Unavailable / Restricted / 2G / 3G / 4G / 5G.
//! `VoIP` uses Unknown / Unavailable / Restricted / false / true.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime plugin name.
pub const NAME: &str = "cellular";
/// Empty input -> validated [`Snapshot`] binary response.
pub const SNAPSHOT: MethodId = MethodId(0);
/// Longest carrier name the wire format accepts.
pub const MAX_CARRIER_BYTES: usize = 256;

/// Whether a snapshot field has a trustworthy platform value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum FieldStatus {
	/// The platform did not say, or the reading is ambiguous.
	Unknown = 0,
	/// This host has no cellular radio / API for the field.
	Unavailable = 1,
	/// The OS denied or restricted access to the field.
	Restricted = 2,
	/// A concrete value follows (or is carried beside this status).
	Ready = 3,
}

impl FieldStatus {
	fn from_code(code: u32) -> Option<Self> {
		match code {
			0 => Some(Self::Unknown),
			1 => Some(Self::Unavailable),
			2 => Some(Self::Restricted),
			3 => Some(Self::Ready),
			_ => None,
		}
	}
}

/// Optional field that refuses to invent a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field<T> {
	/// No trustworthy signal.
	Unknown,
	/// Platform has no source for this field.
	Unavailable,
	/// Access denied or restricted by policy.
	Restricted,
	/// Concrete platform value.
	Value(T),
}

impl<T> Field<T> {
	/// Maps a status without a value onto the matching variant.
	fn from_status(status: FieldStatus) -> Result<Self, Error> {
		match status {
			FieldStatus::Unknown => Ok(Self::Unknown),
			FieldStatus::Unavailable => Ok(Self::Unavailable),
			FieldStatus::Restricted => Ok(Self::Restricted),
			FieldStatus::Ready => Err(Error::InvalidArgument),
		}
	}

	fn status(&self) -> FieldStatus {
		match self {
			Self::Unknown => FieldStatus::Unknown,
			Self::Unavailable => FieldStatus::Unavailable,
			Self::Restricted => FieldStatus::Restricted,
			Self::Value(_) => FieldStatus::Ready,
		}
	}

	/// Returns the inner value when Ready.
	pub fn value(&self) -> Option<&T> {
		match self {
			Self::Value(value) => Some(value),
			_ => None,
		}
	}
}

/// Physical cellular radio generation, or an explicit non-value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum RadioGeneration {
	/// Ambiguous or unreported.
	Unknown = 0,
	/// No cellular radio on this host.
	Unavailable = 1,
	/// Telephony APIs are restricted.
	Restricted = 2,
	/// `2G` / GSM / CDMA 1x class.
	TwoG = 3,
	/// `3G` / UMTS / EVDO class.
	ThreeG = 4,
	/// `4G` / LTE class.
	FourG = 5,
	/// `5G` NR class.
	FiveG = 6,
}

impl RadioGeneration {
	fn from_code(code: u32) -> Option<Self> {
		match code {
			0 => Some(Self::Unknown),
			1 => Some(Self::Unavailable),
			2 => Some(Self::Restricted),
			3 => Some(Self::TwoG),
			4 => Some(Self::ThreeG),
			5 => Some(Self::FourG),
			6 => Some(Self::FiveG),
			_ => None,
		}
	}
}

/// `VoIP` allowance with explicit non-values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Voip {
	/// Ambiguous or unreported.
	Unknown = 0,
	/// Platform has no `VoIP` capability signal.
	Unavailable = 1,
	/// Access to the signal is restricted.
	Restricted = 2,
	/// Carrier / OS reports `VoIP` is not allowed.
	False = 3,
	/// Carrier / OS reports `VoIP` is allowed.
	True = 4,
}

impl Voip {
	fn from_code(code: u32) -> Option<Self> {
		match code {
			0 => Some(Self::Unknown),
			1 => Some(Self::Unavailable),
			2 => Some(Self::Restricted),
			3 => Some(Self::False),
			4 => Some(Self::True),
			_ => None,
		}
	}

	/// Maps an optional boolean onto the wire enum without inventing values.
	pub fn from_bool(value: Option<bool>) -> Self {
		match value {
			None => Self::Unknown,
			Some(false) => Self::False,
			Some(true) => Self::True,
		}
	}
}

/// One cellular identity / radio reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
	/// Display carrier name when Ready.
	pub carrier_name: Field<String>,
	/// Mobile country code when Ready.
	pub mcc: Field<u16>,
	/// Mobile network code when Ready.
	pub mnc: Field<u16>,
	/// Reported radio generation.
	pub generation: RadioGeneration,
	/// Whether the carrier / OS allows `VoIP`.
	pub allows_voip: Voip,
}

impl Snapshot {
	/// Every field Unavailable — used by desktop and honest web stubs.
	pub fn unavailable() -> Self {
		Self {
			carrier_name: Field::Unavailable,
			mcc: Field::Unavailable,
			mnc: Field::Unavailable,
			generation: RadioGeneration::Unavailable,
			allows_voip: Voip::Unavailable,
		}
	}

	/// Encodes the snapshot, rejecting empty Ready strings and oversized names.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(64);
		output.u32(self.generation as u32);
		output.u32(self.allows_voip as u32);
		encode_code(&mut output, &self.mcc);
		encode_code(&mut output, &self.mnc);
		output.u32(self.carrier_name.status() as u32);
		match &self.carrier_name {
			Field::Value(name) => {
				if name.is_empty() || name.len() > MAX_CARRIER_BYTES {
					return Err(Error::InvalidArgument);
				}
				output.bytes(name.as_bytes())?;
			}
			_ => output.bytes(b"")?,
		}
		Ok(output.finish())
	}

	/// Validates every enum, UTF-8, length, and rejects trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let generation = RadioGeneration::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;
		let allows_voip = Voip::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;
		let mcc = decode_code(&mut input)?;
		let mnc = decode_code(&mut input)?;
		let carrier_status = FieldStatus::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;
		let carrier_bytes = input.string()?;
		if carrier_bytes.len() > MAX_CARRIER_BYTES {
			return Err(Error::TooLarge);
		}
		let carrier_name = match carrier_status {
			FieldStatus::Ready => {
				if carrier_bytes.is_empty() {
					return Err(Error::InvalidArgument);
				}
				Field::Value(carrier_bytes.to_owned())
			}
			status => {
				if !carrier_bytes.is_empty() {
					return Err(Error::InvalidArgument);
				}
				Field::from_status(status)?
			}
		};
		input.finish()?;
		Ok(Self {
			carrier_name,
			mcc,
			mnc,
			generation,
			allows_voip,
		})
	}
}

fn encode_code(output: &mut Encoder, field: &Field<u16>) {
	output.u32(field.status() as u32);
	match field {
		Field::Value(code) => output.u32(u32::from(*code)),
		_ => output.u32(0),
	}
}

fn decode_code(input: &mut Decoder<'_>) -> Result<Field<u16>, Error> {
	let status = FieldStatus::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;
	let value = input.u32()?;
	match status {
		FieldStatus::Ready => {
			let code = u16::try_from(value).map_err(|_| Error::InvalidArgument)?;
			Ok(Field::Value(code))
		}
		status => {
			if value != 0 {
				return Err(Error::InvalidArgument);
			}
			Field::from_status(status)
		}
	}
}

/// Desktop hosts have no cellular radio; every field is Unavailable.
#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub mod desktop {
	use super::*;

	/// Synchronous host adapter that never invents carrier data.
	#[derive(Debug, Default)]
	pub struct Cellular;

	impl leptoskit_bridge::Plugin for Cellular {
		fn name(&self) -> &'static str {
			NAME
		}

		fn methods(&self) -> &'static [MethodId] {
			&[SNAPSHOT]
		}

		fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
			if method != SNAPSHOT {
				return Err(Error::Unavailable);
			}
			if !payload.is_empty() {
				return Err(Error::InvalidArgument);
			}
			Snapshot::unavailable().encode()
		}
	}
}

/// Browser builds have no trustworthy carrier APIs; every field is Unavailable.
/// `NetworkInformation.effectiveType` belongs to the network plugin, not here.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Returns an all-Unavailable snapshot.
	pub fn snapshot() -> Snapshot {
		Snapshot::unavailable()
	}

	#[cfg(test)]
	mod tests {
		use wasm_bindgen_test::wasm_bindgen_test;

		use super::*;
		wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

		#[wasm_bindgen_test]
		fn browser_snapshot_stays_unavailable() {
			assert_eq!(snapshot(), Snapshot::unavailable());
		}
	}
}

/// Cached guest client using the binary bridge.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolved cellular plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Cellular(leptoskit_bridge::guest::Client);

	impl Cellular {
		/// Resolves an installed host plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads the current cellular snapshot.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn unavailable_snapshot_round_trips() {
		let reading = Snapshot::unavailable();
		let bytes = reading.encode().unwrap();
		assert_eq!(Snapshot::decode(&bytes), Ok(reading));
	}

	#[test]
	fn ready_fields_round_trip_and_reject_invalid_wire() {
		let reading = Snapshot {
			carrier_name: Field::Value("Example".into()),
			mcc: Field::Value(310),
			mnc: Field::Value(260),
			generation: RadioGeneration::FiveG,
			allows_voip: Voip::True,
		};
		let bytes = reading.encode().unwrap();
		assert_eq!(Snapshot::decode(&bytes), Ok(reading.clone()));

		assert_eq!(
			Snapshot::decode(&bytes[..bytes.len() - 1]),
			Err(Error::InvalidArgument)
		);
		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert_eq!(Snapshot::decode(&trailing), Err(Error::InvalidArgument));

		let mut invalid = bytes.to_vec();
		// Corrupt generation.
		invalid[0] = 99;
		assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));

		let empty_ready = Snapshot {
			carrier_name: Field::Value(String::new()),
			..reading
		};
		assert_eq!(empty_ready.encode(), Err(Error::InvalidArgument));

		let too_long = Snapshot {
			carrier_name: Field::Value("a".repeat(MAX_CARRIER_BYTES + 1)),
			..Snapshot::unavailable()
		};
		assert_eq!(too_long.encode(), Err(Error::InvalidArgument));
	}

	#[test]
	fn every_status_and_generation_round_trips() {
		let statuses = [
			Field::Unknown,
			Field::Unavailable,
			Field::Restricted,
			Field::Value(1_u16),
		];
		let names = [
			Field::Unknown,
			Field::Unavailable,
			Field::Restricted,
			Field::Value("Carrier".into()),
		];
		let generations = [
			RadioGeneration::Unknown,
			RadioGeneration::Unavailable,
			RadioGeneration::Restricted,
			RadioGeneration::TwoG,
			RadioGeneration::ThreeG,
			RadioGeneration::FourG,
			RadioGeneration::FiveG,
		];
		let voips = [
			Voip::Unknown,
			Voip::Unavailable,
			Voip::Restricted,
			Voip::False,
			Voip::True,
		];

		for (index, generation) in generations.into_iter().enumerate() {
			let reading = Snapshot {
				carrier_name: names[index % names.len()].clone(),
				mcc: statuses[index % statuses.len()].clone(),
				mnc: statuses[(index + 1) % statuses.len()].clone(),
				generation,
				allows_voip: voips[index % voips.len()],
			};
			assert_eq!(Snapshot::decode(&reading.encode().unwrap()), Ok(reading));
		}
	}

	#[test]
	fn voip_from_bool_never_invents_restricted() {
		assert_eq!(Voip::from_bool(None), Voip::Unknown);
		assert_eq!(Voip::from_bool(Some(false)), Voip::False);
		assert_eq!(Voip::from_bool(Some(true)), Voip::True);
	}

	#[test]
	fn field_accessors_and_status_mapping() {
		assert_eq!(Field::<u16>::Unknown.value(), None);
		assert_eq!(Field::Value(42_u16).value(), Some(&42));
		assert_eq!(
			Field::<u16>::from_status(FieldStatus::Ready),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Field::<u16>::from_status(FieldStatus::Unknown).unwrap(),
			Field::Unknown
		);
		assert_eq!(FieldStatus::from_code(99), None);
		assert_eq!(RadioGeneration::from_code(99), None);
		assert_eq!(Voip::from_code(99), None);
	}

	#[test]
	fn decode_rejects_inconsistent_and_oversized_wire() {
		let valid = Snapshot {
			carrier_name: Field::Value("Carrier".into()),
			mcc: Field::Value(310),
			mnc: Field::Value(260),
			generation: RadioGeneration::FourG,
			allows_voip: Voip::False,
		}
		.encode()
		.unwrap();

		let mut bad_status = valid.to_vec();
		// Carrier status follows generation, voip, mcc×2, mnc×2 (6×u32 = 24).
		bad_status[24] = 99;
		assert_eq!(Snapshot::decode(&bad_status), Err(Error::InvalidArgument));

		let mut bad_voip = valid.to_vec();
		bad_voip[4] = 99;
		assert_eq!(Snapshot::decode(&bad_voip), Err(Error::InvalidArgument));

		let mut empty_ready = Encoder::with_capacity(64);
		empty_ready.u32(RadioGeneration::TwoG as u32);
		empty_ready.u32(Voip::True as u32);
		encode_code(&mut empty_ready, &Field::Unavailable);
		encode_code(&mut empty_ready, &Field::Unavailable);
		empty_ready.u32(FieldStatus::Ready as u32);
		empty_ready.bytes(b"").unwrap();
		assert_eq!(
			Snapshot::decode(&empty_ready.finish()),
			Err(Error::InvalidArgument)
		);

		let mut named_unknown = Encoder::with_capacity(64);
		named_unknown.u32(RadioGeneration::ThreeG as u32);
		named_unknown.u32(Voip::Unknown as u32);
		encode_code(&mut named_unknown, &Field::Unknown);
		encode_code(&mut named_unknown, &Field::Unknown);
		named_unknown.u32(FieldStatus::Unknown as u32);
		named_unknown.bytes(b"leak").unwrap();
		assert_eq!(
			Snapshot::decode(&named_unknown.finish()),
			Err(Error::InvalidArgument)
		);

		let oversized = "a".repeat(MAX_CARRIER_BYTES + 1);
		let mut too_large = Encoder::with_capacity(64);
		too_large.u32(RadioGeneration::FiveG as u32);
		too_large.u32(Voip::Restricted as u32);
		encode_code(&mut too_large, &Field::Restricted);
		encode_code(&mut too_large, &Field::Restricted);
		too_large.u32(FieldStatus::Ready as u32);
		too_large.bytes(oversized.as_bytes()).unwrap();
		assert_eq!(Snapshot::decode(&too_large.finish()), Err(Error::TooLarge));

		let mut nonzero_unknown = Encoder::with_capacity(32);
		nonzero_unknown.u32(RadioGeneration::Unknown as u32);
		nonzero_unknown.u32(Voip::Unavailable as u32);
		nonzero_unknown.u32(FieldStatus::Unknown as u32);
		nonzero_unknown.u32(7);
		encode_code(&mut nonzero_unknown, &Field::Unavailable);
		nonzero_unknown.u32(FieldStatus::Unavailable as u32);
		nonzero_unknown.bytes(b"").unwrap();
		assert_eq!(
			Snapshot::decode(&nonzero_unknown.finish()),
			Err(Error::InvalidArgument)
		);
	}

	#[cfg(any(
		target_os = "macos",
		target_os = "windows",
		all(target_os = "linux", not(target_env = "ohos"))
	))]
	#[test]
	fn desktop_adapter_reports_unavailable_and_rejects_bad_calls() {
		use leptoskit_bridge::Plugin;

		let mut cellular = desktop::Cellular;
		assert_eq!(cellular.name(), NAME);
		assert_eq!(cellular.methods(), &[SNAPSHOT]);
		let reading = Snapshot::decode(&cellular.invoke(SNAPSHOT, Bytes::new()).unwrap()).unwrap();
		assert_eq!(reading, Snapshot::unavailable());
		assert_eq!(
			cellular.invoke(SNAPSHOT, Bytes::from_static(&[0])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			cellular.invoke(MethodId(9), Bytes::new()),
			Err(Error::Unavailable)
		);
	}
}
