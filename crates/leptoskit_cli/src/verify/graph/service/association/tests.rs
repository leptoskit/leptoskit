use std::collections::BTreeMap;
use std::fs;

use super::*;

fn evidence() -> ServiceEvidence {
	let config: leptoskit_config::ServiceConfig =
		toml::from_str("path = 'api'\ndatabase = false").unwrap();
	ServiceEvidence {
		settings: leptoskit_config::ResolvedService::merge("api", &config, None).unwrap(),
		package: "api_package".into(),
		version: "1.0.0".into(),
		binary: "server".into(),
		profile: "release".into(),
		default_features: true,
		package_manifest: "package.toml".into(),
		workspace_manifest: "workspace.toml".into(),
		lock: "Cargo.lock".into(),
		inputs: BTreeMap::new(),
		sources: BTreeMap::from([
			("package.toml".into(), "api/Cargo.toml".into()),
			("workspace.toml".into(), "Cargo.toml".into()),
			("Cargo.lock".into(), "Cargo.lock".into()),
		]),
	}
}

struct Fixture {
	root: tempfile::TempDir,
	package: toml::Value,
	workspace: toml::Value,
	lock: toml::Value,
}

impl Fixture {
	fn new() -> Self {
		let root = tempfile::tempdir().unwrap();
		for name in ["api", "other"] {
			fs::create_dir(root.path().join(name)).unwrap();
		}
		let package = "[package]\nname = 'api_package'\nversion = '1.0.0'\n";
		let workspace = "[workspace]\nmembers = ['api', 'other']\n";
		let lock = "version = 3\n[[package]]\nname = 'api_package'\nversion = '1.0.0'\n";
		for path in ["api/Cargo.toml", "other/Cargo.toml"] {
			fs::write(root.path().join(path), package).unwrap();
		}
		fs::write(root.path().join("Cargo.toml"), workspace).unwrap();
		fs::write(root.path().join("Cargo.lock"), lock).unwrap();
		Self {
			root,
			package: toml::from_str(package).unwrap(),
			workspace: toml::from_str(workspace).unwrap(),
			lock: toml::from_str(lock).unwrap(),
		}
	}

	fn check(&self, evidence: &ServiceEvidence) -> Result<()> {
		check(
			evidence,
			self.root.path(),
			&ServiceSource::Path { path: "api".into() },
			&self.package,
			&self.workspace,
			&self.lock,
		)
	}
}

#[test]
fn identical_other_package_bytes_do_not_establish_declared_path_identity() {
	let fixture = Fixture::new();
	let mut evidence = evidence();
	fixture.check(&evidence).unwrap();
	evidence
		.sources
		.insert("package.toml".into(), "other/Cargo.toml".into());
	assert!(
		fixture
			.check(&evidence)
			.unwrap_err()
			.to_string()
			.contains("declared service path")
	);
}

#[test]
fn capture_checks_path_before_portable_normalization() {
	let fixture = Fixture::new();
	let declared = ServiceSource::Path {
		path: fixture.root.path().join("api"),
	};
	source(
		&declared,
		fixture.root.path(),
		&fixture.root.path().join("api/Cargo.toml"),
		"api_package",
	)
	.unwrap();
	assert!(
		source(
			&declared,
			fixture.root.path(),
			&fixture.root.path().join("other/Cargo.toml"),
			"api_package"
		)
		.is_err()
	);
}

#[test]
fn lock_and_explicit_workspace_must_belong_to_selected_source() {
	let mut fixture = Fixture::new();
	let mut evidence = evidence();
	evidence
		.sources
		.insert("Cargo.lock".into(), "other/Cargo.lock".into());
	assert!(
		fixture
			.check(&evidence)
			.unwrap_err()
			.to_string()
			.contains("workspace lock")
	);
	evidence
		.sources
		.insert("Cargo.lock".into(), "Cargo.lock".into());
	fixture.package["package"]
		.as_table_mut()
		.unwrap()
		.insert("workspace".into(), toml::Value::String("../other".into()));
	assert!(
		fixture
			.check(&evidence)
			.unwrap_err()
			.to_string()
			.contains("package.workspace")
	);
}

#[test]
fn unrelated_lock_entry_cannot_stand_in_for_selected_local_package() {
	let mut fixture = Fixture::new();
	fixture.lock =
		toml::from_str("version = 3\n[[package]]\nname = 'other_package'\nversion = '1.0.0'")
			.unwrap();
	assert!(fixture.check(&evidence()).is_err());
	fixture.lock = toml::from_str("version = 3\n[[package]]\nname = 'api_package'\nversion = '1.0.0'\nsource = 'registry+https://example.com/index'").unwrap();
	assert!(fixture.check(&evidence()).is_err());
}
