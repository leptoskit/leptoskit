//! Compiled guest acceptance for the native runtime timer.
//!
//! The host in `crates/leptoskit_wasm/tests/timers.rs` enables the reserved
//! timer plugin, routes this guest's sleeps, and checks that they really wait.
//! The guest also abandons a one-second timer, which must reach the host as a
//! cancellation rather than a completion.
#![cfg(target_arch = "wasm32")]
#![allow(unsafe_code)] // Exported C names form the interpreter's entry points.

use std::cell::Cell;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Waker;
use std::time::Duration;

use leptoskit_bridge::guest::Client;
use leptoskit_bridge::timer::SLEEP;
use leptoskit_bridge::timer::TIMER_PLUGIN;
use leptoskit_bridge::timer::encode;

type Task = Pin<Box<dyn Future<Output = ()>>>;

thread_local! {
	static TASK: RefCell<Option<Task>> = const { RefCell::new(None) };
	static DONE: Cell<bool> = const { Cell::new(false) };
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	TASK.with_borrow_mut(|task| {
		*task = Some(Box::pin(async {
			// Submit a long sleep, keep it across a shorter wait, then drop it
			// while the host already runs it. The host must cancel it rather
			// than let it keep the guest alive or complete later.
			let client = Client::new(TIMER_PLUGIN).expect("timer plugin");
			let payload = encode(Duration::from_secs(1)).expect("encode");
			let abandoned = client.call(SLEEP, &payload).expect("submit");

			leptoskit_bridge::guest::sleep(Duration::from_millis(15))
				.await
				.expect("first sleep");
			drop(abandoned);
			leptoskit_bridge::guest::sleep(Duration::from_millis(1))
				.await
				.expect("second sleep");
			DONE.with(|done| done.set(true));
		}));
	});
	leptoskit_poll();
}

/// The host calls this after each plugin reply, through `WasmHost`.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_poll() {
	TASK.with_borrow_mut(|task| {
		if task.as_mut().is_some_and(|future| {
			future
				.as_mut()
				.poll(&mut Context::from_waker(Waker::noop()))
				.is_ready()
		}) {
			*task = None;
		}
	});
}

#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(_id: i64, _kind: i32, _ptr: i32, _len: i32) -> i32 {
	DONE.with(|done| i32::from(done.get()))
}
