//! Serialize typed base styles below the specificity of utility classes.

use std::fmt::Write;
use std::sync::LazyLock;

use leptoskit_yoga::Align;
use leptoskit_yoga::BoxSizing;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Direction;
use leptoskit_yoga::Display;
use leptoskit_yoga::FlexDirection;
use leptoskit_yoga::FlexWrap;
use leptoskit_yoga::GridStyle;
use leptoskit_yoga::JustifyContent;
use leptoskit_yoga::Overflow;
use leptoskit_yoga::PositionType;
use leptoskit_yoga::Rect;
use leptoskit_yoga::TrackSizing;

use crate::Color;
use crate::Element;
use crate::NativeKind;
use crate::NativeProps;
use crate::TextStyle;
use crate::tailwind::keyword;

/// The typed base styles of a plain, unstyled `View`, which most elements
/// share almost entirely. [`super::render`] writes them once per
/// stylesheet, and each scope's sheet holds only its [`delta_for`] from them.
pub(super) fn defaults() -> &'static str {
	static DEFAULTS: LazyLock<String> = LazyLock::new(|| base(&Element::view(), false));
	&DEFAULTS
}

/// Emits only the typed base declarations of `element` that differ from
/// [`defaults`], directly without formatting or filtering default properties.
///
/// For plain `View` / `Text` nodes styled with Tailwind classes, this returns
/// an empty `String` with zero heap allocations.
#[expect(
	clippy::too_many_lines,
	clippy::cognitive_complexity,
	reason = "flat exhaustive emission of non-default base CSS declarations keeps hot-path \
	          zero-allocation"
)]
pub(super) fn delta_for(element: &Element, root: bool) -> String {
	let mut css = String::new();
	let props = &element.props;
	let style = &element.style;

	// Widget defaults that differ from a plain View.
	if matches!(
		super::semantics::tag(
			element.kind,
			props.aria(),
			props.names_itself(),
			element.children.is_empty(),
		),
		super::semantics::Tag::List | super::semantics::Tag::ListItem
	) {
		css.push_str("list-style:none;");
	}

	if matches!(
		element.kind,
		NativeKind::Checkbox
			| NativeKind::Radio
			| NativeKind::Slider
			| NativeKind::LinearProgressIndicator
	) {
		css.push_str("appearance:auto;accent-color:currentColor;");
	}

	if element.kind == NativeKind::Switch {
		if props.corner_radius.is_none() {
			css.push_str("border-radius:999px;");
		}

		if props.background.is_none() {
			css.push_str("background-color:#777;");
		}

		css.push_str(
			"background-image:radial-gradient(circle,#fff 68%,transparent \
			 72%);background-size:20px 20px;background-repeat:no-repeat;background-position:2px \
			 center;",
		);

		if let Some(color) = props.background {
			write_color(&mut css, "--lk-switch-active", color);
		}
	}

	if root {
		css.push_str("font-family:system-ui,sans-serif;");

		if props.font_size.is_none() {
			write!(css, "font-size:{}px;", TextStyle::DEFAULT_FONT_SIZE).expect("write to String");
		}

		if props.font_weight.is_none() {
			css.push_str("font-weight:400;");
		}

		if props.text_align.is_none() && element.kind != NativeKind::Button {
			css.push_str("text-align:start;");
		}
	}

	// Box layout fields that differ from `Style::default()`.
	let final_display = if props.hidden == Some(true) {
		"none"
	} else if style.display == Display::Flex && props.spans.is_some() {
		"block"
	} else {
		keyword::display(style.display)
	};

	if final_display != "flex" {
		property(&mut css, "display", final_display);
	}

	if style.box_sizing != BoxSizing::BorderBox {
		property(
			&mut css,
			"box-sizing",
			keyword::box_sizing(style.box_sizing),
		);
	}

	if style.position != PositionType::Relative {
		property(&mut css, "position", keyword::position(style.position));
	}

	if style.direction != Direction::Ltr {
		property(&mut css, "direction", direction(style.direction));
	}

	if style.flex_direction != FlexDirection::Column {
		property(
			&mut css,
			"flex-direction",
			keyword::flex_direction(style.flex_direction),
		);
	}

	if style.flex_wrap != FlexWrap::NoWrap {
		property(&mut css, "flex-wrap", keyword::flex_wrap(style.flex_wrap));
	}

	if style.justify_content != JustifyContent::FlexStart {
		property(
			&mut css,
			"justify-content",
			keyword::justify_content(style.justify_content),
		);
	}

	if style.align_items != Align::Stretch {
		property(&mut css, "align-items", keyword::align(style.align_items));
	}

	if style.align_self != Align::Auto {
		property(&mut css, "align-self", keyword::align(style.align_self));
	}

	if style.align_content != Align::Auto {
		property(
			&mut css,
			"align-content",
			keyword::align(style.align_content),
		);
	}

	let final_overflow = if props.clip == Some(true) && element.kind != NativeKind::ScrollView {
		"hidden"
	} else {
		overflow(element.kind, style.overflow)
	};

	if final_overflow != "visible" {
		property(&mut css, "overflow", final_overflow);
	}

	if style.flex_grow != 0.0 {
		write!(css, "flex-grow:{};", style.flex_grow).expect("write to String");
	}

	if style.flex_shrink != 0.0 {
		write!(css, "flex-shrink:{};", style.flex_shrink).expect("write to String");
	}

	if style.flex_basis != Dimension::Auto {
		write_dimension(&mut css, "flex-basis", style.flex_basis, "auto");
	}

	if style.width != Dimension::Auto {
		write_dimension(&mut css, "width", style.width, "auto");
	}

	if style.height != Dimension::Auto {
		write_dimension(&mut css, "height", style.height, "auto");
	}

	if !matches!(style.min_width, Dimension::Auto) && style.min_width != Dimension::Points(0.0) {
		write_dimension(&mut css, "min-width", style.min_width, "0px");
	}

	if !matches!(style.min_height, Dimension::Auto) && style.min_height != Dimension::Points(0.0) {
		write_dimension(&mut css, "min-height", style.min_height, "0px");
	}

	if style.max_width != Dimension::Auto {
		write_dimension(&mut css, "max-width", style.max_width, "none");
	}

	if style.max_height != Dimension::Auto {
		write_dimension(&mut css, "max-height", style.max_height, "none");
	}

	if !matches!(style.gap.top, Dimension::Auto) && style.gap.top != Dimension::Points(0.0) {
		write_dimension(&mut css, "row-gap", style.gap.top, "0px");
	}

	if !matches!(style.gap.left, Dimension::Auto) && style.gap.left != Dimension::Points(0.0) {
		write_dimension(&mut css, "column-gap", style.gap.left, "0px");
	}

	for (name, value) in [
		("margin-top", style.margin.top),
		("margin-right", style.margin.right),
		("margin-bottom", style.margin.bottom),
		("margin-left", style.margin.left),
	] {
		if value != Dimension::Points(0.0) {
			write_dimension(&mut css, name, value, "auto");
		}
	}

	for (name, value) in [
		("padding-top", style.padding.top),
		("padding-right", style.padding.right),
		("padding-bottom", style.padding.bottom),
		("padding-left", style.padding.left),
	] {
		if !matches!(value, Dimension::Auto) && value != Dimension::Points(0.0) {
			write_dimension(&mut css, name, value, "0px");
		}
	}

	for (edge, inset, border) in [
		("top", style.inset.top, style.border.top),
		("right", style.inset.right, style.border.right),
		("bottom", style.inset.bottom, style.border.bottom),
		("left", style.inset.left, style.border.left),
	] {
		if inset != Dimension::Auto {
			write_dimension(&mut css, edge, inset, "auto");
		}

		if border != 0.0 {
			write!(css, "border-{edge}-width:{border}px;").expect("write to String");
		}
	}

	if let Some(ratio) = style.aspect_ratio {
		write!(css, "aspect-ratio:{ratio};").expect("write to String");
	}

	if let Some(grid) = &style.grid {
		grid_layout(&mut css, grid);
	}

	paints(&mut css, props);
	corners(&mut css, props);
	motion(&mut css, props);
	text(&mut css, element);
	css
}

/// Native defaults are explicit so browser defaults cannot change layout.
fn base(element: &Element, root: bool) -> String {
	let mut css = String::from(
		"appearance:none;margin:0;border:0 solid \
		 transparent;--lk-border-color:currentColor;background:transparent;font:inherit;color:\
		 inherit;text-align:inherit;white-space:pre-wrap;text-decoration:none;",
	);
	widget_defaults(&mut css, element);

	if root {
		write!(
			css,
			"font-family:system-ui,sans-serif;font-size:{}px;font-weight:400;text-align:start;",
			TextStyle::DEFAULT_FONT_SIZE
		)
		.expect("write to String");
	}

	box_layout(&mut css, element);

	if let Some(grid) = &element.style.grid {
		grid_layout(&mut css, grid);
	}

	presentation(&mut css, element);
	css
}

/// The browser defaults a widget kind's element needs replaced, such as list
/// bullets and native control appearance.
fn widget_defaults(css: &mut String, element: &Element) {
	let props = &element.props;

	if matches!(
		super::semantics::tag(
			element.kind,
			props.aria(),
			props.names_itself(),
			element.children.is_empty(),
		),
		super::semantics::Tag::List | super::semantics::Tag::ListItem
	) {
		css.push_str("list-style:none;");
	}

	if matches!(
		element.kind,
		NativeKind::Checkbox
			| NativeKind::Radio
			| NativeKind::Slider
			| NativeKind::LinearProgressIndicator
	) {
		property(css, "appearance", "auto");
		property(css, "accent-color", "currentColor");
	}

	if element.kind == NativeKind::Switch {
		css.push_str(
			"border-radius:999px;background-color:#777;background-image:radial-gradient(circle,#\
			 fff 68%,transparent 72%);background-size:20px \
			 20px;background-repeat:no-repeat;background-position:2px center;",
		);

		if let Some(color) = props.background {
			write_color(css, "--lk-switch-active", color);
		}
	}
}

/// The flexbox and box-model properties of the typed style. A `Text` of
/// styled runs flows them inline, as one wrapping paragraph, so its flex
/// display becomes a block.
fn box_layout(css: &mut String, element: &Element) {
	let style = &element.style;

	let display = match style.display {
		Display::Flex if element.props.spans.is_some() => "block",
		display => keyword::display(display),
	};

	for (name, value) in [
		("display", display),
		("box-sizing", keyword::box_sizing(style.box_sizing)),
		("position", keyword::position(style.position)),
		("direction", direction(style.direction)),
		(
			"flex-direction",
			keyword::flex_direction(style.flex_direction),
		),
		("flex-wrap", keyword::flex_wrap(style.flex_wrap)),
		(
			"justify-content",
			keyword::justify_content(style.justify_content),
		),
		("align-items", keyword::align(style.align_items)),
		("align-self", keyword::align(style.align_self)),
		("align-content", keyword::align(style.align_content)),
		("overflow", overflow(element.kind, style.overflow)),
	] {
		property(css, name, value);
	}

	write!(
		css,
		"flex-grow:{};flex-shrink:{};",
		style.flex_grow, style.flex_shrink
	)
	.expect("write to String");

	for (name, value, auto) in [
		("flex-basis", style.flex_basis, "auto"),
		("width", style.width, "auto"),
		("height", style.height, "auto"),
		("min-width", style.min_width, "0px"),
		("min-height", style.min_height, "0px"),
		("max-width", style.max_width, "none"),
		("max-height", style.max_height, "none"),
		("row-gap", style.gap.top, "0px"),
		("column-gap", style.gap.left, "0px"),
	] {
		write_dimension(css, name, value, auto);
	}

	edges(css, "margin", style.margin, "auto");
	edges(css, "padding", style.padding, "0px");

	for (edge, value, border) in [
		("top", style.inset.top, style.border.top),
		("right", style.inset.right, style.border.right),
		("bottom", style.inset.bottom, style.border.bottom),
		("left", style.inset.left, style.border.left),
	] {
		write_dimension(css, edge, value, "auto");
		write!(css, "border-{edge}-width:{border}px;").expect("write to String");
	}

	if let Some(ratio) = style.aspect_ratio {
		write!(css, "aspect-ratio:{ratio};").expect("write to String");
	}
}

/// The grid container and grid item properties of the typed style.
fn grid_layout(css: &mut String, grid: &GridStyle) {
	property(
		css,
		"grid-template-columns",
		&tracks(&grid.template_columns),
	);
	property(css, "grid-template-rows", &tracks(&grid.template_rows));
	property(css, "grid-auto-columns", &track(&grid.auto_columns));
	property(css, "grid-auto-rows", &track(&grid.auto_rows));
	property(
		css,
		"grid-auto-flow",
		keyword::grid_auto_flow(grid.auto_flow),
	);
	property(css, "justify-items", keyword::align(grid.justify_items));
	property(css, "justify-self", keyword::align(grid.justify_self));

	for (name, line) in [
		("grid-column-start", grid.column.start),
		("grid-column-end", grid.column.end),
		("grid-row-start", grid.row.start),
		("grid-row-end", grid.row.end),
	] {
		property(css, name, &keyword::grid_line(line));
	}
}

/// The props that change how a box looks rather than where it sits: colors,
/// borders, corners, opacity, text, image fit, and the clip and hidden
/// overrides, which come last so they win over the layout properties.
fn presentation(css: &mut String, element: &Element) {
	paints(css, &element.props);
	corners(css, &element.props);
	motion(css, &element.props);
	text(css, element);
	overrides(css, element);
}

/// The transform, transition, and animations. A keyframe sequence and an
/// entering animation both play as CSS animations; [`motion_rules`] writes
/// their `@keyframes`.
fn motion(css: &mut String, props: &NativeProps) {
	if let Some(transform) = props.transform {
		css.push_str(&transform.css());
	}

	let Some(animation) = props.animation.as_deref() else {
		return;
	};

	if let Some(transition) = animation.transition {
		css.push_str(&transition.css());
	}

	let animations = motion_keyframes(animation)
		.into_iter()
		.map(|(_, value)| value)
		.collect::<Vec<_>>();

	if !animations.is_empty() {
		property(css, "animation", &animations.join(","));
	}
}

/// The `@keyframes` rules a node's animations play.
pub(super) fn motion_rules(props: &NativeProps) -> String {
	props
		.animation
		.as_deref()
		.map(|animation| {
			motion_keyframes(animation)
				.into_iter()
				.map(|(rule, _)| rule)
				.collect()
		})
		.unwrap_or_default()
}

/// Each animation's `@keyframes` rule and `animation` value. Names come
/// from the rule's content, so equal animations share one rule.
fn motion_keyframes(animation: &crate::Animation) -> Vec<(String, String)> {
	let mut keyframes = Vec::new();

	if let Some(sequence) = &animation.keyframes {
		let (rule, _) = sequence.css("_");
		keyframes.push(sequence.css(&keyframes_name("k", &rule)));
	}

	// Entering plays once from the entering look to the widget's own, which
	// a `from`-only rule leaves to the browser. `backwards` holds the
	// entering look through the delay.
	if let Some(entering) = animation.entering {
		let body = format!("from{{{}}}", entering.visual.css());
		let name = keyframes_name("e", &body);
		let (duration, easing) = entering.curve.css();
		keyframes.push((
			format!("@keyframes {name}{{{body}}}"),
			format!(
				"{name} {duration}ms {easing} {}ms backwards",
				entering.delay_ms
			),
		));
	}

	keyframes
}

fn keyframes_name(kind: &str, rule: &str) -> String {
	format!("lk-{kind}{:016x}", super::scope_hash(rule.as_bytes()))
}

/// The colors and borders that paint inside the box.
fn paints(css: &mut String, props: &NativeProps) {
	for (name, color) in [
		("color", props.color),
		("background-color", props.background),
		("--lk-border-color", props.border_color),
	] {
		if let Some(color) = color {
			write_color(css, name, color);
		}
	}

	if let Some(widths) = props.border_widths {
		// Per-edge border props paint inside the box too, one inset shadow
		// per drawn edge.
		let shadows = [
			(widths.top, 0.0, 1.0),
			(widths.right, -1.0, 0.0),
			(widths.bottom, 0.0, -1.0),
			(widths.left, 1.0, 0.0),
		]
		.into_iter()
		.filter(|(width, ..)| *width > 0.0)
		.map(|(width, x, y)| {
			format!(
				"inset {}px {}px 0 0 var(--lk-border-color)",
				x * width,
				y * width
			)
		})
		.collect::<Vec<_>>();

		if !shadows.is_empty() {
			property(css, "box-shadow", &shadows.join(","));
		}
	} else if let Some(width) = props.border_width {
		// Legacy border props paint inside the box without affecting layout.
		write!(
			css,
			"box-shadow:inset 0 0 0 {width}px var(--lk-border-color);"
		)
		.expect("write to String");
	}
}

/// The corners and opacity.
fn corners(css: &mut String, props: &NativeProps) {
	if let Some(radius) = props.corner_radius {
		write!(css, "border-radius:{radius}px;").expect("write to String");

		property(
			css,
			"clip-path",
			&if radius > 0.0 {
				format!("inset(0 round {radius}px)")
			} else {
				"none".to_owned()
			},
		);
	}

	if let Some(opacity) = props.opacity {
		write!(css, "opacity:{};", opacity.clamp(0.0, 1.0)).expect("write to String");
	}
}

/// Text and image fit.
fn text(css: &mut String, element: &Element) {
	let props = &element.props;

	if let Some(italic) = props.italic {
		property(css, "font-style", keyword::font_style(italic));
	}

	if let Some(size) = props.font_size {
		write!(css, "font-size:{size}px;").expect("write to String");
	}

	if let Some(weight) = props.font_weight {
		property(css, "font-weight", keyword::font_weight(weight));
	}

	if let Some(line_height) = props.line_height {
		property(css, "line-height", &keyword::line_height(line_height));
	}

	if let Some(alignment) = props.text_align {
		let value = keyword::text_align(alignment);
		property(css, "text-align", value);
		property(css, "--lk-text-align", value);
	} else if element.kind == NativeKind::Button {
		property(css, "text-align", "var(--lk-text-align,center)");
	}

	if element.kind == NativeKind::Image {
		property(css, "object-fit", "contain");
	}
}

/// The clip and hidden overrides, which win over layout.
fn overrides(css: &mut String, element: &Element) {
	let props = &element.props;

	if props.clip == Some(true) && element.kind != NativeKind::ScrollView {
		property(css, "overflow", "hidden");
	}

	if props.hidden == Some(true) {
		property(css, "display", "none");
	}
}
/// Control decoration stays separate from utility declarations, so shared
/// geometry and colors can still be styled through the usual base props.
///
/// A control without `active:` utilities dims while held, as native
/// controls without a platform highlight do. Browser buttons here never draw
/// native chrome, so every such button dims. The filter multiplies any
/// opacity the element already has, and no utility sets `filter`.
pub(super) fn control(selector: &str, kind: NativeKind, styles_active: bool) -> String {
	let pressed = if crate::element::is_pressable(kind) && !styles_active {
		format!(
			":where({selector}:active:not([aria-disabled=true])){{filter:opacity({});}}",
			crate::element::DEFAULT_PRESSED_OPACITY
		)
	} else {
		String::new()
	};

	match kind {
		NativeKind::Button => pressed,
		NativeKind::Pressable => format!("{selector} *{{pointer-events:none;}}{pressed}"),
		// The drawing covers the element's box and may paint past it, as
		// native drawings do; the element itself stays the hit target.
		NativeKind::Vector => {
			format!(
				"{selector}>svg{{position:absolute;inset:0;width:100%;height:100%;overflow:\
				 visible;pointer-events:none;}}"
			)
		}
		// A modal covers the viewport whatever its classes say, and the
		// browser's own dialog box (its size, margins, and frame) is gone.
		// A closed one shows nothing, though `display` is set for all.
		NativeKind::Modal => {
			format!(
				"{selector}{{position:fixed!important;inset:0!important;width:100%!important;				 \
				 height:100%!important;max-width:none!important;max-height:none!important;margin:				 \
				 0!important;}}{selector}:not([open]){{display:none!important;}}"
			)
		}
		// A popover is placed from its anchor's rectangle, in viewport
		// coordinates.
		NativeKind::Popover => {
			format!(
				"{selector}{{position:fixed!important;inset:auto;margin:0!important;				 \
				 }}{selector}:not(:popover-open){{display:none!important;}}"
			)
		}
		NativeKind::Switch => {
			format!(
				":where({selector}:checked){{background-color:var(--lk-switch-active,#2563eb);\
				 background-position:right 2px \
				 center;}}:where({selector}:focus-visible){{outline:2px solid \
				 Highlight;outline-offset:2px;}}:where({selector}:disabled){{opacity:.5;}}"
			)
		}
		NativeKind::ActivityIndicator => {
			format!(
				"{selector}::before{{content:\"\";box-sizing:border-box;width:100%;height:100%;\
				 border:2px solid \
				 currentColor;border-right-color:transparent;border-radius:50%;\
				 }}{selector}[data-lk-running=false]{{visibility:hidden;\
				 }}{selector}[data-lk-running=true]::before{{animation:lk-spinner .8s linear \
				 infinite;}}@keyframes \
				 lk-spinner{{to{{transform:rotate(360deg);}}}}@media(prefers-reduced-motion:\
				 reduce){{{selector}[data-lk-running=true]::before{{animation-duration:2s;}}}}"
			)
		}
		_ => String::new(),
	}
}

fn property(css: &mut String, name: &str, value: &str) {
	write!(css, "{name}:{value};").expect("write to String");
}

fn write_dimension(css: &mut String, name: &str, value: Dimension, auto: &str) {
	match value {
		Dimension::Auto => write!(css, "{name}:{auto};"),
		Dimension::Points(value) => write!(css, "{name}:{value}px;"),
		Dimension::Percent(value) => write!(css, "{name}:{}%;", value * 100.0),
	}
	.expect("write to String");
}

fn write_color(css: &mut String, name: &str, color: Color) {
	write!(css, "{name}:#{:08x};", color.0).expect("write to String");
}

fn edges(css: &mut String, name: &str, edges: Rect<Dimension>, auto: &str) {
	for (edge, value) in [
		("top", edges.top),
		("right", edges.right),
		("bottom", edges.bottom),
		("left", edges.left),
	] {
		match value {
			Dimension::Auto => write!(css, "{name}-{edge}:{auto};"),
			Dimension::Points(value) => write!(css, "{name}-{edge}:{value}px;"),
			Dimension::Percent(value) => write!(css, "{name}-{edge}:{}%;", value * 100.0),
		}
		.expect("write to String");
	}
}

fn tracks(values: &[TrackSizing]) -> String {
	if values.is_empty() {
		return "none".to_owned();
	}

	values.iter().map(track).collect::<Vec<_>>().join(" ")
}

fn track(value: &TrackSizing) -> String {
	match value {
		TrackSizing::Auto => "auto".to_owned(),
		TrackSizing::Points(value) => format!("{value}px"),
		TrackSizing::Percent(value) => format!("{}%", value * 100.0),
		TrackSizing::Fraction(value) => format!("{value}fr"),
		TrackSizing::MinMax(bounds) => {
			format!("minmax({},{})", track(&bounds.min), track(&bounds.max))
		}
	}
}

pub(super) fn color_css(color: Color) -> String {
	format!("#{:08x}", color.0)
}

fn direction(value: Direction) -> &'static str {
	match value {
		Direction::Ltr => "ltr",
		Direction::Rtl => "rtl",
	}
}

/// Scroll views and scrolling overflow use `auto`, so scrollbars appear only
/// once the content overflows; hidden overflow clips even a scroll view.
fn overflow(kind: NativeKind, value: Overflow) -> &'static str {
	match (kind, value) {
		(_, Overflow::Hidden) => "hidden",
		(NativeKind::ScrollView, _) | (_, Overflow::Scroll) => "auto",
		_ => "visible",
	}
}

/// The inline style of one styled run of a `Text`.
pub(super) fn span(span: &crate::TextSpan) -> String {
	let mut css = String::new();

	if let Some(color) = span.color {
		write_color(&mut css, "color", color);
	}

	if let Some(background) = span.background {
		write_color(&mut css, "background-color", background);
	}

	if let Some(weight) = span.font_weight {
		property(&mut css, "font-weight", keyword::font_weight(weight));
	}

	if let Some(italic) = span.italic {
		property(&mut css, "font-style", keyword::font_style(italic));
	}

	css
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use leptoskit_yoga::Overflow;

	use super::*;
	use crate::BorderWidths;
	use crate::FontWeight;
	use crate::TextAlign;

	/// Each `name:value;` pair in a flat CSS declaration list.
	fn declarations(css: &str) -> impl Iterator<Item = (&str, &str)> {
		css.split(';')
			.filter(|entry| !entry.is_empty())
			.map(|entry| {
				entry
					.split_once(':')
					.expect("base style declarations have a name and a value")
			})
	}

	/// Each property's value once every declaration has applied in order.
	fn cascade<'a>(
		declarations: impl Iterator<Item = (&'a str, &'a str)>,
	) -> BTreeMap<&'a str, &'a str> {
		declarations.collect()
	}

	#[test]
	fn the_defaults_and_a_delta_resolve_every_property_as_the_base_does() {
		let styled = Element::view()
			.with_style(|style| {
				style.width = Dimension::Points(230.0);
				style.margin.top = Dimension::Auto;
				style.padding.left = Dimension::Percent(0.5);
				style.overflow = Overflow::Scroll;
				style.aspect_ratio = Some(1.5);
				style.grid = Some(Box::default());
			})
			.background(Color::rgba(1, 2, 3, 255))
			.corner_radius(8.0)
			.opacity(0.5)
			.font(18.0, FontWeight::Bold)
			.text_align(TextAlign::Center)
			.with_props(|props| {
				props.italic = Some(true);
				props.border_widths = Some(BorderWidths {
					top: 1.0,
					right: 0.0,
					bottom: 2.0,
					left: 0.0,
				});
			});
		let cases = [
			(Element::view(), false),
			(Element::view(), true),
			(Element::text("Plain"), false),
			(Element::button("Press"), false),
			(Element::image("photo.png"), false),
			(Element::scroll_view(), false),
			(
				Element::new(NativeKind::Switch).background(Color::rgba(0, 0, 255, 255)),
				false,
			),
			(Element::new(NativeKind::Switch), false),
			(Element::new(NativeKind::Checkbox), false),
			(styled, false),
			(
				Element::view().with_props(|props| {
					props.hidden = Some(true);
					props.clip = Some(true);
				}),
				false,
			),
		];

		assert_eq!(delta_for(&Element::view(), false), "");

		for (element, root) in cases {
			let base = base(&element, root);
			let delta = delta_for(&element, root);

			assert_eq!(
				cascade(declarations(defaults()).chain(declarations(&delta))),
				cascade(declarations(&base)),
				"{:?} (root: {root}): {delta}",
				element.kind
			);

			// A shorthand in the delta would reset longhands it leaves to the
			// defaults.
			for (name, _) in declarations(&delta) {
				assert!(
					!matches!(
						name,
						"margin" | "padding" | "border" | "background" | "font" | "inset"
					),
					"{name} in {delta}"
				);
			}
		}
	}
}
