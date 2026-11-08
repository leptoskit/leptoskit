//! The `LivePhoto` element: a leaf that names a host-owned session and must say
//! what it shows, or that it is decoration.

use leptoskit_native::ControlTreeError;
use leptoskit_native::Element;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::components::LivePhoto;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_native::flatten::is_layout_only;

#[test]
fn a_labelled_or_decorative_live_photo_is_valid() {
	let photo = LivePhoto(4).aria_label("Birthday live photo");
	assert_eq!(photo.kind, NativeKind::LivePhoto);
	assert_eq!(photo.props.live_photo, Some(4));
	photo.validate().unwrap();
	LivePhoto(4).aria_hidden(true).validate().unwrap();
	View()
		.child(LivePhoto(1).aria_label("Preview").class("w-full h-64"))
		.validate()
		.unwrap();
}

#[test]
fn a_live_photo_needs_a_label_or_to_be_hidden() {
	assert_eq!(
		LivePhoto(4).validate(),
		Err(ControlTreeError::InvalidLivePhoto)
	);
	assert!(
		ControlTreeError::InvalidLivePhoto
			.to_string()
			.contains("aria_hidden")
	);
}

#[test]
fn only_a_live_photo_names_a_session_and_it_has_no_children() {
	let mut view = View();
	view.props.live_photo = Some(1);
	assert_eq!(view.validate(), Err(ControlTreeError::InvalidLivePhoto));
	let tag = LivePhoto::builder().aria_hidden(true);
	assert_eq!(tag.validate(), Err(ControlTreeError::InvalidLivePhoto));
	tag.live_photo_session(2).validate().unwrap();
	let parent = LivePhoto(1).aria_hidden(true).child(Text("no"));
	assert!(matches!(
		parent.validate(),
		Err(ControlTreeError::Children {
			kind: NativeKind::LivePhoto
		})
	));
}

#[test]
fn the_live_photo_is_an_image_to_assistive_technology() {
	let element: Element = LivePhoto(1).aria_label("Preview");
	assert_eq!(
		leptoskit_native::Role::implicit(element.kind),
		Some(leptoskit_native::Role::Img)
	);
	assert_eq!(NativeKind::LivePhoto.wire_name(), "live_photo");
	assert!(is_layout_only(NativeKind::View, &NativeProps::default()));
	let props = NativeProps {
		live_photo: Some(1),
		..NativeProps::default()
	};
	assert!(!is_layout_only(NativeKind::View, &props));
}
