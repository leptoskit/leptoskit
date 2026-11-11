//! Operator-directed graph releases. Plans are compared to freshly checked
//! inputs at every consumption; neither a plan nor its checksum grants
//! approval.
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command as Process;
use std::process::Stdio;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use base64::Engine as _;
use leptoskit_config::LeptoskitConfig;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use sha2::Digest as _;

mod contracts;
mod edge_runtime;
mod payload;
mod recovery;

#[derive(Debug, clap::Subcommand)]
pub enum Command {
	/// Save a release plan after checking independent artifact and host pins.
	#[command(disable_version_flag = true)]
	Plan(Args),
	/// Prepare candidates, recheck inputs, then activate each host.
	#[command(disable_version_flag = true)]
	Apply(Args),
	/// Read journals and current pointers without claiming runtime readiness.
	#[command(disable_version_flag = true)]
	Status(Args),
	/// Reactivate an installed release, rechecking its original inputs.
	#[command(disable_version_flag = true)]
	Rollback(Args),
}

#[derive(Debug, clap::Args)]
pub struct Args {
	/// Directory containing captured graph artifacts and manifest.json.
	#[arg(long)]
	bundle: PathBuf,
	/// Independently obtained graph manifest digest.
	#[arg(long)]
	sha256: String,
	/// Independently maintained host/runtime trust document.
	#[arg(long)]
	trust: PathBuf,
	/// Independently obtained SHA-256 of the host/runtime trust document.
	#[arg(long)]
	trust_sha256: String,
	/// Supplied Linux Caddy executable; required only for selected public
	/// hosts.
	#[arg(long, requires = "caddy_sha256")]
	caddy: Option<PathBuf>,
	/// Independently obtained SHA-256 of the Linux Caddy executable.
	#[arg(long, requires = "caddy")]
	caddy_sha256: Option<String>,
	/// Release identifier; rollback selects a previously installed version.
	#[arg(long)]
	version: String,
	/// Selected graph unit, such as frontends.docs; repeat for multiple units.
	#[arg(long, required = true)]
	unit: Vec<String>,
	/// Saved plan file. Apply/status/rollback compare it to current inputs.
	#[arg(long)]
	plan: PathBuf,
	/// Recover an existing operation using the independently obtained digest of
	/// its original saved plan. Retains historical operation/host identities.
	#[arg(long)]
	recover_plan_sha256: Option<String>,
	/// Graph approvals are not implemented; strict fails before mutation.
	#[arg(long, default_value = "operator", value_parser = ["operator", "strict"])]
	policy: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Trust {
	hosts: BTreeMap<String, TrustedHost>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TrustedHost {
	ipv4: std::net::Ipv4Addr,
	port: u16,
	/// Exact independently trusted ssh-ed25519 public key (type and base64).
	host_key: String,
	/// Absolute local private key path; credential bytes never enter the plan.
	identity: PathBuf,
	architecture: crate::verify::graph::linux::Architecture,
	/// Public certificate or tunnel configuration inputs; never private keys.
	prerequisites: BTreeMap<String, String>,
	/// Required for tunnel ingress; independently pinned systemd service.
	tunnel: Option<Tunnel>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Tunnel {
	unit: String,
	fragment: String,
	sha256: String,
}

fn hash(bytes: &[u8]) -> String {
	hex::encode(sha2::Sha256::digest(bytes))
}
fn encode(bytes: &[u8]) -> String {
	base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn input_root(path: &Path) -> Result<crate::verify::assets::Root> {
	let parent = path
		.parent()
		.filter(|parent| !parent.as_os_str().is_empty())
		.unwrap_or_else(|| Path::new("."));
	crate::verify::assets::Root::open(parent)
}

fn pinned(path: &Path, pin: &str, limit: u64) -> Result<Vec<u8>> {
	ensure!(
		crate::verify::digest_is_valid(pin),
		"expected independent lowercase SHA-256"
	);
	let root = input_root(path)?;
	let bytes = root.release_bytes(
		path.file_name()
			.and_then(|v| v.to_str())
			.context("invalid input name")?,
		limit,
	)?;
	ensure!(hash(&bytes) == pin, "input differs from independent pin");
	Ok(bytes)
}

/// Check intent and prepare all hosts before rechecking inputs for activation.
/// Historical recovery preserves journal identity but uses the current worker.
pub(crate) fn run(root: &Path, config: &LeptoskitConfig, command: &Command) -> Result<()> {
	let (Command::Plan(args)
	| Command::Apply(args)
	| Command::Status(args)
	| Command::Rollback(args)) = command;
	validate(args, command)?;
	let infra_settings = super::lifecycle::settings(config)?;
	ensure!(
		!root
			.join("infra")
			.join(&infra_settings.name)
			.join("backend-trust.json")
			.exists(),
		"backend-only trust cannot authorize graph fields; graph approval integration is pending"
	);
	let checked = crate::verify::graph::check(root, config, &args.bundle, &args.sha256)?;
	// Reject unsupported selected units and readiness prerequisites before
	// writes.
	let inventory = checked.release_inventory(&args.unit)?;
	check_selection(config, &inventory)?;
	let trust_bytes = pinned(&args.trust, &args.trust_sha256, 1024 * 1024)?;
	let trust: Trust = serde_json::from_slice(&trust_bytes)?;
	let caddy = edge_runtime::bytes(args)?;
	let dir = super::lifecycle::directory(root, config)?;
	let _lock = super::lifecycle::lock(&dir)?;
	let historical = recovery::load(args)?;
	let inputs = recovery::inputs(&dir, config, historical.as_ref())?;
	let hosts = bound_hosts(config, args, &inventory, &trust, &inputs, &dir, &checked)?;
	edge_runtime::check_usage(&hosts, caddy.as_deref())?;
	let plan = json!({"format_version":1,"kind":"graph-release","version":args.version,
	"installer_sha256":hash(worker().as_bytes()),
	"transport_sha256":hash(include_bytes!("release/transport.py")),
	"manifest":args.sha256,"trust":args.trust_sha256,"inputs":inputs,"hosts":hosts,
	"evidence":{
		"integrity":"checked", "authorization":"operator-directed",
		"source_reproduction":"not_checked", "runtime_attestation":"absent"
	}});

	if matches!(command, Command::Plan(_)) {
		save_plan(&args.plan, &plan)?;

		return Ok(());
	}

	let plan = match &historical {
		Some(original) => recovery::select(plan, original)?,
		None => plan,
	};

	let hosts = plan["hosts"].as_object().context("release hosts missing")?;
	let saved = recovery::saved(&args.plan, historical.as_ref())?;
	ensure!(
		saved == plan,
		"saved release plan differs from current configuration, selection, provider, trust or \
		 runtime inputs; plan again"
	);
	let operation = hash(&serde_json::to_vec(&plan)?);

	if matches!(command, Command::Status(_)) {
		for host in hosts.values() {
			transport(
				host,
				&json!({"action":"status","operation":operation,"version":args.version,
					"recovery":historical.is_some()}),
			)?;
		}

		return Ok(());
	}

	let staged = checked.stage_release(&dir, &args.unit)?;
	let request = json!({"operation":operation,"version":args.version,
		"rollback":matches!(command, Command::Rollback(_)), "recovery":historical.is_some()});
	prepare(&staged, hosts, &request, caddy.as_deref())?;
	staged.recheck()?;
	let (current_config, current_root) = LeptoskitConfig::load()?;
	ensure!(
		current_root.canonicalize()? == root.canonicalize()?
			&& serde_json::to_value(&current_config)? == serde_json::to_value(config)?,
		"configuration changed during preparation"
	);
	ensure!(
		pinned(&args.trust, &args.trust_sha256, 1024 * 1024)? == trust_bytes
			&& edge_runtime::bytes(args)? == caddy
			&& recovery::inputs(&dir, config, historical.as_ref())? == inputs,
		"release inputs changed during preparation; candidates remain inactive"
	);
	// Each host commits independently. Failure leaves explicit durable partial
	// state.
	for host in hosts.values() {
		staged.recheck()?;
		transport(
			host,
			&json!({"action":"activate","operation":operation,"version":args.version,
				"rollback":matches!(command, Command::Rollback(_)), "recovery":historical.is_some()}),
		)?;
	}

	Ok(())
}

fn check_selection(config: &LeptoskitConfig, inventory: &Value) -> Result<()> {
	let graph = leptoskit_config::deployment::compile(config)
		.map_err(anyhow::Error::msg)?
		.context("graph required")?;

	for name in inventory
		.as_object()
		.context("release inventory missing")?
		.keys()
	{
		ensure!(
			graph.placements()[&super::unit(name)?]
				.depends_on
				.is_empty(),
			"ordered dependency readiness observation is not implemented; release has not started"
		);
	}
	Ok(())
}

fn save_plan(path: &Path, plan: &Value) -> Result<()> {
	let mut file = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(path)?;
	serde_json::to_writer_pretty(&mut file, plan)?;
	file.sync_all()?;
	println!(
		"Saved graph release plan; independent approval and runtime acceptance remain \
		 unestablished."
	);
	Ok(())
}

fn prepare(
	staged: &crate::verify::graph::Staged<'_>,
	hosts: &serde_json::Map<String, Value>,
	context: &Value,
	caddy: Option<&[u8]>,
) -> Result<()> {
	ensure!(
		staged.readiness_only().is_empty(),
		"readiness observer required"
	);

	for host in hosts.values() {
		let runtime = edge_runtime::payload(host, caddy)?;
		let runtime_size = runtime.map_or(0, <[u8]>::len);
		let payload = staged.consume(|source| payload::read(source, host, runtime_size))?;
		let mut request = context.clone();
		request["action"] = json!("prepare");
		request["files"] = json!(payload.files);
		request["backends"] = json!(payload.backends);

		if let Some(bytes) = runtime {
			request["caddy"] = json!(encode(bytes));
		}

		transport(host, &request)?;
	}

	Ok(())
}

fn validate(args: &Args, command: &Command) -> Result<()> {
	ensure!(
		args.policy == "operator",
		"strict graph authorization/source reproduction evidence is unsupported; no mutation \
		 performed"
	);
	ensure!(
		!args.version.is_empty()
			&& args.version.len() <= 80
			&& args
				.version
				.bytes()
				.all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
		"release identifier must contain 1..80 letters, digits, hyphens or underscores"
	);
	ensure!(
		args.recover_plan_sha256.is_none() || !matches!(command, Command::Plan(_)),
		"recovery uses an existing independently pinned plan, not plan generation"
	);
	Ok(())
}

fn bound_hosts(
	config: &LeptoskitConfig,
	args: &Args,
	inventory: &Value,
	trust: &Trust,
	inputs: &Value,
	dir: &Path,
	checked: &crate::verify::graph::Checked,
) -> Result<serde_json::Map<String, Value>> {
	let graph = leptoskit_config::deployment::compile(config)
		.map_err(anyhow::Error::msg)?
		.context("graph required")?;
	let infra = super::lifecycle::settings(config)?;
	let install = format!("/var/lib/leptoskit/{}", infra.name);
	let mut hosts = serde_json::Map::new();

	for name in inventory.as_object().context("invalid inventory")?.keys() {
		let unit = super::unit(name)?;
		let host_name = &graph.placements()[&unit].host;

		if hosts.contains_key(host_name) {
			continue;
		}

		let host = trust
			.hosts
			.get(host_name)
			.context("selected host requires independent SSH trust")?;
		ensure!(
			host.port > 0 && host.identity.is_absolute(),
			"SSH port/identity must be explicit"
		);
		let parts: Vec<_> = host.host_key.split_whitespace().collect();
		ensure!(
			parts.len() == 2
				&& parts[0] == "ssh-ed25519"
				&& base64::engine::general_purpose::STANDARD
					.decode(parts[1])
					.is_ok(),
			"host trust requires one exact ed25519 public key"
		);
		ensure!(
			inputs["outputs"]["hosts"][host_name]["ipv4"] == host.ipv4.to_string(),
			"trusted endpoint differs from typed provider output"
		);
		let public = graph
			.routes()
			.entries()
			.iter()
			.any(|route| route.host == *host_name);
		let edge = edge_runtime::bind(
			dir,
			host_name,
			public,
			args,
			host,
			infra.edge.as_ref().map(|edge| edge.ingress),
		)?;
		let units: BTreeMap<_, _> = inventory
			.as_object()
			.context("inventory missing")?
			.iter()
			.filter(|(name, _)| {
				super::unit(name).is_ok_and(|u| graph.placements()[&u].host == *host_name)
			})
			.map(|(k, v)| (k.clone(), v.clone()))
			.collect();
		let bound = contracts::bind(config, checked, &units, host.architecture)?;
		ensure!(
			public || units.keys().eq(bound.executables.keys()),
			"private hosts require an executable for every selected unit"
		);
		let routes: BTreeMap<_, _> = graph
			.routes()
			.entries()
			.iter()
			.filter(|route| units.contains_key(&route.unit.to_string()))
			.map(|route| {
				(
					route.unit.to_string(),
					json!({"domain":route.domain,"mount":route.mount}),
				)
			})
			.collect();
		hosts.insert(
			host_name.clone(),
			json!({"trust":host,"edge":edge.config,"edge_sha256":edge.sha256,
			"units":units, "routes":routes, "executables":bound.executables,
			"linux_formats":bound.formats, "root":install,
			"caddy_sha256":if public { args.caddy_sha256.as_deref() } else { None },
			"caddy_version":public.then_some(super::edge::CADDY_VERSION)}),
		);
	}

	Ok(hosts)
}

fn worker() -> String {
	format!(
		"{}\n{}\n{}",
		include_str!("release/process.py"),
		include_str!("release/instances.py"),
		include_str!("release/host.py")
	)
}

fn transport(host: &Value, action: &Value) -> Result<()> {
	let mut script = tempfile::NamedTempFile::new()?;
	script.write_all(include_bytes!("release/transport.py"))?;
	let request = json!({"host":host,"request":action,"worker":worker()});
	let mut child = Process::new("python3")
		.arg(script.path())
		.stdin(Stdio::piped())
		.stdout(Stdio::inherit())
		.stderr(Stdio::null())
		.spawn()
		.context("Python 3 is required for bounded SSH transport")?;
	serde_json::to_writer(
		child.stdin.take().context("transport stdin missing")?,
		&request,
	)?;
	ensure!(
		child.wait()?.success(),
		"graph SSH operation failed or timed out; inspect release status before retrying; no \
		 local successful history was recorded"
	);
	Ok(())
}

#[cfg(all(test, unix))]
mod tests {
	#[test]
	fn pinned_inputs_accept_a_filename_in_the_current_directory() {
		use std::io::Write as _;
		let mut file = tempfile::NamedTempFile::new_in(".").unwrap();
		file.write_all(b"pinned public input").unwrap();
		let relative = std::path::Path::new(file.path().file_name().unwrap());
		let pin = super::hash(b"pinned public input");
		assert_eq!(
			super::pinned(relative, &pin, 1024).unwrap(),
			b"pinned public input"
		);
		assert!(super::pinned(relative, &"0".repeat(64), 1024).is_err());
	}

	#[test]
	fn static_release_host_and_transport_contracts() {
		let tests =
			std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/infra/graph/release");
		let status = std::process::Command::new("python3")
			.args(["-m", "unittest", "discover", "-s"])
			.arg(tests)
			.args(["-p", "test_*.py", "-v"])
			.status()
			.expect("Python 3 is required for graph release contract tests");
		assert!(
			status.success(),
			"graph release Python contract tests failed"
		);
	}
}
