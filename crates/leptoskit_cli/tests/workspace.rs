//! The real workspace command validates declarations without building apps.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

const MANIFEST: &str = r#"
format_version = 1
[applications.web]
path = "apps/web"
depends-on = ["accounts"]
[applications.web.bindings.accounts]
application = "accounts"
url-env = "ACCOUNTS_URL"
token-env = "ACCOUNTS_TOKEN"
[applications.accounts]
path = "apps/accounts"
"#;

fn application(root: &Path, name: &str) {
	let path = root.join("apps").join(name);
	std::fs::create_dir_all(&path).unwrap();
	std::fs::write(
		path.join("leptoskit.toml"),
		format!("[app]\nname = '{name}'\nplatforms = ['web']\n[db]\n"),
	)
	.unwrap();
}

fn project() -> tempfile::TempDir {
	let directory = tempfile::tempdir().unwrap();
	application(directory.path(), "web");
	application(directory.path(), "accounts");
	std::fs::write(directory.path().join("leptoskit.workspace.toml"), MANIFEST).unwrap();
	directory
}

#[test]
fn explain_reports_boundaries_without_tools_secrets_or_deployment_claims() {
	let directory = project();
	let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.current_dir(directory.path().join("apps/web"))
		.args(["workspace", "explain", "--json"])
		.env("PATH", directory.path().join("no-tools"))
		.env("ACCOUNTS_TOKEN", "test-value-must-not-be-reported")
		.env("DATABASE_URL", "test-database-must-not-be-opened")
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let raw = String::from_utf8(output.stdout).unwrap();
	assert!(!raw.contains("test-value-must-not-be-reported"));
	assert!(!raw.contains("test-database-must-not-be-opened"));
	let report: Value = serde_json::from_str(&raw).unwrap();
	assert_eq!(
		report["startup_order"],
		serde_json::json!(["accounts", "web"])
	);
	assert_eq!(report["checks"]["configuration"], "passed");
	for check in [
		"artifacts",
		"api_compatibility",
		"database_ownership",
		"service_identity",
		"deployment",
	] {
		assert_eq!(report["checks"][check], "not_checked");
	}
	assert_eq!(
		report["applications"]["web"]["bindings"]["accounts"]["token-env"],
		"ACCOUNTS_TOKEN"
	);
	for name in ["web", "accounts"] {
		let root = directory.path().join("apps").join(name);
		assert!(!root.join(".leptoskit").exists());
		assert!(!root.join("Cargo.lock").exists());
		assert!(!root.join("dist").exists());
	}
}

#[test]
fn duplicate_config_paths_and_missing_files_fail_before_work() {
	let directory = project();
	let manifest = directory.path().join("leptoskit.workspace.toml");
	std::fs::write(&manifest, MANIFEST.replace("apps/accounts", "apps/web")).unwrap();
	let error = leptoskit_cli::workspace::inspect(&manifest).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("repeats another app configuration")
	);
	std::fs::write(&manifest, MANIFEST.replace("apps/accounts", "missing")).unwrap();
	assert!(leptoskit_cli::workspace::inspect(&manifest).is_err());
}

#[test]
fn independent_apps_cannot_claim_the_same_managed_release_identity() {
	let directory = project();
	let managed = r#"
[app]
name = "APP"
render = "ssr"
[serve]
probes = true
[db]
dialect = "postgres"
[infra]
name = "APP-production"
domain = "APP.example.com"
location = "hel1"
server_type = "cx23"
ssh_key_ids = [123]
ssh_cidrs = ["192.0.2.1/32"]
account_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
zone_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
updates_domain = "updates-APP.example.com"
updates_bucket = "APP-releases"
backup_bucket = "APP-backups"
image = "ghcr.io/team/APP"
registry = "ghcr.io"
registry_username = "team"
"#;
	let manifest = directory.path().join("leptoskit.workspace.toml");
	for name in ["accounts", "web"] {
		std::fs::write(
			directory
				.path()
				.join("apps")
				.join(name)
				.join("leptoskit.toml"),
			managed.replace("APP", name),
		)
		.unwrap();
	}
	leptoskit_cli::workspace::inspect(&manifest).unwrap();
	for (original, duplicate, field) in [
		("web-production", "accounts-production", "infra.name"),
		("ghcr.io/team/web", "ghcr.io/team/accounts", "infra.image"),
		(
			"domain = \"web.example.com\"",
			"domain = \"accounts.example.com\"",
			"infra.domain",
		),
		(
			"domain = \"web.example.com\"",
			"domain = \"ACCOUNTS.EXAMPLE.COM\"",
			"infra.domain",
		),
		(
			"domain = \"web.example.com\"",
			"domain = \"UPDATES-ACCOUNTS.EXAMPLE.COM\"",
			"infra.domain",
		),
		(
			"updates_domain = \"updates-web.example.com\"",
			"updates_domain = \"ACCOUNTS.EXAMPLE.COM\"",
			"infra.updates_domain",
		),
		(
			"updates_domain = \"updates-web.example.com\"",
			"updates_domain = \"UPDATES-ACCOUNTS.EXAMPLE.COM\"",
			"infra.updates_domain",
		),
	] {
		std::fs::write(
			directory.path().join("apps/web/leptoskit.toml"),
			managed.replace("APP", "web").replace(original, duplicate),
		)
		.unwrap();
		let error = leptoskit_cli::workspace::inspect(&manifest).unwrap_err();
		assert!(error.to_string().contains(field), "{error:#}");
		assert!(
			error.to_string().contains("applications.accounts"),
			"{error:#}"
		);
	}
}

#[test]
fn oversized_manifests_fail_before_application_loading() {
	let directory = project();
	let manifest = directory.path().join("leptoskit.workspace.toml");
	std::fs::write(&manifest, " ".repeat(1024 * 1024 + 1)).unwrap();
	std::fs::remove_dir_all(directory.path().join("apps")).unwrap();
	let error = leptoskit_cli::workspace::inspect(&manifest).unwrap_err();
	assert!(error.to_string().contains("exceeds 1 MiB"), "{error:#}");
}

#[cfg(unix)]
#[test]
fn app_and_config_symlinks_cannot_escape_the_manifest_root() {
	use std::os::unix::fs::symlink;
	let outside = tempfile::tempdir().unwrap();
	std::fs::write(
		outside.path().join("leptoskit.toml"),
		"[app]\nname='outside'\n",
	)
	.unwrap();
	for directory_link in [true, false] {
		let directory = project();
		let root = directory.path().join("apps/accounts");
		if directory_link {
			std::fs::remove_dir_all(&root).unwrap();
			symlink(outside.path(), &root).unwrap();
		} else {
			std::fs::remove_file(root.join("leptoskit.toml")).unwrap();
			symlink(
				outside.path().join("leptoskit.toml"),
				root.join("leptoskit.toml"),
			)
			.unwrap();
		}
		let error =
			leptoskit_cli::workspace::inspect(&directory.path().join("leptoskit.workspace.toml"))
				.unwrap_err();
		assert!(
			error.to_string().contains("outside the workspace"),
			"{error:#}"
		);
	}
}
