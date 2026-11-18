//! Who may do what in an organisation: the roles people hold, the scopes an
//! API token carries, and the one table that turns either into a
//! [`Permission`].
//!
//! Every operation names the permission it needs, and the server decides it
//! here and nowhere else:
//!
//! | Permission            | Level        | owner | admin | developer | billing | viewer | Token scope |
//! | --------------------- | ------------ | :---: | :---: | :-------: | :-----: | :----: | ----------- |
//! | `view_organization`   | organisation |   ✓   |   ✓   |     ✓     |    ✓    |   ✓    | any         |
//! | `view_projects`       | project      |   ✓   |   ✓   |     ✓     |    ✓    |   ✓    | any         |
//! | `manage_members`      | organisation |   ✓   |   ✓   |           |         |        | `admin`     |
//! | `manage_owners`       | organisation |   ✓   |       |           |         |        | `admin`     |
//! | `manage_tokens`       | organisation |   ✓   |   ✓   |           |         |        | `admin`     |
//! | `create_project`      | organisation |   ✓   |   ✓   |     ✓     |         |        | `manage`    |
//! | `manage_environments` | project      |   ✓   |   ✓   |     ✓     |         |        | `manage`    |
//! | `delete_project`      | project      |   ✓   |   ✓   |           |         |        | `admin`     |
//! | `delete_environment`  | project      |   ✓   |   ✓   |           |         |        | `admin`     |
//!
//! A person has their role's permissions. A token has a permission when one
//! of its scopes grants it *and* its creator's current role does, so a
//! token never outlives a demotion. A token bound to one project has no
//! organisation-level permission except `view_organization`, and sees no
//! other project.
//!
//! The deletions need the `admin` scope by name: no narrower scope grants
//! them, which is how "a scope that names them" reads for the six scopes
//! the contract has.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// What someone is in an organisation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, leptoskit_db::Enum)]
pub enum Role {
	/// Everything, including the owner role itself.
	#[db(rename = "owner")]
	Owner,
	/// Everything but the owner role: members, tokens, projects.
	#[db(rename = "admin")]
	Admin,
	/// Projects and environments, but not who is in the organisation.
	#[db(rename = "developer")]
	Developer,
	/// The plan and invoices, when billing arrives; reads everything else.
	#[db(rename = "billing")]
	Billing,
	/// Reads everything except secret values.
	#[db(rename = "viewer")]
	Viewer,
}

impl Role {
	/// Every role, strongest first.
	pub const ALL: [Self; 5] = [
		Self::Owner,
		Self::Admin,
		Self::Developer,
		Self::Billing,
		Self::Viewer,
	];

	/// Whether this role has `permission`: the table in the [module
	/// docs](self).
	#[must_use]
	pub const fn allows(self, permission: Permission) -> bool {
		match permission {
			Permission::ViewOrganization | Permission::ViewProjects => true,
			Permission::ManageOwners => matches!(self, Self::Owner),
			Permission::ManageMembers
			| Permission::ManageTokens
			| Permission::DeleteProject
			| Permission::DeleteEnvironment => matches!(self, Self::Owner | Self::Admin),
			Permission::CreateProject | Permission::ManageEnvironments => {
				matches!(self, Self::Owner | Self::Admin | Self::Developer)
			}
		}
	}

	/// Whether someone with this role may give a token `scope`: owners any,
	/// admins all but `billing`, developers what they do day to day, and
	/// billing and viewers what they can already see or pay for.
	#[must_use]
	pub const fn grants(self, scope: TokenScope) -> bool {
		match self {
			Self::Owner => true,
			Self::Admin => !matches!(scope, TokenScope::Billing),
			Self::Developer => {
				matches!(
					scope,
					TokenScope::Read
						| TokenScope::Deploy
						| TokenScope::Manage
						| TokenScope::Publish
				)
			}
			Self::Billing => matches!(scope, TokenScope::Read | TokenScope::Billing),
			Self::Viewer => matches!(scope, TokenScope::Read),
		}
	}
}

/// What an API token may do, as the contract names the scopes.
#[derive(
	Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TokenScope {
	/// Read the organisation, its projects, and their environments.
	Read,
	/// Deployments, when they arrive. Reads, today.
	Deploy,
	/// Projects, environments, and later configuration and domains.
	Manage,
	/// Store publishing, when it arrives. Reads, today.
	Publish,
	/// The plan and invoices, when billing arrives. Reads, today.
	Billing,
	/// Members, tokens, and deleting projects and environments.
	Admin,
}

impl TokenScope {
	/// Every scope.
	pub const ALL: [Self; 6] = [
		Self::Read,
		Self::Deploy,
		Self::Manage,
		Self::Publish,
		Self::Billing,
		Self::Admin,
	];

	/// The scope as the wire spells it.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Read => "read",
			Self::Deploy => "deploy",
			Self::Manage => "manage",
			Self::Publish => "publish",
			Self::Billing => "billing",
			Self::Admin => "admin",
		}
	}

	/// Whether this scope grants `permission`: every scope reads, `manage`
	/// creates and changes projects and environments, and `admin` manages
	/// people and tokens and deletes.
	#[must_use]
	pub const fn allows(self, permission: Permission) -> bool {
		match permission {
			Permission::ViewOrganization | Permission::ViewProjects => true,
			Permission::CreateProject | Permission::ManageEnvironments => {
				matches!(self, Self::Manage)
			}
			Permission::ManageMembers
			| Permission::ManageOwners
			| Permission::ManageTokens
			| Permission::DeleteProject
			| Permission::DeleteEnvironment => matches!(self, Self::Admin),
		}
	}
}

impl std::fmt::Display for TokenScope {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str(self.name())
	}
}

/// One thing an operation needs the caller to be allowed to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
	/// See the organisation and who is in it.
	ViewOrganization,
	/// See projects and their environments.
	ViewProjects,
	/// Invite people, list and revoke invitations, and change or remove
	/// members below owner.
	ManageMembers,
	/// Make someone an owner, or demote or remove an owner.
	ManageOwners,
	/// List, create, and revoke API tokens.
	ManageTokens,
	/// Create a project.
	CreateProject,
	/// Create environments and change their scale.
	ManageEnvironments,
	/// Delete a project, with every environment in it.
	DeleteProject,
	/// Delete an environment other than production.
	DeleteEnvironment,
}

impl Permission {
	/// Every permission.
	pub const ALL: [Self; 9] = [
		Self::ViewOrganization,
		Self::ViewProjects,
		Self::ManageMembers,
		Self::ManageOwners,
		Self::ManageTokens,
		Self::CreateProject,
		Self::ManageEnvironments,
		Self::DeleteProject,
		Self::DeleteEnvironment,
	];

	/// The permission's name, as errors and the docs spell it.
	#[must_use]
	pub const fn name(self) -> &'static str {
		match self {
			Self::ViewOrganization => "view_organization",
			Self::ViewProjects => "view_projects",
			Self::ManageMembers => "manage_members",
			Self::ManageOwners => "manage_owners",
			Self::ManageTokens => "manage_tokens",
			Self::CreateProject => "create_project",
			Self::ManageEnvironments => "manage_environments",
			Self::DeleteProject => "delete_project",
			Self::DeleteEnvironment => "delete_environment",
		}
	}

	/// Whether the permission is about one project, which a project-bound
	/// token may have for its own project, rather than the organisation as
	/// a whole.
	#[must_use]
	pub const fn is_project_level(self) -> bool {
		matches!(
			self,
			Self::ViewProjects
				| Self::ManageEnvironments
				| Self::DeleteProject
				| Self::DeleteEnvironment
		)
	}

	/// Whether a token bound to one project may have this permission: the
	/// project-level ones, and seeing the organisation it belongs to.
	#[must_use]
	pub const fn reaches_project_tokens(self) -> bool {
		self.is_project_level() || matches!(self, Self::ViewOrganization)
	}
}

/// What an operation asks of its caller before it reads anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
	/// Anyone signed in: a person, or a live API token.
	SignedIn,
	/// A person, never a token: creating an organisation and accepting an
	/// invitation act for someone, not for an organisation.
	Person,
	/// `permission` in the organisation the arguments name.
	Organization(Permission),
	/// `permission` in the project the arguments name.
	Project(Permission),
}

/// Every operation the control plane serves, with what it asks of its
/// caller. The authorisation tests run each one as every kind of caller
/// from this table, and fail when an operation is served without a row.
pub const OPERATIONS: &[(&str, Requirement)] = &[
	("whoami", Requirement::SignedIn),
	("organizations", Requirement::SignedIn),
	("create_organization", Requirement::Person),
	(
		"members",
		Requirement::Organization(Permission::ViewOrganization),
	),
	(
		"invite_member",
		Requirement::Organization(Permission::ManageMembers),
	),
	("accept_invitation", Requirement::Person),
	(
		"invitations",
		Requirement::Organization(Permission::ManageMembers),
	),
	(
		"revoke_invitation",
		Requirement::Organization(Permission::ManageMembers),
	),
	(
		"set_member_role",
		Requirement::Organization(Permission::ManageMembers),
	),
	(
		"remove_member",
		Requirement::Organization(Permission::ManageMembers),
	),
	(
		"tokens",
		Requirement::Organization(Permission::ManageTokens),
	),
	(
		"create_token",
		Requirement::Organization(Permission::ManageTokens),
	),
	(
		"revoke_token",
		Requirement::Organization(Permission::ManageTokens),
	),
	(
		"projects",
		Requirement::Organization(Permission::ViewOrganization),
	),
	("project", Requirement::Project(Permission::ViewProjects)),
	(
		"create_project",
		Requirement::Organization(Permission::CreateProject),
	),
	(
		"delete_project",
		Requirement::Project(Permission::DeleteProject),
	),
	(
		"environments",
		Requirement::Project(Permission::ViewProjects),
	),
	(
		"create_environment",
		Requirement::Project(Permission::ManageEnvironments),
	),
	(
		"update_environment",
		Requirement::Project(Permission::ManageEnvironments),
	),
	(
		"delete_environment",
		Requirement::Project(Permission::DeleteEnvironment),
	),
];

/// What `name` asks of its caller, from [`OPERATIONS`].
#[must_use]
pub fn requirement(name: &str) -> Option<Requirement> {
	OPERATIONS
		.iter()
		.find(|(operation, _)| *operation == name)
		.map(|(_, requirement)| *requirement)
}

/// Organisation slugs the console's own routes use, so `/<org>` can never
/// shadow one: `/sign-in`, `/device`, `/welcome`, and the rest.
pub const RESERVED_ORGANIZATION_SLUGS: [&str; 8] = [
	"sign-in",
	"device",
	"welcome",
	"invitations",
	"todo",
	"account",
	"organizations",
	"operator",
];

/// Project slugs the console's `/<org>/…` routes use.
pub const RESERVED_PROJECT_SLUGS: [&str; 2] = ["new", "settings"];

/// Environment names the console's `/<org>/<project>/…` routes use.
pub const RESERVED_ENVIRONMENT_NAMES: [&str; 7] = [
	"releases", "stores", "publish", "sign", "settings", "ship", "logs",
];

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_table_is_the_module_docs() {
		let expected: [(Permission, [bool; 5]); 9] = [
			(Permission::ViewOrganization, [true, true, true, true, true]),
			(Permission::ViewProjects, [true, true, true, true, true]),
			(Permission::ManageMembers, [true, true, false, false, false]),
			(Permission::ManageOwners, [true, false, false, false, false]),
			(Permission::ManageTokens, [true, true, false, false, false]),
			(Permission::CreateProject, [true, true, true, false, false]),
			(
				Permission::ManageEnvironments,
				[true, true, true, false, false],
			),
			(Permission::DeleteProject, [true, true, false, false, false]),
			(
				Permission::DeleteEnvironment,
				[true, true, false, false, false],
			),
		];

		for (permission, roles) in expected {
			for (role, allowed) in Role::ALL.into_iter().zip(roles) {
				assert_eq!(role.allows(permission), allowed, "{role} {permission:?}");
			}
		}
	}

	#[test]
	fn deleting_needs_the_admin_scope_by_name() {
		for permission in [Permission::DeleteProject, Permission::DeleteEnvironment] {
			let granting: Vec<TokenScope> = TokenScope::ALL
				.into_iter()
				.filter(|scope| scope.allows(permission))
				.collect();
			assert_eq!(granting, [TokenScope::Admin], "{permission:?}");
		}
	}

	#[test]
	fn every_scope_reads_and_none_grants_more_than_its_role_could() {
		for scope in TokenScope::ALL {
			assert!(scope.allows(Permission::ViewOrganization));
			assert!(scope.allows(Permission::ViewProjects));
		}

		// A role may only give scopes whose permissions it has itself, so a
		// token it mints can never do more than its creator.
		for role in Role::ALL {
			for scope in TokenScope::ALL
				.into_iter()
				.filter(|scope| role.grants(*scope))
			{
				for permission in Permission::ALL {
					if scope.allows(permission) && !role.allows(permission) {
						// `admin` names member management; an admin's
						// `admin` token still can't touch owners, because
						// the role check runs on every call.
						assert_eq!(scope, TokenScope::Admin, "{role} {scope} {permission:?}");
					}
				}
			}
		}
	}

	#[test]
	fn every_operation_has_one_row() {
		let mut names: Vec<&str> = OPERATIONS.iter().map(|(name, _)| *name).collect();
		names.sort_unstable();
		let count = names.len();
		names.dedup();
		assert_eq!(names.len(), count, "an operation is listed twice");
		assert_eq!(
			requirement("delete_project"),
			Some(Requirement::Project(Permission::DeleteProject))
		);
		assert_eq!(requirement("deploy"), None);
	}

	#[test]
	fn roles_and_scopes_spell_themselves_like_the_wire() {
		for role in Role::ALL {
			assert_eq!(
				serde_json::to_string(&role).unwrap(),
				format!("\"{}\"", role.as_str())
			);
		}

		for scope in TokenScope::ALL {
			assert_eq!(
				serde_json::to_string(&scope).unwrap(),
				format!("\"{}\"", scope.name())
			);
		}
	}
}
