use std::time::Duration;

use super::*;

#[allow(clippy::unnecessary_wraps)]
fn known(value: &str) -> Option<String> {
	Some(value.to_owned())
}

fn hex(bytes: &[u8]) -> String {
	use std::fmt::Write;

	bytes.iter().fold(String::new(), |mut text, byte| {
		let _ = write!(text, "{byte:02x}");
		text
	})
}

/// The fixture the Swift tests encode from fake facts. Keep in step with
/// `apple-tests/DeviceTests.swift`.
pub(crate) fn apple_fixture() -> Snapshot {
	Snapshot {
		manufacturer: known("Apple"),
		brand: known("Apple"),
		model: known("iPhone"),
		model_identifier: known("iPhone15,2"),
		os_name: known("iOS"),
		os_version: known("18.1"),
		architecture: known("aarch64"),
		kernel_version: known("24.1.0"),
		total_memory: NonZeroU64::new(6 << 30),
		available_memory: Some(1 << 20),
		total_disk: NonZeroU64::new(128 << 30),
		free_disk: Some(0),
		cpu_count: NonZeroU32::new(6),
		kind: Kind::Phone,
		simulator: Some(true),
		platform: Platform::Apple(Apple {
			ios_app_on_mac: Some(false),
			mac_catalyst: Some(false),
			ios_app_on_vision: None,
			cpu_frequency: None,
			localized_model: known("iPhone"),
			os_build: known("22B83"),
		}),
	}
}

const APPLE_GOLDEN: &str = concat!(
	"02000000010000000200000000000080010000000100000000001000000000000000000020000000",
	"01000000000000000000000006000000050000004170706c65050000004170706c65060000006950",
	"686f6e650a0000006950686f6e6531352c3203000000694f530400000031382e3107000000616172",
	"636836340600000032342e312e300100000001000000010000000000000000000000000000000600",
	"00006950686f6e65050000003232423833",
);

/// The fixture the Kotlin tests encode from fake facts. Keep in step with
/// `tests/android/DevicePluginTest.kt`.
pub(crate) fn android_fixture() -> Snapshot {
	Snapshot {
		manufacturer: known("Google"),
		brand: known("google"),
		model: known("Pixel 9"),
		model_identifier: None,
		os_name: known("Android"),
		os_version: known("15"),
		architecture: known("arm64-v8a"),
		kernel_version: known("6.1.75"),
		total_memory: NonZeroU64::new(12 << 30),
		available_memory: Some(3 << 30),
		total_disk: NonZeroU64::new(256 << 30),
		free_disk: Some(100 << 30),
		cpu_count: NonZeroU32::new(8),
		kind: Kind::Phone,
		simulator: None,
		platform: Platform::Android(Box::new(Android {
			sdk_int: NonZeroU32::new(35),
			preview_sdk_int: Some(0),
			low_ram_device: Some(false),
			low_memory: Some(true),
			build_time: Some(SystemTime::UNIX_EPOCH + Duration::from_millis(1_700_000_000_123)),
			base_os: None,
			codename: known("REL"),
			incremental: known("12345"),
			release: known("15"),
			security_patch: known("2026-10-05"),
			board: known("tokay"),
			bootloader: known("b1"),
			device: known("tokay"),
			display: known("AP4A.1"),
			fingerprint: known("google/tokay/tokay:15/AP4A.1/12345:user/release-keys"),
			hardware: known("tokay"),
			host: known("abfarm"),
			id: known("AP4A.1"),
			product: known("tokay"),
			tags: known("release-keys"),
			build_type: known("user"),
			supported_abis: vec!["arm64-v8a".into()],
			supported_32_bit_abis: Vec::new(),
			supported_64_bit_abis: vec!["arm64-v8a".into()],
			system_features: vec![
				"android.hardware.camera".into(),
				"android.hardware.wifi".into(),
			],
		})),
	}
}

const ANDROID_GOLDEN: &str = concat!(
	"020000000100000000000000000000000300000001000000000000c0000000000000000040000000",
	"0100000000000000190000000800000006000000476f6f676c6506000000676f6f676c6507000000",
	"506978656c20390000000007000000416e64726f69640200000031350900000061726d36342d7638",
	"6106000000362e312e3735020000002300000001000000000000000100000002000000010000007b",
	"68e5cf8b010000000000000300000052454c0500000031323334350200000031350a000000323032",
	"362d31302d303505000000746f6b617902000000623105000000746f6b617906000000415034412e",
	"3134000000676f6f676c652f746f6b61792f746f6b61793a31352f415034412e312f31323334353a",
	"757365722f72656c656173652d6b65797305000000746f6b61790600000061626661726d06000000",
	"415034412e3105000000746f6b61790c00000072656c656173652d6b657973040000007573657201",
	"0000000900000061726d36342d76386100000000010000000900000061726d36342d763861020000",
	"0017000000616e64726f69642e68617264776172652e63616d65726115000000616e64726f69642e",
	"68617264776172652e77696669",
);

pub(crate) fn identifiers_fixture() -> Identifiers {
	Identifiers {
		device_name: known("Ada's iPhone"),
		vendor_id: known("01234567-89AB-CDEF-0123-456789ABCDEF"),
		machine_id: None,
	}
}

const IDENTIFIERS_GOLDEN: &str = concat!(
	"010000000c0000004164612773206950686f6e652400000030313233343536372d383941422d4344",
	"45462d303132332d34353637383941424344454600000000",
);

fn windows_fixture() -> Snapshot {
	Snapshot {
		os_name: known("Windows"),
		kind: Kind::Desktop,
		platform: Platform::Windows(Windows {
			major_version: Some(10),
			minor_version: Some(0),
			build_number: Some(26_100),
			update_build_revision: Some(2_314),
			product_name: known("Windows 11 Pro"),
			display_version: known("24H2"),
			edition_id: known("Professional"),
			release_id: known("2009"),
			build_lab: known("26100.ge_release.240331-1435"),
			build_lab_ex: None,
		}),
		..Snapshot::default()
	}
}

fn linux_fixture() -> Snapshot {
	Snapshot {
		os_name: known("Ubuntu"),
		kind: Kind::Desktop,
		platform: Platform::Linux(Linux {
			name: known("Ubuntu"),
			version: known("24.04.1 LTS (Noble Numbat)"),
			id: known("ubuntu"),
			version_codename: known("noble"),
			version_id: known("24.04"),
			pretty_name: known("Ubuntu 24.04.1 LTS"),
			build_id: None,
			variant: None,
			variant_id: None,
			id_like: vec!["debian".into()],
		}),
		..Snapshot::default()
	}
}

fn web_fixture() -> Snapshot {
	Snapshot {
		os_name: known("macOS"),
		cpu_count: NonZeroU32::new(10),
		platform: Platform::Web(Web {
			mobile: Some(false),
			max_touch_points: Some(0),
			brands: vec![
				Brand {
					brand: "Chromium".into(),
					version: "142".into(),
				},
				Brand {
					brand: "Not_A Brand".into(),
					version: String::new(),
				},
			],
		}),
		..Snapshot::default()
	}
}

fn fixtures() -> Vec<Snapshot> {
	vec![
		Snapshot::default(),
		apple_fixture(),
		android_fixture(),
		windows_fixture(),
		linux_fixture(),
		web_fixture(),
	]
}

fn android(snapshot: &mut Snapshot) -> &mut Android {
	match &mut snapshot.platform {
		Platform::Android(android) => android,
		_ => unreachable!("not the Android fixture"),
	}
}

#[test]
fn every_platform_section_round_trips() {
	for snapshot in fixtures() {
		let encoded = snapshot.encode().unwrap();
		assert_eq!(Snapshot::decode(&encoded), Ok(snapshot.clone()));
		assert_eq!(
			Snapshot::decode(&encoded).unwrap().encode().unwrap(),
			encoded,
			"one valid encoding per value"
		);
	}
}

#[test]
fn golden_bytes_match_the_swift_and_kotlin_tests() {
	assert_eq!(hex(&apple_fixture().encode().unwrap()), APPLE_GOLDEN);
	assert_eq!(hex(&android_fixture().encode().unwrap()), ANDROID_GOLDEN);
	assert_eq!(
		hex(&identifiers_fixture().encode().unwrap()),
		IDENTIFIERS_GOLDEN
	);
}

#[test]
fn every_kind_and_simulator_state_round_trips() {
	let kinds = [
		Kind::Unknown,
		Kind::Phone,
		Kind::Tablet,
		Kind::Television,
		Kind::Desktop,
		Kind::Watch,
		Kind::Car,
		Kind::Headset,
	];

	for kind in kinds {
		for simulator in [None, Some(false), Some(true)] {
			let snapshot = Snapshot {
				kind,
				simulator,
				..Snapshot::default()
			};
			assert_eq!(Snapshot::decode(&snapshot.encode().unwrap()), Ok(snapshot));
		}
	}
}

#[test]
fn round_trip_preserves_unicode_large_numbers_and_zero_free_space() {
	let snapshot = Snapshot {
		manufacturer: known("製造元"),
		total_memory: NonZeroU64::new(u64::MAX),
		available_memory: Some(0),
		total_disk: NonZeroU64::new(1 << 40),
		free_disk: Some(0),
		cpu_count: NonZeroU32::new(u32::MAX),
		..Snapshot::default()
	};
	assert_eq!(Snapshot::decode(&snapshot.encode().unwrap()), Ok(snapshot));
}

/// Hand-built version 1 bytes, as hosts before version 2 sent them.
fn version_one(kind: u32, simulator: u32, memory: u64, strings: [&str; 6]) -> Vec<u8> {
	let mut bytes = Vec::new();

	for word in [1, kind, simulator, memory as u32, (memory >> 32) as u32] {
		bytes.extend_from_slice(&word.to_le_bytes());
	}

	for text in strings {
		bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
		bytes.extend_from_slice(text.as_bytes());
	}

	bytes
}

#[test]
fn version_one_replies_from_older_hosts_still_decode() {
	let bytes = version_one(
		2,
		1,
		16 << 30,
		["Apple", "Apple", "iPad", "iPadOS", "17.5", "aarch64"],
	);
	assert_eq!(
		Snapshot::decode(&bytes),
		Ok(Snapshot {
			manufacturer: known("Apple"),
			brand: known("Apple"),
			model: known("iPad"),
			os_name: known("iPadOS"),
			os_version: known("17.5"),
			architecture: known("aarch64"),
			total_memory: NonZeroU64::new(16 << 30),
			kind: Kind::Tablet,
			simulator: Some(false),
			..Snapshot::default()
		})
	);
	assert_eq!(
		Snapshot::decode(&version_one(0, 0, 0, [""; 6])),
		Ok(Snapshot::default())
	);

	let valid = version_one(1, 0, 0, ["", "", "x", "", "", ""]);

	for length in 0..valid.len() {
		assert_eq!(
			Snapshot::decode(&valid[..length]),
			Err(Error::InvalidArgument)
		);
	}

	for (offset, value) in [(4, 8), (8, 3), (32, 255)] {
		let mut invalid = valid.clone();
		invalid[offset] = value;
		assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
	}

	let mut trailing = valid;
	trailing.push(0);
	assert_eq!(Snapshot::decode(&trailing), Err(Error::InvalidArgument));
	let mut oversized = version_one(0, 0, 0, [""; 6]);
	oversized.splice(28..32, 4097_u32.to_le_bytes());
	oversized.splice(32..32, vec![b'x'; 4097]);
	assert_eq!(Snapshot::decode(&oversized), Err(Error::TooLarge));
}

#[test]
fn rejects_truncation_and_trailing_data_for_every_platform() {
	for snapshot in fixtures() {
		let encoded = snapshot.encode().unwrap();

		for length in 0..encoded.len() {
			assert_eq!(
				Snapshot::decode(&encoded[..length]),
				Err(Error::InvalidArgument),
				"{snapshot:?} truncated to {length}"
			);
		}

		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert_eq!(Snapshot::decode(&trailing), Err(Error::InvalidArgument));
	}
}

/// Offsets into the version 2 header.
const KIND: usize = 4;
const SIMULATOR: usize = 8;
const AVAILABLE_PRESENT: usize = 20;
const AVAILABLE: usize = 24;
const FREE_PRESENT: usize = 40;
const FREE: usize = 44;
/// The first string length, after the CPU count.
const FIRST_STRING: usize = 56;

fn word(bytes: &mut [u8], offset: usize, value: u32) {
	bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn rejects_unknown_versions_codes_and_non_canonical_optionals() {
	let encoded = Snapshot::default().encode().unwrap().to_vec();
	let tag = encoded.len() - 4;

	for (offset, value) in [
		(0, 0),
		(0, 3),
		(KIND, 8),
		(SIMULATOR, 3),
		(AVAILABLE_PRESENT, 2),
		(AVAILABLE, 1),
		(FREE_PRESENT, 7),
		(FREE + 4, 1),
		(tag, 6),
		(tag, u32::MAX),
	] {
		let mut invalid = encoded.clone();
		word(&mut invalid, offset, value);
		assert_eq!(
			Snapshot::decode(&invalid),
			Err(Error::InvalidArgument),
			"word {value} at {offset}"
		);
	}

	let mut utf8 = Snapshot {
		manufacturer: known("x"),
		..Snapshot::default()
	}
	.encode()
	.unwrap()
	.to_vec();
	utf8[FIRST_STRING + 4] = 0xff;
	assert_eq!(Snapshot::decode(&utf8), Err(Error::InvalidArgument));
}

#[test]
fn rejects_invalid_tri_states_inside_platform_sections() {
	let apple = apple_fixture().encode().unwrap().to_vec();
	let header = Snapshot {
		platform: Platform::None,
		..apple_fixture()
	}
	.encode()
	.unwrap()
	.len();

	// The Apple section starts with three tri-states.
	for index in 0..3 {
		let mut invalid = apple.clone();
		word(&mut invalid, header + index * 4, 3);
		assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
	}

	let android = android_fixture().encode().unwrap().to_vec();
	let header = Snapshot {
		platform: Platform::None,
		..android_fixture()
	}
	.encode()
	.unwrap()
	.len();
	// Preview presence, the two tri-states, and the build-time presence.
	for (offset, value) in [(4, 2), (12, 3), (16, 3), (20, 2)] {
		let mut invalid = android.clone();
		word(&mut invalid, header + offset, value);
		assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
	}

	let web = web_fixture().encode().unwrap().to_vec();
	let header = Snapshot {
		platform: Platform::None,
		..web_fixture()
	}
	.encode()
	.unwrap()
	.len();
	let mut invalid = web.clone();
	word(&mut invalid, header, 3);
	assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
	let mut invalid = web;
	word(&mut invalid, header + 4, 2);
	assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
}

#[test]
fn rejects_empty_known_text_in_every_section() {
	let mut snapshots = Vec::new();

	for index in 0..8 {
		let mut snapshot = Snapshot::default();
		let field = [
			&mut snapshot.manufacturer,
			&mut snapshot.brand,
			&mut snapshot.model,
			&mut snapshot.model_identifier,
			&mut snapshot.os_name,
			&mut snapshot.os_version,
			&mut snapshot.architecture,
			&mut snapshot.kernel_version,
		];
		*field.into_iter().nth(index).unwrap() = Some(String::new());
		snapshots.push(snapshot);
	}

	let mut apple = apple_fixture();
	if let Platform::Apple(section) = &mut apple.platform {
		section.os_build = Some(String::new());
	}
	snapshots.push(apple);
	let mut android_empty = android_fixture();
	android(&mut android_empty).fingerprint = Some(String::new());
	snapshots.push(android_empty);
	let mut windows = windows_fixture();
	if let Platform::Windows(section) = &mut windows.platform {
		section.edition_id = Some(String::new());
	}
	snapshots.push(windows);
	let mut linux = linux_fixture();
	if let Platform::Linux(section) = &mut linux.platform {
		section.variant = Some(String::new());
	}
	snapshots.push(linux);

	for snapshot in snapshots {
		assert_eq!(
			snapshot.encode(),
			Err(Error::InvalidArgument),
			"{snapshot:?}"
		);
	}
}

#[test]
fn rejects_oversized_text_in_both_directions() {
	let oversized = Snapshot {
		kernel_version: Some("x".repeat(MAX_TEXT_BYTES + 1)),
		..Snapshot::default()
	};
	assert_eq!(oversized.encode(), Err(Error::TooLarge));
	let fits = Snapshot {
		kernel_version: Some("x".repeat(MAX_TEXT_BYTES)),
		..Snapshot::default()
	};
	assert_eq!(Snapshot::decode(&fits.encode().unwrap()), Ok(fits));

	let mut wire = Snapshot::default().encode().unwrap().to_vec();
	word(&mut wire, FIRST_STRING, 4097);
	wire.splice(FIRST_STRING + 4..FIRST_STRING + 4, vec![b'x'; 4097]);
	assert_eq!(Snapshot::decode(&wire), Err(Error::TooLarge));
}

#[test]
fn bounds_every_list() {
	let mut many_abis = android_fixture();
	android(&mut many_abis).supported_abis = vec!["x86".into(); MAX_SHORT_LIST + 1];
	assert_eq!(many_abis.encode(), Err(Error::TooLarge));

	let mut long_abi = android_fixture();
	android(&mut long_abi).supported_32_bit_abis = vec!["x".repeat(MAX_SHORT_ITEM_BYTES + 1)];
	assert_eq!(long_abi.encode(), Err(Error::TooLarge));

	let mut empty_abi = android_fixture();
	android(&mut empty_abi).supported_64_bit_abis = vec![String::new()];
	assert_eq!(empty_abi.encode(), Err(Error::InvalidArgument));

	let mut features = android_fixture();
	android(&mut features).system_features = (0..MAX_FEATURES)
		.map(|index| format!("feature.{index}"))
		.collect();
	assert_eq!(
		Snapshot::decode(&features.encode().unwrap()),
		Ok(features.clone())
	);
	android(&mut features)
		.system_features
		.push("one.more".into());
	assert_eq!(features.encode(), Err(Error::TooLarge));

	let mut long_feature = android_fixture();
	android(&mut long_feature).system_features = vec!["f".repeat(MAX_FEATURE_BYTES + 1)];
	assert_eq!(long_feature.encode(), Err(Error::TooLarge));

	let mut id_like = linux_fixture();
	if let Platform::Linux(section) = &mut id_like.platform {
		section.id_like = vec!["debian".into(); MAX_SHORT_LIST + 1];
	}
	assert_eq!(id_like.encode(), Err(Error::TooLarge));

	let mut brands = web_fixture();
	if let Platform::Web(section) = &mut brands.platform {
		section.brands = vec![Brand::default(); 1];
	}
	assert_eq!(
		brands.encode(),
		Err(Error::InvalidArgument),
		"a brand needs a name"
	);
	if let Platform::Web(section) = &mut brands.platform {
		section.brands = vec![
			Brand {
				brand: "b".into(),
				version: String::new(),
			};
			MAX_SHORT_LIST + 1
		];
	}
	assert_eq!(brands.encode(), Err(Error::TooLarge));
}

#[test]
fn rejects_list_counts_and_items_the_encoder_never_writes() {
	let linux = Snapshot {
		platform: Platform::Linux(Linux::default()),
		..Snapshot::default()
	};
	let encoded = linux.encode().unwrap().to_vec();
	let count = encoded.len() - 4;

	let mut too_many = encoded.clone();
	word(&mut too_many, count, MAX_SHORT_LIST as u32 + 1);
	assert_eq!(Snapshot::decode(&too_many), Err(Error::TooLarge));

	let mut empty_item = encoded.clone();
	word(&mut empty_item, count, 1);
	empty_item.extend_from_slice(&0_u32.to_le_bytes());
	assert_eq!(Snapshot::decode(&empty_item), Err(Error::InvalidArgument));

	let mut long_item = encoded;
	word(&mut long_item, count, 1);
	long_item.extend_from_slice(&(MAX_SHORT_ITEM_BYTES as u32 + 1).to_le_bytes());
	long_item.extend(vec![b'x'; MAX_SHORT_ITEM_BYTES + 1]);
	assert_eq!(Snapshot::decode(&long_item), Err(Error::TooLarge));

	let web = Snapshot {
		platform: Platform::Web(Web::default()),
		..Snapshot::default()
	};
	let mut unnamed = web.encode().unwrap().to_vec();
	let count = unnamed.len() - 4;
	word(&mut unnamed, count, 1);
	unnamed.extend_from_slice(&0_u32.to_le_bytes());
	unnamed.extend_from_slice(&0_u32.to_le_bytes());
	assert_eq!(Snapshot::decode(&unnamed), Err(Error::InvalidArgument));
}

#[test]
fn build_times_before_the_epoch_are_invalid_and_millis_truncate() {
	let mut early = android_fixture();
	android(&mut early).build_time = Some(SystemTime::UNIX_EPOCH - Duration::from_secs(1));
	assert_eq!(early.encode(), Err(Error::InvalidArgument));

	let mut precise = android_fixture();
	android(&mut precise).build_time =
		Some(SystemTime::UNIX_EPOCH + Duration::from_nanos(1_234_567_891));
	let decoded = Snapshot::decode(&precise.encode().unwrap()).unwrap();
	let Platform::Android(decoded) = decoded.platform else {
		unreachable!()
	};
	assert_eq!(
		decoded.build_time,
		Some(SystemTime::UNIX_EPOCH + Duration::from_millis(1_234))
	);
}

#[test]
fn identifiers_round_trip_and_validate() {
	for identifiers in [Identifiers::default(), identifiers_fixture()] {
		assert_eq!(
			Identifiers::decode(&identifiers.encode().unwrap()),
			Ok(identifiers)
		);
	}

	let encoded = identifiers_fixture().encode().unwrap();

	for length in 0..encoded.len() {
		assert_eq!(
			Identifiers::decode(&encoded[..length]),
			Err(Error::InvalidArgument)
		);
	}

	let mut trailing = encoded.to_vec();
	trailing.push(0);
	assert_eq!(Identifiers::decode(&trailing), Err(Error::InvalidArgument));
	let mut version = encoded.to_vec();
	version[0] = 2;
	assert_eq!(Identifiers::decode(&version), Err(Error::InvalidArgument));
	let mut utf8 = encoded.to_vec();
	utf8[8] = 0xff;
	assert_eq!(Identifiers::decode(&utf8), Err(Error::InvalidArgument));

	assert_eq!(
		Identifiers {
			machine_id: Some(String::new()),
			..Identifiers::default()
		}
		.encode(),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		Identifiers {
			vendor_id: Some("v".repeat(MAX_IDENTIFIER_BYTES + 1)),
			..Identifiers::default()
		}
		.encode(),
		Err(Error::TooLarge)
	);
	let mut oversized = Identifiers::default().encode().unwrap().to_vec();
	word(&mut oversized, 4, MAX_IDENTIFIER_BYTES as u32 + 1);
	oversized.splice(8..8, vec![b'x'; MAX_IDENTIFIER_BYTES + 1]);
	assert_eq!(Identifiers::decode(&oversized), Err(Error::TooLarge));
}

#[test]
fn snapshots_never_carry_identifier_fields() {
	// The snapshot type has no device name, vendor ID or machine ID field, so
	// its wire cannot carry one. This pins the field list.
	let Snapshot {
		manufacturer: _,
		brand: _,
		model: _,
		model_identifier: _,
		os_name: _,
		os_version: _,
		architecture: _,
		kernel_version: _,
		total_memory: _,
		available_memory: _,
		total_disk: _,
		free_disk: _,
		cpu_count: _,
		kind: _,
		simulator: _,
		platform: _,
	} = Snapshot::default();
}

#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
mod desktop_tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	#[test]
	fn adapter_serves_both_methods_and_rejects_others() {
		let mut plugin = desktop::Device;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), [SNAPSHOT, IDENTIFIERS]);
		assert_eq!(
			plugin.invoke(MethodId(2), Bytes::new()),
			Err(Error::Unavailable)
		);

		for method in [SNAPSHOT, IDENTIFIERS] {
			assert_eq!(
				plugin.invoke(method, Bytes::from_static(&[1])),
				Err(Error::InvalidArgument)
			);
		}

		let snapshot = Snapshot::decode(&plugin.invoke(SNAPSHOT, Bytes::new()).unwrap()).unwrap();
		assert_eq!(
			snapshot,
			Snapshot {
				available_memory: snapshot.available_memory,
				free_disk: snapshot.free_disk,
				..desktop::snapshot()
			}
		);
		assert_eq!(snapshot.kind, Kind::Desktop);
		assert!(snapshot.architecture.is_some());
		assert!(snapshot.os_name.is_some());
		assert!(snapshot.kernel_version.is_some());
		assert!(snapshot.total_memory.is_some());
		assert!(snapshot.cpu_count.is_some());
		assert!(snapshot.total_disk.is_some());
		assert!(snapshot.free_disk <= snapshot.total_disk.map(NonZeroU64::get));
		Identifiers::decode(&plugin.invoke(IDENTIFIERS, Bytes::new()).unwrap()).unwrap();
	}

	#[test]
	#[cfg(target_os = "macos")]
	fn macos_reports_its_model_and_build() {
		let snapshot = desktop::snapshot();
		assert_eq!(snapshot.os_name.as_deref(), Some("macOS"));
		assert!(
			snapshot
				.model_identifier
				.as_deref()
				.is_some_and(|model| model.contains("Mac"))
		);
		let Platform::Apple(apple) = snapshot.platform else {
			panic!("macOS reports an Apple section");
		};
		assert!(apple.os_build.is_some());
		assert_eq!(apple.ios_app_on_mac, Some(false));
		let identifiers = desktop::identifiers();
		assert_eq!(identifiers.vendor_id, None);
		if let Some(uuid) = identifiers.machine_id {
			assert_eq!(uuid.len(), 36);
		}
	}

	#[test]
	#[cfg(target_os = "windows")]
	fn windows_reports_its_release() {
		let Platform::Windows(windows) = desktop::snapshot().platform else {
			panic!("Windows reports a Windows section");
		};
		assert!(windows.build_number.is_some());
	}

	#[test]
	#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
	fn linux_reports_os_release() {
		let Platform::Linux(linux) = desktop::snapshot().platform else {
			panic!("Linux reports a Linux section");
		};
		assert!(linux.id.is_some());
	}

	#[test]
	fn windows_11_product_names_are_corrected() {
		assert_eq!(
			desktop::windows_product_name("Windows 10 Pro".into(), Some(22_631)),
			"Windows 11 Pro"
		);
		assert_eq!(
			desktop::windows_product_name("Windows 10 Pro".into(), Some(19_045)),
			"Windows 10 Pro"
		);
		assert_eq!(
			desktop::windows_product_name("Windows 10 Pro".into(), None),
			"Windows 10 Pro"
		);
	}

	#[test]
	fn machine_ids_must_be_32_lowercase_hex_digits() {
		assert_eq!(
			desktop::machine_id_value("0123456789abcdef0123456789abcdef\n").as_deref(),
			Some("0123456789abcdef0123456789abcdef")
		);

		for invalid in [
			"",
			"uninitialized",
			"00000000000000000000000000000000",
			"0123456789ABCDEF0123456789ABCDEF",
			"0123456789abcdef0123456789abcde",
			"0123456789abcdef0123456789abcdef0",
		] {
			assert_eq!(desktop::machine_id_value(invalid), None, "{invalid:?}");
		}
	}
}
