//! Font weights and line heights: the Tailwind utilities, the props they
//! set, how text inherits them, and the size layout reserves for them.
#![allow(clippy::float_cmp)]

use leptoskit_native::ApproxTextMeasurer;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::LineHeight;
use leptoskit_native::NativeProps;
use leptoskit_native::NativeSurface;
use leptoskit_native::StyleContext;
use leptoskit_native::Tailwind;
use leptoskit_native::TextMeasurer;
use leptoskit_native::TextStyle;
use leptoskit_yoga::AvailableSpace;
use leptoskit_yoga::Size;
use leptoskit_yoga::Style;

fn props(classes: &str) -> NativeProps {
	let mut style = Style::default();
	let mut props = NativeProps::default();
	Tailwind::parse(classes)
		.unwrap()
		.apply(&mut style, &mut props, &StyleContext::default());
	props
}

fn css(classes: &str) -> String {
	Tailwind::parse(classes).unwrap().css()
}

#[test]
fn every_tailwind_font_weight_sets_its_weight_and_css_number() {
	let weights = [
		("font-thin", FontWeight::Thin, 100),
		("font-extralight", FontWeight::ExtraLight, 200),
		("font-light", FontWeight::Light, 300),
		("font-normal", FontWeight::Regular, 400),
		("font-medium", FontWeight::Medium, 500),
		("font-semibold", FontWeight::Semibold, 600),
		("font-bold", FontWeight::Bold, 700),
		("font-extrabold", FontWeight::ExtraBold, 800),
		("font-black", FontWeight::Black, 900),
	];

	for (class, weight, value) in weights {
		assert_eq!(props(class).font_weight, Some(weight), "{class}");
		assert_eq!(weight.value(), value, "{class}");
		assert!(
			css(class).contains(&format!("font-weight:{value};")),
			"{class}: {}",
			css(class)
		);
	}

	assert_eq!(
		FontWeight::ALL.map(FontWeight::value),
		[100, 200, 300, 400, 500, 600, 700, 800, 900]
	);
	assert!(FontWeight::ALL.is_sorted());
	assert_eq!(FontWeight::default(), FontWeight::Regular);
}

#[test]
fn named_line_heights_are_multiples_of_the_font_size() {
	let named = [
		("leading-none", 1.0, "1"),
		("leading-tight", 1.25, "1.25"),
		("leading-snug", 1.375, "1.375"),
		("leading-normal", 1.5, "1.5"),
		("leading-relaxed", 1.625, "1.625"),
		("leading-loose", 2.0, "2"),
	];

	for (class, multiple, css_value) in named {
		assert_eq!(
			props(class).line_height,
			Some(LineHeight::Relative(multiple)),
			"{class}"
		);
		assert!(
			css(class).contains(&format!("line-height:{css_value};")),
			"{class}: {}",
			css(class)
		);
	}
}

#[test]
fn numeric_line_heights_follow_the_spacing_scale() {
	for step in 3..=10 {
		let class = format!("leading-{step}");
		assert_eq!(
			props(&class).line_height,
			Some(LineHeight::Fixed(step as f32 * 4.0)),
			"{class}"
		);
		assert!(
			css(&class).contains(&format!("line-height:{}rem;", step as f32 / 4.0)),
			"{class}"
		);
	}

	// Tailwind 4 takes any step on the scale, fractions included.
	assert_eq!(
		props("leading-12").line_height,
		Some(LineHeight::Fixed(48.0))
	);
	assert_eq!(
		props("leading-4.5").line_height,
		Some(LineHeight::Fixed(18.0))
	);
}

#[test]
fn arbitrary_line_heights_are_multiples_lengths_or_percentages() {
	assert_eq!(
		props("leading-[1.4]").line_height,
		Some(LineHeight::Relative(1.4))
	);
	assert!(css("leading-[1.4]").contains("line-height:1.4;"));

	assert_eq!(
		props("leading-[20px]").line_height,
		Some(LineHeight::Fixed(20.0))
	);
	assert!(css("leading-[20px]").contains("line-height:20px;"));

	assert_eq!(
		props("leading-[1.5rem]").line_height,
		Some(LineHeight::Fixed(24.0))
	);
	assert_eq!(
		props("leading-[length:18px]").line_height,
		Some(LineHeight::Fixed(18.0))
	);

	// A percentage is of the font size.
	assert_eq!(
		props("leading-[150%]").line_height,
		Some(LineHeight::Relative(1.5))
	);
	assert!(css("leading-[150%]").contains("line-height:1.5;"));
}

#[test]
fn line_heights_that_cannot_be_drawn_are_errors() {
	for class in [
		"leading-0",
		"leading-[0]",
		"leading-[0px]",
		"leading-[-1]",
		"leading-[0%]",
		"leading-auto",
		"leading-wide",
		"leading-[1em]",
		"leading-[calc(1rem+2px)]",
		"leading--4",
		"font-heavy",
		"font-ultrabold",
	] {
		assert!(Tailwind::parse(class).is_err(), "{class}");
	}
}

#[test]
fn text_sizes_take_a_line_height_after_a_slash() {
	let sized = props("text-sm/6");
	assert_eq!(sized.font_size, Some(14.0));
	assert_eq!(sized.line_height, Some(LineHeight::Fixed(24.0)));
	assert!(css("text-sm/6").contains("font-size:0.875rem;line-height:1.5rem;"));

	let named = props("text-lg/tight");
	assert_eq!(named.font_size, Some(18.0));
	assert_eq!(named.line_height, Some(LineHeight::Relative(1.25)));

	let arbitrary = props("text-[15px]/[22px]");
	assert_eq!(arbitrary.font_size, Some(15.0));
	assert_eq!(arbitrary.line_height, Some(LineHeight::Fixed(22.0)));

	// After a color, a slash is the color's opacity.
	let translucent = props("text-black/50");
	assert_eq!(translucent.color.unwrap().alpha(), 128);
	assert_eq!(translucent.line_height, None);
	assert_eq!(translucent.font_size, None);

	assert!(Tailwind::parse("text-sm/0").is_err());
	assert!(Tailwind::parse("text-sm/wide").is_err());
	assert!(Tailwind::parse("text-[2em]/6").is_err());
}

#[test]
fn line_heights_resolve_against_each_text_s_own_font_size() {
	let relative = NativeProps {
		font_size: Some(20.0),
		line_height: Some(LineHeight::Relative(1.5)),
		..NativeProps::default()
	};
	assert_eq!(relative.text_style().line_height, Some(30.0));

	let default_size = NativeProps {
		line_height: Some(LineHeight::Relative(2.0)),
		..NativeProps::default()
	};
	assert_eq!(
		default_size.text_style().line_height,
		Some(TextStyle::DEFAULT_FONT_SIZE * 2.0)
	);

	let fixed = NativeProps {
		font_size: Some(20.0),
		line_height: Some(LineHeight::Fixed(18.0)),
		..NativeProps::default()
	};
	assert_eq!(fixed.text_style().line_height, Some(18.0));

	assert_eq!(NativeProps::default().text_style().line_height, None);

	for unusable in [0.0, -1.0, f32::NAN, f32::INFINITY] {
		assert_eq!(LineHeight::Fixed(unusable).points(14.0), None);
		assert_eq!(LineHeight::Relative(unusable).points(14.0), None);
	}
}

#[test]
fn descendants_inherit_the_line_height_and_rescale_multiples() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_root(
		&Element::view()
			.class("leading-loose text-xs")
			.child(Element::text("Inherited"))
			.child(Element::text("Larger").class("text-xl"))
			.child(Element::text("Own").class("leading-6")),
	);
	let root = surface.root().unwrap();
	let children = surface.children(root).to_vec();
	let style = |index: usize| surface.props(children[index]).text_style();

	assert_eq!(style(0).line_height, Some(24.0));
	assert_eq!(style(1).line_height, Some(40.0));
	assert_eq!(style(2).line_height, Some(24.0));
}

#[test]
fn a_new_parent_line_height_reaches_its_children() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	let tree = |class: &str| Element::view().class(class).child(Element::text("Line"));
	surface.set_root(&tree("leading-4"));
	let text = surface.children(surface.root().unwrap())[0];
	assert_eq!(surface.props(text).text_style().line_height, Some(16.0));

	surface.set_root(&tree("leading-8"));
	let text = surface.children(surface.root().unwrap())[0];
	assert_eq!(surface.props(text).text_style().line_height, Some(32.0));
}

#[test]
fn the_builder_sets_a_line_height() {
	let element = Element::text("Built").line_height(LineHeight::Fixed(30.0));
	assert_eq!(element.props.line_height, Some(LineHeight::Fixed(30.0)));
}

#[test]
fn the_approximate_measurer_sets_each_line_at_the_line_height() {
	let unbounded = Size::new(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
	let style = TextStyle {
		line_height: Some(30.0),
		..TextStyle::default()
	};

	assert_eq!(
		ApproxTextMeasurer::new().measure("one\ntwo\nthree", style, unbounded),
		Size::new(40.0, 90.0)
	);
	assert_eq!(
		ApproxTextMeasurer::new().measure("one\ntwo\nthree", TextStyle::default(), unbounded),
		Size::new(40.0, 48.0)
	);
}

#[test]
fn layout_reserves_the_line_height_for_labels_but_not_text_fields() {
	let mut surface = NativeSurface::new(HeadlessBackend::new());
	surface.set_viewport(320.0, 240.0);
	surface.set_root(
		&Element::view()
			.child(Element::text("one\ntwo").class("leading-10"))
			.child(
				Element::text_input()
					.placeholder("Typed")
					.class("leading-10"),
			)
			.child(Element::text_input().placeholder("Typed")),
	);
	let root = surface.root().unwrap();
	let children = surface.children(root).to_vec();
	let height = |index: usize| surface.frame(children[index]).unwrap().height;

	assert_eq!(height(0), 80.0);
	assert_eq!(height(1), height(2));
}

#[test]
fn fitting_counts_whole_natural_lines() {
	let style = TextStyle {
		line_height: Some(24.0),
		..TextStyle::default()
	};

	assert_eq!(
		style.fit_line_height(Size::new(50.0, 49.0), 16.0),
		Size::new(50.0, 72.0)
	);
	// At least one line, even for a measurement shorter than one.
	assert_eq!(
		style.fit_line_height(Size::new(50.0, 3.0), 16.0),
		Size::new(50.0, 24.0)
	);
	// No natural line to count by, or no line height: unchanged.
	assert_eq!(
		style.fit_line_height(Size::new(50.0, 49.0), 0.0),
		Size::new(50.0, 49.0)
	);
	assert_eq!(
		style.fit_line_height(Size::new(50.0, 49.0), f32::NAN),
		Size::new(50.0, 49.0)
	);
	assert_eq!(
		TextStyle::default().fit_line_height(Size::new(50.0, 49.0), 16.0),
		Size::new(50.0, 49.0)
	);
}

#[cfg(feature = "serde")]
#[test]
fn weights_and_line_heights_cross_the_wire_in_snake_case() {
	let props = NativeProps {
		font_weight: Some(FontWeight::ExtraLight),
		line_height: Some(LineHeight::Relative(1.25)),
		..NativeProps::text("Hello")
	};
	let json = serde_json::to_value(&props).unwrap();
	assert_eq!(json["font_weight"], "extra_light");
	assert_eq!(json["line_height"], serde_json::json!({ "relative": 1.25 }));
	assert_eq!(serde_json::from_value::<NativeProps>(json).unwrap(), props);

	let fixed = NativeProps {
		font_weight: Some(FontWeight::ExtraBold),
		line_height: Some(LineHeight::Fixed(20.0)),
		..NativeProps::default()
	};
	let json = serde_json::to_value(&fixed).unwrap();
	assert_eq!(json["font_weight"], "extra_bold");
	assert_eq!(json["line_height"], serde_json::json!({ "fixed": 20.0 }));

	let unset = serde_json::to_value(NativeProps::default()).unwrap();
	assert!(unset.get("line_height").is_none());
}
