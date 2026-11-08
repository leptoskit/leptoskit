//! `leptoskit generate deploy` against projects written by hand: every
//! target with no database, SQLite, and Postgres, pinned file by file.

use std::fmt::Write as _;
use std::path::Path;

use leptoskit_config::LeptoskitConfig;

use super::Project;
use super::Target;
use super::generate;
use super::generate_with;

const SERVER: &str = r"
use leptoskit::server::Readiness;

fn main() {
	let readiness = Readiness::new();
}
";

#[derive(Clone, Copy, Debug)]
pub(crate) enum Db {
	None,
	Sqlite,
	Postgres,
}

impl Db {
	fn name(self) -> &'static str {
		match self {
			Self::None => "no_db",
			Self::Sqlite => "sqlite",
			Self::Postgres => "postgres",
		}
	}
}

fn write(root: &Path, file: &str, contents: &str) {
	let path = root.join(file);
	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	std::fs::write(path, contents).unwrap();
}

/// A workspace like the scaffold's: the server package at the root, the
/// client in a member. The Postgres project also signs people in.
fn project(root: &Path, db: Db) -> LeptoskitConfig {
	let mut config = String::from(
		"schema_version = 0\n\n[app]\nname = \"notes\"\nversion = \"0.1.0\"\nrender = \
		 \"ssr\"\n\n[serve]\naddress = \"127.0.0.1\"\nport = 3000\nprobes = \
		 true\n\n[build]\nclient_package = \"notes_app\"\n",
	);

	match db {
		Db::None => {}
		Db::Sqlite => config.push_str("\n[db]\ndialect = \"sqlite\"\n"),
		Db::Postgres => {
			config.push_str(
				"\n[db]\ndialect = \"postgres\"\n\n[auth]\nkeys_env = \
				 \"AUTH_ENCRYPTION_KEYS\"\n\n[auth.mail]\nfrom = \"Notes \
				 <hello@notes.example>\"\nsmtp_url_env = \"SMTP_URL\"\n",
			);
		}
	}

	write(root, "leptoskit.toml", &config);
	write(
		root,
		"Cargo.toml",
		"[package]\nname = \"notes\"\nversion = \"0.1.0\"\n\n[workspace]\nmembers = \
		 [\"notes_app\"]\n",
	);
	write(root, "src/main.rs", SERVER);
	write(
		root,
		"notes_app/Cargo.toml",
		"[package]\nname = \"notes_app\"\nversion = \"0.1.0\"\n\n[lib]\ncrate-type = [\"rlib\", \
		 \"cdylib\"]\n",
	);
	write(root, "notes_app/src/lib.rs", "");
	LeptoskitConfig::from_toml(&config, root).expect("config")
}

fn with_version_masked() -> insta::Settings {
	let mut settings = insta::Settings::clone_current();
	settings.add_filter(
		&format!(
			"LEPTOSKIT_VERSION={}",
			regex_escape(env!("CARGO_PKG_VERSION"))
		),
		"LEPTOSKIT_VERSION=[version]",
	);
	settings
}

fn regex_escape(text: &str) -> String {
	text.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() {
				character.to_string()
			} else {
				format!("\\{character}")
			}
		})
		.collect()
}

/// Every target, with every database: the output, then each file.
#[test]
fn every_target_and_database_is_pinned() {
	let _version = with_version_masked().bind_to_scope();
	for target in [
		Target::Docker,
		Target::Compose,
		Target::Fly,
		Target::Kubernetes,
		Target::Systemd,
	] {
		for db in [Db::None, Db::Sqlite, Db::Postgres] {
			let root = tempfile::tempdir().unwrap();
			let config = project(root.path(), db);
			let report = generate(root.path(), &config, target, false).expect("generate");
			let mut pinned = format!("$ leptoskit generate deploy --target {target}\n{report}");

			for (file, _) in target.files(&Project::read(root.path(), &config).unwrap()) {
				let contents = std::fs::read_to_string(root.path().join(&file)).unwrap();

				write!(pinned, "\n===== {file} =====\n{contents}").unwrap();
			}

			insta::assert_snapshot!(format!("{target}_{}", db.name()), pinned);
		}
	}
}

/// Kubernetes with each role its own Deployment, on Postgres with a relay
/// and a replica: the output, then the manifest.
#[test]
fn split_roles_are_pinned() {
	let _version = with_version_masked().bind_to_scope();
	let root = tempfile::tempdir().unwrap();
	project(root.path(), Db::Postgres);
	let toml = std::fs::read_to_string(root.path().join("leptoskit.toml"))
		.unwrap()
		.replace(
			"dialect = \"postgres\"\n",
			"dialect = \"postgres\"\nreplica_url_envs = [\"DATABASE_REPLICA_URL\"]\n",
		)
		+ "\n[rpc.live]\nrelay_url_env = \"NATS_URL\"\n";
	write(root.path(), "leptoskit.toml", &toml);
	let config = LeptoskitConfig::from_toml(&toml, root.path()).unwrap();
	let report = generate_with(root.path(), &config, Target::Kubernetes, false, true).unwrap();
	let manifest = std::fs::read_to_string(root.path().join("kubernetes.yaml")).unwrap();

	insta::assert_snapshot!(
		"kubernetes_split_roles",
		format!(
			"$ leptoskit generate deploy --target kubernetes --split-roles\n{report}\n===== \
			 kubernetes.yaml =====\n{manifest}"
		)
	);
}

#[test]
fn split_roles_need_kubernetes_and_postgres() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::Postgres);
	let error = generate_with(root.path(), &config, Target::Fly, false, true).unwrap_err();
	assert!(error.to_string().contains("--target kubernetes"), "{error}");

	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::Sqlite);
	let error = generate_with(root.path(), &config, Target::Kubernetes, false, true).unwrap_err();
	assert!(error.to_string().contains("needs Postgres"), "{error}");
}

#[test]
fn running_again_changes_nothing_and_says_so() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::Sqlite);
	generate(root.path(), &config, Target::Fly, false).unwrap();
	let fly = std::fs::read_to_string(root.path().join("fly.toml")).unwrap();

	let again = generate(root.path(), &config, Target::Fly, false).unwrap();
	assert!(
		again.changed.is_empty() && again.kept.is_empty(),
		"{again:?}"
	);
	assert!(
		again
			.to_string()
			.starts_with("nothing changed: the fly files are already in place\n\nnext:\n"),
		"{again}"
	);
	assert_eq!(
		std::fs::read_to_string(root.path().join("fly.toml")).unwrap(),
		fly
	);

	// Another target shares the Dockerfile and adds only its own file.
	let compose = generate(root.path(), &config, Target::Compose, false).unwrap();
	assert_eq!(compose.changed, ["created compose.yaml"]);
}

#[test]
fn an_edited_file_is_kept_unless_forced() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::None);
	generate(root.path(), &config, Target::Docker, false).unwrap();
	let dockerfile = root.path().join("Dockerfile");
	let original = std::fs::read_to_string(&dockerfile).unwrap();
	std::fs::write(&dockerfile, "FROM scratch\n").unwrap();

	let kept = generate(root.path(), &config, Target::Docker, false).unwrap();
	assert_eq!(
		kept.kept,
		["kept Dockerfile: it differs from the template; --force replaces it"]
	);
	assert!(kept.to_string().contains("nothing changed"), "{kept}");
	assert_eq!(
		std::fs::read_to_string(&dockerfile).unwrap(),
		"FROM scratch\n"
	);

	let forced = generate(root.path(), &config, Target::Docker, true).unwrap();
	assert_eq!(forced.changed, ["replaced Dockerfile"]);
	assert_eq!(forced.kept, [] as [String; 0]);
	assert_eq!(std::fs::read_to_string(&dockerfile).unwrap(), original);
}

#[test]
fn a_directory_in_the_way_is_an_error() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::None);
	std::fs::create_dir(root.path().join("Dockerfile")).unwrap();
	let error = generate(root.path(), &config, Target::Docker, true).unwrap_err();
	assert_eq!(error.to_string(), "Dockerfile isn't an ordinary file");
}

#[test]
fn a_server_without_probes_is_told_how_to_add_them() {
	let root = tempfile::tempdir().unwrap();
	let mut config = project(root.path(), Db::None);
	config.serve.probes = false;
	write(
		root.path(),
		"src/main.rs",
		"fn main() { /* Readiness readyz */ }\n",
	);
	let report = generate(root.path(), &config, Target::Docker, false).unwrap();
	assert!(
		report.next[0].starts_with("Backend probes are not declared"),
		"{report}"
	);
}

#[test]
fn the_project_is_read_from_its_manifests() {
	let root = tempfile::tempdir().unwrap();
	let mut config = project(root.path(), Db::None);
	let toolchain_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rust-toolchain.toml");
	let toolchain = std::fs::read_to_string(&toolchain_file).unwrap();
	write(root.path(), "rust-toolchain.toml", &toolchain);
	let rust_version = super::toolchain_channel(&toolchain_file).unwrap_or_else(|| "1".to_owned());
	write(
		root.path(),
		"notes_app/Cargo.toml",
		"[package]\nname = \"notes_app\"\n\n[lib]\nname = \"notes_client\"\n",
	);
	config.app.platforms = vec![leptoskit_config::Platform::Ios];

	config.serve.port = 8080;
	let project = Project::read(root.path(), &config).unwrap();
	assert_eq!(project.binary, "notes");

	assert_eq!(project.output_name, "notes_client");
	assert_eq!(project.port, 8080);
	assert_eq!(project.platform, leptoskit_config::Platform::Server);
	assert_eq!(
		project.toolchain_file.as_deref(),
		Some("rust-toolchain.toml")
	);
	assert_eq!(project.rust_version, rust_version);
	assert!(project.has_probes);

	// A server-only build skips the wasm toolchain and binaryen, and the
	// toolchain file comes in before anything is built.
	let dockerfile = super::dockerfile(&project);
	assert!(!dockerfile.contains("wasm"), "{dockerfile}");
	assert!(dockerfile.contains(&format!("ARG RUST_VERSION={rust_version}\n")));
	assert!(dockerfile.contains("COPY rust-toolchain.toml ./\nCOPY . .\n"));
	assert!(dockerfile.contains("leptoskit build --release --platform server"));
	assert!(dockerfile.contains("EXPOSE 8080\n"));

	// Without a client package, the output name is the app's.
	config.build.client_package = None;
	assert_eq!(
		Project::read(root.path(), &config).unwrap().output_name,
		"notes"
	);
}

#[test]
fn toolchain_channels_pin_the_image_only_when_they_name_a_release() {
	let root = tempfile::tempdir().unwrap();
	let file = root.path().join("rust-toolchain");
	for (contents, expected) in [
		("1.96\n", Some("1.96")),
		("stable\n", None),
		("nightly-2026-10-01\n", None),
		("[toolchain]\nchannel = \"beta\"\n", None),
		("[toolchain]\nprofile = \"minimal\"\n", None),
		("1..2\n", None),
		("\n", None),
	] {
		std::fs::write(&file, contents).unwrap();
		assert_eq!(
			super::toolchain_channel(&file).as_deref(),
			expected,
			"{contents:?}"
		);
	}
	assert_eq!(super::toolchain_channel(&root.path().join("missing")), None);
}

#[test]
fn the_binary_is_the_one_named_after_the_package_or_the_only_one() {
	let mut package = crate::wasm::MetadataPackage {
		name: "notes".to_owned(),
		manifest_path: Path::new("notes/Cargo.toml").to_owned(),
		targets: vec![crate::wasm::MetadataTarget {
			name: "serve".to_owned(),
			kind: vec!["bin".to_owned()],
		}],
		features: std::collections::BTreeMap::default(),
		dependencies: Vec::new(),
		metadata: serde_json::Value::Null,
	};
	assert_eq!(super::binary(&package).unwrap(), "serve");
	package.targets.push(crate::wasm::MetadataTarget {
		name: "notes".to_owned(),
		kind: vec!["bin".to_owned()],
	});
	assert_eq!(super::binary(&package).unwrap(), "notes");
	package.targets[1].name = "other".to_owned();
	let error = super::binary(&package).unwrap_err().to_string();
	assert!(
		error.starts_with("the server package `notes` has several binaries"),
		"{error}"
	);
	package.targets.clear();

	package.targets.push(crate::wasm::MetadataTarget {
		name: "notes".to_owned(),
		kind: vec!["lib".to_owned()],
	});
	let error = super::binary(&package).unwrap_err().to_string();
	assert!(
		error.starts_with("the server package `notes` has no binary"),
		"{error}"
	);
}

#[test]
fn what_is_missing_is_named() {
	let root = tempfile::tempdir().unwrap();
	let mut config = project(root.path(), Db::None);

	config.app.render = leptoskit_config::RenderMode::Csr;
	assert!(Project::read(root.path(), &config).is_ok());
	let client = config.build.client_package.take();
	assert!(Project::read(root.path(), &config).is_ok());
	config.build.backend = Some(false);
	config.serve.probes = false;
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert!(
		error.starts_with("build.backend = false selects a static app"),
		"{error}"
	);

	config.build.backend = Some(true);
	config.app.render = leptoskit_config::RenderMode::Islands;
	config.build.client_package = client;
	config.build.client_package = Some("missing_app".into());
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert_eq!(
		error,
		"build.client_package `missing_app` in leptoskit.toml isn't a package in the workspace"
	);

	config.app.name = "missing".into();
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert!(
		error.starts_with(
			"app.name `missing` in leptoskit.toml must name the server package, but there's no \
			 package called `missing` in "
		),
		"{error}"
	);

	write(root.path(), "Cargo.toml", "[package\n");
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert!(error.starts_with("could not parse "), "{error}");

	std::fs::remove_file(root.path().join("Cargo.toml")).unwrap();
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert!(error.starts_with("there's no Cargo.toml at "), "{error}");
}

#[test]
fn probes_depend_on_the_declared_contract_not_source_spelling() {
	let root = tempfile::tempdir().unwrap();
	let mut config = project(root.path(), Db::None);
	write(root.path(), "src/main.rs", "fn main() { server::run(); }\n");
	assert!(Project::read(root.path(), &config).unwrap().has_probes);
	config.serve.probes = false;
	write(
		root.path(),
		"src/main.rs",
		"// TODO: Readiness /readyz\nfn main() {}\n",
	);
	assert!(!Project::read(root.path(), &config).unwrap().has_probes);
}

/// A Cargo workspace whose app is a directory of member crates with no
/// manifest of its own, the shape Huddle has.
fn nested_project(root: &Path, app: &str) -> LeptoskitConfig {
	write(
		root,
		"Cargo.toml",
		"[workspace]\nmembers = [\"apps/*/server\", \"apps/*/client\"]\nresolver = \
		 \"2\"\n\n[workspace.package]\nedition = \"2024\"\n\n[workspace.dependencies]\nanyhow = \
		 \"1\"\n",
	);
	let app_root = root.join("apps").join(app);
	let config = String::from(
		"schema_version = 0\n\n[app]\nname = \"huddle\"\nversion = \"0.1.0\"\nrender = \
		 \"csr\"\nplatforms = [\"web\"]\n\n[serve]\naddress = \"127.0.0.1\"\nport = 3000\nprobes \
		 = true\n\n[build]\nclient_package = \"huddle_app\"\n",
	);
	write(&app_root, "leptoskit.toml", &config);
	write(
		&app_root,
		"server/Cargo.toml",
		"[package]\nname = \"huddle\"\nversion = \"0.1.0\"\nedition.workspace = \
		 true\n\n[dependencies]\nanyhow.workspace = true\n",
	);
	write(&app_root, "server/src/main.rs", SERVER);
	write(
		&app_root,
		"client/Cargo.toml",
		"[package]\nname = \"huddle_app\"\nversion = \"0.1.0\"\nedition.workspace = \
		 true\n\n[lib]\ncrate-type = [\"rlib\", \"cdylib\"]\n",
	);
	write(&app_root, "client/src/lib.rs", "");

	LeptoskitConfig::from_toml(&config, &app_root).expect("config")
}

#[test]
fn a_nested_app_resolves_through_cargo_workspace_globs() {
	let root = tempfile::tempdir().unwrap();
	let config = nested_project(root.path(), "site");
	let app = root.path().join("apps/site");
	let project = Project::read(&app, &config).unwrap();
	let workspace = std::fs::canonicalize(root.path()).unwrap();

	assert_eq!(project.build_context(), workspace.as_path());
	assert_eq!(project.app_dir, "apps/site");
	assert_eq!(project.context, "../..");
	assert_eq!(project.dockerfile, "apps/site/Dockerfile");
	assert_eq!(project.binary, "huddle");
	assert_eq!(project.output_name, "huddle_app");
	assert_eq!(project.toolchain_file, None);
	assert_eq!(project.rust_version, "1");

	assert_eq!(project.target_dir, "/src/target");

	let report = generate(&app, &config, Target::Docker, false).unwrap();
	assert!(
		report.changed.contains(&"created Dockerfile".to_owned()),
		"{report}"
	);
	assert!(
		report
			.changed
			.contains(&"created Dockerfile.dockerignore".to_owned()),
		"{report}"
	);
	assert!(!app.join(".dockerignore").exists());

	let dockerfile = std::fs::read_to_string(app.join("Dockerfile")).unwrap();
	assert!(
		dockerfile.contains("docker build -f apps/site/Dockerfile -t huddle ."),
		"{dockerfile}"
	);
	assert!(
		dockerfile.contains(
			"RUN mkdir -p apps/site && cd apps/site && rustup target add wasm32-unknown-unknown"
		),
		"{dockerfile}"
	);
	assert!(
		dockerfile.contains("--mount=type=cache,target=/src/apps/site/.leptoskit"),
		"{dockerfile}"
	);
	assert!(
		dockerfile.contains(
			"cd apps/site && CARGO_TARGET_DIR=/src/target leptoskit build --release --platform web"
		),
		"{dockerfile}"
	);
	assert!(
		dockerfile.contains("cp /src/target/release/huddle /usr/local/bin/huddle"),
		"{dockerfile}"
	);
	assert!(
		dockerfile.contains("COPY --from=build /src/apps/site/dist /app/dist"),
		"{dockerfile}"
	);
	assert!(
		!dockerfile.contains("COPY --from=build /src/dist /app/dist"),
		"{dockerfile}"
	);

	let ignore = std::fs::read_to_string(app.join("Dockerfile.dockerignore")).unwrap();
	for pattern in [
		"**/target",
		"**/.leptoskit",
		"**/dist",
		"**/*.sqlite3",
		"**/.env.*",
		"**/infra/**/*.tfstate*",
		"**/.kamal",
	] {
		assert!(ignore.contains(pattern), "{pattern}: {ignore}");
	}
}

#[test]
fn nested_platform_commands_use_the_workspace_context() {
	let root = tempfile::tempdir().unwrap();
	let config = nested_project(root.path(), "site");
	let app = root.path().join("apps/site");
	let compose = generate(&app, &config, Target::Compose, false).unwrap();
	let text = compose.to_string();
	assert!(
		text.contains("docker compose -f apps/site/compose.yaml up --build"),
		"{text}"
	);
	let yaml = std::fs::read_to_string(app.join("compose.yaml")).unwrap();
	assert!(yaml.contains("      context: \"../..\"\n"), "{yaml}");

	assert!(
		yaml.contains("      dockerfile: \"apps/site/Dockerfile\"\n"),
		"{yaml}"
	);

	generate(&app, &config, Target::Fly, false).unwrap();
	let fly = std::fs::read_to_string(app.join("fly.toml")).unwrap();
	assert!(
		fly.contains("dockerfile = \"apps/site/Dockerfile\"\n"),
		"{fly}"
	);
	let fly_next = generate(&app, &config, Target::Fly, false)
		.unwrap()
		.next
		.join("\n");
	assert!(
		fly_next
			.contains("fly deploy --config apps/site/fly.toml --dockerfile apps/site/Dockerfile"),
		"{fly_next}"
	);

	let kubernetes_next = generate(&app, &config, Target::Kubernetes, false)
		.unwrap()
		.next
		.join("\n");
	assert!(
		kubernetes_next.contains("docker build -f apps/site/Dockerfile -t huddle ."),
		"{kubernetes_next}"
	);
}

#[test]
fn workspace_paths_with_spaces_are_quoted_and_kept_inside_the_context() {
	let root = tempfile::tempdir().unwrap();
	let config = nested_project(root.path(), "my app");
	let app = root.path().join("apps/my app");
	let toolchain = std::fs::read_to_string(
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rust-toolchain.toml"),
	)
	.unwrap();
	write(&app, "rust-toolchain.toml", &toolchain);
	let project = Project::read(&app, &config).unwrap();
	assert_eq!(project.app_dir, "apps/my app");
	assert_eq!(
		project.toolchain_file.as_deref(),
		Some("apps/my app/rust-toolchain.toml")
	);

	assert_eq!(project.rust_version, "1");
	assert_eq!(
		project.docker_build_command(),
		"docker build -f 'apps/my app/Dockerfile' -t huddle ."
	);

	generate(&app, &config, Target::Compose, false).unwrap();
	let dockerfile = std::fs::read_to_string(app.join("Dockerfile")).unwrap();
	assert!(
		dockerfile.contains("COPY [\"apps/my app/rust-toolchain.toml\", \"apps/my app/\"]"),
		"{dockerfile}"
	);
	assert!(
		dockerfile.contains(
			"RUN mkdir -p 'apps/my app' && cd 'apps/my app' && rustup target add \
			 wasm32-unknown-unknown"
		),
		"{dockerfile}"
	);
	assert!(
		dockerfile.contains("cd 'apps/my app' && CARGO_TARGET_DIR=/src/target"),
		"{dockerfile}"
	);
	assert!(
		!dockerfile.contains("my app/.leptoskit"),
		"a cache path with whitespace is left out: {dockerfile}"
	);
	assert!(
		dockerfile.contains("COPY --from=build [\"/src/apps/my app/dist\", \"/app/dist\"]"),
		"{dockerfile}"
	);
	let yaml = std::fs::read_to_string(app.join("compose.yaml")).unwrap();

	assert!(yaml.contains("      context: \"../..\"\n"), "{yaml}");
	assert!(
		yaml.contains("      dockerfile: \"apps/my app/Dockerfile\"\n"),
		"{yaml}"
	);
	let next = generate(&app, &config, Target::Compose, false)
		.unwrap()
		.next
		.join("\n");
	assert!(
		next.contains("docker compose -f 'apps/my app/compose.yaml' up --build"),
		"{next}"
	);

	generate(&app, &config, Target::Fly, false).unwrap();
	let fly = std::fs::read_to_string(app.join("fly.toml")).unwrap();
	assert!(
		fly.contains("dockerfile = \"apps/my app/Dockerfile\"\n"),
		"{fly}"
	);
	let next = generate(&app, &config, Target::Fly, false)
		.unwrap()
		.next
		.join("\n");
	assert!(
		next.contains("--dockerfile 'apps/my app/Dockerfile'"),
		"{next}"
	);
}

#[test]
fn the_workspace_ignore_keeps_project_exclusions_without_touching_the_root() {
	let root = tempfile::tempdir().unwrap();
	let config = nested_project(root.path(), "site");
	let app = root.path().join("apps/site");
	write(root.path(), ".dockerignore", "workspace-secret\n");
	write(&app, ".dockerignore", "app-secret\n!published.txt\n");

	generate(&app, &config, Target::Docker, false).unwrap();
	let ignore = std::fs::read_to_string(app.join("Dockerfile.dockerignore")).unwrap();
	assert!(ignore.contains("\nworkspace-secret\n"), "{ignore}");
	assert!(ignore.contains("\napps/site/app-secret\n"), "{ignore}");
	assert!(ignore.contains("\n!apps/site/published.txt\n"), "{ignore}");
	assert_eq!(
		std::fs::read_to_string(root.path().join(".dockerignore")).unwrap(),
		"workspace-secret\n"
	);
	assert_eq!(
		std::fs::read_to_string(app.join(".dockerignore")).unwrap(),
		"app-secret\n!published.txt\n"
	);

	write(&app, "Dockerfile.dockerignore", "custom-ignore\n");
	let report = generate(&app, &config, Target::Docker, false).unwrap();
	assert_eq!(
		report.kept,
		["kept Dockerfile.dockerignore: it differs from the template; --force replaces it"]
	);
	assert_eq!(
		std::fs::read_to_string(app.join("Dockerfile.dockerignore")).unwrap(),
		"custom-ignore\n"
	);
}

#[test]
fn an_app_outside_its_workspace_is_refused_before_it_becomes_a_context() {
	let error = super::app_relative_dir(Path::new("/workspace"), Path::new("/outside/app"))
		.unwrap_err()
		.to_string();
	assert!(error.contains("outside its Cargo workspace"), "{error}");
	assert!(error.contains("can only include files inside"), "{error}");

	let error = super::package_directory(
		Path::new("/workspace"),
		"notes",
		Path::new("/outside/notes/Cargo.toml"),
	)
	.unwrap_err()
	.to_string();
	assert!(error.contains("the package `notes`"), "{error}");
	assert!(error.contains("outside the Cargo workspace"), "{error}");
}

#[test]
fn managed_state_patterns_follow_the_app_into_a_workspace() {
	let root = tempfile::tempdir().unwrap();
	let config = nested_project(root.path(), "site");
	let resolved = Project::read(&root.path().join("apps/site"), &config).unwrap();
	assert_eq!(
		resolved.managed_state_patterns(),
		[
			"/apps/site/infra".to_owned(),
			"/apps/site/.kamal".to_owned()
		]
	);
	assert_eq!(
		resolved.ignore_path(),
		std::fs::canonicalize(root.path())
			.unwrap()
			.join("apps/site/Dockerfile.dockerignore")
	);

	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::None);
	let resolved = Project::read(root.path(), &config).unwrap();
	assert_eq!(
		resolved.managed_state_patterns(),
		["/infra".to_owned(), "/.kamal".to_owned()]
	);
	assert_eq!(
		resolved.ignore_path(),
		std::fs::canonicalize(root.path())
			.unwrap()
			.join(".dockerignore")
	);
}

#[test]
fn huddle_resolves_inside_the_repository_workspace() {
	let repository =
		std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
	let app = repository.join("examples/huddle");
	let raw = std::fs::read_to_string(app.join("leptoskit.toml")).unwrap();
	let config = LeptoskitConfig::from_toml(&raw, &app).unwrap();
	let project = Project::read(&app, &config).unwrap();

	assert_eq!(project.app, "huddle");
	assert_eq!(project.binary, "huddle");
	assert_eq!(project.output_name, "huddle_app");
	assert_eq!(project.app_dir, "examples/huddle");
	assert_eq!(project.context, "../..");
	assert_eq!(project.dockerfile, "examples/huddle/Dockerfile");
	assert_eq!(project.build_context(), repository.as_path());
	assert_eq!(
		project.toolchain_file.as_deref(),
		Some("rust-toolchain.toml")
	);

	assert_eq!(project.target_dir, "/src/target");

	let dockerfile = super::dockerfile(&project);
	assert!(
		dockerfile.contains("COPY rust-toolchain.toml ./"),
		"{dockerfile}"
	);
	assert!(
		dockerfile.contains("cd examples/huddle && CARGO_TARGET_DIR=/src/target leptoskit build"),
		"{dockerfile}"
	);
	assert!(
		dockerfile.contains("COPY --from=build /src/examples/huddle/dist /app/dist"),
		"{dockerfile}"
	);
}

#[cfg(unix)]
#[test]
fn a_package_symlink_cannot_escape_the_workspace_context() {
	let root = tempfile::tempdir().unwrap();
	let external = tempfile::tempdir().unwrap();
	let workspace = root.path().canonicalize().unwrap();
	std::os::unix::fs::symlink(external.path(), workspace.join("linked")).unwrap();
	let error =
		super::package_directory(&workspace, "linked", &workspace.join("linked/Cargo.toml"))
			.unwrap_err()
			.to_string();
	assert!(error.contains("outside the Cargo workspace"), "{error}");
}

#[test]
fn unreadable_ignore_policy_stops_generation() {
	let root = tempfile::tempdir().unwrap();
	let config = nested_project(root.path(), "site");
	let app = root.path().join("apps/site");
	std::fs::create_dir(root.path().join(".dockerignore")).unwrap();
	let error = generate(&app, &config, Target::Docker, false)
		.unwrap_err()
		.to_string();
	assert!(error.contains("could not read"), "{error}");
	assert!(!app.join("Dockerfile").exists());
}

#[test]
fn custom_ignore_exceptions_do_not_reintroduce_managed_state() {
	let root = tempfile::tempdir().unwrap();
	let config = nested_project(root.path(), "site");
	let app = root.path().join("apps/site");
	write(root.path(), ".dockerignore", "!**\n");
	write(&app, ".dockerignore", "!.env\n");
	generate(&app, &config, Target::Docker, false).unwrap();
	let ignore = std::fs::read_to_string(app.join("Dockerfile.dockerignore")).unwrap();
	let exception = ignore.find("!apps/site/.env").unwrap();

	for pattern in ["**/.env", "**/.kamal", "**/infra/**/*.tfstate*"] {
		assert!(
			ignore.rfind(pattern).unwrap() > exception,
			"{pattern}: {ignore}"
		);
	}
}

#[test]
fn deployment_uses_cargos_discovered_and_explicit_binary_targets() {
	let root = tempfile::tempdir().unwrap();
	let config = nested_project(root.path(), "site");
	let app = root.path().join("apps/site");
	std::fs::remove_file(app.join("server/src/main.rs")).unwrap();
	write(&app, "server/src/bin/serve.rs", "fn main() {}\n");
	assert_eq!(Project::read(&app, &config).unwrap().binary, "serve");

	// An ignored src/main.rs must not turn the declared single executable
	// into a different package-named executable in the deployment image.
	write(
		&app,
		"server/src/main.rs",
		"compile_error!(\"autobins is disabled\");\n",
	);
	write(
		&app,
		"server/Cargo.toml",
		"[package]\nname = \"huddle\"\nversion = \"0.1.0\"\nedition.workspace = true\nautobins = \
		 false\n\n[[bin]]\nname = \"serve\"\npath = \"src/bin/serve.rs\"\n",
	);
	assert_eq!(Project::read(&app, &config).unwrap().binary, "serve");
}

#[test]
fn external_path_dependencies_fail_before_writing_deployment_files() {
	let root = tempfile::tempdir().unwrap();
	let external = tempfile::tempdir().unwrap();
	let config = nested_project(root.path(), "notes");
	write(
		external.path(),
		"Cargo.toml",
		"[package]\nname = \"external\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[workspace]\n",
	);
	write(external.path(), "src/lib.rs", "");
	let manifest = root.path().join("apps/notes/server/Cargo.toml");
	let mut text = std::fs::read_to_string(&manifest).unwrap();
	writeln!(text, "external = {{ path = {:?} }}", external.path()).unwrap();

	std::fs::write(manifest, text).unwrap();
	let app = root.path().join("apps/notes");
	let error = generate(&app, &config, Target::Docker, false).unwrap_err();
	assert!(format!("{error:#}").contains("outside the Cargo workspace"));
	assert!(!app.join("Dockerfile").exists());
}

#[test]
fn nested_recipes_offer_an_explicit_workspace_cli_source() {
	let root = tempfile::tempdir().unwrap();
	let config = nested_project(root.path(), "notes");
	let project = Project::read(&root.path().join("apps/notes"), &config).unwrap();
	let recipe = super::dockerfile(&project);
	assert!(recipe.contains("ARG LEPTOSKIT_CLI_PATH="));
	assert!(recipe.contains("cargo install --path \"$cli\" --locked --bin leptoskit"));
	assert!(recipe.find("COPY . .").unwrap() < recipe.find("cargo install --path").unwrap());
	assert!(recipe.contains("CARGO_TARGET_DIR=/src/target"));
}

#[test]
fn standalone_custom_target_is_rejected_before_writing() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::None);
	write(
		root.path(),
		".cargo/config.toml",
		"[build]\ntarget-dir = \"custom-output\"\n",
	);
	let error = generate(root.path(), &config, Target::Docker, false).unwrap_err();
	assert!(format!("{error:#}").contains("remove build.target-dir"));
	assert!(!root.path().join("Dockerfile").exists());
}

#[test]
fn nested_custom_target_is_overridden_in_the_container() {
	let root = tempfile::tempdir().unwrap();
	let config = nested_project(root.path(), "notes");
	write(
		root.path(),
		".cargo/config.toml",
		"[build]\ntarget-dir = \"../outside-target\"\n",
	);
	let project = Project::read(&root.path().join("apps/notes"), &config).unwrap();
	assert_eq!(project.target_dir, "/src/target");
	assert!(super::dockerfile(&project).contains("CARGO_TARGET_DIR=/src/target"));
}

#[test]
fn an_excluded_local_dependency_cannot_hide_an_external_dependency() {
	let root = tempfile::tempdir().unwrap();
	let external = tempfile::tempdir().unwrap();
	let config = nested_project(root.path(), "notes");
	let manifest = root.path().join("Cargo.toml");
	let workspace = std::fs::read_to_string(&manifest).unwrap();
	std::fs::write(
		manifest,
		workspace.replacen("[workspace]", "[workspace]\nexclude = [\"local\"]", 1),
	)
	.unwrap();
	write(
		external.path(),
		"Cargo.toml",
		"[package]\nname = \"external\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[workspace]\n",
	);
	write(external.path(), "src/lib.rs", "");

	let mut local = String::from(
		"[package]\nname = \"local\"\nversion = \"0.1.0\"\nedition = \
		 \"2024\"\n[workspace]\n[dependencies]\n",
	);
	writeln!(local, "external = {{ path = {:?} }}", external.path()).unwrap();
	write(root.path(), "local/Cargo.toml", &local);
	write(root.path(), "local/src/lib.rs", "");
	let manifest = root.path().join("apps/notes/server/Cargo.toml");
	let mut server = std::fs::read_to_string(&manifest).unwrap();
	server.push_str("local = { path = \"../../../local\" }\n");
	std::fs::write(manifest, server).unwrap();
	let app = root.path().join("apps/notes");
	let error = generate(&app, &config, Target::Docker, false).unwrap_err();
	assert!(format!("{error:#}").contains("outside the Cargo workspace"));
	assert!(!app.join("Dockerfile").exists());
}

/// The notes project with services: `analytics`, a public workspace
/// service with a database, a volume, and bindings; `cards`, a private one;
/// and `billing`, which runs elsewhere. `trust` is analytics's.
pub(crate) fn project_with_services(root: &Path, db: Db, trust: &str) -> LeptoskitConfig {
	let db = match db {
		Db::None => String::new(),
		Db::Sqlite => "[db]\ndialect = \"sqlite\"\n\n".to_owned(),
		Db::Postgres => "[db]\ndialect = \"postgres\"\n\n".to_owned(),
	};

	let config = format!(
		"schema_version = 0\n\n[app]\nname = \"notes\"\nversion = \"0.1.0\"\nrender = \
		 \"ssr\"\n\n[serve]\naddress = \"127.0.0.1\"\nport = 3000\nprobes = \
		 true\n\n[build]\nclient_package = \"notes_app\"\n\n{db}[services.analytics]\npackage = \
		 \"analytics_service\"\ntrust = \"{trust}\"\n\n[services.cards]\npackage = \
		 \"cards\"\n\n[services.billing]\nurl = \"https://billing.internal.example/\"\n"
	);
	write(root, "leptoskit.toml", &config);
	write(
		root,
		"Cargo.toml",
		"[package]\nname = \"notes\"\nversion = \"0.1.0\"\nedition = \
		 \"2024\"\n\n[workspace]\nmembers = [\"notes_app\", \"analytics_service\", \"cards\"]\n",
	);
	write(root, "src/main.rs", SERVER);
	write(
		root,
		"notes_app/Cargo.toml",
		"[package]\nname = \"notes_app\"\nversion = \"0.1.0\"\nedition = \
		 \"2024\"\n\n[lib]\ncrate-type = [\"rlib\", \"cdylib\"]\n",
	);
	write(root, "notes_app/src/lib.rs", "");
	write(
		root,
		"analytics_service/Cargo.toml",
		"[package]\nname = \"analytics_service\"\nversion = \"0.1.0\"\nedition = \
		 \"2024\"\n\n[features]\nservice = []\n\n[package.metadata.leptoskit.service]\nname = \
		 \"analytics\"\nport = 3100\npublic = true\nroutes = [\"/_analytics\"]\ndatabase = \
		 true\npool = 4\nfeatures = [\"service\"]\nvolumes = [\"exports\"]\nsecrets = \
		 [\"ANALYTICS_GEOIP_KEY\"]\nenv = { ANALYTICS_RETENTION_DAYS = \"90\" \
		 }\n\n[package.metadata.leptoskit.service.bind]\nANALYTICS_PUBLIC_PATH = \
		 \"{public_path}\"\nANALYTICS_AUTHORIZATION = \"Bearer {token}\"\n",
	);

	write(root, "analytics_service/src/lib.rs", "");
	write(root, "analytics_service/src/main.rs", "fn main() {}\n");
	write(
		root,
		"cards/Cargo.toml",
		"[package]\nname = \"cards\"\nversion = \"0.1.0\"\nedition = \
		 \"2024\"\n\n[package.metadata.leptoskit.service]\nname = \"cards\"\nport = 3200\n",
	);
	write(root, "cards/src/main.rs", "fn main() {}\n");
	LeptoskitConfig::from_toml(&config, root).expect("config")
}

/// Docker, Compose, and Kubernetes with services, on both databases.
#[test]
fn services_are_pinned() {
	let _version = with_version_masked().bind_to_scope();

	for target in [Target::Docker, Target::Compose, Target::Kubernetes] {
		for db in [Db::Sqlite, Db::Postgres] {
			let root = tempfile::tempdir().unwrap();
			// Kubernetes refuses an isolated public service; see below.
			let trust = if target == Target::Kubernetes {
				"first-party"
			} else {
				"isolated"
			};

			let config = project_with_services(root.path(), db, trust);
			let report = generate(root.path(), &config, target, false).expect("generate");
			let mut pinned = format!("$ leptoskit generate deploy --target {target}\n{report}");

			for (file, _) in target.files(&Project::read(root.path(), &config).unwrap()) {
				let contents = std::fs::read_to_string(root.path().join(&file)).unwrap();

				write!(pinned, "\n===== {file} =====\n{contents}").unwrap();
			}

			insta::assert_snapshot!(format!("services_{target}_{}", db.name()), pinned);
		}
	}
}

/// A service's database is a SQLite file on a volume of its own when the
/// app has no `[db]` at all, as `examples/analytics` does: the containers'
/// root filesystems are read-only.
#[test]
fn a_service_database_without_an_app_database_is_on_a_volume() {
	let root = tempfile::tempdir().unwrap();
	let config = project_with_services(root.path(), Db::None, "first-party");
	generate(root.path(), &config, Target::Compose, false).expect("compose");
	let compose = std::fs::read_to_string(root.path().join("compose.yaml")).unwrap();
	assert!(
		compose.contains("      DATABASE_URL: sqlite:///data/analytics.sqlite3\n"),
		"{compose}"
	);
	assert!(
		compose.contains("      - analytics-data:/data\n"),
		"{compose}"
	);
	assert!(
		compose.contains("\nvolumes:\n  analytics-data:\n"),
		"{compose}"
	);
	let dockerfile = std::fs::read_to_string(root.path().join("Dockerfile")).unwrap();
	assert!(
		dockerfile.contains("mkdir -p /data /volumes/exports"),
		"{dockerfile}"
	);

	generate(root.path(), &config, Target::Kubernetes, false).expect("kubernetes");
	let kubernetes = std::fs::read_to_string(root.path().join("kubernetes.yaml")).unwrap();
	let analytics = kubernetes
		.split("\n---\n")
		.find(|manifest| manifest.contains("name: notes-analytics\nspec:\n  replicas:"))
		.expect("the analytics Deployment");
	assert!(
		analytics.contains("  replicas: 1\n  strategy:\n    type: Recreate\n"),
		"{analytics}"
	);
	assert!(
		analytics.contains(
			"            - name: DATABASE_URL\n              value: \
			 \"sqlite:///data/analytics.sqlite3\""
		),
		"{analytics}"
	);
	assert!(
		kubernetes.contains("  name: notes-analytics-data\n"),
		"{kubernetes}"
	);
}

/// Kubernetes names can't hold `_`, which Cargo package names often do, and
/// the kubelet can only check a numeric user isn't root.
#[test]
fn kubernetes_names_and_users_suit_the_kubelet() {
	let root = tempfile::tempdir().unwrap();
	project_with_services(root.path(), Db::Sqlite, "first-party");
	let toml = std::fs::read_to_string(root.path().join("leptoskit.toml"))
		.unwrap()
		.replace("name = \"notes\"", "name = \"my_notes\"");
	write(root.path(), "leptoskit.toml", &toml);
	let cargo = std::fs::read_to_string(root.path().join("Cargo.toml"))
		.unwrap()
		.replace("name = \"notes\"", "name = \"my_notes\"");
	write(root.path(), "Cargo.toml", &cargo);
	let config = LeptoskitConfig::from_toml(&toml, root.path()).expect("config");
	generate(root.path(), &config, Target::Kubernetes, false).expect("kubernetes");
	let kubernetes = std::fs::read_to_string(root.path().join("kubernetes.yaml")).unwrap();

	// Resource names, container names, labels, claims, and the Ingress's
	// backends; environment variables keep their `_`.
	for line in kubernetes.lines() {
		let name = [
			"  name: ",
			"        - name: ",
			"                name: ",
			"            claimName: ",
			"      app: ",
			"    app: ",
			"        app: ",
		]
		.into_iter()
		.find_map(|prefix| line.strip_prefix(prefix));

		if let Some(name) = name {
			assert!(!name.contains('_'), "{line}");
		}
	}

	assert!(
		kubernetes.contains("          image: my_notes\n"),
		"{kubernetes}"
	);
	assert!(
		kubernetes.contains("\"http://my-notes-analytics:3100\""),
		"{kubernetes}"
	);
	assert!(
		kubernetes.contains("\"http://my-notes:80\""),
		"{kubernetes}"
	);
	let pods = kubernetes
		.matches("      terminationGracePeriodSeconds: 30\n")
		.count();
	assert_eq!(
		kubernetes.matches("      securityContext:\n        runAsUser: 10001\n        runAsGroup: 10001\n        fsGroup: 10001\n").count(),
		pods,
		"{kubernetes}"
	);
}

#[test]
fn the_services_are_read_from_their_manifests() {
	let root = tempfile::tempdir().unwrap();
	let config = project_with_services(root.path(), Db::Postgres, "first-party");
	let project = Project::read(root.path(), &config).unwrap();
	let names: Vec<&str> = project
		.services
		.iter()
		.map(|service| service.name.as_str())
		.collect();
	assert_eq!(names, ["analytics", "billing", "cards"]);

	let analytics = &project.services[0];
	assert_eq!(
		analytics.build_path.as_deref(),
		Some("target/release/analytics_service")
	);
	assert_eq!(analytics.port, Some(3100));
	assert_eq!(
		analytics.public_prefixes,
		["/_leptoskit/services/analytics", "/_analytics"]
	);
	assert!(analytics.database);
	assert_eq!(analytics.pool, Some(4));

	let billing = &project.services[1];
	assert!(!billing.is_process());
	assert_eq!(
		billing.url.as_deref(),
		Some("https://billing.internal.example")
	);
}

/// Kubernetes with each role its own Deployment still sends the live paths
/// to the live Service once there's an Ingress for the services.
#[test]
fn split_roles_route_the_live_paths_beside_the_services() {
	let root = tempfile::tempdir().unwrap();
	let config = project_with_services(root.path(), Db::Postgres, "first-party");
	generate_with(root.path(), &config, Target::Kubernetes, false, true).unwrap();
	let manifest = std::fs::read_to_string(root.path().join("kubernetes.yaml")).unwrap();
	let ingress = &manifest[manifest.find("kind: Ingress").unwrap()..];
	let analytics = ingress
		.find("path: /_leptoskit/services/analytics")
		.unwrap();
	let live = ingress.find("path: /live").unwrap();
	let root_path = ingress.find("path: /\n").unwrap();

	assert!(analytics < live && live < root_path, "{ingress}");
	assert!(manifest.contains("name: notes-cards"));
}

#[test]
fn kubernetes_refuses_an_isolated_public_service() {
	let root = tempfile::tempdir().unwrap();
	let config = project_with_services(root.path(), Db::Postgres, "isolated");
	let error = generate(root.path(), &config, Target::Kubernetes, false)
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("services.analytics is isolated and public") && error.contains("Cookie"),
		"{error}"
	);
}

#[test]
fn fly_and_systemd_refuse_services_they_cannot_run() {
	for target in [Target::Fly, Target::Systemd] {
		let root = tempfile::tempdir().unwrap();
		let config = project_with_services(root.path(), Db::Postgres, "first-party");
		let error = generate(root.path(), &config, target, false).unwrap_err();
		assert!(
			error.to_string().contains("services.analytics")
				&& error.to_string().contains("--target compose"),
			"{error}"
		);
	}
}

#[test]
fn the_connection_budget_counts_every_pool() {
	let root = tempfile::tempdir().unwrap();
	let config = project_with_services(root.path(), Db::Postgres, "first-party");
	let project = Project::read(root.path(), &config).unwrap();
	// The app: (1 host + 1 during a deploy) × (8 + 1). analytics: 2 × (4 +
	// 1). cards has no database. 5 kept back.
	let budget = super::services::Budget::new(8, 1, &project.services, 100);
	assert_eq!(budget.total, 18 + 10 + 5);
	assert!(budget.fits());
	let tight = super::services::Budget::new(8, 1, &project.services, 30);
	assert!(!tight.fits());

	assert!(tight.explain().contains("services.analytics"));
}

/// A nested app's services build into the workspace's target directory, and
/// their Compose entries build the app's image from the workspace root.
#[test]
fn a_nested_apps_services_build_from_the_workspace() {
	let root = tempfile::tempdir().unwrap();
	nested_project(root.path(), "site");
	write(
		root.path(),
		"Cargo.toml",
		"[workspace]\nmembers = [\"apps/*/server\", \"apps/*/client\", \
		 \"apps/*/analytics\"]\nresolver = \"2\"\n\n[workspace.package]\nedition = \
		 \"2024\"\n\n[workspace.dependencies]\nanyhow = \"1\"\n",
	);
	let app = root.path().join("apps/site");
	write(
		&app,
		"analytics/Cargo.toml",
		"[package]\nname = \"analytics_service\"\nversion = \"0.1.0\"\nedition.workspace = \
		 true\n\n[package.metadata.leptoskit.service]\nname = \"analytics\"\nport = 3100\n",
	);
	write(&app, "analytics/src/main.rs", "fn main() {}\n");
	let config = std::fs::read_to_string(app.join("leptoskit.toml")).unwrap()
		+ "\n[services.analytics]\npackage = \"analytics_service\"\n";
	write(&app, "leptoskit.toml", &config);

	let config = LeptoskitConfig::from_toml(&config, &app).expect("config");

	generate(&app, &config, Target::Compose, false).unwrap();
	let dockerfile = std::fs::read_to_string(app.join("Dockerfile")).unwrap();
	assert!(
		dockerfile.contains(
			"&& install -D /src/target/release/analytics_service \
			 /usr/local/lib/leptoskit/services/analytics"
		),
		"{dockerfile}"
	);
	assert!(
		dockerfile.contains("COPY --from=build /usr/local/lib/leptoskit/services /app/services"),
		"{dockerfile}"
	);

	let compose = std::fs::read_to_string(app.join("compose.yaml")).unwrap();
	let analytics = &compose[compose.find("\n  analytics:").expect("the service's entry")..];
	assert!(
		analytics.starts_with(
			"\n  analytics:\n    build:\n      context: \"../..\"\n      dockerfile: \
			 \"apps/site/Dockerfile\"\n"
		),
		"{compose}"
	);
}
