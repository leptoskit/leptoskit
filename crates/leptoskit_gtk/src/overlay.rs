//! The window a `Modal` shows in and the popover a `Popover` shows in.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;

use crate::frame::Frame;

/// The class every overlay's window carries, so the theme's own panel,
/// shadow and padding leave the application's drawing alone.
const CLASS: &str = "leptoskit-overlay";
const CSS: &str = "window.leptoskit-overlay, popover.leptoskit-overlay, popover.leptoskit-overlay \
                   > contents { background: none; box-shadow: none; border: none; padding: 0; \
                   margin: 0; border-radius: 0; min-width: 0; min-height: 0; }";

thread_local! {
	static STYLED: RefCell<Vec<gdk::Display>> = const { RefCell::new(Vec::new()) };
}

/// Loads the overlay style once for each display.
fn style(display: &gdk::Display) {
	STYLED.with_borrow_mut(|styled| {
		if styled.contains(display) {
			return;
		}

		let provider = gtk::CssProvider::new();
		provider.load_from_data(CSS);
		gtk::style_context_add_provider_for_display(
			display,
			&provider,
			gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
		);
		styled.push(display.clone());
	});
}

#[derive(Clone)]
enum Shown {
	/// A modal window above the window its parent is in.
	Dialog(gtk::Window),
	/// A popover on the frame whose corner layout measures from.
	Hanging(gtk::Popover),
}

/// What GTK shows a `Modal` or a `Popover` in. The node's own frame is a
/// child of `host` and never of its parent's frame.
#[derive(Clone)]
pub(crate) struct Overlay {
	host: Frame,
	shown: Shown,
	/// What had focus when a popover opened, which gets it back.
	before: Rc<RefCell<Option<glib::WeakRef<gtk::Widget>>>>,
}

impl Overlay {
	/// Builds the window for a node whose own frame is `frame`. `dismiss`
	/// runs when the window manager asks a dialog to close: the window stays
	/// up, and the application decides.
	pub(crate) fn new(popover: bool, frame: &Frame, dismiss: impl Fn() + 'static) -> Self {
		if let Some(display) = gdk::Display::default() {
			style(&display);
		}

		let host = Frame::new();
		host.put(frame, graphene::Rect::zero(), 0, false);
		let shown = if popover {
			let popover = gtk::Popover::builder()
				.autohide(false)
				.has_arrow(false)
				.position(gtk::PositionType::Bottom)
				.child(&host)
				.build();
			popover.add_css_class(CLASS);
			Shown::Hanging(popover)
		} else {
			let window = gtk::Window::builder()
				.modal(true)
				.decorated(false)
				.resizable(false)
				.child(&host)
				.build();
			window.add_css_class(CLASS);
			window.connect_close_request(move |_| {
				dismiss();
				glib::Propagation::Stop
			});
			Shown::Dialog(window)
		};

		Self {
			host,
			shown,
			before: Rc::default(),
		}
	}

	pub(crate) fn is_shown(&self) -> bool {
		match &self.shown {
			Shown::Dialog(window) => window.is_visible(),
			Shown::Hanging(popover) => popover.is_visible(),
		}
	}

	pub(crate) fn window(&self) -> Option<gtk::Window> {
		match &self.shown {
			Shown::Dialog(window) => Some(window.clone()),
			Shown::Hanging(_) => None,
		}
	}

	pub(crate) fn popover(&self) -> Option<gtk::Popover> {
		match &self.shown {
			Shown::Dialog(_) => None,
			Shown::Hanging(popover) => Some(popover.clone()),
		}
	}

	/// The window whose keys and presses reach this overlay: a dialog is its
	/// own, and a popover belongs to the window it hangs in.
	pub(crate) fn root(&self) -> Option<gtk::Window> {
		match &self.shown {
			Shown::Dialog(window) => Some(window.clone()),
			Shown::Hanging(popover) => popover.root().and_downcast(),
		}
	}

	/// Shows a modal above `parent`, as large as `held`: the part of the
	/// viewport-sized `frame` the modal's children cover. Returns whether
	/// this call showed it.
	pub(crate) fn show_dialog(
		&self,
		parent: &gtk::Window,
		frame: &Frame,
		held: graphene::Rect,
		viewport: graphene::Size,
	) -> bool {
		let Shown::Dialog(window) = &self.shown else {
			return false;
		};
		let width = held.width().round() as i32;
		let height = held.height().round() as i32;
		self.host.set_extent(width, height);
		self.host.position(
			frame,
			graphene::Rect::new(-held.x(), -held.y(), viewport.width(), viewport.height()),
		);
		window.set_transient_for(Some(parent));
		window.set_default_size(width, height);

		if window.is_visible() {
			return false;
		}

		window.present();
		true
	}

	/// Shows a popover on `below` at `rect`, in that frame's coordinates.
	/// Layout has already put it beside its anchor. Returns whether this
	/// call showed it.
	pub(crate) fn show_popover(&self, below: &Frame, frame: &Frame, rect: graphene::Rect) -> bool {
		let Shown::Hanging(popover) = &self.shown else {
			return false;
		};

		if popover.parent().as_ref() != Some(below.upcast_ref()) {
			if let Some(old) = popover.parent().and_downcast::<Frame>() {
				old.unhang(popover);
			}

			below.hang(popover);
		}

		let width = rect.width().round() as i32;
		let height = rect.height().round() as i32;
		self.host.set_extent(width, height);
		self.host.position(
			frame,
			graphene::Rect::new(0.0, 0.0, rect.width(), rect.height()),
		);
		// GTK puts a popover below the rectangle it points to, centred on
		// it: a line as wide as the popover, along its top edge.
		popover.set_pointing_to(Some(&gdk::Rectangle::new(
			rect.x().round() as i32,
			rect.y().round() as i32 - 1,
			width,
			1,
		)));

		if popover.is_visible() {
			popover.present();
			return false;
		}

		let focus = popover.root().and_then(|root| root.focus());
		self.before
			.replace(focus.map(|widget| ObjectExt::downgrade(&widget)));
		popover.popup();
		true
	}

	/// Moves focus to the first control inside a popover. A dialog's window
	/// does that itself.
	pub(crate) fn focus_first(&self) {
		if let Shown::Hanging(popover) = &self.shown {
			popover.child_focus(gtk::DirectionType::TabForward);
		}
	}

	/// Takes the overlay off screen without asking anyone.
	pub(crate) fn hide(&self) {
		match &self.shown {
			Shown::Dialog(window) => window.set_visible(false),
			Shown::Hanging(popover) => {
				if !popover.is_visible() {
					return;
				}

				let inside = popover
					.root()
					.and_then(|root| root.focus())
					.is_some_and(|focus| focus.is_ancestor(popover));
				popover.popdown();
				let before = self.before.take().and_then(|before| before.upgrade());

				if inside && let Some(before) = before {
					before.grab_focus();
				}
			}
		}
	}

	/// Takes the overlay down for good, when its node is deleted.
	pub(crate) fn close(&self) {
		self.hide();

		match &self.shown {
			Shown::Dialog(window) => window.destroy(),
			Shown::Hanging(popover) => {
				if let Some(parent) = popover.parent().and_downcast::<Frame>() {
					parent.unhang(popover);
				}
			}
		}
	}
}
