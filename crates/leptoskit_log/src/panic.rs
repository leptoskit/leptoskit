//! The panic hook: logs a panic, from any thread, before the process unwinds
//! or aborts, then runs the hook it replaced.

use std::panic::PanicHookInfo;

use crate::target;

/// Installs the hook, keeping the one it replaces to run after it.
pub(crate) fn install() {
	let previous = std::panic::take_hook();

	std::panic::set_hook(Box::new(move |info| {
		log::error!(target: target::PANIC, "{}", describe(info));
		previous(info);
	}));
}

/// What a panic says: its thread, location, and message, then the stack.
fn describe(info: &PanicHookInfo<'_>) -> String {
	let thread = std::thread::current();
	let thread = thread.name().unwrap_or("<unnamed>");
	let location = info.location().map_or_else(
		|| "an unknown location".to_owned(),
		|location| {
			format!(
				"{}:{}:{}",
				location.file(),
				location.line(),
				location.column()
			)
		},
	);
	let message = info
		.payload()
		.downcast_ref::<&str>()
		.copied()
		.or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
		.unwrap_or("Box<dyn Any>");

	format!(
		"thread '{thread}' panicked at {location}:\n{message}\n{}",
		stack()
	)
}

/// The stack where the panic happened. A release build without symbols
/// prints addresses, which `ndk-stack` or `atos` resolve.
#[cfg(not(target_arch = "wasm32"))]
fn stack() -> String {
	format!(
		"stack backtrace:\n{}",
		std::backtrace::Backtrace::force_capture()
	)
}

/// The JavaScript stack, which names the Wasm functions on it.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
fn stack() -> String {
	use wasm_bindgen::prelude::wasm_bindgen;

	#[wasm_bindgen]
	extern "C" {
		type Error;

		#[wasm_bindgen(constructor)]
		fn new() -> Error;

		#[wasm_bindgen(method, getter)]
		fn stack(this: &Error) -> String;
	}

	format!("stack:\n{}", Error::new().stack())
}

/// A native guest has no stack to read; the host reports its trap.
#[cfg(all(target_arch = "wasm32", not(feature = "web")))]
fn stack() -> String {
	String::new()
}
