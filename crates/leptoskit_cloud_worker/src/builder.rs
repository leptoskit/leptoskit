//! Built-in OCI image builder (`leptoskit-cloud-worker build-app`).
//!
//! Ships a default build pipeline so a `leptoskit-cloud-worker` build worker
//! works out of the box without an operator-authored shell script:
//!
//! 1. Reads [`BuildRequest`] from `LEPTOSKIT_BUILD_INPUT`.
//! 2. Ensures a regular, non-symlink `Dockerfile` exists in
//!    `LEPTOSKIT_BUILD_SOURCE`, generating one via `lk generate deploy --target
//!    docker` (or a standard `LeptosKit` multi-stage fallback when `lk` is
//!    unavailable) if the repository doesn't commit its own.
//! 3. Builds and pushes the OCI image to `request.image_repository` using
//!    rootless `BuildKit` (`buildctl-daemonless.sh` / `buildctl`) or `docker
//!    buildx build --push`, importing persistent `BuildKit` cache layers only
//!    when `index.json` exists and rotating exports atomically on success.
//! 4. Extracts the immutable `sha256:…` digest from `BuildKit`'s bounded
//!    metadata file and writes [`BuildOutcome`] JSON to
//!    `LEPTOSKIT_BUILD_OUTPUT`.

use std::path::Path;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Instant;

use leptoskit_cloud::archive::Digest;
use leptoskit_cloud::deployments::BuildStep;
use leptoskit_cloud::workers::BuildOutcome;
use leptoskit_cloud::workers::BuildServerOutput;
use leptoskit_cloud::workers::BuiltImage;
use leptoskit_cloud::workers::ProcessFacts;
use leptoskit_cloud::workers::WebAssets;
use tokio::io::AsyncReadExt as _;

use crate::command::BuildRequest;

/// Default HTTP port aligned with `lk generate deploy --target docker`.
pub const DEFAULT_APP_PORT: u16 = 3000;

/// Maximum metadata file size accepted from `BuildKit` / `docker buildx`.
const MAX_METADATA_BYTES: u64 = 1024 * 1024;

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
	/// Port the app server listens on inside the container (default 3000).
	#[arg(
		long,
		env = "LEPTOSKIT_CLOUD_BUILDER_PORT",
		default_value_t = DEFAULT_APP_PORT
	)]
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

/// Resolved local `BuildKit` cache paths for one build invocation.
#[derive(Debug, Clone)]
struct CachePlan {
	live_dir: PathBuf,
	import_existing: bool,
	export_temp: PathBuf,
}

/// Validated inputs for a single container image build invocation.
struct BuildContext<'a> {
	source: &'a Path,
	dockerfile: PathBuf,
	push_tag: String,
	platform: Option<&'a str>,
	insecure_registry: bool,
	metadata_path: &'a Path,
	cache: Option<&'a CachePlan>,
}

async fn build_and_push(
	args: &BuildAppArgs,
	request: &BuildRequest,
	source: &Path,
	steps: &mut Vec<BuildStep>,
) -> Result<BuildServerOutput, String> {
	let push_tag = format!(
		"{}:d-{}",
		request.image_repository, request.deployment.number
	);
	validate_inputs(args, source, &push_tag)?;
	let dockerfile = ensure_dockerfile(args, request, source, steps).await?;
	let resolved = resolve_backend(args.backend)?;

	if matches!(resolved, ResolvedBackend::Docker) && args.insecure_registry {
		return Err(
			"--insecure-registry requires the buildctl backend; configure an insecure registry in \
			 Docker Buildx's buildkitd config instead"
				.to_owned(),
		);
	}

	let metadata_file = tempfile::Builder::new()
		.prefix("lk-build-meta-")
		.suffix(".json")
		.tempfile()
		.map_err(|error| format!("couldn't create build metadata temp file: {error}"))?;
	let cache = prepare_cache_plan(args.cache_dir.as_deref(), request.deployment.number).await?;

	let ctx = BuildContext {
		source,
		dockerfile,
		push_tag,
		platform: args.platform.as_deref(),
		insecure_registry: args.insecure_registry,
		metadata_path: metadata_file.path(),
		cache: cache.as_ref(),
	};

	let build_started = Instant::now();
	let status = invoke_backend(&resolved, &ctx).await?;
	steps.push(BuildStep {
		name: "container build & push".to_owned(),
		millis: elapsed_millis(build_started),
		cached_percent: None,
	});

	if !status.success() {
		cleanup_cache_export(cache.as_ref()).await;

		return Err(describe_exit_status(status));
	}

	rotate_cache_export(cache.as_ref()).await;
	let digest = read_metadata_digest(metadata_file.path()).await?;

	Ok(build_server_output(
		request,
		digest,
		args.port,
		steps.clone(),
	))
}

fn validate_inputs(args: &BuildAppArgs, source: &Path, push_tag: &str) -> Result<(), String> {
	if args.port == 0 {
		return Err("builder port must be non-zero".to_owned());
	}

	validate_csv_safe("image tag", push_tag)?;
	validate_csv_safe("source path", &source.to_string_lossy())?;

	if let Some(platform) = &args.platform {
		validate_platform(platform)?;
	}

	if let Some(cache_dir) = &args.cache_dir {
		validate_csv_safe("cache path", &cache_dir.to_string_lossy())?;
	}

	Ok(())
}

fn validate_csv_safe(label: &str, value: &str) -> Result<(), String> {
	if value.is_empty()
		|| value
			.chars()
			.any(|character| matches!(character, ',' | '\n' | '\r' | '\0'))
	{
		return Err(format!(
			"{label} contains forbidden characters (comma or control byte)"
		));
	}

	Ok(())
}

fn validate_platform(platform: &str) -> Result<(), String> {
	validate_csv_safe("platform", platform)?;
	let parts: Vec<&str> = platform.split('/').collect();
	let valid = (2..=3).contains(&parts.len())
		&& parts.iter().all(|part| {
			!part.is_empty()
				&& part
					.bytes()
					.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
		});

	if !valid {
		return Err(format!(
			"invalid OCI platform `{platform}` (expected `os/arch`, e.g. `linux/amd64`)"
		));
	}

	Ok(())
}

async fn prepare_cache_plan(
	cache_dir: Option<&Path>,
	deployment_number: u64,
) -> Result<Option<CachePlan>, String> {
	let Some(root) = cache_dir else {
		return Ok(None);
	};

	let live_dir = root.join("buildkit");
	let export_temp = root.join(format!(
		"buildkit-export-{}-{deployment_number}",
		std::process::id()
	));
	tokio::fs::create_dir_all(root)
		.await
		.map_err(|error| format!("couldn't prepare build cache directory: {error}"))?;
	let _ = tokio::fs::remove_dir_all(&export_temp).await;
	let import_existing = live_dir.join("index.json").is_file();

	Ok(Some(CachePlan {
		live_dir,
		import_existing,
		export_temp,
	}))
}

async fn cleanup_cache_export(cache: Option<&CachePlan>) {
	if let Some(plan) = cache {
		let _ = tokio::fs::remove_dir_all(&plan.export_temp).await;
	}
}

/// Promotes a freshly exported local `BuildKit` cache into `live_dir` with a
/// two-step rename (`live_dir -> backup`, `export_temp -> live_dir`) so a
/// failed or incomplete build never corrupts the last good cache and a failed
/// second rename rolls back to `backup`.
async fn rotate_cache_export(cache: Option<&CachePlan>) {
	let Some(plan) = cache else {
		return;
	};

	if !plan.export_temp.join("index.json").is_file() {
		let _ = tokio::fs::remove_dir_all(&plan.export_temp).await;

		return;
	}

	let backup = plan.live_dir.with_extension("old");
	let _ = tokio::fs::remove_dir_all(&backup).await;
	let had_live = plan.live_dir.exists();

	if had_live && tokio::fs::rename(&plan.live_dir, &backup).await.is_err() {
		let _ = tokio::fs::remove_dir_all(&plan.export_temp).await;

		return;
	}

	if tokio::fs::rename(&plan.export_temp, &plan.live_dir)
		.await
		.is_ok()
	{
		let _ = tokio::fs::remove_dir_all(&backup).await;
	} else if had_live {
		let _ = tokio::fs::rename(&backup, &plan.live_dir).await;
	}
}

async fn invoke_backend(
	backend: &ResolvedBackend,
	ctx: &BuildContext<'_>,
) -> Result<ExitStatus, String> {
	match backend {
		ResolvedBackend::Buildctl(program) => invoke_buildctl(program, ctx).await,
		ResolvedBackend::Docker => invoke_docker_buildx(ctx).await,
	}
}

async fn invoke_buildctl(program: &Path, ctx: &BuildContext<'_>) -> Result<ExitStatus, String> {
	let mut cmd = tokio::process::Command::new(program);
	let filename = ctx
		.dockerfile
		.file_name()
		.and_then(|name| name.to_str())
		.unwrap_or("Dockerfile");
	cmd.kill_on_drop(true)
		.arg("build")
		.arg("--frontend")
		.arg("dockerfile.v0")
		.arg("--local")
		.arg(format!("context={}", ctx.source.display()))
		.arg("--local")
		.arg(format!(
			"dockerfile={}",
			ctx.dockerfile.parent().unwrap_or(ctx.source).display()
		))
		.arg("--opt")
		.arg(format!("filename={filename}"))
		.arg("--metadata-file")
		.arg(ctx.metadata_path);

	if let Some(platform) = ctx.platform {
		cmd.arg("--opt").arg(format!("platform={platform}"));
	}

	let mut output_spec = format!("type=image,name={},push=true", ctx.push_tag);

	if ctx.insecure_registry {
		output_spec.push_str(",registry.insecure=true");
	}

	cmd.arg("--output").arg(output_spec);

	if let Some(cache) = ctx.cache {
		if cache.import_existing {
			cmd.arg("--import-cache")
				.arg(format!("type=local,src={}", cache.live_dir.display()));
		}

		cmd.arg("--export-cache").arg(format!(
			"type=local,dest={},mode=max",
			cache.export_temp.display()
		));
	}

	cmd.current_dir(ctx.source)
		.status()
		.await
		.map_err(|error| format!("couldn't run {}: {error}", program.display()))
}

async fn invoke_docker_buildx(ctx: &BuildContext<'_>) -> Result<ExitStatus, String> {
	let mut cmd = tokio::process::Command::new("docker");
	cmd.kill_on_drop(true)
		.arg("buildx")
		.arg("build")
		.arg("--push")
		.arg("--file")
		.arg(&ctx.dockerfile)
		.arg("--tag")
		.arg(&ctx.push_tag)
		.arg("--metadata-file")
		.arg(ctx.metadata_path);

	if let Some(platform) = ctx.platform {
		cmd.arg("--platform").arg(platform);
	}

	if let Some(cache) = ctx.cache {
		if cache.import_existing {
			cmd.arg("--cache-from")
				.arg(format!("type=local,src={}", cache.live_dir.display()));
		}

		cmd.arg("--cache-to").arg(format!(
			"type=local,dest={},mode=max",
			cache.export_temp.display()
		));
	}

	cmd.arg(ctx.source)
		.current_dir(ctx.source)
		.status()
		.await
		.map_err(|error| format!("couldn't run docker buildx: {error}"))
}

async fn read_metadata_digest(path: &Path) -> Result<Digest, String> {
	let file = tokio::fs::File::open(path)
		.await
		.map_err(|error| format!("container builder did not write metadata file: {error}"))?;
	let mut bytes = Vec::new();
	file.take(MAX_METADATA_BYTES + 1)
		.read_to_end(&mut bytes)
		.await
		.map_err(|error| format!("couldn't read build metadata file: {error}"))?;

	if bytes.len() as u64 > MAX_METADATA_BYTES {
		return Err("container builder metadata file exceeded 1 MiB limit".to_owned());
	}

	extract_digest_from_metadata(&bytes)
}

fn describe_exit_status(status: ExitStatus) -> String {
	match status.code() {
		Some(code) => format!("container build exited with status {code}"),
		None => "container build was terminated by a signal".to_owned(),
	}
}

fn elapsed_millis(started: Instant) -> u64 {
	u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

async fn ensure_dockerfile(
	args: &BuildAppArgs,
	request: &BuildRequest,
	source: &Path,
	steps: &mut Vec<BuildStep>,
) -> Result<PathBuf, String> {
	let existing = source.join("Dockerfile");

	if let Ok(meta) = tokio::fs::symlink_metadata(&existing).await {
		if meta.file_type().is_file() {
			return Ok(existing);
		}

		return Err(
			"repository Dockerfile must be a regular file, not a symlink or directory".to_owned(),
		);
	}

	let started = Instant::now();
	let generated = tokio::process::Command::new(&args.lk_bin)
		.kill_on_drop(true)
		.args(["generate", "deploy", "--target", "docker"])
		.current_dir(source)
		.status()
		.await;
	let millis = elapsed_millis(started);

	if let Ok(status) = generated
		&& status.success()
		&& tokio::fs::symlink_metadata(&existing)
			.await
			.is_ok_and(|meta| meta.file_type().is_file())
	{
		steps.push(BuildStep {
			name: "lk generate deploy --target docker".to_owned(),
			millis,
			cached_percent: None,
		});

		return Ok(existing);
	}

	let locked = source.join("Cargo.lock").is_file();
	let cache_scope = format!(
		"{}-{}",
		sanitize_cache_id(&request.deployment.organization),
		sanitize_cache_id(&request.deployment.project),
	);
	let fallback = render_fallback_dockerfile(&request.toolchain, args.port, locked, &cache_scope);
	tokio::fs::write(&existing, fallback)
		.await
		.map_err(|error| format!("couldn't write fallback Dockerfile: {error}"))?;
	steps.push(BuildStep {
		name: "generate default Dockerfile".to_owned(),
		millis,
		cached_percent: None,
	});

	Ok(existing)
}

fn sanitize_cache_id(raw: &str) -> String {
	let cleaned: String = raw
		.chars()
		.filter(|character| character.is_ascii_alphanumeric() || *character == '-')
		.take(32)
		.collect();

	if cleaned.is_empty() {
		"app".to_owned()
	} else {
		cleaned
	}
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
			find_on_path("buildctl-daemonless.sh")
				.or_else(|| find_on_path("buildctl"))
				.map(ResolvedBackend::Buildctl)
				.or_else(|| find_on_path("docker").map(|_| ResolvedBackend::Docker))
				.ok_or_else(|| {
					"no OCI image builder found on PATH (install rootless buildkit \
					 `buildctl-daemonless.sh` / `buildctl`, or `docker buildx`, or supply \
					 --build-command)"
						.to_owned()
				})
		}
	}
}

fn find_on_path(binary: &str) -> Option<PathBuf> {
	let path_var = std::env::var_os("PATH")?;

	std::env::split_paths(&path_var)
		.map(|dir| dir.join(binary))
		.find(|candidate| candidate.is_file())
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
			path: "/app/dist".to_owned(),
		}),
		migrations: Vec::new(),
		steps,
	}
}

fn builder_image_for_toolchain(toolchain: &str) -> String {
	let trimmed = toolchain.trim();

	if trimmed.is_empty() || trimmed == "stable" {
		return "rust:1-bookworm".to_owned();
	}

	if trimmed
		.bytes()
		.all(|byte| byte.is_ascii_digit() || byte == b'.')
		&& trimmed.contains('.')
	{
		return format!("rust:{trimmed}-bookworm");
	}

	"rustlang/rust:nightly-bookworm".to_owned()
}

/// Generates a minimal multi-stage Rust/LeptosKit Dockerfile when a project
/// does not commit its own and `lk generate deploy` is unavailable.
#[must_use]
pub fn render_fallback_dockerfile(
	toolchain: &str,
	port: u16,
	locked: bool,
	cache_scope: &str,
) -> String {
	let image_tag = builder_image_for_toolchain(toolchain);
	let lock_flag = if locked { " --locked" } else { "" };
	let scope = sanitize_cache_id(cache_scope);

	format!(
		"# syntax=docker/dockerfile:1\nFROM {image_tag} AS builder\nWORKDIR /src\nCOPY . .\nRUN \
		 --mount=type=cache,id=cargo-{scope},target=/usr/local/cargo/registry \
		 \\\n\t--mount=type=cache,id=target-{scope},target=/src/target \\\n\tBIN=$(cargo build \
		 --release{lock_flag} --message-format=json \\\n\t| sed -n \
		 's/.*\"executable\":\"\\([^\"]*\\)\".*/\\1/p' | tail -n 1) \\\n\t&& test -n \"$BIN\" && \
		 cp \"$BIN\" /usr/local/bin/server\n\nFROM debian:bookworm-slim\nRUN apt-get update && \
		 apt-get install -y --no-install-recommends ca-certificates \\\n\t&& rm -rf \
		 /var/lib/apt/lists/*\nCOPY --from=builder /usr/local/bin/server \
		 /usr/local/bin/server\nUSER 65532:65532\nENV PORT={port} LEPTOS_SITE_ADDR=0.0.0.0:{port} \
		 LEPTOS_SITE_ROOT=/app/dist\nEXPOSE {port}\nENTRYPOINT [\"/usr/local/bin/server\"]\n"
	)
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;

	use leptoskit_cloud::deployments::AppFacts;
	use leptoskit_cloud::deployments::ServiceFacts;
	use leptoskit_cloud::workers::DeploymentRef;

	use super::*;

	const CUSTOM_PORT: u16 = 8080;

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
		let output = build_server_output(&request, digest.clone(), DEFAULT_APP_PORT, Vec::new());

		assert_eq!(output.image.repository, request.image_repository);
		assert_eq!(output.image.digest, digest);

		let names: Vec<&str> = output
			.processes
			.iter()
			.map(|process| process.name.as_str())
			.collect();

		assert_eq!(names, vec!["app", "api"]);
		assert_eq!(output.processes[0].port, DEFAULT_APP_PORT);
		assert_eq!(output.processes[0].ready_path, "/readyz");
		assert_eq!(output.processes[0].live_path, "/healthz");
	}

	#[test]
	fn fallback_dockerfile_maps_toolchains_locks_and_scopes_cache_ids() {
		let stable = render_fallback_dockerfile("stable", DEFAULT_APP_PORT, true, "ana-notes");

		assert!(stable.contains("FROM rust:1-bookworm AS builder"));
		assert!(stable.contains("cargo build --release --locked --message-format=json"));
		assert!(stable.contains("id=cargo-ana-notes"));
		assert!(stable.contains("LEPTOS_SITE_ADDR=0.0.0.0:3000"));
		assert!(stable.contains("USER 65532:65532"));

		let pinned = render_fallback_dockerfile("1.86.0", CUSTOM_PORT, false, "org-app");
		assert!(pinned.contains("FROM rust:1.86.0-bookworm AS builder"));
		assert!(!pinned.contains("--locked"));

		let nightly = render_fallback_dockerfile("nightly-2026-10-21", DEFAULT_APP_PORT, true, "a");
		assert!(nightly.contains("FROM rustlang/rust:nightly-bookworm AS builder"));
	}

	#[test]
	fn validates_platforms_and_rejects_csv_injection() {
		assert!(validate_platform("linux/amd64").is_ok());
		assert!(validate_platform("linux/arm/v7").is_ok());
		assert!(validate_platform("linux/amd64,output=bad").is_err());
		assert!(validate_platform("linux").is_err());
		assert!(validate_csv_safe("tag", "repo:tag,push=false").is_err());
	}

	#[tokio::test]
	async fn cache_plan_imports_only_when_index_json_exists_and_rotates_atomically() {
		let dir = tempfile::tempdir().expect("tempdir");
		let initial = prepare_cache_plan(Some(dir.path()), 1)
			.await
			.expect("plan")
			.expect("some");
		assert!(!initial.import_existing);

		// Simulate BuildKit exporting cache to export_temp.
		tokio::fs::create_dir_all(&initial.export_temp)
			.await
			.expect("mkdir");
		tokio::fs::write(initial.export_temp.join("index.json"), b"{}")
			.await
			.expect("write");
		rotate_cache_export(Some(&initial)).await;

		assert!(dir.path().join("buildkit/index.json").is_file());
		assert!(!initial.export_temp.exists());

		let second = prepare_cache_plan(Some(dir.path()), 2)
			.await
			.expect("plan")
			.expect("some");
		assert!(second.import_existing);
	}

	#[tokio::test]
	async fn ensure_dockerfile_rejects_symlink() {
		let dir = tempfile::tempdir().expect("tempdir");
		let outside = dir.path().join("outside");
		tokio::fs::write(&outside, b"FROM scratch\n")
			.await
			.expect("write");
		#[cfg(unix)]
		std::os::unix::fs::symlink(&outside, dir.path().join("Dockerfile")).expect("symlink");

		let args = BuildAppArgs {
			input: dir.path().join("in.json"),
			output: dir.path().join("out.json"),
			source: Some(dir.path().to_path_buf()),
			cache_dir: None,
			backend: BuilderBackend::Auto,
			lk_bin: PathBuf::from("lk"),
			platform: None,
			insecure_registry: false,
			port: DEFAULT_APP_PORT,
		};
		let mut steps = Vec::new();
		let err = ensure_dockerfile(&args, &sample_request(), dir.path(), &mut steps)
			.await
			.expect_err("must reject symlink Dockerfile");
		assert!(err.contains("symlink"), "{err}");
	}
}
