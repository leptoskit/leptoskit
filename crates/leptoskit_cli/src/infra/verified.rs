//! Strict backend and website packaging. Descriptors are operator evidence, not
//! attestations.
use std::fs;
use std::io::Read as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::verify::build::Engine;
use crate::verify::build::{self};

mod rpc;
mod schema_window;
use rpc::Contract as RpcContract;

#[derive(Debug, clap::Args)]
pub struct Args {
	#[command(flatten)]
	reproduction: build::ReproduceArgs,
	/// Digest-pinned runtime image (not a mutable tag).
	#[arg(long)]
	runtime_image: String,
	/// Independently selected SHA-256 of the runtime image's Config JSON.
	#[arg(long)]
	runtime_config_sha256: String,
	/// Local image tag. This command never pushes.
	#[arg(long)]
	tag: String,
	/// Managed infra.name required by Kamal; bound into the image
	/// configuration.
	#[arg(long)]
	service: Option<String>,
	/// Inspect an existing local tag instead of packaging a new image.
	#[arg(long)]
	reuse_existing: bool,
	/// Explicit operator declaration that this backend exposes no RPC API.
	#[arg(long)]
	no_rpc: bool,
	/// Previously published contract to keep compatible; repeat for supported
	/// client versions.
	#[arg(long, requires = "rpc_baseline_sha256", conflicts_with = "no_rpc")]
	rpc_baseline: Vec<PathBuf>,
	/// Independently selected SHA-256 for each RPC baseline, in the same order.
	#[arg(long, requires = "rpc_baseline", conflicts_with = "no_rpc")]
	rpc_baseline_sha256: Vec<String>,
	/// Checked expansion window to bind to this exact executable and
	/// descriptor. This prepares evidence; deployment still enforces its
	/// rollout rules.
	#[arg(long, requires = "migration_window_sha256")]
	migration_window: Option<PathBuf>,
	/// Independent SHA-256 of the migration window's exact bytes.
	#[arg(long, requires = "migration_window")]
	migration_window_sha256: Option<String>,
	/// New descriptor file, written only after image extraction succeeds.
	#[arg(long)]
	out: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CheckState {
	Absent,
	NotChecked,
	Unsupported,
	NotApplicable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Descriptor {
	format_version: u32,
	kind: String,
	pub(super) inventory_sha256: String,
	pub(super) recipe: crate::verify::BuildRecipe,
	pub(super) executable_sha256: String,
	executable_mode: u32,
	pub(super) migrations: String,
	runtime_image: String,
	runtime_config_sha256: String,
	/// Engine content-addressed configuration ID, never a registry digest.
	pub(super) engine_image_id: String,
	image_config_sha256: String,
	platform: String,
	assets: Assets,
	services: CheckState,
	rpc_contract: RpcContract,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	schema_window: Option<schema_window::Evidence>,
	native_guest_compatibility: CheckState,
	runtime_attestation: CheckState,
}

/// Legacy backend descriptors keep the string `not_applicable`. Website
/// descriptors must carry the complete reproduced tree and its profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Assets {
	NotApplicable,
	Reproduced {
		profile: String,
		toolchain_sha256: String,
		entries: crate::verify::website::Tree,
	},
}

fn rpc_contract(
	engine: Engine,
	image: &str,
	no_rpc: bool,
	baselines: &[rpc::Baseline],
) -> Result<RpcContract> {
	if no_rpc {
		return Ok(RpcContract::NotApplicable {
			declaration: "operator_declares_no_rpc".into(),
		});
	}

	RpcContract::exported(
		export_document(
			engine,
			image,
			"LEPTOSKIT_RPC_SPEC_OUT",
			"rpc.json",
			"RPC",
			16 * 1024 * 1024,
		)?,
		baselines,
	)
}

fn export_document(
	engine: Engine,
	image: &str,
	variable: &str,
	filename: &str,
	kind: &str,
	max_bytes: u64,
) -> Result<Vec<u8>> {
	let dir = tempfile::tempdir()?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o777))?;
	}
	let mount = format!("type=bind,source={},target=/spec", dir.path().display());
	let environment = format!("{variable}=/spec/{filename}");
	ensure!(
		!dir.path().to_string_lossy().contains(','),
		"invalid export mount"
	);
	let container = String::from_utf8(output(
		engine,
		&[
			"create",
			"--network=none",
			"--read-only",
			"--cap-drop=ALL",
			"--security-opt=no-new-privileges",
			"--pids-limit=32",
			"--memory=128m",
			"--ulimit=fsize=16777216:16777216",
			"--user=65532:65532",
			"--mount",
			&mount,
			"--env",
			&environment,
			"--entrypoint=/app/backend",
			image,
		],
	)?)?
	.trim()
	.to_owned();
	let checked = (|| -> Result<Vec<u8>> {
		output(engine, &["start", &container])?;
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
		loop {
			let states: Vec<Value> =
				serde_json::from_slice(&output(engine, &["inspect", &container])?)?;
			let state = &states.first().context("missing export container")?["State"];
			if state["Running"] == false {
				ensure!(
					state["ExitCode"] == 0,
					"{kind} export failed before successful exit"
				);
				break;
			}

			ensure!(
				std::time::Instant::now() < deadline,
				"{kind} export timed out; export before database/auth startup"
			);
			std::thread::sleep(std::time::Duration::from_millis(100));
		}

		let path = dir.path().join(filename);
		let metadata = fs::symlink_metadata(&path)
			.with_context(|| format!("executable did not export {kind}; export first in main"))?;
		ensure!(
			metadata.is_file() && metadata.len() <= max_bytes,
			"invalid {kind} export file"
		);
		Ok(fs::read(path)?)
	})();
	let removed = output(engine, &["rm", "--force", &container]);
	let contract = checked?;
	removed?;
	Ok(contract)
}

fn digest(bytes: &[u8]) -> String {
	use sha2::Digest as _;
	hex::encode(sha2::Sha256::digest(bytes))
}

fn hash_json(value: &Value) -> Result<String> {
	Ok(digest(&serde_json::to_vec(value)?))
}

// Docker materializes these absent OCI fields as null when inspecting a
// Podman-built image. The content-addressed image ID must still match exactly.
// Docker includes the algorithm prefix; Podman's inspect reports the same
// content address without it. Both must match all 64 lowercase hex digits.
fn image_id_matches(actual: &str, expected: &str) -> bool {
	let Some(expected) = expected.strip_prefix("sha256:") else {
		return false;
	};
	valid_digest(expected) && actual.strip_prefix("sha256:").unwrap_or(actual) == expected
}

fn remote_config_matches(config: &Value, expected: &str) -> Result<bool> {
	if hash_json(config)? == expected {
		return Ok(true);
	}

	let mut sparse = config.clone();

	if let Some(object) = sparse.as_object_mut() {
		for key in ["Cmd", "OnBuild", "Volumes"] {
			if object.get(key).is_some_and(Value::is_null) {
				object.remove(key);
			}
		}
	}

	Ok(hash_json(&sparse)? == expected)
}

fn output(engine: Engine, args: &[&str]) -> Result<Vec<u8>> {
	let output = Command::new(engine.program()).args(args).output()?;
	ensure!(
		output.status.success(),
		"container engine failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	Ok(output.stdout)
}

fn inspect(engine: Engine, image: &str) -> Result<Value> {
	let values: Vec<Value> =
		serde_json::from_slice(&output(engine, &["image", "inspect", image])?)?;
	values
		.into_iter()
		.next()
		.context("image inspection returned no image")
}

pub(super) fn valid_digest(value: &str) -> bool {
	value.len() == 64
		&& value
			.bytes()
			.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn pinned(image: &str) -> bool {
	image.rsplit_once("@sha256:").is_some_and(|(name, hash)| {
		!name.is_empty()
			&& name
				.bytes()
				.all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
			&& hash.len() == 64
			&& hash
				.bytes()
				.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
	})
}

pub(super) fn backend_only(source: &Path, config: &LeptoskitConfig) -> Result<()> {
	ensure!(
		config.app.platforms == [Platform::Server]
			&& config.build.client_package.is_none()
			&& config.build.backend != Some(false),
		"verified packaging supports backend-only projects; website assets evidence is unsupported"
	);
	no_application_services(source)
}

fn no_application_services(source: &Path) -> Result<()> {
	crate::verify::website::reject_services(source)
}

/// Validate the extracted tar member rather than trusting image labels.
fn executable(bytes: &[u8], expected: &str) -> Result<()> {
	let mut archive = tar::Archive::new(bytes);
	let mut entries = archive.entries()?;
	let mut entry = entries.next().context("missing image executable")??;
	ensure!(
		entry.header().entry_type().is_file() && entry.header().mode()? == 0o755,
		"image executable must be a regular file with mode 0755"
	);
	let mut content = Vec::new();
	entry.read_to_end(&mut content)?;
	ensure!(
		digest(&content) == expected,
		"image executable bytes differ"
	);
	drop(entry);

	ensure!(
		entries.next().is_none(),
		"unexpected extra image executable entries"
	);
	Ok(())
}

fn validate_image(image: &Value, platform: &str, config: &Value) -> Result<()> {
	ensure!(
		format!(
			"{}/{}",
			image["Os"].as_str().unwrap_or(""),
			image["Architecture"].as_str().unwrap_or("")
		) == platform,
		"wrong image platform"
	);
	let mut actual = image["Config"].clone();
	let mut expected = config.clone();

	for value in [&mut actual, &mut expected] {
		if value["Cmd"].is_null() {
			value["Cmd"] = json!([]);
		}
	}

	ensure!(actual == expected, "produced image configuration differs");
	Ok(())
}

fn source_schema(bundle: &build::ReproducedBundle) -> Result<String> {
	let source = bundle.root().join("source");
	let config = LeptoskitConfig::load_from(&source)?;

	match &bundle.recipe.website {
		Some(website) => {
			no_application_services(&source)?;
			website.source(&source, &config)?;
		}
		None => backend_only(&source, &config)?,
	}

	ensure!(
		bundle.recipe.package == config.app.name,
		"recipe package differs from managed backend"
	);
	let path = &config.db.as_ref().context("missing database")?.migrations;
	ensure!(
		!path.is_absolute()
			&& path
				.components()
				.all(|c| matches!(c, std::path::Component::Normal(_))),
		"migrations must be inside the verified source"
	);
	super::build::fingerprint(&source, &config)
}

fn validate_arguments(args: &Args) -> Result<()> {
	ensure!(!args.out.exists(), "descriptor output already exists");
	ensure!(
		!args.no_rpc || args.rpc_baseline.is_empty(),
		"--no-rpc conflicts with RPC compatibility baselines"
	);
	ensure!(
		!args.reproduction.offline && args.reproduction.checkout.is_none(),
		"managed verified packaging requires full reproduction from the canonical repository"
	);
	ensure!(
		pinned(&args.runtime_image),
		"runtime image must be digest-pinned"
	);
	ensure!(
		!args.tag.starts_with('-') && !args.tag.is_empty(),
		"invalid local image tag"
	);

	if let Some(service) = &args.service {
		ensure!(
			!service.is_empty()
				&& !service.starts_with('-')
				&& service
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
			"invalid managed service name"
		);
	}

	Ok(())
}

pub(super) fn run(args: &Args) -> Result<()> {
	let window = schema_window::Input::read(
		args.migration_window.as_deref(),
		args.migration_window_sha256.as_deref(),
	)?;
	validate_arguments(args)?;
	let baselines = rpc::baselines(&args.rpc_baseline, &args.rpc_baseline_sha256)?;
	let bundle = build::reproduce(&args.reproduction)?;
	ensure!(bundle.full, "full reproduction required");
	let migrations = source_schema(&bundle)?;
	let engine = Engine::detect(args.reproduction.engine)?;
	let platform = build::platform(&bundle.recipe.target)?;

	output(
		engine,
		&["pull", "--platform", platform, &args.runtime_image],
	)?;
	let runtime = inspect(engine, &args.runtime_image)?;
	ensure!(
		hash_json(&runtime["Config"])? == args.runtime_config_sha256,
		"runtime configuration pin differs"
	);
	let mut expected = runtime["Config"].clone();
	ensure!(
		expected["OnBuild"].as_array().is_none_or(Vec::is_empty),
		"runtime ONBUILD instructions are forbidden"
	);
	ensure!(
		expected["Volumes"].is_null(),
		"runtime volumes are unsupported"
	);
	// No application source, Dockerfile, build scripts, or credentials enter
	// this context.
	let context = tempfile::tempdir()?;
	let entries = crate::verify::walk(bundle.root())?;
	ensure!(
		matches!(
			entries.get("artifacts/backend"),
			Some(crate::verify::Entry::File {
				unix_mode: Some(0o755),
				..
			})
		),
		"verified executable must have mode 0755"
	);
	let backend = fs::read(bundle.root().join("artifacts/backend"))?;
	let backend_hash = digest(&backend);
	fs::write(context.path().join("backend"), &backend)?;

	let assets = match &bundle.recipe.website {
		Some(website) => {
			Assets::Reproduced {
				profile: website.profile.clone(),
				toolchain_sha256: website.toolchain_sha256.clone(),
				entries: crate::verify::website::copy(
					&bundle.root().join("artifacts/site"),
					&context.path().join("site"),
					&website.output_name,
				)?,
			}
		}
		None => Assets::NotApplicable,
	};
	let website_copy = if bundle.recipe.website.is_some() {
		"COPY site /app/dist/\n"
	} else {
		""
	};

	let dockerfile = format!(
		"FROM {}\nCOPY --chmod=0755 backend /app/backend\n{website_copy}WORKDIR /app\nUSER \
		 65532:65532\nENTRYPOINT [\"/app/backend\"]\nCMD []\n",
		args.runtime_image
	);
	fs::write(context.path().join("Dockerfile"), dockerfile)?;
	expected["WorkingDir"] = json!("/app");
	expected["User"] = json!("65532:65532");
	expected["Entrypoint"] = json!(["/app/backend"]);
	expected["Cmd"] = json!([]);
	let label = format!("{}={migrations}", super::build::SCHEMA_LABEL);

	if expected["Labels"].is_null() {
		expected["Labels"] = json!({});
	}

	expected["Labels"][super::build::SCHEMA_LABEL] = json!(migrations);
	let service_label = args
		.service
		.as_ref()
		.map(|service| format!("service={service}"));

	if let Some(service) = &args.service {
		expected["Labels"]["service"] = json!(service);
	}

	if !args.reuse_existing {
		let mut arguments = vec![
			"build",
			"--network=none",
			"--platform",
			platform,
			"--label",
			&label,
			"--tag",
			&args.tag,
		];

		if let Some(service_label) = &service_label {
			arguments.extend(["--label", service_label]);
		}

		arguments.push(
			context
				.path()
				.to_str()
				.context("context path is not UTF-8")?,
		);
		output(engine, &arguments)?;
	}

	let image = inspect(engine, &args.tag)?;
	validate_image(&image, platform, &expected)?;
	let id = image["Id"].as_str().context("missing image ID")?;
	check_contents(engine, id, context.path(), &backend_hash, &assets)?;
	let rpc_contract = rpc_contract(engine, id, args.no_rpc, &baselines)?;
	let checked_rpc = rpc_contract.checked();
	let schema_window = window
		.map(|window| window.package(engine, id, &bundle))
		.transpose()?;
	let descriptor = Descriptor {
		format_version: if schema_window.is_some() {
			4
		} else if checked_rpc {
			3
		} else if bundle.recipe.website.is_some() {
			2
		} else {
			1
		},

		kind: "leptoskit-verified-backend-image".into(),
		inventory_sha256: bundle.inventory_sha256.clone(),
		recipe: bundle.recipe.clone(),
		executable_sha256: backend_hash,
		executable_mode: 0o755,
		migrations,
		runtime_image: args.runtime_image.clone(),
		runtime_config_sha256: args.runtime_config_sha256.clone(),
		engine_image_id: format!("sha256:{}", id.strip_prefix("sha256:").unwrap_or(id)),
		image_config_sha256: hash_json(&image["Config"])?,
		platform: platform.into(),
		assets,
		services: CheckState::Unsupported,
		rpc_contract,
		schema_window,
		native_guest_compatibility: CheckState::NotApplicable,
		runtime_attestation: CheckState::Absent,
	};
	let bytes = write_descriptor(&args.out, &descriptor)?;
	println!(
		"Packaged {id}; descriptor SHA-256 {}. No registry digest or runtime attestation.",
		digest(&bytes)
	);
	Ok(())
}

fn check_contents(
	engine: Engine,
	id: &str,
	context: &Path,
	backend_hash: &str,
	assets: &Assets,
) -> Result<()> {
	let container = String::from_utf8(output(engine, &["create", id])?)?
		.trim()
		.to_owned();
	let checked = (|| -> Result<()> {
		executable(
			&output(engine, &["cp", &format!("{container}:/app/backend"), "-"])?,
			backend_hash,
		)?;
		// Reject additional application services inherited from the base.
		let exported = context.join("image.tar");
		output(
			engine,
			&[
				"export",
				"--output",
				exported.to_str().context("export path")?,
				&container,
			],
		)?;
		crate::verify::website::image_tree(
			fs::File::open(exported)?,
			match assets {
				Assets::NotApplicable => None,
				Assets::Reproduced { entries, .. } => Some(entries),
			},
		)?;
		Ok(())
	})();

	let removed = output(engine, &["rm", &container]);
	checked?;
	removed?;
	Ok(())
}

fn write_descriptor(path: &Path, descriptor: &Descriptor) -> Result<Vec<u8>> {
	descriptor.validate()?;
	let bytes = serde_json::to_vec_pretty(descriptor)?;
	use std::io::Write as _;
	fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(path)?
		.write_all(&bytes)?;
	Ok(bytes)
}

pub(super) fn read(path: &Path, pin: &str) -> Result<Descriptor> {
	let bytes = fs::read(path)?;
	ensure!(
		digest(&bytes) == pin,
		"descriptor does not match independently authenticated pin"
	);
	let descriptor: Descriptor = serde_json::from_slice(&bytes)?;
	descriptor.validate()?;
	Ok(descriptor)
}

impl Descriptor {
	pub(super) fn migration_window(&self) -> Result<Option<super::schema_window::Window>> {
		self.validate()?;
		self.schema_window
			.as_ref()
			.map(schema_window::Evidence::runtime)
			.transpose()
	}

	pub(super) fn require_rpc(&self, pins: &[String]) -> Result<()> {
		self.rpc_contract.require(pins)
	}

	pub(super) fn validate(&self) -> Result<()> {
		crate::verify::validate(&self.recipe)?;
		ensure!(
			pinned(&self.runtime_image),
			"runtime image must be digest-pinned"
		);
		ensure!(
			build::platform(&self.recipe.target)? == self.platform,
			"recipe target and image platform differ"
		);
		for value in [
			&self.inventory_sha256,
			&self.executable_sha256,
			&self.migrations,
			&self.runtime_config_sha256,
			&self.image_config_sha256,
		] {
			ensure!(valid_digest(value), "invalid descriptor SHA-256");
		}
		self.rpc_contract.validate()?;

		if let Some(window) = &self.schema_window {
			window.validate(&self.migrations)?;
		}

		ensure!(
			(self.format_version
				== if self.schema_window.is_some() {
					4
				} else if self.rpc_contract.checked() {
					3
				} else if self.recipe.website.is_some() {
					2
				} else {
					1
				})
				&& self.kind == "leptoskit-verified-backend-image"
				&& self.services == CheckState::Unsupported
				&& self.native_guest_compatibility == CheckState::NotApplicable
				&& self.runtime_attestation == CheckState::Absent
				&& self.executable_mode == 0o755,
			"unsupported release evidence"
		);

		match (&self.recipe.website, &self.assets) {
			(None, Assets::NotApplicable) => {}
			(
				Some(website),
				Assets::Reproduced {
					profile,
					toolchain_sha256,
					entries,
				},
			) => {
				ensure!(
					profile == &website.profile && toolchain_sha256 == &website.toolchain_sha256,
					"website evidence differs from recipe"
				);
				crate::verify::website::validate_tree(entries, &website.output_name)?;
			}
			_ => {
				anyhow::bail!(
					"website profile requires reproduced assets; backend-only assets must be \
					 not_applicable"
				)
			}
		}

		ensure!(
			self.engine_image_id
				.strip_prefix("sha256:")
				.is_some_and(valid_digest),
			"invalid engine image ID"
		);
		Ok(())
	}

	/// Enforce the release's local application kind even on rollback/recovery.
	/// The recorded configuration pin remains historical; it need not match
	/// today's config bytes when selecting an explicitly authorized old
	/// release.
	pub(super) fn application(&self, source: &Path, config: &LeptoskitConfig) -> Result<()> {
		self.validate()?;
		no_application_services(source)?;

		match &self.recipe.website {
			Some(_) => crate::verify::website::Recipe::configuration(config),
			None => backend_only(source, config),
		}
	}

	pub(super) fn check_remote(&self, host: &str, tag: &str, schema: &str) -> Result<()> {
		self.validate()?;
		ensure!(
			self.migrations == schema,
			"descriptor schema differs from release"
		);
		let bytes = super::remote::execute(
			host,
			&format!("docker image inspect {}", super::remote::quote(tag)),
			None,
		)?;
		let images: Vec<Value> = serde_json::from_slice(&bytes)?;
		let image = images.first().context("remote image absent")?;
		ensure!(
			image["Id"]
				.as_str()
				.is_some_and(|id| image_id_matches(id, &self.engine_image_id))
				&& remote_config_matches(&image["Config"], &self.image_config_sha256)?,
			"remote image differs from reproduced packaging evidence"
		);
		ensure!(
			format!(
				"{}/{}",
				image["Os"].as_str().unwrap_or(""),
				image["Architecture"].as_str().unwrap_or("")
			) == self.platform,
			"remote platform differs"
		);
		Ok(())
	}
}

pub(super) fn validate_rpc_pins(pins: &[String]) -> Result<()> {
	rpc::validate_pins(pins)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn archive(content: &[u8], mode: u32) -> Vec<u8> {
		let mut builder = tar::Builder::new(Vec::new());
		let mut header = tar::Header::new_gnu();
		header.set_size(content.len() as u64);
		header.set_mode(mode);
		header.set_cksum();
		builder
			.append_data(&mut header, "backend", content)
			.unwrap();
		builder.into_inner().unwrap()
	}

	#[test]
	fn engine_image_ids_accept_only_exact_digest_spelling() {
		let digest = "a".repeat(64);
		let pinned = format!("sha256:{digest}");
		assert!(image_id_matches(&digest, &pinned));
		assert!(image_id_matches(&pinned, &pinned));
		for actual in [
			"a".repeat(63),
			"A".repeat(64),
			"b".repeat(64),
			format!("sha512:{digest}"),
			format!("{pinned}extra"),
		] {
			assert!(!image_id_matches(&actual, &pinned));
		}
		assert!(!image_id_matches("bad", "sha256:bad"));
	}

	#[test]
	fn docker_null_defaults_preserve_podman_config_identity() {
		let original =
			json!({"Labels":{"service":"accounts"},"Env":["SAFE=1"],"Entrypoint":["/app/backend"]});
		let pin = hash_json(&original).unwrap();
		let mut docker = original.clone();

		for key in ["Cmd", "OnBuild", "Volumes"] {
			docker[key] = Value::Null;
		}

		assert!(remote_config_matches(&docker, &pin).unwrap());
		assert!(remote_config_matches(&docker, &hash_json(&docker).unwrap()).unwrap());

		for (key, value) in [
			("Cmd", json!(["other"])),
			("OnBuild", json!(["RUN true"])),
			("Volumes", json!({"/data":{}})),
			("Labels", json!({"service":"website"})),
			("Env", json!(["SAFE=0"])),
		] {
			let mut changed = docker.clone();
			changed[key] = value;
			assert!(!remote_config_matches(&changed, &pin).unwrap(), "{key}");
		}
	}

	#[test]
	fn binary_hashing_does_not_collapse_invalid_utf8() {
		assert_ne!(digest(&[0xff]), digest(&[0xfe]));
		assert!(executable(&archive(&[0xfe], 0o755), &digest(&[0xff])).is_err());
	}

	#[test]
	fn image_bytes_and_mode_are_checked_without_trusting_labels() {
		let pin = digest(b"verified");
		assert!(executable(&archive(b"verified", 0o755), &pin).is_ok());
		assert!(executable(&archive(b"substituted", 0o755), &pin).is_err());
		assert!(executable(&archive(b"verified", 0o644), &pin).is_err());
	}

	#[test]
	fn image_platform_entrypoint_user_and_environment_are_bound() {
		let config = json!({"Entrypoint": ["/app/backend"], "Cmd": [], "User": "65532:65532", "Env": ["SAFE=1"]});
		let image = json!({"Os": "linux", "Architecture": "amd64", "Config": config});
		assert!(validate_image(&image, "linux/amd64", &config).is_ok());
		assert!(validate_image(&image, "linux/arm64", &config).is_err());
		for (key, replacement) in [
			("Entrypoint", json!(["/bin/sh"])),
			("User", json!("0")),
			("Env", json!(["SAFE=0"])),
			("Labels", json!({"service":"different-deployment"})),
		] {
			let mut changed = image.clone();
			changed["Config"][key] = replacement;
			assert!(validate_image(&changed, "linux/amd64", &config).is_err());
		}
	}

	#[test]
	fn runtime_tags_and_dockerfile_injection_are_rejected() {
		assert!(pinned(&format!(
			"registry/runtime@sha256:{}",
			"a".repeat(64)
		)));
		for value in [
			"runtime:latest",
			"runtime@sha256:abc",
			"runtime\nRUN evil@sha256:",
		] {
			assert!(!pinned(value));
		}
	}

	#[test]
	fn website_and_service_evidence_cannot_be_omitted() {
		let dir = tempfile::tempdir().unwrap();
		let raw = concat!(
			"[app]\nname='backend'\n",
			"platforms=['server']\ndefault_platform='server'\n",
			"render='ssr'\n",
		);
		fs::write(dir.path().join("leptoskit.toml"), raw).unwrap();
		let mut config = LeptoskitConfig::from_toml(raw, dir.path()).unwrap();
		assert!(backend_only(dir.path(), &config).is_ok());

		config.app.platforms = vec![Platform::Web];
		assert!(backend_only(dir.path(), &config).is_err());
		config.app.platforms = vec![Platform::Server];
		fs::write(
			dir.path().join("leptoskit.toml"),
			format!("{raw}\n[infra.services]\ncache=true\nrelay=true\n"),
		)
		.unwrap();
		assert!(backend_only(dir.path(), &config).is_ok());
		fs::create_dir(dir.path().join("services")).unwrap();
		assert!(backend_only(dir.path(), &config).is_err());
		fs::remove_dir(dir.path().join("services")).unwrap();

		fs::write(
			dir.path().join("leptoskit.toml"),
			format!("{raw}\n[services.worker]\npackage='worker'\n"),
		)
		.unwrap();
		assert!(backend_only(dir.path(), &config).is_err());
	}

	#[test]
	fn legacy_history_has_no_evidence() {
		let release: super::super::Release =
			serde_json::from_value(json!({"version":"old","migrations":"schema"})).unwrap();
		assert!(release.evidence.is_none());
	}

	#[test]
	fn replaced_descriptor_cannot_use_original_pin() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("descriptor.json");
		fs::write(&path, b"{}").unwrap();
		assert!(
			read(&path, &digest(b"original"))
				.unwrap_err()
				.to_string()
				.contains("pin")
		);
		assert!(read(&path, &digest(b"{}")).is_err());
	}
}

#[cfg(test)]
mod acceptance;
