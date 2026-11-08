//! <!-- {=p_networkSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_network` as a direct dependency and select the
//! `network` plugin in `native.plugins`; supported generated hosts discover it
//! from Cargo metadata. Generated Linux hosts register it where it has a Linux
//! Rust implementation; generated Windows and terminal hosts currently omit
//! runtime plugins, so use explicit registration in a custom host there. Guests
//! enable the `guest` feature and reuse `guest::Network` rather than resolving
//! the plugin on every call. Apple uses `NWPathMonitor`, Android uses
//! `ConnectivityManager`, desktop reports interface state, and the browser uses
//! `navigator.onLine` plus `NetworkInformation`. Each reading names a
//! primary connection and the full set of transports in use (Wi-Fi, cellular,
//! ethernet, Bluetooth, VPN, satellite, other), as Flutter's
//! `connectivity_plus` lists them; an active connection never claims a server
//! is reachable.
//!
//! <!-- {/p_networkSummary} -->
//!
//! The wire snapshot is nine little-endian u32s: connection kind, connected,
//! internet validated, expensive, data saver, estimated kbps, estimated RTT ms,
//! effective connection class, and the set of active transports. Boolean
//! fields use unknown/false/true = 0/1/2; numeric estimates use `u32::MAX` for
//! unknown. The transport set is a bitset of [`Transport`] codes; bits the
//! wire does not define are refused.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Registered plugin name.
pub const NAME: &str = "network";
/// Nine little-endian u32 fields.
const SNAPSHOT_BYTES: usize = 36;
/// Empty payload -> current [`Snapshot`].
pub const SNAPSHOT: MethodId = MethodId(0);
/// Previous encoded snapshot -> next differing snapshot. Mobile/Apple only.
/// Dropping the guest call unregisters its native observer. One outstanding
/// wait is supported per plugin; a second receives `Busy`.
pub const WAIT_FOR_CHANGE: MethodId = MethodId(1);

/// Transport reported by the OS, or Unknown when it cannot be determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Connection {
	/// No trustworthy transport information.
	Unknown = 0,
	/// Platform reports no active connection.
	None = 1,
	/// Wireless LAN.
	Wifi = 2,
	/// Cellular data.
	Cellular = 3,
	/// Wired LAN.
	Ethernet = 4,
	/// Bluetooth network.
	Bluetooth = 5,
	/// Virtual private network.
	Vpn = 6,
	/// A known connection with another transport.
	Other = 7,
}

impl Connection {
	/// Every transport the wire snapshot defines, in code order.
	const ALL: [Self; 8] = [
		Self::Unknown,
		Self::None,
		Self::Wifi,
		Self::Cellular,
		Self::Ethernet,
		Self::Bluetooth,
		Self::Vpn,
		Self::Other,
	];

	/// Resolves a wire code, refusing codes the snapshot does not define.
	fn from_code(code: u32) -> Option<Self> {
		Self::ALL
			.into_iter()
			.find(|connection| *connection as u32 == code)
	}
}

/// One transport the active network uses. A network can use several at once:
/// a VPN runs over Wi-Fi or cellular, and the platform reports both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Transport {
	/// Wireless LAN, including Android Wi-Fi Aware.
	Wifi = 0,
	/// Cellular data (`mobile` in Flutter's `connectivity_plus`).
	Cellular = 1,
	/// Wired LAN.
	Ethernet = 2,
	/// Bluetooth tethering.
	Bluetooth = 3,
	/// Virtual private network.
	Vpn = 4,
	/// Satellite: Android's satellite transport, or an Apple path the system
	/// marks ultra-constrained.
	Satellite = 5,
	/// A transport the platform reports that has no code of its own.
	Other = 6,
}

impl Transport {
	/// Every transport, in bit order.
	pub const ALL: [Self; 7] = [
		Self::Wifi,
		Self::Cellular,
		Self::Ethernet,
		Self::Bluetooth,
		Self::Vpn,
		Self::Satellite,
		Self::Other,
	];

	/// The transport set holding only this transport.
	const fn bit(self) -> u32 {
		1 << self as u32
	}
}

/// The set of transports the active network uses, as Flutter's
/// `connectivity_plus` reports a list. Empty means no connection or no
/// transport information; [`Snapshot::connection`] tells the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Transports(u32);

impl Transports {
	/// No transports.
	pub const EMPTY: Self = Self(0);
	/// Every bit the wire defines.
	const MASK: u32 = (1 << Transport::ALL.len()) - 1;

	/// Builds a set from transports.
	pub fn of(transports: impl IntoIterator<Item = Transport>) -> Self {
		transports.into_iter().fold(Self::EMPTY, Self::with)
	}

	/// Resolves wire bits, refusing bits the wire does not define.
	pub const fn from_bits(bits: u32) -> Option<Self> {
		if bits & !Self::MASK == 0 {
			Some(Self(bits))
		} else {
			None
		}
	}

	/// The wire bits.
	pub const fn bits(self) -> u32 {
		self.0
	}

	/// This set plus `transport`.
	#[must_use]
	pub const fn with(self, transport: Transport) -> Self {
		Self(self.0 | transport.bit())
	}

	/// Whether the set holds `transport`.
	pub const fn contains(self, transport: Transport) -> bool {
		self.0 & transport.bit() != 0
	}

	/// Whether the set holds no transports.
	pub const fn is_empty(self) -> bool {
		self.0 == 0
	}

	/// The transports in the set, in bit order.
	pub fn iter(self) -> impl Iterator<Item = Transport> {
		Transport::ALL
			.into_iter()
			.filter(move |transport| self.contains(*transport))
	}
}

impl From<Transport> for Transports {
	fn from(transport: Transport) -> Self {
		Self::EMPTY.with(transport)
	}
}

impl FromIterator<Transport> for Transports {
	fn from_iter<I: IntoIterator<Item = Transport>>(iter: I) -> Self {
		Self::of(iter)
	}
}

impl Connection {
	/// The transport a primary connection implies, if it names one.
	const fn transport(self) -> Option<Transport> {
		match self {
			Self::Unknown | Self::None => None,
			Self::Wifi => Some(Transport::Wifi),
			Self::Cellular => Some(Transport::Cellular),
			Self::Ethernet => Some(Transport::Ethernet),
			Self::Bluetooth => Some(Transport::Bluetooth),
			Self::Vpn => Some(Transport::Vpn),
			Self::Other => Some(Transport::Other),
		}
	}
}

/// Browser's estimated performance class, not the physical cellular generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum EffectiveType {
	/// No estimate available.
	Unknown = 0,
	/// Very slow connection.
	Slow2g = 1,
	/// Estimated 2G performance.
	TwoG = 2,
	/// Estimated 3G performance.
	ThreeG = 3,
	/// Estimated 4G performance.
	FourG = 4,
}

/// A single system reading. Unknown capabilities remain `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
	/// The primary transport. On Android a VPN wins over the network it runs
	/// on; [`transports`](Self::transports) lists every transport.
	pub connection: Connection,
	/// Local connection/online hint; does not establish internet connectivity.
	pub connected: Option<bool>,
	/// OS-validated internet capability (currently Android only).
	pub internet_validated: Option<bool>,
	/// Metered/expensive connection, if reported by the platform.
	pub expensive: Option<bool>,
	/// Low-data/save-data mode, if reported by the platform.
	pub data_saver: Option<bool>,
	/// Browser estimate in kilobits per second; not a measured throughput test.
	pub downlink_kbps: Option<u32>,
	/// Browser estimate in milliseconds.
	pub round_trip_ms: Option<u32>,
	/// Browser's estimated connection performance.
	pub effective_type: EffectiveType,
	/// Every transport the active network uses. It always holds the primary
	/// [`connection`](Self::connection)'s transport, and is empty when there
	/// is no connection.
	pub transports: Transports,
}

fn flag(value: Option<bool>) -> u32 {
	match value {
		None => 0,
		Some(false) => 1,
		Some(true) => 2,
	}
}
fn read_flag(value: u32) -> Result<Option<bool>, Error> {
	match value {
		0 => Ok(None),
		1 => Ok(Some(false)),
		2 => Ok(Some(true)),
		_ => Err(Error::InvalidArgument),
	}
}
fn optional(value: u32) -> Option<u32> {
	(value != u32::MAX).then_some(value)
}

impl Snapshot {
	/// Creates a reading with only a local connectivity hint. The transport
	/// set holds the primary connection's transport alone.
	pub fn connection(connection: Connection, connected: Option<bool>) -> Self {
		Self {
			transports: connection
				.transport()
				.map_or(Transports::EMPTY, Transports::from),
			connection,
			connected,
			internet_validated: None,
			expensive: None,
			data_saver: None,
			downlink_kbps: None,
			round_trip_ms: None,
			effective_type: EffectiveType::Unknown,
		}
	}

	/// The transport set must agree with the primary connection: no
	/// transports without a connection, and the primary's own transport
	/// present.
	fn transports_agree(self) -> bool {
		match (self.connection, self.connection.transport()) {
			(Connection::None, _) => self.transports.is_empty(),
			(_, Some(primary)) => self.transports.contains(primary),
			(_, None) => true,
		}
	}

	/// Encodes a snapshot. `u32::MAX` is reserved and cannot be an estimate,
	/// and the transport set must agree with the primary connection.
	pub fn encode(self) -> Result<Bytes, Error> {
		if self.downlink_kbps == Some(u32::MAX)
			|| self.round_trip_ms == Some(u32::MAX)
			|| !self.transports_agree()
		{
			return Err(Error::InvalidArgument);
		}

		let mut output = Encoder::with_capacity(SNAPSHOT_BYTES);
		for field in [
			self.connection as u32,
			flag(self.connected),
			flag(self.internet_validated),
			flag(self.expensive),
			flag(self.data_saver),
			self.downlink_kbps.unwrap_or(u32::MAX),
			self.round_trip_ms.unwrap_or(u32::MAX),
			self.effective_type as u32,
			self.transports.bits(),
		] {
			output.u32(field);
		}
		Ok(output.finish())
	}

	/// Rejects truncated/trailing data, invalid enum/boolean values, unknown
	/// transport bits, and a transport set that contradicts the connection.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);

		let connection = Connection::from_code(input.u32()?).ok_or(Error::InvalidArgument)?;

		let connected = read_flag(input.u32()?)?;
		let internet_validated = read_flag(input.u32()?)?;
		let expensive = read_flag(input.u32()?)?;
		let data_saver = read_flag(input.u32()?)?;
		let downlink_kbps = optional(input.u32()?);
		let round_trip_ms = optional(input.u32()?);

		let effective_type = match input.u32()? {
			0 => EffectiveType::Unknown,
			1 => EffectiveType::Slow2g,
			2 => EffectiveType::TwoG,
			3 => EffectiveType::ThreeG,
			4 => EffectiveType::FourG,
			_ => return Err(Error::InvalidArgument),
		};
		let transports = Transports::from_bits(input.u32()?).ok_or(Error::InvalidArgument)?;

		input.finish()?;
		let snapshot = Self {
			connection,
			connected,
			internet_validated,
			expensive,
			data_saver,
			downlink_kbps,
			round_trip_ms,
			effective_type,
			transports,
		};

		if !snapshot.transports_agree() {
			return Err(Error::InvalidArgument);
		}

		Ok(snapshot)
	}
}

/// Desktop interface enumeration. A live local interface does not establish a
/// default route or internet access. Virtual interfaces can also be up.
#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub mod desktop {
	use super::*;
	/// Lists local interface names and IP addresses using system APIs.
	pub fn interfaces() -> Result<Vec<if_addrs::Interface>, Error> {
		if_addrs::get_if_addrs().map_err(|_| Error::Native)
	}
	/// Reads operational interface state without inferring transport from
	/// names.
	pub fn snapshot() -> Result<Snapshot, Error> {
		let interfaces = interfaces()?;
		Ok(from_interfaces(&interfaces))
	}
	/// Loopback and link-local addresses exist without any reachable network:
	/// offline Macs keep `awdl0`/`utun*` up with `fe80::` addresses, and failed
	/// DHCP leaves a 169.254.0.0/16 address. Neither indicates connectivity.
	fn routable(entry: &if_addrs::Interface) -> bool {
		!entry.is_loopback()
			&& match entry.ip() {
				std::net::IpAddr::V4(ip) => !ip.is_link_local(),
				std::net::IpAddr::V6(ip) => !ip.is_unicast_link_local(),
			}
	}
	fn from_interfaces(interfaces: &[if_addrs::Interface]) -> Snapshot {
		let connected = interfaces
			.iter()
			.filter(|entry| routable(entry))
			.any(if_addrs::Interface::is_oper_up);
		let unknown = interfaces
			.iter()
			.filter(|entry| routable(entry))
			.any(|entry| entry.oper_status == if_addrs::IfOperStatus::Unknown);

		if connected {
			Snapshot::connection(Connection::Unknown, Some(true))
		} else if unknown {
			Snapshot::connection(Connection::Unknown, None)
		} else {
			Snapshot::connection(Connection::None, Some(false))
		}
	}
	/// Synchronous host adapter. Change waiting is unsupported on this adapter;
	/// Apple UI hosts can install the NWPathMonitor-backed Swift
	/// implementation.
	#[derive(Debug, Default)]
	pub struct Network;
	impl leptoskit_bridge::Plugin for Network {
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

			snapshot()?.encode()
		}
	}

	#[cfg(test)]
	mod tests {
		use std::net::Ipv4Addr;

		use super::*;

		/// A private LAN address, reachable only if a local interface carries
		/// it.
		const LAN: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 2);
		/// The address a failed DHCP lease leaves behind.
		const APIPA: Ipv4Addr = Ipv4Addr::new(169, 254, 10, 2);

		fn interface(ip: Ipv4Addr, oper_status: if_addrs::IfOperStatus) -> if_addrs::Interface {
			if_addrs::Interface {
				name: "test".into(),
				addr: if_addrs::IfAddr::V4(if_addrs::Ifv4Addr {
					ip,
					netmask: Ipv4Addr::new(255, 255, 255, 0),
					prefixlen: 24,
					broadcast: None,
				}),
				index: None,
				oper_status,
				is_p2p: false,
				#[cfg(windows)]
				adapter_name: "test".into(),
			}
		}

		#[test]
		fn local_interfaces_do_not_become_internet_or_invented_transport() {
			let loopback = interface(Ipv4Addr::LOCALHOST, if_addrs::IfOperStatus::Up);
			assert_eq!(from_interfaces(&[loopback]).connected, Some(false));
			let link = interface(LAN, if_addrs::IfOperStatus::Up);
			let reading = from_interfaces(&[link]);
			assert_eq!(reading.connected, Some(true));
			assert_eq!(reading.connection, Connection::Unknown);
			assert_eq!(reading.internet_validated, None);
			let unknown = interface(LAN, if_addrs::IfOperStatus::Unknown);

			assert_eq!(from_interfaces(&[unknown]).connected, None);
			let apipa = interface(APIPA, if_addrs::IfOperStatus::Up);
			assert_eq!(from_interfaces(&[apipa]).connected, Some(false));
			let mut link_local = interface(Ipv4Addr::LOCALHOST, if_addrs::IfOperStatus::Up);
			link_local.addr = if_addrs::IfAddr::V6(if_addrs::Ifv6Addr {
				ip: "fe80::1".parse().unwrap(),
				netmask: "ffff:ffff:ffff:ffff::".parse().unwrap(),
				prefixlen: 64,
				broadcast: None,
			});
			assert_eq!(from_interfaces(&[link_local]).connected, Some(false));
		}
	}
}

/// Reactive network state over this plugin's snapshot sources.
#[cfg(feature = "hooks")]
pub mod signals;

/// `NetworkInformation.type` mappings, kept off the browser so native tests
/// cover them.
#[cfg(any(
	test,
	all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest"))
))]
mod browser_type {
	use super::*;

	/// Maps `NetworkInformation.type` to the primary connection.
	pub(crate) fn connection_from_type(kind: Option<&str>) -> Connection {
		match kind {
			Some("wifi") => Connection::Wifi,
			Some("cellular") => Connection::Cellular,
			Some("ethernet") => Connection::Ethernet,
			Some("bluetooth") => Connection::Bluetooth,
			Some("other") => Connection::Other,
			_ => Connection::Unknown,
		}
	}

	/// Maps `NetworkInformation.type` to the transport set. `wimax` and
	/// `mixed` name real transports without a code of their own; `none` and
	/// `unknown` name none.
	pub(crate) fn transports_from_type(kind: Option<&str>) -> Transports {
		match kind {
			Some("wifi") => Transport::Wifi.into(),
			Some("cellular") => Transport::Cellular.into(),
			Some("ethernet") => Transport::Ethernet.into(),
			Some("bluetooth") => Transport::Bluetooth.into(),
			Some("other" | "mixed" | "wimax") => Transport::Other.into(),
			_ => Transports::EMPTY,
		}
	}
}

/// Browser connection hints. Unsupported optional properties remain unknown.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use std::cell::RefCell;
	use std::rc::Rc;

	use futures::channel::oneshot;
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;

	use super::*;
	fn property(value: &JsValue, key: &str) -> Result<JsValue, Error> {
		js_sys::Reflect::get(value, &JsValue::from_str(key)).map_err(|_| Error::Native)
	}
	fn estimate(value: &JsValue, key: &str, scale: f64) -> Result<Option<u32>, Error> {
		let number = property(value, key)?;

		if number.is_undefined() || number.is_null() {
			return Ok(None);
		}

		let number = number.as_f64().ok_or(Error::Native)? * scale;

		if !number.is_finite() || number < 0.0 || number.round() >= f64::from(u32::MAX) {
			return Err(Error::Native);
		}

		Ok(Some(number.round() as u32))
	}
	/// `navigator.onLine` is an unreliable hint. Never gate application
	/// features solely on it; attempt the actual request and handle its
	/// result.
	pub fn snapshot() -> Result<Snapshot, Error> {
		let navigator = web_sys::window().ok_or(Error::Unavailable)?.navigator();
		let online = navigator.on_line();
		let mut result = Snapshot::connection(
			if online {
				Connection::Unknown
			} else {
				Connection::None
			},
			Some(online),
		);
		let connection = property(navigator.as_ref(), "connection")?;

		if connection.is_null() || connection.is_undefined() {
			return Ok(result);
		}

		if online {
			let kind = property(&connection, "type")?.as_string();
			result.connection = browser_type::connection_from_type(kind.as_deref());
			result.transports = browser_type::transports_from_type(kind.as_deref());
		}

		result.data_saver = property(&connection, "saveData")?.as_bool();

		result.downlink_kbps = estimate(&connection, "downlink", 1000.0)?;
		result.round_trip_ms = estimate(&connection, "rtt", 1.0)?;

		result.effective_type = match property(&connection, "effectiveType")?
			.as_string()
			.as_deref()
		{
			Some("slow-2g") => EffectiveType::Slow2g,
			Some("2g") => EffectiveType::TwoG,
			Some("3g") => EffectiveType::ThreeG,
			Some("4g") => EffectiveType::FourG,
			_ => EffectiveType::Unknown,
		};
		Ok(result)
	}

	/// Answers the first differing reading; later events find the slot empty.
	fn notify(
		sender: &RefCell<Option<oneshot::Sender<Result<Snapshot, Error>>>>,
		previous: Snapshot,
	) {
		let current = snapshot();

		if current == Ok(previous) {
			return;
		}

		// Cancellation can drop the receiver before an already dispatched
		// browser callback finishes.
		if let Some(sender) = sender.borrow_mut().take() {
			let _ = sender.send(current);
		}
	}

	/// Waits for a differing reading, listening to online/offline and optional
	/// connection changes. Dropping this future unregisters all listeners.
	/// Events are coalesced into one result; no unbounded event queue is used.
	pub async fn wait_for_change(previous: Snapshot) -> Result<Snapshot, Error> {
		let window = web_sys::window().ok_or(Error::Unavailable)?;
		let (sender, receiver) = oneshot::channel();
		let sender = Rc::new(RefCell::new(Some(sender)));
		let changed = {
			let sender = Rc::clone(&sender);
			move || notify(&sender, previous)
		};
		let mut listeners = Vec::with_capacity(3);

		for event in ["online", "offline"] {
			let changed = changed.clone();

			listeners.push(gloo_events::EventListener::new(&window, event, move |_| {
				changed();
			}));
		}

		let connection = property(window.navigator().as_ref(), "connection")?;

		if !connection.is_undefined() && !connection.is_null() {
			let target = connection
				.dyn_into::<web_sys::EventTarget>()
				.map_err(|_| Error::Native)?;
			let changed = changed.clone();
			listeners.push(gloo_events::EventListener::new(
				&target,
				"change",
				move |_| changed(),
			));
		}

		// Install before sampling to avoid missing a change between them.
		changed();
		let result = receiver.await.map_err(|_| Error::Cancelled)?;
		drop(listeners);
		result
	}
}

/// Cached guest client using the binary bridge.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// Resolved network plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Network(leptoskit_bridge::guest::Client);
	impl Network {
		/// Resolves an installed host plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads current network hints.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}

		/// Waits for a changed snapshot. Intermediate states can coalesce; this
		/// is a latest-state observation, not a durable transition log.
		pub async fn wait_for_change(self, previous: Snapshot) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(WAIT_FOR_CHANGE, &previous.encode()?)?.await?)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A satisfied Apple path over Wi-Fi plus a utun interface: the bytes
	/// `NetworkPlugin.swift` encodes, asserted in `apple-tests` too.
	const APPLE_WIFI_AND_OTHER: [u8; SNAPSHOT_BYTES] = [
		2, 0, 0, 0, // Wi-Fi
		2, 0, 0, 0, // connected
		0, 0, 0, 0, // internet validation unknown
		1, 0, 0, 0, // not expensive
		1, 0, 0, 0, // not constrained
		255, 255, 255, 255, // no throughput estimate
		255, 255, 255, 255, // no latency estimate
		0, 0, 0, 0, // no effective class
		0x41, 0, 0, 0, // Wi-Fi and other
	];

	/// An Android VPN over Wi-Fi, validated and unmetered: the bytes
	/// `NetworkPlugin.kt` encodes, asserted in its instrumented test too.
	const ANDROID_VPN_OVER_WIFI: [u8; SNAPSHOT_BYTES] = [
		6, 0, 0, 0, // VPN wins as the primary connection
		2, 0, 0, 0, // connected
		2, 0, 0, 0, // validated
		1, 0, 0, 0, // not metered
		0, 0, 0, 0, // data saver unknown
		255, 255, 255, 255, // no throughput estimate
		255, 255, 255, 255, // no latency estimate
		0, 0, 0, 0, // no effective class
		0x11, 0, 0, 0, // Wi-Fi and VPN
	];

	#[test]
	fn binary_snapshot_preserves_unknowns_and_rejects_invalid_fields() {
		// Wifi, connected, and an unknown validation flag, little endian.
		const WIFI_PREFIX: [u8; 12] = [2, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0];

		let reading = Snapshot::connection(Connection::Wifi, Some(true));
		let bytes = reading.encode().unwrap();
		assert_eq!(bytes.len(), SNAPSHOT_BYTES);
		assert_eq!(bytes[..WIFI_PREFIX.len()], WIFI_PREFIX);
		assert_eq!(bytes[32..], [1, 0, 0, 0], "Wi-Fi is its own transport");
		assert_eq!(Snapshot::decode(&bytes), Ok(reading));

		for field in [0, 4, 8, 12, 16, 28, 32] {
			let mut invalid = bytes.to_vec();
			invalid[field] = 0xff;
			assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
		}

		assert_eq!(
			Snapshot::decode(&bytes[..SNAPSHOT_BYTES - 1]),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Snapshot::decode(&bytes[..32]),
			Err(Error::InvalidArgument),
			"the eight-field snapshot of earlier releases is refused"
		);
		let mut trailing = bytes.to_vec();
		trailing.push(0);
		assert_eq!(Snapshot::decode(&trailing), Err(Error::InvalidArgument));
		let mut invalid = reading;
		invalid.downlink_kbps = Some(u32::MAX);
		assert_eq!(invalid.encode(), Err(Error::InvalidArgument));
	}

	#[test]
	fn connected_does_not_imply_internet_validated() {
		let reading = Snapshot::connection(Connection::Ethernet, Some(true));

		assert_eq!(reading.internet_validated, None);
	}

	#[test]
	fn every_connection_flag_and_estimate_round_trips() {
		let connections = [
			Connection::Unknown,
			Connection::None,
			Connection::Wifi,
			Connection::Cellular,
			Connection::Ethernet,
			Connection::Bluetooth,
			Connection::Vpn,
			Connection::Other,
		];
		let estimates = [
			EffectiveType::Unknown,
			EffectiveType::Slow2g,
			EffectiveType::TwoG,
			EffectiveType::ThreeG,
			EffectiveType::FourG,
		];
		/// An estimate the browser can report, in kilobits per second.
		const DOWNLINK_KBPS: u32 = 10_000;
		/// An estimate the browser can report, in milliseconds.
		const ROUND_TRIP_MS: u32 = 40;

		for (index, connection) in connections.into_iter().enumerate() {
			let reading = Snapshot {
				connected: [None, Some(false), Some(true)][index % 3],
				internet_validated: [Some(true), None, Some(false)][index % 3],
				expensive: [Some(false), Some(true), None][index % 3],
				data_saver: Some(index % 2 == 0),
				downlink_kbps: (index % 2 == 0).then_some(DOWNLINK_KBPS),
				round_trip_ms: (index % 2 == 1).then_some(ROUND_TRIP_MS),
				effective_type: estimates[index % estimates.len()],
				..Snapshot::connection(connection, None)
			};
			assert_eq!(Snapshot::decode(&reading.encode().unwrap()), Ok(reading));
		}
	}

	#[test]
	fn every_transport_set_round_trips_and_unknown_bits_are_refused() {
		for bits in 0..=Transports::MASK {
			let transports = Transports::from_bits(bits).unwrap();
			let reading = Snapshot {
				transports,
				..Snapshot::connection(Connection::Unknown, Some(true))
			};
			let bytes = reading.encode().unwrap();
			assert_eq!(bytes[32..], bits.to_le_bytes());
			assert_eq!(Snapshot::decode(&bytes), Ok(reading));
			assert_eq!(transports.iter().collect::<Transports>(), transports);
		}

		for bits in [1 << 7, 1 << 31, u32::MAX] {
			assert_eq!(Transports::from_bits(bits), None);
			let mut bytes = Snapshot::connection(Connection::Unknown, None)
				.encode()
				.unwrap()
				.to_vec();
			bytes[32..].copy_from_slice(&bits.to_le_bytes());
			assert_eq!(Snapshot::decode(&bytes), Err(Error::InvalidArgument));
		}
	}

	#[test]
	fn transports_must_agree_with_the_primary_connection() {
		let offline = Snapshot {
			transports: Transport::Wifi.into(),
			..Snapshot::connection(Connection::None, Some(false))
		};
		assert_eq!(offline.encode(), Err(Error::InvalidArgument));

		let missing_primary = Snapshot {
			transports: Transport::Cellular.into(),
			..Snapshot::connection(Connection::Wifi, Some(true))
		};
		assert_eq!(missing_primary.encode(), Err(Error::InvalidArgument));

		let mut bytes = Snapshot::connection(Connection::None, Some(false))
			.encode()
			.unwrap()
			.to_vec();
		assert_eq!(bytes[32..], [0, 0, 0, 0]);
		bytes[32] = 1;
		assert_eq!(Snapshot::decode(&bytes), Err(Error::InvalidArgument));

		let unknown = Snapshot::connection(Connection::Unknown, Some(true));
		assert!(unknown.transports.is_empty());
		assert_eq!(Snapshot::decode(&unknown.encode().unwrap()), Ok(unknown));
	}

	#[test]
	fn transport_sets_list_members_in_bit_order() {
		let set = Transports::of([Transport::Other, Transport::Wifi, Transport::Vpn]);
		assert_eq!(set.bits(), 0b101_0001);
		assert_eq!(
			set.iter().collect::<Vec<_>>(),
			[Transport::Wifi, Transport::Vpn, Transport::Other]
		);
		assert!(set.contains(Transport::Vpn));
		assert!(!set.contains(Transport::Satellite));
		assert!(Transports::EMPTY.is_empty());
		assert_eq!(Transports::default(), Transports::EMPTY);

		for (index, transport) in Transport::ALL.into_iter().enumerate() {
			assert_eq!(Transports::from(transport).bits(), 1 << index);
		}
	}

	#[test]
	fn browser_connection_types_map_to_agreeing_readings() {
		use browser_type::connection_from_type;
		use browser_type::transports_from_type;

		for (kind, connection, transports) in [
			(
				Some("wifi"),
				Connection::Wifi,
				Transports::from(Transport::Wifi),
			),
			(
				Some("cellular"),
				Connection::Cellular,
				Transport::Cellular.into(),
			),
			(
				Some("ethernet"),
				Connection::Ethernet,
				Transport::Ethernet.into(),
			),
			(
				Some("bluetooth"),
				Connection::Bluetooth,
				Transport::Bluetooth.into(),
			),
			(Some("other"), Connection::Other, Transport::Other.into()),
			(Some("mixed"), Connection::Unknown, Transport::Other.into()),
			(Some("wimax"), Connection::Unknown, Transport::Other.into()),
			(Some("none"), Connection::Unknown, Transports::EMPTY),
			(Some("unknown"), Connection::Unknown, Transports::EMPTY),
			(Some("satellite"), Connection::Unknown, Transports::EMPTY),
			(None, Connection::Unknown, Transports::EMPTY),
		] {
			assert_eq!(connection_from_type(kind), connection, "{kind:?}");
			assert_eq!(transports_from_type(kind), transports, "{kind:?}");
			let reading = Snapshot {
				transports,
				..Snapshot::connection(connection, Some(true))
			};
			assert_eq!(
				Snapshot::decode(&reading.encode().unwrap()),
				Ok(reading),
				"{kind:?}"
			);
		}
	}

	#[test]
	fn native_golden_snapshots_decode_to_their_transport_sets() {
		let apple = Snapshot::decode(&APPLE_WIFI_AND_OTHER).unwrap();
		assert_eq!(apple.connection, Connection::Wifi);
		assert_eq!(apple.connected, Some(true));
		assert_eq!(apple.expensive, Some(false));
		assert_eq!(apple.data_saver, Some(false));
		assert_eq!(
			apple.transports,
			Transports::of([Transport::Wifi, Transport::Other])
		);
		assert_eq!(apple.encode().unwrap()[..], APPLE_WIFI_AND_OTHER);

		let android = Snapshot::decode(&ANDROID_VPN_OVER_WIFI).unwrap();
		assert_eq!(android.connection, Connection::Vpn);
		assert_eq!(android.internet_validated, Some(true));
		assert_eq!(
			android.transports.iter().collect::<Vec<_>>(),
			[Transport::Wifi, Transport::Vpn]
		);
		assert_eq!(android.encode().unwrap()[..], ANDROID_VPN_OVER_WIFI);
	}

	#[cfg(any(
		target_os = "macos",
		target_os = "windows",
		all(target_os = "linux", not(target_env = "ohos"))
	))]
	#[test]
	fn desktop_adapter_reports_a_valid_snapshot_and_rejects_bad_calls() {
		use leptoskit_bridge::Plugin;
		const UNKNOWN_METHOD: MethodId = MethodId(9);
		let mut network = desktop::Network;
		assert_eq!(network.name(), NAME);
		assert_eq!(network.methods(), &[SNAPSHOT]);
		let reading = Snapshot::decode(&network.invoke(SNAPSHOT, Bytes::new()).unwrap()).unwrap();
		assert_eq!(reading, desktop::snapshot().unwrap());
		assert_eq!(
			reading.internet_validated, None,
			"desktops never validate reachability"
		);
		assert_eq!(
			network.invoke(SNAPSHOT, Bytes::from_static(&[0])),
			Err(Error::InvalidArgument)
		);

		assert_eq!(
			network.invoke(UNKNOWN_METHOD, Bytes::new()),
			Err(Error::Unavailable)
		);
	}
}
