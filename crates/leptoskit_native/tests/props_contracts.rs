//! Color and wire contracts stay stable while their private layout changes.

use leptoskit_native::Color;

#[test]
fn color_channels_and_hex_forms_preserve_rgba_bytes() {
	let color = Color::rgba(0x12, 0x34, 0x56, 0x78);
	assert_eq!(color, Color(0x1234_5678));
	assert_eq!(
		(color.red(), color.green(), color.blue(), color.alpha()),
		(0x12, 0x34, 0x56, 0x78)
	);
	assert_eq!(Color::hex("#12345678"), Some(color));
	assert_eq!(Color::hex("#123456"), Some(Color(0x1234_56ff)));
	assert_eq!(Color::hex("#ABCDEF"), Some(Color(0xabcd_efff)));

	for invalid in ["123456", "#12345", "#1234567", "#123456789", "#abcdefzz"] {
		assert_eq!(Color::hex(invalid), None, "{invalid}");
	}
}

#[cfg(feature = "serde")]
#[test]
fn every_kind_retains_canonical_and_legacy_wire_names() {
	use leptoskit_native::NativeKind;
	for (kind, legacy) in [
		(NativeKind::View, "View"),
		(NativeKind::Text, "Text"),
		(NativeKind::TextRun, "TextRun"),
		(NativeKind::Image, "Image"),
		(NativeKind::ScrollView, "ScrollView"),
		(NativeKind::TextInput, "TextInput"),
		(NativeKind::Button, "Button"),
		(NativeKind::Switch, "Switch"),
		(NativeKind::Checkbox, "Checkbox"),
		(NativeKind::Radio, "Radio"),
		(NativeKind::Slider, "Slider"),
		(
			NativeKind::LinearProgressIndicator,
			"LinearProgressIndicator",
		),
		(NativeKind::ActivityIndicator, "ActivityIndicator"),
		(NativeKind::PasswordInput, "PasswordInput"),
		(NativeKind::Pressable, "Pressable"),
		(NativeKind::Scene3d, "Scene3d"),
		(NativeKind::Video, "Video"),
		(NativeKind::Camera, "Camera"),
		(NativeKind::Map, "Map"),
	] {
		let canonical = serde_json::json!(kind.wire_name());
		assert_eq!(
			serde_json::to_value(kind).expect("canonical encoding"),
			canonical
		);
		assert_eq!(
			serde_json::from_value::<NativeKind>(canonical).expect("canonical decoding"),
			kind
		);
		assert_eq!(
			serde_json::from_value::<NativeKind>(serde_json::json!(legacy))
				.expect("legacy decoding"),
			kind
		);
	}
}

#[cfg(feature = "serde")]
#[test]
fn custom_kind_strings_and_legacy_map_validation_are_unchanged() {
	use leptoskit_native::NativeKind;

	for value in [
		serde_json::json!("canvas"),
		serde_json::json!({"Custom":"canvas"}),
	] {
		assert_eq!(
			serde_json::from_value::<NativeKind>(value).expect("custom kind"),
			NativeKind::Custom("canvas")
		);
	}

	for (value, message) in [
		(serde_json::json!({}), "empty legacy custom kind"),
		(
			serde_json::json!({"Wrong":"canvas"}),
			"expected a single legacy Custom kind",
		),
		(
			serde_json::json!({"Custom":"canvas", "Other":"extra"}),
			"expected a single legacy Custom kind",
		),
	] {
		let error = serde_json::from_value::<NativeKind>(value).expect_err("invalid custom map");
		assert!(error.to_string().contains(message), "{error}");
	}
}

#[test]
fn test_ids_ride_in_extra_under_the_documented_key() {
	use leptoskit_native::Element;
	use leptoskit_native::NativeProps;
	use leptoskit_native::PropValue;

	let element = Element::button("+").test_id("increment");
	assert_eq!(element.props.test_id(), Some("increment"));
	assert_eq!(
		element.props.extra.get(NativeProps::TEST_ID),
		Some(&PropValue::Str("increment".to_owned()))
	);
	assert_eq!(Element::button("+").props.test_id(), None);
}

#[test]
fn text_content_reads_the_account_and_password_autocomplete_tokens() {
	use leptoskit_native::TextContent;
	use leptoskit_native::components::PasswordInput;
	use leptoskit_native::components::TextInput;

	for (hint, content) in [
		("username", Some(TextContent::Username)),
		("username webauthn", Some(TextContent::Username)),
		("email", Some(TextContent::Email)),
		("section-login Email", Some(TextContent::Email)),
		("current-password", Some(TextContent::CurrentPassword)),
		("new-password", Some(TextContent::NewPassword)),
		("one-time-code", None),
		("off", None),
		("", None),
	] {
		assert_eq!(
			PasswordInput().autocomplete(hint).props.text_content(),
			content,
			"{hint}"
		);
	}
	assert_eq!(TextInput().props.text_content(), None);
}

#[test]
fn property_values_convert_from_their_rust_types() {
	use leptoskit_native::PropValue;

	assert_eq!(PropValue::from("text"), PropValue::Str("text".to_owned()));
	assert_eq!(
		PropValue::from("text".to_owned()),
		PropValue::Str("text".to_owned())
	);
	assert_eq!(PropValue::from(2.5), PropValue::Number(2.5));
	assert_eq!(PropValue::from(true), PropValue::Bool(true));
	assert_eq!(
		PropValue::from(Color::rgba(1, 2, 3, 4)),
		PropValue::Color(Color::rgba(1, 2, 3, 4))
	);
}

#[test]
fn scroll_offsets_know_whether_they_are_finite() {
	use leptoskit_native::ScrollOffset;

	let offset = ScrollOffset::new(1.0, 2.0);
	assert!(!offset.animated);
	assert!(offset.is_finite());
	let animated = offset.animated();
	assert!(animated.animated);
	assert_eq!((animated.x, animated.y), (1.0, 2.0));
	assert!(!ScrollOffset::new(f32::NAN, 0.0).is_finite());
	assert!(!ScrollOffset::new(0.0, f32::INFINITY).is_finite());
}

#[test]
fn key_presses_carry_their_modifiers() {
	use leptoskit_native::KeyPress;

	let plain = KeyPress::new("Enter");
	assert!(plain.is("Enter", false, false, false));
	let shortcut = KeyPress::new("p").ctrl().alt().shift();
	assert_eq!(
		(
			shortcut.key.as_str(),
			shortcut.ctrl,
			shortcut.alt,
			shortcut.shift
		),
		("p", true, true, true)
	);
	assert!(shortcut.is("p", true, true, true));
	assert!(!shortcut.is("p", true, false, true));
}

#[test]
fn text_spans_convert_from_plain_text() {
	use leptoskit_native::TextSpan;

	let span = TextSpan::from("hello");
	assert_eq!(span.text, "hello");
	assert_eq!(span, TextSpan::new("hello"));
}

#[test]
fn the_default_approximate_measurer_matches_its_constructor() {
	use leptoskit_native::ApproxTextMeasurer;
	use leptoskit_native::TextMeasurer;
	use leptoskit_native::TextStyle;
	use leptoskit_yoga::AvailableSpace;
	use leptoskit_yoga::Size;

	let available = Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
	let measured = ApproxTextMeasurer::default().measure("hello", TextStyle::default(), available);
	let built = ApproxTextMeasurer::new().measure("hello", TextStyle::default(), available);
	assert_eq!(measured, built);
}

#[cfg(feature = "serde")]
#[test]
fn vector_kinds_decode_and_invalid_payloads_name_the_expected_shape() {
	use leptoskit_native::NativeKind;

	assert_eq!(
		serde_json::from_value::<NativeKind>(serde_json::json!("vector")).unwrap(),
		NativeKind::Vector
	);
	let error = serde_json::from_value::<NativeKind>(serde_json::json!(1)).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("a wire name, legacy kind name, or custom widget name"),
		"{error}"
	);
}
