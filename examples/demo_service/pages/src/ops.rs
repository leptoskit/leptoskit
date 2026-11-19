//! What a page asks the server for: the operations, their answers, and the
//! reasons the server names when it turns one down.
//!
//! The browser calls these by type. The service runs their bodies, and each
//! body hands the request to the service's own [`Backend`], which the
//! service builds for that one request from its pass, its session, and where
//! it came from. Nothing here opens a database or reads a cookie.

#![expect(
	unused_qualifications,
	reason = "the operation macros name `leptoskit::rpc` in full beside this module's import"
)]

use leptoskit::rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::model::Persona;

/// The refusal code for a day that has handed out all its people.
pub const FULL: &str = "full";

/// The refusal code for a browser that carries no pass for today.
pub const NO_PASS: &str = "no_pass";

/// The refusal code for a browser whose pass is from an earlier day.
pub const NEW_DAY: &str = "new_day";

/// The refusal code for a visit that is not signed in as one of this
/// browser's people: the account was signed out, or never was.
pub const SIGNED_OUT: &str = "signed_out";

/// The refusal code for a notebook that already holds all the notes a day
/// allows.
pub const NOTES_FULL: &str = "notes_full";

/// The refusal code for a note with nothing in it but space.
pub const EMPTY_NOTE: &str = "empty_note";

/// The refusal code for a note longer than a day allows.
pub const NOTE_TOO_LONG: &str = "note_too_long";

/// One saved note.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Note {
	/// Its id.
	pub id: String,
	/// What the person wrote, exactly. Always shown as text.
	pub text: String,
	/// When it was saved, in Unix milliseconds.
	pub created_at: i64,
}

/// The signed-in person's notebook.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Notebook {
	/// Their notes, newest first.
	pub notes: Vec<Note>,
	/// The most notes a notebook holds today.
	pub limit: usize,
	/// The most characters one note holds.
	pub max_chars: usize,
	/// The pseudonymous actor their events carry in analytics.
	pub actor: String,
}

/// A saved note, and what became of its event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Saved {
	/// The note, as it was stored.
	pub note: Note,
	/// How many words its `note_created` event counted.
	pub words: u32,
	/// `confirmed` when the analytics service took the event, otherwise
	/// `unconfirmed`. The note is saved either way, and nothing resends it.
	pub analytics: String,
	/// The actor the event carried.
	pub actor: String,
}

/// One message in the demo inbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Letter {
	/// The address it was sent to.
	pub to: String,
	/// Its subject, which ends with the code.
	pub subject: String,
	/// Its text.
	pub text: String,
	/// The mailer's receipt for it, which names this one message.
	pub receipt: String,
	/// When it was sent, in Unix milliseconds.
	pub sent_at_ms: i64,
}

/// What the demo inbox holds for one of this browser's people.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Mailbox {
	/// The messages this server still holds, oldest first.
	pub letters: Vec<Letter>,
	/// When the person's latest code was sent, in Unix milliseconds, if this
	/// server no longer holds it.
	pub lost_at_ms: Option<i64>,
}

/// Where a visitor goes once they have a person to be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Minted {
	/// The local sign-in address for the person, carrying the validated
	/// return target.
	pub next: String,
}

/// The demo day a server is serving.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Day {
	/// The generation's UTC day, `YYYY-MM-DD`.
	pub day: String,
}

impl rpc::Resumable for Day {
	type Cursor = String;

	fn cursor(&self) -> String {
		self.day.clone()
	}
}

/// Gives this browser a person to be, or reuses the one it has, and says
/// where to sign in as them.
///
/// Refusals carry a code: [`rpc::RATE_LIMITED`] with how long to wait when a
/// network has asked for too many new people this hour, and [`FULL`] when
/// the day has handed out all it has.
#[rpc::mutation(crate = "leptoskit::rpc")]
pub async fn mint(
	cx: &rpc::MutationCx,
	persona: Persona,
	return_to: String,
) -> rpc::Result<Minted> {
	cx.require::<Port>()?.0.mint(persona, return_to).await
}

/// The mail this server holds for one of this browser's people.
///
/// The pass cookie decides whose mail it is; nothing in the call can name
/// another browser's person. Refused with [`NO_PASS`] when the browser has
/// no pass for today and [`NEW_DAY`] when its pass is from an earlier day.
#[rpc::query(crate = "leptoskit::rpc")]
pub async fn inbox(cx: &rpc::QueryCx, persona: Persona) -> rpc::Result<Mailbox> {
	cx.require::<Port>()?.0.inbox(persona).await
}

/// The signed-in person's notebook.
///
/// The account is the session's and the pass must have minted it; nothing
/// in the call can name another. Refused with [`SIGNED_OUT`], [`NO_PASS`]
/// or [`NEW_DAY`] when the visit is not that.
#[rpc::query(crate = "leptoskit::rpc")]
pub async fn notes(cx: &rpc::QueryCx) -> rpc::Result<Notebook> {
	cx.require::<Port>()?.0.notes().await
}

/// Saves one private note for the signed-in person and sends its
/// `note_created` event once.
///
/// Refused like [`notes`] for a visit that is not signed in, and with
/// [`EMPTY_NOTE`], [`NOTE_TOO_LONG`] or [`NOTES_FULL`] for a note the
/// notebook cannot take. A refusal saves nothing and sends nothing.
#[rpc::mutation(crate = "leptoskit::rpc")]
pub async fn save_note(cx: &rpc::MutationCx, text: String) -> rpc::Result<Saved> {
	cx.require::<Port>()?.0.save_note(text).await
}

/// The day the server is serving, sent once each time the stream opens.
///
/// A page compares it with the day it was made for. The stream stays open
/// and says nothing more; a page that loses it opens it again, and so hears
/// the next day's server name its own day.
#[rpc::stream(resumable, crate = "leptoskit::rpc")]
pub async fn generation(
	cx: &rpc::StreamCx,
	cursor: Option<String>,
) -> rpc::Result<impl futures::Stream<Item = Day>> {
	// The answer is always today's day, wherever the page last got to.
	let _ = cursor;
	cx.require::<Port>()?.0.generation()
}

/// The service's side of the operations, for one request.
#[cfg(feature = "ssr")]
pub trait Backend: Send + Sync + 'static {
	/// Mints or reuses this request's pass and person.
	fn mint(
		&self,
		persona: Persona,
		return_to: String,
	) -> futures::future::BoxFuture<'_, rpc::Result<Minted>>;

	/// The mail held for this request's pass and `persona`.
	fn inbox(&self, persona: Persona) -> futures::future::BoxFuture<'_, rpc::Result<Mailbox>>;

	/// The notebook of the person this request is signed in as.
	fn notes(&self) -> futures::future::BoxFuture<'_, rpc::Result<Notebook>>;

	/// Saves `text` in that notebook.
	fn save_note(&self, text: String) -> futures::future::BoxFuture<'_, rpc::Result<Saved>>;

	/// The serving generation's day, then silence for as long as the
	/// generation serves.
	///
	/// # Errors
	///
	/// When the generation has no room for another watcher.
	fn generation(&self) -> rpc::Result<futures::stream::BoxStream<'static, Day>>;
}

/// The request's [`Backend`], as the operations find it in their context.
#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct Port(pub std::sync::Arc<dyn Backend>);
