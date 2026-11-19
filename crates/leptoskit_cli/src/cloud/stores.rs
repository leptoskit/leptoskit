//! Publishing to stores from the cloud, and what the organization used
//! (CLI 3).

use std::path::Path;
use std::time::Duration;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::bail;
use clap::Subcommand;
use leptoskit_cloud::archive;
use leptoskit_cloud::billing;
use leptoskit_cloud::deployments;
use leptoskit_cloud::operator::JobState;
use leptoskit_cloud::publishing;
use leptoskit_cloud::workers::PublishOutcome;
use leptoskit_cloud::workers::PublishReport;
use leptoskit_cloud::workers::StepTaker;
use serde_json::Value;

use super::CloudArgs;
use super::Context;
use super::printable;
use super::ship::environment;
use super::value;
use crate::publish::steps;

/// How often a followed publish is asked where it stands.
const POLL: Duration = Duration::from_secs(2);

/// A publish's history.
#[derive(Debug, Subcommand)]
pub enum PublicationCommand {
	/// Lists the environment's publishes, newest first.
	List {
		#[arg(long)]
		limit: Option<u32>,
	},
	/// Shows one publish, and its report once it has ended.
	Show { id: uuid::Uuid },
	/// Prints what a publish printed, following it until it ends.
	Logs { id: uuid::Uuid },
}

pub(super) fn store(value: &str) -> Result<String, String> {
	leptoskit_cloud::workers::PUBLISH_STORES
		.contains(&value)
		.then(|| value.to_owned())
		.ok_or_else(|| {
			format!(
				"Use {}. The cloud doesn't publish the macOS download.",
				leptoskit_cloud::workers::PUBLISH_STORES.join(", ")
			)
		})
}

/// Packs the project, uploads the archive unless the organization has it,
/// and queues a publish to `stores`; then follows it unless `detach`.
pub(super) fn publish(
	args: &CloudArgs,
	context: &Context,
	stores: &[String],
	submit: bool,
	detach: bool,
) -> Result<Value> {
	let organization = context.organization(args)?;
	let project = context.project(args)?;
	let environment = environment(args);
	let root = context
		.file
		.as_deref()
		.and_then(Path::parent)
		.context("Run `lk cloud publish` in a project with leptoskit.toml.")?;
	refuse_root(root)?;
	super::ship::announce(
		args,
		&organization,
		&project,
		&environment,
		&format!(
			"Publishing to {}{}",
			stores.join(", "),
			if submit {
				", and submitting for review or release,"
			} else {
				""
			}
		),
	);
	let packed = archive::pack(root, &archive::PackOptions::default())
		.map_err(|error| anyhow::anyhow!("Could not pack the project: {error}."))?;
	note(
		args,
		&format!(
			"Packed {} files ({} bytes) from {} as {}.",
			packed.files.len(),
			packed.bytes.len(),
			printable(&root.display().to_string()),
			packed.digest.as_str()
		),
	);
	upload(context, &organization, &project, &packed)?;
	let publication = context.client.publish(publishing::Publish {
		organization: organization.clone(),
		project: project.clone(),
		environment: environment.clone(),
		archive: packed.digest,
		stores: stores.to_vec(),
		submit,
	})?;
	if detach {
		return value(publication);
	}
	note(
		args,
		&format!(
			"Publishing {} from the cloud ({}). Ctrl-C stops watching, not the publish.",
			publication.stores.join(", "),
			publication.id
		),
	);
	let summary = follow(
		context,
		&organization,
		&project,
		&environment,
		publication.id,
	)?;
	ended(args, summary)
}

/// Refuses to pack a directory that is plainly not one project: the
/// filesystem's root, the home directory, or one holding SSH keys or this
/// CLI's own sign-in. A stray `leptoskit.toml` above the project would
/// otherwise send all of it to the cloud.
fn refuse_root(root: &Path) -> Result<()> {
	let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
	let canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
	let is_home = home
		.as_deref()
		.and_then(|home| home.canonicalize().ok())
		.is_some_and(|home| home == canonical);
	if canonical.parent().is_none()
		|| is_home
		|| root.join(".ssh").exists()
		|| root.join(".config/leptoskit").exists()
	{
		bail!(
			"{} holds leptoskit.toml, but it looks like a home or system directory, not a \
			 project. Run `lk cloud publish` in the project's own directory.",
			printable(&root.display().to_string())
		);
	}
	Ok(())
}

fn upload(
	context: &Context,
	organization: &str,
	project: &str,
	packed: &archive::Packed,
) -> Result<()> {
	let began = context.client.begin_upload(deployments::BeginUpload {
		organization: organization.to_owned(),
		project: project.to_owned(),
		digest: packed.digest.clone(),
		size: packed.bytes.len() as u64,
	})?;
	match began {
		deployments::BeganUpload::Stored { .. } => Ok(()),
		deployments::BeganUpload::Upload { ticket } => {
			super::transport::put(&ticket, &packed.bytes, context.client.is_loopback())?;
			context
				.client
				.complete_upload(deployments::CompleteUpload {
					organization: organization.to_owned(),
					project: project.to_owned(),
					digest: packed.digest.clone(),
				})?;
			Ok(())
		}
	}
}

/// Prints a publish's log as it grows, and answers where it ended.
fn follow(
	context: &Context,
	organization: &str,
	project: &str,
	environment: &str,
	id: uuid::Uuid,
) -> Result<publishing::PublicationSummary> {
	let mut after = None;
	loop {
		let summary = context.client.publication(publishing::Publication {
			organization: organization.to_owned(),
			project: project.to_owned(),
			environment: environment.to_owned(),
			id,
		})?;
		// Read the log after the state, so the lines of a run that just
		// ended are all printed before its report.
		loop {
			let page = context.client.publication_log(publishing::PublicationLog {
				organization: organization.to_owned(),
				project: project.to_owned(),
				environment: environment.to_owned(),
				id,
				after,
				limit: None,
			})?;
			for line in &page.lines {
				anstream::eprintln!("{}", printable(&line.text));
			}
			match page.next {
				Some(next) if !page.lines.is_empty() => after = Some(next),
				_ => break,
			}
		}
		if matches!(summary.state, JobState::Done | JobState::Abandoned) {
			return Ok(summary);
		}
		std::thread::sleep(POLL);
	}
}

/// A publish that has ended: its report, or the reason it failed.
fn ended(args: &CloudArgs, summary: publishing::PublicationSummary) -> Result<Value> {
	match &summary.outcome {
		Some(PublishOutcome::Published(report)) => {
			if !args.json {
				anstream::println!("{}", report_text(report));
			}
			if args.json {
				value(summary)
			} else {
				Ok(Value::Null)
			}
		}
		Some(PublishOutcome::Failed { reason }) => {
			bail!("The publish failed: {}", printable(reason))
		}
		Some(PublishOutcome::Stopped) => bail!("The publish was stopped."),
		None => {
			bail!(
				"The publish ended without a report. Check `lk cloud publications show {}`.",
				summary.id
			)
		}
	}
}

/// Prints the cloud's report as `lk publish` prints its own.
pub(super) fn report_text(report: &PublishReport) -> String {
	steps::Report {
		stores: report
			.stores
			.iter()
			.map(|status| {
				steps::Status {
					store: line(&status.store),
					summary: printable(&status.summary),
				}
			})
			.collect(),
		steps: report
			.steps
			.iter()
			.map(|step| {
				steps::Step {
					store: line(&step.store),
					by: match step.by {
						StepTaker::Agent => steps::Who::Agent,
						StepTaker::Person => steps::Who::Person,
					},
					what: printable(&step.what),
					url: step.url.as_deref().map(line),
					enter: step
						.enter
						.iter()
						.map(|field| {
							steps::Field {
								label: line(&field.label),
								value: printable(&field.value),
							}
						})
						.collect(),
					why: printable(&step.why),
					done_when: printable(&step.done_when),
				}
			})
			.collect(),
	}
	.text()
}

/// A field that is one line: [`printable`], with line breaks as spaces too,
/// so it can't fake lines of the report.
fn line(text: &str) -> String {
	printable(text).replace('\n', " ")
}

fn note(args: &CloudArgs, text: &str) {
	if !args.json {
		anstream::eprintln!("{text}");
	}
}

pub(super) fn publications(
	command: &PublicationCommand,
	args: &CloudArgs,
	context: &Context,
) -> Result<Value> {
	let organization = context.organization(args)?;
	let project = context.project(args)?;
	let environment = environment(args);
	match command {
		PublicationCommand::List { limit } => {
			value(context.client.publications(publishing::Publications {
				organization,
				project,
				environment,
				limit: *limit,
			})?)
		}
		PublicationCommand::Show { id } => {
			let summary = context.client.publication(publishing::Publication {
				organization,
				project,
				environment,
				id: *id,
			})?;
			if !args.json
				&& let Some(PublishOutcome::Published(report)) = &summary.outcome
			{
				anstream::eprintln!("{}", report_text(report));
			}
			value(summary)
		}
		PublicationCommand::Logs { id } => {
			follow(context, &organization, &project, &environment, *id)?;
			Ok(Value::Null)
		}
	}
}

pub(super) fn usage(args: &CloudArgs, context: &Context, period: Option<String>) -> Result<Value> {
	value(context.client.usage(billing::Usage {
		organization: context.organization(args)?,
		period,
	})?)
}
