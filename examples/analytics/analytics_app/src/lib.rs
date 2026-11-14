//! A server-rendered leptoskit app with an analytics service beside it.
//!
//! The page renders on the server, which asks the service for the first
//! summary over HTTP. In the browser, the buttons send events to the service
//! and the summary updates live, both through the app's own origin.

pub mod app;
#[cfg(feature = "ssr")]
pub mod server;

/// The client entry point: `HydrationScripts` calls it once the wasm loads.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
	console_error_panic_hook::set_once();
	// `configure` opens the live connection at once, so the browser's task
	// executor has to be running first; `hydrate_body` would start it too late.
	let _ = any_spawner::Executor::init_wasm_bindgen();
	// The page's own origin: service calls go to
	// `/_leptoskit/services/analytics/` on it.
	leptoskit_rpc::client::configure(leptoskit_rpc::client::ClientConfig::new());
	leptos::mount::hydrate_body(app::App);
}
