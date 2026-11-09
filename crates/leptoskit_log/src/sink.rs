//! The platform's own log, one function per platform family.
//!
//! None of these may panic: the panic hook writes through them.
#![allow(unsafe_code)]

use log::Level;

/// Writes one record under `tag`.
pub(crate) fn write(level: Level, tag: &str, message: &str) {
	platform::write(level, tag, message);
}

/// Copies `text` into a C string, replacing interior NULs, which would end the
/// string early.
#[cfg(any(target_os = "android", target_vendor = "apple"))]
fn c_string(text: &str) -> std::ffi::CString {
	std::ffi::CString::new(text.replace('\0', " ")).unwrap_or_default()
}

/// Logcat, through `liblog`.
#[cfg(target_os = "android")]
mod platform {
	use std::ffi::c_char;
	use std::ffi::c_int;

	use log::Level;

	use super::c_string;

	/// Logcat drops what is past about 4 KiB of one entry, so longer
	/// messages, such as a backtrace, are written as several.
	const MAX_ENTRY_BYTES: usize = 4000;

	#[link(name = "log")]
	unsafe extern "C" {
		fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
	}

	pub(super) fn write(level: Level, tag: &str, message: &str) {
		let priority = match level {
			Level::Error => 6,
			Level::Warn => 5,
			Level::Info => 4,
			Level::Debug => 3,
			Level::Trace => 2,
		};
		let tag = c_string(tag);

		for entry in super::entries(message, MAX_ENTRY_BYTES) {
			let text = c_string(entry);
			// SAFETY: Both are NUL-terminated strings that outlive the call.
			unsafe {
				__android_log_write(priority, tag.as_ptr(), text.as_ptr());
			}
		}
	}
}

/// The unified log, through the `os_log` shim `build.rs` compiles, and
/// standard error as well when a person is reading it in a terminal.
#[cfg(target_vendor = "apple")]
mod platform {
	use std::collections::BTreeMap;
	use std::ffi::c_char;
	use std::ffi::c_void;
	use std::io::IsTerminal;
	use std::sync::Mutex;
	use std::sync::PoisonError;

	use log::Level;

	use super::c_string;

	/// The unified log's subsystem; each tag is a category in it, so
	/// `log stream --predicate 'subsystem == "dev.leptoskit"'` shows them all.
	const SUBSYSTEM: &std::ffi::CStr = c"dev.leptoskit";

	unsafe extern "C" {
		fn leptoskit_os_log_create(
			subsystem: *const c_char,
			category: *const c_char,
		) -> *mut c_void;
		fn leptoskit_os_log_write(log: *mut c_void, kind: u8, message: *const c_char);
	}

	/// One `os_log_t` per category, created once and kept for the process.
	static LOGS: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());

	fn log_for(tag: &str) -> *mut c_void {
		let mut logs = LOGS.lock().unwrap_or_else(PoisonError::into_inner);

		if let Some(log) = logs.get(tag) {
			return *log as *mut c_void;
		}

		let category = c_string(tag);
		// SAFETY: Both are NUL-terminated strings that outlive the call; the
		// returned object is retained for the process.
		let log = unsafe { leptoskit_os_log_create(SUBSYSTEM.as_ptr(), category.as_ptr()) };
		logs.insert(tag.to_owned(), log as usize);
		log
	}

	pub(super) fn write(level: Level, tag: &str, message: &str) {
		// `OS_LOG_TYPE_*`: warnings are `DEFAULT`, which the system keeps, as
		// it keeps errors; `INFO` and `DEBUG` show only while streaming.
		let kind = match level {
			Level::Error => 0x10,
			Level::Warn => 0x00,
			Level::Info => 0x01,
			Level::Debug | Level::Trace => 0x02,
		};
		let text = c_string(message);
		// SAFETY: The log object lives for the process and the message is a
		// NUL-terminated string that outlives the call.
		unsafe { leptoskit_os_log_write(log_for(tag), kind, text.as_ptr()) };

		if std::io::stderr().is_terminal() {
			super::stderr(level, tag, message);
		}
	}
}

/// Standard error, for Linux, Windows, and HarmonyOS hosts.
#[cfg(not(any(target_os = "android", target_vendor = "apple", target_arch = "wasm32")))]
mod platform {
	pub(super) use super::stderr as write;
}

/// The browser console, in a web page.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod platform {
	use log::Level;
	use wasm_bindgen::JsValue;

	pub(super) fn write(level: Level, tag: &str, message: &str) {
		let line = JsValue::from_str(&format!("[{tag}] {message}"));

		match level {
			Level::Error => web_sys::console::error_1(&line),
			Level::Warn => web_sys::console::warn_1(&line),
			Level::Info => web_sys::console::info_1(&line),
			Level::Debug | Level::Trace => web_sys::console::debug_1(&line),
		}
	}
}

/// A native guest: Wasm with no console and no host log to reach.
#[cfg(all(target_arch = "wasm32", not(feature = "web")))]
mod platform {
	use log::Level;

	pub(super) fn write(_: Level, _: &str, _: &str) {}
}

/// One line on standard error, through the test harness's capture when a
/// test is running.
#[cfg(not(target_arch = "wasm32"))]
fn stderr(level: Level, tag: &str, message: &str) {
	eprintln!("{level:<5} {tag}: {message}");
}

/// Splits `message` into pieces of at most `limit` bytes, each ending on a
/// character boundary, preferring to end after a newline.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn entries(message: &str, limit: usize) -> impl Iterator<Item = &str> {
	let mut rest = message;

	std::iter::from_fn(move || {
		if rest.is_empty() {
			return None;
		}

		if rest.len() <= limit {
			return Some(std::mem::take(&mut rest));
		}

		let mut end = limit;

		while !rest.is_char_boundary(end) {
			end -= 1;
		}

		if let Some(newline) = rest[..end].rfind('\n') {
			end = newline + 1;
		}

		let (entry, remainder) = rest.split_at(end);
		rest = remainder;
		Some(entry)
	})
}

#[cfg(test)]
mod tests {
	use super::entries;

	#[test]
	fn long_messages_split_on_character_boundaries_and_lines() {
		let message = format!("{}\n{}", "é".repeat(30), "x".repeat(70));
		let pieces: Vec<&str> = entries(&message, 64).collect();

		assert!(pieces.iter().all(|piece| piece.len() <= 64), "{pieces:?}");
		assert_eq!(pieces.concat(), message);
		assert!(pieces[0].ends_with('\n'), "{pieces:?}");
	}

	#[test]
	fn a_message_without_room_for_a_newline_is_cut_between_characters() {
		let message = "ü".repeat(100);
		let pieces: Vec<&str> = entries(&message, 7).collect();

		assert!(
			pieces
				.iter()
				.all(|piece| piece.len() <= 7 && !piece.is_empty())
		);
		assert_eq!(pieces.concat(), message);
	}
}
