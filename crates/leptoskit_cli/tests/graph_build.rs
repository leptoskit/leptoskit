//! Real graph command boundaries; Cargo service acceptance is opt-in.
#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::Command;

use leptoskit_config::LeptoskitConfig;
use serde_json::Value;

fn project() -> (tempfile::TempDir, LeptoskitConfig) {
	let root = tempfile::tempdir().unwrap();
	let mut config: LeptoskitConfig =
		toml::from_str(include_str!("fixtures/website-graph.toml")).unwrap();
	let recipe = config.artifacts.get_mut("public-site").unwrap();
	recipe.output = "public-output".into();
	recipe.command = vec![
		"sh".into(),
		"-c".into(),
		"umask 022; printf x >> recipe-count; mkdir -p public-output/docs; printf docs > \
		 public-output/docs/index.html"
			.into(),
	];
	(root, config)
}

fn write_config(root: &Path, config: &LeptoskitConfig) {
	fs::write(
		root.join("leptoskit.toml"),
		toml::to_string(config).unwrap(),
	)
	.unwrap();
}

fn build(root: &Path, units: &[&str]) -> std::process::Output {
	let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit"));
	command
		.current_dir(root)
		.args(["build", "--out", "receipts"]);

	for unit in units {
		command.args(["--unit", unit]);
	}

	command.output().unwrap()
}

fn receipt(root: &Path) -> (std::path::PathBuf, Value) {
	let mut entries = fs::read_dir(root.join("receipts")).unwrap();
	let path = entries.next().unwrap().unwrap().path();
	assert!(entries.next().is_none());
	let manifest = serde_json::from_slice(&fs::read(path.join("manifest.json")).unwrap()).unwrap();
	(path, manifest)
}

#[test]
fn docs_build_needs_no_cargo_project_or_service_toolchain() {
	let (root, config) = project();
	write_config(root.path(), &config);
	let output = build(root.path(), &["frontends.docs"]);
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let (path, manifest) = receipt(root.path());
	assert_eq!(manifest["units"].as_object().unwrap().len(), 1);
	assert_eq!(manifest["evidence"]["source_reproduction"], "not_checked");
	assert_eq!(
		fs::read_to_string(root.path().join("recipe-count")).unwrap(),
		"x"
	);

	assert_eq!(
		fs::read_to_string(path.join("frontends.docs/assets/index.html")).unwrap(),
		"docs"
	);
	assert!(!path.join("services.play-api").exists());
	assert!(!root.path().join("Cargo.toml").exists());
}

#[test]
fn graph_receipt_verification_stages_exact_bytes_and_rejects_later_changes() {
	let (root, config) = project();
	write_config(root.path(), &config);
	assert!(build(root.path(), &["frontends.docs"]).status.success());
	let (path, _) = receipt(root.path());
	// The test retains this pin before substitution. A user must obtain it
	// independently, not trust a checksum delivered with an unknown bundle.
	let pin = fs::read_to_string(path.join("manifest.sha256")).unwrap();
	let output = tempfile::tempdir().unwrap();
	let verify = || {
		Command::new(env!("CARGO_BIN_EXE_leptoskit"))
			.current_dir(root.path())
			.args(["verify", "graph", "--bundle"])
			.arg(&path)
			.args(["--sha256", pin.trim(), "--unit", "frontends.docs", "--out"])
			.arg(output.path())
			.output()
			.unwrap()
	};
	let checked = verify();
	assert!(
		checked.status.success(),
		"{}",
		String::from_utf8_lossy(&checked.stderr)
	);
	let report: Value = serde_json::from_slice(&checked.stdout).unwrap();
	assert_eq!(report["evidence"]["artifact_integrity"], "checked");
	assert_eq!(report["evidence"]["source_reproduction"], "not_checked");
	let staged = Path::new(report["staged"].as_str().unwrap());
	assert_eq!(
		fs::read_to_string(staged.join("frontends.docs/assets/index.html")).unwrap(),
		"docs"
	);
	fs::write(path.join("frontends.docs/assets/index.html"), "replacement").unwrap();
	assert!(!verify().status.success());
	assert_eq!(fs::read_dir(output.path()).unwrap().count(), 1);
	assert_eq!(
		fs::read_to_string(root.path().join("recipe-count")).unwrap(),
		"x"
	);
}

#[test]
#[ignore = "builds and executes two real Cargo service artifacts"]
fn services_capture_independent_feature_sets_from_the_same_package() {
	let (root, mut config) = project();
	config.graph_infra_mut().unwrap().frontends.clear();
	config.artifacts.clear();
	let mut red = config.services.remove("play-api").unwrap();
	red.package = Some("graph_fixture".into());
	red.features = Some(vec!["red".into()]);
	red.runtime = None;
	let mut blue = red.clone();

	blue.features = Some(vec!["blue".into()]);
	blue.deploy.as_mut().unwrap().host = "web".into();
	blue.deploy.as_mut().unwrap().domain = Some("blue.example.com".into());
	config.services.insert("red".into(), red);
	config.services.insert("blue".into(), blue);
	write_config(root.path(), &config);
	fs::create_dir(root.path().join("src")).unwrap();
	fs::write(
		root.path().join("Cargo.toml"),
		r#"[package]
name = "graph_fixture"
version = "0.1.0"
edition = "2024"
[workspace]
[features]
red = []
blue = []
"#,
	)
	.unwrap();
	fs::write(
		root.path().join("src/main.rs"),
		r#"#[cfg(all(feature = "red", feature = "blue"))]
compile_error!("service features leaked");
fn main() {
    println!("{}", if cfg!(feature = "red") { "red" } else { "blue" });
}
"#,
	)
	.unwrap();
	assert!(
		Command::new("cargo")
			.current_dir(root.path())
			.args(["generate-lockfile", "--offline"])
			.status()
			.unwrap()
			.success()
	);
	let output = build(root.path(), &["services.red", "services.blue"]);
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let (path, manifest) = receipt(root.path());
	assert_eq!(manifest["units"].as_object().unwrap().len(), 2);

	for name in ["red", "blue"] {
		let output = Command::new(path.join(format!("services.{name}/backend")))
			.output()
			.unwrap();
		assert!(output.status.success());
		assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), name);
	}

	let cargo_path = root.path().join("Cargo.toml");
	let cargo = fs::read_to_string(&cargo_path).unwrap();
	for inherited in [
		"public = true",
		"public = true\nroutes = ['/legacy']",
		"bind = { LEGACY = 'app.url' }",
		"pool = 2",
		"volumes = ['legacy']",
	] {
		fs::write(
			&cargo_path,
			format!("{cargo}\n[package.metadata.leptoskit.service]\nname = \"red\"\n{inherited}\n"),
		)
		.unwrap();
		let output = build(root.path(), &["services.red"]);
		assert!(!output.status.success(), "accepted inherited {inherited}");
		assert!(
			String::from_utf8_lossy(&output.stderr).contains("graph service red"),
			"{}",
			String::from_utf8_lossy(&output.stderr)
		);
		assert_eq!(
			fs::read_dir(root.path().join("receipts")).unwrap().count(),
			1
		);
	}
}

#[test]
fn invalid_selection_and_failed_recipe_publish_no_receipt() {
	for unit in ["all", "frontends.missing", "frontends.docs"] {
		let (root, mut config) = project();
		config.artifacts.get_mut("public-site").unwrap().command =
			vec!["sh".into(), "-c".into(), "exit 17".into()];
		write_config(root.path(), &config);
		assert!(!build(root.path(), &[unit]).status.success());
		let receipts = root.path().join("receipts");
		assert!(!receipts.exists() || fs::read_dir(receipts).unwrap().next().is_none());
	}
}

#[test]
fn multiple_units_run_recipe_once_with_invoking_cli_and_profile() {
	let (root, mut config) = project();
	let recipe = config.artifacts.get_mut("public-site").unwrap();
	recipe.command[2].push_str(
		"; printf landing > public-output/index.html; printf '%s' \"$LEPTOSKIT_CLI\" > cli; \
		 printf '%s' \"$LEPTOSKIT_BUILD_PROFILE\" > profile",
	);
	write_config(root.path(), &config);
	let output = build(root.path(), &["frontends.docs", "frontends.site"]);
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	assert_eq!(
		fs::read_to_string(root.path().join("recipe-count")).unwrap(),
		"x"
	);
	assert_eq!(
		fs::read_to_string(root.path().join("profile")).unwrap(),
		"dev"
	);

	assert_eq!(
		fs::canonicalize(fs::read_to_string(root.path().join("cli")).unwrap()).unwrap(),
		fs::canonicalize(env!("CARGO_BIN_EXE_leptoskit")).unwrap()
	);
}
