//! What a native guest sends its host for each node: the kind codes of
//! `node_create`, and the JSON object of `node_set_props`.
//!
//! The guest writes both and the Wasm host reads both, so each is defined
//! once, here. Two separate tables once disagreed: the host had no code for
//! maps or live photos, and showed both as plain views.

use std::collections::BTreeMap;

use leptoskit_yoga::Direction;

use crate::Appearance;
use crate::BorderWidths;
use crate::Capitalize;
use crate::Color;
use crate::FontFamily;
use crate::FontWeight;
use crate::Keyboard;
use crate::LineHeight;
use crate::NativeKind;
use crate::NativeProps;
use crate::Overlay;
use crate::Progress;
use crate::RadioGroup;
use crate::RevealRequest;
use crate::ScrollOffset;
use crate::SliderState;
use crate::SubmitLabel;
use crate::TextAlign;
use crate::TextSpan;
use crate::Transform;
use crate::aria::Aria;
use crate::props::PropValue;
use crate::vector::Drawing;

/// Leading magic byte of a compact binary [`WireNode`] payload (`LKW1`).
///
/// JSON objects begin with `{` (or ASCII whitespace), never `0x01`, so the
/// host distinguishes binary guests from JSON guests (including raw ABI and
/// older bundles) in one byte inspection.
pub const BINARY_MAGIC: u8 = 0x01;

/// Every kind a guest can create, at the index that is its ABI code.
///
/// Codes are append-only: a kind keeps its index for as long as the ABI
/// version stays the same.
pub const KINDS: [NativeKind; 26] = [
	NativeKind::View,
	NativeKind::Text,
	NativeKind::TextRun,
	NativeKind::Image,
	NativeKind::ScrollView,
	NativeKind::TextInput,
	NativeKind::Button,
	NativeKind::Switch,
	NativeKind::Checkbox,
	NativeKind::Radio,
	NativeKind::Slider,
	NativeKind::LinearProgressIndicator,
	NativeKind::ActivityIndicator,
	NativeKind::PasswordInput,
	NativeKind::Pressable,
	NativeKind::Vector,
	NativeKind::Rive,
	NativeKind::Scene3d,
	NativeKind::Video,
	NativeKind::Camera,
	NativeKind::Map,
	NativeKind::LivePhoto,
	NativeKind::TextArea,
	NativeKind::Modal,
	NativeKind::Popover,
	NativeKind::CanvistEditor,
];

/// The ABI code of `kind`, or `None` for a custom widget, which has none.
#[must_use]
pub fn kind_code(kind: NativeKind) -> Option<i32> {
	KINDS
		.iter()
		.position(|known| *known == kind)
		.and_then(|index| i32::try_from(index).ok())
}

/// The kind an ABI code names, or `None` for a code this host does not
/// know, such as one a newer guest sends.
#[must_use]
pub fn kind(code: i32) -> Option<NativeKind> {
	usize::try_from(code)
		.ok()
		.and_then(|index| KINDS.get(index))
		.copied()
}

/// A node's properties as the host reads them: the native props, plus the
/// keys the guest sends beside them in the same wire payload.
#[derive(Debug, Clone, Default)]
pub struct WireNode {
	/// The widget properties.
	pub props: NativeProps,
	/// The layout style.
	pub style: leptoskit_yoga::Style,
	/// Built-in Tailwind utilities, validated before a tree is committed.
	pub class: String,
	/// Replaces inputs while reusing this node's existing Rive asset and scene.
	/// Valid only without `props.rive`, after that same node received full
	/// content.
	pub rive_inputs: Option<BTreeMap<leptoskit_rive::Name, leptoskit_rive::Update>>,
	/// Where the guest created the node, as `file:line:column`. Only a guest
	/// built with `DevTools` sends it, as `source_location`, for the host's
	/// agent to report.
	pub source: Option<String>,
	/// The `#[component]` whose body created the node. Only a guest built
	/// with `DevTools` sends it.
	pub component: Option<String>,
}

/// The keys of a [`WireNode`] that are not props. A host reads it alone only
/// to name the field a JSON node failed to decode at.
#[derive(serde::Deserialize)]
#[allow(
	dead_code,
	reason = "deserialized by leptoskit_wasm to report JSON field paths on error"
)]
pub struct Envelope {
	#[serde(default)]
	style: leptoskit_yoga::Style,
	#[serde(default)]
	class: String,
	#[serde(default)]
	rive_inputs: Option<BTreeMap<leptoskit_rive::Name, leptoskit_rive::Update>>,
	#[serde(default)]
	source_location: Option<String>,
	#[serde(default)]
	component: Option<String>,
}

/// The keys the guest sends beside the props. No prop may use one.
pub const ENVELOPE_KEYS: [&str; 5] = [
	"style",
	"class",
	"rive_inputs",
	"source_location",
	"component",
];

/// Flat single-pass JSON representation of [`WireNode`].
///
/// Listing every [`NativeProps`] field alongside the envelope keys avoids both
/// `#[serde(flatten)]` (which buffers values and breaks under `serde_json`'s
/// `arbitrary_precision`) and parsing the payload twice. Converting to
/// `NativeProps` with an exhaustive struct literal guarantees compile-time
/// parity whenever a prop is added.
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct FlatWireNode {
	rive: Option<Box<leptoskit_rive::Playback>>,
	scene3d: Option<u32>,
	video: Option<u32>,
	camera: Option<u32>,
	map: Option<u32>,
	live_photo: Option<u32>,
	canvist_editor: Option<u32>,
	text: Option<String>,
	source: Option<String>,
	placeholder: Option<String>,
	autocomplete: Option<String>,
	keyboard: Option<Keyboard>,
	read_only: Option<bool>,
	max_length: Option<usize>,
	capitalize: Option<Capitalize>,
	autocorrect: Option<bool>,
	submit_label: Option<SubmitLabel>,
	value: Option<String>,
	checked: Option<bool>,
	radio_group: Option<RadioGroup>,
	slider: Option<SliderState>,
	progress: Option<Progress>,
	running: Option<bool>,
	drawing: Option<Drawing>,
	color: Option<Color>,
	background: Option<Color>,
	enabled: Option<bool>,
	accessibility_label: Option<String>,
	aria: Option<Box<Aria>>,
	id: Option<String>,
	corner_radius: Option<f32>,
	border_width: Option<f32>,
	border_widths: Option<BorderWidths>,
	border_color: Option<Color>,
	opacity: Option<f32>,
	transform: Option<Transform>,
	animation: Option<Box<crate::Animation>>,
	hidden: Option<bool>,
	clip: Option<bool>,
	font_size: Option<f32>,
	font_weight: Option<FontWeight>,
	line_height: Option<LineHeight>,
	italic: Option<bool>,
	font_family: Option<FontFamily>,
	spans: Option<Vec<TextSpan>>,
	text_align: Option<TextAlign>,
	direction: Option<Direction>,
	focused: Option<bool>,
	focus_request: Option<u64>,
	reveal: Option<RevealRequest>,
	scroll_offset: Option<ScrollOffset>,
	overlay: Option<Overlay>,
	appearance: Option<Appearance>,
	extra: BTreeMap<String, PropValue>,
	// Envelope fields
	style: leptoskit_yoga::Style,
	class: String,
	rive_inputs: Option<BTreeMap<leptoskit_rive::Name, leptoskit_rive::Update>>,
	source_location: Option<String>,
	component: Option<String>,
}

impl From<FlatWireNode> for WireNode {
	fn from(flat: FlatWireNode) -> Self {
		Self {
			props: NativeProps {
				rive: flat.rive,
				scene3d: flat.scene3d,
				video: flat.video,
				camera: flat.camera,
				map: flat.map,
				live_photo: flat.live_photo,
				canvist_editor: flat.canvist_editor,
				text: flat.text,
				source: flat.source,
				placeholder: flat.placeholder,
				autocomplete: flat.autocomplete,
				keyboard: flat.keyboard,
				read_only: flat.read_only,
				max_length: flat.max_length,
				capitalize: flat.capitalize,
				autocorrect: flat.autocorrect,
				submit_label: flat.submit_label,
				value: flat.value,
				checked: flat.checked,
				radio_group: flat.radio_group,
				slider: flat.slider,
				progress: flat.progress,
				running: flat.running,
				drawing: flat.drawing,
				color: flat.color,
				background: flat.background,
				enabled: flat.enabled,
				accessibility_label: flat.accessibility_label,
				aria: flat.aria,
				id: flat.id,
				corner_radius: flat.corner_radius,
				border_width: flat.border_width,
				border_widths: flat.border_widths,
				border_color: flat.border_color,
				opacity: flat.opacity,
				transform: flat.transform,
				animation: flat.animation,
				hidden: flat.hidden,
				clip: flat.clip,
				font_size: flat.font_size,
				font_weight: flat.font_weight,
				line_height: flat.line_height,
				italic: flat.italic,
				font_family: flat.font_family,
				spans: flat.spans,
				text_align: flat.text_align,
				direction: flat.direction,
				focused: flat.focused,
				focus_request: flat.focus_request,
				reveal: flat.reveal,
				scroll_offset: flat.scroll_offset,
				overlay: flat.overlay,
				motion: None,
				appearance: flat.appearance,
				extra: flat.extra,
			},
			style: flat.style,
			class: flat.class,
			rive_inputs: flat.rive_inputs,
			source: flat.source_location,
			component: flat.component,
		}
	}
}

/// Rare fields carried as a compact flat JSON tail when set (never using
/// `#[serde(flatten)]` and never cloning hot fields).
#[derive(serde::Serialize)]
struct RestWireNode<'a> {
	#[serde(skip_serializing_if = "Option::is_none")]
	rive: Option<&'a leptoskit_rive::Playback>,
	#[serde(skip_serializing_if = "Option::is_none")]
	scene3d: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	video: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	camera: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	map: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	live_photo: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	canvist_editor: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	autocomplete: Option<&'a str>,
	#[serde(skip_serializing_if = "Option::is_none")]
	keyboard: Option<Keyboard>,
	#[serde(skip_serializing_if = "Option::is_none")]
	read_only: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	max_length: Option<usize>,
	#[serde(skip_serializing_if = "Option::is_none")]
	capitalize: Option<Capitalize>,
	#[serde(skip_serializing_if = "Option::is_none")]
	autocorrect: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	submit_label: Option<SubmitLabel>,
	#[serde(skip_serializing_if = "Option::is_none")]
	radio_group: Option<&'a RadioGroup>,
	#[serde(skip_serializing_if = "Option::is_none")]
	slider: Option<SliderState>,
	#[serde(skip_serializing_if = "Option::is_none")]
	progress: Option<Progress>,
	#[serde(skip_serializing_if = "Option::is_none")]
	drawing: Option<&'a Drawing>,
	#[serde(skip_serializing_if = "Option::is_none")]
	aria: Option<&'a Aria>,
	#[serde(skip_serializing_if = "Option::is_none")]
	border_widths: Option<BorderWidths>,
	#[serde(skip_serializing_if = "Option::is_none")]
	transform: Option<Transform>,
	#[serde(skip_serializing_if = "Option::is_none")]
	animation: Option<&'a crate::Animation>,
	#[serde(skip_serializing_if = "Option::is_none")]
	font_family: Option<FontFamily>,
	#[serde(skip_serializing_if = "Option::is_none")]
	spans: Option<&'a [TextSpan]>,
	#[serde(skip_serializing_if = "Option::is_none")]
	reveal: Option<RevealRequest>,
	#[serde(skip_serializing_if = "Option::is_none")]
	overlay: Option<&'a Overlay>,
	#[serde(skip_serializing_if = "BTreeMap::is_empty")]
	extra: &'a BTreeMap<String, PropValue>,
	#[serde(skip_serializing_if = "Option::is_none")]
	style: Option<&'a leptoskit_yoga::Style>,
	#[serde(skip_serializing_if = "Option::is_none")]
	rive_inputs: Option<&'a BTreeMap<leptoskit_rive::Name, leptoskit_rive::Update>>,
}

#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct RestWireNodeOwned {
	rive: Option<Box<leptoskit_rive::Playback>>,
	scene3d: Option<u32>,
	video: Option<u32>,
	camera: Option<u32>,
	map: Option<u32>,
	live_photo: Option<u32>,
	canvist_editor: Option<u32>,
	autocomplete: Option<String>,
	keyboard: Option<Keyboard>,
	read_only: Option<bool>,
	max_length: Option<usize>,
	capitalize: Option<Capitalize>,
	autocorrect: Option<bool>,
	submit_label: Option<SubmitLabel>,
	radio_group: Option<RadioGroup>,
	slider: Option<SliderState>,
	progress: Option<Progress>,
	drawing: Option<Drawing>,
	aria: Option<Box<Aria>>,
	border_widths: Option<BorderWidths>,
	transform: Option<Transform>,
	animation: Option<Box<crate::Animation>>,
	font_family: Option<FontFamily>,
	spans: Option<Vec<TextSpan>>,
	reveal: Option<RevealRequest>,
	overlay: Option<Overlay>,
	extra: BTreeMap<String, PropValue>,
	style: leptoskit_yoga::Style,
	rive_inputs: Option<BTreeMap<leptoskit_rive::Name, leptoskit_rive::Update>>,
}

const F_CLASS: u32 = 1 << 0;
const F_TEXT: u32 = 1 << 1;
const F_SOURCE: u32 = 1 << 2;
const F_PLACEHOLDER: u32 = 1 << 3;
const F_VALUE: u32 = 1 << 4;
const F_ACCESSIBILITY_LABEL: u32 = 1 << 5;
const F_ID: u32 = 1 << 6;
const F_COLOR: u32 = 1 << 7;
const F_BACKGROUND: u32 = 1 << 8;
const F_BORDER_COLOR: u32 = 1 << 9;
const F_ENABLED: u32 = 1 << 10;
const F_HIDDEN: u32 = 1 << 11;
const F_CLIP: u32 = 1 << 12;
const F_ITALIC: u32 = 1 << 13;
const F_FOCUSED: u32 = 1 << 14;
const F_CHECKED: u32 = 1 << 15;
const F_RUNNING: u32 = 1 << 16;
const F_CORNER_RADIUS: u32 = 1 << 17;
const F_BORDER_WIDTH: u32 = 1 << 18;
const F_OPACITY: u32 = 1 << 19;
const F_FONT_SIZE: u32 = 1 << 20;
const F_FONT_WEIGHT: u32 = 1 << 21;
const F_TEXT_ALIGN: u32 = 1 << 22;
const F_DIRECTION: u32 = 1 << 23;
const F_LINE_HEIGHT: u32 = 1 << 24;
const F_SCROLL_OFFSET: u32 = 1 << 25;
const F_FOCUS_REQUEST: u32 = 1 << 26;
const F_APPEARANCE: u32 = 1 << 27;
const F_DEVTOOLS_SOURCE: u32 = 1 << 28;
const F_DEVTOOLS_COMPONENT: u32 = 1 << 29;
const F_REST_JSON: u32 = 1 << 31;

/// Returns `true` when any field outside the compact binary fast-path is set.
#[inline]
fn has_rest_fields(
	props: &NativeProps,
	style: Option<&leptoskit_yoga::Style>,
	omit_rive: bool,
	rive_inputs: Option<&BTreeMap<leptoskit_rive::Name, leptoskit_rive::Update>>,
) -> bool {
	// Exhaustive destructure ensures any new `NativeProps` field is classified.
	let NativeProps {
		rive,
		scene3d,
		video,
		camera,
		map,
		live_photo,
		canvist_editor,
		text: _,
		source: _,
		placeholder: _,
		autocomplete,
		keyboard,
		read_only,
		max_length,
		capitalize,
		autocorrect,
		submit_label,
		value: _,
		checked: _,
		radio_group,
		slider,
		progress,
		running: _,
		drawing,
		color: _,
		background: _,
		enabled: _,
		accessibility_label: _,
		aria,
		id: _,
		corner_radius: _,
		border_width: _,
		border_widths,
		border_color: _,
		opacity: _,
		transform,
		animation,
		hidden: _,
		clip: _,
		font_size: _,
		font_weight: _,
		line_height: _,
		italic: _,
		font_family,
		spans,
		text_align: _,
		direction: _,
		focused: _,
		focus_request: _,
		reveal,
		scroll_offset: _,
		overlay,
		motion: _,
		appearance: _,
		extra,
	} = props;

	style.is_some()
		|| rive_inputs.is_some()
		|| (!omit_rive && rive.is_some())
		|| scene3d.is_some()
		|| video.is_some()
		|| camera.is_some()
		|| map.is_some()
		|| live_photo.is_some()
		|| canvist_editor.is_some()
		|| autocomplete.is_some()
		|| keyboard.is_some()
		|| read_only.is_some()
		|| max_length.is_some()
		|| capitalize.is_some()
		|| autocorrect.is_some()
		|| submit_label.is_some()
		|| radio_group.is_some()
		|| slider.is_some()
		|| progress.is_some()
		|| drawing.is_some()
		|| aria.is_some()
		|| border_widths.is_some()
		|| transform.is_some()
		|| animation.is_some()
		|| font_family.is_some()
		|| spans.is_some()
		|| reveal.is_some()
		|| overlay.is_some()
		|| !extra.is_empty()
}

#[inline]
fn put_str(out: &mut Vec<u8>, s: &str) {
	out.extend_from_slice(&(s.len() as u32).to_le_bytes());
	out.extend_from_slice(s.as_bytes());
}

/// Encodes a node into `out` using the compact `LKW1` binary wire format.
///
/// Hot widget properties (text, class, colors, fonts, booleans, layout hints)
/// are written directly with zero `serde_json` overhead and zero cloning of
/// `NativeProps`. Rarely-set structured fields are appended as a compact JSON
/// tail only when present.
#[allow(clippy::too_many_arguments)]
#[expect(
	clippy::too_many_lines,
	clippy::cognitive_complexity,
	reason = "flat binary wire encoder writes every hot NativeProps field sequentially without \
	          serde overhead"
)]
pub fn encode_binary_into(
	out: &mut Vec<u8>,
	props: &NativeProps,
	style: Option<&leptoskit_yoga::Style>,
	class: &str,
	omit_rive: bool,
	rive_inputs: Option<&BTreeMap<leptoskit_rive::Name, leptoskit_rive::Update>>,
	devtools_source: Option<&str>,
	devtools_component: Option<&str>,
) {
	let rest = has_rest_fields(props, style, omit_rive, rive_inputs);
	let mut present: u32 = 0;

	if !class.is_empty() {
		present |= F_CLASS;
	}
	if props.text.is_some() {
		present |= F_TEXT;
	}
	if props.source.is_some() {
		present |= F_SOURCE;
	}
	if props.placeholder.is_some() {
		present |= F_PLACEHOLDER;
	}
	if props.value.is_some() {
		present |= F_VALUE;
	}
	if props.accessibility_label.is_some() {
		present |= F_ACCESSIBILITY_LABEL;
	}
	if props.id.is_some() {
		present |= F_ID;
	}
	if props.color.is_some() {
		present |= F_COLOR;
	}
	if props.background.is_some() {
		present |= F_BACKGROUND;
	}
	if props.border_color.is_some() {
		present |= F_BORDER_COLOR;
	}
	if props.enabled.is_some() {
		present |= F_ENABLED;
	}
	if props.hidden.is_some() {
		present |= F_HIDDEN;
	}
	if props.clip.is_some() {
		present |= F_CLIP;
	}
	if props.italic.is_some() {
		present |= F_ITALIC;
	}
	if props.focused.is_some() {
		present |= F_FOCUSED;
	}
	if props.checked.is_some() {
		present |= F_CHECKED;
	}
	if props.running.is_some() {
		present |= F_RUNNING;
	}
	if props.corner_radius.is_some() {
		present |= F_CORNER_RADIUS;
	}
	if props.border_width.is_some() {
		present |= F_BORDER_WIDTH;
	}
	if props.opacity.is_some() {
		present |= F_OPACITY;
	}
	if props.font_size.is_some() {
		present |= F_FONT_SIZE;
	}
	if props.font_weight.is_some() {
		present |= F_FONT_WEIGHT;
	}
	if props.text_align.is_some() {
		present |= F_TEXT_ALIGN;
	}
	if props.direction.is_some() {
		present |= F_DIRECTION;
	}
	if props.line_height.is_some() {
		present |= F_LINE_HEIGHT;
	}
	if props.scroll_offset.is_some() {
		present |= F_SCROLL_OFFSET;
	}
	if props.focus_request.is_some() {
		present |= F_FOCUS_REQUEST;
	}
	if props.appearance.is_some() {
		present |= F_APPEARANCE;
	}
	if devtools_source.is_some() {
		present |= F_DEVTOOLS_SOURCE;
	}
	if devtools_component.is_some() {
		present |= F_DEVTOOLS_COMPONENT;
	}
	if rest {
		present |= F_REST_JSON;
	}

	out.push(BINARY_MAGIC);
	out.extend_from_slice(&present.to_le_bytes());

	if !class.is_empty() {
		put_str(out, class);
	}
	if let Some(s) = props.text.as_deref() {
		put_str(out, s);
	}
	if let Some(s) = props.source.as_deref() {
		put_str(out, s);
	}
	if let Some(s) = props.placeholder.as_deref() {
		put_str(out, s);
	}
	if let Some(s) = props.value.as_deref() {
		put_str(out, s);
	}
	if let Some(s) = props.accessibility_label.as_deref() {
		put_str(out, s);
	}
	if let Some(s) = props.id.as_deref() {
		put_str(out, s);
	}
	if let Some(c) = props.color {
		out.extend_from_slice(&c.0.to_le_bytes());
	}
	if let Some(c) = props.background {
		out.extend_from_slice(&c.0.to_le_bytes());
	}
	if let Some(c) = props.border_color {
		out.extend_from_slice(&c.0.to_le_bytes());
	}
	if let Some(b) = props.enabled {
		out.push(u8::from(b));
	}
	if let Some(b) = props.hidden {
		out.push(u8::from(b));
	}
	if let Some(b) = props.clip {
		out.push(u8::from(b));
	}
	if let Some(b) = props.italic {
		out.push(u8::from(b));
	}
	if let Some(b) = props.focused {
		out.push(u8::from(b));
	}
	if let Some(b) = props.checked {
		out.push(u8::from(b));
	}
	if let Some(b) = props.running {
		out.push(u8::from(b));
	}
	if let Some(v) = props.corner_radius {
		out.extend_from_slice(&v.to_le_bytes());
	}
	if let Some(v) = props.border_width {
		out.extend_from_slice(&v.to_le_bytes());
	}
	if let Some(v) = props.opacity {
		out.extend_from_slice(&v.to_le_bytes());
	}
	if let Some(v) = props.font_size {
		out.extend_from_slice(&v.to_le_bytes());
	}
	if let Some(w) = props.font_weight {
		out.push(match w {
			FontWeight::Thin => 0,
			FontWeight::ExtraLight => 1,
			FontWeight::Light => 2,
			FontWeight::Regular => 3,
			FontWeight::Medium => 4,
			FontWeight::Semibold => 5,
			FontWeight::Bold => 6,
			FontWeight::ExtraBold => 7,
			FontWeight::Black => 8,
		});
	}
	if let Some(a) = props.text_align {
		out.push(match a {
			TextAlign::Start => 0,
			TextAlign::Center => 1,
			TextAlign::End => 2,
		});
	}
	if let Some(d) = props.direction {
		out.push(match d {
			Direction::Ltr => 0,
			Direction::Rtl => 1,
		});
	}
	if let Some(lh) = props.line_height {
		match lh {
			LineHeight::Relative(v) => {
				out.push(0);
				out.extend_from_slice(&v.to_le_bytes());
			}
			LineHeight::Fixed(v) => {
				out.push(1);
				out.extend_from_slice(&v.to_le_bytes());
			}
		}
	}
	if let Some(so) = props.scroll_offset {
		out.extend_from_slice(&so.x.to_le_bytes());
		out.extend_from_slice(&so.y.to_le_bytes());
		out.push(u8::from(so.animated));
	}
	if let Some(fr) = props.focus_request {
		out.extend_from_slice(&fr.to_le_bytes());
	}
	if let Some(ap) = props.appearance {
		out.push(match ap {
			Appearance::System => 0,
			Appearance::Light => 1,
			Appearance::Dark => 2,
		});
	}
	if let Some(s) = devtools_source {
		put_str(out, s);
	}
	if let Some(s) = devtools_component {
		put_str(out, s);
	}
	if rest {
		let tail = RestWireNode {
			rive: (!omit_rive).then_some(props.rive.as_deref()).flatten(),
			scene3d: props.scene3d,
			video: props.video,
			camera: props.camera,
			map: props.map,
			live_photo: props.live_photo,
			canvist_editor: props.canvist_editor,
			autocomplete: props.autocomplete.as_deref(),
			keyboard: props.keyboard,
			read_only: props.read_only,
			max_length: props.max_length,
			capitalize: props.capitalize,
			autocorrect: props.autocorrect,
			submit_label: props.submit_label,
			radio_group: props.radio_group.as_ref(),
			slider: props.slider,
			progress: props.progress,
			drawing: props.drawing.as_ref(),
			aria: props.aria.as_deref(),
			border_widths: props.border_widths,
			transform: props.transform,
			animation: props.animation.as_deref(),
			font_family: props.font_family,
			spans: props.spans.as_deref(),
			reveal: props.reveal,
			overlay: props.overlay.as_ref(),
			extra: &props.extra,
			style,
			rive_inputs,
		};
		serde_json::to_writer(out, &tail).expect("native element properties serialize");
	}
}

struct Cursor<'a> {
	bytes: &'a [u8],
	pos: usize,
}

impl<'a> Cursor<'a> {
	fn new(bytes: &'a [u8]) -> Self {
		Self { bytes, pos: 0 }
	}

	#[inline]
	fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
		let end = self
			.pos
			.checked_add(n)
			.ok_or_else(|| "binary wire payload truncated".to_owned())?;
		let slice = self
			.bytes
			.get(self.pos..end)
			.ok_or_else(|| "binary wire payload truncated".to_owned())?;
		self.pos = end;
		Ok(slice)
	}

	#[inline]
	fn u8(&mut self) -> Result<u8, String> {
		Ok(self.take(1)?[0])
	}

	#[inline]
	fn bool(&mut self) -> Result<bool, String> {
		match self.u8()? {
			0 => Ok(false),
			1 => Ok(true),
			other => Err(format!("invalid binary bool byte {other}")),
		}
	}

	#[inline]
	fn u32(&mut self) -> Result<u32, String> {
		let b = self.take(4)?;
		Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
	}

	#[inline]
	fn u64(&mut self) -> Result<u64, String> {
		let b = self.take(8)?;
		Ok(u64::from_le_bytes([
			b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
		]))
	}

	#[inline]
	fn f32(&mut self) -> Result<f32, String> {
		Ok(f32::from_bits(self.u32()?))
	}

	#[inline]
	fn string(&mut self) -> Result<String, String> {
		let len = self.u32()? as usize;
		let slice = self.take(len)?;
		std::str::from_utf8(slice)
			.map(str::to_owned)
			.map_err(|error| format!("invalid UTF-8 in binary wire string: {error}"))
	}

	#[inline]
	fn rest(&self) -> &'a [u8] {
		&self.bytes[self.pos..]
	}
}

impl WireNode {
	/// Reads a node the guest sent, accepting either the compact `LKW1` binary
	/// format or a JSON object.
	///
	/// # Errors
	///
	/// Returns a JSON error for malformed JSON payloads or a custom error for
	/// malformed binary payloads.
	pub fn from_wire(bytes: &[u8]) -> serde_json::Result<Self> {
		if bytes.first() == Some(&BINARY_MAGIC) {
			Self::from_binary(bytes).map_err(serde::de::Error::custom)
		} else {
			Self::from_json(bytes)
		}
	}

	/// Reads a `LKW1` binary-encoded node.
	#[expect(
		clippy::too_many_lines,
		clippy::cognitive_complexity,
		reason = "flat binary wire decoder reads every hot NativeProps field sequentially without \
		          serde overhead"
	)]
	fn from_binary(bytes: &[u8]) -> Result<Self, String> {
		let mut cursor = Cursor::new(bytes);
		let magic = cursor.u8()?;
		if magic != BINARY_MAGIC {
			return Err(format!("unexpected binary wire magic {magic}"));
		}
		let present = cursor.u32()?;
		let mut node = Self::default();

		if present & F_CLASS != 0 {
			node.class = cursor.string()?;
		}
		if present & F_TEXT != 0 {
			node.props.text = Some(cursor.string()?);
		}
		if present & F_SOURCE != 0 {
			node.props.source = Some(cursor.string()?);
		}
		if present & F_PLACEHOLDER != 0 {
			node.props.placeholder = Some(cursor.string()?);
		}
		if present & F_VALUE != 0 {
			node.props.value = Some(cursor.string()?);
		}
		if present & F_ACCESSIBILITY_LABEL != 0 {
			node.props.accessibility_label = Some(cursor.string()?);
		}
		if present & F_ID != 0 {
			node.props.id = Some(cursor.string()?);
		}
		if present & F_COLOR != 0 {
			node.props.color = Some(Color(cursor.u32()?));
		}
		if present & F_BACKGROUND != 0 {
			node.props.background = Some(Color(cursor.u32()?));
		}
		if present & F_BORDER_COLOR != 0 {
			node.props.border_color = Some(Color(cursor.u32()?));
		}
		if present & F_ENABLED != 0 {
			node.props.enabled = Some(cursor.bool()?);
		}
		if present & F_HIDDEN != 0 {
			node.props.hidden = Some(cursor.bool()?);
		}
		if present & F_CLIP != 0 {
			node.props.clip = Some(cursor.bool()?);
		}
		if present & F_ITALIC != 0 {
			node.props.italic = Some(cursor.bool()?);
		}
		if present & F_FOCUSED != 0 {
			node.props.focused = Some(cursor.bool()?);
		}
		if present & F_CHECKED != 0 {
			node.props.checked = Some(cursor.bool()?);
		}
		if present & F_RUNNING != 0 {
			node.props.running = Some(cursor.bool()?);
		}
		if present & F_CORNER_RADIUS != 0 {
			node.props.corner_radius = Some(cursor.f32()?);
		}
		if present & F_BORDER_WIDTH != 0 {
			node.props.border_width = Some(cursor.f32()?);
		}
		if present & F_OPACITY != 0 {
			node.props.opacity = Some(cursor.f32()?);
		}
		if present & F_FONT_SIZE != 0 {
			node.props.font_size = Some(cursor.f32()?);
		}
		if present & F_FONT_WEIGHT != 0 {
			node.props.font_weight = Some(match cursor.u8()? {
				0 => FontWeight::Thin,
				1 => FontWeight::ExtraLight,
				2 => FontWeight::Light,
				3 => FontWeight::Regular,
				4 => FontWeight::Medium,
				5 => FontWeight::Semibold,
				6 => FontWeight::Bold,
				7 => FontWeight::ExtraBold,
				8 => FontWeight::Black,
				other => return Err(format!("invalid font_weight code {other}")),
			});
		}
		if present & F_TEXT_ALIGN != 0 {
			node.props.text_align = Some(match cursor.u8()? {
				0 => TextAlign::Start,
				1 => TextAlign::Center,
				2 => TextAlign::End,
				other => return Err(format!("invalid text_align code {other}")),
			});
		}
		if present & F_DIRECTION != 0 {
			node.props.direction = Some(match cursor.u8()? {
				0 => Direction::Ltr,
				1 => Direction::Rtl,
				other => return Err(format!("invalid direction code {other}")),
			});
		}
		if present & F_LINE_HEIGHT != 0 {
			let tag = cursor.u8()?;
			let val = cursor.f32()?;
			node.props.line_height = Some(match tag {
				0 => LineHeight::Relative(val),
				1 => LineHeight::Fixed(val),
				other => return Err(format!("invalid line_height tag {other}")),
			});
		}
		if present & F_SCROLL_OFFSET != 0 {
			let x = cursor.f32()?;
			let y = cursor.f32()?;
			let animated = cursor.bool()?;
			node.props.scroll_offset = Some(ScrollOffset { x, y, animated });
		}
		if present & F_FOCUS_REQUEST != 0 {
			node.props.focus_request = Some(cursor.u64()?);
		}
		if present & F_APPEARANCE != 0 {
			node.props.appearance = Some(match cursor.u8()? {
				0 => Appearance::System,
				1 => Appearance::Light,
				2 => Appearance::Dark,
				other => return Err(format!("invalid appearance code {other}")),
			});
		}
		if present & F_DEVTOOLS_SOURCE != 0 {
			node.source = Some(cursor.string()?);
		}
		if present & F_DEVTOOLS_COMPONENT != 0 {
			node.component = Some(cursor.string()?);
		}
		if present & F_REST_JSON != 0 {
			let rest: RestWireNodeOwned = serde_json::from_slice(cursor.rest())
				.map_err(|error| format!("invalid binary wire tail JSON: {error}"))?;
			node.props.rive = rest.rive;
			node.props.scene3d = rest.scene3d;
			node.props.video = rest.video;
			node.props.camera = rest.camera;
			node.props.map = rest.map;
			node.props.live_photo = rest.live_photo;
			node.props.canvist_editor = rest.canvist_editor;
			node.props.autocomplete = rest.autocomplete;
			node.props.keyboard = rest.keyboard;
			node.props.read_only = rest.read_only;
			node.props.max_length = rest.max_length;
			node.props.capitalize = rest.capitalize;
			node.props.autocorrect = rest.autocorrect;
			node.props.submit_label = rest.submit_label;
			node.props.radio_group = rest.radio_group;
			node.props.slider = rest.slider;
			node.props.progress = rest.progress;
			node.props.drawing = rest.drawing;
			node.props.aria = rest.aria;
			node.props.border_widths = rest.border_widths;
			node.props.transform = rest.transform;
			node.props.animation = rest.animation;
			node.props.font_family = rest.font_family;
			node.props.spans = rest.spans;
			node.props.reveal = rest.reveal;
			node.props.overlay = rest.overlay;
			node.props.extra = rest.extra;
			node.style = rest.style;
			node.rive_inputs = rest.rive_inputs;
		} else if !cursor.rest().is_empty() {
			return Err("trailing bytes in binary wire payload".to_owned());
		}

		Ok(node)
	}

	/// Reads a node the guest sent as JSON in a single pass.
	///
	/// Unlike `#[serde(flatten)]`, this never buffers values, so it remains
	/// compatible with `serde_json`'s `arbitrary_precision` feature while
	/// avoiding the cost of deserializing the payload twice.
	///
	/// # Errors
	///
	/// When the bytes are not a node's JSON object.
	pub fn from_json(bytes: &[u8]) -> serde_json::Result<Self> {
		let flat: FlatWireNode = serde_json::from_slice(bytes)?;
		Ok(flat.into())
	}
}
