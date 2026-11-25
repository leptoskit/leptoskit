//! Publishing's bodies: see [`crate::publishing`].

use std::collections::BTreeSet;

use leptoskit_db::Id;
use leptoskit_db::Tx;
use leptoskit_rpc as rpc;
use uuid::Uuid;

use super::audit;
use super::audit::Actor;
use super::authorize;
use super::delta;
use super::identity::client;
use super::jobs;
use super::jobs::Queue;
use super::jobs::StoredInput;
use super::projects::environment;
use super::services;
use super::storage::archive_key;
use crate::access::Permission;
use crate::archive::Digest;
use crate::deployments::LogPage;
use crate::operator::JobState;
use crate::publishing::PublicationSummary;
use crate::tables::Environment;
use crate::tables::Job;
use crate::tables::Organization;
use crate::tables::Project;
use crate::tables::Secret;
use crate::workers::APPLE_STORES;
use crate::workers::JobKind;
use crate::workers::JobOutcome;
use crate::workers::LeasedSecrets;
use crate::workers::MACOS_CAPABILITY;
use crate::workers::PUBLISH_CAPABILITY;
use crate::workers::PUBLISH_CREDENTIALS;
use crate::workers::PUBLISH_STORES;
use crate::workers::is_publish_credential;

/// The toolchain a publish builds with.
const TOOLCHAIN: &str = "stable";

/// How many publishes `publications` lists unless asked.
const DEFAULT_LIST: u32 = 20;

/// The most `publications` lists.
const MAX_LIST: u32 = 100;

/// `publish`'s arguments.
pub(crate) struct PublishArgs {
	pub(crate) organization: String,
	pub(crate) project: String,
	pub(crate) environment: String,
	pub(crate) archive: Digest,
	pub(crate) stores: Vec<String>,
	pub(crate) submit: bool,
}

/// `publication_log`'s arguments.
pub(crate) struct LogArgs {
	pub(crate) organization: String,
	pub(crate) project: String,
	pub(crate) environment: String,
	pub(crate) id: Uuid,
	pub(crate) after: Option<u64>,
	pub(crate) limit: Option<u32>,
}

/// `stores` in `lk publish`'s order, or why they can't be published to.
fn checked_stores(stores: &[String]) -> rpc::Result<Vec<String>> {
	if stores.is_empty() {
		return Err(rpc::Error::invalid(
			"invalid_stores: name at least one store",
		));
	}

	let mut seen = BTreeSet::new();

	for store in stores {
		if !PUBLISH_STORES.contains(&store.as_str()) {
			return Err(rpc::Error::invalid(format!(
				"invalid_stores: `{}` isn't a store; the stores are {}",
				store.chars().take(40).collect::<String>(),
				PUBLISH_STORES.join(", ")
			)));
		}

		if !seen.insert(store.as_str()) {
			return Err(rpc::Error::invalid(format!(
				"invalid_stores: `{store}` is named twice"
			)));
		}
	}

	Ok(PUBLISH_STORES
		.iter()
		.filter(|store| seen.contains(**store))
		.map(|store| (*store).to_owned())
		.collect())
}

/// The credentials `stores` read, in order, without repeats.
fn credential_names(stores: &[String]) -> Vec<&'static str> {
	let mut names: Vec<&'static str> = Vec::new();

	for (store, name) in PUBLISH_CREDENTIALS {
		if stores.iter().any(|named| named == store) && !names.contains(&name) {
			names.push(name);
		}
	}

	names
}

/// The publish job `id` of `environment`.
async fn job_of(
	db: &impl leptoskit_db::Read,
	environment: &Environment,
	id: Uuid,
) -> rpc::Result<Job> {
	let environment_id = environment.id;
	Job::query()
		.filter(|job| {
			job.id.eq(Id::<Job>::from_uuid(id))
				& job.environment_id.eq(Some(environment_id))
				& job.kind.eq(JobKind::Publish)
		})
		.first(db)
		.await?
		.ok_or_else(|| {
			rpc::Error::not_found(format!(
				"publication_not_found: `{}` has no publish {id}",
				environment.name
			))
		})
}

/// `job` as its organisation sees it.
fn view(job: &Job, environment: &Environment) -> rpc::Result<PublicationSummary> {
	let StoredInput::Publish {
		digest,
		stores,
		submit,
		credentials,
		..
	} = &job.input.0
	else {
		return Err(rpc::Error::server(
			"a publish job holds another job's input",
		));
	};
	Ok(PublicationSummary {
		id: job.id.uuid(),
		environment: environment.name.clone(),
		stores: stores.clone(),
		submit: *submit,
		digest: digest.clone(),
		credentials: credentials.clone(),
		requirements: job.requirements.0.clone(),
		state: job.state,
		outcome: match &job.outcome.0 {
			Some(JobOutcome::Publish(outcome)) => Some(outcome.clone()),
			_ => None,
		},
		created_at: job.created_at,
		updated_at: job.updated_at,
	})
}

pub(crate) async fn publish(
	cx: &rpc::MutationCx,
	args: PublishArgs,
) -> rpc::Result<PublicationSummary> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		&args.organization,
		&args.project,
		Permission::Publish,
	)
	.await?;
	let stores = checked_stores(&args.stores)?;
	let environment = environment(cx.tx(), &project, &args.environment).await?;
	let services = services(cx.context())?;
	let shipping = services.shipping();
	let organization = access.organization.clone();
	let tx = cx.tx();
	let environment_id = environment.id;

	if Job::query()
		.filter(|job| {
			job.environment_id.eq(Some(environment_id))
				& job.kind.eq(JobKind::Publish)
				& (job.state.eq(JobState::Queued) | job.state.eq(JobState::Leased))
		})
		.exists(tx)
		.await?
	{
		return Err(rpc::Error::conflict(format!(
			"publish_in_flight: `{}` is publishing already; wait for it to end",
			environment.name
		)));
	}

	super::billing::refuse_over_cap(tx, shipping, &organization).await?;
	let mut archive =
		super::deployments::verified_archive(tx, organization.id, &args.archive).await?;

	// The secrets the stores read that the environment has: only their
	// names are kept, and the values are leased when a worker claims it.
	let set: Vec<String> = Secret::query()
		.filter(|row| row.environment_id.eq(environment_id))
		.all(tx)
		.await?
		.into_iter()
		.map(|row| row.name)
		.collect();
	let credentials: Vec<String> = credential_names(&stores)
		.into_iter()
		.filter(|name| set.iter().any(|row| row == name))
		.map(str::to_owned)
		.collect();
	let mut requirements = vec![PUBLISH_CAPABILITY.to_owned()];

	if stores
		.iter()
		.any(|store| APPLE_STORES.contains(&store.as_str()))
	{
		requirements.push(MACOS_CAPABILITY.to_owned());
	}

	// The archive stays until the job has had every chance to run, and a
	// while after.
	let settings = shipping.settings;
	let keep = leptoskit_db::now()
		+ delta(settings.unclaimed_build)
		+ delta(settings.publish_deadline)
		+ delta(settings.archive_keep);
	archive.expires_at = archive.expires_at.max(keep);
	let digest = Digest::parse(&archive.digest)
		.map_err(|_| rpc::Error::server("an archive's digest is malformed"))?;
	let key = archive_key(organization.id.uuid(), archive.id.uuid(), &digest);
	let size = u64::try_from(archive.size).unwrap_or_default();
	archive.save(tx).await?;

	let job = jobs::queue(
		tx,
		Queue {
			organization: organization.id,
			deployment: None,
			environment: Some(environment.id),
			cluster: None,
			requirements,
			// It may have reached a store before it stopped.
			max_attempts: 1,
			input: StoredInput::Publish {
				organization: organization.slug.clone(),
				project: project.slug.clone(),
				environment: environment.name.clone(),
				archive_key: key,
				digest,
				size,
				toolchain: TOOLCHAIN.to_owned(),
				stores: stores.clone(),
				submit: args.submit,
				credentials,
			},
		},
	)
	.await?;
	audit::record(
		tx,
		&Actor::from(&access.caller),
		"publication.create",
		Some(organization.id),
		Some(project.id),
		Some(format!(
			"{}: {}{}",
			environment.name,
			stores.join(", "),
			if args.submit { ", submitted" } else { "" }
		)),
		&client(cx.context()),
	)
	.await?;
	let waker = services.clone();
	cx.after_commit(async move { jobs::wake(&waker) });
	view(&job, &environment)
}

pub(crate) async fn publication(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	id: Uuid,
) -> rpc::Result<PublicationSummary> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment = environment(cx.db(), &project, environment_name).await?;
	let job = job_of(cx.db(), &environment, id).await?;
	view(&job, &environment)
}

pub(crate) async fn publications(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	limit: Option<u32>,
) -> rpc::Result<Vec<PublicationSummary>> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	let environment = environment(cx.db(), &project, environment_name).await?;
	let environment_id = environment.id;
	Job::query()
		.filter(|job| job.environment_id.eq(Some(environment_id)) & job.kind.eq(JobKind::Publish))
		.order_by(|job| job.created_at.desc())
		.order_by(|job| job.id.desc())
		.limit(u64::from(limit.unwrap_or(DEFAULT_LIST).clamp(1, MAX_LIST)))
		.all(cx.db())
		.await?
		.iter()
		.map(|job| view(job, &environment))
		.collect()
}

pub(crate) async fn publication_log(cx: &rpc::QueryCx, args: LogArgs) -> rpc::Result<LogPage> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		&args.organization,
		&args.project,
		Permission::ViewProjects,
	)
	.await?;
	let environment = environment(cx.db(), &project, &args.environment).await?;
	let job = job_of(cx.db(), &environment, args.id).await?;
	let job_id = job.id;
	super::logs::build_page(cx, args.after, args.limit, |chunk| {
		chunk.job_id.eq(Some(job_id)) & chunk.deployment_id.is_null()
	})
	.await
}

/// Fails the publishes of `environment` that no worker has claimed: it is
/// being removed.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn abandon_queued(tx: &Tx, environment: Id<Environment>) -> rpc::Result<()> {
	let queued = Job::query()
		.filter(|job| {
			job.environment_id.eq(Some(environment))
				& job.kind.eq(JobKind::Publish)
				& job.state.eq(JobState::Queued)
		})
		.all(tx)
		.await?;

	for mut job in queued {
		job.state = JobState::Abandoned;
		job.outcome = leptoskit_db::Json(Some(JobOutcome::Publish(
			crate::workers::PublishOutcome::Failed {
				reason: "its environment was removed before it started".to_owned(),
			},
		)));
		job.save(tx).await?;
	}

	Ok(())
}

/// The credentials the publish `job` carries, leased to `worker` for this
/// attempt, or why it can't start, in words for its organisation: the
/// environment no longer has one, or the vault can't open one. The error
/// never holds a value, and nothing was leased.
///
/// # Errors
///
/// When the database fails.
pub(crate) async fn credentials(
	tx: &Tx,
	services: &super::Services,
	job: &Job,
	worker: Id<crate::tables::Worker>,
) -> rpc::Result<Result<LeasedSecrets, String>> {
	let StoredInput::Publish { credentials, .. } = &job.input.0 else {
		return Err(rpc::Error::server(
			"a publish job holds another job's input",
		));
	};

	// Only what a store reads, whatever the row says.
	if let Some(name) = credentials.iter().find(|name| !is_publish_credential(name)) {
		return Ok(Err(format!("`{name}` isn't a store's credential")));
	}

	let Some(environment_id) = job.environment_id else {
		return Err(rpc::Error::server("a publish job has no environment"));
	};
	// Removed since it was queued: the publish fails, and nothing is leased.
	let Some(environment) = Environment::query()
		.filter(|row| row.id.eq(environment_id))
		.first(tx)
		.await?
	else {
		return Ok(Err("its environment was removed".to_owned()));
	};
	let Some(project) = Project::query()
		.filter(|row| row.id.eq(environment.project_id))
		.first(tx)
		.await?
	else {
		return Ok(Err("its project was removed".to_owned()));
	};
	let Some(organization) = Organization::query()
		.filter(|row| row.id.eq(project.organization_id))
		.first(tx)
		.await?
	else {
		return Ok(Err("its organisation was removed".to_owned()));
	};

	if organization.id != job.organization_id {
		return Err(rpc::Error::server(
			"a publish job's environment is another organisation's",
		));
	}

	if credentials.is_empty() {
		return Ok(Ok(LeasedSecrets::default()));
	}

	let lessee = super::config::Lessee {
		job,
		worker,
		organization: &organization,
		project: &project,
		environment: &environment,
	};

	match super::config::lease(tx, &services.shipping().vault, &lessee, credentials).await {
		Ok(values) => Ok(Ok(LeasedSecrets::new(values))),
		Err(super::config::LeaseError::Missing(missing)) => {
			Ok(Err(format!(
				"`{}` no longer has {}; set it again and publish again",
				environment.name,
				missing.join(", ")
			)))
		}
		Err(super::config::LeaseError::Unreadable) => {
			Ok(Err("the vault couldn't open a store credential; set it \
			        again, or ask the operator about the vault's keys"
				.to_owned()))
		}
		Err(super::config::LeaseError::Failed(error)) => Err(error),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn named(stores: &[&str]) -> Vec<String> {
		stores.iter().map(|store| (*store).to_owned()).collect()
	}

	#[test]
	fn stores_are_known_once_each_and_in_order() {
		assert_eq!(
			checked_stores(&named(&["play", "chrome"])).unwrap(),
			named(&["chrome", "play"])
		);
		assert!(checked_stores(&[]).is_err());
		assert!(checked_stores(&named(&["chrome", "chrome"])).is_err());
		assert!(checked_stores(&named(&["steam"])).is_err());
		// The macOS download signs with the worker's keychain.
		assert!(checked_stores(&named(&["macos"])).is_err());
	}

	#[test]
	fn each_store_reads_its_own_credentials_once() {
		let names = credential_names(&named(&["play", "solana"]));
		assert_eq!(
			names,
			[
				"LEPTOSKIT_PUBLISH_PLAY_SERVICE_ACCOUNT",
				"LEPTOSKIT_ANDROID_KEYSTORE_BASE64",
				"LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD",
				"LEPTOSKIT_ANDROID_KEY_ALIAS",
				"LEPTOSKIT_ANDROID_KEY_PASSWORD",
				"LEPTOSKIT_PUBLISH_SOLANA_API_KEY",
				"LEPTOSKIT_PUBLISH_SOLANA_KEYPAIR",
				"LEPTOSKIT_SOLANA_KEYSTORE_BASE64",
				"LEPTOSKIT_SOLANA_KEYSTORE_PASSWORD",
				"LEPTOSKIT_SOLANA_KEY_ALIAS",
				"LEPTOSKIT_SOLANA_KEY_PASSWORD",
			],
			"each once, though both stores read Google Play's keystore"
		);
		assert!(
			credential_names(&named(&["chrome"]))
				.iter()
				.all(|name| name.contains("CHROME"))
		);
	}
}
