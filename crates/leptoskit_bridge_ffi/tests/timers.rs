#![cfg(feature = "app")]
#![allow(unsafe_code)]

use std::mem::MaybeUninit;
use std::ptr;
use std::time::Duration;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::CallId;
use leptoskit_bridge::timer::SLEEP;
use leptoskit_bridge::timer::TIMER_PLUGIN;
use leptoskit_bridge::timer::encode;
use leptoskit_bridge_ffi::app::*;

const GUEST: &str = r#"
(module
  (func (export "leptoskit_main"))
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) (i32.const 0)))
"#;

fn create_app() -> *mut App {
	let mut diagnostic = MaybeUninit::uninit();
	// SAFETY: The input lives for the call and the app is exclusively used.
	unsafe {
		let app = create(GUEST.as_ptr(), GUEST.len(), diagnostic.as_mut_ptr(), |_| {
			Ok(())
		});
		assert!(!app.is_null());
		app
	}
}

fn submit(app: *mut App, millis: u64) -> CallId {
	// SAFETY: The test owns the app exclusively.
	unsafe { app.as_mut() }
		.expect("live app")
		.with_host(|host| {
			let timer = host.plugins().resolve(TIMER_PLUGIN).expect("timer plugin");
			let payload = encode(Duration::from_millis(millis)).expect("encode");
			host.plugins()
				.submit(timer, SLEEP, Bytes::copy_from_slice(&payload))
				.expect("submit")
		})
}

fn ready(app: *mut App, id: CallId) -> bool {
	// SAFETY: The test owns the app exclusively.
	unsafe { app.as_mut() }
		.expect("live app")
		.with_host(|host| matches!(host.plugins().result(id), Ok(Some(_))))
}

#[test]
fn a_timer_never_reaches_the_platform_and_completes_when_due() {
	let app = create_app();
	// SAFETY: The test owns the app exclusively.
	unsafe {
		assert_eq!(lk_app_enable_timers(app), 0);
		assert_eq!(lk_app_enable_timers(app), 0, "enabling again keeps it");
		assert_eq!(lk_app_start(app, 100.0, 100.0), 0);
		let id = submit(app, 10);

		let mut request = MaybeUninit::uninit();
		assert_eq!(
			lk_app_next_request(app, request.as_mut_ptr()),
			0,
			"the host keeps the timer request"
		);

		let deadline = std::time::Instant::now() + Duration::from_secs(1);

		while !ready(app, id) && std::time::Instant::now() < deadline {
			assert_eq!(lk_app_poll_timers(app), 0);
			std::thread::sleep(Duration::from_millis(2));
		}

		assert!(ready(app, id), "the timer came due");
		let reply = (*app)
			.with_host(|host| host.plugins().take_result(id))
			.unwrap()
			.unwrap()
			.unwrap();
		assert!(reply.is_empty());
		assert_eq!(lk_app_poll_timers(app), 0);
		lk_app_destroy(app);
	}
}

#[test]
fn cancelling_a_timer_removes_it_before_the_platform_or_its_deadline() {
	let app = create_app();
	// SAFETY: The test owns the app exclusively.
	unsafe {
		assert_eq!(lk_app_enable_timers(app), 0);
		assert_eq!(lk_app_start(app, 100.0, 100.0), 0);
		let id = submit(app, 30);
		let mut request = MaybeUninit::uninit();
		assert_eq!(lk_app_next_request(app, request.as_mut_ptr()), 0);
		(*app)
			.with_host(|host| host.plugins().cancel(id))
			.expect("cancel the running timer");
		assert_eq!(
			(*app).next_cancelled(),
			None,
			"a timer cancellation stops at the host"
		);
		std::thread::sleep(Duration::from_millis(50));
		assert_eq!(lk_app_poll_timers(app), 0);
		assert!(!ready(app, id));
		lk_app_destroy(app);
	}
}

#[test]
fn a_host_without_timers_reports_the_plugin_unavailable() {
	let app = create_app();
	// SAFETY: The test owns the app exclusively.
	unsafe {
		assert!(
			(*app)
				.with_host(|host| host.plugins().resolve(TIMER_PLUGIN))
				.is_none()
		);
		assert_eq!(lk_app_start(app, 100.0, 100.0), 0);
		let mut request = MaybeUninit::uninit();
		assert_eq!(lk_app_next_request(app, request.as_mut_ptr()), 0);
		assert_eq!(lk_app_poll_timers(app), 0);
		lk_app_destroy(app);
	}
}

#[test]
fn the_timer_api_rejects_a_missing_app() {
	// SAFETY: Each function checks for a null app before using it.
	unsafe {
		assert_eq!(lk_app_enable_timers(ptr::null_mut()), -1);
		assert_eq!(lk_app_poll_timers(ptr::null_mut()), -1);
	}
}

#[test]
fn owner_thread_deadlines_disappear_after_cancellation_and_close() {
	let app = create_app();
	// SAFETY: The test owns the app and never uses its pointer after destroy.
	unsafe {
		assert_eq!(lk_app_timer_delay_ms(app), -1);
		assert_eq!(lk_app_enable_timers(app), 0);
		assert_eq!(lk_app_start(app, 100.0, 100.0), 0);
		let id = submit(app, 30_000);
		let mut request = MaybeUninit::uninit();
		assert_eq!(lk_app_next_request(app, request.as_mut_ptr()), 0);
		assert!((0..=30_000).contains(&lk_app_timer_delay_ms(app)));
		(*app).with_host(|host| host.plugins().cancel(id)).unwrap();
		assert_eq!((*app).next_cancelled(), None);
		assert_eq!(lk_app_timer_delay_ms(app), -1);
		lk_app_destroy(app);
		assert_eq!(lk_app_timer_delay_ms(ptr::null_mut()), -1);
	}
}
