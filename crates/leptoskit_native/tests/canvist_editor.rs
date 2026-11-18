//! The `CanvistEditor` element: a leaf that names an editor the app owns and,
//! like any text field, must have an accessible name.

use leptoskit_native::ControlTreeError;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::Role;
use leptoskit_native::components::Button;
use leptoskit_native::components::CanvistEditor;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_native::flatten::is_layout_only;
use leptoskit_native::wire;

#[test]
fn a_named_editor_is_valid() {
	let editor = CanvistEditor(4).accessibility_label("Note");
	assert_eq!(editor.kind, NativeKind::CanvistEditor);
	assert_eq!(editor.props.canvist_editor, Some(4));
	editor.validate().unwrap();
	// A placeholder names a field that has nothing else, as on a text input.
	CanvistEditor(4)
		.placeholder("Write a note…")
		.validate()
		.unwrap();
	View()
		.child(
			CanvistEditor(1)
				.accessibility_label("Note")
				.class("w-full min-h-40"),
		)
		.validate()
		.unwrap();
}

#[test]
fn an_editor_needs_a_name() {
	assert_eq!(
		CanvistEditor(4).validate(),
		Err(ControlTreeError::MissingInputLabel {
			kind: NativeKind::CanvistEditor
		})
	);
}

#[test]
fn only_an_editor_element_names_an_editor() {
	let mut view = View();
	view.props.canvist_editor = Some(1);
	assert_eq!(view.validate(), Err(ControlTreeError::InvalidCanvistEditor));

	let tag = CanvistEditor::builder().accessibility_label("Note");
	assert_eq!(tag.validate(), Err(ControlTreeError::InvalidCanvistEditor));
	tag.editor(2).validate().unwrap();
	assert!(
		ControlTreeError::InvalidCanvistEditor
			.to_string()
			.contains("CanvistEditor")
	);
}

#[test]
fn an_editor_holds_no_children_and_sits_in_no_button() {
	let parent = CanvistEditor(1)
		.accessibility_label("Note")
		.child(Text("no"));
	assert!(matches!(
		parent.validate(),
		Err(ControlTreeError::Children {
			kind: NativeKind::CanvistEditor
		})
	));

	let button = Button("Save").child(CanvistEditor(1).accessibility_label("Note"));
	assert!(matches!(
		button.validate(),
		Err(ControlTreeError::InteractiveDescendant { .. })
	));
}

#[test]
fn the_editor_is_a_text_box_with_its_own_wire_code() {
	assert_eq!(
		Role::implicit(NativeKind::CanvistEditor),
		Some(Role::TextBox)
	);
	assert_eq!(NativeKind::CanvistEditor.wire_name(), "canvist_editor");
	// Codes are append-only, so the newest kind has the last one.
	assert_eq!(wire::KINDS.last(), Some(&NativeKind::CanvistEditor));

	let props = NativeProps {
		canvist_editor: Some(1),
		..NativeProps::default()
	};
	assert!(!is_layout_only(NativeKind::View, &props));
}
