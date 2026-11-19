//! The handle an app holds an editor by.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

use crate::session;
use crate::session::Action;
use crate::session::Session;
use crate::session::Shared;

/// Every live editor, by its handle's id.
static SESSIONS: Mutex<BTreeMap<u32, Shared>> = Mutex::new(BTreeMap::new());

/// The id the next editor gets. Ids are never reused, so a handle to a
/// disposed editor can never reach a newer one. They are `u32` because that
/// is what a `CanvistEditor` element carries.
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

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
pub struct CanvistState(u32);

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
///
/// Start from a set and add to it. What can be added depends on the Cargo
/// features the app turned on, so an editor never carries code for content
/// the app does not use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Extensions {
	kind: Kind,
	#[cfg(feature = "ink")]
	ink: bool,
}

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
		Self {
			kind: Kind::Paragraphs,
			#[cfg(feature = "ink")]
			ink: false,
		}
	}

	/// Add freehand drawings that sit between paragraphs.
	///
	/// [`CanvistState::insert_drawing`] puts an empty drawing at the caret,
	/// and people draw on it with a mouse, a pen or a finger, in the pen
	/// [`CanvistState::set_pen`] chose. Every stroke is one step of undo.
	/// Drawings are saved in the document, in Markdown and JSON alike.
	///
	/// ```
	/// use leptoskit_canvist::CanvistState;
	/// use leptoskit_canvist::Extensions;
	///
	/// let note = CanvistState::new(Extensions::paragraphs().ink());
	///
	/// assert_eq!(note.insert_drawing(), Ok(true));
	/// assert!(!note.is_empty());
	/// # note.dispose();
	/// ```
	#[cfg(feature = "ink")]
	#[must_use]
	pub const fn ink(self) -> Self {
		Self { ink: true, ..self }
	}

	pub(crate) const fn kind(self) -> Kind {
		self.kind
	}

	#[cfg(feature = "ink")]
	pub(crate) const fn has_ink(self) -> bool {
		self.ink
	}
}

/// What the next stroke of a drawing is drawn with.
///
/// ```
/// use leptoskit_canvist::Pen;
///
/// let marker = Pen::new().color(0xe1, 0x1d, 0x48).width(12.0);
/// # let _ = marker;
/// ```
#[cfg(feature = "ink")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pen {
	color: Option<[u8; 3]>,
	width: Option<f32>,
}

#[cfg(feature = "ink")]
impl Pen {
	/// A fine pen in the colour of the text, so its strokes stay readable
	/// when the page switches between light and dark.
	#[must_use]
	pub const fn new() -> Self {
		Self {
			color: None,
			width: None,
		}
	}

	/// Draw in this colour instead of the text's.
	#[must_use]
	pub const fn color(self, red: u8, green: u8, blue: u8) -> Self {
		Self {
			color: Some([red, green, blue]),
			..self
		}
	}

	/// Draw lines this wide. A drawing is 1000 units across however wide
	/// the editor is, so a line keeps its weight on a phone and on a wide
	/// screen; a fine pen is 4. A width that is not a positive number
	/// leaves the pen as fine as it was made.
	#[must_use]
	pub const fn width(self, width: f32) -> Self {
		if width.is_finite() && width > 0.0 {
			Self {
				width: Some(width),
				..self
			}
		} else {
			self
		}
	}

	/// The colour as `#rrggbb`, when the pen has its own.
	#[cfg(any(test, all(feature = "web", target_arch = "wasm32")))]
	pub(crate) fn hex(self) -> Option<String> {
		self.color
			.map(|[red, green, blue]| format!("#{red:02x}{green:02x}{blue:02x}"))
	}

	/// The colour, when the pen has its own.
	#[cfg(all(feature = "web", target_arch = "wasm32"))]
	pub(crate) const fn rgb(self) -> Option<[u8; 3]> {
		self.color
	}

	/// The width in drawing units, when the pen has its own.
	#[cfg(all(feature = "web", target_arch = "wasm32"))]
	pub(crate) const fn units(self) -> Option<f32> {
		self.width
	}
}

#[cfg(feature = "ink")]
impl Default for Pen {
	fn default() -> Self {
		Self::new()
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
	/// The editor was made without the extension this needs, which is named
	/// here: `ink`, for one.
	Missing(&'static str),
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
			Self::Missing(extension) => {
				write!(formatter, "the editor was made without `{extension}`")
			}
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

	/// Undo the last change, as Ctrl+Z or Cmd+Z does, so an app can offer a
	/// button where there is no keyboard. Returns whether there was
	/// anything to undo.
	pub fn undo(self) -> bool {
		self.act(Action::Undo).unwrap_or(false)
	}

	/// Redo the change that was last undone. Returns whether there was one.
	pub fn redo(self) -> bool {
		self.act(Action::Redo).unwrap_or(false)
	}

	/// Put an empty drawing at the caret, between the paragraphs there: a
	/// paragraph the caret is in the middle of is split around it.
	///
	/// Returns whether a drawing went in; none does while a drawing itself
	/// is selected. Listeners hear of the change, and undo takes the drawing
	/// out again. People then draw on it with the pointer, unless the editor
	/// is read-only.
	///
	/// A drawing is as wide as the editor and two fifths as tall.
	///
	/// # Errors
	///
	/// [`CanvistError::Missing`] when the editor was made without
	/// [`Extensions::ink`], and [`CanvistError::Disposed`] when it is gone.
	#[cfg(feature = "ink")]
	pub fn insert_drawing(self) -> Result<bool, CanvistError> {
		self.act(Action::InsertDrawing)
	}

	/// Draw the strokes that follow with `pen`. Strokes already drawn keep
	/// the pen they were drawn with.
	#[cfg(feature = "ink")]
	pub fn set_pen(self, pen: Pen) {
		if let Some(session) = self.session() {
			session::lock(&session).pen = pen;
		}
	}

	/// The pen the next stroke is drawn with.
	#[cfg(feature = "ink")]
	#[must_use]
	pub fn pen(self) -> Pen {
		self.session()
			.map_or_else(Pen::new, |session| session::lock(&session).pen)
	}

	/// Do `action` through the host that shows the editor, so what is on
	/// screen follows, or to the document alone when nothing shows it.
	/// Returns whether anything changed.
	fn act(self, action: Action) -> Result<bool, CanvistError> {
		let session = self.session().ok_or(CanvistError::Disposed)?;

		#[cfg(all(feature = "web", target_arch = "wasm32"))]
		if let Some(shown) = crate::web::act(self.0, action) {
			return shown;
		}

		let change = session::lock(&session).act(action)?;

		if let Some(change) = change {
			session::notify(&session, change);
		}

		Ok(change.is_some())
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

	/// The editor's id: what a `CanvistEditor` element takes to show this
	/// editor.
	///
	/// ```
	/// use leptoskit_canvist::CanvistState;
	/// use leptoskit_canvist::Extensions;
	///
	/// let note = CanvistState::new(Extensions::paragraphs());
	/// let other = CanvistState::new(Extensions::paragraphs());
	///
	/// assert_ne!(note.id(), other.id());
	/// # note.dispose();
	/// # other.dispose();
	/// ```
	///
	/// With leptoskit that is `CanvistEditor(note.id())`. On the web, with
	/// the `web` feature, asking for the id also starts watching the page, so
	/// the editor appears in that element when it does and leaves with it;
	/// see `web` for what the element's own settings do. An id is never
	/// handed out twice, and the server's editor and the browser's have
	/// different ones.
	#[must_use]
	pub fn id(self) -> u32 {
		#[cfg(all(feature = "web", target_arch = "wasm32"))]
		crate::web::watch_elements();

		self.0
	}

	/// The id hosts file this editor under, without watching the page.
	#[cfg(all(feature = "web", target_arch = "wasm32"))]
	pub(crate) fn key(self) -> u32 {
		self.0
	}

	/// The handle with `id`, whether or not such an editor is alive.
	#[cfg(all(feature = "web", target_arch = "wasm32"))]
	pub(crate) fn from_id(id: u32) -> Self {
		Self(id)
	}

	/// Whether the editor is alive: created and not yet disposed.
	#[cfg(all(feature = "web", target_arch = "wasm32"))]
	pub(crate) fn is_alive(self) -> bool {
		sessions().contains_key(&self.0)
	}

	pub(crate) fn session(self) -> Option<Shared> {
		sessions().get(&self.0).cloned()
	}
}

fn sessions() -> std::sync::MutexGuard<'static, BTreeMap<u32, Shared>> {
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
	fn undo_and_redo_are_heard_and_say_when_they_run_out() {
		let state = CanvistState::new(Extensions::paragraphs());
		let calls = Arc::new(AtomicUsize::new(0));
		state.on_change({
			let calls = Arc::clone(&calls);
			move |_| {
				calls.fetch_add(1, Ordering::Relaxed);
			}
		});

		assert!(!state.undo(), "a new document has nothing to undo");
		assert!(!state.redo());
		assert_eq!(calls.load(Ordering::Relaxed), 0);
	}

	#[cfg(feature = "ink")]
	#[test]
	fn a_drawing_goes_in_and_comes_out_with_undo() {
		let state = CanvistState::new(Extensions::paragraphs().ink());
		state.load(Format::Text, "above").expect("plain text");
		let heard = Arc::new(Mutex::new(Vec::new()));
		state.on_change({
			let heard = Arc::clone(&heard);
			move |change: Change| heard.lock().expect("no panics").push(change.revision)
		});

		assert_eq!(state.insert_drawing(), Ok(true));
		assert_eq!(*heard.lock().expect("no panics"), [2]);

		let json = now(state.export(Format::Json)).expect("a live editor");
		// Plain text has no way to write a drawing, and leaves it out.
		assert_eq!(now(state.export(Format::Text)), Ok("above".into()));
		assert!(json.contains(r#""type":"ink""#), "{json}");

		assert!(state.undo());
		assert_eq!(now(state.export(Format::Text)), Ok("above".into()));
		assert!(state.redo());
		assert_eq!(now(state.export(Format::Json)), Ok(json));
		state.dispose();
	}

	#[cfg(feature = "ink")]
	#[test]
	fn drawings_survive_markdown_and_json() {
		let json = r##"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"A sketch:"}]},{"type":"ink","attrs":{"height":300,"strokes":[{"color":"#e11d48","points":[10.0,20.0,200.5,120.0],"width":12.0}]}},{"type":"paragraph","content":[{"type":"text","text":"Done."}]}]}"##;
		let state = CanvistState::new(Extensions::paragraphs().ink());
		state.load(Format::Json, json).expect("a document with ink");
		let markdown = now(state.export(Format::Markdown)).expect("a live editor");

		assert!(markdown.starts_with("A sketch:\n\n<figure data-type=\"ink\""));
		assert!(markdown.ends_with("</figure>\n\nDone."));

		let copy = CanvistState::new(Extensions::paragraphs().ink());
		copy.load(Format::Markdown, &markdown)
			.expect("its own Markdown");

		assert_eq!(
			now(copy.export(Format::Json)),
			now(state.export(Format::Json))
		);
		state.dispose();
		copy.dispose();
	}

	#[cfg(feature = "ink")]
	#[test]
	fn an_editor_without_ink_refuses_drawings() {
		let state = CanvistState::new(Extensions::paragraphs());

		assert_eq!(state.insert_drawing(), Err(CanvistError::Missing("ink")));
		assert_eq!(
			CanvistError::Missing("ink").to_string(),
			"the editor was made without `ink`"
		);

		let error = state
			.load(Format::Json, r#"{"type":"doc","content":[{"type":"ink"}]}"#)
			.expect_err("the schema has no drawings");
		assert!(matches!(error, CanvistError::Parse { .. }), "{error:?}");

		state.dispose();
		assert_eq!(state.insert_drawing(), Err(CanvistError::Disposed));
	}

	#[cfg(feature = "ink")]
	#[test]
	fn a_pen_keeps_a_sensible_width_and_writes_its_colour() {
		let state = CanvistState::new(Extensions::paragraphs().ink());

		assert_eq!(state.pen(), Pen::new());
		assert_eq!(state.pen().hex(), None);

		let marker = Pen::new().color(0xe1, 0x1d, 0x48).width(12.0);
		state.set_pen(marker);

		assert_eq!(state.pen(), marker);
		assert_eq!(marker.hex().as_deref(), Some("#e11d48"));
		assert_eq!(marker.width(0.0), marker, "a pen cannot be made no width");
		assert_eq!(marker.width(f32::NAN), marker);
		state.dispose();
		assert_eq!(state.pen(), Pen::default());
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
