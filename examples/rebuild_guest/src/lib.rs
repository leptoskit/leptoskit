//! Compiled guest fixture: screen state across rebuilds in the native
//! interpreter.
//!
//! The counter screen reads its state in the body, so every press rebuilds
//! it. One count comes from `remember` and survives the rebuild; the other is
//! a `RwSignal::new` in the screen body, which the rebuild creates again. A
//! second screen stands in for another route: leaving the counter releases
//! what it remembered, and coming back starts over.
#![cfg(target_arch = "wasm32")]
#![allow(unsafe_code)] // Exported C names form the interpreter's entry points.

use leptoskit::component;
use leptoskit::prelude::*;
use leptoskit::view;

/// Mounts the counter, with a signal that says which screen is showing.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	let counting = RwSignal::new(true);
	leptoskit::guest::mount(move || {
		if counting.get() {
			view! { <Counter counting=counting /> }
		} else {
			view! { <Elsewhere counting=counting /> }
		}
	});
}

/// Two counts, one remembered and one not, and a way to leave.
#[component]
fn Counter(counting: RwSignal<bool>) -> Element {
	let remembered = remember(|| 0_u32);
	let fresh = RwSignal::new(0_u32);
	let tap = move || {
		remembered.update(|count| *count += 1);
		fresh.update(|count| *count += 1);
	};
	view! {
		<View class="flex-col gap-2 p-4">
			<Button on:press=tap>"Tap"</Button>
			<Text>"Remembered: " {remembered.get()}</Text>
			<Text>"Fresh: " {fresh.get()}</Text>
			<Button on:press=move || counting.set(false)>"Leave"</Button>
		</View>
	}
}

/// Another route, which remembers nothing.
#[component]
fn Elsewhere(counting: RwSignal<bool>) -> Element {
	view! {
		<View class="flex-col gap-2 p-4">
			<Button on:press=move || counting.set(true)>"Back"</Button>
		</View>
	}
}

/// Forwards platform-neutral events to the shared tree's handlers.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	leptoskit::guest::dispatch(node, kind, ptr, len)
}

/// Allocates the interpreter's event payload buffer.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	leptoskit::guest::alloc(len)
}

/// Reports the native UI bridge ABI this guest implements.
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit::guest::ABI_VERSION as i32
}
