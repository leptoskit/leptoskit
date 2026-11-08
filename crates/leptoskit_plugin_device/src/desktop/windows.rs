//! Registry and volume reads on Windows.
#![allow(unsafe_code)]

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE;
use windows_sys::Win32::System::Registry::RRF_RT_REG_DWORD;
use windows_sys::Win32::System::Registry::RRF_RT_REG_SZ;
use windows_sys::Win32::System::Registry::RRF_SUBKEY_WOW6464KEY;
use windows_sys::Win32::System::Registry::RegGetValueW;

use crate::MAX_IDENTIFIER_BYTES;
use crate::MAX_TEXT_BYTES;
use crate::Windows;

const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const SQM_CLIENT: &str = r"SOFTWARE\Microsoft\SQMClient";
/// Longest registry string read, in UTF-16 units.
const MAX_REGISTRY_UNITS: usize = 1024;

fn wide(text: &str) -> Vec<u16> {
	text.encode_utf16().chain(Some(0)).collect()
}

/// A `REG_SZ` value from the 64-bit view of `HKLM\key`.
fn string(key: &str, value: &str) -> Option<String> {
	let key = wide(key);
	let value = wide(value);
	let mut buffer = vec![0_u16; MAX_REGISTRY_UNITS];
	let mut size = (buffer.len() * 2) as u32;

	// SAFETY: both names are NUL-terminated, `buffer` holds `size` writable
	// bytes, and the call writes at most `size` bytes and updates it.
	let status = unsafe {
		RegGetValueW(
			HKEY_LOCAL_MACHINE,
			key.as_ptr(),
			value.as_ptr(),
			RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
			std::ptr::null_mut(),
			buffer.as_mut_ptr().cast(),
			&raw mut size,
		)
	};

	if status != ERROR_SUCCESS {
		return None;
	}

	buffer.truncate(size as usize / 2);
	let end = buffer
		.iter()
		.position(|unit| *unit == 0)
		.unwrap_or(buffer.len());
	String::from_utf16(&buffer[..end])
		.ok()
		.map(|text| text.trim().to_owned())
		.filter(|text| !text.is_empty() && text.len() <= MAX_TEXT_BYTES)
}

/// A `REG_DWORD` value from the 64-bit view of `HKLM\key`.
fn dword(key: &str, value: &str) -> Option<u32> {
	let key = wide(key);
	let value = wide(value);
	let mut data = 0_u32;
	let mut size = size_of::<u32>() as u32;

	// SAFETY: both names are NUL-terminated and `data` provides `size`
	// writable bytes.
	let status = unsafe {
		RegGetValueW(
			HKEY_LOCAL_MACHINE,
			key.as_ptr(),
			value.as_ptr(),
			RRF_RT_REG_DWORD | RRF_SUBKEY_WOW6464KEY,
			std::ptr::null_mut(),
			(&raw mut data).cast(),
			&raw mut size,
		)
	};

	(status == ERROR_SUCCESS).then_some(data)
}

/// Release details from `CurrentVersion`.
pub(super) fn details() -> Windows {
	let build_number =
		string(CURRENT_VERSION, "CurrentBuildNumber").and_then(|value| value.parse().ok());

	Windows {
		major_version: dword(CURRENT_VERSION, "CurrentMajorVersionNumber"),
		minor_version: dword(CURRENT_VERSION, "CurrentMinorVersionNumber"),
		build_number,
		update_build_revision: dword(CURRENT_VERSION, "UBR"),
		product_name: string(CURRENT_VERSION, "ProductName")
			.map(|name| crate::desktop::windows_product_name(name, build_number)),
		display_version: string(CURRENT_VERSION, "DisplayVersion"),
		edition_id: string(CURRENT_VERSION, "EditionID"),
		release_id: string(CURRENT_VERSION, "ReleaseId"),
		build_lab: string(CURRENT_VERSION, "BuildLab"),
		build_lab_ex: string(CURRENT_VERSION, "BuildLabEx"),
	}
}

/// `SQMClient\MachineId`, a GUID Windows telemetry assigns once per
/// installation.
pub(super) fn machine_id() -> Option<String> {
	string(SQM_CLIENT, "MachineId").filter(|value| value.len() <= MAX_IDENTIFIER_BYTES)
}

/// Total size and bytes available to this user on the volume holding `path`.
pub(super) fn disk(path: &Path) -> Option<(u64, u64)> {
	let path = path
		.as_os_str()
		.encode_wide()
		.chain(Some(0))
		.collect::<Vec<_>>();
	let mut available = 0_u64;
	let mut total = 0_u64;

	// SAFETY: `path` is NUL-terminated and both outputs are writable u64s;
	// the total-free output is optional and passed as null.
	let succeeded = unsafe {
		GetDiskFreeSpaceExW(
			path.as_ptr(),
			&raw mut available,
			&raw mut total,
			std::ptr::null_mut(),
		)
	};

	(succeeded != 0).then_some((total, available.min(total)))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_the_real_release_and_volume() {
		let details = details();
		assert!(details.build_number.is_some_and(|build| build > 0));
		assert!(details.product_name.is_some());
		let (total, free) = disk(Path::new("C:\\")).expect("the system volume has a size");
		assert!(total > 0 && free <= total);
		assert_eq!(string(CURRENT_VERSION, "LeptosKitMissingValue"), None);
		assert_eq!(
			dword(CURRENT_VERSION, "ProductName"),
			None,
			"a string is not a DWORD"
		);
	}
}
