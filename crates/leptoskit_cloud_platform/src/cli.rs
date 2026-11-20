//! Command boundaries and conservative local file ownership.
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use clap::Parser;
use clap::Subcommand;

use crate::Generated;
use crate::PlatformError;
use crate::config::PlatformConfig;
use crate::config::Role;
use crate::config::require;

/// Generate inspectable platform files without running provider or cluster
/// tools.
#[derive(Debug, Parser)]
#[command(name = "leptoskit-cloud-platform", version)]
pub struct Cli {
	/// Offline operation to perform.
	#[command(subcommand)]
	pub command: Operation,
}

/// Supported offline commands.
#[derive(Debug, Subcommand)]
pub enum Operation {
	/// Generate phase-one `OpenTofu` files and Kubernetes add-ons.
	Generate {
		/// Operator TOML file.
		#[arg(long)]
		config: PathBuf,
		/// Output directory, with tracked generated-file ownership.
		#[arg(long)]
		out: PathBuf,
	},
	/// Generate phase-two node documents after applying `OpenTofu`.
	Bootstrap {
		/// Operator TOML file.
		#[arg(long)]
		config: PathBuf,
		/// JSON produced by tofu output -json (required for cloud nodes).
		#[arg(long)]
		outputs: Option<PathBuf>,
		/// Output directory.
		#[arg(long)]
		out: PathBuf,
	},
	/// Parse and validate the platform declaration.
	Check {
		/// Operator TOML file.
		#[arg(long)]
		config: PathBuf,
	},
	/// Print declared ownership, topology and failure domains.
	PlanSummary {
		/// Operator TOML file.
		#[arg(long)]
		config: PathBuf,
	},
	/// Provision infrastructure, bootstrap nodes, and apply Kubernetes add-ons
	/// and platform workloads in strict phase order.
	Up {
		/// Operator TOML file.
		#[arg(long)]
		config: PathBuf,
		/// Output directory for generated artifacts.
		#[arg(long)]
		out: PathBuf,
		/// Print the planned commands without running external tools.
		#[arg(long, default_value_t = false)]
		dry_run: bool,
		/// Skip `OpenTofu` init/apply (e.g. for existing bare-metal hosts).
		#[arg(long, default_value_t = false)]
		skip_tofu: bool,
		/// Skip SSH node bootstrap.
		#[arg(long, default_value_t = false)]
		skip_bootstrap: bool,
		/// Skip kubectl manifest apply & wait phases.
		#[arg(long, default_value_t = false)]
		skip_apply: bool,
		/// Optional kubeconfig path passed to kubectl.
		#[arg(long)]
		kubeconfig: Option<PathBuf>,
		/// SSH user for node bootstrap (default `root`).
		#[arg(long, default_value = "root")]
		ssh_user: String,
	},
}

impl Cli {
	/// Execute a local command, returning output suitable for stdout.
	pub fn run(&self) -> Result<String, PlatformError> {
		let (Operation::Generate { config, .. }
		| Operation::Bootstrap { config, .. }
		| Operation::Check { config }
		| Operation::PlanSummary { config }
		| Operation::Up { config, .. }) = &self.command;
		let config = PlatformConfig::parse(&std::fs::read_to_string(config)?)?;
		match &self.command {
			Operation::Check { .. } => Ok("configuration valid\n".into()),
			Operation::PlanSummary { .. } => summary(&config),
			Operation::Generate { out, .. } => {
				let mut files = crate::tofu::generate(&config)?;
				files.extend(crate::addons::generate(&config)?);
				write_files(out, "generate", &files)
			}
			Operation::Bootstrap { outputs, out, .. } => {
				let outputs = outputs.as_ref().map(std::fs::read_to_string).transpose()?;
				write_files(
					out,
					"bootstrap",
					&crate::bootstrap::generate(&config, outputs.as_deref())?,
				)
			}
			Operation::Up {
				out,
				dry_run,
				skip_tofu,
				skip_bootstrap,
				skip_apply,
				kubeconfig,
				ssh_user,
				..
			} => {
				let options = crate::up::UpOptions {
					out: out.clone(),
					dry_run: *dry_run,
					skip_tofu: *skip_tofu,
					skip_bootstrap: *skip_bootstrap,
					skip_apply: *skip_apply,
					kubeconfig: kubeconfig.clone(),
					ssh_user: ssh_user.clone(),
				};
				if *dry_run {
					let mut runner = crate::up::DryRunRunner::default();
					let mut transcript = crate::up::execute(&config, &options, &mut runner)?;
					for (program, args, cwd) in runner.commands {
						match cwd {
							Some(dir) => {
								writeln!(
									transcript,
									"[dry-run] (cd {}) {program} {}",
									dir.display(),
									args.join(" ")
								)?;
							}
							None => {
								writeln!(transcript, "[dry-run] {program} {}", args.join(" "))?;
							}
						}
					}
					Ok(transcript)
				} else {
					let mut runner = crate::up::SystemRunner;
					crate::up::execute(&config, &options, &mut runner)
				}
			}
		}
	}
}

fn summary(config: &PlatformConfig) -> Result<String, PlatformError> {
	let mut output = String::from("REGION/CLUSTER  NODE  ROLE  POOL  OWNERSHIP\n");
	for (name, version) in [
		("cloudnative-pg", &config.addons.cloudnative_pg),
		("keda", &config.addons.keda),
		("keda-add-ons-http", &config.addons.keda_http),
		("cert-manager", &config.addons.cert_manager),
		("traefik (edge cluster)", &config.addons.traefik),
	] {
		writeln!(output, "Add-on {name}: {version}")?;
	}
	if matches!(config.registry, crate::config::Registry::Zot { .. }) {
		writeln!(output, "Add-on zot (edge cluster): {}", config.addons.zot)?;
	}
	writeln!(
		output,
		"Connector cloudflared: 2026.11.0 (tunnel mode only)"
	)?;
	for (region, entry) in &config.regions {
		for (name, cluster) in &entry.clusters {
			let machines = crate::topology::machines(cluster, None)?;
			for machine in &machines {
				writeln!(
					output,
					"{region}/{name}  {}  {:?}  {:?}  {}",
					machine.id,
					machine.node.role(),
					machine.node.pool(),
					if machine.node.cloud() {
						"create VM + provider firewall"
					} else {
						"assume existing host + install host firewall"
					}
				)?;
			}
			if machines
				.iter()
				.filter(|m| m.node.role() == Role::Server)
				.count()
				== 1
			{
				writeln!(
					output,
					"{region}/{name}: single server is a single point of failure"
				)?;
			}
			writeln!(
				output,
				"{region}/{name}: registration uses {}; {}",
				cluster.registration_address.as_deref().unwrap_or(
					"first server public address (deterministic; no load balancer provisioned)"
				),
				if cluster
					.nodes
					.values()
					.any(|n| n.cloud() && n.role() == Role::Server)
				{
					"create one shared private network; server joins use private addresses"
				} else {
					"private links between existing servers are supplied by the operator; none are \
					 created"
				}
			)?;
		}
	}
	writeln!(
		output,
		"Edge DNS and Cloudflare resources are created only for the default-region edge cluster. \
		 Object storage and external registry are assumed to exist. No infrastructure has been \
		 provisioned."
	)?;
	writeln!(
		output,
		"Pools do not imply redundancy: one database instance, local-path storage, one ingress \
		 node or one build node remain single points of failure where configured."
	)?;
	Ok(output)
}

type Ownership = BTreeMap<String, Vec<String>>;

pub(crate) fn write_generated_files(
	root: &Path,
	phase: &str,
	files: &Generated,
) -> Result<String, PlatformError> {
	write_files(root, phase, files)
}

fn write_files(root: &Path, phase: &str, files: &Generated) -> Result<String, PlatformError> {
	std::fs::create_dir_all(root)?;
	require(
		!std::fs::symlink_metadata(root)?.file_type().is_symlink(),
		"output directory must not be a symlink",
	)?;
	let lock = root.join(".platform.lock");
	let guard = std::fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(&lock)?;
	let result = write_owned(root, phase, files);
	drop(guard);
	std::fs::remove_file(lock)?;
	result
}

fn checked_path(root: &Path, relative: &str) -> Result<PathBuf, PlatformError> {
	let mut path = root.to_owned();
	for component in Path::new(relative).components() {
		let Component::Normal(component) = component else {
			return Err(PlatformError::Invalid(
				"generated path must be relative and contained".into(),
			));
		};
		path.push(component);
		if let Ok(metadata) = std::fs::symlink_metadata(&path) {
			require(
				!metadata.file_type().is_symlink(),
				"generated paths must not traverse symlinks",
			)?;
		}
	}
	Ok(path)
}

fn write_owned(root: &Path, phase: &str, files: &Generated) -> Result<String, PlatformError> {
	let manifest = checked_path(root, &format!(".platform-{phase}.json"))?;
	let previous: Ownership = if manifest.exists() {
		serde_json::from_slice(&std::fs::read(&manifest)?)?
	} else {
		Ownership::new()
	};
	let mut journal = previous.clone();
	let mut changes = Vec::new();
	// Preflight every file before mutating anything. A matching foreign file
	// still belongs to its owner and must never be adopted automatically.
	for (name, content) in files {
		let path = checked_path(root, name)?;
		let current = if path.exists() {
			Some(std::fs::read_to_string(path)?)
		} else {
			None
		};
		if let Some(current) = &current {
			require(
				previous
					.get(name)
					.is_some_and(|versions| versions.contains(current)),
				"refusing to overwrite a foreign or locally modified file",
			)?;
		}
		if current.as_ref() != Some(content) {
			changes.push(name.clone());
		}
		let versions = journal.entry(name.clone()).or_default();
		if !versions.contains(content) {
			versions.push(content.clone());
		}
	}
	// Journal both old and intended contents before writing. After interruption
	// a retry recognizes either state instead of treating its own files as
	// foreign.
	atomic_write(&manifest, &serde_json::to_vec_pretty(&journal)?)?;
	for name in &changes {
		let path = checked_path(root, name)?;
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}
		atomic_write(&path, files[name].as_bytes())?;
	}
	let final_manifest: Ownership = journal
		.into_iter()
		.map(|(name, versions)| {
			let versions = files
				.get(&name)
				.map_or(versions, |content| vec![content.clone()]);
			(name, versions)
		})
		.collect();
	atomic_write(&manifest, &serde_json::to_vec_pretty(&final_manifest)?)?;
	Ok(if changes.is_empty() {
		"unchanged\n".into()
	} else {
		format!("wrote {}\n", changes.join("\nwrote "))
	})
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), PlatformError> {
	let temporary = path.with_extension("platform-tmp");
	// The per-root lock serializes writers; create_new refuses a stale
	// temporary or foreign file instead of following it or silently
	// overwriting it.
	let mut file = std::fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(&temporary)?;
	file.write_all(content)?;
	file.sync_all()?;
	drop(file);
	std::fs::rename(temporary, path)?;
	Ok(())
}
