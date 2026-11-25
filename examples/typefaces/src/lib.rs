#![allow(unsafe_code)] // Exported C names form the native runtime's entry points.

//! Typefaces: an app's own fonts, drawn on every platform.
//!
//! `leptoskit.toml` declares two families under `[theme.fonts]`: `hand`, a
//! static hand-drawn face, and `gentle`, a variable face that draws every
//! weight from 300 to 900. The page sets the same line in the system font
//! and in both families, so the three compare at a glance. A family set on a
//! view reaches every text inside it, buttons included, and `font-sans`
//! returns to the system font.
//!
//! ```
//! use leptoskit::prelude::*;
//!
//! let page = typefaces_example::Page();
//! assert!(!page.children.is_empty());
//! ```

use leptoskit::prelude::*;

/// The line every sample sets.
pub const LINE: &str = "Reap the dead market";

/// The page: the line in the system font, then in each bundled family.
#[component]
pub fn Page() -> Element {
	// The backdrop is on a view: macOS draws no scroll view background.
	view! {
		<View class="flex-1 bg-stone-50">
			<ScrollView class="flex-1">
				<View class="flex-col gap-6 p-6">
					<Heading aria_level=1 class="font-[var(--hand)] text-4xl text-stone-900">
						"Typefaces"
					</Heading>
					<Text class="text-base text-stone-600">
						"The same line in the system font and in the two families leptoskit.toml bundles."
					</Text>
					<View class="flex-col gap-2">
						<Text class="text-xs font-semibold text-stone-500">"SYSTEM FONT"</Text>
						<Text class="text-2xl text-stone-900">{LINE}</Text>
						<Text class="text-2xl font-black text-stone-900">{LINE}</Text>
					</View>
					<View class="flex-col gap-2 font-[var(--hand)] text-stone-900">
						<Text class="font-sans text-xs font-semibold text-stone-500">
							"--HAND · SKRIBBLE NOTO SKETCH"
						</Text>
						<Text class="text-2xl">{LINE}</Text>
						<Button class="self-start px-4 py-2 rounded-lg bg-stone-900 text-white text-lg">
							"A button takes the family too"
						</Button>
					</View>
					<View class="flex-col gap-2 font-[var(--gentle)] text-stone-900">
						<Text class="font-sans text-xs font-semibold text-stone-500">
							"--GENTLE · SKRIBBLE GENTLE, 300 TO 900"
						</Text>
						<Text class="text-2xl font-light">{LINE}</Text>
						<Text class="text-2xl">{LINE}</Text>
						<Text class="text-2xl font-black">{LINE}</Text>
					</View>
				</View>
			</ScrollView>
		</View>
	}
}

/// Mounts the page when the browser loads the Wasm module.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	leptos::mount::mount_to_body(|| leptoskit::web::render(Page()));
}

/// Mounts the same page in the native Wasm runtime.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	leptoskit::guest::mount(Page);
}

/// Forwards platform-neutral events to the shared tree's handlers.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	leptoskit::guest::dispatch(node, kind, ptr, len)
}

/// Allocates the native runtime's event payload buffer.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_alloc(len: i32) -> i32 {
	leptoskit::guest::alloc(len)
}

/// Reports the native UI bridge ABI implemented by this app.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_abi_version() -> i32 {
	leptoskit::guest::ABI_VERSION as i32
}
