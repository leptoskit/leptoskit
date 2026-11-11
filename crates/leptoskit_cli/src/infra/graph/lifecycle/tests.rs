use std::cell::RefCell;

use super::*;

pub(super) fn config() -> LeptoskitConfig {
	let mut config: LeptoskitConfig = toml::from_str(include_str!(
		"../../../../tests/fixtures/website-graph.toml"
	))
	.unwrap();
	let leptoskit_config::infra::InfraConfig::Graph(infra) = config.infra.as_mut().unwrap() else {
		panic!("graph");
	};
	infra.edge.as_mut().unwrap().ingress = leptoskit_config::deployment::Ingress::CloudflareTunnel;
	infra.edge.as_mut().unwrap().origin_firewall =
		leptoskit_config::deployment::OriginFirewall::Closed;
	config
		.services
		.get_mut("play-api")
		.unwrap()
		.deploy
		.as_mut()
		.unwrap()
		.ingress = Some(leptoskit_config::deployment::Ingress::CloudflareTunnel);
	config
}

fn fixture() -> (tempfile::TempDir, LeptoskitConfig, PathBuf) {
	let root = tempfile::tempdir().unwrap();
	let config = config();
	super::super::generate(root.path(), &config, false).unwrap();
	let dir = directory(root.path(), &config).unwrap();
	(root, config, dir)
}

#[derive(Default)]
struct Fake {
	calls: RefCell<Vec<String>>,
	fail: Option<&'static str>,
	mutate: Option<&'static str>,
	version: Option<&'static str>,
	workspace: Option<&'static str>,
	plan: Option<Value>,
}

impl Engine for Fake {
	fn invoke(&self, dir: &Path, args: &[&str], _: bool) -> Result<Vec<u8>> {
		self.calls.borrow_mut().push(args.join(" "));

		if self.fail == Some(args[0]) {
			anyhow::bail!("injected failure");
		}

		if self.mutate == Some(args[0]) {
			fs::write(dir.join("main.tf.json"), b"changed")?;
		}

		match args[0] {
			"version" => {
				Ok(serde_json::to_vec(
					&serde_json::json!({"terraform_version":self.version.unwrap_or("1.13.9"),"platform":"darwin_arm64"}),
				)?)
			}
			"workspace" => Ok(self.workspace.unwrap_or("default").as_bytes().to_vec()),
			"init" => {
				fs::write(dir.join(".terraform.lock.hcl"), b"locked providers")?;
				Ok(Vec::new())
			}
			"plan" => {
				fs::write(dir.join(PLAN), [0, 255, 128, 42])?;
				Ok(Vec::new())
			}
			"show" => {
				Ok(serde_json::to_vec(&self.plan.clone().unwrap_or_else(
					|| serde_json::json!({"format_version":"1.2","resource_changes":[]}),
				))?)
			}
			_ => Ok(Vec::new()),
		}
	}
}

fn saved(dir: &Path, config: &LeptoskitConfig) {
	plan(dir, config, &Fake::default(), || Ok(())).unwrap();
}

#[test]
fn saved_graph_plan_is_bound_and_consumed_once() {
	let (_root, config, dir) = fixture();
	saved(&dir, &config);
	let receipt: Receipt = serde_json::from_slice(&fs::read(dir.join(RECEIPT)).unwrap()).unwrap();
	assert_eq!(receipt.files.len(), 6);
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		assert_eq!(
			fs::metadata(dir.join(RECEIPT))
				.unwrap()
				.permissions()
				.mode()
				& 0o777,
			0o600
		);
		assert_eq!(
			fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
			0o700
		);
	}

	let engine = Fake::default();
	apply(&dir, &config, &engine, || Ok(())).unwrap();
	assert!(!dir.join(RECEIPT).exists());
	assert_eq!(
		engine.calls.borrow().last().unwrap(),
		"apply -input=false -lock=true leptoskit.plan"
	);
	assert!(apply(&dir, &config, &engine, || Ok(())).is_err());
	assert_eq!(
		engine
			.calls
			.borrow()
			.iter()
			.filter(|call| call.starts_with("apply "))
			.count(),
		1
	);
}

#[test]
fn changed_files_and_current_configuration_cannot_use_an_old_plan() {
	for name in [
		"main.tf.json",
		"graph.json",
		"edge-web.json",
		"edge-compiler.json",
		".terraform.lock.hcl",
		PLAN,
	] {
		let (_root, config, dir) = fixture();
		saved(&dir, &config);
		fs::write(dir.join(name), b"changed").unwrap();
		let engine = Fake::default();
		assert!(apply(&dir, &config, &engine, || Ok(())).is_err(), "{name}");
		assert_eq!(engine.calls.borrow().len(), 0);
	}
	let (_root, mut config, dir) = fixture();
	saved(&dir, &config);
	config
		.artifacts
		.get_mut("public-site")
		.unwrap()
		.command
		.push("--changed".into());
	assert!(
		apply(&dir, &config, &Fake::default(), || Ok(()))
			.unwrap_err()
			.to_string()
			.contains("configuration")
	);
}

#[test]
fn failed_replan_or_apply_cannot_leave_a_previous_plan_usable() {
	for stage in ["init", "validate", "plan", "show"] {
		let (_root, config, dir) = fixture();
		saved(&dir, &config);
		assert!(
			plan(
				&dir,
				&config,
				&Fake {
					fail: Some(stage),
					..Fake::default()
				},
				|| Ok(())
			)
			.is_err()
		);
		assert!(!dir.join(RECEIPT).exists(), "{stage}");
	}

	let (_root, config, dir) = fixture();
	saved(&dir, &config);
	assert!(
		apply(
			&dir,
			&config,
			&Fake {
				fail: Some("apply"),
				..Fake::default()
			},
			|| Ok(())
		)
		.is_err()
	);

	assert!(!dir.join(RECEIPT).exists());
}

#[test]
fn host_replacement_and_incomplete_plans_are_rejected() {
	for kind in ["hcloud_server", "digitalocean_droplet", "hcloud_network"] {
		let (_root, config, dir) = fixture();
		let engine = Fake {
			plan: Some(
				serde_json::json!({"format_version":"1.2","resource_changes":[{"type":kind,"change":{"actions":["create","delete"]}}]}),
			),
			..Fake::default()
		};
		assert!(
			plan(&dir, &config, &engine, || Ok(()))
				.unwrap_err()
				.to_string()
				.contains("delete or replace")
		);
		assert!(!dir.join(RECEIPT).exists());
	}
	for value in [
		serde_json::json!({"format_version":"2.0"}),
		serde_json::json!({"format_version":"1.2","complete":false}),
		serde_json::json!({"format_version":"1.2","errored":true}),
		serde_json::json!({"format_version":"1.2","resource_changes":[{"type":"hcloud_server","change":{"actions":["forget"]}}]}),
		serde_json::json!({"format_version":"1.2","resource_changes":[{"type":"hcloud_server","change":{"actions":[null]}}]}),
	] {
		let (_root, config, dir) = fixture();
		assert!(
			plan(
				&dir,
				&config,
				&Fake {
					plan: Some(value),
					..Fake::default()
				},
				|| Ok(())
			)
			.is_err()
		);
		assert!(!dir.join(RECEIPT).exists());
	}
}

#[test]
fn tool_or_workspace_changes_and_budget_failures_prevent_apply() {
	let (_root, config, dir) = fixture();
	saved(&dir, &config);
	for engine in [
		Fake {
			version: Some("1.14.0"),
			..Fake::default()
		},
		Fake {
			workspace: Some("production"),
			..Fake::default()
		},
	] {
		assert!(apply(&dir, &config, &engine, || Ok(())).is_err());
		assert!(
			!engine
				.calls
				.borrow()
				.iter()
				.any(|call| call.starts_with("apply "))
		);
	}
	let engine = Fake::default();
	assert!(apply(&dir, &config, &engine, || anyhow::bail!("over budget")).is_err());
	assert!(
		!engine
			.calls
			.borrow()
			.iter()
			.any(|call| call.starts_with("apply "))
	);
	assert!(plan(&dir, &config, &engine, || anyhow::bail!("over budget")).is_err());
	assert!(!dir.join(RECEIPT).exists());
}

#[test]
fn injected_inputs_and_mid_operation_changes_are_detected() {
	for name in [
		"extra.tf",
		"override.tofu.json",
		"secret.auto.tfvars",
		"secret.tofuvars.json",
	] {
		let (_root, config, dir) = fixture();
		saved(&dir, &config);
		fs::write(dir.join(name), b"{}").unwrap();
		assert!(
			apply(&dir, &config, &Fake::default(), || Ok(()))
				.unwrap_err()
				.to_string()
				.contains("unmanaged")
		);
	}

	for stage in ["plan", "show"] {
		let (_root, config, dir) = fixture();
		assert!(
			plan(
				&dir,
				&config,
				&Fake {
					mutate: Some(stage),
					..Fake::default()
				},
				|| Ok(())
			)
			.is_err()
		);
		assert!(!dir.join(RECEIPT).exists());
	}

	let (_root, config, dir) = fixture();
	saved(&dir, &config);
	let engine = Fake {
		mutate: Some("show"),
		..Fake::default()
	};

	assert!(apply(&dir, &config, &engine, || Ok(())).is_err());
	assert!(
		!engine
			.calls
			.borrow()
			.iter()
			.any(|call| call.starts_with("apply "))
	);
}

#[test]
fn backend_state_identity_and_symlink_inputs_are_bound() {
	let (_root, config, dir) = fixture();
	saved(&dir, &config);
	fs::create_dir(dir.join(".terraform")).unwrap();
	fs::write(dir.join(".terraform/terraform.tfstate"), b"changed backend").unwrap();
	assert!(apply(&dir, &config, &Fake::default(), || Ok(())).is_err());
	#[cfg(unix)]
	{
		let (_root, config, dir) = fixture();

		saved(&dir, &config);
		let outside = tempfile::NamedTempFile::new().unwrap();
		fs::remove_file(dir.join(PLAN)).unwrap();
		std::os::unix::fs::symlink(outside.path(), dir.join(PLAN)).unwrap();
		assert!(apply(&dir, &config, &Fake::default(), || Ok(())).is_err());
	}
}

#[test]
fn generated_inputs_are_checked_without_another_range_fetch() {
	let (root, mut config, dir) = fixture();
	let leptoskit_config::infra::InfraConfig::Graph(infra) = config.infra.as_mut().unwrap() else {
		panic!("graph");
	};
	infra.edge.as_mut().unwrap().ingress = leptoskit_config::deployment::Ingress::CloudflareHttps;
	infra.edge.as_mut().unwrap().origin_firewall =
		leptoskit_config::deployment::OriginFirewall::CloudflareOnly;
	config
		.services
		.get_mut("play-api")
		.unwrap()
		.deploy
		.as_mut()
		.unwrap()
		.ingress = Some(leptoskit_config::deployment::Ingress::CloudflareHttps);
	let plan = leptoskit_config::deployment::compile(&config)
		.unwrap()
		.unwrap();

	let ranges = super::super::network::Ranges::parse("173.245.48.0/20", "2400:cb00::/32").unwrap();

	for (name, contents) in super::super::rendered(&config, &plan, Some(&ranges)).unwrap() {
		fs::write(dir.join(name), contents).unwrap();
	}

	saved(&dir, &config);
	apply(&dir, &config, &Fake::default(), || Ok(())).unwrap();
	assert!(
		root.path()
			.join("infra/leptoskit-web/main.tf.json")
			.is_file()
	);
}

#[cfg(unix)]
#[test]
fn concurrent_operations_and_redirected_state_directories_are_refused() {
	let (root, config, dir) = fixture();
	let first = lock(&dir).unwrap();
	assert!(lock(&dir).is_err());
	drop(first);
	assert!(lock(&dir).is_ok());
	fs::remove_file(dir.join("operation.lock")).unwrap();
	let outside = tempfile::NamedTempFile::new().unwrap();
	std::os::unix::fs::symlink(outside.path(), dir.join("operation.lock")).unwrap();

	assert!(lock(&dir).is_err());
	fs::remove_dir_all(root.path().join("infra")).unwrap();
	let elsewhere = tempfile::tempdir().unwrap();
	std::os::unix::fs::symlink(elsewhere.path(), root.path().join("infra")).unwrap();
	assert!(directory(root.path(), &config).is_err());
}

#[test]
fn historical_recovery_checks_exact_retained_files_without_regenerating_them() {
	let (_root, config, dir) = fixture();
	let mut pins = inputs::generated(&dir, &config).unwrap();
	// A prior generator used different cloud-init. The independently pinned
	// historical plan binds those exact bytes, rather than today's rendering.
	let old = b"{\"historical_provider_fixture\":true}";
	fs::write(dir.join("main.tf.json"), old).unwrap();
	pins.insert(
		"main.tf.json".into(),
		hex::encode(<sha2::Sha256 as sha2::Digest>::digest(old)),
	);
	assert!(inputs::generated(&dir, &config).is_err());
	let expected = serde_json::to_value(&pins).unwrap();
	assert_eq!(inputs::historical(&dir, &config, &expected).unwrap(), pins);
	fs::write(dir.join("main.tf.json"), b"changed").unwrap();
	assert!(inputs::historical(&dir, &config, &expected).is_err());
	fs::write(dir.join("main.tf.json"), old).unwrap();
	fs::write(dir.join("extra.tf"), b"unmanaged").unwrap();
	assert!(inputs::historical(&dir, &config, &expected).is_err());
	fs::remove_file(dir.join("extra.tf")).unwrap();
	let mut missing = pins.clone();
	missing.remove("graph.json");
	assert!(inputs::historical(&dir, &config, &serde_json::to_value(missing).unwrap()).is_err());
	let mut extra = pins;
	extra.insert("../other.tf".into(), "a".repeat(64));
	assert!(inputs::historical(&dir, &config, &serde_json::to_value(extra).unwrap()).is_err());
}
