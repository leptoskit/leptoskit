//! The public demo's pages: the front door, sign-in with its demo inbox,
//! and Notes' welcome, plus the "opening today's demo" page the service
//! serves while a generation builds.
//!
//! Every page is a real leptoskit component tree, written as `view!`
//! markup and rendered by the same web renderer the admin uses, in the
//! admin's paper, ink, ember, and mint ([`leptoskit_admin::theme_css`]).
//! The behaviour the web renderer cannot express, such as the sign-in
//! flow, the countdown, and the new-day dialog, lives in `assets/demo.js`,
//! which only ever builds the page's text with `textContent`.
//!
//! What a page knows is a small, safe view value the route builds from
//! the visitor's pass and session: names, tags, and addresses of the
//! visitor's own people, the generation's day and the server's clock, and
//! the admin's address from configuration. A page carries no secret, no
//! code, and nothing about anyone else. Every response a page produces is
//! `no-store`, which the routes set.

mod chrome;
mod front;
mod notes;
mod opening;
mod sign_in;

pub(crate) use front::Door;
pub(crate) use front::Notice;
use leptos::prelude::RenderHtml as _;
use leptoskit::prelude::Element;
pub(crate) use notes::Welcome;
pub(crate) use opening::RETRY_SECONDS;
pub(crate) use opening::opening;
use serde::Serialize;
use serde_json::json;
pub(crate) use sign_in::Flow;
pub(crate) use sign_in::Mail;
pub(crate) use sign_in::SignIn;

use crate::visitors::Persona;

/// The admin's shared `--admin-*` colour tokens, with the demo's own
/// stylesheet after them.
const STYLE: &str = include_str!("../../assets/demo.css");

/// The page's behaviour, as one classic script that runs after the
/// markup.
const SCRIPT: &str = include_str!("../../assets/demo.js");

/// A day, in seconds.
const DAY_SECONDS: i64 = 86_400;

/// What every page knows about the demo day it was served in.
#[derive(Clone, Debug)]
pub(crate) struct Context {
	/// The generation's UTC day, `YYYY-MM-DD`: what the reset countdown
	/// and the new-day check are measured against.
	pub day: String,
	/// The server's clock when the page was made, in Unix milliseconds,
	/// so a browser with a wrong clock still counts down to the right
	/// midnight.
	pub now_ms: i64,
	/// The site's host, and the admin's, as the configured origins spell
	/// them, so the pages name the real addresses and never a fixed one.
	pub site_host: String,
	pub admin_host: String,
	/// Where the admin's overview is, from configuration.
	pub admin_url: String,
}

impl Context {
	/// Whole seconds from the server's clock to 00:00 UTC after the
	/// generation's day, never negative.
	pub(crate) fn seconds_to_reset(&self) -> i64 {
		let Ok(day) = chrono::NaiveDate::parse_from_str(&self.day, "%Y-%m-%d") else {
			return 0;
		};
		let end = day
			.and_hms_opt(0, 0, 0)
			.map_or(0, |midnight| midnight.and_utc().timestamp())
			+ DAY_SECONDS;

		(end - self.now_ms.div_euclid(1000)).max(0)
	}

	/// The countdown the way the band words it: `6h 12m`, then minutes,
	/// and `4m 05s` inside the last ten.
	pub(crate) fn countdown(&self) -> String {
		countdown_text(self.seconds_to_reset())
	}

	/// Whether less than ten minutes remain: anything done now is
	/// cleared soon, and the band says so.
	pub(crate) fn is_late(&self) -> bool {
		self.seconds_to_reset() < 600
	}
}

/// How the band words a number of seconds. `assets/demo.js` words the
/// live countdown the same way.
pub(crate) fn countdown_text(seconds: i64) -> String {
	if seconds < 600 {
		return format!("{}m {:02}s", seconds / 60, seconds % 60);
	}

	let hours = seconds / 3600;
	let minutes = (seconds % 3600) / 60;

	if hours > 0 {
		format!("{hours}h {minutes}m")
	} else {
		format!("{minutes}m")
	}
}

/// One of the visitor's own people, as a page shows them.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Person {
	/// `ada` or `grace`.
	pub persona: &'static str,
	/// `Ada #7F3K`.
	pub name: String,
	/// `7F3K`, the short tag after the name.
	pub tag: String,
	/// The synthetic address, which only this browser's pass can sign in
	/// to.
	pub email: String,
	/// Whether this person is the account the request is signed in as.
	pub signed_in: bool,
}

impl Person {
	/// The persona's first name.
	pub(crate) fn first_name(&self) -> &'static str {
		if self.is_ada() { "Ada" } else { "Grace" }
	}

	/// Whether this is the persona the admin lets in.
	pub(crate) fn is_ada(&self) -> bool {
		self.persona == Persona::Ada.slug()
	}
}

/// A host without its port: what a cookie is tied to.
pub(crate) fn host_only(host: &str) -> &str {
	match host.rsplit_once(':') {
		Some((name, port))
			if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) =>
		{
			name
		}
		_ => host,
	}
}

/// The document around one page: head, the root's initial state, the
/// page's data for its script, and the script itself.
///
/// `after_open` is plain HTML placed first in the body, for what only a
/// visitor without scripts sees; it must already be escaped. `state` is
/// serialized with `<`, `>`, `&`, and the two line separators escaped, so no
/// value in it can end the script element it sits in.
pub(crate) fn document(
	title: &str,
	root: &str,
	state: &serde_json::Value,
	body: Element,
	after_open: &str,
) -> String {
	let body = leptoskit::web::render(body).to_html();
	let state = script_json(state);
	let theme = leptoskit_admin::theme_css();

	let mut html = String::with_capacity(body.len() + STYLE.len() + SCRIPT.len() + 2048);
	html.push_str(r#"<!doctype html><html lang="en""#);
	html.push_str(root);
	html.push_str(
		r#"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover"><meta name="color-scheme" content="light dark"><meta name="robots" content="noindex"><title>"#,
	);
	html.push_str(&escape(title));
	// Scripts on: the page may hide what only a script can reveal.
	html.push_str(
		r#"</title><script>document.documentElement.setAttribute("data-js","")</script><style>"#,
	);
	html.push_str(FONT_FACES);
	html.push_str(&theme);
	html.push_str(STYLE);
	html.push_str("</style></head><body>");
	html.push_str(after_open);
	html.push_str(&body);
	html.push_str(r#"<script type="application/json" id="demo-state">"#);
	html.push_str(&state);
	html.push_str("</script><script>");
	html.push_str(SCRIPT);
	html.push_str("</script></body></html>");
	html
}

/// The faces the admin uses, served from this site so no page reaches
/// another origin for them.
const FONT_FACES: &str = r#"@font-face{font-family:"Skribble Sketch";src:url(/demo/assets/sketch.woff2) format("woff2");font-display:swap}@font-face{font-family:"Skribble Mono";src:url(/demo/assets/mono.woff2) format("woff2");font-display:swap}"#;

/// A JSON value written so it can sit inside a `<script>` element.
fn script_json(value: &serde_json::Value) -> String {
	value
		.to_string()
		.replace('<', "\\u003c")
		.replace('>', "\\u003e")
		.replace('&', "\\u0026")
		.replace('\u{2028}', "\\u2028")
		.replace('\u{2029}', "\\u2029")
}

/// Text made safe for HTML text and attribute values.
pub(crate) fn escape(text: &str) -> String {
	text.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
		.replace('\'', "&#39;")
}

/// The data every page hands its script.
fn base_state(page: &str, context: &Context, return_to: &str) -> serde_json::Value {
	json!({
		"page": page,
		"day": context.day,
		"nowMs": context.now_ms,
		"adminUrl": context.admin_url,
		"returnTo": return_to,
	})
}

/// The front door, as a document.
pub(crate) fn front(door: &Door) -> String {
	front::page(door)
}

/// The sign-in page, as a document.
pub(crate) fn sign_in(view: &SignIn) -> String {
	sign_in::page(view)
}

/// Notes' welcome, as a document.
pub(crate) fn notes(welcome: &Welcome) -> String {
	notes::page(welcome)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn context() -> Context {
		Context {
			day: "2026-11-15".to_owned(),
			now_ms: 1_794_764_880_000,
			site_host: "notes.example".to_owned(),
			admin_host: "admin.example".to_owned(),
			admin_url: "http://admin.example/_leptoskit/admin/".to_owned(),
		}
	}

	fn person(persona: &'static str, signed_in: bool) -> Person {
		Person {
			persona,
			name: format!("{persona} #7F3K"),
			tag: "7F3K".to_owned(),
			email: format!("{persona}-7f3k2q9xw4@demo.invalid"),
			signed_in,
		}
	}

	#[test]
	fn every_page_renders_in_every_state() {
		for notice in [Notice::None, Notice::NewDay, Notice::SignedOut] {
			for people in [
				vec![],
				vec![person("ada", false)],
				vec![person("ada", true), person("grace", false)],
			] {
				let door = Door {
					context: context(),
					return_to: "/notes".to_owned(),
					notice: notice.clone(),
					people,
				};
				let html = front(&door);
				assert!(html.contains("Be Ada or Grace for a day."));
			}
		}

		for flow in [
			Flow::Plain,
			Flow::Admin,
			Flow::Switch(person("grace", true)),
		] {
			for identity in [
				None,
				Some(person("ada", false)),
				Some(person("grace", false)),
			] {
				let view = SignIn {
					context: context(),
					return_to: "/oauth/authorize?client_id=x".to_owned(),
					flow: flow.clone(),
					identity,
					preselect: Some("ada"),
					new_day: true,
					mail: Mail {
						held: 1,
						lost_at_ms: Some(1),
					},
				};
				assert!(sign_in(&view).contains("Sign in to Notes"));
			}
		}

		for who in [person("ada", true), person("grace", true)] {
			let ada = who.is_ada();
			let html = notes(&Welcome {
				context: context(),
				person: who,
			});
			assert!(html.contains("Welcome,"));
			assert!(html.contains("Your notebook"));
			assert!(html.contains("data-testid=\"note-text\""));
			assert!(html.contains("data-testid=\"note-save\""));
			assert!(
				html.contains("http://admin.example/_leptoskit/admin/analytics/live"),
				"the way to Live is drawn from configuration"
			);
			assert_eq!(
				html.contains("Grace has no admin role, so Live"),
				!ada,
				"only Grace is told Live will turn her away"
			);
		}

		assert!(
			opening().contains("Notes is opening today&#39;s demo.")
				|| opening().contains("opening today")
		);
	}

	#[test]
	fn the_countdown_is_worded_by_how_close_midnight_is() {
		assert_eq!(countdown_text(6 * 3600 + 12 * 60), "6h 12m");
		assert_eq!(countdown_text(45 * 60), "45m");
		assert_eq!(countdown_text(599), "9m 59s");
		assert_eq!(countdown_text(4 * 60 + 5), "4m 05s");
		assert_eq!(countdown_text(0), "0m 00s");
	}

	#[test]
	fn the_reset_is_measured_from_the_generations_day_and_the_servers_clock() {
		// 2026-11-15T17:48:00Z, with 2026-11-15 the generation's day.
		let context = Context {
			day: "2026-11-15".to_owned(),
			now_ms: 1_794_764_880_000,
			site_host: "notes.example".to_owned(),
			admin_host: "admin.example".to_owned(),
			admin_url: "http://admin.example/".to_owned(),
		};
		assert_eq!(context.seconds_to_reset(), 6 * 3600 + 12 * 60);
		assert!(!context.is_late());

		let late = Context {
			now_ms: 1_794_787_200_000 - 238_000,
			..context.clone()
		};
		assert_eq!(late.seconds_to_reset(), 238);
		assert!(late.is_late());

		// A clock that has passed the day's end never counts below zero.
		let past = Context {
			now_ms: 1_794_787_200_000 + 5_000,
			..context
		};
		assert_eq!(past.seconds_to_reset(), 0);
	}

	#[test]
	fn script_data_cannot_close_its_element() {
		let value = json!({ "name": "</script><script>alert(1)</script>&\u{2028}" });
		let text = script_json(&value);
		assert!(!text.contains('<'), "{text}");
		assert!(!text.contains('>'), "{text}");
		assert!(!text.contains('&'), "{text}");
		assert!(!text.contains('\u{2028}'), "{text}");
		let back: serde_json::Value = serde_json::from_str(&text).expect("still JSON");
		assert_eq!(back, value);
	}

	#[test]
	fn text_is_escaped_for_html() {
		assert_eq!(
			escape("<a href=\"x\">'&'</a>"),
			"&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
		);
	}
}
