//! What a native guest sends its host for each node: the kind codes of
//! `node_create`, and the JSON object of `node_set_props`.
//!
//! The guest writes both and the Wasm host reads both, so each is defined
//! once, here. Two separate tables once disagreed: the host had no code for
//! maps or live photos, and showed both as plain views.

use std::collections::BTreeMap;

use crate::NativeKind;
use crate::NativeProps;

/// Every kind a guest can create, at the index that is its ABI code.
///
/// Codes are append-only: a kind keeps its index for as long as the ABI
/// version stays the same.
pub const KINDS: [NativeKind; 22] = [
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
/// keys the guest sends beside them in the same JSON object.
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

/// The keys of a [`WireNode`] that are not props, which
/// [`WireNode::from_json`] reads in its second pass. A host reads it alone
/// only to name the field a node failed to decode at.
#[derive(serde::Deserialize)]
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

impl WireNode {
	/// Reads a node the guest sent.
	///
	/// The props and the keys beside them are read in two passes over the
	/// same object, never through `#[serde(flatten)]`. Flattening buffers
	/// every value first, and `serde_json`'s `arbitrary_precision` feature,
	/// which an app's other dependencies can turn on for the whole build
	/// (Solana's clients do), buffers a number with a fraction as a map:
	/// such a number in the props failed with `invalid type: map, expected
	/// f32`, and the app drew nothing.
	///
	/// # Errors
	///
	/// When the bytes are not a node's JSON object.
	pub fn from_json(bytes: &[u8]) -> serde_json::Result<Self> {
		let props = serde_json::from_slice::<NativeProps>(bytes)?;
		let Envelope {
			style,
			class,
			rive_inputs,
			source_location,
			component,
		} = serde_json::from_slice(bytes)?;
		Ok(Self {
			props,
			style,
			class,
			rive_inputs,
			source: source_location,
			component,
		})
	}
}
