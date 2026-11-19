//! The leptoskit admin's pages, built from one component tree on both
//! sides.
//!
//! `leptoskit_admin` renders a page on the server from a [`Model`], a small
//! value it builds from the signed-in admin and the page's data, and writes
//! the same value into the document. The browser runs this crate compiled
//! to Wasm, reads the value back, builds the same tree, and attaches to the
//! markup: the theme, the account menu, search, and the keyboard shortcuts
//! are this crate's Rust, on both sides.
//!
//! Pages move here one at a time. Until a page has, the admin renders it
//! the old way, with the pieces this crate shares ([`style`],
//! [`components`], [`workings`], [`charts`], and the rail).

pub mod analytics;
pub mod charts;
pub mod components;
pub mod confirm;
pub mod model;
pub mod ops;
pub mod pages;
pub mod shell;
pub mod style;
pub mod workings;

use leptos::prelude::*;
pub use model::Environment;
pub use model::Model;

/// Where the admin mounts: every page and endpoint lives under this path.
pub const BASE: &str = "/_leptoskit/admin";

/// Runs `task` on the page's own thread.
pub(crate) fn spawn(task: impl Future<Output = ()> + 'static) {
	leptos::task::spawn_local(task);
}

/// Leaves this page for `address`, as a full page load. On the server there
/// is no page to leave.
pub(crate) fn go(address: &str) {
	#[cfg(feature = "hydrate")]
	if let Some(window) = web_sys::window() {
		let _ = window.location().assign(address);
	}
	#[cfg(not(feature = "hydrate"))]
	let _ = address;
}

/// A key that makes a retried action count once.
pub(crate) fn idempotency_key() -> String {
	#[cfg(feature = "hydrate")]
	{
		web_sys::window()
			.and_then(|window| window.crypto().ok())
			.map(|crypto| crypto.random_uuid())
			.unwrap_or_default()
	}
	#[cfg(not(feature = "hydrate"))]
	String::new()
}

/// Posts `body` as JSON to `path` on this site: whether the answer was a
/// success, and its JSON. `None` when nothing came back.
#[cfg_attr(
	not(feature = "hydrate"),
	expect(clippy::unused_async, reason = "only a browser sends anything")
)]
pub(crate) async fn post_json(path: &str, body: &str) -> Option<(bool, serde_json::Value)> {
	#[cfg(feature = "hydrate")]
	{
		use wasm_bindgen::JsCast as _;
		use wasm_bindgen_futures::JsFuture;

		let init = web_sys::RequestInit::new();
		init.set_method("POST");
		init.set_credentials(web_sys::RequestCredentials::SameOrigin);
		init.set_body(&wasm_bindgen::JsValue::from_str(body));
		let request = web_sys::Request::new_with_str_and_init(path, &init).ok()?;
		request
			.headers()
			.set("content-type", "application/json")
			.ok()?;
		let response: web_sys::Response =
			JsFuture::from(web_sys::window()?.fetch_with_request(&request))
				.await
				.ok()?
				.dyn_into()
				.ok()?;
		let text = JsFuture::from(response.text().ok()?).await.ok()?;
		let data = text
			.as_string()
			.and_then(|text| serde_json::from_str(&text).ok())
			.unwrap_or(serde_json::Value::Null);

		Some((response.ok(), data))
	}
	#[cfg(not(feature = "hydrate"))]
	{
		let _ = (path, body);
		None
	}
}

/// The header that asks a page's address for its [`model::Page`] as JSON,
/// so a builder can put the next page in place of this one.
pub const PAGE_HEADER: &str = "x-admin-page";

/// Gets `path` on this site as JSON: whether the answer was a success, and
/// its JSON, or `Null` when it wasn't JSON. With `page`, asks a page's
/// address for its model. `None` when nothing came back.
#[cfg_attr(
	not(feature = "hydrate"),
	expect(clippy::unused_async, reason = "only a browser fetches anything")
)]
pub(crate) async fn get_json(path: &str, page: bool) -> Option<(bool, serde_json::Value)> {
	#[cfg(feature = "hydrate")]
	{
		use wasm_bindgen::JsCast as _;
		use wasm_bindgen_futures::JsFuture;

		let init = web_sys::RequestInit::new();
		init.set_method("GET");
		init.set_credentials(web_sys::RequestCredentials::SameOrigin);
		let request = web_sys::Request::new_with_str_and_init(path, &init).ok()?;
		request.headers().set("accept", "application/json").ok()?;

		if page {
			request.headers().set(PAGE_HEADER, "json").ok()?;
		}

		let response: web_sys::Response =
			JsFuture::from(web_sys::window()?.fetch_with_request(&request))
				.await
				.ok()?
				.dyn_into()
				.ok()?;
		// A sign-in the session lost answers with a redirect to a page.
		if response.redirected() {
			return Some((false, serde_json::Value::Null));
		}

		let text = JsFuture::from(response.text().ok()?).await.ok()?;
		let data = text
			.as_string()
			.and_then(|text| serde_json::from_str(&text).ok())
			.unwrap_or(serde_json::Value::Null);

		Some((response.ok(), data))
	}
	#[cfg(not(feature = "hydrate"))]
	{
		let _ = (path, page);
		None
	}
}

/// The HTML document a page renders into.
///
/// The admin provides the page's [`Model`] as context for the request it is
/// answering.
pub fn document(options: LeptosOptions) -> impl IntoView {
	let model = use_context::<Model>();
	let title = model
		.as_ref()
		.map(Model::document_title)
		.unwrap_or_default();
	let appearance = model.and_then(|model| model.viewer.appearance);

	view! {
		<!DOCTYPE html>
		<html lang="en" data-lk-appearance=appearance>
			<head>
				<meta charset="utf-8" />
				<meta
					name="viewport"
					content="width=device-width, initial-scale=1, viewport-fit=cover"
				/>
				<meta name="color-scheme" content="light dark" />
				<meta name="robots" content="noindex" />
				<title>{title}</title>
				<style inner_html=shell::stylesheet() />
				<HydrationScripts options />
			</head>
			<body>
				<App />
			</body>
		</html>
	}
}

/// The page the request asked for.
///
/// The server takes the [`Model`] from the request's context and writes it
/// into the document; the browser reads it back from there, so it builds the
/// tree the server built.
#[component]
pub fn App() -> impl IntoView {
	let model = SharedValue::new(|| {
		use_context::<Model>().expect("the route provides the page it is rendering")
	})
	.into_inner();
	// Signing out is a mutation, which keeps its state here.
	leptoskit::query::QueryClient::new().provide();
	let shell = shell::Shell::new(model);
	let build = pages::screen_of(&shell);

	leptoskit::web::screen(build)
}

/// The client entry point: `HydrationScripts` calls it once the Wasm loads.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
	console_error_panic_hook::set_once();
	let _ = any_spawner::Executor::init_wasm_bindgen();
	hydrate_body(App);

	// From here a press is heard. Tests read this to know the page is more
	// than the server's drawing.
	if let Some(root) = web_sys::window()
		.and_then(|window| window.document())
		.and_then(|document| document.document_element())
	{
		let _ = root.set_attribute("data-hydrated", "");
	}
}
