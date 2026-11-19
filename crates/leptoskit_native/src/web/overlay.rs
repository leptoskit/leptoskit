//! Modals and popovers in the browser's top layer.
//!
//! A `Modal` is a `<dialog>` shown with `showModal()`, so the browser puts
//! it above the page, makes the page behind it inert, wraps Tab inside it,
//! and gives focus back when it closes. A `Popover` is an element with the
//! `popover` attribute shown with `showPopover()`, placed beside its anchor.
//!
//! The app owns whether an overlay is open. The element says what the app
//! wants in `data-lk-modal` or `data-lk-popover`, and [`sync_after_render`]
//! makes the browser agree after each render. When the browser wants to
//! close one (Escape, a press outside), the app hears
//! [`NativeEvent::Dismiss`] and decides.

use std::cell::Cell;
use std::sync::Arc;

use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::tachys::html::attribute::Attribute;
use wasm_bindgen::JsCast;

use super::Handlers;
use super::dispatch;
use crate::NativeEvent;
use crate::NativeKind;
use crate::Overlay;

thread_local! {
	/// Whether a render since the last sync held a modal or popover.
	static RENDERED: Cell<bool> = const { Cell::new(false) };
}

const MODAL: &str = "data-lk-modal";
const POPOVER: &str = "data-lk-popover";
const OPEN: &str = "open";

const fn state(open: bool) -> &'static str {
	if open { OPEN } else { "closed" }
}

/// The element of a `Modal` or `Popover`, whose own handlers are the last
/// in `chain`.
pub(super) fn element(
	kind: NativeKind,
	overlay: Option<Overlay>,
	children: Vec<AnyView>,
	chain: &[Handlers],
	attrs: impl Attribute + 'static,
) -> AnyView {
	let overlay = overlay.unwrap_or_default();
	let handlers = chain.last().cloned().unwrap_or_default();

	if kind == NativeKind::Modal {
		modal(overlay, children, handlers, attrs)
	} else {
		popover(overlay, children, handlers, attrs)
	}
}

/// A `<dialog>` the renderer shows as a modal while the app says it is open.
///
/// A server writes `<dialog open>` for a modal that starts open, so it is
/// on screen before any script runs. The browser's own client never writes
/// `open`: `showModal()` and `close()` own that attribute.
fn modal(
	overlay: Overlay,
	children: Vec<AnyView>,
	handlers: Handlers,
	attrs: impl Attribute + 'static,
) -> AnyView {
	RENDERED.set(true);

	let cancel = Arc::clone(&handlers);
	let close = Arc::clone(&handlers);
	let backdrop = handlers;

	html::dialog()
		.attr(MODAL, state(overlay.open))
		.attr("data-lk-initial-focus", overlay.initial_focus)
		.attr(
			"open",
			(cfg!(not(target_arch = "wasm32")) && overlay.open).then_some(""),
		)
		// Escape, and Back on Android. The dialog stays until the app
		// closes it.
		.on(ev::Custom::<web_sys::Event>::new("cancel"), move |event| {
			event.prevent_default();
			dispatch(&cancel, &NativeEvent::Dismiss, &event);
		})
		// A browser closes a dialog whose `cancel` was refused twice in a
		// row. The app hears that as one more request to dismiss.
		.on(ev::close, move |event| {
			if closed_by_browser(&event, MODAL) {
				dispatch(&close, &NativeEvent::Dismiss, &event);
			}
		})
		// Tab past the last control comes back to the first, and Shift+Tab
		// past the first goes to the last, instead of out to the browser.
		.on(ev::keydown, move |event| {
			#[cfg(target_arch = "wasm32")]
			browser::keep_tab_inside(&event);
			#[cfg(not(target_arch = "wasm32"))]
			let _ = event;
		})
		// The dialog covers the viewport, so a press on the dialog itself,
		// not on something inside it, is a press on the backdrop.
		.on(ev::click, move |event| {
			if event.target() == event.current_target() {
				dispatch(&backdrop, &NativeEvent::Dismiss, &event);
			}
		})
		.child(children)
		.add_any_attr(attrs)
		.into_any()
}

/// An element with `popover="auto"`, which the browser closes on Escape or a
/// press outside it. The app hears either as a request to dismiss.
fn popover(
	overlay: Overlay,
	children: Vec<AnyView>,
	handlers: Handlers,
	attrs: impl Attribute + 'static,
) -> AnyView {
	RENDERED.set(true);

	html::div()
		.attr("popover", "auto")
		.attr(POPOVER, state(overlay.open))
		.attr("data-lk-anchor", overlay.anchor)
		.attr("data-lk-placement", overlay.placement.as_str())
		.attr("data-lk-initial-focus", overlay.initial_focus)
		.on(ev::toggle, move |event| {
			if closed_by_browser(&event, POPOVER) {
				dispatch(&handlers, &NativeEvent::Dismiss, &event);
			}
		})
		.child(children)
		.add_any_attr(attrs)
		.into_any()
}

/// Whether the browser closed an overlay the app still wants open. A close
/// the renderer asked for finds the element already marked closed, or open
/// again.
fn closed_by_browser(event: &web_sys::Event, attribute: &str) -> bool {
	let Some(element) = event
		.target()
		.and_then(|target| target.dyn_into::<web_sys::Element>().ok())
	else {
		return false;
	};
	let showing = if attribute == MODAL {
		element.has_attribute("open")
	} else {
		element.matches(":popover-open").unwrap_or(false)
	};

	!showing && element.get_attribute(attribute).as_deref() == Some(OPEN)
}

/// Shows and hides overlays to match the last render, once its DOM is in
/// place. Does nothing when that render held no overlay.
pub(super) fn sync_after_render() {
	#[cfg_attr(
		not(target_arch = "wasm32"),
		expect(unused_variables, reason = "only a browser has overlays to show")
	)]
	let rendered = RENDERED.replace(false);

	#[cfg(target_arch = "wasm32")]
	if rendered {
		queue_microtask(browser::sync);
		// A screen that is still being mounted has no DOM at the microtask.
		let _ = request_animation_frame(browser::sync);
	}
}

#[cfg(target_arch = "wasm32")]
mod browser {
	use wasm_bindgen::JsCast;
	use wasm_bindgen::closure::Closure;

	use super::MODAL;
	use super::OPEN;
	use super::POPOVER;
	use crate::Placement;

	/// What takes focus in a popover that names no `initial_focus`.
	const FOCUSABLE: &str = "a[href],button:not([disabled]),input:not([disabled]),select:\
	                         not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex=\"\
	                         -1\"])";

	fn each<T: JsCast>(document: &web_sys::Document, selector: &str, mut visit: impl FnMut(T)) {
		let Ok(found) = document.query_selector_all(selector) else {
			return;
		};

		for index in 0..found.length() {
			if let Some(element) = found.item(index).and_then(|node| node.dyn_into::<T>().ok()) {
				visit(element);
			}
		}
	}

	pub(super) fn sync() {
		let Some(window) = web_sys::window() else {
			return;
		};
		let Some(document) = window.document() else {
			return;
		};

		each(
			&document,
			"dialog[data-lk-modal]",
			|dialog: web_sys::HtmlDialogElement| {
				let wanted = dialog.get_attribute(MODAL).as_deref() == Some(OPEN);
				let modal = dialog.matches(":modal").unwrap_or(false);

				if wanted && !modal {
					// A server-rendered `<dialog open>` is on screen but not
					// modal: reopen it in the top layer.
					if dialog.open() {
						dialog.close();
					}

					if dialog.show_modal().is_ok() {
						focus_initial(&document, &dialog, false);
					}
				} else if !wanted && dialog.open() {
					dialog.close();
				}
			},
		);

		let mut showing = false;

		each(
			&document,
			"[data-lk-popover]",
			|popover: web_sys::HtmlElement| {
				let wanted = popover.get_attribute(POPOVER).as_deref() == Some(OPEN);
				let shown = popover.matches(":popover-open").unwrap_or(false);
				let anchor = anchor(&document, &popover);

				// A press on the button a popover hangs from is not a press
				// outside it: the browser hides the popover as that button's
				// own action, after the app's handler has run.
				if let Some(button) = anchor
					.as_ref()
					.and_then(|anchor| anchor.dyn_ref::<web_sys::HtmlButtonElement>())
				{
					button.set_popover_target_element(Some(&popover));
					button.set_popover_target_action("hide");
				}

				if !wanted {
					if shown {
						let _ = popover.hide_popover();
					}

					return;
				}

				if !shown && popover.show_popover().is_err() {
					return;
				}

				showing = true;
				place(&document, &popover, anchor.as_ref());

				if !shown {
					focus_initial(&document, &popover, true);
				}
			},
		);

		if showing {
			follow_the_viewport(&window);
		}
	}

	fn anchor(
		document: &web_sys::Document,
		popover: &web_sys::Element,
	) -> Option<web_sys::Element> {
		popover
			.get_attribute("data-lk-anchor")
			.and_then(|id| document.get_element_by_id(&id))
	}

	/// Wraps Tab inside a modal: from its last control to its first, and
	/// with Shift from its first to its last. A focused element that is not
	/// a stop itself, such as a heading given focus, counts by its place.
	pub(super) fn keep_tab_inside(event: &web_sys::KeyboardEvent) {
		if event.key() != "Tab" || event.alt_key() || event.ctrl_key() || event.meta_key() {
			return;
		}
		let Some(dialog) = event
			.current_target()
			.and_then(|target| target.dyn_into::<web_sys::Element>().ok())
		else {
			return;
		};
		let Ok(found) = dialog.query_selector_all(FOCUSABLE) else {
			return;
		};
		let stops = (0..found.length())
			.filter_map(|index| found.item(index))
			.filter_map(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
			.filter(|element| {
				let size = element.get_bounding_client_rect();
				size.width() > 0.0 || size.height() > 0.0
			})
			.collect::<Vec<_>>();
		let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
			return;
		};
		let Some(active) = dialog
			.owner_document()
			.and_then(|document| document.active_element())
		else {
			return;
		};

		let target = if event.shift_key() {
			let before = active.is_same_node(Some(first))
				|| active.is_same_node(Some(&dialog))
				|| active.compare_document_position(first)
					& web_sys::Node::DOCUMENT_POSITION_FOLLOWING
					!= 0;
			before.then_some(last)
		} else {
			let after = active.is_same_node(Some(last))
				|| active.compare_document_position(last)
					& web_sys::Node::DOCUMENT_POSITION_PRECEDING
					!= 0;
			after.then_some(first)
		};

		if let Some(target) = target {
			event.prevent_default();
			let _ = target.focus();
		}
	}

	/// Moves focus to the element the overlay names. A modal with no such
	/// element keeps the browser's choice; a popover takes its first
	/// focusable element, since the browser leaves focus outside it.
	fn focus_initial(document: &web_sys::Document, overlay: &web_sys::Element, first: bool) {
		let named = overlay
			.get_attribute("data-lk-initial-focus")
			.and_then(|id| document.get_element_by_id(&id));
		let target = match named {
			Some(named) => Some(named),
			None if first => overlay.query_selector(FOCUSABLE).ok().flatten(),
			None => None,
		};

		if let Some(target) =
			target.and_then(|target| target.dyn_into::<web_sys::HtmlElement>().ok())
		{
			let _ = target.focus();
		}
	}

	/// Puts a showing popover beside its anchor, in viewport coordinates.
	fn place(
		document: &web_sys::Document,
		popover: &web_sys::HtmlElement,
		anchor: Option<&web_sys::Element>,
	) {
		let Some(root) = document.document_element() else {
			return;
		};
		let mut placement = popover
			.get_attribute("data-lk-placement")
			.and_then(|name| Placement::from_name(&name))
			.unwrap_or_default();
		let direction = web_sys::window()
			.and_then(|window| window.get_computed_style(popover).ok().flatten())
			.and_then(|style| style.get_property_value("direction").ok());

		if direction.as_deref() == Some("rtl") {
			placement = placement.mirrored();
		}

		let size = popover.get_bounding_client_rect();
		let anchor = anchor.map_or((0.0, 0.0, 0.0, 0.0), |anchor| {
			let rect = anchor.get_bounding_client_rect();

			(
				rect.left() as f32,
				rect.top() as f32,
				rect.width() as f32,
				rect.height() as f32,
			)
		});
		let (x, y) = placement.position(
			anchor,
			(size.width() as f32, size.height() as f32),
			(root.client_width() as f32, root.client_height() as f32),
		);
		let style = popover.style();
		let _ = style.set_property("left", &format!("{x}px"));
		let _ = style.set_property("top", &format!("{y}px"));
	}

	/// Keeps showing popovers beside their anchors while the page scrolls
	/// or the window changes size. The listeners are added once.
	fn follow_the_viewport(window: &web_sys::Window) {
		thread_local! {
			static FOLLOW: Closure<dyn Fn()> = Closure::new(|| {
				let Some(document) = web_sys::window().and_then(|window| window.document()) else {
					return;
				};

				each(&document, "[data-lk-popover]:popover-open", |popover: web_sys::HtmlElement| {
					place(&document, &popover, anchor(&document, &popover).as_ref());
				});
			});
			static FOLLOWING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
		}

		if FOLLOWING.replace(true) {
			return;
		}

		FOLLOW.with(|follow| {
			let options = web_sys::AddEventListenerOptions::new();
			options.set_passive(true);
			options.set_capture(true);

			for event in ["scroll", "resize"] {
				let _ = window.add_event_listener_with_callback_and_add_event_listener_options(
					event,
					follow.as_ref().unchecked_ref(),
					&options,
				);
			}
		});
	}
}
