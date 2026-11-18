//! The authorisation matrix, from the table in `access::OPERATIONS`: every
//! operation, called by every role, by a token with each scope, and by
//! nobody, is allowed exactly when the table says. The operations the
//! server publishes must be the table's, so a new operation without a
//! decided permission fails here.

#![cfg(not(target_arch = "wasm32"))]

mod support;

use std::collections::BTreeSet;

use leptoskit_cloud::access::OPERATIONS;
use leptoskit_cloud::access::Requirement;
use leptoskit_cloud::access::Role;
use leptoskit_cloud::access::TokenScope;
use serde_json::Value;
use serde_json::json;
use support::App;
use support::As;

/// The OpenAPI tags the control plane's operations carry.
const TAGS: [&str; 3] = ["account", "tokens", "projects"];

/// Every published control-plane operation has a row in the table, and
/// every row is published.
#[tokio::test]
async fn the_table_names_every_operation() {
	let app = support::app().await;
	let spec = app.get("/openapi.json", As::Nobody).await;
	let spec = spec.expect(200).json();
	let mut published = BTreeSet::new();

	for (path, item) in spec["paths"].as_object().expect("paths") {
		let Some(name) = path.strip_prefix("/rpc/") else {
			continue;
		};

		let tagged = item
			.as_object()
			.expect("an operation")
			.values()
			.filter_map(|operation| operation["tags"].as_array())
			.flatten()
			.any(|tag| TAGS.contains(&tag.as_str().unwrap_or_default()));

		if tagged {
			published.insert(name.to_owned());
		}
	}

	let table: BTreeSet<String> = OPERATIONS
		.iter()
		.map(|(name, _)| (*name).to_owned())
		.collect();
	assert_eq!(
		published, table,
		"every operation needs a row in access::OPERATIONS"
	);
}

/// Someone the matrix calls as.
#[derive(Clone, Copy, Debug)]
enum Caller {
	Role(Role),
	Scope(TokenScope),
}

/// One organisation set up for one caller, with something for every
/// operation to act on.
struct Fixture {
	app: App,
	owner: String,
	organization: String,
	/// The caller's credential: a session cookie or a token's value.
	credential: String,
	caller: Caller,
	/// A scope the caller may give a new token.
	grantable: &'static str,
	extra_account: String,
	pending_invitation: String,
	token_id: String,
}

impl Fixture {
	async fn new(caller: Caller) -> Self {
		let app = support::app().await;
		let owner = app.sign_in("owner@example.com").await;
		let organization = app.personal(As::Session(&owner)).await;
		app.project(As::Session(&owner), &organization, "web").await;
		app.project(As::Session(&owner), &organization, "doomed")
			.await;

		for name in ["staging", "doomed-env"] {
			app.call(
				"create_environment",
				json!({
					"organization": organization,
					"project": "web",
					"name": name,
					"kind": "staging",
				}),
				As::Session(&owner),
			)
			.await
			.expect(200);
		}

		let extra = app
			.join(
				As::Session(&owner),
				&organization,
				"extra@example.com",
				"viewer",
			)
			.await;
		let extra_account = app.account(As::Session(&extra)).await;
		app.invite(
			As::Session(&owner),
			&organization,
			"pending@example.com",
			"viewer",
		)
		.await;
		let pending_invitation = app
			.call(
				"invitations",
				json!({ "organization": organization }),
				As::Session(&owner),
			)
			.await
			.json()[0]["id"]
			.as_str()
			.expect("an invitation")
			.to_owned();
		let (_, token_id) = app
			.token(As::Session(&owner), &organization, &["read"], None)
			.await;

		let (credential, grantable) = match caller {
			Caller::Role(Role::Owner) => (owner.clone(), "read"),
			Caller::Role(role) => {
				let cookie = app
					.join(
						As::Session(&owner),
						&organization,
						"member@example.com",
						role.as_str(),
					)
					.await;
				(cookie, "read")
			}
			Caller::Scope(scope) => {
				let name = scope.name();
				let (value, _) = app
					.token(As::Session(&owner), &organization, &[name], None)
					.await;
				(value, name)
			}
		};

		Self {
			app,
			owner,
			organization,
			credential,
			caller,
			grantable,
			extra_account,
			pending_invitation,
			token_id,
		}
	}

	fn as_caller(&self) -> As<'_> {
		match self.caller {
			Caller::Role(_) => As::Session(&self.credential),
			Caller::Scope(_) => As::Bearer(&self.credential),
		}
	}

	/// Valid arguments for `operation`, so an allowed call succeeds.
	async fn args(&self, operation: &str) -> Value {
		let organization = self.organization.as_str();
		match operation {
			"whoami" | "organizations" => json!({}),
			"create_organization" => json!({ "slug": "new-team", "name": "New team" }),
			"accept_invitation" => json!({ "token": "0".repeat(64) }),
			"members" | "projects" | "invitations" | "tokens" => {
				json!({ "organization": organization })
			}
			"invite_member" => {
				json!({ "organization": organization, "email": "new@example.com", "role": "viewer" })
			}
			"revoke_invitation" => {
				json!({ "organization": organization, "invitation": self.pending_invitation })
			}
			"set_member_role" => {
				json!({
					"organization": organization,
					"account": self.extra_account,
					"role": "developer",
				})
			}
			"remove_member" => {
				json!({ "organization": organization, "account": self.extra_account })
			}
			"create_token" => {
				json!({ "organization": organization, "name": "new", "scopes": [self.grantable] })
			}
			"revoke_token" => json!({ "organization": organization, "token": self.token_id }),
			"create_project" => {
				json!({ "organization": organization, "slug": "fresh", "name": "Fresh" })
			}
			"project" | "environments" => json!({ "organization": organization, "project": "web" }),
			"delete_project" => {
				json!({ "organization": organization, "project": "doomed", "confirm": "doomed" })
			}
			"create_environment" => {
				json!({
					"organization": organization,
					"project": "web",
					"name": "fresh",
					"kind": "preview",
				})
			}
			"update_environment" => {
				let environments = self
					.app
					.call(
						"environments",
						json!({ "organization": organization, "project": "web" }),
						As::Session(&self.owner),
					)
					.await
					.json();
				let staging = environments
					.as_array()
					.expect("a list")
					.iter()
					.find(|environment| environment["name"] == "staging")
					.expect("staging")
					.clone();
				json!({
					"organization": organization,
					"project": "web",
					"environment": "staging",
					"revision": staging["revision"],
					"scale": { "app": { "size": "s", "min": 1, "max": 2, "sleep": "never" } },
				})
			}
			"delete_environment" => {
				json!({
					"organization": organization,
					"project": "web",
					"environment": "doomed-env",
					"confirm": "doomed-env",
				})
			}
			other => panic!("`{other}` has no arguments in the matrix: add them"),
		}
	}

	/// Whether the table lets this caller run something that requires
	/// `requirement`.
	fn allowed(&self, requirement: Requirement) -> bool {
		match (requirement, self.caller) {
			(Requirement::SignedIn, _) | (Requirement::Person, Caller::Role(_)) => true,
			(Requirement::Person, Caller::Scope(_)) => false,
			(
				Requirement::Organization(permission) | Requirement::Project(permission),
				Caller::Role(role),
			) => role.allows(permission),
			(
				Requirement::Organization(permission) | Requirement::Project(permission),
				Caller::Scope(scope),
			) => scope.allows(permission),
		}
	}
}

/// Runs every operation as `caller`, and returns what went other than the
/// table says.
async fn run(caller: Caller) -> Vec<String> {
	let fixture = Fixture::new(caller).await;
	let mut wrong = Vec::new();

	for (operation, requirement) in OPERATIONS {
		let args = fixture.args(operation).await;
		let answer = fixture.app.call(operation, args, fixture.as_caller()).await;
		let allowed = fixture.allowed(*requirement);
		let expected_status = match *operation {
			// The matrix's invitation is nobody's: a person gets as far as
			// finding that out.
			"accept_invitation" if allowed => 404,
			_ if allowed => 200,
			_ => 403,
		};

		if answer.status != expected_status {
			wrong.push(format!(
				"{caller:?} {operation}: expected {expected_status}, got {} {}",
				answer.status, answer.text
			));
		}
	}

	wrong
}

#[tokio::test]
async fn every_role_is_allowed_what_the_table_says() {
	let mut wrong = Vec::new();

	for role in Role::ALL {
		wrong.extend(run(Caller::Role(role)).await);
	}

	assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[tokio::test]
async fn every_scope_is_allowed_what_the_table_says() {
	let mut wrong = Vec::new();

	for scope in TokenScope::ALL {
		wrong.extend(run(Caller::Scope(scope)).await);
	}

	assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[tokio::test]
async fn nobody_is_allowed_anything() {
	let fixture = Fixture::new(Caller::Role(Role::Owner)).await;

	for (operation, _) in OPERATIONS {
		let args = fixture.args(operation).await;
		fixture
			.app
			.call(operation, args, As::Nobody)
			.await
			.refused(401, "sign_in_required");
	}
}
