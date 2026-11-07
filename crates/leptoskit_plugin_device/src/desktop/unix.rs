//! Volume space and small system files on macOS and Linux.
#![allow(unsafe_code)]

use std::ffi::CString;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Largest system file read, in bytes. Real `os-release` and `machine-id`
/// files are well under a kilobyte.
const MAX_FILE_BYTES: usize = 64 * 1024;

/// Total size and bytes available to unprivileged writers on the volume
/// holding `path`.
// The `statvfs` field widths differ between macOS and Linux.
#[allow(clippy::useless_conversion)]
pub(super) fn disk(path: &Path) -> Option<(u64, u64)> {
	let path = CString::new(path.as_os_str().as_bytes()).ok()?;
	// SAFETY: `statvfs` is plain old data; all-zero is a valid value.
	let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };

	// SAFETY: `path` is NUL-terminated and `stats` is writable for the call.
	if unsafe { libc::statvfs(path.as_ptr(), &raw mut stats) } != 0 {
		return None;
	}

	let block = u64::from(stats.f_frsize);
	let total = u64::from(stats.f_blocks).checked_mul(block)?;
	let free = u64::from(stats.f_bavail).checked_mul(block)?;
	Some((total, free.min(total)))
}

/// A small text file, or `None` when it is missing, unreadable, too large or
/// not UTF-8.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(super) fn read_text(path: &str) -> Option<String> {
	let file = std::fs::File::open(path).ok()?;
	let mut text = String::new();
	let limit = MAX_FILE_BYTES as u64;
	file.take(limit + 1).read_to_string(&mut text).ok()?;
	(text.len() as u64 <= limit).then_some(text)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_the_root_volume_and_rejects_missing_paths() {
		let (total, free) = disk(Path::new("/")).expect("the root volume has a size");
		assert!(total > 0 && free <= total);
		assert_eq!(disk(Path::new("/leptoskit/missing/volume")), None);
	}

	#[test]
	fn reads_bounded_text_files() {
		let directory =
			std::env::temp_dir().join(format!("leptoskit-device-{}", std::process::id()));
		std::fs::create_dir_all(&directory).unwrap();
		let small = directory.join("small");
		let large = directory.join("large");
		std::fs::write(&small, "ID=test\n").unwrap();
		std::fs::write(&large, vec![b'x'; MAX_FILE_BYTES + 1]).unwrap();
		assert_eq!(
			read_text(small.to_str().unwrap()).as_deref(),
			Some("ID=test\n")
		);
		assert_eq!(read_text(large.to_str().unwrap()), None);
		assert_eq!(read_text("/leptoskit/missing/file"), None);
		std::fs::remove_dir_all(directory).unwrap();
	}
}
