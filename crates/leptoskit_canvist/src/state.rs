//! The handle an app holds an editor by.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use crate::session;
use crate::session::Session;
use crate::session::Shared;

/// Every live editor, by its handle's id.
static SESSIONS: Mutex<BTreeMap<u64, Shared>> = Mutex::new(BTreeMap::new());

/// The id the next editor gets. Ids are never reused, so a handle to a
/// disposed editor can never reach a newer one.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// An editor the app owns.
///
/// The handle is a small `Copy` id, so it moves into closures and signals
/// freely. The document lives beside it, not in a prop: an app that pushed
/// the whole document back in on every keystroke would break input methods,
/// the selection and undo. Load a document once, listen for changes with
/// [`CanvistState::on_change`], and export when it is time to save.
///
/// ```
/// use leptoskit_canvist::CanvistState;
/// use leptoskit_canvist::Extensions;
/// use leptoskit_canvist::Format;
///
/// let note = CanvistState::new(Extensions::paragraphs());
/// note.load(Format::Markdown, "Hello\n\nWorld")?;
///
/// assert!(!note.is_empty());
/// note.dispose();
/// # Ok::<(), leptoskit_canvist::CanvistError>(())
/// ```
///
/// An editor lives until [`CanvistState::dispose`]: nothing drops it when the
/// last copy of the handle goes away, because a `Copy` handle cannot tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CanvistState(u64);

/// The formats a document loads from and exports to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
	/// Plain text: one paragraph per line.
	Text,
	/// Markdown: paragraphs apart by a blank line, and a hard break as a
	/// backslash at the end of a line.
	Markdown,
	/// ProseMirror and TipTap JSON (`{ "type": "doc", "content": [...] }`),
	/// which round-trips everything the editor holds.
	Json,
}

impl Format {
	pub(crate) fn engine(self) -> canvist_headless::Format {
		match self {
			Self::Text => canvist_headless::Format::Text,
			Self::Markdown => canvist_headless::Format::Markdown,
			Self::Json => canvist_headless::Format::Json,
		}
	}
}

impl fmt::Display for Format {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(match self {
			Self::Text => "text",
			Self::Markdown => "Markdown",
			Self::Json => "JSON",
		})
	}
}

/// What an editor can hold and do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Extensions(Kind);

/// The extension sets on offer. Private, so new sets are not breaking
/// changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
	Paragraphs,
}

impl Extensions {
	/// Plain paragraphs: text, line breaks inside a paragraph (Shift+Enter),
	/// and undo and redo.
	#[must_use]
	pub const fn paragraphs() -> Self {
		Self(Kind::Paragraphs)
	}

	pub(crate) const fn kind(self) -> Kind {
		self.0
	}
}

/// One change to an editor, as its [`CanvistState::on_change`] listeners
/// hear of it.
///
/// It says what kind of change happened, not what the document now holds:
/// export the document when you need it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Change {
	/// Counts the editor's changes from 1, so a listener that debounces can
	/// tell whether anything happened since it last looked.
	pub revision: u64,
	/// The text changed.
	pub doc_changed: bool,
	/// The caret moved, or the selection grew, shrank or went away.
	pub selection_changed: bool,
}

/// Why an editor refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CanvistError {
	/// The handle's editor was disposed.
	Disposed,
	/// The input is not a document in the format it was loaded as.
	Parse {
		/// The format the input was read as.
		format: Format,
		/// What is wrong with it.
		reason: String,
	},
	/// The browser has nothing to draw the editor on: no window, no
	/// document, or no 2D canvas.
	Host(String),
	/// canvist failed in a way the caller did not cause.
	Engine(String),
}

impl fmt::Display for CanvistError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Disposed => formatter.write_str("the editor was disposed"),
			Self::Parse { format, reason } => {
				write!(formatter, "could not read the input as {format}: {reason}")
			}
			Self::Host(reason) => write!(formatter, "the editor cannot be shown here: {reason}"),
			Self::Engine(reason) => write!(formatter, "the editor failed: {reason}"),
		}
	}
}

impl std::error::Error for CanvistError {}

/// Keeps an [`CanvistState::on_change`] listener removable.
///
/// Dropping it leaves the listener in place until the editor is disposed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Subscription {
	state: CanvistState,
	listener: u64,
}

impl Subscription {
	/// Stop calling the listener.
	pub fn cancel(self) {
		if let Some(session) = self.state.session() {
			session::lock(&session).unlisten(self.listener);
		}
	}
}

impl CanvistState {
	/// A new editor holding one empty paragraph.
	///
	/// # Panics
	///
	/// If canvist rejects the extension set, which would be a defect in this
	/// crate: the sets it offers are fixed.
	#[must_use]
	pub fn new(extensions: Extensions) -> Self {
		let session = Session::new(extensions, platform())
			.unwrap_or_else(|error| panic!("the built-in extensions are valid: {error}"));
		let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
		sessions().insert(id, Arc::new(Mutex::new(session)));

		Self(id)
	}

	/// Replace the document with `input`, read as `format`.
	///
	/// The undo history starts again, and listeners hear of the change. When
	/// the input cannot be read, the document stays as it was.
	pub fn load(self, format: Format, input: &str) -> Result<(), CanvistError> {
		let session = self.session().ok_or(CanvistError::Disposed)?;
		let change = session::lock(&session).load(format, input)?;

		#[cfg(all(feature = "web", target_arch = "wasm32"))]
		crate::web::document_replaced(self.0);

		session::notify(&session, change);

		Ok(())
	}

	/// The document as `format`.
	///
	/// On the web the answer is ready at once. It is a future because an
	/// editor that lives in a native host, across a bridge, has to be asked.
	pub fn export(
		self,
		format: Format,
	) -> impl Future<Output = Result<String, CanvistError>> + Send + 'static {
		let exported = self
			.session()
			.map(|session| session::lock(&session).export(format))
			.ok_or(CanvistError::Disposed);

		std::future::ready(exported)
	}

	/// Call `listener` after every change to the document or the selection,
	/// whether a person made it or [`CanvistState::load`] did.
	///
	/// The listener runs after the change is applied, so it may call back
	/// into the handle.
	pub fn on_change(self, listener: impl Fn(Change) + Send + Sync + 'static) -> Subscription {
		let listener = self.session().map_or(0, |session| {
			session::lock(&session).listen(Arc::new(listener))
		});

		Subscription {
			state: self,
			listener,
		}
	}

	/// Stop people changing the document, or let them again.
	///
	/// A read-only editor still takes focus, and people can still select text
	/// with the pointer and the arrow keys and copy it. [`CanvistState::load`]
	/// still works.
	pub fn set_read_only(self, read_only: bool) {
		let Some(session) = self.session() else {
			return;
		};
		let changed =
			std::mem::replace(&mut session::lock(&session).read_only, read_only) != read_only;

		if changed {
			#[cfg(all(feature = "web", target_arch = "wasm32"))]
			crate::web::options_changed(self.0);
		}
	}

	/// Whether people are stopped from changing the document.
	#[must_use]
	pub fn is_read_only(self) -> bool {
		self.session()
			.is_some_and(|session| session::lock(&session).read_only)
	}

	/// Show `text`, dimmed, where the first line starts while the document is
	/// empty. An empty string shows nothing.
	pub fn set_placeholder(self, text: &str) {
		let Some(session) = self.session() else {
			return;
		};

		{
			let mut session = session::lock(&session);

			if session.placeholder == text {
				return;
			}

			text.clone_into(&mut session.placeholder);
		}

		#[cfg(all(feature = "web", target_arch = "wasm32"))]
		crate::web::options_changed(self.0);
	}

	/// Whether the document is a single empty paragraph, as a new one is.
	#[must_use]
	pub fn is_empty(self) -> bool {
		self.session()
			.is_some_and(|session| session::lock(&session).is_empty())
	}

	/// How many changes the editor has reported: the `revision` of the last
	/// [`Change`], or 0 before the first.
	#[must_use]
	pub fn revision(self) -> u64 {
		self.session()
			.map_or(0, |session| session::lock(&session).revision())
	}

	/// Move keyboard focus to the editor, without scrolling to it. It does
	/// nothing until the editor is on screen.
	pub fn focus(self) {
		#[cfg(all(feature = "web", target_arch = "wasm32"))]
		crate::web::focus(self.0);
	}

	/// Free the editor: its document, its history and its listeners.
	///
	/// On the web its canvas stops responding and is removed. Every copy of
	/// the handle is inert afterwards: reads answer as an empty editor would,
	/// and [`CanvistState::load`] and [`CanvistState::export`] fail with
	/// [`CanvistError::Disposed`].
	pub fn dispose(self) {
		#[cfg(all(feature = "web", target_arch = "wasm32"))]
		crate::web::disposed(self.0);

		sessions().remove(&self.0);
	}

	/// The id hosts file this editor under.
	#[cfg_attr(
		not(all(feature = "web", target_arch = "wasm32")),
		expect(dead_code, reason = "only a host files editors by id")
	)]
	pub(crate) fn id(self) -> u64 {
		self.0
	}

	pub(crate) fn session(self) -> Option<Shared> {
		sessions().get(&self.0).cloned()
	}
}

fn sessions() -> std::sync::MutexGuard<'static, BTreeMap<u64, Shared>> {
	SESSIONS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether `Mod` in key bindings is Cmd or Ctrl where this editor runs.
fn platform() -> canvist_headless::canvist_core::Platform {
	#[cfg(all(feature = "web", target_arch = "wasm32"))]
	return crate::web::platform();

	#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
	canvist_headless::canvist_core::Platform::default()
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::AtomicUsize;

	use super::*;

	/// Run a future that is ready at once.
	fn now<T>(future: impl Future<Output = T>) -> T {
		let mut future = std::pin::pin!(future);
		let mut context = std::task::Context::from_waker(std::task::Waker::noop());

		match future.as_mut().poll(&mut context) {
			std::task::Poll::Ready(value) => value,
			std::task::Poll::Pending => panic!("an export on this platform is ready at once"),
		}
	}

	#[test]
	fn load_and_export_round_trip_every_format() {
		let state = CanvistState::new(Extensions::paragraphs());
		state
			.load(Format::Markdown, "Hello\n\nWorld")
			.expect("valid Markdown");

		assert_eq!(now(state.export(Format::Text)), Ok("Hello\nWorld".into()));
		assert_eq!(
			now(state.export(Format::Markdown)),
			Ok("Hello\n\nWorld".into())
		);

		let json = now(state.export(Format::Json)).expect("a live editor");
		let copy = CanvistState::new(Extensions::paragraphs());
		copy.load(Format::Json, &json).expect("its own JSON");

		assert_eq!(now(copy.export(Format::Json)), Ok(json));
		assert_eq!(
			now(copy.export(Format::Markdown)),
			Ok("Hello\n\nWorld".into())
		);

		state.dispose();
		copy.dispose();
	}

	#[test]
	fn listeners_hear_each_load_once_until_cancelled() {
		let state = CanvistState::new(Extensions::paragraphs());
		let heard = Arc::new(Mutex::new(Vec::new()));
		let subscription = state.on_change({
			let heard = Arc::clone(&heard);
			move |change| heard.lock().expect("no panics").push(change)
		});

		state.load(Format::Text, "one").expect("plain text");
		state.load(Format::Text, "two").expect("plain text");

		assert_eq!(
			*heard.lock().expect("no panics"),
			[
				Change {
					revision: 1,
					doc_changed: true,
					selection_changed: true
				},
				Change {
					revision: 2,
					doc_changed: true,
					selection_changed: true
				},
			]
		);
		assert_eq!(state.revision(), 2);

		subscription.cancel();
		state.load(Format::Text, "three").expect("plain text");

		assert_eq!(heard.lock().expect("no panics").len(), 2);
		state.dispose();
	}

	#[test]
	fn a_listener_may_call_back_into_the_handle() {
		let state = CanvistState::new(Extensions::paragraphs());
		let calls = Arc::new(AtomicUsize::new(0));
		state.on_change({
			let calls = Arc::clone(&calls);
			move |change| {
				calls.fetch_add(1, Ordering::Relaxed);

				if change.revision == 1 {
					assert_eq!(now(state.export(Format::Text)), Ok("first".into()));
					state.load(Format::Text, "second").expect("plain text");
				}
			}
		});

		state.load(Format::Text, "first").expect("plain text");

		assert_eq!(calls.load(Ordering::Relaxed), 2);
		assert_eq!(now(state.export(Format::Text)), Ok("second".into()));
		state.dispose();
	}

	#[test]
	fn a_failed_load_changes_nothing_and_tells_no_one() {
		let state = CanvistState::new(Extensions::paragraphs());
		state.load(Format::Text, "kept").expect("plain text");
		let calls = Arc::new(AtomicUsize::new(0));
		state.on_change({
			let calls = Arc::clone(&calls);
			move |_| {
				calls.fetch_add(1, Ordering::Relaxed);
			}
		});

		let error = state
			.load(
				Format::Json,
				r#"{"type":"doc","content":[{"type":"table"}]}"#,
			)
			.expect_err("the paragraph schema has no tables");

		assert!(
			matches!(
				&error,
				CanvistError::Parse {
					format: Format::Json,
					..
				}
			),
			"{error:?}"
		);
		assert!(
			error
				.to_string()
				.starts_with("could not read the input as JSON")
		);
		assert_eq!(calls.load(Ordering::Relaxed), 0);
		assert_eq!(now(state.export(Format::Text)), Ok("kept".into()));
		state.dispose();
	}

	#[test]
	fn options_are_remembered() {
		let state = CanvistState::new(Extensions::paragraphs());

		assert!(!state.is_read_only());
		assert!(state.is_empty());

		state.set_read_only(true);
		state.set_placeholder("Write a note…");

		assert!(state.is_read_only());
		// Read-only stops people, not the app.
		state
			.load(Format::Text, "set by the app")
			.expect("plain text");
		assert!(!state.is_empty());
		state.dispose();
	}

	#[test]
	fn a_disposed_handle_is_inert() {
		let state = CanvistState::new(Extensions::paragraphs());
		let other = CanvistState::new(Extensions::paragraphs());
		state.dispose();

		assert_eq!(
			state.load(Format::Text, "late"),
			Err(CanvistError::Disposed)
		);
		assert_eq!(now(state.export(Format::Text)), Err(CanvistError::Disposed));
		assert!(!state.is_empty());
		assert!(!state.is_read_only());
		state.set_read_only(true);
		state.set_placeholder("ignored");
		state.focus();
		state.on_change(|_| {}).cancel();
		state.dispose();

		assert_ne!(state, other);
		assert!(other.is_empty(), "disposing one editor leaves the others");
		other.dispose();
	}
}
