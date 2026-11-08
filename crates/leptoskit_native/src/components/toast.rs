//! Ding-inspired toast notifications: a morphing pill with spring motion.
//!
//! Ding ([YusufCeng1z/ding](https://github.com/YusufCeng1z/ding)) shows one
//! toast at a time as a 44 px shape that springs open into a pill, morphs in
//! place when a new message arrives, and shrinks back before it leaves. This
//! module keeps that phase model and spring tuning, and draws the pill from
//! ordinary [`View`] / [`Text`] / [`Button`] / [`Vector`] builders so every
//! backend that already draws those kinds can show a toast.
//!
//! Motion uses [`leptoskit_animation`] springs that match Ding's enter
//! (320 / 26), morph (340 / 27), and stack (400 / 36) configs. Platforms that
//! run entering / layout transitions play the springs; others still show the
//! correct resting phase (compact circle or open pill) without interpolating.
//!
//! Drive phase and life timers with [`Toaster::poll`]: pass a monotonic
//! `Instant` from the host clock or from tests. Auto-dismiss pauses while
//! [`Toaster::pause`] is held (hover / focus), like Ding.

#![allow(non_snake_case)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use leptoskit_animation::Curve;
use leptoskit_animation::Easing;
use leptoskit_animation::Presence;
use leptoskit_animation::Spring;
use leptoskit_animation::Transition;
use leptoskit_yoga::Align as BoxAlign;
use leptoskit_yoga::Dimension;
use leptoskit_yoga::JustifyContent;
use leptoskit_yoga::PositionType;
use leptoskit_yoga::Rect;

use super::Button;
use super::Text;
use super::Vector;
use super::View;
use crate::Color;
use crate::Element;
use crate::FontWeight;
use crate::PathData;
use crate::Role;
use crate::Stroke;
use crate::VectorPath;
use crate::ViewBox;

/// Ding enter spring: quick arrival with a hint of overshoot.
pub const ENTER_SPRING: Spring = Spring::new(320.0, 26.0);
/// Ding morph spring: liquid pill open / close.
pub const MORPH_SPRING: Spring = Spring::new(340.0, 27.0);
/// Ding stack spring: tight shuffle with no wobble.
pub const STACK_SPRING: Spring = Spring::new(400.0, 36.0);

/// Compact shape size, matching Ding's `--tst-size`.
pub const TOAST_SIZE: f32 = 44.0;
/// Default auto-close when the toast is not sticky.
pub const DEFAULT_DURATION: Duration = Duration::from_millis(5_000);
/// Shape arrives, then the pill opens.
pub const OPEN_DELAY: Duration = Duration::from_millis(450);
/// Text erase + shrink back to the shape.
pub const CLOSING_DURATION: Duration = Duration::from_millis(340);
/// Shape leaves the screen.
pub const EXIT_DURATION: Duration = Duration::from_millis(220);

/// Toast status kinds, matching Ding's `ToastType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToastKind {
	/// Confirmation.
	Success,
	/// Failure; sticky until dismissed.
	Error,
	/// Caution.
	Warning,
	/// Neutral notice.
	Info,
	/// In-progress; sticky until updated or dismissed.
	Loading,
}

impl ToastKind {
	/// Accent colour for the status glyph.
	#[must_use]
	pub const fn accent(self) -> Color {
		match self {
			Self::Success => Color::rgba(0x12, 0xa3, 0x6b, 255),
			Self::Error => Color::rgba(0xe5, 0x48, 0x4d, 255),
			Self::Warning => Color::rgba(0xc7, 0x7d, 0x00, 255),
			Self::Info => Color::rgba(0x2f, 0x7d, 0xe1, 255),
			Self::Loading => Color::rgba(0x0f, 0x0f, 0x13, 255),
		}
	}

	/// Whether this kind stays until dismissed when no duration is set.
	#[must_use]
	pub const fn sticky_by_default(self) -> bool {
		matches!(self, Self::Error | Self::Loading)
	}
}

/// Lifecycle of one toast, matching Ding's `Phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToastPhase {
	/// Compact shape arriving.
	Enter,
	/// Pill expanded with copy.
	Open,
	/// Copy erased; pill shrinking.
	Closing,
	/// Shape leaving.
	Exit,
}

impl ToastPhase {
	/// Whether the toast still occupies the viewport.
	#[must_use]
	pub const fn is_live(self) -> bool {
		matches!(self, Self::Enter | Self::Open)
	}

	/// Whether the pill shows title and actions.
	#[must_use]
	pub const fn shows_body(self) -> bool {
		matches!(self, Self::Open)
	}
}

/// Where the toast stack anchors, matching Ding's six positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ToastPosition {
	/// Top edge, centred.
	#[default]
	TopCenter,
	/// Top-left corner.
	TopLeft,
	/// Top-right corner.
	TopRight,
	/// Bottom edge, centred.
	BottomCenter,
	/// Bottom-left corner.
	BottomLeft,
	/// Bottom-right corner.
	BottomRight,
}

impl ToastPosition {
	/// `-1` for top stacks (enter from above), `1` for bottom.
	#[must_use]
	pub const fn direction(self) -> f32 {
		match self {
			Self::TopCenter | Self::TopLeft | Self::TopRight => -1.0,
			Self::BottomCenter | Self::BottomLeft | Self::BottomRight => 1.0,
		}
	}

	/// Whether the stack is centred on the horizontal axis.
	#[must_use]
	pub const fn is_centered(self) -> bool {
		matches!(self, Self::TopCenter | Self::BottomCenter)
	}

	/// Whether the stack sits on the top edge.
	#[must_use]
	pub const fn is_top(self) -> bool {
		matches!(self, Self::TopCenter | Self::TopLeft | Self::TopRight)
	}
}

/// Optional button on a toast. Returning a [`ToastPatch`] morphs in place.
#[derive(Clone)]
pub struct ToastAction {
	/// Visible label.
	pub label: String,
	/// Runs on press. `Some(patch)` morphs; `None` dismisses.
	pub on_press: Arc<dyn Fn() -> Option<ToastPatch> + Send + Sync>,
}

impl std::fmt::Debug for ToastAction {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ToastAction")
			.field("label", &self.label)
			.finish_non_exhaustive()
	}
}

impl PartialEq for ToastAction {
	fn eq(&self, other: &Self) -> bool {
		self.label == other.label
	}
}

impl ToastAction {
	/// Builds an action that dismisses unless `on_press` returns a patch.
	pub fn new(
		label: impl Into<String>,
		on_press: impl Fn() -> Option<ToastPatch> + Send + Sync + 'static,
	) -> Self {
		Self {
			label: label.into(),
			on_press: Arc::new(on_press),
		}
	}
}

/// Fields an app may set when showing a toast.
#[derive(Debug, Clone, Default)]
pub struct ToastOptions {
	/// Caller-chosen id; reused so updates stay with one toast.
	pub id: Option<String>,
	/// Second line under the title.
	pub description: Option<String>,
	/// Primary action button.
	pub action: Option<ToastAction>,
	/// Quieter second button.
	pub secondary: Option<ToastAction>,
	/// Small trailing text beside the title.
	pub meta: Option<String>,
	/// Auto-close duration. `Some(0)` keeps it until dismissed.
	pub duration: Option<Duration>,
	/// Progress bar fraction in `0.0..=1.0`.
	pub progress: Option<f32>,
}

impl ToastOptions {
	/// Empty options.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Sets a stable id for later [`Toaster::update`] calls.
	#[must_use]
	pub fn id(mut self, id: impl Into<String>) -> Self {
		self.id = Some(id.into());
		self
	}

	/// Sets the description line.
	#[must_use]
	pub fn description(mut self, description: impl Into<String>) -> Self {
		self.description = Some(description.into());
		self
	}

	/// Sets the primary action.
	#[must_use]
	pub fn action(mut self, action: ToastAction) -> Self {
		self.action = Some(action);
		self
	}

	/// Sets the secondary action.
	#[must_use]
	pub fn secondary(mut self, action: ToastAction) -> Self {
		self.secondary = Some(action);
		self
	}

	/// Sets trailing meta text.
	#[must_use]
	pub fn meta(mut self, meta: impl Into<String>) -> Self {
		self.meta = Some(meta.into());
		self
	}

	/// Sets how long the toast stays open. Zero means sticky.
	#[must_use]
	pub fn duration(mut self, duration: Duration) -> Self {
		self.duration = Some(duration);
		self
	}

	/// Sets a progress bar, clamped to `0.0..=1.0`.
	#[must_use]
	pub fn progress(mut self, progress: f32) -> Self {
		self.progress = Some(progress.clamp(0.0, 1.0));
		self
	}
}

/// Partial update applied by [`Toaster::update`] or an action callback.
#[derive(Debug, Clone, Default)]
pub struct ToastPatch {
	/// New title.
	pub title: Option<String>,
	/// New description; empty string clears it.
	pub description: Option<String>,
	/// New kind.
	pub kind: Option<ToastKind>,
	/// New duration.
	pub duration: Option<Duration>,
	/// New progress; `Some(None)` clears the bar.
	pub progress: Option<Option<f32>>,
	/// New action; `Some(None)` removes it.
	pub action: Option<Option<ToastAction>>,
	/// New secondary action; `Some(None)` removes it.
	pub secondary: Option<Option<ToastAction>>,
	/// New meta; `Some(None)` removes it.
	pub meta: Option<Option<String>>,
}

impl ToastPatch {
	/// Empty patch.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Sets the title.
	#[must_use]
	pub fn title(mut self, title: impl Into<String>) -> Self {
		self.title = Some(title.into());
		self
	}

	/// Sets or clears the description (`""` clears).
	#[must_use]
	pub fn description(mut self, description: impl Into<String>) -> Self {
		self.description = Some(description.into());
		self
	}

	/// Sets the kind.
	#[must_use]
	pub fn kind(mut self, kind: ToastKind) -> Self {
		self.kind = Some(kind);
		self
	}

	/// Sets the remaining life.
	#[must_use]
	pub fn duration(mut self, duration: Duration) -> Self {
		self.duration = Some(duration);
		self
	}

	/// Sets progress, or `None` to remove the bar.
	#[must_use]
	pub fn progress(mut self, progress: Option<f32>) -> Self {
		self.progress = Some(progress.map(|value| value.clamp(0.0, 1.0)));
		self
	}

	/// Sets or clears the primary action.
	#[must_use]
	pub fn action(mut self, action: Option<ToastAction>) -> Self {
		self.action = Some(action);
		self
	}

	/// Sets or clears the secondary action.
	#[must_use]
	pub fn secondary(mut self, action: Option<ToastAction>) -> Self {
		self.secondary = Some(action);
		self
	}

	/// Sets or clears meta text.
	#[must_use]
	pub fn meta(mut self, meta: Option<String>) -> Self {
		self.meta = Some(meta);
		self
	}
}

/// One toast in the stack.
#[derive(Debug, Clone, PartialEq)]
pub struct ToastItem {
	/// Stable id (caller id or generated).
	pub id: String,
	/// Status kind.
	pub kind: ToastKind,
	/// Primary text.
	pub title: String,
	/// Optional second line.
	pub description: Option<String>,
	/// Current phase.
	pub phase: ToastPhase,
	/// Remaining auto-close life; zero is sticky.
	pub duration: Duration,
	/// Primary action.
	pub action: Option<ToastAction>,
	/// Secondary action.
	pub secondary: Option<ToastAction>,
	/// Trailing meta.
	pub meta: Option<String>,
	/// Progress `0.0..=1.0`.
	pub progress: Option<f32>,
	/// Repeat counter when the same toast is bumped.
	pub count: u32,
}

/// Public toaster snapshot for rendering and live regions.
#[derive(Debug, Clone, PartialEq)]
pub struct ToasterState {
	/// Toasts currently in the viewport, oldest first.
	pub toasts: Vec<ToastItem>,
	/// Stack anchor.
	pub position: ToastPosition,
	/// Polite live-region text.
	pub status: String,
	/// Assertive live-region text.
	pub alert: String,
}

impl Default for ToasterState {
	fn default() -> Self {
		Self {
			toasts: Vec::new(),
			position: ToastPosition::TopCenter,
			status: String::new(),
			alert: String::new(),
		}
	}
}

#[derive(Debug)]
struct Entry {
	phase_at: Option<Instant>,
	life_started: Option<Instant>,
	remaining: Duration,
	base: Duration,
	paused: bool,
}

/// Ding-style toaster: one live toast that morphs, with timed phases.
#[derive(Debug)]
pub struct Toaster {
	state: ToasterState,
	entries: HashMap<String, Entry>,
	/// Caller id → live toast id (morph target).
	live: HashMap<String, String>,
	/// Toast id → newest caller id allowed to update it.
	owner: HashMap<String, String>,
	counter: u64,
	announce_flip: bool,
}

impl Default for Toaster {
	fn default() -> Self {
		Self::new()
	}
}

impl Toaster {
	/// An empty toaster at top-centre.
	#[must_use]
	pub fn new() -> Self {
		Self {
			state: ToasterState::default(),
			entries: HashMap::new(),
			live: HashMap::new(),
			owner: HashMap::new(),
			counter: 0,
			announce_flip: false,
		}
	}

	/// Current snapshot for rendering.
	#[must_use]
	pub fn state(&self) -> &ToasterState {
		&self.state
	}

	/// Sets the stack position.
	pub fn set_position(&mut self, position: ToastPosition) {
		self.state.position = position;
	}

	/// Shows a success toast.
	pub fn success(&mut self, title: impl Into<String>, opts: ToastOptions) -> String {
		self.show(ToastKind::Success, title, opts)
	}

	/// Shows an error toast (sticky by default).
	pub fn error(&mut self, title: impl Into<String>, opts: ToastOptions) -> String {
		self.show(ToastKind::Error, title, opts)
	}

	/// Shows a warning toast.
	pub fn warning(&mut self, title: impl Into<String>, opts: ToastOptions) -> String {
		self.show(ToastKind::Warning, title, opts)
	}

	/// Shows an info toast.
	pub fn info(&mut self, title: impl Into<String>, opts: ToastOptions) -> String {
		self.show(ToastKind::Info, title, opts)
	}

	/// Shows a loading toast (sticky by default).
	pub fn loading(&mut self, title: impl Into<String>, opts: ToastOptions) -> String {
		self.show(ToastKind::Loading, title, opts)
	}

	/// Shows a toast of `kind`.
	pub fn show(
		&mut self,
		kind: ToastKind,
		title: impl Into<String>,
		opts: ToastOptions,
	) -> String {
		let title = title.into();
		if opts.id.is_none()
			&& opts.action.is_none()
			&& opts.secondary.is_none()
			&& opts.meta.is_none()
			&& let Some(dup) = self.state.toasts.last()
			&& dup.kind == kind
			&& dup.title == title
			&& dup.description == opts.description
			&& dup.phase.is_live()
		{
			let id = dup.id.clone();
			self.bump(&id);
			return id;
		}

		let id = opts.id.clone().unwrap_or_else(|| {
			self.counter = self.counter.saturating_add(1);
			format!("toast-{}", self.counter)
		});

		if let Some(cur) = self
			.state
			.toasts
			.iter()
			.rev()
			.find(|toast| toast.phase.is_live())
			.map(|toast| toast.id.clone())
		{
			self.live.insert(id.clone(), cur.clone());
			self.owner.insert(cur.clone(), id.clone());
			if let Some(toast) = self.find_mut(&cur) {
				toast.count = 1;
			}
			self.apply(
				&cur,
				ToastPatch {
					title: Some(title),
					description: Some(opts.description.unwrap_or_default()),
					kind: Some(kind),
					duration: opts.duration,
					progress: Some(opts.progress),
					action: Some(opts.action),
					secondary: Some(opts.secondary),
					meta: Some(opts.meta),
				},
			);
			return id;
		}

		self.owner.insert(id.clone(), id.clone());
		let duration = opts.duration.unwrap_or_else(|| {
			auto_duration(kind, opts.action.is_some() || opts.secondary.is_some())
		});
		self.entries.insert(
			id.clone(),
			Entry {
				phase_at: None,
				life_started: None,
				remaining: duration,
				base: duration,
				paused: false,
			},
		);
		self.state.toasts.push(ToastItem {
			id: id.clone(),
			kind,
			title: title.clone(),
			description: opts.description.clone(),
			phase: ToastPhase::Enter,
			duration,
			action: opts.action,
			secondary: opts.secondary,
			meta: opts.meta,
			progress: opts.progress,
			count: 1,
		});
		self.announce(kind, &title, opts.description.as_deref());
		id
	}

	/// Updates the toast owned by `caller_id`, if that caller is still newest.
	pub fn update(&mut self, caller_id: &str, patch: ToastPatch) {
		let id = self
			.live
			.get(caller_id)
			.cloned()
			.unwrap_or_else(|| caller_id.to_owned());
		if self.owner.get(&id).map(String::as_str) == Some(caller_id) {
			self.apply(&id, patch);
		}
	}

	/// Runs a toast action. A returned patch morphs; otherwise the toast
	/// dismisses.
	pub fn act(&mut self, id: &str, secondary: bool) {
		let id = self.resolve(id);
		let action = self.find(&id).and_then(|toast| {
			if secondary {
				toast.secondary.clone()
			} else {
				toast.action.clone()
			}
		});
		let Some(action) = action else {
			return;
		};
		match (action.on_press)() {
			Some(patch) => {
				self.apply(
					&id,
					ToastPatch {
						action: Some(None),
						secondary: Some(None),
						..patch
					},
				)
			}
			None => self.dismiss(Some(&id), false),
		}
	}

	/// Dismisses one toast, or every toast when `id` is `None`.
	///
	/// `immediate` skips the closing shrink (after a fling).
	pub fn dismiss(&mut self, id: Option<&str>, immediate: bool) {
		let Some(id) = id else {
			let ids: Vec<_> = self
				.state
				.toasts
				.iter()
				.map(|toast| toast.id.clone())
				.collect();
			for id in ids {
				self.dismiss(Some(&id), immediate);
			}
			return;
		};
		let id = self.resolve(id);
		let Some(toast) = self.find(&id) else {
			return;
		};
		if toast.phase == ToastPhase::Exit || (toast.phase == ToastPhase::Closing && !immediate) {
			return;
		}
		let Some(entry) = self.entries.get_mut(&id) else {
			return;
		};
		entry.phase_at = None;
		entry.life_started = None;
		if immediate {
			if let Some(toast) = self.find_mut(&id) {
				toast.phase = ToastPhase::Exit;
			}
			return;
		}
		if let Some(toast) = self.find_mut(&id) {
			toast.phase = ToastPhase::Closing;
		}
	}

	/// Pauses auto-dismiss for one toast, or every toast when `id` is `None`.
	pub fn pause(&mut self, id: Option<&str>, now: Instant) {
		let Some(id) = id else {
			let ids: Vec<_> = self.entries.keys().cloned().collect();
			for id in ids {
				self.pause(Some(&id), now);
			}
			return;
		};
		let id = self.resolve(id);
		let Some(entry) = self.entries.get_mut(&id) else {
			return;
		};
		if entry.paused {
			return;
		}
		entry.paused = true;
		if let Some(started) = entry.life_started.take() {
			let elapsed = now.saturating_duration_since(started);
			entry.remaining = entry.remaining.saturating_sub(elapsed);
		}
	}

	/// Resumes auto-dismiss after [`Self::pause`].
	pub fn resume(&mut self, id: Option<&str>) {
		let Some(id) = id else {
			let ids: Vec<_> = self.entries.keys().cloned().collect();
			for id in ids {
				self.resume(Some(&id));
			}
			return;
		};
		let id = self.resolve(id);
		if let Some(entry) = self.entries.get_mut(&id) {
			entry.paused = false;
		}
	}

	/// Advances phase and life timers. Call from the host clock or tests.
	///
	/// `now` should be monotonic. Phase deadlines are absolute instants
	/// recorded on the first poll after a phase change.
	pub fn poll(&mut self, now: Instant) {
		let ids: Vec<_> = self
			.state
			.toasts
			.iter()
			.map(|toast| toast.id.clone())
			.collect();
		for id in ids {
			self.poll_one(&id, now);
		}
	}

	fn poll_one(&mut self, id: &str, now: Instant) {
		let Some(phase) = self.find(id).map(|toast| toast.phase) else {
			return;
		};
		let mut open_due = false;
		let mut close_due = false;
		let mut exit_due = false;
		let mut life_due = false;

		{
			let Some(entry) = self.entries.get_mut(id) else {
				return;
			};
			match phase {
				ToastPhase::Enter => {
					let due = *entry.phase_at.get_or_insert(now + OPEN_DELAY);
					if now >= due {
						entry.phase_at = None;
						open_due = true;
					}
				}
				ToastPhase::Open => {
					if !entry.paused && !entry.remaining.is_zero() {
						let started = *entry.life_started.get_or_insert(now);
						if now.saturating_duration_since(started) >= entry.remaining {
							entry.life_started = None;
							life_due = true;
						}
					}
				}
				ToastPhase::Closing => {
					let due = *entry.phase_at.get_or_insert(now + CLOSING_DURATION);
					if now >= due {
						entry.phase_at = None;
						close_due = true;
					}
				}
				ToastPhase::Exit => {
					let due = *entry.phase_at.get_or_insert(now + EXIT_DURATION);
					if now >= due {
						exit_due = true;
					}
				}
			}
		}

		if open_due && let Some(toast) = self.find_mut(id) {
			toast.phase = ToastPhase::Open;
		}
		if life_due {
			self.dismiss(Some(id), false);
		}
		if close_due && let Some(toast) = self.find_mut(id) {
			toast.phase = ToastPhase::Exit;
		}
		if exit_due {
			self.forget(id);
		}
	}

	fn bump(&mut self, id: &str) {
		if let Some(toast) = self.find_mut(id) {
			toast.count = toast.count.saturating_add(1);
		}
		if let Some(entry) = self.entries.get_mut(id) {
			entry.remaining = entry.base;
			entry.life_started = None;
		}
	}

	fn apply(&mut self, id: &str, patch: ToastPatch) {
		let title_touched = patch.title.is_some();
		let description_touched = patch.description.is_some();
		let kind_touched = patch.kind.is_some();
		let progress_touched = patch.progress.is_some();
		let action_touched = patch.action.is_some();
		let secondary_touched = patch.secondary.is_some();
		let duration_patch = patch.duration;
		let duration_touched =
			duration_patch.is_some() || kind_touched || action_touched || secondary_touched;

		let Some(toast) = self.find_mut(id) else {
			return;
		};
		let previous_kind = toast.kind;
		let previous_title = toast.title.clone();
		let previous_description = toast.description.clone();
		if let Some(title) = patch.title {
			toast.title = title;
		}
		if let Some(description) = patch.description {
			toast.description = if description.is_empty() {
				None
			} else {
				Some(description)
			};
		}
		if let Some(kind) = patch.kind {
			toast.kind = kind;
		}
		if let Some(progress) = patch.progress {
			toast.progress = progress;
		}
		if let Some(action) = patch.action {
			toast.action = action;
		}
		if let Some(secondary) = patch.secondary {
			toast.secondary = secondary;
		}
		if let Some(meta) = patch.meta {
			toast.meta = meta;
		}

		let kind = toast.kind;
		let title = toast.title.clone();
		let description = toast.description.clone();
		let has_action = toast.action.is_some() || toast.secondary.is_some();

		let mut next_duration = None;
		if let Some(entry) = self.entries.get_mut(id) {
			if let Some(duration) = duration_patch {
				entry.remaining = duration;
			} else if duration_touched {
				entry.remaining = auto_duration(kind, has_action);
			}
			if duration_touched {
				entry.base = entry.remaining;
				entry.life_started = None;
				next_duration = Some(entry.remaining);
			}
		}
		if let Some(duration) = next_duration
			&& let Some(toast) = self.find_mut(id)
		{
			toast.duration = duration;
		}

		let quiet = progress_touched && !kind_touched;
		if !quiet
			&& (title_touched
				|| description_touched
				|| kind_touched
				|| title != previous_title
				|| description != previous_description
				|| kind != previous_kind)
		{
			self.announce(kind, &title, description.as_deref());
		}
	}

	fn announce(&mut self, kind: ToastKind, title: &str, description: Option<&str>) {
		self.announce_flip = !self.announce_flip;
		let mut text = match description {
			Some(description) if !description.is_empty() => format!("{title}. {description}"),
			_ => title.to_owned(),
		};
		if self.announce_flip {
			text.push('\u{00a0}');
		}
		if kind == ToastKind::Error {
			self.state.alert = text;
		} else {
			self.state.status = text;
		}
	}

	fn forget(&mut self, id: &str) {
		self.entries.remove(id);
		self.owner.remove(id);
		self.live.retain(|_, live| live != id);
		self.state.toasts.retain(|toast| toast.id != id);
	}

	fn resolve(&self, id: &str) -> String {
		self.live.get(id).cloned().unwrap_or_else(|| id.to_owned())
	}

	fn find(&self, id: &str) -> Option<&ToastItem> {
		self.state.toasts.iter().find(|toast| toast.id == id)
	}

	fn find_mut(&mut self, id: &str) -> Option<&mut ToastItem> {
		self.state.toasts.iter_mut().find(|toast| toast.id == id)
	}
}

fn auto_duration(kind: ToastKind, has_action: bool) -> Duration {
	if kind.sticky_by_default() || has_action {
		Duration::ZERO
	} else {
		DEFAULT_DURATION
	}
}

/// Entering presence for a toast at `position`, matching Ding's pop spring.
#[must_use]
pub fn toast_entering(position: ToastPosition) -> Presence {
	let y0 = position.direction() * (TOAST_SIZE + 16.0 + 24.0);
	Presence::fade()
		.slide_by(0.0, y0)
		.scale(0.5)
		.spring(ENTER_SPRING)
}

/// Exiting presence: short ease-out leave, matching Ding's `tst-out`.
#[must_use]
pub fn toast_exiting(position: ToastPosition) -> Presence {
	Presence::fade()
		.slide_by(0.0, position.direction() * 12.0)
		.scale(0.92)
		.duration(150.0)
		.easing(Easing::EASE_OUT)
}

/// Status glyph paths (24×24), matching Ding's stroke icons.
#[must_use]
pub fn toast_icon_paths(kind: ToastKind) -> &'static [&'static str] {
	const RING: &str = "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18";
	match kind {
		ToastKind::Success => &[RING, "M7.9 12.4l3 3 5.3-6"],
		ToastKind::Error => &[RING, "M9.3 9.3l5.4 5.4", "M14.7 9.3l-5.4 5.4"],
		ToastKind::Warning => &[RING, "M12 7.6v5.2", "M12 16.3h.01"],
		ToastKind::Info => &[RING, "M12 11v5.2", "M12 7.8h.01"],
		ToastKind::Loading => &[RING],
	}
}

/// Draws the status icon as a [`Vector`].
#[track_caller]
pub fn ToastIcon(kind: ToastKind) -> Element {
	let accent = kind.accent();
	let stroke = Stroke::new(accent, 2.2).expect("positive stroke").round();
	let mut icon = Vector(ViewBox::new(0.0, 0.0, 24.0, 24.0).expect("view box"))
		.class("w-[22px] h-[22px]")
		.aria_hidden(true);
	for path in toast_icon_paths(kind) {
		let data = PathData::parse(path).expect("ding icon path");
		icon = icon.path(VectorPath::new(data).stroke(stroke.clone()));
	}
	if kind == ToastKind::Loading {
		icon = icon.keyframes(leptoskit_animation::Keyframes::spin());
	}
	icon
}

/// One toast pill for `item`.
///
/// Compact phases are a 44 px circle; [`ToastPhase::Open`] expands into a
/// pill with copy and optional actions. Springs match Ding's enter / morph
/// tuning.
#[allow(
	clippy::too_many_lines,
	clippy::needless_pass_by_value,
	reason = "pill layout is easier to follow in one builder; Arc callbacks are cloned into \
	          handlers"
)]
#[track_caller]
pub fn Toast(
	item: &ToastItem,
	position: ToastPosition,
	on_dismiss: Arc<dyn Fn() + Send + Sync>,
	on_action: Arc<dyn Fn(bool) + Send + Sync>,
) -> Element {
	let compact = !item.phase.shows_body();
	let open = item.phase == ToastPhase::Open;
	let exiting = item.phase == ToastPhase::Exit;
	let end_pad = if item.action.is_some() || item.secondary.is_some() {
		8.0
	} else {
		18.0
	};

	let mut pill = View()
		.role(Role::Status)
		.test_id(format!("toast-{}", item.id))
		.background(Color::rgba(255, 255, 255, 255))
		.color(Color::rgba(0x0f, 0x0f, 0x13, 255))
		.corner_radius(TOAST_SIZE / 2.0)
		.layout_transition(Curve::spring(MORPH_SPRING))
		.transition(Transition::spring(MORPH_SPRING))
		.entering(toast_entering(position))
		.with_style(move |style| {
			style.flex_direction = leptoskit_yoga::FlexDirection::Row;
			style.align_items = BoxAlign::Center;
			style.min_height = Dimension::Points(TOAST_SIZE);
			style.min_width = Dimension::Points(TOAST_SIZE);
			style.overflow = leptoskit_yoga::Overflow::Hidden;
			if compact {
				style.width = Dimension::Points(TOAST_SIZE);
				style.height = Dimension::Points(TOAST_SIZE);
				style.justify_content = JustifyContent::Center;
			} else {
				style.padding = Rect::all_edges(
					Dimension::Points(0.0),
					Dimension::Points(end_pad),
					Dimension::Points(0.0),
					Dimension::Points(0.0),
				);
			}
		});

	if exiting {
		pill = pill.exiting(toast_exiting(position));
	}

	let icon = View()
		.class("w-11 h-11 items-center justify-center")
		.with_style(|style| {
			style.width = Dimension::Points(TOAST_SIZE);
			style.height = Dimension::Points(TOAST_SIZE);
			style.align_items = BoxAlign::Center;
			style.justify_content = JustifyContent::Center;
			style.flex_grow = 0.0;
			style.flex_shrink = 0.0;
		})
		.child(ToastIcon(item.kind));
	pill = pill.child(icon);

	if open {
		let mut body = View()
			.with_style(|style| {
				style.flex_direction = leptoskit_yoga::FlexDirection::Row;
				style.align_items = BoxAlign::Center;
				style.flex_wrap = leptoskit_yoga::FlexWrap::Wrap;
				style.gap = Rect::all_edges(
					Dimension::Points(0.0),
					Dimension::Points(8.0),
					Dimension::Points(0.0),
					Dimension::Points(8.0),
				);
			})
			.entering(
				Presence::fade()
					.slide_by(-12.0, 0.0)
					.spring(ENTER_SPRING)
					.delay(40.0),
			);

		let mut title_row = View().with_style(|style| {
			style.flex_direction = leptoskit_yoga::FlexDirection::Row;
			style.align_items = BoxAlign::Center;
			style.gap = Rect::all_edges(
				Dimension::Points(0.0),
				Dimension::Points(6.0),
				Dimension::Points(0.0),
				Dimension::Points(6.0),
			);
		});
		title_row = title_row.child(Text(item.title.clone()).font(14.0, FontWeight::Medium));
		if item.count > 1 {
			title_row = title_row.child(
				Text(format!("×{}", item.count))
					.font(14.0, FontWeight::Regular)
					.opacity(0.7),
			);
		}
		if let Some(meta) = &item.meta {
			title_row = title_row.child(
				Text(meta.clone())
					.font(12.0, FontWeight::Regular)
					.opacity(0.7),
			);
		}

		let mut copy = View()
			.with_style(|style| {
				style.flex_direction = leptoskit_yoga::FlexDirection::Column;
				style.max_width = Dimension::Points(340.0);
				style.padding = Rect::all_edges(
					Dimension::Points(12.0),
					Dimension::Points(6.0),
					Dimension::Points(12.0),
					Dimension::Points(0.0),
				);
			})
			.child(title_row);
		if let Some(description) = &item.description {
			copy = copy.child(
				Text(description.clone())
					.font(13.0, FontWeight::Regular)
					.opacity(0.65)
					.entering(
						Presence::fade()
							.slide_by(0.0, 12.0)
							.spring(ENTER_SPRING)
							.delay(100.0),
					),
			);
		}
		body = body.child(copy);

		if let Some(secondary) = &item.secondary {
			let on_action = Arc::clone(&on_action);
			body = body.child(
				Button(secondary.label.clone())
					.class("h-[30px] px-2.5 text-[13px] font-semibold")
					.on_press(move || on_action(true)),
			);
		}
		if let Some(action) = &item.action {
			let on_action = Arc::clone(&on_action);
			body = body.child(
				Button(action.label.clone())
					.class("h-[30px] px-3.5 rounded-full text-[13px] font-semibold")
					.background(Color::rgba(0x0f, 0x0f, 0x13, 255))
					.color(Color::rgba(255, 255, 255, 255))
					.corner_radius(999.0)
					.on_press(move || on_action(false)),
			);
		}
		let sticky =
			item.duration.is_zero() && item.kind != ToastKind::Loading && item.secondary.is_none();
		if sticky {
			let on_dismiss = Arc::clone(&on_dismiss);
			body = body.child(
				Button("×")
					.accessibility_label("Dismiss notification")
					.class("w-[30px] h-[30px] rounded-full")
					.corner_radius(15.0)
					.on_press(move || on_dismiss()),
			);
		}

		pill = pill.child(body);
	}

	if let Some(progress) = item.progress.filter(|_| open) {
		let bar = View()
			.background(Color::rgba(0x0f, 0x0f, 0x13, 36))
			.with_style(|style| {
				style.position = PositionType::Absolute;
				style.inset = Rect::all_edges(
					Dimension::Auto,
					Dimension::Points(0.0),
					Dimension::Points(0.0),
					Dimension::Points(0.0),
				);
				style.height = Dimension::Points(3.0);
			})
			.child(
				View()
					.background(item.kind.accent())
					.with_style(move |style| {
						style.height = Dimension::Points(3.0);
						style.width = Dimension::Percent((progress * 100.0).clamp(0.0, 100.0));
					}),
			);
		pill = pill.child(bar);
	}

	pill
}

/// Toast viewport: live regions plus the stacked pills.
///
/// Place once near the root. Rebuild when [`Toaster::state`] changes after
/// `show` / `poll` / `dismiss`. Stack index `0` is the newest toast.
#[allow(
	clippy::needless_pass_by_value,
	reason = "Arc callbacks are cloned per toast slot"
)]
#[track_caller]
pub fn ToastViewport(
	state: &ToasterState,
	on_dismiss: Arc<dyn Fn(String) + Send + Sync>,
	on_action: Arc<dyn Fn(String, bool) + Send + Sync>,
) -> Element {
	let position = state.position;
	let mut viewport = View()
		.role(Role::Region)
		.accessibility_label("Notifications")
		.test_id("toast-viewport")
		.with_style(|style| {
			style.position = PositionType::Absolute;
			style.inset = Rect::all_edges(
				Dimension::Points(0.0),
				Dimension::Points(0.0),
				Dimension::Points(0.0),
				Dimension::Points(0.0),
			);
		});

	viewport = viewport.child(
		View()
			.role(Role::Status)
			.test_id("toast-live-status")
			.child(Text(state.status.clone())),
	);
	viewport = viewport.child(
		View()
			.role(Role::Alert)
			.test_id("toast-live-alert")
			.child(Text(state.alert.clone())),
	);

	let count = state.toasts.len();
	for (index, item) in state.toasts.iter().enumerate() {
		let stack_i = count - 1 - index;
		let offset = stack_i as f32 * 10.0;
		let id = item.id.clone();
		let dismiss = Arc::clone(&on_dismiss);
		let act = Arc::clone(&on_action);
		let toast_id = id.clone();
		let mut slot = Toast(
			item,
			position,
			Arc::new(move || dismiss(id.clone())),
			Arc::new(move |secondary| act(toast_id.clone(), secondary)),
		)
		.layout_transition(Curve::spring(STACK_SPRING))
		.transition(Transition::spring(STACK_SPRING));

		slot = slot.with_style(move |style| {
			style.position = PositionType::Absolute;
			let pad = Dimension::Points(16.0);
			let stack = Dimension::Points(16.0 + offset);
			style.inset = match position {
				ToastPosition::TopCenter => {
					Rect::all_edges(stack, Dimension::Auto, Dimension::Auto, Dimension::Auto)
				}
				ToastPosition::BottomCenter => {
					Rect::all_edges(Dimension::Auto, Dimension::Auto, stack, Dimension::Auto)
				}
				ToastPosition::TopLeft => {
					Rect::all_edges(stack, Dimension::Auto, Dimension::Auto, pad)
				}
				ToastPosition::TopRight => {
					Rect::all_edges(stack, pad, Dimension::Auto, Dimension::Auto)
				}
				ToastPosition::BottomLeft => {
					Rect::all_edges(Dimension::Auto, Dimension::Auto, stack, pad)
				}
				ToastPosition::BottomRight => {
					Rect::all_edges(Dimension::Auto, pad, stack, Dimension::Auto)
				}
			};
			if position.is_centered() {
				style.align_self = BoxAlign::Center;
				style.margin = Rect::all_edges(
					Dimension::Points(0.0),
					Dimension::Auto,
					Dimension::Points(0.0),
					Dimension::Auto,
				);
			}
		});
		viewport = viewport.child(slot);
	}

	viewport
}

/// Convenience: toast chrome over `child` in a stacking context.
#[allow(
	clippy::needless_pass_by_value,
	reason = "forwards Arc callbacks to ToastViewport"
)]
#[track_caller]
pub fn WithToasts(
	child: Element,
	state: &ToasterState,
	on_dismiss: Arc<dyn Fn(String) + Send + Sync>,
	on_action: Arc<dyn Fn(String, bool) + Send + Sync>,
) -> Element {
	View()
		.with_style(|style| {
			style.flex_grow = 1.0;
			style.flex_shrink = 1.0;
			style.min_width = Dimension::Points(0.0);
			style.min_height = Dimension::Points(0.0);
		})
		.child(child)
		.child(ToastViewport(state, on_dismiss, on_action))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn at(ms: u64) -> Instant {
		Instant::now() + Duration::from_millis(ms)
	}

	#[test]
	fn success_enters_then_opens_then_auto_dismisses() {
		let mut toaster = Toaster::new();
		let start = Instant::now();
		let id = toaster.success("Saved", ToastOptions::new());
		assert_eq!(toaster.state().toasts[0].phase, ToastPhase::Enter);
		assert_eq!(id, "toast-1");

		toaster.poll(start); // arm enter deadline
		toaster.poll(start + OPEN_DELAY);
		assert_eq!(toaster.state().toasts[0].phase, ToastPhase::Open);

		toaster.poll(start + OPEN_DELAY); // arm life timer
		toaster.poll(start + OPEN_DELAY + DEFAULT_DURATION);
		assert_eq!(toaster.state().toasts[0].phase, ToastPhase::Closing);

		toaster.poll(start + OPEN_DELAY + DEFAULT_DURATION); // arm closing
		toaster.poll(start + OPEN_DELAY + DEFAULT_DURATION + CLOSING_DURATION);
		assert_eq!(toaster.state().toasts[0].phase, ToastPhase::Exit);

		toaster.poll(start + OPEN_DELAY + DEFAULT_DURATION + CLOSING_DURATION); // arm exit
		toaster.poll(start + OPEN_DELAY + DEFAULT_DURATION + CLOSING_DURATION + EXIT_DURATION);
		assert_eq!(toaster.state().toasts.len(), 0);
	}

	#[test]
	fn errors_and_loading_are_sticky() {
		let mut toaster = Toaster::new();
		let start = Instant::now();
		toaster.error("Nope", ToastOptions::new());
		toaster.poll(start);
		toaster.poll(start + OPEN_DELAY);
		assert_eq!(toaster.state().toasts[0].phase, ToastPhase::Open);
		toaster.poll(start + OPEN_DELAY + Duration::from_secs(60));
		assert_eq!(toaster.state().toasts[0].phase, ToastPhase::Open);
		assert!(toaster.state().toasts[0].duration.is_zero());
	}

	#[test]
	fn identical_toasts_bump_the_counter() {
		let mut toaster = Toaster::new();
		let first = toaster.success("Saved", ToastOptions::new());
		let second = toaster.success("Saved", ToastOptions::new());
		assert_eq!(first, second);
		assert_eq!(toaster.state().toasts[0].count, 2);
	}

	#[test]
	fn new_toast_morphs_the_live_one() {
		let mut toaster = Toaster::new();
		let start = Instant::now();
		let loading = toaster.loading("Uploading…", ToastOptions::new().id("upload"));
		toaster.poll(start);
		toaster.poll(start + OPEN_DELAY);
		toaster.update(
			&loading,
			ToastPatch::new()
				.kind(ToastKind::Success)
				.title("Uploaded")
				.progress(None),
		);
		assert_eq!(toaster.state().toasts.len(), 1);
		assert_eq!(toaster.state().toasts[0].kind, ToastKind::Success);
		assert_eq!(toaster.state().toasts[0].title, "Uploaded");
	}

	#[test]
	fn stale_caller_cannot_hijack_a_newer_toast() {
		let mut toaster = Toaster::new();
		let start = Instant::now();
		let first = toaster.loading("One", ToastOptions::new().id("a"));
		toaster.poll(start);
		toaster.poll(start + OPEN_DELAY);
		let second = toaster.loading("Two", ToastOptions::new().id("b"));
		assert_eq!(second, "b");
		toaster.update(&first, ToastPatch::new().title("Stale"));
		assert_eq!(toaster.state().toasts[0].title, "Two");
		toaster.update(&second, ToastPatch::new().title("Fresh"));
		assert_eq!(toaster.state().toasts[0].title, "Fresh");
	}

	#[test]
	fn action_patch_morphs_instead_of_dismissing() {
		let mut toaster = Toaster::new();
		let start = Instant::now();
		toaster.success(
			"Deleted",
			ToastOptions::new().action(ToastAction::new("Undo", || {
				Some(ToastPatch::new().kind(ToastKind::Info).title("Restored"))
			})),
		);
		toaster.poll(start);
		toaster.poll(start + OPEN_DELAY);
		let id = toaster.state().toasts[0].id.clone();
		toaster.act(&id, false);
		assert_eq!(toaster.state().toasts[0].kind, ToastKind::Info);
		assert_eq!(toaster.state().toasts[0].title, "Restored");
		assert!(toaster.state().toasts[0].action.is_none());
	}

	#[test]
	fn pause_holds_the_life_timer() {
		let mut toaster = Toaster::new();
		let start = Instant::now();
		toaster.success(
			"Hi",
			ToastOptions::new().duration(Duration::from_millis(1_000)),
		);
		toaster.poll(start);
		let open_at = start + OPEN_DELAY;
		toaster.poll(open_at);
		assert_eq!(toaster.state().toasts[0].phase, ToastPhase::Open);
		toaster.poll(open_at); // arm life
		let paused_at = open_at + Duration::from_millis(200);
		toaster.pause(None, paused_at);
		toaster.poll(open_at + Duration::from_secs(10));
		assert_eq!(toaster.state().toasts[0].phase, ToastPhase::Open);
		toaster.resume(None);
		toaster.poll(open_at + Duration::from_secs(10)); // arm remaining 800ms
		toaster.poll(open_at + Duration::from_secs(10) + Duration::from_millis(800));
		assert_eq!(toaster.state().toasts[0].phase, ToastPhase::Closing);
	}

	#[test]
	fn spring_presets_match_ding() {
		assert!((ENTER_SPRING.stiffness - 320.0).abs() < f32::EPSILON);
		assert!((ENTER_SPRING.damping - 26.0).abs() < f32::EPSILON);
		assert!((MORPH_SPRING.stiffness - 340.0).abs() < f32::EPSILON);
		assert!((STACK_SPRING.stiffness - 400.0).abs() < f32::EPSILON);
	}

	#[test]
	fn entering_comes_from_offscreen_with_ding_scale() {
		let top = toast_entering(ToastPosition::TopCenter);
		assert_eq!(top.visual.opacity, Some(0.0));
		let transform = top.visual.transform.expect("transform");
		assert!((transform.scale_x - 0.5).abs() < f32::EPSILON);
		assert!((transform.translate_y.points + 84.0).abs() < f32::EPSILON);
		assert_eq!(top.curve, Curve::spring(ENTER_SPRING));
	}

	#[test]
	fn icon_paths_parse_for_every_kind() {
		for kind in [
			ToastKind::Success,
			ToastKind::Error,
			ToastKind::Warning,
			ToastKind::Info,
			ToastKind::Loading,
		] {
			for path in toast_icon_paths(kind) {
				PathData::parse(path).expect(path);
			}
			let _ = ToastIcon(kind);
		}
	}

	#[test]
	fn viewport_builds_a_valid_tree() {
		let mut toaster = Toaster::new();
		let start = Instant::now();
		toaster.success("Saved", ToastOptions::new());
		toaster.poll(start);
		toaster.poll(start + OPEN_DELAY);
		let tree = ToastViewport(toaster.state(), Arc::new(|_| {}), Arc::new(|_, _| {}));
		tree.validate().expect("valid toast tree");
	}

	#[test]
	fn open_toast_with_actions_and_progress_builds() {
		let mut toaster = Toaster::new();
		let start = Instant::now();
		toaster.success(
			"Deleted",
			ToastOptions::new()
				.description("gone")
				.meta("now")
				.progress(0.4)
				.action(ToastAction::new("Undo", || None))
				.secondary(ToastAction::new("Keep", || None)),
		);
		toaster.poll(start);
		toaster.poll(start + OPEN_DELAY);
		let _ = toaster.success(
			"Deleted",
			ToastOptions::new()
				.description("gone")
				.meta("now")
				.progress(0.4)
				.action(ToastAction::new("Undo", || None))
				.secondary(ToastAction::new("Keep", || None)),
		);
		let tree = ToastViewport(toaster.state(), Arc::new(|_| {}), Arc::new(|_, _| {}));
		tree.validate().expect("valid rich toast");
	}

	#[test]
	fn announce_errors_into_the_alert_region() {
		let mut toaster = Toaster::new();
		toaster.error("Boom", ToastOptions::new().description("card declined"));
		assert!(toaster.state().alert.contains("Boom"));
		assert!(toaster.state().alert.contains("card declined"));
		assert_eq!(toaster.state().status, "");
	}

	#[test]
	fn options_and_patch_builders_cover_fields() {
		let action = ToastAction::new("Go", || None);
		let opts = ToastOptions::new()
			.id("x")
			.description("d")
			.action(action.clone())
			.secondary(ToastAction::new("More", || None))
			.meta("now")
			.duration(Duration::from_millis(10))
			.progress(1.5);
		assert_eq!(opts.id.as_deref(), Some("x"));
		assert_eq!(opts.progress, Some(1.0));
		assert_eq!(action, action);

		let patch = ToastPatch::new()
			.title("t")
			.description("")
			.kind(ToastKind::Warning)
			.duration(Duration::ZERO)
			.progress(None)
			.action(None)
			.secondary(None)
			.meta(None);
		assert_eq!(patch.title.as_deref(), Some("t"));
		assert_eq!(patch.kind, Some(ToastKind::Warning));
	}

	#[test]
	fn position_helpers_and_phase_flags() {
		assert_eq!(ToastPosition::TopLeft.direction(), -1.0);
		assert_eq!(ToastPosition::BottomRight.direction(), 1.0);
		assert!(ToastPosition::TopCenter.is_centered());
		assert!(!ToastPosition::BottomLeft.is_top());
		assert!(ToastPhase::Enter.is_live());
		assert!(!ToastPhase::Closing.is_live());
		assert!(ToastPhase::Open.shows_body());
		assert!(ToastKind::Error.sticky_by_default());
		assert!(!ToastKind::Success.sticky_by_default());
		let _ = ToastKind::Warning.accent();
		let _ = ToastKind::Info.accent();
		let _ = ToastKind::Loading.accent();
		let _ = at(0);
	}

	#[test]
	fn dismiss_all_and_immediate_exit() {
		let mut toaster = Toaster::new();
		let start = Instant::now();
		toaster.info("A", ToastOptions::new());
		toaster.poll(start);
		toaster.poll(start + OPEN_DELAY);
		toaster.dismiss(None, true);
		assert_eq!(toaster.state().toasts[0].phase, ToastPhase::Exit);
	}

	#[test]
	fn with_toasts_wraps_the_child() {
		let state = ToasterState::default();
		let tree = WithToasts(Text("hi"), &state, Arc::new(|_| {}), Arc::new(|_, _| {}));
		tree.validate().expect("valid");
	}

	#[test]
	fn set_position_and_default_toaster() {
		let mut toaster = Toaster::default();
		toaster.set_position(ToastPosition::BottomRight);
		assert_eq!(toaster.state().position, ToastPosition::BottomRight);
		toaster.warning("Careful", ToastOptions::new());
		toaster.info("Heads up", ToastOptions::new().id("info"));
		assert_eq!(toaster.state().toasts.len(), 1);
		assert_eq!(toaster.state().toasts[0].title, "Heads up");
	}
}
