//! A radio group whose options are any content: cards, rows, swatches.

use std::fmt;
use std::sync::Arc;

use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::FocusRequest;
use leptoskit_native::Role;
use leptoskit_native::components::Pressable;
use leptoskit_native::components::Text;

use crate::roving;

/// What a group calls when an option is chosen.
type OnChange = Arc<dyn Fn(usize) + Send + Sync>;

/// A choice of one option among several, where each option can show any
/// content.
///
/// The native [`Radio`](leptoskit_native::components::Radio) is a small
/// circle. A plan picker or a persona chooser wants the whole card to be the
/// option. `RadioGroup` wraps each card in a pressable with the `radio` role
/// and its checked state, inside a `radiogroup`, so assistive technology
/// reads "Ada, radio button, 1 of 2, checked".
///
/// The group is one Tab stop, as the WAI-ARIA radio group pattern asks. Tab
/// lands on the checked option, or on the first when none is checked. The
/// arrow keys, Home and End move to another option, choose it, and take
/// focus with them. Enter and Space choose the focused option.
///
/// The web and the terminal deliver key presses. The other backends do not
/// yet, and they also ignore `tabindex`, so every option stays reachable
/// with Tab there.
///
/// Each option is named by its label. Build the group on every render from
/// current state and turn it into an element with `Element::from`:
///
/// ```
/// use leptoskit_aria::RadioGroup;
/// use leptoskit_native::Element;
/// use leptoskit_native::Tristate;
/// use leptoskit_native::components::Text;
/// use leptoskit_native::components::View;
///
/// let chosen = Some(1);
/// let group = Element::from(
/// 	RadioGroup::new("persona", ["Ada", "Grace"], chosen)
/// 		.option(
/// 			0,
/// 			View().child(Text("Ada")).child(Text("Writes the engine")),
/// 		)
/// 		.option(
/// 			1,
/// 			View()
/// 				.child(Text("Grace"))
/// 				.child(Text("Writes the compiler")),
/// 		)
/// 		.on_change(|_index| ()),
/// )
/// .aria_label("Persona");
///
/// let grace = &group.children[1];
/// assert_eq!(grace.props.id.as_deref(), Some("persona-option-1"));
/// assert_eq!(grace.props.aria().checked, Some(Tristate::True));
/// assert_eq!(grace.props.aria().tab_index, Some(0));
/// assert_eq!(group.children[0].props.aria().tab_index, Some(-1));
/// assert!(View().child(group).validate().is_ok());
/// ```
pub struct RadioGroup {
	id: String,
	labels: Vec<String>,
	selected: Option<usize>,
	contents: Vec<Option<Element>>,
	on_change: Option<OnChange>,
	/// Moves focus to the checked option after an arrow key chooses it.
	focus: FocusRequest,
}

impl fmt::Debug for RadioGroup {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("RadioGroup")
			.field("id", &self.id)
			.field("labels", &self.labels)
			.field("selected", &self.selected)
			.finish_non_exhaustive()
	}
}

impl RadioGroup {
	/// A group under the id prefix `id` with one option per label, and
	/// `selected` checked. `None` is a group nobody has answered yet.
	///
	/// # Panics
	///
	/// When `selected` is not the index of a label.
	#[track_caller]
	pub fn new<I>(id: impl Into<String>, labels: I, selected: Option<usize>) -> Self
	where
		I: IntoIterator,
		I::Item: Into<String>,
	{
		let labels: Vec<String> = labels.into_iter().map(Into::into).collect();
		assert!(
			selected.is_none_or(|selected| selected < labels.len()),
			"selected option {selected:?} is outside a group of {}",
			labels.len()
		);

		Self {
			id: id.into(),
			contents: labels.iter().map(|_| None).collect(),
			labels,
			selected,
			on_change: None,
			focus: FocusRequest::new(),
		}
	}

	/// Shows `content` as the option at `index`. An option given no content
	/// shows its label as text.
	///
	/// # Panics
	///
	/// When `index` is not the index of a label.
	#[must_use]
	pub fn option(mut self, index: usize, content: Element) -> Self {
		assert!(
			index < self.labels.len(),
			"option {index} is outside a group of {}",
			self.labels.len()
		);
		self.contents[index] = Some(content);
		self
	}

	/// Calls `on_change` with an option's index when it is pressed, or when
	/// an arrow key, Home or End moves to it. Store the index and build the
	/// next group from it; a group that keeps its old state refuses the
	/// choice.
	#[must_use]
	pub fn on_change(mut self, on_change: impl Fn(usize) + Send + Sync + 'static) -> Self {
		self.on_change = Some(Arc::new(on_change));
		self
	}
}

impl From<RadioGroup> for Element {
	fn from(group: RadioGroup) -> Self {
		let RadioGroup {
			id,
			labels,
			selected,
			contents,
			on_change,
			focus,
		} = group;
		let count = labels.len();
		// With nothing checked, Tab lands on the first option.
		let stop = selected.unwrap_or(0);
		let set_size = position(count.saturating_sub(1));
		let options =
			labels
				.into_iter()
				.zip(contents)
				.enumerate()
				.map(|(index, (label, content))| {
					let content = content.unwrap_or_else(|| Text(label.clone()));
					let option = Pressable(label)
						.id(format!("{id}-option-{index}"))
						.role(Role::Radio)
						.aria_checked(selected == Some(index))
						.aria_pos_in_set(position(index))
						.aria_set_size(set_size)
						.tab_index(roving::tab_index(index == stop))
						.child(content);
					let option = match &on_change {
						Some(on_change) => {
							let on_change = Arc::clone(on_change);
							option.on_press(move || on_change(index))
						}
						None => option,
					};

					if selected == Some(index) {
						option.focus_on(focus)
					} else {
						option
					}
				});
		let list = Self::view().role(Role::RadioGroup).children(options);
		let Some(on_change) = on_change else {
			return list;
		};

		list.on_key(move |key| {
			let next = match selected {
				Some(selected) => roving::target(key, None, selected, count),
				None => roving::entry(key, count),
			};
			let Some(next) = next else {
				return EventOutcome::Unhandled;
			};

			if selected != Some(next) {
				on_change(next);
				focus.request();
			}

			EventOutcome::Handled
		})
	}
}

/// The one-based `aria-posinset` of a zero-based index.
fn position(index: usize) -> u32 {
	u32::try_from(index + 1).expect("a radio group fits in u32 positions")
}
