use std::io::Write as _;

use super::*;

fn executable(machine: u16) -> Vec<u8> {
	let mut bytes = vec![0; 128];
	bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
	bytes[16..18].copy_from_slice(&2_u16.to_le_bytes());
	bytes[18..20].copy_from_slice(&machine.to_le_bytes());
	bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
	bytes[24..32].copy_from_slice(&0x0040_0078_u64.to_le_bytes());
	bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
	bytes[52..54].copy_from_slice(&64_u16.to_le_bytes());
	bytes[54..56].copy_from_slice(&56_u16.to_le_bytes());
	bytes[56..58].copy_from_slice(&1_u16.to_le_bytes());
	bytes[64..68].copy_from_slice(&1_u32.to_le_bytes());
	bytes[68..72].copy_from_slice(&5_u32.to_le_bytes());
	bytes[80..88].copy_from_slice(&0x0040_0000_u64.to_le_bytes());
	bytes[96..104].copy_from_slice(&128_u64.to_le_bytes());
	bytes[104..112].copy_from_slice(&128_u64.to_le_bytes());
	bytes
}
fn check(bytes: &[u8], architecture: Architecture) -> Result<Format> {
	let mut file = tempfile::tempfile()?;
	file.write_all(bytes)?;
	inspect(file, architecture)
}
#[test]
fn reads_architecture_without_executing_payload() {
	for (machine, architecture) in [(62, Architecture::X86_64), (183, Architecture::Aarch64)] {
		let format = check(&executable(machine), architecture).unwrap();
		assert_eq!(format.architecture, architecture);
		assert!(format.interpreter.is_none());
		assert_eq!(format.needed, [] as [String; 0]);
	}
	assert!(check(&executable(62), Architecture::Aarch64).is_err());
}
#[test]
fn rejects_foreign_truncated_and_overflowing_files() {
	let original = executable(62);
	for length in 0..original.len() {
		assert!(check(&original[..length], Architecture::X86_64).is_err());
	}
	for (offset, value) in [
		(0, 0xcf),
		(4, 1),
		(5, 2),
		(7, 9),
		(16, 1),
		(18, 40),
		(54, 55),
		(68, 7),
	] {
		let mut corrupt = original.clone();
		corrupt[offset] = value;
		assert!(
			check(&corrupt, Architecture::X86_64).is_err(),
			"offset {offset}"
		);
	}
	for offset in [24, 32, 72, 80, 96] {
		let mut corrupt = original.clone();
		corrupt[offset..offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
		assert!(
			check(&corrupt, Architecture::X86_64).is_err(),
			"offset {offset}"
		);
	}
}
#[test]
fn rejects_non_file_backed_entry_point() {
	let mut bytes = executable(62);
	bytes[24..32].copy_from_slice(&0x0040_0080_u64.to_le_bytes());
	bytes[104..112].copy_from_slice(&4096_u64.to_le_bytes());
	assert!(check(&bytes, Architecture::X86_64).is_err());
}

fn dynamic() -> Vec<u8> {
	let mut bytes = executable(62);
	bytes.resize(512, 0);
	bytes[24..32].copy_from_slice(&0x0040_00f8_u64.to_le_bytes());
	bytes[56..58].copy_from_slice(&3_u16.to_le_bytes());
	bytes[96..104].copy_from_slice(&512_u64.to_le_bytes());
	bytes[104..112].copy_from_slice(&512_u64.to_le_bytes());
	let loader = b"/lib64/ld-linux-x86-64.so.2\0";
	for (offset, kind, start, size) in [
		(120, 3_u32, 256_u64, loader.len() as u64),
		(176, 2, 384, 64),
	] {
		bytes[offset..offset + 4].copy_from_slice(&kind.to_le_bytes());
		bytes[offset + 8..offset + 16].copy_from_slice(&start.to_le_bytes());
		bytes[offset + 32..offset + 40].copy_from_slice(&size.to_le_bytes());
	}
	bytes[256..256 + loader.len()].copy_from_slice(loader);
	bytes[320..331].copy_from_slice(b"\0libc.so.6\0");
	for (index, tag, value) in [(0, 5_u64, 0x0040_0140_u64), (1, 10, 11), (2, 1, 1)] {
		let offset = 384 + index * 16;
		bytes[offset..offset + 8].copy_from_slice(&tag.to_le_bytes());
		bytes[offset + 8..offset + 16].copy_from_slice(&value.to_le_bytes());
	}
	bytes
}

#[test]
fn reports_loader_and_needed_libraries_without_running_ldd() {
	let format = check(&dynamic(), Architecture::X86_64).unwrap();
	assert_eq!(
		format.interpreter.as_deref(),
		Some("/lib64/ld-linux-x86-64.so.2")
	);
	assert_eq!(format.needed, vec!["libc.so.6"]);
}

#[test]
fn rejects_unbounded_or_ambiguous_dynamic_metadata() {
	for (offset, value) in [
		(392, u64::MAX),
		(408, 1024 * 1024 + 1),
		(424, 11),
		(416, 29),
		(432, 1),
	] {
		let mut bytes = dynamic();
		bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
		assert!(
			check(&bytes, Architecture::X86_64).is_err(),
			"offset {offset}"
		);
	}
	let mut bytes = dynamic();
	bytes[321] = b'/';
	assert!(check(&bytes, Architecture::X86_64).is_err());
}
