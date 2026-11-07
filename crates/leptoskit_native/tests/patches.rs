//! Incremental commits: patching parts of a committed tree in place.
#![allow(clippy::float_cmp)]

use leptoskit_native::Color;
use leptoskit_native::ControlTreeError;
use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::Role;
use leptoskit_native::SourcePatch;
use leptoskit_yoga::NodeId;

fn mounted(tree: &Element) -> NativeSurface<HeadlessBackend> {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(tree);
	surface.backend_mut().clear();
	surface
}

fn node(surface: &NativeSurface<HeadlessBackend>, path: &[usize]) -> NodeId {
	path.iter()
		.fold(surface.root().expect("a committed root"), |id, &index| {
			surface.children(id)[index]
		})
}

fn node_patch(node: NodeId, element: Element) -> SourcePatch {
	SourcePatch::Node { node, element }
}

fn subtree_patch(node: NodeId, element: Element) -> SourcePatch {
	SourcePatch::Subtree { node, element }
}

#[test]
fn patches_decline_without_a_tree_and_where_relationships_resolve() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	assert_eq!(surface.try_patch(Vec::new()), Ok(None));

	surface.set_root(&Element::view().id("screen").child(Element::text("Label")));
	assert!(surface.resolves_relationships());

	assert_eq!(
		surface.try_patch(vec![subtree_patch(
			surface.root().expect("root"),
			Element::view()
		)]),
		Ok(None)
	);
}

#[test]
fn patches_name_only_committed_nodes() {
	let mut surface =
		mounted(&Element::view().children([Element::text("one"), Element::text("two")]));
	let removed = node(&surface, &[1]);
	surface.set_root(&Element::view().child(Element::text("one")));

	assert!(!surface.children(surface.root().unwrap()).contains(&removed));
	assert_eq!(
		surface.try_patch(vec![node_patch(removed, Element::text("gone"))]),
		Ok(None)
	);
}

#[test]
fn a_node_patch_updates_one_node_and_ignores_its_children() {
	let mut surface =
		mounted(&Element::view().children([Element::text("Count: 0"), Element::text("Footer")]));
	let counter = node(&surface, &[0]);
	let footer = node(&surface, &[1]);

	assert_eq!(
		surface.try_patch(vec![node_patch(
			counter,
			Element::text("Count: 1").child(Element::text("ignored").class("font-bold")),
		)]),
		Ok(Some(vec![counter]))
	);
	assert_eq!(surface.props(counter).text.as_deref(), Some("Count: 1"));
	assert_eq!(surface.children(counter), [] as [NodeId; 0]);
	assert_eq!(surface.children(surface.root().unwrap()), [counter, footer]);
	assert_eq!(
		surface.backend().log(),
		[Mutation::UpdateProps {
			id: counter,
			props: surface.props(counter).clone(),
		}]
	);
}

#[test]
fn a_kind_change_rebuilds_the_node_in_its_slot_and_keeps_its_children() {
	let mut surface = mounted(
		&Element::view().child(
			Element::view()
				.child(Element::text("Kept"))
				.child(Element::view()),
		),
	);
	let container = node(&surface, &[0]);
	let kept = node(&surface, &[0, 0]);
	let dropped = node(&surface, &[0, 1]);

	let patched = surface
		.try_patch(vec![node_patch(container, Element::button("Run"))])
		.expect("a valid patch")
		.expect("applied in place")
		.remove(0);
	assert_ne!(patched, container);
	assert_eq!(surface.kind(patched), NativeKind::Button);
	assert_eq!(surface.children(patched).len(), 2);
	assert_eq!(surface.props(patched).text.as_deref(), Some("Run"));
	assert_ne!(
		surface.children(patched),
		[kept, dropped],
		"a different kind builds fresh child widgets"
	);
	assert_eq!(
		surface.props(surface.children(patched)[0]).text.as_deref(),
		Some("Kept")
	);
	assert!(
		surface
			.backend()
			.log()
			.iter()
			.any(|mutation| matches!(mutation, Mutation::Delete { id } if *id == container))
	);
}

#[test]
fn a_subtree_patch_replaces_every_descendant() {
	let mut surface = mounted(&Element::view().child(Element::view().child(Element::text("Old"))));
	let container = node(&surface, &[0]);
	let old = node(&surface, &[0, 0]);

	let patched = surface
		.try_patch(vec![subtree_patch(
			container,
			Element::view().children([Element::text("New"), Element::text("Second")]),
		)])
		.expect("a valid patch")
		.expect("applied in place");
	assert_eq!(patched, [container]);
	assert_eq!(surface.children(container).len(), 2);
	assert_eq!(
		surface.children(container)[0],
		old,
		"a positional child keeps its slot and is updated in place"
	);
	assert_eq!(surface.props(old).text.as_deref(), Some("New"));
	assert_eq!(
		surface
			.props(surface.children(container)[1])
			.text
			.as_deref(),
		Some("Second")
	);
}

#[test]
fn nested_node_patches_apply_together_while_subtree_overlaps_decline() {
	let mut surface = mounted(&Element::view().child(Element::view().child(Element::text("Old"))));
	let container = node(&surface, &[0]);
	let text = node(&surface, &[0, 0]);

	let applied = surface
		.try_patch(vec![
			node_patch(text, Element::text("New")),
			node_patch(container, Element::view().class("p-4")),
		])
		.expect("valid patches")
		.expect("applied in place");
	assert_eq!(applied, [container, text]);
	assert_eq!(surface.props(text).text.as_deref(), Some("New"));
	assert_eq!(surface.children(container), [text]);

	let before = surface.props(text).clone();
	assert_eq!(
		surface.try_patch(vec![
			subtree_patch(container, Element::view()),
			node_patch(text, Element::text("Ignored")),
		]),
		Ok(None)
	);
	assert_eq!(surface.props(text), &before);
}

#[test]
fn self_contained_patches_decline_when_they_declare_relationships() {
	let mut surface = mounted(&Element::view().child(Element::text("one")));
	let text = node(&surface, &[0]);

	assert_eq!(
		surface.try_patch(vec![node_patch(text, Element::text("one").id("label"))]),
		Ok(None)
	);
	assert_eq!(
		surface.try_patch(vec![node_patch(
			text,
			Element::text("one").aria_controls(["panel"])
		)]),
		Ok(None)
	);
	assert_eq!(surface.props(text).text.as_deref(), Some("one"));
	assert_eq!(surface.backend().log(), []);
}

#[test]
fn invalid_patches_are_rejected_before_anything_changes() {
	let mut surface = mounted(&Element::view().child(Element::text("one")));
	let text = node(&surface, &[0]);

	assert_eq!(
		surface.try_patch(vec![node_patch(text, Element::text_input())]),
		Err(ControlTreeError::MissingInputLabel {
			kind: NativeKind::TextInput
		})
	);
	assert_eq!(surface.kind(text), NativeKind::Text);
	assert_eq!(surface.backend().log(), []);

	let mut surface = mounted(&Element::button("Outer").child(Element::view()));
	let inner = node(&surface, &[0]);
	assert_eq!(
		surface.try_patch(vec![node_patch(
			inner,
			Element::text_input().aria_label("Name")
		)]),
		Err(ControlTreeError::InteractiveDescendant {
			container: NativeKind::Button,
			descendant: NativeKind::TextInput,
		})
	);
	assert_eq!(surface.kind(inner), NativeKind::View);
}

#[test]
fn a_role_change_rebuilds_the_node_in_its_slot() {
	let mut surface = mounted(&Element::view().child(Element::view().child(Element::text("Body"))));
	let content = node(&surface, &[0]);

	let patched = surface
		.try_patch(vec![node_patch(content, Element::view().role(Role::Main))])
		.expect("a valid patch")
		.expect("applied in place")
		.remove(0);
	assert_ne!(patched, content);
	assert_eq!(surface.kind(patched), NativeKind::View);
	assert_eq!(
		surface.props(patched).aria().role,
		Some(Role::Main),
		"the rebuilt node carries its new role"
	);
	assert_eq!(surface.children(patched).len(), 1);
	assert_eq!(
		surface.props(surface.children(patched)[0]).text.as_deref(),
		Some("Body")
	);
}

#[test]
fn a_parent_patch_resolves_inherited_props_for_its_descendants() {
	let red = Color::rgba(255, 0, 0, 255);
	let mut surface = mounted(&Element::view().child(Element::text("Body")));
	let root = surface.root().expect("root");
	let text = node(&surface, &[0]);
	assert_eq!(surface.props(text).color, None);

	assert_eq!(
		surface.try_patch(vec![node_patch(
			root,
			Element::view().color(red).child(Element::text("Body")),
		)]),
		Ok(Some(vec![root]))
	);
	assert_eq!(surface.props(text).color, Some(red));
	assert_eq!(surface.props(text).text.as_deref(), Some("Body"));
}

#[test]
fn patching_under_a_held_control_recommits_the_whole_source() {
	let mut surface = mounted(&Element::view().children([
		Element::button("Hold").class("bg-black active:bg-white"),
		Element::text("Label"),
	]));
	let button = node(&surface, &[0]);
	let label = node(&surface, &[1]);
	surface.press_in(button);
	assert_eq!(surface.pressed(), Some(button));

	let active = surface.props(button).background;
	assert_eq!(
		surface.try_patch(vec![node_patch(label, Element::text("Changed"))]),
		Ok(Some(vec![label]))
	);
	assert_eq!(surface.props(label).text.as_deref(), Some("Changed"));
	assert_eq!(surface.pressed(), Some(button));
	assert_eq!(surface.props(button).background, active);
}

#[test]
fn patching_the_root_replaces_it() {
	let mut surface = mounted(&Element::view().child(Element::text("Old")));
	let root = surface.root().expect("root");

	let patched = surface
		.try_patch(vec![subtree_patch(
			root,
			Element::scroll_view().child(Element::text("New")),
		)])
		.expect("a valid patch")
		.expect("applied in place")
		.remove(0);
	assert_ne!(patched, root);
	assert_eq!(surface.root(), Some(patched));
	assert_eq!(surface.kind(patched), NativeKind::ScrollView);
	assert_eq!(surface.children(patched).len(), 1);
	assert_eq!(
		surface.props(surface.children(patched)[0]).text.as_deref(),
		Some("New")
	);
}

#[test]
fn an_empty_patch_list_is_a_no_op() {
	let mut surface = mounted(&Element::view().child(Element::text("Unchanged")));
	assert_eq!(surface.try_patch(Vec::new()), Ok(Some(Vec::new())));
	assert_eq!(surface.backend().log(), []);
}
