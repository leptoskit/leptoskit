//! Resource-bounded compilation of a fixed Cargo harness in disposable
//! containers.
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use axum::Json;
use axum::Router;
use axum::extract::ConnectInfo;
use axum::extract::DefaultBodyLimit;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::http::Method;
use axum::http::StatusCode;
use axum::http::header::RETRY_AFTER;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Semaphore;
use tower_http::cors::CorsLayer;

/// All runtime configuration is read once at startup. Image tags resolve to
/// immutable IDs.
#[derive(Clone, Debug)]
pub struct Config {
	pub bind: SocketAddr,
	pub origins: Vec<HeaderValue>,
	pub image: String,
	pub socket: String,
	pub runtime: String,
	pub workers: usize,
	pub queue: usize,
	pub timeout: Duration,
	pub memory: usize,
	pub cpus: usize,
	pub pids: usize,
	pub scratch: usize,
	pub request_bytes: usize,
	pub output_bytes: usize,
	pub cache_entries: usize,
	pub requests_per_minute: usize,
	pub cloudflare_cidrs: Vec<ipnet::IpNet>,
	pub prewarm: bool,
}

impl Config {
	/// Parse configuration and reject zero limits before accepting requests.
	pub fn from_env() -> anyhow::Result<Self> {
		fn number(key: &str, fallback: usize) -> anyhow::Result<usize> {
			let n = env::var(key).map_or(Ok(fallback), |v| v.parse())?;
			anyhow::ensure!(n > 0, "{key} must be positive");
			Ok(n)
		}

		let workers = number("PLAY_WORKERS", std::thread::available_parallelism()?.get())?;
		Ok(Self {
			prewarm: env::var("PLAY_PREWARM")
				.unwrap_or_else(|_| "true".into())
				.parse()?,
			bind: env::var("PLAY_BIND")
				.unwrap_or_else(|_| "127.0.0.1:4702".into())
				.parse()?,
			origins: env::var("PLAY_ORIGINS")
				.unwrap_or_else(|_| {
					"https://leptoskit.dev,http://localhost:4701,http://127.0.0.1:4701".into()
				})
				.split(',')
				.map(str::parse)
				.collect::<Result<_, _>>()?,
			image: env::var("PLAY_IMAGE")
				.unwrap_or_else(|_| "leptoskit-playground-toolchain:local".into()),
			socket: env::var("PLAY_SOCKET").unwrap_or_else(|_| "/var/run/docker.sock".into()),
			runtime: env::var("PLAY_RUNTIME").unwrap_or_else(|_| "docker".into()),
			workers,
			queue: number("PLAY_QUEUE", workers)?,
			timeout: Duration::from_secs(number("PLAY_TIMEOUT_SECONDS", 30)? as u64),
			memory: number("PLAY_MEMORY_MIB", 1024)?,
			cpus: number("PLAY_CPUS", 1)?,
			pids: number("PLAY_PIDS", 128)?,
			scratch: number("PLAY_SCRATCH_MIB", 256)?,
			request_bytes: number("PLAY_REQUEST_BYTES", 65536)?,
			output_bytes: number("PLAY_OUTPUT_BYTES", 16 * 1024 * 1024)?,
			cache_entries: number("PLAY_CACHE_ENTRIES", 32)?,
			requests_per_minute: number("PLAY_REQUESTS_PER_MINUTE", 12)?,
			cloudflare_cidrs: env::var("PLAY_CLOUDFLARE_CIDRS")
				.unwrap_or_default()
				.split(',')
				.filter(|v| !v.is_empty())
				.map(str::parse)
				.collect::<Result<_, _>>()?,
		})
	}

	fn command(&self) -> Command {
		let mut cmd = Command::new(&self.runtime);
		cmd.env("DOCKER_HOST", format!("unix://{}", self.socket));
		cmd.env("CONTAINER_HOST", format!("unix://{}", self.socket));
		cmd.kill_on_drop(true);
		cmd
	}
}

#[derive(Deserialize)]
struct Request {
	source: String,
}

/// Rustc JSON is preserved; mapped locations refer only to the editable file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
	pub line: u64,
	pub column: u64,
	pub message: String,
	pub raw: serde_json::Value,
}

/// Successful artifacts are base64 Wasm and wasm-bindgen's JavaScript module.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Compilation {
	pub js: String,
	pub wasm: String,
	pub stderr: String,
	pub diagnostics: Vec<Diagnostic>,
	#[serde(default)]
	pub raw_diagnostics: Vec<serde_json::Value>,
}

/// Preserve compiler messages even when their spans belong to the harness.
pub fn raw_diagnostics(text: &str) -> Vec<serde_json::Value> {
	text.lines()
		.filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
		.map(|value| {
			value
				.get("message")
				.filter(|value| value.is_object())
				.unwrap_or(&value)
				.clone()
		})
		.filter(|value| value["message"].is_string())
		.collect()
}

/// Map both Cargo-wrapped and direct rustc JSON without subtracting wrapper
/// lines.
pub fn diagnostics(text: &str) -> Vec<Diagnostic> {
	raw_diagnostics(text)
		.into_iter()
		.flat_map(|raw| {
			let message = raw["message"].as_str().unwrap_or_default().to_owned();
			raw["spans"]
				.as_array()
				.into_iter()
				.flatten()
				.filter(|s| {
					s["is_primary"] == true
						&& s["file_name"]
							.as_str()
							.is_some_and(|name| name.ends_with("user.rs"))
				})
				.map(|span| {
					Diagnostic {
						line: span["line_start"].as_u64().unwrap_or(1),
						column: span["column_start"].as_u64().unwrap_or(1),
						message: message.clone(),
						raw: raw.clone(),
					}
				})
				.collect::<Vec<_>>()
		})
		.collect()
}

struct Service {
	config: Config,
	digest: String,
	workers: Semaphore,
	admission: Semaphore,
	starters: HashMap<String, Arc<Compilation>>,
	cache: Mutex<HashMap<String, Arc<Compilation>>>,
	// Bound this table too: rotating client addresses cannot exhaust host memory.
	rate: Mutex<HashMap<std::net::IpAddr, (Instant, usize)>>,
}

impl Service {
	fn remember(&self, key: String, value: &Compilation) {
		let capacity = self
			.config
			.cache_entries
			.saturating_sub(self.starters.len());

		if capacity == 0 {
			return;
		}

		let mut cache = self.cache.lock().expect("cache lock");

		if cache.len() >= capacity {
			cache.clear();
		}

		cache.insert(key, Arc::new(value.clone()));
	}

	fn rate_limit(&self, address: std::net::IpAddr) -> Result<(), &'static str> {
		let mut rates = self.rate.lock().expect("rate lock");
		rates.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));

		if rates.len() >= 16384 && !rates.contains_key(&address) {
			return Err("rate table full");
		}

		let (_, used) = rates.entry(address).or_insert((Instant::now(), 0));

		if *used >= self.config.requests_per_minute {
			return Err("rate limit");
		}

		*used += 1;
		Ok(())
	}
}

/// Resolve the image before serving: cache keys must never depend on a mutable
/// tag.
pub async fn router(config: Config) -> anyhow::Result<Router> {
	let output = config
		.command()
		.args(["image", "inspect", "--format", "{{.Id}}", &config.image])
		.output()
		.await?;
	anyhow::ensure!(
		output.status.success(),
		"toolchain image inspect failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	let image_id = String::from_utf8(output.stdout)?;
	let hash = image_id
		.trim()
		.strip_prefix("sha256:")
		.unwrap_or(image_id.trim());
	anyhow::ensure!(
		hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
		"invalid image ID"
	);
	let digest = format!("sha256:{hash}");
	let mut starters = HashMap::new();

	if config.prewarm {
		let starter = include_str!("../../leptoskit_app/web/play-counter.rs");

		for source in [
			starter.to_owned(),
			starter.replace(
				"<Text>\"A small idea starts here\"</Text>",
				"<TextInput placeholder=\"Your next idea\" />",
			),
			starter.replace(
				"<Text>\"A small idea starts here\"</Text>",
				"<View>{[\"Sketch\", \"Build\", \"Ship\"].map(|item| view! { <Text>{item}</Text> \
				 })}</View>",
			),
			starter.replace(
				"<Text>\"A small idea starts here\"</Text>",
				"<Text motion=Motion::enter(Pose::new().scale(0.3)).spring(Spring::BOUNCY)>\"\
				 Hello, motion\"</Text>",
			),
			starter.replace("\"p-8 gap-4 flex-col\"", "\"p-8 gap-4 flex-row flex-wrap\""),
		]
		.into_iter()
		.take(config.cache_entries)
		{
			let result = run(&config, &digest, &source).await?;
			anyhow::ensure!(
				!result.wasm.is_empty(),
				"starter prewarm failed; validate the toolchain image"
			);
			starters.insert(
				format!("{digest}:{:x}", Sha256::digest(source.as_bytes())),
				Arc::new(result),
			);
		}
	}

	let state = Arc::new(Service {
		workers: Semaphore::new(config.workers),
		admission: Semaphore::new(config.workers + config.queue),
		config,
		digest,
		starters,
		cache: Mutex::new(HashMap::new()),
		rate: Mutex::new(HashMap::new()),
	});
	Ok(routes(state))
}

fn routes(state: Arc<Service>) -> Router {
	let cors = CorsLayer::new()
		.allow_origin(state.config.origins.clone())
		.allow_methods([Method::POST, Method::GET])
		.allow_headers([axum::http::header::CONTENT_TYPE])
		.expose_headers([RETRY_AFTER]);
	let limit = state.config.request_bytes;
	Router::new()
		.route("/healthz", get(|| async { "ok" }))
		.route("/version", get(version))
		.route("/status", get(queue_status))
		.route("/compile", post(compile))
		.layer(DefaultBodyLimit::max(limit))
		.layer(cors)
		.with_state(state)
}

async fn version(State(s): State<Arc<Service>>) -> Json<serde_json::Value> {
	Json(serde_json::json!({
		"toolchain": "nightly-2026-10-21",
		"leptoskit": env!("CARGO_PKG_VERSION"),
		"image_digest": s.digest,
	}))
}

async fn queue_status(State(s): State<Arc<Service>>) -> Json<serde_json::Value> {
	let running = s.config.workers - s.workers.available_permits();
	let admitted = s.config.workers + s.config.queue - s.admission.available_permits();
	Json(serde_json::json!({
		"running": running,
		"queued": admitted.saturating_sub(running),
		"capacity": s.config.workers,
	}))
}

fn unavailable(status: StatusCode, message: &str) -> Response {
	(
		status,
		[(RETRY_AFTER, "5")],
		Json(serde_json::json!({"error":message})),
	)
		.into_response()
}

fn client_ip(
	peer: std::net::IpAddr,
	headers: &axum::http::HeaderMap,
	trusted: &[ipnet::IpNet],
) -> std::net::IpAddr {
	let peer = peer.to_canonical();

	if trusted.iter().any(|range| range.contains(&peer)) {
		return headers
			.get("cf-connecting-ip")
			.and_then(|v| v.to_str().ok())
			.and_then(|v| v.parse().ok())
			.unwrap_or(peer);
	}
	peer
}

async fn compile(
	State(s): State<Arc<Service>>,
	ConnectInfo(peer): ConnectInfo<SocketAddr>,
	headers: axum::http::HeaderMap,
	Json(request): Json<Request>,
) -> Response {
	let address = client_ip(peer.ip(), &headers, &s.config.cloudflare_cidrs);

	if let Err(message) = s.rate_limit(address) {
		return unavailable(StatusCode::TOO_MANY_REQUESTS, message);
	}

	let key = format!(
		"{}:{:x}",
		s.digest,
		Sha256::digest(request.source.as_bytes())
	);

	if let Some(hit) = s.starters.get(&key) {
		return Json(hit.as_ref().clone()).into_response();
	}

	if let Some(hit) = s.cache.lock().expect("cache lock").get(&key) {
		return Json(hit.as_ref().clone()).into_response();
	}

	let Ok(_admission) = s.admission.try_acquire() else {
		return unavailable(StatusCode::SERVICE_UNAVAILABLE, "compile queue full");
	};

	let Ok(_worker) = s.workers.acquire().await else {
		return unavailable(StatusCode::SERVICE_UNAVAILABLE, "service stopping");
	};
	let began = Instant::now();
	let result = run(&s.config, &s.digest, &request.source).await;
	// Only metadata, never user source or compiler diagnostics, enters host
	// logs.
	eprintln!(
		"{}",
		serde_json::json!({
			"event": "compile",
			"elapsed_ms": began.elapsed().as_millis(),
			"success": result.as_ref().is_ok_and(|value| !value.wasm.is_empty()),
		})
	);

	match result {
		Ok(value) => {
			if !value.wasm.is_empty() {
				s.remember(key, &value);
			}

			Json(value).into_response()
		}
		Err(error) => unavailable(StatusCode::UNPROCESSABLE_ENTITY, &error.to_string()),
	}
}

fn container_command(config: &Config, digest: &str, name: &str) -> Command {
	let mut command = config.command();
	command.args([
		"run",
		"--rm",
		"--log-driver=none",
		"--name",
		name,
		"--network",
		"none",
		"--read-only",
		"--cap-drop=ALL",
		"--security-opt=no-new-privileges",
		"--user",
		"65534:65534",
		"--memory",
		&format!("{}m", config.memory),
		"--memory-swap",
		&format!("{}m", config.memory),
		"--cpus",
		&config.cpus.to_string(),
		"--pids-limit",
		&config.pids.to_string(),
		"--tmpfs",
		&format!("/work:rw,nosuid,nodev,size={}m,mode=1777", config.scratch),
		"--tmpfs",
		"/tmp:rw,nosuid,nodev,noexec,size=16m",
		"--entrypoint",
		"/usr/bin/timeout",
		"-i",
		digest,
		"--signal=KILL",
		&config.timeout.as_secs_f64().to_string(),
		"playground-worker",
	]);
	command
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped());
	command
}

async fn bounded_output(
	stream: impl tokio::io::AsyncRead + Unpin,
	limit: usize,
	label: &str,
) -> anyhow::Result<Vec<u8>> {
	let mut bytes = Vec::new();
	stream
		.take(limit as u64 + 1)
		.read_to_end(&mut bytes)
		.await?;
	anyhow::ensure!(bytes.len() <= limit, "{label} output limit exceeded");
	Ok(bytes)
}

/// Enforce output and wall-clock limits while draining both pipes concurrently.
pub async fn run(config: &Config, digest: &str, source: &str) -> anyhow::Result<Compilation> {
	static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
	let name = format!(
		"lk-play-{}-{}-{}",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)?
			.as_nanos(),
		ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
	);
	let mut command = container_command(config, digest, &name);
	let mut child = command.spawn()?;
	let mut stdin = child.stdin.take().expect("piped stdin");
	let stdout = child.stdout.take().expect("piped stdout");
	let stderr = child.stderr.take().expect("piped stderr");
	let operation = async {
		let write = async {
			stdin.write_all(source.as_bytes()).await?;
			drop(stdin);
			Ok::<_, std::io::Error>(())
		};

		let ((), bytes, errors) = tokio::try_join!(
			async { write.await.map_err(anyhow::Error::from) },
			bounded_output(stdout, config.output_bytes, "artifact"),
			bounded_output(stderr, config.output_bytes, "diagnostic")
		)?;
		let status = child.wait().await?;
		let stderr = String::from_utf8_lossy(&errors).into_owned();

		if !status.success() {
			return Ok(Compilation {
				diagnostics: diagnostics(&stderr),
				stderr,
				..Default::default()
			});
		}

		Ok(serde_json::from_slice(&bytes)?)
	};
	let result = tokio::time::timeout(config.timeout, operation).await;
	// Stop the client before cleanup so it cannot continue starting a container
	// after the removal request. Successful exits are already reaped.

	if child.id().is_some() {
		let _ = child.kill().await;
		let _ = child.wait().await;
	}
	// A killed runtime client does not necessarily kill its container.
	let cleanup = config.command().args(["rm", "-f", &name]).output();
	let _ = tokio::time::timeout(Duration::from_secs(5), cleanup).await;
	result.map_err(|_| anyhow::anyhow!("compilation exceeded wall-clock limit"))?
}

#[cfg(test)]
mod tests {
	use tower::ServiceExt as _;

	use super::*;

	#[tokio::test]
	async fn oversized_source_is_rejected_before_queueing() {
		let mut config = Config::from_env().unwrap();
		config.request_bytes = 16;
		let state = Arc::new(Service {
			config,
			digest: "digest".into(),
			workers: Semaphore::new(0),
			admission: Semaphore::new(0),
			starters: HashMap::new(),
			cache: Mutex::new(HashMap::new()),
			rate: Mutex::new(HashMap::new()),
		});
		let mut request = axum::http::Request::builder()
			.method("POST")
			.uri("/compile")
			.header("content-type", "application/json")
			.body(axum::body::Body::from(
				r#"{"source":"this exceeds the configured limit"}"#,
			))
			.unwrap();
		request
			.extensions_mut()
			.insert(ConnectInfo("127.0.0.1:1234".parse::<SocketAddr>().unwrap()));
		let response = routes(state).oneshot(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
	}
	#[tokio::test]
	#[ignore = "requires Docker-compatible socket and the built toolchain image"]
	async fn container_compile_error_timeout_and_network_boundary() -> anyhow::Result<()> {
		let config = Config::from_env()?;
		let source = "use leptoskit::prelude::*; pub fn app() -> impl Fn() -> Element { || \
		              View().child(Text(\"Hello\")) }";
		let result = run(&config, &config.image, source).await?;
		assert!(!result.wasm.is_empty(), "{}", result.stderr);
		assert_ne!(result.js, "");
		let mut tiny = config.clone();
		tiny.output_bytes = 32;
		assert!(
			run(&tiny, &config.image, source)
				.await
				.unwrap_err()
				.to_string()
				.contains("output limit")
		);
		let error = run(&config, &config.image, "pub fn app() { missing_symbol(); }").await?;
		assert!(!error.diagnostics.is_empty(), "{}", error.stderr);
		assert_eq!(error.diagnostics[0].line, 1);
		let error = run(&config, &config.image, "pub fn app() {}").await?;
		assert_eq!(error.wasm, "");
		assert!(!error.raw_diagnostics.is_empty(), "{}", error.stderr);
		let mut short = config.clone();
		short.timeout = Duration::from_secs(2);
		let nonterminating = "#![allow(long_running_const_eval)] const _: () = loop {}; use \
		                      leptoskit::prelude::*; pub fn app() -> impl Fn() -> Element { || \
		                      View() }";
		assert!(
			run(&short, &config.image, nonterminating)
				.await
				.unwrap_err()
				.to_string()
				.contains("wall-clock")
		);
		let network = config
			.command()
			.args([
				"run",
				"--rm",
				"--network=none",
				"--read-only",
				"--cap-drop=ALL",
				"--security-opt=no-new-privileges",
				&config.image,
				"--probe-network",
			])
			.output()
			.await?;
		assert!(
			network.status.success(),
			"{}",
			String::from_utf8_lossy(&network.stderr)
		);
		assert!(String::from_utf8_lossy(&network.stdout).contains("network denied"));
		Ok(())
	}
	#[test]
	fn forwarding_requires_trusted_transport() {
		let mut headers = axum::http::HeaderMap::new();
		headers.insert("cf-connecting-ip", "192.0.2.7".parse().unwrap());
		let peer = "198.51.100.3".parse().unwrap();
		assert_eq!(client_ip(peer, &headers, &[]), peer);
		assert_eq!(
			client_ip(peer, &headers, &["198.51.100.0/24".parse().unwrap()]),
			"192.0.2.7".parse::<std::net::IpAddr>().unwrap()
		);
	}
	#[test]
	fn maps_only_primary_user_spans() {
		let input = r#"{"message":"wrong type","spans":[{"file_name":"/work/user.rs","line_start":7,"column_start":4,"is_primary":true},{"file_name":"main.rs","line_start":1,"is_primary":true}]}"#;
		let mapped = diagnostics(input);
		assert_eq!(mapped.len(), 1);
		assert_eq!((mapped[0].line, mapped[0].column), (7, 4));
	}
	#[test]
	fn preserves_harness_diagnostics_without_inventing_user_locations() {
		let input = r#"{"message":"wrong app return type","spans":[{"file_name":"/work/lib.rs","line_start":5,"column_start":1,"is_primary":true}]}"#;
		assert!(diagnostics(input).is_empty());
		let raw = raw_diagnostics(input);
		assert_eq!(raw.len(), 1);
		assert_eq!(raw[0]["message"], "wrong app return type");
		assert_eq!(raw[0]["spans"][0]["file_name"], "/work/lib.rs");
	}
	#[tokio::test]
	async fn cached_response_bypasses_full_queue_and_uncached_gets_retry_after() {
		let mut config = Config::from_env().unwrap();
		config.runtime = "must-not-start-a-runtime-for-a-cache-hit".into();
		config.cache_entries = 2;
		let key = format!("digest:{:x}", Sha256::digest(b"cached"));
		let starter_key = format!("digest:{:x}", Sha256::digest(b"starter"));
		let state = Arc::new(Service {
			config,
			digest: "digest".into(),
			workers: Semaphore::new(1),
			admission: Semaphore::new(1),
			starters: HashMap::from([(
				starter_key,
				Arc::new(Compilation {
					js: "starter JavaScript".into(),
					..Default::default()
				}),
			)]),
			cache: Mutex::new(HashMap::from([(
				key,
				Arc::new(Compilation {
					js: "cached JavaScript".into(),
					..Default::default()
				}),
			)])),
			rate: Mutex::new(HashMap::new()),
		});
		let _permit = state.admission.acquire().await.unwrap();
		let peer = ConnectInfo("127.0.0.1:1234".parse().unwrap());
		let cached = compile(
			State(state.clone()),
			peer,
			axum::http::HeaderMap::new(),
			Json(Request {
				source: "cached".into(),
			}),
		)
		.await;
		assert_eq!(cached.status(), StatusCode::OK);
		state.remember("replacement".into(), &Compilation::default());
		let starter = compile(
			State(state.clone()),
			peer,
			axum::http::HeaderMap::new(),
			Json(Request {
				source: "starter".into(),
			}),
		)
		.await;
		assert_eq!(starter.status(), StatusCode::OK);
		let missing = compile(
			State(state.clone()),
			peer,
			axum::http::HeaderMap::new(),
			Json(Request {
				source: "cached".into(),
			}),
		)
		.await;
		assert_eq!(missing.status(), StatusCode::SERVICE_UNAVAILABLE);
		assert_eq!(missing.headers()[RETRY_AFTER], "5");
	}
}
