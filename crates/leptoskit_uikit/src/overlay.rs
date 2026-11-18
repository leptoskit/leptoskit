//! Sheets and popovers: the UIKit presentations behind `Modal` and `Popover`.
//!
//! A `Modal` is a form sheet and a `Popover` is a popover hanging from its
//! anchor's view, each in a view controller of its own that UIKit presents
//! over the one holding the rest of the tree. Neither one's view sits in its
//! parent's view: UIKit owns where an overlay goes, and the backend only
//! says when it shows, what it holds and how large that is.

use std::rc::Rc;

use leptoskit_native::NativeEvent;
use leptoskit_native::Placement;
use leptoskit_yoga::NodeId;
use objc2::DefinedClass;
use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2::define_class;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::runtime::NSObject;
use objc2::runtime::NSObjectProtocol;
use objc2::runtime::ProtocolObject;
use objc2::sel;
use objc2_core_foundation::CGPoint;
use objc2_core_foundation::CGRect;
use objc2_core_foundation::CGSize;
use objc2_foundation::NSArray;
use objc2_foundation::NSString;
use objc2_ui_kit::UIAdaptivePresentationControllerDelegate;
use objc2_ui_kit::UIColor;
use objc2_ui_kit::UIKeyCommand;
use objc2_ui_kit::UIKeyInputEscape;
use objc2_ui_kit::UIKeyModifierFlags;
use objc2_ui_kit::UIModalPresentationStyle;
use objc2_ui_kit::UIPopoverArrowDirection;
use objc2_ui_kit::UIPopoverPresentationControllerDelegate;
use objc2_ui_kit::UIPresentationController;
use objc2_ui_kit::UIResponder;
use objc2_ui_kit::UITraitCollection;
use objc2_ui_kit::UIUserInterfaceStyle;
use objc2_ui_kit::UIView;
use objc2_ui_kit::UIViewController;

use crate::events::EventSink;

/// Which node an overlay speaks for, and where its request to close goes.
pub(crate) struct Dismissal {
	node: NodeId,
	sink: Rc<EventSink>,
	/// Whether the overlay is a popover, not a sheet.
	popover: bool,
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
	// - `UIViewController` is initialized through its designated
	//   initializer, which `OverlayController::new` calls.
	// - The class does not implement `Drop`; its ivars are dropped by objc2.
	#[unsafe(super(UIViewController, UIResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = Dismissal]
	/// The controller UIKit presents for one overlay. It keeps what the
	/// overlay holds inside the safe area, and turns Escape and the
	/// `VoiceOver` escape gesture into a request to close.
	pub(crate) struct OverlayController;

	impl OverlayController {
		#[unsafe(method(viewDidLayoutSubviews))]
		fn layout(&self) {
			// SAFETY: `UIViewController` implements this lifecycle selector.
			unsafe {
				let _: () = msg_send![super(self), viewDidLayoutSubviews];
			}
			let Some(view) = self.view() else {
				return;
			};
			let bounds = view.bounds().size;
			let insets = view.safeAreaInsets();

			for holder in &view.subviews() {
				let size = holder.frame().size;
				// A popover's arrow takes part of the view, which the safe
				// area leaves out. A sheet can be wider than what it holds.
				let x = if self.ivars().popover {
					insets.left
				} else {
					((bounds.width - size.width) / 2.0).max(insets.left)
				};
				holder.setFrame(CGRect::new(CGPoint::new(x, insets.top), size));
			}
		}

		#[unsafe(method(accessibilityPerformEscape))]
		fn accessibility_escape(&self) -> bool {
			self.ivars().request();
			true
		}

		#[unsafe(method_id(keyCommands))]
		fn key_commands(&self) -> Option<Retained<NSArray<UIKeyCommand>>> {
			// SAFETY: `dismissRequested:` is implemented below, and UIKit
			// exports the Escape input string.
			let escape = unsafe {
				UIKeyCommand::keyCommandWithInput_modifierFlags_action(
					UIKeyInputEscape,
					UIKeyModifierFlags::empty(),
					sel!(dismissRequested:),
					self.mtm(),
				)
			};
			Some(NSArray::from_retained_slice(&[escape]))
		}

		#[unsafe(method(dismissRequested:))]
		fn dismiss_requested(&self, _sender: Option<&AnyObject>) {
			self.ivars().request();
		}
	}

	unsafe impl NSObjectProtocol for OverlayController {}
);

impl OverlayController {
	fn new(mtm: MainThreadMarker, dismissal: Dismissal) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(dismissal);
		// SAFETY: A nil nib name and bundle initialize a programmatically
		// built `UIViewController` through its designated initializer.
		unsafe {
			msg_send![super(this), initWithNibName: Option::<&NSString>::None, bundle: Option::<&objc2_foundation::NSBundle>::None]
		}
	}
}

define_class!(
	// SAFETY:
	// - `NSObject` has no subclassing requirements.
	// - The class does not implement `Drop`; its ivars are dropped by objc2.
	#[unsafe(super(NSObject))]
	#[thread_kind = MainThreadOnly]
	#[ivars = Dismissal]
	/// Turns UIKit's own wish to take an overlay down (a swipe on a sheet, a
	/// press outside a popover) into a request the app answers.
	pub(crate) struct PresentationDelegate;

	unsafe impl NSObjectProtocol for PresentationDelegate {}

	unsafe impl UIAdaptivePresentationControllerDelegate for PresentationDelegate {
		#[unsafe(method(presentationControllerShouldDismiss:))]
		fn should_dismiss(&self, _presentation: &UIPresentationController) -> bool {
			// A press outside a popover ends here. A sheet's swipe goes on
			// to `presentationControllerDidAttemptToDismiss:`.
			if self.ivars().popover {
				self.ivars().request();
			}

			// The app closes it by building the next tree without it open.
			false
		}

		#[unsafe(method(presentationControllerDidAttemptToDismiss:))]
		fn did_attempt_to_dismiss(&self, _presentation: &UIPresentationController) {
			if !self.ivars().popover {
				self.ivars().request();
			}
		}

		#[unsafe(method(adaptivePresentationStyleForPresentationController:traitCollection:))]
		fn adaptive_style(
			&self,
			_presentation: &UIPresentationController,
			_traits: &UITraitCollection,
		) -> UIModalPresentationStyle {
			// A popover stays a popover on a phone, and does not turn into a
			// sheet that covers the control it hangs from.
			UIModalPresentationStyle::None
		}
	}

	unsafe impl UIPopoverPresentationControllerDelegate for PresentationDelegate {}
);

impl PresentationDelegate {
	fn new(mtm: MainThreadMarker, dismissal: Dismissal) -> Retained<Self> {
		let this = Self::alloc(mtm).set_ivars(dismissal);
		// SAFETY: `init` is `NSObject`'s designated initializer.
		unsafe { msg_send![super(this), init] }
	}
}

/// The view controller whose view holds `view`, found along the responder
/// chain.
pub(crate) fn controller_of(view: &UIView) -> Option<Retained<UIViewController>> {
	let mut responder = view.nextResponder();

	while let Some(current) = responder {
		match current.downcast::<UIViewController>() {
			Ok(controller) => return Some(controller),
			Err(other) => responder = other.nextResponder(),
		}
	}

	None
}

/// One overlay's presentation: the controller UIKit presents, and the view
/// inside it that holds the overlay's own view.
pub(crate) struct Presented {
	controller: Retained<OverlayController>,
	/// The presentation holds its delegate weakly.
	delegate: Retained<PresentationDelegate>,
	/// Sized to what the overlay shows, and placed by the controller.
	holder: Retained<UIView>,
	popover: bool,
}

impl Presented {
	/// A presentation for `node` that holds `view` and is not yet shown.
	pub(crate) fn new(
		mtm: MainThreadMarker,
		node: NodeId,
		sink: &Rc<EventSink>,
		popover: bool,
		view: &UIView,
	) -> Self {
		let dismissal = || {
			Dismissal {
				node,
				sink: Rc::clone(sink),
				popover,
			}
		};
		let controller = OverlayController::new(mtm, dismissal());
		let delegate = PresentationDelegate::new(mtm, dismissal());
		let holder = UIView::initWithFrame(UIView::alloc(mtm), CGRect::ZERO);
		holder.addSubview(view);
		controller.setModalPresentationStyle(if popover {
			UIModalPresentationStyle::Popover
		} else {
			UIModalPresentationStyle::FormSheet
		});

		if let Some(root) = controller.view() {
			root.addSubview(&holder);

			if !popover {
				root.setBackgroundColor(Some(&UIColor::systemBackgroundColor()));
			}
		}

		Self {
			controller,
			delegate,
			holder,
			popover,
		}
	}

	/// The controller UIKit presents.
	pub(crate) fn controller(&self) -> &UIViewController {
		&self.controller
	}

	/// Whether UIKit is presenting the overlay.
	pub(crate) fn is_shown(&self) -> bool {
		self.controller.presentingViewController().is_some() && !self.controller.isBeingDismissed()
	}

	/// Follows the app's explicit colour scheme.
	pub(crate) fn set_appearance(&self, style: UIUserInterfaceStyle) {
		self.controller.setOverrideUserInterfaceStyle(style);
	}

	/// Sizes the presentation to `size`, which is what it shows.
	pub(crate) fn resize(&self, size: CGSize) {
		if self.holder.frame().size != size {
			self.holder
				.setFrame(CGRect::new(self.holder.frame().origin, size));

			if let Some(root) = self.controller.view() {
				root.setNeedsLayout();
			}
		}

		if self.controller.preferredContentSize() != size {
			self.controller.setPreferredContentSize(size);
		}
	}

	/// Shows a modal's sheet over `presenter`, sized to `content`: the part
	/// of the modal's own `view` that holds something. Returns whether this
	/// call presented it.
	pub(crate) fn show_sheet(
		&self,
		view: &UIView,
		content: CGRect,
		presenter: &UIViewController,
		animated: bool,
	) -> bool {
		// The modal covers the viewport; the sheet shows only what it holds.
		view.setFrame(CGRect::new(
			CGPoint::new(-content.origin.x, -content.origin.y),
			view.frame().size,
		));
		self.resize(content.size);
		self.present(presenter, animated)
	}

	/// Shows a popover hanging from `anchor`, over `presenter`. Returns
	/// whether this call presented it.
	pub(crate) fn show_popover(
		&self,
		anchor: &UIView,
		placement: Placement,
		presenter: &UIViewController,
		animated: bool,
	) -> bool {
		if self.is_shown() {
			return false;
		}

		if let Some(popover) = self.controller.popoverPresentationController() {
			popover.setSourceView(Some(anchor));
			popover.setSourceRect(anchor.bounds());
			// The arrow points at the anchor, from the side the popover is on.
			popover.setPermittedArrowDirections(if placement.is_above() {
				UIPopoverArrowDirection::Down
			} else {
				UIPopoverArrowDirection::Up
			});
		}

		self.present(presenter, animated)
	}

	fn present(&self, presenter: &UIViewController, animated: bool) -> bool {
		if self.is_shown() || self.controller.isBeingDismissed() {
			return false;
		}

		// UIKit makes a new presentation controller for each presentation,
		// so the delegate is set each time.
		if self.popover {
			if let Some(popover) = self.controller.popoverPresentationController() {
				// SAFETY: the delegate outlives the presentation: `self`
				// retains it, and `hide` runs before `self` is dropped.
				unsafe {
					popover.setDelegate(Some(ProtocolObject::from_ref(&*self.delegate)));
				}
			}
		} else if let Some(presentation) = self.controller.presentationController() {
			// SAFETY: as above.
			unsafe {
				presentation.setDelegate(Some(ProtocolObject::from_ref(&*self.delegate)));
			}
		}

		presenter.presentViewController_animated_completion(&self.controller, animated, None);
		self.is_shown()
	}

	/// Takes the overlay down, along with anything presented over it.
	pub(crate) fn hide(&self, animated: bool) {
		if self.controller.isBeingDismissed() {
			return;
		}

		// Asking the presenter, not the controller itself, takes this one
		// down even when it has presented something of its own.
		if let Some(presenter) = self.controller.presentingViewController() {
			presenter.dismissViewControllerAnimated_completion(animated, None);
		}
	}

	/// Asks to close as the `VoiceOver` escape gesture and the Escape key do.
	pub(crate) fn cancel(&self) {
		self.controller.ivars().request();
	}
}
