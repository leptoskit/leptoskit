//! The committed widget tree as text and as data.

use std::fmt::Write;

use leptoskit_native::Frame;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::Role;
use leptoskit_yoga::NodeId;

use crate::inspect::Inspector;

/// One committed widget and its subtree.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TreeNode {
	/// The widget kind, such as `button` or `text`.
	pub kind: String,
	/// The ARIA role, explicit or implied by the kind.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub role: Option<String>,
	/// The accessible name: the accessibility label, else the text.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	/// The visible text.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub text: Option<String>,
	/// The accessibility label.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub label: Option<String>,
	/// A text input's value; password values are redacted.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub value: Option<String>,
	/// A text input's placeholder.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub placeholder: Option<String>,
	/// A switch, checkbox, or radio's state.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub checked: Option<bool>,
	/// A slider's value.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub slider: Option<f64>,
	/// The identifier `test_id` set.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub test_id: Option<String>,
	/// The border box in viewport coordinates.
	pub frame: TreeFrame,
	/// Whether queries can see it: inside the viewport, not hidden,
	/// transparent, or clipped away.
	pub visible: bool,
	/// Whether neither it nor an ancestor is disabled.
	pub enabled: bool,
	/// Its children, in order.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub children: Vec<TreeNode>,
}

/// A frame in viewport coordinates, rounded to a tenth of a unit.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct TreeFrame {
	/// The left edge.
	pub x: f32,
	/// The top edge.
	pub y: f32,
	/// The width.
	pub width: f32,
	/// The height.
	pub height: f32,
}

impl From<Frame> for TreeFrame {
	fn from(frame: Frame) -> Self {
		Self {
			x: tenth(frame.x),
			y: tenth(frame.y),
			width: tenth(frame.width),
			height: tenth(frame.height),
		}
	}
}

pub(super) fn tree<B: NativeBackend>(inspect: &Inspector<'_, B>) -> Option<TreeNode> {
	let root = inspect.surface.root()?;
	Some(node(inspect, root))
}

fn node<B: NativeBackend>(inspect: &Inspector<'_, B>, id: NodeId) -> TreeNode {
	let surface = inspect.surface;
	let kind = surface.kind(id);
	let props = surface.props(id);
	let password = kind == NativeKind::PasswordInput;
	let redact = |value: Option<&String>| {
		value.map(|value| {
			if password {
				"<redacted>".to_owned()
			} else {
				value.clone()
			}
		})
	};

	TreeNode {
		kind: kind.wire_name().to_owned(),
		role: props
			.aria()
			.effective_role(kind)
			.map(|role| Role::wire_name(role).to_owned()),
		name: redact(props.accessibility_label.as_ref().or(props.text.as_ref())),
		text: redact(props.text.as_ref()),
		label: props.accessibility_label.clone(),
		value: redact(props.value.as_ref()),
		placeholder: props.placeholder.clone(),
		checked: matches!(
			kind,
			NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio
		)
		.then(|| props.checked.unwrap_or(false)),
		slider: (kind == NativeKind::Slider).then(|| props.slider.unwrap_or_default().value()),
		test_id: props.test_id().map(str::to_owned),
		frame: inspect.absolute_frame(id).into(),
		visible: inspect.visible(id),
		enabled: inspect.enabled(id),
		children: surface
			.children(id)
			.iter()
			.map(|&child| node(inspect, child))
			.collect(),
	}
}

pub(super) fn text<B: NativeBackend>(inspect: &Inspector<'_, B>) -> String {
	let mut output = String::new();

	if let Some(root) = tree(inspect) {
		write_node(&mut output, &root, 0);
	}

	output
}

fn write_node(output: &mut String, node: &TreeNode, depth: usize) {
	// Writing to a String cannot fail.
	write!(output, "{}{}", "  ".repeat(depth), node.kind).expect("writing to a string");

	if let Some(name) = &node.name {
		write!(output, " {name:?}").expect("writing to a string");
	}

	if let Some(role) = node.role.as_ref().filter(|role| **role != node.kind) {
		write!(output, " role={role}").expect("writing to a string");
	}

	if let Some(text) = node.text.as_ref().filter(|_| node.label.is_some()) {
		write!(output, " text={text:?}").expect("writing to a string");
	}

	if let Some(value) = &node.value {
		write!(output, " value={value:?}").expect("writing to a string");
	}

	if let Some(placeholder) = &node.placeholder {
		write!(output, " placeholder={placeholder:?}").expect("writing to a string");
	}

	if let Some(checked) = node.checked {
		output.push_str(if checked { " checked" } else { " unchecked" });
	}

	if let Some(value) = node.slider {
		write!(output, " value={value}").expect("writing to a string");
	}

	if let Some(id) = &node.test_id {
		write!(output, " test_id={id:?}").expect("writing to a string");
	}

	let TreeFrame {
		x,
		y,
		width,
		height,
	} = node.frame;
	write!(output, " frame=({x},{y} {width}x{height})").expect("writing to a string");

	if !node.visible {
		output.push_str(" [hidden]");
	}

	if !node.enabled {
		output.push_str(" [disabled]");
	}

	output.push('\n');

	for child in &node.children {
		write_node(output, child, depth + 1);
	}
}

/// Rounds to a tenth, so float noise never reaches a dump or its diff.
fn tenth(value: f32) -> f32 {
	let rounded = (value * 10.0).round() / 10.0;
	// Fold negative zero, which prints as `-0`.
	if rounded == 0.0 { 0.0 } else { rounded }
}
