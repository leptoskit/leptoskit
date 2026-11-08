//! Multi-frontend workspace scaffolds (`landing` / `app` / `admin`).

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use camino::Utf8PathBuf;
use leptoskit_config::Platform;
use leptoskit_config::workspace::Application as WorkspaceApplication;
use leptoskit_config::workspace::ApplicationKind;
use leptoskit_config::workspace::WORKSPACE_FILE;
use leptoskit_config::workspace::WORKSPACE_VERSION;
use leptoskit_config::workspace::Workspace;

use super::NewAppOptions;
use super::ensure_package_name;
use super::preflight_file;
use super::run_configured;
use super::write_new_bytes;

/// Scaffolds a multi-frontend workspace: `landing`, `app`, and/or `admin`
/// under `apps/`, plus `leptoskit.workspace.toml`.
///
/// Each frontend is its own leptoskit application (own `leptoskit.toml`,
/// routes, and deploy). Landing and admin are web-only; `app` keeps the full
/// platform set. Shared sign-in across frontends is out of scope here.
///
/// # Errors
///
/// Unknown frontend names, empty lists, duplicates, invalid package names, or
/// scaffold conflicts.
pub fn run_frontends(
	path: &str,
	frontends: &[String],
	name: Option<&str>,
	sdk: Option<&str>,
) -> Result<()> {
	anyhow::ensure!(
		!frontends.is_empty(),
		"--frontends needs at least one of landing, app, admin"
	);

	let mut kinds = Vec::with_capacity(frontends.len());
	let mut seen = BTreeSet::new();

	for entry in frontends {
		let kind = ApplicationKind::parse_frontend(entry).map_err(anyhow::Error::msg)?;
		anyhow::ensure!(
			seen.insert(kind.application_id()),
			"duplicate frontend `{}`",
			kind.application_id()
		);
		kinds.push(kind);
	}

	std::fs::create_dir_all(path).with_context(|| format!("could not create {path}"))?;
	let dir = std::fs::canonicalize(path).with_context(|| format!("could not resolve {path}"))?;
	let dir = Utf8PathBuf::from_path_buf(dir)
		.map_err(|dir| anyhow::anyhow!("{} is not UTF-8", dir.display()))?;
	let product = name
		.or_else(|| dir.file_name())
		.context("the project directory has no name")?;
	ensure_package_name(product)?;
	let workspace_path = dir.join(WORKSPACE_FILE);
	preflight_file(&dir, &workspace_path)?;
	preflight_file(&dir, &dir.join("README.md"))?;

	let mut applications = BTreeMap::new();
	let ports = [3000_u16, 3001, 3002, 3003, 3004, 3005];

	for (index, kind) in kinds.iter().enumerate() {
		let id = kind.application_id();
		let app_dir = dir.join("apps").join(id);
		let platforms = if kind.includes_native() {
			None
		} else {
			Some(vec![Platform::Web])
		};
		run_configured(NewAppOptions {
			path: app_dir.as_str(),
			platform: Platform::Web,
			name: Some(id),
			sdk,
			database: None,
			extension: false,
			auth: false,
			platforms,
			serve_port: Some(ports[index % ports.len()]),
			quiet: true,
		})?;
		applications.insert(
			id.to_owned(),
			WorkspaceApplication {
				path: PathBuf::from(format!("apps/{id}")),
				kind: Some(*kind),
				depends_on: Vec::new(),
				bindings: BTreeMap::new(),
			},
		);
	}

	let workspace = Workspace {
		format_version: WORKSPACE_VERSION,
		applications,
	};
	workspace.validate().map_err(anyhow::Error::msg)?;
	let mut document = String::from(
		"# Multiple frontend deployments in one repository.\n# Each apps/<name> has its own \
		 leptoskit.toml, routes, and release.\n# Same-origin shared auth is not wired by this \
		 scaffold.\n\n",
	);
	document.push_str(&toml::to_string_pretty(&workspace)?);
	write_new_bytes(&workspace_path, document.as_bytes())?;
	write_new_bytes(
		&dir.join("README.md"),
		frontends_readme(&kinds, crate::cli::invoked_name()).as_bytes(),
	)?;

	let cli = crate::cli::invoked_name();
	let listed = kinds
		.iter()
		.map(|kind| kind.application_id())
		.collect::<Vec<_>>()
		.join(", ");
	crate::ui::success(format_args!(
		"created {dir} with frontends: {listed} ({WORKSPACE_FILE})"
	));
	crate::ui::hint(
		"next",
		format_args!("cd {dir}/apps/{} && {cli} build", kinds[0].application_id()),
	);
	crate::ui::hint(
		"then",
		format_args!("{cli} workspace explain   # from {dir} or any apps/*"),
	);

	Ok(())
}

fn frontends_readme(kinds: &[ApplicationKind], cli: &str) -> String {
	let mut body = String::from(
		"# Multi-frontend leptoskit workspace\n\nEach directory under `apps/` is a separate \
		 leptoskit application with its own `leptoskit.toml`, routes, build, and deploy. \
		 `leptoskit.workspace.toml` lists them.\n\n",
	);

	for kind in kinds {
		let id = kind.application_id();
		let role = match kind {
			ApplicationKind::Landing => "marketing site (web only)",
			ApplicationKind::Product => "product app (web and native hosts)",
			ApplicationKind::Admin => "admin console (web only)",
			ApplicationKind::Server => "API server",
		};
		let _ = writeln!(body, "- `apps/{id}` — {role}");
	}

	let _ = write!(
		body,
		"\n## Commands\n\n```bash\ncd apps/<name>\n{cli} build\n{cli} dev\n```\n\nFrom the \
		 repository root (or any app):\n\n```bash\n{cli} workspace explain\n{cli} workspace check \
		 --json\n```\n\nCross-frontend links are ordinary URLs between deployments. Shared \
		 sign-in across frontends is a separate design; this scaffold does not wire it.\n"
	);
	body
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_frontends_accepts_aliases_and_rejects_unknown() {
		assert_eq!(
			ApplicationKind::parse_frontend("app").unwrap(),
			ApplicationKind::Product
		);
		assert_eq!(
			ApplicationKind::parse_frontend("product").unwrap(),
			ApplicationKind::Product
		);
		assert!(ApplicationKind::parse_frontend("marketing").is_err());
	}

	#[test]
	fn run_frontends_writes_workspace_and_web_only_apps() {
		let root = tempfile::tempdir().unwrap();
		let path = root.path().join("shop");
		run_frontends(
			path.to_str().unwrap(),
			&["landing".into(), "app".into(), "admin".into()],
			None,
			None,
		)
		.unwrap();

		let manifest = std::fs::read_to_string(path.join(WORKSPACE_FILE)).unwrap();
		assert!(manifest.contains("kind = \"landing\""));
		assert!(manifest.contains("kind = \"product\""));
		assert!(manifest.contains("kind = \"admin\""));
		assert!(path.join("apps/landing/leptoskit.toml").is_file());
		assert!(path.join("apps/app/leptoskit.toml").is_file());
		assert!(path.join("apps/admin/leptoskit.toml").is_file());
		assert!(!path.join("apps/landing/ios").exists());
		assert!(path.join("apps/app/ios/Cargo.toml").is_file());
		assert!(!path.join("apps/admin/android").exists());

		let landing =
			leptoskit_config::LeptoskitConfig::load_from(&path.join("apps/landing")).unwrap();
		assert_eq!(landing.app.platforms, [Platform::Web]);
		assert_eq!(landing.serve.port, 3000);

		let product = leptoskit_config::LeptoskitConfig::load_from(&path.join("apps/app")).unwrap();
		assert!(product.app.platforms.contains(&Platform::Ios));
		assert_eq!(product.serve.port, 3001);

		let admin = leptoskit_config::LeptoskitConfig::load_from(&path.join("apps/admin")).unwrap();
		assert_eq!(admin.serve.port, 3002);
	}
}
