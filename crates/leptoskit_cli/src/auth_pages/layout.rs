//! Cargo owns package and target discovery for auth generation.

use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;

#[derive(Debug)]
pub(super) struct Package {
	pub manifest: PathBuf,
	pub entry: PathBuf,
}

#[derive(Debug)]
pub(super) struct Layout {
	pub client: Package,
	pub server: Option<Package>,
}

#[derive(Deserialize)]
struct Metadata {
	workspace_root: PathBuf,
	packages: Vec<Member>,
}

#[derive(Deserialize)]
struct Member {
	name: String,
	manifest_path: PathBuf,
	targets: Vec<Target>,
	dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Dependency {
	name: String,
	rename: Option<String>,
	features: Vec<String>,
}

#[derive(Deserialize)]
struct Target {
	name: String,
	kind: Vec<String>,
	src_path: PathBuf,
}

impl Layout {
	pub(super) fn read(
		root: &Path,
		app: &str,
		client: Option<&str>,
		backend: Option<bool>,
	) -> Result<Self> {
		anyhow::ensure!(
			root.ancestors().any(|dir| dir.join("Cargo.toml").is_file()),
			"there's no package around {}; add a Cargo.toml first",
			root.display()
		);
		let output = std::process::Command::new("cargo")
			.args(["metadata", "--no-deps", "--format-version", "1"])
			.current_dir(root)
			.output()
			.context("could not run cargo metadata for auth generation")?;
		anyhow::ensure!(
			output.status.success(),
			"could not resolve auth packages: {}",
			String::from_utf8_lossy(&output.stderr)
		);
		let metadata: Metadata = serde_json::from_slice(&output.stdout)
			.context("could not parse auth Cargo metadata")?;
		Self::resolve(root, app, client, backend, &metadata)
	}

	fn resolve(
		root: &Path,
		app: &str,
		client: Option<&str>,
		backend: Option<bool>,
		metadata: &Metadata,
	) -> Result<Self> {
		let workspace = metadata.workspace_root.canonicalize()?;
		anyhow::ensure!(
			root.canonicalize()?.starts_with(&workspace),
			"the app is outside its Cargo workspace"
		);
		let selected = metadata.client(app, client)?;
		selected.leptoskit()?;
		let target = selected
			.targets
			.iter()
			.find(|target| target.library())
			.map_or_else(|| selected.binary(), Ok)?;
		let client = Package::checked(selected, target, &workspace)?;
		let server = metadata.server(app, backend, &workspace)?;
		Ok(Self { client, server })
	}
}

impl Metadata {
	fn client(&self, app: &str, client: Option<&str>) -> Result<&Member> {
		let selected = if let Some(client) = client {
			self.packages
				.iter()
				.find(|package| package.name == client)
				.with_context(|| {
					format!("couldn't find the client package `{client}` in the Cargo workspace")
				})?
		} else if let Some(package) = self.packages.iter().find(|package| {
			package.name == app
				&& package
					.targets
					.iter()
					.any(|target| target.kind.iter().any(|kind| kind == "cdylib"))
		}) {
			package
		} else {
			let candidates: Vec<_> = self
				.packages
				.iter()
				.filter(|package| {
					package
						.targets
						.iter()
						.any(|target| target.kind.iter().any(|kind| kind == "cdylib"))
				})
				.collect();
			match candidates.as_slice() {
				[package] => package,
				[] => {
					self.packages
						.iter()
						.find(|package| package.name == app)
						.context("set build.client_package to the app's Cargo package")?
				}
				_ => {
					anyhow::bail!(
						"several packages have client libraries; set build.client_package before \
						 generating auth"
					)
				}
			}
		};
		Ok(selected)
	}

	fn server(
		&self,
		app: &str,
		backend: Option<bool>,
		workspace: &Path,
	) -> Result<Option<Package>> {
		let server = if backend == Some(false) {
			None
		} else if let Some(member) = self.packages.iter().find(|package| package.name == app) {
			let enabled = member.dependencies.iter().any(|dependency| {
				dependency.name == "leptoskit"
					&& dependency
						.features
						.iter()
						.any(|feature| feature == "server")
			});
			if backend == Some(true)
				|| (enabled
					&& member
						.targets
						.iter()
						.any(|target| target.kind.iter().any(|kind| kind == "bin")))
			{
				member.leptoskit()?;
				Some(Package::checked(member, member.binary()?, workspace)?)
			} else {
				None
			}
		} else {
			anyhow::ensure!(
				backend != Some(true),
				"app.name `{app}` does not name a backend package"
			);
			None
		};
		if let Some(server) = &server {
			anyhow::ensure!(
				server
					.entry
					.file_name()
					.is_none_or(|name| name != "sign_in.rs"),
				"the backend target is named sign_in.rs, which conflicts with the generated \
				 module; rename the target source before generating auth"
			);
		}
		Ok(server)
	}
}

impl Target {
	fn library(&self) -> bool {
		self.kind.iter().any(|kind| {
			matches!(
				kind.as_str(),
				"lib" | "rlib" | "cdylib" | "staticlib" | "dylib"
			)
		})
	}
}

impl Member {
	fn leptoskit(&self) -> Result<()> {
		let dependency = self
			.dependencies
			.iter()
			.find(|dependency| dependency.name == "leptoskit")
			.with_context(|| {
				format!(
					"{} doesn't depend on leptoskit",
					self.manifest_path.display()
				)
			})?;
		anyhow::ensure!(
			dependency
				.rename
				.as_deref()
				.is_none_or(|name| name == "leptoskit"),
			"auth templates use `leptoskit`; keep that dependency name in {}",
			self.manifest_path.display()
		);
		Ok(())
	}

	fn binary(&self) -> Result<&Target> {
		let targets: Vec<_> = self
			.targets
			.iter()
			.filter(|target| target.kind.iter().any(|kind| kind == "bin"))
			.collect();
		if let Some(target) = targets.iter().find(|target| target.name == self.name) {
			return Ok(target);
		}
		match targets.as_slice() {
			[target] => Ok(target),
			[] => {
				anyhow::bail!("package `{}` has no binary source target", self.name)
			}
			_ => {
				anyhow::bail!(
					"package `{}` has several binaries; name the app binary `{}`",
					self.name,
					self.name
				)
			}
		}
	}
}

impl Package {
	fn checked(member: &Member, target: &Target, workspace: &Path) -> Result<Self> {
		let manifest = member.manifest_path.canonicalize()?;
		let entry = target.src_path.canonicalize().with_context(|| {
			format!("could not read target source {}", target.src_path.display())
		})?;
		anyhow::ensure!(
			manifest.starts_with(workspace) && entry.starts_with(workspace),
			"package `{}` or its source target resolves outside the Cargo workspace",
			member.name
		);
		anyhow::ensure!(entry.is_file(), "{} is not a source file", entry.display());
		Ok(Self { manifest, entry })
	}
}

/// Both paths have already been resolved inside one workspace.
pub(super) fn relative(from: &Path, to: &Path) -> PathBuf {
	let common = from
		.ancestors()
		.find(|ancestor| to.starts_with(ancestor))
		.expect("absolute paths share a root");
	let mut path = PathBuf::new();
	for _ in from.strip_prefix(common).expect("ancestor").components() {
		path.push("..");
	}
	path.push(to.strip_prefix(common).expect("shared ancestor"));
	if path.as_os_str().is_empty() {
		path.push(".");
	}
	path
}

#[cfg(test)]
mod tests;
