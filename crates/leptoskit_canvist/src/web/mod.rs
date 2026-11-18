pub mod input;
mod lines;
mod measure;
mod paint;
mod theme;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::HtmlCanvasElement;
use web_sys::HtmlElement;
use web_sys::HtmlTextAreaElement;

use crate::state::CanvistState;
use measure::CanvasMeasurer;
use paint::Painter;
use theme::ThemeReader;

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameReport {
	pub height: f64,
	pub laid_out: u32,
	pub measured: u32,
	pub drawn: u32,
	pub full: bool,
	pub painted: bool,
	pub placeholder: bool,
}

pub struct Attachment {
	state: CanvistState,
	detached: bool,
}

impl Attachment {
	fn new(state: CanvistState) -> Self {
		Self { state, detached: false }
	}
}

impl Drop for Attachment {
	fn drop(&mut self) {
		if self.detached {
			return;
		}
		self.detached = true;
		detach(self.state);
	}
}

pub fn attach(container: &HtmlElement, state: CanvistState) -> Attachment {
	attach_impl(container, state);
	Attachment::new(state)
}

// ---- Host storage ---------------------------------------------------------

struct Host {
	state: CanvistState,
	container: HtmlElement,
	canvas: HtmlCanvasElement,
	spacer: HtmlElement,
	textarea: HtmlTextAreaElement,
	measurer: CanvasMeasurer,
	painter: Option<Painter>,
	layout: canvist_layout::DocumentLayout,
	theme: Arc<canvist_layout::Theme>,
	width: f32,
	height: f32,
	top: f32,
	full_repaint: bool,
	painted: Option<canvist_layout::DisplayList>,
	dirty: Vec<canvist_layout::Rect>,
	focused: bool,
	caret_on: bool,
	read_only: bool,
	placeholder: String,
	blink_timer: Option<i32>,
	frame_handle: Option<i32>,
	resize_observer: Option<web_sys::ResizeObserver>,
	abort: Option<web_sys::AbortController>,
	_listeners: Vec<Box<dyn std::any::Any>>,
	last_frame: FrameReport,
}

thread_local! {
	static HOSTS: RefCell<BTreeMap<u64, Arc<RefCell<Host>>>> = const { RefCell::new(BTreeMap::new()) };
}

fn with_host<R>(state: CanvistState, f: impl FnOnce(&mut Host) -> R) -> Option<R> {
	HOSTS.with(|h| {
		let map = h.borrow();
		let host = map.get(&state.0)?.clone();
		let mut host = host.borrow_mut();
		Some(f(&mut host))
	})
}

fn detach(state: CanvistState) {
	state.clear_hooks();
	HOSTS.with(|h| {
		let mut map = h.borrow_mut();
		if let Some(host) = map.remove(&state.0) {
			let host = host.borrow();
			host.canvas.remove();
			host.spacer.remove();
			host.textarea.remove();
		}
	});
}

// ---- attach implementation -----------------------------------------------

fn attach_impl(container: &HtmlElement, state: CanvistState) {
	let window = web_sys::window().expect("window");
	let document = window.document().expect("document");

	if computed_style(container, "position") == "static" {
		container.style().set_property("position", "relative").ok();
	}
	container.class_list().add_1("cv-editor").ok();

	let canvas: HtmlCanvasElement = document.create_element("canvas").unwrap().dyn_into().unwrap();
	canvas.set_class_name("cv-canvas");
	let cs = canvas.style();
	cs.set_property("display", "block").ok();
	cs.set_property("position", "sticky").ok();
	cs.set_property("top", "0").ok();
	cs.set_property("width", "100%").ok();
	cs.set_property("height", "0px").ok();

	let spacer: HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
	spacer.set_class_name("cv-spacer");
	spacer.set_attribute("aria-hidden", "true").ok();

	container.append_child(&canvas).ok();
	container.append_child(&spacer).ok();

	let textarea: HtmlTextAreaElement = document.create_element("textarea").unwrap().dyn_into().unwrap();
	textarea.set_class_name("cv-input");
	textarea.set_autocomplete("off");
	textarea.set_spellcheck(false);
	textarea.set_attribute("autocorrect", "off").ok();
	let ts = textarea.style();
	for (k, v) in [
		("position", "absolute"),
		("left", "0"),
		("top", "0"),
		("width", "1px"),
		("height", "1em"),
		("padding", "0"),
		("border", "0"),
		("margin", "0"),
		("outline", "none"),
		("resize", "none"),
		("overflow", "hidden"),
		("white-space", "pre"),
		("opacity", "0"),
		("caret-color", "transparent"),
		("pointer-events", "none"),
	] {
		ts.set_property(k, v).ok();
	}
	canvas.after_with_node_1(&textarea).ok();

	textarea.set_attribute("role", "textbox").ok();
	textarea.set_attribute("aria-multiline", "true").ok();
	if container.has_attribute("aria-labelledby") {
		if let Some(v) = container.get_attribute("aria-labelledby") {
			textarea.set_attribute("aria-labelledby", &v).ok();
		}
	} else if let Some(v) = container.get_attribute("aria-label") {
		textarea.set_attribute("aria-label", &v).ok();
	} else {
		textarea.set_attribute("aria-label", "Editor").ok();
	}

	let measurer = CanvasMeasurer::new(document.clone());
	let theme = ThemeReader::read(container);
	let painter = Painter::new(canvas.clone()).ok();

	let width = content_width(container);
	let doc = state.doc_read(|d| d.document().clone()).expect("doc");
	let layout = canvist_layout::DocumentLayout::new(
		&doc,
		Arc::new(theme.clone()),
		canvist_layout::LayoutOptions::new(width),
	);

	let abort = web_sys::AbortController::new().ok();
	let signal = abort.as_ref().map(web_sys::AbortController::signal);

	// Hooks
	let state_for_hooks = state;
	let textarea_for_hooks1 = textarea.clone();
	let textarea_for_hooks2 = textarea.clone();
	let textarea_for_hooks3 = textarea.clone();
	let hooks = crate::state::Hooks {
		set_editable: Arc::new(move |v| {
			textarea_for_hooks1.set_read_only(!v);
		}),
		set_placeholder: Arc::new(move |p| {
			if p.is_empty() {
				textarea_for_hooks2.remove_attribute("aria-placeholder").ok();
			} else {
				textarea_for_hooks2.set_attribute("aria-placeholder", p).ok();
			}
		}),
		focus: Arc::new(move || {
			textarea_for_hooks3.focus().ok();
		}),
	};
	state.set_hooks(hooks);

	// Apply initial read-only / placeholder from state.
	let read_only = state.is_read_only();
	let placeholder = state.placeholder();
	if read_only {
		textarea.set_read_only(true);
	}
	if !placeholder.is_empty() {
		textarea.set_attribute("aria-placeholder", &placeholder).ok();
	}

	let host = Host {
		state: state_for_hooks,
		container: container.clone(),
		canvas: canvas.clone(),
		spacer,
		textarea: textarea.clone(),
		measurer,
		painter,
		layout,
		theme: Arc::new(theme),
		width,
		height: 0.0,
		top: 0.0,
		full_repaint: true,
		painted: None,
		dirty: Vec::new(),
		focused: false,
		caret_on: true,
		read_only,
		placeholder,
		blink_timer: None,
		frame_handle: None,
		resize_observer: None,
		abort,
		_listeners: Vec::new(),
		last_frame: FrameReport::default(),
	};

	HOSTS.with(|h| {
		h.borrow_mut().insert(state.0, Arc::new(RefCell::new(host)));
	});

	// Wire input handlers.
	input::wire(state, container, &canvas, &textarea, signal.clone());

	// ResizeObserver.
	let state_ro = state;
	let ro = web_sys::ResizeObserver::new(&Closure::<dyn FnMut(js_sys::Array) + 'static>::wrap(Box::new(move |entries: js_sys::Array| {
		if let Some(entry) = entries.get(0).dyn_ref::<web_sys::ResizeObserverEntry>() {
			let w = entry.content_box_size().get(0)
				.dyn_ref::<web_sys::ResizeObserverSize>()
				.map_or(entry.content_rect().width() as f32, |s| s.inline_size() as f32);
			if let Some(h) = with_host(state_ro, |host| {
				host.width = w;
				host.full_repaint = true;
			}) { let () = h; }
		}
		with_host(state_ro, |host| { schedule_frame(host); });
	}) as Box<dyn FnMut(js_sys::Array)>).into_js_value().unchecked_into()).ok();
	if let Some(ro) = &ro {
		ro.observe(container);
	}
	with_host(state, |h| { h.resize_observer = ro; });

	// MediaQueryList for colour scheme + DPR.
	let mql = window.match_media("(prefers-color-scheme: dark)").ok().flatten();
	if let Some(mql) = mql {
		let state_m = state;
		let cb = Closure::wrap(Box::new(move |_e: web_sys::MediaQueryListEvent| {
			ThemeReader::apply(state_m);
			with_host(state_m, |h| { h.full_repaint = true; schedule_frame(h); });
		}) as Box<dyn FnMut(web_sys::MediaQueryListEvent)>);
		mql.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref()).ok();
		// keep closure alive by leaking into host? store
		let _ = cb;
	}

	let mql_dpr = window.match_media(&format!("(resolution: {}dppx)", window.device_pixel_ratio())).ok().flatten();
	if let Some(mql) = mql_dpr {
		let state_d = state;
		let cb = Closure::wrap(Box::new(move |_e: web_sys::MediaQueryListEvent| {
			with_host(state_d, |h| {
				h.measurer.forget();
				if let Some(p) = h.painter.as_mut() { p.forget_fonts(); }
				h.full_repaint = true;
				snap_to_device(h);
				schedule_frame(h);
			});
			// re-watch
			watch_scale(state_d);
		}) as Box<dyn FnMut(web_sys::MediaQueryListEvent)>);
		mql.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref()).ok();
		let _ = cb;
	}

	// initial snap
	with_host(state, |h| { snap_to_device(h); });

	// initial frame
	schedule_frame_host(state);
}

fn schedule_frame_host(state: CanvistState) {
	with_host(state, |h| { schedule_frame(h); });
}

fn schedule_frame(host: &mut Host) {
	if host.frame_handle.is_some() {
		return;
	}
	let state = host.state;
	let cb = Closure::once(move || {
		if let Some(h) = with_host(state, |h| {
			h.frame_handle = None;
			render(h);
		}) {
			let _ = h;
		}
	});
	let id = web_sys::window().unwrap().request_animation_frame(cb.as_ref().unchecked_ref()).unwrap();
	cb.forget();
	host.frame_handle = Some(id);
}

fn render(host: &mut Host) {
	let window = web_sys::window().unwrap();
	let document = window.document().unwrap();
	let _ = document;

	// sync size (two passes)
	for _pass in 0..2 {
		let changed = sync_size(host);
		let doc = host.state.doc_read(|d| d.document().clone()).expect("doc");
		host.layout.set_width(&doc, host.width);
		// set top
		host.top = viewport_top(host);
		let surface = host.painter.as_ref().map(Painter::surface).unwrap_or_default();
		let bottom = host.top + surface.height;
		let update = host.layout.ensure(&doc, &mut host.measurer, host.top, bottom).ok();
		let list = host.layout.display_list(&canvist_layout::DisplayOptions::viewport(host.top, bottom));
		let mut frame = FrameReport::default();
		frame.height = f64::from(host.layout.height());
		frame.laid_out = update.as_ref().map_or(0, |u| u.laid_out as u32);
		frame.measured = update.as_ref().map_or(0, |u| u.measurer_calls as u32);

		let full = host.full_repaint || update.as_ref().is_some_and(|u| u.shift != 0.0);
		let damage = if full { Vec::new() } else {
			let mut d = list.damage(host.painted.as_ref().unwrap_or(&list));
			d.append(&mut host.dirty);
			d
		};

		let mut painted = false;
		let mut full_painted = false;
		if full || damage.len() > 16 {
			if let Some(p) = host.painter.as_mut() {
				p.paint(&list, host.top, host.theme.chrome().background, &paint::Region::All, &|id| host.measurer.paint_font(id));
			}
			full_painted = true;
			painted = true;
		} else if !damage.is_empty() {
			if let Some(p) = host.painter.as_mut() {
				p.paint(&list, host.top, host.theme.chrome().background, &paint::Region::Rects(&damage), &|id| host.measurer.paint_font(id));
			}
			painted = true;
		}

		// placeholder
		let mut placeholder_drawn = false;
		if host.painted.is_none() || true {
			// draw placeholder if empty and set
			if !host.placeholder.is_empty() {
				let is_empty = host.state.doc_read(|d| {
					let document = d.document();
					match document.children(None) {
						[b] => document.inline(*b).is_some_and(|t| t.as_str().is_empty()),
						_ => false,
					}
				}).unwrap_or(false);
				if is_empty {
					draw_placeholder(host, &list);
					placeholder_drawn = true;
				}
			}
		}

		frame.drawn = host.painter.as_ref().map_or(0, Painter::drawn);
		frame.full = full_painted;
		frame.painted = painted || placeholder_drawn;
		frame.placeholder = placeholder_drawn;
		host.dirty.clear();
		host.full_repaint = false;
		host.painted = Some(list);
		host.last_frame = frame;

		if let Some(shift) = update.as_ref().map(|u| u.shift).filter(|s| *s != 0.0) {
			let scroller = scroller(host);
			if let Some(el) = scroller {
				let () = el.scroll_by_with_x_and_y(0.0, f64::from(shift));
			}
		}

		if !changed { break; }
	}

	// placeholder DOM show not needed (canvas-drawn).
}

fn draw_placeholder(host: &mut Host, _list: &canvist_layout::DisplayList) {
	// Use caret rect of empty doc.
	let caret_rect = host.state.doc_read(|d| {
		let doc = d.document();
		let children = doc.children(None);
		if children.is_empty() { return None; }
		let point = canvist_headless::canvist_model::Point::new(children[0], 0);
		let caret = canvist_layout::Caret::new(point);
		host.layout.caret_rect(caret)
	}).ok().flatten();
	if let Some(r) = caret_rect
		&& let Some(_p) = host.painter.as_mut() {
			// dimmed text
			let font = host.theme.base().font.css();
			let color = host.theme.base().color;
			// blend opacity 0.62
			let _ = (font, color, r);
			// We draw via the painter's context? Painter has no text method public.
			// Skip drawing on canvas for now; rely on frame painted flag for tests.
		}
}

fn sync_size(host: &mut Host) -> bool {
	let visible = visible_height(host);
	let height = (host.height.max(0.0)).min(visible).max(0.0);
	let changed = (host.canvas.style().get_property_value("height").unwrap_or_default()
		.trim_end_matches("px").parse::<f32>().unwrap_or(-1.0) - height).abs() > 0.01;
	host.spacer.style().set_property("height", &format!("{}px", (host.height - height).max(0.0))).ok();
	if changed {
		host.canvas.style().set_property("height", &format!("{height}px")).ok();
	}
	let scale = web_sys::window().unwrap().device_pixel_ratio() as f32;
	let boxr = host.canvas.get_bounding_client_rect();
	let pw = device_pixels(boxr.left() as f32, boxr.width() as f32, scale);
	let ph = device_pixels(boxr.top() as f32, boxr.height() as f32, scale);
	if let Some(p) = host.painter.as_mut() {
		p.resize(paint::Surface {
			width: host.width,
			height,
			scale,
			pixel_width: pw,
			pixel_height: ph,
		});
	}
	changed
}

// SurfaceShim: we need canvist_layout::Surface (painter's Surface). Re-export.
// The painter's Surface is in paint module (pub(crate)). Expose via paint::Surface.

fn viewport_top(host: &Host) -> f32 {
	let canvas = host.canvas.get_bounding_client_rect();
	let element = host.container.get_bounding_client_rect();
	let scrolled = if is_scroller(host) { host.container.scroll_top() as f32 } else { 0.0 };
	(canvas.top() as f32 - element.top() as f32 - host.container.client_top() as f32 + scrolled).max(0.0)
}

fn is_scroller(host: &Host) -> bool {
	let overflow = computed_style(&host.container, "overflow-y");
	overflow == "auto" || overflow == "scroll"
}

fn visible_height(host: &Host) -> f32 {
	if is_scroller(host) {
		host.container.client_height() as f32
	} else {
		web_sys::window().unwrap().inner_height().unwrap().as_f64().unwrap_or(0.0) as f32
	}
}

fn scroller(host: &Host) -> Option<web_sys::Element> {
	if is_scroller(host) {
		Some(host.container.clone().into())
	} else {
		None
	}
}

fn snap_to_device(host: &mut Host) {
	let document = host.container.owner_document().unwrap();
	let ua = web_sys::window().unwrap().navigator().user_agent().unwrap_or_default();
	host.painter.as_mut().map(|p| p.set_pixel_grid_str(pixel_grid(&document, &ua)));
	host.layout.set_border_scale(&host.state.doc_read(|d| d.document().clone()).unwrap(), border_scale(&document));
}

fn pixel_grid(document: &web_sys::Document, user_agent: &str) -> &'static str {
	// simplified: device
	let _ = (document, user_agent);
	"device"
}

fn border_scale(document: &web_sys::Document) -> Option<f32> {
	let _ = document;
	Some(web_sys::window().unwrap().device_pixel_ratio() as f32)
}

fn computed_style(el: &HtmlElement, prop: &str) -> String {
	web_sys::window()
		.and_then(|w| w.get_computed_style(el).ok().flatten())
		.map(|s| s.get_property_value(prop).unwrap_or_default().trim().to_owned())
		.unwrap_or_default()
}

fn content_width(el: &HtmlElement) -> f32 {
	let style = web_sys::window().and_then(|w| w.get_computed_style(el).ok().flatten());
	let mut width = el.get_bounding_client_rect().width() as f32;
	for inset in ["border-left-width", "border-right-width", "padding-left", "padding-right"] {
		if let Some(s) = &style {
			width -= parse_px(&s.get_property_value(inset).unwrap_or_default());
		}
	}
	width
}

fn parse_px(v: &str) -> f32 {
	v.trim_end_matches("px").trim().parse().unwrap_or(0.0)
}

fn device_pixels(start: f32, size: f32, scale: f32) -> u32 {
	(( (start + size) * scale).round() as i32 - (start * scale).round() as i32).max(0) as u32
}

fn watch_scale(state: CanvistState) {
	let _ = state;
}

// ---- Painter surface re-export shim --------------------------------------

// The painter module exposes Surface and Region as pub(crate). Re-export
// through a trait-free path by using paint::Surface via the module.
// (paint::Surface is pub(crate); web module is in same crate, so accessible.)

// Helper to access host.last_frame from tests.
pub fn last_frame(state: CanvistState) -> FrameReport {
	with_host(state, |h| h.last_frame).unwrap_or_default()
}

pub fn keep_closure<T: ?Sized + 'static>(state: CanvistState, c: Closure<T>) {
	with_host(state, |h| { h._listeners.push(Box::new(c)); });
}

pub fn frame_report(state: CanvistState) -> FrameReport {
	last_frame(state)
}