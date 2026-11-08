//! Multi-frontend workspace scaffolds (`landing` / `app` / `admin`).

use std::collections::BTreeMap;
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
/// Pass an empty `frontends` slice for the default trio
/// ([`ApplicationKind::DEFAULT_FRONTENDS`]). Landing and admin are web-only;
/// `app` keeps the full platform set and takes the product package name from
/// the workspace directory (or `--name`). Shared sign-in across frontends is
/// out of scope here.
///
/// # Errors
///
/// Unknown frontend names, duplicates, invalid package names, or scaffold
/// conflicts.
pub fn run_frontends(
	path: &str,
	frontends: &[String],
	name: Option<&str>,
	sdk: Option<&str>,
) -> Result<()> {
	let kinds = ApplicationKind::parse_frontend_list(frontends).map_err(anyhow::Error::msg)?;

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

	for kind in &kinds {
		let id = kind.application_id();
		let package_name = package_name_for(*kind, product);
		ensure_package_name(package_name)?;
		let app_dir = dir.join("apps").join(id);
		let platforms = if kind.includes_native() {
			None
		} else {
			Some(vec![Platform::Web])
		};
		run_configured(NewAppOptions {
			path: app_dir.as_str(),
			platform: Platform::Web,
			name: Some(package_name),
			sdk,
			database: None,
			extension: false,
			auth: false,
			platforms,
			serve_port: Some(kind.default_port()),
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

/// Cargo package name for a frontend app. The product app uses the workspace
/// product name so crates are `shop_app`, not `app_app`.
fn package_name_for(kind: ApplicationKind, product: &str) -> &str {
	if kind == ApplicationKind::Product {
		product
	} else {
		kind.application_id()
	}
}

fn frontends_readme(kinds: &[ApplicationKind], cli: &str) -> String {
	let mut body = String::from(
		"# Multi-frontend leptoskit workspace\n\nEach directory under `apps/` is a separate \
		 leptoskit application with its own `leptoskit.toml`, routes, build, and deploy. \
		 `leptoskit.workspace.toml` lists them.\n\n",
	);

	for kind in kinds {
		let id = kind.application_id();
		let _ = writeln!(body, "- `apps/{id}` — {}", kind.role_label());
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
	fn parse_frontends_accepts_aliases_case_and_whitespace() {
		assert_eq!(
			ApplicationKind::parse_frontend(" App ").unwrap(),
			ApplicationKind::Product
		);
		assert_eq!(
			ApplicationKind::parse_frontend("PRODUCT").unwrap(),
			ApplicationKind::Product
		);
		assert_eq!(
			ApplicationKind::parse_frontend("landing").unwrap(),
			ApplicationKind::Landing
		);
		assert_eq!(
			ApplicationKind::parse_frontend("admin").unwrap(),
			ApplicationKind::Admin
		);
		let error = ApplicationKind::parse_frontend("marketing").unwrap_err();
		assert!(error.contains("landing, app, or admin"));
		assert!(error.contains("--frontends"));
	}

	#[test]
	fn empty_frontends_list_is_the_default_trio() {
		let kinds = ApplicationKind::parse_frontend_list(&[]).unwrap();
		assert_eq!(kinds, ApplicationKind::DEFAULT_FRONTENDS);
		assert_eq!(
			ApplicationKind::parse_frontend_list(&["admin".into(), "landing".into()]).unwrap(),
			[ApplicationKind::Admin, ApplicationKind::Landing]
		);
		assert!(
			ApplicationKind::parse_frontend_list(&["app".into(), "product".into()])
				.unwrap_err()
				.contains("duplicate")
		);
	}

	#[test]
	fn package_name_uses_product_for_app_kind() {
		assert_eq!(package_name_for(ApplicationKind::Product, "shop"), "shop");
		assert_eq!(
			package_name_for(ApplicationKind::Landing, "shop"),
			"landing"
		);
		assert_eq!(package_name_for(ApplicationKind::Admin, "shop"), "admin");
	}

	#[test]
	fn readme_lists_each_kind_including_server() {
		let text = frontends_readme(
			&[
				ApplicationKind::Landing,
				ApplicationKind::Product,
				ApplicationKind::Admin,
				ApplicationKind::Server,
			],
			"lk",
		);
		assert!(text.contains("apps/landing"));
		assert!(text.contains(ApplicationKind::Server.role_label()));
		assert!(text.contains("lk workspace explain"));
	}

	#[test]
	fn run_frontends_defaults_and_keeps_stable_ports() {
		let root = tempfile::tempdir().unwrap();
		let path = root.path().join("shop");
		// Empty list → default trio; order of a custom list must not change
		// ports.
		run_frontends(path.to_str().unwrap(), &[], Some("shop"), None).unwrap();

		let manifest = std::fs::read_to_string(path.join(WORKSPACE_FILE)).unwrap();
		assert!(manifest.contains("kind = \"landing\""));
		assert!(manifest.contains("kind = \"product\""));
		assert!(manifest.contains("kind = \"admin\""));
		assert!(path.join("apps/landing/leptoskit.toml").is_file());
		assert!(path.join("apps/app/shop_app/Cargo.toml").is_file());
		let product_toml = std::fs::read_to_string(path.join("apps/app/Cargo.toml")).unwrap();
		assert!(product_toml.contains("name = \"shop\""), "{product_toml}");
		assert!(!path.join("apps/landing/ios").exists());
		assert!(path.join("apps/app/ios/Cargo.toml").is_file());

		let landing =
			leptoskit_config::LeptoskitConfig::load_from(&path.join("apps/landing")).unwrap();
		assert_eq!(landing.app.platforms, [Platform::Web]);
		assert_eq!(landing.serve.port, 3000);
		assert_eq!(landing.app.name, "landing");
		assert_file_router(&path.join("apps/landing"), "landing");

		let product = leptoskit_config::LeptoskitConfig::load_from(&path.join("apps/app")).unwrap();
		assert!(product.app.platforms.contains(&Platform::Ios));
		assert_eq!(product.serve.port, 3001);
		assert_eq!(product.app.name, "shop");
		assert_file_router(&path.join("apps/app"), "shop");

		let admin = leptoskit_config::LeptoskitConfig::load_from(&path.join("apps/admin")).unwrap();
		assert_eq!(admin.serve.port, 3002);
		assert_file_router(&path.join("apps/admin"), "admin");
	}

	fn assert_file_router(app_root: &std::path::Path, package: &str) {
		let toml = std::fs::read_to_string(app_root.join("leptoskit.toml")).unwrap();
		assert!(
			toml.contains("name = \"router\"")
				&& toml.contains(&format!("dir = \"{package}_app/src/routes\"")),
			"{toml}"
		);
		let routes = app_root.join(format!("{package}_app/src/routes"));
		assert!(routes.join("index.rs").is_file());
		assert!(routes.join("_layout.rs").is_file());
		assert!(routes.join("+not-found.rs").is_file());
		assert!(!routes.join("home.rs").exists());
		let generated =
			std::fs::read_to_string(app_root.join(format!("{package}_app/src/routes.rs"))).unwrap();
		assert!(
			generated.contains("@generated") || generated.contains("Navigation"),
			"{generated}"
		);
		let cargo =
			std::fs::read_to_string(app_root.join(format!("{package}_app/Cargo.toml"))).unwrap();
		assert!(cargo.contains("leptoskit_router"), "{cargo}");
	}

	#[test]
	fn custom_order_still_uses_kind_ports() {
		let root = tempfile::tempdir().unwrap();
		let path = root.path().join("boutique");
		run_frontends(
			path.to_str().unwrap(),
			&["admin".into(), "landing".into()],
			None,
			None,
		)
		.unwrap();
		let admin = leptoskit_config::LeptoskitConfig::load_from(&path.join("apps/admin")).unwrap();
		let landing =
			leptoskit_config::LeptoskitConfig::load_from(&path.join("apps/landing")).unwrap();
		assert_eq!(admin.serve.port, 3002);
		assert_eq!(landing.serve.port, 3000);
		assert!(!path.join("apps/app").exists());
	}

	#[test]
	fn rejects_unknown_and_duplicate_frontends() {
		let root = tempfile::tempdir().unwrap();
		let path = root.path().join("bad");
		assert!(
			run_frontends(path.to_str().unwrap(), &["website".into()], None, None,)
				.unwrap_err()
				.to_string()
				.contains("unknown frontend")
		);
		assert!(
			run_frontends(
				path.to_str().unwrap(),
				&["landing".into(), "landing".into()],
				None,
				None,
			)
			.unwrap_err()
			.to_string()
			.contains("duplicate")
		);
	}

	#[test]
	fn refuses_to_overwrite_an_existing_workspace_file() {
		let root = tempfile::tempdir().unwrap();
		let path = root.path().join("existing");
		std::fs::create_dir_all(&path).unwrap();
		std::fs::write(path.join(WORKSPACE_FILE), "format_version = 1\n").unwrap();
		let error = run_frontends(path.to_str().unwrap(), &["landing".into()], None, None)
			.unwrap_err()
			.to_string();
		assert!(
			error.contains(WORKSPACE_FILE) || error.contains("exists") || error.contains("already"),
			"{error}"
		);
	}
}
