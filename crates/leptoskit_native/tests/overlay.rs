//! Modals and popovers in the shared surface: where they are laid out, what
//! a press reaches, and who hears a dismissal.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_native::ControlTreeError;
use leptoskit_native::Element;
use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::Mutation;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_native::NativeSurface;
use leptoskit_native::Placement;
use leptoskit_native::components::Button;
use leptoskit_native::components::Modal;
use leptoskit_native::components::Popover;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_native::surface::Frame;
use leptoskit_yoga::NodeId;

fn surface(screen: &Element) -> NativeSurface<HeadlessBackend> {
	let mut surface = NativeSurface::new(HeadlessBackend::new().presenting_overlays());
	surface.set_viewport(400.0, 300.0);
	surface.set_root(screen);
	surface
}

/// The first node of a kind, in document order.
fn first(surface: &NativeSurface<HeadlessBackend>, kind: NativeKind) -> NodeId {
	let mut pending = vec![surface.root().expect("a committed root")];

	while let Some(id) = pending.pop() {
		if surface.kind(id) == kind {
			return id;
		}

		pending.extend(surface.children(id).iter().rev());
	}

	panic!("no {kind:?} in the tree");
}

fn confirm(open: bool) -> Element {
	View()
		.class("p-10")
		.child(Button("Delete").id("delete"))
		.child(
			View().class("p-6").child(
				Modal(open)
					.aria_label("Delete the note?")
					.child(View().class("w-40 h-20").child(Button("Keep it"))),
			),
		)
}

#[test]
fn a_closed_modal_takes_no_room_and_is_never_hit() {
	let surface = surface(&confirm(false));
	let modal = first(&surface, NativeKind::Modal);

	assert_eq!(surface.layout(modal).width, 0.0);
	assert_eq!(surface.open_overlays(), []);
	// The page's own button is where it would be without the modal.
	assert_eq!(
		surface.hit_test(50.0, 50.0).map(|id| surface.kind(id)),
		Some(NativeKind::Button)
	);
}

#[test]
fn an_open_modal_covers_the_viewport_wherever_it_sits_in_the_tree() {
	let surface = surface(&confirm(true));
	let modal = first(&surface, NativeKind::Modal);

	assert_eq!(surface.open_overlays(), [modal]);
	// Its parents are padded 40 and 24 in from the corner; its frame is in
	// viewport coordinates all the same.
	assert_eq!(
		surface.frame(modal),
		Some(Frame {
			x: 0.0,
			y: 0.0,
			width: 400.0,
			height: 300.0,
		})
	);

	// What it holds sits in the middle.
	let panel = surface.children(modal)[0];
	assert_eq!(
		surface.frame(panel),
		Some(Frame {
			x: 120.0,
			y: 110.0,
			width: 160.0,
			height: 80.0,
		})
	);
}

#[test]
fn nothing_behind_an_open_modal_is_hit() {
	let surface = surface(&confirm(true));
	let modal = first(&surface, NativeKind::Modal);

	// Where the page's Delete button is: the press lands on the backdrop.
	assert_eq!(surface.hit_test(50.0, 50.0), Some(modal));
	// Inside the panel, the modal's own button.
	let hit = surface.hit_test(130.0, 120.0).expect("the panel is hit");
	assert_eq!(surface.kind(hit), NativeKind::Button);
	assert_eq!(surface.props(hit).text.as_deref(), Some("Keep it"));
}

fn menu(open: bool, placement: Placement) -> Element {
	View()
		.class("p-10 items-start")
		.child(Button("Account").id("me").class("w-20 h-8"))
		.child(Text("Below the button"))
		.child(
			Popover("me", open)
				.placement(placement)
				.child(View().class("w-32 h-24").child(Button("Sign out"))),
		)
}

#[test]
fn an_open_popover_hangs_from_its_anchor() {
	let below = surface(&menu(true, Placement::BelowStart));
	let popover = first(&below, NativeKind::Popover);
	assert_eq!(
		below.frame(popover),
		Some(Frame {
			x: 40.0,
			y: 72.0,
			width: 128.0,
			height: 96.0,
		})
	);

	let end = surface(&menu(true, Placement::BelowEnd));
	let popover = first(&end, NativeKind::Popover);
	// The anchor's end edge is at 120, so a 128-wide popover would start
	// off screen: it is moved back to the edge.
	assert_eq!(end.frame(popover).map(|frame| frame.x), Some(0.0));

	let closed = surface(&menu(false, Placement::BelowStart));
	assert_eq!(closed.open_overlays(), []);
}

#[test]
fn a_popover_is_hit_before_the_page_and_the_page_around_it_still_is() {
	let surface = surface(&menu(true, Placement::BelowStart));

	// Over the text the popover covers.
	let hit = surface.hit_test(60.0, 80.0).expect("the popover is hit");
	assert_eq!(surface.props(hit).text.as_deref(), Some("Sign out"));
	// The anchor is outside the popover and still pressable.
	let hit = surface.hit_test(60.0, 50.0).expect("the anchor is hit");
	assert_eq!(surface.props(hit).text.as_deref(), Some("Account"));
}

#[test]
fn a_dismissal_is_heard_by_its_overlay_alone() {
	let outer = Arc::new(AtomicUsize::new(0));
	let inner = Arc::new(AtomicUsize::new(0));
	let screen = {
		let (outer, inner) = (Arc::clone(&outer), Arc::clone(&inner));

		View().child(
			Modal(true)
				.aria_label("Settings")
				.on_dismiss(move || {
					outer.fetch_add(1, Ordering::Relaxed);
				})
				.child(Button("Theme").id("theme"))
				.child(Popover("theme", true).child(Text("Dark")))
				.child(
					Popover("theme", true)
						.on_dismiss(move || {
							inner.fetch_add(1, Ordering::Relaxed);
						})
						.child(Text("Light")),
				),
		)
	};
	let surface = surface(&screen);
	let [modal, quiet, heard] = *surface.open_overlays() else {
		panic!("three overlays are open");
	};

	// A popover without a handler does not pass the dismissal to the modal
	// around it.
	assert_eq!(
		surface.dispatch_event(quiet, &NativeEvent::Dismiss),
		EventOutcome::Unhandled
	);
	assert_eq!(outer.load(Ordering::Relaxed), 0);

	assert_eq!(
		surface.dispatch_event(heard, &NativeEvent::Dismiss),
		EventOutcome::Handled
	);
	assert_eq!(
		surface.dispatch_event(modal, &NativeEvent::Dismiss),
		EventOutcome::Handled
	);
	assert_eq!(
		(outer.load(Ordering::Relaxed), inner.load(Ordering::Relaxed)),
		(1, 1)
	);
}

#[test]
fn an_overlay_must_name_elements_that_exist() {
	let no_anchor = View().child(Popover("missing", false).child(Text("Menu")));
	assert!(matches!(
		no_anchor.validate(),
		Err(ControlTreeError::UnknownIdReference { .. })
	));

	let no_focus = View().child(
		Modal(false)
			.aria_label("Confirm")
			.initial_focus("missing")
			.child(Text("Sure?")),
	);
	assert!(matches!(
		no_focus.validate(),
		Err(ControlTreeError::UnknownIdReference { .. })
	));
}

/// The kind each node was created with, in creation order.
fn created(surface: &mut NativeSurface<HeadlessBackend>) -> Vec<NativeKind> {
	surface
		.backend_mut()
		.take_log()
		.into_iter()
		.filter_map(|mutation| {
			match mutation {
				Mutation::Create { kind, .. } => Some(kind),
				_ => None,
			}
		})
		.collect()
}

#[test]
fn a_backend_without_overlays_is_given_a_container_in_the_same_place() {
	let mut plain = NativeSurface::new(HeadlessBackend::new());
	plain.set_viewport(400.0, 300.0);
	plain.set_root(&confirm(true));

	assert!(!created(&mut plain).contains(&NativeKind::Modal));

	// The frame is relative to a parent 40 in from the start edge, so it
	// starts that far back to cover the viewport.
	let modal = plain.open_overlays()[0];
	let frame = plain.frame(modal).expect("the modal has a frame");
	assert_eq!((frame.x, frame.width, frame.height), (-40.0, 400.0, 300.0));
	assert!(frame.y < -40.0, "{frame:?}");
	// Presses still stop at it.
	assert_eq!(plain.hit_test(50.0, 50.0), Some(modal));

	let mut overlays = NativeSurface::new(HeadlessBackend::new().presenting_overlays());
	overlays.set_root(&confirm(true));
	assert!(created(&mut overlays).contains(&NativeKind::Modal));
}
