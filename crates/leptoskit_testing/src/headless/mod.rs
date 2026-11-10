//! Runs a compiled guest `.wasm` without a device or a window.
//!
//! [`HeadlessApp`] loads the same bundle a native app downloads, renders it
//! through the production renderer and Yoga layout against the recording
//! backend, and drives it with the semantic events a platform backend would
//! report. On top of it sit the tooling coding agents use to check their work:
//!
//! - [`Script`], a JSON list of presses, typing, scrolls, waits, and
//!   assertions, run step by step into a [`ScriptReport`];
//! - [`HeadlessApp::tree`] and [`HeadlessApp::tree_json`], the committed widget
//!   tree with kinds, key props, frames, and accessible names;
//! - [`HeadlessApp::snapshot`], a deterministic SVG or PNG of the laid-out
//!   tree.
//!
//! ```no_run
//! use leptoskit_testing::Finder;
//! use leptoskit_testing::headless::HeadlessApp;
//!
//! let wasm = std::fs::read("dist/app.wasm")?;
//! let mut app = HeadlessApp::start(&wasm, 390.0, 844.0)?;
//! app.tap(&Finder::text("Increment"))?;
//! app.find(&Finder::text("Count: 1"))?;
//! println!("{}", app.tree());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Text uses the backend's approximate fixed-cell metrics, so layouts are
//! the same on every machine, and snapshots draw native controls as simple
//! outlines. They show structure, not a platform's pixels.

use leptoskit_native::EventOutcome;
use leptoskit_native::Frame;
use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_wasm::FrameStats;
use leptoskit_wasm::WasmError;
use leptoskit_wasm::WasmHost;
use leptoskit_yoga::NodeId;

use crate::Finder;
use crate::TestError;
use crate::Widget;
use crate::inspect::Inspector;

mod dump;
mod script;
mod snapshot;

pub use dump::TreeFrame;
pub use dump::TreeNode;
pub use script::Script;
pub use script::ScriptReport;
pub use script::Selector;
pub use script::Step;
pub use script::StepReport;
pub use script::Viewport;
pub use snapshot::Snapshot;
pub use snapshot::SnapshotFormat;

/// The viewport scripts use when they do not name one: a typical phone.
pub const DEFAULT_VIEWPORT: (f32, f32) = (390.0, 844.0);

/// Plugin wakes a wait makes before it gives up, unless the step says.
pub const DEFAULT_MAX_WAKES: u32 = 100;

/// A guest that could not be loaded, rendered, or driven.
#[derive(Debug, thiserror::Error)]
pub enum HeadlessError {
	/// The guest failed to compile, trapped, or ran out of budget.
	#[error(transparent)]
	Wasm(#[from] WasmError),
	/// A query, interaction, or assertion failed.
	#[error(transparent)]
	Test(#[from] TestError),
	/// Writing a snapshot or reading a file failed.
	#[error(transparent)]
	Io(#[from] std::io::Error),
	/// A snapshot could not be rendered.
	#[error("{0}")]
	Snapshot(String),
	/// A viewport was empty or not finite.
	#[error("viewport dimensions must be finite and positive")]
	Viewport,
}

/// One guest running against the headless backend.
pub struct HeadlessApp {
	host: WasmHost<HeadlessBackend>,
	viewport: (f32, f32),
	dark: bool,
}

impl HeadlessApp {
	/// Loads a guest and runs its initial render at `width` by `height`.
	pub fn start(wasm: &[u8], width: f32, height: f32) -> Result<Self, HeadlessError> {
		check_viewport(width, height)?;
		let mut host = WasmHost::headless(wasm)?;
		host.start(width, height)?;
		Ok(Self {
			host,
			viewport: (width, height),
			dark: false,
		})
	}

	/// The underlying host, for plugin registration and lower-level calls.
	pub fn host(&self) -> &WasmHost<HeadlessBackend> {
		&self.host
	}

	/// Mutable access to the underlying host.
	pub fn host_mut(&mut self) -> &mut WasmHost<HeadlessBackend> {
		&mut self.host
	}

	/// The viewport, in layout units.
	pub fn viewport(&self) -> (f32, f32) {
		self.viewport
	}

	/// The number of commits the guest has made.
	pub fn commits(&self) -> u32 {
		self.host.commits()
	}

	/// What the most recent commit cost.
	pub fn last_frame(&self) -> Option<&FrameStats> {
		self.host.last_frame()
	}

	/// Lays out at a new viewport, which a guest that listens for it hears.
	pub fn resize(&mut self, width: f32, height: f32) -> Result<(), HeadlessError> {
		check_viewport(width, height)?;
		self.viewport = (width, height);
		self.host.set_viewport(width, height);
		self.check_render()
	}

	/// Switches the appearance `dark:` utilities resolve against.
	pub fn set_dark_mode(&mut self, dark: bool) -> Result<(), HeadlessError> {
		self.dark = dark;
		self.host.set_dark_mode(dark);
		self.check_render()
	}

	/// Exactly one visible widget matching `finder`.
	pub fn find(&self, finder: &Finder) -> Result<Widget, TestError> {
		let id = self.unique(finder)?;
		Ok(self.inspect().widget(id))
	}

	/// Every visible widget matching `finder`, in tree order.
	pub fn find_all(&self, finder: &Finder) -> Vec<Widget> {
		let inspect = self.inspect();
		inspect
			.matches(finder)
			.into_iter()
			.map(|id| inspect.widget(id))
			.collect()
	}

	/// Presses the centre of a visible, enabled widget, as a tap does.
	///
	/// A widget covering that centre is an error. Switches and checkboxes
	/// propose their inverse checked state and radios propose selection, as
	/// their native controls report a tap.
	pub fn tap(&mut self, finder: &Finder) -> Result<(), HeadlessError> {
		let id = self.action_target(finder)?;
		let inspect = self.inspect();
		let frame = inspect.absolute_frame(id);
		let hit = self
			.host
			.surface()
			.hit_test(frame.x + frame.width / 2.0, frame.y + frame.height / 2.0)
			.filter(|&hit| {
				inspect.visible(hit)
					&& (inspect.is_descendant(hit, id)
						|| (self.host.surface().kind(hit) == NativeKind::Pressable
							&& inspect.is_descendant(id, hit)))
			});
		let Some(hit) = hit else {
			return Err(self
				.error(format!("{finder:?} is obscured at its center"))
				.into());
		};

		if !inspect.enabled(hit) {
			return Err(self
				.error(format!("{finder:?} has a disabled hit target"))
				.into());
		}

		let surface = self.host.surface();
		let (target, event) = match surface.kind(id) {
			NativeKind::Switch | NativeKind::Checkbox => {
				let checked = !surface.props(id).checked.unwrap_or(false);
				(id, NativeEvent::Toggle { checked })
			}
			NativeKind::Radio => (id, NativeEvent::Toggle { checked: true }),
			_ => (hit, NativeEvent::Press),
		};
		self.dispatch(target, &event, finder)
	}

	/// Proposes a checked state for a switch, checkbox, or radio.
	pub fn toggle(&mut self, finder: &Finder, checked: bool) -> Result<(), HeadlessError> {
		let id = self.action_target(finder)?;

		if !matches!(
			self.host.surface().kind(id),
			NativeKind::Switch | NativeKind::Checkbox | NativeKind::Radio
		) {
			return Err(self
				.error(format!("{finder:?} is not a boolean control"))
				.into());
		}

		self.dispatch(id, &NativeEvent::Toggle { checked }, finder)
	}

	/// Proposes a slider value; the guest snaps and clamps it.
	pub fn set_slider_value(&mut self, finder: &Finder, value: f64) -> Result<(), HeadlessError> {
		let id = self.action_target(finder)?;

		if self.host.surface().kind(id) != NativeKind::Slider {
			return Err(self.error(format!("{finder:?} is not a slider")).into());
		}

		self.dispatch(id, &NativeEvent::ValueChange { value }, finder)
	}

	/// Replaces a text input's value, as typing into it does.
	pub fn enter_text(&mut self, finder: &Finder, value: &str) -> Result<(), HeadlessError> {
		let id = self.text_input(finder)?;
		self.dispatch(
			id,
			&NativeEvent::Change {
				value: value.to_owned(),
			},
			finder,
		)
	}

	/// Submits a text input with its current value, as the return key does.
	pub fn submit(&mut self, finder: &Finder) -> Result<(), HeadlessError> {
		let id = self.text_input(finder)?;
		let value = self
			.host
			.surface()
			.props(id)
			.value
			.clone()
			.unwrap_or_default();
		self.dispatch(id, &NativeEvent::Submit { value }, finder)
	}

	/// Reports a scroll offset for a scroll view, as scrolling it does.
	pub fn scroll(&mut self, finder: &Finder, x: f32, y: f32) -> Result<(), HeadlessError> {
		let id = self.action_target(finder)?;

		if self.host.surface().kind(id) != NativeKind::ScrollView
			|| !x.is_finite()
			|| !y.is_finite()
		{
			return Err(self
				.error("scroll requires a scroll view and finite offsets")
				.into());
		}

		self.dispatch(
			id,
			&NativeEvent::Scroll {
				offset_x: x,
				offset_y: y,
			},
			finder,
		)
	}

	/// Lets the guest's async work run until it has committed `commits` more
	/// times, waking it at most `max_wakes` times.
	pub fn wait_for_commits(&mut self, commits: u32, max_wakes: u32) -> Result<(), HeadlessError> {
		let target = self.host.commits().saturating_add(commits);

		for _ in 0..max_wakes {
			if self.host.commits() >= target {
				return Ok(());
			}

			self.wake()?;
		}

		if self.host.commits() >= target {
			return Ok(());
		}

		Err(self
			.error(format!(
				"the guest committed {} of {commits} times within {max_wakes} wakes",
				commits - (target - self.host.commits())
			))
			.into())
	}

	/// Lets the guest's async work run until exactly one visible widget
	/// matches, waking it at most `max_wakes` times.
	pub fn wait_for(&mut self, finder: &Finder, max_wakes: u32) -> Result<Widget, HeadlessError> {
		for wake in 0..=max_wakes {
			match self.inspect().matches(finder).len() {
				1 => return Ok(self.find(finder)?),
				0 if wake < max_wakes => self.wake()?,
				0 => break,
				count => {
					return Err(self
						.error(format!("expected one match for {finder:?}, found {count}"))
						.into());
				}
			}
		}

		Err(self
			.error(format!(
				"{finder:?} did not appear within {max_wakes} wakes"
			))
			.into())
	}

	/// The committed tree as indented text: one line per widget with its
	/// kind, accessible name, role, key props, and viewport frame.
	pub fn tree(&self) -> String {
		dump::text(&self.inspect())
	}

	/// The committed tree as data, for JSON output. `None` before the guest
	/// commits.
	pub fn tree_json(&self) -> Option<TreeNode> {
		dump::tree(&self.inspect())
	}

	/// What assistive technology sees, as an indented outline.
	pub fn semantics(&self) -> String {
		self.inspect().semantics()
	}

	/// Renders the laid-out tree. The same tree always gives the same bytes.
	pub fn snapshot(&self, format: SnapshotFormat) -> Result<Snapshot, HeadlessError> {
		snapshot::render(&self.inspect(), self.dark, format)
	}

	fn inspect(&self) -> Inspector<'_, HeadlessBackend> {
		Inspector {
			surface: self.host.surface(),
			viewport: self.viewport,
		}
	}

	/// Runs the guest's plugin callback, which drives its async tasks.
	fn wake(&mut self) -> Result<(), HeadlessError> {
		match self.host.wake_plugins() {
			Ok(()) => Ok(()),
			Err(WasmError::MissingExport(_)) => {
				Err(self.error("the guest has no async work to wait for").into())
			}
			Err(error) => Err(error.into()),
		}
	}

	fn dispatch(
		&mut self,
		id: NodeId,
		event: &NativeEvent,
		finder: &Finder,
	) -> Result<(), HeadlessError> {
		if self.host.dispatch(id, event)? == EventOutcome::Unhandled {
			let event = match event {
				NativeEvent::Change { .. } if self.is_password(id) => "a password change",
				_ => event_name(event),
			};
			return Err(self
				.error(format!("{finder:?} did not handle {event}"))
				.into());
		}

		self.check_render()
	}

	fn is_password(&self, id: NodeId) -> bool {
		self.host.surface().kind(id) == NativeKind::PasswordInput
	}

	fn check_render(&self) -> Result<(), HeadlessError> {
		match self.host.render_error() {
			Some(error) => Err(WasmError::Trap(error.to_owned()).into()),
			None => Ok(()),
		}
	}

	fn text_input(&self, finder: &Finder) -> Result<NodeId, TestError> {
		let id = self.action_target(finder)?;

		if !matches!(
			self.host.surface().kind(id),
			NativeKind::TextInput | NativeKind::PasswordInput
		) {
			return Err(self.error(format!("{finder:?} is not a text input")));
		}

		Ok(id)
	}

	fn action_target(&self, finder: &Finder) -> Result<NodeId, TestError> {
		let id = self.unique(finder)?;
		let inspect = self.inspect();

		if !inspect.enabled(id) {
			return Err(self.error(format!("{finder:?} is disabled")));
		}

		let Frame {
			x,
			y,
			width,
			height,
		} = inspect.absolute_frame(id);
		let (x, y) = (x + width / 2.0, y + height / 2.0);

		if x < 0.0 || y < 0.0 || x >= self.viewport.0 || y >= self.viewport.1 {
			return Err(self.error(format!("{finder:?} has its center outside the viewport")));
		}

		Ok(id)
	}

	fn unique(&self, finder: &Finder) -> Result<NodeId, TestError> {
		match self.inspect().matches(finder).as_slice() {
			[id] => Ok(*id),
			matches => {
				Err(self.error(format!(
					"expected one match for {finder:?}, found {}",
					matches.len()
				)))
			}
		}
	}

	fn error(&self, reason: impl Into<String>) -> TestError {
		TestError {
			reason: reason.into(),
			tree: self.tree(),
		}
	}
}

fn check_viewport(width: f32, height: f32) -> Result<(), HeadlessError> {
	if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
		return Err(HeadlessError::Viewport);
	}

	Ok(())
}

fn event_name(event: &NativeEvent) -> &'static str {
	match event {
		NativeEvent::Press => "a press",
		NativeEvent::Toggle { .. } => "a toggle",
		NativeEvent::ValueChange { .. } => "a value change",
		NativeEvent::Change { .. } => "a text change",
		NativeEvent::Submit { .. } => "a submit",
		NativeEvent::Scroll { .. } => "a scroll",
		_ => "the event",
	}
}
