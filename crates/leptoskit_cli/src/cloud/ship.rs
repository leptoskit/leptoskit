//! An environment's configuration and history: variables, secrets, domains,
//! deployments, rollback and the database (CLI 2).

use std::io::IsTerminal as _;
use std::io::Read as _;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::bail;
use anyhow::ensure;
use clap::Subcommand;
use leptoskit_cloud::config;
use leptoskit_cloud::databases;
use leptoskit_cloud::deployments;
use leptoskit_cloud::domains;
use serde_json::Value;

use super::CloudArgs;
use super::Context;
use super::confirm;
use super::value;

/// Variables: plain values the app reads from its environment.
#[derive(Debug, Subcommand)]
pub enum VarCommand {
	/// Lists the environment's variables, with their values.
	List,
	/// Sets a variable. It applies at the environment's next deployment.
	Set { name: String, value: String },
	/// Removes a variable at the environment's next deployment.
	Unset { name: String },
}

/// Secrets: values the cloud keeps sealed and never shows again.
#[derive(Debug, Subcommand)]
pub enum SecretCommand {
	/// Lists the environment's secrets: names and versions, never values.
	List,
	/// Stores a secret, read from standard input or from a variable of
	/// this shell, never from the command line. It applies at the
	/// environment's next deployment.
	Set {
		name: String,
		/// Reads the value from this environment variable instead of
		/// standard input.
		#[arg(long, value_name = "VARIABLE")]
		from_env: Option<String>,
	},
	/// Removes a secret at the environment's next deployment.
	Unset { name: String },
}

/// Custom domains.
#[derive(Debug, Subcommand)]
pub enum DomainCommand {
	/// Lists the environment's domains and the DNS records each needs.
	List,
	/// Attaches a domain and shows the DNS records to publish.
	Add { hostname: String },
	/// Looks for the domain's challenge record and marks it verified.
	Verify { hostname: String },
	/// Detaches a domain; requires the hostname again as confirmation.
	Remove {
		hostname: String,
		#[arg(long)]
		confirm: String,
	},
}

/// The environment's managed Postgres database.
#[derive(Debug, Subcommand)]
pub enum DbCommand {
	/// Shows the environment's database, if it has one.
	Show,
	/// Lists the database's backups and its latest restore.
	Backups,
	/// Asks for a backup of the database now.
	Backup,
	/// Puts a backup back over the database, replacing everything written
	/// since; requires `<project>/<environment>` as confirmation.
	Restore {
		backup: uuid::Uuid,
		#[arg(long)]
		confirm: String,
	},
}

/// The environment `--environment` names, production without one.
pub(super) fn environment(args: &CloudArgs) -> String {
	args.environment
		.clone()
		.unwrap_or_else(|| leptoskit_cloud::projects::PRODUCTION.to_owned())
}

/// Organization, project and environment, as every command here needs.
struct Target {
	organization: String,
	project: String,
	environment: String,
}

impl Target {
	fn of(args: &CloudArgs, context: &Context) -> Result<Self> {
		Ok(Self {
			organization: context.organization(args)?,
			project: context.project(args)?,
			environment: environment(args),
		})
	}

	/// Says which environment a change is about to reach, on standard
	/// error, so production is never changed unawares.
	fn announce(&self, args: &CloudArgs, what: &str) {
		announce(
			args,
			&self.organization,
			&self.project,
			&self.environment,
			what,
		);
	}
}

/// Says which environment a change is about to reach, on standard error.
pub(super) fn announce(
	args: &CloudArgs,
	organization: &str,
	project: &str,
	environment: &str,
	what: &str,
) {
	if !args.json {
		anstream::eprintln!(
			"{what} in {}/{}/{}.",
			super::printable(organization),
			super::printable(project),
			super::printable(environment)
		);
	}
}

pub(super) fn var(command: &VarCommand, args: &CloudArgs, context: &Context) -> Result<Value> {
	let target = Target::of(args, context)?;
	match command {
		VarCommand::List => {}
		VarCommand::Set { name, .. } => target.announce(args, &format!("Setting {name}")),
		VarCommand::Unset { name } => target.announce(args, &format!("Removing {name}")),
	}
	let Target {
		organization,
		project,
		environment,
	} = target;
	let client = &context.client;
	match command {
		VarCommand::List => {
			value(client.variables(config::Variables {
				organization,
				project,
				environment,
			})?)
		}
		VarCommand::Set { name, value: text } => {
			value(client.set_variable(config::SetVariable {
				organization,
				project,
				environment,
				name: name.clone(),
				value: text.clone(),
			})?)
		}
		VarCommand::Unset { name } => {
			client.unset_variable(config::UnsetVariable {
				organization,
				project,
				environment,
				name: name.clone(),
			})?;
			Ok(Value::Null)
		}
	}
}

pub(super) fn secret(
	command: &SecretCommand,
	args: &CloudArgs,
	context: &Context,
	stdin: &mut dyn FnMut() -> Result<SecretInput>,
) -> Result<Value> {
	let target = Target::of(args, context)?;
	match command {
		SecretCommand::List => {}
		SecretCommand::Set { name, .. } => target.announce(args, &format!("Storing {name}")),
		SecretCommand::Unset { name } => target.announce(args, &format!("Removing {name}")),
	}
	let Target {
		organization,
		project,
		environment,
	} = target;
	let client = &context.client;
	match command {
		SecretCommand::List => {
			value(client.secrets(config::Secrets {
				organization,
				project,
				environment,
			})?)
		}
		SecretCommand::Set { name, from_env } => {
			let text = match from_env {
				// Never keep `VarError` as a cause: a value that isn't
				// UTF-8 would be printed in it.
				Some(variable) => {
					std::env::var_os(variable)
						.with_context(|| format!("${variable} isn't set. Export it, then retry."))?
						.into_string()
						.map_err(|_| {
							anyhow::anyhow!(
								"${variable} isn't UTF-8 text. Encode the value as base64 first."
							)
						})?
				}
				None => {
					match stdin()? {
						SecretInput::Text(text) => text,
						SecretInput::Terminal => {
							bail!(
								"Pipe the value in, such as `printf %s \"$VALUE\" | lk cloud \
								 secret set {name}`, or pass --from-env <VARIABLE>. A value on \
								 the command line would stay in your shell's history."
							)
						}
					}
				}
			};
			ensure!(
				!text.is_empty(),
				"The value is empty. Pipe in the value to store."
			);
			value(client.set_secret(&config::SetSecret {
				organization,
				project,
				environment,
				name: name.clone(),
				value: config::SecretValue::new(text),
			})?)
		}
		SecretCommand::Unset { name } => {
			client.unset_secret(config::UnsetSecret {
				organization,
				project,
				environment,
				name: name.clone(),
			})?;
			Ok(Value::Null)
		}
	}
}

/// What standard input holds for `secret set`.
pub(super) enum SecretInput {
	/// A terminal: nothing was piped in.
	Terminal,
	/// What was piped in, without one trailing line break.
	Text(String),
}

/// Reads a piped secret, at most 64 KiB, dropping one trailing line break
/// so `echo` works as well as `printf`.
pub(super) fn read_stdin() -> Result<SecretInput> {
	let stdin = std::io::stdin();
	if stdin.is_terminal() {
		return Ok(SecretInput::Terminal);
	}
	let mut bytes = Vec::new();
	stdin
		.lock()
		.take(64 * 1024 + 1)
		.read_to_end(&mut bytes)
		.context("Could not read the value from standard input.")?;
	ensure!(
		bytes.len() <= 64 * 1024,
		"The value is over 64 KiB. Store a smaller value."
	);
	let mut text = String::from_utf8(bytes)
		.map_err(|_| anyhow::anyhow!("The value isn't UTF-8 text. Encode it as base64 first."))?;
	if text.ends_with('\n') {
		text.pop();
		if text.ends_with('\r') {
			text.pop();
		}
	}
	Ok(SecretInput::Text(text))
}

pub(super) fn domain(
	command: &DomainCommand,
	args: &CloudArgs,
	context: &Context,
) -> Result<Value> {
	let target = Target::of(args, context)?;
	match command {
		DomainCommand::List | DomainCommand::Verify { .. } => {}
		DomainCommand::Add { hostname } => target.announce(args, &format!("Attaching {hostname}")),
		DomainCommand::Remove { hostname, .. } => {
			target.announce(args, &format!("Detaching {hostname}"));
		}
	}
	let Target {
		organization,
		project,
		environment,
	} = target;
	let client = &context.client;
	match command {
		DomainCommand::List => {
			value(client.domains(domains::Domains {
				organization,
				project,
				environment,
			})?)
		}
		DomainCommand::Add { hostname } => {
			value(client.attach_domain(domains::AttachDomain {
				organization,
				project,
				environment,
				hostname: hostname.clone(),
			})?)
		}
		DomainCommand::Verify { hostname } => {
			value(client.verify_domain(domains::VerifyDomain {
				organization,
				project,
				environment,
				hostname: hostname.clone(),
			})?)
		}
		DomainCommand::Remove {
			hostname,
			confirm: confirmation,
		} => {
			confirm(hostname, confirmation)?;
			client.detach_domain(domains::DetachDomain {
				organization,
				project,
				environment,
				hostname: hostname.clone(),
			})?;
			Ok(Value::Null)
		}
	}
}

pub(super) fn deployments(
	args: &CloudArgs,
	context: &Context,
	limit: Option<u32>,
) -> Result<Value> {
	value(context.client.deployments(deployments::Deployments {
		organization: context.organization(args)?,
		project: context.project(args)?,
		environment: Some(environment(args)),
		before: None,
		limit,
	})?)
}

pub(super) fn rollback(args: &CloudArgs, context: &Context, to: u64) -> Result<Value> {
	let target = Target::of(args, context)?;
	target.announce(args, &format!("Rolling back to deployment {to}"));
	let Target {
		organization,
		project,
		environment,
	} = target;
	value(context.client.rollback(deployments::Rollback {
		organization,
		project,
		environment,
		to,
	})?)
}

pub(super) fn db(command: &DbCommand, args: &CloudArgs, context: &Context) -> Result<Value> {
	let target = Target::of(args, context)?;
	match command {
		DbCommand::Show | DbCommand::Backups => {}
		DbCommand::Backup => target.announce(args, "Backing up the database"),
		DbCommand::Restore { backup, .. } => {
			target.announce(args, &format!("Restoring backup {backup}"));
		}
	}
	let Target {
		organization,
		project,
		environment,
	} = target;
	let client = &context.client;
	match command {
		DbCommand::Show => {
			value(client.database(databases::Database {
				organization,
				project,
				environment,
			})?)
		}
		DbCommand::Backups => {
			value(client.database_backups(databases::DatabaseBackups {
				organization,
				project,
				environment,
			})?)
		}
		DbCommand::Backup => {
			value(client.create_backup(databases::CreateBackup {
				organization,
				project,
				environment,
			})?)
		}
		DbCommand::Restore {
			backup,
			confirm: confirmation,
		} => {
			confirm(&format!("{project}/{environment}"), confirmation)?;
			value(client.restore_backup(databases::RestoreBackup {
				organization,
				project,
				environment,
				backup: *backup,
				confirm: confirmation.clone(),
			})?)
		}
	}
}

pub(super) fn notifications(args: &CloudArgs, context: &Context) -> Result<Value> {
	let Target {
		organization,
		project,
		environment,
	} = Target::of(args, context)?;
	value(
		context
			.client
			.notifications(leptoskit_cloud::notifications::Notifications {
				organization,
				project,
				environment,
			})?,
	)
}

/// `lk cloud notifications` for a person: what the state means, and what
/// to do about it, rather than the summary's fields.
pub(super) fn notifications_text(summary: &Value) -> String {
	if summary.is_null() {
		return "This environment has no hosted notifications. Add [services.notifications] with \
		        crate = \"leptoskit_notifications\" to leptoskit.toml and deploy, and the \
		        cloud's shared notifications service serves them."
			.to_owned();
	}

	let text = |field: &str| super::printable(summary[field].as_str().unwrap_or_default());
	let state = match summary["state"]["state"].as_str().unwrap_or_default() {
		"pending" => "pending: no deployment that uses them has been applied yet".to_owned(),
		"unused" => {
			"unused: the live deployment doesn't declare them; their inbox is kept".to_owned()
		}
		"waiting" => "starting: the shared service picks them up within a minute".to_owned(),
		"serving" => "serving".to_owned(),
		"failed" => {
			format!(
				"failed: {}",
				super::printable(summary["state"]["reason"].as_str().unwrap_or_default())
			)
		}
		"removing" => "removing: the environment is being deleted".to_owned(),
		other => super::printable(other),
	};
	let providers = summary["providers"]
		.as_array()
		.map(|providers| {
			providers
				.iter()
				.filter_map(Value::as_str)
				.map(super::printable)
				.collect::<Vec<_>>()
		})
		.unwrap_or_default();
	let providers = if providers.is_empty() {
		"none (set LEPTOSKIT_NOTIFICATIONS_* with `lk cloud secret set`)".to_owned()
	} else {
		providers.join(", ")
	};

	let mut lines = vec![
		format!("Notifications: {state}"),
		format!("Push providers: {providers}"),
		format!(
			"Pushes this month (whole project): {}",
			summary["project_pushes_this_month"]
				.as_u64()
				.unwrap_or_default()
		),
		format!("Database: {}", text("database")),
	];
	if let Some(reported) = summary["reported_at"].as_str() {
		lines.push(format!("Last report: {}", super::printable(reported)));
	}
	if summary["pushes_paused"] == true {
		lines.push(
			"Pushes are paused: the organisation's free plan has used this month's push \
			 allowance. They resume next month or on a paid plan, and the inbox keeps working."
				.to_owned(),
		);
	}
	if summary["rotating"] == true {
		lines.push(
			"Its credentials are being replaced because a worker that held them was revoked. \
			 Deploy to finish. The shared service held your push provider keys too: rotate them \
			 with each provider if the audit log says the notifications hub was revoked."
				.to_owned(),
		);
	}
	lines.join("\n")
}
