//! The declarative element tree apps build each frame.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::Arc;

use leptoskit_yoga::Style;

use crate::Capitalize;
use crate::Keyboard;
use crate::SubmitLabel;
use crate::aria::Aria;
use crate::aria::Current;
use crate::aria::HasPopup;
use crate::aria::Invalid;
use crate::aria::Live;
use crate::aria::Orientation;
use crate::aria::Role;
use crate::aria::Tristate;
use crate::controls::Progress;
use crate::controls::RadioGroup;
use crate::controls::SliderState;
use crate::hash::FastHash;
use crate::props::Color;
use crate::props::EventHandler;
use crate::props::FontWeight;
use crate::props::NativeEvent;
use crate::props::NativeKind;
use crate::props::NativeProps;
use crate::props::PropValue;
use crate::props::ScrollOffset;
use crate::props::TextAlign;
use crate::reactive::Binding;
use crate::reactive::Bindings;
use crate::tailwind::ClassError;
use crate::tailwind::StyleContext;
use crate::tailwind::Tailwind;
use crate::vector::Drawing;
use crate::vector::VectorAlignment;
use crate::vector::VectorPath;
use crate::vector::VectorScaling;
use crate::vector::ViewBox;

/// A background colour: a value, or a closure that reads one from application
/// state.
///
/// [`Element::background`] takes either. A closure binds the value, so writing
/// the signal it reads rewrites the node instead of rebuilding the screen
/// around it.
pub trait IntoBackground {
	/// Paints this background onto `element`.
	fn into_background(self, element: Element) -> Element;
}

impl IntoBackground for Color {
	fn into_background(self, mut element: Element) -> Element {
		element.props.background = Some(self);
		element
	}
}

impl<F> IntoBackground for F
where
	F: Fn() -> Color + Send + Sync + 'static,
{
	fn into_background(self, mut element: Element) -> Element {
		let binding = Binding::new(self);
		element.props.background = Some(crate::reactive::read_binding(&binding));
		element.bindings.get_or_insert_default().background = Some(binding);
		element
	}
}

/// One node of a declarative interface: a widget kind, its style, its props,
/// its children, and its event handlers.
///
/// Elements are cheap values that describe an interface; the
/// [`NativeSurface`](crate::NativeSurface) diffs them against the committed
/// tree and turns the difference into native mutations. This mirrors how a
/// leptos component's view is a description that the renderer reconciles.
#[derive(Clone)]
pub struct Element {
	/// The widget kind.
	pub kind: NativeKind,
	/// The layout style.
	pub style: Style,
	/// The widget properties.
	pub props: NativeProps,
	/// The children, in order.
	pub children: Vec<Element>,
	/// Where the element was created, when the `devtools` feature is on.
	#[cfg(feature = "devtools")]
	source: Option<&'static std::panic::Location<'static>>,
	/// The `#[component]` whose body created the element, when the
	/// `devtools` feature is on.
	#[cfg(feature = "devtools")]
	component: Option<&'static str>,
	/// The classes a resolved element was built from, which `DevTools`
	/// reports; empty on an element that is not resolved.
	#[cfg(feature = "devtools")]
	applied: Tailwind,
	classes: Tailwind,
	handlers: Vec<HandlerEntry>,
	gesture: Option<Box<crate::gesture::Gesture>>,
	/// Values that follow application state instead of being rebuilt with the
	/// tree: see [`Element::text_from`] and [`Element::background_from`].
	bindings: Option<Box<Bindings>>,
	/// Children rebuilt on their own when the signals they read change.
	/// [`For`](crate::reactive::For) and [`Show`](crate::reactive::Show) set
	/// it.
	pub(crate) region: Option<crate::reactive::Region>,
}

/// Describes what the element renders, leaving out where it was created: the
/// `devtools` source location would make the output change with the feature,
/// and with every edit that moves a line.
#[cfg_attr(
	feature = "devtools",
	expect(
		clippy::missing_fields_in_debug,
		reason = "the source location is left out on purpose, so the output describes only what \
		          renders"
	)
)]
impl std::fmt::Debug for Element {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let mut debug = f.debug_struct("Element");
		debug
			.field("kind", &self.kind)
			.field("style", &self.style)
			.field("props", &self.props)
			.field("children", &self.children)
			.field("classes", &self.classes)
			.field("handlers", &self.handlers);
		if let Some(gesture) = &self.gesture {
			debug.field("gesture", gesture);
		}
		// Bindings and a region are absent on almost every node. Leaving them
		// out of the debug output when absent keeps the digest of an unchanged
		// node stable.
		if let Some(bindings) = self.bindings.as_deref().filter(|b| !b.is_empty()) {
			debug.field("bindings", bindings);
		}

		if self.region.is_some() {
			debug.field("region", &true);
		}

		debug.finish()
	}
}

#[derive(Clone)]
pub(crate) struct HandlerEntry {
	pub(crate) event: NativeEventSlot,
	pub(crate) handler: EventHandler,
}

/// Which events an element listens for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeEventSlot {
	/// [`NativeEvent::Press`].
	Press,
	/// [`NativeEvent::Change`].
	Change,
	/// [`NativeEvent::Submit`].
	Submit,
	/// [`NativeEvent::Focus`].
	Focus,
	/// [`NativeEvent::Blur`].
	Blur,
	/// [`NativeEvent::Toggle`].
	Toggle,
	/// [`NativeEvent::ValueChange`].
	ValueChange,
	/// [`NativeEvent::Scroll`].
	Scroll,
	/// [`NativeEvent::Key`].
	Key,
	/// A request to close an open overlay.
	Dismiss,
}

impl NativeEventSlot {
	/// Whether a concrete event belongs to this slot.
	pub fn matches(self, event: &NativeEvent) -> bool {
		matches!(
			(self, event),
			(Self::Press, NativeEvent::Press)
				| (Self::Change, NativeEvent::Change { .. })
				| (Self::Submit, NativeEvent::Submit { .. })
				| (Self::Focus, NativeEvent::Focus)
				| (Self::Blur, NativeEvent::Blur)
				| (Self::Scroll, NativeEvent::Scroll { .. })
				| (Self::Toggle, NativeEvent::Toggle { .. })
				| (Self::ValueChange, NativeEvent::ValueChange { .. })
				| (Self::Key, NativeEvent::Key(_))
				| (Self::Dismiss, NativeEvent::Dismiss)
		)
	}
}

/// The share of its opacity a held control without `active:` utilities
/// keeps. Strong enough to read through a quick tap, light enough not to
/// flash; the web adapter emits the same factor as an `opacity()` filter.
pub(crate) const DEFAULT_PRESSED_OPACITY: f32 = 0.7;

/// Whether a kind has a pressed state for `active:` utilities to style.
pub(crate) fn is_pressable(kind: NativeKind) -> bool {
	matches!(kind, NativeKind::Button | NativeKind::Pressable)
}

/// The subset of a parent's resolved props inherited by its children.
///
/// Kept small and `Copy` (~40 bytes) so patching and streaming reconciliation
/// never clone the parent's full `NativeProps`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InheritedProps {
	pub(crate) color: Option<Color>,
	pub(crate) font_size: Option<f32>,
	pub(crate) font_weight: Option<FontWeight>,
	pub(crate) line_height: Option<crate::props::LineHeight>,
	pub(crate) italic: Option<bool>,
	pub(crate) text_align: Option<TextAlign>,
	pub(crate) disabled: bool,
	pub(crate) aria_hidden: bool,
	pub(crate) hidden: bool,
}

impl InheritedProps {
	#[inline]
	pub(crate) fn from_props(props: &NativeProps) -> Self {
		Self {
			color: props.color,
			font_size: props.font_size,
			font_weight: props.font_weight,
			line_height: props.line_height,
			italic: props.italic,
			text_align: props.text_align,
			disabled: props.enabled == Some(false),
			aria_hidden: props.aria().hidden == Some(true),
			hidden: props.hidden == Some(true),
		}
	}
}

/// The resolved fields of a single element without cloning its children,
/// handlers, bindings, or region metadata.
pub(crate) struct ResolvedOwn {
	pub(crate) kind: NativeKind,
	pub(crate) style: Style,
	pub(crate) props: NativeProps,
}

impl Element {
	/// Checks structural control and accessibility contracts before any
	/// backend mutations occur.
	///
	/// Radio names are scoped to this complete tree. At most one option may be
	/// checked in each group; the application owns that selection. Ids are
	/// unique within the tree, ARIA relationships reference ids that exist,
	/// and pressables and text inputs carry an accessible name.
	pub fn validate(&self) -> Result<(), crate::controls::ControlTreeError> {
		self.validate_under(None, true)
	}

	/// Validates this subtree where it sits inside a larger tree, below its
	/// nearest `Button` or `Pressable` ancestor. Only sound for a subtree
	/// that passes [`Self::is_self_contained`], since ids and radio groups
	/// are scoped to the complete tree.
	pub fn validate_within(
		&self,
		button_ancestor: Option<NativeKind>,
	) -> Result<(), crate::controls::ControlTreeError> {
		self.validate_under(button_ancestor, false)
	}

	/// Whether nothing in this subtree depends on the rest of the tree for
	/// validation or resolution: no ids, id references, radio options, or
	/// live regions.
	pub fn is_self_contained(&self) -> bool {
		let mut pending = vec![self];

		while let Some(element) = pending.pop() {
			if element.declares_relationships()
				|| element
					.props
					.aria
					.as_deref()
					.is_some_and(|aria| !aria.controls.is_empty())
			{
				return false;
			}

			pending.extend(&element.children);
		}

		true
	}

	fn validate_under(
		&self,
		button_ancestor: Option<NativeKind>,
		collect_ids: bool,
	) -> Result<(), crate::controls::ControlTreeError> {
		let mut pending = vec![(self, button_ancestor)];
		let mut ids: HashSet<&str, FastHash> = HashSet::default();
		let mut id_refs: Vec<(&'static str, &str)> = Vec::new();
		let mut selected_groups: HashSet<&str, FastHash> = HashSet::default();

		while let Some((element, button_ancestor)) = pending.pop() {
			if collect_ids
				&& let Some(id) = element.props.id.as_deref()
				&& !ids.insert(id)
			{
				return Err(crate::controls::ControlTreeError::DuplicateId { id: id.to_owned() });
			}

			element.check(button_ancestor, &mut id_refs, &mut selected_groups)?;

			let button_ancestor =
				if matches!(element.kind, NativeKind::Button | NativeKind::Pressable) {
					Some(element.kind)
				} else {
					button_ancestor
				};

			pending.extend(
				element
					.children
					.iter()
					.map(|child| (child, button_ancestor)),
			);
		}

		for (attribute, id) in id_refs {
			if !ids.contains(id) {
				return Err(crate::controls::ControlTreeError::UnknownIdReference {
					attribute,
					id: id.to_owned(),
				});
			}
		}

		Ok(())
	}

	/// The control and accessibility checks for one element.
	#[allow(clippy::too_many_lines)]
	fn check<'a>(
		&'a self,
		button_ancestor: Option<NativeKind>,
		id_refs: &mut Vec<(&'static str, &'a str)>,
		selected_groups: &mut HashSet<&'a str, FastHash>,
	) -> Result<(), crate::controls::ControlTreeError> {
		use crate::controls::ControlTreeError;

		#[cfg(not(feature = "rive"))]
		if self.kind == NativeKind::Rive {
			return Err(ControlTreeError::RiveNotEnabled);
		}

		if let Some(container) = button_ancestor
			&& (matches!(
				self.kind,
				NativeKind::Button
					| NativeKind::Pressable
					| NativeKind::ScrollView
					| NativeKind::TextInput
					| NativeKind::TextArea
					| NativeKind::PasswordInput
					| NativeKind::CanvistEditor
					| NativeKind::Switch
					| NativeKind::Checkbox
					| NativeKind::Radio
					| NativeKind::Slider
			) || self.style.overflow == leptoskit_yoga::Overflow::Scroll
				|| self.classes.may_scroll()
				|| (container == NativeKind::Pressable
					&& (!self.handlers.is_empty() || matches!(self.kind, NativeKind::Custom(_)))))
		{
			return Err(ControlTreeError::InteractiveDescendant {
				container,
				descendant: self.kind,
			});
		}

		if self.kind == NativeKind::Pressable && !self.props.names_itself() {
			return Err(ControlTreeError::MissingPressableLabel);
		}

		if self.kind != NativeKind::Rive && self.props.rive.is_some() {
			return Err(ControlTreeError::InvalidRive);
		}

		if (self.kind == NativeKind::Scene3d) != self.props.scene3d.is_some()
			|| (self.kind == NativeKind::Scene3d
				&& self
					.props
					.accessibility_label
					.as_deref()
					.is_none_or(|label| label.trim().is_empty())
				&& self.props.aria().hidden != Some(true))
		{
			return Err(ControlTreeError::InvalidScene3d);
		}

		if (self.kind == NativeKind::Video) != self.props.video.is_some()
			|| (self.kind == NativeKind::Video
				&& self
					.props
					.accessibility_label
					.as_deref()
					.is_none_or(|label| label.trim().is_empty())
				&& self.props.aria().hidden != Some(true))
		{
			return Err(ControlTreeError::InvalidVideo);
		}

		if (self.kind == NativeKind::Camera) != self.props.camera.is_some()
			|| (self.kind == NativeKind::Camera
				&& self
					.props
					.accessibility_label
					.as_deref()
					.is_none_or(|label| label.trim().is_empty())
				&& self.props.aria().hidden != Some(true))
		{
			return Err(ControlTreeError::InvalidCamera);
		}

		if (self.kind == NativeKind::Map) != self.props.map.is_some()
			|| (self.kind == NativeKind::Map
				&& self
					.props
					.accessibility_label
					.as_deref()
					.is_none_or(|label| label.trim().is_empty())
				&& self.props.aria().hidden != Some(true))
		{
			return Err(ControlTreeError::InvalidMap);
		}

		if (self.kind == NativeKind::LivePhoto) != self.props.live_photo.is_some()
			|| (self.kind == NativeKind::LivePhoto
				&& self
					.props
					.accessibility_label
					.as_deref()
					.is_none_or(|label| label.trim().is_empty())
				&& self.props.aria().hidden != Some(true))
		{
			return Err(ControlTreeError::InvalidLivePhoto);
		}

		if (self.kind == NativeKind::CanvistEditor) != self.props.canvist_editor.is_some() {
			return Err(ControlTreeError::InvalidCanvistEditor);
		}

		if self.kind == NativeKind::Rive
			&& (self.props.rive.is_none()
				|| self.props.source.is_some()
				|| (self
					.props
					.accessibility_label
					.as_deref()
					.is_none_or(|label| label.trim().is_empty())
					&& self.props.aria().hidden != Some(true)))
		{
			return Err(ControlTreeError::InvalidRive);
		}

		if matches!(
			self.kind,
			NativeKind::Image
				| NativeKind::Rive
				| NativeKind::Scene3d
				| NativeKind::Video
				| NativeKind::Camera
				| NativeKind::Map
				| NativeKind::LivePhoto
				| NativeKind::TextInput
				| NativeKind::TextArea
				| NativeKind::PasswordInput
				| NativeKind::CanvistEditor
				| NativeKind::Switch
				| NativeKind::Checkbox
				| NativeKind::Radio
				| NativeKind::Slider
				| NativeKind::LinearProgressIndicator
				| NativeKind::ActivityIndicator
				| NativeKind::Vector
		) && !self.children.is_empty()
		{
			return Err(ControlTreeError::Children { kind: self.kind });
		}

		if matches!(
			self.kind,
			NativeKind::TextInput
				| NativeKind::TextArea
				| NativeKind::PasswordInput
				| NativeKind::CanvistEditor
		) && !self.props.names_itself()
			&& self
				.props
				.placeholder
				.as_deref()
				.is_none_or(|placeholder| placeholder.trim().is_empty())
		{
			return Err(ControlTreeError::MissingInputLabel { kind: self.kind });
		}

		if matches!(self.kind, NativeKind::TextInput | NativeKind::PasswordInput)
			&& self
				.props
				.aria
				.as_deref()
				.is_some_and(|a| a.multiline == Some(true))
		{
			return Err(ControlTreeError::MultilineTextInput { kind: self.kind });
		}

		if self
			.props
			.scroll_offset
			.is_some_and(|offset| !offset.is_finite())
		{
			return Err(ControlTreeError::InvalidScrollOffset);
		}

		if self.props.aria.is_some() || self.props.overlay.is_some() {
			self.validate_aria(id_refs)?;
		}

		if self.kind == NativeKind::Radio {
			let group = self
				.props
				.radio_group
				.as_ref()
				.ok_or(ControlTreeError::MissingRadioGroup)?;

			if self.props.checked == Some(true) && !selected_groups.insert(group.name()) {
				return Err(ControlTreeError::MultipleSelectedRadio {
					group: group.name().to_owned(),
				});
			}
		}

		Ok(())
	}

	/// Checks one element's ARIA properties against the spec rules that do not
	/// depend on the platform.
	fn validate_aria<'a>(
		&'a self,
		id_refs: &mut Vec<(&'static str, &'a str)>,
	) -> Result<(), crate::controls::ControlTreeError> {
		use crate::controls::ControlTreeError;

		if let Some(overlay) = &self.props.overlay {
			if let Some(id) = overlay.anchor.as_deref() {
				id_refs.push(("the popover's anchor", id));
			}

			if let Some(id) = overlay.initial_focus.as_deref() {
				id_refs.push(("initial_focus", id));
			}
		}

		let Some(aria) = self.props.aria.as_deref() else {
			return Ok(());
		};

		if aria.role.is_some_and(Role::is_presentational) && self.props.names_itself() {
			return Err(ControlTreeError::LabelOnPresentation);
		}

		if aria.level == Some(0) {
			return Err(ControlTreeError::InvalidAriaLevel);
		}

		for (attribute, references) in [
			("aria-labelledby", &aria.labelled_by),
			("aria-describedby", &aria.described_by),
			("aria-controls", &aria.controls),
		] {
			for id in references {
				id_refs.push((attribute, id.as_str()));
			}
		}

		if let Some(name) = aria
			.extra
			.keys()
			.find(|name| !crate::aria::accepts_attribute(name))
		{
			return Err(ControlTreeError::InvalidAriaAttribute { name: name.clone() });
		}

		Ok(())
	}

	/// Creates an element of the given kind with default style and props.
	#[inline]
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn new(kind: NativeKind) -> Self {
		Self {
			kind,
			style: Style::default(),
			props: NativeProps::default(),
			children: Vec::new(),
			#[cfg(feature = "devtools")]
			source: Some(std::panic::Location::caller()),
			#[cfg(feature = "devtools")]
			component: crate::devtools::current_component(),
			#[cfg(feature = "devtools")]
			applied: Tailwind::default(),
			classes: Tailwind::default(),
			handlers: Vec::new(),
			gesture: None,
			bindings: None,
			region: None,
		}
	}

	/// Where the element was created, when the `devtools` feature is on.
	#[cfg(feature = "devtools")]
	#[must_use]
	pub fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
		self.source
	}

	/// The `#[component]` whose body created the element, when the
	/// `devtools` feature is on.
	#[cfg(feature = "devtools")]
	#[must_use]
	pub fn component_name(&self) -> Option<&'static str> {
		self.component
	}

	/// The classes a resolved element was built from, when the `devtools`
	/// feature is on. Resolving applies an element's classes and clears them;
	/// these keep what was applied.
	#[cfg(feature = "devtools")]
	#[must_use]
	pub fn applied_classes(&self) -> &Tailwind {
		&self.applied
	}

	/// A `View` element.
	#[inline]
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn view() -> Self {
		Self::new(NativeKind::View)
	}

	/// A `Text` element whose content follows application state.
	///
	/// The closure reads a [signal](crate::reactive::RwSignal), so a renderer
	/// that supports bindings keeps this node and rewrites only its text when
	/// the signal changes: a hundred changed labels cost a hundred nodes
	/// rather than a rebuilt tree. A renderer that does not support bindings
	/// still shows the text read when the element was built.
	///
	/// Writing the signal — from an event, a timer, or a plugin reply —
	/// rewrites this node. The guest does not rebuild the screen to do it.
	/// See [`guest::flush`](crate::guest::flush).
	#[cfg_attr(feature = "devtools", track_caller)]
	#[must_use]
	pub fn text_from(read: impl Fn() -> String + Send + Sync + 'static) -> Self {
		let reactive = Binding::new(read);
		let mut element = Self::new(NativeKind::Text);
		element.props.text = Some(crate::reactive::read_binding(&reactive));
		element.bindings.get_or_insert_default().text = Some(reactive);
		element
	}

	/// A background colour that follows application state.
	///
	/// The same rule as [`text_from`](Self::text_from): the renderer keeps the
	/// node and rewrites its colour when the bound signal changes, instead of
	/// rebuilding the tree around it.
	#[cfg_attr(feature = "devtools", track_caller)]
	#[must_use]
	pub fn background_from(read: impl Fn() -> Color + Send + Sync + 'static) -> Self {
		let reactive = Binding::new(read);
		let mut element = Self::new(NativeKind::View);
		element.props.background = Some(crate::reactive::read_binding(&reactive));
		element.bindings.get_or_insert_default().background = Some(reactive);
		element
	}

	/// The values this node reads from application state.
	pub fn bindings(&self) -> &Bindings {
		static EMPTY: Bindings = Bindings {
			text: None,
			background: None,
		};
		self.bindings.as_deref().unwrap_or(&EMPTY)
	}

	/// A `Text` element with the given content.
	#[inline]
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn text(content: impl Into<String>) -> Self {
		let mut element = Self::new(NativeKind::Text);
		element.props.text = Some(content.into());
		element
	}

	/// An `Image` element with the given source.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn image(source: impl Into<String>) -> Self {
		let mut element = Self::new(NativeKind::Image);
		element.props.source = Some(source.into());
		element
	}

	/// A `Vector` element drawing paths in `view_box` coordinates, scaled to
	/// fit its frame. Add paths with [`Self::path`].
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn vector(view_box: ViewBox) -> Self {
		let mut element = Self::new(NativeKind::Vector);
		element.props.drawing = Some(Drawing::new(view_box));
		element
	}

	/// A Rive scene owned and animated by the native host. Give it an explicit
	/// size and an accessible label, or mark it decorative with `aria_hidden`.
	/// Playback is initially paused; use `running(true)` to request animation.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn rive(playback: leptoskit_rive::Playback) -> Self {
		let mut element = Self::new(NativeKind::Rive);
		element.props.rive = Some(Box::new(playback));
		element
	}

	/// A 3D scene the host owns and draws on the GPU, by its id. Give it an
	/// explicit size and an accessible label, or mark it decorative with
	/// `aria_hidden`. The guest changes the scene with ops (see
	/// `leptoskit_3d::client::Scene`), never by re-rendering this element.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn scene3d(scene: leptoskit_3d::SceneId) -> Self {
		let mut element = Self::new(NativeKind::Scene3d);
		element.props.scene3d = Some(scene.0);
		element
	}

	/// Sets the scene a `Scene3d` view tag shows.
	#[must_use]
	pub fn scene(mut self, scene: leptoskit_3d::SceneId) -> Self {
		self.props.scene3d = Some(scene.0);
		self
	}

	/// A video surface the host owns, by player id. Give it an explicit size
	/// and an accessible label, or mark it decorative with `aria_hidden`. The
	/// guest changes the player through the `video` plugin, never by
	/// re-rendering this element.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn video(player: u32) -> Self {
		let mut element = Self::new(NativeKind::Video);
		element.props.video = Some(player);
		element
	}

	/// Sets the player a `Video` view tag shows.
	#[must_use]
	pub fn player(mut self, player: u32) -> Self {
		self.props.video = Some(player);
		self
	}

	/// A camera preview the host owns, by session id. Give it an explicit size
	/// and an accessible label, or mark it decorative with `aria_hidden`. The
	/// guest changes the session through the `camera` plugin, never by
	/// re-rendering this element.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn camera(session: u32) -> Self {
		let mut element = Self::new(NativeKind::Camera);
		element.props.camera = Some(session);
		element
	}

	/// Sets the session a `Camera` view tag shows.
	#[must_use]
	pub fn session(mut self, session: u32) -> Self {
		self.props.camera = Some(session);
		self
	}

	/// A map surface the host owns, by session id. Give it an explicit size
	/// and an accessible label, or mark it decorative with `aria_hidden`. The
	/// guest changes the session through the `maps` plugin, never by
	/// re-rendering this element.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn map(session: u32) -> Self {
		let mut element = Self::new(NativeKind::Map);
		element.props.map = Some(session);
		element
	}

	/// Sets the session a `Map` view tag shows.
	#[must_use]
	pub fn map_session(mut self, session: u32) -> Self {
		self.props.map = Some(session);
		self
	}

	/// A Live Photo surface the host owns, by session id. Give it an explicit
	/// size and an accessible label, or mark it decorative with `aria_hidden`.
	/// The guest changes the session through the `live-photo` plugin, never by
	/// re-rendering this element.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn live_photo(session: u32) -> Self {
		let mut element = Self::new(NativeKind::LivePhoto);
		element.props.live_photo = Some(session);
		element
	}

	/// Sets the session a `LivePhoto` view tag shows.
	#[must_use]
	pub fn live_photo_session(mut self, session: u32) -> Self {
		self.props.live_photo = Some(session);
		self
	}

	/// A canvist text editor the app owns, by its id: where the editor shows.
	/// The document, its changes and its selection stay with the editor's
	/// state handle and never pass through this element. Give it an accessible
	/// label, a labelling reference, or a placeholder, as for any text field.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn canvist_editor(editor: u32) -> Self {
		let mut element = Self::new(NativeKind::CanvistEditor);
		element.props.canvist_editor = Some(editor);
		element
	}

	/// Sets the editor a `CanvistEditor` view tag shows.
	#[must_use]
	pub fn editor(mut self, editor: u32) -> Self {
		self.props.canvist_editor = Some(editor);
		self
	}

	/// Sets the content of a `Rive` view tag. Parsed players are retained when
	/// the asset, artboard, and scene are unchanged.
	#[must_use]
	pub fn rive_playback(mut self, playback: leptoskit_rive::Playback) -> Self {
		self.props.rive = Some(Box::new(playback));
		self
	}

	/// A `ScrollView` element; its children scroll when they overflow it.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn scroll_view() -> Self {
		Self::new(NativeKind::ScrollView)
	}

	/// A `TextInput` element.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn text_input() -> Self {
		Self::new(NativeKind::TextInput)
	}

	/// A `TextArea` element: a multi-line text field.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn text_area() -> Self {
		Self::new(NativeKind::TextArea)
	}

	/// A `Modal` element: content above the screen that blocks it while
	/// `open`.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn modal(open: bool) -> Self {
		let mut element = Self::new(NativeKind::Modal);
		// A modal fills the screen, and what it holds sits in the middle.
		element.style.align_items = leptoskit_yoga::Align::Center;
		element.style.justify_content = leptoskit_yoga::JustifyContent::Center;
		element.props.overlay = Some(crate::Overlay {
			open,
			..crate::Overlay::default()
		});
		element
	}

	/// A `Popover` element: content above the screen while `open`, hanging
	/// from the element whose `id` is `anchor`.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn popover(anchor: impl Into<String>, open: bool) -> Self {
		let mut element = Self::new(NativeKind::Popover);
		element.props.overlay = Some(crate::Overlay {
			open,
			anchor: Some(anchor.into()),
			..crate::Overlay::default()
		});
		element
	}

	/// A `Button` element with the given label.
	#[cfg_attr(feature = "devtools", track_caller)]
	pub fn button(label: impl Into<String>) -> Self {
		let mut element = Self::new(NativeKind::Button);
		element.style.justify_content = leptoskit_yoga::JustifyContent::Center;
		element.props.text = Some(label.into());
		element
	}

	/// Appends built-in Tailwind utilities to this component.
	///
	/// Classes override the typed layout and visual properties, regardless of
	/// builder call order. Responsive rules are retained until rendering, so
	/// viewport changes do not require rebuilding the screen. `active:` rules
	/// style a `Button` or `Pressable` while it is held down; other kinds have
	/// no pressed state to style.
	///
	/// # Panics
	///
	/// Panics with a diagnostic for an invalid or unsupported class. Use
	/// [`Self::try_class`] when classes come from dynamic input.
	#[must_use]
	pub fn class(self, classes: impl AsRef<str>) -> Self {
		self.try_class(classes)
			.unwrap_or_else(|error| panic!("invalid leptoskit class: {error}"))
	}

	/// Appends utilities, returning a diagnostic before any rendering occurs.
	pub fn try_class(mut self, classes: impl AsRef<str>) -> Result<Self, ClassError> {
		// Most elements get one class string; joining strings without
		// `format!` matters inside the native guest's interpreter, where
		// formatting cost more than the cached parse itself.
		let existing = self.classes.classes();
		let parsed = if existing.is_empty() {
			Tailwind::parse(classes.as_ref())?
		} else {
			let mut combined = String::with_capacity(existing.len() + 1 + classes.as_ref().len());
			combined.push_str(existing);
			combined.push(' ');
			combined.push_str(classes.as_ref());
			Tailwind::parse(&combined)?
		};

		// On the web, `:active` also matches containers while the pointer is
		// held anywhere inside them, which native platforms cannot report.
		// Pressed styles stay on the controls themselves so both agree.
		if !is_pressable(self.kind)
			&& let Some(class) = parsed.active_class()
		{
			return Err(ClassError {
				class: class.to_owned(),
				reason: format!(
					"`active:` styles a pressed Button or Pressable, and {} has no pressed state",
					self.kind
				),
			});
		}

		self.classes = parsed;
		Ok(self)
	}

	/// The parsed utility program shared by native and web rendering.
	pub fn classes(&self) -> &Tailwind {
		&self.classes
	}

	/// Resolves this tree for an environment, including inherited text styles
	/// and accessibility relationships.
	///
	/// The source tree remains unchanged. The returned snapshot retains no
	/// classes: it is ready for backend reconciliation or serialization.
	/// Names referenced through `aria_labelled_by` and `aria_described_by`
	/// are folded into `label` and `description`, radio options learn their
	/// position in their group, and live regions carry the text to announce,
	/// so native backends never resolve relationships themselves.
	///
	/// A held control, one resolved with [`StyleContext::active`], takes its
	/// `active:` utilities. Without any, a `Button` drawing its own
	/// background, border, or corners and every `Pressable` dim to
	/// seventy percent of their opacity instead; a button keeping its native
	/// chrome keeps the platform's own pressed highlight.
	#[must_use]
	pub fn resolved(&self, context: &StyleContext) -> Self {
		self.resolved_holding(context, None)
	}

	/// Resolves with the control at child-index path `held` pressed, the
	/// empty path naming this element.
	pub(crate) fn resolved_holding(&self, context: &StyleContext, held: Option<&[usize]>) -> Self {
		self.resolved_tracking(context, held).0
	}

	/// [`Self::resolved_holding`], also reporting whether the tree declares
	/// accessibility relationships that resolve across nodes.
	pub(crate) fn resolved_tracking(
		&self,
		context: &StyleContext,
		held: Option<&[usize]>,
	) -> (Self, bool) {
		let (mut element, uses_aria) =
			self.resolve_inherited(context, InheritedProps::default(), held);
		// Nothing resolves relationships, set positions, or live text in a
		// tree that declares none, and the walk would dominate commits.
		if uses_aria {
			element.resolve_accessibility();
		}

		(element, uses_aria)
	}

	/// Resolves this subtree under its parent's inherited props, without the
	/// tree-wide accessibility pass. Callers check
	/// [`Self::declares_relationships`] first.
	pub(crate) fn resolve_inherited(
		&self,
		context: &StyleContext,
		parent: InheritedProps,
		held: Option<&[usize]>,
	) -> (Self, bool) {
		let mut element = self.resolve_own(context, parent, held);
		let uses_aria = element.declares_relationships();
		let child_parent = InheritedProps::from_props(&element.props);
		let mut children_uses_aria = false;
		element.children = self
			.children
			.iter()
			.enumerate()
			.map(|(index, child)| {
				let held = match held {
					Some([first, rest @ ..]) if *first == index => Some(rest),
					_ => None,
				};

				let (child, uses) = child.resolve_inherited(context, child_parent, held);
				children_uses_aria |= uses;
				child
			})
			.collect();
		(element, uses_aria || children_uses_aria)
	}

	/// Whether this element takes part in the tree-wide accessibility pass:
	/// it has an id, references other ids, is a radio option, or announces
	/// live changes.
	#[inline]
	pub(crate) fn declares_relationships(&self) -> bool {
		self.props.id.is_some()
			|| self.kind == NativeKind::Radio
			|| self.props.aria.as_deref().is_some_and(|aria| {
				!aria.labelled_by.is_empty()
					|| !aria.described_by.is_empty()
					|| aria.live.is_some()
					|| aria.role.is_some_and(|role| role.implicit_live().is_some())
			})
	}

	/// Whether any element in this subtree declares accessibility
	/// relationships. Read-only scan that allocates nothing.
	pub(crate) fn tree_declares_relationships(&self) -> bool {
		if self.declares_relationships() {
			return true;
		}

		self.children.iter().any(Self::tree_declares_relationships)
	}

	/// Resolves this element's own kind, layout style, and props without
	/// constructing a full `Element` or cloning children/bindings/region.
	pub(crate) fn resolve_own_into(
		&self,
		context: &StyleContext,
		parent: InheritedProps,
		held: Option<&[usize]>,
	) -> ResolvedOwn {
		let mut kind = self.kind;
		let mut style = self.style.clone();
		let mut props = self.props.clone();

		props.color = props.color.or(parent.color);
		props.font_size = props.font_size.or(parent.font_size);
		props.font_weight = props.font_weight.or(parent.font_weight);
		props.line_height = props.line_height.or(parent.line_height);
		props.italic = props.italic.or(parent.italic);
		props.text_align = props.text_align.or(parent.text_align);

		if parent.disabled {
			props.enabled = Some(false);
		}

		if parent.aria_hidden {
			props.aria_mut().hidden = Some(true);
		}

		// Disabled controls cannot be pressed, even if a hold began before
		// the application disabled them.
		let pressed = is_pressable(self.kind)
			&& props.enabled != Some(false)
			&& (context.active || held.is_some_and(<[usize]>::is_empty));
		self.classes.apply(
			&mut style,
			&mut props,
			&StyleContext {
				active: pressed,
				..*context
			},
		);
		props.direction = Some(style.direction);

		// Native backends paint fixed colors; `currentColor` becomes the text
		// color this element inherited or set, else CSS's initial black. The
		// web keeps the keyword and lets CSS resolve it.
		if let Some(drawing) = &mut props.drawing {
			drawing.resolve_current_color(props.color.unwrap_or(Color::rgba(0, 0, 0, 255)));
		}

		if let Some(overlay) = &props.overlay {
			if overlay.open {
				style.position = leptoskit_yoga::PositionType::Absolute;

				if kind == NativeKind::Modal {
					style.width =
						leptoskit_yoga::Dimension::Points(context.viewport.width.max(0.0));
					style.height =
						leptoskit_yoga::Dimension::Points(context.viewport.height.max(0.0));
				}
			} else {
				style.display = leptoskit_yoga::Display::None;
			}
		}

		if style.overflow == leptoskit_yoga::Overflow::Scroll && kind == NativeKind::View {
			kind = NativeKind::ScrollView;
		}

		if kind == NativeKind::ScrollView
			&& (style.overflow == leptoskit_yoga::Overflow::Hidden || props.clip == Some(false))
		{
			kind = NativeKind::View;
		}

		props.sanitize_motion();
		props.hidden = (style.display == leptoskit_yoga::Display::None)
			.then_some(true)
			.or(props.hidden);

		if parent.hidden {
			props.hidden = Some(true);
		}

		props.clip = (style.overflow != leptoskit_yoga::Overflow::Visible
			|| kind == NativeKind::ScrollView
			|| props.corner_radius.is_some_and(|radius| radius > 0.0))
		.then_some(true)
		.or(props.clip);

		if pressed
			&& self.classes.active_class().is_none()
			&& (self.kind == NativeKind::Pressable || props.is_custom_drawn())
		{
			props.opacity = Some(props.opacity.unwrap_or(1.0) * DEFAULT_PRESSED_OPACITY);
		}

		ResolvedOwn { kind, style, props }
	}

	/// Resolves this element alone under its parent's inherited props. The
	/// result has no children.
	pub(crate) fn resolve_own(
		&self,
		context: &StyleContext,
		parent: InheritedProps,
		held: Option<&[usize]>,
	) -> Self {
		let resolved = self.resolve_own_into(context, parent, held);
		Self {
			kind: resolved.kind,
			style: resolved.style,
			props: resolved.props,
			children: Vec::new(),
			#[cfg(feature = "devtools")]
			source: self.source,
			#[cfg(feature = "devtools")]
			component: self.component,
			#[cfg(feature = "devtools")]
			applied: self.classes.clone(),
			classes: Tailwind::default(),
			handlers: self.handlers.clone(),
			gesture: self.gesture.clone(),
			bindings: self.bindings.clone(),
			region: self.region.clone(),
		}
	}

	/// Whether resolving a child under `next` could differ from resolving it
	/// under `previous`: the props children inherit changed.
	pub(crate) fn inherited_differs(previous: &NativeProps, next: &NativeProps) -> bool {
		previous.color != next.color
			|| previous.font_size != next.font_size
			|| previous.font_weight != next.font_weight
			|| previous.line_height != next.line_height
			|| previous.italic != next.italic
			|| previous.text_align != next.text_align
			|| (previous.enabled == Some(false)) != (next.enabled == Some(false))
			|| (previous.aria().hidden == Some(true)) != (next.aria().hidden == Some(true))
			|| (previous.hidden == Some(true)) != (next.hidden == Some(true))
	}

	/// Replaces everything about this element but its children.
	pub(crate) fn replace_own(&mut self, mut next: Self) {
		next.children = std::mem::take(&mut self.children);
		*self = next;
	}

	/// Folds relationships, radio set positions, and live-region text into
	/// concrete fields, since native accessibility APIs have no id references.
	fn resolve_accessibility(&mut self) {
		let mut names = BTreeMap::new();
		self.collect_names(&mut names);
		let mut set_sizes = BTreeMap::new();
		self.count_radios(&mut set_sizes);
		let mut positions = BTreeMap::new();
		self.fill_accessibility(&names, &set_sizes, &mut positions);
	}

	/// The accessible text of each element with an id.
	fn collect_names(&self, names: &mut BTreeMap<String, String>) {
		if let Some(id) = &self.props.id {
			names.insert(id.clone(), self.text_content());
		}

		for child in &self.children {
			child.collect_names(names);
		}
	}

	/// The number of options in each radio group.
	fn count_radios(&self, sizes: &mut BTreeMap<String, u32>) {
		if self.kind == NativeKind::Radio
			&& let Some(group) = &self.props.radio_group
		{
			*sizes.entry(group.name().to_owned()).or_insert(0) += 1;
		}

		for child in &self.children {
			child.count_radios(sizes);
		}
	}

	fn fill_accessibility(
		&mut self,
		names: &BTreeMap<String, String>,
		set_sizes: &BTreeMap<String, u32>,
		positions: &mut BTreeMap<String, u32>,
	) {
		let referenced = |ids: &[String]| {
			let joined = ids
				.iter()
				.filter_map(|id| names.get(id))
				.filter(|name| !name.is_empty())
				.cloned()
				.collect::<Vec<_>>()
				.join(" ");
			(!joined.is_empty()).then_some(joined)
		};

		if let Some(label) = referenced(&self.props.aria().labelled_by) {
			self.props.accessibility_label = Some(label);
		}

		if let Some(description) = referenced(&self.props.aria().described_by) {
			self.props.aria_mut().description = Some(description);
		}

		if self.kind == NativeKind::Radio
			&& let Some(group) = self.props.radio_group.as_ref().map(RadioGroup::name)
		{
			let position = positions.entry(group.to_owned()).or_insert(0);
			*position += 1;
			let set_size = set_sizes.get(group).copied();
			let aria = self.props.aria_mut();
			aria.pos_in_set = aria.pos_in_set.or(Some(*position));
			aria.set_size = aria.set_size.or(set_size);
		}

		if self.props.aria().effective_live(self.kind) != Live::Off
			&& !self.props.aria().is_hidden()
			&& self.props.hidden != Some(true)
		{
			let content = self.text_content();
			self.props.aria_mut().live_text = Some(content);
		}

		for child in &mut self.children {
			child.fill_accessibility(names, set_sizes, positions);
		}
	}

	/// The text assistive technology reads for this subtree: the accessible
	/// label when set, else the visible text of every descendant that is not
	/// hidden, in document order.
	#[must_use]
	pub fn text_content(&self) -> String {
		if let Some(label) = self
			.props
			.accessibility_label
			.as_deref()
			.filter(|label| !label.trim().is_empty())
		{
			return label.trim().to_owned();
		}

		let mut parts = Vec::new();

		if let Some(text) = self
			.props
			.text
			.as_deref()
			.filter(|text| !text.trim().is_empty())
		{
			parts.push(text.trim().to_owned());
		}

		for child in &self.children {
			if child.props.hidden == Some(true) || child.props.aria().hidden == Some(true) {
				continue;
			}

			let content = child.text_content();

			if !content.is_empty() {
				parts.push(content);
			}
		}

		parts.join(" ")
	}

	/// Replaces the style.
	#[must_use]
	pub fn style(mut self, style: Style) -> Self {
		self.style = style;
		self
	}

	/// Merges into the current style.
	#[must_use]
	pub fn with_style(mut self, edit: impl FnOnce(&mut Style)) -> Self {
		edit(&mut self.style);
		self
	}

	/// Replaces the props.
	#[must_use]
	pub fn props(mut self, props: NativeProps) -> Self {
		self.props = props;
		self
	}

	/// Edits the props in place.
	#[must_use]
	pub fn with_props(mut self, edit: impl FnOnce(&mut NativeProps)) -> Self {
		edit(&mut self.props);
		self
	}

	/// Makes a [`Pressable`](crate::components::Pressable) a link to `href`.
	///
	/// The web renders it as an `<a href>`, so the browser can open it in a
	/// new tab and crawlers can follow it, while a plain click still runs the
	/// `on_press` handler instead of leaving the page. Native platforms keep
	/// it a pressable. With the `links` feature, pressing one that has no
	/// `on_press` handler opens an absolute link in the person's browser,
	/// mail or phone app, through `leptoskit_plugin_linking`.
	///
	/// [`Self::target`] opens the link in a new tab, and [`Self::download`]
	/// saves it as a file.
	///
	/// Accepts relative URLs, fragments, and HTTP(S), mailto, tel, or sms URLs.
	/// Empty targets, whitespace, controls, backslashes, and other schemes
	/// are ignored: the web renders an ordinary button with its press handler.
	#[must_use]
	pub fn href(mut self, href: impl Into<String>) -> Self {
		self.props
			.extra
			.insert(NativeProps::HREF.to_owned(), PropValue::Str(href.into()));
		self
	}

	/// Says where a link made with [`Self::href`] opens.
	///
	/// With [`LinkTarget::NewTab`](crate::LinkTarget::NewTab) the web opens
	/// the link in a new tab that cannot reach back into this page
	/// (`target="_blank" rel="noopener noreferrer"`), and tells screen
	/// reader users that it opens in a new tab. A native app opens every
	/// link outside itself, so the target changes nothing there.
	///
	/// ```
	/// use leptoskit_native::LinkTarget;
	/// use leptoskit_native::components::Pressable;
	/// use leptoskit_native::components::Text;
	///
	/// let link = Pressable("Open the admin")
	/// 	.href("https://admin.example.com/")
	/// 	.target(LinkTarget::NewTab)
	/// 	.child(Text("Open the admin ↗"));
	/// assert_eq!(link.props.link_target(), LinkTarget::NewTab);
	/// ```
	#[must_use]
	pub fn target(mut self, target: crate::LinkTarget) -> Self {
		self.props.set_link_target(target);
		self
	}

	/// Makes a link made with [`Self::href`] save its target as a file
	/// called `file_name`, instead of opening it.
	///
	/// The web renders `<a href download>`. A browser only honours the
	/// name for a file from the page's own site; for another site it uses
	/// the name that server sends.
	///
	/// The name is one file name, not a path: an empty name, or one with a
	/// `/`, a `\\` or a control character, is ignored and the link opens as
	/// usual.
	///
	/// ```
	/// use leptoskit_native::components::Pressable;
	/// use leptoskit_native::components::Text;
	///
	/// let export = Pressable("Export")
	/// 	.href("/people/7/export")
	/// 	.download("person-7.json")
	/// 	.child(Text("Export"));
	/// assert_eq!(export.props.download(), Some("person-7.json"));
	/// ```
	#[must_use]
	pub fn download(mut self, file_name: impl Into<String>) -> Self {
		self.props.extra.insert(
			NativeProps::DOWNLOAD.to_owned(),
			PropValue::Str(file_name.into()),
		);
		self
	}

	/// Sets the accessible name (`aria-label`): what a screen reader announces
	/// first, overriding visible text. Doubles as the image description on
	/// the web.
	#[must_use]
	pub fn aria_label(mut self, label: impl Into<String>) -> Self {
		self.props.accessibility_label = Some(label.into());
		self
	}

	/// Sets the accessible name; the same as [`Self::aria_label`].
	#[must_use]
	pub fn accessibility_label(self, label: impl Into<String>) -> Self {
		self.aria_label(label)
	}

	/// Sets a stable identifier, unique within the tree, that ARIA
	/// relationships reference and the web renders as the DOM `id`.
	#[must_use]
	pub fn id(mut self, id: impl Into<String>) -> Self {
		self.props.id = Some(id.into());
		self
	}

	/// Sets an identifier for tests to find this element by when its text
	/// and accessible name are not stable enough, such as `Finder::test_id`
	/// in `leptoskit_testing`. The web renders it as `data-testid`; native
	/// backends ignore it.
	#[must_use]
	pub fn test_id(mut self, id: impl Into<String>) -> Self {
		self.props
			.extra
			.insert(NativeProps::TEST_ID.to_owned(), PropValue::Str(id.into()));
		self
	}

	/// Sets the ARIA role, overriding the widget kind's implicit role.
	///
	/// On the web, structural roles become semantic elements: a heading with
	/// a level renders as `<h1>` to `<h6>`, a list as `<ul>`, and landmarks as
	/// their HTML elements.
	#[must_use]
	pub fn role(mut self, role: Role) -> Self {
		self.props.aria_mut().role = Some(role);
		self
	}

	/// Names this element after other elements (`aria-labelledby`), by id.
	/// References win over `aria_label`.
	#[must_use]
	pub fn aria_labelled_by<I>(mut self, ids: I) -> Self
	where
		I: IntoIterator,
		I::Item: Into<String>,
	{
		self.props.aria_mut().labelled_by = ids.into_iter().map(Into::into).collect();
		self
	}

	/// Sets a description beyond the name (`aria-description`).
	#[must_use]
	pub fn aria_description(mut self, description: impl Into<String>) -> Self {
		self.props.aria_mut().description = Some(description.into());
		self
	}

	/// Describes this element with other elements (`aria-describedby`), by
	/// id; the usual home of help text and error messages.
	#[must_use]
	pub fn aria_described_by<I>(mut self, ids: I) -> Self
	where
		I: IntoIterator,
		I::Item: Into<String>,
	{
		self.props.aria_mut().described_by = ids.into_iter().map(Into::into).collect();
		self
	}

	/// Sets a human-readable role label (`aria-roledescription`).
	#[must_use]
	pub fn aria_role_description(mut self, description: impl Into<String>) -> Self {
		self.props.aria_mut().role_description = Some(description.into());
		self
	}

	/// Sets the heading, list, or tree depth (`aria-level`), starting at one.
	#[must_use]
	pub fn aria_level(mut self, level: u32) -> Self {
		self.props.aria_mut().level = Some(level);
		self
	}

	/// Sets the one-based position in a set (`aria-posinset`).
	#[must_use]
	pub fn aria_pos_in_set(mut self, position: u32) -> Self {
		self.props.aria_mut().pos_in_set = Some(position);
		self
	}

	/// Sets the number of items in the set (`aria-setsize`).
	#[must_use]
	pub fn aria_set_size(mut self, size: u32) -> Self {
		self.props.aria_mut().set_size = Some(size);
		self
	}

	/// Sets the widget's axis (`aria-orientation`).
	#[must_use]
	pub fn aria_orientation(mut self, orientation: Orientation) -> Self {
		self.props.aria_mut().orientation = Some(orientation);
		self
	}

	/// Hides the element and its descendants from assistive technology while
	/// keeping them visible (`aria-hidden`), for decorative content.
	#[must_use]
	pub fn aria_hidden(mut self, hidden: bool) -> Self {
		self.props.aria_mut().hidden = Some(hidden);
		self
	}

	/// Marks the element as being modified (`aria-busy`).
	#[must_use]
	pub fn aria_busy(mut self, busy: bool) -> Self {
		self.props.aria_mut().busy = Some(busy);
		self
	}

	/// Sets the check state (`aria-checked`) of a custom checkbox, option,
	/// menu item, or switch; `true`, `false`, or [`Tristate::Mixed`].
	#[must_use]
	pub fn aria_checked(mut self, checked: impl Into<Tristate>) -> Self {
		self.props.aria_mut().checked = Some(checked.into());
		self
	}

	/// Sets the pressed state (`aria-pressed`) of a toggle button.
	#[must_use]
	pub fn aria_pressed(mut self, pressed: impl Into<Tristate>) -> Self {
		self.props.aria_mut().pressed = Some(pressed.into());
		self
	}

	/// Sets whether the element is selected (`aria-selected`).
	#[must_use]
	pub fn aria_selected(mut self, selected: bool) -> Self {
		self.props.aria_mut().selected = Some(selected);
		self
	}

	/// Sets whether a control's popup or section is open (`aria-expanded`).
	#[must_use]
	pub fn aria_expanded(mut self, expanded: bool) -> Self {
		self.props.aria_mut().expanded = Some(expanded);
		self
	}

	/// Marks what the element currently represents (`aria-current`).
	#[must_use]
	pub fn aria_current(mut self, current: Current) -> Self {
		self.props.aria_mut().current = Some(current);
		self
	}

	/// Sets whether a dialog blocks the rest of the page (`aria-modal`).
	#[must_use]
	pub fn aria_modal(mut self, modal: bool) -> Self {
		self.props.aria_mut().modal = Some(modal);
		self
	}

	/// Sets whether a field must be filled in (`aria-required`).
	#[must_use]
	pub fn aria_required(mut self, required: bool) -> Self {
		self.props.aria_mut().required = Some(required);
		self
	}

	/// Sets why a field's value is invalid (`aria-invalid`); `true`, `false`,
	/// or a specific [`Invalid`] kind.
	#[must_use]
	pub fn aria_invalid(mut self, invalid: impl Into<Invalid>) -> Self {
		self.props.aria_mut().invalid = Some(invalid.into());
		self
	}

	/// Sets whether a field cannot be edited (`aria-readonly`).
	#[must_use]
	pub fn aria_read_only(mut self, read_only: bool) -> Self {
		self.props.aria_mut().read_only = Some(read_only);
		self
	}

	/// Sets whether a text field accepts multiple lines (`aria-multiline`).
	#[must_use]
	pub fn aria_multiline(mut self, multiline: bool) -> Self {
		self.props.aria_mut().multiline = Some(multiline);
		self
	}

	/// Sets what a control opens (`aria-haspopup`); `true`, `false`, or a
	/// specific [`HasPopup`] kind.
	#[must_use]
	pub fn aria_has_popup(mut self, popup: impl Into<HasPopup>) -> Self {
		self.props.aria_mut().has_popup = Some(popup.into());
		self
	}

	/// Sets a custom range widget's current value (`aria-valuenow`).
	#[must_use]
	pub fn aria_value_now(mut self, value: f64) -> Self {
		self.props.aria_mut().value_now = Some(value);
		self
	}

	/// Sets a custom range widget's minimum (`aria-valuemin`).
	#[must_use]
	pub fn aria_value_min(mut self, value: f64) -> Self {
		self.props.aria_mut().value_min = Some(value);
		self
	}

	/// Sets a custom range widget's maximum (`aria-valuemax`).
	#[must_use]
	pub fn aria_value_max(mut self, value: f64) -> Self {
		self.props.aria_mut().value_max = Some(value);
		self
	}

	/// Sets a human-readable range value (`aria-valuetext`), such as
	/// "50 percent" for a slider at half.
	#[must_use]
	pub fn aria_value_text(mut self, text: impl Into<String>) -> Self {
		self.props.aria_mut().value_text = Some(text.into());
		self
	}

	/// Makes the element a live region (`aria-live`) whose text changes are
	/// announced.
	#[must_use]
	pub fn aria_live(mut self, live: Live) -> Self {
		self.props.aria_mut().live = Some(live);
		self
	}

	/// Sets whether a live region announces as a whole (`aria-atomic`).
	#[must_use]
	pub fn aria_atomic(mut self, atomic: bool) -> Self {
		self.props.aria_mut().atomic = Some(atomic);
		self
	}

	/// Links this element to the elements it controls (`aria-controls`), by
	/// id.
	#[must_use]
	pub fn aria_controls<I>(mut self, ids: I) -> Self
	where
		I: IntoIterator,
		I::Item: Into<String>,
	{
		self.props.aria_mut().controls = ids.into_iter().map(Into::into).collect();
		self
	}

	/// Sets the position in sequential keyboard focus (`tabindex`); zero
	/// joins the natural order and a negative value leaves it.
	#[must_use]
	pub fn tab_index(mut self, index: i32) -> Self {
		self.props.aria_mut().tab_index = Some(index);
		self
	}

	/// Sets any other `aria-*` attribute, emitted verbatim on the web and
	/// ignored by native backends. Attributes with typed builders are
	/// rejected by [`Self::validate`].
	#[must_use]
	pub fn aria_attr(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
		self.props
			.aria_mut()
			.extra
			.insert(name.into(), value.into());
		self
	}

	/// Edits every accessibility property in place.
	#[must_use]
	pub fn with_aria(mut self, edit: impl FnOnce(&mut Aria)) -> Self {
		edit(self.props.aria_mut());
		self
	}

	/// Sets whether a control accepts interaction.
	#[must_use]
	pub fn enabled(mut self, enabled: bool) -> Self {
		self.props.enabled = Some(enabled);
		self
	}

	/// Sets a text input's controlled value.
	#[must_use]
	pub fn value(mut self, value: impl Into<String>) -> Self {
		self.props.value = Some(value.into());
		self
	}

	/// Replaces a `Text`'s content with styled runs that flow and wrap as one
	/// paragraph, such as syntax-highlighted code or a bold word in a
	/// sentence.
	///
	/// The joined runs become the text too, so layout, accessibility, and
	/// backends that draw one style per label see the same characters.
	#[must_use]
	pub fn spans<I>(mut self, spans: I) -> Self
	where
		I: IntoIterator,
		I::Item: Into<crate::TextSpan>,
	{
		let spans: Vec<crate::TextSpan> = spans.into_iter().map(Into::into).collect();
		self.props.text = Some(spans.iter().map(|span| span.text.as_str()).collect());
		self.props.spans = Some(spans);
		self
	}

	/// Asks for keyboard focus while `focused` is true: after each commit,
	/// the platform moves focus to this widget if it is not already there,
	/// the way a controlled value overrides what the user typed. Keep it true
	/// on one widget at a time, such as a chat prompt, and move it to a
	/// dialog's search field while the dialog is open.
	#[must_use]
	pub fn focused(mut self, focused: bool) -> Self {
		self.props.focused = Some(focused);
		self
	}

	/// Moves keyboard focus here once each time `request` is
	/// [requested](crate::FocusRequest::request), after the commit that
	/// follows. Between requests focus is left alone, so the person can move
	/// on; use [`focused`](Self::focused) for a widget that should hold focus
	/// for as long as a condition is true.
	///
	/// ```
	/// use leptoskit_native::FocusRequest;
	/// use leptoskit_native::components::TextInput;
	///
	/// # reactive_graph::owner::Owner::new().with(|| {
	/// let to_code = FocusRequest::new();
	/// let field = TextInput().aria_label("Code").focus_on(&to_code);
	/// assert_eq!(field.props.focus_request, None);
	///
	/// // Later, in a handler: the next build carries the request.
	/// to_code.request();
	/// let field = TextInput().aria_label("Code").focus_on(&to_code);
	/// assert!(field.props.focus_request.is_some());
	/// # });
	/// ```
	#[must_use]
	pub fn focus_on(mut self, request: impl std::borrow::Borrow<crate::FocusRequest>) -> Self {
		self.props.focus_request = request.borrow().serial();
		self
	}

	/// Scrolls this element into view once each time `reveal` is
	/// [requested](crate::Reveal::request), after the commit that follows:
	/// the nearest `ScrollView` around it scrolls, or the page on the web.
	/// `align` says where the element ends up. Between requests the person
	/// scrolls freely.
	///
	/// ```
	/// use leptoskit_native::Reveal;
	/// use leptoskit_native::ScrollAlign;
	/// use leptoskit_native::components::Text;
	///
	/// # reactive_graph::owner::Owner::new().with(|| {
	/// let to_notice = Reveal::new();
	/// let notice = Text("Saved.").reveal_on(&to_notice, ScrollAlign::Nearest);
	/// assert_eq!(notice.props.reveal, None);
	///
	/// // Later, in a handler: the next build carries the request.
	/// to_notice.request();
	/// let notice = Text("Saved.").reveal_on(&to_notice, ScrollAlign::Nearest);
	/// assert_eq!(notice.props.reveal.unwrap().align, ScrollAlign::Nearest);
	/// # });
	/// ```
	#[must_use]
	pub fn reveal_on(
		mut self,
		reveal: impl std::borrow::Borrow<crate::Reveal>,
		align: crate::ScrollAlign,
	) -> Self {
		self.props.reveal = reveal
			.borrow()
			.serial()
			.map(|serial| crate::RevealRequest { serial, align });
		self
	}

	/// Scrolls a scroll view's content to `offset` whenever the offset
	/// changes, like a feed that follows a live position or a "back to top"
	/// button. Between changes the person scrolls freely; the view's
	/// [`Scroll`](NativeEventSlot::Scroll) events report where it ends up,
	/// including after this offset is applied, so to scroll back to a place
	/// you set before, keep the reported offset in state and set the target
	/// again from it.
	///
	/// ```
	/// use leptoskit_native::ScrollOffset;
	/// use leptoskit_native::components::ScrollView;
	///
	/// // Jump to the top, or glide to the second screen.
	/// let top = ScrollView().scroll_offset(ScrollOffset::new(0.0, 0.0));
	/// let next = ScrollView().scroll_offset(ScrollOffset::new(0.0, 800.0).animated());
	/// ```
	///
	/// Only scroll views move; on any other element the offset does nothing.
	#[must_use]
	pub fn scroll_offset(mut self, offset: ScrollOffset) -> Self {
		self.props.scroll_offset = Some(offset);
		self
	}

	/// Sets the hint shown when a text input is empty.
	#[must_use]
	pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
		self.props.placeholder = Some(placeholder.into());
		self
	}

	/// What a browser may suggest for a `TextInput` or `PasswordInput`, as
	/// HTML's `autocomplete`: `email`, `current-password`, `new-password`,
	/// or `username webauthn` to offer saved passkeys in the field's
	/// suggestions. Native platforms give password managers the same hint
	/// for `username`, `email`, `current-password`, and `new-password`.
	#[must_use]
	pub fn autocomplete(mut self, hint: impl Into<String>) -> Self {
		self.props.autocomplete = Some(hint.into());
		self
	}

	/// Which keyboard a `TextInput` asks its platform for: `Text` (the
	/// default), `Email`, `Number`, `Decimal`, `Phone`, `Url`, or `Search`.
	///
	/// The web gets a matching `type` (`email`, `tel`, `url`, `search`) or,
	/// where HTML has no type, an `inputmode` (`numeric`, `decimal`). iOS
	/// and Android shape their keyboards; GTK hints at the field's purpose.
	/// macOS has no touch keyboard, so the prop does nothing there.
	///
	/// ```
	/// use leptoskit_native::Keyboard;
	/// use leptoskit_native::components::TextInput;
	///
	/// let email = TextInput().keyboard(Keyboard::Email);
	/// ```
	#[must_use]
	pub fn keyboard(mut self, keyboard: Keyboard) -> Self {
		self.props.keyboard = Some(keyboard);
		self
	}

	/// Whether a `TextInput` refuses edits but stays selectable and
	/// copyable. Also announced as `aria-readonly`; an explicit
	/// [`aria_read_only`](Self::aria_read_only) overrides it.
	///
	/// This is not [`enabled`](Self::enabled): a disabled field is grayed
	/// out of the tab order entirely, while a read-only one keeps focus,
	/// selection, and copy.
	///
	/// ```
	/// use leptoskit_native::components::TextInput;
	///
	/// let frozen = TextInput().read_only(true);
	/// ```
	#[must_use]
	pub fn read_only(mut self, read_only: bool) -> Self {
		self.props.read_only = Some(read_only);
		self
	}

	/// The longest value a `TextInput` accepts, in characters. Typing past
	/// it inserts nothing, the way HTML's `maxlength` behaves; the platform
	/// that cannot stop insertion truncates what it reports.
	///
	/// ```
	/// use leptoskit_native::components::TextInput;
	///
	/// let short = TextInput().max_length(9);
	/// ```
	#[must_use]
	pub fn max_length(mut self, max_length: usize) -> Self {
		self.props.max_length = Some(max_length);
		self
	}

	/// How typing is capitalized in a `TextInput`: `None`, `Words`,
	/// `Sentences`, or `Characters`. The keyboards that support it apply the
	/// rule to what the person types; text the application sets is never
	/// rewritten.
	///
	/// ```
	/// use leptoskit_native::Capitalize;
	/// use leptoskit_native::components::TextInput;
	///
	/// let shouty = TextInput().capitalize(Capitalize::Characters);
	/// ```
	#[must_use]
	pub fn capitalize(mut self, capitalize: Capitalize) -> Self {
		self.props.capitalize = Some(capitalize);
		self
	}

	/// Whether a `TextInput` corrects spelling while typing. `false` turns
	/// off suggestions and spelling marks; leaving it unset (or `true`)
	/// keeps the platform's default.
	///
	/// ```
	/// use leptoskit_native::components::TextInput;
	///
	/// let literal = TextInput().autocorrect(false);
	/// ```
	#[must_use]
	pub fn autocorrect(mut self, autocorrect: bool) -> Self {
		self.props.autocorrect = Some(autocorrect);
		self
	}

	/// What a `TextInput`'s return key is named: `Done` (the default),
	/// `Go`, `Next`, `Search`, or `Send`. The platforms that draw a return
	/// key name it; the web forwards it as `enterkeyhint`. A platform
	/// without one, such as macOS, ignores it.
	///
	/// ```
	/// use leptoskit_native::SubmitLabel;
	/// use leptoskit_native::components::TextInput;
	///
	/// let search = TextInput().submit_label(SubmitLabel::Go);
	/// ```
	#[must_use]
	pub fn submit_label(mut self, submit_label: SubmitLabel) -> Self {
		self.props.submit_label = Some(submit_label);
		self
	}

	/// Sets an image's source, understood by the platform backend.
	#[must_use]
	pub fn source(mut self, source: impl Into<String>) -> Self {
		self.props.source = Some(source.into());
		self
	}

	/// Sets whether a switch, checkbox, or radio option is on.
	#[must_use]
	pub fn checked(mut self, checked: bool) -> Self {
		self.props.checked = Some(checked);
		self
	}

	/// Sets the group a radio option belongs to.
	#[must_use]
	pub fn radio_group(mut self, group: RadioGroup) -> Self {
		self.props.radio_group = Some(group);
		self
	}

	/// Sets a slider's value and step grid.
	#[must_use]
	pub fn slider(mut self, state: SliderState) -> Self {
		self.props.slider = Some(state);
		self
	}

	/// Sets a progress indicator's progress.
	#[must_use]
	pub fn progress(mut self, progress: Progress) -> Self {
		self.props.progress = Some(progress);
		self
	}

	/// Sets whether an activity indicator is animating.
	#[must_use]
	pub fn running(mut self, running: bool) -> Self {
		self.props.running = Some(running);
		self
	}

	/// Appends a path to a `Vector`'s drawing, painted over earlier paths.
	#[must_use]
	pub fn path(mut self, path: VectorPath) -> Self {
		self.drawing_mut().push(path);
		self
	}

	/// Appends several paths to a `Vector`'s drawing, in painting order.
	#[must_use]
	pub fn paths(mut self, paths: impl IntoIterator<Item = VectorPath>) -> Self {
		let drawing = self.drawing_mut();

		for path in paths {
			drawing.push(path);
		}

		self
	}

	/// Replaces a `Vector`'s coordinate space.
	#[must_use]
	pub fn view_box(mut self, view_box: ViewBox) -> Self {
		self.drawing_mut().set_view_box(view_box);
		self
	}

	/// Sets how a `Vector`'s view box maps onto its frame.
	#[must_use]
	pub fn scaling(mut self, scaling: VectorScaling) -> Self {
		self.drawing_mut().set_scaling(scaling);
		self
	}

	/// Sets where a fitted or filled `Vector` drawing sits in its frame.
	#[must_use]
	pub fn align(mut self, alignment: VectorAlignment) -> Self {
		self.drawing_mut().set_alignment(alignment);
		self
	}

	/// Replaces a `Vector`'s whole drawing, such as one imported from an SVG
	/// document. Sharing one drawing between elements copies a pointer.
	#[must_use]
	pub fn drawing(mut self, drawing: Drawing) -> Self {
		*self.drawing_mut() = drawing;
		self
	}

	/// The drawing of a `Vector` element; any other kind has no drawing to
	/// edit.
	fn drawing_mut(&mut self) -> &mut Drawing {
		assert!(
			self.kind == NativeKind::Vector,
			"paths, view boxes, and scaling apply only to Vector elements, not {}",
			self.kind
		);
		self.props.drawing.get_or_insert_with(Drawing::default)
	}

	/// Sets the background color.
	#[must_use]
	pub fn background(self, color: impl IntoBackground) -> Self {
		color.into_background(self)
	}

	/// Sets the foreground (text) color.
	#[must_use]
	pub fn color(mut self, color: Color) -> Self {
		self.props.color = Some(color);
		self
	}

	/// Rounds the corners, clipping the content to the rounded shape.
	#[must_use]
	pub fn corner_radius(mut self, radius: f32) -> Self {
		self.props.corner_radius = Some(radius);
		self
	}

	/// Draws a border stroke of the given width and color.
	///
	/// The stroke is visual only; set the style's `border` as well when it
	/// should take layout space.
	#[must_use]
	pub fn border(mut self, width: f32, color: Color) -> Self {
		self.props.border_width = Some(width);
		self.props.border_color = Some(color);
		self
	}

	/// Sets the opacity of the widget and its content.
	#[must_use]
	pub fn opacity(mut self, opacity: f32) -> Self {
		self.props.opacity = Some(opacity);
		self
	}

	/// Moves, scales, or rotates the widget's drawing around its center,
	/// without moving anything else. Combine it with
	/// [`transition`](Self::transition) to animate the change.
	///
	/// ```
	/// use leptoskit_native::Transform;
	/// use leptoskit_native::components::View;
	///
	/// let tilted = View().transform(Transform::rotate(-3.0).scaled(1.05, 1.05));
	/// # let _ = tilted;
	/// ```
	#[must_use]
	pub fn transform(mut self, transform: crate::Transform) -> Self {
		self.props.transform = Some(transform);
		self
	}

	/// Animates changes to this widget's opacity, transform, and colors
	/// instead of jumping, as Tailwind's `transition` class does. The
	/// platform runs the animation; the app commits the new value once.
	///
	/// ```
	/// use leptoskit_native::animation::Spring;
	/// use leptoskit_native::animation::Transition;
	/// use leptoskit_native::components::View;
	///
	/// let open = true;
	/// let panel = View()
	/// 	.opacity(if open { 1.0 } else { 0.0 })
	/// 	.transition(Transition::spring(Spring::SNAPPY));
	/// # let _ = panel;
	/// ```
	#[must_use]
	pub fn transition(mut self, transition: crate::animation::Transition) -> Self {
		self.props.animation_mut().transition = Some(transition);
		self
	}

	/// Plays a keyframe sequence while the widget is shown, such as
	/// [`Keyframes::spin`](crate::animation::Keyframes::spin).
	#[must_use]
	pub fn keyframes(mut self, keyframes: crate::animation::Keyframes) -> Self {
		self.props.animation_mut().keyframes = Some(keyframes);
		self
	}

	/// Animates the widget in when it is added, from the look `presence`
	/// describes to its own.
	#[must_use]
	pub fn entering(mut self, presence: crate::animation::Presence) -> Self {
		self.props.animation_mut().entering = Some(presence);
		self
	}

	/// Animates the widget out when it is removed, from its own look to the
	/// one `presence` describes. The platform keeps drawing it until the
	/// animation ends.
	#[must_use]
	pub fn exiting(mut self, presence: crate::animation::Presence) -> Self {
		self.props.animation_mut().exiting = Some(presence);
		self
	}

	/// Animates the widget to its new position and size when layout moves
	/// or resizes it, instead of jumping.
	#[must_use]
	pub fn layout_transition(mut self, curve: crate::animation::Curve) -> Self {
		self.props.animation_mut().layout = Some(curve);
		self
	}

	/// Moves the element as `motion` describes: into place when it appears,
	/// or with scrolling. See [`crate::motion`].
	///
	/// ```
	/// use leptoskit_native::components::Text;
	/// use leptoskit_native::motion::Motion;
	/// use leptoskit_native::motion::Pose;
	/// use leptoskit_native::motion::ViewRange;
	///
	/// // Slides up into place as it scrolls into view.
	/// let line = Text("Every screen.")
	/// 	.motion(Motion::enter(Pose::new().y(32.0).opacity(0.0)).in_view(ViewRange::entry()));
	/// ```
	///
	/// The web plays it with CSS, and shows the element at rest at once to
	/// people who ask for reduced motion. Native platforms show the element
	/// at rest.
	#[must_use]
	pub fn motion(mut self, motion: crate::motion::Motion) -> Self {
		self.motion_props().motion = Some(motion);
		self
	}

	/// Names this element's passage through the viewport, so descendants
	/// can follow it with [`Motion::follow`](crate::motion::Motion::follow).
	///
	/// A tall container with a sticky child makes a stage: the child stays
	/// in view while the container scrolls past, and everything following
	/// the container's timeline moves with the scroll. Names are made of
	/// ASCII letters, digits, `-`, and `_`; other characters become `-`.
	#[must_use]
	pub fn timeline(mut self, name: impl Into<String>) -> Self {
		self.motion_props().timeline = Some(name.into());
		self
	}

	/// Starts each child's timed motion `step` later than the one before,
	/// so a list arrives one item after another. Motions that scrolling
	/// drives keep their own ranges.
	#[must_use]
	pub fn stagger(mut self, step: std::time::Duration) -> Self {
		self.motion_props().stagger = Some(step);
		self
	}

	fn motion_props(&mut self) -> &mut crate::motion::MotionProps {
		self.props.motion.get_or_insert_with(Box::default)
	}

	/// Sets the font size and weight.
	#[inline]
	#[must_use]
	pub fn font(mut self, size: f32, weight: FontWeight) -> Self {
		self.props.font_size = Some(size);
		self.props.font_weight = Some(weight);
		self
	}

	/// Sets the height of each line of text, which descendants inherit.
	#[inline]
	#[must_use]
	pub fn line_height(mut self, line_height: crate::LineHeight) -> Self {
		self.props.line_height = Some(line_height);
		self
	}

	/// Sets the horizontal text alignment.
	#[inline]
	#[must_use]
	pub fn text_align(mut self, align: TextAlign) -> Self {
		self.props.text_align = Some(align);
		self
	}

	/// Appends a child.
	///
	/// A `Text` or `Button` takes plain text children (unstyled `Text`
	/// elements with no children or handlers) as more of its content, so
	/// `Text("Count: ").child(Text("3"))` reads "Count: 3".
	#[inline]
	#[must_use]
	pub fn child(mut self, child: Element) -> Self {
		self.push_child(child);
		self
	}

	/// Appends every child, joining plain text into a `Text` or `Button` as
	/// [`Self::child`] does.
	#[inline]
	#[must_use]
	pub fn children(mut self, children: impl IntoIterator<Item = Element>) -> Self {
		let children = children.into_iter();
		// Growing the list moves every element already in it, and elements
		// are large; reserving once spares that, above all in the guest's
		// interpreter.
		self.children.reserve(children.size_hint().0);

		for child in children {
			self.push_child(child);
		}

		self
	}

	#[inline]
	fn push_child(&mut self, child: Element) {
		if matches!(self.kind, NativeKind::Text | NativeKind::Button) {
			self.push_child_text_or_button(child);
		} else {
			self.children.push(child);
		}
	}

	#[cold]
	#[inline(never)]
	fn push_child_text_or_button(&mut self, child: Element) {
		if child.is_plain_text() {
			let text = child.props.text.unwrap_or_default();
			self.props.text.get_or_insert_default().push_str(&text);
		} else {
			self.children.push(child);
		}
	}

	/// Whether this is text with nothing but its content: no style, classes,
	/// props, children, or handlers.
	fn is_plain_text(&self) -> bool {
		self.kind == NativeKind::Text
			&& self.children.is_empty()
			&& self.handlers.is_empty()
			&& self.gesture.is_none()
			&& self.classes == Tailwind::default()
			&& self.style == Style::default()
			&& has_no_props_other_than_text(&self.props)
	}

	/// Registers a handler for one event slot.
	#[must_use]
	pub fn on(
		mut self,
		slot: NativeEventSlot,
		handler: impl Fn(&NativeEvent) -> crate::props::EventOutcome + Send + Sync + 'static,
	) -> Self {
		self.handlers.push(HandlerEntry {
			event: slot,
			handler: Arc::new(handler),
		});
		self
	}

	/// Registers a press handler, the most common case.
	#[must_use]
	pub fn on_press(self, handler: impl Fn() + Send + Sync + 'static) -> Self {
		self.on(NativeEventSlot::Press, move |_| {
			handler();
			crate::props::EventOutcome::Handled
		})
	}

	/// Handles a request to close a `Modal` or `Popover`: Escape, a press
	/// outside it, Android's Back, or a swipe down on an iOS sheet.
	///
	/// The overlay does not close itself. Set the state it is built from,
	/// and the next build closes it; leave the state alone to refuse, as
	/// while a form inside is saving. An overlay with no handler cannot be
	/// dismissed this way, only by something inside it.
	#[must_use]
	pub fn on_dismiss(self, handler: impl Fn() + Send + Sync + 'static) -> Self {
		self.on(NativeEventSlot::Dismiss, move |event| {
			match event {
				NativeEvent::Dismiss => {
					handler();
					crate::EventOutcome::Handled
				}
				_ => crate::EventOutcome::Unhandled,
			}
		})
	}

	/// Sets whether a `Modal` or `Popover` is showing, as the `open`
	/// attribute of their `view!` tags does: `<Modal open=confirming>`.
	#[must_use]
	pub fn open(mut self, open: bool) -> Self {
		self.props.overlay.get_or_insert_default().open = open;
		self
	}

	/// Names the element a `Popover` hangs from, by `id`, as the `anchor`
	/// attribute of its `view!` tag does: `<Popover anchor="me" open=shown>`.
	#[must_use]
	pub fn anchor(mut self, id: impl Into<String>) -> Self {
		self.props.overlay.get_or_insert_default().anchor = Some(id.into());
		self
	}

	/// Names the element inside a `Modal` or `Popover` that takes focus
	/// when it opens, by `id`. Without one, the first focusable element
	/// inside does. Pick the safe choice in a confirmation: "Keep it", not
	/// "Delete".
	#[must_use]
	pub fn initial_focus(mut self, id: impl Into<String>) -> Self {
		self.props.overlay.get_or_insert_default().initial_focus = Some(id.into());
		self
	}

	/// Sets which side of its anchor a `Popover` opens on.
	#[must_use]
	pub fn placement(mut self, placement: crate::Placement) -> Self {
		self.props.overlay.get_or_insert_default().placement = placement;
		self
	}

	/// Handles key presses while focus is inside this element, before the
	/// focused widget acts on them. Return [`EventOutcome::Handled`] to keep
	/// the key from the widget: a root view that takes ctrl+p for a command
	/// palette while a text field is focused.
	///
	/// [`EventOutcome::Handled`]: crate::EventOutcome::Handled
	#[must_use]
	pub fn on_key(
		self,
		handler: impl Fn(&crate::KeyPress) -> crate::EventOutcome + Send + Sync + 'static,
	) -> Self {
		self.on(NativeEventSlot::Key, move |event| {
			match event {
				NativeEvent::Key(key) => handler(key),
				_ => crate::EventOutcome::Unhandled,
			}
		})
	}

	/// Handles an editable field's new value.
	#[must_use]
	pub fn on_change(self, handler: impl Fn(&str) + Send + Sync + 'static) -> Self {
		self.on(NativeEventSlot::Change, move |event| {
			if let NativeEvent::Change { value } = event {
				handler(value);
			}

			crate::props::EventOutcome::Handled
		})
	}

	/// Handles a boolean control's proposed checked state.
	#[must_use]
	pub fn on_toggle(self, handler: impl Fn(bool) + Send + Sync + 'static) -> Self {
		self.on(NativeEventSlot::Toggle, move |event| {
			if let NativeEvent::Toggle { checked } = event {
				handler(*checked);
			}

			crate::props::EventOutcome::Handled
		})
	}

	/// Handles a slider's proposed value on its configured step grid.
	#[must_use]
	pub fn on_value_change(self, handler: impl Fn(f64) + Send + Sync + 'static) -> Self {
		self.on(NativeEventSlot::ValueChange, move |event| {
			if let NativeEvent::ValueChange { value } = event {
				handler(*value);
			}

			crate::props::EventOutcome::Handled
		})
	}

	/// The handlers registered on this element.
	pub(crate) fn handlers(&self) -> &[HandlerEntry] {
		&self.handlers
	}

	/// Recognizers for pointers that hit this element.
	///
	/// A tap, pan, long press, pinch, rotation, or fling. Compose them with
	/// [`Gesture::simultaneous`](crate::Gesture::simultaneous),
	/// [`Gesture::exclusive`](crate::Gesture::exclusive), or
	/// [`Gesture::race`](crate::Gesture::race). Calling this more than once
	/// keeps every gesture: the later one runs at the same time as the one
	/// already attached.
	///
	/// Android and Linux deliver the pointer into the arena. A drag that wins
	/// claims the pointer so the control underneath does not also click. A tap
	/// does not, so a `Button` still presses. Other platforms keep their
	/// existing press handling until they forward pointer samples.
	#[must_use]
	pub fn gesture(mut self, gesture: crate::gesture::Gesture) -> Self {
		self.gesture = Some(Box::new(match self.gesture.take() {
			Some(existing) => existing.simultaneous(gesture),
			None => gesture,
		}));
		self
	}

	pub(crate) fn attached_gesture(&self) -> Option<&crate::gesture::Gesture> {
		self.gesture.as_deref()
	}
}

/// Returns `true` when every property in `props` other than `text` is unset,
/// without cloning `NativeProps` or constructing `NativeProps::default()`.
fn has_no_props_other_than_text(props: &NativeProps) -> bool {
	let NativeProps {
		rive,
		scene3d,
		video,
		camera,
		map,
		live_photo,
		canvist_editor,
		text: _,
		source,
		placeholder,
		autocomplete,
		keyboard,
		read_only,
		max_length,
		capitalize,
		autocorrect,
		submit_label,
		value,
		checked,
		radio_group,
		slider,
		progress,
		running,
		drawing,
		color,
		background,
		enabled,
		accessibility_label,
		aria,
		id,
		corner_radius,
		border_width,
		border_widths,
		border_color,
		opacity,
		transform,
		animation,
		hidden,
		clip,
		font_size,
		font_weight,
		line_height,
		italic,
		spans,
		text_align,
		direction,
		focused,
		focus_request,
		reveal,
		scroll_offset,
		overlay,
		motion,
		appearance,
		extra,
	} = props;

	rive.is_none()
		&& scene3d.is_none()
		&& video.is_none()
		&& camera.is_none()
		&& map.is_none()
		&& live_photo.is_none()
		&& canvist_editor.is_none()
		&& source.is_none()
		&& placeholder.is_none()
		&& autocomplete.is_none()
		&& keyboard.is_none()
		&& read_only.is_none()
		&& max_length.is_none()
		&& capitalize.is_none()
		&& autocorrect.is_none()
		&& submit_label.is_none()
		&& value.is_none()
		&& checked.is_none()
		&& radio_group.is_none()
		&& slider.is_none()
		&& progress.is_none()
		&& running.is_none()
		&& drawing.is_none()
		&& color.is_none()
		&& background.is_none()
		&& enabled.is_none()
		&& accessibility_label.is_none()
		&& aria.is_none()
		&& id.is_none()
		&& corner_radius.is_none()
		&& border_width.is_none()
		&& border_widths.is_none()
		&& border_color.is_none()
		&& opacity.is_none()
		&& transform.is_none()
		&& animation.is_none()
		&& hidden.is_none()
		&& clip.is_none()
		&& font_size.is_none()
		&& font_weight.is_none()
		&& line_height.is_none()
		&& italic.is_none()
		&& spans.is_none()
		&& text_align.is_none()
		&& direction.is_none()
		&& focused.is_none()
		&& focus_request.is_none()
		&& reveal.is_none()
		&& scroll_offset.is_none()
		&& overlay.is_none()
		&& motion.is_none()
		&& appearance.is_none()
		&& extra.is_empty()
}

#[cfg(all(test, feature = "devtools"))]
mod devtools_tests {
	use super::Element;

	#[test]
	fn constructors_record_the_call_site() {
		let line = line!() + 1;
		let view = Element::view();
		let source = view.source_location().expect("a source location");
		assert_eq!(source.file(), file!());
		assert_eq!(source.line(), line);
	}

	#[test]
	fn components_record_the_call_site() {
		use crate::components::Heading;
		use crate::components::ScrollView;
		use crate::components::Switch;
		use crate::components::View;

		let line = line!() + 1;
		let built = [Switch(true), View(), ScrollView(), Heading(1, "Clips")];

		for element in built {
			let source = element.source_location().expect("a source location");
			assert_eq!(source.file(), file!());
			assert_eq!(source.line(), line);
		}

		// `view!` starts each tag from its builder.
		let line = line!() + 1;
		let tag = View::builder();

		assert_eq!(
			tag.source_location().map(std::panic::Location::line),
			Some(line)
		);
	}

	#[test]
	fn resolved_elements_keep_their_source() {
		use super::StyleContext;

		let view = Element::view();
		let source = view.source_location();
		let resolved = view.resolved(&StyleContext::default());
		assert_eq!(resolved.source_location(), source);
	}
}

impl std::fmt::Debug for HandlerEntry {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("HandlerEntry")
			.field("event", &self.event)
			.finish_non_exhaustive()
	}
}
