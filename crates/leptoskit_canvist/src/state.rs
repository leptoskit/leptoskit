use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use canvist_headless::Engine;
use canvist_headless::Format as CanvistFormat;
use canvist_headless::Doc;
use canvist_headless::canvist_model::Point;
use canvist_headless::canvist_model::Selection;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
use canvist_headless::canvist_core::Editor;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
use canvist_headless::canvist_core::Env;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
use std::time::Duration;
use std::future::Future;

use canvist_extension_document::Document;
use canvist_extension_hard_break::HardBreak;
use canvist_extension_history::History;
use canvist_extension_paragraph::Paragraph;
use canvist_headless::canvist_core::BaseKeymap;

/// A `Copy` handle to a canvist editor. The engine and document live in a
/// process-wide registry keyed by this handle's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CanvistState(pub(crate) u64);

/// The document formats the editor can load and export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
	Text,
	Markdown,
	Json,
	Html,
}

impl Format {
	fn to_canvist(self) -> CanvistFormat {
		match self {
			Self::Text => CanvistFormat::Text,
			Self::Markdown => CanvistFormat::Markdown,
			Self::Json => CanvistFormat::Json,
			Self::Html => CanvistFormat::Html,
		}
	}
}

/// A record of what changed in a single edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
	pub doc_changed: bool,
	pub selection_changed: bool,
}

/// The set of extensions registered for an editor. Only paragraphs today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extensions {
	Paragraphs,
}

impl Extensions {
	pub fn paragraphs() -> Self {
		Self::Paragraphs
	}
}

/// Errors returned by the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
	pub kind: ErrorKind,
	pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
	Parse,
	Internal,
}

impl std::fmt::Display for Error {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}: {}", self.kind, self.message)
	}
}

impl std::error::Error for Error {}

impl ErrorKind {
	fn as_str(self) -> &'static str {
		match self {
			Self::Parse => "parse",
			Self::Internal => "internal",
		}
	}
}

impl std::fmt::Display for ErrorKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.as_str())
	}
}

pub(crate) struct Slot {
	engine: Engine<()>,
	doc: Doc,
	listeners: Vec<Box<dyn Fn(Change) + Send + Sync>>,
	read_only: bool,
	placeholder: String,
	#[cfg(all(target_arch = "wasm32", feature = "web"))]
	hooks: Option<Hooks>,
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub(crate) struct Hooks {
	pub set_editable: std::sync::Arc<dyn Fn(bool) + Send + Sync>,
	pub set_placeholder: std::sync::Arc<dyn Fn(&str) + Send + Sync>,
	pub focus: std::sync::Arc<dyn Fn() + Send + Sync>,
}

static REGISTRY: Mutex<BTreeMap<u64, Slot>> = Mutex::new(BTreeMap::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn configure_paragraphs(builder: &mut canvist_headless::canvist_core::EditorBuilder) {
	builder.add(Document);
	builder.add(Paragraph);
	builder.add(History);
	builder.add(HardBreak);
	builder.add(BaseKeymap);
}

impl CanvistState {
	pub fn new(_extensions: Extensions) -> Self {
		let id = NEXT_ID.fetch_add(1, AtomicOrdering::SeqCst);
		let engine = Engine::new(configure_paragraphs).expect("paragraphs are valid");
		let doc = engine.create().expect("create doc");
		let slot = Slot {
			engine,
			doc,
			listeners: Vec::new(),
			read_only: false,
			placeholder: String::new(),
			#[cfg(all(target_arch = "wasm32", feature = "web"))]
			hooks: None,
		};
		REGISTRY.lock().unwrap().insert(id, slot);
		Self(id)
	}

	pub fn load(&self, format: Format, input: &str) -> Result<(), Error> {
		let mut registry = REGISTRY.lock().unwrap();
		let slot = registry.get_mut(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		let doc = slot.engine.parse(format.to_canvist(), input).map_err(|e| Error {
			kind: ErrorKind::Parse,
			message: e.to_string(),
		})?;
		slot.doc = doc;
		Ok(())
	}

	pub fn export(&self, format: Format) -> impl Future<Output = Result<String, Error>> {
		let result = (|| {
			let registry = REGISTRY.lock().unwrap();
			let slot = registry.get(&self.0).ok_or_else(|| Error {
				kind: ErrorKind::Internal,
				message: "unknown state".to_owned(),
			})?;
			Ok(slot.doc.export(format.to_canvist()))
		})();
		std::future::ready(result)
	}

	pub fn on_change(&self, listener: impl Fn(Change) + Send + Sync + 'static) {
		let mut registry = REGISTRY.lock().unwrap();
		if let Some(slot) = registry.get_mut(&self.0) {
			slot.listeners.push(Box::new(listener));
		}
	}

	pub fn set_read_only(&self, read_only: bool) {
		let mut registry = REGISTRY.lock().unwrap();
		if let Some(slot) = registry.get_mut(&self.0) {
			slot.read_only = read_only;
			#[cfg(all(target_arch = "wasm32", feature = "web"))]
			{
				let hooks = slot.hooks.as_ref().map(|h| (h.set_editable.clone(), read_only));
				drop(registry);
				if let Some((f, v)) = hooks {
					f(v);
				}
			}
		}
	}

	pub fn set_placeholder(&self, placeholder: &str) {
		let mut registry = REGISTRY.lock().unwrap();
		if let Some(slot) = registry.get_mut(&self.0) {
			placeholder.clone_into(&mut slot.placeholder);
			#[cfg(all(target_arch = "wasm32", feature = "web"))]
			{
				let placeholder = slot.placeholder.clone();
				let hooks = slot.hooks.as_ref().map(|h| h.set_placeholder.clone());
				drop(registry);
				if let Some(f) = hooks {
					f(&placeholder);
				}
			}
		}
	}

	pub fn focus(&self) {
		let registry = REGISTRY.lock().unwrap();
		#[cfg(all(target_arch = "wasm32", feature = "web"))]
		{
			let hooks = registry.get(&self.0).and_then(|s| s.hooks.as_ref().map(|h| h.focus.clone()));
			drop(registry);
			if let Some(f) = hooks {
				f();
			}
		}
		#[cfg(not(target_arch = "wasm32"))]
		{
			drop(registry);
		}
	}

	#[allow(dead_code)]
	pub(crate) fn with_slot<R>(
		self,
		f: impl FnOnce(&mut Slot) -> R,
	) -> Result<R, Error> {
		let mut registry = REGISTRY.lock().unwrap();
		let slot = registry.get_mut(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		Ok(f(slot))
	}

	#[allow(dead_code)]
	pub(crate) fn read_slot<R>(
		self,
		f: impl FnOnce(&Slot) -> R,
	) -> Result<R, Error> {
		let registry = REGISTRY.lock().unwrap();
		let slot = registry.get(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		Ok(f(slot))
	}

	#[allow(dead_code)]
	pub(crate) fn is_read_only(self) -> bool {
		self.read_slot(|s| s.read_only).unwrap_or(false)
	}

	#[allow(dead_code)]
	pub(crate) fn placeholder(self) -> String {
		self.read_slot(|s| s.placeholder.clone()).unwrap_or_default()
	}

	#[cfg(all(target_arch = "wasm32", feature = "web"))]
	pub(crate) fn set_hooks(&self, hooks: Hooks) {
		let mut registry = REGISTRY.lock().unwrap();
		if let Some(slot) = registry.get_mut(&self.0) {
			slot.hooks = Some(hooks);
		}
	}

	#[cfg(all(target_arch = "wasm32", feature = "web"))]
	pub(crate) fn clear_hooks(&self) {
		let mut registry = REGISTRY.lock().unwrap();
		if let Some(slot) = registry.get_mut(&self.0) {
			slot.hooks = None;
		}
	}

	#[cfg(all(target_arch = "wasm32", feature = "web"))]
	pub(crate) fn notify_listeners(&self, change: Change) {
		let listeners: Vec<_> = {
			let registry = REGISTRY.lock().unwrap();
			match registry.get(&self.0) {
				Some(slot) => slot.listeners.iter().map(std::ptr::from_ref).collect(),
				None => Vec::new(),
			}
		};
		// Re-acquire to call; listeners are boxed closures. Simpler: clone
		// the Vec of callbacks by re-locking and invoking.
		let _ = listeners;
		let registry = REGISTRY.lock().unwrap();
		if let Some(slot) = registry.get(&self.0) {
			for l in &slot.listeners {
				l(change);
			}
		}
	}

	#[cfg(all(target_arch = "wasm32", feature = "web"))]
	pub(crate) fn doc_mut<R>(&self, f: impl FnOnce(&mut Doc) -> R) -> Result<R, Error> {
		let mut registry = REGISTRY.lock().unwrap();
		let slot = registry.get_mut(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		Ok(f(&mut slot.doc))
	}

	#[cfg(all(target_arch = "wasm32", feature = "web"))]
	pub(crate) fn doc_read<R>(&self, f: impl FnOnce(&Doc) -> R) -> Result<R, Error> {
		let registry = REGISTRY.lock().unwrap();
		let slot = registry.get(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		Ok(f(&slot.doc))
	}

	#[cfg(all(target_arch = "wasm32", feature = "web"))]
	pub(crate) fn edit_with_navigator(
		&self,
		lines: &mut dyn canvist_headless::canvist_core::LineNavigator,
		change: impl FnOnce(&mut Editor, &mut Env<'_>) -> CommandResult,
	) -> Result<Option<Change>, Error> {
		let mut registry = REGISTRY.lock().unwrap();
		let slot = registry.get_mut(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		let now = Duration::from_secs_f64(js_sys_date_now() / 1000.0);
		let lines = lines;
		let result = slot.doc.edit(now, &mut *lines, change);
		let update = match result {
			Ok(u) => u,
			Err(e) => {
				drop(registry);
				return Err(Error { kind: ErrorKind::Internal, message: e.to_string() });
			}
		};
		let (doc_changed, selection_changed) = match update {
			Some(ref u) => (u.doc_changed(), u.selection_changed()),
			None => (false, false),
		};
		let _ = update;
		drop(registry);
		if doc_changed || selection_changed {
			self.notify_listeners(Change { doc_changed, selection_changed });
		}
		Ok(Some(Change { doc_changed, selection_changed }))
	}
}

#[cfg(all(target_arch = "wasm32", feature = "web"))]
fn js_sys_date_now() -> f64 {
	// js-sys Date
	js_sys::Date::now()
}

// Expose Engine for web module.
// Re-export needed types for web module.
pub use canvist_headless::canvist_core::CommandResult;
pub use canvist_headless::canvist_core::Editor as CanvistEditor;
pub use canvist_headless::canvist_core::Env as CanvistEnv;

// Native-only helper for tests: insert text through Doc's own method.
impl CanvistState {
	pub fn insert_text_native(&self, text: &str) -> Result<bool, Error> {
		let mut registry = REGISTRY.lock().unwrap();
		let slot = registry.get_mut(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		let r = slot.doc.insert_text(text).map_err(|e| Error {
			kind: ErrorKind::Internal,
			message: e.to_string(),
		})?;
		drop(registry);
		self.notify_listeners_native(Change { doc_changed: r, selection_changed: r });
		Ok(r)
	}

	pub fn select_points(&self, anchor: Point, head: Point) -> Result<bool, Error> {
		let mut registry = REGISTRY.lock().unwrap();
		let slot = registry.get_mut(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		let selection = Selection::text(anchor, head);
		slot.doc.select(selection).map_err(|e| Error {
			kind: ErrorKind::Internal,
			message: e.to_string(),
		})?;
		let changed = true;
		drop(registry);
		self.notify_listeners_native(Change { doc_changed: false, selection_changed: changed });
		Ok(changed)
	}

	pub fn copy_selection(&self) -> Result<String, Error> {
		let registry = REGISTRY.lock().unwrap();
		let slot = registry.get(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		let slice = slot.doc.editor().copy().map_err(|e| Error {
			kind: ErrorKind::Internal,
			message: e.to_string(),
		})?;
		let text = slice.to_plain_text();
		Ok(text)
	}

	pub fn cut_selection(&self) -> Result<String, Error> {
		let text = self.copy_selection()?;
		let mut registry = REGISTRY.lock().unwrap();
		let slot = registry.get_mut(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		let _ = slot.doc.delete_selection();
		drop(registry);
		self.notify_listeners_native(Change { doc_changed: true, selection_changed: true });
		Ok(text)
	}

	pub fn paste_text(&self, text: &str) -> Result<bool, Error> {
		let mut registry = REGISTRY.lock().unwrap();
		let slot = registry.get_mut(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		let r = slot.doc.paste(canvist_headless::Format::Text, text).map_err(|e| Error {
			kind: ErrorKind::Internal,
			message: e.to_string(),
		})?;
		drop(registry);
		self.notify_listeners_native(Change { doc_changed: r, selection_changed: r });
		Ok(r)
	}

	pub fn is_empty_doc(&self) -> bool {
		self.read_slot(|s| {
			let document = s.doc.document();
			match document.children(None) {
				[b] => document.inline(*b).is_some_and(|t| t.as_str().is_empty()),
				_ => false,
			}
		}).unwrap_or(false)
	}

	pub fn press_native(&self, key: &str) -> Result<bool, Error> {
		let mut registry = REGISTRY.lock().unwrap();
		let slot = registry.get_mut(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		let r = slot.doc.press(key).map_err(|e| Error {
			kind: ErrorKind::Internal,
			message: e.to_string(),
		})?;
		drop(registry);
		self.notify_listeners_native(Change { doc_changed: r, selection_changed: r });
		Ok(r)
	}

	pub fn undo_native(&self) -> Result<bool, Error> {
		let mut registry = REGISTRY.lock().unwrap();
		let slot = registry.get_mut(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		let r = slot.doc.undo().map_err(|e| Error {
			kind: ErrorKind::Internal,
			message: e.to_string(),
		})?;
		drop(registry);
		self.notify_listeners_native(Change { doc_changed: r, selection_changed: r });
		Ok(r)
	}

	pub fn redo_native(&self) -> Result<bool, Error> {
		let mut registry = REGISTRY.lock().unwrap();
		let slot = registry.get_mut(&self.0).ok_or_else(|| Error {
			kind: ErrorKind::Internal,
			message: "unknown state".to_owned(),
		})?;
		let r = slot.doc.redo().map_err(|e| Error {
			kind: ErrorKind::Internal,
			message: e.to_string(),
		})?;
		drop(registry);
		self.notify_listeners_native(Change { doc_changed: r, selection_changed: r });
		Ok(r)
	}

	#[allow(dead_code)]
	fn notify_listeners_native(self, change: Change) {
		let registry = REGISTRY.lock().unwrap();
		if let Some(slot) = registry.get(&self.0) {
			for l in &slot.listeners {
				l(change);
			}
		}
	}
}

// Native notification helper (used by tests).
impl CanvistState {
	pub fn notify_change(&self, change: Change) {
		self.notify_listeners_native(change);
	}
}