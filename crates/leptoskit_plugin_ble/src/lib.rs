//! Bluetooth Low Energy (BLE) GATT central scanning, subscriptions, and
//! standard wearable sensor decoders.
//!
//! Works with heart-rate chest straps (Polar, Garmin, Wahoo), smart rings,
//! pulse oximeters, thermometers, and cycling/running sensors on iOS, watchOS,
//! macOS, Android, Wear OS, and HarmonyOS.

#![forbid(unsafe_code)]

use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

pub mod gatt;

/// Automatic native registration name.
pub const NAME: &str = "ble";
/// Adapter availability and permission state, empty request.
pub const AVAILABILITY: MethodId = MethodId(0);
/// Request Bluetooth permissions, empty request.
pub const REQUEST_PERMISSION: MethodId = MethodId(1);
/// Scan for nearby BLE peripherals matching [`ScanOptions`].
pub const SCAN: MethodId = MethodId(2);
/// Connect to a peripheral by id.
pub const CONNECT: MethodId = MethodId(3);
/// Disconnect from a peripheral by id.
pub const DISCONNECT: MethodId = MethodId(4);
/// Subscribe to GATT characteristic notifications ([`Subscription`]).
pub const SUBSCRIBE: MethodId = MethodId(5);
/// Unsubscribe from a GATT characteristic ([`Subscription`]).
pub const UNSUBSCRIBE: MethodId = MethodId(6);
/// Drain the next [`NotificationBatch`], empty request.
pub const NEXT_NOTIFICATIONS: MethodId = MethodId(7);
/// Complete native method table.
pub const METHODS: &[MethodId] = &[
	AVAILABILITY,
	REQUEST_PERMISSION,
	SCAN,
	CONNECT,
	DISCONNECT,
	SUBSCRIBE,
	UNSUBSCRIBE,
	NEXT_NOTIFICATIONS,
];

/// Maximum notifications returned per batch.
pub const MAX_NOTIFICATIONS: usize = 128;
const ABSENT: u32 = u32::MAX;

/// Bluetooth adapter state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum BleAvailability {
	/// Ready to scan and connect.
	PoweredOn = 0,
	/// Radio is switched off.
	PoweredOff = 1,
	/// App lacks Bluetooth permission.
	Unauthorized = 2,
	/// Hardware or platform does not support BLE.
	Unsupported = 3,
}

impl BleAvailability {
	/// Decodes an availability reply.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut r = Reader::new(payload);
		let code = r.u32()?;
		r.finish()?;
		match code {
			0 => Ok(Self::PoweredOn),
			1 => Ok(Self::PoweredOff),
			2 => Ok(Self::Unauthorized),
			3 => Ok(Self::Unsupported),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Parameters for a bounded BLE scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOptions {
	/// Optional GATT service UUID filters (lowercase canonical or 16-bit hex).
	pub service_uuids: Vec<String>,
	/// Scan duration in milliseconds (`500..=30_000`).
	pub timeout_ms: u32,
}

impl ScanOptions {
	/// Encodes a scan request.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		if !(500..=30_000).contains(&self.timeout_ms) || self.service_uuids.len() > 32 {
			return Err(Error::InvalidArgument);
		}
		let mut w = Writer::default();
		w.u32(self.timeout_ms).u32(self.service_uuids.len() as u32);
		for uuid in &self.service_uuids {
			w.str(uuid)?;
		}
		Ok(w.finish())
	}
}

/// One discovered BLE peripheral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleDevice {
	/// Platform peripheral identifier (UUID on Apple, MAC address on
	/// Android/HarmonyOS).
	pub id: String,
	/// Advertised local name, when present.
	pub name: Option<String>,
	/// Signal strength in dBm.
	pub rssi: i32,
	/// Advertised GATT service UUIDs.
	pub service_uuids: Vec<String>,
}

/// Encodes a list of discovered devices.
pub fn encode_devices(devices: &[BleDevice]) -> Result<Vec<u8>, Error> {
	let mut w = Writer::default();
	w.u32(devices.len() as u32);
	for dev in devices {
		w.str(&dev.id)?
			.opt_str(dev.name.as_deref())?
			.i32(dev.rssi)
			.u32(dev.service_uuids.len() as u32);
		for uuid in &dev.service_uuids {
			w.str(uuid)?;
		}
	}
	Ok(w.finish())
}

/// Decodes a scan reply.
pub fn decode_devices(payload: &[u8]) -> Result<Vec<BleDevice>, Error> {
	let mut r = Reader::new(payload);
	let count = r.u32()? as usize;
	if count > 256 || count.saturating_mul(16) > r.remaining() {
		return Err(Error::InvalidArgument);
	}
	let mut out = Vec::with_capacity(count);
	for _ in 0..count {
		let id = r.str()?.to_owned();
		let name = r.opt_str()?.map(str::to_owned);
		let rssi = r.i32()?;
		let uuid_count = r.u32()? as usize;
		if uuid_count > 64 {
			return Err(Error::InvalidArgument);
		}
		let mut service_uuids = Vec::with_capacity(uuid_count);
		for _ in 0..uuid_count {
			service_uuids.push(r.str()?.to_owned());
		}
		out.push(BleDevice {
			id,
			name,
			rssi,
			service_uuids,
		});
	}
	r.finish()?;
	Ok(out)
}

/// Identifies a GATT characteristic on a connected peripheral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription<'a> {
	/// Peripheral id.
	pub device_id: &'a str,
	/// Service UUID.
	pub service_uuid: &'a str,
	/// Characteristic UUID.
	pub characteristic_uuid: &'a str,
}

impl Subscription<'_> {
	/// Encodes a subscribe/unsubscribe request.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		if self.device_id.is_empty()
			|| self.service_uuid.is_empty()
			|| self.characteristic_uuid.is_empty()
		{
			return Err(Error::InvalidArgument);
		}
		let mut w = Writer::default();
		w.str(self.device_id)?
			.str(self.service_uuid)?
			.str(self.characteristic_uuid)?;
		Ok(w.finish())
	}
}

/// One GATT characteristic notification received from a connected sensor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GattNotification {
	/// Peripheral id.
	pub device_id: String,
	/// Service UUID (lowercase).
	pub service_uuid: String,
	/// Characteristic UUID (lowercase).
	pub characteristic_uuid: String,
	/// Unix milliseconds when received.
	pub time_ms: i64,
	/// Raw GATT characteristic bytes.
	pub value: Vec<u8>,
}

/// One bounded batch of GATT notifications.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotificationBatch {
	/// Notifications in arrival order.
	pub notifications: Vec<GattNotification>,
	/// Notifications dropped by the native buffer before drain.
	pub dropped: u32,
}

impl NotificationBatch {
	/// Encodes a notification batch.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		if self.notifications.len() > MAX_NOTIFICATIONS {
			return Err(Error::TooLarge);
		}
		let mut w = Writer::default();
		w.u32(self.dropped).u32(self.notifications.len() as u32);
		for n in &self.notifications {
			w.str(&n.device_id)?
				.str(&n.service_uuid)?
				.str(&n.characteristic_uuid)?
				.i64(n.time_ms)
				.bytes(&n.value)?;
		}
		Ok(w.finish())
	}

	/// Decodes a notification batch.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut r = Reader::new(payload);
		let dropped = r.u32()?;
		let count = r.u32()? as usize;
		if count > MAX_NOTIFICATIONS || count.saturating_mul(24) > r.remaining() {
			return Err(Error::InvalidArgument);
		}
		let mut notifications = Vec::with_capacity(count);
		for _ in 0..count {
			let device_id = r.str()?.to_owned();
			let service_uuid = r.str()?.to_owned();
			let characteristic_uuid = r.str()?.to_owned();
			let time_ms = r.i64()?;
			let value = r.bytes()?.to_vec();
			notifications.push(GattNotification {
				device_id,
				service_uuid,
				characteristic_uuid,
				time_ms,
				value,
			});
		}
		r.finish()?;
		Ok(Self {
			notifications,
			dropped,
		})
	}
}

/// Encodes a peripheral id request for `CONNECT` / `DISCONNECT`.
pub fn device_request(device_id: &str) -> Result<Vec<u8>, Error> {
	if device_id.is_empty() {
		return Err(Error::InvalidArgument);
	}
	let mut w = Writer::default();
	w.str(device_id)?;
	Ok(w.finish())
}

#[derive(Default)]
struct Writer(Vec<u8>);

impl Writer {
	fn u32(&mut self, value: u32) -> &mut Self {
		self.0.extend_from_slice(&value.to_le_bytes());
		self
	}

	fn i32(&mut self, value: i32) -> &mut Self {
		self.0.extend_from_slice(&value.to_le_bytes());
		self
	}

	fn i64(&mut self, value: i64) -> &mut Self {
		self.0.extend_from_slice(&value.to_le_bytes());
		self
	}

	fn bytes(&mut self, value: &[u8]) -> Result<&mut Self, Error> {
		let len = u32::try_from(value.len())
			.ok()
			.filter(|l| *l != ABSENT)
			.ok_or(Error::TooLarge)?;
		self.u32(len);
		self.0.extend_from_slice(value);
		Ok(self)
	}

	fn str(&mut self, value: &str) -> Result<&mut Self, Error> {
		self.bytes(value.as_bytes())
	}

	fn opt_str(&mut self, value: Option<&str>) -> Result<&mut Self, Error> {
		match value {
			Some(s) => self.str(s),
			None => Ok(self.u32(ABSENT)),
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

	fn i32(&mut self) -> Result<i32, Error> {
		self.take().map(i32::from_le_bytes)
	}

	fn i64(&mut self) -> Result<i64, Error> {
		self.take().map(i64::from_le_bytes)
	}

	fn bytes(&mut self) -> Result<&'a [u8], Error> {
		let len = self.u32()?;
		if len == ABSENT {
			return Err(Error::InvalidArgument);
		}
		let (head, tail) = self
			.0
			.split_at_checked(len as usize)
			.ok_or(Error::InvalidArgument)?;
		self.0 = tail;
		Ok(head)
	}

	fn str(&mut self) -> Result<&'a str, Error> {
		std::str::from_utf8(self.bytes()?).map_err(|_| Error::InvalidArgument)
	}

	fn opt_str(&mut self) -> Result<Option<&'a str>, Error> {
		let len = self.u32()?;
		if len == ABSENT {
			return Ok(None);
		}
		let (head, tail) = self
			.0
			.split_at_checked(len as usize)
			.ok_or(Error::InvalidArgument)?;
		self.0 = tail;
		std::str::from_utf8(head)
			.map(Some)
			.map_err(|_| Error::InvalidArgument)
	}

	fn finish(self) -> Result<(), Error> {
		self.0
			.is_empty()
			.then_some(())
			.ok_or(Error::InvalidArgument)
	}
}

/// Native guest client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Cached bridge handle.
	#[derive(Debug, Clone, Copy)]
	pub struct Ble(leptoskit_bridge::guest::Client);

	impl Ble {
		/// Resolves the selected plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Adapter availability.
		pub async fn availability(self) -> Result<BleAvailability, Error> {
			BleAvailability::decode(&self.0.call(AVAILABILITY, &[])?.await?)
		}

		/// Requests Bluetooth scan/connect permission.
		pub async fn request_permission(self) -> Result<BleAvailability, Error> {
			BleAvailability::decode(&self.0.call(REQUEST_PERMISSION, &[])?.await?)
		}

		/// Scans for nearby peripherals.
		pub async fn scan(self, options: &ScanOptions) -> Result<Vec<BleDevice>, Error> {
			let req = options.encode()?;
			decode_devices(&self.0.call(SCAN, &req)?.await?)
		}

		/// Connects to peripheral `device_id`.
		pub async fn connect(self, device_id: &str) -> Result<(), Error> {
			let req = device_request(device_id)?;
			self.0.call(CONNECT, &req)?.await?;
			Ok(())
		}

		/// Disconnects from peripheral `device_id`.
		pub async fn disconnect(self, device_id: &str) -> Result<(), Error> {
			let req = device_request(device_id)?;
			self.0.call(DISCONNECT, &req)?.await?;
			Ok(())
		}

		/// Subscribes to characteristic notifications.
		pub async fn subscribe(self, sub: &Subscription<'_>) -> Result<(), Error> {
			let req = sub.encode()?;
			self.0.call(SUBSCRIBE, &req)?.await?;
			Ok(())
		}

		/// Unsubscribes from characteristic notifications.
		pub async fn unsubscribe(self, sub: &Subscription<'_>) -> Result<(), Error> {
			let req = sub.encode()?;
			self.0.call(UNSUBSCRIBE, &req)?.await?;
			Ok(())
		}

		/// Drains the next batch of GATT notifications.
		pub async fn next_notifications(self) -> Result<NotificationBatch, Error> {
			NotificationBatch::decode(&self.0.call(NEXT_NOTIFICATIONS, &[])?.await?)
		}
	}
}

/// Web fallback.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Reports unsupported BLE in non-native builds.
	pub fn availability() -> BleAvailability {
		BleAvailability::Unsupported
	}
}

/// Desktop fallback.
#[cfg(not(target_arch = "wasm32"))]
pub mod desktop {
	use super::*;

	/// Reports unsupported BLE on desktops without native registration.
	pub fn availability() -> BleAvailability {
		BleAvailability::Unsupported
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
	fn devices_and_notification_batches_round_trip() {
		let devices = vec![BleDevice {
			id: "AA:BB:CC:DD:EE:FF".into(),
			name: Some("Polar H10".into()),
			rssi: -58,
			service_uuids: vec![gatt::SERVICE_HEART_RATE.into()],
		}];
		assert_eq!(
			decode_devices(&encode_devices(&devices).unwrap()),
			Ok(devices)
		);

		let batch = NotificationBatch {
			dropped: 1,
			notifications: vec![GattNotification {
				device_id: "AA:BB:CC:DD:EE:FF".into(),
				service_uuid: gatt::SERVICE_HEART_RATE.into(),
				characteristic_uuid: gatt::CHAR_HEART_RATE_MEASUREMENT.into(),
				time_ms: 1_700_000_000_000,
				value: vec![0x06, 74],
			}],
		};
		assert_eq!(
			NotificationBatch::decode(&batch.encode().unwrap()),
			Ok(batch)
		);
	}
}
