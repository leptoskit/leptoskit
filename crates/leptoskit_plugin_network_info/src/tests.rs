use std::net::Ipv4Addr;
use std::net::Ipv6Addr;

use super::*;

/// A fully populated snapshot, as hex. The Swift and Kotlin suites assert the
/// same string, so all three encoders agree byte for byte.
const GOLDEN_HEX: &str = concat!(
	"7f000000", // every field present
	"04000000", // a four-byte SSID
	"686f6d65", // "home"
	"00000000000000000000000000000000000000000000000000000000",
	"123456789abc", // BSSID
	"0000",         // reserved
	"c0a80114",     // 192.168.1.20
	"20010db8000000000000000000000001",
	"ffffff00", // 255.255.255.0
	"c0a801ff", // 192.168.1.255
	"c0a80101", // 192.168.1.1
);

fn unhex(text: &str) -> Vec<u8> {
	(0..text.len())
		.step_by(2)
		.map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
		.collect()
}

fn golden() -> Snapshot {
	Snapshot {
		ssid: Some(Ssid::new(b"home").unwrap()),
		bssid: Some(Bssid::new([0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc]).unwrap()),
		ipv4: Some(Ipv4Addr::new(192, 168, 1, 20)),
		ipv6: Some("2001:db8::1".parse().unwrap()),
		subnet_mask: Some(Ipv4Addr::new(255, 255, 255, 0)),
		broadcast: Some(Ipv4Addr::new(192, 168, 1, 255)),
		gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
	}
}

#[test]
fn a_full_snapshot_matches_the_shared_golden_bytes() {
	let bytes = golden().encode().unwrap();
	assert_eq!(bytes.len(), SNAPSHOT_BYTES);
	assert_eq!(bytes[..], unhex(GOLDEN_HEX)[..]);
	assert_eq!(Snapshot::decode(&bytes), Ok(golden()));
}

#[test]
fn an_empty_snapshot_is_all_zero() {
	let bytes = Snapshot::default().encode().unwrap();
	assert_eq!(bytes[..], [0; SNAPSHOT_BYTES]);
	assert_eq!(Snapshot::decode(&bytes), Ok(Snapshot::default()));
}

#[test]
fn each_field_round_trips_alone_with_only_its_presence_bit() {
	let full = golden();
	let alone = [
		(
			present::SSID,
			Snapshot {
				ssid: full.ssid,
				..Snapshot::default()
			},
		),
		(
			present::BSSID,
			Snapshot {
				bssid: full.bssid,
				..Snapshot::default()
			},
		),
		(
			present::IPV4,
			Snapshot {
				ipv4: full.ipv4,
				..Snapshot::default()
			},
		),
		(
			present::IPV6,
			Snapshot {
				ipv6: full.ipv6,
				..Snapshot::default()
			},
		),
		(
			present::SUBNET_MASK,
			Snapshot {
				subnet_mask: full.subnet_mask,
				..Snapshot::default()
			},
		),
		(
			present::BROADCAST,
			Snapshot {
				broadcast: full.broadcast,
				..Snapshot::default()
			},
		),
		(
			present::GATEWAY,
			Snapshot {
				gateway: full.gateway,
				..Snapshot::default()
			},
		),
	];

	for (bit, snapshot) in alone {
		let bytes = snapshot.encode().unwrap();
		assert_eq!(bytes[..4], bit.to_le_bytes(), "{snapshot:?}");
		assert_eq!(Snapshot::decode(&bytes), Ok(snapshot));
	}
}

#[test]
fn ssids_are_bounded_raw_bytes_that_need_not_be_utf8() {
	let raw = Ssid::new(&[0xff, 0x00, b'A']).unwrap();
	assert_eq!(raw.as_bytes(), [0xff, 0x00, b'A']);
	assert_eq!(raw.to_str(), None);
	assert_eq!(raw.to_string_lossy(), "\u{fffd}\0A");
	let snapshot = Snapshot {
		ssid: Some(raw),
		..Snapshot::default()
	};
	assert_eq!(Snapshot::decode(&snapshot.encode().unwrap()), Ok(snapshot));

	let longest = Ssid::new(&[b'x'; Ssid::MAX_BYTES]).unwrap();
	assert_eq!(longest.as_bytes().len(), 32);
	let snapshot = Snapshot {
		ssid: Some(longest),
		..Snapshot::default()
	};
	assert_eq!(Snapshot::decode(&snapshot.encode().unwrap()), Ok(snapshot));

	assert_eq!(Ssid::new(&[]), Err(Error::InvalidArgument));
	assert_eq!(Ssid::new(&[b'x'; 33]), Err(Error::InvalidArgument));
	let text = Ssid::new("café".as_bytes()).unwrap();
	assert_eq!(text.to_str(), Some("café"));
	assert_eq!(text.to_string(), "café");
	assert_eq!(format!("{text:?}"), "Ssid(\"café\")");
	assert_eq!(format!("{raw:?}"), "Ssid([255, 0, 65])");
}

#[test]
fn placeholder_and_group_bssids_are_refused() {
	for octets in [
		[0; 6],
		[0x02, 0, 0, 0, 0, 0],
		[0xff; 6],
		[0x01, 0x00, 0x5e, 0, 0, 1],
	] {
		assert_eq!(
			Bssid::new(octets),
			Err(Error::InvalidArgument),
			"{octets:?}"
		);
	}

	let bssid = Bssid::new([0xa0, 0xB1, 0x0c, 0, 0xff, 0x2e]).unwrap();
	assert_eq!(bssid.to_string(), "a0:b1:0c:00:ff:2e");
	assert_eq!(bssid.octets(), [0xa0, 0xb1, 0x0c, 0, 0xff, 0x2e]);
}

#[test]
fn encoding_refuses_values_the_wire_would_reject_and_sanitizing_drops_them() {
	let unusable = Snapshot {
		ipv4: Some(Ipv4Addr::UNSPECIFIED),
		ipv6: Some(Ipv6Addr::UNSPECIFIED),
		subnet_mask: Some(Ipv4Addr::new(255, 0, 255, 0)),
		broadcast: Some(Ipv4Addr::UNSPECIFIED),
		gateway: Some(Ipv4Addr::UNSPECIFIED),
		..golden()
	};
	assert_eq!(unusable.encode(), Err(Error::InvalidArgument));
	let kept = unusable.sanitized();
	assert_eq!(kept.ssid, golden().ssid);
	assert_eq!(
		(
			kept.ipv4,
			kept.ipv6,
			kept.subnet_mask,
			kept.broadcast,
			kept.gateway
		),
		(None, None, None, None, None)
	);

	for mask in [Ipv4Addr::UNSPECIFIED, Ipv4Addr::new(0, 255, 255, 255)] {
		let snapshot = Snapshot {
			subnet_mask: Some(mask),
			..Snapshot::default()
		};
		assert_eq!(snapshot.encode(), Err(Error::InvalidArgument));
	}

	for prefix in 1..=32 {
		let mask = Ipv4Addr::from_bits(u32::MAX << (32 - prefix));
		let snapshot = Snapshot {
			subnet_mask: Some(mask),
			..Snapshot::default()
		};
		assert_eq!(Snapshot::decode(&snapshot.encode().unwrap()), Ok(snapshot));
	}
}

#[test]
fn decoding_rejects_every_malformed_field() {
	let golden = unhex(GOLDEN_HEX);
	let edit = |change: &dyn Fn(&mut Vec<u8>)| {
		let mut bytes = golden.clone();
		change(&mut bytes);
		Snapshot::decode(&bytes)
	};

	assert_eq!(Snapshot::decode(&golden[..79]), Err(Error::InvalidArgument));
	assert_eq!(edit(&|bytes| bytes.push(0)), Err(Error::InvalidArgument));
	assert_eq!(
		edit(&|bytes| bytes[0] = 0xff),
		Err(Error::InvalidArgument),
		"unknown presence bit"
	);
	assert_eq!(
		edit(&|bytes| bytes[3] = 0x80),
		Err(Error::InvalidArgument),
		"high presence bit"
	);
	assert_eq!(
		edit(&|bytes| bytes[46] = 1),
		Err(Error::InvalidArgument),
		"reserved byte"
	);
	assert_eq!(
		edit(&|bytes| bytes[4] = 0),
		Err(Error::InvalidArgument),
		"empty SSID"
	);
	assert_eq!(
		edit(&|bytes| bytes[4] = 33),
		Err(Error::InvalidArgument),
		"over-long SSID"
	);
	assert_eq!(
		edit(&|bytes| bytes[5] = 1),
		Err(Error::InvalidArgument),
		"huge SSID length"
	);
	assert_eq!(
		edit(&|bytes| bytes[12] = b'!'),
		Err(Error::InvalidArgument),
		"bytes past the SSID"
	);
	assert_eq!(
		edit(&|bytes| bytes[40..46].copy_from_slice(&[2, 0, 0, 0, 0, 0])),
		Err(Error::InvalidArgument),
		"redacted BSSID"
	);
	assert_eq!(
		edit(&|bytes| bytes[40..46].fill(0)),
		Err(Error::InvalidArgument),
		"zero BSSID"
	);
	assert_eq!(
		edit(&|bytes| bytes[40] = 0x13),
		Err(Error::InvalidArgument),
		"group BSSID"
	);
	assert_eq!(
		edit(&|bytes| bytes[48..52].fill(0)),
		Err(Error::InvalidArgument),
		"unspecified IPv4"
	);
	assert_eq!(
		edit(&|bytes| bytes[52..68].fill(0)),
		Err(Error::InvalidArgument),
		"unspecified IPv6"
	);
	assert_eq!(
		edit(&|bytes| bytes[69] = 0),
		Err(Error::InvalidArgument),
		"non-contiguous mask"
	);
	assert_eq!(
		edit(&|bytes| bytes[72..76].fill(0)),
		Err(Error::InvalidArgument),
		"unspecified broadcast"
	);
	assert_eq!(
		edit(&|bytes| bytes[76..80].fill(0)),
		Err(Error::InvalidArgument),
		"unspecified gateway"
	);

	// Clearing a presence bit leaves that field's bytes behind, which is
	// refused.
	for bit in 0..7 {
		assert_eq!(
			edit(&|bytes| bytes[0] &= !(1 << bit)),
			Err(Error::InvalidArgument),
			"stale bytes for bit {bit}"
		);
	}
}

#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
mod desktop {
	use if_addrs::IfAddr;
	use if_addrs::IfOperStatus;
	use if_addrs::Ifv4Addr;
	use if_addrs::Ifv6Addr;
	use if_addrs::Interface;
	use leptoskit_bridge::Plugin;

	use super::*;
	use crate::desktop::DefaultRoute;
	use crate::desktop::assemble;
	use crate::desktop::parse_proc_route;
	use crate::desktop::parse_route_dump;

	const GATEWAY: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 1);

	fn interface(name: &str, index: u32, addr: IfAddr) -> Interface {
		Interface {
			name: name.into(),
			addr,
			index: Some(index),
			oper_status: IfOperStatus::Up,
			is_p2p: false,
			#[cfg(windows)]
			adapter_name: name.into(),
		}
	}

	fn v4(ip: [u8; 4], prefix: u8, broadcast: Option<[u8; 4]>) -> IfAddr {
		IfAddr::V4(Ifv4Addr {
			ip: ip.into(),
			netmask: Ipv4Addr::from_bits(u32::MAX << (32 - prefix)),
			prefixlen: prefix,
			broadcast: broadcast.map(Ipv4Addr::from),
		})
	}

	fn v6(ip: &str) -> IfAddr {
		IfAddr::V6(Ifv6Addr {
			ip: ip.parse().unwrap(),
			netmask: "ffff:ffff:ffff:ffff::".parse().unwrap(),
			prefixlen: 64,
			broadcast: None,
		})
	}

	fn interfaces() -> Vec<Interface> {
		vec![
			interface("lo0", 1, v4([127, 0, 0, 1], 8, None)),
			interface("en0", 4, v6("fe80::1")),
			interface(
				"en0",
				4,
				v4([169, 254, 3, 3], 16, Some([169, 254, 255, 255])),
			),
			interface(
				"en0",
				4,
				v4([192, 168, 1, 20], 24, Some([192, 168, 1, 255])),
			),
			interface("en0", 4, v6("2001:db8::20")),
			interface("en7", 9, v4([10, 0, 0, 5], 8, Some([10, 255, 255, 255]))),
		]
	}

	#[test]
	fn the_default_route_interface_supplies_routable_addresses() {
		let route = DefaultRoute {
			index: Some(4),
			name: None,
			gateway: GATEWAY,
		};
		let snapshot = assemble(&interfaces(), Some(&route));
		assert_eq!(
			snapshot,
			Snapshot {
				ipv4: Some(Ipv4Addr::new(192, 168, 1, 20)),
				ipv6: Some("2001:db8::20".parse().unwrap()),
				subnet_mask: Some(Ipv4Addr::new(255, 255, 255, 0)),
				broadcast: Some(Ipv4Addr::new(192, 168, 1, 255)),
				gateway: Some(GATEWAY),
				..Snapshot::default()
			}
		);
		assert_eq!(snapshot.ssid, None, "desktop Rust never reads an SSID");

		let by_name = DefaultRoute {
			index: None,
			name: Some("en7".into()),
			gateway: Ipv4Addr::new(10, 0, 0, 1),
		};
		let snapshot = assemble(&interfaces(), Some(&by_name));
		assert_eq!(snapshot.ipv4, Some(Ipv4Addr::new(10, 0, 0, 5)));
		assert_eq!(snapshot.ipv6, None);
		assert_eq!(snapshot.subnet_mask, Some(Ipv4Addr::new(255, 0, 0, 0)));
	}

	#[test]
	fn link_local_addresses_are_used_only_without_routable_ones() {
		let only_link_local = [
			interface("en0", 4, v4([169, 254, 3, 3], 16, None)),
			interface("en0", 4, v6("fe80::1")),
		];
		let route = DefaultRoute {
			index: Some(4),
			name: None,
			gateway: GATEWAY,
		};
		let snapshot = assemble(&only_link_local, Some(&route));
		assert_eq!(snapshot.ipv4, Some(Ipv4Addr::new(169, 254, 3, 3)));
		assert_eq!(snapshot.ipv6, Some("fe80::1".parse().unwrap()));
		assert_eq!(snapshot.broadcast, None, "no broadcast was reported");
	}

	#[test]
	fn without_a_default_route_nothing_is_reported() {
		assert_eq!(assemble(&interfaces(), None), Snapshot::default());
		let elsewhere = DefaultRoute {
			index: Some(77),
			name: None,
			gateway: GATEWAY,
		};
		assert_eq!(
			assemble(&interfaces(), Some(&elsewhere)),
			Snapshot {
				gateway: Some(GATEWAY),
				..Snapshot::default()
			}
		);
	}

	#[test]
	fn proc_net_route_yields_the_lowest_metric_default_route() {
		// Real Linux columns; addresses are little-endian words on x86 and arm.
		let table = [
			"Iface Destination Gateway Flags RefCnt Use Metric Mask MTU Window IRTT",
			"wlan0 00000000 0101A8C0 0003 0 0 600 00000000 0 0 0",
			"eth0 00000000 0100000A 0003 0 0 100 00000000 0 0 0",
			"eth0 0000000A 00000000 0001 0 0 100 000000FF 0 0 0",
			"tun0 00000000 0200000A 0001 0 0 1 00000000 0 0 0",
			"broken line",
		]
		.join("\n")
		.replace(' ', "\t");
		let table = table.as_str();
		let route = parse_proc_route(table).unwrap();
		assert_eq!(route.name.as_deref(), Some("eth0"));
		assert_eq!(route.index, None);
		let expected = if cfg!(target_endian = "little") {
			Ipv4Addr::new(10, 0, 0, 1)
		} else {
			Ipv4Addr::new(1, 0, 0, 10)
		};
		assert_eq!(route.gateway, expected);

		assert_eq!(parse_proc_route(""), None);
		assert_eq!(
			parse_proc_route(
				"Iface\tDestination\n eth0\t0000000A\t0100000A\t0003\t0\t0\t1\t000000FF\n"
			),
			None,
			"only a 0.0.0.0/0 destination is a default route"
		);
	}

	/// Builds a Darwin routing message for an IPv4 destination and gateway.
	fn message(
		index: u16,
		flags: u32,
		destination: [u8; 4],
		gateway: [u8; 4],
		mask: Option<&[u8]>,
	) -> Vec<u8> {
		fn sockaddr(address: [u8; 4]) -> [u8; 16] {
			let mut bytes = [0; 16];
			bytes[0] = 16;
			bytes[1] = 2;
			bytes[4..8].copy_from_slice(&address);
			bytes
		}

		let mut body = Vec::new();
		body.extend_from_slice(&sockaddr(destination));
		body.extend_from_slice(&sockaddr(gateway));
		let mut addresses = 0b011u32;

		if let Some(mask) = mask {
			// An empty address still takes four bytes.
			let padded = if mask[0] == 0 {
				4
			} else {
				mask.len().next_multiple_of(4)
			};
			let start = body.len();
			addresses |= 0b100;
			body.extend_from_slice(mask);
			body.resize(start + padded, 0);
		}

		let mut header = vec![0u8; 92];
		let length = (header.len() + body.len()) as u16;
		header[..2].copy_from_slice(&length.to_ne_bytes());
		header[2] = 5;
		header[4..6].copy_from_slice(&index.to_ne_bytes());
		header[8..12].copy_from_slice(&flags.to_ne_bytes());
		header[12..16].copy_from_slice(&addresses.to_ne_bytes());
		header.extend_from_slice(&body);
		header
	}

	#[test]
	fn darwin_route_dumps_yield_the_primary_default_route() {
		const UP_GATEWAY: u32 = 0x3;
		const IFSCOPE: u32 = 0x0100_0000;
		let mut dump = Vec::new();
		dump.extend(message(
			9,
			UP_GATEWAY | IFSCOPE,
			[0; 4],
			[10, 0, 0, 1],
			None,
		));
		dump.extend(message(
			4,
			UP_GATEWAY,
			[10, 0, 0, 0],
			[10, 0, 0, 1],
			Some(&[8, 2, 0, 0, 255, 0, 0, 0]),
		));
		dump.extend(message(4, 0x1, [0; 4], [10, 0, 0, 1], None));
		dump.extend(message(6, UP_GATEWAY, [0; 4], [192, 168, 1, 1], Some(&[0])));
		dump.extend(message(4, UP_GATEWAY, [0; 4], [172, 16, 0, 1], None));

		assert_eq!(
			parse_route_dump(&dump),
			Some(DefaultRoute {
				index: Some(6),
				name: None,
				gateway: GATEWAY,
			})
		);
		assert_eq!(
			parse_route_dump(&dump[..100]),
			None,
			"a truncated message stops parsing"
		);
		assert_eq!(parse_route_dump(&[]), None);
	}

	#[cfg(target_os = "macos")]
	#[test]
	fn the_darwin_header_size_matches_libc() {
		assert_eq!(size_of::<libc::rt_msghdr>(), 92);
	}

	#[test]
	fn the_desktop_adapter_reports_a_valid_snapshot_and_rejects_bad_calls() {
		let mut plugin = crate::desktop::NetworkInfo;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), &[SNAPSHOT]);
		let reading = Snapshot::decode(&plugin.invoke(SNAPSHOT, Bytes::new()).unwrap()).unwrap();
		assert_eq!(reading.ssid, None);
		assert_eq!(reading.bssid, None);
		assert_eq!(
			plugin.invoke(SNAPSHOT, Bytes::from_static(&[0])),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			plugin.invoke(MethodId(1), Bytes::new()),
			Err(Error::Unavailable)
		);
	}
}
