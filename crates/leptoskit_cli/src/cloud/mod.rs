//! `lk cloud`: accounts and projects (CLI 1), an environment's
//! configuration and history (CLI 2), store publishing and usage (CLI 3),
//! and the first-class shipping workflow (`launch`, `deploy`, `status`,
//! `logs`, `estimate`, and `open`).

pub mod client;
mod credentials;
pub(crate) mod deploy;
mod external;
pub(crate) mod ship;
mod stores;
#[cfg(test)]
mod tests;
mod transport;

use std::ffi::OsString;
use std::io::IsTerminal as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use clap::Args;
use clap::Subcommand;
use client::Client;
use leptoskit_cloud::access::Permission;
use leptoskit_cloud::access::Requirement;
use leptoskit_cloud::access::Role;
use leptoskit_cloud::access::TokenScope;
use leptoskit_cloud::account;
use leptoskit_cloud::projects;
use leptoskit_cloud::tokens;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::publish::secrets::Secret;

/// The hosted cloud's default origin.
pub const DEFAULT_URL: &str = "https://cloud.leptoskit.dev";

/// Connects to a cloud and manages its accounts and projects.
#[derive(Debug, Args)]
#[command(
	after_help = "External subcommands:\n  Any `leptoskit-cloud-<name>` executable beside `lk` or \
	              on PATH (such as `platform`, `worker`, or `control`) can be invoked as `lk \
	              cloud <name> [ARGS]...`."
)]
pub struct CloudArgs {
	/// Cloud origin; overrides `LEPTOSKIT_CLOUD_URL` and [cloud].url.
	#[arg(long, global = true)]
	pub url: Option<String>,
	/// Prints the documented JSON result.
	#[arg(long, global = true)]
	pub json: bool,
	/// Organization slug; defaults to [cloud].organization or your personal
	/// one.
	#[arg(long, global = true)]
	pub org: Option<String>,
	/// Project slug; defaults to [cloud].project.
	#[arg(long, global = true)]
	pub project: Option<String>,
	/// Environment name, for the commands about one; defaults to
	/// production.
	#[arg(long, short = 'e', global = true)]
	pub environment: Option<String>,
	#[command(subcommand)]
	pub command: CloudCommand,
}

/// The cloud's commands. Every one takes `--json`.
#[derive(Debug, Subcommand)]
pub enum CloudCommand {
	/// Creates & links a cloud project if needed, previews cost, and ships the
	/// first deployment.
	Launch {
		/// Project display name (defaults to `[app].name` in leptoskit.toml).
		#[arg(long)]
		name: Option<String>,
		/// Region slug (defaults to `[cloud].region` or the cloud default).
		#[arg(long)]
		region: Option<String>,
		/// Skip interactive confirmation after printing the release preview.
		#[arg(long, short = 'y')]
		yes: bool,
		/// Return as soon as the deployment is queued without following logs.
		#[arg(long)]
		detach: bool,
	},
	/// Packs, uploads, and deploys the current project.
	#[command(about = "Packs, uploads, and deploys the current project to LeptosKit Cloud")]
	Deploy {
		/// Return as soon as the deployment is queued without following logs.
		#[arg(long)]
		detach: bool,
	},
	/// Shows the observed status, hostname, and process readiness of an
	/// environment.
	Status,
	/// Reads recent runtime logs for an environment (pass `--follow` to
	/// stream).
	Logs {
		/// Continuously poll for new runtime log lines.
		#[arg(long, short = 'f')]
		follow: bool,
		/// Filter to a single process (e.g. `app` or a service name).
		#[arg(long)]
		process: Option<String>,
		/// Only include lines from the last N minutes.
		#[arg(long)]
		since_minutes: Option<u32>,
		/// Only include lines containing this substring.
		#[arg(long)]
		contains: Option<String>,
		/// Maximum lines per page.
		#[arg(long)]
		limit: Option<u32>,
	},
	/// Previews the resolved release plan and estimated monthly cost without
	/// deploying.
	Estimate,
	/// Opens the environment's live HTTPS URL in your browser.
	Open {
		/// Print the URL without launching a browser.
		#[arg(long)]
		print: bool,
	},
	/// Signs in through your browser with a short code.
	Login {
		/// Prints the URL and code without opening a browser.
		#[arg(long)]
		no_browser: bool,
	},
	/// Revokes your login and removes the local credentials.
	Logout,
	/// Shows who is signed in and which organizations they can see.
	Whoami,
	/// Manages organizations and membership.
	Org {
		#[command(subcommand)]
		command: OrgCommand,
	},
	/// Manages projects and their production environment.
	Project {
		#[command(subcommand)]
		command: ProjectCommand,
	},
	/// Manages a project's environments.
	Env {
		#[command(subcommand)]
		command: EnvCommand,
	},
	/// Manages an environment's variables.
	Var {
		#[command(subcommand)]
		command: ship::VarCommand,
	},
	/// Manages an environment's secrets.
	Secret {
		#[command(subcommand)]
		command: ship::SecretCommand,
	},
	/// Manages an environment's custom domains.
	Domain {
		#[command(subcommand)]
		command: ship::DomainCommand,
	},
	/// Lists an environment's deployments, newest first.
	Deployments {
		#[arg(long)]
		limit: Option<u32>,
	},
	/// Returns an environment to an earlier deployment that was live,
	/// reusing its image.
	Rollback {
		/// The deployment's number, from `lk cloud deployments`.
		to: u64,
	},
	/// Manages an environment's database and its backups.
	Db {
		#[command(subcommand)]
		command: ship::DbCommand,
	},
	/// Shows an environment's hosted notifications: whether the cluster's
	/// shared service serves them, which push providers are set up, and
	/// the project's pushes this month.
	Notifications,
	/// Builds the project on a build worker and uploads it to stores, as
	/// `lk publish` does, with the environment's secrets as credentials.
	Publish {
		/// The stores to publish to.
		#[arg(required = true, value_parser = stores::store)]
		stores: Vec<String>,
		/// Also sends each upload for review or release, which reaches
		/// users.
		#[arg(long)]
		submit: bool,
		/// Answers once the publish is queued instead of following it.
		#[arg(long)]
		detach: bool,
	},
	/// Shows an environment's publishes and what they printed.
	Publications {
		#[command(subcommand)]
		command: stores::PublicationCommand,
	},
	/// Shows what the organization used in a month, and its cost.
	Usage {
		/// The month, such as 2026-11; defaults to this month.
		#[arg(long)]
		period: Option<String>,
	},
	/// Manages scoped API tokens for CI.
	Token {
		#[command(subcommand)]
		command: TokenCommand,
	},
	/// Links leptoskit.toml to an existing cloud project.
	Link {
		#[arg(long)]
		force: bool,
	},
	/// Cargo-style external subcommand: invokes `leptoskit-cloud-<name>` beside
	/// `lk` or on `PATH`.
	#[command(external_subcommand)]
	External(Vec<OsString>),
}

/// Organization operations. Membership changes require a person signed in.
#[derive(Debug, Subcommand)]
pub enum OrgCommand {
	/// Lists organizations you can see.
	List,
	/// Creates an organization with you as owner.
	Create {
		name: String,
		#[arg(long)]
		slug: Option<String>,
	},
	/// Lists the members of an organization.
	Members { organization: String },
	/// Emails an invitation to join.
	Invite {
		organization: String,
		email: String,
		#[arg(long, value_parser = role)]
		role: Role,
	},
	/// Lists pending invitations.
	Invitations { organization: String },
	/// Revokes an invitation; requires its ID again as confirmation.
	RevokeInvitation {
		organization: String,
		invitation: uuid::Uuid,
		#[arg(long)]
		confirm: String,
	},
	/// Changes a member's role.
	SetRole {
		organization: String,
		account: uuid::Uuid,
		#[arg(long, value_parser = role)]
		role: Role,
	},
	/// Removes a member; requires their ID again as confirmation.
	RemoveMember {
		organization: String,
		account: uuid::Uuid,
		#[arg(long)]
		confirm: String,
	},
}

/// Project operations.
#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
	/// Lists the selected organization's projects.
	List,
	/// Creates a project with a production environment.
	Create {
		name: String,
		#[arg(long)]
		region: Option<String>,
	},
	/// Shows the selected project and its environments.
	Show,
	/// Deletes a project and everything in it.
	Delete {
		slug: String,
		#[arg(long)]
		confirm: String,
	},
}

/// Environment operations.
#[derive(Debug, Subcommand)]
pub enum EnvCommand {
	/// Lists the selected project's environments.
	List,
	/// Creates a staging or preview environment.
	Create {
		name: String,
		#[arg(long, default_value = "staging", value_parser = kind)]
		kind: projects::EnvironmentKind,
	},
	/// Reads the current revision and changes the app's scale.
	Update {
		name: String,
		#[arg(long, value_parser = size)]
		size: Option<leptoskit_cloud::size::Size>,
		#[arg(long)]
		min: Option<u32>,
		#[arg(long)]
		max: Option<u32>,
		#[arg(long, value_parser = sleep)]
		sleep: Option<leptoskit_cloud::size::Sleep>,
	},
	/// Deletes an environment other than production.
	Delete {
		name: String,
		#[arg(long)]
		confirm: String,
	},
}

/// Token operations. Values appear only at creation.
#[derive(Debug, Subcommand)]
pub enum TokenCommand {
	/// Lists live token metadata, never values.
	List,
	/// Creates a scoped token and shows its value once.
	Create {
		name: String,
		#[arg(long = "scope", required = true, value_delimiter = ',', value_parser = scope)]
		scopes: Vec<TokenScope>,
		#[arg(long, default_value = "90", value_parser = expiry)]
		expires: u32,
	},
	/// Revokes a token; requires its ID again as confirmation.
	Revoke {
		token: uuid::Uuid,
		#[arg(long)]
		confirm: String,
	},
}

fn role(value: &str) -> Result<Role, String> {
	Role::ALL
		.into_iter()
		.find(|role| role.as_str() == value)
		.ok_or_else(|| "Use owner, admin, developer, billing or viewer.".into())
}
fn scope(value: &str) -> Result<TokenScope, String> {
	TokenScope::ALL
		.into_iter()
		.find(|scope| scope.name() == value)
		.ok_or_else(|| "Use read, deploy, manage, publish, billing or admin.".into())
}
fn kind(value: &str) -> Result<projects::EnvironmentKind, String> {
	match value {
		"staging" => Ok(projects::EnvironmentKind::Staging),
		"preview" => Ok(projects::EnvironmentKind::Preview),
		_ => Err("Use staging or preview.".into()),
	}
}
fn size(value: &str) -> Result<leptoskit_cloud::size::Size, String> {
	serde_json::from_value(json!(value)).map_err(|_| "Use xs, s, m, l or xl.".into())
}
fn sleep(value: &str) -> Result<leptoskit_cloud::size::Sleep, String> {
	serde_json::from_value(json!(value)).map_err(|_| "Use never or when-idle.".into())
}
fn expiry(value: &str) -> Result<u32, String> {
	value
		.parse()
		.ok()
		.filter(|value| tokens::LIFETIMES_DAYS.contains(value))
		.ok_or_else(|| "Use 30, 90 or 365 days.".into())
}

fn slug(name: &str) -> String {
	name.to_lowercase()
		.split(|character: char| !character.is_ascii_alphanumeric())
		.filter(|part| !part.is_empty())
		.collect::<Vec<_>>()
		.join("-")
}

struct Context {
	client: Client,
	url: String,
	config: leptoskit_config::cloud::CloudConfig,
	file: Option<PathBuf>,
}

impl Context {
	fn load(args: &CloudArgs) -> Result<Self> {
		let file = std::env::current_dir()?
			.ancestors()
			.map(|dir| dir.join("leptoskit.toml"))
			.find(|path| path.is_file());
		let config: leptoskit_config::cloud::CloudConfig = if let Some(file) = &file {
			let document: toml::Value = toml::from_str(&std::fs::read_to_string(file)?)?;
			document
				.get("cloud")
				.cloned()
				.map(toml::Value::try_into)
				.transpose()?
				.unwrap_or_default()
		} else {
			leptoskit_config::cloud::CloudConfig::default()
		};
		let environment_url = std::env::var("LEPTOSKIT_CLOUD_URL").ok();
		let explicit = args.url.as_deref().or(environment_url.as_deref());
		let url = transport::origin(explicit.or(config.url.as_deref()).unwrap_or(DEFAULT_URL))?;
		let token = std::env::var("LEPTOSKIT_CLOUD_TOKEN").ok().map(Secret::new);
		check_token_origin(token.is_some(), explicit, &url)?;
		let client = Client::new(&url, &credentials::Store::default_root()?, token)?;
		Ok(Self {
			client,
			url,
			config,
			file,
		})
	}

	fn organization(&self, args: &CloudArgs) -> Result<String> {
		if let Some(org) = args.org.as_deref().or(self
			.config
			.organization
			.as_ref()
			.map(leptoskit_cloud::slug::Slug::as_str))
		{
			return Ok(org.to_owned());
		}
		self.client
			.organizations(account::Organizations {})?
			.into_iter()
			.find(|org| org.personal)
			.map(|org| org.slug)
			.context("Choose an organization with --org.")
	}

	fn project(&self, args: &CloudArgs) -> Result<String> {
		args.project
			.clone()
			.or_else(|| self.config.project.as_ref().map(ToString::to_string))
			.context("Choose a project with --project, or run `lk cloud link`.")
	}
}

/// A token from the environment goes only to the hosted cloud, or to an
/// origin named outside the project: by `--url` or `LEPTOSKIT_CLOUD_URL`.
/// A `[cloud].url` in a cloned repository must not be able to collect a
/// CI token, the secrets set with it, or the source archive.
fn check_token_origin(token: bool, explicit: Option<&str>, url: &str) -> Result<()> {
	let named = explicit
		.map(transport::origin)
		.transpose()?
		.is_some_and(|origin| origin == url);
	ensure!(
		!token || named || url == DEFAULT_URL,
		"leptoskit.toml points [cloud].url at {}, and LEPTOSKIT_CLOUD_TOKEN only goes to an \
		 origin you name. Pass --url or set LEPTOSKIT_CLOUD_URL to send it there.",
		printable(url)
	);
	Ok(())
}

fn value(value: impl Serialize) -> Result<Value> {
	Ok(serde_json::to_value(value)?)
}
fn confirm(actual: &impl ToString, confirmation: &str) -> Result<()> {
	ensure!(
		actual.to_string() == confirmation,
		"Pass the exact slug or ID with --confirm."
	);
	Ok(())
}

/// Returns true when the nearest `leptoskit.toml` configures `[cloud]` with a
/// linked `project` slug (meaning `lk deploy` should ship to `LeptosKit`
/// Cloud).
pub(crate) fn has_cloud_project() -> Result<bool> {
	let Some(file) = std::env::current_dir()?
		.ancestors()
		.map(|dir| dir.join("leptoskit.toml"))
		.find(|path| path.is_file())
	else {
		return Ok(false);
	};
	let document: toml::Value = toml::from_str(&std::fs::read_to_string(file)?)?;
	Ok(document
		.get("cloud")
		.and_then(|cloud| cloud.get("project"))
		.and_then(toml::Value::as_str)
		.is_some_and(|slug| !slug.trim().is_empty()))
}

/// Dispatches `lk deploy` to `lk cloud deploy` when `[cloud].project` is set.
pub(crate) fn deploy_from_cli(environment: Option<String>, detach: bool) -> Result<()> {
	let args = CloudArgs {
		url: None,
		json: false,
		org: None,
		project: None,
		environment,
		command: CloudCommand::Deploy { detach },
	};
	run(&args)
}

/// Runs one cloud command and returns its structured JSON value (used by MCP
/// tools).
pub(crate) fn execute_json(args: &CloudArgs) -> Result<Value> {
	let context = Context::load(args)?;
	execute(args, &context)
}

/// Executes one cloud command.
///
/// # Errors
/// Invalid configuration, authentication, permissions, network or server
/// errors.
pub fn run(args: &CloudArgs) -> Result<()> {
	if let CloudCommand::External(argv) = &args.command {
		return external::run_external(args, argv);
	}
	let context = Context::load(args)?;
	let result = execute(args, &context)?;
	if args.json {
		anstream::println!("{}", serde_json::to_string_pretty(&result)?);
	} else if matches!(args.command, CloudCommand::Logout) {
		crate::ui::success("Local cloud credentials removed.");
		if result["revoked"] == false {
			crate::ui::warning(
				"The cloud could not revoke this session. It may still be live on the server.",
			);
		}
	} else if matches!(args.command, CloudCommand::Notifications) {
		anstream::println!("{}", ship::notifications_text(&result));
	} else {
		anstream::println!("{}", human(&result));
	}
	Ok(())
}

fn execute(args: &CloudArgs, context: &Context) -> Result<Value> {
	let client = &context.client;
	let requirement = match &args.command {
		CloudCommand::Token { .. } => Requirement::Organization(Permission::ManageTokens),
		CloudCommand::Usage { .. } => Requirement::Organization(Permission::ViewBilling),
		CloudCommand::Org { command } => {
			match command {
				OrgCommand::List => Requirement::SignedIn,
				OrgCommand::Create { .. } => Requirement::Person,
				OrgCommand::Members { .. } => {
					Requirement::Organization(Permission::ViewOrganization)
				}
				_ => Requirement::Organization(Permission::ManageMembers),
			}
		}
		_ => Requirement::SignedIn,
	};
	let person_only = match requirement {
		Requirement::Person => true,
		Requirement::Organization(permission) | Requirement::Project(permission) => {
			permission.requires_person()
		}
		_ => false,
	};
	ensure!(
		!person_only || !client.uses_api_token(),
		"This needs `lk cloud login`. Unset LEPTOSKIT_CLOUD_TOKEN first."
	);
	match &args.command {
		CloudCommand::Launch {
			name,
			region,
			yes,
			detach,
		} => {
			deploy::launch(
				args,
				context,
				name.as_deref(),
				region.as_deref(),
				*yes,
				*detach,
			)
		}
		CloudCommand::Deploy { detach } => deploy::deploy(args, context, *detach),
		CloudCommand::Status => deploy::status(args, context),
		CloudCommand::Logs {
			follow,
			process,
			since_minutes,
			contains,
			limit,
		} => {
			deploy::logs(
				args,
				context,
				*follow,
				process.clone(),
				*since_minutes,
				contains.clone(),
				*limit,
			)
		}
		CloudCommand::Estimate => deploy::estimate(args, context),
		CloudCommand::Open { print } => deploy::open(args, context, *print),
		CloudCommand::Login { no_browser } => {
			client.login(|verification| {
				if args.json {
					anstream::eprintln!(
						"Open {} and enter {}",
						verification.url,
						verification.code
					);
				} else {
					anstream::println!("Open {} and enter {}", verification.url, verification.code);
				}
				if !no_browser && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
				{
					open_browser(&verification.url);
				}
				Ok(())
			})?;
			Ok(json!({ "signed_in": true }))
		}
		CloudCommand::Logout => {
			Ok(json!({ "local_credentials_removed": true, "revoked": client.logout()? }))
		}
		CloudCommand::Whoami => value(client.whoami(account::Whoami {})?),
		CloudCommand::Org { command } => organization(command, client),
		CloudCommand::Project { command } => project(command, args, context),
		CloudCommand::Env { command } => environment(command, args, context),
		CloudCommand::Token { command } => token(command, args, context),
		CloudCommand::Link { force } => link(args, context, *force),
		CloudCommand::Var { command } => ship::var(command, args, context),
		CloudCommand::Secret { command } => {
			ship::secret(command, args, context, &mut ship::read_stdin)
		}
		CloudCommand::Domain { command } => ship::domain(command, args, context),
		CloudCommand::Deployments { limit } => ship::deployments(args, context, *limit),
		CloudCommand::Rollback { to } => ship::rollback(args, context, *to),
		CloudCommand::Db { command } => ship::db(command, args, context),
		CloudCommand::Notifications => ship::notifications(args, context),
		CloudCommand::Publish {
			stores,
			submit,
			detach,
		} => stores::publish(args, context, stores, *submit, *detach),
		CloudCommand::Publications { command } => stores::publications(command, args, context),
		CloudCommand::Usage { period } => stores::usage(args, context, period.clone()),
		CloudCommand::External(_) => {
			anyhow::bail!("External cloud subcommands cannot run through JSON RPC execution.")
		}
	}
}

fn open_browser(url: &str) {
	let program = if cfg!(target_os = "macos") {
		"open"
	} else if cfg!(windows) {
		"explorer"
	} else {
		"xdg-open"
	};
	match std::process::Command::new(program).arg(url).status() {
		Ok(status) if status.success() => {}
		_ => crate::ui::warning("Could not open a browser. Open the URL above manually."),
	}
}

fn organization(command: &OrgCommand, client: &Client) -> Result<Value> {
	match command {
		OrgCommand::List => value(client.organizations(account::Organizations {})?),
		OrgCommand::Create {
			name,
			slug: requested,
		} => {
			value(client.create_organization(account::CreateOrganization {
				name: name.clone(),
				slug: requested.clone().unwrap_or_else(|| slug(name)),
			})?)
		}
		OrgCommand::Members { organization } => {
			value(client.members(account::Members {
				organization: organization.clone(),
			})?)
		}
		OrgCommand::Invite {
			organization,
			email,
			role,
		} => {
			value(client.invite_member(account::InviteMember {
				organization: organization.clone(),
				email: email.clone(),
				role: *role,
			})?)
		}
		OrgCommand::Invitations { organization } => {
			value(client.invitations(account::Invitations {
				organization: organization.clone(),
			})?)
		}
		OrgCommand::RevokeInvitation {
			organization,
			invitation,
			confirm: confirmation,
		} => {
			confirm(invitation, confirmation)?;
			client.revoke_invitation(account::RevokeInvitation {
				organization: organization.clone(),
				invitation: *invitation,
			})?;
			Ok(Value::Null)
		}
		OrgCommand::SetRole {
			organization,
			account,
			role,
		} => {
			client.set_member_role(account::SetMemberRole {
				organization: organization.clone(),
				account: *account,
				role: *role,
			})?;
			Ok(Value::Null)
		}
		OrgCommand::RemoveMember {
			organization,
			account,
			confirm: confirmation,
		} => {
			confirm(account, confirmation)?;
			client.remove_member(account::RemoveMember {
				organization: organization.clone(),
				account: *account,
			})?;
			Ok(Value::Null)
		}
	}
}

fn project(command: &ProjectCommand, args: &CloudArgs, context: &Context) -> Result<Value> {
	let organization = context.organization(args)?;
	let client = &context.client;
	match command {
		ProjectCommand::List => value(client.projects(projects::Projects { organization })?),
		ProjectCommand::Create { name, region } => {
			value(client.create_project(projects::CreateProject {
				organization,
				slug: slug(name),
				name: name.clone(),
				region: region.clone(),
				detected: projects::Detected::default(),
			})?)
		}
		ProjectCommand::Show => {
			value(client.project(projects::Project {
				organization,
				slug: context.project(args)?,
			})?)
		}
		ProjectCommand::Delete {
			slug,
			confirm: confirmation,
		} => {
			confirm(slug, confirmation)?;
			client.delete_project(projects::DeleteProject {
				organization,
				project: slug.clone(),
				confirm: confirmation.clone(),
			})?;
			Ok(Value::Null)
		}
	}
}

fn environment(command: &EnvCommand, args: &CloudArgs, context: &Context) -> Result<Value> {
	let organization = context.organization(args)?;
	let project = context.project(args)?;
	let client = &context.client;
	match command {
		EnvCommand::List => {
			value(client.environments(projects::Environments {
				organization,
				project,
			})?)
		}
		EnvCommand::Create { name, kind } => {
			value(client.create_environment(projects::CreateEnvironment {
				organization,
				project,
				name: name.clone(),
				kind: *kind,
			})?)
		}
		EnvCommand::Update {
			name,
			size,
			min,
			max,
			sleep,
		} => {
			let current = client
				.environments(projects::Environments {
					organization: organization.clone(),
					project: project.clone(),
				})?
				.into_iter()
				.find(|env| &env.name == name)
				.context("Environment not found. Check `lk cloud env list`.")?;
			let mut scale = current.scale;
			if let Some(size) = size {
				scale.app.size = *size;
			}
			if let Some(min) = min {
				scale.app.min = *min;
			}
			if let Some(max) = max {
				scale.app.max = *max;
			}
			if let Some(sleep) = sleep {
				scale.app.sleep = *sleep;
			}
			scale.check().map_err(anyhow::Error::msg)?;
			value(client.update_environment(projects::UpdateEnvironment {
				organization,
				project,
				environment: name.clone(),
				revision: current.revision,
				scale,
			})?)
		}
		EnvCommand::Delete {
			name,
			confirm: confirmation,
		} => {
			confirm(name, confirmation)?;
			client.delete_environment(projects::DeleteEnvironment {
				organization,
				project,
				environment: name.clone(),
				confirm: confirmation.clone(),
			})?;
			Ok(Value::Null)
		}
	}
}

fn token(command: &TokenCommand, args: &CloudArgs, context: &Context) -> Result<Value> {
	let organization = context.organization(args)?;
	let client = &context.client;
	match command {
		TokenCommand::List => value(client.tokens(tokens::Tokens { organization })?),
		TokenCommand::Create {
			name,
			scopes,
			expires,
		} => {
			let created = client.create_token(tokens::CreateToken {
				organization,
				name: name.clone(),
				scopes: scopes.clone(),
				project: args
					.project
					.clone()
					.or_else(|| context.config.project.as_ref().map(ToString::to_string)),
				expires_in_days: Some(*expires),
			})?;
			crate::ui::warning("Save this token now. Its value is shown only once.");
			value(created)
		}
		TokenCommand::Revoke {
			token,
			confirm: confirmation,
		} => {
			confirm(token, confirmation)?;
			client.revoke_token(tokens::RevokeToken {
				organization,
				token: *token,
			})?;
			Ok(Value::Null)
		}
	}
}

fn link(args: &CloudArgs, context: &Context, force: bool) -> Result<Value> {
	let organization = context.organization(args)?;
	let project = context.project(args)?;
	context.client.project(projects::Project {
		organization: organization.clone(),
		slug: project.clone(),
	})?;
	let personal = context
		.client
		.organizations(account::Organizations {})?
		.iter()
		.any(|org| org.slug == organization && org.personal);
	let file = context
		.file
		.as_ref()
		.context("Run `lk cloud link` in a project with leptoskit.toml.")?;
	write_link(file, &organization, &project, &context.url, personal, force)?;
	Ok(json!({ "organization": organization, "project": project, "url": context.url }))
}

fn write_link(
	file: &Path,
	organization: &str,
	project: &str,
	url: &str,
	personal: bool,
	force: bool,
) -> Result<()> {
	let mut document = std::fs::read_to_string(file)?.parse::<toml_edit::DocumentMut>()?;
	let existing_project = document
		.get("cloud")
		.and_then(|cloud| cloud.get("project"))
		.and_then(toml_edit::Item::as_str);
	let existing_org = document
		.get("cloud")
		.and_then(|cloud| cloud.get("organization"))
		.and_then(toml_edit::Item::as_str);
	let existing_url = document
		.get("cloud")
		.and_then(|cloud| cloud.get("url"))
		.and_then(toml_edit::Item::as_str)
		.unwrap_or(DEFAULT_URL);
	ensure!(
		force
			|| existing_project.is_none()
			|| (existing_project == Some(project)
				&& existing_org == (!personal).then_some(organization)
				&& existing_url == url),
		"This project is linked to a different cloud project. Use --force to replace the link."
	);
	set_link_value(&mut document, "project", project);
	if personal {
		document["cloud"]
			.as_table_mut()
			.context("[cloud] must be a table.")?
			.remove("organization");
	} else {
		set_link_value(&mut document, "organization", organization);
	}
	if url == DEFAULT_URL {
		document["cloud"]
			.as_table_mut()
			.context("[cloud] must be a table.")?
			.remove("url");
	} else {
		set_link_value(&mut document, "url", url);
	}
	std::fs::write(file, document.to_string())?;
	Ok(())
}

fn set_link_value(document: &mut toml_edit::DocumentMut, key: &str, value: &str) {
	let decor = document
		.get("cloud")
		.and_then(|table| table.get(key))
		.and_then(toml_edit::Item::as_value)
		.map(|value| value.decor().clone());
	let mut replacement = toml_edit::Value::from(value);
	if let Some(decor) = decor {
		*replacement.decor_mut() = decor;
	}
	document["cloud"][key] = toml_edit::Item::Value(replacement);
}

/// Text from the cloud or the organization's own program, safe for a
/// terminal: control characters other than a line break, and the format
/// characters that reorder or hide text (bidi overrides and isolates,
/// zero-width characters, line and paragraph separators), become spaces.
/// A value can't move the cursor, write the clipboard, retitle the window,
/// or show itself reversed.
pub(super) fn printable(text: &str) -> String {
	text.chars()
		.map(|c| {
			let hidden = (c.is_control() && c != '\n')
				|| matches!(
					c,
					'\u{061C}'
						| '\u{180E}' | '\u{200B}'..='\u{200F}'
						| '\u{2028}'..='\u{202E}'
						| '\u{2060}'..='\u{206F}'
						| '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}'
				);
			if hidden { ' ' } else { c }
		})
		.collect()
}

fn human(value: &Value) -> String {
	match value {
		Value::Null => "Done.".into(),
		Value::Array(values) if values.is_empty() => "None.".into(),
		Value::Array(values) => values.iter().map(human).collect::<Vec<_>>().join("\n\n"),
		Value::Object(fields) => {
			fields
				.iter()
				.map(|(key, value)| format!("{}: {}", printable(key), human(value)))
				.collect::<Vec<_>>()
				.join("\n")
		}
		Value::String(value) => printable(value),
		other => printable(&other.to_string()),
	}
}
