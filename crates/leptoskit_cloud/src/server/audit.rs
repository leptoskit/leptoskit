//! The audit log: one row for everything that changes an organisation, in
//! the same transaction as the change, so a change is never without its
//! entry. Entries name tokens by ID and invitations by ID and address, and
//! never carry a secret's value.

use leptoskit_auth::Account;
use leptoskit_db::Id;

use super::identity::Caller;
use crate::tables::ActorKind;
use crate::tables::ApiToken;
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
