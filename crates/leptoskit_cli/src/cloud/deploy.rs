//! First-class `LeptosKit` Cloud shipping workflow: `launch`, `deploy`,
//! `status`, `logs`, `estimate`, and `open`.

use std::io::IsTerminal as _;
use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::bail;
use leptoskit_cloud::account;
use leptoskit_cloud::archive;
use leptoskit_cloud::deployment::DeploymentState;
use leptoskit_cloud::deployment::Source;
use leptoskit_cloud::deployments;
use leptoskit_cloud::deployments::AppFacts;
use leptoskit_cloud::deployments::DatabaseDialect;
use leptoskit_cloud::deployments::DeployConfig;
use leptoskit_cloud::deployments::HostedService;
use leptoskit_cloud::deployments::ServiceFacts;
use leptoskit_cloud::projects;
use leptoskit_config::DbDialect;
use leptoskit_config::DbLiveCapture;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::ResolvedService;
use serde_json::Value;
use serde_json::json;

use super::CloudArgs;
use super::Context;
use super::open_browser;
use super::printable;
use super::ship::announce;
use super::ship::environment;
use super::slug;
use super::stores::refuse_root;
use super::stores::upload;
use super::value;
use super::write_link;

/// How often a followed deployment or log stream polls the control plane.
const POLL: Duration = Duration::from_secs(2);

/// Derives control-plane [`AppFacts`] from a loaded `leptoskit.toml`.
#[must_use]
pub(crate) fn app_facts(config: &LeptoskitConfig) -> AppFacts {
	let database = config.db.as_ref().map(|db| {
		match db.dialect {
			DbDialect::Sqlite => DatabaseDialect::Sqlite,
			DbDialect::Postgres => DatabaseDialect::Postgres,
		}
	});
	let live = config.rpc.live.relay_url_env.is_some()
		|| config.rpc.live_path.is_some()
		|| config
			.db
			.as_ref()
			.is_some_and(|db| db.live_capture == DbLiveCapture::Replication);
	let services = config
		.services
		.iter()
		.map(|(name, service)| {
			(
				name.clone(),
				ServiceFacts {
					remote: service.url.is_some(),
					public: service.public.unwrap_or(false),
					isolated: service.trust == Some(leptoskit_config::ServiceTrust::Isolated),
					database: service.database.unwrap_or(false),
					// As `[services]` settles it; a table that doesn't
					// resolve is refused when the app is built.
					forward_identity: ResolvedService::merge(name, service, None)
						.is_ok_and(|resolved| resolved.forward_identity),
					hosted: hosted(service),
				},
			)
		})
		.collect();
	AppFacts {
		database,
		live,
		services,
	}
}

/// The service the cloud serves itself that `service` is, if any: a
/// registry `crate` it knows, fetched from the registry rather than Git or a
/// path, and run by the cloud rather than elsewhere.
fn hosted(service: &leptoskit_config::ServiceConfig) -> Option<HostedService> {
	if service.url.is_some()
		|| service.git.is_some()
		|| service.path.is_some()
		|| service.package.is_some()
	{
		return None;
	}

	service.krate.as_deref().and_then(HostedService::for_crate)
}

fn load_full_config(context: &Context) -> Result<(LeptoskitConfig, &Path)> {
	let file = context
		.file
		.as_deref()
		.context("Run this command inside a LeptosKit project containing leptoskit.toml.")?;
	let root = file
		.parent()
		.context("Could not resolve project root from leptoskit.toml.")?;
	let config = LeptoskitConfig::load_from(root)?;
	Ok((config, root))
}

fn local_hostname() -> Option<String> {
	let raw = std::env::var("HOSTNAME")
		.ok()
		.or_else(|| std::fs::read_to_string("/etc/hostname").ok())?;
	let cleaned = raw.trim();
	deployments::check_host(cleaned)
		.ok()
		.map(|()| cleaned.to_owned())
}

/// Estimates cost and shows the resolved release plan without deploying.
pub(super) fn estimate(args: &CloudArgs, context: &Context) -> Result<Value> {
	let (full_config, _root) = load_full_config(context)?;
	let cloud = full_config.cloud.clone().unwrap_or_default();
	let resolved = cloud
		.resolved(&full_config)
		.map_err(|error| anyhow::anyhow!("Invalid [cloud] configuration: {error}"))?;

	let project = args
		.project
		.clone()
		.or_else(|| cloud.project.as_ref().map(ToString::to_string));

	// If linked to a project and authenticated, ask the control plane for the
	// live price catalogue preview; otherwise return the local resolved plan.
	if let Some(project) = project
		&& let Ok(organization) = context.organization(args)
	{
		let environment = environment(args);
		if let Ok(preview) = context.client.preview_release(deployments::PreviewRelease {
			organization,
			project,
			environment,
			config: Some(DeployConfig {
				cloud: cloud.clone(),
				app: app_facts(&full_config),
			}),
			rollback_to: None,
		}) {
			return value(preview);
		}
	}

	Ok(json!({
		"mode": "local_plan",
		"resolved": resolved,
		"note": "Sign in (`lk cloud login`) and link a project (`lk cloud launch`) for live pricing from the control plane's price catalogue."
	}))
}

/// First-time launch: creates & links the cloud project if needed, previews
/// the release, and ships the first deployment.
pub(super) fn launch(
	args: &CloudArgs,
	context: &Context,
	name: Option<&str>,
	region: Option<&str>,
	yes: bool,
	detach: bool,
) -> Result<Value> {
	let (full_config, root) = load_full_config(context)?;
	refuse_root(root)?;
	let mut cloud = full_config.cloud.clone().unwrap_or_default();
	cloud
		.resolved(&full_config)
		.map_err(|error| anyhow::anyhow!("Invalid [cloud] configuration: {error}"))?;

	let organization = context.organization(args)?;
	let personal = context
		.client
		.organizations(account::Organizations {})?
		.iter()
		.any(|org| org.slug == organization && org.personal);

	let project_slug = if let Some(existing) = args
		.project
		.clone()
		.or_else(|| cloud.project.as_ref().map(ToString::to_string))
	{
		existing
	} else {
		let display_name = name.unwrap_or(&full_config.app.name);
		let desired_slug = slug(display_name);
		if !args.json {
			anstream::eprintln!(
				"Creating project {}/{}…",
				printable(&organization),
				printable(&desired_slug)
			);
		}
		let created = context.client.create_project(projects::CreateProject {
			organization: organization.clone(),
			slug: desired_slug.clone(),
			name: display_name.to_owned(),
			region: region
				.map(str::to_owned)
				.or_else(|| cloud.region.as_ref().map(ToString::to_string)),
			detected: projects::Detected {
				targets: full_config
					.app
					.platforms
					.iter()
					.map(|platform| format!("{platform:?}").to_lowercase())
					.collect(),
				variables: Vec::new(),
			},
		})?;
		let file = context
			.file
			.as_ref()
			.context("leptoskit.toml is required to record [cloud].project")?;
		write_link(
			file,
			&organization,
			&created.project.slug,
			&context.url,
			personal,
			false,
		)?;
		cloud.project = Some(
			created
				.project
				.slug
				.parse()
				.map_err(|error| anyhow::anyhow!("Invalid project slug: {error}"))?,
		);
		created.project.slug
	};

	let env_name = environment(args);
	let deploy_config = DeployConfig {
		cloud,
		app: app_facts(&full_config),
	};

	let preview = context
		.client
		.preview_release(deployments::PreviewRelease {
			organization: organization.clone(),
			project: project_slug.clone(),
			environment: env_name.clone(),
			config: Some(deploy_config.clone()),
			rollback_to: None,
		})?;

	if !yes && !args.json && std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
		anstream::eprintln!(
			"Launch plan for {}/{}/{}:",
			printable(&organization),
			printable(&project_slug),
			printable(&env_name)
		);
		for line in &preview.lines {
			anstream::eprintln!("  - {:?}", line.kind);
		}
		match &preview.cost {
			deployments::PreviewCost::Estimate(estimate) => {
				anstream::eprintln!(
					"  Estimated monthly total ({}): {} {}",
					estimate.plan,
					estimate.total,
					estimate.currency
				);
			}
			deployments::PreviewCost::Unpriced { reason } => {
				anstream::eprintln!("  Pricing: {}", printable(reason));
			}
		}
		anstream::eprint!("Continue with launch? [y/N] ");
		std::io::stderr().flush()?;
		let mut answer = String::new();
		std::io::stdin().read_line(&mut answer)?;
		let trimmed = answer.trim().to_ascii_lowercase();
		if trimmed != "y" && trimmed != "yes" {
			bail!("Launch cancelled.");
		}
	}

	ship(
		args,
		context,
		root,
		&organization,
		&project_slug,
		&env_name,
		deploy_config,
		detach,
	)
}

/// Packs, uploads, and deploys the current project to `LeptosKit` Cloud.
pub(super) fn deploy(args: &CloudArgs, context: &Context, detach: bool) -> Result<Value> {
	let (full_config, root) = load_full_config(context)?;
	refuse_root(root)?;
	let cloud = full_config.cloud.clone().unwrap_or_default();
	cloud
		.resolved(&full_config)
		.map_err(|error| anyhow::anyhow!("Invalid [cloud] configuration: {error}"))?;
	let organization = context.organization(args)?;
	let project = context.project(args)?;
	let env_name = environment(args);
	let deploy_config = DeployConfig {
		cloud,
		app: app_facts(&full_config),
	};

	ship(
		args,
		context,
		root,
		&organization,
		&project,
		&env_name,
		deploy_config,
		detach,
	)
}

#[expect(
	clippy::too_many_arguments,
	reason = "internal helper taking resolved org/project/env/config"
)]
fn ship(
	args: &CloudArgs,
	context: &Context,
	root: &Path,
	organization: &str,
	project: &str,
	env_name: &str,
	config: DeployConfig,
	detach: bool,
) -> Result<Value> {
	announce(args, organization, project, env_name, "Deploying");
	let packed = archive::pack(root, &archive::PackOptions::default())
		.map_err(|error| anyhow::anyhow!("Could not pack the project: {error}."))?;
	if !args.json {
		anstream::eprintln!(
			"Packed {} files ({} bytes) as {}.",
			packed.files.len(),
			packed.bytes.len(),
			packed.digest.as_str()
		);
	}
	upload(context, organization, project, &packed)?;

	let summary = context.client.deploy(deployments::Deploy {
		organization: organization.to_owned(),
		project: project.to_owned(),
		environment: env_name.to_owned(),
		source: Source::Upload {
			archive: packed.digest,
		},
		config,
		host: local_hostname(),
	})?;

	if detach {
		return value(summary);
	}

	if !args.json {
		anstream::eprintln!(
			"Deployment #{} queued. Following build and rollout (Ctrl-C stops watching, not the \
			 deployment)…",
			summary.number
		);
	}
	follow_deployment(
		args,
		context,
		organization,
		project,
		env_name,
		summary.number,
	)
}

fn follow_deployment(
	args: &CloudArgs,
	context: &Context,
	organization: &str,
	project: &str,
	env_name: &str,
	number: u64,
) -> Result<Value> {
	let mut after = None;
	loop {
		let sheet = context.client.deployment(deployments::Deployment {
			organization: organization.to_owned(),
			project: project.to_owned(),
			environment: env_name.to_owned(),
			number,
		})?;

		loop {
			let page = context.client.build_logs(deployments::BuildLogs {
				organization: organization.to_owned(),
				project: project.to_owned(),
				environment: env_name.to_owned(),
				number,
				after,
				limit: None,
			})?;
			if !args.json {
				for line in &page.lines {
					anstream::eprintln!("{}", printable(&line.text));
				}
			}
			match page.next {
				Some(next) if !page.lines.is_empty() => after = Some(next),
				_ => break,
			}
		}

		match &sheet.deployment.state {
			DeploymentState::Live | DeploymentState::Superseded => {
				let status = context
					.client
					.environment_status(projects::EnvironmentStatus {
						organization: organization.to_owned(),
						project: project.to_owned(),
						environment: env_name.to_owned(),
					})
					.ok();
				let url = status
					.as_ref()
					.and_then(|s| s.hostname.as_deref())
					.map(|host| format!("https://{host}"));
				if !args.json {
					if let Some(url) = &url {
						anstream::eprintln!("Live at {url}");
					} else {
						anstream::eprintln!("Deployment #{number} is live.");
					}
				}
				return Ok(json!({
					"deployment": sheet.deployment,
					"url": url,
				}));
			}
			DeploymentState::Failed { stage, blocker } => {
				bail!("Deployment #{number} failed during {stage:?} ({blocker:?}).");
			}
			DeploymentState::Cancelled => {
				bail!("Deployment #{number} was cancelled.");
			}
			DeploymentState::Queued
			| DeploymentState::Building
			| DeploymentState::Migrating
			| DeploymentState::RollingOut => {
				std::thread::sleep(POLL);
			}
		}
	}
}

/// Shows observed process status and hostname for an environment.
pub(super) fn status(args: &CloudArgs, context: &Context) -> Result<Value> {
	let organization = context.organization(args)?;
	let project = context.project(args)?;
	let env_name = environment(args);
	value(
		context
			.client
			.environment_status(projects::EnvironmentStatus {
				organization,
				project,
				environment: env_name,
			})?,
	)
}

/// Reads (and optionally follows) runtime logs for an environment.
pub(super) fn logs(
	args: &CloudArgs,
	context: &Context,
	follow: bool,
	process: Option<String>,
	since_minutes: Option<u32>,
	contains: Option<String>,
	limit: Option<u32>,
) -> Result<Value> {
	let organization = context.organization(args)?;
	let project = context.project(args)?;
	let env_name = environment(args);

	if !follow {
		return value(context.client.runtime_logs(deployments::RuntimeLogs {
			organization,
			project,
			environment: env_name,
			process,
			since_minutes,
			contains,
			after: None,
			limit,
		})?);
	}

	let mut after = None;
	loop {
		let page = context.client.runtime_logs(deployments::RuntimeLogs {
			organization: organization.clone(),
			project: project.clone(),
			environment: env_name.clone(),
			process: process.clone(),
			since_minutes,
			contains: contains.clone(),
			after,
			limit,
		})?;
		for line in &page.lines {
			if args.json {
				anstream::println!("{}", serde_json::to_string(line)?);
			} else {
				let prefix = line.process.as_deref().unwrap_or("app");
				anstream::println!("[{prefix}] {}", printable(&line.text));
			}
		}
		if let Some(next) = page.next
			&& !page.lines.is_empty()
		{
			after = Some(next);
		}
		std::thread::sleep(POLL);
	}
}

/// Opens (or prints) the live URL of the selected environment.
pub(super) fn open(args: &CloudArgs, context: &Context, print_only: bool) -> Result<Value> {
	let organization = context.organization(args)?;
	let project = context.project(args)?;
	let env_name = environment(args);
	let status = context
		.client
		.environment_status(projects::EnvironmentStatus {
			organization,
			project,
			environment: env_name,
		})?;
	let host = status
		.hostname
		.context("This environment does not have a live hostname yet. Run `lk cloud deploy`.")?;
	let url = format!("https://{host}");
	if !print_only
		&& !args.json
		&& std::io::stdin().is_terminal()
		&& std::io::stdout().is_terminal()
	{
		open_browser(&url);
	}
	Ok(json!({ "url": url }))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn app_facts_extracts_database_live_and_services() {
		let toml = r#"
schema_version = 0

[app]
name = "demo"

[db]
dialect = "postgres"
live_capture = "replication"

[services.worker]
package = "demo_worker"
public = true
database = true
trust = "isolated"

[services.external]
url = "https://api.example.com"
"#;
		let config = LeptoskitConfig::from_toml(toml, Path::new(".")).unwrap();
		let facts = app_facts(&config);
		assert_eq!(facts.database, Some(DatabaseDialect::Postgres));
		assert!(facts.live);
		let worker = facts.services.get("worker").expect("worker service");
		assert!(!worker.remote);
		assert!(worker.public);
		assert!(worker.isolated);
		assert!(worker.database);
		let external = facts.services.get("external").expect("external service");
		assert!(external.remote);
		assert!(!external.public);
	}
}
