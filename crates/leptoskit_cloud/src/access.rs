//! Who may do what in an organisation: the roles people hold, the scopes an
//! API token carries, and the one table that turns either into a
//! [`Permission`].
//!
//! Every operation names the permission it needs, and the server decides it
//! here and nowhere else:
//!
//! | Permission            | Level        | owner | admin | developer | billing | viewer | API token        |
//! | --------------------- | ------------ | :---: | :---: | :-------: | :-----: | :----: | ---------------- |
//! | `view_organization`   | organisation |   ✓   |   ✓   |     ✓     |    ✓    |   ✓    | any scope        |
//! | `view_projects`       | project      |   ✓   |   ✓   |     ✓     |    ✓    |   ✓    | any scope        |
//! | `manage_members`      | organisation |   ✓   |   ✓   |           |         |        | never: a person  |
//! | `manage_owners`       | organisation |   ✓   |       |           |         |        | never: a person  |
//! | `manage_tokens`       | organisation |   ✓   |   ✓   |           |         |        | never: a person  |
//! | `create_project`      | organisation |   ✓   |   ✓   |     ✓     |         |        | `manage`         |
//! | `manage_environments` | project      |   ✓   |   ✓   |     ✓     |         |        | `manage`         |
//! | `delete_project`      | project      |   ✓   |   ✓   |           |         |        | `admin`          |
//! | `delete_environment`  | project      |   ✓   |   ✓   |           |         |        | `admin`          |
//! | `deploy`              | project      |   ✓   |   ✓   |     ✓     |         |        | `deploy`         |
//! | `manage_config`       | project      |   ✓   |   ✓   |     ✓     |         |        | `manage`         |
//! | `view_billing`        | organisation |   ✓   |   ✓   |           |    ✓    |        | `billing`        |
//! | `manage_billing`      | organisation |   ✓   |       |           |    ✓    |        | never: a person  |
//!
//! A person, signed in themselves or through the CLI, has their role's
//! permissions. A token has a permission when one of its scopes grants it
//! *and* its creator's current role does, so a token never outlives a
//! demotion. A token bound to one project has no organisation-level
//! permission except `view_organization`, and sees no other project.
//!
//! Some permissions only a person has, whatever a token's scopes
//! ([`Permission::requires_person`]): who is in an organisation, with what
//! role, and which tokens act in it. Those are the authority to act there,
//! and a token that could hand them out could turn itself into a
//! membership or a token that outlives its own scopes, expiry, and
//! revocation.
//!
//! The deletions need the `admin` scope by name: no narrower scope grants
//! them, which is how "a scope that names them" reads for the six scopes
//! the contract has. Shipping (`deploy`) needs the `deploy` scope by name in
//! the same way: uploading an archive, deploying, cancelling, and rolling
//! back. Reading deployments, their events, and logs is `view_projects`.
//!
//! Taking a backup of an environment's database is `deploy` too. Putting
//! one back replaces everything written since, so it needs what deleting
//! the environment does: `delete_environment`.
//!
//! Billing: `view_billing` reads what an organisation has used and what it
//! costs. `manage_billing` changes what it may spend, which is its owners'
//! and its billing members' to decide, and never a token's.
//!
//! Configuration (`manage_config`) is setting and removing an environment's
//! variables and secrets. Reading variables, and the names of secrets, is
//! `view_projects`; nobody reads a secret's value, whatever their role.
//!
//! Two kinds of caller are outside the table. Operators run the cloud
//! itself: a person in the `operators` table, never a token
//! ([`Requirement::Operator`]). Workers call with a worker key, which works
//! on the worker operations and nowhere else, and no other credential works
//! on those ([`Requirement::Worker`]); pairing takes only a pairing code
//! ([`Requirement::Pairing`]).

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
	/// Usage, the bill and the spend cap; reads everything else.
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
			Permission::ViewBilling => matches!(self, Self::Owner | Self::Admin | Self::Billing),
			Permission::ManageBilling => matches!(self, Self::Owner | Self::Billing),
			Permission::ManageMembers
			| Permission::ManageTokens
			| Permission::DeleteProject
			| Permission::DeleteEnvironment => matches!(self, Self::Owner | Self::Admin),
			Permission::CreateProject
			| Permission::ManageEnvironments
			| Permission::Deploy
			| Permission::ManageConfig => {
				matches!(self, Self::Owner | Self::Admin | Self::Developer)
			}
		}
	}

	/// Whether someone with this role may give someone else `role`, by
	/// inviting them or by changing their role, or take it away: managing
	/// members, and for the owner role, being an owner. An invitation is
	/// only as good as this is for its inviter when it is accepted.
	#[must_use]
	pub const fn may_give(self, role: Self) -> bool {
		self.allows(Permission::ManageMembers)
			&& (!matches!(role, Self::Owner) || self.allows(Permission::ManageOwners))
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
	/// Shipping: uploading archives, deploying, cancelling, and rolling back.
	Deploy,
	/// Projects, environments, variables and secrets, and later domains.
	Manage,
	/// Store publishing, when it arrives. Reads, today.
	Publish,
	/// Usage and the bill. The spend cap is a person's to set.
	Billing,
	/// Deleting projects and environments. Members and tokens are a
	/// person's to manage, whatever a token's scopes.
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
	/// creates and changes projects and environments, `admin` deletes them,
	/// and none grants what only a person may do
	/// ([`Permission::requires_person`]).
	#[must_use]
	pub const fn allows(self, permission: Permission) -> bool {
		match permission {
			Permission::ViewOrganization | Permission::ViewProjects => true,
			Permission::CreateProject
			| Permission::ManageEnvironments
			| Permission::ManageConfig => {
				matches!(self, Self::Manage)
			}
			Permission::DeleteProject | Permission::DeleteEnvironment => {
				matches!(self, Self::Admin)
			}
			Permission::Deploy => matches!(self, Self::Deploy),
			Permission::ViewBilling => matches!(self, Self::Billing),
			Permission::ManageMembers
			| Permission::ManageOwners
			| Permission::ManageTokens
			| Permission::ManageBilling => false,
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
	/// Ship an environment: upload archives, deploy, cancel, and roll back.
	Deploy,
	/// Set and remove an environment's variables and secrets, and attach,
	/// verify, and detach its domains.
	ManageConfig,
	/// See what the organisation has used, what it costs, and its spend
	/// cap.
	ViewBilling,
	/// Change the organisation's spend cap.
	ManageBilling,
}

impl Permission {
	/// Every permission.
	pub const ALL: [Self; 13] = [
		Self::ViewOrganization,
		Self::ViewProjects,
		Self::ManageMembers,
		Self::ManageOwners,
		Self::ManageTokens,
		Self::CreateProject,
		Self::ManageEnvironments,
		Self::DeleteProject,
		Self::DeleteEnvironment,
		Self::Deploy,
		Self::ManageConfig,
		Self::ViewBilling,
		Self::ManageBilling,
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
			Self::Deploy => "deploy",
			Self::ManageConfig => "manage_config",
			Self::ViewBilling => "view_billing",
			Self::ManageBilling => "manage_billing",
		}
	}

	/// Whether only a person may have this permission, signed in themselves
	/// or through the CLI: never an API token, whatever its scopes. Who is
	/// in an organisation, with what role, and which tokens act in it are
	/// the authority to act there. A token that could invite someone,
	/// change a role, or make a token could turn itself into a membership
	/// or a credential that outlives its own scopes, expiry, and
	/// revocation, so only people hand those out.
	#[must_use]
	pub const fn requires_person(self) -> bool {
		matches!(
			self,
			Self::ManageMembers | Self::ManageOwners | Self::ManageTokens | Self::ManageBilling
		)
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
				| Self::Deploy
				| Self::ManageConfig
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
	/// One of `leptoskit_rpc`'s own `rpc.*` operations, which every
	/// leptoskit server serves. They reach no organisation: the inbox and
	/// job operations read and change only the caller's own notices and
	/// jobs, of which the cloud has none, and `rpc.service_token` answers
	/// `null` because the cloud forwards nobody's identity to a service.
	Builtin,
	/// An operator: a person in the `operators` table, never a token.
	Operator,
	/// A worker, with its key. Nobody else, whatever their role.
	Worker,
	/// A pairing code in the arguments, which is the credential: anyone
	/// holding a live one may pair a worker with it, once.
	Pairing,
}

/// Every operation the control plane serves, with what it asks of its
/// caller. The authorisation tests run each one as every kind of caller
/// from this table, and fail when the server serves an operation without a
/// row.
pub const OPERATIONS: &[(&str, Requirement)] = &[
	("rpc.inbox", Requirement::Builtin),
	("rpc.mark_read", Requirement::Builtin),
	("rpc.mark_all_read", Requirement::Builtin),
	("rpc.job_status", Requirement::Builtin),
	("rpc.cancel_job", Requirement::Builtin),
	("rpc.service_token", Requirement::Builtin),
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
	(
		"environment_status",
		Requirement::Project(Permission::ViewProjects),
	),
	("begin_upload", Requirement::Project(Permission::Deploy)),
	("complete_upload", Requirement::Project(Permission::Deploy)),
	("deploy", Requirement::Project(Permission::Deploy)),
	(
		"preview_release",
		Requirement::Project(Permission::ViewProjects),
	),
	(
		"cancel_deployment",
		Requirement::Project(Permission::Deploy),
	),
	("rollback", Requirement::Project(Permission::Deploy)),
	("deployment", Requirement::Project(Permission::ViewProjects)),
	(
		"deployments",
		Requirement::Project(Permission::ViewProjects),
	),
	(
		"deployment_events",
		Requirement::Project(Permission::ViewProjects),
	),
	(
		"in_flight",
		Requirement::Organization(Permission::ViewOrganization),
	),
	("build_logs", Requirement::Project(Permission::ViewProjects)),
	(
		"runtime_logs",
		Requirement::Project(Permission::ViewProjects),
	),
	("variables", Requirement::Project(Permission::ViewProjects)),
	(
		"set_variable",
		Requirement::Project(Permission::ManageConfig),
	),
	(
		"unset_variable",
		Requirement::Project(Permission::ManageConfig),
	),
	("secrets", Requirement::Project(Permission::ViewProjects)),
	("set_secret", Requirement::Project(Permission::ManageConfig)),
	(
		"unset_secret",
		Requirement::Project(Permission::ManageConfig),
	),
	("database", Requirement::Project(Permission::ViewProjects)),
	(
		"database_backups",
		Requirement::Project(Permission::ViewProjects),
	),
	("create_backup", Requirement::Project(Permission::Deploy)),
	(
		"restore_backup",
		Requirement::Project(Permission::DeleteEnvironment),
	),
	("domains", Requirement::Project(Permission::ViewProjects)),
	(
		"attach_domain",
		Requirement::Project(Permission::ManageConfig),
	),
	(
		"verify_domain",
		Requirement::Project(Permission::ManageConfig),
	),
	(
		"detach_domain",
		Requirement::Project(Permission::ManageConfig),
	),
	("plans", Requirement::SignedIn),
	("usage", Requirement::Organization(Permission::ViewBilling)),
	(
		"set_spend_cap",
		Requirement::Organization(Permission::ManageBilling),
	),
	("pair", Requirement::Pairing),
	("claim", Requirement::Worker),
	("renew", Requirement::Worker),
	("append_log", Requirement::Worker),
	("finish", Requirement::Worker),
	("report_status", Requirement::Worker),
	("clusters", Requirement::Operator),
	("register_cluster", Requirement::Operator),
	("set_cluster_facts", Requirement::Operator),
	("workers", Requirement::Operator),
	("jobs", Requirement::Operator),
	("issue_pairing", Requirement::Operator),
	("revoke_worker", Requirement::Operator),
	("reconcile", Requirement::Operator),
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
		let expected: [(Permission, [bool; 5]); 11] = [
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
			(Permission::Deploy, [true, true, true, false, false]),
			(Permission::ManageConfig, [true, true, true, false, false]),
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
	fn deploying_needs_the_deploy_scope_by_name() {
		let granting: Vec<TokenScope> = TokenScope::ALL
			.into_iter()
			.filter(|scope| scope.allows(Permission::Deploy))
			.collect();
		assert_eq!(granting, [TokenScope::Deploy]);
	}

	#[test]
	fn configuration_needs_the_manage_scope_by_name() {
		let granting: Vec<TokenScope> = TokenScope::ALL
			.into_iter()
			.filter(|scope| scope.allows(Permission::ManageConfig))
			.collect();
		assert_eq!(granting, [TokenScope::Manage]);
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
					assert!(
						!scope.allows(permission) || role.allows(permission),
						"{role} {scope} {permission:?}"
					);
				}
			}
		}
	}

	#[test]
	fn only_people_manage_members_tokens_and_the_spend_cap() {
		let person_only: Vec<Permission> = Permission::ALL
			.into_iter()
			.filter(|permission| permission.requires_person())
			.collect();
		assert_eq!(
			person_only,
			[
				Permission::ManageMembers,
				Permission::ManageOwners,
				Permission::ManageTokens,
				Permission::ManageBilling
			]
		);

		for scope in TokenScope::ALL {
			for permission in person_only.iter().copied() {
				assert!(!scope.allows(permission), "{scope} {permission:?}");
			}
		}
	}

	#[test]
	fn giving_a_role_is_managing_members_and_owning_for_owner() {
		for giver in Role::ALL {
			for role in Role::ALL {
				let expected = match giver {
					Role::Owner => true,
					Role::Admin => role != Role::Owner,
					Role::Developer | Role::Billing | Role::Viewer => false,
				};
				assert_eq!(giver.may_give(role), expected, "{giver} {role}");
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
		assert_eq!(
			requirement("invite_member"),
			Some(Requirement::Organization(Permission::ManageMembers))
		);
		assert_eq!(requirement("rpc.inbox"), Some(Requirement::Builtin));
		assert_eq!(
			requirement("deploy"),
			Some(Requirement::Project(Permission::Deploy))
		);
		assert_eq!(requirement("claim"), Some(Requirement::Worker));
		assert_eq!(requirement("reconcile"), Some(Requirement::Operator));
		assert_eq!(requirement("ship_it"), None);
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
