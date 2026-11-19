//! The bodies of [`crate::deployments`]'s operations, and what a job's
//! outcome does to its deployment.
//!
//! Every state change goes through [`Changing`](super::lifecycle::Changing),
//! in the transaction that decided it: `deploy`, `cancel_deployment`, and
//! `rollback` here; a worker's `claim` and `finish`; an operator's
//! `reconcile` and `revoke_worker`; and the sweeper.

use std::collections::BTreeMap;

use leptoskit_cloud_runtime::DatabaseAccess;
use leptoskit_cloud_runtime::EnvironmentRef;
use leptoskit_cloud_runtime::NamedRef;
use leptoskit_cloud_runtime::ProcessKind;
use leptoskit_config::cloud::ResolvedCloud;
use leptoskit_config::cloud::ResolvedEnvironment;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Max;
use leptoskit_db::Tx;
use leptoskit_rpc as rpc;
use sha2::Digest as _;
use uuid::Uuid;

use super::Shipping;
use super::audit;
use super::audit::Actor;
use super::authorize;
use super::delta;
use super::identity::Caller;
use super::identity::client;
use super::jobs;
use super::jobs::Queue;
use super::jobs::StoredInput;
use super::lifecycle;
use super::lifecycle::Changing;
use super::primary_email;
use super::projects::environment;
use super::services;
use super::storage::archive_key;
use crate::access::Permission;
use crate::account::Via;
use crate::archive::Digest;
use crate::deployment::Blocker;
use crate::deployment::DeploymentState;
use crate::deployment::Source;
use crate::deployment::Stage;
use crate::deployments::CostEstimate;
use crate::deployments::CostLine;
use crate::deployments::DeployConfig;
use crate::deployments::DeploymentEvent;
use crate::deployments::DeploymentSheet;
use crate::deployments::DeploymentSummary;
use crate::deployments::EventKind;
use crate::deployments::InFlightDeployment;
use crate::deployments::Line;
use crate::deployments::LineKind;
use crate::deployments::MAX_DEPLOYMENTS_PAGE;
use crate::deployments::PreviewCost;
use crate::deployments::ReleasePreview;
use crate::deployments::SignedBy;
use crate::deployments::Signer;
use crate::deployments::Stamp;
use crate::deployments::check_host;
use crate::operator::ClusterFacts;
use crate::operator::JobState;
use crate::pricing::Assumptions;
use crate::pricing::EstimateInput;
use crate::projects::EnvironmentKind;
use crate::release;
use crate::release::APP;
use crate::release::DATABASE_URL;
use crate::release::Plan;
use crate::release::PlannedProcess;
use crate::release::SpecTarget;
use crate::slug::Slug;
use crate::tables::Cluster;
use crate::tables::Deployment;
use crate::tables::Environment;
use crate::tables::EventRecord;
use crate::tables::Job;
use crate::tables::NewDeployment;
use crate::tables::NewPlacement;
use crate::tables::Organization;
use crate::tables::Placement;
use crate::tables::Project;
use crate::tables::SourceArchive;
use crate::workers::BUILD_ATTEMPTS;
use crate::workers::BuildOutcome;
use crate::workers::BuildServerOutput;
use crate::workers::BuiltImage;
use crate::workers::DeploymentRef;
use crate::workers::DestroyEnvironmentInput;
use crate::workers::JobKind;
use crate::workers::MigrationFingerprint;
use crate::workers::MigrationStep;
use crate::workers::ReleaseInput;
use crate::workers::ReleaseOutcome;
use crate::workers::RuntimeValue;
use crate::workers::WebAssets;

/// How many deployments `deployments` lists unless asked.
const DEFAULT_PAGE: u32 = 20;

/// The longest reason a line keeps.
const MAX_DETAIL_LEN: usize = 500;

/// The toolchain builds use until a project can choose one.
const TOOLCHAIN: &str = "stable";

fn number(value: i64) -> u64 {
	u64::try_from(value).unwrap_or_default()
}

/// `text` cut to [`MAX_DETAIL_LEN`] bytes at a character boundary, with
/// what a log line would have redacted replaced: a worker's reason may
/// quote an address with a password in it.
fn detail(text: &str) -> String {
	let text = super::logs::scrub(text.trim());
	let text = text.as_str();

	if text.len() <= MAX_DETAIL_LEN {
		return text.to_owned();
	}

	let mut end = MAX_DETAIL_LEN;

	while !text.is_char_boundary(end) {
		end -= 1;
	}

	format!("{}…", &text[..end])
}

/// A digest as people read it: its first twelve hex digits.
fn short(digest: &str) -> String {
	let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
	format!("sha256:{}", &hex[..hex.len().min(12)])
}

fn short_image(image: &BuiltImage) -> String {
	format!("{}@{}", image.repository, short(image.digest.as_str()))
}

/// `size` bytes as people read them.
fn bytes(size: i64) -> String {
	let size = number(size);

	if size < 1024 {
		return format!("{size} bytes");
	}

	let (unit, scale) = if size < 1024 * 1024 {
		("KiB", 1024)
	} else {
		("MiB", 1024 * 1024)
	};
	let tenths = size * 10 / scale;
	format!("{}.{} {unit}", tenths / 10, tenths % 10)
}

/// The deployment `number` of `environment`, or `NotFound`.
pub(crate) async fn by_number(
	db: &impl leptoskit_db::Read,
	environment: &Environment,
	number: u64,
) -> rpc::Result<Deployment> {
	let not_found = || {
		rpc::Error::not_found(format!(
			"deployment_not_found: `{}` has no deployment #{number}",
			environment.name
		))
	};
	let wanted = i64::try_from(number).map_err(|_| not_found())?;
	let environment_id = environment.id;
	Deployment::query()
		.filter(|row| row.environment_id.eq(environment_id) & row.number.eq(wanted))
		.first(db)
		.await?
		.ok_or_else(not_found)
}

fn summary(row: &Deployment, project: &str, environment: &str) -> DeploymentSummary {
	DeploymentSummary {
		id: row.id.uuid(),
		project: project.to_owned(),
		environment: environment.to_owned(),
		number: number(row.number),
		state: row.state.0,
		source: row.source.0.clone(),
		rollback_of: row.rollback_of.map(number),
		signer: row.signer.0.clone(),
		unknown: row.unknown,
		created_at: row.created_at,
		updated_at: row.updated_at,
	}
}

/// `Conflict` (`deployment_in_flight`) while a deployment of `environment`
/// is queued, building, migrating, or rolling out, naming it.
async fn refuse_in_flight(
	db: &impl leptoskit_db::Read,
	environment: &Environment,
) -> rpc::Result<()> {
	let environment_id = environment.id;

	if let Some(row) = Deployment::query()
		.filter(|row| row.in_flight_environment.eq(Some(environment_id)))
		.first(db)
		.await?
	{
		let unknown = if row.unknown {
			", and what its cluster runs is unknown until an operator reconciles it"
		} else {
			""
		};
		return Err(rpc::Error::conflict(format!(
			"deployment_in_flight: #{} of `{}` is {}{unknown}; wait for it, or cancel it while it \
			 is queued or building",
			row.number,
			environment.name,
			row.state.0.name()
		)));
	}

	Ok(())
}

/// The next deployment's number in `environment`.
async fn next_number(db: &impl leptoskit_db::Read, environment: &Environment) -> rpc::Result<i64> {
	let environment_id = environment.id;
	let last = Deployment::query()
		.filter(|row| row.environment_id.eq(environment_id))
		.aggregate(|row| Max(row.number), db)
		.await?;
	Ok(last.unwrap_or_default() + 1)
}

/// `Invalid` unless `[cloud]` names this project and organisation, and
/// its region, when it names them.
fn check_config(
	config: &DeployConfig,
	organization: &Organization,
	project: &Project,
) -> rpc::Result<()> {
	let mismatch = |key: &str, written: &str, actual: &str| {
		Err(rpc::Error::invalid(format!(
			"invalid_config: [cloud] {key} is `{written}`, but this deploys to `{actual}`"
		)))
	};

	if let Some(named) = &config.cloud.project
		&& named.as_str() != project.slug
	{
		return mismatch("project", named.as_str(), &project.slug);
	}

	if let Some(named) = &config.cloud.organization
		&& named.as_str() != organization.slug
	{
		return mismatch("organization", named.as_str(), &organization.slug);
	}

	if let Some(region) = &config.cloud.region
		&& region.as_str() != project.region
	{
		return mismatch("region", region.as_str(), &project.region);
	}

	Ok(())
}

/// The environment as `config` resolves it, or `Invalid`.
fn resolve(config: &DeployConfig, environment: &Environment) -> rpc::Result<ResolvedEnvironment> {
	release::resolve(config, &environment.name)
		.map_err(|error| rpc::Error::invalid(error.to_string()))
}

/// The scale the console set, when it differs from `leptoskit.toml`.
fn stored_scale(environment: &Environment) -> Option<&crate::projects::EnvironmentScale> {
	environment.scale_differs.then_some(&environment.scale.0)
}

/// The cluster a new environment in `region` goes to: the one with the
/// fewest environments, ties going to the oldest.
async fn choose_cluster(
	db: &impl leptoskit_db::Read,
	region: &str,
) -> rpc::Result<Option<Cluster>> {
	let clusters = Cluster::query()
		.filter(|cluster| cluster.region.eq(region))
		.order_by(|cluster| cluster.created_at.asc())
		.all(db)
		.await?;
	let mut chosen: Option<(u64, Cluster)> = None;

	for cluster in clusters {
		let id = cluster.id;
		let placed = Placement::query()
			.filter(|placement| placement.cluster_id.eq(id))
			.count(db)
			.await?;

		if chosen.as_ref().is_none_or(|(least, _)| placed < *least) {
			chosen = Some((placed, cluster));
		}
	}

	Ok(chosen.map(|(_, cluster)| cluster))
}

fn cluster_unavailable(region: &str) -> rpc::Error {
	rpc::Error::retry(format!(
		"cluster_unavailable: no cluster runs environments in `{region}` yet; nothing was \
		 deployed, so try again once one does"
	))
}

/// The hostname `environment` wants.
fn wanted_hostname(
	shipping: &Shipping,
	organization: &Organization,
	project: &Project,
	environment: &Environment,
) -> String {
	let label = release::hostname_label(
		&organization.slug,
		&project.slug,
		&environment.name,
		environment.kind == EnvironmentKind::Production,
	);
	format!("{label}.{}", shipping.apps_domain)
}

/// Where `environment` runs: its placement, made now for its first
/// deployment on the least-loaded cluster in its project's region.
async fn place(
	tx: &Tx,
	shipping: &Shipping,
	organization: &Organization,
	project: &Project,
	environment: &Environment,
) -> rpc::Result<(Cluster, Placement)> {
	let environment_id = environment.id;

	if let Some(placement) = Placement::query()
		.filter(|placement| placement.environment_id.eq(environment_id))
		.first(tx)
		.await?
	{
		let cluster_id = placement.cluster_id;
		let cluster = Cluster::query()
			.filter(|cluster| cluster.id.eq(cluster_id))
			.one(tx)
			.await?;
		return Ok((cluster, placement));
	}

	let cluster = choose_cluster(tx, &project.region)
		.await?
		.ok_or_else(|| cluster_unavailable(&project.region))?;
	let wanted = wanted_hostname(shipping, organization, project, environment);
	let taken = Placement::query()
		.filter(|placement| placement.hostname.eq(wanted.as_str()))
		.exists(tx)
		.await?;
	let hostname = if taken {
		format!(
			"{}.{}",
			release::fallback_label(&organization.slug, &project.slug, environment_id.uuid()),
			shipping.apps_domain
		)
	} else {
		wanted
	};
	let placement = NewPlacement {
		environment_id,
		cluster_id: cluster.id,
		hostname,
		migrations: Json(Vec::new()),
		schema_uncertain: false,
	}
	.insert(tx)
	.await?;
	Ok((cluster, placement))
}

/// The verified archive `digest` names in `organization`.
async fn verified_archive(
	tx: &Tx,
	organization: Id<Organization>,
	digest: &Digest,
) -> rpc::Result<SourceArchive> {
	SourceArchive::query()
		.filter(|archive| {
			archive.organization_id.eq(organization)
				& archive.digest.eq(digest.as_str())
				& archive.verified_at.is_not_null()
		})
		.first(tx)
		.await?
		.ok_or_else(|| {
			rpc::Error::not_found(format!(
				"archive_not_found: there is no uploaded archive {digest}; upload it with \
				 `begin_upload` and `complete_upload` first"
			))
		})
}

/// Who signed a deployment: the person, or the token and its maker; how
/// they reached the cloud; and, from the CLI or a token, the host they
/// say they deployed from.
///
/// A person's address is their own, so it is read in the caller's
/// transaction: a second connection, on SQLite's one, would wait for
/// that transaction forever.
async fn signer(
	tx: &impl leptoskit_db::Read,
	caller: &Caller,
	host: Option<String>,
) -> rpc::Result<Signer> {
	let via = caller.via();
	let host = match via {
		Via::Web => None,
		Via::Cli | Via::Token => host,
	};

	if let Some(host) = &host {
		check_host(host).map_err(|error| rpc::Error::invalid(format!("invalid_host: {error}")))?;
	}

	let by = match caller {
		Caller::Person { account, .. } => {
			SignedBy::Person {
				account: account.uuid(),
				email: primary_email(tx, *account).await?,
			}
		}
		Caller::Token(token) => {
			SignedBy::Token {
				token: token.id.uuid(),
				name: token.name.clone(),
				prefix: token.prefix.clone(),
				created_by: token.created_by.uuid(),
			}
		}
	};
	Ok(Signer { by, via, host })
}

fn line(kind: LineKind, title: String, detail: String, stamp: Stamp) -> Line {
	Line {
		kind,
		title,
		detail,
		stamp,
		stamped_at: None,
		steps: Vec::new(),
	}
}

/// A process's size and scale, as a line says it.
fn describe(process: &PlannedProcess) -> String {
	let replicas = if process.min == process.max {
		process.max.to_string()
	} else {
		format!("{}–{}", process.min, process.max)
	};
	let sleeps = if process.sleep {
		" · sleeps when idle"
	} else {
		""
	};
	format!("`{}` × {replicas}{sleeps}", process.size)
}

/// A deployment's lines before anything happened: the server, each
/// service, the migrations, and the web assets.
fn initial_lines(plan: &Plan, source: &str) -> Vec<Line> {
	let app = plan
		.processes
		.iter()
		.find(|process| process.kind == ProcessKind::App);
	let mut lines = vec![line(
		LineKind::Server,
		"Server".to_owned(),
		match app {
			Some(app) => format!("{source} · {}", describe(app)),
			None => source.to_owned(),
		},
		Stamp::Pending,
	)];

	for process in plan
		.processes
		.iter()
		.filter(|process| process.kind == ProcessKind::Service)
	{
		lines.push(line(
			LineKind::Service {
				name: process.name.clone(),
			},
			format!("Service `{}`", process.name),
			describe(process),
			Stamp::Pending,
		));
	}

	lines.push(if plan.database {
		line(
			LineKind::Migrations,
			"Migrations".to_owned(),
			"checked against the database once the build says which it carries".to_owned(),
			Stamp::Pending,
		)
	} else {
		line(
			LineKind::Migrations,
			"Migrations".to_owned(),
			"the app has no database".to_owned(),
			Stamp::Skipped,
		)
	});
	lines.push(line(
		LineKind::WebAssets,
		"Web assets".to_owned(),
		"built with the server".to_owned(),
		Stamp::Pending,
	));
	lines
}

/// The lines of a rollback to `target`: nothing is built or migrated.
fn rollback_lines(target: &Deployment) -> Vec<Line> {
	let from = format!("#{}", target.number);
	target
		.lines
		.0
		.iter()
		.map(|old| {
			let detail = match &old.kind {
				LineKind::Migrations => {
					format!("no change: the database has every migration {from} knows")
				}
				_ => format!("from {from}'s image · nothing to build"),
			};
			line(old.kind.clone(), old.title.clone(), detail, Stamp::Skipped)
		})
		.collect()
}

/// The organisation, project, and environment, by slug and ID.
fn environment_ref(
	organization: &Organization,
	project: &Project,
	environment: &Environment,
) -> EnvironmentRef {
	EnvironmentRef {
		organization: NamedRef {
			slug: organization.slug.clone(),
			id: organization.id.uuid(),
		},
		project: NamedRef {
			slug: project.slug.clone(),
			id: project.id.uuid(),
		},
		environment: NamedRef {
			slug: environment.name.clone(),
			id: environment.id.uuid(),
		},
	}
}

/// The secrets a spec's processes start with, by the names they are kept
/// under.
fn secret_names(spec: &leptoskit_cloud_runtime::ReleaseSpec) -> Vec<&String> {
	let mut names: Vec<&String> = spec
		.processes
		.iter()
		.flat_map(|process| process.secret_env.values())
		.collect();
	names.sort_unstable();
	names.dedup();
	names
}

/// Refuses while a deployment of `environment` is in flight, for a change
/// that deployment's release depends on.
///
/// # Errors
///
/// `Conflict` (`deployment_in_flight`).
pub(crate) async fn settled(
	db: &impl leptoskit_db::Read,
	environment: &Environment,
) -> rpc::Result<()> {
	refuse_in_flight(db, environment).await
}

/// The secrets the release `job` carries names, leased to `worker` for this
/// attempt.
///
/// The inner error is why the release can't start, in words for its sheet:
/// the environment no longer has one of them (which `unset_secret` and
/// `rollback` each refuse to let happen), or the vault can't open one. It
/// never holds a value, and nothing was leased.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn release_secrets(
	tx: &Tx,
	services: &super::Services,
	job: &Job,
	worker: Id<crate::tables::Worker>,
) -> rpc::Result<Result<crate::workers::LeasedSecrets, String>> {
	let StoredInput::Release(input) = &job.input.0 else {
		return Ok(Ok(crate::workers::LeasedSecrets::default()));
	};
	// A managed database's address is put together here, not kept as one
	// of the environment's secrets.
	let managed = input.database.is_some();
	let names: Vec<String> = secret_names(&input.spec.0)
		.into_iter()
		.filter(|name| !(managed && name.as_str() == DATABASE_URL))
		.cloned()
		.collect();

	if names.is_empty() && !managed {
		return Ok(Ok(crate::workers::LeasedSecrets::default()));
	}

	let Some(deployment_id) = job.deployment_id else {
		return Err(rpc::Error::server("a release job has no deployment"));
	};
	let deployment = Deployment::query()
		.filter(|row| row.id.eq(deployment_id))
		.one(tx)
		.await?;
	let (organization, project, environment) = owners(tx, &deployment).await?;
	let lessee = super::config::Lessee {
		job,
		worker,
		organization: &organization,
		project: &project,
		environment: &environment,
	};

	let vault = &services.shipping().vault;
	// Opened before any lease is written: a lease row says a value went.
	let address = if managed {
		// A release goes to its deployment's cluster and no other.
		if job.cluster_id != Some(deployment.cluster_id) {
			return Ok(Err("this release isn't queued for its deployment's \
			               cluster"
				.to_owned()));
		}

		match super::databases::address(
			tx,
			vault,
			&organization,
			&environment,
			deployment.cluster_id,
			input.spec.0.policy.database.as_ref(),
		)
		.await?
		{
			Ok(address) => Some(address),
			Err(why) => return Ok(Err(why)),
		}
	} else {
		None
	};

	match super::config::lease(tx, vault, &lessee, &names).await {
		Ok(mut values) => {
			if let Some(address) = address {
				let url = super::databases::leased(tx, &lessee, address).await?;
				values.insert(DATABASE_URL.to_owned(), url);
			}

			Ok(Ok(crate::workers::LeasedSecrets::new(values)))
		}
		Err(super::config::LeaseError::Missing(missing)) => {
			Ok(Err(format!(
				"`{}` no longer has {}",
				environment.name,
				missing.join(", ")
			)))
		}
		Err(super::config::LeaseError::Unreadable) => {
			Ok(Err("the vault couldn't open a secret this release starts \
			        with; set it again, or ask the operator about the \
			        vault's keys"
				.to_owned()))
		}
		Err(super::config::LeaseError::Failed(error)) => Err(error),
	}
}

/// A release that can't start, because a secret it starts with can't be
/// read: it fails before any worker is given it, and the previous release
/// keeps serving. Nothing reached the cluster.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn release_unstartable(tx: &Tx, job: &Job, why: &str) -> rpc::Result<()> {
	let Some(mut changing) = changing_for(tx, job).await? else {
		return Ok(());
	};

	if changing.state() != DeploymentState::RollingOut {
		return Ok(());
	}

	changing.stamp(
		&LineKind::Server,
		Stamp::Failed,
		Some(detail(&format!("not started: {why}. Nothing was changed"))),
	);
	fail(&mut changing, Stage::RollingOut, Blocker::SecretUnavailable)?;
	changing.save(tx).await?;
	Ok(())
}

/// The rows a deployment belongs to.
async fn owners(
	db: &impl leptoskit_db::Read,
	deployment: &Deployment,
) -> rpc::Result<(Organization, Project, Environment)> {
	let organization_id = deployment.organization_id;
	let project_id = deployment.project_id;
	let environment_id = deployment.environment_id;
	Ok((
		Organization::query()
			.filter(|row| row.id.eq(organization_id))
			.one(db)
			.await?,
		Project::query()
			.filter(|row| row.id.eq(project_id))
			.one(db)
			.await?,
		Environment::query()
			.filter(|row| row.id.eq(environment_id))
			.one(db)
			.await?,
	))
}

/// SHA-256 over every migration's name and checksum, in order.
fn history(migrations: &[MigrationFingerprint]) -> String {
	let mut hasher = sha2::Sha256::new();

	for migration in migrations {
		hasher.update(migration.name.as_bytes());
		hasher.update(b"\0");
		hasher.update(migration.checksum.as_bytes());
		hasher.update(b"\n");
	}

	format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// What `preview_release` prices: the environment alone, on the
/// organisation's plan, or why it can't be priced.
fn cost(
	shipping: &Shipping,
	organization: &Organization,
	environment: &str,
	resolved: ResolvedEnvironment,
) -> PreviewCost {
	let Some(catalogue) = &shipping.catalogue else {
		return PreviewCost::Unpriced {
			reason: "no price catalogue: this control plane wasn't given one, so nothing is priced"
				.to_owned(),
		};
	};
	let Some(plan) = catalogue.plan(&organization.plan) else {
		return PreviewCost::Unpriced {
			reason: format!(
				"the `{}` plan isn't in the price catalogue",
				organization.plan
			),
		};
	};
	let Ok(slug) = Slug::parse(environment) else {
		return PreviewCost::Unpriced {
			reason: format!("`{environment}` isn't a slug"),
		};
	};
	let cloud = ResolvedCloud {
		project: None,
		organization: None,
		region: None,
		url: String::new(),
		environments: BTreeMap::from([(slug, resolved)]),
	};
	let input = EstimateInput::from_cloud(&cloud, &Assumptions::default());
	let estimate = crate::pricing::estimate(catalogue, plan, &input);
	PreviewCost::Estimate(CostEstimate {
		currency: estimate.currency.as_str().to_owned(),
		plan: estimate.plan.as_str().to_owned(),
		plan_price: estimate.plan_price,
		lines: estimate
			.lines
			.iter()
			.map(|line| {
				CostLine {
					meter: line.meter.to_string(),
					quantity: line.quantity,
					allowance: line.allowance,
					billable: line.billable,
					amount: line.amount,
				}
			})
			.collect(),
		usage: estimate.usage,
		included: estimate.included,
		total: estimate.total,
		assumptions: estimate.assumptions,
	})
}

/// Fine print every release carries, after the plan's.
fn fine_print(plan: &Plan, placement_uncertain: bool) -> Vec<String> {
	let mut fine_print = plan.fine_print.clone();

	if placement_uncertain {
		fine_print.push(
			"A release before this one stopped after its server may have applied migrations, so \
			 the database may have more than the control plane knows; if it does, the new server \
			 refuses to start and the release fails while the previous one keeps serving."
				.to_owned(),
		);
	}

	fine_print
}

pub(crate) async fn deploy(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	source: Source,
	config: DeployConfig,
	host: Option<String>,
) -> rpc::Result<DeploymentSummary> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::Deploy,
	)
	.await?;
	let environment = environment(cx.tx(), &project, environment_name).await?;
	let services = services(cx.context())?;
	let shipping = services.shipping();
	let organization = access.organization.clone();
	let archive = match &source {
		Source::Upload { archive } => verified_archive(cx.tx(), organization.id, archive).await?,
		Source::Git { .. } | Source::Template { .. } => {
			return Err(rpc::Error::invalid(
				"source_unavailable: deploying from Git or a template is not available yet; \
				 upload an archive",
			));
		}
		Source::Rollback { .. } => {
			return Err(rpc::Error::invalid(
				"source_unavailable: roll back with `rollback`, which checks the database's \
				 migrations first",
			));
		}
	};
	let signer = signer(cx.tx(), &access.caller, host).await?;
	refuse_in_flight(cx.tx(), &environment).await?;
	check_config(&config, &organization, &project)?;
	let resolved = resolve(&config, &environment)?;
	let (cluster, placement) =
		place(cx.tx(), shipping, &organization, &project, &environment).await?;
	let plan = release::plan(
		&resolved,
		stored_scale(&environment),
		&config.app,
		&cluster.facts.0,
	)
	.map_err(|error| rpc::Error::invalid(error.to_string()))?;

	let planned_database = plan.postgres;
	let number = next_number(cx.tx(), &environment).await?;
	let source_detail = format!(
		"builds from {} ({})",
		short(&archive.digest),
		bytes(archive.size)
	);
	let row = lifecycle::create(
		cx.tx(),
		NewDeployment {
			organization_id: organization.id,
			project_id: project.id,
			environment_id: environment.id,
			number,
			state: lifecycle::queued(),
			in_flight_environment: None,
			live_environment: None,
			source: Json(source),
			archive_id: Some(archive.id),
			rollback_of: None,
			signer: Json(signer.clone()),
			lines: Json(initial_lines(&plan, &source_detail)),
			fine_print: Json(fine_print(&plan, placement.schema_uncertain)),
			config: Json(config.clone()),
			plan: Json(plan),
			cluster_id: cluster.id,
			hostname: placement.hostname.clone(),
			build: Json(None),
			spec: Json(None),
			unknown: false,
			superseded_by: None,
		},
		vec![EventKind::Created { signer }],
	)
	.await?;

	// A Postgres app's database is decided now, and its role's password
	// sealed: the cluster's worker makes both when the release reaches it.
	// After the deployment's row, so of two first deployments at once the
	// second is told one is in flight.
	if let Some(planned) = planned_database {
		super::databases::ensure(
			cx.tx(),
			&shipping.vault,
			&organization,
			&environment,
			cluster.id,
			planned,
		)
		.await?;
	}

	// The archive stays while this deployment needs it, and a while after.
	let mut archive = archive;
	archive.expires_at = leptoskit_db::now() + delta(shipping.settings.archive_keep);
	let digest = Digest::parse(&archive.digest)
		.map_err(|_| rpc::Error::server("an archive's digest is malformed"))?;
	let key = archive_key(organization.id.uuid(), archive.id.uuid(), &digest);
	let size = number_of_bytes(archive.size);
	archive.save(cx.tx()).await?;

	jobs::queue(
		cx.tx(),
		Queue {
			organization: organization.id,
			deployment: Some(row.id),
			environment: Some(environment.id),
			cluster: None,
			requirements: Vec::new(),
			max_attempts: BUILD_ATTEMPTS,
			input: StoredInput::BuildServer {
				deployment: DeploymentRef {
					organization: organization.slug.clone(),
					project: project.slug.clone(),
					environment: environment.name.clone(),
					number: self::number(number),
				},
				archive_key: key,
				digest,
				size,
				toolchain: TOOLCHAIN.to_owned(),
				targets: project.detected_targets.0.clone(),
				image_repository: format!(
					"{}/{}/{}",
					shipping.registry,
					organization.id.uuid(),
					project.id.uuid()
				),
				app: config.app,
			},
		},
	)
	.await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"deployment.create",
		Some(organization.id),
		Some(project.id),
		Some(format!("{} #{number}", environment.name)),
		&client(cx.context()),
	)
	.await?;
	let waker = services.clone();
	cx.after_commit(async move { jobs::wake(&waker) });
	Ok(summary(&row, &project.slug, &environment.name))
}

fn number_of_bytes(size: i64) -> u64 {
	u64::try_from(size).unwrap_or_default()
}

pub(crate) async fn preview_release(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	config: Option<DeployConfig>,
	rollback_to: Option<u64>,
) -> rpc::Result<ReleasePreview> {
	let (access, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment = environment(cx.db(), &project, environment_name).await?;
	let services = services(cx.context())?;
	let shipping = services.shipping();
	let organization = &access.organization;
	let next = number(next_number(cx.db(), &environment).await?);
	let environment_id = environment.id;
	let placement = Placement::query()
		.filter(|placement| placement.environment_id.eq(environment_id))
		.first(cx.db())
		.await?;
	let hostname = placement.as_ref().map_or_else(
		|| wanted_hostname(shipping, organization, &project, &environment),
		|placement| placement.hostname.clone(),
	);

	if let Some(to) = rollback_to {
		let target = by_number(cx.db(), &environment, to).await?;
		rollback_target(&target, placement.as_ref())?;
		let resolved = resolve(&target.config.0, &environment)?;
		let mut fine_print = target.fine_print.0.clone();
		fine_print.push(rollback_note(&target));
		return Ok(ReleasePreview {
			environment: environment.name.clone(),
			number: next,
			lines: rollback_lines(&target),
			cost: cost(shipping, organization, &environment.name, resolved),
			fine_print,
			hostname,
		});
	}

	let config = match config {
		Some(config) => config,
		None => {
			Deployment::query()
				.filter(|row| row.environment_id.eq(environment_id))
				.order_by(|row| row.number.desc())
				.first(cx.db())
				.await?
				.map(|row| row.config.0)
				.ok_or_else(|| {
					rpc::Error::invalid(format!(
						"invalid_config: `{}` has never been deployed: preview with the `config` \
						 the CLI read from `leptoskit.toml`",
						environment.name
					))
				})?
		}
	};
	check_config(&config, organization, &project)?;
	let resolved = resolve(&config, &environment)?;
	let mut extra = Vec::new();
	let facts = match &placement {
		Some(placement) => {
			let cluster_id = placement.cluster_id;
			Cluster::query()
				.filter(|cluster| cluster.id.eq(cluster_id))
				.one(cx.db())
				.await?
				.facts
				.0
		}
		None => {
			if let Some(cluster) = choose_cluster(cx.db(), &project.region).await? {
				cluster.facts.0
			} else {
				extra.push(format!(
					"No cluster runs environments in `{}` yet, so deploying now fails with \
					 `cluster_unavailable`.",
					project.region
				));
				ClusterFacts::default()
			}
		}
	};
	let plan = release::plan(&resolved, stored_scale(&environment), &config.app, &facts)
		.map_err(|error| rpc::Error::invalid(error.to_string()))?;
	let mut fine_print = fine_print(
		&plan,
		placement
			.as_ref()
			.is_some_and(|placement| placement.schema_uncertain),
	);
	fine_print.extend(extra);
	Ok(ReleasePreview {
		environment: environment.name.clone(),
		number: next,
		lines: initial_lines(&plan, "builds from the archive the CLI uploads"),
		cost: cost(shipping, organization, &environment.name, resolved),
		fine_print,
		hostname,
	})
}

/// `Conflict` (`rollback_unavailable`) unless `target` went live, has its
/// build, and the database has exactly the migrations it knows.
fn rollback_target(target: &Deployment, placement: Option<&Placement>) -> rpc::Result<()> {
	let unavailable = |why: String| {
		Err(rpc::Error::conflict(format!(
			"rollback_unavailable: #{} {why}",
			target.number
		)))
	};

	match target.state.0 {
		DeploymentState::Superseded => {}
		DeploymentState::Live => return unavailable("is what's live already".to_owned()),
		state => {
			return unavailable(format!(
				"never went live (it is {}), so there is nothing to return to",
				state.name()
			));
		}
	}

	let (Some(build), Some(placement)) = (target.build.0.as_ref(), placement) else {
		return unavailable("has no build to return to".to_owned());
	};

	// A release that failed after its server started may have applied
	// migrations the control plane never heard about: until that is
	// settled, nothing can say the database is where the target expects.
	if placement.schema_uncertain {
		return unavailable(
			"can't be returned to while the database's migrations are uncertain: a release that \
			 failed may have applied some; deploy a fix forward, which settles them"
				.to_owned(),
		);
	}

	let known = &placement.migrations.0;

	if *known != build.migrations {
		let newer: Vec<&str> = known
			.iter()
			.filter(|migration| !build.migrations.contains(migration))
			.map(|migration| migration.name.as_str())
			.collect();
		return unavailable(if newer.is_empty() {
			"carries migrations the database doesn't have".to_owned()
		} else {
			format!(
				"doesn't know migrations the database has applied since ({}), so its server would \
				 refuse to start; deploy a fix forward instead",
				newer.join(", ")
			)
		});
	}

	Ok(())
}

/// What a rollback to `target` tells the cluster about the environment's
/// managed database, and the door to it where the cluster's server is now:
/// `None` when `target` didn't use one.
///
/// # Errors
///
/// `Conflict` (`rollback_unavailable`) when the database is gone, or the
/// cluster's facts no longer say where its server is.
async fn rollback_database(
	tx: &Tx,
	target: &Deployment,
	environment: &Environment,
	cluster: Id<Cluster>,
) -> rpc::Result<Option<(crate::workers::DatabaseStep, DatabaseAccess)>> {
	if target.plan.0.postgres.is_none() {
		return Ok(None);
	}

	let unavailable = |why: &str| {
		rpc::Error::conflict(format!(
			"rollback_unavailable: #{} uses a Postgres database, and {why}",
			target.number
		))
	};
	let Some(step) = super::databases::step(tx, environment.id).await? else {
		return Err(unavailable(&format!(
			"`{}` no longer has one",
			environment.name
		)));
	};
	let server = Cluster::query()
		.filter(|row| row.id.eq(cluster))
		.one(tx)
		.await?
		.facts
		.0
		.postgres;
	let Some(server) = server else {
		return Err(unavailable(
			"its cluster no longer says where its server is",
		));
	};
	Ok(Some((
		step,
		DatabaseAccess {
			namespace: server.namespace,
			port: server.port,
		},
	)))
}

fn rollback_note(target: &Deployment) -> String {
	format!(
		"It runs #{}'s image again, built from the same source, and changes no schema. If the \
		 registry no longer keeps that image, the release fails and the live deployment keeps \
		 serving.",
		target.number
	)
}

pub(crate) async fn rollback(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	to: u64,
) -> rpc::Result<DeploymentSummary> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::Deploy,
	)
	.await?;
	let environment = environment(cx.tx(), &project, environment_name).await?;
	let services = services(cx.context())?;
	let target = by_number(cx.tx(), &environment, to).await?;
	let environment_id = environment.id;
	let placement = Placement::query()
		.filter(|placement| placement.environment_id.eq(environment_id))
		.first(cx.tx())
		.await?;
	rollback_target(&target, placement.as_ref())?;
	refuse_in_flight(cx.tx(), &environment).await?;
	let (Some(placement), Some(build), Some(spec)) =
		(placement, target.build.0.clone(), target.spec.0.clone())
	else {
		return Err(rpc::Error::conflict(format!(
			"rollback_unavailable: #{to} has no build to return to"
		)));
	};
	// A rollback applies the spec as it was recorded, variables included.
	// Its secrets are named there and read when the release is claimed, so
	// each must still exist: with their values as they are now.
	// A managed database's address isn't one of them: it is read from the
	// database's own row, which must still be there.
	let database = rollback_database(cx.tx(), &target, &environment, placement.cluster_id).await?;
	let missing = super::config::missing(
		cx.tx(),
		environment_id,
		secret_names(&spec)
			.into_iter()
			.filter(|name| !(database.is_some() && name.as_str() == DATABASE_URL)),
	)
	.await?;
	let (database, door) = database.unzip();

	if !missing.is_empty() {
		return Err(rpc::Error::conflict(format!(
			"secret_missing: #{to} starts with {} that `{environment_name}` no longer has: {}. \
			 Set {} again, or deploy instead",
			if missing.len() == 1 {
				"a secret"
			} else {
				"secrets"
			},
			missing.join(", "),
			if missing.len() == 1 { "it" } else { "them" },
		)));
	}

	let organization = access.organization.clone();
	let number = next_number(cx.tx(), &environment).await?;
	let mut spec = spec;
	spec.deployment = self::number(number);
	// The routes as recorded, less any custom domain the environment no
	// longer holds: it may be someone else's by now.
	let held = super::domains::for_release(cx.tx(), environment_id).await?;
	spec.routes
		.retain(|route| route.hostname == placement.hostname || held.contains(&route.hostname));
	super::domains::routing(cx.tx(), environment_id, &held).await?;

	// And the door to its database where the cluster's server is now.
	if door.is_some() {
		spec.policy.database = door;
	}

	spec.validate()
		.map_err(|error| rpc::Error::server(format!("a rollback's spec is invalid: {error}")))?;
	let signer = signer(cx.tx(), &access.caller, None).await?;
	let mut fine_print = fine_print(&target.plan.0, placement.schema_uncertain);
	fine_print.push(rollback_note(&target));
	let row = lifecycle::create(
		cx.tx(),
		NewDeployment {
			organization_id: organization.id,
			project_id: project.id,
			environment_id,
			number,
			state: lifecycle::queued(),
			in_flight_environment: None,
			live_environment: None,
			source: Json(Source::Rollback {
				deployment: target.id.uuid(),
			}),
			archive_id: None,
			rollback_of: Some(target.number),
			signer: Json(signer.clone()),
			config: Json(target.config.0.clone()),
			plan: Json(target.plan.0.clone()),
			lines: Json(rollback_lines(&target)),
			fine_print: Json(fine_print),
			cluster_id: placement.cluster_id,
			hostname: placement.hostname.clone(),
			build: Json(Some(build)),
			spec: Json(Some(spec.clone())),
			unknown: false,
			superseded_by: None,
		},
		vec![EventKind::Created { signer }],
	)
	.await?;
	let mut changing = Changing::new(row);
	// Nothing to build; the schema check is the one above.
	changing.advance(DeploymentState::Migrating)?;
	changing.advance(DeploymentState::RollingOut)?;
	let row = changing.save(cx.tx()).await?;
	let previous = live_number(cx.tx(), environment_id).await?;
	jobs::queue(
		cx.tx(),
		Queue {
			organization: organization.id,
			deployment: Some(row.id),
			environment: Some(environment_id),
			cluster: Some(placement.cluster_id),
			requirements: Vec::new(),
			max_attempts: 1,
			input: StoredInput::Release(Box::new(ReleaseInput {
				deployment: DeploymentRef {
					organization: organization.slug.clone(),
					project: project.slug.clone(),
					environment: environment.name.clone(),
					number: self::number(number),
				},
				spec: RuntimeValue(spec),
				migration: MigrationStep::None {
					reason: format!("a rollback to #{to} changes no schema"),
				},
				previous,
				database,
			})),
		},
	)
	.await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"deployment.rollback",
		Some(organization.id),
		Some(project.id),
		Some(format!("{} #{number} to #{to}", environment.name)),
		&client(cx.context()),
	)
	.await?;
	let waker = services.clone();
	cx.after_commit(async move { jobs::wake(&waker) });
	Ok(summary(&row, &project.slug, &environment.name))
}

/// The number of the deployment live in `environment`.
async fn live_number(tx: &Tx, environment: Id<Environment>) -> rpc::Result<Option<u64>> {
	Ok(Deployment::query()
		.filter(|row| row.live_environment.eq(Some(environment)))
		.first(tx)
		.await?
		.map(|row| number(row.number)))
}

/// Stops the jobs of a deployment that is being cancelled: a queued job is
/// abandoned, and a leased one is told to stop on its next `renew`.
async fn stop_jobs(tx: &Tx, deployment: Id<Deployment>) -> rpc::Result<()> {
	let jobs = Job::query()
		.filter(|job| {
			job.deployment_id.eq(Some(deployment))
				& (job.state.eq(JobState::Queued) | job.state.eq(JobState::Leased))
		})
		.all(tx)
		.await?;

	for mut job in jobs {
		if job.state == JobState::Queued {
			job.state = JobState::Abandoned;
		} else {
			job.cancel_requested = true;
		}

		job.save(tx).await?;
	}

	Ok(())
}

pub(crate) async fn cancel_deployment(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	number: u64,
) -> rpc::Result<DeploymentSummary> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::Deploy,
	)
	.await?;
	let environment = environment(cx.tx(), &project, environment_name).await?;
	let row = by_number(cx.tx(), &environment, number).await?;

	if !matches!(
		row.state.0,
		DeploymentState::Queued | DeploymentState::Building
	) {
		return Err(rpc::Error::conflict(format!(
			"not_cancellable: #{number} is {}; only a queued or building deployment can be \
			 cancelled",
			row.state.0.name()
		)));
	}

	stop_jobs(cx.tx(), row.id).await?;
	let mut changing = Changing::new(row);
	changing.record(EventKind::CancelRequested);
	changing.stamp_all(Stamp::Pending, Stamp::Skipped);
	changing.stamp_all(Stamp::Working, Stamp::Skipped);
	changing.advance(DeploymentState::Cancelled)?;
	let row = changing.save(cx.tx()).await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"deployment.cancel",
		Some(access.organization.id),
		Some(project.id),
		Some(format!("{} #{number}", environment.name)),
		&client(cx.context()),
	)
	.await?;
	Ok(summary(&row, &project.slug, &environment.name))
}

pub(crate) async fn deployment(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	number: u64,
) -> rpc::Result<DeploymentSheet> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment = environment(cx.db(), &project, environment_name).await?;
	let row = by_number(cx.db(), &environment, number).await?;
	Ok(DeploymentSheet {
		deployment: summary(&row, &project.slug, &environment.name),
		lines: row.lines.0.clone(),
		fine_print: row.fine_print.0.clone(),
		hostname: Some(row.hostname.clone()),
		image: row.build.0.as_ref().map(|build| build.image.clone()),
		spec: row.spec.0.clone().map(RuntimeValue),
		superseded_by: row.superseded_by.map(self::number),
	})
}

pub(crate) async fn deployments(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: Option<String>,
	before: Option<Uuid>,
	limit: Option<u32>,
) -> rpc::Result<Vec<DeploymentSummary>> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let only = match &environment_name {
		Some(name) => Some(environment(cx.db(), &project, name).await?.id),
		None => None,
	};
	let project_id = project.id;
	let names: BTreeMap<Id<Environment>, String> = Environment::query()
		.filter(|row| row.project_id.eq(project_id))
		.all(cx.db())
		.await?
		.into_iter()
		.map(|row| (row.id, row.name))
		.collect();
	let limit = limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_DEPLOYMENTS_PAGE);
	let rows = Deployment::query()
		.filter(|row| {
			let mut condition = row.project_id.eq(project_id);

			if let Some(environment) = only {
				condition = condition & row.environment_id.eq(environment);
			}

			if let Some(before) = before {
				condition = condition & row.id.lt(Id::<Deployment>::from_uuid(before));
			}

			condition
		})
		.order_by(|row| row.id.desc())
		.limit(u64::from(limit))
		.all(cx.db())
		.await?;
	Ok(rows
		.iter()
		.map(|row| {
			let environment = names.get(&row.environment_id).map_or("", String::as_str);
			summary(row, &project.slug, environment)
		})
		.collect())
}

pub(crate) async fn deployment_events(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	number: u64,
) -> rpc::Result<Vec<DeploymentEvent>> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment = environment(cx.db(), &project, environment_name).await?;
	let row = by_number(cx.db(), &environment, number).await?;
	let id = row.id;
	let events = EventRecord::query()
		.filter(|event| event.deployment_id.eq(id))
		.order_by(|event| event.seq.asc())
		.all(cx.db())
		.await?;
	Ok(events
		.into_iter()
		.map(|event| {
			DeploymentEvent {
				id: event.id.uuid(),
				seq: u32::try_from(event.seq).unwrap_or(u32::MAX),
				at: event.created_at,
				event: event.event.0,
			}
		})
		.collect())
}

pub(crate) async fn in_flight(
	cx: &rpc::QueryCx,
	organization: &str,
) -> rpc::Result<Vec<InFlightDeployment>> {
	let access = authorize::organization(
		cx.db(),
		cx.context(),
		organization,
		Permission::ViewOrganization,
	)
	.await?;
	let organization_id = access.organization.id;
	let bound = access.caller.token().and_then(|token| token.project);
	let rows = Deployment::query()
		.filter(|row| {
			row.organization_id.eq(organization_id) & row.in_flight_environment.is_not_null()
		})
		.order_by(|row| row.created_at.asc())
		.all(cx.db())
		.await?;
	let projects: BTreeMap<Id<Project>, String> = Project::query()
		.filter(|row| row.organization_id.eq(organization_id))
		.all(cx.db())
		.await?
		.into_iter()
		.map(|row| (row.id, row.slug))
		.collect();
	let mut out = Vec::new();

	for row in rows
		.iter()
		.filter(|row| bound.is_none_or(|bound| bound == row.project_id))
	{
		let environment_id = row.environment_id;
		let environment = Environment::query()
			.filter(|environment| environment.id.eq(environment_id))
			.first(cx.db())
			.await?
			.map(|environment| environment.name)
			.unwrap_or_default();
		out.push(InFlightDeployment {
			id: row.id.uuid(),
			project: projects.get(&row.project_id).cloned().unwrap_or_default(),
			environment,
			number: number(row.number),
			state: row.state.0,
			unknown: row.unknown,
			created_at: row.created_at,
		});
	}

	Ok(out)
}

/// Readies an environment for deletion, in the deleting transaction: a
/// queued or building deployment stops (its build's worker learns it on
/// `renew`); one migrating or rolling out, or whose release is unknown,
/// refuses the deletion; and when the environment was placed, a removal is
/// queued for its cluster's worker.
///
/// # Errors
///
/// `Conflict` (`deployment_in_flight`) while a release is under way.
pub(crate) async fn before_removal(
	tx: &Tx,
	organization: &Organization,
	project: &Project,
	environment: &Environment,
) -> rpc::Result<Option<Job>> {
	let environment_id = environment.id;

	if let Some(row) = Deployment::query()
		.filter(|row| row.in_flight_environment.eq(Some(environment_id)))
		.first(tx)
		.await?
	{
		if matches!(
			row.state.0,
			DeploymentState::Queued | DeploymentState::Building
		) {
			stop_jobs(tx, row.id).await?;
		} else {
			return Err(rpc::Error::conflict(format!(
				"deployment_in_flight: #{} of `{}` is {}; wait for it to finish before deleting \
				 the environment",
				row.number,
				environment.name,
				row.state.0.name()
			)));
		}
	}

	let unsettled = Job::query()
		.filter(|job| {
			job.environment_id.eq(Some(environment_id))
				& job.kind.ne(JobKind::BuildServer)
				& (job.state.eq(JobState::Leased) | job.state.eq(JobState::Unknown))
		})
		.exists(tx)
		.await?;

	if unsettled {
		return Err(rpc::Error::conflict(format!(
			"deployment_in_flight: `{}`'s cluster is still applying or removing a release, or an \
			 operator hasn't reconciled one; wait for it before deleting the environment",
			environment.name
		)));
	}

	let Some(placement) = Placement::query()
		.filter(|placement| placement.environment_id.eq(environment_id))
		.first(tx)
		.await?
	else {
		super::domains::environment_removed(tx, environment_id, false).await?;
		return Ok(None);
	};
	// The cluster may still answer on its domains until the removal is
	// done: they keep their hostnames until then.
	super::domains::environment_removed(tx, environment_id, true).await?;
	// Its database goes with it: the row with the environment's, and the
	// database itself when the cluster's worker removes the environment.
	let database = super::databases::for_environment(tx, environment_id)
		.await?
		.map(|row| row.name);
	let job = jobs::queue(
		tx,
		Queue {
			organization: organization.id,
			deployment: None,
			environment: Some(environment_id),
			cluster: Some(placement.cluster_id),
			requirements: Vec::new(),
			max_attempts: 1,
			input: StoredInput::DestroyEnvironment(DestroyEnvironmentInput {
				environment: RuntimeValue(environment_ref(organization, project, environment)),
				database,
			}),
		},
	)
	.await?;
	Ok(Some(job))
}

/// Records `event` on `job`'s deployment, while it exists.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn record(tx: &Tx, job: &Job, event: EventKind) -> rpc::Result<()> {
	if let Some(changing) = changing_for(tx, job).await? {
		lifecycle::record(tx, &changing.row, vec![event]).await?;
	}

	Ok(())
}

/// `job`'s deployment, to change, while it exists.
async fn changing_for(tx: &Tx, job: &Job) -> rpc::Result<Option<Changing>> {
	let Some(id) = job.deployment_id else {
		return Ok(None);
	};
	Ok(Deployment::query()
		.filter(|row| row.id.eq(id))
		.first(tx)
		.await?
		.map(Changing::new))
}

/// Fails a deployment in `stage` for `blocker`: lines nothing reached are
/// skipped.
fn fail(changing: &mut Changing, stage: Stage, blocker: Blocker) -> rpc::Result<()> {
	changing.stamp_all(Stamp::Pending, Stamp::Skipped);
	changing.stamp_all(Stamp::Working, Stamp::Skipped);
	changing.advance(DeploymentState::Failed { stage, blocker })
}

/// A claimed build: its deployment starts building.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn build_claimed(tx: &Tx, job: &Job, attempt: u32) -> rpc::Result<()> {
	let Some(mut changing) = changing_for(tx, job).await? else {
		return Ok(());
	};

	if changing.state() == DeploymentState::Queued {
		changing.advance(DeploymentState::Building)?;
	}

	changing.stamp(
		&LineKind::Server,
		Stamp::Working,
		Some(format!("building, attempt {attempt}")),
	);
	changing.record(EventKind::Claimed {
		job: job.id.uuid(),
		kind: job.kind,
		attempt,
	});
	changing.save(tx).await?;
	Ok(())
}

/// A claimed release or removal.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn release_claimed(tx: &Tx, job: &Job, attempt: u32) -> rpc::Result<()> {
	record(
		tx,
		job,
		EventKind::Claimed {
			job: job.id.uuid(),
			kind: job.kind,
			attempt,
		},
	)
	.await
}

/// What a build's outcome does to its deployment. A deployment that was
/// cancelled, or isn't building any more, keeps its state.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn build_finished(tx: &Tx, job: &Job, outcome: &BuildOutcome) -> rpc::Result<()> {
	let Some(mut changing) = changing_for(tx, job).await? else {
		return Ok(());
	};

	if changing.state() != DeploymentState::Building {
		return Ok(());
	}

	match outcome {
		BuildOutcome::Succeeded(output) => {
			if let Some(stray) = stray_image(job, output) {
				changing.stamp(
					&LineKind::Server,
					Stamp::Failed,
					Some(detail(&format!(
						"the build named the image repository `{stray}`, not the one it was given"
					))),
				);
				fail(&mut changing, Stage::Building, Blocker::BuildFailed)?;
				changing.save(tx).await?;
				return Ok(());
			}

			built(tx, changing, output).await
		}
		BuildOutcome::Failed { reason, steps } => {
			changing.steps(&LineKind::Server, steps.clone());
			changing.stamp(&LineKind::Server, Stamp::Failed, Some(detail(reason)));
			fail(&mut changing, Stage::Building, Blocker::BuildFailed)?;
			changing.save(tx).await?;
			Ok(())
		}
		BuildOutcome::Stopped => {
			changing.stamp(
				&LineKind::Server,
				Stamp::Failed,
				Some("the build worker stopped without a result".to_owned()),
			);
			fail(&mut changing, Stage::Building, Blocker::BuildFailed)?;
			changing.save(tx).await?;
			Ok(())
		}
	}
}

/// The first image repository `output` names that isn't the one `job` gave
/// its build, if any: the server's image, or a process's own. A build's
/// images stay in its organisation's and project's repository.
fn stray_image<'o>(job: &Job, output: &'o BuildServerOutput) -> Option<&'o str> {
	let StoredInput::BuildServer {
		image_repository, ..
	} = &job.input.0
	else {
		return Some(output.image.repository.as_str());
	};

	std::iter::once(&output.image)
		.chain(
			output
				.processes
				.iter()
				.filter_map(|process| process.image.as_ref()),
		)
		.map(|image| image.repository.as_str())
		.find(|repository| *repository != image_repository)
}

/// Stamps what a successful build made: the server, each service, and the
/// web assets.
fn stamp_built(changing: &mut Changing, output: &BuildServerOutput) {
	let image = short_image(&output.image);
	changing.steps(&LineKind::Server, output.steps.clone());
	changing.stamp(
		&LineKind::Server,
		Stamp::Built,
		Some(format!("image {image}")),
	);

	for process in output
		.processes
		.iter()
		.filter(|process| process.name != APP)
	{
		let own = process.image.as_ref().map_or_else(
			|| format!("in the server's image {image}"),
			|own| format!("image {}", short_image(own)),
		);
		changing.stamp(
			&LineKind::Service {
				name: process.name.clone(),
			},
			Stamp::Built,
			Some(own),
		);
	}

	match &output.web_assets {
		Some(WebAssets::InImage { path }) => {
			changing.stamp(
				&LineKind::WebAssets,
				Stamp::Built,
				Some(format!("in the server's image, at `{path}`")),
			);
		}
		None => {
			changing.stamp(
				&LineKind::WebAssets,
				Stamp::Skipped,
				Some("the server has no web assets".to_owned()),
			);
		}
	}
}

/// A build that succeeded: its spec is made, the schema checked, and its
/// release queued.
async fn built(tx: &Tx, mut changing: Changing, output: &BuildServerOutput) -> rpc::Result<()> {
	stamp_built(&mut changing, output);
	let (organization, project, environment) = owners(tx, &changing.row).await?;
	let cluster_id = changing.row.cluster_id;
	let cluster = Cluster::query()
		.filter(|cluster| cluster.id.eq(cluster_id))
		.one(tx)
		.await?;
	let hostname = changing.row.hostname.clone();
	// The environment's variables and secret names as they are now, when
	// the build ends: what was set while it built is in.
	let (variables, secrets) = super::config::for_release(tx, environment.id).await?;
	// And its verified domains: the hostnames that are its to route.
	let domains = super::domains::for_release(tx, environment.id).await?;
	let target = SpecTarget {
		environment: environment_ref(&organization, &project, &environment),
		deployment: number(changing.row.number),
		hostname: &hostname,
		cluster: &cluster.facts.0,
		variables: &variables,
		secrets: &secrets,
		domains: &domains,
	};
	let spec = match release::spec(&changing.row.plan.0, output, &target) {
		Ok(spec) => spec,
		Err(error) => {
			changing.stamp(
				&LineKind::Server,
				Stamp::Failed,
				Some(detail(&error.to_string())),
			);
			fail(&mut changing, Stage::Building, Blocker::BuildFailed)?;
			changing.save(tx).await?;
			return Ok(());
		}
	};
	changing.row.build = Json(Some(output.clone()));
	changing.row.spec = Json(Some(spec.clone()));
	// From here the cluster may be asked to route them.
	super::domains::routing(tx, environment.id, &domains).await?;
	changing.advance(DeploymentState::Migrating)?;

	// The schema check: the database's migrations must be where this
	// build's history starts.
	let environment_id = environment.id;
	let placement = Placement::query()
		.filter(|placement| placement.environment_id.eq(environment_id))
		.one(tx)
		.await?;
	let known = &placement.migrations.0;
	let carried = &output.migrations;

	if carried.len() < known.len() || carried[..known.len()] != known[..] {
		let missing: Vec<&str> = known
			.iter()
			.filter(|migration| !carried.contains(migration))
			.map(|migration| migration.name.as_str())
			.collect();
		let why = if missing.is_empty() {
			"this build's migrations don't start with the ones the database has applied".to_owned()
		} else {
			format!(
				"the database has migrations this build doesn't know ({}); its server would \
				 refuse to start",
				missing.join(", ")
			)
		};
		changing.stamp(&LineKind::Migrations, Stamp::Failed, Some(why));
		fail(&mut changing, Stage::Migrating, Blocker::SchemaIncompatible)?;
		changing.save(tx).await?;
		return Ok(());
	}

	let pending: Vec<String> = carried[known.len()..]
		.iter()
		.map(|migration| migration.name.clone())
		.collect();
	let migration = if !changing.row.plan.0.database {
		MigrationStep::None {
			reason: "the app has no database".to_owned(),
		}
	} else if pending.is_empty() {
		changing.stamp(
			&LineKind::Migrations,
			Stamp::Skipped,
			Some("no new migrations: the database has every one this build carries".to_owned()),
		);
		MigrationStep::None {
			reason: "no new migrations".to_owned(),
		}
	} else {
		changing.stamp(
			&LineKind::Migrations,
			Stamp::Working,
			Some(format!(
				"the new server applies {} as it starts, before it answers `/readyz`: {}",
				plural(pending.len(), "migration"),
				pending.join(", ")
			)),
		);
		MigrationStep::AtStartup {
			history: history(carried),
			pending,
		}
	};
	let database = if changing.row.plan.0.postgres.is_some() {
		let Some(step) = super::databases::step(tx, environment_id).await? else {
			return Err(rpc::Error::server(
				"a deployment planned with Postgres has no database",
			));
		};
		Some(step)
	} else {
		None
	};
	changing.advance(DeploymentState::RollingOut)?;
	let previous = live_number(tx, environment_id).await?;
	let deployment_number = number(changing.row.number);
	let deployment_id = changing.row.id;
	changing.save(tx).await?;
	jobs::queue(
		tx,
		Queue {
			organization: organization.id,
			deployment: Some(deployment_id),
			environment: Some(environment_id),
			cluster: Some(cluster.id),
			requirements: Vec::new(),
			max_attempts: 1,
			input: StoredInput::Release(Box::new(ReleaseInput {
				deployment: DeploymentRef {
					organization: organization.slug,
					project: project.slug,
					environment: environment.name,
					number: deployment_number,
				},
				spec: RuntimeValue(spec),
				migration,
				previous,
				database,
			})),
		},
	)
	.await?;
	Ok(())
}

fn plural(count: usize, noun: &str) -> String {
	if count == 1 {
		format!("1 {noun}")
	} else {
		format!("{count} {noun}s")
	}
}

/// A build given up on: its deployment fails with `builder_unavailable`
/// where it stands.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn build_abandoned(tx: &Tx, job: &Job, reason: &str) -> rpc::Result<()> {
	let Some(mut changing) = changing_for(tx, job).await? else {
		return Ok(());
	};
	let stage = match changing.state() {
		DeploymentState::Queued => Stage::Queued,
		DeploymentState::Building => Stage::Building,
		_ => return Ok(()),
	};
	changing.stamp(&LineKind::Server, Stamp::Failed, Some(detail(reason)));
	fail(&mut changing, stage, Blocker::BuilderUnavailable)?;
	changing.save(tx).await?;
	Ok(())
}

/// The line a process's name stands for.
fn process_line(process: &str) -> LineKind {
	if process == APP {
		LineKind::Server
	} else {
		LineKind::Service {
			name: process.to_owned(),
		}
	}
}

/// Lines left unknown by a lapsed release, settled now its outcome is
/// known: migrations stay as the caller stamps them, the rest are built.
fn settle_unknown(changing: &mut Changing) {
	let kinds: Vec<LineKind> = changing
		.row
		.lines
		.0
		.iter()
		.filter(|line| line.stamp == Stamp::Unknown && line.kind != LineKind::Migrations)
		.map(|line| line.kind.clone())
		.collect();

	for kind in kinds {
		changing.stamp(&kind, Stamp::Built, None);
	}
}

/// Whether the release may have applied migrations: its line says the
/// server applies them, or that it is unknown whether it did.
fn migrations_pending(changing: &Changing) -> bool {
	changing.row.lines.0.iter().any(|line| {
		line.kind == LineKind::Migrations && matches!(line.stamp, Stamp::Working | Stamp::Unknown)
	})
}

/// What a release's outcome does to its deployment and the environment:
/// live, superseding the deployment before it, or failed while the
/// previous one keeps serving. It settles an unknown release too.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn release_finished(
	tx: &Tx,
	job: &Job,
	outcome: &ReleaseOutcome,
) -> rpc::Result<()> {
	let Some(mut changing) = changing_for(tx, job).await? else {
		return Ok(());
	};

	if changing.state() != DeploymentState::RollingOut {
		return Ok(());
	}

	let environment_id = changing.row.environment_id;
	let mut placement = Placement::query()
		.filter(|placement| placement.environment_id.eq(environment_id))
		.one(tx)
		.await?;

	if changing.row.unknown {
		changing.row.unknown = false;
		changing.record(EventKind::Reconciled {
			job: job.id.uuid(),
			outcome: match outcome {
				ReleaseOutcome::Applied => "applied",
				ReleaseOutcome::NotReady { .. } => "not_ready",
				ReleaseOutcome::MigrationsFailed { .. } => "migrations_failed",
			}
			.to_owned(),
		});
	}

	let pending = migrations_pending(&changing);

	match outcome {
		ReleaseOutcome::Applied => {
			if let Some(previous) = Deployment::query()
				.filter(|row| row.live_environment.eq(Some(environment_id)))
				.first(tx)
				.await?
			{
				let mut previous = Changing::new(previous);
				previous.advance(DeploymentState::Superseded)?;
				previous.row.superseded_by = Some(changing.row.number);
				previous.save(tx).await?;
			}

			if pending {
				changing.stamp(
					&LineKind::Migrations,
					Stamp::Migrated,
					Some("applied as the new server started".to_owned()),
				);
			}

			changing.stamp_all(Stamp::Built, Stamp::Live);
			changing.stamp_all(Stamp::Unknown, Stamp::Live);
			changing.advance(DeploymentState::Live)?;

			// Domains detached while a deployment still served them go
			// now, if this one doesn't.
			let routed: std::collections::BTreeSet<&str> = changing
				.row
				.spec
				.0
				.as_ref()
				.map(|spec| {
					spec.routes
						.iter()
						.map(|route| route.hostname.as_str())
						.collect()
				})
				.unwrap_or_default();
			super::domains::released(tx, environment_id, &routed).await?;

			if changing.row.plan.0.postgres.is_some() {
				super::databases::applied(tx, environment_id).await?;
			}

			if let Some(build) = changing.row.build.0.as_ref() {
				placement.migrations = Json(build.migrations.clone());
			}

			placement.schema_uncertain = false;
		}
		ReleaseOutcome::NotReady { process, reason } => {
			settle_unknown(&mut changing);

			if pending {
				changing.stamp(
					&LineKind::Migrations,
					Stamp::Unknown,
					Some("the new server may have applied them before it stopped".to_owned()),
				);
				placement.schema_uncertain = true;
			}

			changing.stamp(
				&process_line(process),
				Stamp::Failed,
				Some(detail(&format!("never became ready: {reason}"))),
			);
			fail(&mut changing, Stage::RollingOut, Blocker::NotReady)?;
		}
		ReleaseOutcome::MigrationsFailed { reason } => {
			settle_unknown(&mut changing);
			changing.stamp(&LineKind::Migrations, Stamp::Failed, Some(detail(reason)));
			placement.schema_uncertain = true;
			fail(&mut changing, Stage::RollingOut, Blocker::MigrationFailed)?;
		}
	}

	placement.save(tx).await?;
	changing.save(tx).await?;
	Ok(())
}

/// A release whose worker went quiet: what its cluster runs is unknown,
/// and the environment stays in flight until it is reconciled.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn release_unknown(tx: &Tx, job: &Job, reason: &str) -> rpc::Result<()> {
	let Some(mut changing) = changing_for(tx, job).await? else {
		return Ok(());
	};

	if changing.state() != DeploymentState::RollingOut {
		return Ok(());
	}

	changing.row.unknown = true;
	changing.stamp_all(Stamp::Built, Stamp::Unknown);
	changing.stamp_all(Stamp::Working, Stamp::Unknown);
	changing.record(EventKind::Reconciling {
		job: job.id.uuid(),
		reason: format!("{reason}; an operator reconciles it, or its worker reports its outcome"),
	});
	changing.save(tx).await?;
	Ok(())
}

/// An unknown release an operator found not applied: it fails as
/// interrupted, and the previous release keeps serving.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn release_interrupted(tx: &Tx, job: &Job, found: &str) -> rpc::Result<()> {
	let Some(mut changing) = changing_for(tx, job).await? else {
		return Ok(());
	};

	if changing.state() != DeploymentState::RollingOut {
		return Ok(());
	}

	let environment_id = changing.row.environment_id;
	let pending = migrations_pending(&changing);
	changing.row.unknown = false;
	changing.record(EventKind::Reconciled {
		job: job.id.uuid(),
		outcome: "not_applied".to_owned(),
	});
	settle_unknown(&mut changing);
	changing.stamp(
		&LineKind::Server,
		Stamp::Failed,
		Some(detail(&format!("interrupted: {found}"))),
	);

	if pending {
		let mut placement = Placement::query()
			.filter(|placement| placement.environment_id.eq(environment_id))
			.one(tx)
			.await?;
		placement.schema_uncertain = true;
		placement.save(tx).await?;
		changing.stamp(
			&LineKind::Migrations,
			Stamp::Unknown,
			Some("the new server may have applied them before it was interrupted".to_owned()),
		);
	}

	fail(&mut changing, Stage::RollingOut, Blocker::Interrupted)?;
	changing.save(tx).await?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sizes_and_digests_read_plainly() {
		assert_eq!(bytes(12), "12 bytes");
		assert_eq!(bytes(1536), "1.5 KiB");
		assert_eq!(bytes(5 * 1024 * 1024), "5.0 MiB");
		assert_eq!(
			short(&format!("sha256:{}", "ab".repeat(32))),
			"sha256:abababababab"
		);
		assert_eq!(
			detail(&"x".repeat(600)).len(),
			MAX_DETAIL_LEN + '…'.len_utf8()
		);
	}

	#[test]
	fn a_migration_history_is_ordered_and_exact() {
		let one = MigrationFingerprint {
			name: "0001_init".to_owned(),
			checksum: "a".to_owned(),
		};
		let two = MigrationFingerprint {
			name: "0002_more".to_owned(),
			checksum: "b".to_owned(),
		};
		assert_eq!(
			history(&[one.clone(), two.clone()]),
			history(&[one.clone(), two.clone()])
		);
		assert_ne!(history(&[one.clone(), two.clone()]), history(&[two, one]));
	}
}
