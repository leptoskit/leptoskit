//! Saved graph provisioning plans. Applying infrastructure does not install
//! application artifacts or establish runtime evidence.

use std::fs;
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::deployment::InfraGraph;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

#[cfg(test)]
mod acceptance;
mod inputs;
mod outputs;
#[cfg(test)]
mod tests;

const RECEIPT: &str = "plan.sha256";
const PLAN: &str = "leptoskit.plan";

pub(in crate::infra) fn settings(config: &LeptoskitConfig) -> Result<&InfraGraph> {
	config.validate()?;
	config
		.infra
		.as_ref()
		.and_then(leptoskit_config::infra::InfraConfig::as_graph)
		.context("graph infrastructure declaration required")
}

pub(in crate::infra) fn directory(root: &Path, config: &LeptoskitConfig) -> Result<PathBuf> {
	let infra = settings(config)?;
	let mut dir = root.canonicalize()?;

	for name in ["infra", infra.name.as_str()] {
		dir.push(name);

		match fs::symlink_metadata(&dir) {
			Ok(metadata) => {
				ensure!(
					metadata.is_dir() && !metadata.file_type().is_symlink(),
					"graph infrastructure directory must not be a link or file"
				);
			}

			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
				let builder = fs::DirBuilder::new();
				#[cfg(unix)]
				let builder = {
					use std::os::unix::fs::DirBuilderExt as _;
					let mut builder = builder;
					builder.mode(0o700);
					builder
				};
				builder.create(&dir)?;
			}
			Err(error) => return Err(error.into()),
		}
	}

	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
	}

	Ok(dir)
}

pub(in crate::infra) fn lock(dir: &Path) -> Result<fs::File> {
	#[cfg(unix)]
	{
		use rustix::fs::Mode;
		use rustix::fs::OFlags;
		let file: fs::File = rustix::fs::open(
			dir.join("operation.lock"),
			OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
			Mode::RUSR | Mode::WUSR,
		)?
		.into();
		ensure!(
			file.metadata()?.is_file(),
			"graph operation lock must be an ordinary file"
		);
		fs4::FileExt::try_lock(&file)
			.context("another graph infrastructure operation is running")?;
		Ok(file)
	}
	#[cfg(not(unix))]
	{
		let _ = dir;
		anyhow::bail!("graph provisioning requires a Unix operator host");
	}
}

pub(in crate::infra) fn invalidate(dir: &Path) -> Result<()> {
	remove(&dir.join(RECEIPT))?;
	sync_directory(dir)
}

fn remove(path: &Path) -> Result<()> {
	match fs::remove_file(path) {
		Ok(()) => Ok(()),

		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(error) => Err(error).context("could not invalidate the saved graph plan"),
	}
}

fn sync_directory(dir: &Path) -> Result<()> {
	fs::File::open(dir)?.sync_all()?;
	Ok(())
}

pub(in crate::infra) fn run(
	root: &Path,
	config: &LeptoskitConfig,
	command: &super::super::InfraCommand,
) -> Result<()> {
	use super::super::InfraCommand;
	let dir = directory(root, config)?;
	let _lock = lock(&dir)?;

	if matches!(command, InfraCommand::Plan) {
		invalidate(&dir)?;
	}

	environment()?;

	match command {
		InfraCommand::Plan => {
			invalidate(&dir)?;
			if !dir.join("graph.json").exists() {
				super::generate_locked(root, config, false)?;
			}

			plan(&dir, config, &Tofu, || {
				super::super::pricing::guard_graph(settings(config)?)
			})
		}
		InfraCommand::Apply { offline, yes } => {
			ensure!(
				!offline && !yes,
				"--offline/--yes recovery flags belong to the managed application preset, not \
				 graph provisioning"
			);
			apply(&dir, config, &Tofu, || {
				super::super::pricing::guard_graph(settings(config)?)
			})
		}
		InfraCommand::Output => {
			let value = outputs::read(&dir, config, &Tofu)?;
			println!("{}", serde_json::to_string_pretty(&value)?);
			Ok(())
		}
		_ => anyhow::bail!("this command is not a graph provisioning operation"),
	}
}

/// Explicit CLI arguments and the declared backend own this operation.
fn environment() -> Result<()> {
	for (name, _) in std::env::vars_os() {
		let name = name.to_string_lossy();
		ensure!(
			!name.starts_with("TF_CLI_ARGS")
				&& !matches!(name.as_ref(), "TF_DATA_DIR" | "TF_WORKSPACE"),
			"unset {name} before graph provisioning; implicit OpenTofu argument/data/workspace \
			 overrides are not supported"
		);
	}

	Ok(())
}

trait Engine {
	fn invoke(&self, dir: &Path, args: &[&str], capture: bool) -> Result<Vec<u8>>;
}

struct Tofu;
impl Engine for Tofu {
	fn invoke(&self, dir: &Path, args: &[&str], capture: bool) -> Result<Vec<u8>> {
		let mut command = Command::new("tofu");
		command
			.current_dir(dir)
			.args(args)
			.stdin(Stdio::null())
			.env("TF_INPUT", "false");

		if capture {
			let mut child = command
				.stdout(Stdio::piped())
				.stderr(Stdio::null())
				.spawn()
				.context("could not run OpenTofu; install it on the operator host")?;
			let mut bytes = Vec::new();
			let read = child
				.stdout
				.take()
				.context("OpenTofu output pipe missing")?
				.take(64 * 1024 * 1024 + 1)
				.read_to_end(&mut bytes);

			if read.is_err() || bytes.len() > 64 * 1024 * 1024 {
				let _ = child.kill();

				let _ = child.wait();
				anyhow::bail!("OpenTofu inspection output failed or exceeded its size limit");
			}

			ensure!(
				child.wait()?.success(),
				"OpenTofu inspection failed; output values were not logged"
			);
			Ok(bytes)
		} else {
			let status = command
				.status()
				.context("could not run OpenTofu; install it on the operator host")?;
			ensure!(
				status.success(),
				"OpenTofu operation failed; create a fresh graph plan before retrying apply"
			);
			Ok(Vec::new())
		}
	}
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
	version: String,
	platform: String,
}

fn identity(dir: &Path, engine: &impl Engine) -> Result<Identity> {
	let value: Value = serde_json::from_slice(&engine.invoke(dir, &["version", "-json"], true)?)?;
	let version = value["terraform_version"]
		.as_str()
		.context("OpenTofu did not report its version")?;
	let parsed = semver::Version::parse(version)?;
	ensure!(
		parsed.major == 1 && parsed.minor >= 10 && parsed.pre.is_empty(),
		"graph provisioning requires stable OpenTofu >=1.10,<2"
	);
	let platform = value["platform"]
		.as_str()
		.filter(|platform| !platform.is_empty())
		.context("OpenTofu did not report its platform")?;
	Ok(Identity {
		version: version.into(),
		platform: platform.into(),
	})
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
	format_version: u32,
	configuration_sha256: String,
	engine: Identity,
	workspace: String,
	files: std::collections::BTreeMap<String, String>,
}

fn workspace(dir: &Path, engine: &impl Engine) -> Result<String> {
	let bytes = engine.invoke(dir, &["workspace", "show"], true)?;
	let value = std::str::from_utf8(&bytes)?.trim();
	ensure!(
		value == "default",
		"graph provisioning uses the default workspace; declare independent environments through \
		 infra.name/state.key"
	);
	Ok(value.into())
}

fn plan(
	dir: &Path,
	config: &LeptoskitConfig,
	engine: &impl Engine,
	budget: impl FnOnce() -> Result<()>,
) -> Result<()> {
	invalidate(dir)?;
	remove(&dir.join(PLAN))?;
	let before = inputs::generated(dir, config)?;
	budget()?;
	let engine_id = identity(dir, engine)?;
	engine.invoke(dir, &["init", "-input=false"], false)?;
	let workspace = workspace(dir, engine)?;
	engine.invoke(dir, &["validate"], false)?;
	engine.invoke(
		dir,
		&["plan", "-input=false", "-lock=true", "-out=leptoskit.plan"],
		false,
	)?;
	protect(dir, engine)?;
	ensure!(
		before == inputs::generated(dir, config)?,
		"graph inputs changed while planning; plan again"
	);
	let receipt = Receipt {
		format_version: 1,
		configuration_sha256: inputs::configuration(config)?,
		engine: engine_id,
		workspace,
		files: inputs::fingerprint(dir, config)?,
	};
	let mut file = tempfile::NamedTempFile::new_in(dir)?;
	serde_json::to_writer_pretty(&mut file, &receipt)?;
	file.write_all(b"\n")?;
	file.as_file().sync_all()?;
	file.persist_noclobber(dir.join(RECEIPT))?;
	sync_directory(dir)?;
	println!(
		"Saved graph provisioning plan. Review it with `tofu show leptoskit.plan` in {} and run \
		 `lk infra apply` to apply these exact inputs.",
		dir.display()
	);
	Ok(())
}

fn apply(
	dir: &Path,
	config: &LeptoskitConfig,
	engine: &impl Engine,
	budget: impl FnOnce() -> Result<()>,
) -> Result<()> {
	let root = crate::verify::assets::Root::open(dir)?;
	let receipt: Receipt = serde_json::from_slice(
		&root
			.metadata(RECEIPT)
			.context("run `lk infra plan` first")?,
	)?;
	ensure!(
		receipt.format_version == 1,
		"unsupported saved graph plan receipt"
	);
	ensure!(
		receipt.configuration_sha256 == inputs::configuration(config)?
			&& receipt.files == inputs::fingerprint(dir, config)?,
		"graph configuration or saved plan changed; plan again"
	);
	ensure!(
		receipt.engine == identity(dir, engine)? && receipt.workspace == workspace(dir, engine)?,
		"OpenTofu identity/workspace changed; plan again"
	);
	budget()?;
	protect(dir, engine)?;
	ensure!(
		receipt.files == inputs::fingerprint(dir, config)?,
		"saved graph inputs changed during inspection; plan again"
	);
	invalidate(dir)?;
	engine.invoke(
		dir,
		&["apply", "-input=false", "-lock=true", "leptoskit.plan"],
		false,
	)?;
	println!(
		"Applied graph infrastructure. Application artifacts, host runtimes, certificates and \
		 release activation have not been installed by this operation."
	);
	Ok(())
}

fn protect(dir: &Path, engine: &impl Engine) -> Result<()> {
	let value: Value =
		serde_json::from_slice(&engine.invoke(dir, &["show", "-json", PLAN], true)?)?;
	ensure!(
		value["format_version"]
			.as_str()
			.is_some_and(|version| version.starts_with("1.")),
		"unsupported OpenTofu plan format"
	);
	ensure!(
		value["errored"] != true && value["complete"] != false,
		"OpenTofu plan is incomplete or errored"
	);

	if let Some(changes) = value.get("resource_changes") {
		for change in changes
			.as_array()
			.context("invalid planned resource changes")?
		{
			let kind = change["type"]
				.as_str()
				.context("invalid planned resource type")?;
			let actions = change["change"]["actions"]
				.as_array()
				.context("invalid planned resource actions")?;
			ensure!(
				!actions.is_empty()
					&& actions.iter().all(|action| {
						matches!(
							action.as_str(),
							Some("no-op" | "create" | "read" | "update" | "delete")
						)
					}),
				"unsupported planned resource action"
			);
			ensure!(
				!matches!(
					kind,
					"hcloud_server" | "digitalocean_droplet" | "hcloud_network"
				) || !actions.iter().any(|action| action == "delete"),
				"plan would delete or replace a graph host/network; migrate to a separate \
				 declared environment before retiring it"
			);
		}
	}

	Ok(())
}

/// Read typed public provider bindings and reproducible generated inputs.
pub(super) fn release_inputs(dir: &Path, config: &LeptoskitConfig) -> Result<Value> {
	environment()?;
	Ok(serde_json::json!({
		"files": inputs::generated(dir, config)?,
		"outputs": outputs::read(dir, config, &Tofu)?,
		"configuration": inputs::configuration(config)?
	}))
}

/// Recheck the exact retained inputs of an independently pinned recovery plan.
/// This never authorizes new provisioning or replaces current-generation
/// checks.
pub(super) fn recovery_inputs(
	dir: &Path,
	config: &LeptoskitConfig,
	expected: &Value,
) -> Result<Value> {
	environment()?;
	let configuration = inputs::configuration(config)?;
	ensure!(
		expected["configuration"] == configuration,
		"configuration differs from historical recovery plan"
	);
	let files = inputs::historical(dir, config, &expected["files"])?;
	let outputs = outputs::read_bound(dir, config, &Tofu)?;
	ensure!(
		outputs == expected["outputs"],
		"provider bindings differ from historical recovery plan"
	);
	ensure!(
		inputs::historical(dir, config, &expected["files"])? == files,
		"historical files changed during inspection"
	);
	Ok(serde_json::json!({"files":files,"outputs":outputs,"configuration":configuration}))
}
