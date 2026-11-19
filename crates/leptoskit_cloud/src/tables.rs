//! The control plane's tables: organisations and who is in them,
//! invitations, API tokens, projects, environments, operators, and the
//! audit log.
//!
//! Every key is a UUID v7, so rows order by creation. People are
//! `Id<Account>`s from `leptoskit_auth`, stored without a foreign key: a
//! membership, a token, or an audit entry outlives nothing it shouldn't when
//! an account goes, because every check reads the membership again, and the
//! audit log must keep naming who did what after they've left.
//!
//! None of these tables is `public`: clients never read them directly, only
//! through the operations, which decide who sees which rows.

// Tables are server-only; client types are plain serde types and operations
// never return rows.
use leptoskit_auth::Account;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Timestamp;
use leptoskit_db::table;

use crate::access::Role;
use crate::access::TokenScope;
use crate::projects::EnvironmentKind;
use crate::projects::EnvironmentScale;

/// An organisation: the tenant everything else belongs to. Each person has
/// one personal organisation, made the first time they use the cloud, and
/// may make or join others.
#[cfg_attr(feature = "server", derive(Clone))]
#[table(name = "organizations")]
pub struct Organization {
	pub id: Id<Self>,
	/// What `/<org>` and `--org` call it, unique across the cloud.
	#[db(unique, min_len = 3, max_len = 40)]
	pub slug: String,
	/// What the console shows.
	#[db(min_len = 1, max_len = 100)]
	pub name: String,
	/// The plan it is on, such as `hobby`: an identifier billing reads,
	/// never a price.
	#[db(min_len = 1, max_len = 40)]
	pub plan: String,
	/// The person whose personal organisation this is. Unique, so a person
	/// has at most one, however many first requests race to make it.
	#[db(unique, no_foreign_key)]
	pub personal_owner: Option<Id<Account>>,
	/// Who made it.
	#[db(no_foreign_key)]
	pub created_by: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Someone in an organisation, with their role. One row per person and
/// organisation.
#[table(
	name = "memberships",
	unique(organization_id, account_id),
	index(account_id)
)]
pub struct Membership {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub organization_id: Id<Organization>,
	#[db(no_foreign_key)]
	pub account_id: Id<Account>,
	pub role: Role,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// An invitation to join an organisation, sent by email. The link's token
/// works once, until it expires; only its SHA-256 is kept.
#[table(name = "invitations", index(organization_id, email))]
pub struct Invitation {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub organization_id: Id<Organization>,
	/// The invited address, trimmed and lowercased. Accepting needs a
	/// verified address equal to it.
	#[db(min_len = 3, max_len = 320)]
	pub email: String,
	/// The role accepting gives.
	pub role: Role,
	/// SHA-256 of the link's token.
	#[db(unique, secret)]
	pub token_hash: Vec<u8>,
	pub expires_at: Timestamp,
	#[db(no_foreign_key)]
	pub invited_by: Id<Account>,
	pub accepted_at: Option<Timestamp>,
	#[db(no_foreign_key)]
	pub accepted_by: Option<Id<Account>>,
	pub revoked_at: Option<Timestamp>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// An API token, `lkc_<prefix>_<secret>`. The prefix finds the row; only
/// the SHA-256 of the secret is kept, and the whole value is shown once,
/// when it is made.
#[table(name = "api_tokens", index(organization_id))]
pub struct ApiToken {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub organization_id: Id<Organization>,
	/// The one project it may touch, when it is bound to one.
	#[db(on_delete = cascade)]
	pub project_id: Option<Id<Project>>,
	#[db(min_len = 1, max_len = 64)]
	pub name: String,
	pub scopes: Json<Vec<TokenScope>>,
	/// The non-secret part of the value, which finds the row.
	#[db(unique, min_len = 12, max_len = 12)]
	pub prefix: String,
	/// SHA-256 of the secret part.
	#[db(secret)]
	pub secret_hash: Vec<u8>,
	pub expires_at: Timestamp,
	/// When it last authenticated a request, to within a few minutes.
	pub last_used_at: Option<Timestamp>,
	pub revoked_at: Option<Timestamp>,
	/// The person it acts for: it never does more than their current role
	/// allows.
	#[db(no_foreign_key)]
	pub created_by: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A project: one app, deployed to one or more environments.
#[table(name = "projects", unique(organization_id, slug))]
pub struct Project {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub organization_id: Id<Organization>,
	/// Unique within its organisation.
	#[db(min_len = 3, max_len = 40)]
	pub slug: String,
	#[db(min_len = 1, max_len = 100)]
	pub name: String,
	#[db(min_len = 2, max_len = 32)]
	pub region: String,
	/// The targets `lk cloud launch` found, such as `server` or `ios`.
	pub detected_targets: Json<Vec<String>>,
	/// The variable names `lk cloud launch` noticed in `.env`, never their
	/// values.
	pub detected_variables: Json<Vec<String>>,
	#[db(no_foreign_key)]
	pub created_by: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// A running copy of a project, such as `production`. Its scale is stored
/// state, changed only with the revision it was read at.
#[table(name = "environments", unique(project_id, name))]
pub struct Environment {
	pub id: Id<Self>,
	#[db(on_delete = cascade)]
	pub project_id: Id<Project>,
	/// Unique within its project.
	#[db(min_len = 3, max_len = 40)]
	pub name: String,
	pub kind: EnvironmentKind,
	pub scale: Json<EnvironmentScale>,
	/// Bumped by every write, so a change based on an older read is a
	/// conflict instead of a lost update.
	#[db(version)]
	pub revision: i64,
	/// Whether the scale was changed here since the last deployment's
	/// `leptoskit.toml`, which the next deployment should be told about.
	pub scale_differs: bool,
	#[db(created_at)]
	pub created_at: Timestamp,
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// Someone who runs the cloud itself. Granted and revoked from the server's
/// command line, never through an operation.
#[table(name = "operators")]
pub struct Operator {
	pub id: Id<Self>,
	#[db(unique, no_foreign_key)]
	pub account_id: Id<Account>,
	#[db(created_at)]
	pub created_at: Timestamp,
}

/// Who acted: a person, an API token, or the server's own command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, leptoskit_db::Enum)]
pub enum ActorKind {
	#[db(rename = "person")]
	Person,
	#[db(rename = "token")]
	Token,
	#[db(rename = "system")]
	System,
}

/// One thing someone did, kept forever: who, what, in which organisation
/// and project, when, and from which client. Never a secret's value: a
/// token is named by its ID and prefix, an invitation by its ID and
/// address. Nothing updates or deletes these rows.
#[table(name = "audit_log", index(organization_id, created_at))]
pub struct AuditEvent {
	pub id: Id<Self>,
	pub actor: ActorKind,
	/// The person, or the person a token acts for.
	#[db(no_foreign_key)]
	pub account_id: Option<Id<Account>>,
	#[db(no_foreign_key)]
	pub token_id: Option<Id<ApiToken>>,
	/// What happened, such as `project.create`.
	#[db(min_len = 1, max_len = 64)]
	pub action: String,
	#[db(no_foreign_key)]
	pub organization_id: Option<Id<Organization>>,
	#[db(no_foreign_key)]
	pub project_id: Option<Id<Project>>,
	/// What it happened to, such as a member's account or a token's ID.
	#[db(max_len = 400)]
	pub target: Option<String>,
	/// Where it came from: the web, the CLI, or an API token.
	#[db(min_len = 1, max_len = 300)]
	pub client: String,
	#[db(created_at)]
	pub created_at: Timestamp,
}
