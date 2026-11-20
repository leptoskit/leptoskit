//! One-click, idempotent cluster provisioning (`leptoskit-cloud-platform up`).
//!
//! Orchestrates the existing deterministic generators in strict dependency
//! order behind a testable [`CommandRunner`] boundary:
//!
//! 1. Generate Phase 1 (`tofu/` and `<region>/<cluster>/addons/`).
//! 2. Run `tofu init` + `tofu apply` and capture `tofu output -json` in memory
//!    (never writing sensitive outputs to disk) when cloud resources exist.
//! 3. Generate Phase 2 (`<region>/<cluster>/bootstrap/`) using the captured
//!    outputs.
//! 4. Bootstrap nodes over SSH in deterministic quorum order (first server →
//!    remaining servers → agents).
//! 5. Apply Kubernetes add-ons and platform workloads phase-by-phase with
//!    explicit readiness waits between phases: `00-namespaces` → `01-secrets` →
//!    `10-*` → `20-charts` (wait CRDs) → `21-http` (wait KEDA HTTP) → `30-*`
//!    (wait CNPG + wildcard cert) → `40-tunnel` → `50-control-plane` (wait
//!    migration + rollout) → `60-workers` (wait worker rollouts).

use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use crate::PlatformError;
use crate::config::PlatformConfig;
use crate::config::Role;

/// Options controlling `leptoskit-cloud-platform up`.
#[derive(Debug, Clone)]
#[expect(
	clippy::struct_excessive_bools,
	reason = "direct mirror of independent CLI skip and dry-run flags"
)]
pub struct UpOptions {
	/// Output directory for generated artifacts (uses ownership tracking).
	pub out: PathBuf,
	/// When true, records the exact commands without executing them.
	pub dry_run: bool,
	/// Skip `OpenTofu` init/apply (e.g. for existing bare-metal hosts).
	pub skip_tofu: bool,
	/// Skip SSH node bootstrap.
	pub skip_bootstrap: bool,
	/// Skip kubectl apply & readiness wait phases.
	pub skip_apply: bool,
	/// Optional kubeconfig path passed to every `kubectl` invocation.
	pub kubeconfig: Option<PathBuf>,
	/// SSH user for node bootstrap (default `root`).
	pub ssh_user: String,
}

/// Execution boundary for external CLI tools (`tofu`, `ssh`, `scp`, `kubectl`).
pub trait CommandRunner {
	/// Executes `program` with `args` (no shell interpolation) and returns
	/// stdout on success.
	///
	/// # Errors
	///
	/// Returns [`PlatformError::Invalid`] or [`PlatformError::Io`] on failure.
	fn run(
		&mut self,
		program: &str,
		args: &[String],
		cwd: Option<&Path>,
	) -> Result<String, PlatformError>;
}

/// Real process runner using [`std::process::Command`].
#[derive(Debug, Default)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
	fn run(
		&mut self,
		program: &str,
		args: &[String],
		cwd: Option<&Path>,
	) -> Result<String, PlatformError> {
		let mut cmd = Command::new(program);
		cmd.args(args);
		if let Some(dir) = cwd {
			cmd.current_dir(dir);
		}
		let output = cmd.output()?;
		if !output.status.success() {
			let stderr = String::from_utf8_lossy(&output.stderr);
			return Err(PlatformError::Invalid(format!(
				"`{program} {}` failed ({}): {}",
				args.join(" "),
				output.status,
				stderr.trim()
			)));
		}
		Ok(String::from_utf8_lossy(&output.stdout).into_owned())
	}
}

/// Recording dry-run runner that never spawns external processes.
#[derive(Debug, Default)]
pub struct DryRunRunner {
	/// Recorded `(program, args, cwd)` invocations in execution order.
	pub commands: Vec<(String, Vec<String>, Option<PathBuf>)>,
}

impl CommandRunner for DryRunRunner {
	fn run(
		&mut self,
		program: &str,
		args: &[String],
		cwd: Option<&Path>,
	) -> Result<String, PlatformError> {
		self.commands.push((
			program.to_owned(),
			args.to_vec(),
			cwd.map(Path::to_path_buf),
		));
		if program == "tofu" && args.first().map(String::as_str) == Some("output") {
			return Ok("{}".to_owned());
		}
		Ok(String::new())
	}
}

/// Strict phase ordering for `kubectl apply` and intervening readiness gates.
const APPLY_PHASES: &[&str] = &[
	"00-namespaces",
	"01-secrets",
	"10-runtime",
	"10-worker-rbac",
	"10-policies",
	"20-charts",
	"21-http",
	"30-certificates",
	"30-database",
	"30-registry",
	"40-tunnel",
	"50-control-plane",
	"60-workers",
];

/// Executes the full idempotent `up` workflow and returns a human-readable
/// summary transcript.
///
/// # Errors
///
/// Returns [`PlatformError`] if config validation, file generation, or any
/// external tool invocation fails.
pub fn execute(
	config: &PlatformConfig,
	options: &UpOptions,
	runner: &mut dyn CommandRunner,
) -> Result<String, PlatformError> {
	config.validate()?;
	let mut transcript = String::new();

	// 1. Phase-one generation: OpenTofu + Kubernetes manifests.
	let mut phase1 = crate::tofu::generate(config)?;
	let addon_files = crate::addons::generate(config)?;
	phase1.extend(addon_files.clone());
	crate::cli::write_generated_files(&options.out, "generate", &phase1)?;
	writeln!(transcript, "phase 1: generated {} files", phase1.len())?;

	// 2. OpenTofu apply + in-memory output capture.
	let tofu_dir = options.out.join("tofu");
	let has_cloud = config
		.regions
		.values()
		.flat_map(|r| r.clusters.values())
		.flat_map(|c| c.nodes.values())
		.any(crate::config::Node::cloud);

	let tofu_outputs = if !options.skip_tofu && tofu_dir.is_dir() {
		runner.run(
			"tofu",
			&["init".into(), "-input=false".into()],
			Some(&tofu_dir),
		)?;
		runner.run(
			"tofu",
			&[
				"apply".into(),
				"-auto-approve".into(),
				"-input=false".into(),
			],
			Some(&tofu_dir),
		)?;
		writeln!(transcript, "phase 2: OpenTofu applied")?;
		if has_cloud {
			let raw = runner.run("tofu", &["output".into(), "-json".into()], Some(&tofu_dir))?;
			Some(raw)
		} else {
			None
		}
	} else {
		writeln!(transcript, "phase 2: OpenTofu skipped")?;
		None
	};

	// 3. Phase-two generation: node bootstrap scripts & configs.
	let can_bootstrap_generate = !has_cloud || (tofu_outputs.is_some() && !options.dry_run);
	if can_bootstrap_generate {
		let bootstrap_files = crate::bootstrap::generate(config, tofu_outputs.as_deref())?;
		crate::cli::write_generated_files(&options.out, "bootstrap", &bootstrap_files)?;
		writeln!(
			transcript,
			"phase 3: generated {} bootstrap files",
			bootstrap_files.len()
		)?;
	} else {
		writeln!(
			transcript,
			"phase 3: bootstrap generation deferred until real OpenTofu outputs exist (dry-run)"
		)?;
	}

	// 4. SSH node bootstrap in strict quorum order (first server -> remaining
	// servers -> agents).
	if !options.skip_bootstrap && can_bootstrap_generate {
		for (region, entry) in &config.regions {
			for (cluster_name, cluster) in &entry.clusters {
				let machines = crate::topology::machines(cluster, None)?;
				let servers: Vec<_> = machines
					.iter()
					.filter(|m| m.node.role() == Role::Server)
					.collect();
				let agents: Vec<_> = machines
					.iter()
					.filter(|m| m.node.role() == Role::Agent)
					.collect();
				for machine in servers.into_iter().chain(agents) {
					let target_host = match &machine.node {
						crate::config::Node::Existing { public_address, .. } => {
							public_address.to_string()
						}
						crate::config::Node::HetznerCloud { .. } => machine.id.clone(),
					};
					let node_dir = options
						.out
						.join(region)
						.join(cluster_name)
						.join("bootstrap")
						.join(&machine.id);
					let dest = format!(
						"{}@{target_host}:/root/leptoskit-bootstrap",
						options.ssh_user
					);
					runner.run(
						"scp",
						&["-r".into(), node_dir.to_string_lossy().into_owned(), dest],
						None,
					)?;
					runner.run(
						"ssh",
						&[
							format!("{}@{target_host}", options.ssh_user),
							"bash /root/leptoskit-bootstrap/bootstrap.sh".into(),
						],
						None,
					)?;
				}
				writeln!(transcript, "phase 4: bootstrapped {region}/{cluster_name}")?;
			}
		}
	} else {
		writeln!(transcript, "phase 4: node bootstrap skipped")?;
	}

	// 5. Phase-ordered kubectl apply + readiness gates.
	if options.skip_apply {
		writeln!(transcript, "phase 5: kubectl apply skipped")?;
	} else {
		for (region, entry) in &config.regions {
			for cluster_name in entry.clusters.keys() {
				let prefix = format!("{region}/{cluster_name}/addons/");
				let cluster_files: Vec<&String> = addon_files
					.keys()
					.filter(|path| path.starts_with(&prefix))
					.collect();

				for &phase in APPLY_PHASES {
					let phase_prefix = format!("{prefix}{phase}-");
					let matching: Vec<PathBuf> = cluster_files
						.iter()
						.filter(|path| path.starts_with(&phase_prefix))
						.map(|path| options.out.join(path))
						.collect();
					if matching.is_empty() {
						continue;
					}

					for file in &matching {
						kubectl(
							runner,
							options.kubeconfig.as_deref(),
							&[
								"apply".into(),
								"-f".into(),
								file.to_string_lossy().into_owned(),
							],
						)?;
					}

					wait_after_phase(runner, options.kubeconfig.as_deref(), phase)?;
				}
				writeln!(
					transcript,
					"phase 5: applied add-ons and workloads for {region}/{cluster_name}"
				)?;
			}
		}
	}

	Ok(transcript)
}

fn kubectl(
	runner: &mut dyn CommandRunner,
	kubeconfig: Option<&Path>,
	args: &[String],
) -> Result<String, PlatformError> {
	let mut full = Vec::with_capacity(args.len() + 2);
	if let Some(kc) = kubeconfig {
		full.push("--kubeconfig".into());
		full.push(kc.to_string_lossy().into_owned());
	}
	full.extend(args.iter().cloned());
	runner.run("kubectl", &full, None)
}

fn wait_after_phase(
	runner: &mut dyn CommandRunner,
	kubeconfig: Option<&Path>,
	phase: &str,
) -> Result<(), PlatformError> {
	match phase {
		"20-charts" => {
			for crd in [
				"crd/clusters.postgresql.cnpg.io",
				"crd/scaledobjects.keda.sh",
				"crd/certificates.cert-manager.io",
			] {
				kubectl(
					runner,
					kubeconfig,
					&[
						"wait".into(),
						"--for=condition=Established".into(),
						"--timeout=300s".into(),
						crd.into(),
					],
				)?;
			}
		}
		"21-http" => {
			kubectl(
				runner,
				kubeconfig,
				&[
					"-n".into(),
					"keda".into(),
					"rollout".into(),
					"status".into(),
					"deployment/keda-add-ons-http-interceptor".into(),
					"--timeout=300s".into(),
				],
			)?;
		}
		"30-database" => {
			kubectl(
				runner,
				kubeconfig,
				&[
					"-n".into(),
					"database".into(),
					"wait".into(),
					"--for=condition=Ready".into(),
					"cluster/shared".into(),
					"--timeout=600s".into(),
				],
			)?;
		}
		"50-control-plane" => {
			kubectl(
				runner,
				kubeconfig,
				&[
					"-n".into(),
					"leptoskit-cloud-control".into(),
					"wait".into(),
					"--for=condition=complete".into(),
					"job/control-migrate".into(),
					"--timeout=300s".into(),
				],
			)?;
			kubectl(
				runner,
				kubeconfig,
				&[
					"-n".into(),
					"leptoskit-cloud-control".into(),
					"rollout".into(),
					"status".into(),
					"deployment/control".into(),
					"--timeout=300s".into(),
				],
			)?;
		}
		"60-workers" => {
			kubectl(
				runner,
				kubeconfig,
				&[
					"-n".into(),
					"kube-system".into(),
					"rollout".into(),
					"status".into(),
					"deployment/cluster-worker".into(),
					"--timeout=300s".into(),
				],
			)?;
		}
		_ => {}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn dry_run_orders_phases_and_waits_for_crds_before_http_and_db() {
		let config = PlatformConfig::parse(include_str!("../examples/launch.toml")).unwrap();
		let dir = tempfile::tempdir().unwrap();
		let mut runner = DryRunRunner::default();
		let options = UpOptions {
			out: dir.path().to_path_buf(),
			dry_run: true,
			skip_tofu: false,
			skip_bootstrap: false,
			skip_apply: false,
			kubeconfig: Some(PathBuf::from("/tmp/k3s.yaml")),
			ssh_user: "root".into(),
		};

		let transcript = execute(&config, &options, &mut runner).unwrap();
		assert!(transcript.contains("phase 1: generated"));
		assert!(transcript.contains("phase 5: applied add-ons"));

		let kubectl_args: Vec<String> = runner
			.commands
			.iter()
			.filter(|(program, ..)| program == "kubectl")
			.map(|(_, args, _)| args.join(" "))
			.collect();

		let pos = |needle: &str| {
			kubectl_args
				.iter()
				.position(|line| line.contains(needle))
				.unwrap_or_else(|| panic!("missing `{needle}` in {kubectl_args:?}"))
		};

		assert!(pos("00-namespaces-") < pos("01-secrets-"));
		assert!(pos("01-secrets-") < pos("10-worker-rbac-"));
		assert!(pos("10-worker-rbac-") < pos("20-charts-"));
		assert!(pos("20-charts-") < pos("crd/clusters.postgresql.cnpg.io"));
		assert!(pos("crd/clusters.postgresql.cnpg.io") < pos("21-http-"));
		assert!(pos("21-http-") < pos("keda-add-ons-http-interceptor"));
		assert!(pos("keda-add-ons-http-interceptor") < pos("30-database-"));
		assert!(pos("30-database-") < pos("cluster/shared"));
		assert!(pos("cluster/shared") < pos("40-tunnel-"));
	}

	#[test]
	fn skip_flags_omit_disabled_stages() {
		let config = PlatformConfig::parse(include_str!("../examples/launch.toml")).unwrap();
		let dir = tempfile::tempdir().unwrap();
		let mut runner = DryRunRunner::default();
		let options = UpOptions {
			out: dir.path().to_path_buf(),
			dry_run: true,
			skip_tofu: true,
			skip_bootstrap: true,
			skip_apply: true,
			kubeconfig: None,
			ssh_user: "root".into(),
		};
		let transcript = execute(&config, &options, &mut runner).unwrap();
		assert_eq!(runner.commands.len(), 0);
		assert!(transcript.contains("OpenTofu skipped"));
		assert!(transcript.contains("node bootstrap skipped"));
		assert!(transcript.contains("kubectl apply skipped"));
	}
}
