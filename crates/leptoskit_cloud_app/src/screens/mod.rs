//! The console's screens (`docs/agents/cloud-design.md` §7, §10.1, §14).
//!
//! Each route file's page calls one function here. A page reads the
//! console's state, works out which of its states it is in, and builds that
//! state's view from plain values, so the component tests can draw every
//! state without a server.

pub mod account;
pub mod device;
pub mod organizations;
pub mod projects;
pub mod sign_in;
pub mod welcome;

#[cfg(feature = "ssr")]
use leptoskit::leptos::prelude::use_context;
use leptoskit::leptos::task::spawn_local;
use leptoskit::native::Role;
use leptoskit::prelude::*;
use leptoskit_router::Routable as _;

use crate::components::Kind;
use crate::components::button;
use crate::components::link;
use crate::components::link_button;
use crate::components::mark;
use crate::components::problem;
use crate::components::sheet;
use crate::components::skeleton;
use crate::console::Console;
use crate::console::Load;
use crate::console::Who;
use crate::console::first_organization;
use crate::console::route_of;
use crate::routes::Navigation;
use crate::routes::Route;
use crate::routes::SignIn;
use crate::style;

/// Sends the browser to `href` instead of this page: a redirect response
/// while the server renders, and a replaced history entry in the browser.
pub fn redirect(navigation: &Navigation, href: &str) {
	#[cfg(feature = "ssr")]
	if leptoskit::query::is_server() {
		leptoskit::server::leptos_axum::redirect(href, false);
		return;
	}

	let Some(route) = route_of(href) else {
		return;
	};
	let navigation = navigation.clone();
	spawn_local(async move {
		if navigation.current() != route {
			navigation.replace(route);
		}
	});
}

/// Sets the response's status while the server renders. In the browser
/// there's no response to set.
pub fn respond_with(status: u16) {
	#[cfg(feature = "ssr")]
	if let Some(response) = use_context::<leptoskit::server::leptos_axum::ResponseOptions>()
		&& let Ok(status) = leptoskit::server::axum::http::StatusCode::from_u16(status)
	{
		response.set_status(status);
	}

	#[cfg(not(feature = "ssr"))]
	let _ = status;
}

/// `/sign-in?next=<href>`: where a signed-out visit to `href` goes.
#[must_use]
pub fn sign_in_href(next: &str) -> String {
	Route::SignIn(SignIn {
		next: Some(next.to_owned()),
		via: None,
		auth: None,
		auth_error: None,
		auth_factors: None,
	})
	.href()
	.to_string()
}

/// `/`: the person's organisation, or sign in.
pub fn home(navigation: &Navigation) -> Element {
	let console = Console::get();

	match console.who() {
		Who::SignedOut => redirect(navigation, &sign_in_href("/")),
		Who::SignedIn(_) => {
			if let Load::Ready(me) = console.me() {
				let href = first_organization(&me).map_or_else(
					|| "/organizations".to_owned(),
					|org| format!("/{}", org.slug),
				);
				redirect(navigation, &href);
			}
		}
		Who::Checking | Who::Unknown(_) => {}
	}

	crate::shell::desk(Fragment::new().with(desk_message(
		"LeptosKit Cloud",
		"Opening your projects.",
		Some(("Your organisations", "/organizations")),
	)))
}

/// Any URL the console has no page for, or one the person can't see.
pub fn not_found() -> Element {
	respond_with(404);
	crate::shell::desk(Fragment::new().with(desk_message(
		"Not here.",
		"There's no page at this address, or you can't see it.",
		Some(("Your organisations", "/organizations")),
	)))
}

/// A short sheet with a heading, a sentence, and a way on.
pub fn desk_message(heading: &str, sentence: &str, next: Option<(&str, &str)>) -> Element {
	sheet(
		[
			masthead(),
			Heading(1, heading).class(style::DISPLAY),
			Text(sentence).class(style::SECONDARY),
		]
		.into_iter()
		.chain(next.map(|(label, href)| link(format!("{label} →"), href))),
	)
}

/// The sheet's masthead: the mark and the product's name.
pub fn masthead() -> Element {
	View()
		.class("flex-row items-center gap-2")
		.child(mark(20.0))
		.child(Text("LeptosKit Cloud").class("text-sm font-semibold text-[var(--cloud-ink)]"))
}

/// Signed out on a page that needs someone signed in: the page stays, and a
/// sheet offers to sign in and come back.
pub fn signed_out(next: &str) -> Element {
	sheet([
		Heading(1, "You've been signed out.").class(style::DISPLAY),
		Text("Sign in and we'll bring you back here.").class(style::SECONDARY),
		link_button(Kind::Primary, "Sign in", sign_in_href(next)),
	])
	.test_id("signed-out")
}

/// A region that's loading: skeletons in the final layout, busy, and named
/// for assistive technology.
pub fn loading(label: &str, shapes: &[&str]) -> Element {
	View()
		.class("flex-col gap-3 w-full")
		.role(Role::Group)
		.accessibility_label(label)
		.aria_busy(true)
		.children(shapes.iter().map(|shape| skeleton(shape)))
}

/// A region that failed to load: it keeps its place and says what failed,
/// the error, and how to try again.
pub fn load_failed(what: &str, error: &str, retry: impl Fn() + Send + Sync + 'static) -> Element {
	View()
		.class("flex-col items-start gap-3")
		.child(problem(format!("We couldn't load {what}: {error}.")))
		.child(button(Kind::Secondary, "Try again", None, retry))
}

/// The session couldn't be read: say so and offer another try.
pub fn session_failed(console: Console, error: &str) -> Element {
	let session = console.session;
	load_failed("your session", error, move || {
		drop(session.refetch());
	})
}
