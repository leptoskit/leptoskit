//! One application shared by the browser, server, and native runners.

pub mod auth;
pub mod components;
pub mod notes;
pub mod routes;
pub mod services;

use leptoskit::prelude::Get as _;
use leptoskit::prelude::Set as _;
pub use notes::Notes;

/// Where native apps find the server; browsers use the page's own origin.
/// An Android emulator reaches the computer running `leptoskit dev` at
/// `http://10.0.2.2:3000`. Change this to your site's origin before you ship.
pub const SERVER: &str = "http://localhost:3000";

/// The signed-in screens' state. The notes screen is built once for each
/// account, under its own owner, so signing out drops its queries.
#[derive(Clone, Copy)]
pub struct Shell {
	notes: leptoskit::prelude::RwSignal<Option<Notes>>,
}

impl Shell {
	/// The signed-in person's notes, once their screen is built. `None` while
	/// the session is checked, signed out, or the moment the account changes.
	pub fn notes(&self) -> Option<Notes> {
		let identity = self.identity()?;
		self.notes.get().filter(|notes| notes.owns(&identity))
	}

	/// The account the session names, if someone is signed in.
	fn identity(&self) -> Option<String> {
		auth::screens()
			.sign_in
			.session()
			.map(|session| session.account_id.to_string())
	}
}

/// Renders the portable application tree for any host: the sign-in screen
/// until someone signs in, then their notes.
///
/// It reads the sign-in screens from context, so call it inside the
/// reactive owner where [`provide`] ran.
pub fn screen(shell: &Shell) -> leptoskit::prelude::Element {
	reactive_graph::owner::provide_context(*shell);
	auth::screens().gate(|| routes::Navigation::new(routes::Route::Home).render())
}

/// Starts the app's shared state in the current reactive owner: the query
/// cache the sign-in client keeps the session in, the sign-in screens, and
/// the signed-in notes screen.
pub fn provide() -> Shell {
	use leptoskit::leptos::prelude::GetValue as _;
	use leptoskit::leptos::prelude::SetValue as _;
	use leptoskit::leptos::prelude::UpdateValue as _;

	leptoskit::query::QueryClient::new().provide();
	auth::provide();
	let notes = leptoskit::prelude::RwSignal::new(None);
	let shell = Shell { notes };
	let owner =
		leptoskit::leptos::prelude::Owner::current().expect("provide runs inside a reactive owner");
	let signed_in = leptoskit::leptos::prelude::StoredValue::new(
		None::<(String, leptoskit::leptos::prelude::Owner)>,
	);

	// Each account gets a fresh notes screen under its own owner. Signing out
	// drops it, and signing in as someone else builds theirs with the previous
	// account's cached list removed, so one person's notes never show under
	// another's name.
	leptoskit::leptos::prelude::Effect::new_isomorphic(move |_| {
		let identity = shell.identity();
		let active = signed_in
			.try_get_value()
			.flatten()
			.map(|(identity, _)| identity);

		if identity == active {
			return;
		}

		if let Some((_, child)) = signed_in.try_update_value(Option::take).flatten() {
			notes.set(None);
			child.cleanup();
		}

		if let Some(identity) = identity {
			let child = owner.child();
			let notes_state = child.with(|| Notes::new(identity.clone()));
			notes.set(Some(notes_state));
			signed_in.set_value(Some((identity, child)));
		}
	});

	shell
}

/// The browser and server adapter for the same application tree.
#[cfg(feature = "web")]
#[leptos::component]
pub fn App() -> impl leptos::prelude::IntoView {
	let shell = provide();
	reactive_graph::owner::provide_context(shell);
	#[cfg(target_arch = "wasm32")]
	let navigation = routes::Navigation::from_browser()
		.unwrap_or_else(|_| routes::Navigation::new(routes::Route::Home));
	#[cfg(not(target_arch = "wasm32"))]
	let navigation = routes::Navigation::new(routes::Route::Home);

	move || leptoskit::web::render(auth::screens().gate(|| navigation.render()))
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
	// The browser's sign-in client talks to the page's own origin, and
	// configures the RPC client and live socket too.
	leptoskit::auth::client::configure(leptoskit::auth::client::ClientConfig::new());
	leptos::mount::hydrate_body(App);
}

/// Mounts the CSR application when the browser loads its Wasm module.
#[cfg(all(target_arch = "wasm32", feature = "web", not(feature = "hydrate")))]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
	#[cfg(feature = "devtools")]
	leptoskit::devtools::install(env!("CARGO_PKG_NAME"));
	leptoskit::auth::client::configure(leptoskit::auth::client::ClientConfig::new());
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
	leptoskit::auth::client::configure(leptoskit::auth::client::ClientConfig::new().origin(SERVER));
	let owner = leptoskit::leptos::prelude::Owner::new();
	let shell = owner.with(provide);
	let navigation = routes::Navigation::new(routes::Route::Home);
	navigation.bind_system_back();
	leptoskit::guest::mount(move || {
		owner.with(|| {
			reactive_graph::owner::provide_context(shell);
			auth::screens().gate(|| navigation.render())
		})
	});
	tasks::poll();
}

/// Android system Back and similar host signals: 1 when the router popped a
/// route, 0 when the host should leave the app.
#[cfg(all(target_arch = "wasm32", feature = "native"))]
#[unsafe(no_mangle)]
pub extern "C" fn leptoskit_system_back() -> i32 {
	match leptoskit_router::dispatch_system_back() {
		leptoskit_router::SystemBack::Consumed => 1,
		leptoskit_router::SystemBack::Propagate => 0,
	}
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

/// Runs the tasks a plugin reply woke, such as a sign-in request's answer.
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
