//! Exact's example apps, rebuilt with leptoskit.
//!
//! Each screen is one function. `cargo run -p exact_examples -- <name>` prints
//! what a screen reader would read. `leptoskit dev` in this directory serves
//! the same screens in the browser.

mod catalog;
mod frame;
mod screens;

pub use catalog::EXAMPLES;
pub use catalog::Example;
pub use catalog::find;
pub use catalog::screen_in_query;

/// Mounts the picker when the browser loads the Wasm client.
#[cfg(all(target_arch = "wasm32", feature = "csr"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	let named = web_sys_screen();
	leptos::mount::mount_to_body(move || {
		leptoskit::web::screen(move || {
			match named {
				Some(example) => (example.render)(),
				None => picker(),
			}
		})
	});
}

/// The screen named by `?screen=`, when the page has one.
#[cfg(all(target_arch = "wasm32", feature = "csr"))]
fn web_sys_screen() -> Option<&'static Example> {
	web_sys::window()
		.and_then(|window| window.location().search().ok())
		.and_then(|query| screen_in_query(&query))
}

/// The picker of every example, and the one that is open.
#[cfg(all(target_arch = "wasm32", feature = "csr"))]
fn picker() -> leptoskit::prelude::Element {
	use leptoskit::prelude::*;

	let selected = remember(|| 0_usize);
	let index = selected.get();
	let example = &EXAMPLES[index];
	let buttons = EXAMPLES
		.iter()
		.enumerate()
		.map(|(item, example)| {
			view! {
				<Button
					class="px-3 py-2 rounded-lg text-sm font-semibold bg-white text-slate-900"
					on:press=move || selected.set(item)
				>
					{example.title}
				</Button>
			}
		})
		.collect::<Fragment>();
	let screen = (example.render)();
	view! {
		<View class="gap-4 p-4 bg-slate-50">
			<Heading aria_level=1 class="text-2xl font-bold text-slate-900">
				"Exact examples"
			</Heading>
			<View class="flex-row flex-wrap gap-2">{buttons}</View>
			{screen}
		</View>
	}
}
