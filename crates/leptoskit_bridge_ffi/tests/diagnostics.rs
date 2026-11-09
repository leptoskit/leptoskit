//! What the runtime logs when a host calls across the FFI: refused plugins,
//! failed calls, records the Kotlin and Swift hosts write, and an app that
//! cannot be created or stops drawing.
//!
//! The test binary has one logger, so each test looks for records only it
//! writes: a plugin name, a size, a message.

#![allow(unsafe_code)]

use std::mem::MaybeUninit;
use std::ptr;

use leptoskit_bridge::Error;
use leptoskit_bridge_ffi::NativeRequest;
use leptoskit_bridge_ffi::lk_bridge_complete;
use leptoskit_bridge_ffi::lk_bridge_create;
use leptoskit_bridge_ffi::lk_bridge_destroy;
use leptoskit_bridge_ffi::lk_bridge_grant;
use leptoskit_bridge_ffi::lk_bridge_next_request;
use leptoskit_bridge_ffi::lk_bridge_register;
use leptoskit_bridge_ffi::lk_bridge_submit;
use leptoskit_bridge_ffi::lk_buffer_release;
use leptoskit_bridge_ffi::lk_log_write;
use leptoskit_log::capture::Records;
use log::Level;

/// Registers `name` with one method, 3, through the FFI.
fn register(bridge: *mut leptoskit_bridge::Bridge, name: &[u8]) -> i64 {
	let methods = [3_u32];
	// SAFETY: The bridge is live and both arrays outlive the call.
	unsafe {
		lk_bridge_register(
			bridge,
			name.as_ptr(),
			name.len(),
			methods.as_ptr(),
			methods.len(),
		)
	}
}

#[test]
fn every_error_a_host_completes_with_names_its_plugin_method_and_call() {
	let records = leptoskit_log::capture::start();
	let bridge = lk_bridge_create();
	let plugin = register(bridge, b"ffi-diagnostics");
	assert!(plugin >= 0, "{plugin}");
	let plugin = u32::try_from(plugin).unwrap();

	// SAFETY: The bridge is live until it is destroyed below, and every
	// request's payload is released once.
	unsafe {
		assert_eq!(lk_bridge_grant(bridge, plugin, 3, true), 0);

		for error in Error::ALL {
			let id = lk_bridge_submit(bridge, plugin, 3, b"{}".as_ptr(), 2);
			assert!(id > 0, "{id}");
			let mut request = MaybeUninit::<NativeRequest>::uninit();
			assert_eq!(lk_bridge_next_request(bridge, request.as_mut_ptr()), 1);
			let request = request.assume_init();
			lk_buffer_release(request.payload);

			// What a Kotlin or Swift plugin sends when it fails.
			assert_eq!(
				lk_bridge_complete(bridge, request.id, error as i32, ptr::null(), 0),
				0
			);

			let record = records.expect(
				"leptoskit/plugin/ffi-diagnostics",
				&format!(
					"call #{} to `ffi-diagnostics` method 3 failed with {error:?}: {error}",
					request.id
				),
			);
			let level = if error == Error::Cancelled {
				Level::Debug
			} else {
				Level::Warn
			};
			assert_eq!(record.level, level, "{error:?}");
			assert!(!error.to_string().is_empty(), "{error:?} has no message");
		}

		lk_bridge_destroy(bridge);
	}
}

#[test]
fn a_name_refused_before_the_bridge_sees_it_is_logged() {
	let records = leptoskit_log::capture::start();
	let bridge = lk_bridge_create();

	assert_eq!(
		register(bridge, &[b'n'; 257]),
		-i64::from(Error::TooLarge as i32)
	);
	let record = records.expect(
		"leptoskit/bridge",
		"cannot register a plugin: its name is 257 bytes and its method count 1",
	);
	assert_eq!(record.level, Level::Error);

	assert_eq!(
		register(bridge, b"ffi-\xffbroken"),
		-i64::from(Error::InvalidArgument as i32)
	);
	records.expect(
		"leptoskit/bridge",
		"cannot register plugin `ffi-\u{fffd}broken`: its name is not UTF-8",
	);

	// SAFETY: The bridge was created above and is destroyed once.
	unsafe { lk_bridge_destroy(bridge) };
}

/// Writes one record the way the Kotlin and Swift hosts do.
fn host_log(level: u32, target: &[u8], message: &[u8]) {
	// SAFETY: Both slices outlive the call.
	unsafe {
		lk_log_write(
			level,
			target.as_ptr(),
			target.len(),
			message.as_ptr(),
			message.len(),
		);
	}
}

#[test]
fn host_records_keep_their_level_and_tag() {
	let records = leptoskit_log::capture::start();

	for (code, level) in [
		(1, Level::Error),
		(2, Level::Warn),
		(3, Level::Info),
		(4, Level::Debug),
		(5, Level::Trace),
		(0, Level::Error),
		(99, Level::Error),
	] {
		let message = format!("host-record {code}");
		host_log(code, b"leptoskit/gpu", message.as_bytes());
		assert_eq!(records.expect("leptoskit/gpu", &message).level, level);
	}

	host_log(2, b"", b"host-record without a tag");
	records.expect("leptoskit/bridge", "host-record without a tag");

	host_log(1, b"leptoskit/bridge", b"host-record \xff\xfe");
	records.expect("leptoskit/bridge", "host-record \u{fffd}\u{fffd}");

	// A null message of no length is an empty record, not a crash.
	// SAFETY: Null with length 0 is allowed.
	unsafe { lk_log_write(1, ptr::null(), 0, ptr::null(), 0) };
	assert!(
		records
			.all()
			.iter()
			.any(|record| { record.target == "leptoskit/bridge" && record.message.is_empty() })
	);
}

/// Keeps the capturing logger's type in view, so a change to it is a
/// compile error here rather than a silently empty record list.
const _: fn() -> Records = leptoskit_log::capture::start;

#[cfg(feature = "app")]
mod app {
	use std::mem::MaybeUninit;
	use std::ptr;

	use leptoskit_bridge::Bytes;
	use leptoskit_bridge::Error;
	use leptoskit_bridge::MethodId;
	use leptoskit_bridge::Plugin;
	use leptoskit_bridge_ffi::Buffer;
	use leptoskit_bridge_ffi::NativeRequest;
	use leptoskit_bridge_ffi::app::*;
	use leptoskit_bridge_ffi::lk_buffer_release;
	use log::Level;

	/// Renders one view, then calls method 7 of the plugin named at byte 16.
	fn guest(plugin: &str) -> String {
		format!(
			r#"(module
 (import "leptoskit" "node_create" (func $create (param i64 i32)))
 (import "leptoskit" "commit" (func $commit))
 (import "leptoskit_plugins_v1" "resolve" (func $resolve (param i32 i32) (result i64)))
 (import "leptoskit_plugins_v1" "submit" (func $submit (param i32 i32 i32 i32) (result i64)))
 (memory (export "memory") 1)
 (data (i32.const 16) "{plugin}")
 (func (export "leptoskit_main")
   (call $create (i64.const 1) (i32.const 0)) (call $commit)
   (drop (call $submit (i32.wrap_i64 (call $resolve (i32.const 16) (i32.const {len}))) (i32.const 7) (i32.const 0) (i32.const 0))))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
 (func (export "leptoskit_plugins_ready"))
 (func (export "leptoskit_poll"))
)"#,
			len = plugin.len()
		)
	}

	/// An inline Rust plugin whose every call fails with one error.
	struct Fails {
		name: &'static str,
		error: Error,
	}

	impl Plugin for Fails {
		fn name(&self) -> &'static str {
			self.name
		}

		fn methods(&self) -> &'static [MethodId] {
			&[MethodId(7)]
		}

		fn invoke(&mut self, _: MethodId, _: Bytes) -> Result<Bytes, Error> {
			Err(self.error)
		}
	}

	fn take(buffer: Buffer) -> String {
		// SAFETY: The FFI returned this live owned buffer, released exactly
		// once.
		unsafe {
			let bytes = std::slice::from_raw_parts(buffer.data, buffer.len).to_vec();
			lk_buffer_release(buffer);
			String::from_utf8(bytes).unwrap()
		}
	}

	#[test]
	fn an_app_that_cannot_be_created_says_why() {
		let records = leptoskit_log::capture::start();
		let mut error = MaybeUninit::uninit();
		// SAFETY: A null bundle of no length is the case under test.
		unsafe {
			assert!(create(ptr::null(), 0, error.as_mut_ptr(), |_| Ok(())).is_null());
			take(error.assume_init());
		}
		let record = records.expect(
			"leptoskit/bridge",
			"cannot create the app: empty application bundle",
		);
		assert_eq!(record.level, Level::Error);

		let broken = b"\0asm\x01\0\0\0 not a module";
		let mut error = MaybeUninit::uninit();
		// SAFETY: The bundle outlives the call; the diagnostic is released.
		let diagnostic = unsafe {
			assert!(
				create(
					broken.as_ptr(),
					broken.len(),
					error.as_mut_ptr(),
					|_| Ok(())
				)
				.is_null()
			);
			take(error.assume_init())
		};
		records.expect(
			"leptoskit/bridge",
			&format!("cannot create the app: {diagnostic}"),
		);
	}

	#[test]
	fn a_running_app_leaves_breadcrumbs_and_names_each_failed_call() {
		let records = leptoskit_log::capture::start();

		for error in Error::ALL {
			let name: &'static str =
				Box::leak(format!("inline-{error:?}").to_lowercase().into_boxed_str());
			let wasm = guest(name);
			let mut diagnostic = MaybeUninit::uninit();
			// SAFETY: Every app access is exclusive and the module outlives
			// the call.
			unsafe {
				let app = create(wasm.as_ptr(), wasm.len(), diagnostic.as_mut_ptr(), |app| {
					app.install(Box::new(Fails { name, error })).map(|_| ())
				});
				assert!(!app.is_null());
				records.expect(&format!("leptoskit/plugin/{name}"), "registered plugin");
				records.expect(
					"leptoskit/bridge",
					&format!("created the app from {} bytes", wasm.len()),
				);

				assert_eq!(
					lk_app_start(app, 320.0, 480.0),
					0,
					"{}",
					take(lk_app_error(app))
				);
				records.expect("leptoskit/bridge", "starting the guest at 320×480");
				take(lk_app_render(app));
				records.expect("leptoskit/render", "first commit: ");

				// The inline plugin answers the guest's call here.
				let mut request = MaybeUninit::<NativeRequest>::uninit();
				assert_eq!(lk_app_next_request(app, request.as_mut_ptr()), 0);
				let record = records.expect(
					&format!("leptoskit/plugin/{name}"),
					&format!("to `{name}` method 7 failed with {error:?}: {error}"),
				);
				assert!(record.message.starts_with("call #"), "{}", record.message);
				lk_app_destroy(app);
			}
		}
	}

	#[test]
	fn a_guest_that_traps_is_logged_with_its_export() {
		let records = leptoskit_log::capture::start();
		let wasm = r#"(module
 (memory (export "memory") 1)
 (func (export "leptoskit_main") unreachable)
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) i32.const 0)
 (func (export "leptoskit_plugins_ready"))
 (func (export "leptoskit_poll"))
)"#;
		let mut diagnostic = MaybeUninit::uninit();
		// SAFETY: Every app access is exclusive and the module outlives the
		// call.
		unsafe {
			let app = create(wasm.as_ptr(), wasm.len(), diagnostic.as_mut_ptr(), |_| {
				Ok(())
			});
			assert!(!app.is_null());
			assert_eq!(lk_app_start(app, 64.0, 64.0), -1);
			let error = take(lk_app_error(app));
			assert!(error.contains("leptoskit_main"), "{error}");
			let record = records.expect("leptoskit/bridge", &error);
			assert_eq!(record.level, Level::Error);
			lk_app_destroy(app);
		}
	}
}
