//! The `Video` element: a leaf that names a host-owned player and must say
//! what it shows, or that it is decoration.

use leptoskit_native::ControlTreeError;
use leptoskit_native::Element;
use leptoskit_native::NativeKind;
use leptoskit_native::components::Text;
use leptoskit_native::components::Video;
use leptoskit_native::components::View;

#[test]
fn a_labelled_or_decorative_video_is_valid() {
	let video = Video(4).aria_label("Product demo");
	assert_eq!(video.kind, NativeKind::Video);
	assert_eq!(video.props.video, Some(4));
	video.validate().unwrap();
	Video(4).aria_hidden(true).validate().unwrap();
	View()
		.child(Video(1).aria_label("Clip").class("w-full h-64"))
		.validate()
		.unwrap();
}

#[test]
fn a_video_needs_a_label_or_to_be_hidden() {
	assert_eq!(Video(4).validate(), Err(ControlTreeError::InvalidVideo));
	assert_eq!(
		Video(4).aria_label("  ").validate(),
		Err(ControlTreeError::InvalidVideo)
	);
	assert!(
		ControlTreeError::InvalidVideo
			.to_string()
			.contains("aria_hidden")
	);
}

#[test]
fn only_a_video_names_a_player_and_it_has_no_children() {
	let mut view = View();
	view.props.video = Some(1);
	assert_eq!(view.validate(), Err(ControlTreeError::InvalidVideo));
	let tag = Video::builder().aria_hidden(true);
	assert_eq!(tag.validate(), Err(ControlTreeError::InvalidVideo));
	tag.player(2).validate().unwrap();
	let parent = Video(1).aria_hidden(true).child(Text("no"));
	assert!(matches!(
		parent.validate(),
		Err(ControlTreeError::Children {
			kind: NativeKind::Video
		})
	));
}

#[test]
fn the_video_is_an_image_to_assistive_technology() {
	let element: Element = Video(1).aria_label("Clip");
	assert_eq!(
		leptoskit_native::Role::implicit(element.kind),
		Some(leptoskit_native::Role::Img)
	);
}

#[cfg(feature = "serde")]
#[test]
fn the_player_travels_in_the_props() {
	let props = Video(9).aria_hidden(true).props;
	let json = serde_json::to_value(&props).unwrap();
	assert_eq!(json["video"], 9);
	let back: leptoskit_native::NativeProps = serde_json::from_value(json).unwrap();
	assert_eq!(back.video, Some(9));
	let empty = serde_json::to_value(leptoskit_native::NativeProps::default()).unwrap();
	assert!(empty.get("video").is_none());
}
