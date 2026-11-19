#![cfg(all(feature = "web", not(target_arch = "wasm32")))]

use leptos::prelude::RenderHtml;
use leptoskit_native::Capitalize;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::FontWeight;
use leptoskit_native::Keyboard;
use leptoskit_native::NativeKind;
use leptoskit_native::SubmitLabel;
use leptoskit_native::components::Pressable;
use leptoskit_native::components::Text;
use leptoskit_native::components::TextInput;
use leptoskit_native::components::View;
use leptoskit_yoga::Dimension;

#[path = "support/href.rs"]
mod href_targets;

/// Server markup, without the `DevTools` attributes: those name where each
/// element was created, so they change with every edit, and these tests are
/// about what renders.
struct Markup(leptos::prelude::AnyView);

impl Markup {
	#[expect(
		clippy::wrong_self_convention,
		reason = "it stands in for `RenderHtml::to_html`, which the tests call"
	)]
	fn to_html(self) -> String {
		let html = self.0.to_html();

		if cfg!(feature = "devtools") {
			without_devtools_attributes(&html)
		} else {
			html
		}
	}
}

fn render(element: Element) -> Markup {
	Markup(leptoskit_native::web::render(element))
}

fn without_devtools_attributes(html: &str) -> String {
	let mut out = String::with_capacity(html.len());
	let mut rest = html;

	while let Some(start) = rest.find(" data-lk-") {
		let name_end = rest[start + 1..]
			.find('=')
			.map_or(rest.len(), |at| start + 1 + at);
		let name = &rest[start + 1..name_end];
		if !matches!(
			name,
			"data-lk-node"
				| "data-lk-key"
				| "data-lk-kind"
				| "data-lk-source"
				| "data-lk-component"
		) {
			out.push_str(&rest[..name_end]);
			rest = &rest[name_end..];

			continue;
		}
		out.push_str(&rest[..start]);
		// Skip `="value"`.
		let value_end = rest[name_end + 2..]
			.find('"')
			.map_or(rest.len(), |at| name_end + 2 + at + 1);
		rest = &rest[value_end..];
	}

	out.push_str(rest);
	out
}

#[test]
fn devtools_builds_mark_nodes_for_picking() {
	let html = leptoskit_native::web::render(View().child(Text("Hi"))).to_html();
	assert_eq!(html.contains(" data-lk-key=\""), cfg!(feature = "devtools"));
	assert_eq!(
		html.contains(" data-lk-node=\""),
		cfg!(feature = "devtools")
	);
}

#[test]
fn devtools_attributes_are_stripped_and_others_kept() {
	assert_eq!(
		without_devtools_attributes(
			r#"<div data-lk-style="a" data-lk-node=".0" data-lk-key="00ff" class="x"></div>"#
		),
		r#"<div data-lk-style="a" class="x"></div>"#
	);
}
#[cfg(feature = "rive")]
#[test]
fn rive_server_markup_names_the_canvas_without_serializing_its_asset() {
	use leptoskit_rive::Asset;
	use leptoskit_rive::Playback;
	let playback = Playback::new(Asset::new(b"RIVE\x07\x00\x00\x00").unwrap());
	let html = render(Element::rive(playback).aria_label("Upload progress")).to_html();
	assert!(html.contains("<canvas"));
	assert!(html.contains("role=\"img\""));
	assert!(html.contains("aria-label=\"Upload progress\""));
	assert!(
		!html.contains("UklWRQ"),
		"encoded files belong to the client owner, not page markup"
	);
}

#[test]
fn rich_pressables_render_semantic_buttons_with_an_explicit_action_name() {
	let html = render(
		Pressable("Open <profile>")
			.class("p-4 rounded-xl")
			.child(View().child(Text("Visible profile content"))),
	)
	.to_html();
	assert!(html.contains("<button"));
	assert!(html.contains("type=\"button\""));
	assert!(html.contains("aria-label=\"Open &lt;profile&gt;\""));
	assert!(html.contains("Visible profile content"));
	assert!(!html.contains(">Open &lt;profile&gt;"));
}

#[test]
#[should_panic(expected = "pressable requires a non-whitespace accessibility label")]
fn rich_pressables_reject_an_empty_accessible_name_before_rendering() {
	let _ = render(Pressable(" \t").child(Text("Content")));
}

#[test]
#[should_panic(expected = "pressable cannot contain interactive view descendants")]
fn rich_pressables_reject_hidden_interaction_in_decorative_content() {
	let _ = render(Pressable("Action").child(View().on_press(|| {})));
}

#[test]
fn renders_semantic_controls_and_escapes_user_content() {
	let element = Element::view()
		.class("p-4 md:p-8 text-slate-900")
		.child(Element::text("<script>alert('hello') & goodbye</script>"))
		.child(Element::button("Save").with_props(|props| props.enabled = Some(false)))
		.child(Element::image("/photo.png").with_props(|props| {
			props.accessibility_label = Some("A \"safe\" photo".into());
		}))
		.child(Element::text_input().with_props(|props| {
			props.value = Some("<hello> & \"world\"".into());
			props.placeholder = Some("Your name".into());
			props.accessibility_label = Some("Name".into());
		}));
	let html = render(element).to_html();

	assert!(html.contains("<div"));
	assert!(html.contains("<span"));
	assert!(html.contains("<button"));
	assert!(html.contains("type=\"button\""));
	assert!(html.contains(" disabled"));
	assert!(html.contains("<input"));
	assert!(html.contains("placeholder=\"Your name\""));
	assert!(html.contains("aria-label=\"Name\""));

	assert!(html.contains("src=\"/photo.png\""));
	assert!(html.contains("alt=\"A &quot;safe&quot; photo\""));
	assert!(html.contains("&lt;script&gt;"));
	assert!(!html.contains("<script>"));
	assert!(html.contains("value=\"&lt;hello&gt; &amp; &quot;world&quot;\""));
}

#[test]
fn a_text_naming_one_field_renders_as_its_label() {
	use leptoskit_native::components::*;

	let element = View().children(vec![
		Text("Email").id("email-label"),
		TextInput().id("email").aria_labelled_by(["email-label"]),
		Text("Remember me").id("remember-label"),
		Checkbox(false)
			.id("remember")
			.aria_labelled_by(["remember-label"]),
		// Named by two elements: neither is the field's one label.
		Text("Street").id("street-label"),
		Text("Optional").id("street-hint"),
		TextInput()
			.id("street")
			.aria_labelled_by(["street-label", "street-hint"]),
		// Two fields named after one text: a label has a single target.
		Text("Phone").id("phone-label"),
		TextInput().id("phone-1").aria_labelled_by(["phone-label"]),
		TextInput().id("phone-2").aria_labelled_by(["phone-label"]),
		// A field without an id has nothing for `for` to point at.
		Text("City").id("city-label"),
		TextInput().aria_labelled_by(["city-label"]),
		// A heading keeps its element, and a button is not a field.
		Text("Notes")
			.id("notes-label")
			.role(leptoskit_native::Role::Heading)
			.aria_level(2),
		TextArea().id("notes").aria_labelled_by(["notes-label"]),
		Text("Save").id("save-label"),
		Button("Go").id("save").aria_labelled_by(["save-label"]),
	]);
	let html = render(element).to_html();
	let labels = html
		.split("<label")
		.skip(1)
		.map(|label| label.split_once("</label>").expect("closing tag").0)
		.collect::<Vec<_>>();

	assert_eq!(labels.len(), 2, "{html}");
	assert!(labels[0].contains("for=\"email\""), "{}", labels[0]);
	assert!(labels[0].contains("id=\"email-label\""), "{}", labels[0]);
	assert!(labels[0].contains(">Email"), "{}", labels[0]);
	assert!(labels[1].contains("for=\"remember\""), "{}", labels[1]);
	assert!(labels[1].contains(">Remember me"), "{}", labels[1]);
	// The fields keep the relationship assistive technology reads.
	assert!(html.contains("aria-labelledby=\"email-label\""), "{html}");
	assert!(html.contains("<h2"), "{html}");
}

#[test]
fn a_canvist_editor_renders_the_container_its_host_looks_for() {
	let element = Element::view().children([
		Element::canvist_editor(7)
			.accessibility_label("Note")
			.placeholder("Write a note…")
			.with_props(|props| props.read_only = Some(true)),
		Element::canvist_editor(8).accessibility_label("Plain"),
	]);
	let html = render(element).to_html();

	let editor = html
		.split("<div data-lk-canvist-editor")
		.nth(1)
		.expect("first editor");
	let (tag, rest) = editor.split_once('>').expect("opening tag");
	assert!(tag.starts_with("=\"7\""), "{tag}");
	assert!(
		tag.contains("data-lk-placeholder=\"Write a note…\""),
		"{tag}"
	);
	assert!(tag.contains(" data-lk-read-only"), "{tag}");
	// The host's own text field is the text box; the container only carries
	// the name for it.
	assert!(tag.contains("aria-label=\"Note\""), "{tag}");
	assert!(!tag.contains("role="), "{tag}");
	assert!(!tag.contains("tabindex"), "{tag}");
	// Nothing inside until the host builds the editor.
	assert!(rest.starts_with("</div>"), "{rest}");

	let plain = html
		.split("<div data-lk-canvist-editor")
		.nth(2)
		.expect("second editor");
	let (tag, _) = plain.split_once('>').expect("opening tag");
	assert!(tag.starts_with("=\"8\""), "{tag}");
	assert!(!tag.contains("data-lk-placeholder"), "{tag}");
	assert!(!tag.contains("data-lk-read-only"), "{tag}");
}

#[test]
fn text_areas_render_a_textarea_with_their_lines_and_hints() {
	let element = Element::view().children([
		Element::text_area()
			.value("one\n<two>")
			.placeholder("Notes")
			.with_props(|props| {
				props.read_only = Some(true);
				props.max_length = Some(40);
				props.capitalize = Some(Capitalize::Sentences);
				props.autocorrect = Some(false);
				// A text area's return key adds a line: these two name
				// a keyboard and a return key only single-line fields have.
				props.keyboard = Some(Keyboard::Email);
				props.submit_label = Some(SubmitLabel::Go);
			}),
		Element::text_area().accessibility_label("Plain"),
	]);
	let html = render(element).to_html();

	let area = html.split("<textarea").nth(1).expect("first textarea");
	let (tag, rest) = area.split_once('>').expect("opening tag");
	assert!(tag.contains("placeholder=\"Notes\""), "{tag}");
	assert!(tag.contains(" readonly"), "{tag}");
	assert!(tag.contains("maxlength=\"40\""), "{tag}");
	assert!(tag.contains("autocapitalize=\"sentences\""), "{tag}");
	assert!(tag.contains("autocorrect=\"off\""), "{tag}");
	assert!(tag.contains("spellcheck=\"false\""), "{tag}");
	assert!(!tag.contains("type="), "{tag}");
	assert!(!tag.contains("inputmode"), "{tag}");
	assert!(!tag.contains("enterkeyhint"), "{tag}");
	// The server writes the value as the element's escaped text.
	assert!(rest.starts_with("one\n&lt;two&gt;</textarea>"), "{rest}");

	let plain = html.split("<textarea").nth(2).expect("second textarea");
	let (tag, rest) = plain.split_once('>').expect("opening tag");
	assert!(tag.contains("aria-label=\"Plain\""), "{tag}");
	assert!(!tag.contains("readonly"), "{tag}");
	assert!(!tag.contains("maxlength"), "{tag}");
	assert!(rest.starts_with("</textarea>"), "{rest}");
	assert!(!html.contains("<input"));
}

#[test]
fn typed_text_input_hints_render_exact_attributes() {
	let element = Element::view().children([
		Element::text_input().with_props(|props| {
			props.keyboard = Some(Keyboard::Email);
			props.read_only = Some(true);
			props.max_length = Some(9);
			props.capitalize = Some(Capitalize::Characters);
			props.autocorrect = Some(false);
			props.submit_label = Some(SubmitLabel::Go);
			props.accessibility_label = Some("Email".into());
		}),
		Element::text_input().with_props(|props| {
			props.keyboard = Some(Keyboard::Number);
			props.accessibility_label = Some("field".into());
		}),
		Element::text_input().with_props(|props| {
			props.keyboard = Some(Keyboard::Decimal);
			props.accessibility_label = Some("field".into());
		}),
		Element::text_input().with_props(|props| {
			props.keyboard = Some(Keyboard::Phone);
			props.accessibility_label = Some("field".into());
		}),
		Element::text_input().with_props(|props| {
			props.keyboard = Some(Keyboard::Url);
			props.accessibility_label = Some("field".into());
		}),
		Element::text_input().with_props(|props| {
			props.keyboard = Some(Keyboard::Search);
			props.accessibility_label = Some("field".into());
		}),
		Element::text_input().with_props(|props| {
			props.submit_label = Some(SubmitLabel::Send);
			props.accessibility_label = Some("field".into());
		}),
	]);
	let html = render(element).to_html();

	let input = html.split("<input").nth(1).expect("first input");
	assert!(input.contains("type=\"email\""), "{input}");
	assert!(input.contains(" readonly"), "{input}");
	assert!(input.contains("maxlength=\"9\""), "{input}");
	assert!(input.contains("autocapitalize=\"characters\""), "{input}");
	assert!(input.contains("autocorrect=\"off\""), "{input}");
	assert!(input.contains("spellcheck=\"false\""), "{input}");
	assert!(input.contains("enterkeyhint=\"go\""), "{input}");

	// Number and decimal shape the keyboard through inputmode, not type.
	let rest = html
		.split("<input")
		.skip(2)
		.collect::<Vec<_>>()
		.join("<input");
	assert!(rest.contains("inputmode=\"numeric\""), "{rest}");
	assert!(rest.contains("inputmode=\"decimal\""), "{rest}");
	assert!(rest.contains("type=\"tel\""), "{rest}");
	assert!(rest.contains("type=\"url\""), "{rest}");
	assert!(rest.contains("type=\"search\""), "{rest}");
	assert!(rest.contains("enterkeyhint=\"send\""), "{rest}");
}

#[test]
fn emits_deterministic_scoped_styles_below_class_specificity() {
	let element = Element::view()
		.class("p-4 md:p-8")
		.with_style(|style| style.width = Dimension::Points(230.0))
		.background(Color::rgba(1, 2, 3, 255))
		.child(Element::text("Inherited").class("font-semibold"));
	let first = render(element.clone()).to_html();
	let second = render(element).to_html();

	assert_eq!(first, second);
	assert!(first.contains(":where([data-lk-style=\"lk-"));
	assert!(first.contains("width:230px;"));
	assert!(first.contains("background-color:#010203ff;"));
	assert!(first.contains("flex-direction:column;"));
	assert!(first.contains("flex-shrink:0;"));
	assert!(first.contains("box-sizing:border-box;"));
	assert!(first.contains("@media"));

	assert!(first.contains("font-weight:600"));
}

/// A list of rows that each look a little different, as a page's do.
fn varied_rows(count: u16) -> Element {
	(0..count).fold(Element::view().class("p-4"), |list, row| {
		list.child(
			Element::view()
				.with_style(|style| style.width = Dimension::Points(f32::from(row) * 10.0))
				.child(Element::text(format!("Row {row}")))
				.child(Element::button("Open")),
		)
	})
}

#[test]
fn the_style_reset_is_written_once_per_render() {
	let html = render(varied_rows(24)).to_html();
	let reset = "appearance:none;margin:0;border:0 solid transparent;";

	assert_eq!(html.matches(reset).count(), 1, "{html}");
	assert_eq!(html.matches("flex-direction:column;").count(), 1);
	// Each row's sheet holds only its width.
	for row in 1..24 {
		assert_eq!(
			html.matches(&format!("{{width:{}px;}}", row * 10)).count(),
			1,
			"{html}"
		);
	}

	// 24 distinct rows wrote 24 full resets, of about a kilobyte each.
	assert!(html.len() < 9_000, "{} bytes", html.len());
}

#[test]
fn the_reset_applies_under_its_own_render_only() {
	let html = render(varied_rows(2)).to_html();
	let scope = html
		.split("data-lk-root=\"")
		.nth(1)
		.and_then(|rest| rest.split('"').next())
		.expect("the root names its render");

	assert!(scope.starts_with("lk-r"), "{scope}");
	assert!(html.contains(&format!(
		":where([data-lk-root=\"{scope}\"],[data-lk-root=\"{scope}\"] \
		 [data-lk-style]){{appearance:none;"
	)));
	assert_eq!(html.matches(" data-lk-root=").count(), 1, "only the root");
	assert_eq!(html, render(varied_rows(2)).to_html(), "deterministic");
	// A stylesheet later in the page can't reset an earlier render's
	// elements, since its rule names its own root.
	assert!(!render(varied_rows(3)).to_html().contains(scope));
}

#[test]
fn motion_props_become_transforms_transitions_and_keyframes() {
	use leptoskit_native::Transform;
	use leptoskit_native::animation::Curve;
	use leptoskit_native::animation::Keyframes;
	use leptoskit_native::animation::Presence;
	use leptoskit_native::animation::Spring;
	use leptoskit_native::animation::Transition;

	let element = Element::view()
		.transform(Transform::rotate(-3.0))
		.transition(Transition::spring(Spring::SNAPPY))
		.keyframes(Keyframes::spin())
		.entering(Presence::fade().delay(40.0))
		.exiting(Presence::zoom())
		.layout_transition(Curve::DEFAULT);
	let animation = element.props.animation.as_deref().expect("an animation");
	assert_eq!(animation.exiting, Some(Presence::zoom()));
	assert_eq!(animation.layout, Some(Curve::DEFAULT));

	let html = render(element.clone()).to_html();
	assert_eq!(html, render(element).to_html());
	assert!(html.contains("rotate:-3deg;"), "{html}");
	assert!(
		html.contains("transition-property:background-color"),
		"{html}"
	);
	assert!(
		html.contains("transition-timing-function:linear(0,"),
		"{html}"
	);

	// The spin and the entering fade each get a rule named by its content,
	// played together.
	assert!(html.contains("@keyframes lk-k"), "{html}");
	assert!(html.contains("@keyframes lk-e"), "{html}");
	assert!(html.contains("from{opacity:0;}"), "{html}");
	assert!(html.contains("ms linear 0ms infinite,lk-e"), "{html}");
	assert!(html.contains("ms backwards"), "{html}");
	assert!(html.contains(" 40ms backwards"), "{html}");

	// Without an animation only the transform is written.
	let still = render(Element::view().transform(Transform::scale(2.0))).to_html();
	assert!(still.contains("scale:2 2;"), "{still}");
	assert!(!still.contains("animation:"), "{still}");
	assert!(!still.contains("@keyframes"), "{still}");
}

#[test]
fn classes_override_typed_bases_regardless_of_builder_order() {
	let before = Element::text("Hello")
		.font(17.0, FontWeight::Bold)
		.class("text-sm font-medium");
	let after = Element::text("Hello")
		.class("text-sm font-medium")
		.font(17.0, FontWeight::Bold);

	assert_eq!(render(before).to_html(), render(after).to_html());
}

#[test]
fn test_ids_render_as_data_testid_only_where_set() {
	let html = render(
		View()
			.child(Element::button("Save").test_id("save <draft>"))
			.child(Element::text("Unmarked")),
	)
	.to_html();

	assert!(html.contains("data-testid=\"save &lt;draft&gt;\""));
	assert_eq!(html.matches("data-testid").count(), 1);
}

#[test]
#[should_panic(expected = "platform-specific Custom widget `metal-view`")]
fn rejects_custom_native_widgets() {
	let _ = render(Element::new(NativeKind::Custom("metal-view")));
}

#[test]
#[should_panic(expected = "image is a leaf control and cannot contain children")]
fn rejects_children_of_void_controls() {
	let _ = render(Element::image("/image.png").child(Element::text("Invalid child")));
}

/// The server's markup as an app without devtools gets it, which is what
/// the fixtures hold. [`render`] already removes the `DevTools` attributes.
fn as_without_devtools(html: String) -> String {
	assert!(!html.contains(" data-lk-node=\""));
	html
}

/// A page whose server made editor 57. The browser test hydrates it with the
/// page's own editor, which has another id.
#[test]
fn canvist_editor_hydration_fixture_matches_server_rendering() {
	let html = as_without_devtools(
		render(
			Element::view().child(
				Element::canvist_editor(57)
					.accessibility_label("Note")
					.placeholder("Write a note…")
					.class("w-full"),
			),
		)
		.to_html(),
	);

	if std::env::var_os("UPDATE_LEPTOSKIT_WEB_FIXTURE").is_some() {
		std::fs::write(
			concat!(
				env!("CARGO_MANIFEST_DIR"),
				"/tests/fixtures/web_canvist_editor_hydration.html"
			),
			&html,
		)
		.expect("write explicit fixture update");

		return;
	}

	assert_eq!(
		html,
		include_str!("fixtures/web_canvist_editor_hydration.html")
	);
}

#[test]
fn hydration_fixture_matches_server_rendering() {
	let html = as_without_devtools(render(hydration_fixture()).to_html());

	if std::env::var_os("UPDATE_LEPTOSKIT_WEB_FIXTURE").is_some() {
		std::fs::write(
			concat!(
				env!("CARGO_MANIFEST_DIR"),
				"/tests/fixtures/web_hydration.html"
			),
			&html,
		)
		.expect("write explicit fixture update");

		return;
	}

	assert_eq!(html, include_str!("fixtures/web_hydration.html"));
}

fn hydration_fixture() -> Element {
	Element::view()
		.class("p-4")
		.child(Element::text("Hydrated"))
		.child(Element::button("Activate"))
}

#[test]
fn new_controls_render_semantic_state_without_browser_only_radio_names() {
	use leptoskit_native::components::*;
	let html = render(control_fixture()).to_html();

	assert!(html.contains("type=\"checkbox\" role=\"switch\""));
	assert!(html.contains("type=\"radio\""));
	assert!(!html.contains("name=\"lk-radio"));
	assert!(html.contains("type=\"range\""));
	assert!(html.contains("min=\"0\" max=\"10\" step=\"2\" value=\"4\""));
	assert!(html.contains("<progress max=\"1\" value=\"0.4\""));
	assert!(html.contains("role=\"progressbar\" aria-busy=\"true\""));
	assert!(html.contains("type=\"password\""));

	assert!(html.contains("autocomplete=\"new-password\""));
	assert!(html.contains("appearance:auto;"));
	assert!(html.contains("@keyframes lk-spinner"));
	assert_eq!(html, render(control_fixture()).to_html());
	assert!(
		!render(LinearProgressIndicator(Progress::Indeterminate))
			.to_html()
			.contains("<progress max=\"1\" value=")
	);
}

#[test]
fn controls_hydration_fixture_matches_server_rendering() {
	let html = as_without_devtools(render(control_fixture()).to_html());

	if std::env::var_os("UPDATE_LEPTOSKIT_WEB_FIXTURE").is_some() {
		std::fs::write(
			concat!(
				env!("CARGO_MANIFEST_DIR"),
				"/tests/fixtures/web_controls_hydration.html"
			),
			&html,
		)
		.expect("write explicit controls fixture update");

		return;
	}

	assert_eq!(html, include_str!("fixtures/web_controls_hydration.html"));
}

fn control_fixture() -> Element {
	use leptoskit_native::components::*;
	Element::view()
		.child(Switch(true).accessibility_label("Switch"))
		.child(Checkbox(false).accessibility_label("Checkbox"))
		.child(Radio(RadioGroup::new("choice").expect("group"), true).accessibility_label("Radio"))
		.child(
			Slider(SliderState::new(0.0, 10.0, 4.0, Some(2.0)).expect("slider"))
				.accessibility_label("Slider"),
		)
		.child(
			LinearProgressIndicator(Progress::Determinate(
				ProgressValue::new(0.4).expect("progress"),
			))
			.accessibility_label("Progress"),
		)
		.child(ActivityIndicator(true).accessibility_label("Loading"))
		.child(
			PasswordInput()
				.value("secret")
				.autocomplete("new-password")
				.accessibility_label("Password"),
		)
}

#[test]
fn visibility_utilities_override_hidden_without_changing_enabled_state() {
	use leptoskit_native::StyleContext;
	use leptoskit_native::components::Checkbox;
	use leptoskit_yoga::Size;

	for classes in ["flex", "md:flex"] {
		let element = Checkbox(false)
			.with_props(|props| props.hidden = Some(true))
			.class(classes);
		let wide = StyleContext {
			viewport: Size::new(900.0, 400.0),
			..StyleContext::default()
		};
		let resolved = element.resolved(&wide);
		let html = render(element).to_html();
		let input = html.split("<input").nth(1).expect("semantic checkbox");

		assert_eq!(resolved.props.hidden, Some(false));
		assert_ne!(resolved.props.enabled, Some(false));
		assert!(
			html.contains("display:none;"),
			"hidden remains a low-specificity CSS base"
		);
		assert!(html.contains("display:flex"));
		assert!(!input.contains(" disabled"));
		assert!(!input.contains("aria-disabled"));
		assert!(!input.contains("data-lk-hidden"));
	}

	let disabled = Element::view()
		.enabled(false)
		.with_props(|props| props.hidden = Some(true))
		.class("flex")
		.child(Checkbox(false).enabled(true));
	let html = render(disabled).to_html();
	let input = html.split("<input").nth(1).expect("disabled checkbox");
	assert!(input.contains(" disabled"));
	assert!(input.contains("aria-disabled=\"true\""));
}

#[test]
fn radio_group_names_are_valid_before_server_rendering_and_hydration() {
	use leptoskit_native::components::*;

	for invalid in ["a\0b", "a\rb", "a\nb", "a\r\nb", "a\tb"] {
		assert!(RadioGroup::new(invalid).is_err());
	}

	let html = as_without_devtools(
		render(View().children([
			Radio(RadioGroup::new("a�b").unwrap(), true).accessibility_label("Replacement"),
			Radio(RadioGroup::new("aéb").unwrap(), true).accessibility_label("Unicode"),
		]))
		.to_html(),
	);
	assert!(html.contains("data-lk-radio-group=\"a�b\""));
	assert!(html.contains("data-lk-radio-group=\"aéb\""));

	if std::env::var_os("UPDATE_LEPTOSKIT_WEB_FIXTURE").is_some() {
		std::fs::write(
			concat!(
				env!("CARGO_MANIFEST_DIR"),
				"/tests/fixtures/web_radio_unicode_hydration.html"
			),
			&html,
		)
		.unwrap();

		return;
	}

	assert_eq!(
		html,
		include_str!("fixtures/web_radio_unicode_hydration.html")
	);
}

#[test]
fn pressables_with_an_href_render_as_links_and_disabled_links_lose_their_target() {
	let html = render(
		View().children([
			Pressable("Open note").href("/notes/42").child(Text("Open")),
			Pressable("Closed")
				.href("/closed")
				.enabled(false)
				.child(Text("Closed")),
		]),
	)
	.to_html();
	assert!(html.contains("<a href=\"/notes/42\""), "{html}");
	assert!(html.contains("aria-label=\"Open note\""), "{html}");
	assert!(!html.contains("href=\"/closed\""), "{html}");
	assert!(html.contains("aria-disabled=\"true\""), "{html}");
	assert!(!html.contains("<button"), "{html}");
}

#[test]
fn a_new_tab_link_is_isolated_and_announced() {
	use leptoskit_native::LinkTarget;

	let html = render(
		Pressable("Open the admin")
			.href("https://admin.example.com/")
			.target(LinkTarget::NewTab)
			.child(Text("Open the admin")),
	)
	.to_html();
	assert!(html.contains("target=\"_blank\""), "{html}");
	assert!(html.contains("rel=\"noopener noreferrer\""), "{html}");
	assert!(
		html.contains("aria-description=\"Opens in a new tab\""),
		"{html}"
	);

	// The app's own description is kept, and the hint follows it.
	let html = render(
		Pressable("Status")
			.href("https://status.example.com/")
			.target(LinkTarget::NewTab)
			.with_props(|props| {
				props.aria_mut().description = Some("Live service status.".to_owned());
			})
			.child(Text("Status")),
	)
	.to_html();
	assert!(
		html.contains("aria-description=\"Live service status. Opens in a new tab\""),
		"{html}"
	);

	// An ordinary link, a disabled one, and a button say nothing of tabs.
	let html = render(
		View().children([
			Pressable("Here").href("/here").child(Text("Here")),
			Pressable("Closed")
				.href("https://example.com/")
				.target(LinkTarget::NewTab)
				.enabled(false)
				.child(Text("Closed")),
			Pressable("Button")
				.target(LinkTarget::NewTab)
				.child(Text("Button")),
		]),
	)
	.to_html();
	assert!(!html.contains("target="), "{html}");
	assert!(!html.contains("rel="), "{html}");
	assert!(html.contains("<button"), "{html}");
}

#[test]
fn a_download_link_names_its_file() {
	let html = render(
		View().children([
			Pressable("Export")
				.href("/people/7/export")
				.download("person-7.json")
				.child(Text("Export")),
			Pressable("Unsafe")
				.href("/unsafe")
				.download("../etc/passwd")
				.child(Text("Unsafe")),
			Pressable("Closed")
				.href("/closed")
				.download("closed.json")
				.enabled(false)
				.child(Text("Closed")),
		]),
	)
	.to_html();
	assert!(
		html.contains("href=\"/people/7/export\"") && html.contains("download=\"person-7.json\""),
		"{html}"
	);
	assert_eq!(html.matches("download=").count(), 1, "{html}");
	assert!(html.contains("href=\"/unsafe\""), "{html}");
}

#[test]
fn styled_runs_render_as_inline_spans_in_a_block() {
	let element = Text("").spans([
		leptoskit_native::TextSpan::new("let ").italic(),
		leptoskit_native::TextSpan::new("x")
			.color(Color::rgba(0xe0, 0x6c, 0x75, 0xff))
			.background(Color::rgba(0, 0, 0, 0xff))
			.bold(),
		leptoskit_native::TextSpan::new(" = 1"),
	]);
	assert_eq!(element.props.text.as_deref(), Some("let x = 1"));
	let html = render(element).to_html();

	assert!(html.contains("display:block;"), "{html}");
	assert!(
		html.contains("<span style=\"font-style:italic;\">let </span>"),
		"{html}"
	);
	assert!(
		html.contains(
			"<span style=\"color:#e06c75ff;background-color:#000000ff;font-weight:700;\">x</span>"
		),
		"{html}"
	);
	assert!(html.contains("<span> = 1</span>"), "{html}");
}

#[test]
fn typed_weights_and_line_heights_render_as_css() {
	let relative = render(
		Element::text("Loose")
			.font(15.0, FontWeight::Black)
			.line_height(leptoskit_native::LineHeight::Relative(1.5)),
	)
	.to_html();
	assert!(relative.contains("font-weight:900;"), "{relative}");
	assert!(relative.contains("line-height:1.5;"), "{relative}");

	let fixed = render(
		Element::text("Fixed")
			.font(15.0, FontWeight::Thin)
			.line_height(leptoskit_native::LineHeight::Fixed(22.0)),
	)
	.to_html();
	assert!(fixed.contains("font-weight:100;"), "{fixed}");
	assert!(fixed.contains("line-height:22px;"), "{fixed}");

	let span = render(
		Text("").spans([leptoskit_native::TextSpan::new("light").weight(FontWeight::Light)]),
	)
	.to_html();
	assert!(
		span.contains("<span style=\"font-weight:300;\">light</span>"),
		"{span}"
	);
}

#[test]
fn italic_and_per_edge_border_props_render_as_css() {
	let element = View().with_props(|props| {
		props.italic = Some(true);
		props.border_widths = Some(leptoskit_native::BorderWidths {
			left: 2.0,
			bottom: 1.0,
			..leptoskit_native::BorderWidths::default()
		});
	});
	let html = render(element).to_html();

	assert!(html.contains("font-style:italic;"), "{html}");
	assert!(
		html.contains(
			"box-shadow:inset 0px -1px 0 0 var(--lk-border-color),inset 2px 0px 0 0 \
			 var(--lk-border-color);"
		),
		"{html}"
	);
}

#[test]
fn vectors_render_inline_svg_with_non_scaling_strokes() {
	use leptoskit_native::PathData;
	use leptoskit_native::Stroke;
	use leptoskit_native::VectorPath;
	use leptoskit_native::VectorScaling;
	use leptoskit_native::ViewBox;
	use leptoskit_native::components::Vector;

	let ink = Color::rgba(0x11, 0x22, 0x33, 0xff);
	let html = render(
		View()
			.child(
				Vector(ViewBox::new(0.0, 0.0, 24.0, 24.0).expect("a valid view box"))
					.class("w-6 h-6")
					.aria_label("Done")
					.path(
						VectorPath::new(PathData::parse("m4 13 l5 5 L20 6").expect("valid data"))
							.stroke(Stroke::new(ink, 2.0).expect("a positive width").round()),
					),
			)
			.child(
				Vector(ViewBox::UNIT)
					.scaling(VectorScaling::Stretch)
					.class("absolute inset-0")
					.path(
						VectorPath::new(PathData::parse("M0 0 H1 V1 Z").expect("valid data"))
							.fill(ink),
					),
			),
	)
	.to_html();

	assert!(html.contains("role=\"img\""), "{html}");
	assert!(html.contains("aria-label=\"Done\""));
	assert!(html.contains("viewBox=\"0 0 24 24\""));
	assert!(html.contains("preserveAspectRatio=\"xMidYMid meet\""));
	assert!(html.contains("d=\"M 4 13 L 9 18 L 20 6\""));
	assert!(html.contains("fill=\"none\""));
	assert!(html.contains("stroke=\"#112233ff\""));
	assert!(html.contains("stroke-width=\"2\""));

	assert!(html.contains("stroke-linecap=\"round\""));
	assert!(html.contains("stroke-linejoin=\"round\""));
	assert!(html.contains("stroke-miterlimit=\"4\""));
	assert!(html.contains("vector-effect=\"non-scaling-stroke\""));
	// The unnamed, stretched drawing is decoration.
	assert!(html.contains("preserveAspectRatio=\"none\""));
	assert!(html.contains("d=\"M 0 0 L 1 0 L 1 1 Z\" fill=\"#112233ff\""));
	assert_eq!(html.matches("aria-hidden=\"true\"").count(), 3, "{html}");
	assert!(
		html.contains(">svg{position:absolute;inset:0;width:100%;height:100%;overflow:visible;")
	);

	let styled = |stroke: Stroke| {
		render(Vector(ViewBox::UNIT).path(
			VectorPath::new(PathData::parse("M0 0 L1 1").expect("valid data")).stroke(stroke),
		))
		.to_html()
	};
	let square = styled(
		Stroke::new(ink, 1.0)
			.expect("a positive width")
			.cap(leptoskit_native::LineCap::Square)
			.join(leptoskit_native::LineJoin::Bevel),
	);
	assert!(square.contains("stroke-linecap=\"square\""));
	assert!(square.contains("stroke-linejoin=\"bevel\""));
	let plain = styled(Stroke::new(ink, 1.0).expect("a positive width"));
	assert!(plain.contains("stroke-linecap=\"butt\""));
	assert!(plain.contains("stroke-linejoin=\"miter\""));
}

#[test]
fn pressable_href_policy_applies_to_server_rendering() {
	for href in href_targets::VALID_HREFS {
		let html = render(Pressable("Open").href(*href).child(Text("Open"))).to_html();
		assert!(html.contains("<a href="), "{href:?}: {html}");
	}

	for href in href_targets::INVALID_HREFS {
		let html = render(Pressable("Open").href(*href).child(Text("Open"))).to_html();
		assert!(!html.contains("href="), "{href:?}: {html}");
		assert!(html.contains("<button"), "{href:?}: {html}");
		assert!(html.contains("aria-label=\"Open\""), "{href:?}: {html}");
	}
}

#[test]
fn held_controls_style_through_css_active_and_dim_without_active_utilities() {
	let html =
		render(
			View().children([
				Element::button("Add one")
					.class("rounded-lg bg-indigo-500 px-6 py-3 active:bg-indigo-700"),
				Element::button("Plain"),
				Pressable("Card").child(Text("Card content")),
			]),
		)
		.to_html();

	// `active:` compiles to `:active`, scoped like every other utility, and
	// never matches a disabled link or button.
	assert!(html.contains(
		".active\\3a bg-indigo-700:active:not([aria-disabled=true]){background-color:#432dd7ff;}"
	));
	// Controls without `active:` utilities dim to the native default.
	assert_eq!(
		html.matches(":active:not([aria-disabled=true])){filter:opacity(0.7);}")
			.count(),
		2,
		"{html}"
	);
	// The pressed style changes neither the element nor its accessible name.
	assert!(html.contains("aria-label=\"Card\""));
	assert_eq!(html.matches("<button").count(), 3);
}

#[test]
fn focused_and_scrolled_nodes_serialize_their_renderer_directives() {
	use leptoskit_native::ScrollOffset;

	let page = render(View().children([
		Element::scroll_view().scroll_offset(ScrollOffset::new(1.0, 40.0).animated()),
		Element::text_input().aria_label("Search").focused(true),
	]))
	.to_html();

	assert!(page.contains("data-lk-scroll=\"1 40 smooth\""), "{page}");
	assert!(page.contains("data-lk-focus=\"true\""), "{page}");
}

#[test]
fn a_focus_request_is_written_where_the_browser_can_carry_it_out() {
	use leptoskit_native::FocusRequest;

	let page = reactive_graph::owner::Owner::new().with(|| {
		let to_code = FocusRequest::new();
		let before = render(TextInput().aria_label("Code").focus_on(to_code)).to_html();
		assert!(!before.contains("data-lk-focus-request"), "{before}");

		to_code.request();
		render(View().children([
			TextInput().aria_label("Code").focus_on(to_code),
			Text("Nothing to focus here").focus_on(to_code),
		]))
		.to_html()
	});

	assert_eq!(
		page.matches("data-lk-focus-request=\"").count(),
		2,
		"{page}"
	);
	assert!(!page.contains("data-lk-focus=\"true\""), "{page}");
}

#[test]
fn a_reveal_is_written_where_the_browser_can_carry_it_out() {
	use leptoskit_native::Reveal;
	use leptoskit_native::ScrollAlign;

	let page = reactive_graph::owner::Owner::new().with(|| {
		let to_notice = Reveal::new();
		let before = render(Text("Saved.").reveal_on(to_notice, ScrollAlign::Center)).to_html();
		assert!(!before.contains("data-lk-reveal"), "{before}");

		to_notice.request();
		render(Text("Saved.").reveal_on(to_notice, ScrollAlign::Center)).to_html()
	});
	let (_, request) = page
		.split_once("data-lk-reveal=\"")
		.unwrap_or_else(|| panic!("{page}"));
	let (request, _) = request.split_once('"').unwrap();
	let (serial, align) = request.split_once(' ').unwrap();

	assert!(serial.parse::<u64>().unwrap() > 0, "{page}");
	assert_eq!(align, "center");
}

#[test]
fn a_modal_is_a_dialog_and_a_popover_carries_the_popover_attribute() {
	use leptoskit_native::Placement;
	use leptoskit_native::components::Button;
	use leptoskit_native::components::Modal;
	use leptoskit_native::components::Popover;

	let html = render(
		View()
			.child(Button("Account").id("me"))
			.child(
				Popover("me", false)
					.placement(Placement::BelowEnd)
					.child(Button("Sign out")),
			)
			.child(
				Modal(true)
					.aria_label("Delete the note?")
					.initial_focus("keep")
					.child(Button("Keep it").id("keep")),
			),
	)
	.to_html();
	let tag = |name: &str| {
		let start = html
			.find(&format!("<{name} "))
			.unwrap_or_else(|| panic!("{html}"));
		let end = start + html[start..].find('>').expect("the tag closes");

		&html[start..end]
	};

	// A server writes a modal that starts open as an open dialog, so it is
	// on screen before any script runs. The element implies its role.
	let dialog = tag("dialog");
	assert!(dialog.contains(r#"data-lk-modal="open""#), "{dialog}");
	assert!(dialog.contains(" open"), "{dialog}");
	assert!(
		dialog.contains(r#"data-lk-initial-focus="keep""#),
		"{dialog}"
	);
	assert!(
		dialog.contains(r#"aria-label="Delete the note?""#),
		"{dialog}"
	);
	assert!(!dialog.contains("role="), "{dialog}");

	let popover = &html[html.find("popover=").expect("a popover")..];
	let popover = &popover[..popover.find('>').expect("the tag closes")];
	assert!(popover.contains(r#"popover="auto""#), "{popover}");
	assert!(popover.contains(r#"data-lk-popover="closed""#), "{popover}");
	assert!(popover.contains(r#"data-lk-anchor="me""#), "{popover}");
	assert!(
		popover.contains(r#"data-lk-placement="below_end""#),
		"{popover}"
	);

	let closed = render(
		Modal(false)
			.aria_label("Delete the note?")
			.child(Text("Sure?")),
	)
	.to_html();
	assert!(closed.contains(r#"data-lk-modal="closed""#), "{closed}");
	assert!(!closed.contains(" open"), "{closed}");
	// A closed dialog shows nothing even though every element is a flex box.
	assert!(
		closed.contains(":not([open]){display:none!important;}"),
		"{closed}"
	);
}
