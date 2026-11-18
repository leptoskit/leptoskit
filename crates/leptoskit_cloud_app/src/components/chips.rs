//! The top bar's chips (shipping, cost, offline) and the environment pills.

use std::time::Duration;

use leptoskit::native::Current;
use leptoskit::native::Live;
use leptoskit::native::Role;
use leptoskit::prelude::*;

use super::Tone;
use crate::data::InFlight;
use crate::data::Spend;
use crate::data::plan_name;
use crate::style;

/// An environment, as its pill shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Environment {
	/// `production`: ember fill.
	Production,
	/// `staging`: ember outline.
	Staging,
	/// A preview, by name: sunken fill.
	Preview(String),
}

impl Environment {
	/// The environment named `name`.
	#[must_use]
	pub fn named(name: &str) -> Self {
		match name {
			"production" => Self::Production,
			"staging" => Self::Staging,
			other => Self::Preview(other.to_owned()),
		}
	}

	/// Its name.
	#[must_use]
	pub fn name(&self) -> &str {
		match self {
			Self::Production => "production",
			Self::Staging => "staging",
			Self::Preview(name) => name,
		}
	}

	fn pill(&self) -> &'static str {
		match self {
			Self::Production => style::PILL_PRODUCTION,
			Self::Staging => style::PILL_STAGING,
			Self::Preview(_) => style::PILL_PREVIEW,
		}
	}
}

/// An environment's pill: production filled, staging outlined, previews
/// sunken, each with its name, so colour is never the only difference.
pub fn env_pill(environment: &Environment) -> Element {
	Text(environment.name().to_owned()).class(environment.pill())
}

/// `EnvRow`: a wrapping row of environment pills, each a link, with an ink
/// underline under the selected one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvRow {
	/// The environments, production first, then staging, then previews by
	/// recency.
	pub environments: Vec<Environment>,
	/// The one being looked at.
	pub selected: Option<String>,
	/// Where each pill goes: `{env}` is replaced with its name.
	pub href: String,
}

impl EnvRow {
	/// The row.
	#[must_use]
	pub fn build(&self) -> Element {
		let pills = self.environments.iter().map(|environment| {
			let selected = self.selected.as_deref() == Some(environment.name());
			let mut pill = Pressable(format!("Environment {}", environment.name()))
				.href(self.href.replace("{env}", environment.name()))
				.class("min-h-11 flex-col items-center justify-center gap-1 px-1")
				.child(env_pill(environment))
				.child(View().class(if selected {
					"h-0.5 w-full rounded-full bg-[var(--cloud-ink)]"
				} else {
					"h-0.5 w-full"
				}));

			if selected {
				pill = pill.aria_current(Current::Page);
			}

			pill
		});

		View()
			.class("flex-row flex-wrap items-center gap-2")
			.role(Role::List)
			.accessibility_label("Environments")
			.children(pills)
	}
}

/// `ShippingChip`: releases in flight in the organisation, newest first.
/// Hidden when nothing is shipping, or while the console can't know.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShippingChip {
	/// What's in flight.
	pub in_flight: InFlight,
}

impl ShippingChip {
	/// The chip, or nothing.
	#[must_use]
	pub fn build(&self) -> Option<Element> {
		let InFlight::Releases(releases) = &self.in_flight else {
			return None;
		};
		let first = releases.first()?;
		let words = match releases.len() {
			1 => format!("{} v{} · {}", first.project, first.number, first.stage),
			count => format!("{count} shipping"),
		};

		Some(
			Pressable(format!("Shipping: {words}"))
				.href(first.href.clone())
				.class(style::CHIP)
				.child(
					View()
						.class(style::CHIP_FACE)
						.child(
							View()
								.class("h-2 w-2 rounded-full bg-[var(--cloud-ember)] animate-pulse")
								.aria_hidden(true),
						)
						.child(Text(words)),
				),
		)
	}
}

/// `CostMeter` as the top bar's chip: spend against the cap. A number the
/// console doesn't have is never invented: until `billing::usage` exists the
/// chip names the plan and says spend isn't available yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CostMeter {
	/// The organisation's plan, such as `hobby`.
	pub plan: String,
	/// What it has spent, when the console can know.
	pub spend: Spend,
	/// Where the chip goes: the organisation's billing page, once there is
	/// one.
	pub href: Option<String>,
}

impl CostMeter {
	/// The words the chip shows, and its tone when it needs one.
	#[must_use]
	pub fn words(&self) -> (String, Option<Tone>) {
		match &self.spend {
			Spend::Known {
				spent_cents,
				cap_cents,
				forecast_cents,
			} => {
				let words = format!("{} of {}", dollars(*spent_cents), dollars(*cap_cents));

				if spent_cents >= cap_cents {
					(format!("{} {words}", Tone::Bad.icon()), Some(Tone::Bad))
				} else if forecast_cents > cap_cents {
					(format!("{} {words}", Tone::Warn.icon()), Some(Tone::Warn))
				} else {
					(words, None)
				}
			}
			Spend::NotAvailable => (plan_name(&self.plan), None),
		}
	}

	/// What a screen reader hears.
	#[must_use]
	pub fn label(&self) -> String {
		match &self.spend {
			Spend::Known {
				spent_cents,
				cap_cents,
				forecast_cents,
			} => {
				let spent = format!(
					"Spend this month: {} of a {} cap",
					dollars(*spent_cents),
					dollars(*cap_cents)
				);

				if spent_cents >= cap_cents {
					format!("{spent}. The cap is reached.")
				} else if forecast_cents > cap_cents {
					format!("{spent}. On course to pass it.")
				} else {
					format!("{spent}.")
				}
			}
			Spend::NotAvailable => {
				format!(
					"Plan: {}. Spend isn't available yet.",
					plan_name(&self.plan)
				)
			}
		}
	}

	/// The chip.
	#[must_use]
	pub fn build(&self) -> Element {
		let (words, tone) = self.words();
		let mut text = Text(words);

		if let Some(tone) = tone {
			text = text.class(tone.text()).class("font-semibold");
		}

		let face = View().class(style::CHIP_FACE).child(text.aria_hidden(true));

		match &self.href {
			Some(href) => {
				Pressable(self.label())
					.href(href.clone())
					.class(style::CHIP)
					.test_id("cost-chip")
					.child(face)
			}
			None => {
				View()
					.class(style::CHIP)
					.role(Role::Status)
					.accessibility_label(self.label())
					.test_id("cost-chip")
					.child(face)
			}
		}
	}
}

fn dollars(cents: u64) -> String {
	format!("${}.{:02}", cents / 100, cents % 100)
}

/// Where the console's live connection is, as the offline chip reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
	/// Connected, or not yet known: nothing to say.
	Fine,
	/// The connection dropped; the next try is in `next_in`.
	Reconnecting {
		/// How long until the next try.
		next_in: Duration,
	},
	/// Gone, after it had been open.
	Offline,
}

/// `OfflineChip`: says when the console can't reach the cloud, and that what
/// it shows may be out of date. Hidden while connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OfflineChip {
	/// The connection.
	pub connection: Connection,
}

impl OfflineChip {
	/// The chip, or nothing.
	#[must_use]
	pub fn build(&self) -> Option<Element> {
		let words = match self.connection {
			Connection::Fine => return None,
			Connection::Reconnecting { next_in } => {
				format!(
					"Reconnecting · trying again in {}s",
					next_in.as_secs().max(1)
				)
			}
			Connection::Offline => "Offline · showing what was last loaded".to_owned(),
		};

		Some(
			Status(words.clone())
				.aria_live(Live::Polite)
				.class(style::CHIP_FACE)
				.class("text-xs font-medium")
				.class(Tone::Warn.text())
				.test_id("offline-chip"),
		)
	}
}
