#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use serde_json::Value;

use super::*;

fn fixture() -> (tempfile::TempDir, LeptoskitConfig) {
	let root = tempfile::tempdir().unwrap();
	let mut config: LeptoskitConfig = toml::from_str(include_str!(
		"../../../../../tests/fixtures/website-graph.toml"
	))
	.unwrap();
	let recipe = config.artifacts.get_mut("public-site").unwrap();
	recipe.output = "out".into();
	recipe.command = vec![
		"sh".into(),
		"-c".into(),
		"umask 022; test -x \"$LEPTOSKIT_CLI\" || exit 9; printf x >> recipe-count; mkdir -p \
		 out/docs/pkg out/play; printf landing > out/index.html; printf docs > \
		 out/docs/index.html; printf wasm > out/docs/pkg/app.wasm; printf playground > \
		 out/play/index.html"
			.into(),
	];
	(root, config)
}

fn build_frontends(root: &Path, config: &LeptoskitConfig, units: &[&str]) -> Result<PathBuf> {
	let selected = selection(
		config,
		&units
			.iter()
			.map(|value| (*value).into())
			.collect::<Vec<_>>(),
	)?;
	execute(root, config, &selected, true, Path::new("receipts"), |_| {
		panic!("unselected service was built")
	})
}

fn manifest(path: &Path) -> Value {
	let bytes = fs::read(path.join("manifest.json")).unwrap();
	assert_eq!(
		fs::read_to_string(path.join("manifest.sha256"))
			.unwrap()
			.trim(),
		hash(&bytes)
	);
	serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn one_actual_recipe_builds_three_disjoint_frontend_captures() {
	let (root, config) = fixture();
	let output = build_frontends(
		root.path(),
		&config,
		&["frontends.site", "frontends.docs", "frontends.play"],
	)
	.unwrap();
	assert_eq!(
		fs::read_to_string(root.path().join("recipe-count")).unwrap(),
		"x"
	);
	let manifest = manifest(&output);
	assert_eq!(
		fs::metadata(&output).unwrap().permissions().mode() & 0o777,
		0o700
	);
	assert_eq!(manifest["units"].as_object().unwrap().len(), 3);
	assert_eq!(manifest["recipes"].as_object().unwrap().len(), 1);
	assert_eq!(manifest["evidence"]["source_reproduction"], "not_checked");
	assert!(!output.join("frontends.site/assets/docs").exists());

	assert!(!output.join("frontends.site/assets/play").exists());
	assert_eq!(
		fs::read_to_string(output.join("frontends.docs/assets/index.html")).unwrap(),
		"docs"
	);
	assert!(output.join("frontends.docs/assets/pkg/app.wasm").is_file());
	assert!(!output.join("services.play-api").exists());
}

#[test]
fn docs_selection_does_not_build_services_or_publish_other_frontends() {
	let (root, config) = fixture();
	fs::write(root.path().join("service-state"), "unchanged").unwrap();
	let output = build_frontends(root.path(), &config, &["frontends.docs"]).unwrap();
	assert_eq!(
		manifest(&output)["units"]
			.as_object()
			.unwrap()
			.keys()
			.map(String::as_str)
			.collect::<Vec<_>>(),
		["frontends.docs"]
	);
	assert_eq!(
		fs::read_to_string(root.path().join("service-state")).unwrap(),
		"unchanged"
	);
	assert!(!output.join("frontends.site").exists());
}

#[test]
fn recipe_failure_does_not_publish_a_successful_capture() {
	let (root, mut config) = fixture();
	config.artifacts.get_mut("public-site").unwrap().command[2] = "exit 7".into();
	assert!(build_frontends(root.path(), &config, &["frontends.docs"]).is_err());
	assert_eq!(
		fs::read_dir(root.path().join("receipts")).unwrap().count(),
		0
	);
}

#[test]
fn invalid_selection_and_overlapping_receipts_fail_before_the_recipe() {
	let (root, config) = fixture();
	for names in [
		vec!["frontends.unknown"],
		vec!["frontends.docs", "frontends.docs"],
	] {
		assert!(build_frontends(root.path(), &config, &names).is_err());
	}
	let selected = selection(&config, &["frontends.docs".into()]).unwrap();
	assert!(
		execute(
			root.path(),
			&config,
			&selected,
			true,
			Path::new("out/captures"),
			|_| unreachable!()
		)
		.is_err()
	);
	assert!(!root.path().join("recipe-count").exists());
}

#[test]
fn missing_or_mutable_toolchain_pin_fails_before_recipes() {
	let (root, config) = fixture();
	let selected = selection(&config, &[]).unwrap();
	assert!(
		execute(
			root.path(),
			&config,
			&selected,
			true,
			Path::new("receipts"),
			|_| unreachable!()
		)
		.is_err()
	);
	fs::create_dir(root.path().join("deploy")).unwrap();
	fs::write(
		root.path().join("deploy/playground-toolchain.digest"),
		"registry/toolchain:latest",
	)
	.unwrap();
	assert!(
		execute(
			root.path(),
			&config,
			&selected,
			true,
			Path::new("receipts"),
			|_| unreachable!()
		)
		.is_err()
	);
	assert!(!root.path().join("recipe-count").exists());
}

#[test]
fn ssr_executable_is_captured_outside_its_public_asset_tree() {
	let (root, mut config) = fixture();
	let site = config
		.graph_infra_mut()
		.unwrap()
		.frontends
		.get_mut("site")
		.unwrap();
	site.kind = FrontendKind::Ssr;
	site.backend = Some("server".into());
	site.serve = Some(leptoskit_config::ServeConfig {
		probes: true,
		..Default::default()
	});
	config.artifacts.get_mut("public-site").unwrap().command[2]
		.push_str("; printf executable > out/server; chmod 755 out/server");
	let output = build_frontends(root.path(), &config, &["frontends.site"]).unwrap();
	assert_eq!(
		fs::read_to_string(output.join("frontends.site/backend")).unwrap(),
		"executable"
	);

	assert!(!output.join("frontends.site/assets/server").exists());
	assert_eq!(
		manifest(&output)["units"]["frontends.site"]["backend"]["mode"],
		0o755
	);
}

#[test]
fn service_capture_binds_its_executable_and_exact_toolchain_pin() {
	let (root, config) = fixture();
	fs::create_dir(root.path().join("deploy")).unwrap();
	let pin = format!("registry/toolchain@sha256:{}", "a".repeat(64));
	fs::write(root.path().join("deploy/playground-toolchain.digest"), &pin).unwrap();
	let binary = root.path().join("binary");
	fs::write(&binary, "service executable fixture").unwrap();
	fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
	let selected = selection(&config, &["services.play-api".into()]).unwrap();

	let mut calls = 0;
	let output = execute(
		root.path(),
		&config,
		&selected,
		true,
		Path::new("receipts"),
		|name| {
			assert_eq!(name, "play-api");
			calls += 1;
			service_build(root.path(), &config, &binary)
		},
	)
	.unwrap();
	assert_eq!(calls, 1);
	assert_eq!(
		manifest(&output)["units"]["services.play-api"]["toolchain_image"],
		pin
	);
	assert!(!output.join("services.play-api/assets").exists());
	assert!(!root.path().join("recipe-count").exists());
}

#[test]
fn changed_pin_during_service_build_prevents_receipt_publication() {
	let (root, config) = fixture();
	fs::create_dir(root.path().join("deploy")).unwrap();
	let pin_path = root.path().join("deploy/playground-toolchain.digest");
	fs::write(&pin_path, format!("image@sha256:{}", "a".repeat(64))).unwrap();
	let binary = root.path().join("binary");
	fs::write(&binary, "executable").unwrap();
	fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
	let selected = selection(&config, &["services.play-api".into()]).unwrap();

	assert!(
		execute(
			root.path(),
			&config,
			&selected,
			true,
			Path::new("receipts"),
			|_| {
				fs::write(&pin_path, format!("image@sha256:{}", "b".repeat(64)))?;
				service_build(root.path(), &config, &binary)
			}
		)
		.is_err()
	);
	assert_eq!(
		fs::read_dir(root.path().join("receipts")).unwrap().count(),
		0
	);
}

#[test]
fn preexisting_output_symlink_is_refused_before_running_the_recipe() {
	let (root, config) = fixture();
	let external = tempfile::tempdir().unwrap();
	std::os::unix::fs::symlink(external.path(), root.path().join("out")).unwrap();
	assert!(build_frontends(root.path(), &config, &["frontends.docs"]).is_err());
	assert!(!root.path().join("recipe-count").exists());
	assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
}

#[test]
fn server_only_ssr_captures_an_existing_empty_asset_tree() {
	let (root, mut config) = fixture();
	let site = config
		.graph_infra_mut()
		.unwrap()
		.frontends
		.get_mut("site")
		.unwrap();
	site.kind = FrontendKind::Ssr;
	site.backend = Some("server".into());
	site.serve = Some(leptoskit_config::ServeConfig {
		probes: true,
		..Default::default()
	});
	config.artifacts.get_mut("public-site").unwrap().command[2] =
		"umask 022; mkdir out; printf server > out/server; chmod 755 out/server".into();
	let output = build_frontends(root.path(), &config, &["frontends.site"]).unwrap();
	assert!(output.join("frontends.site/assets").is_dir());

	assert_eq!(
		manifest(&output)["units"]["frontends.site"]["assets"],
		serde_json::json!({})
	);
}

#[test]
fn unselected_service_values_are_hashed_instead_of_written_into_frontend_receipts() {
	let (root, mut config) = fixture();
	config
		.services
		.get_mut("play-api")
		.unwrap()
		.env
		.insert("PRIVATE_VALUE".into(), "keep-this-private".into());
	let output = build_frontends(root.path(), &config, &["frontends.docs"]).unwrap();
	let bytes = fs::read_to_string(output.join("manifest.json")).unwrap();
	assert!(!bytes.contains("keep-this-private"));
	assert!(!bytes.contains("PRIVATE_VALUE"));
	assert!(!bytes.contains(root.path().to_str().unwrap()));
}

fn service_build(
	root: &Path,
	config: &LeptoskitConfig,
	binary: &Path,
) -> Result<super::super::services::Built> {
	let canonical = root.canonicalize()?;
	let root = canonical.as_path();
	let manifest = root.join("Cargo.toml");
	fs::write(
		&manifest,
		"[package]\nname = \"leptoskit_playground_server\"\nversion = \"1.0.0\"\n",
	)?;
	fs::write(
		root.join("Cargo.lock"),
		"version = 3\n[[package]]\nname = \"leptoskit_playground_server\"\nversion = \"1.0.0\"\n",
	)?;
	let metadata: crate::wasm::CargoMetadata = serde_json::from_value(serde_json::json!({
		"workspace_root": root, "target_directory": root.join("target"),
		"packages": [{"name": "leptoskit_playground_server", "manifest_path": manifest, "targets": []}]
	}))?;
	let service = crate::services::Service {
		settings: leptoskit_config::ResolvedService::merge(
			"play-api",
			&config.services["play-api"],
			None,
		)
		.map_err(anyhow::Error::msg)?,
		code: crate::services::Code::Workspace {
			package: "leptoskit_playground_server".into(),
			manifest,
		},
	};
	Ok(super::super::services::Built {
		binary: binary.to_owned(),
		inputs: super::super::services::inputs::Inputs::prepare(root, &metadata, &service, true)?,
	})
}

#[test]
fn service_evidence_rechecks_original_and_copied_inputs_and_independent_pin() {
	let (root, mut config) = fixture();
	config
		.services
		.get_mut("play-api")
		.unwrap()
		.env
		.insert("LABEL".into(), "public 世界 %n $HOME".into());
	fs::create_dir(root.path().join("deploy")).unwrap();
	fs::write(
		root.path().join("deploy/playground-toolchain.digest"),
		format!("image@sha256:{}", "a".repeat(64)),
	)
	.unwrap();
	let binary = root.path().join("binary");
	fs::write(&binary, "executable").unwrap();
	fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
	let selected = selection(&config, &["services.play-api".into()]).unwrap();
	let output = execute(
		root.path(),
		&config,
		&selected,
		true,
		Path::new("receipts"),
		|_| service_build(root.path(), &config, &binary),
	)
	.unwrap();
	let original = fs::read(output.join("manifest.json")).unwrap();
	let pin = hash(&original);
	let checked = crate::verify::graph::check(root.path(), &config, &output, &pin).unwrap();
	let inventory = checked.executable_inventory(&[]).unwrap();
	assert_eq!(inventory.len(), 1);
	assert_eq!(inventory[0].unit, "services.play-api");
	assert_eq!(inventory[0].service.as_ref().unwrap().version, "1.0.0");
	assert_eq!(
		inventory[0].service.as_ref().unwrap().settings.env["LABEL"],
		"public 世界 %n $HOME"
	);
	assert!(
		!String::from_utf8(original.clone())
			.unwrap()
			.contains(root.path().to_str().unwrap())
	);
	assert_eq!(
		checked.release_inventory(&[]).unwrap(),
		serde_json::json!({"services.play-api": {}})
	);
	let parent = tempfile::tempdir().unwrap();
	let staged = checked.stage_release(parent.path(), &[]).unwrap();
	staged.recheck().unwrap();
	let lock = root.path().join("Cargo.lock");
	let lock_bytes = fs::read(&lock).unwrap();
	fs::write(&lock, "changed").unwrap();
	assert!(checked.recheck().is_err());
	assert!(staged.recheck().is_err());
	fs::write(&lock, &lock_bytes).unwrap();
	for path in [
		"services.play-api/service/Cargo.lock",
		"services.play-api/backend",
	] {
		let path = output.join(path);
		let bytes = fs::read(&path).unwrap();
		fs::write(&path, "substitution").unwrap();
		assert!(checked.recheck().is_err());
		fs::write(&path, bytes).unwrap();
	}
	let copied = output.join("services.play-api/service/Cargo.lock");
	fs::set_permissions(&copied, fs::Permissions::from_mode(0o600)).unwrap();
	assert!(checked.recheck().is_err());
	fs::set_permissions(&copied, fs::Permissions::from_mode(0o644)).unwrap();
	fs::write(output.join("services.play-api/service/extra"), "extra").unwrap();
	assert!(checked.recheck().is_err());
	fs::remove_file(output.join("services.play-api/service/extra")).unwrap();
	let mut altered_env: Value = serde_json::from_slice(&original).unwrap();
	altered_env["units"]["services.play-api"]["service"]["settings"]["env"]["LABEL"] =
		serde_json::json!("other");
	let altered = serde_json::to_vec(&altered_env).unwrap();
	fs::write(output.join("manifest.json"), &altered).unwrap();
	fs::write(
		output.join("manifest.sha256"),
		format!("{}\n", hash(&altered)),
	)
	.unwrap();
	let error = crate::verify::graph::check(root.path(), &config, &output, &hash(&altered))
		.err()
		.unwrap();
	assert!(
		format!("{error:#}").contains("resolved service settings differ"),
		"{error:#}"
	);
	fs::write(output.join("manifest.json"), &original).unwrap();
	fs::write(output.join("manifest.sha256"), format!("{pin}\n")).unwrap();
	checked.recheck().unwrap();
	let mut changed: Value = serde_json::from_slice(&original).unwrap();
	changed["units"]["services.play-api"]["service"]["settings"]["features"] =
		serde_json::json!(["other"]);
	let changed = serde_json::to_vec(&changed).unwrap();
	fs::write(output.join("manifest.json"), &changed).unwrap();
	fs::write(
		output.join("manifest.sha256"),
		format!("{}\n", hash(&changed)),
	)
	.unwrap();
	assert!(checked.recheck().is_err());
	assert!(crate::verify::graph::check(root.path(), &config, &output, &pin).is_err());
}

#[test]
fn absent_or_changed_lock_cannot_survive_prepared_capture() {
	let (root, config) = fixture();
	let built = service_build(root.path(), &config, &root.path().join("binary")).unwrap();
	fs::remove_file(root.path().join("Cargo.lock")).unwrap();
	assert!(built.inputs.recheck().is_err());
	fs::write(root.path().join("Cargo.lock"), "different lock").unwrap();
	assert!(built.inputs.recheck().is_err());
}

#[test]
fn portable_path_identity_and_unsupported_registry_git_sources_are_explicit() {
	use leptoskit_config::ServiceSource;

	use crate::verify::graph::service::portable;
	let (_, config) = fixture();
	let mut settings =
		leptoskit_config::ResolvedService::merge("play-api", &config.services["play-api"], None)
			.unwrap();
	settings.source = ServiceSource::Path {
		path: "/machine/private/source".into(),
	};
	let value = portable(settings.clone()).unwrap();
	assert!(matches!(value.source, ServiceSource::Path { path } if path == Path::new("source")));
	settings.source = ServiceSource::Crate {
		name: "api".into(),
		version: Some("1.0".into()),
	};
	assert!(portable(settings.clone()).is_err());
	settings.source = ServiceSource::Git {
		url: "https://example.com/api".into(),
		package: "api".into(),
		reference: None,
	};
	assert!(portable(settings).is_err());
}
