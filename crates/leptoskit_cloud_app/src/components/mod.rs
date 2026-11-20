//! The pieces every screen is built from (`docs/agents/cloud-design.md` §5).
//!
//! The ones the admin has too (`Tone`, `status_word`, `mint_tab`, `Kind`,
//! `link_button`, `link`, `raised`, `margin_note`, `mark`, `icon`,
//! `env_pill`, and the account screen's `card`) keep the admin's names and
//! behaviour, so moving both into one crate later is mechanical. Behaviour
//! the admin wires in its script is a signal here.

mod chips;
mod command_block;
mod mark;
mod qr;
mod sheet;
mod turn_card;

pub use chips::Connection;
pub use chips::CostMeter;
pub use chips::EnvRow;
pub use chips::Environment;
pub use chips::OfflineChip;
pub use chips::ShippingChip;
pub use chips::env_pill;
pub use command_block::COPIED_FOR;
pub use command_block::CommandBlock;
use leptoskit::native::Live;
use leptoskit::prelude::*;
pub use mark::ICON_ACCOUNT;
pub use mark::ICON_APPS;
pub use mark::ICON_BILLING;
pub use mark::ICON_INBOX;
pub use mark::ICON_PLUS;
pub use mark::ICON_RELEASES;
pub use mark::ICON_SEARCH;
pub use mark::ICON_SWITCH;
pub use mark::icon;
pub use mark::mark;
pub use qr::QrCode;
pub use sheet::Stamp;
pub use sheet::StampSize;
pub use sheet::sheet;
pub use sheet::sheet_rule;
pub use turn_card::TurnCard;
pub use turn_card::Turned;
pub use turn_card::mint_tab;

use crate::style;

/// A state word's tone, always shown with an icon and a word, never colour
/// alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
	/// ✓ Live, Running, Approved.
	Good,
	/// ! Waiting, Needs you.
	Warn,
	/// ✕ Failed, Denied.
	Bad,
	/// • Queued, Cancelled.
	Info,
}

impl Tone {
	/// The tone's icon.
	#[must_use]
	pub const fn icon(self) -> &'static str {
		match self {
			Self::Good => "✓",
			Self::Warn => "!",
			Self::Bad => "✕",
			Self::Info => "•",
		}
	}

	/// The text colour class for this tone.
	#[must_use]
	pub const fn text(self) -> &'static str {
		match self {
			Self::Good => "text-[var(--cloud-good)]",
			Self::Warn => "text-[var(--cloud-warn)]",
			Self::Bad => "text-[var(--cloud-bad)]",
			Self::Info => "text-[var(--cloud-info)]",
		}
	}

	/// The border colour class for this tone, for stamps.
	#[must_use]
	pub const fn border(self) -> &'static str {
		match self {
			Self::Good => "border-[var(--cloud-good)]",
			Self::Warn => "border-[var(--cloud-warn)]",
			Self::Bad => "border-[var(--cloud-bad)]",
			Self::Info => "border-[var(--cloud-info)]",
		}
	}
}

/// A state word with its icon: `✓ Approved`.
pub fn status_word(tone: Tone, word: impl Into<String>) -> Element {
	Text(format!("{} {}", tone.icon(), word.into()))
		.class("text-sm font-semibold")
		.class(tone.text())
}

/// A sentence that says something went wrong, announced as it appears, with
/// ✕ so it never relies on colour.
pub fn problem(text: impl Into<String>) -> Element {
	Alert(format!("✕ {}", text.into())).class(style::FIELD_ERROR)
}

/// A sentence that reports progress or an outcome politely.
pub fn notice(text: impl Into<String>) -> Element {
	Status(text.into())
		.aria_live(Live::Polite)
		.class(style::SECONDARY)
}

/// What a button looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	/// Ink fill on an ember-warm offset block.
	Primary,
	/// Surface with a control-contrast border.
	Secondary,
	/// Text only.
	Quiet,
	/// Bad text and border.
	Danger,
	/// Bad text only.
	DangerQuiet,
}

impl Kind {
	/// The button's classes.
	#[must_use]
	pub const fn class_name(self) -> &'static str {
		match self {
			Self::Primary => style::BUTTON_PRIMARY,
			Self::Secondary => style::BUTTON_SECONDARY,
			Self::Quiet => style::BUTTON_QUIET,
			Self::Danger => style::BUTTON_DANGER,
			Self::DangerQuiet => style::BUTTON_DANGER_QUIET,
		}
	}
}

/// A link that looks like a button: a real `<a href>` on the web.
pub fn link_button(kind: Kind, label: &str, href: impl Into<String>) -> Element {
	let link = Pressable(label)
		.href(href)
		.class(kind.class_name())
		.child(Text(label));

	if kind == Kind::Primary {
		raised(link)
	} else {
		link
	}
}

/// A plain text link.
pub fn link(label: impl Into<String>, href: impl Into<String>) -> Element {
	let label = label.into();
	Pressable(label.clone())
		.href(href)
		.class("min-h-11 justify-center cursor-pointer")
		.child(Text(label).class(style::LINK))
}

/// A button that runs `action`, styled as `kind`, before any elevation. A
/// busy button keeps its place, says what it is doing, and shows a spinner.
pub fn plain_button(
	kind: Kind,
	label: impl Into<String>,
	busy: Option<&str>,
	action: impl Fn() + Send + Sync + 'static,
) -> Element {
	let label = label.into();
	let text = busy.map_or_else(|| label.clone(), str::to_owned);
	let mut button = Pressable(text.clone())
		.class(kind.class_name())
		.role(leptoskit::native::Role::Button)
		.enabled(busy.is_none())
		.aria_busy(busy.is_some())
		.on_press(action);

	if busy.is_some() {
		button = button.child(ActivityIndicator(true).aria_hidden(true));
	}

	button.child(Text(text))
}

/// A button that runs `action`, styled as `kind`: a primary one sits on its
/// offset block.
pub fn button(
	kind: Kind,
	label: impl Into<String>,
	busy: Option<&str>,
	action: impl Fn() + Send + Sync + 'static,
) -> Element {
	let button = plain_button(kind, label, busy, action);

	if kind == Kind::Primary {
		raised(button)
	} else {
		button
	}
}

/// A primary button across its row, on its offset block: a sheet's main
/// action.
pub fn block_button(
	label: impl Into<String>,
	busy: Option<&str>,
	action: impl Fn() + Send + Sync + 'static,
) -> Element {
	raised_block(plain_button(Kind::Primary, label, busy, action).class("w-full"))
}

/// Puts a control on the brand's hard offset block, the elevation that
/// replaces shadows.
pub fn raised(control: Element) -> Element {
	View()
		.class("relative self-start")
		.child(View().class(style::BUTTON_PRIMARY_BLOCK).aria_hidden(true))
		.child(control)
}

/// A raised control that fills its row, for the sheets' main actions.
pub fn raised_block(control: Element) -> Element {
	View()
		.class("relative w-full")
		.child(View().class(style::BUTTON_PRIMARY_BLOCK).aria_hidden(true))
		.child(control)
}

/// A margin note in the hand, with its mint pencil line.
pub fn margin_note(text: impl Into<String>) -> Element {
	View()
		.class(style::NOTE)
		.role(leptoskit::native::Role::Note)
		.child(View().class(style::NOTE_LINE).aria_hidden(true))
		.child(Text(text.into()).class(style::NOTE_TEXT))
}

/// A placeholder block while something loads: a fixed shape, no shimmer.
pub fn skeleton(class: &str) -> Element {
	View().class(style::SKELETON).class(class).aria_hidden(true)
}

/// The keyboard focus ring: a 2px ember border on a wrapper, set while the
/// control inside has focus. `focus` holds the focused control's `id`, so one
/// signal serves every ring on the screen.
pub fn focus_ring(focus: RwSignal<Option<String>>, id: &str, control: Element) -> Element {
	let focused = focus.with(|current| current.as_deref() == Some(id));
	let entered = id.to_owned();
	let left = id.to_owned();
	let control = control
		.on(NativeEventSlot::Focus, move |_| {
			focus.set(Some(entered.clone()));
			EventOutcome::Unhandled
		})
		.on(NativeEventSlot::Blur, move |_| {
			if focus.with_untracked(|current| current.as_deref() == Some(left.as_str())) {
				focus.set(None);
			}

			EventOutcome::Unhandled
		});

	View()
		.class(if focused {
			style::FOCUS_RING
		} else {
			style::FOCUS_RING_OFF
		})
		.class("self-start")
		.child(control)
}

/// A labelled text field with its error under it. The label names the field
/// for assistive technology and stays visible above it.
pub fn field(id: &str, label: &str, input: Element, error: Option<&str>) -> Element {
	let error_id = format!("{id}-error");
	let mut input = input
		.id(id)
		.aria_label(label)
		.class(if error.is_some() {
			style::INPUT_ERROR
		} else {
			style::INPUT
		})
		.class("w-full");

	if error.is_some() {
		input = input
			.aria_invalid(true)
			.aria_described_by([error_id.clone()]);
	}

	View()
		.class("flex-col gap-1.5 w-full")
		.child(Text(label).class(style::LABEL).aria_hidden(true))
		.child(input)
		.children(error.map(|error| problem(error).id(error_id)))
}
