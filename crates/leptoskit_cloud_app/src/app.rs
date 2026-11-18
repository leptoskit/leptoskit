//! The console as a leptos app: rendered on the server, hydrated in the
//! browser, with one tree of leptoskit components for both.

use leptos::prelude::*;
use leptoskit::rpc::client::LiveState;

use crate::components::Connection;
use crate::console::Console;
use crate::console::Services;
use crate::routes::Navigation;
use crate::routes::Route;

/// The route the browser or the request is on.
fn navigation() -> Navigation {
	#[cfg(target_arch = "wasm32")]
	{
		Navigation::from_browser().unwrap_or_else(|_| Navigation::new(Route::Home))
	}

	#[cfg(all(not(target_arch = "wasm32"), feature = "ssr"))]
	{
		leptoskit_router::ssr::navigation_from_request::<Route>()
			.unwrap_or_else(|_| Navigation::new(Route::Home))
	}

	#[cfg(all(not(target_arch = "wasm32"), not(feature = "ssr")))]
	{
		Navigation::new(Route::Home)
	}
}

/// Where the console is, as the address bar shows it: the request's `Host`
/// on the server and the page's host in the browser, so both render the
/// same words.
fn host() -> String {
	#[cfg(target_arch = "wasm32")]
	{
		web_sys::window()
			.and_then(|window| window.location().host().ok())
			.unwrap_or_default()
	}

	#[cfg(all(not(target_arch = "wasm32"), feature = "ssr"))]
	{
		use_context::<leptoskit::server::axum::http::request::Parts>()
			.and_then(|parts| {
				parts
					.headers
					.get(leptoskit::server::axum::http::header::HOST)
					.and_then(|host| host.to_str().ok())
					.map(str::to_owned)
			})
			.unwrap_or_default()
	}

	#[cfg(all(not(target_arch = "wasm32"), not(feature = "ssr")))]
	{
		String::new()
	}
}

/// The console: its state, made once, and the current route's page inside
/// its layouts.
#[component]
pub fn App() -> impl IntoView {
	leptoskit::query::QueryClient::new().provide();
	let navigation = navigation();

	Suspense(
		SuspenseProps::builder()
			.children(TypedChildren::to_children(move || {
				// The console is made inside the boundary, once it has taken
				// its serialization number, so the queries are numbered the
				// same on the server and in the browser (`Console::new`).
				let console = Console::new(&navigation, Services::default(), host()).provide();

				// The page goes live in the browser only. A server built with
				// both `ssr` and `hydrate` (as `--all-features` builds it) has
				// effects turned on, so they're made only off the server.
				if !leptoskit::query::is_server() {
					live(&console);
				}

				move || leptoskit::web::render(navigation.render())
			}))
			.build(),
	)
}

/// What only the browser knows: that the page is live, so the server's page
/// and the browser's first render agree; and whether the live connection
/// is up, for the offline chip. Offline counts only after the connection
/// has been open: before that there's nothing to say.
fn live(console: &Console) {
	let console = *console;
	Effect::new(move |_| console.mounted.set(true));

	let live = leptoskit::rpc::client::use_live_status();
	let was_live = StoredValue::new(false);
	Effect::new(move |_| {
		let connection = match live.get().state {
			LiveState::Live { .. } => {
				was_live.set_value(true);
				Connection::Fine
			}
			LiveState::Reconnecting { next_in, .. } => Connection::Reconnecting { next_in },
			LiveState::Offline if was_live.get_value() => Connection::Offline,
			LiveState::Connecting | LiveState::Offline => Connection::Fine,
		};

		if console.connection.get_untracked() != connection {
			console.connection.set(connection);
		}
	});
}

/// The whole document the server sends: the tokens and fonts in the head,
/// the hydration loader, and the console in the body.
#[cfg(feature = "ssr")]
pub fn shell(options: LeptosOptions) -> impl IntoView {
	let css = crate::style::document_css();

	view! {
		<!DOCTYPE html>
		<html lang="en">
			<head>
				<meta charset="utf-8" />
				<meta
					name="viewport"
					content="width=device-width, initial-scale=1, viewport-fit=cover"
				/>
				<meta name="color-scheme" content="light dark" />
				<link rel="icon" href="data:," />
				<title>"LeptosKit Cloud"</title>
				<HydrationScripts options />
				<style>{css}</style>
			</head>
			<body>
				<App />
			</body>
		</html>
	}
}
