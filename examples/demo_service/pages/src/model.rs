//! What a page is rendered from: small, safe values the service builds from
//! the visitor's pass and session.
//!
//! The server renders a page from one of these and hands the same value to
//! the browser, which builds the same tree before it attaches to the markup.
//! A page value carries the visitor's own people, the generation's day and
//! the server's clock, and addresses from configuration. It carries no
//! secret, no code, and nothing about anyone else.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// Which of the two demo people someone is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Persona {
	/// Carries the `admin` role.
	Ada,
	/// Carries no role, so the admin turns her away.
	Grace,
}

impl Persona {
	/// Both, in the order the pages show them.
	pub const ALL: [Self; 2] = [Self::Ada, Self::Grace];

	/// `ada` or `grace`: the name the routes and test ids use.
	#[must_use]
	pub const fn slug(self) -> &'static str {
		match self {
			Self::Ada => "ada",
			Self::Grace => "grace",
		}
	}

	/// The persona's first name.
	#[must_use]
	pub const fn first_name(self) -> &'static str {
		match self {
			Self::Ada => "Ada",
			Self::Grace => "Grace",
		}
	}

	/// The other persona.
	#[must_use]
	pub const fn other(self) -> Self {
		match self {
			Self::Ada => Self::Grace,
			Self::Grace => Self::Ada,
		}
	}

	/// The persona a slug names.
	#[must_use]
	pub fn of_slug(slug: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|persona| persona.slug() == slug)
	}
}

/// What every page knows about the demo day it was served in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
	/// The generation's UTC day, `YYYY-MM-DD`: what the new-day check is
	/// measured against.
	pub day: String,
	/// The server's clock when the page was made, in Unix milliseconds,
	/// so a browser with a wrong clock still counts down to the right
	/// midnight.
	pub now_ms: i64,
	/// 00:00 UTC after the generation's day, in Unix milliseconds.
	pub resets_at_ms: i64,
	/// The site's host, and the admin's, as the configured origins spell
	/// them, so the pages name the real addresses and never a fixed one.
	pub site_host: String,
	/// The admin's host.
	pub admin_host: String,
	/// Where the admin's overview is, from configuration.
	pub admin_url: String,
}

impl Context {
	/// Whole seconds from the server's clock, `elapsed_ms` after the page
	/// was made, to the reset. Never negative.
	#[must_use]
	pub fn seconds_to_reset_after(&self, elapsed_ms: i64) -> i64 {
		let now = self.now_ms.saturating_add(elapsed_ms.max(0));

		(self.resets_at_ms.div_euclid(1000) - now.div_euclid(1000)).max(0)
	}

	/// Whole seconds from the server's clock to the reset when the page was
	/// made.
	#[must_use]
	pub fn seconds_to_reset(&self) -> i64 {
		self.seconds_to_reset_after(0)
	}
}

/// Whether so few seconds remain that anything done now is cleared soon.
#[must_use]
pub const fn is_late(seconds: i64) -> bool {
	seconds < 600
}

/// How the band words a number of seconds: `6h 12m`, then minutes, and
/// `4m 05s` inside the last ten.
#[must_use]
pub fn countdown_text(seconds: i64) -> String {
	if is_late(seconds) {
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
	/// Ada or Grace.
	pub persona: Persona,
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
	#[must_use]
	pub const fn first_name(&self) -> &'static str {
		self.persona.first_name()
	}

	/// Whether this is the persona the admin lets in.
	#[must_use]
	pub fn is_ada(&self) -> bool {
		self.persona == Persona::Ada
	}
}

/// What the front door says before the choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Notice {
	/// Nothing special: a first visit, or a pass that is still good.
	None,
	/// The browser carried a pass from an earlier UTC day. The pass was
	/// cleared; nothing from yesterday exists.
	NewDay,
	/// The visitor just signed out. Their pass, and so their person,
	/// stays theirs until the reset.
	SignedOut,
}

/// What the front door is rendered from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Door {
	/// The day and the addresses.
	pub context: Context,
	/// The validated local path a sign-in should end at.
	pub return_to: String,
	/// What to say above the choice.
	pub notice: Notice,
	/// This browser's own people, none without a pass.
	pub people: Vec<Person>,
}

/// One page, with everything it is rendered from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Page {
	/// The front door.
	Front(Door),
}

impl Page {
	/// The page's document title.
	#[must_use]
	pub const fn title(&self) -> &'static str {
		match self {
			Self::Front(_) => "Notes: a leptoskit demo",
		}
	}

	/// The day and the addresses the page was made with.
	#[must_use]
	pub const fn context(&self) -> &Context {
		match self {
			Self::Front(door) => &door.context,
		}
	}
}

/// A value percent-encoded the way a form encodes a query value, for a
/// validated return target riding in a link.
#[must_use]
pub fn query_value(value: &str) -> String {
	let mut encoded = String::with_capacity(value.len());

	for byte in value.bytes() {
		match byte {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
				encoded.push(char::from(byte));
			}
			b' ' => encoded.push('+'),
			_ => {
				encoded.push('%');
				encoded.push(char::from(HEX[usize::from(byte >> 4)]));
				encoded.push(char::from(HEX[usize::from(byte & 0x0F)]));
			}
		}
	}

	encoded
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

#[cfg(test)]
mod tests {
	use super::*;

	fn context(now_ms: i64) -> Context {
		Context {
			day: "2026-11-15".to_owned(),
			now_ms,
			resets_at_ms: 1_794_787_200_000,
			site_host: "notes.example".to_owned(),
			admin_host: "admin.example".to_owned(),
			admin_url: "http://admin.example/".to_owned(),
		}
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
	fn the_reset_is_measured_from_the_servers_clock() {
		// 2026-11-15T17:48:00Z, with 2026-11-15 the generation's day.
		let noon = context(1_794_764_880_000);
		assert_eq!(noon.seconds_to_reset(), 6 * 3600 + 12 * 60);
		assert!(!is_late(noon.seconds_to_reset()));
		// The browser counts forward from the server's clock.
		assert_eq!(noon.seconds_to_reset_after(60_000), 6 * 3600 + 11 * 60);

		let late = context(1_794_787_200_000 - 238_000);
		assert_eq!(late.seconds_to_reset(), 238);
		assert!(is_late(late.seconds_to_reset()));

		// A clock that has passed the day's end never counts below zero.
		assert_eq!(context(1_794_787_200_000 + 5_000).seconds_to_reset(), 0);
		assert_eq!(late.seconds_to_reset_after(86_400_000), 0);
	}

	#[test]
	fn a_query_value_is_form_encoded() {
		assert_eq!(query_value("/notes"), "%2Fnotes");
		assert_eq!(
			query_value("/oauth/authorize?client_id=a b&x=1"),
			"%2Foauth%2Fauthorize%3Fclient_id%3Da+b%26x%3D1"
		);
	}

	#[test]
	fn a_slug_names_its_persona() {
		assert_eq!(Persona::of_slug("ada"), Some(Persona::Ada));
		assert_eq!(Persona::of_slug("grace"), Some(Persona::Grace));
		assert_eq!(Persona::of_slug("Ada"), None);
	}
}
