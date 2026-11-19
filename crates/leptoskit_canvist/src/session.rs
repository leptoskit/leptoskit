//! One editor's engine and document, behind the lock a [`CanvistState`]
//! reaches it through.
//!
//! [`CanvistState`]: crate::CanvistState

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;

use canvist_extension_document::Document;
use canvist_extension_hard_break::HardBreak;
use canvist_extension_history::History;
#[cfg(feature = "ink")]
use canvist_extension_ink::Ink;
#[cfg(feature = "ink")]
use canvist_extension_ink::InkHandle;
use canvist_extension_paragraph::Paragraph;
use canvist_headless::Doc;
use canvist_headless::Engine;
use canvist_headless::HeadlessError;
use canvist_headless::canvist_core::BaseKeymap;
use canvist_headless::canvist_core::Platform;
use canvist_headless::canvist_core::UndoInputRule;
use canvist_headless::canvist_core::Update;

use crate::CanvistError;
use crate::Change;
use crate::Extensions;
use crate::Format;
#[cfg(feature = "ink")]
use crate::Pen;
use crate::state::Kind;

/// Something to call with every [`Change`].
pub(crate) type Listener = Arc<dyn Fn(Change) + Send + Sync>;

/// A session shared between a handle and the host that draws it.
pub(crate) type Shared = Arc<Mutex<Session>>;

/// What the extensions an editor was made with let code do to it.
#[derive(Clone, Copy)]
struct Handles {
	#[cfg(feature = "ink")]
	ink: Option<InkHandle>,
}

/// A change an app makes through the handle rather than a person through
/// the keyboard or the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
	Undo,
	Redo,
	#[cfg(feature = "ink")]
	InsertDrawing,
}

/// The engine, the document, and what the app asked of the editor.
pub(crate) struct Session {
	engine: Engine<Handles>,
	pub(crate) doc: Doc,
	listeners: Vec<(u64, Listener)>,
	next_listener: u64,
	pub(crate) read_only: bool,
	pub(crate) placeholder: String,
	/// What the next stroke of a drawing is drawn with.
	#[cfg(feature = "ink")]
	pub(crate) pen: Pen,
	revision: u64,
}

// The registry of sessions is shared by every thread, so a session has to be
// able to cross one. This fails to compile if a canvist release makes its
// documents thread-bound.
const _: fn() = || {
	fn assert_send<T: Send>() {}
	assert_send::<Session>();
};

impl Session {
	/// A session with an empty document: one empty paragraph.
	pub(crate) fn new(extensions: Extensions, platform: Platform) -> Result<Self, CanvistError> {
		let engine = match extensions.kind() {
			Kind::Paragraphs => {
				Engine::new(move |builder| {
					builder.add(Document);
					builder.add(BaseKeymap);
					builder.add(UndoInputRule);
					builder.add(Paragraph);
					builder.add(HardBreak);
					builder.add(History);

					Handles {
						#[cfg(feature = "ink")]
						ink: extensions.has_ink().then(|| builder.add(Ink)),
					}
				})
			}
		}
		.map_err(|error| engine_error(&error))?
		.with_platform(platform);
		let doc = engine.create().map_err(|error| engine_error(&error))?;

		Ok(Self {
			engine,
			doc,
			listeners: Vec::new(),
			next_listener: 0,
			read_only: false,
			placeholder: String::new(),
			#[cfg(feature = "ink")]
			pen: Pen::new(),
			revision: 0,
		})
	}

	/// The handle to this editor's drawings, when it was made with ink.
	#[cfg(feature = "ink")]
	pub(crate) fn ink(&self) -> Option<InkHandle> {
		self.engine.handles().ink
	}

	/// Do `action` to a document no host is showing. `None` when it changed
	/// nothing.
	pub(crate) fn act(&mut self, action: Action) -> Result<Option<Change>, CanvistError> {
		let applied = match action {
			Action::Undo => self.doc.undo(),
			Action::Redo => self.doc.redo(),
			#[cfg(feature = "ink")]
			Action::InsertDrawing => {
				let ink = self.ink().ok_or(CanvistError::Missing("ink"))?;
				self.doc
					.run(&ink.insert(canvist_extension_ink::DEFAULT_HEIGHT))
			}
		}
		.map_err(|error| engine_error(&error))?;

		Ok(applied.then(|| self.changed(true, true)))
	}

	/// Replace the document with `input` read as `format`. The undo history
	/// starts again, as it belongs to the document that was replaced.
	pub(crate) fn load(&mut self, format: Format, input: &str) -> Result<Change, CanvistError> {
		self.doc = self.engine.parse(format.engine(), input).map_err(|error| {
			match error {
				HeadlessError::Parse { reason, .. } | HeadlessError::Schema(reason) => {
					CanvistError::Parse { format, reason }
				}
				other => engine_error(&other),
			}
		})?;

		Ok(self.changed(true, true))
	}

	/// The document as `format`.
	pub(crate) fn export(&self, format: Format) -> String {
		self.doc.export(format.engine())
	}

	/// Whether the document is a single empty text block, as a new one is.
	pub(crate) fn is_empty(&self) -> bool {
		let document = self.doc.document();

		match document.children(None) {
			[block] => {
				document
					.inline(*block)
					.is_some_and(|text| text.as_str().is_empty())
			}
			_ => false,
		}
	}

	/// How many changes the editor has reported.
	pub(crate) fn revision(&self) -> u64 {
		self.revision
	}

	/// The change `update` made, or `None` when it moved nothing.
	#[cfg_attr(
		not(all(feature = "web", target_arch = "wasm32")),
		expect(dead_code, reason = "only a host applies updates")
	)]
	pub(crate) fn record(&mut self, update: &Update) -> Option<Change> {
		let (doc, selection) = (update.doc_changed(), update.selection_changed());
		(doc || selection).then(|| self.changed(doc, selection))
	}

	fn changed(&mut self, doc_changed: bool, selection_changed: bool) -> Change {
		self.revision += 1;

		Change {
			revision: self.revision,
			doc_changed,
			selection_changed,
		}
	}

	/// Call `listener` with every change from now on. Returns the id that
	/// removes it.
	pub(crate) fn listen(&mut self, listener: Listener) -> u64 {
		self.next_listener += 1;
		self.listeners.push((self.next_listener, listener));
		self.next_listener
	}

	pub(crate) fn unlisten(&mut self, id: u64) {
		self.listeners.retain(|(listener, _)| *listener != id);
	}

	fn listeners(&self) -> Vec<Listener> {
		self.listeners
			.iter()
			.map(|(_, listener)| Arc::clone(listener))
			.collect()
	}
}

/// Lock `session`. A panic in another holder leaves the document as that
/// holder last wrote it, which is still a valid document.
pub(crate) fn lock(session: &Shared) -> MutexGuard<'_, Session> {
	session.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Tell every listener about `change`.
///
/// Call this with the session unlocked: a listener may call back into the
/// handle, which locks it again.
pub(crate) fn notify(session: &Shared, change: Change) {
	let listeners = lock(session).listeners();

	for listener in listeners {
		listener(change);
	}
}

/// A failure inside canvist that the caller could not have caused.
pub(crate) fn engine_error(error: &dyn std::fmt::Display) -> CanvistError {
	CanvistError::Engine(error.to_string())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn session() -> Session {
		Session::new(Extensions::paragraphs(), Platform::Other).expect("a paragraph editor")
	}

	#[test]
	fn a_new_document_is_one_empty_paragraph() {
		let session = session();

		assert!(session.is_empty());
		assert_eq!(session.export(Format::Text), "");
		assert_eq!(session.revision(), 0);
	}

	#[test]
	fn typing_and_keys_edit_paragraphs() {
		let mut session = session();
		session.doc.insert_text("Hello").expect("typing");
		session.doc.press("Enter").expect("a binding for Enter");
		session.doc.insert_text("World").expect("typing");

		assert_eq!(session.export(Format::Text), "Hello\nWorld");
		assert!(!session.is_empty());

		session.doc.press("Shift-Enter").expect("a hard break");
		session.doc.insert_text("again").expect("typing");

		assert_eq!(session.export(Format::Markdown), "Hello\n\nWorld\\\nagain");
	}

	#[test]
	fn undo_and_redo_step_through_history() {
		let mut session = session();
		session.doc.insert_text("Hello").expect("typing");
		session.doc.press("Enter").expect("a binding for Enter");

		assert!(session.doc.undo().expect("undo"));
		assert_eq!(session.export(Format::Text), "Hello");
		assert!(session.doc.redo().expect("redo"));
		assert_eq!(session.export(Format::Text), "Hello\n");
	}

	#[test]
	fn loading_replaces_the_document_and_its_history() {
		let mut session = session();
		session.doc.insert_text("draft").expect("typing");
		let change = session
			.load(Format::Markdown, "one\n\ntwo")
			.expect("valid Markdown");

		assert_eq!(change.revision, 1);
		assert!(change.doc_changed);
		assert_eq!(session.export(Format::Text), "one\ntwo");
		assert!(!session.doc.can_undo());
	}

	#[test]
	fn loading_rejects_what_the_schema_cannot_hold() {
		let mut session = session();
		session.doc.insert_text("kept").expect("typing");
		let error = session
			.load(Format::Json, "{ not json")
			.expect_err("unreadable JSON");

		assert!(matches!(
			error,
			CanvistError::Parse {
				format: Format::Json,
				..
			}
		));
		assert_eq!(session.export(Format::Text), "kept");
		assert_eq!(session.revision(), 0);
	}

	#[test]
	fn listeners_are_removed_by_id() {
		let mut session = session();
		let first = session.listen(Arc::new(|_| {}));
		let second = session.listen(Arc::new(|_| {}));

		assert_ne!(first, second);
		session.unlisten(first);
		assert_eq!(session.listeners().len(), 1);
	}
}
