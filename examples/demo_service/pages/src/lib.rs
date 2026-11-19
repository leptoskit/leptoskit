//! The public demo's pages: the front door, sign-in with its demo inbox,
//! and Notes' notebook with the account menu.
//!
//! Every page is a leptoskit component tree written as `view!` markup. The
//! demo service renders it on the server, in the admin's paper, ink, ember,
//! and mint, and the browser hydrates the same tree compiled to Wasm: the
//! countdown, the drawers, the new-day dialog, and every request a page
//! makes are this crate's Rust, on both sides.
//!
//! What a page knows is a small, safe [`Page`] value the service builds from
//! the visitor's pass and session. The server renders from it and hands the
//! same value to the browser, so both build the same tree.

mod chrome;
mod front;
pub mod model;
mod notes;
pub mod ops;
mod sign_in;

use std::sync::Arc;
use std::time::Duration;

pub use front::noscript_form;
use leptos::prelude::*;
use leptoskit::hooks::Clock;
use leptoskit::hooks::use_clock;
use leptoskit::prelude::Element;

use crate::model::Context;
use crate::model::Page;
pub use crate::notes::noscript as noscript_notes;
pub use crate::sign_in::noscript as noscript_sign_in;

/// What every page keeps while it is open: the clock it counts down on, and
/// the pieces of the chrome that open and close.
#[derive(Clone)]
pub(crate) struct Shell {
	/// The day and the addresses the page was made with.
	context: Arc<Context>,
	/// How long the page has been open. Zero while the server renders and
	/// on the browser's first pass, so both word the countdown the same.
	clock: Clock,
	/// Whether less than ten minutes remain.
	late: Signal<bool>,
	/// Whether the "What's real?" panel is open.
	band_open: RwSignal<bool>,
	/// Whether the new-day dialog is open.
	reset_open: RwSignal<bool>,
	/// What the polite status region says.
	announcement: RwSignal<String>,
}

impl Shell {
	fn new(context: Context) -> Self {
		let context = Arc::new(context);
		let clock = use_clock(Duration::from_secs(1));
		let late = {
			let context = Arc::clone(&context);
			Memo::new(move |_| model::is_late(seconds_after(&context, clock.get())))
		};

		Self {
			context,
			clock,
			late: late.into(),
			band_open: RwSignal::new(false),
			reset_open: RwSignal::new(false),
			announcement: RwSignal::new(String::new()),
		}
	}

	/// Whole seconds to the reset, measured from the server's clock and
	/// counted forward on this page's.
	fn seconds_to_reset(&self) -> impl Fn() -> i64 + Clone + Send + Sync + 'static {
		let context = Arc::clone(&self.context);
		let clock = self.clock;
		move || seconds_after(&context, clock.get())
	}

	/// Tells a screen reader `message` without moving focus. The region is
	/// emptied first, so saying the same thing twice is heard twice.
	fn announce(&self, message: &str) {
		let announcement = self.announcement;
		let message = message.to_owned();
		announcement.set(String::new());
		spawn(async move {
			leptoskit::query::sleep(Duration::from_millis(30)).await;
			announcement.try_set(message);
		});
	}
}

fn seconds_after(context: &Context, elapsed: Duration) -> i64 {
	context.seconds_to_reset_after(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

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

/// Opens `address` in a new tab that has no handle on this page. On the
/// server there is no page to open it from.
pub(crate) fn open_tab(address: &str) {
	#[cfg(feature = "hydrate")]
	if let Some(window) = web_sys::window() {
		let _ = window.open_with_url_and_target_and_features(address, "_blank", "noopener");
	}
	#[cfg(not(feature = "hydrate"))]
	let _ = address;
}

/// Text made safe for HTML text and attribute values.
#[must_use]
pub fn escape(text: &str) -> String {
	text.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
		.replace('\'', "&#39;")
}

/// What the document around a page needs that only the server has: the
/// stylesheet, and plain HTML for a visitor without scripts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
	/// The page's whole stylesheet: font faces, the admin's colour tokens,
	/// and the demo's own rules.
	pub style: String,
	/// Plain HTML placed first in the body, already escaped.
	pub before: String,
}

/// The HTML document a page renders into.
///
/// The service provides the [`Page`] and the [`Document`] as context for the
/// request it is answering.
pub fn shell(options: LeptosOptions) -> impl IntoView {
	let style = use_context::<Document>().unwrap_or_default().style;
	let title = use_context::<Page>().map_or("Notes", |page| page.title());

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
				<meta name="robots" content="noindex" />
				<title>{title}</title>
				<style inner_html=style />
				<HydrationScripts options />
			</head>
			<body>
				<Body />
			</body>
		</html>
	}
}

/// Everything in the document's body: the plain HTML only the server has,
/// then the page.
///
/// The browser attaches to this, and has no [`Document`]. It leaves the plain
/// HTML as the server wrote it: attaching never rewrites an element's inner
/// HTML.
#[component]
pub fn Body() -> impl IntoView {
	let before = use_context::<Document>().unwrap_or_default().before;

	view! {
		<div inner_html=before />
		<App />
	}
}

/// The page the request asked for.
///
/// The server takes the [`Page`] from the request's context and writes it
/// into the document; the browser reads it back from there, so it builds the
/// tree the server built.
#[component]
pub fn App() -> impl IntoView {
	let page = SharedValue::new(|| {
		use_context::<Page>().expect("the route provides the page it is rendering")
	})
	.into_inner();
	// The requests to Auth, signing in and out, are mutations, which keep
	// their state here.
	leptoskit::query::QueryClient::new().provide();
	let shell = Shell::new(page.context().clone());
	watch_generation(&shell);
	let build = screen_of(&shell, page);

	leptoskit::web::screen(build)
}

/// The function that builds the page's tree, with the page's own state made
/// once beside it.
fn screen_of(shell: &Shell, page: Page) -> Box<dyn Fn() -> Element + Send + Sync> {
	let shell = shell.clone();

	match page {
		Page::Front(door) => {
			let front = front::Front::new();
			Box::new(move || front::page(&shell, &door, front))
		}
		Page::SignIn(view) => {
			let view = Arc::new(view);
			let state = sign_in::State::new(&shell, &view);
			sign_in::open(&shell, &view, &state);
			Box::new(move || sign_in::page(&shell, &view, &state))
		}
		Page::Notes(welcome) => {
			let welcome = Arc::new(welcome);
			let state = notes::State::new();
			notes::open(&shell, &welcome, &state);
			Box::new(move || notes::page(&shell, &welcome, &state))
		}
	}
}

/// Opens the new-day dialog, once, when the server names a different day
/// than the one this page was made for.
///
/// A dropped stream proves nothing: the page opens it again, and only a
/// server that says another day's name opens the dialog.
fn watch_generation(shell: &Shell) {
	let watch = leptoskit::rpc::use_stream(|| ops::Generation { cursor: None });
	let day = shell.context.day.clone();
	let reset_open = shell.reset_open;
	let told = RwSignal::new(false);

	Effect::new(move |_| {
		let another = watch
			.items()
			.with(|days| days.last().is_some_and(|latest| latest.day != day));

		if another && !told.get_untracked() {
			told.set(true);
			reset_open.set(true);
		}
	});
}

/// The client entry point: `HydrationScripts` calls it once the Wasm loads.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
	console_error_panic_hook::set_once();
	// `configure` opens the live connection at once, so the browser's task
	// executor has to be running first.
	let _ = any_spawner::Executor::init_wasm_bindgen();
	leptoskit::rpc::client::configure(leptoskit::rpc::client::ClientConfig::new());
	hydrate_body(Body);

	// From here a press is heard. Styles and tests read this to know the
	// page is more than the server's drawing.
	if let Some(root) = web_sys::window()
		.and_then(|window| window.document())
		.and_then(|document| document.document_element())
	{
		let _ = root.set_attribute("data-hydrated", "");
	}
}
