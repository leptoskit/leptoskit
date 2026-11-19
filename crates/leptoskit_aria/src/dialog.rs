//! Dialog: a window separated from the page content.

use leptoskit_native::Element;
use leptoskit_native::Role;
use leptoskit_native::components::Heading;
use leptoskit_yoga::Display;

/// The naming contract of one dialog: its title and optional description.
///
/// One `Dialog` builds the container and the elements that name and
/// describe it, so the references always agree: the container is labelled by
/// the title element and, when a description is set, described by the
/// description element. Place the title and description inside the
/// container; the renderer's validation rejects a container whose title
/// element is missing.
///
/// [`Dialog::modal`] is the dialog most apps want: it sits above the screen,
/// blocks what is behind it, keeps focus inside, and closes on Escape.
/// [`Dialog::container`] is a dialog in the flow of the page, which does
/// none of that and so is not marked `aria-modal`: that mark promises focus
/// stays inside.
///
/// ```
/// use leptoskit_aria::Dialog;
/// use leptoskit_native::NativeKind;
/// use leptoskit_native::components::Button;
/// use leptoskit_native::components::View;
///
/// let dialog = Dialog::new("discard", "Discard draft?").description("This cannot be undone.");
/// let screen = dialog
/// 	.modal(true)
/// 	.initial_focus("keep")
/// 	.on_dismiss(|| ())
/// 	.child(
/// 		View()
/// 			.child(dialog.title_element(2))
/// 			.children(dialog.description_element())
/// 			.child(Button("Keep editing").id("keep")),
/// 	);
///
/// assert_eq!(screen.kind, NativeKind::Modal);
/// assert_eq!(screen.props.aria().labelled_by, ["discard-title"]);
/// assert!(screen.validate().is_ok());
/// ```
///
/// ```
/// use leptoskit_aria::Dialog;
///
/// let dialog = Dialog::new("discard", "Discard draft?").description("This cannot be undone.");
/// let screen = dialog
/// 	.container(true)
/// 	.child(dialog.title_element(2))
/// 	.children(dialog.description_element());
///
/// assert_eq!(screen.props.id.as_deref(), Some("discard"));
/// assert_eq!(
/// 	screen.props.aria().role,
/// 	Some(leptoskit_native::Role::Dialog)
/// );
/// assert_eq!(screen.props.aria().labelled_by, ["discard-title"]);
/// assert_eq!(screen.props.aria().described_by, ["discard-description"]);
/// assert!(screen.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
	id: String,
	title: String,
	description: Option<String>,
}

impl Dialog {
	/// A dialog with the container id `id`, named `title`.
	pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
		Self {
			id: id.into(),
			title: title.into(),
			description: None,
		}
	}

	/// Attaches a description, read after the title.
	#[must_use]
	pub fn description(mut self, text: impl Into<String>) -> Self {
		self.description = Some(text.into());
		self
	}

	/// The dialog container, collapsed out of layout while closed.
	///
	/// The container stays mounted while closed so its references always
	/// resolve; build `open` from state and the same tree opens and closes.
	#[must_use]
	pub fn container(&self, open: bool) -> Element {
		let dialog = Element::view()
			.id(self.id.clone())
			.role(Role::Dialog)
			.aria_labelled_by([self.title_id()])
			.aria_described_by(self.description.as_ref().map(|_| self.description_id()));

		if open {
			dialog
		} else {
			dialog.with_style(|style| style.display = Display::None)
		}
	}

	/// The dialog as a [`Modal`](leptoskit_native::components::Modal):
	/// above the screen while `open`, named by the title and described by
	/// the description. Chain `.on_dismiss(...)` to hear Escape and a press
	/// on the backdrop, and `.initial_focus(id)` to pick what takes focus.
	///
	/// The modal covers the screen and is the backdrop, so style it to dim
	/// what is behind and put the panel inside it, with the title and
	/// description in the panel. It stays mounted while closed.
	#[must_use]
	#[track_caller]
	pub fn modal(&self, open: bool) -> Element {
		Element::modal(open)
			.id(self.id.clone())
			.aria_labelled_by([self.title_id()])
			.aria_described_by(self.description.as_ref().map(|_| self.description_id()))
	}

	/// The title element: a heading at `level`, starting at one.
	#[must_use]
	pub fn title_element(&self, level: u8) -> Element {
		Heading(level, self.title.clone()).id(self.title_id())
	}

	/// The description element, when a description is set.
	#[must_use]
	pub fn description_element(&self) -> Option<Element> {
		self.description
			.as_ref()
			.map(|text| Element::text(text.clone()).id(self.description_id()))
	}

	fn title_id(&self) -> String {
		format!("{}-title", self.id)
	}

	fn description_id(&self) -> String {
		format!("{}-description", self.id)
	}
}
