//! The control plane, compiled only with the `server` feature: who is
//! calling ([`CloudIdentify`]), what they may do ([`authorize`]), the
//! operations' bodies, the audit log, and the routes ([`Control`]).
//!
//! # Identity
//!
//! [`CloudIdentify`] checks a request's credentials in this order, and the
//! first that holds decides who is calling:
//!
//! 1. a session: the browser's cookie, or a session token as a bearer;
//! 2. the CLI's access token from the device flow, which `leptoskit_idp`'s
//!    layer has already verified into a delegation for the first-party CLI
//!    client. A delegation for any other client is anonymous;
//! 3. an API token, `lkc_<prefix>_<secret>`, as a bearer.
//!
//! Anything else, including a credential that is unknown, malformed,
//! expired, or revoked, is anonymous, and an anonymous caller reaches no
//! operation: every one answers `Unauthorized`.

pub mod account;
mod audit;
mod authorize;
mod control;
mod identity;
mod personal;
pub mod projects;
mod secret;
pub mod tokens;

use std::sync::Arc;

pub use control::CLI_CLIENT_NAME;
pub use control::CLI_SCOPES;
pub use control::Control;
pub use control::ControlConfig;
pub use control::ControlError;
pub use control::DEFAULT_REGION;
pub use control::DISCOVERY_PATH;
pub use control::DevMode;
pub use control::OAuthApp;
pub use control::RPC_PATH;
pub use control::SharedMailer;
pub use identity::Caller;
pub use identity::ClientDescription;
pub use identity::CloudIdentify;
pub use identity::TokenCaller;
use leptoskit_auth::Account;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;

use crate::tables::NewOperator;
use crate::tables::Operator;

/// What the operations' bodies need beyond the caller's transaction: the
/// database without policies, for reading other people's addresses; the
/// mailer, for invitations; and the public origin, for the links mail
/// carries. [`CloudIdentify`] adds it to every caller's context.
#[derive(Clone)]
pub struct Services {
	inner: Arc<ServicesInner>,
}

struct ServicesInner {
	db: Db,
	mailer: SharedMailer,
	origin: String,
	default_region: String,
}

impl Services {
	/// The services over `db`, sending mail with `mailer`, at `origin`.
	pub fn new(
		db: &Db,
		mailer: SharedMailer,
		origin: impl Into<String>,
		default_region: impl Into<String>,
	) -> Self {
		Self {
			inner: Arc::new(ServicesInner {
				db: db.system("cloud: identity, addresses, and the audit log"),
				mailer,
				origin: origin.into().trim_end_matches('/').to_owned(),
				default_region: default_region.into(),
			}),
		}
	}

	/// The database, with policies bypassed.
	pub(crate) fn db(&self) -> &Db {
		&self.inner.db
	}

	pub(crate) fn mailer(&self) -> &SharedMailer {
		&self.inner.mailer
	}

	pub(crate) fn origin(&self) -> &str {
		&self.inner.origin
	}

	pub(crate) fn default_region(&self) -> &str {
		&self.inner.default_region
	}
}

impl std::fmt::Debug for Services {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Services")
			.field("origin", &self.inner.origin)
			.finish_non_exhaustive()
	}
}

/// The services a caller's context carries.
///
/// # Errors
///
/// [`rpc::Error::Server`] when the server was built without
/// [`CloudIdentify`], which adds them: a wiring mistake, never the caller's.
pub(crate) fn services(context: &leptoskit_db::Context) -> rpc::Result<&Services> {
	context
		.get::<Services>()
		.ok_or_else(|| rpc::Error::server("the control plane's services aren't configured"))
}

/// Makes the person with the verified address `email` an operator. Run
/// from the server's command line, and recorded in the audit log.
///
/// # Errors
///
/// [`OperatorError::UnknownEmail`] when no account has verified it.
pub async fn grant_operator(db: &Db, email: &str) -> Result<Id<Account>, OperatorError> {
	let db = db.system("cloud: operator grant");
	let account = account_by_email(&db, email).await?;
	let existing = Operator::query()
		.filter(|operator| operator.account_id.eq(account))
		.first(&db)
		.await?;

	if existing.is_none() {
		let tx = db.begin().await?;
		NewOperator {
			account_id: account,
		}
		.insert(&tx)
		.await?;
		audit::record_system(&tx, "operator.grant", Some(account.to_string())).await?;
		tx.commit().await?;
	}

	Ok(account)
}

/// Stops the person with the verified address `email` being an operator.
///
/// # Errors
///
/// [`OperatorError::UnknownEmail`] when no account has verified it.
pub async fn revoke_operator(db: &Db, email: &str) -> Result<bool, OperatorError> {
	let db = db.system("cloud: operator revoke");
	let account = account_by_email(&db, email).await?;
	let tx = db.begin().await?;
	let Some(operator) = Operator::query()
		.filter(|operator| operator.account_id.eq(account))
		.first(&tx)
		.await?
	else {
		return Ok(false);
	};
	operator.destroy(&tx).await?;
	audit::record_system(&tx, "operator.revoke", Some(account.to_string())).await?;
	tx.commit().await?;
	Ok(true)
}

/// Every operator's primary verified address, or their account ID when they
/// have none.
///
/// # Errors
///
/// When the database fails.
pub async fn operators(db: &Db) -> Result<Vec<String>, OperatorError> {
	let db = db.system("cloud: operator list");
	let rows = Operator::query()
		.order_by(|operator| operator.created_at.asc())
		.all(&db)
		.await?;
	let mut out = Vec::with_capacity(rows.len());

	for row in rows {
		let email = primary_email(&db, row.account_id).await?;
		out.push(email.unwrap_or_else(|| row.account_id.to_string()));
	}

	Ok(out)
}

/// Why an operator command failed.
#[derive(Debug, thiserror::Error)]
pub enum OperatorError {
	/// No account has verified the address.
	#[error("no account has verified `{0}`; sign in with it first")]
	UnknownEmail(String),
	/// The database failed.
	#[error(transparent)]
	Db(#[from] leptoskit_db::DbError),
}

/// The account that has verified `email`.
async fn account_by_email(db: &Db, email: &str) -> Result<Id<Account>, OperatorError> {
	let normalized = email.trim().to_ascii_lowercase();
	let address = leptoskit_db::Email::parse(&normalized)
		.map_err(|_| OperatorError::UnknownEmail(normalized.clone()))?;
	let row = leptoskit_auth::EmailAddress::query()
		.filter(|row| row.address.eq(address) & row.verified_at.is_not_null())
		.first(db)
		.await?;
	row.map(|row| row.account_id)
		.ok_or(OperatorError::UnknownEmail(normalized))
}

/// The account's primary verified address, or any verified one.
pub(crate) async fn primary_email(
	db: &impl leptoskit_db::Read,
	account: Id<Account>,
) -> leptoskit_db::Result<Option<String>> {
	let rows = leptoskit_auth::EmailAddress::query()
		.filter(|row| row.account_id.eq(account) & row.verified_at.is_not_null())
		.all(db)
		.await?;
	let chosen = rows
		.iter()
		.find(|row| row.is_primary)
		.or_else(|| rows.first());
	Ok(chosen.map(|row| row.address.to_string()))
}

/// Whether `account` is an operator.
pub(crate) async fn is_operator(
	db: &impl leptoskit_db::Read,
	account: Id<Account>,
) -> leptoskit_db::Result<bool> {
	Ok(Operator::query()
		.filter(|operator| operator.account_id.eq(account))
		.first(db)
		.await?
		.is_some())
}
