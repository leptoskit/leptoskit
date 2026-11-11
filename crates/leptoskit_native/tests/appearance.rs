//! An app's own light or dark choice: the stylesheet the web reads it from,
//! and the native surface that resolves `dark:` against it.

use leptoskit_native::Appearance;
use leptoskit_native::Color;
use leptoskit_native::Element;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeBackend;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeProps;
use leptoskit_native::NativeSurface;
use leptoskit_native::SharedTextMeasurer;
use leptoskit_native::Tailwind;
use leptoskit_native::appearance::ATTRIBUTE;
use leptoskit_native::surface::SourcePatch;
use leptoskit_yoga::NodeId;

const BLACK: Option<Color> = Some(Color::rgba(0, 0, 0, 255));
const WHITE: Option<Color> = Some(Color::rgba(255, 255, 255, 255));

#[test]
fn dark_rules_follow_the_device_unless_the_root_picks_light_and_whenever_it_picks_dark() {
	let css = Tailwind::parse("dark:bg-black").unwrap().css();

	assert_eq!(
		css,
		"@media (prefers-color-scheme:dark){:where(:root:not([data-lk-appearance=light])) \
		 .dark\\3a bg-black{background-color:#000000ff;}}:where(:root[data-lk-appearance=dark]) \
		 .dark\\3a bg-black{background-color:#000000ff;}"
	);
	assert!(css.contains(&format!("[{ATTRIBUTE}=light]")));
	assert!(css.contains(&format!("[{ATTRIBUTE}=dark]")));
}

#[test]
fn light_rules_and_other_conditions_are_unchanged() {
	assert_eq!(
		Tailwind::parse("bg-white").unwrap().css(),
		".bg-white{background-color:#ffffffff;}"
	);

	let css = Tailwind::parse("md:dark:p-4")
		.unwrap()
		.css_scoped("[data-lk-style=\"x\"]");
	assert_eq!(
		css,
		"@media (prefers-color-scheme:dark){@media \
		 (width>=48rem){:where(:root:not([data-lk-appearance=light])) \
		 :where([data-lk-style=\"x\"]).md\\3a dark\\3a \
		 p-4{padding-top:1rem;padding-right:1rem;padding-bottom:1rem;padding-left:1rem;}}}@media \
		 (width>=48rem){:where(:root[data-lk-appearance=dark]) \
		 :where([data-lk-style=\"x\"]).md\\3a dark\\3a \
		 p-4{padding-top:1rem;padding-right:1rem;padding-bottom:1rem;padding-left:1rem;}}"
	);

	let starting = Tailwind::parse("dark:starting:opacity-0").unwrap().css();
	assert!(
		starting.contains(
			"@media (prefers-color-scheme:dark){@starting-style{:where(:root:\
			 not([data-lk-appearance=light]))"
		),
		"{starting}"
	);
	assert!(
		starting.contains("@starting-style{:where(:root[data-lk-appearance=dark])"),
		"{starting}"
	);
}

/// A backend that remembers the appearance it was last told.
#[derive(Default)]
struct Appearing {
	inner: HeadlessBackend,
	dark: Option<bool>,
}

impl NativeBackend for Appearing {
	fn create(&mut self, id: NodeId, kind: NativeKind, props: &NativeProps) {
		self.inner.create(id, kind, props);
	}

	fn update_props(&mut self, id: NodeId, props: &NativeProps) {
		self.inner.update_props(id, props);
	}

	fn update_layout(&mut self, id: NodeId, x: f32, y: f32, width: f32, height: f32) {
		self.inner.update_layout(id, x, y, width, height);
	}

	fn insert_child(&mut self, parent: NodeId, child: NodeId, index: usize) {
		self.inner.insert_child(parent, child, index);
	}

	fn remove_child(&mut self, parent: NodeId, child: NodeId) {
		self.inner.remove_child(parent, child);
	}

	fn delete(&mut self, id: NodeId) {
		self.inner.delete(id);
	}

	fn text_measurer(&self) -> SharedTextMeasurer {
		self.inner.text_measurer()
	}

	fn set_dark_mode(&mut self, dark: bool) {
		self.dark = Some(dark);
	}
}

fn card(appearance: Option<Appearance>) -> Element {
	Element::view()
		.class("bg-white dark:bg-black")
		.with_props(|props| props.appearance = appearance)
}

fn background(surface: &NativeSurface<Appearing>) -> Option<Color> {
	surface.props(surface.root().unwrap()).background
}

#[test]
fn the_surface_s_choice_overrides_the_device_and_tells_the_backend() {
	let mut surface = NativeSurface::new(Appearing::default());
	surface.set_root(&card(None));
	assert_eq!(surface.appearance(), Appearance::System);
	assert_eq!(background(&surface), WHITE);

	surface.set_appearance(Appearance::Dark);
	assert!(surface.is_dark());
	assert_eq!(surface.backend().dark, Some(true));
	assert_eq!(background(&surface), BLACK);

	// The device going light or dark changes nothing while the app chose.
	surface.set_dark_mode(false);
	assert_eq!(background(&surface), BLACK);
	assert_eq!(surface.backend().dark, Some(true));

	surface.set_appearance(Appearance::Light);
	surface.set_dark_mode(true);
	assert!(!surface.is_dark());
	assert_eq!(surface.backend().dark, Some(false));
	assert_eq!(background(&surface), WHITE);

	// Following the device again takes its current setting.
	surface.set_appearance(Appearance::System);
	assert!(surface.is_dark());
	assert_eq!(surface.backend().dark, Some(true));
	assert_eq!(background(&surface), BLACK);
	assert_eq!(surface.appearance(), Appearance::System);
}

#[test]
fn the_root_carries_a_guest_s_choice() {
	let mut surface = NativeSurface::new(Appearing::default());
	surface.set_dark_mode(false);
	surface.set_root(&card(Some(Appearance::Dark)));
	assert_eq!(surface.appearance(), Appearance::Dark);
	assert_eq!(surface.backend().dark, Some(true));
	assert_eq!(background(&surface), BLACK);

	// A root without a choice keeps the last one.
	surface.set_root(&card(None));
	assert_eq!(surface.appearance(), Appearance::Dark);
	assert_eq!(background(&surface), BLACK);

	surface.set_root(&card(Some(Appearance::System)));
	assert_eq!(surface.backend().dark, Some(false));
	assert_eq!(background(&surface), WHITE);

	// Choosing what is already shown commits nothing more.
	surface.set_root(&card(Some(Appearance::Light)));
	assert_eq!(surface.appearance(), Appearance::Light);
	assert_eq!(background(&surface), WHITE);
}

#[test]
fn a_patched_root_s_choice_resolves_the_whole_tree_again() {
	let mut surface = NativeSurface::new(Appearing::default());
	let tree = |appearance| {
		card(appearance).child(Element::text("child").class("text-white dark:text-black"))
	};
	surface.set_root(&tree(None));
	let root = surface.root().unwrap();
	let child = surface.children(root)[0];
	assert_eq!(surface.props(child).color, WHITE);

	let patched = surface
		.try_patch(vec![SourcePatch::Node {
			node: root,
			element: card(Some(Appearance::Dark)),
		}])
		.unwrap();
	assert!(patched.is_some());
	assert_eq!(surface.backend().dark, Some(true));
	let root = surface.root().unwrap();
	assert_eq!(background(&surface), BLACK);
	assert_eq!(surface.props(surface.children(root)[0]).color, BLACK);

	// A patch below the root leaves the choice alone.
	let child = surface.children(root)[0];
	surface
		.try_patch(vec![SourcePatch::Node {
			node: child,
			element: Element::text("again").class("text-white dark:text-black"),
		}])
		.unwrap();
	assert_eq!(surface.appearance(), Appearance::Dark);
	assert_eq!(surface.props(surface.children(root)[0]).color, BLACK);
}

#[cfg(feature = "serde")]
#[test]
fn the_choice_crosses_the_wire_only_when_made() {
	let props = NativeProps {
		appearance: Some(Appearance::Dark),
		..NativeProps::default()
	};
	let json = serde_json::to_value(&props).unwrap();
	assert_eq!(json["appearance"], "dark");
	assert_eq!(serde_json::from_value::<NativeProps>(json).unwrap(), props);

	let unset = serde_json::to_value(NativeProps::default()).unwrap();
	assert!(unset.get("appearance").is_none());
}
