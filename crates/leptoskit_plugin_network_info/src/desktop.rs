//! Desktop interface addresses for the interface that carries the IPv4 default
//! route, read with `if-addrs` and the operating system's routing table:
//! `/proc/net/route` on Linux, a `PF_ROUTE` sysctl on macOS, and
//! `GetIpForwardTable2` on Windows. A wired interface qualifies as readily as
//! a Wi-Fi one; nothing is inferred from interface names.

use std::net::Ipv4Addr;

use if_addrs::IfAddr;
use if_addrs::Interface;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

use crate::NAME;
use crate::SNAPSHOT;
use crate::Snapshot;

/// The IPv4 default route: its gateway and the interface it leaves through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefaultRoute {
	/// Interface index, when the routing table names one.
	pub index: Option<u32>,
	/// Interface name, when the routing table names one instead.
	pub name: Option<String>,
	/// The next hop.
	pub gateway: Ipv4Addr,
}

impl DefaultRoute {
	fn carries(&self, interface: &Interface) -> bool {
		match (self.index, &self.name) {
			(Some(index), _) => interface.index == Some(index),
			(None, Some(name)) => interface.name == *name,
			(None, None) => false,
		}
	}
}

/// Reads the current snapshot. Without a default route every field is `None`.
pub fn snapshot() -> Result<Snapshot, Error> {
	let interfaces = if_addrs::get_if_addrs().map_err(|_| Error::Native)?;
	Ok(assemble(&interfaces, default_route().as_ref()))
}

/// Picks the default-route interface's addresses, preferring routable ones
/// over link-local ones.
pub(crate) fn assemble(interfaces: &[Interface], route: Option<&DefaultRoute>) -> Snapshot {
	let Some(route) = route else {
		return Snapshot::default();
	};
	let carried = || {
		interfaces
			.iter()
			.filter(|interface| route.carries(interface) && !interface.is_loopback())
	};
	let v4 = carried()
		.filter_map(|interface| {
			match &interface.addr {
				IfAddr::V4(address) => Some(address),
				IfAddr::V6(_) => None,
			}
		})
		.min_by_key(|address| address.ip.is_link_local());
	let ipv6 = carried()
		.filter_map(|interface| {
			match &interface.addr {
				IfAddr::V6(address) => Some(address.ip),
				IfAddr::V4(_) => None,
			}
		})
		.min_by_key(std::net::Ipv6Addr::is_unicast_link_local);

	Snapshot {
		ssid: None,
		bssid: None,
		ipv4: v4.map(|address| address.ip),
		ipv6,
		subnet_mask: v4.map(|address| address.netmask),
		broadcast: v4.and_then(|address| address.broadcast),
		gateway: Some(route.gateway),
	}
	.sanitized()
}

/// Synchronous host adapter for the snapshot method.
#[derive(Debug, Default)]
pub struct NetworkInfo;

impl leptoskit_bridge::Plugin for NetworkInfo {
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

/// `RTF_UP | RTF_GATEWAY` on Linux and the BSDs.
const UP_GATEWAY: u32 = 0x1 | 0x2;

/// Reads the lowest-metric default route from `/proc/net/route` text.
/// Addresses there are hexadecimal words in the kernel's byte order.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn parse_proc_route(text: &str) -> Option<DefaultRoute> {
	text.lines()
		.skip(1)
		.filter_map(|line| {
			let columns: Vec<&str> = line.split_whitespace().collect();
			let [name, destination, gateway, flags, _, _, metric, mask, ..] = columns[..] else {
				return None;
			};
			let hex = |value: &str| u32::from_str_radix(value, 16).ok();
			let default = hex(destination)? == 0 && hex(mask)? == 0;
			let up_gateway = hex(flags)? & UP_GATEWAY == UP_GATEWAY;
			let gateway = Ipv4Addr::from(hex(gateway)?.to_ne_bytes());

			(default && up_gateway && !gateway.is_unspecified())
				.then(|| (metric.parse::<u32>().ok(), name, gateway))
		})
		.min_by_key(|(metric, ..)| metric.unwrap_or(u32::MAX))
		.map(|(_, name, gateway)| {
			DefaultRoute {
				index: None,
				name: Some(name.to_owned()),
				gateway,
			}
		})
}

#[cfg(target_os = "linux")]
fn default_route() -> Option<DefaultRoute> {
	parse_proc_route(&std::fs::read_to_string("/proc/net/route").ok()?)
}

/// Darwin's `PF_ROUTE` message layout, read by offset so tests can build one.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod darwin {
	/// `sizeof(struct rt_msghdr)`.
	pub const HEADER_BYTES: usize = 92;
	/// `RTF_IFSCOPE`: a route bound to one interface, not the primary default.
	pub const IFSCOPE: u32 = 0x0100_0000;
	/// `RTA_DST`, `RTA_GATEWAY` and `RTA_NETMASK`, the first three addresses.
	pub const DESTINATION: usize = 0;
	pub const GATEWAY: usize = 1;
	pub const NETMASK: usize = 2;
	/// `RTAX_MAX`.
	pub const ADDRESS_SLOTS: usize = 8;
	/// `AF_INET` on Darwin.
	pub const INET: u8 = 2;
}

/// Reads the primary IPv4 default route from a `NET_RT_FLAGS` sysctl dump:
/// routes up, through a gateway, to 0.0.0.0/0, not scoped to one interface.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn parse_route_dump(dump: &[u8]) -> Option<DefaultRoute> {
	let mut rest = dump;

	while rest.len() >= darwin::HEADER_BYTES {
		let length = usize::from(u16::from_ne_bytes([rest[0], rest[1]]));

		if length < darwin::HEADER_BYTES || length > rest.len() {
			return None;
		}

		let (message, next) = rest.split_at(length);
		rest = next;

		if let Some(route) = route_message(message) {
			return Some(route);
		}
	}

	None
}

/// Decodes one routing message when it is the primary default route.
fn route_message(message: &[u8]) -> Option<DefaultRoute> {
	let word = |at: usize| {
		u32::from_ne_bytes([
			message[at],
			message[at + 1],
			message[at + 2],
			message[at + 3],
		])
	};
	let index = u16::from_ne_bytes([message[4], message[5]]);
	let flags = word(8);
	let addresses = word(12);

	if flags & UP_GATEWAY != UP_GATEWAY || flags & darwin::IFSCOPE != 0 {
		return None;
	}

	let mut slots: [Option<&[u8]>; darwin::ADDRESS_SLOTS] = [None; darwin::ADDRESS_SLOTS];
	let mut at = darwin::HEADER_BYTES;

	for (slot, entry) in slots.iter_mut().enumerate() {
		if addresses & (1 << slot) == 0 {
			continue;
		}

		let length = usize::from(*message.get(at)?);
		*entry = Some(message.get(at..at + length.max(1))?);
		// Darwin pads each address to four bytes; an empty one still takes
		// four.
		at += if length == 0 {
			4
		} else {
			length.next_multiple_of(4)
		};
	}

	let inet = |sockaddr: &[u8]| -> Option<Ipv4Addr> {
		(sockaddr.len() >= 8 && sockaddr[1] == darwin::INET)
			.then(|| Ipv4Addr::new(sockaddr[4], sockaddr[5], sockaddr[6], sockaddr[7]))
	};
	let destination = inet(slots[darwin::DESTINATION]?)?;
	let gateway = inet(slots[darwin::GATEWAY]?)?;
	// A default route's mask is absent, empty, or all zero.
	let mask_is_zero =
		slots[darwin::NETMASK].is_none_or(|mask| mask.iter().skip(4).all(|byte| *byte == 0));

	(destination.is_unspecified() && mask_is_zero && !gateway.is_unspecified()).then_some(
		DefaultRoute {
			index: Some(u32::from(index)),
			name: None,
			gateway,
		},
	)
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn default_route() -> Option<DefaultRoute> {
	let mut mib = [
		libc::CTL_NET,
		libc::PF_ROUTE,
		0,
		libc::AF_INET,
		libc::NET_RT_FLAGS,
		libc::RTF_GATEWAY,
	];

	// The table can grow between sizing and reading, so retry a few times.
	for _ in 0..3 {
		let mut length: libc::size_t = 0;
		// SAFETY: `mib` is a valid six-entry name; a null buffer asks only for
		// the size, written to `length`.
		let sized = unsafe {
			libc::sysctl(
				mib.as_mut_ptr(),
				mib.len() as libc::c_uint,
				std::ptr::null_mut(),
				&raw mut length,
				std::ptr::null_mut(),
				0,
			)
		};

		if sized != 0 {
			return None;
		}

		let mut buffer = vec![0u8; length];
		// SAFETY: `buffer` holds `length` writable bytes, and the kernel
		// writes at most `length` and stores the count it wrote.
		let read = unsafe {
			libc::sysctl(
				mib.as_mut_ptr(),
				mib.len() as libc::c_uint,
				buffer.as_mut_ptr().cast(),
				&raw mut length,
				std::ptr::null_mut(),
				0,
			)
		};

		if read == 0 {
			buffer.truncate(length);
			return parse_route_dump(&buffer);
		}

		if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOMEM) {
			return None;
		}
	}

	None
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn default_route() -> Option<DefaultRoute> {
	use windows_sys::Win32::NetworkManagement::IpHelper::FreeMibTable;
	use windows_sys::Win32::NetworkManagement::IpHelper::GetIpForwardTable2;
	use windows_sys::Win32::NetworkManagement::IpHelper::MIB_IPFORWARD_TABLE2;
	use windows_sys::Win32::Networking::WinSock::AF_INET;

	let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
	// SAFETY: `table` is a valid out-pointer; on success the system allocates
	// the table, which `FreeMibTable` releases below.
	if unsafe { GetIpForwardTable2(AF_INET, &raw mut table) } != 0 || table.is_null() {
		return None;
	}

	// SAFETY: on success `table` points at `NumEntries` contiguous rows.
	let rows = unsafe {
		std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
	};
	let route = rows
		.iter()
		.filter_map(|row| {
			// SAFETY: the table was requested for AF_INET, so every address
			// union holds an IPv4 socket address.
			let (family, gateway) =
				unsafe { (row.NextHop.si_family, row.NextHop.Ipv4.sin_addr.S_un.S_addr) };
			let gateway = Ipv4Addr::from(gateway.to_ne_bytes());
			let default = row.DestinationPrefix.PrefixLength == 0;

			(default && family == AF_INET && !gateway.is_unspecified()).then_some((
				row.Metric,
				row.InterfaceIndex,
				gateway,
			))
		})
		.min_by_key(|(metric, ..)| *metric)
		.map(|(_, index, gateway)| {
			DefaultRoute {
				index: Some(index),
				name: None,
				gateway,
			}
		});

	// SAFETY: `table` came from `GetIpForwardTable2` and is freed once.
	unsafe { FreeMibTable(table.cast()) };
	route
}
