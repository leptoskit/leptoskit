//! Variables and secrets: the operations' bodies, what a release is built
//! with, and the lease that hands a cluster worker a release's secrets.

use std::collections::BTreeMap;

use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use serde::Deserialize;
use serde::Serialize;

use super::audit;
use super::audit::Actor;
use super::authorize;
use super::identity::client;
use super::projects::environment;
use super::services;
use super::vault::Place;
use crate::access::Permission;
use crate::config::MAX_SECRET_VALUE_LEN;
use crate::config::MAX_SECRETS;
use crate::config::MAX_SECRETS_TOTAL_LEN;
use crate::config::MAX_VARIABLE_VALUE_LEN;
use crate::config::MAX_VARIABLES;
use crate::config::MAX_VARIABLES_TOTAL_LEN;
use crate::config::SecretSummary;
use crate::config::Variable;
use crate::config::check_name;
use crate::tables::Environment;
use crate::tables::Job;
use crate::tables::NewSecret;
use crate::tables::NewSecretLease;
use crate::tables::NewVariable;
use crate::tables::Organization;
use crate::tables::Project;
use crate::tables::Secret;
use crate::tables::Worker;

fn invalid_name(error: &str) -> rpc::Error {
	rpc::Error::invalid(format!("invalid_name: {error}"))
}

fn summary(row: &Secret) -> SecretSummary {
	SecretSummary {
		name: row.name.clone(),
		version: u64::try_from(row.version).unwrap_or(0),
		updated_at: row.updated_at,
	}
}

fn variable(row: &crate::tables::Variable) -> Variable {
	Variable {
		name: row.name.clone(),
		value: row.value.clone(),
		updated_at: row.updated_at,
	}
}

async fn variable_named(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
	name: &str,
) -> leptoskit_db::Result<Option<crate::tables::Variable>> {
	crate::tables::Variable::query()
		.filter(|row| row.environment_id.eq(environment) & row.name.eq(name))
		.first(db)
		.await
}

async fn secret_named(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
	name: &str,
) -> leptoskit_db::Result<Option<Secret>> {
	Secret::query()
		.filter(|row| row.environment_id.eq(environment) & row.name.eq(name))
		.first(db)
		.await
}

pub(crate) async fn variables(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
) -> rpc::Result<Vec<Variable>> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment_id = environment(cx.db(), &project, environment_name).await?.id;
	let rows = crate::tables::Variable::query()
		.filter(|row| row.environment_id.eq(environment_id))
		.order_by(|row| row.name.asc())
		.all(cx.db())
		.await?;
	Ok(rows.iter().map(variable).collect())
}

pub(crate) async fn set_variable(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	name: String,
	value: String,
) -> rpc::Result<Variable> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::ManageConfig,
	)
	.await?;
	let environment_id = environment(cx.tx(), &project, environment_name).await?.id;
	check_variable_name(&name).map_err(|error| invalid_name(&error))?;

	if value.len() > MAX_VARIABLE_VALUE_LEN {
		return Err(rpc::Error::invalid(format!(
			"value_too_long: a variable's value is at most {MAX_VARIABLE_VALUE_LEN} bytes"
		)));
	}

	if value.contains('\0') {
		return Err(rpc::Error::invalid(
			"value_not_text: a variable's value can't hold a NUL character",
		));
	}

	if secret_named(cx.tx(), environment_id, &name)
		.await?
		.is_some()
	{
		return Err(rpc::Error::conflict(format!(
			"name_is_secret: `{environment_name}` has a secret `{name}`; remove it first, or \
			 choose another name"
		)));
	}

	let updated_by = access.caller.account();
	let mut rows = crate::tables::Variable::query()
		.filter(|row| row.environment_id.eq(environment_id))
		.all(cx.tx())
		.await?;
	let existing = rows
		.iter()
		.position(|row| row.name == name)
		.map(|at| rows.swap_remove(at));
	let total = rows
		.iter()
		.map(|row| row.name.len() + row.value.len())
		.sum::<usize>()
		+ name.len()
		+ value.len();

	if total > MAX_VARIABLES_TOTAL_LEN {
		return Err(rpc::Error::conflict(format!(
			"variables_too_large: the variables of `{environment_name}` would take more than \
			 {MAX_VARIABLES_TOTAL_LEN} bytes together; remove or shorten some first"
		)));
	}

	let row = if let Some(mut row) = existing {
		row.value = value;
		row.updated_by = updated_by;
		row.save(cx.tx()).await?;
		row
	} else {
		if rows.len() >= MAX_VARIABLES {
			return Err(rpc::Error::conflict(format!(
				"too_many_variables: `{environment_name}` has {MAX_VARIABLES} variables; remove \
				 some first"
			)));
		}

		NewVariable {
			environment_id,
			name,
			value,
			updated_by,
		}
		.insert(cx.tx())
		.await?
	};
	if crate::notifications::is_setting(&row.name) {
		super::notifications::changed(cx.tx(), environment_id).await?;
	}

	// The name only: a value, even a variable's, isn't the audit log's to
	// keep.
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"variable.set",
		Some(access.organization.id),
		Some(project.id),
		Some(format!("{environment_name} {}", row.name)),
		&client(cx.context()),
	)
	.await?;
	Ok(variable(&row))
}

pub(crate) async fn unset_variable(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	name: &str,
) -> rpc::Result<()> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::ManageConfig,
	)
	.await?;
	let environment_id = environment(cx.tx(), &project, environment_name).await?.id;
	// A name that isn't one may be a value sent by mistake: the answer
	// never repeats it.
	check_variable_name(name).map_err(|error| invalid_name(&error))?;
	let Some(mut row) = variable_named(cx.tx(), environment_id, name).await? else {
		return Err(rpc::Error::not_found(format!(
			"variable_not_found: `{environment_name}` has no variable `{name}`"
		)));
	};
	row.delete(cx.tx()).await?;

	if crate::notifications::is_setting(name) {
		super::notifications::changed(cx.tx(), environment_id).await?;
	}

	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"variable.unset",
		Some(access.organization.id),
		Some(project.id),
		Some(format!("{environment_name} {name}")),
		&client(cx.context()),
	)
	.await?;
	Ok(())
}

pub(crate) async fn secrets(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
) -> rpc::Result<Vec<SecretSummary>> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment_id = environment(cx.db(), &project, environment_name).await?.id;
	let rows = Secret::query()
		.filter(|row| row.environment_id.eq(environment_id))
		.order_by(|row| row.name.asc())
		.all(cx.db())
		.await?;
	Ok(rows.iter().map(summary).collect())
}

/// `set_secret`'s body. It is an action so that nothing keeps its
/// arguments: a mutation called with an idempotency key leaves a receipt
/// holding a SHA-256 of them, which for a short secret is as good as the
/// secret. The writes still happen in one serializable transaction, with
/// the mutation runner's retries, through [`StoreSecret`], which is never
/// registered and so never runs with a request ID.
pub(crate) async fn set_secret(
	cx: &rpc::ActionCx,
	organization: String,
	project: String,
	environment: String,
	name: String,
	value: String,
) -> rpc::Result<SecretSummary> {
	cx.mutate(StoreSecret {
		organization,
		project,
		environment,
		name,
		value,
	})
	.await
}

/// The authorisation, the sealed value, and its audit entry, in one
/// transaction. Only [`set_secret`] runs it: it isn't served, so no client
/// calls it, and it never has a receipt. Its `Debug` never shows the value.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StoreSecret {
	organization: String,
	project: String,
	environment: String,
	name: String,
	value: String,
}

impl std::fmt::Debug for StoreSecret {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("StoreSecret")
			.field("organization", &self.organization)
			.field("project", &self.project)
			.field("environment", &self.environment)
			.field("value", &"[redacted]")
			.finish_non_exhaustive()
	}
}

impl rpc::Operation for StoreSecret {
	type Context = rpc::MutationCx;
	type Output = SecretSummary;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	/// What the change log records the commit as.
	const NAME: &'static str = "set_secret";

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<SecretSummary>> + Send {
		store(cx, self)
	}
}

async fn store(cx: &rpc::MutationCx, args: StoreSecret) -> rpc::Result<SecretSummary> {
	let StoreSecret {
		organization,
		project,
		environment: environment_name,
		name,
		value,
	} = args;
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		&organization,
		&project,
		Permission::ManageConfig,
	)
	.await?;
	let environment_id = environment(cx.tx(), &project, &environment_name).await?.id;
	check_secret_name(&name).map_err(|error| invalid_name(&error))?;

	if value.is_empty() {
		return Err(rpc::Error::invalid(
			"value_empty: a secret needs a value; to remove it, unset it",
		));
	}

	if value.len() > MAX_SECRET_VALUE_LEN {
		return Err(rpc::Error::invalid(format!(
			"value_too_long: a secret's value is at most {MAX_SECRET_VALUE_LEN} bytes"
		)));
	}

	if value.contains('\0') {
		return Err(rpc::Error::invalid(
			"value_not_text: a secret's value can't hold a NUL character",
		));
	}

	if variable_named(cx.tx(), environment_id, &name)
		.await?
		.is_some()
	{
		return Err(rpc::Error::conflict(format!(
			"name_is_variable: `{environment_name}` has a variable `{name}`; remove it first, or \
			 choose another name"
		)));
	}

	let vault = &services(cx.context())?.shipping().vault;
	let key = vault
		.data_key_for_writing(cx.tx(), access.organization.id)
		.await?;
	let updated_by = access.caller.account();
	let mut rows = Secret::query()
		.filter(|row| row.environment_id.eq(environment_id))
		.all(cx.tx())
		.await?;
	let mut row = if let Some(at) = rows.iter().position(|row| row.name == name) {
		rows.swap_remove(at)
	} else {
		if rows.len() >= MAX_SECRETS {
			return Err(rpc::Error::conflict(format!(
				"too_many_secrets: `{environment_name}` has {MAX_SECRETS} secrets; remove some \
				 first"
			)));
		}

		// The row first, without a value: the value is sealed to the row's
		// own ID, so a name set again after it was removed never shares a
		// place with what it held before. A refusal below takes the row
		// back with the transaction.
		NewSecret {
			environment_id,
			name,
			version: 0,
			sealed: Vec::new(),
			updated_by,
		}
		.insert(cx.tx())
		.await?
	};
	let version = row.version + 1;
	row.sealed = key.seal(
		&Place {
			organization: access.organization.id,
			project: project.id,
			environment: environment_id,
			secret: row.id,
			name: &row.name,
			version,
		},
		&value,
	)?;
	let total = rows
		.iter()
		.chain(std::iter::once(&row))
		.map(|row| row.name.len() + row.sealed.len())
		.sum::<usize>();

	if total > MAX_SECRETS_TOTAL_LEN {
		return Err(rpc::Error::conflict(format!(
			"secrets_too_large: the secrets of `{environment_name}` would take more than \
			 {MAX_SECRETS_TOTAL_LEN} bytes together; remove or shorten some first"
		)));
	}

	row.version = version;
	row.updated_by = updated_by;
	row.save(cx.tx()).await?;

	if crate::notifications::is_setting(&row.name) {
		super::notifications::changed(cx.tx(), environment_id).await?;
	}

	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"secret.set",
		Some(access.organization.id),
		Some(project.id),
		Some(format!(
			"{environment_name} {} version {}",
			row.name, row.version
		)),
		&client(cx.context()),
	)
	.await?;
	Ok(summary(&row))
}

pub(crate) async fn unset_secret(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	name: &str,
) -> rpc::Result<()> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::ManageConfig,
	)
	.await?;
	let environment = environment(cx.tx(), &project, environment_name).await?;
	let environment_id = environment.id;
	check_secret_name(name).map_err(|error| invalid_name(&error))?;
	// A deployment in flight may already name this secret in the release it
	// is about to apply, which would then have nothing to start with.
	super::deployments::settled(cx.tx(), &environment).await?;
	let Some(mut row) = secret_named(cx.tx(), environment_id, name).await? else {
		return Err(rpc::Error::not_found(format!(
			"secret_not_found: `{environment_name}` has no secret `{name}`"
		)));
	};
	let version = row.version;
	row.delete(cx.tx()).await?;

	if crate::notifications::is_setting(name) {
		super::notifications::changed(cx.tx(), environment_id).await?;
	}

	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"secret.unset",
		Some(access.organization.id),
		Some(project.id),
		Some(format!("{environment_name} {name} version {version}")),
		&client(cx.context()),
	)
	.await?;
	Ok(())
}

/// Checks that `name` can name a variable: one the cloud doesn't set, or
/// one of the hosted notifications service's settings, which only its hub
/// reads.
fn check_variable_name(name: &str) -> Result<(), String> {
	if crate::notifications::SECRET_SETTINGS.contains(&name) {
		return Err(format!(
			"`{name}` is a private key: set it as a secret, which nobody can read back"
		));
	}

	if crate::notifications::is_setting(name) {
		return Ok(());
	}

	check_name(name)
}

/// Checks that `name` can name a secret: as a variable's name, or one of
/// the store credentials a publish carries, which no release is given.
fn check_secret_name(name: &str) -> Result<(), String> {
	if crate::workers::is_publish_credential(name) || crate::notifications::is_setting(name) {
		return Ok(());
	}

	check_name(name)
}

/// What a release of `environment` starts its processes with: the
/// variables and their values, and the names of the secrets. A store
/// credential is a publish's, and a notifications setting its cluster's
/// hub's: never the app's.
pub(crate) async fn for_release(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
) -> leptoskit_db::Result<(BTreeMap<String, String>, Vec<String>)> {
	let variables = crate::tables::Variable::query()
		.filter(|row| row.environment_id.eq(environment))
		.all(db)
		.await?
		.into_iter()
		.filter(|row| !crate::notifications::is_setting(&row.name))
		.map(|row| (row.name, row.value))
		.collect();
	let mut secrets: Vec<String> = Secret::query()
		.filter(|row| row.environment_id.eq(environment))
		.all(db)
		.await?
		.into_iter()
		.map(|row| row.name)
		.filter(|name| {
			!crate::workers::is_publish_credential(name) && !crate::notifications::is_setting(name)
		})
		.collect();
	secrets.sort_unstable();
	Ok((variables, secrets))
}

/// The environment's secrets that `names` asks for which it doesn't have.
pub(crate) async fn missing(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
	names: impl IntoIterator<Item = &String>,
) -> leptoskit_db::Result<Vec<String>> {
	let have: Vec<String> = Secret::query()
		.filter(|row| row.environment_id.eq(environment))
		.all(db)
		.await?
		.into_iter()
		.map(|row| row.name)
		.collect();
	let mut missing: Vec<String> = names
		.into_iter()
		.filter(|name| !have.contains(name))
		.cloned()
		.collect();
	missing.sort_unstable();
	missing.dedup();
	Ok(missing)
}

/// Who a lease is for.
pub(crate) struct Lessee<'a> {
	pub(crate) job: &'a Job,
	pub(crate) worker: Id<Worker>,
	pub(crate) organization: &'a Organization,
	pub(crate) project: &'a Project,
	pub(crate) environment: &'a Environment,
}

/// The values leased to `job`'s current attempt that can still be opened,
/// for hiding them in what its worker reports: a value since replaced or
/// removed, or one the vault can no longer open, is left out. Never sent
/// anywhere.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn leased_values(
	tx: &impl leptoskit_db::Connection,
	services: &super::Services,
	job: &Job,
) -> rpc::Result<Vec<String>> {
	let job_id = job.id;
	let attempt = job.attempt;
	let leases = crate::tables::SecretLease::query()
		.filter(|lease| lease.job_id.eq(job_id) & lease.attempt.eq(attempt))
		.all(tx)
		.await?;
	let Some(first) = leases.first() else {
		return Ok(Vec::new());
	};
	let Some(environment) = Environment::query()
		.filter(|row| row.id.eq(first.environment_id))
		.first(tx)
		.await?
	else {
		return Ok(Vec::new());
	};
	let Ok(Some(key)) = services
		.shipping()
		.vault
		.data_key(tx, first.organization_id)
		.await
	else {
		return Ok(Vec::new());
	};
	let mut values = Vec::new();

	for lease in &leases {
		if let Some(value) = super::notifications::leased_value(tx, &key, lease).await? {
			values.push(value);
			continue;
		}

		let secret_id = lease.secret_id;
		let version = lease.version;
		let Some(row) = Secret::query()
			.filter(|row| row.id.eq(secret_id) & row.version.eq(version))
			.first(tx)
			.await?
		else {
			continue;
		};

		if let Ok(value) = key.open(
			&Place {
				organization: lease.organization_id,
				project: environment.project_id,
				environment: environment.id,
				secret: row.id,
				name: &row.name,
				version: row.version,
			},
			&row.sealed,
		) {
			values.push(value);
		}
	}

	Ok(values)
}

/// Why a lease gave nothing.
#[derive(Debug)]
pub(crate) enum LeaseError {
	/// The release names secrets the environment no longer has.
	Missing(Vec<String>),
	/// The vault couldn't open a secret: its key is gone, or the row isn't
	/// what was sealed there.
	Unreadable,
	/// The database failed.
	Failed(rpc::Error),
}

impl From<leptoskit_db::DbError> for LeaseError {
	fn from(error: leptoskit_db::DbError) -> Self {
		Self::Failed(error.into())
	}
}

impl From<rpc::Error> for LeaseError {
	fn from(error: rpc::Error) -> Self {
		if super::vault::is_unavailable(&error) {
			Self::Unreadable
		} else {
			Self::Failed(error)
		}
	}
}

/// The values of `names`, the secrets a release of the lessee's environment
/// asks for, for its job's worker and nobody else. A row per secret says
/// which version went where, written in the caller's transaction, which is
/// the one that leases the job: no value leaves unless that commits.
///
/// # Errors
///
/// [`LeaseError::Missing`] when the environment no longer has one of them,
/// and [`LeaseError::Unreadable`] when the vault can't open one. The caller
/// must then leave the transaction's lease rows unwritten or abandon the
/// job: no value was returned.
pub(crate) async fn lease(
	tx: &impl leptoskit_db::Connection,
	vault: &super::vault::Vault,
	lessee: &Lessee<'_>,
	names: &[String],
) -> Result<BTreeMap<String, String>, LeaseError> {
	if names.is_empty() {
		return Ok(BTreeMap::new());
	}

	let environment_id = lessee.environment.id;
	let rows = Secret::query()
		.filter(|row| row.environment_id.eq(environment_id))
		.all(tx)
		.await?;
	let mut missing: Vec<String> = names
		.iter()
		.filter(|name| !rows.iter().any(|row| &row.name == *name))
		.cloned()
		.collect();

	if !missing.is_empty() {
		missing.sort_unstable();
		return Err(LeaseError::Missing(missing));
	}

	let Some(key) = vault.data_key(tx, lessee.organization.id).await? else {
		// Secrets without the key they were sealed with.
		return Err(LeaseError::Unreadable);
	};
	// Every value is opened before any row is written: a lease row says a
	// value went to the worker, and none goes unless all of them open.
	let wanted: Vec<&Secret> = rows
		.iter()
		.filter(|row| names.contains(&row.name))
		.collect();
	let mut values = BTreeMap::new();

	for row in &wanted {
		let value = key.open(
			&Place {
				organization: lessee.organization.id,
				project: lessee.project.id,
				environment: environment_id,
				secret: row.id,
				name: &row.name,
				version: row.version,
			},
			&row.sealed,
		)?;
		values.insert(row.name.clone(), value);
	}

	for row in wanted {
		NewSecretLease {
			job_id: lessee.job.id,
			attempt: lessee.job.attempt,
			worker_id: lessee.worker,
			organization_id: lessee.organization.id,
			environment_id,
			secret_id: row.id,
			name: row.name.clone(),
			version: row.version,
		}
		.insert(tx)
		.await?;
	}

	Ok(values)
}
