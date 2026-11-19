//! Native execution only: modals and popovers in real owned popup windows.
#![cfg(target_os = "windows")]
#![allow(unsafe_code, clippy::float_cmp)]

use std::cell::RefCell;
use std::rc::Rc;

use leptoskit_native::Element;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeSurface;
use leptoskit_native::Placement;
use leptoskit_native::components::*;
use leptoskit_windows::WindowsBackend;
use leptoskit_windows::WindowsWindow;
use leptoskit_yoga::NodeId;
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows::Win32::UI::WindowsAndMessaging::*;

type Events = Rc<RefCell<Vec<(NodeId, NativeEvent)>>>;

/// Builds the tree, then mounts and shows it, as an application does, and
/// records what its widgets ask for.
fn mount(root: &Element) -> (WindowsWindow, NativeSurface<WindowsBackend>, Events) {
	let window = WindowsWindow::new("Leptoskit native overlay verification", 520, 680).unwrap();
	let mut backend = WindowsBackend::new().unwrap();
	backend.set_dpi(window.dpi());
	let events = Events::default();
	let recorded = Rc::clone(&events);
	backend.set_event_listener(Rc::new(move |id, event| {
		recorded.borrow_mut().push((id, event));
	}));
	let mut surface = NativeSurface::new(backend);
	surface.set_viewport(520.0, 680.0);
	surface.set_root(root);
	window
		.mount(surface.backend(), surface.root().unwrap())
		.unwrap();
	window.show();
	window.pump_messages();
	events.borrow_mut().clear();
	(window, surface, events)
}

fn child(surface: &NativeSurface<WindowsBackend>, index: usize) -> NodeId {
	surface.children(surface.root().unwrap())[index]
}

/// A window's bounds on the screen.
fn bounds(hwnd: HWND) -> RECT {
	let mut rect = RECT::default();
	// SAFETY: The call fills a local for a window the fixture owns.
	unsafe {
		GetWindowRect(hwnd, &raw mut rect).unwrap();
	}
	rect
}

/// Layout units as device pixels on the fixture's monitor.
fn pixels(window: &WindowsWindow, value: f32) -> i32 {
	(value * window.dpi() as f32 / 96.0).round() as i32
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

#[test]
fn a_modal_shows_in_an_owned_window_sized_to_what_it_holds() {
	// The modal is open before the host is mounted or shown.
	let (window, mut surface, events) = mount(&confirmation(true));
	let modal = child(&surface, 1);
	let field = surface.children(surface.children(modal)[0])[0];

	assert_eq!(surface.backend().overlay_shown(child(&surface, 0)), None);
	assert_eq!(surface.backend().overlay_shown(modal), Some(true));
	let shell = surface.backend().overlay_window(modal).unwrap();
	// SAFETY: Every handle belongs to this fixture's backend or host.
	unsafe {
		assert!(IsWindowVisible(shell).as_bool());
		assert_eq!(GetWindow(shell, GW_OWNER).unwrap(), window.handle());
		// The window behind takes no input while the modal is up.
		assert!(!IsWindowEnabled(window.handle()).as_bool());
		// The modal's own windows are in the popup, not in the host.
		assert_eq!(
			GetAncestor(surface.backend().view(modal).unwrap(), GA_ROOT),
			shell
		);
		// The field named by `initial_focus` has focus.
		assert_eq!(GetFocus(), surface.backend().control(field).unwrap());
	}
	// The modal covers the viewport and centres its panel. The popup is the
	// panel alone, where the panel would have been drawn.
	let rect = bounds(shell);
	assert_eq!(rect.right - rect.left, pixels(&window, 256.0));
	assert_eq!(rect.bottom - rect.top, pixels(&window, 128.0));
	let panel = bounds(surface.backend().view(surface.children(modal)[0]).unwrap());
	assert_eq!((panel.left, panel.top), (rect.left, rect.top));
	events.borrow_mut().clear();

	// Alt+F4 and Escape ask, and the modal stays until the app answers.
	// SAFETY: Synchronous messages to the fixture's own popup.
	unsafe {
		SendMessageW(shell, WM_CLOSE, None, None);
		SendMessageW(shell, WM_COMMAND, Some(WPARAM(IDCANCEL.0 as usize)), None);
	}
	assert_eq!(
		*events.borrow(),
		vec![(modal, NativeEvent::Dismiss), (modal, NativeEvent::Dismiss)]
	);
	assert_eq!(surface.backend().overlay_shown(modal), Some(true));

	surface.set_root(&confirmation(false));
	assert_eq!(surface.backend().overlay_shown(modal), Some(false));
	// SAFETY: As above.
	unsafe {
		assert!(!IsWindowVisible(shell).as_bool());
		assert!(IsWindowEnabled(window.handle()).as_bool());
	}

	surface.set_root(&confirmation(true));
	assert_eq!(surface.backend().overlay_shown(modal), Some(true));

	// Removing an open modal takes its window down and frees the host.
	surface.set_root(&View().class("w-full h-full"));
	// SAFETY: IsWindow explicitly permits testing a formerly valid HWND.
	unsafe {
		assert!(!IsWindow(Some(shell)).as_bool());
		assert!(IsWindowEnabled(window.handle()).as_bool());
	}
	window.close().unwrap();
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

#[test]
fn a_popover_hangs_beside_its_anchor_and_asks_before_it_closes() {
	let (window, mut surface, events) = mount(&menu(false, Placement::BelowStart));
	let anchor = child(&surface, 0);
	let popover = child(&surface, 1);
	let item = surface.children(surface.children(popover)[0])[0];
	assert_eq!(surface.backend().overlay_shown(popover), Some(false));
	assert_eq!(surface.backend().overlay_window(popover), None);

	surface.set_root(&menu(true, Placement::BelowStart));
	assert_eq!(surface.backend().overlay_shown(popover), Some(true));
	let shell = surface.backend().overlay_window(popover).unwrap();
	// Below the button, start edges together, as large as what it holds.
	let button = bounds(surface.backend().view(anchor).unwrap());
	let rect = bounds(shell);
	assert_eq!(rect.right - rect.left, pixels(&window, 160.0));
	assert_eq!(rect.bottom - rect.top, pixels(&window, 80.0));
	assert_eq!(rect.left, button.left);
	assert!(
		rect.top >= button.bottom && rect.top <= button.bottom + pixels(&window, 16.0),
		"the popover's top {} is just under the anchor's bottom {}",
		rect.top,
		button.bottom
	);
	// SAFETY: Every handle belongs to this fixture's backend or host.
	unsafe {
		assert_eq!(GetWindow(shell, GW_OWNER).unwrap(), window.handle());
		// A popover leaves the window behind in use.
		assert!(IsWindowEnabled(window.handle()).as_bool());
		// Nothing was named to take focus, so the first control inside does.
		assert_eq!(GetFocus(), surface.backend().control(item).unwrap());
	}
	events.borrow_mut().clear();

	// Losing activation, as a press outside causes, asks. The popover stays
	// until the app answers.
	// SAFETY: A synchronous message to the fixture's own popup.
	unsafe {
		SendMessageW(shell, WM_ACTIVATE, Some(WPARAM(WA_INACTIVE as usize)), None);
		SendMessageW(shell, WM_COMMAND, Some(WPARAM(IDCANCEL.0 as usize)), None);
	}
	assert_eq!(
		*events.borrow(),
		vec![
			(popover, NativeEvent::Dismiss),
			(popover, NativeEvent::Dismiss)
		]
	);
	assert_eq!(surface.backend().overlay_shown(popover), Some(true));

	surface.set_root(&menu(false, Placement::BelowStart));
	assert_eq!(surface.backend().overlay_shown(popover), Some(false));
	// SAFETY: As above.
	unsafe {
		assert!(!IsWindowVisible(shell).as_bool());
	}

	// Above: the popover's bottom meets the anchor's top, centred on it.
	surface.set_root(&menu(true, Placement::Above));
	let rect = bounds(surface.backend().overlay_window(popover).unwrap());
	assert!(
		rect.bottom <= button.top && rect.bottom >= button.top - pixels(&window, 16.0),
		"the popover's bottom {} is just over the anchor's top {}",
		rect.bottom,
		button.top
	);
	assert!(((rect.left + rect.right) - (button.left + button.right)).abs() <= 2);
	window.close().unwrap();
}

#[test]
fn overlay_windows_stay_out_of_their_parent() {
	let page = |open: bool| {
		View()
			.class("w-full h-full")
			.child(Button("First").id("first").class("w-32 h-10"))
			.child(Modal(open).child(View().class("w-64 h-32")))
			.child(Popover("first", open).child(View().class("w-40 h-20")))
			.child(Text("After").class("w-32 h-10"))
	};
	let (window, mut surface, _events) = mount(&page(false));
	let root = |surface: &NativeSurface<WindowsBackend>, index: usize| {
		// SAFETY: The view is a live window of this fixture's backend.
		unsafe {
			GetAncestor(
				surface.backend().view(child(surface, index)).unwrap(),
				GA_ROOT,
			)
		}
	};

	// Closed overlays wait outside the host, and take no room in it.
	assert_eq!(root(&surface, 0), window.handle());
	assert_eq!(root(&surface, 3), window.handle());
	assert_ne!(root(&surface, 1), window.handle());
	assert_ne!(root(&surface, 2), window.handle());
	let first = bounds(surface.backend().view(child(&surface, 0)).unwrap());
	let after = bounds(surface.backend().view(child(&surface, 3)).unwrap());
	assert_eq!(after.top, first.bottom);

	surface.set_root(&page(true));
	assert_eq!(
		surface.backend().overlay_shown(child(&surface, 1)),
		Some(true)
	);
	assert_eq!(
		root(&surface, 1),
		surface
			.backend()
			.overlay_window(child(&surface, 1))
			.unwrap()
	);
	assert_eq!(root(&surface, 3), window.handle());
	window.close().unwrap();
}
