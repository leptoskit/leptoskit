//! The one place an operation learns whether its caller may act: it names
//! an organisation (and maybe a project) by slug and a [`Permission`], and
//! gets back the rows, read again from the database, or an error.
//!
//! The order of the checks is the rule: first whether the caller can *see*
//! the organisation or project at all, which answers "not found" with the
//! same words whether it exists or not, and only then whether they may do
//! this, which answers "forbidden". So nothing outside a tenant learns what
//! is inside it.

use leptoskit_db::Context;
use leptoskit_rpc as rpc;

use super::identity::Caller;
use super::identity::caller;
use crate::access::Permission;
use crate::access::Role;
use crate::tables::Membership;
use crate::tables::Organization;
use crate::tables::Project;

/// What a caller may do in an organisation, after the checks.
#[derive(Clone, Debug)]
pub(crate) struct Access {
	pub(crate) caller: Caller,
	pub(crate) organization: Organization,
	/// The person's role, or the role of the person a token acts for, as
	/// it is now.
	pub(crate) role: Role,
}

impl Access {
	/// `Forbidden` unless the caller has `permission`, by role and, for a
	/// token, by scope and project binding; saying why.
	pub(crate) fn require(&self, permission: Permission) -> rpc::Result<()> {
		if !self.role.allows(permission) {
			return Err(rpc::Error::forbidden(format!(
				"insufficient_role: a {} of `{}` can't {}",
				self.role,
				self.organization.slug,
				describe(permission)
			)));
		}

		if let Some(token) = self.caller.token() {
			if !token.scopes.iter().any(|scope| scope.allows(permission)) {
				return Err(rpc::Error::forbidden(format!(
					"insufficient_scope: this token's scopes don't let it {}",
					describe(permission)
				)));
			}

			if token.project.is_some() && !permission.reaches_project_tokens() {
				return Err(rpc::Error::forbidden(format!(
					"project_token: a token bound to one project can't {}",
					describe(permission)
				)));
			}
		}

		Ok(())
	}
}

/// The organisation `slug` names, if the caller can see it, with `permission`
/// checked.
///
/// # Errors
///
/// `Unauthorized` for an anonymous caller; `NotFound` for an organisation
/// they can't see; `Forbidden` when they may not do this.
pub(crate) async fn organization(
	db: &impl leptoskit_db::Read,
	context: &Context,
	slug: &str,
	permission: Permission,
) -> rpc::Result<Access> {
	let access = visible_organization(db, context, slug).await?;
	access.require(permission)?;
	Ok(access)
}

/// The project `project` in the organisation `organization`, if the caller
/// can see both, with `permission` checked.
///
/// # Errors
///
/// As [`organization`], and `NotFound` for a project they can't see, which
/// includes every other project for a project-bound token.
pub(crate) async fn project(
	db: &impl leptoskit_db::Read,
	context: &Context,
	organization: &str,
	project: &str,
	permission: Permission,
) -> rpc::Result<(Access, Project)> {
	let access = visible_organization(db, context, organization).await?;
	let not_found = || {
		rpc::Error::not_found(format!(
			"project_not_found: there is no project `{project}` in `{organization}` that you can \
			 see"
		))
	};
	let organization_id = access.organization.id;
	let row = Project::query()
		.filter(|row| row.organization_id.eq(organization_id) & row.slug.eq(project))
		.first(db)
		.await?
		.ok_or_else(not_found)?;

	if access
		.caller
		.token()
		.and_then(|token| token.project)
		.is_some_and(|bound| bound != row.id)
	{
		return Err(not_found());
	}

	access.require(permission)?;
	Ok((access, row))
}

/// The organisation `slug` names, if the caller is in it or it's their
/// token's, with the caller's current role. Nothing about permissions yet.
pub(crate) async fn visible_organization(
	db: &impl leptoskit_db::Read,
	context: &Context,
	slug: &str,
) -> rpc::Result<Access> {
	let caller = caller(context)?.clone();
	let not_found = || {
		rpc::Error::not_found(format!(
			"organization_not_found: there is no organisation `{slug}` that you can see"
		))
	};
	let organization = Organization::query()
		.filter(|organization| organization.slug.eq(slug))
		.first(db)
		.await?
		.ok_or_else(not_found)?;

	if let Some(token) = caller.token()
		&& token.organization != organization.id
	{
		return Err(not_found());
	}

	let account = caller.account();
	let organization_id = organization.id;
	let membership = Membership::query()
		.filter(|member| member.organization_id.eq(organization_id) & member.account_id.eq(account))
		.first(db)
		.await?
		.ok_or_else(not_found)?;

	Ok(Access {
		caller,
		organization,
		role: membership.role,
	})
}

/// What a permission lets someone do, for errors.
fn describe(permission: Permission) -> &'static str {
	match permission {
		Permission::ViewOrganization => "see this organisation",
		Permission::ViewProjects => "see its projects",
		Permission::ManageMembers => "manage its members or invitations",
		Permission::ManageOwners => "give, change, or take away the owner role",
		Permission::ManageTokens => "manage its API tokens",
		Permission::CreateProject => "create projects",
		Permission::ManageEnvironments => "create or change environments",
		Permission::DeleteProject => "delete projects",
		Permission::DeleteEnvironment => "delete environments",
	}
}
