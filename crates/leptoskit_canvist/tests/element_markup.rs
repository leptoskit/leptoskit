//! The server's markup for a `CanvistEditor` element, which the browser
//! tests hydrate. The fixture is checked against what leptoskit renders, so
//! the hydration test never runs on markup the server no longer writes.
#![cfg(not(target_arch = "wasm32"))]

use leptos::prelude::RenderHtml;
use leptoskit_native::Element;
use leptoskit_native::components::CanvistEditor;
use leptoskit_native::web::render;

/// The editor the server made. The page that hydrates this markup has an
/// editor of its own, with another id.
const SERVER_EDITOR: u32 = 57;

#[test]
fn the_hydration_fixture_is_what_the_server_renders() {
	// The same screen as `hydration_screen` in `element.rs`.
	let html = without_devtools_attributes(
		&render(
			Element::view().child(
				CanvistEditor(SERVER_EDITOR)
					.accessibility_label("Note")
					.placeholder("Write a note…")
					.class("w-96 text-base"),
			),
		)
		.to_html(),
	);

	if std::env::var_os("UPDATE_LEPTOSKIT_WEB_FIXTURE").is_some() {
		std::fs::write(
			concat!(
				env!("CARGO_MANIFEST_DIR"),
				"/tests/fixtures/element_hydration.html"
			),
			&html,
		)
		.expect("write explicit fixture update");

		return;
	}

	assert_eq!(html, include_str!("fixtures/element_hydration.html"));
}

/// Markup as an app without devtools gets it. With leptoskit's `devtools`
/// feature on, which a build of the whole workspace turns on, each element
/// also says where in the source it was created.
fn without_devtools_attributes(html: &str) -> String {
	const DEVTOOLS: [&str; 5] = [
		"data-lk-node",
		"data-lk-key",
		"data-lk-kind",
		"data-lk-source",
		"data-lk-component",
	];
	let mut out = String::with_capacity(html.len());
	let mut rest = html;

	while let Some(start) = rest.find(" data-lk-") {
		let name_end = rest[start + 1..]
			.find(['=', ' ', '>'])
			.map_or(rest.len(), |at| start + 1 + at);

		if DEVTOOLS.contains(&&rest[start + 1..name_end]) && rest[name_end..].starts_with("=\"") {
			out.push_str(&rest[..start]);
			// Skip `="value"`.
			let value_end = rest[name_end + 2..]
				.find('"')
				.map_or(rest.len(), |at| name_end + 2 + at + 1);
			rest = &rest[value_end..];
		} else {
			out.push_str(&rest[..name_end]);
			rest = &rest[name_end..];
		}
	}

	out.push_str(rest);

	out
}
