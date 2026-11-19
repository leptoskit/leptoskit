//! Main-thread integration tests of sheets and popovers on real windows.
#![allow(unsafe_code, clippy::float_cmp)]

fn main() {
	let mut args = libtest_mimic::Arguments::from_args();
	args.test_threads = Some(1);
	#[cfg(target_os = "macos")]
	let trials = macos::trials();
	#[cfg(not(target_os = "macos"))]
	let trials = Vec::new();
	libtest_mimic::run(&args, trials).exit();
}

#[cfg(target_os = "macos")]
mod macos {
	use std::cell::RefCell;
	use std::rc::Rc;

	use leptoskit_appkit::AppKitBackend;
	use leptoskit_appkit::MainThreadMarker;
	use leptoskit_native::Element;
	use leptoskit_native::NativeEvent;
	use leptoskit_native::NativeSurface;
	use leptoskit_native::Placement;
	use leptoskit_native::components::*;
	use leptoskit_yoga::NodeId;
	use libtest_mimic::Trial;
	use objc2_app_kit::NSApplication;
	use objc2_app_kit::NSTextField;
	use objc2_foundation::NSSize;

	pub fn trials() -> Vec<Trial> {
		[
			(
				"a_modal_shows_as_a_sheet_sized_to_what_it_holds",
				a_modal_shows_as_a_sheet_sized_to_what_it_holds as fn(),
			),
			(
				"a_popover_hangs_below_its_anchor_and_asks_before_it_closes",
				a_popover_hangs_below_its_anchor_and_asks_before_it_closes,
			),
			(
				"a_popover_that_starts_open_shows_with_its_window_on_the_side_asked_for",
				a_popover_that_starts_open_shows_with_its_window_on_the_side_asked_for,
			),
			(
				"overlay_views_stay_out_of_their_parent",
				overlay_views_stay_out_of_their_parent,
			),
		]
		.into_iter()
		.map(|(name, test)| {
			Trial::test(name, move || {
				test();
				Ok(())
			})
		})
		.collect()
	}

	fn mounted(root: &Element) -> NativeSurface<AppKitBackend> {
		let mtm = MainThreadMarker::new().expect("the harness runs on the main thread");
		NSApplication::sharedApplication(mtm);
		let mut surface = NativeSurface::new(AppKitBackend::new(mtm));
		surface.set_viewport(400.0, 300.0);
		surface.set_root(root);
		surface
	}

	fn child(surface: &NativeSurface<AppKitBackend>, index: usize) -> NodeId {
		surface.children(surface.root().unwrap())[index]
	}

	/// Records what the backend's widgets ask for.
	fn listen(surface: &NativeSurface<AppKitBackend>) -> Rc<RefCell<Vec<(NodeId, NativeEvent)>>> {
		let events = Rc::new(RefCell::new(Vec::new()));
		surface.backend().set_event_listener({
			let events = Rc::clone(&events);
			move |id, event| events.borrow_mut().push((id, event.clone()))
		});
		events
	}

	/// A page with a confirmation: a 256 by 128 panel holding one field.
	fn confirmation(open: bool) -> Element {
		View()
			.class("w-full h-full")
			.child(Button("Delete").id("delete"))
			.child(
				Modal(open)
					.aria_label("Delete the note?")
					.class("bg-black")
					.initial_focus("reason")
					.child(
						View().class("w-64 h-32").child(
							TextInput()
								.id("reason")
								.aria_label("Reason")
								.class("w-48 h-8"),
						),
					),
			)
	}

	fn a_modal_shows_as_a_sheet_sized_to_what_it_holds() {
		let mut surface = mounted(&confirmation(true));
		let modal = child(&surface, 1);
		let field = surface.children(surface.children(modal)[0])[0];

		assert_eq!(
			surface.backend().overlay_shown(modal),
			Some(false),
			"with no window there is nothing to show a sheet on"
		);
		assert_eq!(surface.backend().overlay_shown(child(&surface, 0)), None);

		let window = surface
			.backend()
			.open_window(surface.root().unwrap(), "Sheet test")
			.unwrap();
		let events = listen(&surface);
		let sheet = surface.backend().overlay_window(modal).unwrap();

		assert_eq!(surface.backend().overlay_shown(modal), Some(true));
		assert_eq!(window.attachedSheet().as_deref(), Some(&*sheet));
		// The modal covers the viewport and centres its panel. The sheet is
		// the panel alone, and the modal's backdrop is not drawn.
		assert_eq!(
			sheet.contentView().unwrap().frame().size,
			NSSize::new(256.0, 128.0)
		);
		let modal_view = surface.backend().view(modal).unwrap();
		assert_eq!(modal_view.frame().size, NSSize::new(400.0, 300.0));
		assert_eq!(modal_view.frame().origin.x, -72.0);
		assert_eq!(modal_view.frame().origin.y, -86.0);
		assert!(modal_view.layer().unwrap().backgroundColor().is_none());
		// The field named by `initial_focus` is being edited in the sheet.
		let native = surface
			.backend()
			.view(field)
			.unwrap()
			.downcast_ref::<NSTextField>()
			.expect("a text field");
		assert_eq!(native.window().as_deref(), Some(&*sheet));
		assert!(native.currentEditor().is_some());

		// Escape asks, and the sheet stays until the app answers.
		assert!(surface.backend().perform_dismiss(modal));
		assert_eq!(*events.borrow(), vec![(modal, NativeEvent::Dismiss)]);
		assert_eq!(surface.backend().overlay_shown(modal), Some(true));

		surface.set_root(&confirmation(false));
		assert_eq!(surface.backend().overlay_shown(modal), Some(false));
		assert!(!surface.backend().perform_dismiss(modal));

		surface.set_root(&confirmation(true));
		assert_eq!(surface.backend().overlay_shown(modal), Some(true));

		// Removing an open modal takes its sheet down with it.
		surface.set_root(&View().class("w-full h-full"));
		assert_eq!(surface.children(surface.root().unwrap()).len(), 0);
		window.close();
	}

	/// A page with a menu hanging from a button.
	fn menu(open: bool, placement: Placement) -> Element {
		View()
			.class("w-full h-full items-center justify-center")
			.child(Button("Account").id("me"))
			.child(
				Popover("me", open).placement(placement).child(
					View()
						.class("w-40 h-20")
						.child(Button("Sign out").id("out")),
				),
			)
	}

	fn a_popover_hangs_below_its_anchor_and_asks_before_it_closes() {
		let mut surface = mounted(&menu(false, Placement::BelowStart));
		let anchor = child(&surface, 0);
		let popover = child(&surface, 1);
		let window = surface
			.backend()
			.open_window(surface.root().unwrap(), "Popover test")
			.unwrap();
		let events = listen(&surface);

		assert_eq!(surface.backend().overlay_shown(popover), Some(false));
		assert!(surface.backend().overlay_window(popover).is_none());

		surface.set_root(&menu(true, Placement::BelowStart));
		assert_eq!(surface.backend().overlay_shown(popover), Some(true));
		let content = surface.backend().view(popover).unwrap();
		assert_eq!(content.frame().size, NSSize::new(160.0, 80.0));

		// In screen coordinates y grows upward, so below is smaller.
		let middle = |rect: objc2_foundation::NSRect| rect.origin.y + rect.size.height / 2.0;
		let anchor_view = surface.backend().view(anchor).unwrap();
		let anchor_on_screen =
			window.convertRectToScreen(anchor_view.convertRect_toView(anchor_view.bounds(), None));
		let shown = surface.backend().overlay_window(popover).unwrap();
		assert!(
			middle(shown.frame()) < middle(anchor_on_screen),
			"{:?} is below {anchor_on_screen:?}",
			shown.frame()
		);

		// Escape and a press outside ask, and the popover stays until the
		// app answers.
		events.borrow_mut().clear();
		assert!(surface.backend().perform_dismiss(popover));
		assert_eq!(*events.borrow(), vec![(popover, NativeEvent::Dismiss)]);
		assert_eq!(surface.backend().overlay_shown(popover), Some(true));

		surface.set_root(&menu(false, Placement::BelowStart));
		assert_eq!(surface.backend().overlay_shown(popover), Some(false));
		assert!(!surface.backend().perform_dismiss(popover));
		window.close();
	}

	fn a_popover_that_starts_open_shows_with_its_window_on_the_side_asked_for() {
		let mut surface = mounted(&menu(true, Placement::Above));
		let anchor = child(&surface, 0);
		let popover = child(&surface, 1);

		assert_eq!(
			surface.backend().overlay_shown(popover),
			Some(false),
			"an anchor in no window has nowhere to hang a popover"
		);

		let window = surface
			.backend()
			.open_window(surface.root().unwrap(), "Open popover test")
			.unwrap();
		assert_eq!(surface.backend().overlay_shown(popover), Some(true));

		let middle = |rect: objc2_foundation::NSRect| rect.origin.y + rect.size.height / 2.0;
		let anchor_view = surface.backend().view(anchor).unwrap();
		let anchor_on_screen =
			window.convertRectToScreen(anchor_view.convertRect_toView(anchor_view.bounds(), None));
		let shown = surface.backend().overlay_window(popover).unwrap();
		assert!(
			middle(shown.frame()) > middle(anchor_on_screen),
			"{:?} is above {anchor_on_screen:?}",
			shown.frame()
		);

		// Removing an open popover closes it.
		surface.set_root(&View());
		window.close();
	}

	fn overlay_views_stay_out_of_their_parent() {
		let page = |extra: bool| {
			View()
				.class("w-full h-full")
				.child(Text("First"))
				.children(extra.then(|| Text("Added")))
				.child(Modal(false).aria_label("Closed").child(Text("Inside")))
				.child(Popover("last", false).child(Text("Menu")))
				.child(Text("Last").id("last"))
		};
		let mut surface = mounted(&page(false));
		let root = surface.root().unwrap();
		let subviews = |surface: &NativeSurface<AppKitBackend>| {
			surface.backend().view(root).unwrap().subviews().len()
		};

		assert_eq!(subviews(&surface), 2);

		// A view inserted before an overlay goes below the next view that is
		// really in the parent.
		surface.set_root(&page(true));
		assert_eq!(subviews(&surface), 3);
		let views = surface.backend().view(root).unwrap().subviews();
		let added = surface.backend().view(child(&surface, 1)).unwrap();
		assert_eq!(&*views.objectAtIndex(1), added);

		surface.set_root(&page(false));
		assert_eq!(subviews(&surface), 2);
	}
}
