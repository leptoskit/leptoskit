//! Private notes: the one thing a signed-in visitor writes.
//!
//! The rules, and where each one lives:
//!
//! - **A note belongs to the account that wrote it.** The table's policy lets
//!   `cx.signed_in` read and write only rows whose `account_id` is its own
//!   account, and every operation here runs on a database handle acting for
//!   that account — the ordinary request path, never a system bypass. The
//!   account always comes from the request's session, never from the body.
//! - **The notebook is bounded.** An account keeps at most [`MAX_NOTES`] notes,
//!   counted and inserted in one immediate transaction, so two requests, or two
//!   service processes on one volume, cannot race past the cap. There is no
//!   edit or delete in this slice, so the cap also bounds how many events one
//!   person can cause in a day.
//! - **A note is short text.** [`NoteText`] trims it and requires one to
//!   [`MAX_CHARS`] Unicode scalar values, the same unit the column enforces.

use anyhow::Context as _;
use anyhow::Result;
use leptoskit_auth::Account;
use leptoskit_auth::AuthContext as _;
use leptoskit_auth::SignedIn;
use leptoskit_db::Db;
use leptoskit_db::DbError;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::policy;
use leptoskit_db::table;

/// The most notes one account keeps.
pub const MAX_NOTES: usize = 20;

/// The most Unicode scalar values a note holds, after trimming.
pub const MAX_CHARS: usize = 280;

/// One private note.
#[table(name = "demo_notes", index(account_id))]
#[policy(all, |note, cx| cx.signed_in(|me| note.account_id.eq(me.account)),
	message = "A note belongs to the account that wrote it")]
pub struct DemoNote {
	pub id: Id<Self>,
	/// The account that wrote it. The notes go with the account.
	#[db(on_delete = cascade)]
	pub account_id: Id<Account>,
	/// What the note says: trimmed, 1 to [`MAX_CHARS`] characters.
	#[db(min_len = 1, max_len = 280)]
	pub text: String,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Why a note's text was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextError {
	/// Nothing is left once whitespace is trimmed.
	Empty,
	/// More than [`MAX_CHARS`] characters.
	TooLong,
}

impl TextError {
	/// The stable code the JSON contract uses.
	#[must_use]
	pub const fn code(self) -> &'static str {
		match self {
			Self::Empty => "empty_note",
			Self::TooLong => "note_too_long",
		}
	}
}

/// A note's text that passed validation: trimmed, non-empty, within the
/// limit. The only way into [`Notebook::create`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteText(String);

impl NoteText {
	/// Trims `raw` and checks its length in Unicode scalar values.
	///
	/// # Errors
	///
	/// [`TextError::Empty`] when only whitespace is left,
	/// [`TextError::TooLong`] past [`MAX_CHARS`].
	pub fn parse(raw: &str) -> Result<Self, TextError> {
		let text = raw.trim();

		if text.is_empty() {
			return Err(TextError::Empty);
		}

		if text.chars().count() > MAX_CHARS {
			return Err(TextError::TooLong);
		}

		Ok(Self(text.to_owned()))
	}

	/// The text.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

/// Why a note could not be saved.
#[derive(Debug, thiserror::Error)]
pub enum CreateError {
	/// The account already keeps [`MAX_NOTES`] notes.
	#[error("the notebook is full")]
	Full,
	/// The database failed: an explicit error, never a bypassed cap.
	#[error("the database refused the note: {0}")]
	Database(#[source] anyhow::Error),
}

impl From<DbError> for CreateError {
	fn from(error: DbError) -> Self {
		Self::Database(error.into())
	}
}

/// The site's notes, read and written as the account a request is signed in
/// as.
#[derive(Clone)]
pub struct Notebook {
	db: Db,
}

impl Notebook {
	/// The notebook over the site's database.
	#[must_use]
	pub const fn new(db: Db) -> Self {
		Self { db }
	}

	/// The database acting for `who`: the policy sees their account.
	fn acting(&self, who: &SignedIn) -> Db {
		self.db
			.with_context(leptoskit_auth::context(Some(who.clone())))
	}

	/// `who`'s notes, newest first, at most [`MAX_NOTES`].
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn list(&self, who: &SignedIn) -> Result<Vec<DemoNote>> {
		let db = self.acting(who);
		DemoNote::query()
			.order_by(|note| note.created_at.desc())
			.order_by(|note| note.id.desc())
			.limit(u64::try_from(MAX_NOTES).unwrap_or(u64::MAX))
			.all(&db)
			.await
			.context("reading the notes")
	}

	/// Saves a note for `who`, unless the notebook is full. The count and
	/// the insert share one immediate transaction.
	///
	/// # Errors
	///
	/// [`CreateError::Full`] at the cap, [`CreateError::Database`] when the
	/// database fails; either way nothing is saved.
	pub async fn create(&self, who: &SignedIn, text: &NoteText) -> Result<DemoNote, CreateError> {
		let db = self.acting(who);
		let account = who.account;
		let cap = u64::try_from(MAX_NOTES).unwrap_or(u64::MAX);

		db.transaction(async |tx| {
			let held = DemoNote::query().count(tx).await?;

			if held >= cap {
				return Err(CreateError::Full);
			}

			let note = NewDemoNote {
				account_id: account,
				text: text.as_str().to_owned(),
			}
			.insert(tx)
			.await?;

			Ok(note)
		})
		.await
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn text_is_trimmed_and_bounded_in_scalar_values() {
		assert_eq!(NoteText::parse("  hi  ").expect("fine").as_str(), "hi");
		assert_eq!(NoteText::parse(""), Err(TextError::Empty));
		assert_eq!(NoteText::parse(" \n\t "), Err(TextError::Empty));

		let at_limit = "é".repeat(MAX_CHARS);
		assert!(at_limit.len() > MAX_CHARS, "bytes exceed characters");
		assert!(NoteText::parse(&at_limit).is_ok());
		assert_eq!(
			NoteText::parse(&"é".repeat(MAX_CHARS + 1)),
			Err(TextError::TooLong)
		);

		// Four-byte scalars count once each; a surrogate-pair pretender
		// does not count twice.
		assert!(NoteText::parse(&"😀".repeat(MAX_CHARS)).is_ok());
		assert_eq!(
			NoteText::parse(&"😀".repeat(MAX_CHARS + 1)),
			Err(TextError::TooLong)
		);

		// Trimming happens before counting.
		let padded = format!("  {}  ", "a".repeat(MAX_CHARS));
		assert!(NoteText::parse(&padded).is_ok());
	}

	#[test]
	fn error_codes_are_stable() {
		assert_eq!(TextError::Empty.code(), "empty_note");
		assert_eq!(TextError::TooLong.code(), "note_too_long");
	}
}
