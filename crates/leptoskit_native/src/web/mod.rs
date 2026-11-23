//! Render shared leptoskit elements as genuine Leptos DOM views.
//!
//! Pass [`render`] the same [`Element`] used with a native surface. Wrap the
//! call in a reactive closure to rebuild when Leptos signals change. Leptos
//! reconciles existing DOM nodes, preserving focus and input selection.
//!
//! Styles have deterministic, content-derived scopes, so server rendering and
//! hydration agree and independently rendered trees cannot affect one another's
//! class precedence. Browser media queries update responsive classes directly.
//!
//! Accessibility properties render as their ARIA attributes, and structural
//! roles become semantic elements: headings, lists, and landmarks use their
//! HTML elements rather than a `div` with a `role`. Containers that declare a
//! control role join the tab order and activate from Enter and Space.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use leptos::either::Either;
use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::tachys::html::attribute::Attribute;
use leptos::tachys::html::attribute::custom::custom_attribute;
use leptos::tachys::html::event::on;
use wasm_bindgen::JsCast;

use crate::Capitalize;
use crate::Element;
use crate::EventOutcome;
use crate::Keyboard;
use crate::NativeEvent;
use crate::NativeEventSlot;
use crate::NativeKind;
use crate::NativeProps;
use crate::Progress;
use crate::SubmitLabel;
use crate::element::HandlerEntry;

mod controls;
mod labels;
mod motion;
mod overlay;
#[cfg(feature = "rive")]
mod rive;
mod semantics;
#[cfg(target_arch = "wasm32")]
pub(crate) mod shortcuts;
mod styles;
mod vector;

#[cfg(all(feature = "rive", target_arch = "wasm32", target_os = "unknown"))]
pub use rive::provide_rive_runtime;
use semantics::Tag;

type Handlers = Arc<[HandlerEntry]>;

/// Deterministic, allocation-free 64-bit hasher for CSS scope identifiers.
///
/// Server rendering and browser hydration hash the same inputs to the same
/// scope without linking a cryptographic hash crate into every web bundle.
struct ScopeHasher(u64);

impl ScopeHasher {
	const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
	const PRIME: u64 = 0x0000_0100_0000_01b3;

	#[inline]
	const fn new() -> Self {
		Self(Self::OFFSET)
	}

	#[inline]
	fn update(&mut self, bytes: &[u8]) {
		let mut state = self.0;

		for &byte in bytes {
			state ^= u64::from(byte);
			state = state.wrapping_mul(Self::PRIME);
		}

		self.0 = state;
	}

	#[inline]
	const fn finish(self) -> u64 {
		let mut z = self.0;
		z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
		z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
		z ^ (z >> 31)
	}
}

#[inline]
pub(super) fn scope_hash(bytes: &[u8]) -> u64 {
	let mut hasher = ScopeHasher::new();
	hasher.update(bytes);
	hasher.finish()
}

/// A screen that rebuilds whenever a signal `view` reads changes, keeping
/// what it [remembers](crate::remember::remember) between builds.
///
/// Call it where the app's root component builds its view, with the function
/// that builds the current screen:
///
/// ```ignore
/// #[leptos::component]
/// fn WebApp() -> impl leptos::prelude::IntoView {
/// 	let navigation = Navigation::new(Route::Home);
/// 	leptoskit::web::screen(move || navigation.render())
/// }
/// ```
///
/// It is `move || render(view())` with a [`ScreenState`] around each build,
/// released with the component.
///
/// [`ScreenState`]: crate::remember::ScreenState
pub fn screen(view: impl Fn() -> Element + Send + Sync + 'static) -> impl IntoView {
	rebuilds(view)
}

/// The closure [`screen`] hands leptos: one build per call.
fn rebuilds(
	view: impl Fn() -> Element + Send + Sync + 'static,
) -> impl Fn() -> AnyView + Send + Sync + 'static {
	let state = crate::remember::ScreenState::new();
	state.provide();
	move || render(crate::reactive::defer_regions_scope(|| state.build(&view)))
}

/// Expands every `For`/`Show` region in `element` once, subscribing the active
/// Leptos observer to whatever signals the regions read.
fn expand_regions(element: &mut Element) {
	if let Some(region) = element.region.take() {
		element.children = crate::reactive::defer_regions_scope(|| region.render())
			.into_iter()
			.map(|(_, child)| child)
			.collect();
	}

	for child in &mut element.children {
		expand_regions(child);
	}
}

/// Converts a shared element tree into a Leptos view, including its stylesheet.
///
/// This supports server rendering, hydration, and reactive client rendering.
/// Classes override typed base styles regardless of builder method order.
/// Existing event handlers receive platform-neutral events and stop DOM
/// bubbling when they return [`EventOutcome::Handled`].
/// Controlled inputs propose changes; the application owns their committed
/// state. A rejected proposal is restored after the event. Radio group names
/// are scoped per mounted tree. Server markup omits those names until
/// hydration, so radios become a keyboard group when the client attaches their
/// handlers.
///
/// # Panics
///
/// Panics for platform-specific `Custom` widgets or an invalid control tree
/// (see [`Element::validate`]).
pub fn render(mut element: Element) -> AnyView {
	// In a browser, leptoskit's records and panics go to the console. A
	// server rendering the same tree keeps its own logger.
	#[cfg(target_arch = "wasm32")]
	leptoskit_log::install();

	expand_regions(&mut element);

	element
		.validate()
		.unwrap_or_else(|error| panic!("invalid leptoskit element tree: {error}"));
	let mut sheets = BTreeMap::new();
	let mut labels = labels::Labels::default();
	let mut scan = TreeScan::default();
	scan_tree(&element, &mut labels, &mut scan);
	let root = render_node(
		element,
		Inherited::ROOT,
		&[],
		&mut sheets,
		&NodePath::root(),
		&labels,
	);
	let mut css = crate::Theme::css();
	let scope = root_scope(&sheets);
	let defaults = styles::defaults();
	let sheets_len: usize = sheets.values().map(String::len).sum();
	css.reserve(defaults.len() + sheets_len + 96);
	write!(
		css,
		":where([data-lk-root=\"{scope}\"],[data-lk-root=\"{scope}\"] \
		 [data-lk-style]){{{defaults}}}"
	)
	.expect("write to String");

	for sheet in sheets.into_values() {
		css.push_str(&sheet);
	}

	if scan.wants_focus {
		focus_after_render();
	}

	if scan.scrolls {
		scroll_after_render();
	}

	if scan.reveals {
		reveal_after_render();
	}

	overlay::sync_after_render();
	lift_splash_after_render();

	// Tachys does not hydrate raw-text children of <style>. Keep the sheet in
	// an attribute so a hydrated screen can replace it when styles change.
	(html::style().inner_html(css), root).into_any()
}

/// Lifts the HTML launch screen once a render is painted, unless the app
/// holds it with [`crate::splash::hold`]: the web's `until = "first_frame"`.
/// After it is gone, a render only looks it up.
fn lift_splash_after_render() {
	#[cfg(target_arch = "wasm32")]
	{
		if crate::splash::is_held() || splash_element().is_none() {
			return;
		}
		// Scheduling fails only without a window, and then there is no splash.
		let _ = request_animation_frame(|| {
			if !crate::splash::is_held() {
				lift_splash();
			}
		});
	}
}

#[cfg(target_arch = "wasm32")]
fn splash_element() -> Option<web_sys::Element> {
	web_sys::window()?
		.document()?
		.get_element_by_id(crate::splash::HTML_ID)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
	/// The fade-out the injected splash defines, `crate::splash::HTML_DISMISS`
	/// (a literal here, as wasm-bindgen needs). Throws when there is none.
	#[wasm_bindgen(js_namespace = window, js_name = leptoskitDismissSplash, catch)]
	fn dismiss_html_splash() -> Result<(), wasm_bindgen::JsValue>;
}

/// Lifts the HTML launch screen now: through the page's own
/// `window.leptoskitDismissSplash`, which fades it out, or by removing it
/// when the page defines none.
pub(crate) fn lift_splash() {
	#[cfg(target_arch = "wasm32")]
	if dismiss_html_splash().is_err()
		&& let Some(splash) = splash_element()
	{
		splash.remove();
	}
}

/// Post-render browser work discovered in one pass over the element tree.
#[derive(Default)]
struct TreeScan {
	wants_focus: bool,
	scrolls: bool,
	reveals: bool,
}

/// Collects label targets and post-render focus/scroll/reveal flags in one
/// walk.
fn scan_tree(element: &Element, labels: &mut labels::Labels, scan: &mut TreeScan) {
	if element.props.focused == Some(true) || element.props.focus_request.is_some() {
		scan.wants_focus = true;
	}

	if element.props.scroll_offset.is_some() {
		scan.scrolls = true;
	}

	if element.props.reveal.is_some() {
		scan.reveals = true;
	}

	labels.record(element);

	for child in &element.children {
		scan_tree(child, labels, scan);
	}
}

/// Scrolls the element carrying the newest reveal request into view, once
/// the new DOM is in place, when the renderer has not carried that request
/// out yet. Requests are numbered in the order they are made, so the
/// highest number carried out is all there is to remember, and an element
/// rebuilt with a request it already answered leaves the person's own
/// scrolling alone.
///
/// The browser scrolls every scrolling ancestor that hides the element,
/// the page included. It glides there unless the person asked their system
/// for less motion.
fn reveal_after_render() {
	#[cfg(target_arch = "wasm32")]
	queue_microtask(|| {
		thread_local! {
			static ANSWERED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
		}

		let Some(window) = web_sys::window() else {
			return;
		};
		let Some(document) = window.document() else {
			return;
		};
		let Ok(candidates) = document.query_selector_all("[data-lk-reveal]") else {
			return;
		};
		let newest = (0..candidates.length())
			.filter_map(|index| candidates.item(index)?.dyn_into::<web_sys::Element>().ok())
			.filter_map(|element| {
				let request = element.get_attribute("data-lk-reveal")?;
				let (serial, align) = request.split_once(' ')?;
				Some((serial.parse::<u64>().ok()?, align.to_owned(), element))
			})
			.max_by_key(|(serial, ..)| *serial);
		let Some((serial, align, target)) = newest.filter(|(serial, ..)| *serial > ANSWERED.get())
		else {
			return;
		};
		ANSWERED.set(serial);

		let still = window
			.match_media("(prefers-reduced-motion: reduce)")
			.ok()
			.flatten()
			.is_some_and(|query| query.matches());
		let options = web_sys::ScrollIntoViewOptions::new();
		options.set_block(match align.as_str() {
			"start" => web_sys::ScrollLogicalPosition::Start,
			"center" => web_sys::ScrollLogicalPosition::Center,
			"end" => web_sys::ScrollLogicalPosition::End,
			_ => web_sys::ScrollLogicalPosition::Nearest,
		});
		options.set_inline(web_sys::ScrollLogicalPosition::Nearest);
		options.set_behavior(if still {
			web_sys::ScrollBehavior::Instant
		} else {
			web_sys::ScrollBehavior::Smooth
		});
		target.scroll_into_view_with_scroll_into_view_options(&options);
	});
}

/// The attributes that carry an element's one-shot requests to the page:
/// to take focus, and to be scrolled into view.
fn one_shot_attributes(focus: Option<String>, reveal: Option<String>) -> impl Attribute {
	(
		custom_attribute("data-lk-focus-request", focus),
		custom_attribute("data-lk-reveal", reveal),
	)
}

/// The `data-lk-reveal` value: the request's serial, then where the element
/// ends up.
fn reveal_target(reveal: crate::RevealRequest) -> String {
	format!("{} {}", reveal.serial, reveal.align.as_str())
}

/// The `data-lk-scroll` value naming a scroll target, which the renderer
/// compares with the last target it applied to the same DOM element.
fn scroll_target(offset: crate::ScrollOffset) -> String {
	let animated = if offset.animated { " smooth" } else { "" };
	format!("{} {}{animated}", offset.x, offset.y)
}

/// Scrolls every element whose target changed since the renderer last
/// scrolled it, once the new DOM is in place: in a microtask, so a target set
/// from an animation frame scrolls before that frame paints. The applied
/// target is kept on the element, so a rebuild that keeps the DOM node leaves
/// the person's own scrolling alone, and a new node scrolls to its target.
fn scroll_after_render() {
	#[cfg(target_arch = "wasm32")]
	queue_microtask(|| {
		const APPLIED: &str = "data-lk-scrolled";

		let Some(document) = web_sys::window().and_then(|window| window.document()) else {
			return;
		};
		let Ok(targets) = document.query_selector_all("[data-lk-scroll]") else {
			return;
		};

		for index in 0..targets.length() {
			let Some(element) = targets
				.item(index)
				.and_then(|node| node.dyn_into::<web_sys::Element>().ok())
			else {
				continue;
			};
			let Some(target) = element.get_attribute("data-lk-scroll") else {
				continue;
			};

			if element.get_attribute(APPLIED).as_deref() == Some(target.as_str()) {
				continue;
			}

			let mut parts = target.split(' ');
			let (Some(Ok(x)), Some(Ok(y))) = (
				parts.next().map(str::parse::<f64>),
				parts.next().map(str::parse::<f64>),
			) else {
				continue;
			};
			let options = web_sys::ScrollToOptions::new();
			options.set_left(x);
			options.set_top(y);
			options.set_behavior(if parts.next() == Some("smooth") {
				web_sys::ScrollBehavior::Smooth
			} else {
				web_sys::ScrollBehavior::Instant
			});

			element.scroll_to_with_scroll_to_options(&options);
			let _ = element.set_attribute(APPLIED, &target);
		}
	});
}

/// Moves focus to the last element asking for it, once the browser has the
/// new DOM, unless it already holds focus.
fn focus_after_render() {
	// Scheduling fails only without a window, and then nothing can take focus.
	#[cfg(target_arch = "wasm32")]
	let _ = request_animation_frame(|| {
		let Some(document) = web_sys::window().and_then(|window| window.document()) else {
			return;
		};
		if focus_requested(&document) {
			return;
		}

		let Ok(candidates) = document.query_selector_all("[data-lk-focus]") else {
			return;
		};
		let target = candidates
			.length()
			.checked_sub(1)
			.and_then(|last| candidates.item(last))
			.and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok());

		if let Some(target) = target {
			let active = document.active_element();
			if active.as_ref() != Some(target.as_ref()) {
				let _ = target.focus();
			}
		}
	});
}

/// Moves focus to the element carrying the newest focus request, when the
/// renderer has not carried that request out yet. Requests are numbered in
/// the order they are made, so the highest number carried out is all there
/// is to remember, and an element that is rebuilt with a request it already
/// answered leaves focus where the person has since put it.
#[cfg(target_arch = "wasm32")]
fn focus_requested(document: &web_sys::Document) -> bool {
	thread_local! {
		static ANSWERED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
	}

	let Ok(candidates) = document.query_selector_all("[data-lk-focus-request]") else {
		return false;
	};
	let newest = (0..candidates.length())
		.filter_map(|index| {
			candidates
				.item(index)?
				.dyn_into::<web_sys::HtmlElement>()
				.ok()
		})
		.filter_map(|element| {
			let serial = element
				.get_attribute("data-lk-focus-request")?
				.parse::<u64>()
				.ok()?;
			Some((serial, element))
		})
		.max_by_key(|(serial, _)| *serial);
	let Some((serial, target)) = newest.filter(|(serial, _)| *serial > ANSWERED.get()) else {
		return false;
	};
	ANSWERED.set(serial);

	// A heading or a container takes focus from code once it has a tab
	// index; `-1` keeps it out of the Tab order.
	if target.tab_index() < 0 && !target.has_attribute("tabindex") {
		let _ = target.set_attribute("tabindex", "-1");
	}

	let _ = target.focus();
	true
}

/// What a node's typed DOM element is built from, besides the attributes
/// every kind shares.
struct NodeParts {
	kind: NativeKind,
	tag: Tag,
	props: NativeProps,
	text: String,
	/// What a text-bearing element shows: its text, or its styled runs.
	content: AnyView,
	children: Vec<AnyView>,
	enabled: bool,
	/// The target of a pressable that renders as a link.
	href: Option<String>,
	label: Option<String>,
	/// The handlers of this node and its ancestors, innermost last.
	chain: Vec<Handlers>,
}

/// What a node takes from its parent while rendering.
#[derive(Debug, Clone, Copy)]
struct Inherited {
	/// Whether this is the render root.
	root: bool,
	/// Whether every ancestor is enabled.
	enabled: bool,
	/// Whether an ancestor hides its subtree from assistive technology.
	hidden: bool,
	/// How much later a staggering parent starts this node's timed motion.
	stagger: std::time::Duration,
}

impl Inherited {
	const ROOT: Self = Self {
		root: true,
		enabled: true,
		hidden: false,
		stagger: std::time::Duration::ZERO,
	};
}

/// A read-only text field announces itself, the way `aria_read_only` does.
fn announce_read_only(kind: NativeKind, props: &mut NativeProps) {
	if matches!(
		kind,
		NativeKind::TextInput | NativeKind::TextArea | NativeKind::PasswordInput
	) && props.read_only == Some(true)
	{
		let aria = props.aria_mut();
		aria.read_only = aria.read_only.or(Some(true));
	}
}

/// Builds each typed DOM control before adding attributes common to all kinds.
#[expect(
	clippy::too_many_lines,
	reason = "single-pass node render keeps lazy event listener and style emission inline"
)]
fn render_node(
	element: Element,
	inherited: Inherited,
	ancestors: &[Handlers],
	sheets: &mut BTreeMap<String, String>,
	path: &NodePath,
	labels: &labels::Labels,
) -> AnyView {
	let Inherited {
		root,
		enabled: parent_enabled,
		hidden: parent_hidden,
		stagger,
	} = inherited;

	let devtools = path.attributes(&element);
	// The render's stylesheet sets the defaults once, so each scope's sheet
	// holds only what differs from them.
	let base = styles::delta_for(&element, root);
	let styles_active = element.classes().active_class().is_some();
	let motion = element
		.props
		.motion
		.as_deref()
		.map(|props| motion::css(props, stagger))
		.unwrap_or_default();
	let scope = style_scope(
		&base,
		element.classes().classes(),
		element.kind,
		styles_active,
	);

	if !sheets.contains_key(&scope) {
		let sheet = scoped_sheet(
			&scope,
			&base,
			element.classes(),
			element.kind,
			styles_active,
			&styles::motion_rules(&element.props),
		);
		sheets.insert(scope.clone(), sheet);
	}

	// Motion has a scope of its own, so moving elements still share their
	// style scope's sheet with the elements that look like them.
	let motion_scope = (!motion.rules.is_empty()).then(|| motion_scope(&motion.rules));

	if let Some(motion_scope) = &motion_scope
		&& !sheets.contains_key(motion_scope)
	{
		let selector = format!("[data-lk-motion=\"{motion_scope}\"]");
		sheets.insert(
			motion_scope.clone(),
			motion.rules.replace(motion::SELECTOR, &selector),
		);
	}

	if let Some((name, keyframes)) = motion.keyframes {
		sheets.entry(name).or_insert(keyframes);
	}

	let child_stagger = element
		.props
		.motion
		.as_deref()
		.and_then(|props| props.stagger)
		.unwrap_or_default();
	let handlers: Option<Handlers> =
		(!element.handlers().is_empty()).then(|| element.handlers().to_vec().into());
	let owned_chain: Option<Vec<Handlers>> = handlers.as_ref().map(|current| {
		let mut chain = Vec::with_capacity(ancestors.len() + 1);
		chain.extend_from_slice(ancestors);
		chain.push(Arc::clone(current));
		chain
	});
	let chain_slice: &[Handlers] = owned_chain.as_deref().unwrap_or(ancestors);
	let classes = element.classes().classes().to_owned();
	let kind = element.kind;
	let mut props = element.props;
	let id = props.id.take();
	announce_read_only(kind, &mut props);
	let test_id = props.test_id().map(str::to_owned);
	let enabled = parent_enabled && props.enabled.unwrap_or(true);
	let href = link_href(kind, &mut props);
	let link = href.is_some();
	// Hidden subtrees stay hidden: assistive technology ignores descendants of
	// an `aria-hidden` element, and the attribute on each node keeps the
	// element tree and the DOM in agreement.
	let hidden = parent_hidden || props.aria().hidden == Some(true);
	let named = props.names_itself();
	// An unnamed drawing is decoration, as an image with empty alt text is.
	let decorative = kind == NativeKind::Vector && !named && props.aria().role.is_none();
	let tag = semantics::tag(kind, props.aria(), named, element.children.is_empty());
	let label_for = labels.target(kind, tag, &props, id.as_deref());
	let tag = if label_for.is_some() { Tag::Label } else { tag };
	let label = props
		.accessibility_label
		.clone()
		.or_else(|| (kind == NativeKind::ActivityIndicator).then(|| "Loading".to_owned()));
	let is_text_input = matches!(
		kind,
		NativeKind::TextInput | NativeKind::TextArea | NativeKind::PasswordInput
	);
	// Styled runs render as inline spans; plain text is one text node.
	let (text, content) = if is_text_input {
		(
			props.text.take().unwrap_or_default(),
			String::new().into_any(),
		)
	} else {
		let content = match props.spans.take() {
			Some(spans) => spans_view(spans),
			None => props.text.take().unwrap_or_default().into_any(),
		};
		(String::new(), content)
	};

	let custom_control = semantics::custom_control(kind, props.aria());

	#[cfg(feature = "devtools")]
	let children = {
		let child_paths = path.children(&element.children);
		element
			.children
			.into_iter()
			.zip(child_paths)
			.enumerate()
			.map(|(index, (child, child_path))| {
				let inherited = Inherited {
					root: false,
					enabled,
					hidden,
					stagger: child_stagger.saturating_mul(u32::try_from(index).unwrap_or(u32::MAX)),
				};
				render_node(child, inherited, chain_slice, sheets, &child_path, labels)
			})
			.collect::<Vec<_>>()
	};
	#[cfg(not(feature = "devtools"))]
	let children = element
		.children
		.into_iter()
		.enumerate()
		.map(|(index, child)| {
			let inherited = Inherited {
				root: false,
				enabled,
				hidden,
				stagger: child_stagger.saturating_mul(u32::try_from(index).unwrap_or(u32::MAX)),
			};
			render_node(child, inherited, chain_slice, sheets, path, labels)
		})
		.collect::<Vec<_>>();
	let focused = props.focused == Some(true);
	let scroll = props.scroll_offset.map(scroll_target);
	let focus_request = props.focus_request.as_ref().map(u64::to_string);
	let reveal = props.reveal.map(reveal_target);
	// Every sheet is in by now: the children's were added above.
	let render_scope = root.then(|| root_scope(sheets));
	let scroll_chain = chain_slice
		.iter()
		.any(|group| {
			group
				.iter()
				.any(|entry| entry.event == NativeEventSlot::Scroll)
		})
		.then(|| chain_slice.to_vec());
	let needs_chain = matches!(
		kind,
		NativeKind::TextInput
			| NativeKind::TextArea
			| NativeKind::PasswordInput
			| NativeKind::Switch
			| NativeKind::Checkbox
			| NativeKind::Radio
			| NativeKind::Slider
			| NativeKind::Modal
			| NativeKind::Popover
	);
	let control_chain = if needs_chain {
		owned_chain.unwrap_or_else(|| ancestors.to_vec())
	} else {
		Vec::new()
	};

	let attrs = (
		custom_attribute("data-lk-root", render_scope),
		custom_attribute("data-lk-style", scope),
		custom_attribute("data-lk-motion", motion_scope),
		custom_attribute("data-lk-focus", focused.then_some("true")),
		one_shot_attributes(focus_request, reveal),
		custom_attribute("data-lk-scroll", scroll),
		custom_attribute("for", label_for),
		custom_attribute("class", classes),
		custom_attribute("data-lk-node", devtools.node),
		custom_attribute("data-lk-key", devtools.key),
		custom_attribute("data-lk-source", devtools.source),
		custom_attribute("data-lk-kind", devtools.kind),
		custom_attribute("data-lk-component", devtools.component),
		custom_attribute("aria-label", label.clone()),
		custom_attribute("aria-disabled", (!enabled).then_some("true")),
		custom_attribute("data-testid", test_id),
		semantics::attributes(
			id,
			props.aria(),
			kind,
			tag,
			hidden || decorative,
			custom_control,
		),
		listeners(handlers, scroll_chain, link, custom_control),
	);
	let node = typed_element(
		NodeParts {
			kind,
			tag,
			props,
			text,
			content,
			children,
			enabled,
			href,
			label,
			chain: control_chain,
		},
		attrs,
	);

	if root {
		node.directive(
			|element: web_sys::Element| {
				controls::scope_root(&element);
				controls::enable_touch_active(&element);
			},
			(),
		)
		.into_any()
	} else {
		node
	}
}

/// Discriminates control decoration outputs of [`styles::control`] without
/// formatting the rule on cache hits.
#[inline]
fn control_discriminant(kind: NativeKind, styles_active: bool) -> u8 {
	match kind {
		NativeKind::Button => u8::from(!styles_active),
		NativeKind::Pressable => {
			if styles_active {
				2
			} else {
				3
			}
		}
		NativeKind::Vector => 4,
		NativeKind::Modal => 5,
		NativeKind::Popover => 6,
		NativeKind::Switch => 7,
		NativeKind::ActivityIndicator => 8,
		_ => 0,
	}
}

/// The style scope a node's content hashes to, so equal styles share one
/// sheet.
///
/// The scope derives from the same content as the sheet: the typed base
/// styles, the normalized class string, and the control decoration. The
/// normalized classes determine the generated utilities — the same string
/// always compiles to the same CSS — so hashing the string costs a fraction
/// of generating the stylesheet to hash it.
fn style_scope(base: &str, classes: &str, kind: NativeKind, styles_active: bool) -> String {
	let mut hasher = ScopeHasher::new();
	hasher.update(base.as_bytes());
	hasher.update(b"\0");
	hasher.update(classes.as_bytes());
	hasher.update(b"\0");
	hasher.update(&[control_discriminant(kind, styles_active)]);
	short_scope("lk-", hasher.finish())
}

/// The scope of one render: the defaults its stylesheet writes apply only
/// under its root, since a stylesheet later in the document would otherwise
/// reset the elements of renders before it. Two renders share a scope only
/// when they wrote the same sheets, so either one's defaults are followed by
/// every delta both trees need.
fn root_scope(sheets: &BTreeMap<String, String>) -> String {
	let mut hasher = ScopeHasher::new();

	for scope in sheets.keys() {
		hasher.update(scope.as_bytes());
		hasher.update(b"\0");
	}

	short_scope("lk-r", hasher.finish())
}

/// `prefix` and 64 bits of `digest` in hex: unique enough within
/// one document, and short, since every element carries its scope.
fn short_scope(prefix: &str, digest: u64) -> String {
	let mut scope = String::with_capacity(prefix.len() + 16);
	scope.push_str(prefix);
	write!(scope, "{digest:016x}").expect("write to String");
	scope
}

/// The motion scope a node's motion rules hash to, so equal motions share
/// one sheet. Motion rules differ by delay as well as by movement, so a
/// short hash keeps the many scopes of a staggered list small.
fn motion_scope(rules: &str) -> String {
	short_scope("lk-m", scope_hash(rules.as_bytes()))
}

/// The node's stylesheet: typed base styles that differ from the defaults,
/// utility classes, and control decoration, under the selector its scope
/// names. Only built for a scope the document has not seen, so a re-render
/// that reuses every style builds none.
fn scoped_sheet(
	scope: &str,
	base: &str,
	classes: &crate::tailwind::Tailwind,
	kind: NativeKind,
	styles_active: bool,
	motion: &str,
) -> String {
	let selector = format!("[data-lk-style=\"{scope}\"]");
	let base = if base.is_empty() {
		String::new()
	} else {
		format!(":where({selector}){{{base}}}")
	};
	format!(
		"{base}{}{}{motion}",
		classes.css_scoped(&selector),
		styles::control(&selector, kind, styles_active),
	)
}

/// The target of a pressable that is a link: an `<a>` the browser can open
/// in a new tab, whose plain clicks the app handles in place.
///
/// A link that opens in a new tab also says so to screen reader users,
/// after anything the app already says about it. Sighted people get that
/// hint from the link's own text or icon.
fn link_href(kind: NativeKind, props: &mut NativeProps) -> Option<String> {
	const HINT: &str = "Opens in a new tab";

	if kind != NativeKind::Pressable {
		return None;
	}

	let href = props.href()?.to_owned();

	if props.link_target() == crate::LinkTarget::NewTab {
		let aria = props.aria_mut();
		aria.description = Some(match aria.description.take() {
			Some(description) => format!("{description} {HINT}"),
			None => HINT.to_owned(),
		});
	}

	Some(href)
}

/// The DOM listeners a node carries, attached only for event slots the node
/// (or its scroll ancestor chain) actually handles.
fn listeners(
	handlers: Option<Handlers>,
	scroll_chain: Option<Vec<Handlers>>,
	link: bool,
	custom_control: bool,
) -> impl Attribute {
	let (has_press, listens_for_keys, has_submit, has_focus, has_blur) = match &handlers {
		Some(handlers) => {
			let mut press = false;
			let mut key = false;
			let mut submit = false;
			let mut focus = false;
			let mut blur = false;

			for entry in handlers.iter() {
				match entry.event {
					NativeEventSlot::Press => press = true,
					NativeEventSlot::Key => key = true,
					NativeEventSlot::Submit => submit = true,
					NativeEventSlot::Focus => focus = true,
					NativeEventSlot::Blur => blur = true,
					_ => {}
				}
			}

			(press, key, submit, focus, blur)
		}
		None => (false, false, false, false, false),
	};

	let click = if has_press {
		let press = Arc::clone(handlers.as_ref().expect("press handler present"));
		Either::Left(on(ev::click, move |event| {
			if !controls::interactive(&event) || controls::typed_target(&event) {
				return;
			}
			if link && controls::modified(&event) {
				return;
			}

			let outcome = dispatch(&press, &NativeEvent::Press, &event);
			// A handled link click stays in the app; an unhandled one is
			// followed by the browser.
			if link && outcome == EventOutcome::Handled {
				event.prevent_default();
			}
		}))
	} else {
		Either::Right(())
	};

	let keydown = if listens_for_keys || has_submit || (custom_control && has_press) {
		let keys = Arc::clone(handlers.as_ref().expect("keydown handler present"));
		Either::Left(on(ev::keydown, move |event| {
			on_keydown(
				keys.as_ref(),
				keys.as_ref(),
				keys.as_ref(),
				listens_for_keys,
				custom_control,
				&event,
			);
		}))
	} else {
		Either::Right(())
	};

	let focusin = if has_focus {
		let focus = Arc::clone(handlers.as_ref().expect("focus handler present"));
		Either::Left(on(ev::focusin, move |event| {
			if !controls::interactive(event.as_ref()) {
				return;
			}

			dispatch(&focus, &NativeEvent::Focus, event.as_ref());
		}))
	} else {
		Either::Right(())
	};

	let focusout = if has_blur {
		let blur = handlers.expect("blur handler present");
		Either::Left(on(ev::focusout, move |event| {
			if !controls::interactive(event.as_ref()) {
				return;
			}

			dispatch(&blur, &NativeEvent::Blur, event.as_ref());
		}))
	} else {
		Either::Right(())
	};

	let scroll = match scroll_chain {
		Some(scroll_chain) => {
			Either::Left(on(ev::scroll, move |event| {
				on_scroll(&scroll_chain, &event);
			}))
		}
		None => Either::Right(()),
	};

	(click, keydown, focusin, focusout, scroll)
}

/// The DOM element for a node's kind, with the shared attributes added.
///
/// Changing handlers attach to typed elements before erasure. In the pinned
/// Tachys version, rebuilding attributes spread onto `AnyView` keeps the
/// original listener state instead of replacing its callbacks.
/// Where a node sits, for the `DevTools` attributes: its path from the
/// render root, and its cross-platform match key.
struct NodePath {
	#[cfg(feature = "devtools")]
	path: String,
	#[cfg(feature = "devtools")]
	key: u64,
}

/// The `DevTools` attributes of one node; all `None` without the feature.
#[derive(Default)]
struct DevtoolsAttributes {
	node: Option<String>,
	key: Option<String>,
	source: Option<String>,
	kind: Option<String>,
	component: Option<&'static str>,
}

impl NodePath {
	fn root() -> Self {
		Self {
			#[cfg(feature = "devtools")]
			path: String::new(),
			#[cfg(feature = "devtools")]
			key: crate::devtools::ROOT,
		}
	}

	/// Each child's path, in order.
	#[cfg(feature = "devtools")]
	fn children(&self, children: &[Element]) -> Vec<Self> {
		use crate::devtools::Origin;

		let origins: Vec<Origin> = children
			.iter()
			.map(|child| Origin::of(child.source_location(), child.kind))
			.collect();
		let ordinals = crate::devtools::ordinals(origins.iter().copied());
		origins
			.into_iter()
			.zip(ordinals)
			.enumerate()
			.map(|(index, (origin, ordinal))| {
				Self {
					path: format!("{}.{index}", self.path),
					key: crate::devtools::child(self.key, origin, ordinal),
				}
			})
			.collect()
	}

	#[cfg_attr(
		not(feature = "devtools"),
		expect(
			clippy::unused_self,
			reason = "the attributes exist only with the devtools feature"
		)
	)]
	fn attributes(&self, element: &Element) -> DevtoolsAttributes {
		#[cfg(feature = "devtools")]
		{
			DevtoolsAttributes {
				node: Some(self.path.clone()),
				key: Some(crate::devtools::format(self.key)),
				source: element.source_location().map(|location| {
					format!(
						"{}:{}:{}",
						location.file(),
						location.line(),
						location.column()
					)
				}),
				kind: Some(format!("{:?}", element.kind)),
				component: element.component_name(),
			}
		}
		#[cfg(not(feature = "devtools"))]
		{
			let _ = element;
			DevtoolsAttributes::default()
		}
	}
}

/// A `Scene3d`'s canvas. `leptoskit_3d`'s web host finds it by its scene and
/// draws into it, so apps without 3D never link a renderer.
fn scene3d_canvas(scene: Option<u32>, attrs: impl Attribute + 'static) -> AnyView {
	html::canvas()
		.attr("data-lk-scene3d", scene.map(|scene| scene.to_string()))
		.add_any_attr(attrs)
		.into_any()
}

/// A `Video` surface. The `video` plugin binds a host player by id.
fn video_element(player: Option<u32>, attrs: impl Attribute + 'static) -> AnyView {
	html::video()
		.attr("data-lk-video", player.map(|player| player.to_string()))
		.attr("playsinline", true)
		.add_any_attr(attrs)
		.into_any()
}

/// A `Camera` surface. The `camera` plugin binds a host session by id.
fn camera_element(session: Option<u32>, attrs: impl Attribute + 'static) -> AnyView {
	html::video()
		.attr("data-lk-camera", session.map(|session| session.to_string()))
		.attr("playsinline", true)
		.attr("muted", true)
		.attr("autoplay", true)
		.add_any_attr(attrs)
		.into_any()
}

/// A `Map` surface. The `maps` plugin binds a host session by id; browsers
/// leave a placeholder until a web map host lands.
fn map_element(session: Option<u32>, attrs: impl Attribute + 'static) -> AnyView {
	html::div()
		.attr("data-lk-map", session.map(|session| session.to_string()))
		.add_any_attr(attrs)
		.into_any()
}

/// A `LivePhoto` surface placeholder until a browser host lands.
fn live_photo_element(session: Option<u32>, attrs: impl Attribute + 'static) -> AnyView {
	html::div()
		.attr(
			"data-lk-live-photo",
			session.map(|session| session.to_string()),
		)
		.add_any_attr(attrs)
		.into_any()
}

/// The canvas a Rive animation draws on.
#[cfg_attr(
	not(feature = "rive"),
	expect(
		unused_variables,
		clippy::needless_pass_by_value,
		reason = "nothing renders without the rive feature"
	)
)]
fn rive_canvas(props: NativeProps, attrs: impl Attribute + 'static) -> AnyView {
	#[cfg(feature = "rive")]
	{
		html::canvas()
			.add_any_attr(attrs)
			.add_any_attr(rive::Binding {
				content: *props.rive.expect("validated Rive content"),
				running: props.running == Some(true),
			})
			.into_any()
	}
	#[cfg(not(feature = "rive"))]
	panic!("enable leptoskit_native's rive feature before rendering a Rive element")
}

/// The element for a semantic tag, with `inner` as its content.
///
/// Each tag has its own typed element, so a rebuild patches it in place. A
/// custom element built from the tag's name would be replaced on every
/// rebuild instead: tachys compares the name with the document's upper-case
/// `tagName`, and a field inside, with its focus and selection, would go with
/// it.
fn semantic_element(
	tag: Tag,
	inner: impl IntoView + 'static,
	attrs: impl Attribute + 'static,
) -> AnyView {
	macro_rules! typed {
		($element:expr) => {
			$element.child(inner).add_any_attr(attrs).into_any()
		};
	}

	match tag {
		Tag::Div => typed!(html::div()),
		Tag::Span => typed!(html::span()),
		Tag::Heading(1) => typed!(html::h1()),
		Tag::Heading(2) => typed!(html::h2()),
		Tag::Heading(3) => typed!(html::h3()),
		Tag::Heading(4) => typed!(html::h4()),
		Tag::Heading(5) => typed!(html::h5()),
		Tag::Heading(_) => typed!(html::h6()),
		Tag::List => typed!(html::ul()),
		Tag::ListItem => typed!(html::li()),
		Tag::Nav => typed!(html::nav()),
		Tag::Main => typed!(html::main()),
		Tag::Header => typed!(html::header()),
		Tag::Footer => typed!(html::footer()),
		Tag::Aside => typed!(html::aside()),
		Tag::Article => typed!(html::article()),
		Tag::Section => typed!(html::section()),
		Tag::Search => typed!(html::search()),
		Tag::Paragraph => typed!(html::p()),
		Tag::Label => typed!(html::label()),
		// A rule is empty: it has no content to keep.
		Tag::Rule => html::hr().add_any_attr(attrs).into_any(),
	}
}

/// A progress bar: `<progress>` with a value, or without one while the amount
/// is unknown.
fn linear_progress(progress: Progress, attrs: impl Attribute + 'static) -> AnyView {
	let value = match progress {
		Progress::Indeterminate => None,
		Progress::Determinate(value) => Some(value.get().to_string()),
	};

	html::progress()
		.max("1")
		.value(value)
		.add_any_attr(attrs)
		.into_any()
}

/// A `CanvistEditor`'s container. `leptoskit_canvist`'s web host finds it by
/// its editor and builds the editor inside, so apps without an editor never
/// link one. The host takes the placeholder and the read-only state from the
/// attributes here, and the editor's name from the container's label.
fn canvist_editor_element(props: NativeProps, attrs: impl Attribute + 'static) -> AnyView {
	html::div()
		.attr(
			"data-lk-canvist-editor",
			props.canvist_editor.map(|editor| editor.to_string()),
		)
		.attr("data-lk-placeholder", props.placeholder)
		.attr("data-lk-read-only", props.read_only == Some(true))
		.directive(
			|element: web_sys::Element, editor| name_canvist_editor(&element, editor),
			props.canvist_editor,
		)
		.add_any_attr(attrs)
		.into_any()
}

/// Writes the page's own editor onto a `CanvistEditor`'s container.
///
/// An editor's id is handed out when the editor is created, so the server's
/// editor and the page's seldom share one, and hydration leaves attributes as
/// the server wrote them. Without this a server-rendered container would go
/// on naming an editor the page does not have.
fn name_canvist_editor(element: &web_sys::Element, editor: Option<u32>) {
	let Some(editor) = editor.map(|editor| editor.to_string()) else {
		return;
	};

	if element.get_attribute("data-lk-canvist-editor").as_deref() != Some(editor.as_str()) {
		let _ = element.set_attribute("data-lk-canvist-editor", &editor);
	}
}

fn typed_element(parts: NodeParts, attrs: impl Attribute + 'static) -> AnyView {
	let NodeParts {
		kind,
		tag,
		props,
		text,
		content,
		children,
		enabled,
		href,
		label,
		chain,
	} = parts;

	match kind {
		NativeKind::View | NativeKind::ScrollView => {
			match tag {
				Tag::Div => {
					html::div()
						.child((content, children))
						.add_any_attr(attrs)
						.into_any()
				}
				semantic => semantic_element(semantic, (content, children), attrs),
			}
		}
		NativeKind::Text | NativeKind::TextRun => {
			match tag {
				Tag::Heading(_) | Tag::Label => semantic_element(tag, (content, children), attrs),
				_ => {
					html::span()
						.child((content, children))
						.add_any_attr(attrs)
						.into_any()
				}
			}
		}
		NativeKind::Button => {
			html::button()
				.r#type("button")
				.disabled(!enabled)
				.child((content, children))
				.add_any_attr(attrs)
				.into_any()
		}
		NativeKind::Pressable => {
			match href {
				// A disabled link has no target, so it cannot be followed.
				Some(href) => {
					// A new tab gets no handle on this page and is not told
					// where the person came from.
					let new_tab = enabled && props.link_target() == crate::LinkTarget::NewTab;
					let download = enabled
						.then(|| props.download().map(str::to_owned))
						.flatten();

					html::a()
						.href(enabled.then_some(href))
						.target(new_tab.then_some("_blank"))
						.rel(new_tab.then_some("noopener noreferrer"))
						.download(download)
						.child(children)
						.add_any_attr(attrs)
						.into_any()
				}
				None => {
					html::button()
						.r#type("button")
						.disabled(!enabled)
						.child(children)
						.add_any_attr(attrs)
						.into_any()
				}
			}
		}
		NativeKind::Image => {
			html::img()
				.src(crate::asset::web_url(&props.source.unwrap_or_default()).into_owned())
				.alt(label.unwrap_or_default())
				.add_any_attr(attrs)
				.into_any()
		}
		NativeKind::Rive => rive_canvas(props, attrs),
		NativeKind::Scene3d => scene3d_canvas(props.scene3d, attrs),
		NativeKind::Video => video_element(props.video, attrs),
		NativeKind::Camera => camera_element(props.camera, attrs),
		NativeKind::Map => map_element(props.map, attrs),
		NativeKind::LivePhoto => live_photo_element(props.live_photo, attrs),
		NativeKind::CanvistEditor => canvist_editor_element(props, attrs),
		NativeKind::TextInput | NativeKind::TextArea | NativeKind::PasswordInput => {
			text_input(kind, props, text, enabled, chain, attrs)
		}
		NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio => {
			controls::toggle(
				props.checked.unwrap_or(false),
				enabled,
				if kind == NativeKind::Radio {
					props.radio_group.map(|group| group.name().to_owned())
				} else {
					None
				},
				kind == NativeKind::Switch,
				chain,
			)
			.add_any_attr(attrs)
			.into_any()
		}

		NativeKind::Slider => {
			controls::slider(props.slider.unwrap_or_default(), enabled, chain)
				.add_any_attr(attrs)
				.into_any()
		}
		NativeKind::LinearProgressIndicator => {
			linear_progress(props.progress.unwrap_or_default(), attrs)
		}
		NativeKind::ActivityIndicator => activity_indicator(props.running.unwrap_or(true), attrs),
		NativeKind::Vector => {
			html::span()
				.attr(
					"role",
					(props.names_itself() && props.aria().role.is_none()).then_some("img"),
				)
				.child(vector::svg(props.drawing.as_ref()))
				.add_any_attr(attrs)
				.into_any()
		}
		NativeKind::Modal | NativeKind::Popover => {
			overlay::element(kind, props.overlay, children, &chain, attrs)
		}
		NativeKind::Custom(name) => {
			panic!(
				"leptoskit web renderer does not support platform-specific Custom widget `{name}`"
			)
		}
	}
}

/// A text or password `<input>` whose edits propose values to the
/// application, which owns the committed value.
fn text_input(
	kind: NativeKind,
	props: NativeProps,
	text: String,
	enabled: bool,
	chain: Vec<Handlers>,
	attrs: impl Attribute + 'static,
) -> AnyView {
	if kind == NativeKind::TextArea {
		return text_area(props, text, enabled, chain, attrs);
	}

	let value = props.value.unwrap_or(text);
	// A shaped keyboard is a `type` where HTML has one, and an `inputmode`
	// where it has none. A password field keeps its type whatever it asks
	// for: masking is the one keyboard shape that matters there.
	let keyboard = (kind == NativeKind::TextInput).then_some(props.keyboard.unwrap_or_default());
	let (typed, input_mode) = match keyboard {
		Some(Keyboard::Email) => (Some("email"), None),
		Some(Keyboard::Phone) => (Some("tel"), None),
		Some(Keyboard::Url) => (Some("url"), None),
		Some(Keyboard::Search) => (Some("search"), None),
		Some(Keyboard::Number) => (None, Some("numeric")),
		Some(Keyboard::Decimal) => (None, Some("decimal")),
		_ => (None, None),
	};
	let input_type = typed.unwrap_or(if kind == NativeKind::PasswordInput {
		"password"
	} else {
		"text"
	});
	// Spelling suggestions and marks follow `autocorrect`, the way HTML's
	// `autocorrect` and `spellcheck` do.
	let autocorrect = kind != NativeKind::PasswordInput
		&& props.autocorrect.is_some_and(|autocorrect| !autocorrect);

	html::input()
		.r#type(input_type)
		.disabled(!enabled)
		.placeholder(props.placeholder.unwrap_or_default())
		.attr("autocomplete", props.autocomplete)
		.attr("inputmode", input_mode)
		.attr("readonly", props.read_only == Some(true))
		.attr(
			"maxlength",
			props.max_length.map(|length| length.to_string()),
		)
		.attr(
			"autocapitalize",
			props
				.capitalize
				.filter(|_| kind == NativeKind::TextInput)
				.map(capitalize_value),
		)
		.attr("autocorrect", autocorrect.then_some("off"))
		.attr("spellcheck", autocorrect.then_some("false"))
		.attr(
			"enterkeyhint",
			props
				.submit_label
				.filter(|_| kind == NativeKind::TextInput)
				.map(SubmitLabel::wire_name),
		)
		.value(value.clone())
		.attr("data-lk-value", value)
		.directive(controls::restore_input, ())
		.on(ev::input, move |event| {
			controls::text_event(&event, &chain);
		})
		.add_any_attr(attrs)
		.into_any()
}

/// A `<textarea>` whose edits propose values to the application, which owns
/// the committed value. Enter adds a line, so it raises no `Submit`.
fn text_area(
	props: NativeProps,
	text: String,
	enabled: bool,
	chain: Vec<Handlers>,
	attrs: impl Attribute + 'static,
) -> AnyView {
	let value = props.value.unwrap_or(text);
	let autocorrect = props.autocorrect.is_some_and(|autocorrect| !autocorrect);

	html::textarea()
		.disabled(!enabled)
		.placeholder(props.placeholder.unwrap_or_default())
		.attr("autocomplete", props.autocomplete)
		.attr("readonly", props.read_only == Some(true))
		.attr(
			"maxlength",
			props.max_length.map(|length| length.to_string()),
		)
		.attr("autocapitalize", props.capitalize.map(capitalize_value))
		.attr("autocorrect", autocorrect.then_some("off"))
		.attr("spellcheck", autocorrect.then_some("false"))
		// The server writes the value as the element's text; the browser
		// keeps the live value in a property, which `restore` reasserts.
		.prop("value", value.clone())
		.attr("data-lk-value", value.clone())
		.directive(controls::restore_input, ())
		.on(ev::input, move |event| {
			controls::text_event(&event, &chain);
		})
		.child(value)
		.add_any_attr(attrs)
		.into_any()
}

/// The `autocapitalize` value HTML reads for each capitalization rule.
fn capitalize_value(capitalize: Capitalize) -> &'static str {
	match capitalize {
		Capitalize::None => "none",
		Capitalize::Words => "words",
		Capitalize::Sentences => "sentences",
		Capitalize::Characters => "characters",
	}
}

/// A spinner whose animation and accessibility follow whether it is running.
fn activity_indicator(running: bool, attrs: impl Attribute + 'static) -> AnyView {
	let flag = if running { "true" } else { "false" };
	html::span()
		.role("progressbar")
		.attr("aria-hidden", (!running).then_some("true"))
		.attr("aria-busy", flag)
		.attr("data-lk-running", flag)
		.add_any_attr(attrs)
		.into_any()
}

/// Mirrors `NativeSurface`: invoke every matching handler on the current node.
/// Browser scroll does not bubble, so walk the native ancestor chain.
fn on_scroll(scroll_chain: &[Handlers], event: &web_sys::Event) {
	if !controls::interactive(event) {
		return;
	}

	let target = event
		.target()
		.and_then(|target| target.dyn_into::<web_sys::Element>().ok())
		.expect("scroll events from leptoskit controls target DOM elements");
	let native = NativeEvent::Scroll {
		offset_x: target.scroll_left() as f32,
		offset_y: target.scroll_top() as f32,
	};

	// Browser scroll does not bubble, so walk the native ancestor
	// chain.
	for handlers in scroll_chain.iter().rev() {
		if dispatch(handlers, &native, event) == EventOutcome::Handled {
			break;
		}
	}

	event.stop_propagation();
}

/// Key handlers hear every key inside them, typing included, before the
/// focused control acts on it.
fn on_keydown(
	handlers: &[HandlerEntry],
	submit: &[HandlerEntry],
	key_press: &[HandlerEntry],
	listens_for_keys: bool,
	custom_control: bool,
	event: &web_sys::KeyboardEvent,
) {
	if !controls::interactive(event.as_ref()) {
		return;
	}

	// Key handlers hear every key inside them, typing included, before
	// the focused control acts on it. A handled key loses its browser
	// meaning, so ctrl+p does not print. A key no handler takes keeps
	// it, so the browser's own command-key shortcuts still work.
	if listens_for_keys && !event.is_composing() {
		let native = NativeEvent::Key(crate::KeyPress {
			key: event.key(),
			ctrl: event.ctrl_key(),
			alt: event.alt_key(),
			shift: event.shift_key(),
			meta: event.meta_key(),
		});

		if dispatch(handlers, &native, event.as_ref()) == EventOutcome::Handled {
			event.prevent_default();

			return;
		}
	}

	if controls::typed_target(event.as_ref()) {
		return;
	}

	let input = event
		.target()
		.and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok());

	if let Some(input) = input.filter(|_| event.key() == "Enter" && !event.is_composing()) {
		let native = NativeEvent::Submit {
			value: input.value(),
		};

		if dispatch(submit, &native, event.as_ref()) == EventOutcome::Handled {
			event.prevent_default();
		}

		return;
	}

	// A container with a control role has no native activation, so
	// Enter and Space press it, as they do a `<button>`. Only the
	// focused element itself activates, not an ancestor control.
	if custom_control
		&& matches!(event.key().as_str(), "Enter" | " ")
		&& !event.is_composing()
		&& event.target() == event.current_target()
		&& dispatch(key_press, &NativeEvent::Press, event.as_ref()) == EventOutcome::Handled
	{
		event.prevent_default();
	}
}

fn dispatch(
	handlers: &[HandlerEntry],
	native: &NativeEvent,
	event: &web_sys::Event,
) -> EventOutcome {
	let mut outcome = EventOutcome::Unhandled;

	for entry in handlers {
		if entry.event.matches(native) && (entry.handler)(native) == EventOutcome::Handled {
			outcome = EventOutcome::Handled;
		}
	}

	if outcome == EventOutcome::Handled {
		event.stop_propagation();
	}

	outcome
}

/// A `Text`'s styled runs as inline spans, in reading order.
fn spans_view(spans: Vec<crate::TextSpan>) -> AnyView {
	spans
		.into_iter()
		.map(|span| {
			let style = styles::span(&span);
			html::span().child(span.text).add_any_attr(custom_attribute(
				"style",
				(!style.is_empty()).then_some(style),
			))
		})
		.collect_view()
		.into_any()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod screen_tests {
	use super::*;
	use crate::remember::remember;

	#[test]
	fn a_screen_keeps_what_it_remembers_from_build_to_build() {
		Owner::new().with(|| {
			let taps = Arc::new(std::sync::Mutex::new(Vec::new()));
			let seen = Arc::clone(&taps);
			let build = rebuilds(move || {
				let count = remember(|| 0_u32);
				count.update(|count| *count += 1);
				seen.lock().unwrap().push(count.get_untracked());
				Element::text(count.get_untracked().to_string())
			});
			let first = build().to_html();
			let second = build().to_html();
			assert_eq!(*taps.lock().unwrap(), [1, 2]);
			assert!(first.contains(">1<"), "{first}");
			assert!(second.contains(">2<"), "{second}");
		});
	}

	#[test]
	fn a_screen_renders_what_its_view_builds() {
		Owner::new().with(|| {
			let html = screen(|| Element::text("Home")).into_view().to_html();
			assert!(html.contains("Home"), "{html}");
		});
	}
}

#[cfg(test)]
mod devtools_tests {
	use super::*;

	fn tree() -> Element {
		let card = |title: &str| Element::view().child(Element::text(title));
		Element::view()
			.child(Element::text("Clips"))
			.child(card("first"))
			.child(card("second"))
	}

	#[test]
	fn attributes_appear_only_with_the_feature() {
		let element = tree();
		let attributes = NodePath::root().attributes(&element);
		#[cfg(feature = "devtools")]
		{
			assert_eq!(attributes.node.as_deref(), Some(""));
			assert_eq!(attributes.kind.as_deref(), Some("View"));
			assert!(
				attributes
					.source
					.is_some_and(|source| source.contains("mod.rs"))
			);
			assert_eq!(attributes.key.map(|key| key.len()), Some(16));
		}

		#[cfg(not(feature = "devtools"))]
		assert!(attributes.node.is_none() && attributes.key.is_none());
		#[cfg(feature = "devtools")]
		{
			let paths = NodePath::root().children(&element.children);
			assert_eq!(
				paths
					.iter()
					.map(|path| path.path.as_str())
					.collect::<Vec<_>>(),
				[".0", ".1", ".2"]
			);
		}
	}

	/// The web and native builds name each element the same way, so a node
	/// picked in one finds its twin in the other.
	#[cfg(feature = "devtools")]
	#[test]
	fn web_keys_match_native_keys() {
		fn web_keys(element: &Element, path: &NodePath, keys: &mut Vec<String>) {
			keys.push(crate::devtools::format(path.key));

			for (child, path) in element
				.children
				.iter()
				.zip(path.children(&element.children))
			{
				web_keys(child, &path, keys);
			}
		}

		let element = tree();
		let mut web = Vec::new();
		web_keys(&element, &NodePath::root(), &mut web);

		let mut surface = crate::NativeSurface::new(crate::HeadlessBackend::new());
		surface.set_viewport(320.0, 480.0);
		surface.set_root(&element);
		let native: Vec<String> = surface
			.devtools_tree()
			.nodes
			.into_iter()
			.map(|node| node.key.0)
			.collect();

		assert_eq!(web, native);
		let unique: std::collections::BTreeSet<_> = web.iter().collect();
		assert_eq!(unique.len(), web.len(), "every node has its own key");
	}
}
