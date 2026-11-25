//! The audit log: one row for everything that changes an organisation, in
//! the same transaction as the change, so a change is never without its
//! entry. Entries name tokens by ID and invitations by ID and address, and
//! never carry a secret's value.

use leptoskit_auth::Account;
use leptoskit_db::Id;

use super::identity::Caller;
use crate::tables::ActorKind;
use crate::tables::ApiToken;
use crate::tables::AuditEvent;
use crate::tables::NewAuditEvent;
use crate::tables::Organization;
use crate::tables::Project;

/// Who acted.
pub(crate) enum Actor {
	/// A person, in their own name.
	Person(Id<Account>),
	/// An API token, for the person who made it.
	Token {
		token: Id<ApiToken>,
		account: Id<Account>,
	},
}

impl From<&Caller> for Actor {
	fn from(caller: &Caller) -> Self {
		match caller {
			Caller::Person { account, .. } => Self::Person(*account),
			Caller::Token(token) => {
				Self::Token {
					token: token.id,
					account: token.created_by,
				}
			}
		}
	}
}

/// Appends one entry.
///
/// # Errors
///
/// When the write fails, which fails the change it records.
pub(crate) async fn record(
	tx: &impl leptoskit_db::Connection,
	actor: &Actor,
	action: &str,
	organization: Option<Id<Organization>>,
	project: Option<Id<Project>>,
	target: Option<String>,
	client: &str,
) -> leptoskit_db::Result<()> {
	let (kind, account, token) = match actor {
		Actor::Person(account) => (ActorKind::Person, Some(*account), None),
		Actor::Token { token, account } => (ActorKind::Token, Some(*account), Some(*token)),
	};

	NewAuditEvent {
		actor: kind,
		account_id: account,
		token_id: token,
		action: action.to_owned(),
		organization_id: organization,
		project_id: project,
		target,
		client: client.to_owned(),
	}
	.insert(tx)
	.await
	.map(|_| ())
}

/// Appends an entry for something the payment provider said happened to
/// `organization`.
///
/// # Errors
///
/// When the write fails, which fails the change it records.
pub(crate) async fn record_provider(
	tx: &impl leptoskit_db::Connection,
	action: &str,
	organization: Id<Organization>,
	target: Option<String>,
) -> leptoskit_db::Result<()> {
	NewAuditEvent {
		actor: ActorKind::System,
		account_id: None,
		token_id: None,
		action: action.to_owned(),
		organization_id: Some(organization),
		project_id: None,
		target,
		client: "payment-provider".to_owned(),
	}
	.insert(tx)
	.await
	.map(|_| ())
}

/// Appends an entry for the server's own command line.
///
/// # Errors
///
/// When the write fails.
pub(crate) async fn record_system(
	tx: &impl leptoskit_db::Connection,
	action: &str,
	target: Option<String>,
) -> leptoskit_db::Result<()> {
	NewAuditEvent {
		actor: ActorKind::System,
		account_id: None,
		token_id: None,
		action: action.to_owned(),
		organization_id: None,
		project_id: None,
		target,
		client: "leptoskit-cloud-control".to_owned(),
	}
	.insert(tx)
	.await
	.map(|_| ())
}

/// Whether the log has ever recorded any of `actions`. The one question
/// this module answers by reading: every write here is still an insert.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn ever_recorded(
	db: &impl leptoskit_db::Read,
	actions: &[&str],
) -> leptoskit_db::Result<bool> {
	AuditEvent::query()
		.filter(|event| event.action.in_list(actions.iter().copied()))
		.exists(db)
		.await
}

#[cfg(test)]
mod tests {
	use std::path::Path;
	use std::path::PathBuf;

	/// Every `.rs` file under `dir`, relative to `root`, with `/` between
	/// its parts.
	fn sources(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
		for entry in std::fs::read_dir(dir).expect("a source directory") {
			let path = entry.expect("an entry").path();

			if path.is_dir() {
				sources(root, &path, out);
			} else if path.extension().is_some_and(|extension| extension == "rs") {
				let relative = path
					.strip_prefix(root)
					.expect("under the root")
					.components()
					.map(|part| part.as_os_str().to_string_lossy().into_owned())
					.collect::<Vec<_>>()
					.join("/");
				out.push((relative, path));
			}
		}
	}

	/// The audit log is append-only by construction, since the migrations
	/// can't say so: `leptoskit_db` expresses no rule that refuses updates
	/// and deletes on an app's own table in both SQLite and Postgres. So
	/// only this module names the table's rows, it only inserts, nothing
	/// writes raw SQL against the table, and no foreign key can delete an
	/// entry along with what it names.
	#[test]
	fn only_this_module_writes_the_audit_log_and_only_inserts() {
		let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
		let src = crate_root.join("src");
		let mut files = Vec::new();
		sources(&src, &src, &mut files);
		assert!(files.iter().any(|(name, _)| name == "server/audit.rs"));

		for (name, path) in &files {
			let text = std::fs::read_to_string(path).expect("a source file");

			match name.as_str() {
				// Where the table is declared.
				"tables.rs" => {}
				"server/audit.rs" => {
					let code = text
						.split("#[cfg(test)]")
						.next()
						.expect("the module's code");
					// `query(` is allowed for the one read, `ever_recorded`.
					for forbidden in ["update(", "delete(", "destroy(", "save(", "Statement"] {
						assert!(
							!code.contains(forbidden),
							"server/audit.rs calls `{forbidden}`: the audit log is only inserted \
							 into"
						);
					}
				}
				_ => {
					for forbidden in ["AuditEvent", "audit_log"] {
						assert!(
							!text.contains(forbidden),
							"{name} names `{forbidden}`: only server/audit.rs writes the audit \
							 log, and only by inserting"
						);
					}
				}
			}
		}

		let mut created = 0;

		for directory in ["migrations", "migrations-postgres"] {
			for entry in std::fs::read_dir(crate_root.join(directory)).expect("migrations") {
				let sql = entry.expect("an entry").path().join("migration.sql");

				let Ok(sql) = std::fs::read_to_string(&sql) else {
					continue;
				};

				for statement in sql.split(';').map(str::trim) {
					let on_the_log = statement.starts_with("CREATE TABLE \"audit_log\"")
						|| statement.starts_with("ALTER TABLE \"audit_log\"");
					created += usize::from(statement.starts_with("CREATE TABLE \"audit_log\""));
					assert!(
						!(on_the_log && statement.contains("REFERENCES")),
						"{directory}: a foreign key could delete audit entries: {statement}"
					);
				}
			}
		}

		assert_eq!(created, 2, "the audit log's table, once per dialect");
	}
}
