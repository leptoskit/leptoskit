//! The binary render transaction the generated iOS and Android apps read.
//!
//! Decoding the JSON transaction cost the platforms more than some of the
//! work it describes: Swift's `JSONDecoder` took 30 ms for the 6,000
//! mutations that create 1,000 cards, and Android parsed every batch into a
//! tree of `JSONObject`s, then looked each property up by name. This form is
//! read with a cursor and fixed-width loads instead. It carries exactly what
//! the JSON transaction does: properties the renderers apply on every view
//! are typed fields, and any other property that is set travels in the
//! JSON form, inside the record.
//!
//! Little endian throughout:
//!
//! ```text
//! transaction = "LKB1" root:i64 (-1 for none) count:u32 mutation{count}
//! mutation    = 1 id:u32 kind:str8 props          create
//!             | 2 id:u32 props                    update props
//!             | 3 id:u32 x:f32 y:f32 w:f32 h:f32  update layout
//!             | 4 parent:u32 child:u32 index:u32  insert child
//!             | 5 parent:u32 child:u32            remove child
//!             | 6 id:u32                          delete
//! props       = present:u32, then each present field in bit order:
//!                0 text str             1 source str
//!                2 placeholder str      3 value str
//!                4 accessibility_label str
//!                5 color u32            6 background u32
//!                7 border_color u32     8 enabled u8
//!                9 hidden u8           10 clip u8
//!               11 corner_radius f32   12 border_width f32
//!               13 opacity f32         14 font_size f32
//!               15 font_weight u8 (0 regular, 1 medium, 2 semibold, 3 bold,
//!                  4 thin, 5 extra_light, 6 light, 7 extra_bold, 8 black)
//!               16 text_align u8 (0 start, 1 center, 2 end)
//!               17 transform
//!               18 animation
//!               31 rest str: every other set property, as the JSON object
//!                  the JSON transaction sends for props
//! str8        = length:u8 utf8
//! str         = length:u32 utf8
//! transform   = translate_x:offset translate_y:offset scale_x:f32 scale_y:f32
//!               rotate:f32 (clockwise degrees)
//! offset      = points:f32 percent:f32 (of the widget's own size)
//! animation   = parts:u8, then each present part in bit order:
//!               0 transition = properties:u8 (1 opacity, 2 transform,
//!                 4 colors) delay:f32 curve
//!               1 keyframes  = duration:f32 delay:f32 iterations:f32
//!                 (+inf plays forever) alternate:u8 easing count:u8
//!                 keyframe{count}
//!               2 entering   = presence
//!               3 exiting    = presence
//!               4 layout     = curve
//! keyframe    = offset:f32 visual easing_set:u8 [easing]
//! presence    = visual delay:f32 curve
//! visual      = present:u8 (1 opacity, 2 transform) [opacity:f32] [transform]
//! curve       = 0 duration:f32 easing | 1 mass:f32 stiffness:f32 damping:f32
//! easing      = 0 (linear) | 1 x1:f32 y1:f32 x2:f32 y2:f32 (cubic Bézier)
//! ```
//!
//! Durations and delays are milliseconds. Every animation value arrives
//! already sanitized: finite, bounded, and at most sixteen keyframes.

use std::io;

use leptoskit_native::FontWeight;
use leptoskit_native::Mutation;
use leptoskit_native::NativeProps;
use leptoskit_native::TextAlign;
use leptoskit_native::animation::Animation;
use leptoskit_native::animation::Curve;
use leptoskit_native::animation::Easing;
use leptoskit_native::animation::Iterations;
use leptoskit_native::animation::Presence;
use leptoskit_native::animation::Transform;
use leptoskit_native::animation::Visual;

/// The first four bytes of every binary transaction.
pub(super) const MAGIC: &[u8; 4] = b"LKB1";

const CREATE: u8 = 1;
const UPDATE_PROPS: u8 = 2;
const UPDATE_LAYOUT: u8 = 3;
const INSERT_CHILD: u8 = 4;
const REMOVE_CHILD: u8 = 5;
const DELETE: u8 = 6;

const TEXT: u32 = 1 << 0;
const SOURCE: u32 = 1 << 1;
const PLACEHOLDER: u32 = 1 << 2;
const VALUE: u32 = 1 << 3;
const ACCESSIBILITY_LABEL: u32 = 1 << 4;
const COLOR: u32 = 1 << 5;
const BACKGROUND: u32 = 1 << 6;
const BORDER_COLOR: u32 = 1 << 7;

const ENABLED: u32 = 1 << 8;
const HIDDEN: u32 = 1 << 9;
const CLIP: u32 = 1 << 10;
const CORNER_RADIUS: u32 = 1 << 11;
const BORDER_WIDTH: u32 = 1 << 12;
const OPACITY: u32 = 1 << 13;
const FONT_SIZE: u32 = 1 << 14;
const FONT_WEIGHT: u32 = 1 << 15;

const TEXT_ALIGN: u32 = 1 << 16;
const TRANSFORM: u32 = 1 << 17;
const ANIMATION: u32 = 1 << 18;
const REST: u32 = 1 << 31;

/// Encodes one commit's mutations, failing once the transaction would pass
/// `limit` bytes.
pub(super) fn encode(
	root: Option<usize>,
	mutations: &[Mutation],
	limit: usize,
) -> io::Result<Vec<u8>> {
	let mut writer = Writer {
		bytes: Vec::with_capacity(16 + mutations.len() * 24),
		limit,
	};
	writer.put(MAGIC)?;

	let root = match root {
		Some(root) => i64::try_from(root).map_err(|_| too_large("root id"))?,
		None => -1,
	};

	writer.put(&root.to_le_bytes())?;
	writer.u32(u32::try_from(mutations.len()).map_err(|_| too_large("mutation count"))?)?;

	for mutation in mutations {
		match mutation {
			Mutation::Create { id, kind, props } => {
				writer.put(&[CREATE])?;
				writer.id(id.index())?;
				let name = kind.wire_name();
				writer.put(&[u8::try_from(name.len()).map_err(|_| too_large("kind name"))?])?;
				writer.put(name.as_bytes())?;
				writer.props(props)?;
			}
			Mutation::UpdateProps { id, props } => {
				writer.put(&[UPDATE_PROPS])?;
				writer.id(id.index())?;
				writer.props(props)?;
			}
			Mutation::UpdateLayout {
				id,
				x,
				y,
				width,
				height,
			} => {
				writer.put(&[UPDATE_LAYOUT])?;
				writer.id(id.index())?;

				for value in [x, y, width, height] {
					writer.put(&value.to_le_bytes())?;
				}
			}
			Mutation::InsertChild {
				parent,
				child,
				index,
			} => {
				writer.put(&[INSERT_CHILD])?;
				writer.id(parent.index())?;
				writer.id(child.index())?;
				writer.id(*index)?;
			}
			Mutation::RemoveChild { parent, child } => {
				writer.put(&[REMOVE_CHILD])?;
				writer.id(parent.index())?;
				writer.id(child.index())?;
			}
			Mutation::Delete { id } => {
				writer.put(&[DELETE])?;
				writer.id(id.index())?;
			}
		}
	}

	Ok(writer.bytes)
}

fn too_large(what: &str) -> io::Error {
	io::Error::other(format!("render transaction {what} out of range"))
}

struct Writer {
	bytes: Vec<u8>,
	limit: usize,
}

impl Writer {
	fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
		if bytes.len() > self.limit - self.bytes.len() {
			return Err(io::Error::other("render transaction byte limit exceeded"));
		}

		self.bytes.extend_from_slice(bytes);
		Ok(())
	}

	fn u32(&mut self, value: u32) -> io::Result<()> {
		self.put(&value.to_le_bytes())
	}

	fn id(&mut self, value: usize) -> io::Result<()> {
		self.u32(u32::try_from(value).map_err(|_| too_large("id"))?)
	}

	fn string(&mut self, value: &str) -> io::Result<()> {
		self.u32(u32::try_from(value.len()).map_err(|_| too_large("string"))?)?;
		self.put(value.as_bytes())
	}

	fn props(&mut self, props: &NativeProps) -> io::Result<()> {
		let rest = encode_rest(props)?;

		let mut present = 0;
		for (bit, set) in [
			(TEXT, props.text.is_some()),
			(SOURCE, props.source.is_some()),
			(PLACEHOLDER, props.placeholder.is_some()),
			(VALUE, props.value.is_some()),
			(ACCESSIBILITY_LABEL, props.accessibility_label.is_some()),
			(COLOR, props.color.is_some()),
			(BACKGROUND, props.background.is_some()),
			(BORDER_COLOR, props.border_color.is_some()),
			(ENABLED, props.enabled.is_some()),
			(HIDDEN, props.hidden.is_some()),
			(CLIP, props.clip.is_some()),
			(CORNER_RADIUS, props.corner_radius.is_some()),
			(BORDER_WIDTH, props.border_width.is_some()),
			(OPACITY, props.opacity.is_some()),
			(FONT_SIZE, props.font_size.is_some()),
			(FONT_WEIGHT, props.font_weight.is_some()),
			(TEXT_ALIGN, props.text_align.is_some()),
			(TRANSFORM, props.transform.is_some()),
			(ANIMATION, props.animation.is_some()),
			(REST, rest.as_str().is_some()),
		] {
			if set {
				present |= bit;
			}
		}
		self.u32(present)?;

		for value in [
			&props.text,
			&props.source,
			&props.placeholder,
			&props.value,
			&props.accessibility_label,
		]
		.into_iter()
		.flatten()
		{
			self.string(value)?;
		}

		for value in [props.color, props.background, props.border_color]
			.into_iter()
			.flatten()
		{
			self.u32(value.0)?;
		}

		for value in [props.enabled, props.hidden, props.clip]
			.into_iter()
			.flatten()
		{
			self.put(&[u8::from(value)])?;
		}

		for value in [
			props.corner_radius,
			props.border_width,
			props.opacity,
			props.font_size,
		]
		.into_iter()
		.flatten()
		{
			self.put(&value.to_le_bytes())?;
		}

		if let Some(weight) = props.font_weight {
			// The weights added later follow the first four, so a renderer
			// built before them still reads those.
			self.put(&[match weight {
				FontWeight::Regular => 0,
				FontWeight::Medium => 1,
				FontWeight::Semibold => 2,
				FontWeight::Bold => 3,
				FontWeight::Thin => 4,
				FontWeight::ExtraLight => 5,
				FontWeight::Light => 6,
				FontWeight::ExtraBold => 7,
				FontWeight::Black => 8,
			}])?;
		}

		if let Some(align) = props.text_align {
			self.put(&[match align {
				TextAlign::Start => 0,
				TextAlign::Center => 1,
				TextAlign::End => 2,
			}])?;
		}

		if let Some(transform) = &props.transform {
			self.transform(transform)?;
		}

		if let Some(animation) = props.animation.as_deref() {
			self.animation(animation)?;
		}

		if let Some(rest) = rest.as_str() {
			self.string(rest)?;
		}

		Ok(())
	}

	fn f32s(&mut self, values: &[f32]) -> io::Result<()> {
		for value in values {
			self.put(&value.to_le_bytes())?;
		}

		Ok(())
	}

	fn transform(&mut self, transform: &Transform) -> io::Result<()> {
		self.f32s(&[
			transform.translate_x.points,
			transform.translate_x.percent,
			transform.translate_y.points,
			transform.translate_y.percent,
			transform.scale_x,
			transform.scale_y,
			transform.rotate,
		])
	}

	fn animation(&mut self, animation: &Animation) -> io::Result<()> {
		let parts = [
			animation.transition.is_some(),
			animation.keyframes.is_some(),
			animation.entering.is_some(),
			animation.exiting.is_some(),
			animation.layout.is_some(),
		]
		.into_iter()
		.enumerate()
		.fold(0_u8, |parts, (bit, set)| parts | (u8::from(set) << bit));
		self.put(&[parts])?;

		if let Some(transition) = animation.transition {
			self.put(&[transition.properties.bits()])?;
			self.f32s(&[transition.delay_ms])?;
			self.curve(transition.curve)?;
		}

		if let Some(keyframes) = &animation.keyframes {
			self.f32s(&[
				keyframes.duration_ms,
				keyframes.delay_ms,
				match keyframes.iterations {
					Iterations::Count(count) => count,
					Iterations::Infinite => f32::INFINITY,
				},
			])?;
			self.put(&[u8::from(keyframes.alternate)])?;
			self.easing(keyframes.easing)?;
			self.put(&[u8::try_from(keyframes.frames.len()).map_err(|_| too_large("keyframes"))?])?;

			for frame in &keyframes.frames {
				self.f32s(&[frame.offset])?;
				self.visual(&frame.visual)?;
				self.put(&[u8::from(frame.easing.is_some())])?;

				if let Some(easing) = frame.easing {
					self.easing(easing)?;
				}
			}
		}

		for presence in [animation.entering, animation.exiting]
			.into_iter()
			.flatten()
		{
			self.presence(&presence)?;
		}

		if let Some(curve) = animation.layout {
			self.curve(curve)?;
		}

		Ok(())
	}

	fn presence(&mut self, presence: &Presence) -> io::Result<()> {
		self.visual(&presence.visual)?;
		self.f32s(&[presence.delay_ms])?;
		self.curve(presence.curve)
	}

	fn visual(&mut self, visual: &Visual) -> io::Result<()> {
		self.put(&[
			u8::from(visual.opacity.is_some()) | (u8::from(visual.transform.is_some()) << 1)
		])?;

		if let Some(opacity) = visual.opacity {
			self.f32s(&[opacity])?;
		}

		if let Some(transform) = &visual.transform {
			self.transform(transform)?;
		}

		Ok(())
	}

	fn curve(&mut self, curve: Curve) -> io::Result<()> {
		match curve {
			Curve::Timing {
				duration_ms,
				easing,
			} => {
				self.put(&[0])?;
				self.f32s(&[duration_ms])?;
				self.easing(easing)
			}
			Curve::Spring(spring) => {
				self.put(&[1])?;
				self.f32s(&[spring.mass, spring.stiffness, spring.damping])
			}
		}
	}

	fn easing(&mut self, easing: Easing) -> io::Result<()> {
		match easing {
			Easing::Linear => self.put(&[0]),
			Easing::CubicBezier(points) => {
				self.put(&[1])?;
				self.f32s(&points)
			}
		}
	}
}

enum RestProps {
	None,
	Static(&'static str),
	Owned(String),
}

impl RestProps {
	fn as_str(&self) -> Option<&str> {
		match self {
			Self::None => None,
			Self::Static(s) => Some(s),
			Self::Owned(s) => Some(s.as_str()),
		}
	}
}

/// Encodes any property outside the typed `LKB1` bitset as JSON, fast-pathing
/// the overwhelmingly common cases (empty or `direction`-only) with zero
/// cloning and zero JSON allocation.
fn encode_rest(props: &NativeProps) -> io::Result<RestProps> {
	let NativeProps {
		// Typed fields sent directly in the binary record.
		text: _,
		source: _,
		placeholder: _,
		value: _,
		accessibility_label: _,
		color: _,
		background: _,
		border_color: _,
		enabled: _,
		hidden: _,
		clip: _,
		corner_radius: _,
		border_width: _,
		opacity: _,
		font_size: _,
		font_weight: _,
		text_align: _,
		transform: _,
		animation: _,
		// `motion` is `#[serde(skip)]`, so it never serializes.
		motion: _,
		// Non-typed fields that ride in the JSON `rest` payload.
		rive,
		scene3d,
		video,
		camera,
		map,
		live_photo,
		canvist_editor,
		autocomplete,
		keyboard,
		read_only,
		max_length,
		capitalize,
		autocorrect,
		submit_label,
		checked,
		radio_group,
		slider,
		progress,
		running,
		drawing,
		aria,
		id,
		border_widths,
		line_height,
		italic,
		font_family,
		spans,
		direction,
		focused,
		focus_request,
		reveal,
		scroll_offset,
		overlay,
		appearance,
		extra,
	} = props;

	let has_other_rest = rive.is_some()
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
		|| checked.is_some()
		|| radio_group.is_some()
		|| slider.is_some()
		|| progress.is_some()
		|| running.is_some()
		|| drawing.is_some()
		|| aria.is_some()
		|| id.is_some()
		|| border_widths.is_some()
		|| line_height.is_some()
		|| italic.is_some()
		|| font_family.is_some()
		|| spans.is_some()
		|| focused.is_some()
		|| focus_request.is_some()
		|| reveal.is_some()
		|| scroll_offset.is_some()
		|| overlay.is_some()
		|| appearance.is_some()
		|| !extra.is_empty();

	if !has_other_rest {
		return Ok(match direction {
			None => RestProps::None,
			Some(leptoskit_yoga::Direction::Ltr) => RestProps::Static("{\"direction\":\"Ltr\"}"),
			Some(leptoskit_yoga::Direction::Rtl) => RestProps::Static("{\"direction\":\"Rtl\"}"),
		});
	}

	let mut rest = props.clone();
	rest.text = None;
	rest.source = None;
	rest.placeholder = None;
	rest.value = None;
	rest.accessibility_label = None;
	rest.color = None;
	rest.background = None;
	rest.border_color = None;
	rest.enabled = None;
	rest.hidden = None;
	rest.clip = None;
	rest.corner_radius = None;
	rest.border_width = None;
	rest.opacity = None;
	rest.font_size = None;
	rest.font_weight = None;
	rest.text_align = None;
	rest.transform = None;
	rest.animation = None;
	serde_json::to_string(&rest)
		.map(RestProps::Owned)
		.map_err(io::Error::other)
}

#[cfg(test)]
mod tests;
