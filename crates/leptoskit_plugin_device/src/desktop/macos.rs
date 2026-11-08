//! `sysctl` and host UUID reads on macOS.
#![allow(unsafe_code)]

use std::ffi::CString;
use std::fmt::Write;

/// Longest `sysctl` string read, in bytes.
const MAX_SYSCTL_BYTES: usize = 1024;

/// A NUL-terminated `sysctl` string such as `hw.model`.
pub(super) fn string(name: &str) -> Option<String> {
	let name = CString::new(name).ok()?;
	let mut size = 0_usize;

	// SAFETY: a null output buffer asks only for the value's size, which is
	// written to `size`.
	let status = unsafe {
		libc::sysctlbyname(
			name.as_ptr(),
			std::ptr::null_mut(),
			&raw mut size,
			std::ptr::null_mut(),
			0,
		)
	};

	if status != 0 || size == 0 || size > MAX_SYSCTL_BYTES {
		return None;
	}

	let mut buffer = vec![0_u8; size];

	// SAFETY: `buffer` holds `size` writable bytes, and the kernel writes at
	// most `size` of them, updating `size` to the count written.
	let status = unsafe {
		libc::sysctlbyname(
			name.as_ptr(),
			buffer.as_mut_ptr().cast(),
			&raw mut size,
			std::ptr::null_mut(),
			0,
		)
	};

	if status != 0 {
		return None;
	}

	buffer.truncate(size);
	let end = buffer
		.iter()
		.position(|byte| *byte == 0)
		.unwrap_or(buffer.len());
	buffer.truncate(end);
	String::from_utf8(buffer)
		.ok()
		.filter(|value| !value.is_empty())
}

/// A 32- or 64-bit `sysctl` number such as `hw.cpufrequency`.
pub(super) fn number(name: &str) -> Option<u64> {
	let name = CString::new(name).ok()?;
	let mut value = 0_u64;
	let mut size = size_of::<u64>();

	// SAFETY: `value` provides `size` writable bytes; the kernel writes at most
	// that many and reports how many it wrote.
	let status = unsafe {
		libc::sysctlbyname(
			name.as_ptr(),
			(&raw mut value).cast(),
			&raw mut size,
			std::ptr::null_mut(),
			0,
		)
	};

	match (status, size) {
		(0, 8) => Some(value),
		(0, 4) => Some(u64::from(value as u32)),
		_ => None,
	}
}

/// The hardware UUID (`IOPlatformUUID`) through `gethostuuid`. The App
/// Sandbox denies this call, so sandboxed apps get `None`.
pub(super) fn host_uuid() -> Option<String> {
	let mut uuid = [0_u8; 16];
	let timeout = libc::timespec {
		tv_sec: 1,
		tv_nsec: 0,
	};

	// SAFETY: `uuid` is the 16-byte buffer the call fills, and `timeout`
	// outlives the call.
	let status = unsafe { libc::gethostuuid(uuid.as_mut_ptr(), &raw const timeout) };

	(status == 0 && uuid.iter().any(|byte| *byte != 0)).then(|| format_uuid(&uuid))
}

/// Uppercase hyphenated UUID text, as `IOPlatformUUID` is written.
pub(super) fn format_uuid(uuid: &[u8; 16]) -> String {
	let mut hex = String::with_capacity(32);

	for byte in uuid {
		let _ = write!(hex, "{byte:02X}");
	}

	format!(
		"{}-{}-{}-{}-{}",
		&hex[..8],
		&hex[8..12],
		&hex[12..16],
		&hex[16..20],
		&hex[20..]
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_real_sysctl_values_and_rejects_unknown_names() {
		let model = string("hw.model").expect("every Mac reports hw.model");
		assert!(model.contains("Mac"), "unexpected model {model:?}");
		assert!(string("kern.osversion").is_some());
		assert_eq!(string("leptoskit.missing"), None);
		assert_eq!(string("bad\0name"), None);
		assert!(number("hw.ncpu").is_some_and(|count| count > 0));
		assert_eq!(number("leptoskit.missing"), None);
	}

	#[test]
	fn formats_uuids_like_io_platform_uuid() {
		let uuid = [
			0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x10, 0x32, 0x54, 0x76, 0x98, 0xba,
			0xdc, 0xfe,
		];
		assert_eq!(format_uuid(&uuid), "01234567-89AB-CDEF-1032-547698BADCFE");
	}
}
