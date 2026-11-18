//! `LeptosKit` Cloud, as a program: `leptoskit-cloud-control`, and the console
//! it serves.
//!
//! The control plane itself, its operations, sign-in, and routes, is
//! [`leptoskit_cloud`]'s `server` feature. This crate reads the operator's
//! environment, refuses a configuration that is incomplete or unsafe, and
//! runs the commands: `serve`, `migrate`, and `operator`. The readme lists
//! every variable it reads.
//!
//! The console is leptoskit components in file routes (`src/routes/`),
//! rendered on the server with the `ssr` feature and hydrated in the browser
//! from the `hydrate` feature's Wasm. `docs/agents/cloud-design.md` is its
//! design; `docs/agents/cloud.md` says how to build and run it.

#[cfg(feature = "web")]
pub mod app;
pub mod components;
pub mod console;
pub mod data;
pub mod routes;
pub mod screens;
#[cfg(feature = "server")]
pub mod server;
pub mod shell;
pub mod style;

/// Hydrates the server's page when the document's loader calls it.
#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
	console_error_panic_hook::set_once();
	// The queries start fetching while the page hydrates, so the executor
	// has to be running first; `hydrate_body` would start it too late.
	let _ = any_spawner::Executor::init_wasm_bindgen();
	// The page's own origin serves sign-in, the operations, and the live
	// socket.
	leptos::mount::hydrate_body(|| {
		leptoskit::auth::client::configure(leptoskit::auth::client::ClientConfig::new());
		app::App()
	});
}
