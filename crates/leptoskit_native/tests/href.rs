//! Navigation targets are validated regardless of how props were supplied.

#[path = "support/href.rs"]
mod targets;

use leptoskit_native::NativeProps;
use leptoskit_native::PropValue;
use leptoskit_native::components::Pressable;

#[test]
fn pressable_href_allows_navigation_and_rejects_executable_targets() {
	for href in targets::VALID_HREFS {
		assert_eq!(Pressable("Open").href(*href).props.href(), Some(*href));
	}

	for href in targets::INVALID_HREFS {
		assert_eq!(Pressable("Open").href(*href).props.href(), None, "{href:?}");
	}
}

#[test]
fn direct_props_cannot_bypass_the_navigation_policy() {
	let mut props = NativeProps::default();
	props.extra.insert(
		NativeProps::HREF.to_owned(),
		PropValue::Str("javascript:alert(1)".to_owned()),
	);
	assert_eq!(props.href(), None);
}

#[cfg(feature = "serde")]
#[test]
fn deserialized_props_cannot_bypass_the_navigation_policy() {
	let props: NativeProps = serde_json::from_value(serde_json::json!({
		"extra": {"href": {"Str": "javascript:alert(1)"}}
	}))
	.expect("props decode");
	assert_eq!(props.href(), None);
}

#[test]
fn a_link_opens_in_place_unless_it_asks_for_a_new_tab() {
	use leptoskit_native::LinkTarget;

	let plain = Pressable("Open").href("/notes");
	assert_eq!(plain.props.link_target(), LinkTarget::SameTab);
	assert!(!plain.props.extra.contains_key(NativeProps::LINK_TARGET));

	let new_tab = Pressable("Open")
		.href("https://example.com/")
		.target(LinkTarget::NewTab);
	assert_eq!(new_tab.props.link_target(), LinkTarget::NewTab);
	// Setting the default again leaves the props as if nothing was set.
	let back = new_tab.target(LinkTarget::SameTab);
	assert_eq!(back.props.link_target(), LinkTarget::SameTab);
	assert!(!back.props.extra.contains_key(NativeProps::LINK_TARGET));

	// A value this version does not know is the default, not an error.
	let mut props = NativeProps::default();
	props.extra.insert(
		NativeProps::LINK_TARGET.to_owned(),
		PropValue::Str("_top".to_owned()),
	);
	assert_eq!(props.link_target(), LinkTarget::SameTab);
}

#[test]
fn a_download_name_is_one_plain_file_name() {
	for name in ["person-7.json", "Résumé (final).pdf", ".env", "a b"] {
		assert_eq!(
			Pressable("Export")
				.href("/export")
				.download(name)
				.props
				.download(),
			Some(name)
		);
	}

	for name in [
		"",
		".",
		"..",
		"../secret",
		"folder/file.txt",
		"C:\\file.txt",
		"line\nbreak",
	] {
		assert_eq!(
			Pressable("Export")
				.href("/export")
				.download(name)
				.props
				.download(),
			None,
			"{name:?}"
		);
	}

	assert_eq!(Pressable("Plain").href("/export").props.download(), None);
}
