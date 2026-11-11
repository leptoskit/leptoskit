#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use serde_json::Value;
use serde_json::json;

use super::*;
use crate::verify::assets;

struct Fixture {
	project: tempfile::TempDir,
	bundle: tempfile::TempDir,
	config: LeptoskitConfig,
}

impl Fixture {
	fn new() -> Self {
		let project = tempfile::tempdir().unwrap();
		let bundle = tempfile::tempdir().unwrap();
		let config: LeptoskitConfig =
			toml::from_str(include_str!("../../../tests/fixtures/website-graph.toml")).unwrap();
		let mut units = serde_json::Map::new();
		for (unit, body) in [("frontends.site", "landing"), ("frontends.docs", "docs")] {
			let assets = bundle.path().join(unit).join("assets");
			fs::create_dir_all(&assets).unwrap();
			fs::set_permissions(&assets, fs::Permissions::from_mode(0o755)).unwrap();
			fs::write(assets.join("index.html"), body).unwrap();
			fs::set_permissions(assets.join("index.html"), fs::Permissions::from_mode(0o644))
				.unwrap();
			units.insert(
				unit.into(),
				json!({"host":"web","assets":Root::open(&assets).unwrap().tree(&[]).unwrap()}),
			);
		}
		let manifest = json!({"format_version":1,"kind":"leptoskit-graph-build",
			"configuration_sha256":hash(&serde_json::to_vec(&config).unwrap()),
			"service_profile":"release","cli_sha256":"a".repeat(64),
			"recipes":{"public-site":hash(&serde_json::to_vec(&config.artifacts["public-site"]).unwrap())},
			"units":units,"readiness_only":[],"evidence":{"artifact_integrity":"captured",
			"source_reproduction":"not_checked","isolated_build":"not_checked",
			"origin_authorization":"not_checked","runtime":"not_checked"}});
		let result = Self {
			project,
			bundle,
			config,
		};
		result.write(&manifest);
		result
	}

	fn manifest(&self) -> Value {
		serde_json::from_slice(&fs::read(self.bundle.path().join("manifest.json")).unwrap())
			.unwrap()
	}

	fn write(&self, value: &Value) -> String {
		self.bytes(&serde_json::to_vec_pretty(value).unwrap())
	}

	fn bytes(&self, bytes: &[u8]) -> String {
		let pin = hash(bytes);
		fs::write(self.bundle.path().join("manifest.json"), bytes).unwrap();
		fs::write(
			self.bundle.path().join("manifest.sha256"),
			format!("{pin}\n"),
		)
		.unwrap();
		pin
	}

	fn pin(&self) -> String {
		hash(&fs::read(self.bundle.path().join("manifest.json")).unwrap())
	}

	fn check(&self) -> Result<Checked> {
		check(
			self.project.path(),
			&self.config,
			self.bundle.path(),
			&self.pin(),
		)
	}
}

#[test]
fn selected_staging_keeps_original_pin_and_never_selects_a_call_target() {
	let fixture = Fixture::new();
	let checked = fixture.check().unwrap();
	let selection = checked.selection(&["frontends.docs".into()]).unwrap();
	let parent = tempfile::tempdir().unwrap();
	let stage = checked.stage(parent.path(), &selection).unwrap();
	assert_eq!(
		fs::read_to_string(stage.join("frontends.docs/assets/index.html")).unwrap(),
		"docs"
	);
	assert!(!stage.join("frontends.site").exists());
	assert!(!stage.join("services.play-api").exists());
	assert_eq!(
		hash(&fs::read(stage.join("source-manifest.json")).unwrap()),
		fixture.pin()
	);
	let document: Value =
		serde_json::from_slice(&fs::read(stage.join("stage.json")).unwrap()).unwrap();
	assert_eq!(document["units"], json!(["frontends.docs"]));
	for field in [
		"runtime",
		"source_reproduction",
		"release_approval",
		"executable_target",
	] {
		assert_eq!(document["evidence"][field], "not_checked");
	}
	assert_eq!(
		fs::metadata(&stage).unwrap().permissions().mode() & 0o777,
		0o700
	);
	assert_eq!(
		fs::read_to_string(stage.join("stage.sha256"))
			.unwrap()
			.trim(),
		hash(&fs::read(stage.join("stage.json")).unwrap())
	);
}

#[test]
fn replacing_both_receipt_and_bytes_cannot_replace_the_independent_pin() {
	let fixture = Fixture::new();
	let original = fixture.pin();
	fs::write(
		fixture
			.bundle
			.path()
			.join("frontends.docs/assets/index.html"),
		"changed",
	)
	.unwrap();
	let mut manifest = fixture.manifest();
	manifest["units"]["frontends.docs"]["assets"] = json!(
		Root::open(&fixture.bundle.path().join("frontends.docs/assets"))
			.unwrap()
			.tree(&[])
			.unwrap()
	);
	fixture.write(&manifest);
	assert!(
		check(
			fixture.project.path(),
			&fixture.config,
			fixture.bundle.path(),
			&original
		)
		.is_err()
	);
	assert!(
		fixture.check().is_ok(),
		"new independent pin is an explicit different integrity claim"
	);
}

#[test]
fn changed_content_modes_membership_and_links_are_refused() {
	for change in 0..7 {
		let fixture = Fixture::new();
		let unit = fixture.bundle.path().join("frontends.docs");
		let file = unit.join("assets/index.html");
		match change {
			0 => fs::write(&file, "changed").unwrap(),
			1 => fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap(),
			2 => fs::write(unit.join("assets/extra"), "extra").unwrap(),
			3 => fs::write(unit.join("extra"), "extra").unwrap(),
			4 => fs::write(fixture.bundle.path().join("extra"), "extra").unwrap(),
			5 => {
				fs::remove_file(&file).unwrap();
				std::os::unix::fs::symlink("../../../../elsewhere", &file).unwrap();
			}
			_ => fs::remove_file(&file).unwrap(),
		}
		assert!(fixture.check().is_err(), "mutation {change}");
	}
}

#[test]
fn consumption_revalidates_held_handles_and_failed_stage_publishes_nothing() {
	let fixture = Fixture::new();
	let checked = fixture.check().unwrap();
	let selection = checked.selection(&["frontends.docs".into()]).unwrap();
	let parent = tempfile::tempdir().unwrap();
	fs::write(
		fixture
			.bundle
			.path()
			.join("frontends.docs/assets/index.html"),
		"changed",
	)
	.unwrap();
	assert!(checked.stage(parent.path(), &selection).is_err());
	assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
}

#[test]
fn current_configuration_and_declarations_are_independent_checks() {
	for change in 0..10 {
		let fixture = Fixture::new();
		let mut manifest = fixture.manifest();
		match change {
			0 => manifest["format_version"] = json!(3),
			1 => manifest["kind"] = json!("other"),
			2 => manifest["configuration_sha256"] = json!("b".repeat(64)),
			3 => manifest["recipes"]["public-site"] = json!("c".repeat(64)),
			4 => manifest["units"]["frontends.docs"]["host"] = json!("compiler"),
			5 => manifest["readiness_only"] = json!(["services.play-api"]),
			6 => manifest["evidence"]["source_reproduction"] = json!("verified"),
			7 => {
				manifest["units"]["frontends.docs"]["assets"]["index.html"]["size"] =
					json!(assets::MAX_BYTES + 1);
			}
			8 => {
				manifest["units"]["frontends.docs"]["backend"] =
					json!({"kind":"file","mode":493,"size":1,"sha256":"a".repeat(64)});
			}
			_ => {
				manifest["units"]["frontends.unknown"] =
					manifest["units"]["frontends.docs"].clone();
			}
		}
		fixture.write(&manifest);
		assert!(fixture.check().is_err(), "declaration {change}");
	}
	let mut fixture = Fixture::new();
	fixture.config.frontends.get_mut("docs").unwrap().mount = "/reference/".into();
	assert!(fixture.check().is_err());
}

#[test]
fn duplicate_keys_at_any_depth_and_escaped_aliases_are_refused() {
	let fixture = Fixture::new();
	let original =
		String::from_utf8(fs::read(fixture.bundle.path().join("manifest.json")).unwrap()).unwrap();
	for bytes in [
		original.replacen('{', "{\"format_version\":1,", 1),
		original.replacen("\"host\":", "\"host\":\"web\",\"h\\u006fst\":", 1),
		original.replacen("\"mode\":", "\"mode\":420,\"mode\":", 1),
	] {
		fixture.bytes(bytes.as_bytes());
		assert!(fixture.check().is_err());
	}
}

#[test]
fn ownership_and_selection_cannot_reintroduce_excluded_units() {
	let fixture = Fixture::new();
	let checked = fixture.check().unwrap();
	for units in [
		vec!["services.play-api".into()],
		vec!["frontends.docs".into(), "frontends.docs".into()],
	] {
		assert!(checked.selection(&units).is_err());
	}
	let selection = checked.selection(&["frontends.docs".into()]).unwrap();
	assert!(checked.stage(fixture.bundle.path(), &selection).is_err());
	let mut manifest = fixture.manifest();
	manifest["units"]["frontends.site"]["assets"]["docs"] = json!({"kind":"directory","mode":493});
	fixture.write(&manifest);
	assert!(fixture.check().is_err());
}

#[test]
fn legacy_and_missing_service_evidence_are_rejected_without_execution() {
	let fixture = Fixture::new();
	let directory = fixture.bundle.path().join("services.play-api");
	fs::create_dir(&directory).unwrap();
	fs::write(directory.join("backend"), "#!/bin/sh\nexit 77\n").unwrap();
	fs::set_permissions(directory.join("backend"), fs::Permissions::from_mode(0o755)).unwrap();
	let pin = format!("registry/toolchain@sha256:{}", "a".repeat(64));
	fs::create_dir(fixture.project.path().join("deploy")).unwrap();
	let path = fixture
		.project
		.path()
		.join("deploy/playground-toolchain.digest");
	fs::write(&path, &pin).unwrap();
	let mut manifest = fixture.manifest();
	manifest["units"]["services.play-api"] = json!({"host":"compiler","backend":Root::open(&directory).unwrap().executable("backend").unwrap(),"toolchain_image":pin});
	fixture.write(&manifest);
	assert!(
		fixture
			.check()
			.err()
			.unwrap()
			.to_string()
			.contains("legacy service receipt")
	);
	manifest["format_version"] = json!(2);
	fixture.write(&manifest);
	assert!(fixture.check().is_err());
}

#[test]
fn metadata_and_aggregate_artifact_budgets_are_enforced_before_consumption() {
	let fixture = Fixture::new();
	let mut manifest = fixture.manifest();
	for unit in ["frontends.site", "frontends.docs"] {
		manifest["units"][unit]["assets"]["index.html"]["size"] = json!(assets::MAX_BYTES / 2 + 1);
	}
	fixture.write(&manifest);
	let error = fixture.check().err().unwrap().to_string();
	assert!(error.contains("aggregate byte limit"), "{error}");
	let path = fixture.bundle.path().join("manifest.json");
	fs::OpenOptions::new()
		.write(true)
		.open(path)
		.unwrap()
		.set_len(assets::MAX_METADATA_BYTES + 1)
		.unwrap();
	let error = fixture.check().err().unwrap().to_string();
	assert!(error.contains("byte limit"), "{error}");
}

#[test]
fn readiness_dependencies_are_retained_without_staging_their_artifacts() {
	let mut fixture = Fixture::new();
	fixture.config.frontends.get_mut("docs").unwrap().depends_on = vec!["services.play-api".into()];
	let mut manifest = fixture.manifest();
	manifest["configuration_sha256"] = json!(hash(&serde_json::to_vec(&fixture.config).unwrap()));
	manifest["readiness_only"] = json!(["services.play-api"]);
	fixture.write(&manifest);
	let checked = fixture.check().unwrap();
	let selection = checked.selection(&["frontends.docs".into()]).unwrap();
	let parent = tempfile::tempdir().unwrap();
	let staged = checked.stage(parent.path(), &selection).unwrap();
	assert!(!staged.join("services.play-api").exists());
	let document: Value =
		serde_json::from_slice(&fs::read(staged.join("stage.json")).unwrap()).unwrap();
	assert_eq!(document["readiness_only"], json!(["services.play-api"]));
	assert_eq!(document["evidence"]["runtime"], "not_checked");
}

#[test]
fn typed_stage_rechecks_source_and_private_copy_during_consumption() {
	let fixture = Fixture::new();
	let checked = fixture.check().unwrap();
	let parent = tempfile::tempdir().unwrap();
	let staged = checked
		.stage_release(parent.path(), &["frontends.docs".into()])
		.unwrap();
	assert_eq!(staged.selected(), vec!["frontends.docs"]);
	assert_eq!(staged.readiness_only(), [] as [String; 0]);
	assert_eq!(
		fs::metadata(&staged.path).unwrap().permissions().mode() & 0o777,
		0o700
	);
	assert!(!staged.path.join("frontends.site").exists());
	staged
		.consume(|root| {
			root.beneath("frontends.docs")?
				.beneath("assets")?
				.metadata("index.html")
		})
		.unwrap();
	let result = staged.consume(|_| {
		fs::write(
			staged.path.join("frontends.docs/assets/index.html"),
			"substitution",
		)?;
		Ok(())
	});
	assert!(result.is_err());
	let fresh = checked
		.stage_release(parent.path(), &["frontends.docs".into()])
		.unwrap();
	assert!(
		fresh
			.consume(|_| {
				fs::write(
					fixture
						.bundle
						.path()
						.join("frontends.docs/assets/index.html"),
					"changed source",
				)?;
				Ok(())
			})
			.is_err()
	);
}

#[test]
fn typed_stage_rejects_metadata_substitution() {
	let fixture = Fixture::new();
	let checked = fixture.check().unwrap();
	let parent = tempfile::tempdir().unwrap();
	let staged = checked
		.stage_release(parent.path(), &["frontends.docs".into()])
		.unwrap();
	fs::write(staged.path.join("stage.json"), "{}").unwrap();
	fs::write(
		staged.path.join("stage.sha256"),
		format!("{}\n", hash(b"{}")),
	)
	.unwrap();
	assert!(staged.consume(|_| Ok(())).is_err());
}

#[test]
fn static_release_inventory_keeps_unselected_service_out() {
	let fixture = Fixture::new();
	assert!(!fixture.config.services.is_empty());
	let checked = fixture.check().unwrap();
	let inventory = checked
		.release_inventory(&["frontends.docs".into()])
		.unwrap();
	assert_eq!(
		inventory.as_object().unwrap().keys().collect::<Vec<_>>(),
		vec!["frontends.docs"]
	);
	assert!(
		checked
			.release_inventory(&["services.play-api".into()])
			.is_err()
	);
	fs::write(
		fixture
			.bundle
			.path()
			.join("frontends.docs/assets/index.html"),
		"changed",
	)
	.unwrap();
	assert!(
		checked
			.release_inventory(&["frontends.docs".into()])
			.is_err()
	);
}
