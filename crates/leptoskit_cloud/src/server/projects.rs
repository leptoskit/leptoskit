//! The bodies of [`crate::projects`]'s operations.

use leptoskit_db::Json;
use leptoskit_rpc as rpc;

use super::account::check_name;
use super::account::slug_taken;
use super::audit;
use super::audit::Actor;
use super::authorize;
use super::identity::client;
use super::services;
use crate::access::Permission;
use crate::access::RESERVED_ENVIRONMENT_NAMES;
use crate::access::RESERVED_PROJECT_SLUGS;
use crate::projects::CreatedProject;
use crate::projects::Detected;
use crate::projects::EnvironmentKind;
use crate::projects::EnvironmentScale;
use crate::projects::EnvironmentSummary;
use crate::projects::PRODUCTION;
use crate::projects::ProjectDetail;
use crate::projects::ProjectSummary;
use crate::size::Region;
use crate::slug::Slug;
use crate::tables::Environment;
use crate::tables::NewEnvironment;
use crate::tables::NewProject;
use crate::tables::Project;

/// The most projects one organisation may have.
const MAX_PROJECTS: u64 = 200;

/// The most environments one project may have.
const MAX_ENVIRONMENTS: u64 = 50;

pub(crate) async fn projects(
	cx: &rpc::QueryCx,
	organization: &str,
) -> rpc::Result<Vec<ProjectSummary>> {
	let access = authorize::organization(
		cx.db(),
		cx.context(),
		organization,
		Permission::ViewOrganization,
	)
	.await?;
	let organization_id = access.organization.id;
	let bound = access.caller.token().and_then(|token| token.project);
	let rows = Project::query()
		.filter(|project| project.organization_id.eq(organization_id))
		.order_by(|project| project.slug.asc())
		.all(cx.db())
		.await?;
	Ok(rows
		.iter()
		// A project-bound token sees its project and no other.
		.filter(|project| bound.is_none_or(|bound| bound == project.id))
		.map(summary)
		.collect())
}

fn summary(project: &Project) -> ProjectSummary {
	ProjectSummary {
		id: project.id.uuid(),
		slug: project.slug.clone(),
		name: project.name.clone(),
		region: project.region.clone(),
		created_at: project.created_at,
	}
}

fn environment_summary(environment: &Environment) -> EnvironmentSummary {
	EnvironmentSummary {
		id: environment.id.uuid(),
		name: environment.name.clone(),
		kind: environment.kind,
		scale: environment.scale.0.clone(),
		revision: environment.revision,
		scale_differs: environment.scale_differs,
		created_at: environment.created_at,
		updated_at: environment.updated_at,
	}
}

fn detected(project: &Project) -> Detected {
	Detected {
		targets: project.detected_targets.0.clone(),
		variables: project.detected_variables.0.clone(),
	}
}

/// A project's environments, `production` first and then by age.
async fn environments_of(
	db: &impl leptoskit_db::Read,
	project: &Project,
) -> rpc::Result<Vec<EnvironmentSummary>> {
	let project_id = project.id;
	let mut rows = Environment::query()
		.filter(|environment| environment.project_id.eq(project_id))
		.order_by(|environment| environment.created_at.asc())
		.all(db)
		.await?;
	rows.sort_by_key(|environment| environment.kind != EnvironmentKind::Production);
	Ok(rows.iter().map(environment_summary).collect())
}

pub(crate) async fn project(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
) -> rpc::Result<ProjectDetail> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	Ok(ProjectDetail {
		environments: environments_of(cx.db(), &project).await?,
		detected: detected(&project),
		project: summary(&project),
	})
}

/// Checks a slug: the slug rules, and not one of `reserved`.
fn check_slug(slug: &str, reserved: &[&str], what: &str) -> rpc::Result<()> {
	Slug::parse(slug).map_err(|error| rpc::Error::invalid(format!("invalid_slug: {error}")))?;

	if reserved.contains(&slug) {
		return Err(rpc::Error::invalid(format!(
			"invalid_slug: `{slug}` is reserved for the console's own pages, so it can't name \
			 {what}"
		)));
	}

	Ok(())
}

pub(crate) async fn create_project(
	cx: &rpc::MutationCx,
	organization: &str,
	slug: String,
	name: String,
	region: Option<String>,
	detected: Detected,
) -> rpc::Result<CreatedProject> {
	let access = authorize::organization(
		cx.tx(),
		cx.context(),
		organization,
		Permission::CreateProject,
	)
	.await?;
	check_slug(&slug, &RESERVED_PROJECT_SLUGS, "a project")?;
	let name = check_name(&name)?;
	let region = match region {
		Some(region) => region,
		None => services(cx.context())?.default_region().to_owned(),
	};
	let region = Region::parse(&region)
		.map_err(|error| rpc::Error::invalid(format!("invalid_region: {error}")))?;
	let detected = detected
		.checked()
		.map_err(|error| rpc::Error::invalid(format!("invalid_detected: {error}")))?;
	let organization_id = access.organization.id;
	let count = Project::query()
		.filter(|project| project.organization_id.eq(organization_id))
		.count(cx.tx())
		.await?;

	super::billing::refuse_another_project(
		cx.tx(),
		services(cx.context())?.shipping(),
		&access.organization,
	)
	.await?;

	if count >= MAX_PROJECTS {
		return Err(rpc::Error::conflict(format!(
			"too_many_projects: `{organization}` has {MAX_PROJECTS} projects"
		)));
	}

	let project = NewProject {
		organization_id,
		slug: slug.clone(),
		name,
		region: region.as_str().to_owned(),
		detected_targets: Json(detected.targets.clone()),
		detected_variables: Json(detected.variables.clone()),
		created_by: access.caller.account(),
	}
	.insert(cx.tx())
	.await
	.map_err(|error| slug_taken(error, &slug))?;
	let environment = NewEnvironment {
		project_id: project.id,
		name: PRODUCTION.to_owned(),
		kind: EnvironmentKind::Production,
		scale: Json(EnvironmentScale::sleeping()),
		scale_differs: false,
	}
	.insert(cx.tx())
	.await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"project.create",
		Some(organization_id),
		Some(project.id),
		Some(slug),
		&client(cx.context()),
	)
	.await?;

	Ok(CreatedProject {
		project: summary(&project),
		environment: environment_summary(&environment),
		detected,
	})
}

/// `Invalid` unless `confirm` is `expected`, typed back.
fn confirmed(confirm: &str, expected: &str, what: &str) -> rpc::Result<()> {
	if confirm != expected {
		return Err(rpc::Error::invalid(format!(
			"confirmation_mismatch: type the {what}'s name, `{expected}`, as `confirm` to delete \
			 it"
		)));
	}

	Ok(())
}

pub(crate) async fn delete_project(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	confirm: &str,
) -> rpc::Result<()> {
	let (access, row) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::DeleteProject,
	)
	.await?;
	confirmed(confirm, &row.slug, "project")?;
	let organization_id = access.organization.id;
	let project_id = row.id;
	let removals = before_removal(cx, &access.organization, &row).await?;
	let target = format!("{} {}", row.id, row.slug);
	// Its environments and the tokens bound to it go with it.
	row.destroy(cx.tx()).await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"project.delete",
		Some(organization_id),
		Some(project_id),
		Some(target),
		&client(cx.context()),
	)
	.await?;
	wake_after(cx, removals)
}

pub(crate) async fn environments(
	cx: &rpc::QueryCx,
	organization: &str,
	project: &str,
) -> rpc::Result<Vec<EnvironmentSummary>> {
	let (_, project) = authorize::project(
		cx.db(),
		cx.context(),
		organization,
		project,
		Permission::ViewProjects,
	)
	.await?;
	environments_of(cx.db(), &project).await
}

/// The environment `name` in `project`, or `NotFound`.
pub(crate) async fn environment(
	db: &impl leptoskit_db::Read,
	project: &Project,
	name: &str,
) -> rpc::Result<Environment> {
	let project_id = project.id;
	Environment::query()
		.filter(|environment| environment.project_id.eq(project_id) & environment.name.eq(name))
		.first(db)
		.await?
		.ok_or_else(|| {
			rpc::Error::not_found(format!(
				"environment_not_found: `{}` has no environment `{name}`",
				project.slug
			))
		})
}

pub(crate) async fn create_environment(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	name: String,
	kind: EnvironmentKind,
) -> rpc::Result<EnvironmentSummary> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::ManageEnvironments,
	)
	.await?;

	if kind == EnvironmentKind::Production || name == PRODUCTION {
		return Err(rpc::Error::invalid(
			"invalid_environment: a project has one production environment, `production`, which \
			 it was made with",
		));
	}

	check_slug(&name, &RESERVED_ENVIRONMENT_NAMES, "an environment")?;
	let project_id = project.id;
	let count = Environment::query()
		.filter(|environment| environment.project_id.eq(project_id))
		.count(cx.tx())
		.await?;

	if count >= MAX_ENVIRONMENTS {
		return Err(rpc::Error::conflict(format!(
			"too_many_environments: `{}` has {MAX_ENVIRONMENTS} environments",
			project.slug
		)));
	}

	let environment = NewEnvironment {
		project_id,
		name: name.clone(),
		kind,
		scale: Json(EnvironmentScale::sleeping()),
		scale_differs: false,
	}
	.insert(cx.tx())
	.await
	.map_err(|error| {
		match error {
			leptoskit_db::DbError::Conflict { .. } => {
				rpc::Error::conflict(format!(
					"name_taken: `{}` already has an environment `{name}`",
					project.slug
				))
			}
			other => other.into(),
		}
	})?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"environment.create",
		Some(access.organization.id),
		Some(project_id),
		Some(format!("{name} ({kind})")),
		&client(cx.context()),
	)
	.await?;
	Ok(environment_summary(&environment))
}

pub(crate) async fn update_environment(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	revision: i64,
	scale: EnvironmentScale,
) -> rpc::Result<EnvironmentSummary> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::ManageEnvironments,
	)
	.await?;
	let mut row = environment(cx.tx(), &project, environment_name).await?;
	let out_of_date = || {
		rpc::Error::conflict(format!(
			"stale_revision: `{environment_name}` changed since revision {revision}; read it \
			 again and retry"
		))
	};

	if row.revision != revision {
		return Err(out_of_date());
	}

	scale
		.check()
		.map_err(|error| rpc::Error::invalid(format!("invalid_scale: {error}")))?;

	if row.scale.0 != scale {
		row.scale = Json(scale);
		// The console's scale now differs from the last deployment's
		// `leptoskit.toml` until the next deployment reads it.
		row.scale_differs = true;
		row.save(cx.tx()).await.map_err(|error| {
			if error.is_stale() {
				out_of_date()
			} else {
				error.into()
			}
		})?;
		audit::record(
			cx.tx(),
			&Actor::from(&access.caller),
			"environment.update",
			Some(access.organization.id),
			Some(project.id),
			Some(format!("{environment_name} revision {}", row.revision)),
			&client(cx.context()),
		)
		.await?;
	}

	Ok(environment_summary(&row))
}

pub(crate) async fn delete_environment(
	cx: &rpc::MutationCx,
	organization: &str,
	project: &str,
	environment_name: &str,
	confirm: &str,
) -> rpc::Result<()> {
	let (access, project) = authorize::project(
		cx.tx(),
		cx.context(),
		organization,
		project,
		Permission::DeleteEnvironment,
	)
	.await?;
	let row = environment(cx.tx(), &project, environment_name).await?;

	if row.kind == EnvironmentKind::Production {
		return Err(rpc::Error::forbidden(
			"production: a project's production environment can't be deleted; delete the project",
		));
	}

	confirmed(confirm, &row.name, "environment")?;
	let removal =
		super::deployments::before_removal(cx.tx(), &access.organization, &project, &row).await?;
	let target = format!("{} {}", row.id, row.name);
	row.destroy(cx.tx()).await?;
	audit::record(
		cx.tx(),
		&Actor::from(&access.caller),
		"environment.delete",
		Some(access.organization.id),
		Some(project.id),
		Some(target),
		&client(cx.context()),
	)
	.await?;
	wake_after(cx, usize::from(removal.is_some()))
}

/// Readies every environment of `project` for deletion, as
/// [`before_removal`](super::deployments::before_removal) does; answers how
/// many removals it queued.
async fn before_removal(
	cx: &rpc::MutationCx,
	organization: &crate::tables::Organization,
	project: &Project,
) -> rpc::Result<usize> {
	let project_id = project.id;
	let environments = Environment::query()
		.filter(|environment| environment.project_id.eq(project_id))
		.all(cx.tx())
		.await?;
	let mut queued = 0;

	for environment in &environments {
		queued += usize::from(
			super::deployments::before_removal(cx.tx(), organization, project, environment)
				.await?
				.is_some(),
		);
	}

	Ok(queued)
}

/// Wakes the cluster workers waiting to claim once the deletion commits,
/// when it queued removals.
fn wake_after(cx: &rpc::MutationCx, removals: usize) -> rpc::Result<()> {
	if removals > 0 {
		let waker = services(cx.context())?.clone();
		cx.after_commit(async move { super::jobs::wake(&waker) });
	}

	Ok(())
}
