//! The `Scene3d` element: a leaf that names a host-owned scene and must say
//! what it shows, or that it is decoration.

use leptoskit_3d::SceneId;
use leptoskit_native::ControlTreeError;
use leptoskit_native::Element;
use leptoskit_native::NativeKind;
use leptoskit_native::components::Scene3d;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;

#[test]
fn a_labelled_or_decorative_scene_is_valid() {
	let scene = Scene3d(SceneId(4)).aria_label("A reaper turning slowly");
	assert_eq!(scene.kind, NativeKind::Scene3d);
	assert_eq!(scene.props.scene3d, Some(4));
	scene.validate().unwrap();
	Scene3d(SceneId(4)).aria_hidden(true).validate().unwrap();
	View()
		.child(Scene3d(SceneId(1)).aria_label("Arena").class("w-full h-96"))
		.validate()
		.unwrap();
}

#[test]
fn a_scene_needs_a_label_or_to_be_hidden() {
	assert_eq!(
		Scene3d(SceneId(4)).validate(),
		Err(ControlTreeError::InvalidScene3d)
	);
	assert_eq!(
		Scene3d(SceneId(4)).aria_label("  ").validate(),
		Err(ControlTreeError::InvalidScene3d)
	);
	assert!(
		ControlTreeError::InvalidScene3d
			.to_string()
			.contains("aria_hidden")
	);
}

#[test]
fn only_a_scene_names_a_scene_and_it_has_no_children() {
	let mut view = View();
	view.props.scene3d = Some(1);
	assert_eq!(view.validate(), Err(ControlTreeError::InvalidScene3d));
	// The `view!` tag starts without a scene until `scene` sets one.
	let tag = Scene3d::builder().aria_hidden(true);
	assert_eq!(tag.validate(), Err(ControlTreeError::InvalidScene3d));
	tag.scene(SceneId(2)).validate().unwrap();
	let parent = Scene3d(SceneId(1)).aria_hidden(true).child(Text("no"));
	assert!(matches!(
		parent.validate(),
		Err(ControlTreeError::Children {
			kind: NativeKind::Scene3d
		})
	));
}

#[test]
fn the_scene_is_an_image_to_assistive_technology() {
	let element: Element = Scene3d(SceneId(1)).aria_label("Arena");
	assert_eq!(
		leptoskit_native::Role::implicit(element.kind),
		Some(leptoskit_native::Role::Img)
	);
}

#[cfg(feature = "serde")]
#[test]
fn the_scene_travels_in_the_props() {
	let props = Scene3d(SceneId(9)).aria_hidden(true).props;
	let json = serde_json::to_value(&props).unwrap();
	assert_eq!(json["scene3d"], 9);
	let back: leptoskit_native::NativeProps = serde_json::from_value(json).unwrap();
	assert_eq!(back.scene3d, Some(9));
	let empty = serde_json::to_value(leptoskit_native::NativeProps::default()).unwrap();
	assert!(empty.get("scene3d").is_none());
}
