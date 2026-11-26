//! Hosted notifications: each environment's tenant on its cluster's shared
//! hub, and the one way a tenant's values leave.
//!
//! The control plane decides a tenant's database, its role's password, and
//! the token the app and the hub share, and keeps both sealed in the vault.
//! It never talks to the hub or the tenant's database:
//!
//! - the cluster's worker makes the database with a release that uses it
//!   ([`step`]), with the password it is leased under [`DATABASE_URL_LEASE`];
//! - the app's process is leased the token with that release, under
//!   [`TOKEN_ENV`];
//! - the hub asks for its cluster's tenants ([`tenants`]) and is answered each
//!   changed one's values, each answer recorded in `notification_leases` before
//!   it is sent.
//!
//! A tenant is served while a live release uses it ([`released`]) and
//! until its environment is deleted ([`removing`]). Anything the hub is
//! given for it that changes, a setting or a domain, raises its version
//! ([`changed`]), and the hub reloads it.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use leptoskit_cloud_runtime::ReleaseSpec;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_rpc as rpc;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::authorize;
use super::config::Lessee;
use super::identity::worker;
use super::projects::environment;
use super::secret;
use super::services;
use super::vault;
use super::vault::Place;
use super::vault::TenantPlace;
use super::vault::TenantValue;
use super::vault::Unwrapped;
use super::vault::Vault;
use super::workers::live_worker;
use crate::access::Permission;
use crate::notifications::DATABASE_URL_LEASE;
use crate::notifications::HubState;
use crate::notifications::KnownTenant;
use crate::notifications::MAX_CURSOR_LEN;
use crate::notifications::MAX_KNOWN_TENANTS;
use crate::notifications::MAX_PUSHES_PER_REPORT;
use crate::notifications::MAX_REASON_LEN;
use crate::notifications::MAX_REPORTS;
use crate::notifications::MAX_TENANTS_PER_ANSWER;
use crate::notifications::NextSecrets;
use crate::notifications::NotificationsReported;
use crate::notifications::NotificationsSummary;
use crate::notifications::PLATFORM_SERVICE;
use crate::notifications::TOKEN_ENV;
use crate::notifications::TenantChanges;
use crate::notifications::TenantConfig;
use crate::notifications::TenantReport;
use crate::notifications::TenantSecrets;
use crate::notifications::TenantState;
use crate::notifications::URL_ENV;
use crate::notifications::UsageReport;
use crate::notifications::is_setting;
use crate::notifications::providers;
use crate::notifications::tenant_path;
use crate::operator::ClusterNotifications;
use crate::pricing::Meter;
use crate::size::Backups;
use crate::size::DatabaseSize;
use crate::tables::Cluster;
use crate::tables::Environment;
use crate::tables::NewNotificationLease;
use crate::tables::NewNotificationTenant;
use crate::tables::NewSecretLease;
use crate::tables::NotificationTenant;
use crate::tables::Organization;
use crate::tables::Placement;
use crate::tables::Project;
use crate::tables::Secret;
use crate::tables::UsageTotal;
use crate::tables::Variable;
use crate::workers::DatabaseStep;
use crate::workers::RuntimeValue;
use crate::workers::WorkerRole;

/// How many random bytes a tenant's role's password is made of.
const PASSWORD_BYTES: usize = 24;

/// How many random bytes a tenant's token is made of.
const TOKEN_BYTES: usize = 32;

/// What [`HubState::Serving`] is kept as.
const SERVING: &str = "serving";

/// What [`HubState::Failed`] is kept as.
const FAILED: &str = "failed";

/// The name of `environment`'s tenant's database and of its role: 36
/// characters, inside Postgres's 63, and never an app database's
/// (`env_…`).
fn name_for(environment: Id<Environment>) -> String {
	format!("ntf_{}", environment.uuid().simple())
}

/// `environment`'s tenant, when it has one.
pub(crate) async fn for_environment(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
) -> leptoskit_db::Result<Option<NotificationTenant>> {
	NotificationTenant::query()
		.filter(|row| row.environment_id.eq(environment))
		.first(db)
		.await
}

/// Whether any environment is a tenant of `cluster`'s hub.
pub(crate) async fn on_cluster(
	db: &impl leptoskit_db::Read,
	cluster: Id<Cluster>,
) -> leptoskit_db::Result<bool> {
	NotificationTenant::query()
		.filter(|row| row.cluster_id.eq(cluster))
		.exists(db)
		.await
}

/// How `row` is doing, as people are told.
fn state(row: &NotificationTenant) -> TenantState {
	if row.removing {
		return TenantState::Removing;
	}

	if !row.ready {
		return TenantState::Pending;
	}

	if !row.serving {
		return TenantState::Unused;
	}

	if row.reported_version != Some(row.version) {
		return TenantState::Waiting;
	}

	match row.hub_state.as_deref() {
		Some(SERVING) => TenantState::Serving,
		Some(FAILED) => {
			TenantState::Failed {
				reason: row.hub_reason.clone().unwrap_or_default(),
			}
		}
		_ => TenantState::Waiting,
	}
}

/// The [`SETTINGS`](crate::notifications::SETTINGS) `environment` has, as
/// variables or secrets, by name.
async fn setting_names(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
) -> leptoskit_db::Result<Vec<String>> {
	let mut names: Vec<String> = Variable::query()
		.filter(|row| row.environment_id.eq(environment))
		.all(db)
		.await?
		.into_iter()
		.map(|row| row.name)
		.chain(
			Secret::query()
				.filter(|row| row.environment_id.eq(environment))
				.all(db)
				.await?
				.into_iter()
				.map(|row| row.name),
		)
		.filter(|name| is_setting(name))
		.collect();
	names.sort_unstable();
	names.dedup();
	Ok(names)
}

pub(crate) async fn summary(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
) -> rpc::Result<Option<NotificationsSummary>> {
	let (access, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment_id = environment(cx.db(), &project, environment_name).await?.id;
	let Some(row) = for_environment(cx.db(), environment_id).await? else {
		return Ok(None);
	};
	let settings = setting_names(cx.db(), environment_id).await?;
	let organization_id = access.organization.id;
	let project_id = project.id;
	let month = super::metering::period(leptoskit_db::now());
	let code = Meter::NotificationPushes.code();
	let pushes = UsageTotal::query()
		.filter(|usage| {
			usage.organization_id.eq(organization_id)
				& usage.project_id.eq(project_id)
				& usage.period.eq(month.as_str())
				& usage.meter.eq(code)
		})
		.first(cx.db())
		.await?
		.map_or(0, |usage| u64::try_from(usage.quantity).unwrap_or(0));
	let pushes_paused = super::billing::pushes_stopped(
		cx.db(),
		services(cx.context())?.shipping(),
		&BTreeSet::from([organization_id]),
	)
	.await?
	.contains(&organization_id);
	Ok(Some(NotificationsSummary {
		tenant: row.id.uuid(),
		database: row.database.clone(),
		state: state(&row),
		reported_at: row.reported_at,
		providers: providers(settings.iter().map(String::as_str)),
		settings,
		project_pushes_this_month: pushes,
		pushes_paused,
		rotating: row.next_version.is_some(),
		created_at: row.created_at,
	}))
}

/// Makes sure `environment` is a tenant of `cluster`'s hub, for a
/// deployment planned with hosted notifications: made now, with a new
/// password and token, when it isn't, and otherwise kept as it is.
///
/// # Errors
///
/// `Conflict` (`vault_busy`) when the organisation's data key is being made
/// by another request; `Server` (`vault_unavailable`) when the vault can't
/// seal; `Conflict` (`notifications_moved`) when the tenant is on another
/// cluster.
pub(crate) async fn ensure(
	tx: &impl leptoskit_db::Connection,
	vault: &Vault,
	organization: &Organization,
	project: &Project,
	environment: &Environment,
	cluster: Id<Cluster>,
) -> rpc::Result<()> {
	if let Some(row) = for_environment(tx, environment.id).await? {
		// An environment stays on its cluster, and its tenant with it.
		if row.cluster_id != cluster {
			return Err(rpc::Error::conflict(format!(
				"notifications_moved: `{}`'s notifications are on another cluster than its \
				 deployments; ask the operator",
				environment.name
			)));
		}

		return Ok(());
	}

	let key = vault.data_key_for_writing(tx, organization.id).await?;
	let password = secret::random_hex(PASSWORD_BYTES).map_err(|_| vault::unavailable())?;
	let token = secret::random_hex(TOKEN_BYTES).map_err(|_| vault::unavailable())?;
	// The row first: its ID is part of what each value is sealed to.
	let mut row = NewNotificationTenant {
		environment_id: environment.id,
		organization_id: organization.id,
		project_id: project.id,
		cluster_id: cluster,
		database: name_for(environment.id),
		credential_version: 1,
		sealed_password: Vec::new(),
		sealed_token: Vec::new(),
		next_version: None,
		next_sealed_password: None,
		next_sealed_token: None,
		ready: false,
		serving: false,
		removing: false,
		version: 1,
		hub_state: None,
		hub_reason: None,
		reported_version: None,
		reported_at: None,
		usage_cursor: None,
	}
	.insert(tx)
	.await?;
	let place = place(&row);
	row.sealed_password = key.seal_tenant(&place, TenantValue::Password, &password)?;
	row.sealed_token = key.seal_tenant(&place, TenantValue::Token, &token)?;
	row.save(tx).await?;
	Ok(())
}

fn place(row: &NotificationTenant) -> TenantPlace {
	place_at(row, row.credential_version)
}

fn place_at(row: &NotificationTenant, version: i64) -> TenantPlace {
	TenantPlace {
		organization: row.organization_id,
		environment: row.environment_id,
		tenant: row.id,
		version,
	}
}

/// A tenant's sealed password and token at one credential version: its
/// current ones, or those replacing them.
struct Sealed<'a> {
	version: i64,
	password: &'a [u8],
	token: &'a [u8],
}

fn current(row: &NotificationTenant) -> Sealed<'_> {
	Sealed {
		version: row.credential_version,
		password: &row.sealed_password,
		token: &row.sealed_token,
	}
}

/// The values replacing `row`'s, while they are replaced.
fn next(row: &NotificationTenant) -> Option<Sealed<'_>> {
	match (
		row.next_version,
		&row.next_sealed_password,
		&row.next_sealed_token,
	) {
		(Some(version), Some(password), Some(token)) => {
			Some(Sealed {
				version,
				password,
				token,
			})
		}
		_ => None,
	}
}

/// One tenant [`rotate`] reached.
pub(crate) struct Rotated {
	pub(crate) organization: Id<Organization>,
	pub(crate) project: Id<Project>,
	pub(crate) tenant: Uuid,
	/// Whether it has new values: `false` when its organisation's data key
	/// couldn't be had, which is said, and leaves it on its old ones.
	pub(crate) done: bool,
}

/// A worker that was given tenants' values was revoked: every tenant on
/// `cluster` gets a new password and token to move to, replacing any it
/// was already moving to, as the revoked worker may have had those too.
/// The hub takes both until a release that leased the new ones is
/// applied (see [`released`]). A tenant whose organisation's data key
/// can't be had is skipped, so one organisation's vault never keeps a
/// worker from being revoked.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn rotate(
	tx: &impl leptoskit_db::Connection,
	vault: &Vault,
	cluster: Id<Cluster>,
) -> rpc::Result<Vec<Rotated>> {
	let rows = NotificationTenant::query()
		.filter(|row| row.cluster_id.eq(cluster) & row.removing.eq(false))
		.order_by(|row| row.id.asc())
		.all(tx)
		.await?;
	let mut rotated = Vec::with_capacity(rows.len());

	for mut row in rows {
		// Never a version anything was sealed to before.
		let version = row
			.next_version
			.unwrap_or(row.credential_version)
			.max(row.credential_version)
			.saturating_add(1);
		let place = place_at(&row, version);
		let sealed = async {
			let key = vault.data_key_for_writing(tx, row.organization_id).await?;
			let password = secret::random_hex(PASSWORD_BYTES).map_err(|_| vault::unavailable())?;
			let token = secret::random_hex(TOKEN_BYTES).map_err(|_| vault::unavailable())?;
			Ok::<_, rpc::Error>((
				key.seal_tenant(&place, TenantValue::Password, &password)?,
				key.seal_tenant(&place, TenantValue::Token, &token)?,
			))
		}
		.await;
		let done = if let Ok((password, token)) = sealed {
			row.next_sealed_password = Some(password);
			row.next_sealed_token = Some(token);
			row.next_version = Some(version);
			row.version = row.version.saturating_add(1);
			row.save(tx).await?;
			true
		} else {
			false
		};
		rotated.push(Rotated {
			organization: row.organization_id,
			project: row.project_id,
			tenant: row.id.uuid(),
			done,
		});
	}

	Ok(rotated)
}

/// What a release tells its cluster's worker about `environment`'s
/// tenant's database, when it has one.
pub(crate) async fn step(
	db: &impl leptoskit_db::Read,
	environment: Id<Environment>,
) -> leptoskit_db::Result<Option<DatabaseStep>> {
	Ok(for_environment(db, environment).await?.map(|row| {
		DatabaseStep {
			role: row.database.clone(),
			name: row.database,
			size: DatabaseSize::Shared,
			backups: Backups::Daily,
		}
	}))
}

/// A release of `environment` was applied: when it uses hosted
/// notifications its tenant's database exists and the hub serves it, and
/// when it doesn't the hub stops.
///
/// # Errors
///
/// When the database fails.
///
/// When `job`, the release's, leased the values the tenant is moving to,
/// the app and its database now have them: they become the tenant's, and
/// the old ones are given to no one again.
pub(crate) async fn released(
	tx: &impl leptoskit_db::Connection,
	environment: Id<Environment>,
	uses: bool,
	job: &crate::tables::Job,
) -> leptoskit_db::Result<()> {
	let Some(mut row) = for_environment(tx, environment).await? else {
		return Ok(());
	};
	let ready = row.ready || uses;
	let mut changed = row.ready != ready || row.serving != uses;
	row.ready = ready;
	row.serving = uses;

	if let Some(version) = row.next_version
		&& uses
		&& leased_at(tx, job, &row, version).await?
		&& let (Some(password), Some(token)) = (
			row.next_sealed_password.take(),
			row.next_sealed_token.take(),
		) {
		row.credential_version = version;
		row.sealed_password = password;
		row.sealed_token = token;
		row.next_version = None;
		row.version = row.version.saturating_add(1);
		changed = true;
	}

	if changed {
		row.save(tx).await?;
	}

	Ok(())
}

/// A release `job` ended, however it did. When it leased a tenant's new
/// values, its database may now take only the new password: the hub is
/// told to open it again, which tries both.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn release_ended(
	tx: &impl leptoskit_db::Connection,
	job: &crate::tables::Job,
) -> leptoskit_db::Result<()> {
	let job_id = job.id;
	let attempt = job.attempt;
	let leases = crate::tables::SecretLease::query()
		.filter(|lease| {
			lease.job_id.eq(job_id) & lease.attempt.eq(attempt) & lease.name.eq(TOKEN_ENV)
		})
		.all(tx)
		.await?;

	for lease in leases {
		let tenant: Id<NotificationTenant> = Id::from_uuid(lease.secret_id.uuid());

		if let Some(mut row) = NotificationTenant::query()
			.filter(|row| row.id.eq(tenant))
			.first(tx)
			.await?
			&& row.next_version == Some(lease.version)
		{
			row.version = row.version.saturating_add(1);
			row.save(tx).await?;
		}
	}

	Ok(())
}

/// Whether `job`'s current attempt leased `row`'s token at `version`.
async fn leased_at(
	db: &impl leptoskit_db::Read,
	job: &crate::tables::Job,
	row: &NotificationTenant,
	version: i64,
) -> leptoskit_db::Result<bool> {
	let job_id = job.id;
	let attempt = job.attempt;
	let tenant = Id::from_uuid(row.id.uuid());
	Ok(crate::tables::SecretLease::query()
		.filter(|lease| {
			lease.job_id.eq(job_id)
				& lease.attempt.eq(attempt)
				& lease.secret_id.eq(tenant)
				& lease.name.eq(TOKEN_ENV)
				& lease.version.eq(version)
		})
		.first(db)
		.await?
		.is_some())
}

/// Something the hub is given for `environment`'s tenant changed: a
/// setting, or its hostnames. The hub reloads it.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn changed(
	tx: &impl leptoskit_db::Connection,
	environment: Id<Environment>,
) -> leptoskit_db::Result<()> {
	if let Some(mut row) = for_environment(tx, environment).await?
		&& !row.removing
	{
		row.version = row.version.saturating_add(1);
		row.save(tx).await?;
	}

	Ok(())
}

/// `environment` is being deleted: the hub drops its tenant, and the
/// removal drops its database, by the name this gives.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn removing(
	tx: &impl leptoskit_db::Connection,
	environment: Id<Environment>,
) -> leptoskit_db::Result<Option<String>> {
	let Some(mut row) = for_environment(tx, environment).await? else {
		return Ok(None);
	};

	if !row.removing {
		row.removing = true;
		row.serving = false;
		row.version = row.version.saturating_add(1);
		row.save(tx).await?;
	}

	Ok(Some(row.database))
}

/// Points `spec`'s notifications at `hub` and `tenant`: its platform
/// service and the app's address of it. A release recorded before the hub
/// moved is rolled back to where it is now. Its routes name the bridge in
/// the environment's namespace, which doesn't move.
pub(crate) fn point(spec: &mut ReleaseSpec, hub: &ClusterNotifications, tenant: Uuid) {
	for platform in &mut spec.platform {
		if platform.name == PLATFORM_SERVICE {
			platform.namespace.clone_from(&hub.namespace);
			platform.host.clone_from(&hub.host);
			platform.port = hub.port;
		}
	}

	for process in &mut spec.processes {
		if let Some(url) = process.env.get_mut(URL_ENV) {
			*url = format!("http://{}:{}{}", hub.host, hub.port, tenant_path(tenant));
		}
	}
}

/// What a rollback or a rescale of a release that uses hosted
/// notifications needs: its tenant's database step, where the hub is now,
/// and the tenant.
///
/// # Errors
///
/// `Conflict` (`rollback_unavailable`) when the environment no longer has
/// its tenant, or the cluster no longer has its hub or Postgres server.
pub(crate) async fn for_rollback(
	db: &impl leptoskit_db::Read,
	number: i64,
	environment: &Environment,
	cluster: Id<Cluster>,
) -> rpc::Result<(DatabaseStep, ClusterNotifications, Uuid)> {
	let unavailable = |why: &str| {
		rpc::Error::conflict(format!(
			"rollback_unavailable: #{number} uses hosted notifications, and {why}"
		))
	};
	let Some(row) = for_environment(db, environment.id).await? else {
		return Err(unavailable(&format!(
			"`{}` is no longer a tenant",
			environment.name
		)));
	};

	if row.removing || row.cluster_id != cluster {
		return Err(unavailable("its tenant isn't on this cluster"));
	}

	let facts = Cluster::query()
		.filter(|row| row.id.eq(cluster))
		.one(db)
		.await?
		.facts
		.0;
	let (Some(hub), Some(_)) = (facts.notifications, facts.postgres) else {
		return Err(unavailable(
			"its cluster no longer has a notifications service",
		));
	};
	let tenant = row.id.uuid();
	let Some(step) = step(db, environment.id).await? else {
		return Err(unavailable("its tenant is gone"));
	};
	Ok((step, hub, tenant))
}

/// A tenant's values for a release, opened and not yet recorded as leased.
pub(crate) struct Credentials {
	row: NotificationTenant,
	/// The credential version they are: the tenant's, or the one it is
	/// moving to.
	version: i64,
	url: String,
	token: String,
}

impl std::fmt::Debug for Credentials {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Credentials")
			.field("database", &self.row.database)
			.field("version", &self.version)
			.field("url", &"[redacted]")
			.field("token", &"[redacted]")
			.finish()
	}
}

/// Why a tenant's values can't be opened, in words for a release's sheet
/// or the hub's report. Never holds a value.
type Unopened = String;

/// The tenant's database address and token, `sealed` (its current ones or
/// those replacing them), opened with `key`.
async fn open(
	db: &impl leptoskit_db::Read,
	key: &Unwrapped,
	row: &NotificationTenant,
	sealed: &Sealed<'_>,
	cluster: Id<Cluster>,
) -> rpc::Result<Result<(String, String), Unopened>> {
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
	let unreadable = || {
		"the vault couldn't open its notifications credentials; ask the operator about the vault's \
		 keys"
			.to_owned()
	};
	let place = place_at(row, sealed.version);
	let password = match key.open_tenant(&place, TenantValue::Password, sealed.password) {
		Ok(password) => password,
		Err(error) if vault::is_unavailable(&error) => return Ok(Err(unreadable())),
		Err(error) => return Err(error),
	};
	let token = match key.open_tenant(&place, TenantValue::Token, sealed.token) {
		Ok(token) => token,
		Err(error) if vault::is_unavailable(&error) => return Ok(Err(unreadable())),
		Err(error) => return Err(error),
	};
	Ok(Ok((
		super::databases::url(&server, &row.database, &password),
		token,
	)))
}

/// The data key of `organization`, or why it can't be opened.
async fn data_key(
	db: &impl leptoskit_db::Read,
	vault: &Vault,
	organization: Id<Organization>,
) -> rpc::Result<Result<Unwrapped, Unopened>> {
	let unreadable = || {
		"the vault couldn't open its notifications credentials; ask the operator about the vault's \
		 keys"
			.to_owned()
	};

	match vault.data_key(db, organization).await {
		Ok(Some(key)) => Ok(Ok(key)),
		Ok(None) => Ok(Err(unreadable())),
		Err(error) if vault::is_unavailable(&error) => Ok(Err(unreadable())),
		Err(error) => Err(error),
	}
}

/// The values a release of `environment` on `cluster` starts with for its
/// hosted notifications, when `spec` uses them. Nothing is written:
/// [`leased`] records that they went.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn credentials(
	db: &impl leptoskit_db::Read,
	vault: &Vault,
	organization: &Organization,
	environment: &Environment,
	cluster: Id<Cluster>,
	spec: &ReleaseSpec,
) -> rpc::Result<Result<Credentials, Unopened>> {
	let Some(row) = for_environment(db, environment.id).await? else {
		return Ok(Err(format!(
			"`{}` no longer has its notifications",
			environment.name
		)));
	};

	// A row whose owners aren't the release's is never opened for it.
	if row.organization_id != organization.id || row.cluster_id != cluster || row.removing {
		return Ok(Err(format!(
			"`{}`'s notifications aren't on this release's cluster",
			environment.name
		)));
	}

	// The spec opens the network to where the hub was when the release was
	// built: an address somewhere else would be one nothing can reach.
	let hub = Cluster::query()
		.filter(|row| row.id.eq(cluster))
		.one(db)
		.await?
		.facts
		.0
		.notifications;
	let built_for = spec
		.platform
		.iter()
		.find(|platform| platform.name == PLATFORM_SERVICE);
	let moved = match (&hub, built_for) {
		(Some(hub), Some(built_for)) => {
			hub.namespace != built_for.namespace
				|| hub.host != built_for.host
				|| hub.port != built_for.port
		}
		_ => true,
	};

	if moved {
		return Ok(Err("its cluster's notifications service moved after this \
		               release was built; deploy again"
			.to_owned()));
	}

	let key = match data_key(db, vault, organization.id).await? {
		Ok(key) => key,
		Err(why) => return Ok(Err(why)),
	};

	// A release takes the values the tenant is moving to, when it is: once
	// applied, the app and its database have them, and the old ones go.
	let sealed = next(&row).unwrap_or_else(|| current(&row));
	let version = sealed.version;
	let opened = open(db, &key, &row, &sealed, cluster).await?;
	Ok(match opened {
		Ok((url, token)) => {
			Ok(Credentials {
				row,
				version,
				url,
				token,
			})
		}
		Err(why) => Err(why),
	})
}

/// Records that `credentials` go to the lessee's worker, in the transaction
/// that leases the job, and gives the values to send, by the names the
/// release reads them under.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn leased(
	tx: &impl leptoskit_db::Connection,
	lessee: &Lessee<'_>,
	credentials: Credentials,
) -> leptoskit_db::Result<BTreeMap<String, String>> {
	let Credentials {
		row,
		version,
		url,
		token,
	} = credentials;

	for name in [TOKEN_ENV, DATABASE_URL_LEASE] {
		NewSecretLease {
			job_id: lessee.job.id,
			attempt: lessee.job.attempt,
			worker_id: lessee.worker,
			organization_id: lessee.organization.id,
			environment_id: lessee.environment.id,
			// The lease table's one ID column: the tenant's row.
			secret_id: Id::from_uuid(row.id.uuid()),
			name: name.to_owned(),
			version,
		}
		.insert(tx)
		.await?;
	}

	Ok(BTreeMap::from([
		(TOKEN_ENV.to_owned(), token),
		(DATABASE_URL_LEASE.to_owned(), url),
	]))
}

/// The value a lease of a tenant's token or database address gave, for
/// hiding it in what the worker reports: the token, or the role's
/// password. `None` for any other lease, or one that no longer opens.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn leased_value(
	db: &impl leptoskit_db::Read,
	key: &Unwrapped,
	lease: &crate::tables::SecretLease,
) -> leptoskit_db::Result<Option<String>> {
	let which = match lease.name.as_str() {
		TOKEN_ENV => TenantValue::Token,
		DATABASE_URL_LEASE => TenantValue::Password,
		_ => return Ok(None),
	};
	let tenant: Id<NotificationTenant> = Id::from_uuid(lease.secret_id.uuid());
	let Some(row) = NotificationTenant::query()
		.filter(|row| row.id.eq(tenant))
		.first(db)
		.await?
	else {
		return Ok(None);
	};
	let Some(sealed) = std::iter::once(current(&row))
		.chain(next(&row))
		.find(|sealed| sealed.version == lease.version)
	else {
		return Ok(None);
	};
	let value = match which {
		TenantValue::Token => sealed.token,
		TenantValue::Password => sealed.password,
	};
	Ok(key
		.open_tenant(&place_at(&row, sealed.version), which, value)
		.ok())
}

/// The calling worker, when it is a notifications worker that is still
/// paired, and its cluster.
async fn hub(
	db: &impl leptoskit_db::Read,
	context: &leptoskit_db::Context,
) -> rpc::Result<(Id<crate::tables::Worker>, Option<Id<Cluster>>)> {
	let caller = worker(context)?;

	if caller.role != WorkerRole::Notifications {
		return Err(rpc::Error::forbidden(
			"notifications_worker_required: only a cluster's notifications worker serves its \
			 tenants",
		));
	}

	let row = live_worker(db, caller).await?;
	Ok((row.id, caller.cluster))
}

/// The hub asks what changed. Only [`tenants`] runs it: it isn't served, so
/// no client calls it, and it never has a receipt.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct AskTenants {
	known: Vec<KnownTenant>,
}

impl rpc::Operation for AskTenants {
	type Context = rpc::MutationCx;
	type Output = TenantChanges;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	/// What the change log records the commit as.
	const NAME: &'static str = "notification_tenants";

	fn run(self, cx: &rpc::MutationCx) -> impl Future<Output = rpc::Result<TenantChanges>> + Send {
		answer(cx, self.known)
	}
}

pub(crate) async fn tenants(
	cx: &rpc::ActionCx,
	known: Vec<KnownTenant>,
) -> rpc::Result<TenantChanges> {
	if known.len() > MAX_KNOWN_TENANTS {
		return Err(rpc::Error::invalid(format!(
			"too_many_tenants: a hub says at most {MAX_KNOWN_TENANTS} tenants it holds"
		)));
	}

	cx.mutate(AskTenants { known }).await
}

async fn answer(cx: &rpc::MutationCx, known: Vec<KnownTenant>) -> rpc::Result<TenantChanges> {
	let tx = cx.tx();
	let (worker_id, cluster) = hub(tx, cx.context()).await?;
	let held: BTreeMap<Uuid, i64> = known
		.iter()
		.map(|tenant| (tenant.tenant, tenant.version))
		.collect();
	let Some(cluster) = cluster else {
		return Ok(TenantChanges {
			changed: Vec::new(),
			removed: held.into_keys().collect(),
			more: false,
			paused: Vec::new(),
		});
	};
	let rows = NotificationTenant::query()
		.filter(|row| {
			row.cluster_id.eq(cluster)
				& row.ready.eq(true)
				& row.serving.eq(true)
				& row.removing.eq(false)
		})
		.order_by(|row| row.id.asc())
		.all(tx)
		.await?;
	let served: BTreeSet<Uuid> = rows.iter().map(|row| row.id.uuid()).collect();
	let mut removed: Vec<Uuid> = held
		.keys()
		.filter(|tenant| !served.contains(*tenant))
		.copied()
		.collect();
	let services = services(cx.context())?;
	let vault = &services.shipping().vault;
	let stopped = super::billing::pushes_stopped(
		tx,
		services.shipping(),
		&rows
			.iter()
			.map(|row| row.organization_id)
			.collect::<BTreeSet<_>>(),
	)
	.await?;
	let paused: Vec<Uuid> = rows
		.iter()
		.filter(|row| stopped.contains(&row.organization_id))
		.map(|row| row.id.uuid())
		.collect();
	let mut changed = Vec::new();
	let mut more = false;
	// Tenants whose values were opened this answer, given or not: one that
	// fails counts too, so a cluster of failing tenants can't make each ask
	// open them all.
	let mut opened = 0;

	for mut row in rows {
		if held.get(&row.id.uuid()) == Some(&row.version) {
			continue;
		}

		if opened == MAX_TENANTS_PER_ANSWER {
			more = true;
			break;
		}

		opened += 1;

		match config(tx, vault, &row, cluster).await? {
			Ok((config, names)) => {
				NewNotificationLease {
					tenant_id: row.id,
					worker_id,
					organization_id: row.organization_id,
					environment_id: row.environment_id,
					version: row.version,
					names: Json(names),
				}
				.insert(tx)
				.await?;
				changed.push(config);
			}
			// The control plane can't give it, so the hub can't serve it:
			// said on the tenant as the hub would say it. A hub holding an
			// older version drops it rather than serve what changed since,
			// such as a domain it no longer has.
			Err(why) => {
				if held.contains_key(&row.id.uuid()) {
					removed.push(row.id.uuid());
				}

				let reason = Some(why);

				if row.hub_state.as_deref() != Some(FAILED)
					|| row.hub_reason != reason
					|| row.reported_version != Some(row.version)
				{
					row.hub_state = Some(FAILED.to_owned());
					row.hub_reason = reason;
					row.reported_version = Some(row.version);
					row.reported_at = Some(leptoskit_db::now());
					row.save(tx).await?;
				}
			}
		}
	}

	Ok(TenantChanges {
		changed,
		removed,
		more,
		paused,
	})
}

/// Everything the hub is given for `row`, and the names of the values in
/// it, for the lease.
async fn config(
	tx: &impl leptoskit_db::Connection,
	vault: &Vault,
	row: &NotificationTenant,
	cluster: Id<Cluster>,
) -> rpc::Result<Result<(TenantConfig, Vec<String>), Unopened>> {
	let environment_id = row.environment_id;
	let Some(environment) = Environment::query()
		.filter(|environment| environment.id.eq(environment_id))
		.first(tx)
		.await?
	else {
		return Ok(Err("its environment is gone".to_owned()));
	};
	let project_id = environment.project_id;
	let project = Project::query()
		.filter(|project| project.id.eq(project_id))
		.one(tx)
		.await?;
	let organization_id = row.organization_id;
	let organization = Organization::query()
		.filter(|organization| organization.id.eq(organization_id))
		.one(tx)
		.await?;

	if project.organization_id != organization.id {
		return Ok(Err("its environment isn't its organisation's".to_owned()));
	}

	let Some(placement) = Placement::query()
		.filter(|placement| placement.environment_id.eq(environment_id))
		.first(tx)
		.await?
	else {
		return Ok(Err("its environment has no hostname".to_owned()));
	};

	if placement.cluster_id != cluster {
		return Ok(Err("its environment runs on another cluster".to_owned()));
	}

	let key = match data_key(tx, vault, organization.id).await? {
		Ok(key) => key,
		Err(why) => return Ok(Err(why)),
	};
	let (database_url, token) = match open(tx, &key, row, &current(row), cluster).await? {
		Ok(opened) => opened,
		Err(why) => return Ok(Err(why)),
	};
	let next = match next(row) {
		Some(sealed) => {
			match open(tx, &key, row, &sealed, cluster).await? {
				Ok((database_url, token)) => {
					Some(NextSecrets {
						database_url,
						token,
					})
				}
				Err(why) => return Ok(Err(why)),
			}
		}
		None => None,
	};
	let mut settings: BTreeMap<String, String> = Variable::query()
		.filter(|variable| variable.environment_id.eq(environment_id))
		.all(tx)
		.await?
		.into_iter()
		.filter(|variable| is_setting(&variable.name))
		.map(|variable| (variable.name, variable.value))
		.collect();

	for secret in Secret::query()
		.filter(|secret| secret.environment_id.eq(environment_id))
		.all(tx)
		.await?
	{
		if !is_setting(&secret.name) {
			continue;
		}

		let value = match key.open(
			&Place {
				organization: organization.id,
				project: project.id,
				environment: environment_id,
				secret: secret.id,
				name: &secret.name,
				version: secret.version,
			},
			&secret.sealed,
		) {
			Ok(value) => value,
			Err(error) if vault::is_unavailable(&error) => {
				return Ok(Err(format!(
					"the vault couldn't open `{}`; set it again, or ask the operator about the \
					 vault's keys",
					secret.name
				)));
			}
			Err(error) => return Err(error),
		};
		settings.insert(secret.name, value);
	}

	let domains = super::domains::for_release(tx, environment_id).await?;
	let origin = domains
		.is_empty()
		.then(|| format!("https://{}", placement.hostname));
	let hostnames: Vec<String> = std::iter::once(placement.hostname).chain(domains).collect();
	let mut names = vec![DATABASE_URL_LEASE.to_owned(), TOKEN_ENV.to_owned()];

	if next.is_some() {
		names.extend([
			format!("{DATABASE_URL_LEASE} (next)"),
			format!("{TOKEN_ENV} (next)"),
		]);
	}

	names.extend(settings.keys().cloned());
	Ok(Ok((
		TenantConfig {
			tenant: row.id.uuid(),
			version: row.version,
			environment: RuntimeValue(super::deployments::environment_ref(
				&organization,
				&project,
				&environment,
			)),
			hostnames,
			origin,
			usage_cursor: row.usage_cursor.clone(),
			secrets: TenantSecrets {
				database_url,
				token,
				settings,
				next,
			},
		},
		names,
	)))
}

/// The hub's reports. Only [`report`] runs it.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Report {
	tenants: Vec<TenantReport>,
	usage: Vec<UsageReport>,
}

impl rpc::Operation for Report {
	type Context = rpc::MutationCx;
	type Output = NotificationsReported;

	const KIND: rpc::Kind = rpc::Kind::Mutation;
	/// What the change log records the commit as.
	const NAME: &'static str = "report_notifications";

	fn run(
		self,
		cx: &rpc::MutationCx,
	) -> impl Future<Output = rpc::Result<NotificationsReported>> + Send {
		reported(cx, self)
	}
}

/// Checks a report is no bigger than any real one.
fn check_report(tenants: &[TenantReport], usage: &[UsageReport]) -> rpc::Result<()> {
	if tenants.len() > MAX_REPORTS || usage.len() > MAX_REPORTS {
		return Err(rpc::Error::invalid(format!(
			"too_many_reports: a report holds at most {MAX_REPORTS} tenants and {MAX_REPORTS} \
			 usage counts"
		)));
	}

	for report in tenants {
		if let HubState::Failed { reason } = &report.state
			&& reason.chars().count() > MAX_REASON_LEN
		{
			return Err(rpc::Error::invalid(format!(
				"reason_too_long: a reason is at most {MAX_REASON_LEN} characters"
			)));
		}
	}

	for count in usage {
		let too_long = |cursor: &str| cursor.is_empty() || cursor.chars().count() > MAX_CURSOR_LEN;

		if too_long(&count.until) || count.after.as_deref().is_some_and(too_long) {
			return Err(rpc::Error::invalid(format!(
				"invalid_cursor: a cursor has 1 to {MAX_CURSOR_LEN} characters"
			)));
		}

		if count.after.as_deref() == Some(count.until.as_str()) {
			return Err(rpc::Error::invalid(
				"invalid_cursor: a count ends somewhere other than where it starts",
			));
		}

		if count.pushes > MAX_PUSHES_PER_REPORT {
			return Err(rpc::Error::invalid(format!(
				"too_many_pushes: one count is at most {MAX_PUSHES_PER_REPORT} pushes"
			)));
		}
	}

	Ok(())
}

pub(crate) async fn report(
	cx: &rpc::ActionCx,
	tenants: Vec<TenantReport>,
	usage: Vec<UsageReport>,
) -> rpc::Result<NotificationsReported> {
	check_report(&tenants, &usage)?;
	cx.mutate(Report { tenants, usage }).await
}

async fn reported(cx: &rpc::MutationCx, report: Report) -> rpc::Result<NotificationsReported> {
	let Report { tenants, usage } = report;
	check_report(&tenants, &usage)?;
	let tx = cx.tx();
	let services = services(cx.context())?;
	let (_, cluster) = hub(tx, cx.context()).await?;
	let mut out = NotificationsReported {
		tenants: 0,
		cursors: BTreeMap::new(),
	};
	let Some(cluster) = cluster else {
		return Ok(out);
	};
	let named: BTreeSet<Uuid> = tenants
		.iter()
		.map(|report| report.tenant)
		.chain(usage.iter().map(|count| count.tenant))
		.collect();
	// Read once and changed in place, so two reports of one tenant in a
	// call see each other.
	let mut rows: BTreeMap<Uuid, NotificationTenant> = BTreeMap::new();

	for tenant in named {
		let id: Id<NotificationTenant> = Id::from_uuid(tenant);

		if let Some(row) = NotificationTenant::query()
			.filter(|row| row.id.eq(id) & row.cluster_id.eq(cluster))
			.first(tx)
			.await?
		{
			rows.insert(tenant, row);
		}
	}

	let now = leptoskit_db::now();
	let mut dirty = BTreeSet::new();

	for report in tenants {
		let Some(row) = rows.get_mut(&report.tenant) else {
			continue;
		};

		// Only about the version it is now: an older one's word is stale.
		if row.version != report.version {
			continue;
		}

		let (state, reason) = match report.state {
			HubState::Serving => (SERVING, None),
			HubState::Failed { reason } => {
				(FAILED, Some(scrubbed(tx, services, row, &reason).await?))
			}
		};
		row.hub_state = Some(state.to_owned());
		row.hub_reason = reason;
		row.reported_version = Some(report.version);
		row.reported_at = Some(now);
		dirty.insert(report.tenant);
		out.tenants += 1;
	}

	for count in usage {
		let Some(row) = rows.get_mut(&count.tenant) else {
			continue;
		};

		// Counted once: only from where the last counted one ended.
		if row.usage_cursor == count.after {
			super::metering::record(
				tx,
				row.organization_id,
				row.project_id,
				Meter::NotificationPushes,
				count.pushes,
			)
			.await?;
			row.usage_cursor = Some(count.until);
			dirty.insert(count.tenant);
		}

		out.cursors.insert(count.tenant, row.usage_cursor.clone());
	}

	for tenant in dirty {
		if let Some(mut row) = rows.remove(&tenant) {
			row.save(tx).await?;
		}
	}

	Ok(out)
}

/// The tenant's values, sealed: its role's password, its token, and the
/// settings kept as secrets. `None` when any of them doesn't open.
async fn sealed_values(
	db: &impl leptoskit_db::Read,
	key: &Unwrapped,
	row: &NotificationTenant,
) -> leptoskit_db::Result<Option<Vec<String>>> {
	let place = place(row);
	let mut values = Vec::new();

	for (which, sealed) in [
		(TenantValue::Password, &row.sealed_password),
		(TenantValue::Token, &row.sealed_token),
	] {
		let Ok(value) = key.open_tenant(&place, which, sealed) else {
			return Ok(None);
		};
		values.push(value);
	}

	let environment = row.environment_id;

	for secret in Secret::query()
		.filter(|secret| secret.environment_id.eq(environment))
		.all(db)
		.await?
		.into_iter()
		.filter(|secret| is_setting(&secret.name))
	{
		let Ok(value) = key.open(
			&Place {
				organization: row.organization_id,
				project: row.project_id,
				environment,
				secret: secret.id,
				name: &secret.name,
				version: secret.version,
			},
			&secret.sealed,
		) else {
			return Ok(None);
		};
		values.push(value);
	}

	Ok(Some(values))
}

/// The values a process of `environment` may print that aren't among its
/// secrets: its tenant's token and its role's password, for hiding in its
/// logs.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn printable(
	db: &impl leptoskit_db::Read,
	key: &Unwrapped,
	environment: Id<Environment>,
) -> leptoskit_db::Result<Vec<String>> {
	let Some(row) = for_environment(db, environment).await? else {
		return Ok(Vec::new());
	};
	let place = place(&row);
	Ok([
		(TenantValue::Password, &row.sealed_password),
		(TenantValue::Token, &row.sealed_token),
	]
	.into_iter()
	.filter_map(|(which, sealed)| key.open_tenant(&place, which, sealed).ok())
	.collect())
}

/// A hub's reason for failing `row`, as anyone who can view its project
/// reads it: the service's own words can quote a setting or an address, so
/// every one of the tenant's values, and every configured redaction, is
/// hidden, and control characters are written out. When its values can't
/// be opened to look for them, the reason isn't kept.
async fn scrubbed(
	db: &impl leptoskit_db::Read,
	services: &super::Services,
	row: &NotificationTenant,
	reason: &str,
) -> rpc::Result<String> {
	let withheld = || {
		"the notifications service couldn't serve it, and its reason couldn't be checked for \
		 secrets"
			.to_owned()
	};
	let Ok(key) = data_key(db, &services.shipping().vault, row.organization_id).await? else {
		return Ok(withheld());
	};
	let Some(values) = sealed_values(db, &key, row).await? else {
		return Ok(withheld());
	};
	let mut secrets = services.shipping().redactions.to_vec();
	secrets.extend(values);
	let text = super::logs::redact_lines(&[reason], &secrets).remove(0);
	Ok(text.chars().take(MAX_REASON_LEN).collect())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_tenants_name_is_its_environments_and_no_apps() {
		let environment = Id::from_uuid(Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef));
		let name = name_for(environment);
		assert_eq!(name, "ntf_0123456789abcdef0123456789abcdef");
		assert!(name.len() <= 63);
		assert!(!name.starts_with("env_"));
	}

	#[test]
	fn a_report_is_bounded() {
		let tenant = Uuid::from_u128(1);
		let count = |after: Option<&str>, until: &str, pushes: u64| {
			UsageReport {
				tenant,
				after: after.map(str::to_owned),
				until: until.to_owned(),
				pushes,
			}
		};
		assert!(check_report(&[], &[count(None, "a", 3)]).is_ok());
		assert!(check_report(&[], &[count(Some("a"), "a", 3)]).is_err());
		assert!(check_report(&[], &[count(None, "", 3)]).is_err());
		assert!(check_report(&[], &[count(None, &"a".repeat(MAX_CURSOR_LEN + 1), 3)]).is_err());
		assert!(check_report(&[], &[count(None, "a", MAX_PUSHES_PER_REPORT + 1)]).is_err());
		let failed = TenantReport {
			tenant,
			version: 1,
			state: HubState::Failed {
				reason: "x".repeat(MAX_REASON_LEN + 1),
			},
		};
		assert!(check_report(&[failed], &[]).is_err());
	}
}
