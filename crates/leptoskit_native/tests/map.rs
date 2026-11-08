//! The `Map` element: a leaf that names a host-owned session and must say what
//! it shows, or that it is decoration.

use leptoskit_native::ControlTreeError;
use leptoskit_native::Element;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::components::Map;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_native::flatten::is_layout_only;

#[test]
fn a_labelled_or_decorative_map_is_valid() {
	let map = Map(4).aria_label("City map");
	assert_eq!(map.kind, NativeKind::Map);
	assert_eq!(map.props.map, Some(4));
	map.validate().unwrap();
	Map(4).aria_hidden(true).validate().unwrap();
	View()
		.child(Map(1).aria_label("Preview").class("w-full h-64"))
		.validate()
		.unwrap();
}

#[test]
fn a_map_needs_a_label_or_to_be_hidden() {
	assert_eq!(Map(4).validate(), Err(ControlTreeError::InvalidMap));
	assert_eq!(
		Map(4).aria_label("  ").validate(),
		Err(ControlTreeError::InvalidMap)
	);
	assert!(ControlTreeError::InvalidMap.to_string().contains("aria_hidden"));
}

#[test]
fn only_a_map_names_a_session_and_it_has_no_children() {
	let mut view = View();
	view.props.map = Some(1);
	assert_eq!(view.validate(), Err(ControlTreeError::InvalidMap));
	let tag = Map::builder().aria_hidden(true);
	assert_eq!(tag.validate(), Err(ControlTreeError::InvalidMap));
	tag.map_session(2).validate().unwrap();
	let parent = Map(1).aria_hidden(true).child(Text("no"));
	assert!(matches!(
		parent.validate(),
		Err(ControlTreeError::Children {
			kind: NativeKind::Map
		})
	));
}

#[test]
fn the_map_is_an_image_to_assistive_technology() {
	let element: Element = Map(1).aria_label("Preview");
	assert_eq!(
		leptoskit_native::Role::implicit(element.kind),
		Some(leptoskit_native::Role::Img)
	);
	assert_eq!(NativeKind::Map.wire_name(), "map");
	assert!(is_layout_only(NativeKind::View, &NativeProps::default()));
	let mut props = NativeProps::default();
	props.map = Some(1);
	assert!(!is_layout_only(NativeKind::View, &props));
}

#[cfg(feature = "serde")]
#[test]
fn the_session_travels_in_the_props() {
	let props = Map(9).aria_hidden(true).props;
	let json = serde_json::to_value(&props).unwrap();
	assert_eq!(json["map"], 9);
	let back: leptoskit_native::NativeProps = serde_json::from_value(json).unwrap();
	assert_eq!(back.map, Some(9));
	let empty = serde_json::to_value(leptoskit_native::NativeProps::default()).unwrap();
	assert!(empty.get("map").is_none());
}
