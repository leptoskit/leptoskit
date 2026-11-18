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
	/// Generate phase-one `OpenTofu` files.
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
}

impl Cli {
	/// Execute a local command, returning output suitable for stdout.
	pub fn run(&self) -> Result<String, PlatformError> {
		let (Operation::Generate { config, .. }
		| Operation::Bootstrap { config, .. }
		| Operation::Check { config }
		| Operation::PlanSummary { config }) = &self.command;
		let config = PlatformConfig::parse(&std::fs::read_to_string(config)?)?;
		match &self.command {
			Operation::Check { .. } => Ok("configuration valid\n".into()),
			Operation::PlanSummary { .. } => summary(&config),
			Operation::Generate { out, .. } => {
				write_files(out, "generate", &crate::tofu::generate(&config)?)
			}
			Operation::Bootstrap { outputs, out, .. } => {
				let outputs = outputs.as_ref().map(std::fs::read_to_string).transpose()?;
				write_files(
					out,
					"bootstrap",
					&crate::bootstrap::generate(&config, outputs.as_deref())?,
				)
			}
		}
	}
}

fn summary(config: &PlatformConfig) -> Result<String, PlatformError> {
	let mut output = String::from("REGION/CLUSTER  NODE  ROLE  POOL  OWNERSHIP\n");
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
