use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::deployment;
use leptoskit_config::deployment::FrontendKind;
use leptoskit_config::graph::Unit;
use leptoskit_config::graph::plan::Plan;
use leptoskit_config::graph::plan::Selection;
use serde::Serialize;
use sha2::Digest as _;

use crate::verify::assets::Member;
use crate::verify::assets::Root;
use crate::verify::assets::Tree;

#[cfg(test)]
mod tests;

pub(super) struct Selected {
	pub(super) plan: Plan,
	pub(super) selection: Selection,
}

pub(super) fn selection(config: &LeptoskitConfig, names: &[String]) -> Result<Selected> {
	config.validate()?;
	let plan = deployment::compile(config)
		.map_err(anyhow::Error::msg)?
		.context("graph build requires infra.mode=graph")?;

	for (name, frontend) in &config
		.graph_infra()
		.context("graph infrastructure required")?
		.frontends
	{
		if let Some(path) = &frontend.backend {
			let path = path.to_str().context("backend path is not UTF-8")?;
			let owner = plan
				.trees()
				.owner(&frontend.artifact, path)
				.map_err(anyhow::Error::msg)?;
			ensure!(
				owner.is_none_or(|owner| owner.unit == Unit::Frontend(name.clone())),
				"frontend backend lies in another frontend's public tree"
			);
		}
	}
	let units = if names.is_empty() {
		plan.placements().keys().cloned().collect()
	} else {
		names
			.iter()
			.map(|name| {
				match name.split_once('.') {
					Some(("frontends", name)) => Ok(Unit::Frontend(name.into())),
					Some(("services", name)) => Ok(Unit::Service(name.into())),
					_ => anyhow::bail!("unit must be frontends.<name> or services.<name>"),
				}
			})
			.collect::<Result<Vec<_>>>()?
	};

	let selection = plan.select(&units).map_err(anyhow::Error::msg)?;
	ensure!(
		!selection.stages.is_empty(),
		"graph has no selected build units"
	);
	Ok(Selected { plan, selection })
}

#[derive(Serialize)]
struct Captured {
	#[serde(skip_serializing_if = "Option::is_none")]
	service: Option<crate::verify::graph::service::ServiceEvidence>,
	host: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	assets: Option<Tree>,
	#[serde(skip_serializing_if = "Option::is_none")]
	backend: Option<Member>,
	#[serde(skip_serializing_if = "Option::is_none")]
	toolchain_image: Option<String>,
}

/// Commands are trusted repository recipes. Capture happens only after every
/// selected command succeeds; temporary receipts disappear on failure.
pub(super) fn execute(
	root: &Path,
	config: &LeptoskitConfig,
	selected: &Selected,
	release: bool,
	out: &Path,
	mut build_service: impl FnMut(&str) -> Result<super::services::Built>,
) -> Result<PathBuf> {
	let root = root.canonicalize()?;
	let output = output_path(&root, out)?;

	for artifact in config.artifacts.values() {
		check_output_parents(&root, &artifact.output)?;
		let source = output_path(&root, &artifact.output)?;
		ensure!(
			!output.starts_with(&source) && !source.starts_with(&output),
			"graph receipt output overlaps an artifact producer output"
		);
	}

	let project = Root::open(&root)?;
	let image_pins = pins(&project, config, selected)?;
	let cli_sha256 = cli_hash()?;
	let state = root.join(".leptoskit/graph");
	std::fs::create_dir_all(&state)?;
	let lock = std::fs::OpenOptions::new()
		.create(true)
		.truncate(false)
		.write(true)
		.open(state.join("build.lock"))?;
	fs4::FileExt::try_lock(&lock).context("another graph build is running")?;
	std::fs::create_dir_all(&output)?;
	let mut builder = tempfile::Builder::new();
	builder.prefix("build-");
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		builder.permissions(std::fs::Permissions::from_mode(0o700));
	}
	let temporary = builder.tempdir_in(&output)?;
	let mut recipes = BTreeMap::new();

	for name in &selected.selection.artifacts {
		if let Some(recipe) = config.artifacts.get(name) {
			run_recipe(&root, name, recipe, release)?;
			recipes.insert(name.clone(), hash(&serde_json::to_vec(recipe)?));
		}
	}

	let mut captured = BTreeMap::new();
	let mut service_inputs = Vec::new();

	for unit in selected.selection.stages.iter().flatten() {
		let directory = temporary.path().join(unit.to_string());
		std::fs::create_dir(&directory)?;

		let mut record = match unit {
			Unit::Frontend(name) => frontend(&project, config, &selected.plan, name, &directory)?,
			Unit::Service(name) => {
				let built = build_service(name)?;
				let binary = &built.binary;
				let service = built.inputs.capture(&directory.join("service"), binary)?;
				service_inputs.push(built.inputs);
				let parent = Root::open(
					binary
						.parent()
						.context("service executable has no parent")?,
				)?;
				let name = binary
					.file_name()
					.and_then(|name| name.to_str())
					.context("service binary name is not UTF-8")?;
				Captured {
					host: String::new(),
					service: Some(service),
					assets: None,
					backend: Some(parent.copy_executable(name, &directory.join("backend"))?),
					toolchain_image: None,
				}
			}
		};

		record
			.host
			.clone_from(&selected.plan.placements()[unit].host);

		if let Unit::Service(name) = unit {
			record.toolchain_image = image_pins.get(name).cloned();
		}

		captured.insert(unit.to_string(), record);
	}

	for inputs in &service_inputs {
		inputs.recheck()?;
	}
	ensure!(
		pins(&project, config, selected)? == image_pins,
		"toolchain image pins changed during the graph build"
	);
	ensure!(
		cli_hash()? == cli_sha256,
		"recipe CLI changed during the graph build"
	);
	let manifest = serde_json::to_vec_pretty(&serde_json::json!({
		"format_version":2, "kind":"leptoskit-graph-build", "configuration_sha256":hash(&serde_json::to_vec(config)?),
		"service_profile":if release {"release"} else {"dev"}, "cli_sha256":cli_sha256, "recipes":recipes, "units":captured,
		"readiness_only":selected.selection.readiness.iter().map(ToString::to_string).collect::<Vec<_>>(),
		"evidence":{"artifact_integrity":"captured", "source_reproduction":"not_checked", "isolated_build":"not_checked", "origin_authorization":"not_checked", "runtime":"not_checked"}
	}))?;
	std::fs::write(
		temporary.path().join("manifest.sha256"),
		format!("{}\n", hash(&manifest)),
	)?;
	std::fs::write(temporary.path().join("manifest.json"), &manifest)?;
	// Use the same bounded consumer as later verification before publishing
	// success, so a receipt can never exceed the reader's budget or shape.
	crate::verify::graph::check(&root, config, temporary.path(), &hash(&manifest))?;
	Ok(temporary.keep())
}

fn output_path(root: &Path, path: &Path) -> Result<PathBuf> {
	let path = if path.is_absolute() {
		path.to_owned()
	} else {
		root.join(path)
	};

	ensure!(
		!path
			.components()
			.any(|part| matches!(part, std::path::Component::ParentDir)),
		"output path must not contain parent traversal"
	);

	if path.exists() {
		return Ok(path.canonicalize()?);
	}

	let parent = path.parent().context("output path has no parent")?;

	Ok(output_path(root, parent)?.join(path.file_name().context("output path has no filename")?))
}

fn check_output_parents(root: &Path, relative: &Path) -> Result<()> {
	let mut path = root.to_owned();

	for component in relative.components() {
		path.push(component);

		match std::fs::symlink_metadata(&path) {
			Ok(metadata) => {
				ensure!(
					metadata.is_dir() && !metadata.file_type().is_symlink(),
					"artifact output ancestors must be real directories"
				);
			}

			Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
			Err(error) => return Err(error.into()),
		}
	}

	Ok(())
}

fn run_recipe(root: &Path, name: &str, recipe: &deployment::Artifact, release: bool) -> Result<()> {
	let (program, args) = recipe
		.command
		.split_first()
		.context("artifact recipe is empty")?;
	let result = Command::new(program)
		.args(args)
		.current_dir(root)
		.env("LEPTOSKIT_CLI", std::env::current_exe()?)
		.env(
			"LEPTOSKIT_BUILD_PROFILE",
			if release { "release" } else { "dev" },
		)
		.status()
		.with_context(|| format!("could not run artifact recipe {name}"))?;
	ensure!(
		result.success(),
		"artifact recipe {name} failed ({result}); no build receipt published"
	);
	Ok(())
}

fn frontend(
	project: &Root,
	config: &LeptoskitConfig,
	plan: &Plan,
	name: &str,
	destination: &Path,
) -> Result<Captured> {
	let declared = &config
		.graph_infra()
		.context("graph infrastructure required")?
		.frontends[name];
	let unit = Unit::Frontend(name.into());
	let artifact = project.beneath(
		config.artifacts[&declared.artifact]
			.output
			.to_str()
			.context("artifact output is not UTF-8")?,
	)?;
	let directory = if declared.directory == "." {
		""
	} else {
		&declared.directory
	};

	let assets = artifact.beneath(directory)?;
	let mut exclusions = plan.trees().exclusions(&unit).map_err(anyhow::Error::msg)?;
	let backend = if let Some(path) = &declared.backend {
		let path = path.to_str().context("backend path is not UTF-8")?;
		let owner = plan
			.trees()
			.owner(&declared.artifact, path)
			.map_err(anyhow::Error::msg)?;
		ensure!(
			owner.is_none_or(|owner| owner.unit == unit),
			"frontend backend lies in another frontend's public tree"
		);

		if directory.is_empty() {
			exclusions.push(path.into());
		} else if let Some(relative) = path
			.strip_prefix(directory)
			.and_then(|path| path.strip_prefix('/'))
		{
			exclusions.push(relative.into());
		}

		Some(artifact.copy_executable(path, &destination.join("backend"))?)
	} else {
		None
	};

	let tree = assets.tree(&exclusions)?;

	if declared.kind == FrontendKind::Static {
		ensure!(
			matches!(tree.get("index.html"),Some(Member::File {size,..}) if *size>0),
			"static frontend {name} requires a nonempty index.html"
		);
	}

	assets.copy(&destination.join("assets"), &exclusions, &tree)?;
	Ok(Captured {
		host: String::new(),
		service: None,
		assets: Some(tree),
		backend,
		toolchain_image: None,
	})
}

fn pins(
	project: &Root,
	config: &LeptoskitConfig,
	selected: &Selected,
) -> Result<BTreeMap<String, String>> {
	let mut pins = BTreeMap::new();

	for unit in selected.selection.stages.iter().flatten() {
		let Unit::Service(name) = unit else { continue };

		if let Some(runtime) = &config.services[name].runtime {
			let path = runtime
				.container
				.image_pin
				.to_str()
				.context("image-pin path is not UTF-8")?;
			let value = String::from_utf8(project.read_file(path, 4096)?)?;
			let value = value.trim();
			ensure!(
				value.rsplit_once("@sha256:").is_some_and(|(name, digest)| {
					!name.is_empty()
						&& name
							.bytes()
							.all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
						&& digest.len() == 64
						&& digest
							.bytes()
							.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
				}),
				"services.{name} needs an immutable image pin with a lowercase SHA-256 digest"
			);
			pins.insert(name.clone(), value.to_owned());
		}
	}

	Ok(pins)
}

fn hash(bytes: &[u8]) -> String {
	hex::encode(sha2::Sha256::digest(bytes))
}

fn cli_hash() -> Result<String> {
	let mut file = std::fs::File::open(std::env::current_exe()?)?;
	let mut hash = sha2::Sha256::new();
	std::io::copy(&mut file, &mut hash)?;
	Ok(hex::encode(hash.finalize()))
}
