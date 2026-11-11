#![allow(unsafe_code)] // Exported C names form the native runtime's entry points.

//! The `LeptosKit` app: scan the code `leptoskit dev` prints and run that
//! guest.

pub mod home;
pub mod routes;

/// The browser application.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
#[leptos::component]
pub fn App() -> impl leptos::prelude::IntoView {
	let navigation = routes::Navigation::new(routes::Route::Join);
	let session = routes::index::session();
	move || {
		reactive_graph::owner::provide_context(session.clone());
		leptoskit::web::render(navigation.render())
	}
}

/// Mounts the browser application when the page loads its Wasm module.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	leptos::mount::mount_to_body(App);
}

/// Mounts the same application in the native Wasm runtime.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	let navigation = routes::Navigation::new(routes::Route::Join);
	let session = routes::index::session();
	navigation.bind_system_back();
	leptoskit::guest::mount(move || {
		reactive_graph::owner::provide_context(session.clone());
		navigation.render()
	});
}

/// Android system Back: 1 when the router popped a route.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_system_back() -> i32 {
	match leptoskit_router::dispatch_system_back() {
		leptoskit_router::SystemBack::Consumed => 1,
		leptoskit_router::SystemBack::Propagate => 0,
	}
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
