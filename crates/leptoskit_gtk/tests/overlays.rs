//! Modals and popovers on real GTK windows: where each shows, what asks it
//! to close, and that nothing closes until the application says so.

fn main() {
	let mut args = libtest_mimic::Arguments::from_args();
	args.test_threads = Some(1);
	#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
	let trials = linux::trials();
	#[cfg(not(all(target_os = "linux", not(target_env = "ohos"))))]
	let trials = Vec::new();
	libtest_mimic::run(&args, trials).exit();
}

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
mod linux {
	use std::cell::RefCell;
	use std::process::Command;
	use std::rc::Rc;
	use std::time::Duration;

	use gtk::prelude::*;
	use leptoskit_gtk::GtkBackend;
	use leptoskit_gtk::gtk;
	use leptoskit_native::Element;
	use leptoskit_native::NativeEvent;
	use leptoskit_native::NativeSurface;
	use leptoskit_native::Placement;
	use leptoskit_native::components::*;
	use leptoskit_yoga::NodeId;
	use libtest_mimic::Trial;

	const TITLE: &str = "Leptoskit GTK Overlays";

	pub fn trials() -> Vec<Trial> {
		gtk::init().expect("GTK needs a display; run under dbus-run-session and xvfb-run");
		[
			(
				"a_modal_shows_in_a_window_sized_to_what_it_holds",
				a_modal_shows_in_a_window_sized_to_what_it_holds as fn(),
			),
			(
				"a_popover_hangs_beside_its_anchor_and_asks_before_it_closes",
				a_popover_hangs_beside_its_anchor_and_asks_before_it_closes,
			),
			(
				"a_popover_that_starts_open_shows_with_its_window",
				a_popover_that_starts_open_shows_with_its_window,
			),
			(
				"overlay_frames_stay_out_of_their_parent",
				overlay_frames_stay_out_of_their_parent,
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

	fn pump() {
		let context = gtk::glib::MainContext::default();

		for _ in 0..20 {
			while context.pending() {
				context.iteration(false);
			}

			std::thread::sleep(Duration::from_millis(5));
		}
	}

	/// X is asynchronous; wait for a native effect instead of assuming one
	/// pump suffices on a loaded display server.
	fn pump_until(condition: impl Fn() -> bool) {
		let deadline = std::time::Instant::now() + Duration::from_secs(5);

		while !condition() {
			assert!(
				std::time::Instant::now() < deadline,
				"native state did not settle within five seconds"
			);
			pump();
		}
	}

	fn sync_display() {
		gtk::gdk::Display::default().expect("an X display").sync();
		pump();
	}

	type Events = Rc<RefCell<Vec<(NodeId, NativeEvent)>>>;

	struct Mounted {
		surface: NativeSurface<GtkBackend>,
		window: gtk::Window,
		events: Events,
	}

	impl Drop for Mounted {
		fn drop(&mut self) {
			self.surface.backend().clear_event_listener();
			self.surface.set_root(&View());
			self.window.destroy();
			// Later tests find their window by title; finish this destruction
			// first.
			sync_display();
		}
	}

	impl Mounted {
		fn child(&self, index: usize) -> NodeId {
			self.surface.children(self.surface.root().unwrap())[index]
		}

		fn shown(&self, id: NodeId) -> bool {
			self.surface.backend().overlay_shown(id) == Some(true)
		}

		/// What the widgets have asked for since the last call.
		fn asked(&self) -> Vec<(NodeId, NativeEvent)> {
			std::mem::take(&mut *self.events.borrow_mut())
		}

		/// A widget's corner and size in the main window.
		fn bounds(&self, id: NodeId) -> (f32, f32, f32, f32) {
			let bounds = self
				.surface
				.backend()
				.view(id)
				.unwrap()
				.compute_bounds(&self.window)
				.expect("a widget in the window");
			(bounds.x(), bounds.y(), bounds.width(), bounds.height())
		}
	}

	/// Builds the tree with no window yet, and records what its widgets ask.
	fn built(tree: &Element) -> (NativeSurface<GtkBackend>, Events) {
		let mut surface = NativeSurface::new(GtkBackend::new());
		surface.set_dark_mode(false);
		surface.set_viewport(480.0, 640.0);
		surface.set_root(tree);
		let events = Events::default();
		surface.backend().set_event_listener({
			let events = Rc::clone(&events);
			move |id, event| events.borrow_mut().push((id, event))
		});
		(surface, events)
	}

	fn mounted(tree: &Element) -> Mounted {
		let (surface, events) = built(tree);
		let window = surface
			.backend()
			.open_window(surface.root().unwrap(), TITLE, 480, 640)
			.unwrap();
		let root = surface.backend().view(surface.root().unwrap()).unwrap();
		pump_until(|| window.is_mapped() && root.width() > 0);
		pump();
		events.borrow_mut().clear();
		Mounted {
			surface,
			window,
			events,
		}
	}

	fn key(name: &str) {
		assert!(
			Command::new("xdotool")
				.args(["key", "--clearmodifiers", name])
				.status()
				.expect("xdotool installed")
				.success()
		);
		sync_display();
	}

	/// Clicks the primary button at a point of the main window.
	fn click(x: f32, y: f32) {
		let output = Command::new("xdotool")
			.args(["search", "--name", &format!("^{TITLE}$")])
			.output()
			.unwrap();
		assert!(output.status.success());
		let window = String::from_utf8(output.stdout).unwrap();
		let window = window.lines().last().unwrap();
		assert!(
			Command::new("xdotool")
				.args([
					"mousemove",
					"--sync",
					"--window",
					window,
					&(x.round() as i32).to_string(),
					&(y.round() as i32).to_string(),
					"click",
					"1"
				])
				.status()
				.unwrap()
				.success()
		);
		sync_display();
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

	fn a_modal_shows_in_a_window_sized_to_what_it_holds() {
		let (surface, _events) = built(&confirmation(true));
		let modal = surface.children(surface.root().unwrap())[1];
		assert_eq!(
			surface.backend().overlay_shown(modal),
			Some(false),
			"with no window there is nothing to show a modal above"
		);
		assert_eq!(
			surface
				.backend()
				.overlay_shown(surface.children(surface.root().unwrap())[0]),
			None
		);
		drop(surface);

		let mut mounted = mounted(&confirmation(true));
		let modal = mounted.child(1);
		let field = mounted.surface.children(mounted.surface.children(modal)[0])[0];
		let dialog = mounted.surface.backend().overlay_window(modal).unwrap();
		pump_until(|| dialog.is_mapped() && dialog.width() > 0);

		assert!(mounted.shown(modal));
		assert!(dialog.is_modal());
		assert_eq!(dialog.transient_for().as_ref(), Some(&mounted.window));
		// The modal covers the viewport and centres its panel. The window is
		// the panel alone.
		assert_eq!((dialog.width(), dialog.height()), (256, 128));
		let frame = mounted.surface.backend().view(modal).unwrap();
		assert_eq!((frame.width(), frame.height()), (480, 640));
		assert_eq!(
			frame.root().and_downcast::<gtk::Window>(),
			Some(dialog.clone())
		);
		let panel = mounted
			.surface
			.backend()
			.view(mounted.surface.children(modal)[0])
			.unwrap();
		let bounds = panel.compute_bounds(&dialog).unwrap();
		assert_eq!((bounds.x(), bounds.y()), (0.0, 0.0));

		// The field named by `initial_focus` has focus, in the dialog.
		let entry = mounted.surface.backend().control(field).unwrap();
		pump_until(|| {
			entry.has_focus() || entry.first_child().is_some_and(|text| text.has_focus())
		});
		mounted.asked();

		// The window manager's close asks, and the dialog stays.
		dialog.close();
		pump();
		assert_eq!(mounted.asked(), vec![(modal, NativeEvent::Dismiss)]);
		assert!(mounted.shown(modal));

		// So does Escape.
		key("Escape");
		pump_until(|| !mounted.events.borrow().is_empty());
		assert_eq!(mounted.asked(), vec![(modal, NativeEvent::Dismiss)]);
		assert!(mounted.shown(modal));

		mounted.surface.set_root(&confirmation(false));
		assert!(!mounted.shown(modal));
		pump();

		mounted.surface.set_root(&confirmation(true));
		assert!(mounted.shown(modal));
		pump();

		// Removing an open modal takes its window down with it.
		mounted.surface.set_root(&View().class("w-full h-full"));
		pump();
		assert!(!dialog.is_visible());
	}

	/// A page with a menu hanging from a button.
	fn menu(open: bool, placement: Placement) -> Element {
		View()
			.class("w-full h-full items-center justify-center")
			.child(Button("Account").id("me").class("w-32 h-10"))
			.child(
				Popover("me", open).placement(placement).child(
					View()
						.class("w-40 h-20 bg-white")
						.child(Button("Sign out").id("out").class("w-32 h-10")),
				),
			)
	}

	/// Where a popover's own surface is, from the corner of the window it
	/// hangs in.
	fn popup_bounds(popover: &gtk::Popover) -> (i32, i32, i32, i32) {
		let popup = popover
			.surface()
			.and_downcast::<gtk::gdk::Popup>()
			.expect("a popover on screen has a popup surface");
		(
			popup.position_x(),
			popup.position_y(),
			popup.width(),
			popup.height(),
		)
	}

	fn a_popover_hangs_beside_its_anchor_and_asks_before_it_closes() {
		let mut mounted = mounted(&menu(false, Placement::BelowStart));
		let anchor = mounted.child(0);
		let menu_id = mounted.child(1);
		let item = mounted
			.surface
			.children(mounted.surface.children(menu_id)[0])[0];
		assert!(!mounted.shown(menu_id));
		let button = mounted.surface.backend().control(anchor).unwrap();
		button.grab_focus();
		pump_until(|| button.has_focus());

		mounted.surface.set_root(&menu(true, Placement::BelowStart));
		assert!(mounted.shown(menu_id));
		let popover = mounted.surface.backend().overlay_popover(menu_id).unwrap();
		pump_until(|| popover.is_mapped() && popover.width() > 0);
		pump();

		// Below the button, start edges together, as large as what it holds.
		let (left, top, width, height) = mounted.bounds(anchor);
		let (x, y, popup_width, popup_height) = popup_bounds(&popover);
		assert_eq!((popup_width, popup_height), (160, 80));
		assert_eq!(x, left.round() as i32);
		assert!(
			y >= (top + height).round() as i32 && y <= (top + height).round() as i32 + 16,
			"the popover's top {y} is just under the anchor's bottom {}",
			top + height
		);
		// Nothing was named to take focus, so the first control inside does.
		let out = mounted.surface.backend().control(item).unwrap();
		pump_until(|| out.has_focus());
		mounted.asked();

		// A press inside is the popover's own business.
		click(x as f32 + 150.0, y as f32 + 70.0);
		assert!(
			mounted
				.asked()
				.iter()
				.all(|(_, event)| *event != NativeEvent::Dismiss)
		);

		// A press outside asks, and the popover stays until the app answers.
		click(20.0, 20.0);
		pump_until(|| !mounted.events.borrow().is_empty());
		assert!(mounted.asked().contains(&(menu_id, NativeEvent::Dismiss)));
		assert!(mounted.shown(menu_id));

		// A press on the anchor is the anchor's to answer.
		click(left + width / 2.0, top + height / 2.0);
		pump_until(|| !mounted.events.borrow().is_empty());
		let asked = mounted.asked();
		assert!(asked.contains(&(anchor, NativeEvent::Press)), "{asked:?}");
		assert!(
			!asked.contains(&(menu_id, NativeEvent::Dismiss)),
			"{asked:?}"
		);

		// Escape asks too.
		key("Escape");
		pump_until(|| !mounted.events.borrow().is_empty());
		assert!(mounted.asked().contains(&(menu_id, NativeEvent::Dismiss)));
		assert!(mounted.shown(menu_id));

		// Closing gives focus back to what had it.
		mounted
			.surface
			.set_root(&menu(false, Placement::BelowStart));
		assert!(!mounted.shown(menu_id));
		pump();
		assert!(!popover.is_visible());
		assert!(button.has_focus());

		// Above: the popover's bottom meets the anchor's top.
		mounted.surface.set_root(&menu(true, Placement::Above));
		pump_until(|| popover.is_mapped());
		pump();
		let (x, y, popup_width, popup_height) = popup_bounds(&popover);
		assert_eq!(x, (left + width / 2.0).round() as i32 - popup_width / 2);
		assert!(
			y + popup_height <= top.round() as i32 && y + popup_height >= top.round() as i32 - 16,
			"the popover's bottom {} is just over the anchor's top {top}",
			y + popup_height
		);

		// With no popover open, Escape and presses ask for nothing.
		mounted.surface.set_root(&menu(false, Placement::Above));
		pump();
		mounted.asked();
		key("Escape");
		click(20.0, 20.0);
		assert!(
			mounted
				.asked()
				.iter()
				.all(|(_, event)| *event != NativeEvent::Dismiss)
		);
	}

	fn a_popover_that_starts_open_shows_with_its_window() {
		let mounted = mounted(&menu(true, Placement::BelowEnd));
		let anchor = mounted.child(0);
		let menu_id = mounted.child(1);
		let popover = mounted.surface.backend().overlay_popover(menu_id).unwrap();
		pump_until(|| mounted.shown(menu_id) && popover.is_mapped() && popover.width() > 0);
		pump();

		let (left, _, width, _) = mounted.bounds(anchor);
		let (x, _, popup_width, _) = popup_bounds(&popover);
		assert_eq!(x + popup_width, (left + width).round() as i32);
	}

	fn overlay_frames_stay_out_of_their_parent() {
		let page = |open: bool| {
			View()
				.class("w-full h-full")
				.child(Button("First").id("first").class("w-32 h-10"))
				.child(Modal(open).child(View().class("w-64 h-32")))
				.child(Popover("first", open).child(View().class("w-40 h-20")))
				.child(Text("After"))
		};
		let mut mounted = mounted(&page(false));
		let view = |mounted: &Mounted, index: usize| {
			mounted
				.surface
				.backend()
				.view(mounted.child(index))
				.unwrap()
		};
		let first = view(&mounted, 0);
		let after = view(&mounted, 3);

		// The overlays hold a place in the tree, and none among the widgets.
		assert_eq!(after.parent(), first.parent());
		assert_eq!(after.prev_sibling().as_ref(), Some(&first));
		assert_ne!(view(&mounted, 1).parent(), first.parent());
		assert_ne!(view(&mounted, 2).parent(), first.parent());

		mounted.surface.set_root(&page(true));
		pump();
		assert!(mounted.shown(mounted.child(1)));
		assert_eq!(after.prev_sibling().as_ref(), Some(&first));
		assert_ne!(view(&mounted, 1).root(), first.root());
	}
}
