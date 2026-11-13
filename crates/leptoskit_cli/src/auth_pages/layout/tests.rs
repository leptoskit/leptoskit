use super::*;

fn read_layout(
	root: &Path,
	app: &str,
	client: Option<&str>,
	backend: Option<bool>,
) -> Result<Layout> {
	Layout::read(root, app, client, backend, app)
}

fn write(root: &Path, file: &str, text: &str) {
	let path = root.join(file);
	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	std::fs::write(path, text).unwrap();
}

fn fixture() -> tempfile::TempDir {
	let root = tempfile::tempdir().unwrap();
	write(
		root.path(),
		"Cargo.toml",
		r#"
[workspace]
members = ["packages/*"]
resolver = "3"
[workspace.package]
version = "0.1.0"
edition = "2024"
[workspace.dependencies]
leptoskit = { path = "packages/toolkit", features = ["server"] }
"#,
	);
	write(
		root.path(),
		"packages/toolkit/Cargo.toml",
		r#"
[package]
name = "leptoskit"
version.workspace = true
edition.workspace = true
[features]
server = []
"#,
	);
	write(root.path(), "packages/toolkit/src/lib.rs", "");
	write(
		root.path(),
		"packages/frontend/Cargo.toml",
		r#"
[package]
name = "ui"
version.workspace = true
edition.workspace = true
[lib]
path = "interface/application.rs"
crate-type = ["cdylib", "rlib"]
[dependencies]
leptoskit.workspace = true
"#,
	);
	write(
		root.path(),
		"packages/frontend/interface/application.rs",
		"pub fn app() {}\n",
	);
	write(
		root.path(),
		"packages/backend/Cargo.toml",
		r#"
[package]
name = "api"
version.workspace = true
edition.workspace = true
[[bin]]
name = "api"
path = "backend/server.rs"
[dependencies]
leptoskit.workspace = true
"#,
	);
	write(
		root.path(),
		"packages/backend/backend/server.rs",
		"fn main() {}\n",
	);
	write(root.path(), "apps/website/leptoskit.toml", "");
	root
}

#[test]
fn auth_generation_writes_only_the_selected_server_package() {
	let root = fixture();
	write(
		root.path(),
		"packages/identity/Cargo.toml",
		"[package]\nname = 'website'\nversion.workspace = true\nedition.workspace = \
		 true\n[dependencies]\nleptoskit.workspace = true\n",
	);
	let decoy = "// Application-owned backend, not selected.\nfn main() {}\n";
	write(root.path(), "packages/identity/src/main.rs", decoy);
	let app = root.path().join("apps/website");
	write(
		&app,
		"leptoskit.toml",
		"[app]\nname = 'website'\n[build]\nclient_package = 'ui'\nserver_package = 'api'\nbackend \
		 = true\n[db]\ndialect = 'sqlite'\n",
	);
	let config = leptoskit_config::LeptoskitConfig::load_from(&app).unwrap();
	let before_manifest = std::fs::read(root.path().join("packages/identity/Cargo.toml")).unwrap();
	super::super::generate(&app, &config, false, false).unwrap();
	assert!(
		root.path()
			.join("packages/backend/backend/sign_in.rs")
			.is_file()
	);
	assert!(
		!root
			.path()
			.join("packages/identity/src/sign_in.rs")
			.exists()
	);
	assert_eq!(
		std::fs::read_to_string(root.path().join("packages/identity/src/main.rs")).unwrap(),
		decoy
	);
	assert_eq!(
		std::fs::read(root.path().join("packages/identity/Cargo.toml")).unwrap(),
		before_manifest
	);
}

#[test]
fn missing_selected_backend_refuses_before_generator_writes() {
	let root = fixture();
	let app = root.path().join("apps/website");
	let config: leptoskit_config::LeptoskitConfig = toml::from_str(
		"[app]\nname = 'api'\n[build]\nclient_package = 'ui'\nserver_package = 'missing'\nbackend \
		 = true\n[db]\ndialect = 'sqlite'\n",
	)
	.unwrap();
	let manifest = root.path().join("packages/frontend/Cargo.toml");
	let before = std::fs::read(&manifest).unwrap();
	let error = super::super::generate(&app, &config, false, true).unwrap_err();
	assert!(
		error.to_string().contains("backend package `missing`"),
		"{error:#}"
	);
	assert!(
		!root
			.path()
			.join("packages/frontend/interface/auth")
			.exists()
	);
	assert_eq!(std::fs::read(&manifest).unwrap(), before);
	assert_eq!(
		std::fs::read_to_string(app.join("leptoskit.toml")).unwrap(),
		""
	);
}

#[test]
fn graph_roots_refuse_auth_generation_without_changing_application_files() {
	let root = fixture();
	let app = root.path().join("apps/website");
	let mut config: leptoskit_config::LeptoskitConfig = toml::from_str(
		&std::fs::read_to_string(
			Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/website-graph.toml"),
		)
		.unwrap(),
	)
	.unwrap();
	config.build.client_package = Some("ui".into());
	config.validate().unwrap();
	let original = toml::to_string(&config).unwrap();
	write(&app, "leptoskit.toml", &original);
	let manifest = root.path().join("packages/frontend/Cargo.toml");
	let before = std::fs::read(&manifest).unwrap();
	let error = super::super::generate(&app, &config, false, true).unwrap_err();
	assert_eq!(
		std::fs::read_to_string(app.join("leptoskit.toml")).unwrap(),
		original
	);
	assert_eq!(std::fs::read(&manifest).unwrap(), before);
	assert!(
		!root
			.path()
			.join("packages/frontend/interface/auth")
			.exists()
	);
	assert!(
		error.to_string().contains("deployment graph root"),
		"{error:#}"
	);
}

#[test]
fn nested_app_globs_inheritance_and_custom_targets_resolve_without_building() {
	let root = fixture();
	let app = root.path().join("apps/website");
	let layout = read_layout(&app, "api", Some("ui"), None).unwrap();
	assert_eq!(
		layout.client.entry,
		root.path()
			.join("packages/frontend/interface/application.rs")
			.canonicalize()
			.unwrap()
	);
	let server = layout.server.unwrap();
	assert_eq!(
		server.entry,
		root.path()
			.join("packages/backend/backend/server.rs")
			.canonicalize()
			.unwrap()
	);
	assert_eq!(
		relative(
			server.entry.parent().unwrap(),
			&app.canonicalize().unwrap().join("leptoskit.toml")
		),
		Path::new("../../../apps/website/leptoskit.toml")
	);
	assert_eq!(
		relative(
			server.manifest.parent().unwrap(),
			&app.canonicalize().unwrap()
		),
		Path::new("../../apps/website")
	);
	assert!(!root.path().join("Cargo.lock").exists());
	assert!(!root.path().join("target").exists());
}

#[test]
fn automatic_client_and_explicit_static_backend_agree_with_cargo_targets() {
	let root = fixture();
	let layout = read_layout(&root.path().join("apps/website"), "api", None, Some(false)).unwrap();
	assert!(
		layout
			.client
			.entry
			.ends_with("frontend/interface/application.rs")
	);
	assert!(layout.server.is_none());
}

#[test]
fn ambiguous_clients_and_binaries_fail_before_any_write() {
	let root = fixture();
	write(
		root.path(),
		"packages/other/Cargo.toml",
		"[package]\nname = \"other\"\nversion = \"0.1.0\"\n[lib]\ncrate-type = [\"cdylib\"]\n",
	);
	write(root.path(), "packages/other/src/lib.rs", "");
	let error = read_layout(root.path(), "api", None, None).unwrap_err();
	assert!(error.to_string().contains("several packages"), "{error:#}");
	let path = root.path().join("packages/backend/Cargo.toml");
	let source = std::fs::read_to_string(&path)
		.unwrap()
		.replace("name = \"api\"\npath", "name = \"worker\"\npath");
	std::fs::write(
		path,
		format!("{source}\n[[bin]]\nname = \"maintenance\"\npath = \"backend/maintenance.rs\"\n"),
	)
	.unwrap();
	write(
		root.path(),
		"packages/backend/backend/maintenance.rs",
		"fn main() {}\n",
	);
	let error = read_layout(root.path(), "api", Some("ui"), None).unwrap_err();
	assert!(error.to_string().contains("several binaries"), "{error:#}");
	assert!(
		!root
			.path()
			.join("packages/frontend/interface/auth")
			.exists()
	);
}

#[test]
fn missing_and_renamed_dependencies_are_rejected_before_generation() {
	let root = fixture();
	let path = root.path().join("packages/frontend/Cargo.toml");
	let source = std::fs::read_to_string(&path).unwrap();
	std::fs::write(&path, source.replace("leptoskit.workspace = true", "")).unwrap();
	let error = read_layout(root.path(), "api", Some("ui"), None).unwrap_err();
	assert!(
		error.to_string().contains("doesn't depend on leptoskit"),
		"{error:#}"
	);
	std::fs::write(
		&path,
		source.replace(
			"leptoskit.workspace = true",
			"kit = { package = \"leptoskit\", path = \"../toolkit\" }",
		),
	)
	.unwrap();
	let error = read_layout(root.path(), "api", Some("ui"), None).unwrap_err();
	assert!(
		error.to_string().contains("keep that dependency name"),
		"{error:#}"
	);
}

#[cfg(unix)]
#[test]
fn source_target_outside_workspace_is_rejected() {
	let root = fixture();
	let outside = tempfile::tempdir().unwrap();
	write(outside.path(), "app.rs", "pub fn app() {}\n");
	let entry = root
		.path()
		.join("packages/frontend/interface/application.rs");
	std::fs::remove_file(&entry).unwrap();
	std::os::unix::fs::symlink(outside.path().join("app.rs"), entry).unwrap();
	let error = read_layout(root.path(), "api", Some("ui"), None).unwrap_err();
	assert!(
		error.to_string().contains("outside the Cargo workspace"),
		"{error:#}"
	);
}

#[test]
fn a_library_only_app_does_not_invent_a_backend_from_dependency_features() {
	let root = fixture();
	let layout = read_layout(root.path(), "ui", Some("ui"), None).unwrap();
	assert!(layout.server.is_none());
	let error = read_layout(root.path(), "ui", Some("ui"), Some(true)).unwrap_err();
	assert!(
		error.to_string().contains("no binary source target"),
		"{error:#}"
	);
}

#[test]
fn a_backend_source_cannot_be_its_own_generated_module() {
	let root = fixture();
	let manifest = root.path().join("packages/backend/Cargo.toml");
	let contents = std::fs::read_to_string(&manifest)
		.unwrap()
		.replace("backend/server.rs", "backend/sign_in.rs");
	std::fs::write(manifest, contents).unwrap();
	std::fs::rename(
		root.path().join("packages/backend/backend/server.rs"),
		root.path().join("packages/backend/backend/sign_in.rs"),
	)
	.unwrap();
	let error = read_layout(root.path(), "api", Some("ui"), None).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("conflicts with the generated module"),
		"{error:#}"
	);
}
