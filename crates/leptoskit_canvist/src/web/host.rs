//! The editor in a page: the element that hosts the canvas, kept sized to
//! the visible part of the document, with scrolling, resizing, zoom, and
//! font loading turned into repaints on the next animation frame.
//!
//! Ported from canvist's `packages/canvist-web/src/editor.ts`.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use canvist_layout::Rect;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::AbortController;
use web_sys::AddEventListenerOptions;
use web_sys::Document;
use web_sys::Event;
use web_sys::EventTarget;
use web_sys::FocusOptions;
use web_sys::FontFaceSetLoadEvent;
use web_sys::HtmlCanvasElement;
use web_sys::HtmlElement;
use web_sys::HtmlTextAreaElement;
use web_sys::ResizeObserver;
use web_sys::ResizeObserverBoxOptions;
use web_sys::ResizeObserverEntry;
use web_sys::ResizeObserverOptions;
use web_sys::ResizeObserverSize;
use web_sys::Window;

use super::device;
use super::dom;
use super::editor::Editor;
use super::editor::Tailoring;
use super::fonts;
use super::fonts::FontWatcher;
use super::input;
use super::mirror::Mirror;
use super::paint::Surface;
use super::theme::ThemeReader;
use crate::CanvistError;
use crate::CanvistState;
use crate::Change;
use crate::session;
use crate::session::Session;
use crate::session::Shared;

/// How the caret blinks: on and off for this long each.
const CARET_BLINK_MS: i32 = 530;

/// How long the mirror waits for an idle moment before refreshing anyway.
const MIRROR_IDLE_MS: i32 = 500;

/// The canvas sticks to the top of the scrolling area and is only as tall
/// as the visible part of the document; the spacer after it holds the rest
/// of the document's height.
const CANVAS_STYLE: &str = "display:block;position:sticky;top:0;width:100%;height:0px;cursor:text";

/// The transparent text field that holds focus and receives text. It
/// follows the caret so input methods open their windows there. Its font is
/// 16px because iOS zooms the page into any smaller focused field.
const INPUT_STYLE: &str = "position:absolute;left:0;top:0;width:1px;height:1em;padding:0;border:0;\
                           margin:0;outline:none;resize:none;overflow:hidden;white-space:pre;\
                           opacity:0;caret-color:transparent;pointer-events:none;font-size:16px";

/// The accessible name of an editor whose container has no `aria-label`.
const DEFAULT_LABEL: &str = "Editor";

thread_local! {
	/// The host drawing each attached editor, by its handle's id.
	static HOSTS: RefCell<HashMap<u64, Rc<Host>>> = RefCell::new(HashMap::new());

	/// Tells one attachment of a handle from the next.
	static NEXT_SERIAL: Cell<u64> = const { Cell::new(1) };
}

/// A DOM event listener kept alive for as long as it is registered.
type Callback = Closure<dyn FnMut(Event)>;

/// What a resize observer calls with its entries.
type ResizeCallback = Closure<dyn FnMut(js_sys::Array)>;

/// What a listener does with an event.
pub(super) type Handler = fn(&Rc<Host>, &Event);

/// Whose scrolling moves the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scroller {
	/// The container scrolls: it has a height and `overflow: auto`.
	Own,
	/// The container grows with the document and the page scrolls.
	Page,
}

/// What of the container's box the canvas is placed by.
#[derive(Clone, Copy, Debug)]
struct Geometry {
	scroller: Scroller,
	/// Padding above the canvas, which document coordinates start below.
	padding_top: f64,
}

/// The callbacks the host hands to timers, built once.
struct Timers {
	frame: Closure<dyn FnMut()>,
	blink: Closure<dyn FnMut()>,
	mirror: Closure<dyn FnMut()>,
}

/// A mirror refresh that is waiting.
#[derive(Clone, Copy)]
enum Waiting {
	Idle(u32),
	Timeout(i32),
}

/// How a listener is registered.
#[derive(Clone, Copy, Default)]
pub(super) struct Listen {
	/// The listener never cancels the event, so the browser need not wait
	/// for it before scrolling.
	passive: bool,
	/// Remove the listener after its first event.
	once: bool,
}

/// One editor in one container.
pub(super) struct Host {
	serial: u64,
	pub(super) session: Shared,
	pub(super) window: Window,
	document: Document,
	container: HtmlElement,
	pub(super) canvas: HtmlCanvasElement,
	spacer: HtmlElement,
	pub(super) textarea: HtmlTextAreaElement,
	mirror: Option<Mirror>,
	/// Removes every DOM listener at once.
	abort: AbortController,
	pub(super) editor: RefCell<Editor>,
	theme: RefCell<ThemeReader>,
	fonts: Option<FontWatcher>,
	callbacks: RefCell<Vec<Callback>>,
	timers: RefCell<Option<Timers>>,
	resize: RefCell<Option<(ResizeObserver, ResizeCallback)>>,
	/// The animation frame a repaint is waiting for.
	frame: Cell<Option<i32>>,
	blink: Cell<Option<i32>>,
	blink_on: Cell<bool>,
	mirror_wait: Cell<Option<Waiting>>,
	/// The container's content width, which the document wraps to.
	width: Cell<f64>,
	/// The document's height as of the last frame.
	height: Cell<f32>,
	/// The canvas's CSS height: the visible part of the document.
	canvas_height: Cell<f64>,
	pub(super) focused: Cell<bool>,
	/// An input method is composing text, which it owns until it ends.
	pub(super) composing: Cell<bool>,
	/// The pointer is pressed and selecting.
	pub(super) pressed: Cell<bool>,
	pub(super) last_press: Cell<input::Press>,
	/// The container was `position: static` and the host changed that.
	positioned: Cell<bool>,
	pub(super) detached: Cell<bool>,
}

/// Put `state`'s editor in `container`, replacing any attachment it had.
/// Returns the attachment's serial.
pub(super) fn attach(container: &HtmlElement, state: CanvistState) -> Result<u64, CanvistError> {
	let session = state.session().ok_or(CanvistError::Disposed)?;
	detach(state.id(), None);
	let host = Host::new(container, session)?;
	HOSTS.with(|hosts| hosts.borrow_mut().insert(state.id(), Rc::clone(&host)));
	host.start();

	Ok(host.serial)
}

/// Take the editor with `id` out of the page: any attachment, or only the
/// one with `serial`.
pub(super) fn detach(id: u64, serial: Option<u64>) {
	let host = HOSTS.with(|hosts| {
		let mut hosts = hosts.borrow_mut();
		let current = hosts.get(&id)?.serial;
		serial
			.is_none_or(|serial| serial == current)
			.then(|| hosts.remove(&id))
			.flatten()
	});

	if let Some(host) = host {
		host.detach();
	}
}

/// Run `run` with the host of the editor with `id`, when it is attached.
pub(super) fn with(id: u64, serial: Option<u64>, run: impl FnOnce(&Rc<Host>)) {
	// The registry is released before `run`, which may attach or detach.
	let host = HOSTS.with(|hosts| hosts.borrow().get(&id).cloned());

	if let Some(host) = host.filter(|host| serial.is_none_or(|serial| serial == host.serial)) {
		run(&host);
	}
}

impl Host {
	fn new(container: &HtmlElement, session: Shared) -> Result<Rc<Self>, CanvistError> {
		let window = web_sys::window().ok_or_else(|| unavailable("there is no window"))?;
		let document = container
			.owner_document()
			.ok_or_else(|| unavailable("the container is in no document"))?;
		let create = |tag, css| {
			dom::element(&document, tag, css)
				.ok_or_else(|| unavailable("the page cannot create elements"))
		};
		let canvas = create("canvas", CANVAS_STYLE)?
			.dyn_into::<HtmlCanvasElement>()
			.map_err(|_| unavailable("the page has no canvas"))?;
		let textarea = create("textarea", INPUT_STYLE)?
			.dyn_into::<HtmlTextAreaElement>()
			.map_err(|_| unavailable("the page has no text field"))?;
		let spacer = create("div", "")?;
		let label = container
			.get_attribute("aria-label")
			.filter(|label| !label.trim().is_empty())
			.unwrap_or_else(|| DEFAULT_LABEL.to_owned());
		let mirror = Mirror::new(&document, &label);
		let mut theme = ThemeReader::new(&document);
		let first_theme = theme
			.read(&window, &document, container)
			.ok_or_else(|| unavailable("the container's styles cannot be read"))?;
		let tailoring = Tailoring {
			// One screen ahead, so scrolling a little never waits on
			// measurement.
			overscan: inner_height(&window) as f32,
			chromium_breaks: device::chromium(&window),
		};
		let editor = Editor::new(
			canvas.clone(),
			&document,
			&session::lock(&session).doc,
			first_theme,
			tailoring,
		)
		.map_err(|error| unavailable(&describe(&error)))?;
		let abort = AbortController::new().map_err(|error| unavailable(&describe(&error)))?;

		let _ = canvas.set_attribute("data-canvist", "canvas");
		let _ = spacer.set_attribute("data-canvist", "spacer");
		let _ = spacer.set_attribute("aria-hidden", "true");
		prepare_input(&textarea, &label);

		// Absolutely positioned children are placed against the container.
		let positioned = dom::computed(&window, container)
			.is_some_and(|style| dom::property(&style, "position") == "static")
			&& container
				.style()
				.set_property("position", "relative")
				.is_ok();

		// The text field comes right after the canvas, so screen readers meet
		// the field before the document's mirror.
		let children: [Option<&web_sys::Node>; 4] = [
			Some(&canvas),
			Some(&textarea),
			Some(&spacer),
			mirror.as_ref().map(|mirror| mirror.element().as_ref()),
		];

		for child in children.into_iter().flatten() {
			container
				.append_child(child)
				.map_err(|error| unavailable(&describe(&error)))?;
		}

		Ok(Rc::new(Self {
			serial: NEXT_SERIAL.with(|next| next.replace(next.get() + 1)),
			session,
			fonts: document.fonts().dyn_into().ok().map(FontWatcher::new),
			width: Cell::new(content_width(&window, container)),
			window,
			document,
			container: container.clone(),
			canvas,
			spacer,
			textarea,
			mirror,
			abort,
			editor: RefCell::new(editor),
			theme: RefCell::new(theme),
			callbacks: RefCell::new(Vec::new()),
			timers: RefCell::new(None),
			resize: RefCell::new(None),
			frame: Cell::new(None),
			blink: Cell::new(None),
			blink_on: Cell::new(true),
			mirror_wait: Cell::new(None),
			height: Cell::new(0.0),
			canvas_height: Cell::new(0.0),
			focused: Cell::new(false),
			composing: Cell::new(false),
			pressed: Cell::new(false),
			last_press: Cell::new(input::Press::default()),
			positioned: Cell::new(positioned),
			detached: Cell::new(false),
		}))
	}

	/// Start listening, and paint the first frame.
	fn start(self: &Rc<Self>) {
		self.build_timers();
		input::listen(self);
		self.listen_for_viewport_changes();
		self.observe_size();
		self.watch_scale();
		self.snap_to_device();
		self.apply_options();
		self.render();
		// Not before first paint: a long document takes a moment to copy.
		self.schedule_mirror();
	}

	/// Take the editor out of the page: its elements, listeners, observers
	/// and timers. The container is left as it was found.
	fn detach(&self) {
		if self.detached.replace(true) {
			return;
		}

		self.abort.abort();
		self.cancel_frame();
		self.cancel_blink();
		self.cancel_mirror();

		if let Some((observer, _)) = self.resize.borrow_mut().take() {
			observer.disconnect();
		}

		self.canvas.remove();
		self.textarea.remove();
		self.spacer.remove();

		if let Some(mirror) = &self.mirror {
			mirror.element().remove();
		}

		if self.positioned.get() {
			let _ = self.container.style().remove_property("position");
		}
	}

	/// Call `handler` for every `kind` event on `target`, until the host is
	/// detached.
	pub(super) fn on(
		self: &Rc<Self>,
		target: &EventTarget,
		kind: &str,
		handler: Handler,
		how: Listen,
	) {
		let weak = Rc::downgrade(self);
		let callback = Callback::new(move |event: Event| {
			if let Some(host) = weak.upgrade().filter(|host| !host.detached.get()) {
				handler(&host, &event);
			}
		});
		let options = AddEventListenerOptions::new();
		options.set_signal(&self.abort.signal());

		if how.passive {
			options.set_passive(true);
		}

		if how.once {
			options.set_once(true);
		}

		let _ = target.add_event_listener_with_callback_and_add_event_listener_options(
			kind,
			callback.as_ref().unchecked_ref(),
			&options,
		);
		self.callbacks.borrow_mut().push(callback);
	}

	fn build_timers(self: &Rc<Self>) {
		let timer = |run: fn(&Rc<Self>)| {
			let weak = Rc::downgrade(self);
			Closure::<dyn FnMut()>::new(move || {
				if let Some(host) = weak.upgrade().filter(|host| !host.detached.get()) {
					run(&host);
				}
			})
		};

		*self.timers.borrow_mut() = Some(Timers {
			frame: timer(|host| {
				host.frame.set(None);
				host.render();
			}),
			blink: timer(|host| {
				host.blink.set(None);
				let on = !host.blink_on.get();
				host.blink_on.set(on);

				if let Ok(mut editor) = host.editor.try_borrow_mut() {
					editor.set_caret_on(on);
				}

				host.invalidate();
				host.schedule_blink();
			}),
			mirror: timer(|host| {
				host.mirror_wait.set(None);
				host.refresh_mirror();
			}),
		});
	}

	/// Repaint on the next frame whenever the document scrolls, the window
	/// resizes, a font arrives, or the colour scheme flips.
	fn listen_for_viewport_changes(self: &Rc<Self>) {
		let passive = Listen {
			passive: true,
			once: false,
		};
		// Both possible scrollers: which one moves the document can change
		// with the container's styles.
		self.on(&self.window, "scroll", |host, _| host.invalidate(), passive);
		self.on(
			&self.container,
			"scroll",
			|host, _| host.invalidate(),
			passive,
		);
		self.on(&self.window, "resize", |host, _| host.invalidate(), passive);

		if let Some(fonts) = &self.fonts {
			self.on(
				fonts.set(),
				"loadingdone",
				|host, event| {
					if let Some(event) = event.dyn_ref::<FontFaceSetLoadEvent>() {
						host.fonts_loaded(&fonts::families(&event.fontfaces()));
					}
				},
				Listen::default(),
			);
		}

		if let Ok(Some(scheme)) = self.window.match_media("(prefers-color-scheme: dark)") {
			self.on(
				&scheme,
				"change",
				|host, _| host.refresh_theme(),
				Listen::default(),
			);
		}
	}

	/// Wrap to the container's content width as it changes.
	///
	/// The callback only records the size and paints on the next frame.
	/// Resizing the canvas inside it would change an observed box inside the
	/// observer's own callback, which browsers report as a loop.
	fn observe_size(self: &Rc<Self>) {
		let weak = Rc::downgrade(self);
		let callback = ResizeCallback::new(move |entries: js_sys::Array| {
			let Some(host) = weak.upgrade().filter(|host| !host.detached.get()) else {
				return;
			};

			for entry in entries.iter() {
				if let Some(width) = entry
					.dyn_ref::<ResizeObserverEntry>()
					.and_then(observed_width)
				{
					host.width.set(width);
				}
			}

			// A resize is also when the container's own styles tend to change.
			host.refresh_theme();
			host.invalidate();
		});

		let Ok(observer) = ResizeObserver::new(callback.as_ref().unchecked_ref()) else {
			return;
		};
		let options = ResizeObserverOptions::new();
		options.set_box(ResizeObserverBoxOptions::ContentBox);
		observer.observe_with_options(&self.container, &options);
		*self.resize.borrow_mut() = Some((observer, callback));
	}

	/// Measure again when the device pixel ratio changes (browser zoom, or a
	/// window moving to another screen): browsers round line metrics to
	/// device pixels, so line positions move with it.
	fn watch_scale(self: &Rc<Self>) {
		let query = format!("(resolution: {}dppx)", self.window.device_pixel_ratio());

		if let Ok(Some(query)) = self.window.match_media(&query) {
			self.on(
				&query,
				"change",
				|host, _| {
					if let Ok(mut editor) = host.editor.try_borrow_mut() {
						editor.forget_measurements();
					}

					host.snap_to_device();
					host.invalidate();
					host.watch_scale();
				},
				Listen {
					passive: false,
					once: true,
				},
			);
		}
	}

	/// Snap to the pixel grid and border widths this browser uses at the
	/// current ratio.
	fn snap_to_device(&self) {
		let grid = device::pixel_grid(&self.window, &self.document);
		let border = device::border_scale(&self.window, &self.document);

		self.with_editor(|editor, session| {
			editor.set_pixel_grid(grid);
			editor.set_border_scale(&session.doc, border);
		});
	}

	/// Run `run` with the editor and its session, unless the editor is busy:
	/// nothing the browser does synchronously inside a handler should ever
	/// find it so, and skipping is kinder than failing if something does.
	pub(super) fn with_editor<T>(
		&self,
		run: impl FnOnce(&mut Editor, &mut Session) -> T,
	) -> Option<T> {
		let mut editor = self.editor.try_borrow_mut().ok()?;
		let mut session = session::lock(&self.session);
		Some(run(&mut editor, &mut session))
	}

	/// Make a change through the editor. `None` when nothing changed.
	///
	/// Neither the editor nor the session is held when this returns, so the
	/// caller may then focus elements and call listeners, both of which can
	/// call back in.
	pub(super) fn edit(
		&self,
		change: impl FnOnce(&mut Editor, &mut Session) -> Result<Option<Change>, CanvistError>,
	) -> Option<Change> {
		match self.with_editor(change)? {
			Ok(change) => change,
			Err(error) => {
				report(&error);
				None
			}
		}
	}

	/// An input event changed the document or selection: paint in the same
	/// task, so the change reaches the screen in the next frame, then tell
	/// the app.
	pub(super) fn input_changed(self: &Rc<Self>, change: Change) {
		if let Ok(mut editor) = self.editor.try_borrow_mut() {
			editor.set_caret_on(true);
		}

		self.restart_blink();
		self.render();
		self.reveal_caret();
		self.schedule_mirror();
		session::notify(&self.session, change);
	}

	/// The app replaced the document.
	pub(super) fn document_replaced(self: &Rc<Self>) {
		self.with_editor(|editor, session| editor.replace(&session.doc));
		self.invalidate();
		self.schedule_mirror();
	}

	/// The app changed read-only or the placeholder: tell the text field,
	/// and repaint.
	pub(super) fn apply_options(self: &Rc<Self>) {
		let (read_only, placeholder) = {
			let session = session::lock(&self.session);
			(session.read_only, session.placeholder.clone())
		};
		let textarea = &self.textarea;
		let _ = textarea.set_attribute("aria-readonly", if read_only { "true" } else { "false" });
		// A read-only text field takes focus but no text or composition.
		textarea.set_read_only(read_only);

		if placeholder.is_empty() {
			let _ = textarea.remove_attribute("aria-placeholder");
		} else {
			let _ = textarea.set_attribute("aria-placeholder", &placeholder);
		}

		self.show_caret();
	}

	/// Move keyboard focus to the editor without scrolling to it.
	pub(super) fn focus(&self) {
		let options = FocusOptions::new();
		options.set_prevent_scroll(true);
		let _ = self.textarea.focus_with_options(&options);
	}

	/// The editor gained or lost focus.
	pub(super) fn set_focused(self: &Rc<Self>, focused: bool) {
		if self.focused.replace(focused) == focused {
			return;
		}

		if focused {
			self.refresh_theme();
		}

		self.show_caret();
	}

	/// Whether people are stopped from changing the document.
	pub(super) fn read_only(&self) -> bool {
		session::lock(&self.session).read_only
	}

	/// Draw the caret while the editor has focus and accepts typing.
	fn show_caret(self: &Rc<Self>) {
		let shown = self.focused.get() && !self.read_only();

		if let Ok(mut editor) = self.editor.try_borrow_mut() {
			editor.set_focused(shown);
		}

		self.restart_blink();
		self.invalidate();
	}

	pub(super) fn restart_blink(self: &Rc<Self>) {
		self.cancel_blink();
		self.blink_on.set(true);

		if self.focused.get() && !self.read_only() {
			self.schedule_blink();
		}
	}

	fn schedule_blink(&self) {
		let timers = self.timers.borrow();

		if let Some(timers) = timers.as_ref() {
			self.blink.set(
				self.window
					.set_timeout_with_callback_and_timeout_and_arguments_0(
						timers.blink.as_ref().unchecked_ref(),
						CARET_BLINK_MS,
					)
					.ok(),
			);
		}
	}

	fn cancel_blink(&self) {
		if let Some(blink) = self.blink.take() {
			self.window.clear_timeout_with_handle(blink);
		}
	}

	/// Read the container's styles again, and repaint if the theme they ask
	/// for changed.
	pub(super) fn refresh_theme(self: &Rc<Self>) {
		let Ok(mut reader) = self.theme.try_borrow_mut() else {
			return;
		};
		let theme = reader.read(&self.window, &self.document, &self.container);
		drop(reader);

		if let Some(theme) = theme {
			self.with_editor(|editor, session| editor.set_theme(&session.doc, theme));
			self.invalidate();
		}
	}

	/// A family's faces load one subset, weight, or style at a time, so the
	/// same family can arrive more than once; each arrival can change
	/// advances, so each forgets that family's measurements again.
	fn fonts_loaded(self: &Rc<Self>, families: &[String]) {
		if families.is_empty() || self.detached.get() {
			return;
		}

		if self
			.with_editor(|editor, _| editor.fonts_loaded(families))
			.is_some_and(|blocks| blocks > 0)
		{
			self.invalidate();
		}
	}

	/// Paint on the next animation frame.
	pub(super) fn invalidate(&self) {
		if self.frame.get().is_some() || self.detached.get() {
			return;
		}

		let timers = self.timers.borrow();

		if let Some(timers) = timers.as_ref() {
			self.frame.set(
				self.window
					.request_animation_frame(timers.frame.as_ref().unchecked_ref())
					.ok(),
			);
		}
	}

	fn cancel_frame(&self) {
		if let Some(frame) = self.frame.take() {
			let _ = self.window.cancel_animation_frame(frame);
		}
	}

	/// Lay out and paint now instead of on the next animation frame.
	pub(super) fn render(self: &Rc<Self>) {
		if self.detached.get() {
			return;
		}

		self.cancel_frame();
		let mut used = None;

		// The canvas is as tall as the visible part of the document, which
		// depends on the document's height, which the frame measures. A
		// second pass settles a height change in the same frame.
		for _ in 0..2 {
			let geometry = self.geometry();
			let resized = self.sync_size(geometry);
			let top = self.viewport_top(geometry);
			let frame = self.with_editor(|editor, session| {
				editor.set_top(top as f32);
				let frame = editor.frame(session);
				used = Some(editor.take_used_fonts());
				frame
			});
			let frame = match frame {
				Some(Ok(frame)) => frame,
				Some(Err(error)) => {
					report(&error);
					return;
				}
				None => return,
			};
			let height_changed =
				frame.height.to_bits() != self.height.replace(frame.height).to_bits();

			if frame.shift != 0.0 {
				self.scroll_by(geometry.scroller, f64::from(frame.shift));
			}

			if !height_changed && !resized {
				break;
			}
		}

		if let (Some(fonts), Some(used)) = (&self.fonts, used) {
			let weak = Rc::downgrade(self);
			fonts.request(&used, &move |families: Vec<String>| {
				if let Some(host) = weak.upgrade() {
					host.fonts_loaded(&families);
				}
			});
		}

		if self.focused.get() {
			self.sync_input(self.geometry());
		}
	}

	/// Size the canvas to the visible part of the document. Returns whether
	/// its height changed.
	fn sync_size(&self, geometry: Geometry) -> bool {
		let visible = match geometry.scroller {
			Scroller::Own => f64::from(self.container.client_height()),
			Scroller::Page => inner_height(&self.window),
		};
		let document_height = f64::from(self.height.get());
		let height = document_height.min(visible).max(0.0);
		#[allow(clippy::float_cmp, reason = "an unchanged height is the same number")]
		let changed = self.canvas_height.replace(height) != height;
		let _ = self.spacer.style().set_property(
			"height",
			&format!("{}px", (document_height - height).max(0.0)),
		);

		if changed {
			let _ = self
				.canvas
				.style()
				.set_property("height", &format!("{height}px"));
		}

		let scale = self.window.device_pixel_ratio();
		let bounds = self.canvas.get_bounding_client_rect();
		let surface = Surface {
			width: self.width.get() as f32,
			height: height as f32,
			scale: scale as f32,
			pixel_width: device::device_pixels(bounds.left(), bounds.width(), scale),
			pixel_height: device::device_pixels(bounds.top(), bounds.height(), scale),
		};
		// Resizing to the same size is free: the editor compares first.
		self.with_editor(|editor, session| editor.resize(&session.doc, surface));

		changed
	}

	fn geometry(&self) -> Geometry {
		let style = dom::computed(&self.window, &self.container);
		let overflow = style
			.as_ref()
			.map(|style| dom::property(style, "overflow-y"))
			.unwrap_or_default();

		Geometry {
			scroller: if matches!(overflow.as_str(), "auto" | "scroll") {
				Scroller::Own
			} else {
				Scroller::Page
			},
			padding_top: style
				.as_ref()
				.map_or(0.0, |style| dom::pixels(style, "padding-top")),
		}
	}

	/// The document coordinate at the canvas's top edge.
	fn viewport_top(&self, geometry: Geometry) -> f64 {
		let canvas = self.canvas.get_bounding_client_rect();
		let container = self.container.get_bounding_client_rect();
		let scrolled = match geometry.scroller {
			Scroller::Own => f64::from(self.container.scroll_top()),
			Scroller::Page => 0.0,
		};

		(canvas.top()
			- container.top()
			- f64::from(self.container.client_top())
			- geometry.padding_top
			+ scrolled)
			.max(0.0)
	}

	fn scroll_by(&self, scroller: Scroller, delta: f64) {
		match scroller {
			Scroller::Own => self.container.scroll_by_with_x_and_y(0.0, delta),
			Scroller::Page => self.window.scroll_by_with_x_and_y(0.0, delta),
		}
	}

	/// A pointer position in the page as a document coordinate.
	pub(super) fn document_point(&self, client_x: f64, client_y: f64) -> (f32, f32) {
		let canvas = self.canvas.get_bounding_client_rect();
		let top = self.viewport_top(self.geometry());

		(
			(client_x - canvas.left()) as f32,
			(client_y - canvas.top() + top) as f32,
		)
	}

	fn caret_rect(&self) -> Option<Rect> {
		self.with_editor(|editor, session| editor.caret_rect(&session.doc))
			.flatten()
	}

	/// Scroll the caret into view.
	fn reveal_caret(&self) {
		let Some(caret) = self.caret_rect() else {
			return;
		};
		let geometry = self.geometry();
		let (caret_top, caret_height) = (f64::from(caret.y), f64::from(caret.height));

		match geometry.scroller {
			Scroller::Own => {
				// The document starts below the container's padding.
				let caret_top = caret_top + geometry.padding_top;
				let top = f64::from(self.container.scroll_top());
				let bottom = top + f64::from(self.container.client_height());
				let delta = if caret_top < top {
					caret_top - top
				} else {
					(caret_top + caret_height - bottom).max(0.0)
				};

				if delta != 0.0 {
					self.scroll_by(Scroller::Own, delta);
				}
			}
			Scroller::Page => {
				let container = self.container.get_bounding_client_rect();
				let top = container.top()
					+ f64::from(self.container.client_top())
					+ geometry.padding_top
					+ caret_top;
				let delta = if top < 0.0 {
					top
				} else {
					(top + caret_height - inner_height(&self.window)).max(0.0)
				};

				if delta != 0.0 {
					self.scroll_by(Scroller::Page, delta);
				}
			}
		}
	}

	/// Move the text field to the caret, so input methods open their windows
	/// there. Left alone during a composition, which the input method owns.
	fn sync_input(&self, geometry: Geometry) {
		if self.composing.get() {
			return;
		}

		let Some(caret) = self.caret_rect() else {
			return;
		};
		let canvas = self.canvas.get_bounding_client_rect();
		let container = self.container.get_bounding_client_rect();
		let top = self.viewport_top(geometry);
		// From the page's coordinates to the container's padding box, which
		// absolutely positioned children are placed in.
		let left = canvas.left() + f64::from(caret.x)
			- container.left()
			- f64::from(self.container.client_left())
			+ f64::from(self.container.scroll_left());
		let top = canvas.top() + f64::from(caret.y)
			- top
			- container.top()
			- f64::from(self.container.client_top())
			+ f64::from(self.container.scroll_top());
		let style = self.textarea.style();
		let _ = style.set_property("left", &format!("{left}px"));
		let _ = style.set_property("top", &format!("{top}px"));
		let _ = style.set_property("height", &format!("{}px", caret.height));
	}

	/// The document changed: refresh the mirror when the browser is next
	/// idle.
	fn schedule_mirror(&self) {
		if self.mirror.is_none() || self.mirror_wait.get().is_some() || self.detached.get() {
			return;
		}

		let timers = self.timers.borrow();
		let Some(callback) = timers
			.as_ref()
			.map(|timers| timers.mirror.as_ref().unchecked_ref::<js_sys::Function>())
		else {
			return;
		};
		let options = web_sys::IdleRequestOptions::new();
		options.set_timeout(MIRROR_IDLE_MS as u32);
		// Safari has no idle callbacks.
		let idle = has_idle_callbacks(&self.window)
			.then(|| {
				self.window
					.request_idle_callback_with_options(callback, &options)
					.ok()
			})
			.flatten();

		self.mirror_wait.set(match idle {
			Some(handle) => Some(Waiting::Idle(handle)),
			None => {
				self.window
					.set_timeout_with_callback_and_timeout_and_arguments_0(callback, MIRROR_IDLE_MS)
					.ok()
					.map(Waiting::Timeout)
			}
		});
	}

	fn cancel_mirror(&self) {
		match self.mirror_wait.take() {
			Some(Waiting::Idle(handle)) => self.window.cancel_idle_callback(handle),
			Some(Waiting::Timeout(handle)) => self.window.clear_timeout_with_handle(handle),
			None => {}
		}
	}

	fn refresh_mirror(&self) {
		if let Some(mirror) = &self.mirror {
			mirror.refresh(&self.document, &session::lock(&self.session).doc);
		}
	}
}

/// Make `textarea` the editor's text field: named, announced as a
/// multi-line text box, and left alone by the browser's own text helpers,
/// whose suggestions would be about text it cannot see.
fn prepare_input(textarea: &HtmlTextAreaElement, label: &str) {
	let attributes = [
		("data-canvist", "input"),
		("role", "textbox"),
		("aria-multiline", "true"),
		("aria-label", label),
		("autocapitalize", "off"),
		("autocomplete", "off"),
		("autocorrect", "off"),
		("spellcheck", "false"),
	];

	for (name, value) in attributes {
		let _ = textarea.set_attribute(name, value);
	}
}

/// The width of `element`'s content box, with its fraction.
fn content_width(window: &Window, element: &HtmlElement) -> f64 {
	let width = element.get_bounding_client_rect().width();
	let Some(style) = dom::computed(window, element) else {
		return width;
	};

	[
		"border-left-width",
		"border-right-width",
		"padding-left",
		"padding-right",
	]
	.iter()
	.fold(width, |width, inset| width - dom::pixels(&style, inset))
	.max(0.0)
}

/// The content width a resize observer saw.
fn observed_width(entry: &ResizeObserverEntry) -> Option<f64> {
	entry
		.content_box_size()
		.get(0)
		.dyn_ref::<ResizeObserverSize>()
		.map(ResizeObserverSize::inline_size)
		.or_else(|| Some(entry.content_rect().width()))
}

fn inner_height(window: &Window) -> f64 {
	window
		.inner_height()
		.ok()
		.and_then(|height| height.as_f64())
		.unwrap_or(0.0)
}

fn has_idle_callbacks(window: &Window) -> bool {
	js_sys::Reflect::get(window, &"requestIdleCallback".into())
		.is_ok_and(|callback| callback.is_function())
}

fn unavailable(reason: &str) -> CanvistError {
	CanvistError::Host(reason.to_owned())
}

/// What a JavaScript exception says.
fn describe(error: &JsValue) -> String {
	error
		.as_string()
		.or_else(|| {
			error
				.dyn_ref::<js_sys::Error>()
				.map(|error| String::from(error.message()))
		})
		.unwrap_or_else(|| "the browser refused".to_owned())
}

/// Report a failure no caller is waiting for, where a developer will see
/// it. The editor carries on with the document it has.
pub(super) fn report(error: &CanvistError) {
	web_sys::console::error_1(&format!("leptoskit_canvist: {error}").into());
}
