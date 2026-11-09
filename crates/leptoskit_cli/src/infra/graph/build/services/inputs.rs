use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use sha2::Digest as _;

use crate::services::Code;
use crate::services::Service;
use crate::verify::assets::Member;
use crate::verify::assets::Root;
use crate::verify::assets::Tree;
use crate::verify::graph::service::ServiceEvidence;
use crate::verify::graph::service::manifest;
use crate::verify::graph::service::portable;

pub(in crate::infra::graph::build) struct Inputs {
	root: Root,
	files: BTreeMap<String, (String, Vec<u8>)>,
	evidence: ServiceEvidence,
}

impl Inputs {
	pub(in crate::infra::graph::build) fn prepare(
		root: &Path,
		metadata: &crate::wasm::CargoMetadata,
		service: &Service,
		release: bool,
	) -> Result<Self> {
		let (package, source, outside) = match &service.code {
			Code::Workspace { package, manifest } => (package, manifest, None),
			Code::External { package, manifest } => {
				(
					package,
					manifest,
					Some(crate::wasm::cargo_metadata(
						manifest.parent().context("manifest parent missing")?,
					)?),
				)
			}
			Code::Remote => anyhow::bail!("remote services have no executable evidence"),
		};
		let metadata = outside.as_ref().unwrap_or(metadata);
		let root_path = root.canonicalize()?;
		crate::verify::graph::service::association::source(
			&service.settings.source,
			&root_path,
			source,
			package,
		)?;
		let settings = portable(service.settings.clone())?;
		let opened = Root::open(&root_path)?;
		ensure!(metadata.packages.len() <= 255, "too many service packages");
		let mut package_names = std::collections::BTreeSet::new();
		for package in &metadata.packages {
			ensure!(
				package_names.insert(&package.name),
				"duplicate conflicting package names in service metadata"
			);
		}
		let mut paths = std::collections::BTreeSet::new();
		paths.insert(metadata.workspace_root.join("Cargo.toml"));
		for package in &metadata.packages {
			paths.insert(package.manifest_path.clone());
		}
		ensure!(paths.len() <= 255, "too many service manifests");
		let mut files = BTreeMap::new();
		let mut names = BTreeMap::new();
		let mut total = 0usize;
		for (index, path) in paths.iter().enumerate() {
			let relative = path
				.strip_prefix(&root_path)
				.context("service source outside project: exact portable capture unsupported")?
				.to_str()
				.context("non UTF-8 input")?;
			let bytes = opened.metadata(relative)?;
			let parsed = manifest(&bytes)?;
			check_paths(
				&parsed,
				path.parent().context("input parent missing")?,
				&paths,
			)?;
			total += bytes.len();
			ensure!(
				total <= 16 * 1024 * 1024,
				"service input byte budget exceeded"
			);
			let name = format!("manifest-{index}.toml");
			names.insert(path.clone(), name.clone());
			files.insert(name, (relative.to_owned(), bytes));
		}
		let lock_path = metadata.workspace_root.join("Cargo.lock");
		let relative = lock_path
			.strip_prefix(&root_path)?
			.to_str()
			.context("non UTF-8 lock")?;
		let lock = opened
			.metadata(relative)
			.context("service capture requires an existing exact Cargo.lock")?;
		manifest(&lock)?;
		ensure!(
			total + lock.len() <= 16 * 1024 * 1024,
			"service input byte budget exceeded"
		);
		files.insert("Cargo.lock".into(), (relative.into(), lock));
		let inputs: Tree = files
			.iter()
			.map(|(name, (_, bytes))| {
				(
					name.clone(),
					Member::File {
						mode: 0o644,
						size: bytes.len() as u64,
						sha256: hex::encode(sha2::Sha256::digest(bytes)),
					},
				)
			})
			.collect();
		let package_manifest = names
			.get(source)
			.context("selected manifest absent from metadata")?;
		let workspace_manifest = names
			.get(&metadata.workspace_root.join("Cargo.toml"))
			.context("workspace manifest absent from metadata")?;
		let evidence = ServiceEvidence {
			version: crate::verify::graph::service::version(
				&manifest(&files[package_manifest].1)?,
				&manifest(&files[workspace_manifest].1)?,
			)?,
			settings,
			package: package.clone(),
			binary: String::new(),
			profile: if release { "release" } else { "dev" }.into(),
			default_features: true,
			package_manifest: package_manifest.clone(),
			workspace_manifest: workspace_manifest.clone(),
			lock: "Cargo.lock".into(),
			inputs,
			sources: files
				.iter()
				.map(|(name, (path, _))| (name.clone(), path.clone()))
				.collect(),
		};
		Ok(Self {
			root: opened,
			files,
			evidence,
		})
	}

	pub(in crate::infra::graph::build) fn recheck(&self) -> Result<()> {
		for (relative, bytes) in self.files.values() {
			ensure!(
				self.root.metadata(relative)? == *bytes,
				"service build inputs changed during capture"
			);
		}
		Ok(())
	}

	pub(in crate::infra::graph::build) fn capture(
		&self,
		destination: &Path,
		binary: &Path,
	) -> Result<ServiceEvidence> {
		self.recheck()?;
		std::fs::create_dir(destination)?;
		for (name, (_, bytes)) in &self.files {
			std::fs::write(destination.join(name), bytes)?;
			#[cfg(unix)]
			{
				use std::os::unix::fs::PermissionsExt as _;
				std::fs::set_permissions(
					destination.join(name),
					std::fs::Permissions::from_mode(0o644),
				)?;
			}
		}
		ensure!(
			Root::open(destination)?.tree(&[])? == self.evidence.inputs,
			"copied service inputs differ"
		);
		self.recheck()?;
		let mut evidence = self.evidence.clone();
		binary
			.file_name()
			.and_then(|name| name.to_str())
			.context("service binary name is not UTF-8")?
			.clone_into(&mut evidence.binary);
		Ok(evidence)
	}
}

// --no-deps metadata does not establish manifests of nonmember path
// dependencies. Refuse those rather than claim an incomplete manifest
// inventory.
fn check_paths(
	value: &toml::Value,
	parent: &Path,
	known: &std::collections::BTreeSet<std::path::PathBuf>,
) -> Result<()> {
	if let Some(table) = value.as_table() {
		if let Some(path) = table.get("path").and_then(toml::Value::as_str) {
			let candidate = parent.join(path).join("Cargo.toml");
			if candidate.exists() {
				ensure!(
					known.contains(&candidate.canonicalize()?),
					"nonmember path dependency capture unsupported"
				);
			}
		}
		for value in table.values() {
			check_paths(value, parent, known)?;
		}
	}
	if let Some(values) = value.as_array() {
		for value in values {
			check_paths(value, parent, known)?;
		}
	}
	Ok(())
}
