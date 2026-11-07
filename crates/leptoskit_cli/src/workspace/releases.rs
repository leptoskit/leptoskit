//! Ordered release execution; the journal is progress, never deployment proof.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;

mod plan;

/// Freeze a reviewed set of prebuilt image releases into a local plan.
#[derive(Debug, clap::Args)]
pub struct PlanArgs {
	/// Workspace manifest; otherwise locate it from the current directory.
	#[arg(long)]
	pub manifest: Option<PathBuf>,
	/// TOML release set, with versions and independently authenticated pins.
	#[arg(long)]
	pub releases: PathBuf,
	/// New plan file; never overwrites an existing plan.
	#[arg(long)]
	pub out: PathBuf,
}

/// Apply the exact reviewed plan using per-application release reservations.
#[derive(Debug, clap::Args)]
pub struct DeployArgs {
	/// Workspace manifest; otherwise locate it from the current directory.
	#[arg(long)]
	pub manifest: Option<PathBuf>,
	/// Reviewed JSON plan written by workspace plan.
	#[arg(long)]
	pub plan: PathBuf,
	/// Independently reviewed SHA-256 of the exact plan bytes.
	#[arg(long)]
	pub sha256: String,
	/// Maximum seconds for each application's local release process tree.
	#[arg(long, default_value_t = 900, value_parser = clap::value_parser!(u64).range(1..=7200))]
	pub timeout_seconds: u64,
}

const MAX_DOCUMENT: usize = 1024 * 1024;
const MAX_DESCRIPTOR: usize = 256 * 1024 * 1024;

fn hash(bytes: &[u8]) -> String {
	hex::encode(sha2::Sha256::digest(bytes))
}

fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
	ensure!(
		fs::metadata(path)?.is_file(),
		"input must be a regular file"
	);
	let file = fs::File::open(path).with_context(|| format!("read {}", path.display()))?;
	ensure!(file.metadata()?.is_file(), "input must be a regular file");
	let mut bytes = Vec::new();
	file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
	ensure!(bytes.len() <= limit, "input exceeds its size limit");
	Ok(bytes)
}

fn manifest(path: Option<&Path>) -> Result<PathBuf> {
	path.map_or_else(
		|| super::find_manifest(&std::env::current_dir()?),
		|path| Ok(path.to_owned()),
	)?
	.canonicalize()
	.context("locate workspace manifest")
}

pub(super) fn plan(args: &PlanArgs) -> Result<()> {
	let manifest = manifest(args.manifest.as_deref())?;
	let release_set = read(&args.releases, MAX_DOCUMENT)?;
	let release_set = toml::from_str(std::str::from_utf8(&release_set)?)?;
	let plan = plan::create(&manifest, release_set)?;
	let bytes = serde_json::to_vec_pretty(&plan)?;
	let mut file = fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(&args.out)?;
	file.write_all(&bytes)?;
	file.sync_all()?;
	println!("Plan: {}\nSHA-256: {}", args.out.display(), hash(&bytes));
	println!(
		"Not checked: operator authorization, live readiness, private connectivity, or caller \
		 identity."
	);
	Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
	format_version: u32,
	plan_sha256: String,
	applications: BTreeMap<String, Progress>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Progress {
	attempts: u32,
	state: State,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum State {
	Pending,
	Running,
	CommandCompleted,
	Failed,
}

fn save(path: &Path, journal: &Journal) -> Result<()> {
	let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("journal needs parent")?)?;
	temp.write_all(&serde_json::to_vec_pretty(journal)?)?;
	temp.as_file().sync_all()?;
	temp.persist(path).map_err(|error| error.error)?;
	#[cfg(unix)]
	fs::File::open(path.parent().unwrap())?.sync_all()?;
	Ok(())
}

fn journal(path: &Path, plan: &plan::Plan, pin: &str) -> Result<Journal> {
	if path.exists() {
		let journal: Journal = serde_json::from_slice(&read(path, MAX_DOCUMENT)?)?;
		ensure!(
			journal.format_version == 1 && journal.plan_sha256 == pin,
			"journal belongs to another plan"
		);
		ensure!(
			journal.applications.keys().eq(plan.applications.keys()),
			"journal application membership differs"
		);
		return Ok(journal);
	}
	Ok(Journal {
		format_version: 1,
		plan_sha256: pin.into(),
		applications: plan
			.applications
			.keys()
			.map(|name| {
				(
					name.clone(),
					Progress {
						attempts: 0,
						state: State::Pending,
					},
				)
			})
			.collect(),
	})
}

pub(super) fn deploy(args: &DeployArgs) -> Result<()> {
	deploy_with(args, &std::env::current_exe()?)
}

fn deploy_with(args: &DeployArgs, executable: &Path) -> Result<()> {
	let manifest = manifest(args.manifest.as_deref())?;
	let root = manifest.parent().context("workspace needs parent")?;
	let bytes = read(&args.plan, MAX_DOCUMENT)?;
	ensure!(
		hash(&bytes) == args.sha256,
		"plan differs from independently reviewed SHA-256"
	);
	let plan: plan::Plan = serde_json::from_slice(&bytes)?;
	plan::validate(&manifest, &plan)?;
	let environments = plan
		.applications
		.iter()
		.map(|(name, app)| {
			resolved_environment(name, app).map(|environment| (name.clone(), environment))
		})
		.collect::<Result<BTreeMap<_, _>>>()?;
	let state = root.join(".leptoskit/workspace");
	fs::create_dir_all(&state)?;
	ensure!(
		state.canonicalize()?.starts_with(root),
		"workspace state escapes its root"
	);
	let lock = fs::OpenOptions::new()
		.create(true)
		.truncate(false)
		.write(true)
		.open(state.join("operation.lock"))?;
	fs4::FileExt::try_lock(&lock).context("another workspace release is running")?;
	let journal_path = state.join(format!("{}.json", args.sha256));
	let mut journal = journal(&journal_path, &plan, &args.sha256)?;
	// Stage every exact descriptor before the first deployment. Never rely on
	// mutable source paths after a previous application's traffic changed.
	let staging = tempfile::tempdir_in(&state)?;
	plan::stage(root, &plan, staging.path())?;
	save(&journal_path, &journal)?;
	for name in &plan.startup_order {
		let application = &plan.applications[name];
		plan::unchanged(&manifest, &plan, name)?;
		let progress = journal
			.applications
			.get_mut(name)
			.context("missing progress")?;
		progress.attempts = progress
			.attempts
			.checked_add(1)
			.context("attempt counter exhausted")?;
		progress.state = State::Running;
		save(&journal_path, &journal)?;
		println!("Workspace release: {name}");
		let outcome = execute(
			executable,
			root,
			name,
			application,
			staging.path(),
			&environments[name],
			Duration::from_secs(args.timeout_seconds),
		);
		journal.applications.get_mut(name).unwrap().state = if outcome.is_ok() {
			State::CommandCompleted
		} else {
			State::Failed
		};
		save(&journal_path, &journal)?;
		outcome.with_context(|| {
			format!(
				"application {name} did not complete; earlier apps retain their releases; journal \
				 {}",
				journal_path.display()
			)
		})?;
	}
	println!(
		"Workspace release commands completed; journal {}",
		journal_path.display()
	);
	Ok(())
}

fn execute(
	executable: &Path,
	root: &Path,
	name: &str,
	application: &plan::Application,
	staging: &Path,
	environment: &BTreeMap<String, String>,
	timeout: Duration,
) -> Result<()> {
	let mut command = Command::new(executable);
	command
		.current_dir(root.join(&application.path))
		.args([
			"deploy",
			"--version",
			&application.version,
			"--reuse-current",
			"--verified-descriptor",
		])
		.arg(staging.join(format!("{name}.json")))
		.args(["--descriptor-sha256", &application.descriptor_sha256])
		.args(["--configuration-sha256", &application.configuration_sha256])
		.env("LEPTOSKIT_CONTROL_LOCAL", "1");
	command.envs(environment);
	if application.setup {
		command.arg("--setup");
	}
	if application.maintenance {
		command.arg("--maintenance");
	}
	crate::testing::isolate(&mut command);
	let mut child = command.spawn().context("start per-application release")?;
	let start = Instant::now();
	loop {
		if let Some(status) = child.try_wait()? {
			ensure!(
				status.success(),
				"per-application release failed ({status})"
			);
			return Ok(());
		}
		if start.elapsed() >= timeout {
			crate::testing::terminate(&mut child)?;
			child.wait()?;
			anyhow::bail!(
				"release deadline exceeded; remote reservation may remain; inspect infra status \
				 before recovery"
			);
		}
		std::thread::sleep(Duration::from_millis(100));
	}
}

fn resolved_environment(
	name: &str,
	application: &plan::Application,
) -> Result<BTreeMap<String, String>> {
	application
		.environment
		.iter()
		.map(|(destination, source)| {
			let value = std::env::var(source)
				.ok()
				.filter(|value| !value.is_empty())
				.with_context(|| {
					format!("set application {name} environment reference {source}")
				})?;
			Ok((destination.clone(), value))
		})
		.collect()
}

#[cfg(test)]
mod tests;
