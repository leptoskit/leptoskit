//! The element builder calls apps compose by hand, without a view macro.

use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_native::Color;
use leptoskit_native::ControlTreeError;
use leptoskit_native::Drawing;
use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::FontWeight;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::KeyPress;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::NativeSurface;
use leptoskit_native::PathData;
use leptoskit_native::Progress;
use leptoskit_native::ProgressValue;
use leptoskit_native::RadioGroup;
use leptoskit_native::ScrollOffset;
use leptoskit_native::SliderState;
use leptoskit_native::TextAlign;
use leptoskit_native::VectorAlign;
use leptoskit_native::VectorAlignment;
use leptoskit_native::VectorPath;
use leptoskit_native::VectorScaling;
use leptoskit_native::ViewBox;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::Style;

#[test]
fn style_and_prop_setters_replace_or_edit_their_groups() {
	let element = Element::view()
		.style(Style {
			width: Dimension::Points(10.0),
			..Style::default()
		})
		.with_style(|style| style.height = Dimension::Points(20.0))
		.props(NativeProps {
			opacity: Some(0.25),
			..NativeProps::default()
		})
		.with_props(|props| props.corner_radius = Some(4.0));

	assert_eq!(element.style.width, Dimension::Points(10.0));
	assert_eq!(element.style.height, Dimension::Points(20.0));
	assert_eq!(element.props.opacity, Some(0.25));
	assert_eq!(element.props.corner_radius, Some(4.0));
}

#[test]
fn control_and_scroll_setters_store_their_typed_values() {
	let group = RadioGroup::new("plan").expect("a valid group");
	let slider = SliderState::new(0.0, 10.0, 5.0, Some(1.0)).expect("a valid slider");
	let progress = Progress::Determinate(ProgressValue::new(0.5).expect("a valid progress"));
	let offset = ScrollOffset::new(0.0, 120.0).animated();
	let element = Element::new(NativeKind::Radio)
		.checked(true)
		.radio_group(group.clone())
		.slider(slider)
		.progress(progress)
		.running(false)
		.focused(true)
		.scroll_offset(offset);

	assert_eq!(element.props.checked, Some(true));
	assert_eq!(element.props.radio_group.as_ref(), Some(&group));
	assert_eq!(element.props.slider, Some(slider));
	assert_eq!(element.props.progress, Some(progress));
	assert_eq!(element.props.running, Some(false));
	assert_eq!(element.props.focused, Some(true));
	assert_eq!(element.props.scroll_offset, Some(offset));
	assert!(offset.animated);
}

#[test]
fn vector_setters_edit_the_drawing_in_painting_order() {
	let wide = ViewBox::new(0.0, 0.0, 20.0, 10.0).expect("a valid view box");
	let element = Element::vector(ViewBox::UNIT)
		.drawing(Drawing::new(ViewBox::UNIT))
		.path(VectorPath::new(PathData::default()))
		.paths([VectorPath::new(PathData::default())])
		.view_box(wide)
		.scaling(VectorScaling::Stretch)
		.align(VectorAlignment::new(VectorAlign::Start, VectorAlign::End));

	let drawing = element.props.drawing.as_ref().expect("a drawing");
	assert_eq!(drawing.paths().len(), 2);
	assert_eq!(drawing.view_box(), wide);
	assert_eq!(drawing.vector_scaling(), VectorScaling::Stretch);
	assert_eq!(drawing.alignment().svg_keyword(), "xMinYMax");
	assert_eq!(element.props.opacity, None);
}

#[test]
fn paint_and_text_setters_reach_the_props() {
	let element = Element::view()
		.opacity(0.4)
		.color(Color::rgba(1, 2, 3, 4))
		.background(Color::rgba(5, 6, 7, 8))
		.corner_radius(6.0)
		.border(2.0, Color::rgba(9, 10, 11, 12))
		.font(18.0, FontWeight::Semibold)
		.text_align(TextAlign::Center);

	assert_eq!(element.props.opacity, Some(0.4));
	assert_eq!(element.props.color, Some(Color::rgba(1, 2, 3, 4)));
	assert_eq!(element.props.background, Some(Color::rgba(5, 6, 7, 8)));
	assert_eq!(element.props.corner_radius, Some(6.0));
	assert_eq!(element.props.border_width, Some(2.0));
	assert_eq!(element.props.border_color, Some(Color::rgba(9, 10, 11, 12)));
	assert_eq!(element.props.font_size, Some(18.0));
	assert_eq!(element.props.font_weight, Some(FontWeight::Semibold));
	assert_eq!(element.props.text_align, Some(TextAlign::Center));
}

#[test]
fn closures_bind_text_and_background_values() {
	let background = Element::view().background(|| Color::rgba(1, 2, 3, 4));
	assert!(background.bindings().background.is_some());
	assert_eq!(background.props.background, Some(Color::rgba(1, 2, 3, 4)));
	assert!(
		format!("{background:?}").contains("bindings"),
		"a bound element says so when debugged"
	);

	let text = Element::text_from(|| "live".to_owned());
	assert!(text.bindings().text.is_some());
	assert_eq!(text.props.text.as_deref(), Some("live"));

	let from = Element::background_from(|| Color::rgba(5, 6, 7, 8));
	assert_eq!(from.kind, NativeKind::View);
	assert!(from.bindings().background.is_some());
	assert_eq!(from.props.background, Some(Color::rgba(5, 6, 7, 8)));
}

#[test]
fn plain_text_children_join_their_parent_and_styled_ones_stay() {
	let joined = Element::text("Count: ").child(Element::text("3"));
	assert_eq!(joined.props.text.as_deref(), Some("Count: 3"));
	assert!(joined.children.is_empty());

	let button = Element::button("Go").child(Element::text("!"));
	assert_eq!(button.props.text.as_deref(), Some("Go!"));

	for child in [
		Element::text("styled").class("font-bold"),
		Element::text("handled").on_press(|| {}),
		Element::text("sized").with_style(|style| style.width = Dimension::Points(1.0)),
		Element::text("colored").color(Color::rgba(1, 2, 3, 4)),
	] {
		let parent = Element::text("base").child(child);
		assert_eq!(parent.props.text.as_deref(), Some("base"));
		assert_eq!(parent.children.len(), 1);
	}
}

#[test]
fn text_content_reads_visible_children_and_skips_hidden_ones() {
	let element = Element::view()
		.child(Element::text("Shown").class("font-bold"))
		.child(Element::text("Hidden").aria_hidden(true));

	assert_eq!(element.text_content(), "Shown");
}

#[test]
fn typed_event_helpers_forward_their_events() {
	let keys = Arc::new(Mutex::new(Vec::new()));
	let changes = Arc::new(Mutex::new(Vec::new()));
	let toggles = Arc::new(Mutex::new(Vec::new()));
	let pressed = Arc::new(Mutex::new(0_usize));

	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(
		&Element::view()
			.on_key({
				let keys = Arc::clone(&keys);
				move |key| {
					keys.lock().unwrap().push(key.key.clone());
					EventOutcome::Handled
				}
			})
			.on_press({
				let pressed = Arc::clone(&pressed);
				move || *pressed.lock().unwrap() += 1
			})
			.child(Element::text_input().aria_label("Name").on_change({
				let changes = Arc::clone(&changes);
				move |value| changes.lock().unwrap().push(value.to_owned())
			}))
			.child(Element::new(NativeKind::Switch).on_toggle({
				let toggles = Arc::clone(&toggles);
				move |checked| toggles.lock().unwrap().push(checked)
			})),
	);

	let root = surface.root().expect("root");
	let input = surface.children(root)[0];
	let switch = surface.children(root)[1];

	assert_eq!(
		surface.dispatch_event(root, &NativeEvent::Key(KeyPress::new("p"))),
		EventOutcome::Handled
	);
	assert_eq!(
		surface.dispatch_event(
			input,
			&NativeEvent::Change {
				value: "typed".to_owned(),
			}
		),
		EventOutcome::Handled
	);
	assert_eq!(
		surface.dispatch_event(switch, &NativeEvent::Toggle { checked: true }),
		EventOutcome::Handled
	);
	assert_eq!(
		surface.dispatch_event(root, &NativeEvent::Press),
		EventOutcome::Handled
	);

	assert_eq!(*keys.lock().unwrap(), ["p"]);
	assert_eq!(*changes.lock().unwrap(), ["typed"]);
	assert_eq!(*toggles.lock().unwrap(), [true]);
	assert_eq!(*pressed.lock().unwrap(), 1);
}

#[test]
fn invalid_scroll_offsets_fail_validation() {
	let error = Element::scroll_view()
		.scroll_offset(ScrollOffset::new(f32::NAN, 0.0))
		.validate()
		.expect_err("a nonfinite offset cannot be applied");
	assert_eq!(error, ControlTreeError::InvalidScrollOffset);
}

#[test]
fn a_handler_is_part_of_the_debug_description() {
	let debug = format!("{:?}", Element::button("Go").on_press(|| {}));
	assert!(debug.contains("HandlerEntry"), "{debug}");
}

#[test]
fn a_component_scope_enters_and_leaves_cleanly() {
	let scope = leptoskit_native::view::__private::ComponentScope::enter("Card");
	drop(scope);
}
