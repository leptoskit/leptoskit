//! The confirm sheet: a modal that asks before an action runs.
//!
//! An action button calls [`Confirm::ask`] with what it does and what to run
//! once the person agrees. The sheet is a [`Modal`](leptoskit::prelude::Modal),
//! so the browser keeps focus inside it and makes the page behind it inert;
//! Escape and a press outside it cancel. Either way focus goes back to the
//! button that asked.
//!
//! A sheet can also take a line of text, such as a name typed to confirm a
//! delete, and hold its button until the text matches; and its button can be
//! a download link, so a confirmed export is the browser's own download.
//! A sheet can offer choices, such as which dashboard to add a card to,
//! with the line of text for the choice that needs one.

use std::sync::Arc;

use leptos::context::use_context;
use leptos::prelude::GetValue as _;
use leptos::prelude::SetValue as _;
use leptos::prelude::StoredValue;
use leptos::prelude::UpdateValue as _;
use leptoskit::aria::Dialog;
use leptoskit::native::EventOutcome;
use leptoskit::native::FocusRequest;
use leptoskit::native::NativeEventSlot;
use leptoskit::prelude::*;

use crate::components;
use crate::style;

/// What the sheet asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
	/// The title, which is also the confirming button's label.
	pub title: String,
	/// What happens, with `commands` in backticks.
	pub explanation: String,
	/// A line to type, when the sheet takes one.
	pub field: Option<Field>,
	/// The text the field must hold before the button works.
	pub must_match: Option<String>,
	/// Whether the button does something that can't be undone.
	pub danger: bool,
	/// The address the button downloads, and the file's name, when it is a
	/// download rather than a press.
	pub download: Option<(String, String)>,
	/// What the field holds when the sheet opens.
	pub value: String,
	/// The choices, each a value and its label; the field is for the one
	/// whose value is empty.
	pub choices: Vec<(String, String)>,
}

/// What the person answered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
	/// What they typed, trimmed.
	pub text: String,
	/// The value of the choice they made, when the sheet offered choices.
	pub choice: Option<String>,
}

/// A line of text the sheet asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
	pub label: String,
	pub placeholder: String,
}

impl Ask {
	/// A sheet that asks `title`, explaining what happens.
	#[must_use]
	pub fn new(title: impl Into<String>, explanation: impl Into<String>) -> Self {
		Self {
			title: title.into(),
			explanation: explanation.into(),
			field: None,
			must_match: None,
			danger: false,
			download: None,
			value: String::new(),
			choices: Vec::new(),
		}
	}

	/// Asks for a line of text, starting as `value`, that can't be empty.
	#[must_use]
	pub fn named(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
		self.field = Some(Field {
			label: label.into(),
			placeholder: String::new(),
		});
		self.value = value.into();
		self
	}

	/// Offers `choices`, each a value and its label, the first chosen. A
	/// choice whose value is empty is the one the field is for.
	#[must_use]
	pub fn choices(mut self, choices: Vec<(String, String)>) -> Self {
		self.choices = choices;
		self
	}

	/// Whether the field is for `choice`: always, without choices.
	#[must_use]
	pub fn field_for(&self, choice: usize) -> bool {
		self.choices
			.get(choice)
			.is_none_or(|(value, _)| value.is_empty())
	}

	/// Asks the person to type `expected` before the button works.
	#[must_use]
	pub fn typed(mut self, label: impl Into<String>, expected: impl Into<String>) -> Self {
		let expected = expected.into();
		self.field = Some(Field {
			label: label.into(),
			placeholder: expected.clone(),
		});
		self.must_match = Some(expected);
		self
	}

	/// Marks the button as one that can't be undone.
	#[must_use]
	pub fn danger(mut self) -> Self {
		self.danger = true;
		self
	}

	/// Makes the button download `href` as `file_name`.
	#[must_use]
	pub fn download(mut self, href: impl Into<String>, file_name: impl Into<String>) -> Self {
		self.download = Some((href.into(), file_name.into()));
		self
	}

	/// Whether `typed`, with `choice` chosen, lets the button work.
	#[must_use]
	pub fn accepts(&self, typed: &str, choice: usize) -> bool {
		match &self.must_match {
			Some(expected) => typed.trim() == expected,
			// A field that names something can't be left empty.
			None => self.field.is_none() || !self.field_for(choice) || !typed.trim().is_empty(),
		}
	}
}

/// What runs once the person confirms, with their answer.
type Yes = Arc<dyn Fn(Answer) + Send + Sync>;

/// The page's confirm sheet. The shell provides one as context.
#[derive(Clone, Copy)]
pub struct Confirm {
	ask: RwSignal<Option<Ask>>,
	/// What the person has typed into the sheet's field.
	typed: RwSignal<String>,
	/// The choice they made, by its place.
	chosen: RwSignal<usize>,
	yes: StoredValue<Option<Yes>>,
	/// The control that asked, which gets focus back.
	opener: StoredValue<Option<FocusRequest>>,
}

impl Confirm {
	#[must_use]
	pub fn new() -> Self {
		Self {
			ask: RwSignal::new(None),
			typed: RwSignal::new(String::new()),
			chosen: RwSignal::new(0),
			yes: StoredValue::new(None),
			opener: StoredValue::new(None),
		}
	}

	/// The page's sheet, when the page runs as this crate's.
	#[must_use]
	pub fn current() -> Option<Self> {
		use_context::<Self>()
	}

	/// Whether the sheet is asking now, without tracking it.
	#[cfg_attr(
		not(feature = "hydrate"),
		expect(dead_code, reason = "only the browser re-reads a builder")
	)]
	pub(crate) fn asking_untracked(&self) -> bool {
		self.ask.with_untracked(Option::is_some)
	}

	/// Shows the sheet; `yes` runs with the answer if the person confirms,
	/// and `from` gets focus back either way.
	pub fn ask(
		&self,
		ask: Ask,
		from: Option<FocusRequest>,
		yes: impl Fn(Answer) + Send + Sync + 'static,
	) {
		self.yes.set_value(Some(Arc::new(yes)));
		self.opener.set_value(from);
		self.typed.set(ask.value.clone());
		self.chosen.set(0);
		self.ask.set(Some(ask));
	}

	/// Whether the sheet is asking.
	#[must_use]
	pub fn is_open(&self) -> bool {
		self.ask.with(Option::is_some)
	}

	/// Closes the sheet and runs what it was asked for, if the answer lets
	/// it.
	fn confirm(&self) {
		let typed = self.typed.get_untracked();
		let chosen = self.chosen.get_untracked();
		let answer = self.ask.with_untracked(|ask| {
			let ask = ask.as_ref().filter(|ask| ask.accepts(&typed, chosen))?;

			Some(Answer {
				text: if ask.field_for(chosen) {
					typed.trim().to_owned()
				} else {
					String::new()
				},
				choice: ask.choices.get(chosen).map(|(value, _)| value.clone()),
			})
		});
		let Some(answer) = answer else {
			return;
		};

		if let Some(yes) = self.close() {
			yes(answer);
		}
	}

	fn close(&self) -> Option<Yes> {
		self.ask.set(None);

		if let Some(opener) = self.opener.get_value() {
			// Once the modal has closed: while it is open, the page behind it
			// is inert and can't take focus.
			after_close(move || opener.request());
		}

		self.opener.set_value(None);
		self.yes.try_update_value(Option::take).flatten()
	}

	/// The sheet, closed until something asks.
	#[must_use]
	pub fn view(&self) -> Element {
		let confirm = *self;
		let ask = self
			.ask
			.get()
			.unwrap_or_else(|| Ask::new("Confirm", String::new()));
		let open = self.ask.with(Option::is_some);
		let typed = self.typed.get();
		let chosen = self.chosen.get();
		let ready = ask.accepts(&typed, chosen);
		let dialog = Dialog::new("confirm", ask.title.clone()).description(ask.explanation.clone());
		let mut panel = View()
			.class(
				"w-full max-w-[440px] flex-col gap-3.5 rounded-xl border \
				 border-[var(--admin-line)] bg-[var(--admin-surface)] p-5",
			)
			.child(
				dialog
					.title_element(2)
					.class("text-[17px] font-semibold text-[var(--admin-ink)]"),
			)
			.child(
				components::prose(&ask.explanation)
					.id("confirm-description")
					.class(style::SECONDARY),
			);

		if !ask.choices.is_empty() {
			let chosen = chosen.min(ask.choices.len() - 1);
			let mut group = leptoskit::aria::RadioGroup::new(
				"confirm-choice",
				ask.choices.iter().map(|(_, label)| label.clone()),
				Some(chosen),
			);

			for (index, (_, label)) in ask.choices.iter().enumerate() {
				group = group.option(
					index,
					View()
						.class(
							"flex-row items-center gap-2.5 rounded-lg px-2 py-1.5 cursor-pointer \
							 hover:bg-[var(--admin-sunken)]",
						)
						.child(
							View()
								.class(if index == chosen {
									"h-4 w-4 shrink-0 rounded-full border-[5px] \
									 border-[var(--admin-ink)]"
								} else {
									"h-4 w-4 shrink-0 rounded-full border-2 \
									 border-[var(--admin-line-strong)]"
								})
								.aria_hidden(true),
						)
						.child(Text(label.clone()).class("text-sm text-[var(--admin-ink)]")),
				);
			}

			panel = panel.child(
				Element::from(group.on_change(move |index| confirm.chosen.set(index)))
					.aria_label(ask.title.clone())
					.class("flex-col gap-1")
					.test_id("confirm-choices"),
			);
		}

		if let Some(field) = &ask.field {
			let enabled = ask.field_for(chosen);
			panel = panel.child(
				View()
					.class("flex-col gap-1.5")
					.child(
						Text(field.label.clone())
							.class("text-sm font-semibold text-[var(--admin-ink-2)]"),
					)
					.child(
						TextInput()
							.id("confirm-input")
							.value(typed)
							.placeholder(field.placeholder.clone())
							.aria_label(field.label.clone())
							.enabled(enabled)
							.class(
								"rounded-lg border border-[var(--admin-line-strong)] \
								 bg-[var(--admin-surface)] px-3 py-2 text-sm \
								 text-[var(--admin-ink)]",
							)
							.test_id("confirm-input")
							.on_change(move |text| confirm.typed.set(text.to_owned()))
							.on(NativeEventSlot::Submit, move |_| {
								confirm.confirm();
								EventOutcome::Handled
							}),
					),
			);
		}

		let yes_class = if ask.danger {
			style::BUTTON_DANGER_FILLED
		} else {
			style::BUTTON_PRIMARY
		};
		let yes = match &ask.download {
			// A download is the browser's own: the link is followed, and the
			// sheet closes once it has been.
			Some((href, file_name)) => {
				Pressable(ask.title.clone())
					.href(href.clone())
					.download(file_name.clone())
					.child(Text(ask.title.clone()))
					.on(NativeEventSlot::Press, move |_| {
						after_close(move || {
							confirm.close();
						});
						EventOutcome::Unhandled
					})
			}
			None => {
				Button(ask.title.clone())
					.enabled(ready)
					.on_press(move || confirm.confirm())
			}
		};
		let yes = yes
			.id("confirm-yes")
			.class(yes_class)
			.class(if ready { "" } else { "opacity-50" })
			.test_id("confirm-yes");

		panel = panel.child(
			View()
				.class("flex-row flex-wrap justify-end gap-2.5")
				.child(
					Button("Cancel")
						.class(style::BUTTON_SECONDARY)
						.test_id("confirm-no")
						.on_press(move || {
							confirm.close();
						}),
				)
				.child(yes),
		);

		dialog
			.modal(open)
			.class("items-center justify-center bg-[#16111f]/45 p-4")
			.test_id("confirm-sheet")
			.initial_focus(if ask.field.is_some() && ask.field_for(0) {
				"confirm-input"
			} else {
				"confirm-yes"
			})
			.on_dismiss(move || {
				confirm.close();
			})
			.child(panel)
	}
}

impl Default for Confirm {
	fn default() -> Self {
		Self::new()
	}
}

/// Runs `then` once the closed sheet has left the page, or once the
/// browser has followed the link just pressed: after the render that closes
/// it and the renderer's task that closes the dialog.
///
/// The browser's own timer, not leptos's: leptos ties a timer to the
/// reactive owner that set it, and the press that closes the sheet belongs
/// to the build that closing replaces, so its timer would be cancelled
/// before it ran.
fn after_close(then: impl FnOnce() + 'static) {
	#[cfg(feature = "hydrate")]
	{
		let callback = wasm_bindgen::closure::Closure::once_into_js(then);

		if let Some(window) = web_sys::window() {
			let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
				wasm_bindgen::JsCast::unchecked_ref(&callback),
				0,
			);
		}
	}
	#[cfg(not(feature = "hydrate"))]
	then();
}
