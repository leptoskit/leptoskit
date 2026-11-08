//! The `Camera` element: a leaf that names a host-owned session and must say
//! what it shows, or that it is decoration.

use leptoskit_native::ControlTreeError;
use leptoskit_native::Element;
use leptoskit_native::NativeKind;
use leptoskit_native::components::Camera;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;

#[test]
fn a_labelled_or_decorative_camera_is_valid() {
	let camera = Camera(4).aria_label("Rear camera");
	assert_eq!(camera.kind, NativeKind::Camera);
	assert_eq!(camera.props.camera, Some(4));
	camera.validate().unwrap();
	Camera(4).aria_hidden(true).validate().unwrap();
	View()
		.child(Camera(1).aria_label("Preview").class("w-full h-64"))
		.validate()
		.unwrap();
}

#[test]
fn a_camera_needs_a_label_or_to_be_hidden() {
	assert_eq!(Camera(4).validate(), Err(ControlTreeError::InvalidCamera));
	assert_eq!(
		Camera(4).aria_label("  ").validate(),
		Err(ControlTreeError::InvalidCamera)
	);
	assert!(
		ControlTreeError::InvalidCamera
			.to_string()
			.contains("aria_hidden")
	);
}

#[test]
fn only_a_camera_names_a_session_and_it_has_no_children() {
	let mut view = View();
	view.props.camera = Some(1);
	assert_eq!(view.validate(), Err(ControlTreeError::InvalidCamera));
	let tag = Camera::builder().aria_hidden(true);
	assert_eq!(tag.validate(), Err(ControlTreeError::InvalidCamera));
	tag.session(2).validate().unwrap();
	let parent = Camera(1).aria_hidden(true).child(Text("no"));
	assert!(matches!(
		parent.validate(),
		Err(ControlTreeError::Children {
			kind: NativeKind::Camera
		})
	));
}

#[test]
fn the_camera_is_an_image_to_assistive_technology() {
	let element: Element = Camera(1).aria_label("Preview");
	assert_eq!(
		leptoskit_native::Role::implicit(element.kind),
		Some(leptoskit_native::Role::Img)
	);
}

#[cfg(feature = "serde")]
#[test]
fn the_session_travels_in_the_props() {
	let props = Camera(9).aria_hidden(true).props;
	let json = serde_json::to_value(&props).unwrap();
	assert_eq!(json["camera"], 9);
	let back: leptoskit_native::NativeProps = serde_json::from_value(json).unwrap();
	assert_eq!(back.camera, Some(9));
	let empty = serde_json::to_value(leptoskit_native::NativeProps::default()).unwrap();
	assert!(empty.get("camera").is_none());
}
