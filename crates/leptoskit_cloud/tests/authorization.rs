//! The authorisation matrix, from the table in `access::OPERATIONS`: every
//! operation, called by a person with each role, by that person through
//! the CLI, by a token each role made with each scope it may give, bound
//! to a project and not, and by nobody, is answered exactly as the table
//! says. The operations the server serves must be the table's, so a new
//! operation without a decided permission fails here.

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

/// Every operation the server serves has a row in the table, and every row
/// is served: the inventory is what the server's router mounts, the
/// toolkit's own `rpc.*` operations included, with nothing filtered out.
#[test]
fn the_table_names_every_served_operation() {
	let served: BTreeSet<String> = leptoskit_rpc::server::registrations_in(None)
		.expect("no two operations share a name")
		.iter()
		.map(|registration| registration.name.to_owned())
		.collect();
	let table: BTreeSet<String> = OPERATIONS
		.iter()
		.map(|(name, _)| (*name).to_owned())
		.collect();
	assert_eq!(
		served, table,
		"every served operation needs a row in access::OPERATIONS"
	);
}

/// The OpenAPI document publishes the same operations: none is served
/// without being described, or described without being served.
#[tokio::test]
async fn the_openapi_document_publishes_what_is_served() {
	let app = support::app().await;
	let spec = app.get("/openapi.json", As::Nobody).await;
	let spec = spec.expect(200).json();
	let published: BTreeSet<String> = spec["paths"]
		.as_object()
		.expect("paths")
		.keys()
		.filter_map(|path| path.strip_prefix("/rpc/"))
		.map(ToOwned::to_owned)
		.collect();
	let served: BTreeSet<String> = leptoskit_rpc::server::registrations_in(None)
		.expect("no two operations share a name")
		.iter()
		.map(|registration| registration.name.to_owned())
		.collect();
	assert!(
		published.is_subset(&served),
		"published but not served: {:?}",
		published.difference(&served).collect::<Vec<_>>()
	);
}

/// Who the matrix calls as: always for a member with a role.
#[derive(Clone, Copy, Debug)]
enum Caller {
	/// The member, signed in with a session.
	Person(Role),
	/// The member, through the first-party CLI's device-flow access token.
	Cli(Role),
	/// An API token the member made, with one scope, bound to the project
	/// `web` or to none. Only owners and admins make tokens, so a member
	/// with another role made it as an owner and was demoted since, which
	/// is how such a token comes to be.
	Token {
		maker: Role,
		scope: TokenScope,
		bound: bool,
	},
}

impl Caller {
	/// The role of the member the caller acts for.
	const fn role(self) -> Role {
		match self {
			Self::Person(role) | Self::Cli(role) | Self::Token { maker: role, .. } => role,
		}
	}

	/// How the toolkit's own `rpc.*` operations see the caller.
	const fn who(self) -> Who {
		match self {
			Self::Person(_) => Who::Person,
			Self::Cli(_) => Who::Cli,
			Self::Token { .. } => Who::Token,
		}
	}

	/// Every token the matrix calls with: each role, with each scope, bound
	/// and unbound.
	fn tokens() -> Vec<Self> {
		let mut tokens = Vec::new();

		for maker in Role::ALL {
			for scope in TokenScope::ALL {
				for bound in [false, true] {
					tokens.push(Self::Token {
						maker,
						scope,
						bound,
					});
				}
			}
		}

		tokens
	}
}

/// How the toolkit's own `rpc.*` operations see a caller. They know
/// nothing of the cloud's organisations or tokens: a session is its person,
/// an API token is nobody, and the CLI's access token is refused by the
/// OAuth layer, since it is only for the cloud's operations.
#[derive(Clone, Copy, Debug)]
enum Who {
	Nobody,
	Person,
	Cli,
	Token,
}

/// What a call should get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
	/// An answer with this status, whatever it says.
	Answered(u16),
	/// An answer with this status and exactly this body.
	Body(u16, &'static str),
	/// A refusal with this status, whose message starts with this code.
	Refused(u16, &'static str),
}

impl Outcome {
	fn matches(self, answer: &support::Answer) -> bool {
		match self {
			Self::Answered(status) => answer.status == status,
			Self::Body(status, body) => answer.status == status && answer.text == body,
			Self::Refused(status, code) => {
				answer.status == status && answer.message().starts_with(code)
			}
		}
	}
}

/// What the table, with the roles' and scopes' predicates, says `caller`
/// gets from `operation`, in the order the server checks: a token for what
/// only a person may do, then the member's role, then the token's scopes,
/// then its project binding.
fn expected(caller: Caller, operation: &str, requirement: Requirement) -> Outcome {
	let token = match caller {
		Caller::Token { scope, bound, .. } => Some((scope, bound)),
		Caller::Person(_) | Caller::Cli(_) => None,
	};
	let permission = match requirement {
		Requirement::Builtin => return builtin(operation, caller.who()),
		Requirement::Person if token.is_some() => return Outcome::Refused(403, "person_required"),
		// The matrix's invitation is nobody's: a person gets as far as
		// finding that out.
		Requirement::Person if operation == "accept_invitation" => return Outcome::Answered(404),
		Requirement::SignedIn | Requirement::Person => return Outcome::Answered(200),
		Requirement::Organization(permission) | Requirement::Project(permission) => permission,
	};

	if token.is_some() && permission.requires_person() {
		return Outcome::Refused(403, "person_required");
	}

	if !caller.role().allows(permission) {
		return Outcome::Refused(403, "insufficient_role");
	}

	match token {
		Some((scope, _)) if !scope.allows(permission) => {
			Outcome::Refused(403, "insufficient_scope")
		}
		Some((_, true)) if !permission.reaches_project_tokens() => {
			Outcome::Refused(403, "project_token")
		}
		_ => Outcome::Answered(200),
	}
}

/// What the toolkit's own operations answer `who`. They reach no
/// organisation, so roles, scopes, and bindings change nothing, and they
/// find nothing: the cloud writes no notices and enqueues no jobs, and it
/// forwards nobody's identity to a service.
fn builtin(operation: &str, who: Who) -> Outcome {
	if matches!(who, Who::Cli) {
		return Outcome::Body(403, r#"{"error":"insufficient_scope"}"#);
	}

	match operation {
		"rpc.inbox" => Outcome::Body(200, r#"{"notices":[],"unread":0}"#),
		"rpc.mark_read" => Outcome::Refused(404, "no such notice"),
		"rpc.mark_all_read" if matches!(who, Who::Person) => Outcome::Body(200, "0"),
		"rpc.mark_all_read" => Outcome::Refused(401, "sign in to read your notices"),
		"rpc.job_status" | "rpc.cancel_job" => Outcome::Refused(404, "no such job"),
		"rpc.service_token" => Outcome::Body(200, "null"),
		other => panic!("`{other}` has no expected answer in the matrix: add one"),
	}
}

/// One organisation set up for one caller, with something for every
/// operation to act on.
struct Fixture {
	app: App,
	owner: String,
	organization: String,
	/// The caller's credential: a session cookie, or a bearer value.
	credential: String,
	caller: Caller,
	extra_account: String,
	pending_invitation: String,
	token_id: String,
}

/// An organisation being arranged, as its owner sees it.
#[derive(Clone, Copy)]
struct Arranging<'a> {
	app: &'a App,
	owner: &'a str,
	organization: &'a str,
}

impl Arranging<'_> {
	fn as_owner(&self) -> As<'_> {
		As::Session(self.owner)
	}

	/// The projects `web` and `doomed`, and on `web` the environments
	/// `staging` and `doomed-env`, for the project operations to act on.
	async fn projects(self) {
		self.app
			.project(self.as_owner(), self.organization, "web")
			.await;
		self.app
			.project(self.as_owner(), self.organization, "doomed")
			.await;

		for name in ["staging", "doomed-env"] {
			self.app
				.call(
					"create_environment",
					json!({
						"organization": self.organization,
						"project": "web",
						"name": name,
						"kind": "staging",
					}),
					self.as_owner(),
				)
				.await
				.expect(200);
		}
	}

	/// Invites someone as a viewer, and returns the invitation's ID.
	async fn pending_invitation(self) -> String {
		self.app
			.invite(
				self.as_owner(),
				self.organization,
				"pending@example.com",
				"viewer",
			)
			.await;
		self.app
			.call(
				"invitations",
				json!({ "organization": self.organization }),
				self.as_owner(),
			)
			.await
			.json()[0]["id"]
			.as_str()
			.expect("an invitation")
			.to_owned()
	}

	/// A session for someone with `role`: the owner's, or a member who joins
	/// with it.
	async fn member(self, role: Role) -> String {
		if role == Role::Owner {
			return self.owner.to_owned();
		}

		self.app
			.join(
				self.as_owner(),
				self.organization,
				"member@example.com",
				role.as_str(),
			)
			.await
	}

	/// `caller`'s credential: a session cookie, or a bearer value.
	async fn credential(self, caller: Caller) -> String {
		match caller {
			Caller::Person(role) => self.member(role).await,
			Caller::Cli(role) => {
				let member = self.member(role).await;
				self.app.cli_access(&member).await
			}
			Caller::Token {
				maker,
				scope,
				bound,
			} => self.token(maker, scope, bound).await,
		}
	}

	/// A token with `scope` whose maker has `maker`'s role now. Only owners
	/// and admins make tokens, so its maker makes it as an owner and is
	/// given `maker`'s role afterwards.
	async fn token(self, maker: Role, scope: TokenScope, bound: bool) -> String {
		let session = if maker == Role::Owner {
			self.owner.to_owned()
		} else {
			self.app
				.join(
					self.as_owner(),
					self.organization,
					"member@example.com",
					"owner",
				)
				.await
		};
		let (value, _) = self
			.app
			.token(
				As::Session(&session),
				self.organization,
				&[scope.name()],
				bound.then_some("web"),
			)
			.await;

		if maker != Role::Owner {
			let account = self.app.account(As::Session(&session)).await;
			self.app
				.call(
					"set_member_role",
					json!({
						"organization": self.organization,
						"account": account,
						"role": maker.as_str(),
					}),
					self.as_owner(),
				)
				.await
				.expect(200);
		}

		value
	}
}

impl Fixture {
	async fn new(caller: Caller) -> Self {
		let app = support::app().await;
		let owner = app.sign_in("owner@example.com").await;
		let organization = app.personal(As::Session(&owner)).await;
		let arranging = Arranging {
			app: &app,
			owner: &owner,
			organization: &organization,
		};
		arranging.projects().await;
		let extra = app
			.join(
				As::Session(&owner),
				&organization,
				"extra@example.com",
				"viewer",
			)
			.await;
		let extra_account = app.account(As::Session(&extra)).await;
		let pending_invitation = arranging.pending_invitation().await;
		let (_, token_id) = app
			.token(As::Session(&owner), &organization, &["read"], None)
			.await;
		let credential = arranging.credential(caller).await;

		Self {
			app,
			owner,
			organization,
			credential,
			caller,
			extra_account,
			pending_invitation,
			token_id,
		}
	}

	fn as_caller(&self) -> As<'_> {
		match self.caller {
			Caller::Person(_) => As::Session(&self.credential),
			Caller::Cli(_) | Caller::Token { .. } => As::Bearer(&self.credential),
		}
	}

	/// Valid arguments for `operation`, so an allowed call succeeds.
	async fn args(&self, operation: &str) -> Value {
		let organization = self.organization.as_str();
		// A bound token can see only its own project, so it deletes that,
		// last.
		let doomed = match self.caller {
			Caller::Token { bound: true, .. } => "web",
			_ => "doomed",
		};
		let nothing = "00000000-0000-0000-0000-000000000000";

		match operation {
			"rpc.inbox" => json!({ "limit": 10 }),
			"rpc.mark_read" | "rpc.job_status" | "rpc.cancel_job" => json!({ "id": nothing }),
			"rpc.mark_all_read" | "whoami" | "organizations" => json!({}),
			"rpc.service_token" => json!({ "service": "billing" }),
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
				json!({ "organization": organization, "name": "new", "scopes": ["read"] })
			}
			"revoke_token" => json!({ "organization": organization, "token": self.token_id }),
			"create_project" => {
				json!({ "organization": organization, "slug": "fresh", "name": "Fresh" })
			}
			"project" | "environments" => json!({ "organization": organization, "project": "web" }),
			"delete_project" => {
				json!({ "organization": organization, "project": doomed, "confirm": doomed })
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
}

/// The table's operations in the order the matrix runs them: deleting a
/// project last, since a bound token deletes the project the others use.
fn in_matrix_order() -> impl Iterator<Item = &'static (&'static str, Requirement)> {
	let last = |(name, _): &&(&str, Requirement)| *name == "delete_project";
	OPERATIONS
		.iter()
		.filter(move |row| !last(row))
		.chain(OPERATIONS.iter().filter(last))
}

/// Runs every operation as `caller`, and returns what went other than the
/// table says.
async fn run(caller: Caller) -> Vec<String> {
	let fixture = Fixture::new(caller).await;
	let mut wrong = Vec::new();

	for (operation, requirement) in in_matrix_order() {
		let args = fixture.args(operation).await;
		let answer = fixture.app.call(operation, args, fixture.as_caller()).await;
		let expected = expected(caller, operation, *requirement);

		if !expected.matches(&answer) {
			wrong.push(format!(
				"{caller:?} {operation}: expected {expected:?}, got {} {}",
				answer.status, answer.text
			));
		}
	}

	wrong
}

/// Runs the matrix for every caller in `callers`, each on its own server,
/// and fails with everything that went other than the table says.
async fn run_all(callers: Vec<Caller>) {
	let wrong: Vec<String> = futures::future::join_all(callers.into_iter().map(run))
		.await
		.into_iter()
		.flatten()
		.collect();
	assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[tokio::test]
async fn every_person_is_allowed_what_their_role_is() {
	run_all(Role::ALL.into_iter().map(Caller::Person).collect()).await;
}

/// The CLI's access token is its person: the same answers as their
/// session, except from the toolkit's own operations, which aren't the
/// cloud's to answer.
#[tokio::test]
async fn the_cli_is_allowed_what_its_person_is() {
	run_all(Role::ALL.into_iter().map(Caller::Cli).collect()).await;
}

/// A token is allowed what its maker's role, as it is now, and its scope
/// both allow, in its project when it is bound to one, and never what only
/// a person may do.
#[tokio::test]
async fn every_token_is_allowed_what_its_maker_and_scope_both_are() {
	let tokens = Caller::tokens();
	assert_eq!(
		tokens.len(),
		60,
		"every role, with every scope, bound or not"
	);
	run_all(tokens).await;
}

#[tokio::test]
async fn nobody_is_allowed_anything() {
	let fixture = Fixture::new(Caller::Person(Role::Owner)).await;

	for (operation, requirement) in in_matrix_order() {
		let args = fixture.args(operation).await;
		let answer = fixture.app.call(operation, args, As::Nobody).await;

		if *requirement == Requirement::Builtin {
			let expected = builtin(operation, Who::Nobody);
			assert!(
				expected.matches(&answer),
				"{operation}: {expected:?}, got {answer:?}"
			);
		} else {
			answer.refused(401, "sign_in_required");
		}
	}
}
