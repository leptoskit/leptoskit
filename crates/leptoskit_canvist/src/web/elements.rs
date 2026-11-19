//! Editors in `CanvistEditor` elements.
//!
//! leptoskit renders `CanvistEditor(id)` as an empty
//! `<div data-lk-canvist-editor="id">`. Once an app has asked an editor for
//! its id, this module watches the page with a `MutationObserver`. It
//! attaches the editor to the container that names it when the container
//! appears, follows the settings the element carries, and detaches when the
//! container leaves, so the app never calls [`attach`] itself and the
//! interface code never links this crate.
//!
//! The same pattern puts a 3D scene in a `Scene3d` element.

use std::cell::RefCell;

use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use web_sys::Document;
use web_sys::Element;
use web_sys::HtmlElement;
use web_sys::MutationObserver;
use web_sys::MutationObserverInit;
use web_sys::MutationRecord;

use super::Attachment;
use super::attach;
use super::host;
use crate::CanvistState;

/// Names the editor a container shows.
const EDITOR: &str = "data-lk-canvist-editor";
/// The element's placeholder.
const PLACEHOLDER: &str = "data-lk-placeholder";
/// Present while the element is read-only.
const READ_ONLY: &str = "data-lk-read-only";
/// Present while the element asks for keyboard focus.
const FOCUS: &str = "data-lk-focus";
/// The styles leptoskit generated for an element.
const STYLE_SCOPE: &str = "data-lk-style";
/// The app's choice of light or dark, on the page's root element.
const APPEARANCE: &str = "data-lk-appearance";

/// The attributes whose changes can concern an editor's container.
const WATCHED: [&str; 10] = [
	EDITOR,
	PLACEHOLDER,
	READ_ONLY,
	FOCUS,
	STYLE_SCOPE,
	APPEARANCE,
	"class",
	"aria-label",
	"aria-labelledby",
	"aria-disabled",
];

/// What a container's element says about its editor, as last read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Settings {
	placeholder: Option<String>,
	read_only: bool,
	focus: bool,
	/// The container's `aria-label` and `aria-labelledby`.
	name: (Option<String>, Option<String>),
	/// Everything that can change how the container's text looks without
	/// resizing it: its classes, its generated styles, and the app's choice
	/// of light or dark.
	look: (Option<String>, Option<String>, Option<String>),
}

impl Settings {
	fn read(container: &HtmlElement, appearance: Option<&str>) -> Self {
		let attribute = |name| container.get_attribute(name);

		Self {
			placeholder: attribute(PLACEHOLDER),
			// A disabled element cannot be edited either. It still takes
			// focus, so its text can be selected and copied.
			read_only: container.has_attribute(READ_ONLY)
				|| attribute("aria-disabled").as_deref() == Some("true"),
			focus: container.has_attribute(FOCUS),
			name: (attribute("aria-label"), attribute("aria-labelledby")),
			look: (
				attribute("class"),
				attribute(STYLE_SCOPE),
				appearance.map(str::to_owned),
			),
		}
	}
}

/// An editor in the container that names it.
struct Mounted {
	container: HtmlElement,
	state: CanvistState,
	/// `None` when the browser could not show the editor. It is not tried
	/// again for this container: the reason does not change.
	attachment: Option<Attachment>,
	settings: Settings,
}

impl Mounted {
	/// Put `state`'s editor in `container`, set up as `settings` says.
	fn new(container: HtmlElement, state: CanvistState, settings: Settings) -> Self {
		let mut mounted = Self {
			container,
			state,
			attachment: None,
			settings: Settings::default(),
		};
		// Before the first frame, so it is painted once.
		mounted.apply_options(&settings);
		mounted.attachment = attach(&mounted.container, state)
			.inspect_err(host::report)
			.ok();

		if settings.focus {
			state.focus();
		}

		mounted.settings = settings;

		mounted
	}

	/// Whether the container is still in the page and still names this
	/// editor, and the editor has not been disposed.
	fn current(&self) -> bool {
		self.container.is_connected()
			&& named(&self.container) == Some(self.state.key())
			&& self.state.is_alive()
	}

	/// The element changed: bring the editor in line with `settings`.
	fn follow(&mut self, settings: Settings) {
		if settings == self.settings {
			return;
		}

		self.apply_options(&settings);

		if let Some(attachment) = &self.attachment {
			if settings.name != self.settings.name {
				host::with(self.state.key(), Some(attachment.serial), |host| {
					host.apply_name();
				});
			}

			if settings.look != self.settings.look {
				attachment.refresh_theme();
			}
		}

		// Focus is asked for once each time the element starts asking.
		if settings.focus && !self.settings.focus {
			self.state.focus();
		}

		self.settings = settings;
	}

	/// The placeholder and read-only, where the element's differ from what
	/// it last said. An element that sets neither leaves alone what the app
	/// set on the handle.
	fn apply_options(&self, settings: &Settings) {
		if settings.placeholder != self.settings.placeholder {
			self.state
				.set_placeholder(settings.placeholder.as_deref().unwrap_or_default());
		}

		if settings.read_only != self.settings.read_only {
			self.state.set_read_only(settings.read_only);
		}
	}
}

/// The page's watcher and the editors it has mounted.
struct Watch {
	_observer: MutationObserver,
	_callback: Closure<dyn FnMut(js_sys::Array)>,
	mounted: Vec<Mounted>,
}

thread_local! {
	static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
}

/// Start watching the page for `CanvistEditor` containers. Later calls do
/// nothing.
pub(crate) fn watch() {
	if WATCH.with_borrow(Option::is_some) {
		return;
	}

	let Some(root) = document().and_then(|document| document.document_element()) else {
		return;
	};
	let callback = Closure::<dyn FnMut(js_sys::Array)>::new(|records: js_sys::Array| {
		if concerns_the_page(&records) {
			rescan();
		}
	});
	let Ok(observer) = MutationObserver::new(callback.as_ref().unchecked_ref()) else {
		return;
	};
	let options = MutationObserverInit::new();
	options.set_child_list(true);
	options.set_subtree(true);
	options.set_attributes(true);
	options.set_attribute_filter(
		&WATCHED
			.into_iter()
			.map(JsValue::from_str)
			.collect::<js_sys::Array>(),
	);

	if observer.observe_with_options(&root, &options).is_err() {
		return;
	}

	WATCH.with_borrow_mut(|watch| {
		*watch = Some(Watch {
			_observer: observer,
			_callback: callback,
			mounted: Vec::new(),
		});
	});
	// Containers that are already in the page, as on a server-rendered one,
	// change nothing for the observer to report. Look for them once whatever
	// is building the page right now has finished: hydration renames each
	// container to the page's own editor first.
	wasm_bindgen_futures::spawn_local(async { rescan() });
}

/// Whether any of `records` happened outside the elements an editor's host
/// built. The host's own changes, such as refreshing the hidden copy of the
/// text after typing, never add or change a container.
fn concerns_the_page(records: &js_sys::Array) -> bool {
	records.iter().any(|record| {
		let target = record.unchecked_into::<MutationRecord>().target();
		let element = target.and_then(|node| {
			node.dyn_ref::<Element>()
				.cloned()
				.or_else(|| node.parent_element())
		});

		!element.is_some_and(|element| matches!(element.closest("[data-canvist]"), Ok(Some(_))))
	})
}

/// Detach editors whose containers left or changed editor, attach editors
/// to containers that appeared, and follow the settings of the rest.
fn rescan() {
	let Some(document) = document() else {
		return;
	};
	let Ok(found) = document.query_selector_all("[data-lk-canvist-editor]") else {
		return;
	};
	// Attaching changes the page, so the list is worked on outside the
	// registry.
	let Some(known) = WATCH.with_borrow_mut(|watch| {
		watch
			.as_mut()
			.map(|watch| std::mem::take(&mut watch.mounted))
	}) else {
		return;
	};
	let (mut mounted, gone): (Vec<_>, Vec<_>) = known.into_iter().partition(Mounted::current);
	// Dropping an attachment takes its editor off the page.
	drop(gone);

	let appearance = document
		.document_element()
		.and_then(|root| root.get_attribute(APPEARANCE));

	for index in 0..found.length() {
		let Some(container) = found
			.item(index)
			.and_then(|node| node.dyn_into::<HtmlElement>().ok())
		else {
			continue;
		};
		let settings = Settings::read(&container, appearance.as_deref());

		if let Some(existing) = mounted
			.iter_mut()
			.find(|mounted| mounted.container == container)
		{
			existing.follow(settings);
		} else if let Some(state) = named(&container)
			.map(CanvistState::from_id)
			.filter(|state| state.is_alive())
		{
			mounted.push(Mounted::new(container, state, settings));
		}
	}

	WATCH.with_borrow_mut(|watch| {
		if let Some(watch) = watch {
			// Nothing above runs another scan, so nothing was added here
			// in the meantime.
			watch.mounted = mounted;
		}
	});
}

/// The editor a container names.
fn named(container: &HtmlElement) -> Option<u32> {
	container.get_attribute(EDITOR)?.parse().ok()
}

fn document() -> Option<Document> {
	web_sys::window().and_then(|window| window.document())
}
