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
pub(crate) mod backups;
pub(crate) mod billing;
pub(crate) mod config;
mod control;
pub(crate) mod databases;
pub(crate) mod deployments;
mod dns;
pub(crate) mod domains;
mod identity;
mod jobs;
mod lifecycle;
pub(crate) mod logs;
pub(crate) mod metering;
pub(crate) mod operator;
mod payments;
mod personal;
pub mod projects;
pub(crate) mod publishing;
mod secret;
pub(crate) mod status;
mod storage;
pub(crate) mod subscriptions;
mod throttle;
pub mod tokens;
pub(crate) mod uploads;
mod vault;
pub(crate) mod workers;

use std::sync::Arc;
use std::time::Duration;

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
pub use control::catalogue_from_json;
pub use dns::DnsOverHttps;
pub use dns::Lookup;
pub use dns::ResolveError;
pub use dns::Resolver;
pub use dns::SharedResolver;
pub use identity::Caller;
pub use identity::ClientDescription;
pub use identity::CloudIdentify;
pub use identity::TokenCaller;
pub use identity::WorkerCaller;
pub use jobs::StoredInput;
pub use jobs::Swept;
use leptoskit_auth::Account;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;
pub use payments::Call;
pub use payments::Checkout;
pub use payments::NewCustomer;
pub use payments::PaymentError;
pub use payments::Payments;
pub use payments::PaymentsConfig;
pub use payments::STRIPE_API_VERSION;
pub use payments::SharedPayments;
pub use payments::Stripe;
pub use payments::Subscription;
pub use payments::signature_header;
pub use storage::S3Config;
pub use storage::SecretValue;
pub use storage::StorageConfig;
pub use subscriptions::STRIPE_WEBHOOK_PATH;
pub use throttle::Rate;
pub use throttle::RateLimits;

use self::storage::ObjectStore;
use crate::pricing::Catalogue;
use crate::tables::NewOperator;
use crate::tables::Operator;

/// What the operations' bodies need beyond the caller's transaction: the
/// database without policies, for reading other people's addresses; the
/// mailer, for invitations; the public origin, for the links mail
/// carries; and what shipping needs ([`Shipping`]). [`CloudIdentify`] adds
/// it to every caller's context.
#[derive(Clone)]
pub struct Services {
	inner: Arc<ServicesInner>,
}

struct ServicesInner {
	db: Db,
	mailer: SharedMailer,
	origin: String,
	default_region: String,
	shipping: Shipping,
}

/// What deployments need: where archives and logs are kept, the domain
/// environments are served under, the registry images go to, the price
/// catalogue, the settings, and the signal that wakes workers waiting to
/// claim a job.
#[derive(Clone, Debug)]
pub(crate) struct Shipping {
	pub(crate) store: ObjectStore,
	pub(crate) apps_domain: String,
	pub(crate) registry: String,
	pub(crate) catalogue: Option<Catalogue>,
	pub(crate) settings: ShippingSettings,
	pub(crate) jobs: Arc<tokio::sync::Notify>,
	/// Values no log may show: the store's secret key.
	pub(crate) redactions: Arc<[String]>,
	/// Where secrets are kept.
	pub(crate) vault: vault::Vault,
	/// What reads a domain's challenge record.
	pub(crate) resolver: SharedResolver,
	/// How payment is taken, when it is.
	pub(crate) payments: Option<PaymentsConfig>,
	/// How often each kind of caller may ask, and the counts so far.
	pub(crate) rates: RateLimits,
	pub(crate) throttle: Arc<throttle::Throttle>,
	/// The header a proxy in front of the control plane puts the client's
	/// address in, when there is one.
	pub(crate) client_ip_header: Option<String>,
}

/// How long leases and addresses last, how long things are kept, and how
/// much an organisation may store. Tests shorten them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShippingSettings {
	/// How long a claim holds a job before the worker must renew it.
	pub lease: Duration,
	/// How long an upload address works.
	pub upload: Duration,
	/// How long a build's download address works.
	pub download: Duration,
	/// How long a build may wait for a worker before it fails with
	/// `builder_unavailable`.
	pub unclaimed_build: Duration,
	/// How often the sweeper looks for expired leases, archives, and logs.
	pub sweep: Duration,
	/// How long an archive no deployment uses is kept.
	pub archive_keep: Duration,
	/// How long a build's logs are kept.
	pub build_logs: Duration,
	/// How long an environment's runtime logs are kept.
	pub runtime_logs: Duration,
	/// The most archive bytes one organisation may keep.
	pub storage_cap: u64,
	/// The longest one attempt at a build may run, from its claim:
	/// renewing never takes its lease past this.
	pub build_deadline: Duration,
	/// The longest one publish may run, from its claim: past a worker's
	/// own limit for the program, two hours by default, and the fetch
	/// before it.
	pub publish_deadline: Duration,
	/// The most log bytes one build may send, over all its attempts. The
	/// bytes are its chunks' as stored, and a build's log also has at most
	/// [`crate::workers::MAX_BUILD_LOG_CHUNKS`] chunks, whatever their bytes.
	pub build_log_cap: u64,
	/// The most build log bytes one organisation may keep at once; they
	/// count until their retention ends.
	pub organization_log_cap: u64,
	/// The most runtime log bytes one organisation may keep at once across
	/// its environments; newer chunks are dropped once full until older
	/// chunks expire, while status and compute metering continue uninterrupted.
	pub organization_runtime_log_cap: u64,
	/// The most bytes one backup's dump may be. A worker is told it, and a
	/// dump a worker says is larger is a backup that failed.
	pub backup_cap: u64,
	/// Whether this replica runs the background sweeper loop. Leave it on for
	/// every replica: on Postgres the sweeper's advisory lock lets only one
	/// replica sweep at a time. `false` is for a replica that should never
	/// sweep.
	pub run_sweeper: bool,
}

impl Default for ShippingSettings {
	/// Two-minute leases, fifteen-minute addresses, an hour for a build to
	/// be claimed, a sweep every fifteen seconds, archives kept a week,
	/// build logs thirty days and runtime logs seven, 5 GiB of archives
	/// per organisation, two hours for an attempt at a build, 64 MiB of log
	/// per build, 2 GiB of build logs per organisation, 512 MiB of runtime
	/// logs per organisation, and 20 GiB for a backup.
	fn default() -> Self {
		Self {
			lease: Duration::from_mins(2),
			upload: Duration::from_mins(15),
			download: Duration::from_mins(15),
			unclaimed_build: Duration::from_hours(1),
			sweep: Duration::from_secs(15),
			archive_keep: Duration::from_hours(7 * 24),
			build_logs: Duration::from_hours(30 * 24),
			runtime_logs: Duration::from_hours(7 * 24),
			storage_cap: 5 * 1024 * 1024 * 1024,
			build_deadline: Duration::from_hours(2),
			publish_deadline: Duration::from_hours(3),
			build_log_cap: 64 * 1024 * 1024,
			organization_log_cap: 2 * 1024 * 1024 * 1024,
			organization_runtime_log_cap: 512 * 1024 * 1024,
			backup_cap: 20 * 1024 * 1024 * 1024,
			run_sweeper: true,
		}
	}
}

impl ShippingSettings {
	/// A lease, in whole seconds, for the wire.
	pub(crate) fn lease_seconds(&self) -> u32 {
		u32::try_from(self.lease.as_secs())
			.unwrap_or(u32::MAX)
			.max(1)
	}
}

/// `duration` as a database interval.
pub(crate) fn delta(duration: Duration) -> chrono::TimeDelta {
	chrono::TimeDelta::from_std(duration).unwrap_or(chrono::TimeDelta::MAX)
}

impl Services {
	/// The services over `db`, sending mail with `mailer`, at `origin`.
	pub(crate) fn new(
		db: &Db,
		mailer: SharedMailer,
		origin: impl Into<String>,
		default_region: impl Into<String>,
		shipping: Shipping,
	) -> Self {
		Self {
			inner: Arc::new(ServicesInner {
				db: db.system("cloud: identity, addresses, and the audit log"),
				mailer,
				origin: origin.into().trim_end_matches('/').to_owned(),
				default_region: default_region.into(),
				shipping,
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

	pub(crate) fn shipping(&self) -> &Shipping {
		&self.inner.shipping
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

/// The longest piece of a caller's own text an error repeats to them.
const MAX_SHOWN: usize = 100;

/// `text` as an error repeats it: its first [`MAX_SHOWN`] bytes, so an
/// answer is never as long as whatever was sent.
pub(crate) fn shown(text: &str) -> &str {
	let mut end = text.len().min(MAX_SHOWN);

	while !text.is_char_boundary(end) {
		end -= 1;
	}

	&text[..end]
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

/// Idempotently creates a verified account for `email` when none exists and
/// makes it an operator. Used during first-time cluster bootstrap (for example
/// via `LEPTOSKIT_CLOUD_BOOTSTRAP_OPERATOR_EMAIL`) so an operator can register
/// clusters and pair workers before interactive email sign-in.
///
/// # Errors
///
/// [`OperatorError::InvalidEmail`] when `email` is not a valid email address,
/// [`OperatorError::DisabledAccount`] when the account for `email` is disabled,
/// or [`OperatorError::Db`] when the database fails.
pub async fn bootstrap_operator(db: &Db, email: &str) -> Result<Id<Account>, OperatorError> {
	let db = db.system("cloud: bootstrap operator");
	let normalized = email.trim().to_ascii_lowercase();
	let address = leptoskit_db::Email::parse(&normalized)
		.map_err(|_| OperatorError::InvalidEmail(normalized.clone()))?;
	let account = leptoskit_auth::ensure_verified_email_account(&db, &address).await?;
	let account_row = Account::query()
		.filter(|row| row.id.eq(account))
		.one(&db)
		.await?;
	if account_row.disabled_at.is_some() {
		return Err(OperatorError::DisabledAccount(normalized));
	}
	let existing = Operator::query()
		.filter(|operator| operator.account_id.eq(account))
		.first(&db)
		.await?;
	if existing.is_none() {
		let tx = db.begin().await?;
		match (NewOperator {
			account_id: account,
		})
		.insert(&tx)
		.await
		{
			Ok(_) => {
				audit::record_system(&tx, "operator.bootstrap", Some(account.to_string())).await?;
				tx.commit().await?;
			}
			Err(leptoskit_db::DbError::Conflict { .. }) => {
				let _ = tx.rollback().await;
			}
			Err(error) => {
				let _ = tx.rollback().await;
				return Err(error.into());
			}
		}
	}
	Ok(account)
}

/// Whether the configured address still needs bootstrapping: until this
/// database has made its first operator, and never again, so a revocation
/// outlives every later start. The audit log is only ever appended to: a
/// grant it already holds, from the command line or from a bootstrap, means
/// the first operator has been.
///
/// # Errors
///
/// [`OperatorError::Db`] when the database fails.
pub async fn bootstrap_needed(db: &Db) -> Result<bool, OperatorError> {
	let db = db.system("cloud: bootstrap check");
	Ok(!audit::ever_recorded(&db, &["operator.bootstrap", "operator.grant"]).await?)
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
	/// The address is not a valid email address.
	#[error("`{0}` is not a valid email address")]
	InvalidEmail(String),
	/// The account for the address is disabled.
	#[error("the account for `{0}` is disabled")]
	DisabledAccount(String),
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
