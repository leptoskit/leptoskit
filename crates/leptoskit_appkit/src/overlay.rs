//! Sheets and popovers: the AppKit objects behind `Modal` and `Popover`.
//!
//! A `Modal` is a sheet on the window that holds its parent, and a `Popover`
//! is an `NSPopover` hanging from its anchor's view. Neither one's view sits
//! in its parent's view: AppKit owns where an overlay goes, and the backend
//! only says when it shows, what it holds and how large that is.

use std::cell::RefCell;
use std::rc::Rc;

use leptoskit_native::NativeEvent;
use leptoskit_native::Placement;
use leptoskit_yoga::NodeId;
use objc2::DefinedClass;
use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::Message;
use objc2::define_class;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::runtime::NSObject;
use objc2::runtime::NSObjectProtocol;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSBackingStoreType;
use objc2_app_kit::NSColor;
use objc2_app_kit::NSPanel;
use objc2_app_kit::NSPopover;
use objc2_app_kit::NSPopoverBehavior;
use objc2_app_kit::NSPopoverDelegate;
use objc2_app_kit::NSResponder;
use objc2_app_kit::NSView;
use objc2_app_kit::NSViewController;
use objc2_app_kit::NSWindow;
use objc2_app_kit::NSWindowStyleMask;
use objc2_foundation::NSPoint;
use objc2_foundation::NSRect;
use objc2_foundation::NSRectEdge;
use objc2_foundation::NSSize;

use crate::views::EventSink;
use crate::views::FlippedView;

/// Which node an overlay speaks for, and where its request to close goes.
pub(crate) struct Dismissal {
	node: NodeId,
	sink: Rc<EventSink>,
}

impl Dismissal {
	/// Asks the app to close the overlay. The overlay stays as it is until
	/// the next tree says otherwise.
	fn request(&self) {
		self.sink.emit(self.node, &NativeEvent::Dismiss);
	}
}

define_class!(
	// SAFETY:
	// - `NSPanel` is initialized through `NSWindow`'s designated initializer,
	//   which `SheetPanel::new` calls.
	// - The class does not implement `Drop`; its ivars are dropped by objc2.
	#[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = Dismissal]
	/// The window a `Modal` shows as a sheet. Escape reaches it as
	/// `cancelOperation:` once nothing inside has taken the key, and becomes
	/// a request to close.
	pub(crate) struct SheetPanel;

	impl SheetPanel {
		#[unsafe(method(cancelOperation:))]
		fn cancel_operation(&self, _sender: Option<&AnyObject>) {
			self.ivars().request();
		}

		#[unsafe(method(canBecomeKeyWindow))]
		fn can_become_key_window(&self) -> bool {
			true
		}
	}
);

impl SheetPanel {
	/// A panel with nothing drawn behind its content, holding `content`.
	fn new(mtm: MainThreadMarker, dismissal: Dismissal, content: &NSView) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(dismissal);
		// SAFETY: `initWithContentRect:styleMask:backing:defer:` is the
		// designated initializer of `NSWindow` and `NSPanel`, and a deferred
		// buffered window with an empty content rectangle is valid.
		let panel: Retained<Self> = unsafe {
			msg_send![
				super(this),
				initWithContentRect: NSRect::ZERO,
				styleMask: NSWindowStyleMask::Titled,
				backing: NSBackingStoreType::Buffered,
				defer: true,
			]
		};
		// SAFETY: the backend owns the panel through `Retained`, so AppKit
		// must not also release it when the sheet ends.
		unsafe { panel.setReleasedWhenClosed(false) };
		// The app draws the dialog's own panel, corners and all.
		panel.setOpaque(false);
		panel.setBackgroundColor(Some(&NSColor::clearColor()));
		panel.setContentView(Some(content));
		panel
	}
}

define_class!(
	// SAFETY:
	// - `NSObject` has no subclassing requirements.
	// - The class does not implement `Drop`; its ivars are dropped by objc2.
	#[unsafe(super(NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = Dismissal]
	/// Turns a popover's own wish to close (Escape, a press outside, the
	/// app losing focus) into a request the app answers.
	pub(crate) struct PopoverDelegate;

	unsafe impl NSObjectProtocol for PopoverDelegate {}

	unsafe impl NSPopoverDelegate for PopoverDelegate {
		#[unsafe(method(popoverShouldClose:))]
		fn popover_should_close(&self, _popover: &NSPopover) -> bool {
			self.ivars().request();
			// The app closes it by building the next tree without it open.
			false
		}
	}
);

impl PopoverDelegate {
	fn new(mtm: MainThreadMarker, dismissal: Dismissal) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(dismissal);
		// SAFETY: `init` is `NSObject`'s designated initializer.
		unsafe { msg_send![super(this), init] }
	}
}

/// A `Modal`'s sheet.
pub(crate) struct Sheet {
	panel: Retained<SheetPanel>,
	/// The panel's content view, as large as what the modal holds. The
	/// modal's own view sits inside it, shifted so that content fills it.
	frame: Retained<FlippedView>,
	/// The window the sheet is showing on.
	host: RefCell<Option<Retained<NSWindow>>>,
}

impl Sheet {
	/// A sheet holding `view`, the modal's own view, not showing yet.
	pub(crate) fn new(
		mtm: MainThreadMarker,
		node: NodeId,
		sink: Rc<EventSink>,
		view: &NSView,
	) -> Self {
		let frame = FlippedView::new(mtm);
		frame.addSubview(view);
		let panel = SheetPanel::new(mtm, Dismissal { node, sink }, &frame);

		Self {
			panel,
			frame,
			host: RefCell::new(None),
		}
	}

	/// The sheet's window, which holds the modal's views while it shows.
	pub(crate) fn window(&self) -> &NSWindow {
		&self.panel
	}

	/// Whether the sheet is on a window.
	pub(crate) fn is_shown(&self) -> bool {
		self.host.borrow().is_some()
	}

	/// Asks to close as Escape does.
	pub(crate) fn cancel(&self) {
		// SAFETY: `cancelOperation:` takes an optional sender, and the panel
		// implements it.
		unsafe {
			let _: () = msg_send![&*self.panel, cancelOperation: None::<&AnyObject>];
		}
	}

	/// Sizes the sheet to `content`, the rectangle inside the modal's view
	/// that its children cover, and shows it on `window` if it is not
	/// showing yet. Returns whether this call showed it.
	///
	/// A modal covers the viewport and centres what it holds, which is how
	/// the web and the terminal draw a backdrop. A sheet brings its own, so
	/// only the content is shown.
	pub(crate) fn show(&self, view: &NSView, content: NSRect, window: &NSWindow) -> bool {
		self.frame
			.setFrame(NSRect::new(NSPoint::ZERO, content.size));
		view.setFrameOrigin(NSPoint::new(-content.origin.x, -content.origin.y));
		self.panel.setContentSize(content.size);

		if self.is_shown() {
			return false;
		}

		window.beginSheet_completionHandler(&self.panel, None);
		*self.host.borrow_mut() = Some(window.retain());
		true
	}

	/// Takes the sheet off its window, if it is on one.
	pub(crate) fn hide(&self) {
		let host = self.host.borrow_mut().take();

		if let Some(window) = host {
			window.endSheet(&self.panel);
		}
	}
}

/// A `Popover`'s `NSPopover`.
pub(crate) struct Hanging {
	popover: Retained<NSPopover>,
	/// The popover holds its delegate weakly, so the widget keeps it alive.
	_delegate: Retained<PopoverDelegate>,
}

impl Hanging {
	/// A popover holding `view`, not showing yet.
	pub(crate) fn new(
		mtm: MainThreadMarker,
		node: NodeId,
		sink: Rc<EventSink>,
		view: &NSView,
	) -> Self {
		let controller = NSViewController::new(mtm);
		controller.setView(view);
		let popover = NSPopover::new(mtm);
		popover.setContentViewController(Some(&controller));
		// Transient: AppKit wants to close it on Escape and on a press
		// outside, and the delegate turns that wish into a request.
		popover.setBehavior(NSPopoverBehavior::Transient);
		let delegate = PopoverDelegate::new(mtm, Dismissal { node, sink });
		popover.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

		Self {
			popover,
			_delegate: delegate,
		}
	}

	/// Whether the popover is on screen.
	pub(crate) fn is_shown(&self) -> bool {
		self.popover.isShown()
	}

	/// Sets how large the popover's content is.
	pub(crate) fn resize(&self, size: NSSize) {
		self.popover.setContentSize(size);
	}

	/// Shows the popover beside `anchor` if it is not showing. Returns
	/// whether this call showed it.
	///
	/// AppKit centres a popover on its anchor's edge and points an arrow at
	/// it, so a placement picks the side and nothing more. It also moves
	/// the popover to another side when the chosen one has no room.
	pub(crate) fn show(&self, anchor: &NSView, placement: Placement) -> bool {
		// AppKit raises when the anchor is in no window.
		if self.popover.isShown() || anchor.window().is_none() {
			return false;
		}

		// The edge follows the anchor's own coordinates: in a flipped view
		// the largest y is the bottom edge.
		let below = if anchor.isFlipped() {
			NSRectEdge::MaxY
		} else {
			NSRectEdge::MinY
		};
		let above = if anchor.isFlipped() {
			NSRectEdge::MinY
		} else {
			NSRectEdge::MaxY
		};
		let edge = if placement.is_above() { above } else { below };

		self.popover
			.showRelativeToRect_ofView_preferredEdge(anchor.bounds(), anchor, edge);
		self.popover.isShown()
	}

	/// Asks to close as Escape and a press outside do.
	pub(crate) fn cancel(&self) {
		// SAFETY: `performClose:` takes an optional sender.
		unsafe { self.popover.performClose(None) };
	}

	/// Closes the popover without asking its delegate.
	pub(crate) fn hide(&self) {
		if self.popover.isShown() {
			self.popover.close();
		}
	}

	/// Detaches the delegate before the popover's node goes away.
	pub(crate) fn deactivate(&self) {
		self.popover.setDelegate(None);
		self.hide();
	}
}
