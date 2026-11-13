//! A pinned graph receipt is evidence about bytes, not build reproduction,
//! approval, executable target compatibility or a running deployment.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::graph::plan::Plan;
use leptoskit_config::graph::plan::Selection;
use sha2::Digest as _;

use super::assets::Root;

pub(crate) mod linux;
mod manifest;
pub(crate) mod service;
#[cfg(test)]
mod tests;

/// Check a local graph build receipt without running its recipes or binaries.
#[derive(Debug, clap::Args)]
pub struct Args {
	/// Directory produced by `lk build` in a graph project.
	#[arg(long)]
	pub bundle: PathBuf,
	/// SHA-256 of manifest.json, obtained independently of this directory.
	#[arg(long)]
	pub sha256: String,
	/// Unit to stage, such as frontends.docs; repeat for several units.
	/// Omit to select every unit recorded in the receipt.
	#[arg(long)]
	pub unit: Vec<String>,
	/// Existing parent directory for a new private staging child. A failed
	/// check publishes no staging child. Omit to check without copying.
	#[arg(long)]
	pub out: Option<PathBuf>,
	/// Inspect selected executables without running them. Does not check host
	/// libraries.
	#[arg(long, value_enum)]
	pub linux_architecture: Option<linux::Architecture>,
}

pub(super) fn run(args: &Args) -> Result<()> {
	let (config, project) = LeptoskitConfig::load()?;
	let checked = check(&project, &config, &args.bundle, &args.sha256)?;
	let selection = checked.selection(&args.unit)?;
	let formats = args
		.linux_architecture
		.map(|architecture| checked.inspect_linux(&selection, architecture))
		.transpose()?;
	let staged = args
		.out
		.as_ref()
		.map(|out| checked.stage(out, &selection))
		.transpose()?;
	println!(
		"{}",
		serde_json::to_string_pretty(&serde_json::json!({
			"format_version":1, "kind":"leptoskit-graph-artifact-check",
			"manifest_sha256":args.sha256,"configuration_sha256":checked.manifest.configuration_sha256,
			"profile":checked.manifest.service_profile,"selected":names(&selection),
			"staged":staged,"linux_formats":formats,"evidence":evidence()
		}))?
	);
	Ok(())
}

/// The constructor is private to exact pin, declaration and artifact checks.
/// Holding this value never skips revalidation when files are consumed.
pub(crate) struct Checked {
	root: Root,
	project: Root,
	project_path: PathBuf,
	config: LeptoskitConfig,
	path: PathBuf,
	bytes: Vec<u8>,
	manifest: manifest::Manifest,
	plan: Plan,
}

pub(crate) fn check(
	project: &Path,
	config: &LeptoskitConfig,
	bundle: &Path,
	pin: &str,
) -> Result<Checked> {
	ensure!(
		super::digest_is_valid(pin),
		"graph receipt pin must be a lowercase SHA-256"
	);
	config.validate()?;
	let plan = leptoskit_config::deployment::compile(config)
		.map_err(anyhow::Error::msg)?
		.context("graph verification requires infra.mode=graph")?;
	let root = Root::open(bundle)?;
	let bytes = root.metadata("manifest.json")?;
	ensure!(
		hash(&bytes) == pin,
		"graph receipt differs from independently obtained digest"
	);
	let manifest = manifest::parse(&bytes)?;
	let result = Checked {
		root,
		project: Root::open(project)?,
		project_path: project.canonicalize()?,
		config: config.clone(),
		path: bundle.canonicalize()?,
		bytes,
		manifest,
		plan,
	};
	result.recheck()?;
	Ok(result)
}

impl Checked {
	pub(crate) fn selection(&self, requested: &[String]) -> Result<Selection> {
		let units = if requested.is_empty() {
			self.manifest.units.keys().cloned().collect()
		} else {
			requested.to_vec()
		};
		ensure!(
			units
				.iter()
				.all(|unit| self.manifest.units.contains_key(unit)),
			"selected unit is absent from this build receipt"
		);
		let units = units
			.iter()
			.map(|unit| manifest::unit(unit))
			.collect::<Result<Vec<_>>>()?;
		self.plan.select(&units).map_err(anyhow::Error::msg)
	}

	fn inspect_linux(
		&self,
		selection: &Selection,
		architecture: linux::Architecture,
	) -> Result<std::collections::BTreeMap<String, linux::Format>> {
		self.recheck()?;
		let mut formats = std::collections::BTreeMap::new();
		for name in names(selection) {
			if self.manifest.units[&name].backend.is_some() {
				let root = self.root.beneath(&name)?;
				formats.insert(
					name,
					linux::inspect(root.open_executable("backend")?, architecture)?,
				);
			}
		}
		self.recheck()?;
		Ok(formats)
	}

	pub(crate) fn recheck(&self) -> Result<()> {
		self.manifest
			.validate(&self.project, &self.config, &self.plan)?;
		ensure!(
			self.root.metadata("manifest.json")? == self.bytes,
			"graph manifest changed after verification"
		);
		let pin = self.root.read_file("manifest.sha256", 65)?;
		ensure!(
			pin == format!("{}\n", hash(&self.bytes)).as_bytes(),
			"graph receipt's convenience checksum differs; it is not an independent pin"
		);
		let mut expected: BTreeSet<_> = self.manifest.units.keys().cloned().collect();
		expected.extend(["manifest.json".into(), "manifest.sha256".into()]);
		ensure!(
			self.root.names()?.into_iter().collect::<BTreeSet<_>>() == expected,
			"graph receipt root membership differs"
		);
		for (name, captured) in &self.manifest.units {
			captured.check(&self.root.beneath(name)?)?;
			if let Some(service) = &captured.service {
				let name_only = name
					.strip_prefix("services.")
					.context("service evidence on frontend")?;
				service.validate(
					&self.root.beneath(name)?.beneath("service")?,
					&self.project,
					&self.config,
					name_only,
					&self.manifest.service_profile,
					&self.project_path,
				)?;
			}
		}
		Ok(())
	}

	fn stage(&self, parent: &Path, selection: &Selection) -> Result<PathBuf> {
		let parent = parent.canonicalize().context("staging parent must exist")?;
		ensure!(
			!parent.starts_with(&self.path),
			"staging output must be outside the input receipt"
		);
		self.recheck()?;
		let mut builder = tempfile::Builder::new();
		builder.prefix("graph-");
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			builder.permissions(std::fs::Permissions::from_mode(0o700));
		}
		let temporary = builder.tempdir_in(&parent)?;
		for name in names(selection) {
			let captured = &self.manifest.units[&name];
			let source = self.root.beneath(&name)?;
			let destination = temporary.path().join(&name);
			std::fs::create_dir(&destination)?;
			if let Some(tree) = &captured.assets {
				source
					.beneath("assets")?
					.copy(&destination.join("assets"), &[], tree)?;
			}
			if let Some(expected) = &captured.backend {
				ensure!(
					source.copy_executable("backend", &destination.join("backend"))? == *expected,
					"graph executable changed while staging"
				);
			}
			if let Some(service) = &captured.service {
				source.beneath("service")?.copy(
					&destination.join("service"),
					&[],
					&service.inputs,
				)?;
			}
			captured.check(&Root::open(&destination)?)?;
		}
		let document = serde_json::to_vec_pretty(&serde_json::json!({
			"format_version":1,"kind":"leptoskit-graph-staged-artifacts",
			"source_manifest_sha256":hash(&self.bytes),"configuration_sha256":self.manifest.configuration_sha256,
			"units":names(selection),
			"readiness_only":selection.readiness.iter().map(ToString::to_string).collect::<Vec<_>>(),
			"evidence":evidence()
		}))?;
		for (name, bytes) in [
			("source-manifest.json", self.bytes.as_slice()),
			("stage.json", document.as_slice()),
			("stage.sha256", format!("{}\n", hash(&document)).as_bytes()),
		] {
			let mut file = std::fs::OpenOptions::new()
				.create_new(true)
				.write(true)
				.open(temporary.path().join(name))?;
			file.write_all(bytes)?;
			file.sync_all()?;
		}
		self.recheck()?;
		#[cfg(unix)]
		std::fs::File::open(temporary.path())?.sync_all()?;
		Ok(temporary.keep())
	}
}

/// A release consumer retains this handle through installation. It cannot be
/// reconstructed from stage.json. Source and copied bytes are rechecked at
/// each consumption boundary; the consumer still owns remote verification.
pub(crate) struct Staged<'a> {
	checked: &'a Checked,
	selection: Selection,
	path: PathBuf,
	root: Root,
	metadata: Vec<u8>,
}

impl Checked {
	pub(crate) fn stage_release(&self, parent: &Path, requested: &[String]) -> Result<Staged<'_>> {
		let selection = self.selection(requested)?;
		let path = self.stage(parent, &selection)?;
		let root = Root::open(&path)?;
		let metadata = root.metadata("stage.json")?;
		let result = Staged {
			checked: self,
			selection,
			path,
			root,
			metadata,
		};
		result.recheck()?;
		Ok(result)
	}
}

impl Staged<'_> {
	pub(crate) fn selected(&self) -> Vec<String> {
		names(&self.selection)
	}

	pub(crate) fn readiness_only(&self) -> Vec<String> {
		self.selection
			.readiness
			.iter()
			.map(ToString::to_string)
			.collect()
	}

	pub(crate) fn recheck(&self) -> Result<()> {
		self.checked.recheck()?;
		let mut expected: BTreeSet<_> = self.selected().into_iter().collect();
		expected.extend([
			"source-manifest.json".into(),
			"stage.json".into(),
			"stage.sha256".into(),
		]);
		ensure!(
			self.root
				.names()
				.with_context(|| format!("private stage {} unavailable", self.path.display()))?
				.into_iter()
				.collect::<BTreeSet<_>>()
				== expected,
			"staged release membership changed"
		);
		ensure!(
			self.root.metadata("source-manifest.json")? == self.checked.bytes,
			"staged source manifest changed"
		);
		ensure!(
			self.root.metadata("stage.json")? == self.metadata
				&& self.root.read_file("stage.sha256", 65)?
					== format!("{}\n", hash(&self.metadata)).as_bytes(),
			"staged metadata changed"
		);
		for name in self.selected() {
			self.checked.manifest.units[&name].check(&self.root.beneath(&name)?)?;
		}
		Ok(())
	}

	/// The callback must finish copying before returning. A successful result
	/// is withheld if any checked input changed while it was being consumed.
	/// Callers must verify the destination before committing an activation.
	pub(crate) fn consume<T>(&self, action: impl FnOnce(&Root) -> Result<T>) -> Result<T> {
		self.recheck()?;
		let result = action(&self.root)?;
		self.recheck()?;
		Ok(result)
	}
}

fn names(selection: &Selection) -> Vec<String> {
	selection
		.stages
		.iter()
		.flatten()
		.map(ToString::to_string)
		.collect()
}

fn evidence() -> serde_json::Value {
	serde_json::json!({"artifact_integrity":"checked","source_reproduction":"not_checked",
		"isolated_build":"not_checked","origin_authorization":"not_checked",
		"release_approval":"not_checked","executable_target":"not_checked","runtime":"not_checked"})
}

fn hash(bytes: &[u8]) -> String {
	hex::encode(sha2::Sha256::digest(bytes))
}

impl Checked {
	pub(crate) fn release_inventory(&self, requested: &[String]) -> Result<serde_json::Value> {
		self.recheck()?;
		let selection = self.selection(requested)?;
		ensure!(
			selection.readiness.is_empty(),
			"readiness-only dependencies require a fresh runtime observer; graph releases cannot \
			 establish their readiness yet"
		);
		let mut result = serde_json::Map::new();
		for name in names(&selection) {
			let captured = &self.manifest.units[&name];
			let assets = if let Some(tree) = &captured.assets {
				serde_json::to_value(tree)?
			} else {
				ensure!(captured.backend.is_some(), "unit has no release artifacts");
				serde_json::json!({})
			};
			result.insert(name, assets);
		}
		self.recheck()?;
		Ok(result.into())
	}

	/// Inspect exactly one already selected backend. Never execute it.
	pub(crate) fn executable_format(
		&self,
		unit: &str,
		architecture: linux::Architecture,
	) -> Result<linux::Format> {
		self.recheck()?;
		ensure!(
			self.manifest
				.units
				.get(unit)
				.is_some_and(|value| value.backend.is_some()),
			"selected backend missing from checked receipt"
		);
		let result = linux::inspect(
			self.root.beneath(unit)?.open_executable("backend")?,
			architecture,
		)?;
		self.recheck()?;
		Ok(result)
	}
}

/// Inventory is descriptive evidence only, never executable promotion
/// authority.
#[derive(Debug, Clone)]
pub(crate) struct Executable {
	pub(crate) unit: String,
	pub(crate) backend: super::assets::Member,
	/// SSR has no service resolver metadata.
	pub(crate) service: Option<service::ServiceEvidence>,
}

impl Checked {
	pub(crate) fn executable_inventory(&self, requested: &[String]) -> Result<Vec<Executable>> {
		self.recheck()?;
		let selection = self.selection(requested)?;
		let mut result = Vec::new();
		for unit in names(&selection) {
			let captured = &self.manifest.units[&unit];
			if let Some(backend) = &captured.backend {
				result.push(Executable {
					unit,
					backend: backend.clone(),
					service: captured.service.clone(),
				});
			}
		}
		self.recheck()?;
		Ok(result)
	}
}
