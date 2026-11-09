//! The guest's node JSON, read back by the host's decoder: every prop and
//! every kind must cross unchanged.
//!
//! These run in the workspace's suite and again under `serde_json`'s
//! `arbitrary_precision` (see `test:wire` in `devenv.nix`), which an app's
//! other dependencies can turn on for the whole build. Under it, a decoder
//! that buffers values (`#[serde(flatten)]`, `#[serde(untagged)]`) reads a
//! number with a fraction as a map; Reapers drew nothing on Android until
//! the host stopped buffering props.

use std::collections::BTreeMap;

use serde::de::Visitor;

use super::Host;
use super::Tree;
use super::kind_code;
use crate::Aria;
use crate::BorderWidths;
use crate::Color;
use crate::Current;
use crate::Element;
use crate::FontWeight;
use crate::HasPopup;
use crate::Invalid;
use crate::Live;
use crate::NativeKind;
use crate::NativeProps;
use crate::Orientation;
use crate::Progress;
use crate::ProgressValue;
use crate::RadioGroup;
use crate::Role;
use crate::SliderState;
use crate::Transform;
use crate::Tristate;
use crate::animation::Animation;
use crate::animation::Easing;
use crate::animation::Keyframe;
use crate::animation::Keyframes;
use crate::animation::Transition;
use crate::props::PropValue;
use crate::props::ScrollOffset;
use crate::props::TextAlign;
use crate::props::TextSpan;
use crate::vector::Drawing;
use crate::vector::Paint;
use crate::vector::PathData;
use crate::vector::Stroke;
use crate::vector::VectorPath;
use crate::vector::ViewBox;
use crate::wire;
use crate::wire::WireNode;

/// A host that keeps the bytes the guest sends, as the Wasm host gets them.
#[derive(Default)]
struct Bytes {
	kinds: BTreeMap<i64, i32>,
	props: BTreeMap<i64, Vec<u8>>,
}

impl Host for Bytes {
	fn create(&mut self, id: i64, kind: i32) {
		self.kinds.insert(id, kind);
	}

	fn props(&mut self, id: i64, bytes: &[u8]) {
		self.props.insert(id, bytes.to_vec());
	}

	fn insert(&mut self, _parent: i64, _child: i64, _index: i32) {}

	fn delete(&mut self, id: i64) {
		self.kinds.remove(&id);
		self.props.remove(&id);
	}

	fn commit(&mut self) {}
}

/// Every prop set, each to something other than its default.
///
/// A struct literal without `..`, so a new prop does not compile until it
/// is set here, and so crosses the wire in these tests.
fn every_prop() -> NativeProps {
	let mut rive = leptoskit_rive::Playback::new(
		leptoskit_rive::Asset::new(b"RIVE\x07\x00\x00\x00scene").expect("a Rive header"),
	);
	rive.set_input(
		leptoskit_rive::Name::new("level").expect("an input name"),
		leptoskit_rive::Input::Number(0.75),
		1,
	)
	.expect("a number input");
	let drawing = Drawing::new(ViewBox::new(0.0, -2.5, 100.0, 50.25).expect("a view box"))
		.path(
			VectorPath::new(PathData::parse("M 108 5 C 55 2 2 20 -6 52").expect("a path"))
				.stroke(Stroke::new(Color(0xE9E4_D82E), 1.4).expect("a stroke")),
		)
		.path(
			VectorPath::new(PathData::parse("M4 13 L9 18 L20 6 Z").expect("a path"))
				.fill(Paint::CurrentColor),
		);
	let transform = Transform::translate(12.5, -3.0).scaled(1.5, 0.75);

	NativeProps {
		rive: Some(Box::new(rive)),
		scene3d: Some(3),
		video: Some(4),
		camera: Some(5),
		map: Some(6),
		live_photo: Some(7),
		text: Some("Take the oath".into()),
		source: Some("nfts/degenApes_1.webp".into()),
		placeholder: Some("Your name".into()),
		autocomplete: Some("username webauthn".into()),
		value: Some("Ifiok".into()),
		checked: Some(true),
		radio_group: Some(RadioGroup::new("class").expect("a radio group")),
		slider: Some(SliderState::new(0.0, 10.0, 2.5, Some(0.5)).expect("a slider")),
		progress: Some(Progress::Determinate(
			ProgressValue::new(0.375).expect("a progress value"),
		)),
		running: Some(true),
		drawing: Some(drawing),
		color: Some(Color(0xE9E4_D8FF)),
		background: Some(Color(0x0708_0BFF)),
		enabled: Some(false),
		accessibility_label: Some("Oath".into()),
		aria: Some(Box::new(Aria {
			labelled_by: vec!["title".into()],
			description: Some("Be bound to a reaper".into()),
			described_by: vec!["subtitle".into()],
			role_description: Some("pact".into()),
			role: Some(Role::Slider),
			level: Some(2),
			pos_in_set: Some(1),
			set_size: Some(3),
			orientation: Some(Orientation::Vertical),
			hidden: Some(true),
			busy: Some(true),
			checked: Some(Tristate::Mixed),
			pressed: Some(Tristate::True),
			selected: Some(true),
			expanded: Some(true),
			current: Some(Current::Page),
			modal: Some(true),
			required: Some(true),
			invalid: Some(Invalid::True),
			read_only: Some(true),
			multiline: Some(true),
			has_popup: Some(HasPopup::Menu),
			value_now: Some(2.5),
			value_min: Some(-1.25),
			value_max: Some(10.0),
			value_text: Some("two and a half".into()),
			live: Some(Live::Polite),
			atomic: Some(true),
			live_text: Some("Bound".into()),
			controls: vec!["panel".into()],
			tab_index: Some(-1),
			extra: BTreeMap::from([("aria-keyshortcuts".into(), "Alt+O".into())]),
		})),
		id: Some("oath".into()),
		corner_radius: Some(8.5),
		border_width: Some(1.25),
		border_widths: Some(BorderWidths {
			top: 1.0,
			right: 2.5,
			bottom: 0.0,
			left: 4.0,
		}),
		border_color: Some(Color(0x3A35_2EFF)),
		opacity: Some(0.6),
		transform: Some(transform),
		animation: Some(Box::new(Animation {
			transition: Some(Transition::timing(150.0, Easing::Linear).delay(20.0)),
			keyframes: Some(
				Keyframes::new(
					1000.0,
					[
						Keyframe::at(0.0).opacity(0.0),
						Keyframe::at(0.5)
							.transform(transform)
							.easing(Easing::Linear),
						Keyframe::at(1.0).opacity(1.0),
					],
				)
				.forever(),
			),
			..Animation::default()
		})),
		hidden: Some(true),
		clip: Some(true),
		font_size: Some(13.5),
		font_weight: Some(FontWeight::Semibold),
		italic: Some(true),
		spans: Some(vec![
			TextSpan::new("The dead market ").color(Color(0xFFFF_FFFF)),
			TextSpan::new("did not stay dead.").background(Color(0x0000_00FF)),
		]),
		text_align: Some(TextAlign::Center),
		direction: Some(leptoskit_yoga::Direction::Rtl),
		focused: Some(true),
		scroll_offset: Some(ScrollOffset::new(0.0, 640.5).animated()),
		// Never sent: the guest runs motion itself.
		motion: None,
		extra: BTreeMap::from([
			("tint".into(), PropValue::Color(Color(0x1122_33FF))),
			("weight".into(), PropValue::Number(0.125)),
			("label".into(), PropValue::Str("x".into())),
			("on".into(), PropValue::Bool(true)),
		]),
	}
}

/// The keys `NativeProps` reads, from its own `Deserialize`.
fn prop_keys() -> &'static [&'static str] {
	struct Keys<'a>(&'a mut &'static [&'static str]);

	impl<'de> serde::Deserializer<'de> for Keys<'_> {
		type Error = serde::de::value::Error;

		serde::forward_to_deserialize_any! {
			bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
			bytes byte_buf option unit unit_struct newtype_struct seq tuple
			tuple_struct map enum identifier ignored_any
		}

		fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
			Err(serde::de::Error::custom("only a struct names its keys"))
		}

		fn deserialize_struct<V: Visitor<'de>>(
			self,
			_: &'static str,
			fields: &'static [&'static str],
			_: V,
		) -> Result<V::Value, Self::Error> {
			*self.0 = fields;
			Err(serde::de::Error::custom("keys read"))
		}
	}

	let mut keys: &'static [&'static str] = &[];
	let _ = <NativeProps as serde::Deserialize>::deserialize(Keys(&mut keys));
	assert!(!keys.is_empty(), "NativeProps deserializes as a struct");
	keys
}

/// The guest's own encoding of a node, as `Tree` sends it.
fn encode(props: &NativeProps, style: &leptoskit_yoga::Style, class: &str) -> Vec<u8> {
	serde_json::to_vec(&super::WireNode {
		props,
		style: super::sent_style(style),
		class,
		rive_inputs: None,
		#[cfg(feature = "devtools")]
		source: Some("src/oath.rs:12:5".into()),
		#[cfg(feature = "devtools")]
		component: Some("Oath"),
	})
	.expect("a node serializes")
}

/// What the guest sends for `element` through its tree, as the host gets it.
fn sent(element: Element) -> (i32, Vec<u8>, NativeProps) {
	let mut tree = Tree::default();
	let mut host = Bytes::default();
	tree.update(element, &mut host);
	let root = tree.root.expect("a root");
	let props = tree.nodes[&root].element.props.clone();
	(host.kinds[&root], host.props[&root].clone(), props)
}

#[test]
fn every_prop_crosses_from_guest_to_host() {
	let props = every_prop();
	let style = leptoskit_yoga::Style {
		flex_grow: 1.5,
		..leptoskit_yoga::Style::default()
	};
	let bytes = encode(&props, &style, "p-2 rounded-lg");
	let json = String::from_utf8_lossy(&bytes);
	let node = WireNode::from_json(&bytes)
		.unwrap_or_else(|error| panic!("the host can't read {json}: {error}"));

	assert_eq!(node.props, props, "the props changed on the way: {json}");
	assert_eq!(node.style, style);
	assert_eq!(node.class, "p-2 rounded-lg");
	#[cfg(feature = "devtools")]
	{
		assert_eq!(node.source.as_deref(), Some("src/oath.rs:12:5"));
		assert_eq!(node.component.as_deref(), Some("Oath"));
	}
	#[cfg(not(feature = "devtools"))]
	assert_eq!(
		node.source, None,
		"an image source is never a DevTools location"
	);
}

#[test]
fn each_prop_crosses_alone() {
	let bytes = encode(&every_prop(), &leptoskit_yoga::Style::default(), "");
	let serde_json::Value::Object(sent) = serde_json::from_slice(&bytes).expect("a JSON object")
	else {
		panic!("the guest sends an object");
	};

	for key in prop_keys() {
		let value = sent
			.get(*key)
			.unwrap_or_else(|| panic!("every_prop() leaves `{key}` unsent: set it"));
		let alone = serde_json::json!({ *key: value });
		let node = WireNode::from_json(alone.to_string().as_bytes())
			.unwrap_or_else(|error| panic!("the host can't read `{key}` alone: {error}"));
		// Through text, as the guest writes it: a `Value` made from an `f32`
		// keeps its widened digits.
		let back: serde_json::Value =
			serde_json::from_slice(&serde_json::to_vec(&node.props).expect("props serialize"))
				.expect("JSON");

		assert_eq!(back, alone, "`{key}` changed on the way");
		assert_ne!(node.props, NativeProps::default(), "`{key}` was dropped");
	}
}

/// Props every kind takes, with numbers in them.
fn common_props(kind: NativeKind) -> Element {
	let transform = Transform::translate(1.5, -2.0).scaled(1.25, 1.25);
	let element = Element::new(kind)
		.class("p-2 rounded-lg")
		.with_props(|props| {
			props.color = Some(Color(0xE9E4_D8FF));
			props.background = Some(Color(0x0708_0BFF));
			props.opacity = Some(0.6);
			props.corner_radius = Some(8.5);
			props.border_width = Some(1.25);
			props.transform = Some(transform);
			props.animation = Some(Box::new(Animation {
				transition: Some(Transition::timing(150.0, Easing::Linear)),
				keyframes: Some(Keyframes::new(
					900.0,
					[
						Keyframe::at(0.0).opacity(0.0),
						Keyframe::at(1.0).opacity(1.0),
					],
				)),
				..Animation::default()
			}));
			props.accessibility_label = Some(format!("{kind}"));
		});

	// What each kind shows, from the same values `every_prop` sends.
	let all = every_prop();
	element.with_props(|props| {
		match kind {
			NativeKind::Rive => props.rive = all.rive,
			NativeKind::Radio => props.radio_group = all.radio_group,
			NativeKind::Scene3d => props.scene3d = all.scene3d,
			NativeKind::Video => props.video = all.video,
			NativeKind::Camera => props.camera = all.camera,
			NativeKind::Map => props.map = all.map,
			NativeKind::LivePhoto => props.live_photo = all.live_photo,
			NativeKind::Image => props.source = all.source,
			NativeKind::Vector => props.drawing = all.drawing,
			NativeKind::Slider => props.slider = all.slider,
			NativeKind::LinearProgressIndicator => props.progress = all.progress,
			NativeKind::Text | NativeKind::TextRun | NativeKind::Button => props.text = all.text,
			_ => {}
		}
	})
}

#[test]
fn every_kind_crosses_from_guest_to_host() {
	for kind in wire::KINDS {
		// The tree refuses Rive content without the feature; the workspace's
		// suite runs with it, and `every_prop_crosses_from_guest_to_host`
		// sends the content either way.
		if kind == NativeKind::Rive && !cfg!(feature = "rive") {
			continue;
		}

		let (code, bytes, props) = sent(common_props(kind));
		let json = String::from_utf8_lossy(&bytes);
		let node = WireNode::from_json(&bytes)
			.unwrap_or_else(|error| panic!("{kind}: the host can't read {json}: {error}"));

		assert_eq!(wire::kind(code), Some(kind), "{kind}'s code reads back");
		assert_eq!(
			node.props, props,
			"{kind}: the props changed on the way: {json}"
		);
		assert_eq!(node.class, "p-2 rounded-lg", "{kind}'s classes");
	}
}

#[test]
fn no_prop_shares_a_key_with_what_is_sent_beside_the_props() {
	// `source` was both the image source and the DevTools location, and the
	// location won: native images never got their source.
	for key in prop_keys() {
		assert!(
			!wire::ENVELOPE_KEYS.contains(key),
			"the prop `{key}` would be read as the node's own `{key}`"
		);
	}

	let node = WireNode::from_json(br#"{"source":"a.webp","source_location":"a.rs:1:2"}"#)
		.expect("a node");
	assert_eq!(node.props.source.as_deref(), Some("a.webp"));
	assert_eq!(node.source.as_deref(), Some("a.rs:1:2"));
}

#[test]
fn every_kind_has_the_code_both_sides_read() {
	for (code, kind) in wire::KINDS.into_iter().enumerate() {
		let code = i32::try_from(code).expect("a small code");
		assert_eq!(kind_code(kind), code, "the guest's code for {kind}");
		assert_eq!(wire::kind(code), Some(kind), "the host's kind for {code}");
	}

	// A match without a wildcard: a new kind does not compile until it has
	// a code.
	for kind in wire::KINDS {
		match kind {
			NativeKind::View
			| NativeKind::Text
			| NativeKind::TextRun
			| NativeKind::Image
			| NativeKind::ScrollView
			| NativeKind::TextInput
			| NativeKind::Button
			| NativeKind::Switch
			| NativeKind::Checkbox
			| NativeKind::Radio
			| NativeKind::Slider
			| NativeKind::LinearProgressIndicator
			| NativeKind::ActivityIndicator
			| NativeKind::PasswordInput
			| NativeKind::Pressable
			| NativeKind::Vector
			| NativeKind::Rive
			| NativeKind::Scene3d
			| NativeKind::Video
			| NativeKind::Camera
			| NativeKind::Map
			| NativeKind::LivePhoto => {}
			NativeKind::Custom(name) => panic!("custom widget `{name}` has a code"),
		}
	}

	assert_eq!(wire::kind(-1), None);
	assert_eq!(wire::kind(wire::KINDS.len() as i32), None);
	assert_eq!(wire::kind_code(NativeKind::Custom("chart")), None);
}

#[test]
fn numbers_in_buffered_positions_read_back() {
	// What failed on the Seeker: the numbers of a vector's view box.
	let json = br#"{"drawing":{"view_box":{"min_x":0.0,"min_y":0.0,"width":100.0,"height":100.0},"scaling":"stretch","paths":[{"data":"M 108 5 C 55 2 2 20 -6 52","stroke":{"color":3924087854,"width":1.0,"cap":"butt","join":"miter"}}]},"aria":{"aria_hidden":true},"class":"absolute inset-0 transition-opacity"}"#;
	let node = WireNode::from_json(json).expect("the Seeker's vector");
	let drawing = node.props.drawing.expect("a drawing");
	assert_eq!(
		drawing.paths()[0].stroke_style().map(Stroke::paint),
		Some(Paint::Color(Color(3_924_087_854)))
	);
	assert_eq!(node.class, "absolute inset-0 transition-opacity");
}
