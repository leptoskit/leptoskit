//! `CommandBlock`: commands to paste into a terminal, with a copy button.

use leptoskit::native::Live;
use leptoskit::prelude::*;

use crate::style;

/// How long the button says "Copied" before it says "Copy" again.
pub const COPIED_FOR: std::time::Duration = std::time::Duration::from_secs(2);

/// Commands on paper, each on its own line after a `$`, and one button that
/// copies them all. After a copy the button says "Copied" for two seconds,
/// and a screen reader hears it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandBlock {
	/// The commands, one per line.
	pub commands: Vec<String>,
}

impl CommandBlock {
	/// One command.
	#[must_use]
	pub fn one(command: impl Into<String>) -> Self {
		Self {
			commands: vec![command.into()],
		}
	}

	/// What the button copies.
	#[must_use]
	pub fn text(&self) -> String {
		self.commands.join("\n")
	}

	/// The block. `copied` holds what was copied last, while that's recent:
	/// one signal serves every block on the screen.
	#[must_use]
	pub fn build(&self, copied: RwSignal<Option<String>>) -> Element {
		let text = self.text();
		let is_copied = copied.with(|copied| copied.as_deref() == Some(text.as_str()));
		let lines = self.commands.iter().map(|command| {
			View()
				.class("flex-row gap-2 px-3")
				.child(
					Text("$")
						.class(style::CODE)
						.class("text-[var(--cloud-ink-3)]")
						.aria_hidden(true),
				)
				.child(Text(command.clone()).class(style::CODE))
		});

		View()
			.class(style::CODE_BLOCK_PAPER)
			.class("w-full flex-row flex-wrap items-center gap-2 pr-2")
			.child(View().class("flex-1 flex-col gap-1").children(lines))
			.child(
				Pressable(if is_copied {
					"Copied".to_owned()
				} else {
					format!("Copy {}", self.commands.join(", "))
				})
				.class(style::BUTTON_QUIET)
				.test_id("copy-commands")
				.on_press({
					let text = text.clone();
					move || copy(&text, copied)
				})
				.child(Text(if is_copied { "Copied" } else { "Copy" })),
			)
			.children(is_copied.then(|| {
				Status("Copied to the clipboard.")
					.aria_live(Live::Polite)
					.class(style::LABEL)
					.class("w-full px-3")
			}))
	}
}

/// Copies `text`, then says so for [`COPIED_FOR`]. When the clipboard
/// refuses, the button keeps its label rather than claim a copy, and the
/// command stays on screen to select by hand.
#[cfg(all(target_arch = "wasm32", feature = "hydrate"))]
fn copy(text: &str, copied: RwSignal<Option<String>>) {
	let text = text.to_owned();
	leptos::task::spawn_local(async move {
		if leptoskit_plugin_clipboard::web::write_text(&text)
			.await
			.is_err()
		{
			copied.set(None);
			return;
		}

		copied.set(Some(text.clone()));
		let shown = text.clone();
		let reset = move || {
			if copied.with_untracked(|copied| copied.as_deref() == Some(shown.as_str())) {
				copied.set(None);
			}
		};

		// Without a timer, "Copied" would never go, so it goes now.
		if leptos::prelude::set_timeout(reset, COPIED_FOR).is_err()
			&& copied.with_untracked(|copied| copied.as_deref() == Some(text.as_str()))
		{
			copied.set(None);
		}
	});
}

/// Off the browser there's no clipboard to write to: the press only marks
/// the block copied, which the component tests read.
#[cfg(not(all(target_arch = "wasm32", feature = "hydrate")))]
fn copy(text: &str, copied: RwSignal<Option<String>>) {
	copied.set(Some(text.to_owned()));
}
