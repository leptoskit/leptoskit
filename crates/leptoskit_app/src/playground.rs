//! Static playground shell. Its browser controller is a separate, page-only
//! module.
#[cfg(feature = "server")]
use leptoskit::leptos::html;
#[cfg(feature = "server")]
use leptoskit::leptos::prelude::*;

#[cfg(feature = "server")]
pub fn html(origin: &str) -> String {
	let owner = Owner::new();
	owner.with(|| {
		let page = html::html().lang("en").child((
			page_head(origin),
			html::body().child(
				html::main()
					.id("playground")
					.attr("data-service", SERVICE)
					.child((
						html::a().href("/").child("← LeptosKit"),
						html::p().class("eyebrow").child("YOUR NEXT SMALL IDEA"),
						html::h1().child("Make something move."),
						html::p().child("Real Rust. Real components. A little room to experiment."),
						toolbar(),
						html::p()
							.id("status")
							.role("status")
							.attr("aria-live", "polite")
							.child("Ready · Run with ⌘ / Ctrl + Enter"),
						html::div()
							.class("play-panels")
							.child((source_panel(), output_panel())),
						html::section()
							.id("diagnostics")
							.attr("aria-label", "Compiler diagnostics"),
						html::p().class("play-note").child(
							"The server compiles your code without network access. The preview \
							 runs in a sandbox with network access blocked. Share links store \
							 code in the URL, not a database.",
						),
						html::noscript().child("Enable JavaScript to compile and run this source."),
					)),
			),
		));
		format!("<!doctype html>{}", page.to_html())
	})
}

#[cfg(feature = "server")]
fn page_head(origin: &str) -> impl IntoView {
	html::head().child((
		html::meta().charset("utf-8"),
		html::meta()
			.name("viewport")
			.content("width=device-width, initial-scale=1"),
		html::meta().name("description").content(
			"Write real Rust, compile LeptosKit components, and run them in your browser.",
		),
		html::title().child("LeptosKit Playground · Make something move"),
		html::link()
			.rel("canonical")
			.href(format!("{}/play/", origin.trim_end_matches('/'))),
		html::link().rel("icon").href("/docs/assets/mark.svg"),
		html::link()
			.rel("stylesheet")
			.href("/docs/theme/skribble.css"),
		html::link().rel("stylesheet").href("/landing.css"),
		html::link().rel("stylesheet").href("/playground.css"),
		html::script().r#type("module").src("/play/playground.mjs"),
	))
}

#[cfg(feature = "server")]
fn source_panel() -> impl IntoView {
	html::section()
		.attr("aria-labelledby", "source-label")
		.child((
			html::label()
				.id("source-label")
				.r#for("source")
				.child("Your Rust code"),
			html::textarea()
				.id("source")
				.spellcheck("false")
				.attr("autocapitalize", "off")
				.attr("autocomplete", "off")
				.attr("aria-describedby", "editor-help")
				.child(STARTER),
			html::p()
				.id("editor-help")
				.child("Tab indents. Escape, then Tab moves to the next control."),
		))
}

#[cfg(feature = "server")]
fn output_panel() -> impl IntoView {
	html::section().attr("aria-label", "Live output").child((
		html::div().class("play-tools").child((
			html::h2().child("Live web app"),
			html::button().id("reload").child("Reload"),
			html::button().id("stop").child("Stop"),
		)),
		html::div().id("output"),
	))
}

#[cfg(feature = "server")]
fn toolbar() -> impl IntoView {
	html::div().class("play-tools").child((
		html::label().r#for("starter").child("Start with"),
		html::select().id("starter").child(
			["Counter", "Form", "List", "Motion", "Layout"]
				.map(|name| html::option().value(name).child(name)),
		),
		html::button().id("run").child("Run ↗"),
		html::button().id("share").child("Share link"),
	))
}

pub const SERVICE: &str = match option_env!("LEPTOSKIT_PLAY_SERVICE") {
	Some(value) => value,
	None => "https://play-api.leptoskit.dev",
};

pub const STARTER: &str = include_str!("../web/play-counter.rs");
