//! Which `Text` elements are the `<label>` of a field.
//!
//! A field named through `aria_labelled_by` by exactly one element has the
//! relationship a `<label for>` expresses. Rendering that element as a
//! `<label>` gives the page what browsers do for labels: a click on the text
//! focuses a text field, and toggles a checkbox, switch or radio.

use std::collections::BTreeMap;

use super::semantics::Tag;
use crate::Element;
use crate::NativeKind;
use crate::NativeProps;

/// The field each label names, by the label's id.
#[derive(Debug, Clone, Default)]
pub(super) struct Labels {
	/// The id of the field a label names, or `None` once two fields name
	/// themselves after the same label: a `<label>` has one target.
	fields: BTreeMap<String, Option<String>>,
}

impl Labels {
	/// The id of the one field the element with `id` labels.
	fn field(&self, id: &str) -> Option<&str> {
		self.fields.get(id)?.as_deref()
	}

	/// The id of the field a node is the `<label>` of: a plain `Text`, with
	/// no role of its own, that names exactly one field.
	pub(super) fn target(
		&self,
		kind: NativeKind,
		tag: Tag,
		props: &NativeProps,
		id: Option<&str>,
	) -> Option<String> {
		if kind != NativeKind::Text || tag != Tag::Span || props.aria().role.is_some() {
			return None;
		}

		self.field(id?).map(str::to_owned)
	}

	/// Adds the labels declared by the fields in `elements` and below.
	pub(super) fn collect(&mut self, elements: &[Element]) {
		let mut pending = elements.iter().collect::<Vec<_>>();

		while let Some(element) = pending.pop() {
			if let Some((label, field)) = labelled_field(element) {
				self.fields
					.entry(label.to_owned())
					.and_modify(|target| *target = None)
					.or_insert_with(|| Some(field.to_owned()));
			}

			pending.extend(&element.children);
		}
	}

	/// These labels and the ones `elements` declare, when they declare any.
	pub(super) fn with(&self, elements: &[Element]) -> Option<Self> {
		let mut found = Self::default();
		found.collect(elements);

		if found.fields.is_empty() {
			return None;
		}

		let mut all = self.clone();

		for (label, field) in found.fields {
			all.fields
				.entry(label)
				.and_modify(|target| *target = None)
				.or_insert(field);
		}

		Some(all)
	}
}

/// The label and field ids of a field that one element names: a field with
/// an id of its own whose `aria_labelled_by` holds a single id.
fn labelled_field(element: &Element) -> Option<(&str, &str)> {
	if !matches!(
		element.kind,
		NativeKind::TextInput
			| NativeKind::TextArea
			| NativeKind::PasswordInput
			| NativeKind::Switch
			| NativeKind::Checkbox
			| NativeKind::Radio
			| NativeKind::Slider
	) {
		return None;
	}

	let [label] = element.props.aria().labelled_by.as_slice() else {
		return None;
	};

	Some((label, element.props.id.as_deref()?))
}
