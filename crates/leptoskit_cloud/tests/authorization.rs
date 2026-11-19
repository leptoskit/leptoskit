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
use support::shipping;

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
		Requirement::Worker => return Outcome::Refused(403, "worker_required"),
		Requirement::Operator | Requirement::Person if token.is_some() => {
			return Outcome::Refused(403, "person_required");
		}
		Requirement::Operator => return Outcome::Refused(403, "operator_required"),
		// The matrix's invitation is nobody's: a person gets as far as
		// finding that out.
		Requirement::Person if operation == "accept_invitation" => return Outcome::Answered(404),
		// A pairing code is its own credential: whoever holds the fixture's
		// pairs a worker with it.
		Requirement::Pairing | Requirement::SignedIn | Requirement::Person => {
			return Outcome::Answered(200);
		}
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
		// The fixture's production has no earlier release to return to: a
		// caller who may roll back gets as far as finding that out.
		_ if operation == "rollback" => Outcome::Refused(409, "rollback_unavailable"),
		// The fixture's DNS holds no challenge record: a caller who may
		// verify gets as far as finding that out.
		_ if operation == "verify_domain" => Outcome::Refused(409, "domain_not_verified"),
		// The fixture's staging has no database, and so no backup: a caller
		// who may take or restore one gets as far as finding that out.
		// The matrix's control plane has no price catalogue: a caller who
		// may set a cap gets as far as finding that out.
		_ if operation == "set_spend_cap" || operation == "estimate" => {
			Outcome::Refused(409, "unpriced")
		}
		// It takes no payment either: a caller who may manage the bill gets
		// as far as finding that out.
		_ if operation == "checkout" || operation == "portal" => {
			Outcome::Refused(409, "payments_unavailable")
		}
		_ if operation == "create_backup" => Outcome::Refused(409, "no_database"),
		_ if operation == "restore_backup" => Outcome::Refused(404, "backup_not_found"),
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
	/// A live pairing code for a build worker.
	pairing_code: String,
	/// An archive begun and uploaded, waiting for `complete_upload`.
	pending_digest: String,
	/// An archive ready to deploy.
	ready_digest: String,
}

/// What [`Arranging::shipping`] made.
struct Shipping {
	pairing_code: String,
	pending_digest: String,
	ready_digest: String,
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

	/// A cluster for `web` to run on, a pairing code, an archive waiting to be
	/// completed, and one ready, deployed to `web`'"'"'s production as #1,
	/// which stays queued: no worker takes it.
	async fn shipping(self) -> Shipping {
		let ops = shipping::operator(self.app).await;
		shipping::register_cluster(self.app, &ops, "eu-one").await;
		let issued = self
			.app
			.call(
				"issue_pairing",
				json!({ "role": "build" }),
				As::Session(&ops),
			)
			.await;
		let pairing_code = issued.expect(200).json()["code"]
			.as_str()
			.expect("a code")
			.to_owned();
		let ready_digest = shipping::upload(
			self.app,
			self.as_owner(),
			self.organization,
			"web",
			b"ready",
		)
		.await;
		let pending = b"pending";
		let began =
			shipping::begin(self.app, self.as_owner(), self.organization, "web", pending).await;
		let ticket = began.expect(200).json()["ticket"].clone();
		assert_eq!(shipping::put(self.app, &ticket, pending).await, 200);
		self.app
			.call(
				"deploy",
				json!({
					"organization": self.organization,
					"project": "web",
					"environment": "production",
					"source": { "kind": "upload", "archive": ready_digest },
				}),
				self.as_owner(),
			)
			.await
			.expect(200);

		Shipping {
			pairing_code,
			pending_digest: shipping::digest_of(pending),
			ready_digest,
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
		let shipping = arranging.shipping().await;
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
			pairing_code: shipping.pairing_code,
			pending_digest: shipping.pending_digest,
			ready_digest: shipping.ready_digest,
		}
	}

	fn as_caller(&self) -> As<'_> {
		match self.caller {
			Caller::Person(_) => As::Session(&self.credential),
			Caller::Cli(_) | Caller::Token { .. } => As::Bearer(&self.credential),
		}
	}

	/// Valid arguments for `operation`, so an allowed call succeeds.
	#[expect(
		clippy::too_many_lines,
		reason = "one arm per operation: the matrix's table of arguments"
	)]
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
			"rpc.mark_all_read" | "whoami" | "organizations" | "claim" | "clusters" | "workers"
			| "jobs" | "plans" => json!({}),
			"rpc.service_token" => json!({ "service": "billing" }),
			"create_organization" => json!({ "slug": "new-team", "name": "New team" }),
			"accept_invitation" => json!({ "token": "0".repeat(64) }),
			"members" | "projects" | "invitations" | "tokens" | "in_flight" => {
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
			"project" | "environments" | "deployments" => {
				json!({ "organization": organization, "project": "web" })
			}
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
			"environment_status" | "runtime_logs" | "preview_release" => {
				json!({ "organization": organization, "project": "web", "environment": "production" })
			}
			"begin_upload" => {
				json!({
					"organization": organization,
					"project": "web",
					"digest": shipping::digest_of(b"fresh"),
					"size": 5,
				})
			}
			"complete_upload" => {
				json!({ "organization": organization, "project": "web", "digest": self.pending_digest })
			}
			"deploy" => {
				json!({
					"organization": organization,
					"project": "web",
					"environment": "staging",
					"source": { "kind": "upload", "archive": self.ready_digest },
				})
			}
			// Whoever may cancel may deploy, and deployed staging #1 just now.
			"cancel_deployment" => {
				json!({
					"organization": organization,
					"project": "web",
					"environment": "staging",
					"number": 1,
				})
			}
			"rollback" => {
				json!({
					"organization": organization,
					"project": "web",
					"environment": "production",
					"to": 1,
				})
			}
			"deployment" | "deployment_events" | "build_logs" => {
				json!({
					"organization": organization,
					"project": "web",
					"environment": "production",
					"number": 1,
				})
			}
			"variables" | "secrets" | "domains" | "database" | "database_backups"
			| "create_backup" => {
				json!({ "organization": organization, "project": "web", "environment": "staging" })
			}
			// Whoever may unset may set, and set these just before.
			"set_variable" | "set_secret" => {
				json!({
					"organization": organization,
					"project": "web",
					"environment": "staging",
					"name": if operation == "set_variable" { "MATRIX_VARIABLE" } else { "MATRIX_SECRET" },
					"value": "matrix-value",
				})
			}
			"unset_variable" | "unset_secret" => {
				json!({
					"organization": organization,
					"project": "web",
					"environment": "staging",
					"name": if operation == "unset_variable" { "MATRIX_VARIABLE" } else { "MATRIX_SECRET" },
				})
			}
			"attach_domain" | "verify_domain" | "detach_domain" => {
				json!({
					"organization": organization,
					"project": "web",
					"environment": "staging",
					"hostname": "matrix.example.org",
				})
			}
			"pair" => json!({ "code": self.pairing_code, "name": "matrix-worker" }),
			"renew" => json!({ "job": nothing }),
			"append_log" => json!({ "job": nothing, "attempt": 1, "seq": 0, "lines": ["hi"] }),
			"finish" => {
				json!({
					"job": nothing,
					"attempt": 1,
					"outcome": { "kind": "release", "result": "applied" },
				})
			}
			"report_status" => {
				let named = json!({ "slug": "web", "id": nothing });
				json!({
					"environment": nothing,
					"observed": {
						"environment": { "organization": named, "project": named, "environment": named },
						"processes": [],
					},
				})
			}
			"restore_backup" => {
				json!({
					"organization": organization,
					"project": "web",
					"environment": "staging",
					"backup": nothing,
					"confirm": "web/staging",
				})
			}
			"usage" | "subscription" | "portal" => json!({ "organization": organization }),
			"checkout" => json!({ "organization": organization, "plan": "pro" }),
			"estimate" => json!({ "organization": organization, "config": {} }),
			"set_spend_cap" => json!({ "organization": organization, "cap": 100 }),
			"register_cluster" => json!({ "name": "eu-two", "region": "eu-central" }),
			"set_cluster_facts" => json!({ "name": "eu-one", "facts": { "wakes_sleeping": true } }),
			"issue_pairing" => json!({ "role": "build" }),
			"revoke_worker" => json!({ "worker": nothing }),
			"reconcile" => json!({ "job": nothing, "found": { "found": "applied" } }),
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

		match requirement {
			Requirement::Builtin => {
				let expected = builtin(operation, Who::Nobody);
				assert!(
					expected.matches(&answer),
					"{operation}: {expected:?}, got {answer:?}"
				);
			}
			// The code is the credential.
			Requirement::Pairing => {
				answer.expect(200);
			}
			Requirement::Worker => answer.refused(401, "worker_key_required"),
			_ => answer.refused(401, "sign_in_required"),
		}
	}
}

/// An operator is a person: their session and their CLI run every operator
/// operation, and their API token, whatever its scope, runs none.
#[tokio::test]
async fn operators_and_only_operators_run_the_cloud() {
	let fixture = Fixture::new(Caller::Person(Role::Owner)).await;
	let app = &fixture.app;
	let ops = shipping::operator(app).await;
	let cli = app.cli_access(&ops).await;
	let ops_org = app.personal(As::Session(&ops)).await;
	let (token, _) = app
		.token(As::Session(&ops), &ops_org, &["admin"], None)
		.await;

	for (operation, requirement) in OPERATIONS {
		if *requirement != Requirement::Operator {
			continue;
		}

		for (caller, label) in [(As::Session(&ops), "session"), (As::Bearer(&cli), "cli")] {
			let args = match *operation {
				"register_cluster" => {
					json!({ "name": format!("eu-{label}"), "region": "eu-central" })
				}
				_ => fixture.args(operation).await,
			};
			let answer = app.call(operation, args, caller).await;
			let expected = match *operation {
				"revoke_worker" => Outcome::Refused(404, "worker_not_found"),
				"reconcile" => Outcome::Refused(404, "job_not_found"),
				_ => Outcome::Answered(200),
			};
			assert!(
				expected.matches(&answer),
				"{label} {operation}: {expected:?}, got {answer:?}"
			);
		}

		let answer = app
			.call(operation, fixture.args(operation).await, As::Bearer(&token))
			.await;
		answer.refused(403, "person_required");
	}
}

/// A worker's key runs the worker operations and nothing else: no
/// organisation's, and no operator's.
#[tokio::test]
async fn a_worker_runs_only_the_protocol() {
	let fixture = Fixture::new(Caller::Person(Role::Owner)).await;
	let app = &fixture.app;
	let paired = app
		.call(
			"pair",
			json!({ "code": fixture.pairing_code, "name": "matrix-worker" }),
			As::Nobody,
		)
		.await;
	let key = paired.expect(200).json()["key"]
		.as_str()
		.expect("a key")
		.to_owned();

	for (operation, requirement) in in_matrix_order() {
		let answer = app
			.call(operation, fixture.args(operation).await, As::Bearer(&key))
			.await;
		let fine = match requirement {
			Requirement::Worker => {
				match *operation {
					"claim" => answer.status == 200,
					"report_status" => answer.message().starts_with("cluster_worker_required"),
					_ => answer.message().starts_with("job_not_held"),
				}
			}
			// Spent by the fixture's own pairing above.
			Requirement::Pairing => answer.message().starts_with("pairing_invalid"),
			Requirement::Builtin => answer.status != 500,
			_ => answer.status == 403 && answer.message().starts_with("not_for_workers"),
		};
		assert!(fine, "{operation}: {} {}", answer.status, answer.text);
	}
}

/// A cluster's worker is held to the same line: the worker operations and
/// nothing else, and of those only what is its own. It can't renew, log to,
/// or finish a job another worker holds, by that job's real ID, and a build
/// worker can't report for an environment that exists.
#[tokio::test]
async fn a_cluster_worker_runs_only_the_protocol() {
	let fixture = Fixture::new(Caller::Person(Role::Owner)).await;
	let app = &fixture.app;
	let ops = shipping::operator(app).await;
	let cluster = shipping::pair(
		app,
		&ops,
		json!({ "role": "cluster", "cluster": "eu-one" }),
		"matrix-cluster-worker",
	)
	.await;

	for (operation, requirement) in in_matrix_order() {
		let answer = cluster.call(operation, fixture.args(operation).await).await;
		let fine = match requirement {
			Requirement::Worker => {
				match *operation {
					"claim" => answer.status == 200,
					"report_status" => {
						answer.status == 404
							&& answer.message().starts_with("environment_not_found")
					}
					_ => answer.status == 404 && answer.message().starts_with("job_not_held"),
				}
			}
			Requirement::Pairing | Requirement::Builtin => answer.status != 500,
			_ => answer.status == 403 && answer.message().starts_with("not_for_workers"),
		};
		assert!(fine, "{operation}: {} {}", answer.status, answer.text);
	}

	// The fixture's deployment waits for its build: a build worker takes
	// it, and the cluster's worker can do nothing to that job.
	let build = shipping::pair(app, &ops, json!({ "role": "build" }), "matrix-builder").await;
	let held = build.take().await;
	cluster.renew(&held).await.refused(404, "job_not_held");
	cluster
		.log(&held, 0, &["not mine"])
		.await
		.refused(404, "job_not_held");
	cluster
		.finish(&held, shipping::built(&held, b"not mine", &[]))
		.await
		.refused(404, "job_not_held");
	cluster
		.finish(&held, shipping::applied())
		.await
		.refused(404, "job_not_held");

	// Nor can the build worker speak for the cluster about a real
	// environment, or a second build worker for the first.
	let environments = app
		.call(
			"environments",
			json!({ "organization": fixture.organization, "project": "web" }),
			As::Session(&fixture.owner),
		)
		.await
		.expect(200)
		.json();
	let environment = environments[0]["id"].clone();
	let named = json!({ "slug": "web", "id": environment });
	build
		.call(
			"report_status",
			json!({
				"environment": environment,
				"observed": {
					"environment": { "organization": named, "project": named, "environment": named },
					"processes": [],
				},
			}),
		)
		.await
		.refused(403, "cluster_worker_required");
	let other = shipping::pair(app, &ops, json!({ "role": "build" }), "matrix-builder-2").await;
	other.renew(&held).await.refused(404, "job_not_held");
	other
		.finish(&held, shipping::built(&held, b"not mine", &[]))
		.await
		.refused(404, "job_not_held");
	build.renew(&held).await.expect(200);
}
