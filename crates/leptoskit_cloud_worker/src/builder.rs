//! Built-in OCI image builder (`leptoskit-cloud-worker build-app`).
//!
//! Ships a default build pipeline so a `leptoskit-cloud-worker` build worker
//! works out of the box without an operator-authored shell script:
//!
//! 1. Reads [`BuildRequest`] from `LEPTOSKIT_BUILD_INPUT`.
//! 2. Ensures a `Dockerfile` exists in `LEPTOSKIT_BUILD_SOURCE`, generating one
//!    via `lk generate deploy --target docker` (or a standard `LeptosKit`
//!    multi-stage fallback when `lk` is unavailable) if the repository doesn't
//!    commit its own.
//! 3. Builds and pushes the OCI image to `request.image_repository` using
//!    rootless `BuildKit` (`buildctl-daemonless.sh` / `buildctl`) or `docker
//!    buildx build --push`, reusing persistent `BuildKit` cache layers under
//!    `LEPTOSKIT_BUILD_CACHE_DIR/buildkit` when available.
//! 4. Extracts the immutable `sha256:…` digest from `BuildKit`'s metadata file
//!    and writes [`BuildOutcome`] JSON to `LEPTOSKIT_BUILD_OUTPUT`.

use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use leptoskit_cloud::archive::Digest;
use leptoskit_cloud::deployments::BuildStep;
use leptoskit_cloud::workers::BuildOutcome;
use leptoskit_cloud::workers::BuildServerOutput;
use leptoskit_cloud::workers::BuiltImage;
use leptoskit_cloud::workers::ProcessFacts;
use leptoskit_cloud::workers::WebAssets;

use crate::command::BuildRequest;

/// Which container image builder backend `build-app` invokes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum BuilderBackend {
	/// Prefer rootless `buildctl-daemonless.sh`, then `buildctl`, then
	/// `docker`.
	#[default]
	Auto,
	/// Rootless `BuildKit` (`buildctl-daemonless.sh` or `buildctl`).
	Buildctl,
	/// Docker Buildx (`docker buildx build --push`).
	Docker,
}

/// Arguments for the built-in `leptoskit-cloud-worker build-app` subcommand.
#[derive(Clone, Debug, clap::Args)]
pub struct BuildAppArgs {
	/// Path to the `BuildRequest` JSON file (defaults to
	/// `LEPTOSKIT_BUILD_INPUT`).
	#[arg(long, env = "LEPTOSKIT_BUILD_INPUT")]
	pub input: PathBuf,
	/// Path where `BuildOutcome` JSON is written (defaults to
	/// `LEPTOSKIT_BUILD_OUTPUT`).
	#[arg(long, env = "LEPTOSKIT_BUILD_OUTPUT")]
	pub output: PathBuf,
	/// Source directory to build (defaults to `LEPTOSKIT_BUILD_SOURCE` or `.`).
	#[arg(long, env = "LEPTOSKIT_BUILD_SOURCE")]
	pub source: Option<PathBuf>,
	/// Persistent cache directory (defaults to `LEPTOSKIT_BUILD_CACHE_DIR`).
	#[arg(long, env = "LEPTOSKIT_BUILD_CACHE_DIR")]
	pub cache_dir: Option<PathBuf>,
	/// Container builder backend to invoke.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_BUILDER_BACKEND",
		value_enum,
		default_value_t = BuilderBackend::Auto
	)]
	pub backend: BuilderBackend,
	/// Path to the `lk` / `leptoskit` CLI used to generate a Dockerfile when
	/// the project does not include one.
	#[arg(long, env = "LEPTOSKIT_CLOUD_BUILDER_LK_BIN", default_value = "lk")]
	pub lk_bin: PathBuf,
	/// Optional target OCI platform (e.g. `linux/amd64` or `linux/arm64`).
	#[arg(long, env = "LEPTOSKIT_CLOUD_BUILDER_PLATFORM")]
	pub platform: Option<String>,
	/// Push to an HTTP (non-TLS) cluster-local registry such as Zot.
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_BUILDER_INSECURE_REGISTRY",
		default_value_t = false
	)]
	pub insecure_registry: bool,
	/// Port the app server listens on inside the container (default 8080).
	#[arg(long, env = "LEPTOSKIT_CLOUD_BUILDER_PORT", default_value_t = 8080)]
	pub port: u16,
}

/// Runs the built-in OCI app builder. Always attempts to write a structured
/// [`BuildOutcome`] JSON to `args.output` so the worker reports actionable
/// step timings and error messages back to the control plane.
///
/// # Errors
///
/// Returns an error string when the build fails or `args.output` cannot be
/// written.
pub async fn run(args: BuildAppArgs) -> Result<(), String> {
	let input_bytes = tokio::fs::read(&args.input)
		.await
		.map_err(|error| format!("couldn't read {}: {error}", args.input.display()))?;
	let request: BuildRequest = serde_json::from_slice(&input_bytes)
		.map_err(|error| format!("invalid build input JSON: {error}"))?;
	let source = args
		.source
		.clone()
		.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

	let mut steps = Vec::new();
	let outcome = match build_and_push(&args, &request, &source, &mut steps).await {
		Ok(output) => BuildOutcome::Succeeded(output),
		Err(reason) => BuildOutcome::Failed { reason, steps },
	};

	let encoded = serde_json::to_vec_pretty(&outcome)
		.map_err(|error| format!("couldn't serialize build outcome: {error}"))?;
	tokio::fs::write(&args.output, encoded)
		.await
		.map_err(|error| format!("couldn't write {}: {error}", args.output.display()))?;

	match outcome {
		BuildOutcome::Succeeded(_) => Ok(()),
		BuildOutcome::Failed { reason, .. } => Err(reason),
		BuildOutcome::Stopped => Err("build stopped".to_owned()),
	}
}

async fn build_and_push(
	args: &BuildAppArgs,
	request: &BuildRequest,
	source: &Path,
	steps: &mut Vec<BuildStep>,
) -> Result<BuildServerOutput, String> {
	let dockerfile = ensure_dockerfile(args, request, source, steps).await?;
	let resolved = resolve_backend(args.backend)?;
	let metadata_file = std::env::temp_dir().join(format!(
		"lk-build-meta-{}-{}.json",
		request.deployment.number,
		std::process::id()
	));
	let _ = tokio::fs::remove_file(&metadata_file).await;

	let build_started = Instant::now();
	let push_tag = format!(
		"{}:d-{}",
		request.image_repository, request.deployment.number
	);
	let cache_root = args.cache_dir.as_ref().map(|dir| dir.join("buildkit"));
	if let Some(dir) = &cache_root {
		let _ = tokio::fs::create_dir_all(dir).await;
	}

	let status = match resolved {
		ResolvedBackend::Buildctl(program) => {
			let mut cmd = tokio::process::Command::new(&program);
			cmd.arg("build")
				.arg("--frontend")
				.arg("dockerfile.v0")
				.arg("--local")
				.arg(format!("context={}", source.display()))
				.arg("--local")
				.arg(format!(
					"dockerfile={}",
					dockerfile.parent().unwrap_or(source).display()
				))
				.arg("--opt")
				.arg(format!(
					"filename={}",
					dockerfile
						.file_name()
						.and_then(|name| name.to_str())
						.unwrap_or("Dockerfile")
				))
				.arg("--metadata-file")
				.arg(&metadata_file);
			if let Some(platform) = &args.platform {
				cmd.arg("--opt").arg(format!("platform={platform}"));
			}
			let mut output_spec = format!("type=image,name={push_tag},push=true");
			if args.insecure_registry {
				output_spec.push_str(",registry.insecure=true");
			}
			cmd.arg("--output").arg(output_spec);
			if let Some(dir) = &cache_root {
				cmd.arg("--import-cache")
					.arg(format!("type=local,src={}", dir.display()))
					.arg("--export-cache")
					.arg(format!("type=local,dest={},mode=max", dir.display()));
			}
			cmd.current_dir(source)
				.status()
				.await
				.map_err(|error| format!("couldn't run {}: {error}", program.display()))?
		}
		ResolvedBackend::Docker => {
			let mut cmd = tokio::process::Command::new("docker");
			cmd.arg("buildx")
				.arg("build")
				.arg("--push")
				.arg("--file")
				.arg(&dockerfile)
				.arg("--tag")
				.arg(&push_tag)
				.arg("--metadata-file")
				.arg(&metadata_file);
			if let Some(platform) = &args.platform {
				cmd.arg("--platform").arg(platform);
			}
			if let Some(dir) = &cache_root {
				cmd.arg("--cache-from")
					.arg(format!("type=local,src={}", dir.display()))
					.arg("--cache-to")
					.arg(format!("type=local,dest={},mode=max", dir.display()));
			}
			cmd.arg(source)
				.current_dir(source)
				.status()
				.await
				.map_err(|error| format!("couldn't run docker buildx: {error}"))?
		}
	};

	let elapsed_ms = u64::try_from(build_started.elapsed().as_millis()).unwrap_or(u64::MAX);
	steps.push(BuildStep {
		name: "container build & push".to_owned(),
		millis: elapsed_ms,
		cached_percent: None,
	});

	if !status.success() {
		let _ = tokio::fs::remove_file(&metadata_file).await;
		return Err(match status.code() {
			Some(code) => format!("container build exited with status {code}"),
			None => "container build was terminated by a signal".to_owned(),
		});
	}

	let metadata_bytes = tokio::fs::read(&metadata_file)
		.await
		.map_err(|error| format!("container builder did not write metadata file: {error}"))?;
	let _ = tokio::fs::remove_file(&metadata_file).await;
	let digest = extract_digest_from_metadata(&metadata_bytes)?;

	Ok(build_server_output(
		request,
		digest,
		args.port,
		steps.clone(),
	))
}

async fn ensure_dockerfile(
	args: &BuildAppArgs,
	request: &BuildRequest,
	source: &Path,
	steps: &mut Vec<BuildStep>,
) -> Result<PathBuf, String> {
	let existing = source.join("Dockerfile");
	if existing.is_file() {
		return Ok(existing);
	}

	let started = Instant::now();
	let generated = tokio::process::Command::new(&args.lk_bin)
		.args(["generate", "deploy", "--target", "docker"])
		.current_dir(source)
		.status()
		.await;

	let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
	match generated {
		Ok(status) if status.success() && existing.is_file() => {
			steps.push(BuildStep {
				name: "lk generate deploy --target docker".to_owned(),
				millis: elapsed_ms,
				cached_percent: None,
			});
			return Ok(existing);
		}
		_ => {}
	}

	let fallback = render_fallback_dockerfile(&request.toolchain, args.port);
	tokio::fs::write(&existing, fallback)
		.await
		.map_err(|error| format!("couldn't write fallback Dockerfile: {error}"))?;
	steps.push(BuildStep {
		name: "generate default Dockerfile".to_owned(),
		millis: elapsed_ms,
		cached_percent: None,
	});
	Ok(existing)
}

#[derive(Debug, PartialEq, Eq)]
enum ResolvedBackend {
	Buildctl(PathBuf),
	Docker,
}

fn resolve_backend(requested: BuilderBackend) -> Result<ResolvedBackend, String> {
	match requested {
		BuilderBackend::Buildctl => {
			find_on_path("buildctl-daemonless.sh")
				.or_else(|| find_on_path("buildctl"))
				.map(ResolvedBackend::Buildctl)
				.ok_or_else(|| "buildctl-daemonless.sh / buildctl not found on PATH".to_owned())
		}
		BuilderBackend::Docker => {
			find_on_path("docker")
				.map(|_| ResolvedBackend::Docker)
				.ok_or_else(|| "docker not found on PATH".to_owned())
		}
		BuilderBackend::Auto => {
			if let Some(program) = find_on_path("buildctl-daemonless.sh") {
				Ok(ResolvedBackend::Buildctl(program))
			} else if let Some(program) = find_on_path("buildctl") {
				Ok(ResolvedBackend::Buildctl(program))
			} else if find_on_path("docker").is_some() {
				Ok(ResolvedBackend::Docker)
			} else {
				Err(
					"no OCI image builder found on PATH (install rootless buildkit \
					 `buildctl-daemonless.sh` / `buildctl`, or `docker buildx`, or supply \
					 --build-command)"
						.to_owned(),
				)
			}
		}
	}
}

fn find_on_path(binary: &str) -> Option<PathBuf> {
	let path_var = std::env::var_os("PATH")?;
	for dir in std::env::split_paths(&path_var) {
		let candidate = dir.join(binary);
		if candidate.is_file() {
			return Some(candidate);
		}
	}
	None
}

/// Parses a `BuildKit` / `docker buildx` `--metadata-file` JSON document and
/// extracts the pushed image's `sha256:…` digest.
///
/// # Errors
///
/// Returns an error if the JSON is malformed, omits `containerimage.digest`,
/// or contains an invalid digest.
pub fn extract_digest_from_metadata(bytes: &[u8]) -> Result<Digest, String> {
	let value: serde_json::Value = serde_json::from_slice(bytes)
		.map_err(|error| format!("invalid build metadata JSON: {error}"))?;
	let raw = value
		.get("containerimage.digest")
		.and_then(serde_json::Value::as_str)
		.ok_or_else(|| "build metadata JSON is missing `containerimage.digest`".to_owned())?;
	raw.parse::<Digest>()
		.map_err(|error| format!("invalid image digest `{raw}` in build metadata: {error}"))
}

/// Constructs the [`BuildServerOutput`] for a built image from `request`.
#[must_use]
pub fn build_server_output(
	request: &BuildRequest,
	digest: Digest,
	port: u16,
	steps: Vec<BuildStep>,
) -> BuildServerOutput {
	let mut processes = vec![ProcessFacts {
		name: "app".to_owned(),
		image: None,
		command: Vec::new(),
		args: Vec::new(),
		port,
		ready_path: "/readyz".to_owned(),
		live_path: "/healthz".to_owned(),
	}];

	for (name, service) in &request.app.services {
		if service.remote {
			continue;
		}
		processes.push(ProcessFacts {
			name: name.clone(),
			image: None,
			command: Vec::new(),
			args: Vec::new(),
			port,
			ready_path: "/readyz".to_owned(),
			live_path: "/healthz".to_owned(),
		});
	}

	BuildServerOutput {
		image: BuiltImage {
			repository: request.image_repository.clone(),
			digest,
		},
		processes,
		web_assets: Some(WebAssets::InImage {
			path: "/srv/site".to_owned(),
		}),
		migrations: Vec::new(),
		steps,
	}
}

/// Generates a minimal multi-stage Rust/LeptosKit Dockerfile when a project
/// does not commit its own and `lk generate deploy` is unavailable.
#[must_use]
pub fn render_fallback_dockerfile(toolchain: &str, port: u16) -> String {
	let image_tag = if toolchain.trim().is_empty() || toolchain == "stable" {
		"rust:1-bookworm".to_owned()
	} else {
		"rustlang/rust:nightly-bookworm".to_owned()
	};
	format!(
		"# syntax=docker/dockerfile:1\nFROM {image_tag} AS builder\nWORKDIR /src\nCOPY . .\nRUN \
		 --mount=type=cache,target=/usr/local/cargo/registry \
		 \\\n--mount=type=cache,target=/src/target \\\ncargo build --release --locked \\\n&& find \
		 /src/target/release -maxdepth 1 -type f -executable -exec cp {{}} /usr/local/bin/server \
		 \\; -quit\n\nFROM debian:bookworm-slim\nRUN apt-get update && apt-get install -y \
		 --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*\nCOPY \
		 --from=builder /usr/local/bin/server /usr/local/bin/server\nUSER 65532:65532\nENV \
		 PORT={port}\nEXPOSE {port}\nENTRYPOINT [\"/usr/local/bin/server\"]\n"
	)
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use leptoskit_cloud::deployments::AppFacts;
	use leptoskit_cloud::deployments::ServiceFacts;
	use leptoskit_cloud::workers::DeploymentRef;

	use super::*;

	fn sample_request() -> BuildRequest {
		let mut services = BTreeMap::new();
		services.insert(
			"api".to_owned(),
			ServiceFacts {
				remote: false,
				public: true,
				isolated: false,
				database: false,
			},
		);
		services.insert(
			"external".to_owned(),
			ServiceFacts {
				remote: true,
				public: false,
				isolated: false,
				database: false,
			},
		);
		BuildRequest {
			deployment: DeploymentRef {
				organization: "ana".to_owned(),
				project: "notes".to_owned(),
				environment: "production".to_owned(),
				number: 7,
			},
			digest: Digest::of(b"source"),
			toolchain: "stable".to_owned(),
			targets: vec!["server".to_owned(), "web".to_owned()],
			image_repository: "zot.registry.svc.cluster.local:5000/ana/notes".to_owned(),
			app: AppFacts {
				database: None,
				live: false,
				services,
			},
		}
	}

	#[test]
	fn extracts_digest_from_buildkit_metadata_json() {
		let digest = Digest::of(b"oci-manifest");
		let json = format!(r#"{{"containerimage.digest":"{}"}}"#, digest.as_str());
		assert_eq!(
			extract_digest_from_metadata(json.as_bytes()).expect("parses"),
			digest
		);
		assert!(extract_digest_from_metadata(b"{}").is_err());
		assert!(
			extract_digest_from_metadata(br#"{"containerimage.digest":"not-a-digest"}"#).is_err()
		);
	}

	#[test]
	fn build_server_output_includes_app_and_local_services_only() {
		let request = sample_request();
		let digest = Digest::of(b"image");
		let output = build_server_output(&request, digest.clone(), 8080, Vec::new());
		assert_eq!(output.image.repository, request.image_repository);
		assert_eq!(output.image.digest, digest);
		let names: Vec<&str> = output
			.processes
			.iter()
			.map(|process| process.name.as_str())
			.collect();
		assert_eq!(names, vec!["app", "api"]);
		assert_eq!(output.processes[0].port, 8080);
		assert_eq!(output.processes[0].ready_path, "/readyz");
		assert_eq!(output.processes[0].live_path, "/healthz");
	}

	#[test]
	fn fallback_dockerfile_sets_non_root_user_and_port() {
		let dockerfile = render_fallback_dockerfile("stable", 9090);
		assert!(dockerfile.contains("FROM rust:1-bookworm AS builder"));
		assert!(dockerfile.contains("USER 65532:65532"));
		assert!(dockerfile.contains("EXPOSE 9090"));
		assert!(dockerfile.contains("--mount=type=cache,target=/usr/local/cargo/registry"));
	}
}
