use super::*;

#[allow(clippy::unnecessary_wraps)]
fn known(value: &str) -> Option<String> {
	Some(value.to_owned())
}

#[allow(clippy::unnecessary_wraps)]
fn at(millis: u64) -> Option<SystemTime> {
	Some(SystemTime::UNIX_EPOCH + Duration::from_millis(millis))
}

fn hex(bytes: &[u8]) -> String {
	use std::fmt::Write;

	bytes.iter().fold(String::new(), |mut text, byte| {
		let _ = write!(text, "{byte:02x}");
		text
	})
}

const SIGNATURE: &str = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";

/// The fixture `tests/android/ApplicationPluginTest.kt` encodes.
fn android_fixture() -> Snapshot {
	Snapshot {
		id: known("dev.example.app"),
		name: known("Example"),
		version: known("1.2.3"),
		build: known("42"),
		build_signature: known(SIGNATURE),
		installer_store: known(installer::GOOGLE_PLAY),
		install_time: at(1_700_000_000_000),
		update_time: at(1_710_000_000_456),
	}
}

const ANDROID_GOLDEN: &str = concat!(
	"020000000f0000006465762e6578616d706c652e617070070000004578616d706c6505000000312e",
	"322e3302000000343240000000303132333435363738394142434445463031323334353637383941",
	"42434445463031323334353637383941424344454630313233343536373839414243444546130000",
	"00636f6d2e616e64726f69642e76656e64696e67010000000068e5cf8b01000001000000c84df123",
	"8e010000",
);

/// The fixture `apple-tests/ApplicationTests.swift` encodes.
fn apple_fixture() -> Snapshot {
	Snapshot {
		id: known("dev.example.app"),
		name: known("Example"),
		version: known("1.2.3"),
		build: known("42"),
		build_signature: None,
		installer_store: known(installer::TESTFLIGHT),
		install_time: at(1_700_000_000_000),
		update_time: None,
	}
}

const APPLE_GOLDEN: &str = concat!(
	"020000000f0000006465762e6578616d706c652e617070070000004578616d706c6505000000312e",
	"322e330200000034320000000014000000636f6d2e6170706c652e74657374666c69676874010000",
	"000068e5cf8b010000000000000000000000000000",
);

#[test]
fn golden_bytes_match_the_swift_and_kotlin_tests() {
	assert_eq!(hex(&android_fixture().encode().unwrap()), ANDROID_GOLDEN);
	assert_eq!(hex(&apple_fixture().encode().unwrap()), APPLE_GOLDEN);
}

#[test]
fn identity_round_trip_and_host_validation() {
	let identity = Snapshot {
		name: known("你好"),
		build: known("4294967296"),
		..android_fixture()
	};
	let mut host = Application::new(&identity).unwrap();
	assert_eq!(host.name(), NAME);
	assert_eq!(host.methods(), &[SNAPSHOT]);
	assert_eq!(
		Snapshot::decode(&host.invoke(SNAPSHOT, Bytes::new()).unwrap()),
		Ok(identity)
	);
	assert_eq!(
		host.invoke(MethodId(1), Bytes::new()),
		Err(Error::Unavailable)
	);
	assert_eq!(
		host.invoke(SNAPSHOT, Bytes::from_static(&[1])),
		Err(Error::InvalidArgument)
	);
	assert_eq!(
		Application::new(&Snapshot {
			name: Some(String::new()),
			..Snapshot::default()
		})
		.err(),
		Some(Error::InvalidArgument)
	);
}

#[test]
fn every_fixture_round_trips_with_one_encoding() {
	for snapshot in [Snapshot::default(), android_fixture(), apple_fixture()] {
		let encoded = snapshot.encode().unwrap();
		assert_eq!(Snapshot::decode(&encoded), Ok(snapshot.clone()));
		assert_eq!(
			Snapshot::decode(&encoded).unwrap().encode().unwrap(),
			encoded
		);
	}
}

/// Hand-built version 1 bytes, as hosts before version 2 sent them.
fn version_one(strings: [&str; 4]) -> Vec<u8> {
	let mut bytes = 1_u32.to_le_bytes().to_vec();

	for text in strings {
		bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
		bytes.extend_from_slice(text.as_bytes());
	}

	bytes
}

#[test]
fn version_one_replies_from_older_hosts_still_decode() {
	assert_eq!(
		Snapshot::decode(&version_one(["dev.example.app", "Example", "1.2.3", "42"])),
		Ok(Snapshot {
			id: known("dev.example.app"),
			name: known("Example"),
			version: known("1.2.3"),
			build: known("42"),
			..Snapshot::default()
		})
	);
	let empty = version_one([""; 4]);
	assert_eq!(Snapshot::decode(&empty), Ok(Snapshot::default()));
	let mut trailing = empty;
	trailing.extend_from_slice(&0_u32.to_le_bytes());
	assert_eq!(
		Snapshot::decode(&trailing),
		Err(Error::InvalidArgument),
		"version 1 has no signature field"
	);
}

#[test]
fn decoder_rejects_truncated_utf8_versions_and_trailing_messages() {
	let encoded = android_fixture().encode().unwrap();

	for length in 0..encoded.len() {
		assert_eq!(
			Snapshot::decode(&encoded[..length]),
			Err(Error::InvalidArgument)
		);
	}

	let mut invalid = encoded.to_vec();
	invalid[8] = 255;
	assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));

	for version in [0, 3, u32::MAX] {
		let mut invalid = encoded.to_vec();
		invalid[..4].copy_from_slice(&version.to_le_bytes());
		assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
	}

	let mut invalid = encoded.to_vec();
	invalid.push(0);
	assert_eq!(Snapshot::decode(&invalid), Err(Error::InvalidArgument));
}

#[test]
fn times_need_a_canonical_presence_word() {
	let encoded = Snapshot::default().encode().unwrap().to_vec();
	// Version and six empty strings, then install then update time.
	let install = 4 + 6 * 4;
	let update = install + 12;

	for (offset, value) in [(install, 2), (install + 4, 1), (update, 9), (update + 8, 1)] {
		let mut invalid = encoded.clone();
		invalid[offset..offset + 4].copy_from_slice(&(value as u32).to_le_bytes());
		assert_eq!(
			Snapshot::decode(&invalid),
			Err(Error::InvalidArgument),
			"word {value} at {offset}"
		);
	}

	let mut epoch = encoded;
	epoch[install..install + 4].copy_from_slice(&1_u32.to_le_bytes());
	assert_eq!(
		Snapshot::decode(&epoch).unwrap().install_time,
		Some(SystemTime::UNIX_EPOCH)
	);
}

#[test]
fn times_truncate_to_milliseconds_and_reject_dates_before_1970() {
	let precise = Snapshot {
		update_time: Some(SystemTime::UNIX_EPOCH + Duration::from_nanos(1_999_999_999)),
		..Snapshot::default()
	};
	assert_eq!(
		Snapshot::decode(&precise.encode().unwrap())
			.unwrap()
			.update_time,
		at(1_999)
	);
	assert_eq!(
		Snapshot {
			install_time: Some(SystemTime::UNIX_EPOCH - Duration::from_millis(1)),
			..Snapshot::default()
		}
		.encode(),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn build_signatures_are_64_uppercase_hex_digits() {
	for invalid in [
		&SIGNATURE.to_lowercase(),
		&SIGNATURE[..63],
		&format!("{}G", &SIGNATURE[..63]),
		&format!("{}:", &SIGNATURE[..63]),
	] {
		assert_eq!(
			Snapshot {
				build_signature: Some(invalid.to_string()),
				..Snapshot::default()
			}
			.encode(),
			Err(Error::InvalidArgument),
			"{invalid}"
		);
	}

	assert_eq!(
		Snapshot {
			build_signature: Some(format!("{SIGNATURE}0")),
			..Snapshot::default()
		}
		.encode(),
		Err(Error::TooLarge)
	);

	// A lowercase digest on the wire is rejected too.
	let mut wire = android_fixture().encode().unwrap().to_vec();
	let start = wire
		.windows(SIGNATURE_LENGTH)
		.position(|window| window == SIGNATURE.as_bytes())
		.unwrap();
	wire[start + 10] = b'a';
	assert_eq!(Snapshot::decode(&wire), Err(Error::InvalidArgument));
}

#[test]
fn unknown_identity_and_bounded_text() {
	let unknown = Snapshot::default();
	assert_eq!(Snapshot::decode(&unknown.encode().unwrap()), Ok(unknown));

	for empty in [
		Snapshot {
			name: Some(String::new()),
			..Snapshot::default()
		},
		Snapshot {
			build_signature: Some(String::new()),
			..Snapshot::default()
		},
		Snapshot {
			installer_store: Some(String::new()),
			..Snapshot::default()
		},
	] {
		assert_eq!(empty.encode(), Err(Error::InvalidArgument));
	}

	assert_eq!(
		Snapshot {
			name: Some("x".repeat(MAX_TEXT_BYTES + 1)),
			..Snapshot::default()
		}
		.encode(),
		Err(Error::TooLarge)
	);
	assert_eq!(
		Snapshot {
			installer_store: Some("x".repeat(MAX_INSTALLER_BYTES + 1)),
			..Snapshot::default()
		}
		.encode(),
		Err(Error::TooLarge)
	);
	let mut wire = Snapshot::default().encode().unwrap().to_vec();
	wire.splice(4..8, 4097_u32.to_le_bytes());
	wire.splice(8..8, vec![b'x'; 4097]);
	assert_eq!(Snapshot::decode(&wire), Err(Error::TooLarge));

	let mut installer = Snapshot::default().encode().unwrap().to_vec();
	let offset = 4 + 5 * 4;
	installer.splice(
		offset..offset + 4,
		(MAX_INSTALLER_BYTES as u32 + 1).to_le_bytes(),
	);
	installer.splice(offset + 4..offset + 4, vec![b'x'; MAX_INSTALLER_BYTES + 1]);
	assert_eq!(Snapshot::decode(&installer), Err(Error::TooLarge));
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn executable_times_describe_the_test_binary() {
	let (created, modified) = executable_times();
	let modified = modified.expect("every supported file system records modification");
	assert!(modified <= SystemTime::now());
	let snapshot = Snapshot {
		install_time: created,
		update_time: Some(modified),
		..Snapshot::default()
	};
	assert!(snapshot.encode().is_ok());
}
