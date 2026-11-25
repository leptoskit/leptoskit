//! Managed Postgres: the database an environment's app is given, and the
//! one way its address leaves.
//!
//! The control plane decides a database's name and its role's password, and
//! keeps the password sealed in the vault. It never talks to a Postgres
//! server: the cluster's worker makes the database and the role as it
//! applies a release ([`DatabaseStep`]), with the password it is leased.
//!
//! - The name comes from the environment's ID, so no two environments share
//!   one, and a name is never used again.
//! - The password is 24 random bytes, sealed with the organisation's data key
//!   and bound to the organisation, the environment, this row, and its version.
//! - `DATABASE_URL` is put together when a release is claimed, from the
//!   cluster's facts as they are then, and a lease row records that it went.

use leptoskit_cloud_runtime::DatabaseAccess;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_rpc as rpc;

use super::authorize;
use super::config::Lessee;
use super::projects::environment;
use super::secret;
use super::vault;
use super::vault::DatabasePlace;
use super::vault::Vault;
use crate::access::Permission;
use crate::databases::DatabaseSummary;
use crate::operator::ClusterPostgres;
use crate::release::DATABASE_URL;
use crate::release::PlannedDatabase;
use crate::tables::Cluster;
use crate::tables::Database;
use crate::tables::Environment;
use crate::tables::NewDatabase;
use crate::tables::NewSecretLease;
use crate::tables::Organization;
use crate::workers::DatabaseStep;

/// How many random bytes a role's password is made of.
const PASSWORD_BYTES: usize = 24;

/// The name of `environment`'s database and of its role: 36 characters,
/// inside Postgres's 63.
fn name_for(environment: Id<Environment>) -> String {
	format!("env_{}", environment.uuid().simple())
}

/// `environment`'s database, when it has one.
pub(crate) async fn for_environment(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
) -> leptoskit_db::Result<Option<Database>> {
	Database::query()
		.filter(|row| row.environment_id.eq(environment))
		.first(db)
		.await
}

/// Whether any environment keeps a database on `cluster`.
pub(crate) async fn on_cluster(
	db: &impl leptoskit_db::Read,
	cluster: Id<Cluster>,
) -> leptoskit_db::Result<bool> {
	Database::query()
		.filter(|row| row.cluster_id.eq(cluster))
		.exists(db)
		.await
}

fn summary(row: &Database) -> DatabaseSummary {
	DatabaseSummary {
		name: row.name.clone(),
		size: row.settings.0.size,
		backups: row.settings.0.backups,
		ready: row.ready,
		created_at: row.created_at,
	}
}

pub(crate) async fn database(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
) -> rpc::Result<Option<DatabaseSummary>> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment_id = environment(cx.db(), &project, environment_name).await?.id;
	Ok(for_environment(cx.db(), environment_id)
		.await?
		.as_ref()
		.map(summary))
}

/// Makes sure `environment` has a database row for a deployment planned
/// with Postgres: made now, with a new password, when it has none, and
/// otherwise kept, with what the plan asks of it.
///
/// # Errors
///
/// `Conflict` (`vault_busy`) when the organisation's data key is being made
/// by another request; `Server` (`vault_unavailable`) when the vault can't
/// seal.
pub(crate) async fn ensure(
	tx: &impl leptoskit_db::Connection,
	vault: &Vault,
	organization: &Organization,
	environment: &Environment,
	cluster: Id<Cluster>,
	planned: PlannedDatabase,
) -> rpc::Result<()> {
	if let Some(mut row) = for_environment(tx, environment.id).await? {
		if row.settings.0 != planned {
			row.settings = Json(planned);
			row.save(tx).await?;
		}

		return Ok(());
	}

	let key = vault.data_key_for_writing(tx, organization.id).await?;
	let password = secret::random_hex(PASSWORD_BYTES).map_err(|_| vault::unavailable())?;
	// The row first: its ID is part of what the password is sealed to.
	let mut row = NewDatabase {
		environment_id: environment.id,
		organization_id: organization.id,
		cluster_id: cluster,
		name: name_for(environment.id),
		settings: Json(planned),
		credential_version: 1,
		sealed_password: Vec::new(),
		ready: false,
	}
	.insert(tx)
	.await?;
	row.sealed_password = key.seal_password(
		&DatabasePlace {
			organization: organization.id,
			environment: environment.id,
			database: row.id,
			version: row.credential_version,
		},
		&password,
	)?;
	row.save(tx).await?;
	Ok(())
}

/// What a release tells its cluster's worker about `environment`'s
/// database, when it has one.
pub(crate) async fn step(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
) -> leptoskit_db::Result<Option<DatabaseStep>> {
	Ok(for_environment(db, environment).await?.map(|row| {
		DatabaseStep {
			role: row.name.clone(),
			name: row.name,
			size: row.settings.0.size,
			backups: row.settings.0.backups,
		}
	}))
}

/// A release that uses `environment`'s database was applied: the worker
/// has made it.
pub(crate) async fn applied(
	tx: &impl leptoskit_db::Connection,
	environment: Id<Environment>,
) -> leptoskit_db::Result<()> {
	if let Some(mut row) = for_environment(tx, environment).await?
		&& !row.ready
	{
		row.ready = true;
		row.save(tx).await?;
	}

	Ok(())
}

/// A database's address, opened and not yet recorded as leased.
pub(crate) struct Address {
	row: Database,
	url: String,
}

impl std::fmt::Debug for Address {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Address")
			.field("database", &self.row.name)
			.field("url", &"[redacted]")
			.finish()
	}
}

/// The `DATABASE_URL` of a role called `name` on `server`. The name and
/// the password are lower-case hex and `_`, and the host was checked when
/// the facts were set, so nothing needs escaping.
pub(crate) fn url(server: &ClusterPostgres, name: &str, password: &str) -> String {
	format!(
		"postgres://{name}:{password}@{}:{}/{name}?sslmode={}",
		server.host,
		server.port,
		// Said either way, so no client falls back to its own default.
		if server.tls { "require" } else { "disable" }
	)
}

/// The address of `environment`'s database, for a release on `cluster`.
/// Nothing is written: [`leased`] records that it went, once everything
/// else the release starts with has opened too.
///
/// The inner error is why the release can't start, in words for its sheet.
/// It never holds the password.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn address(
	db: &impl leptoskit_db::Read,
	vault: &Vault,
	organization: &Organization,
	environment: &Environment,
	cluster: Id<Cluster>,
	door: Option<&DatabaseAccess>,
) -> rpc::Result<Result<Address, String>> {
	let Some(row) = for_environment(db, environment.id).await? else {
		return Ok(Err(format!(
			"`{}` no longer has its database",
			environment.name
		)));
	};

	// A row whose owners aren't the release's is never opened for it.
	if row.organization_id != organization.id || row.cluster_id != cluster {
		return Ok(Err(format!(
			"`{}`'s database isn't on this release's cluster",
			environment.name
		)));
	}

	let server = Cluster::query()
		.filter(|row| row.id.eq(cluster))
		.one(db)
		.await?
		.facts
		.0
		.postgres;
	let Some(server) = server else {
		return Ok(Err("its cluster no longer says where its Postgres server \
		               is; ask the operator"
			.to_owned()));
	};
	// The spec opens the network to where the server was when the release
	// was built. An address somewhere else would be one nothing can reach.
	if door.is_none_or(|door| door.namespace != server.namespace || door.port != server.port) {
		return Ok(Err("its cluster's Postgres server moved after this \
		               release was built; deploy again"
			.to_owned()));
	}

	let unreadable = || {
		"the vault couldn't open the database's password; ask the operator about the vault's keys"
			.to_owned()
	};
	let key = match vault.data_key(db, organization.id).await {
		Ok(Some(key)) => key,
		Ok(None) => return Ok(Err(unreadable())),
		Err(error) if vault::is_unavailable(&error) => return Ok(Err(unreadable())),
		Err(error) => return Err(error),
	};
	let password = match key.open_password(
		&DatabasePlace {
			organization: organization.id,
			environment: environment.id,
			database: row.id,
			version: row.credential_version,
		},
		&row.sealed_password,
	) {
		Ok(password) => password,
		Err(error) if vault::is_unavailable(&error) => return Ok(Err(unreadable())),
		Err(error) => return Err(error),
	};
	let url = url(&server, &row.name, &password);
	Ok(Ok(Address { row, url }))
}

/// Records that `address` goes to the lessee's worker, in the transaction
/// that leases the job, and gives the value to send.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn leased(
	tx: &impl leptoskit_db::Connection,
	lessee: &Lessee<'_>,
	address: Address,
) -> leptoskit_db::Result<String> {
	NewSecretLease {
		job_id: lessee.job.id,
		attempt: lessee.job.attempt,
		worker_id: lessee.worker,
		organization_id: lessee.organization.id,
		environment_id: lessee.environment.id,
		// The lease table's one ID column: the database's row.
		secret_id: Id::from_uuid(address.row.id.uuid()),
		name: DATABASE_URL.to_owned(),
		version: address.row.credential_version,
	}
	.insert(tx)
	.await?;
	Ok(address.url)
}

#[cfg(test)]
mod tests {
	use uuid::Uuid;

	use super::*;

	#[test]
	fn a_name_is_the_environments_alone_and_fits_postgres() {
		let environment = Id::from_uuid(Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef));
		let name = name_for(environment);
		assert_eq!(name, "env_0123456789abcdef0123456789abcdef");
		assert!(name.len() <= 63);
		assert_ne!(name, name_for(Id::from_uuid(Uuid::from_u128(1))));
	}

	#[test]
	fn an_address_is_a_postgres_url() {
		let mut server = ClusterPostgres {
			namespace: "leptoskit-postgres".to_owned(),
			host: "postgres-rw.leptoskit-postgres.svc".to_owned(),
			port: 5432,
			tls: false,
		};
		assert_eq!(
			url(&server, "env_ab", "c0ffee"),
			"postgres://env_ab:c0ffee@postgres-rw.leptoskit-postgres.svc:5432/env_ab?\
			 sslmode=disable"
		);
		server.tls = true;
		assert!(url(&server, "env_ab", "c0ffee").ends_with("/env_ab?sslmode=require"));
	}
}
