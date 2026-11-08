//! <!-- {=p_networkInfoSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_network_info` as a direct dependency and select the
//! `network-info` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Guests enable the `guest` feature and reuse
//! `guest::NetworkInfo`. One snapshot reports the Wi-Fi name (SSID, as raw
//! bytes), BSSID, IPv4 and IPv6 addresses, subnet mask, broadcast address and
//! gateway, as Flutter's `network_info_plus` does. Each field is `None` when
//! the platform withholds it; the plugin never returns placeholders and
//! never asks for permission. The SSID and BSSID need location authorization,
//! which the `location` plugin requests, and on iOS the Access Wi-Fi
//! Information entitlement too. Browsers return `Unavailable`.
//!
//! <!-- {/p_networkInfoSummary} -->
//!
//! The wire snapshot is 80 bytes: a little-endian u32 presence bitset, a
//! little-endian u32 SSID length, 32 SSID bytes, six BSSID bytes, two reserved
//! zero bytes, then the IPv4 address, IPv6 address, IPv4 subnet mask, IPv4
//! broadcast address and IPv4 gateway in network byte order. Absent fields are
//! all zero. Decoding refuses unknown presence bits, nonzero padding or absent
//! fields, out-of-range SSID lengths, and the placeholders platforms use for
//! withheld values (`02:00:00:00:00:00`, an unspecified address).

use std::borrow::Cow;
use std::fmt;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

/// Registered plugin name.
pub const NAME: &str = "network-info";
/// Empty payload -> current [`Snapshot`].
pub const SNAPSHOT: MethodId = MethodId(0);

/// Encoded snapshot size.
const SNAPSHOT_BYTES: usize = 80;

/// Field offsets inside the encoded snapshot.
mod offset {
	pub const PRESENCE: usize = 0;
	pub const SSID_LEN: usize = 4;
	pub const SSID: usize = 8;
	pub const BSSID: usize = 40;
	pub const RESERVED: usize = 46;
	pub const IPV4: usize = 48;
	pub const IPV6: usize = 52;
	pub const SUBNET_MASK: usize = 68;
	pub const BROADCAST: usize = 72;
	pub const GATEWAY: usize = 76;
}

/// Presence bits, one per optional field.
mod present {
	pub const SSID: u32 = 1 << 0;
	pub const BSSID: u32 = 1 << 1;
	pub const IPV4: u32 = 1 << 2;
	pub const IPV6: u32 = 1 << 3;
	pub const SUBNET_MASK: u32 = 1 << 4;
	pub const BROADCAST: u32 = 1 << 5;
	pub const GATEWAY: u32 = 1 << 6;
	/// Every bit the wire defines.
	pub const MASK: u32 = (1 << 7) - 1;
}

/// The BSSID Android reports when it withholds the real one.
const REDACTED_BSSID: [u8; 6] = [0x02, 0, 0, 0, 0, 0];

/// A Wi-Fi network name: 1 to 32 arbitrary bytes, as 802.11 defines it.
///
/// SSIDs are not necessarily UTF-8. [`Ssid::as_bytes`] returns exactly what
/// the platform reported; [`Ssid::to_str`] returns text only when the bytes
/// are valid UTF-8. iOS hands over an already-decoded string, so its bytes
/// are that string's UTF-8; macOS and Android report the raw bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ssid {
	len: u8,
	bytes: [u8; Ssid::MAX_BYTES],
}

impl Ssid {
	/// The longest SSID 802.11 allows.
	pub const MAX_BYTES: usize = 32;

	/// Wraps raw SSID bytes, refusing an empty or over-long name.
	pub fn new(bytes: &[u8]) -> Result<Self, Error> {
		if bytes.is_empty() || bytes.len() > Self::MAX_BYTES {
			return Err(Error::InvalidArgument);
		}

		let mut ssid = Self {
			len: bytes.len() as u8,
			bytes: [0; Self::MAX_BYTES],
		};
		ssid.bytes[..bytes.len()].copy_from_slice(bytes);
		Ok(ssid)
	}

	/// The raw bytes the platform reported.
	pub fn as_bytes(&self) -> &[u8] {
		&self.bytes[..usize::from(self.len)]
	}

	/// The name as text, when its bytes are valid UTF-8.
	pub fn to_str(&self) -> Option<&str> {
		std::str::from_utf8(self.as_bytes()).ok()
	}

	/// The name as text, replacing invalid UTF-8 with U+FFFD.
	pub fn to_string_lossy(&self) -> Cow<'_, str> {
		String::from_utf8_lossy(self.as_bytes())
	}
}

impl fmt::Debug for Ssid {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self.to_str() {
			Some(text) => formatter.debug_tuple("Ssid").field(&text).finish(),
			None => {
				formatter
					.debug_tuple("Ssid")
					.field(&self.as_bytes())
					.finish()
			}
		}
	}
}

impl fmt::Display for Ssid {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.to_string_lossy())
	}
}

/// The access point's MAC address. Displays as lowercase `aa:bb:cc:dd:ee:ff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Bssid([u8; 6]);

impl Bssid {
	/// Wraps a BSSID, refusing the all-zero address, group addresses, and
	/// Android's `02:00:00:00:00:00` redaction placeholder.
	pub fn new(octets: [u8; 6]) -> Result<Self, Error> {
		let group = octets[0] & 1 == 1;

		if octets == [0; 6] || octets == REDACTED_BSSID || group {
			return Err(Error::InvalidArgument);
		}

		Ok(Self(octets))
	}

	/// The six address bytes.
	pub const fn octets(self) -> [u8; 6] {
		self.0
	}
}

impl fmt::Display for Bssid {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		for (index, octet) in self.0.iter().enumerate() {
			let separator = if index == 0 { "" } else { ":" };
			write!(formatter, "{separator}{octet:02x}")?;
		}

		Ok(())
	}
}

/// What the platform reports about the Wi-Fi network (on desktop Rust, the
/// interface carrying the IPv4 default route). Every field is `None` when the
/// platform withholds it or has no such value; nothing is ever a placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Snapshot {
	/// The Wi-Fi network name. Needs location authorization on every
	/// platform that reports it, and the Access Wi-Fi Information
	/// entitlement on iOS.
	pub ssid: Option<Ssid>,
	/// The access point's MAC address, under the same conditions as the SSID.
	pub bssid: Option<Bssid>,
	/// The interface's IPv4 address.
	pub ipv4: Option<Ipv4Addr>,
	/// The interface's IPv6 address, preferring a global address over a
	/// link-local one. Link-local addresses lose their scope on the wire.
	pub ipv6: Option<Ipv6Addr>,
	/// The IPv4 subnet mask, always contiguous.
	pub subnet_mask: Option<Ipv4Addr>,
	/// The IPv4 broadcast address the interface reports.
	pub broadcast: Option<Ipv4Addr>,
	/// The IPv4 default gateway reached through this interface.
	pub gateway: Option<Ipv4Addr>,
}

/// Whether `mask` is a usable, contiguous, non-empty IPv4 subnet mask.
fn valid_mask(mask: Ipv4Addr) -> bool {
	let bits = mask.to_bits();
	bits != 0 && bits.leading_ones() + bits.trailing_zeros() == 32
}

/// Whether an IPv4 address the wire carries is a real address.
fn valid_v4(address: Ipv4Addr) -> bool {
	!address.is_unspecified()
}

impl Snapshot {
	/// Drops addresses the wire would refuse, so a reading assembled from OS
	/// data never fails to encode.
	#[must_use]
	pub fn sanitized(self) -> Self {
		Self {
			ipv4: self.ipv4.filter(|address| valid_v4(*address)),
			ipv6: self.ipv6.filter(|address| !address.is_unspecified()),
			subnet_mask: self.subnet_mask.filter(|mask| valid_mask(*mask)),
			broadcast: self.broadcast.filter(|address| valid_v4(*address)),
			gateway: self.gateway.filter(|address| valid_v4(*address)),
			..self
		}
	}

	/// Encodes a snapshot, refusing unspecified addresses and a
	/// non-contiguous subnet mask.
	pub fn encode(self) -> Result<Bytes, Error> {
		if self.sanitized() != self {
			return Err(Error::InvalidArgument);
		}

		let mut output = [0u8; SNAPSHOT_BYTES];
		let mut presence = 0;
		let mut put = |bit: u32, at: usize, bytes: Option<&[u8]>| {
			if let Some(bytes) = bytes {
				presence |= bit;
				output[at..at + bytes.len()].copy_from_slice(bytes);
			}
		};

		put(
			present::SSID,
			offset::SSID,
			self.ssid.as_ref().map(Ssid::as_bytes),
		);
		put(
			present::BSSID,
			offset::BSSID,
			self.bssid
				.map(Bssid::octets)
				.as_ref()
				.map(|octets| &octets[..]),
		);
		put(
			present::IPV4,
			offset::IPV4,
			self.ipv4
				.map(|address| address.octets())
				.as_ref()
				.map(|octets| &octets[..]),
		);
		put(
			present::IPV6,
			offset::IPV6,
			self.ipv6
				.map(|address| address.octets())
				.as_ref()
				.map(|octets| &octets[..]),
		);
		put(
			present::SUBNET_MASK,
			offset::SUBNET_MASK,
			self.subnet_mask
				.map(|address| address.octets())
				.as_ref()
				.map(|octets| &octets[..]),
		);
		put(
			present::BROADCAST,
			offset::BROADCAST,
			self.broadcast
				.map(|address| address.octets())
				.as_ref()
				.map(|octets| &octets[..]),
		);
		put(
			present::GATEWAY,
			offset::GATEWAY,
			self.gateway
				.map(|address| address.octets())
				.as_ref()
				.map(|octets| &octets[..]),
		);

		let ssid_len = self.ssid.map_or(0, |ssid| u32::from(ssid.len));
		output[offset::PRESENCE..offset::SSID_LEN].copy_from_slice(&presence.to_le_bytes());
		output[offset::SSID_LEN..offset::SSID].copy_from_slice(&ssid_len.to_le_bytes());
		Ok(Bytes::copy_from_slice(&output))
	}

	/// Rejects truncated or trailing data, unknown presence bits, nonzero
	/// padding or absent fields, bad SSID lengths, placeholder BSSIDs,
	/// unspecified addresses, and non-contiguous subnet masks.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let bytes: &[u8; SNAPSHOT_BYTES] = bytes.try_into().map_err(|_| Error::InvalidArgument)?;
		let word = |at: usize| {
			u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
		};
		let presence = word(offset::PRESENCE);

		if presence & !present::MASK != 0 || bytes[offset::RESERVED..offset::IPV4] != [0, 0] {
			return Err(Error::InvalidArgument);
		}

		// Reads a field, which must be all zero when its presence bit is clear.
		let field = |bit: u32, range: std::ops::Range<usize>| -> Result<Option<&[u8]>, Error> {
			let slice = &bytes[range];

			if presence & bit != 0 {
				Ok(Some(slice))
			} else if slice.iter().all(|byte| *byte == 0) {
				Ok(None)
			} else {
				Err(Error::InvalidArgument)
			}
		};
		let v4 = |bit: u32, at: usize| -> Result<Option<Ipv4Addr>, Error> {
			Ok(field(bit, at..at + 4)?
				.map(|octets| Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3])))
		};

		let ssid_len = word(offset::SSID_LEN) as usize;
		let ssid = if presence & present::SSID == 0 {
			field(present::SSID, offset::SSID_LEN..offset::BSSID)?;
			None
		} else {
			let name = bytes[offset::SSID..offset::BSSID]
				.get(..ssid_len)
				.ok_or(Error::InvalidArgument)?;

			if bytes[offset::SSID + ssid_len..offset::BSSID]
				.iter()
				.any(|byte| *byte != 0)
			{
				return Err(Error::InvalidArgument);
			}

			Some(Ssid::new(name)?)
		};
		let bssid = field(present::BSSID, offset::BSSID..offset::RESERVED)?
			.map(|octets| Bssid::new(octets.try_into().map_err(|_| Error::InvalidArgument)?))
			.transpose()?;
		let ipv6 = field(present::IPV6, offset::IPV6..offset::SUBNET_MASK)?
			.map(|octets| -> Result<Ipv6Addr, Error> {
				let octets: [u8; 16] = octets.try_into().map_err(|_| Error::InvalidArgument)?;
				Ok(Ipv6Addr::from(octets))
			})
			.transpose()?;

		let snapshot = Self {
			ssid,
			bssid,
			ipv4: v4(present::IPV4, offset::IPV4)?,
			ipv6,
			subnet_mask: v4(present::SUBNET_MASK, offset::SUBNET_MASK)?,
			broadcast: v4(present::BROADCAST, offset::BROADCAST)?,
			gateway: v4(present::GATEWAY, offset::GATEWAY)?,
		};

		if snapshot.sanitized() != snapshot {
			return Err(Error::InvalidArgument);
		}

		Ok(snapshot)
	}
}

/// Desktop interface addresses for the interface that carries the IPv4
/// default route. The SSID and BSSID stay `None`: the macOS Swift plugin reads
/// them through `CoreWLAN`, and Windows and Linux have no dependency-free API
/// for them.
#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub mod desktop;

/// Browser builds: no browser API reveals Wi-Fi or interface details.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always `Unavailable`: browsers expose no network details to pages.
	pub fn snapshot() -> Result<Snapshot, Error> {
		Err(Error::Unavailable)
	}
}

/// Cached guest client using the binary bridge.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolved network-info plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct NetworkInfo(leptoskit_bridge::guest::Client);

	impl NetworkInfo {
		/// Resolves an installed host plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads the current Wi-Fi name, BSSID and interface addresses. It
		/// never asks for permission; fields the platform withholds are
		/// `None`.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}
	}
}

#[cfg(test)]
mod tests;
