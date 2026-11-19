//! The confirm sheet: a modal that asks before an action runs.
//!
//! An action button calls [`Confirm::ask`] with what it does and what to run
//! once the person agrees. The sheet is a [`Modal`](leptoskit::prelude::Modal),
//! so the browser keeps focus inside it and makes the page behind it inert;
//! Escape and a press outside it cancel. Either way focus goes back to the
//! button that asked.

use std::sync::Arc;

use leptos::context::use_context;
use leptos::prelude::GetValue as _;
use leptos::prelude::SetValue as _;
use leptos::prelude::StoredValue;
use leptos::prelude::UpdateValue as _;
use leptoskit::aria::Dialog;
use leptoskit::native::FocusRequest;
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
}

type Yes = Arc<dyn Fn() + Send + Sync>;

/// The page's confirm sheet. The shell provides one as context.
#[derive(Clone, Copy)]
pub struct Confirm {
	ask: RwSignal<Option<Ask>>,
	yes: StoredValue<Option<Yes>>,
	/// The control that asked, which gets focus back.
	opener: StoredValue<Option<FocusRequest>>,
}

impl Confirm {
	#[must_use]
	pub fn new() -> Self {
		Self {
			ask: RwSignal::new(None),
			yes: StoredValue::new(None),
			opener: StoredValue::new(None),
		}
	}

	/// The page's sheet, when the page runs as this crate's.
	#[must_use]
	pub fn current() -> Option<Self> {
		use_context::<Self>()
	}

	/// Shows the sheet; `yes` runs if the person confirms, and `from` gets
	/// focus back either way.
	pub fn ask(
		&self,
		ask: Ask,
		from: Option<FocusRequest>,
		yes: impl Fn() + Send + Sync + 'static,
	) {
		self.yes.set_value(Some(Arc::new(yes)));
		self.opener.set_value(from);
		self.ask.set(Some(ask));
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
		let ask = self.ask.get();
		let open = ask.is_some();
		let Ask { title, explanation } = ask.unwrap_or_else(|| {
			Ask {
				title: "Confirm".to_owned(),
				explanation: String::new(),
			}
		});
		let dialog = Dialog::new("confirm", title.clone()).description(explanation.clone());

		let panel = View()
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
				components::prose(&explanation)
					.id("confirm-description")
					.class(style::SECONDARY),
			)
			.child(
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
					.child(
						Button(title)
							.id("confirm-yes")
							.class(style::BUTTON_PRIMARY)
							.test_id("confirm-yes")
							.on_press(move || {
								if let Some(yes) = confirm.close() {
									yes();
								}
							}),
					),
			);

		dialog
			.modal(open)
			.class("items-center justify-center bg-[#16111f]/45 p-4")
			.test_id("confirm-sheet")
			.initial_focus("confirm-yes")
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

/// Runs `then` once the browser has drawn the closed sheet.
fn after_close(then: impl FnOnce() + 'static) {
	#[cfg(feature = "hydrate")]
	{
		// Without a frame to wait for, there's no page to focus either.
		let _ = leptos::prelude::request_animation_frame(then);
	}
	#[cfg(not(feature = "hydrate"))]
	then();
}
