//! One application shared by the browser, server, and native runners.

pub mod components;
pub mod notes;
pub mod routes;
pub mod services;

pub use notes::Notes;

/// Where native apps find the server; browsers use the page's own origin.
/// An Android emulator reaches the computer running `leptoskit dev` at
/// `http://10.0.2.2:3000`. Change this to your site's origin before you ship.
pub const SERVER: &str = "http://localhost:3000";

/// The browser and server adapter for the same application tree.
#[cfg(feature = "web")]
#[leptos::component]
pub fn App() -> impl leptos::prelude::IntoView {
	use leptos::prelude::*;

	let notes = provide();

	move || leptoskit::web::render(screen(&notes))
}

/// Starts the app's shared state in the current reactive owner: the query
/// cache the notes screen keeps its list in, and the list itself.
///
/// During SSR, query data loads in process and streams into the browser
/// cache. The initial HTML can show the loading state; rendering never
/// calls the server through its own relative URL.
pub fn provide() -> Notes {
	leptoskit::query::QueryClient::new().provide();
	Notes::shared()
}

/// Renders the portable application tree for any host.
pub fn screen(notes: &Notes) -> leptoskit::prelude::Element {
	routes::home::screen(notes)
}

/// The complete HTML document sent by the SSR server.
#[cfg(feature = "web")]
pub fn shell(options: leptos::config::LeptosOptions) -> impl leptos::prelude::IntoView {
	use leptos::prelude::*;

	leptos::view! {
		<!DOCTYPE html>
		<html lang="en">
			<head>
				<meta charset="utf-8" />
				<meta name="viewport" content="width=device-width, initial-scale=1" />
				<link rel="icon" href="data:," />
				<title>"__NAME__"</title>
				<HydrationScripts options />
				<style>{include_str!("../../assets/app.css")}</style>
			</head>
			<body>
				<App />
			</body>
		</html>
	}
}

/// Hydrates server markup when the document's hydration script calls it.
#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
	#[cfg(feature = "devtools")]
	leptoskit::devtools::install(env!("CARGO_PKG_NAME"));
	// The browser talks to the page's own origin, with the session cookie
	// when the app has sign-in. This configures the RPC client and the live
	// socket too.
	leptoskit::rpc::client::configure(leptoskit::rpc::client::ClientConfig::new());
	leptos::mount::hydrate_body(App);
}

/// Mounts the CSR application when the browser loads its Wasm module.
#[cfg(all(target_arch = "wasm32", feature = "web", not(feature = "hydrate")))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	#[cfg(feature = "devtools")]
	leptoskit::devtools::install(env!("CARGO_PKG_NAME"));
	leptoskit::rpc::client::configure(leptoskit::rpc::client::ClientConfig::new());
	leptos::mount::mount_to_body(App);
}

/// The native guest's executor for requests the query client starts itself.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
mod tasks;

/// Mounts the same application in the native Wasm runtime.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_main() {
	tasks::start();
	// A native app has to be told where its server is.
	leptoskit::rpc::client::configure(leptoskit::rpc::client::ClientConfig::new().origin(SERVER));
	let owner = leptoskit::leptos::prelude::Owner::new();
	let notes = owner.with(provide);
	leptoskit::guest::mount(move || owner.with(|| screen(&notes)));
	tasks::poll();
}

/// Forwards platform-neutral events to the shared tree's handlers, then runs
/// the tasks they started.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_event(node: i64, kind: i32, ptr: i32, len: i32) -> i32 {
	let handled = leptoskit::guest::dispatch(node, kind, ptr, len);
	tasks::poll();
	handled
}

/// Settles the values application state bound, after a turn.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_flush() {
	leptoskit::guest::flush();
}

/// Runs the tasks a plugin reply woke, such as the notes list's answer.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_poll() {
	tasks::poll();
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
