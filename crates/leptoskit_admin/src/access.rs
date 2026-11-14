//! Who gets in, and what the admin remembers about them.
//!
//! The admin signs people in through the main site only. Its gate lets an
//! attempt through when the main site released one of the configured roles
//! (`admin` by default) in its `roles` claim, and keeps those roles by the
//! main site's subject, so the admin's own [`Roles`] hook can read them back
//! on every request. A role taken away at the main site applies from the
//! next sign-in.
//!
//! The admin's database holds only its own sessions, those roles, and each
//! person's preferences: Workings mode, and the margin notes they've
//! dismissed or used.

use std::collections::BTreeMap;
use std::path::Path;

use leptoskit_auth::Account;
use leptoskit_auth::AuthError;
use leptoskit_auth::Identity;
use leptoskit_auth::Result as AuthResult;
use leptoskit_auth::Roles;
use leptoskit_auth::SignInAttempt;
use leptoskit_auth::SignInGate;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Timestamp;
use leptoskit_db::Tx;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_db_schema::RequireHints;
use leptoskit_db_schema::Schema;
use serde::Deserialize;
use serde::Serialize;

use crate::AdminError;

/// The provider ID the main site signs in under.
pub(crate) const PROVIDER: &str = "main";

/// How many times a person uses a control before its margin note retires.
pub(crate) const NOTE_RETIRES_AFTER: u32 = 3;

/// How long Workings mode stays on by default for a new admin.
const FIRST_WEEK_SECONDS: i64 = 7 * 24 * 60 * 60;

/// The roles the main site released for someone, by its subject.
#[table(name = "admin_roles")]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct AdminRole {
	pub id: Id<Self>,
	#[db(unique, max_len = 255)]
	pub subject: String,
	/// The roles, comma-separated.
	#[db(max_len = 1000)]
	pub roles: String,
}

/// One admin's preferences.
#[table(name = "admin_people")]
#[policy(select, |_, _| leptoskit_db::Expr::never())]
pub struct AdminPerson {
	pub id: Id<Self>,
	#[db(unique, on_delete = cascade)]
	pub account_id: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
	/// Workings mode, once they've chosen; until then it's on for their
	/// first week.
	pub workings_mode: Option<bool>,
	/// Their margin notes, as JSON: `{"plan-review": {"uses": 2}}`.
	#[db(max_len = 20000)]
	pub notes: String,
}

/// What a person has done with one margin note.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NoteState {
	#[serde(default)]
	pub uses: u32,
	#[serde(default)]
	pub dismissed: bool,
}

impl NoteState {
	/// Whether the note still shows.
	pub(crate) fn showing(&self) -> bool {
		!self.dismissed && self.uses < NOTE_RETIRES_AFTER
	}
}

/// One admin's preferences, read.
#[derive(Clone, Debug, Default)]
pub(crate) struct Preferences {
	pub workings_mode: bool,
	pub notes: BTreeMap<String, NoteState>,
}

impl Preferences {
	/// Whether the note `id` still shows.
	pub(crate) fn note(&self, id: &str) -> bool {
		self.notes.get(id).is_none_or(NoteState::showing)
	}
}

/// A change a person makes to their preferences.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PreferenceChange {
	/// They flipped **Show workings**.
	WorkingsMode { on: bool },
	/// They pressed **Got it**.
	DismissNote { note: String },
	/// They used the control a note explains.
	UseNote { note: String },
}

/// The gate: the main site must release one of `roles`.
pub(crate) struct AdminGate {
	pub roles: Vec<String>,
}

impl SignInGate for AdminGate {
	async fn check(&self, tx: &Tx, attempt: &SignInAttempt<'_>) -> AuthResult<()> {
		if attempt.method != PROVIDER {
			return Err(AuthError::NotAllowed);
		}

		let released = attempt.roles();

		if !released
			.iter()
			.any(|role| self.roles.iter().any(|allowed| allowed == role))
		{
			return Err(AuthError::NotAllowed);
		}

		let subject = attempt.subject.ok_or(AuthError::NotAllowed)?;
		let roles = released.join(",");
		let existing = AdminRole::query()
			.filter(|stored| stored.subject.eq(subject.to_owned()))
			.first(tx)
			.await?;

		match existing {
			Some(stored) => {
				AdminRole::update()
					.filter(|row| row.id.eq(stored.id))
					.set(|row| row.roles, roles)
					.run(tx)
					.await?;
			}
			None => {
				NewAdminRole {
					subject: subject.to_owned(),
					roles,
				}
				.insert(tx)
				.await?;
			}
		}

		Ok(())
	}
}

/// The admin's [`Roles`] hook: what the gate kept for the person's
/// main-site subject.
pub(crate) struct AdminRoles;

impl Roles for AdminRoles {
	async fn roles(db: &Db, account: Id<Account>) -> AuthResult<Vec<String>> {
		let db = db.system("admin: roles from the main site");
		let identity = Identity::query()
			.filter(|identity| {
				identity.account_id.eq(account) & identity.provider.eq(PROVIDER.to_owned())
			})
			.first(&db)
			.await?;
		let Some(identity) = identity else {
			return Ok(Vec::new());
		};
		let stored = AdminRole::query()
			.filter(|stored| stored.subject.eq(identity.subject.clone()))
			.first(&db)
			.await?;

		Ok(stored
			.map(|stored| {
				stored
					.roles
					.split(',')
					.filter(|role| !role.is_empty())
					.map(str::to_owned)
					.collect()
			})
			.unwrap_or_default())
	}
}

/// The name the admin shows for someone: their email at the main site, or
/// their name there, or a short form of their account ID.
pub(crate) async fn display_name(db: &Db, account: Id<Account>) -> Result<String, AdminError> {
	let db = db.system("admin: who is signed in");
	let identity = Identity::query()
		.filter(|identity| {
			identity.account_id.eq(account) & identity.provider.eq(PROVIDER.to_owned())
		})
		.first(&db)
		.await
		.map_err(AdminError::database)?;

	Ok(identity
		.and_then(|identity| identity.email.or(identity.name))
		.unwrap_or_else(|| {
			let id = account.to_string();
			format!("account {}", &id[..id.len().min(8)])
		}))
}

/// Reads someone's preferences.
pub(crate) async fn preferences(db: &Db, account: Id<Account>) -> Result<Preferences, AdminError> {
	let db = db.system("admin: preferences");
	let person = AdminPerson::query()
		.filter(|person| person.account_id.eq(account))
		.first(&db)
		.await
		.map_err(AdminError::database)?;

	let Some(person) = person else {
		return Ok(Preferences {
			workings_mode: true,
			notes: BTreeMap::new(),
		});
	};
	let first_week =
		leptoskit_db::now().timestamp() - person.created_at.timestamp() < FIRST_WEEK_SECONDS;

	Ok(Preferences {
		workings_mode: person.workings_mode.unwrap_or(first_week),
		notes: serde_json::from_str(&person.notes).unwrap_or_default(),
	})
}

/// Makes sure someone has a preferences row, so their first week starts
/// when they first sign in.
pub(crate) async fn welcome(db: &Db, account: Id<Account>) -> Result<(), AdminError> {
	let db = db.system("admin: preferences");
	let existing = AdminPerson::query()
		.filter(|person| person.account_id.eq(account))
		.first(&db)
		.await
		.map_err(AdminError::database)?;

	if existing.is_none() {
		NewAdminPerson {
			account_id: account,
			workings_mode: None,
			notes: "{}".to_owned(),
		}
		.insert(&db)
		.await
		.map_err(AdminError::database)?;
	}

	Ok(())
}

/// Applies a change to someone's preferences.
pub(crate) async fn change(
	db: &Db,
	account: Id<Account>,
	change: PreferenceChange,
) -> Result<Preferences, AdminError> {
	welcome(db, account).await?;
	let mut current = preferences(db, account).await?;
	let system = db.system("admin: preferences");
	let mut workings_mode = None;

	match change {
		PreferenceChange::WorkingsMode { on } => {
			current.workings_mode = on;
			workings_mode = Some(on);
		}
		PreferenceChange::DismissNote { note } => {
			valid_note(&note)?;
			current.notes.entry(note).or_default().dismissed = true;
		}
		PreferenceChange::UseNote { note } => {
			valid_note(&note)?;
			let state = current.notes.entry(note).or_default();
			state.uses = state.uses.saturating_add(1).min(NOTE_RETIRES_AFTER);
		}
	}

	let notes = serde_json::to_string(&current.notes)
		.map_err(|error| AdminError::Database(error.to_string()))?;
	let mut update = AdminPerson::update()
		.filter(|person| person.account_id.eq(account))
		.set(|person| person.notes, notes);

	if let Some(on) = workings_mode {
		update = update.set(|person| person.workings_mode, Some(on));
	}

	update.run(&system).await.map_err(AdminError::database)?;
	Ok(current)
}

fn valid_note(note: &str) -> Result<(), AdminError> {
	if note.is_empty()
		|| note.len() > 64
		|| !note
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
	{
		return Err(AdminError::Invalid(format!("`{note}` isn't a margin note")));
	}

	Ok(())
}

/// Brings the admin's database up to the tables this build links: the
/// auth tables for its sessions, and its own. The schema it last applied is
/// kept beside the database, so an upgrade adds what's new.
///
/// # Errors
///
/// When the change needs a decision (a rename, say) or the database
/// refuses it. The database holds only sessions and preferences, so moving
/// it aside and starting again is always safe.
pub(crate) async fn migrate(db: &Db, snapshot: &Path) -> Result<(), AdminError> {
	let dialect = db.dialect();
	let target = leptoskit_db::schema(dialect)
		.map_err(|error| AdminError::Database(error.to_string()))?
		.without_rename_hints();
	let previous = match std::fs::read(snapshot) {
		Ok(bytes) => {
			serde_json::from_slice::<Schema>(&bytes).map_err(|error| {
				AdminError::Database(format!(
					"{} isn't a schema snapshot ({error}); move the admin's database and it aside \
					 to start again",
					snapshot.display()
				))
			})?
		}
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Schema::empty(dialect),
		Err(error) => return Err(AdminError::Io(error.to_string())),
	};
	let plan =
		leptoskit_db_schema::diff(&previous, &target, &mut RequireHints).map_err(|error| {
			AdminError::Database(format!(
				"the admin's tables changed in a way it can't apply by itself ({error}); its \
				 database holds only sessions and preferences, so move {} and its snapshot aside \
				 to start again",
				snapshot.with_extension("sqlite3").display()
			))
		})?;
	let migration = leptoskit_db_schema::render(&plan, &previous, &target);
	let sql = migration.sql();

	if !sql.trim().is_empty() {
		db.run_script(&sql).await.map_err(AdminError::database)?;
	}

	let json = serde_json::to_vec_pretty(&target)
		.map_err(|error| AdminError::Database(error.to_string()))?;
	std::fs::write(snapshot, json).map_err(|error| AdminError::Io(error.to_string()))
}
