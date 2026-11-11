//! `leptoskit generate auth`: writes sign-in screens into an existing app,
//! and wires them up as far as can be done without rewriting the app's own
//! code. `leptoskit new --auth` writes the same files.
//!
//! Every write is idempotent. A file that already matches is left alone; a
//! file that differs is the app's to keep unless `--force` replaces it; and
//! edits to existing files only add what's missing.

use std::fmt;
use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;

mod layout;
mod source;

/// The screens, relative to the client crate's `src/`, and whether each is
/// only for `--idp`. The templates hold every optional part; [`render`]
/// leaves out the ones a [`Mode`] doesn't write.
const PAGES: [(&str, &str, bool); 8] = [
	(
		"auth/mod.rs",
		include_str!("auth_pages/templates/mod.rs"),
		false,
	),
	(
		"auth/sign_in.rs",
		include_str!("auth_pages/templates/sign_in.rs"),
		false,
	),
	(
		"auth/devices.rs",
		include_str!("auth_pages/templates/devices.rs"),
		false,
	),
	(
		"auth/security.rs",
		include_str!("auth_pages/templates/security.rs"),
		false,
	),
	(
		"auth/consent.rs",
		include_str!("auth_pages/templates/consent.rs"),
		true,
	),
	(
		"auth/device_code.rs",
		include_str!("auth_pages/templates/device_code.rs"),
		true,
	),
	(
		"auth/connected_apps.rs",
		include_str!("auth_pages/templates/connected_apps.rs"),
		true,
	),
	(
		"auth/developer_apps.rs",
		include_str!("auth_pages/templates/developer_apps.rs"),
		true,
	),
];

/// The route files for an app that uses the file router, relative to its
/// routes directory: the file, its template, and its `--idp` template, or
/// `None` when it's only for `--idp`.
const ROUTES: [(&str, Option<&str>, Option<&str>); 7] = [
	(
		"sign-in.rs",
		Some(include_str!("auth_pages/templates/route_sign_in.rs")),
		Some(include_str!("auth_pages/templates/route_sign_in_idp.rs")),
	),
	(
		"devices.rs",
		Some(include_str!("auth_pages/templates/route_devices.rs")),
		None,
	),
	(
		"security.rs",
		Some(include_str!("auth_pages/templates/route_security.rs")),
		None,
	),
	(
		"consent.rs",
		None,
		Some(include_str!("auth_pages/templates/route_consent.rs")),
	),
	(
		"device.rs",
		None,
		Some(include_str!("auth_pages/templates/route_device.rs")),
	),
	(
		"connected-apps.rs",
		None,
		Some(include_str!("auth_pages/templates/route_connected_apps.rs")),
	),
	(
		"developer-apps.rs",
		None,
		Some(include_str!("auth_pages/templates/route_developer_apps.rs")),
	),
];

/// The `leptoskit` features the screens use.
const CLIENT_FEATURES: [&str; 3] = ["auth", "leptos", "query"];

/// The `[auth]` table added to a `leptoskit.toml` that has none.
const AUTH_TABLE: &str = "
# Sign-in. Secrets stay in the environment: AUTH_ENCRYPTION_KEYS holds the
# server's keys, as `id:key` pairs.
[auth]
keys_env = \"AUTH_ENCRYPTION_KEYS\"
";

/// The `[auth.idp]` table added for `--idp` when there's none.
const IDP_TABLE: &str = "
# Sign in with this app: other apps, CLIs, and TVs sign people in through it.
[auth.idp]
";

/// The optional parts of the screens a run writes. A template holds each
/// part's lines between `// leptoskit: <part>` and `// leptoskit: end
/// <part>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mode {
	/// The "Sign in with your app" screens, for `--idp`.
	pub(crate) idp: bool,
	/// Password sign-in, reset, and settings, for `[auth.passwords]
	/// enabled`.
	pub(crate) passwords: bool,
}

impl Mode {
	/// Every mode, to recognise a file as another one wrote it.
	const ALL: [Self; 4] = [
		Self {
			idp: false,
			passwords: false,
		},
		Self {
			idp: true,
			passwords: false,
		},
		Self {
			idp: false,
			passwords: true,
		},
		Self {
			idp: true,
			passwords: true,
		},
	];

	/// Whether this mode writes every part `other` does.
	fn covers(self, other: Self) -> bool {
		(self.idp || !other.idp) && (self.passwords || !other.passwords)
	}

	/// What this mode writes that `earlier` didn't, for the report.
	fn added_to(self, earlier: Self) -> &'static str {
		match (
			self.idp && !earlier.idp,
			self.passwords && !earlier.passwords,
		) {
			(true, true) => "what --idp needs and the password fields",
			(true, false) => "what --idp needs",
			_ => "the password fields",
		}
	}
}

/// A template with the parts `mode` doesn't write left out. The marker
/// lines themselves always are.
pub(crate) fn render(template: &str, mode: Mode) -> String {
	let mut out = String::with_capacity(template.len());
	let mut in_idp = false;
	let mut in_passwords = false;

	for line in template.split_inclusive('\n') {
		match line.trim() {
			"// leptoskit: idp" => in_idp = true,
			"// leptoskit: end idp" => in_idp = false,
			"// leptoskit: passwords" => in_passwords = true,
			"// leptoskit: end passwords" => in_passwords = false,

			_ if (in_idp && !mode.idp) || (in_passwords && !mode.passwords) => {}
			_ => out.push_str(line),
		}
	}

	out
}

/// The screens, relative to the client crate's `src/`, as `generate auth`
/// writes them in `mode`.
pub(crate) fn pages(mode: Mode) -> Vec<(&'static str, String)> {
	PAGES
		.iter()
		.filter(|(_, _, only_idp)| mode.idp || !only_idp)
		.map(|(path, template, _)| (*path, render(template, mode)))
		.collect()
}

/// The route files, relative to the routes directory, in `mode`.
pub(crate) fn routes(mode: Mode) -> Vec<(&'static str, String)> {
	ROUTES
		.iter()
		.filter_map(|(file, base, with_idp)| {
			let template = if mode.idp { with_idp.or(*base) } else { *base };
			template.map(|template| (*file, render(template, mode)))
		})
		.collect()
}

/// The server's sign-in module, reading the configuration at `config`,
/// relative to the module, with the provider's builder for `idp`.
pub(crate) fn server_module(config: &str, idp: bool) -> String {
	let mode = Mode {
		idp,
		..Mode::default()
	};
	render(include_str!("auth_pages/templates/server.rs"), mode)
		.replace("__CONFIG_PATH__", &config.escape_default().to_string())
}

/// Embed configuration relative to the actual binary target and app directory.
fn server_module_at(config: &str, root: &str, idp: bool) -> String {
	let source = server_module(config, idp);
	if root == "." {
		return source;
	}
	source.replace(
		"env!(\"CARGO_MANIFEST_DIR\")",
		&format!(
			"concat!(env!(\"CARGO_MANIFEST_DIR\"), {:?})",
			format!("/{root}")
		),
	)
}

/// The same file as every other mode writes it, where it differs: a page
/// written without `--idp` or passwords that a run with them brings up to
/// date, or one written with them that a run without them leaves alone.
fn other_modes(
	file: &str,
	mode: Mode,
	all: fn(Mode) -> Vec<(&'static str, String)>,
) -> Vec<(Mode, String)> {
	Mode::ALL
		.into_iter()
		.filter(|other| *other != mode)
		.filter_map(|other| {
			all(other)
				.into_iter()
				.find(|(path, _)| *path == file)
				.map(|(_, contents)| (other, contents))
		})
		.collect()
}

/// Runs `leptoskit generate auth` in the project around the current
/// directory, and prints what it did and what's left to do. `idp` adds the
/// "Sign in with your app" screens.
///
/// # Errors
///
/// Returns an error when there's no project, its client crate can't be
/// found, or a file can't be read or written.
pub fn run(idp: bool, force: bool) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let report = generate(&root, &config, idp, force)?;
	print!("{report}");
	Ok(())
}

/// What one run did, in the order it did it, and what's left for the
/// developer.
#[derive(Debug, Default)]
pub struct Report {
	/// One line per file created, replaced, or edited.
	pub changed: Vec<String>,
	/// One line per file left as it was because it differs.
	pub kept: Vec<String>,
	/// The steps the generator can't take itself.
	pub next: Vec<String>,
}

impl fmt::Display for Report {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		for line in &self.changed {
			writeln!(f, "{line}")?;
		}

		for line in &self.kept {
			writeln!(f, "{line}")?;
		}

		if self.changed.is_empty() {
			writeln!(
				f,
				"nothing changed: the sign-in screens are already in place"
			)?;
		}

		if !self.next.is_empty() {
			writeln!(f, "\nnext:")?;

			for (number, step) in self.next.iter().enumerate() {
				writeln!(f, "  {}. {step}", number + 1)?;
			}
		}

		Ok(())
	}
}

/// Writes the screens into the project at `root`, with the "Sign in with
/// your app" ones for `idp`, and reports what changed.
///
/// # Errors
///
/// As [`run`].
pub fn generate(root: &Path, config: &LeptoskitConfig, idp: bool, force: bool) -> Result<Report> {
	let mut report = Report::default();
	let mode = Mode {
		idp,
		passwords: config
			.auth
			.as_ref()
			.is_some_and(|auth| auth.passwords.enabled),
	};
	let root = std::fs::canonicalize(root).context("resolve auth project directory")?;
	let root = root.as_path();
	let layout = layout::Layout::read(
		root,
		&config.app.name,
		config.build.client_package.as_deref(),
		config.build.backend,
	)?;
	check_feature_destination(root, &layout.client.manifest)?;
	if config.db.is_some()
		&& let Some(server) = &layout.server
	{
		check_feature_destination(root, &server.manifest)?;
	}
	let client = layout
		.client
		.manifest
		.parent()
		.context("client manifest directory")?;
	let entry = &layout.client.entry;
	let source = entry.parent().context("client target directory")?;
	anyhow::ensure!(
		!source.join("auth.rs").try_exists()?,
		"{} already exists; the screens go in src/auth/, so rename it first",
		relative(root, &source.join("auth.rs"))
	);
	let client_source = source::preflight(&layout, config.db.is_some())?;

	for (path, contents) in pages(mode) {
		let file = Template {
			contents: &contents,
			others: &other_modes(path, mode, pages),
			mode,
		};
		write_template(root, &source.join(path), &file, force, &mut report)?;
	}

	declare_module(root, entry, "auth", &mut report)?;
	let features: Vec<&str> = CLIENT_FEATURES
		.into_iter()
		.chain(idp.then_some("idp"))
		.collect();
	add_features(root, &client.join("Cargo.toml"), &features, &mut report)?;

	if let Some(routes) = routes_dir(root, config) {
		add_routes(root, config, &routes, mode, force, &mut report)?;
	}

	if config.auth.is_none() {
		add_auth_table(root, AUTH_TABLE, "[auth]", &mut report)?;
	}

	if idp && config.auth.as_ref().is_none_or(|auth| auth.idp.is_none()) {
		add_auth_table(root, IDP_TABLE, "[auth.idp]", &mut report)?;
	}

	let granted = native_plugins(root, &mut report)?;
	let wired = client_source.calls.provide && client_source.calls.gate;

	if !wired {
		report.next.push(format!(
			"In {}, call `leptoskit::query::QueryClient::new().provide()` and then \
			 `auth::provide()` where the app starts, in its reactive owner, and show \
			 `auth::screens().gate(|| home)` in front of the screens that need someone signed in.",
			relative(root, entry)
		));
		report.next.push(if granted {
			"A native app also calls `leptoskit::auth::client::configure(ClientConfig::new().\
			 origin(\"https://your.site\"))` when it starts, with an origin listed in \
			 `[native.plugin-options.http] origins`; the sign-in guide shows how."
				.to_owned()
		} else {
			"A native app also calls `leptoskit::auth::client::configure(ClientConfig::new().\
			 origin(\"https://your.site\"))` when it starts, and needs the `http` and \
			 `secure-store` plugins; the sign-in guide shows how."
				.to_owned()
		});
	}

	if idp && !client_source.calls.navigate {
		report.next.push(format!(
			"In {}, after `auth::provide()`, say how the screens leave the app, such as back to \
			 an app asking for consent: `auth::screens().navigate_with(|address| ...)`, with \
			 `window.location.assign(address)` in the browser. The screens' module docs show how.",
			relative(root, entry)
		));
	}

	server(root, config, &layout, idp, force, &mut report)?;
	Ok(report)
}

/// The server's side: its sign-in module and the `auth` feature, when the
/// project has a server and a database.
fn server(
	root: &Path,
	config: &LeptoskitConfig,
	layout: &layout::Layout,
	idp: bool,
	force: bool,
	report: &mut Report,
) -> Result<()> {
	let Some(server) = &layout.server else {
		return Ok(());
	};
	let manifest = &server.manifest;
	let main = &server.entry;
	let source = main.parent().context("server target directory")?;
	let shown = relative(root, main);
	if config.db.is_none() {
		report.next.push(
			"Sign-in keeps its accounts in your database, so the server's side wasn't added. Add \
			 a `[db]` table to leptoskit.toml, then run `leptoskit generate auth` again."
				.to_owned(),
		);

		return Ok(());
	}

	// A server that is also the client crate already has `src/auth/`, so the
	// server's module takes another name.
	let module = source.join("sign_in.rs");
	let config_path = layout::relative(source, &root.join(leptoskit_config::CONFIG_FILE));
	let config_path = config_path
		.to_str()
		.context("config path must be UTF-8")?
		.replace(std::path::MAIN_SEPARATOR, "/");
	let config_root = layout::relative(
		manifest.parent().context("server manifest directory")?,
		root,
	);
	let config_root = config_root
		.to_str()
		.context("config root must be UTF-8")?
		.replace(std::path::MAIN_SEPARATOR, "/");
	let render = |idp| server_module_at(&config_path, &config_root, idp);
	let contents = render(idp);
	let other = Mode {
		idp: !idp,
		..Mode::default()
	};
	let file = Template {
		contents: &contents,
		others: &[(other, render(!idp))],
		mode: Mode {
			idp,
			..Mode::default()
		},
	};
	write_template(root, &module, &file, force, report)?;
	declare_module(root, main, "sign_in", report)?;

	// Passwords hash on the server, so only the server's build gets them.
	let passwords = config
		.auth
		.as_ref()
		.is_some_and(|auth| auth.passwords.enabled);

	if layout.client.manifest == server.manifest {
		if !has_feature(manifest, "leptoskit", "server")? {
			report.next.push(format!(
				"The generated server module in {shown} needs leptoskit's `server` feature. \
				 Enable `leptoskit/server` through the Cargo feature your backend build turns on, \
				 or use a separate backend package. Keep it out of the shared dependency's \
				 features so browser and guest builds do not acquire server dependencies."
			));
		}
		if passwords && !has_feature(manifest, "leptoskit", "auth-passwords")? {
			report.next.push(
				"Passwords are on in [auth.passwords]: turn on leptoskit's `auth-passwords` \
				 feature for the server's build only (it hashes with Argon2id, which the browser \
				 bundle doesn't need), such as through the feature your server build turns on."
					.to_owned(),
			);
		}
	} else {
		let features: Vec<&str> = ["server", "auth"]
			.into_iter()
			.chain(idp.then_some("idp"))
			.chain(passwords.then_some("auth-passwords"))
			.collect();
		add_features(root, manifest, &features, report)?;
	}

	let main_source = source::Source::read(main)?;

	if idp && !main_source.calls.provider {
		report.next.push(format!(
			"In {shown}, build the provider next to the sign-in service, with your scopes and \
			 claims: `let idp = sign_in::provider(&db, \
			 &auth)?.scopes::<Scope>().claims::<User>().build()?;`. Merge `idp.router()` next to \
			 `auth.router()`, and add `.layer(idp.layer())` after `.layer(auth.layer())`, so it's \
			 outermost. Add `leptoskit_idp` after `leptoskit_auth` in `packs!`, and run \
			 `leptoskit db migration create idp`."
		));
	}

	if !main_source.calls.service {
		report.next.push(format!(
			"In {shown}, build the service once the database is migrated, with your user table: \
			 `let auth = sign_in::service::<User>(&db)?;`. Then add `.merge(auth.router())` and \
			 `.merge(sign_in::associations()?)` to the router, and `.layer(auth.layer())` last, \
			 so it wraps every route."
		));
		report.next.push(
			"In the crate that declares your tables, add \
			 `leptoskit::db::packs!(leptoskit_auth);`, key your user table by \
			 `Id<auth::Account>`, implement `auth::OnSignUp` for it, and run `leptoskit db \
			 migration create auth`."
				.to_owned(),
		);
		report.next.push(
			"Set AUTH_ENCRYPTION_KEYS before starting the server: `export \
			 AUTH_ENCRYPTION_KEYS=\"$(date +%Y-%m):$(openssl rand 32 | basenc --base64url | tr -d \
			 =)\"`."
				.to_owned(),
		);
	}

	Ok(())
}

/// The routes directory, when the app uses the file router.
fn routes_dir(root: &Path, config: &LeptoskitConfig) -> Option<PathBuf> {
	let spec = crate::build::router_spec(config)?;
	let dir = spec
		.options
		.get("dir")
		.and_then(toml::Value::as_str)
		.map_or_else(
			|| leptoskit_config::RouterConfig::default().dir,
			PathBuf::from,
		);
	Some(root.join(dir))
}

/// Adds `/sign-in`, `/devices`, and `/security` to the file router, and for
/// `--idp` `/consent`, `/device`, `/connected-apps`, and `/developer-apps`,
/// unless the app already has routes by those names, then brings the
/// generated module up to date.
fn add_routes(
	root: &Path,
	config: &LeptoskitConfig,
	dir: &Path,
	mode: Mode,
	force: bool,
	report: &mut Report,
) -> Result<()> {
	let mut wrote = false;

	for (file, contents) in routes(mode) {
		let target = dir.join(file);
		let existing = walkdir::WalkDir::new(dir)
			.into_iter()
			.filter_map(Result::ok)
			.find(|entry| entry.file_name() == file && entry.path() != target);

		if let Some(existing) = existing {
			report.kept.push(format!(
				"kept {}: the app already has that route",
				relative(root, existing.path())
			));
			continue;
		}

		let template = Template {
			contents: &contents,
			others: &other_modes(file, mode, routes),
			mode,
		};
		wrote |= write_template(root, &target, &template, force, report)?;
	}

	if wrote && let Err(error) = crate::routes::sync(root, config) {
		report.next.push(format!(
			"Regenerating the routes module failed ({error:#}); fix that, then run `leptoskit \
			 plugin:router routes`."
		));
	}

	Ok(())
}

/// Gives the native apps the plugins the sign-in client needs: each host
/// that registers plugins (iOS, Android, macOS, and Linux) depends on them, and
/// `leptoskit.toml` grants them. Returns whether the app has such a host.
fn native_plugins(root: &Path, report: &mut Report) -> Result<bool> {
	let hosts: Vec<PathBuf> = crate::new_project::PLUGIN_PLATFORMS
		.into_iter()
		.map(|platform| root.join(platform).join("Cargo.toml"))
		.filter(|manifest| manifest.is_file())
		.collect();

	if hosts.is_empty() {
		return Ok(false);
	}

	for manifest in &hosts {
		add_plugin_dependencies(root, manifest, report)?;
	}

	grant_plugins(root, report)?;
	Ok(true)
}

/// `a`, `a and b`, or `a, b, and c`.
fn and_list(items: &[&str]) -> String {
	match items {
		[] => String::new(),
		[only] => (*only).to_owned(),
		[first, second] => format!("{first} and {second}"),
		[rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
	}
}

/// Adds the sign-in plugins next to a host's `leptoskit_bridge_ffi`, at the
/// same version, and from the same checkout when it uses a path.
fn add_plugin_dependencies(root: &Path, manifest: &Path, report: &mut Report) -> Result<()> {
	let shown = relative(root, manifest);
	let contents =
		std::fs::read_to_string(manifest).with_context(|| format!("could not read {shown}"))?;
	let mut document = contents
		.parse::<toml_edit::DocumentMut>()
		.with_context(|| format!("could not parse {shown}"))?;
	let Some(dependencies) = bridge_dependencies(&mut document) else {
		report.next.push(format!(
			"{shown} doesn't depend on leptoskit_bridge_ffi; add leptoskit_plugin_http, \
			 leptoskit_plugin_secure_store, leptoskit_plugin_passkeys, and \
			 leptoskit_plugin_web_auth to the dependencies of the crate that does."
		));

		return Ok(());
	};
	let bridge = dependencies
		.get("leptoskit_bridge_ffi")
		.and_then(toml_edit::Item::as_table_like);

	let version = bridge
		.and_then(|bridge| bridge.get("version"))
		.and_then(toml_edit::Item::as_str)
		.or_else(|| {
			dependencies
				.get("leptoskit_bridge_ffi")
				.and_then(toml_edit::Item::as_str)
		})
		.map(str::to_owned);
	let checkout = bridge
		.and_then(|bridge| bridge.get("path"))
		.and_then(toml_edit::Item::as_str)
		.map(str::to_owned);
	let mut added = Vec::new();

	for (_, package) in crate::new_project::AUTH_PLUGINS {
		if dependencies.contains_key(package) {
			continue;
		}

		let mut dependency = toml_edit::InlineTable::new();

		if let Some(version) = &version {
			dependency.insert("version", version.as_str().into());
		}

		dependency.insert("default-features", false.into());

		if let Some(checkout) = &checkout {
			dependency.insert("path", sibling(checkout, package).into());
		}

		dependencies.insert(package, toml_edit::value(dependency));
		added.push(package);
	}

	if added.is_empty() {
		return Ok(());
	}

	std::fs::write(manifest, document.to_string())
		.with_context(|| format!("could not write {shown}"))?;
	report
		.changed
		.push(format!("updated {shown}: added {}", and_list(&added)));
	Ok(())
}

/// The path of a crate next to `leptoskit_bridge_ffi`, keeping the
/// separator style the manifest already uses, so a Windows checkout
/// doesn't get `/sdk/crates\leptoskit_plugin_http`.
fn sibling(path: &str, package: &str) -> String {
	match path.rfind(['/', '\\']) {
		Some(end) => format!("{}{package}", &path[..=end]),
		None => package.to_owned(),
	}
}

/// The dependency table that holds `leptoskit_bridge_ffi`: `[dependencies]`,
/// or a `[target.'cfg(…)'.dependencies]`.
fn bridge_dependencies(
	document: &mut toml_edit::DocumentMut,
) -> Option<&mut dyn toml_edit::TableLike> {
	let has_bridge = |item: &toml_edit::Item| {
		item.as_table_like()
			.is_some_and(|table| table.contains_key("leptoskit_bridge_ffi"))
	};

	if document.get("dependencies").is_some_and(has_bridge) {
		return document.get_mut("dependencies")?.as_table_like_mut();
	}

	let targets = document.get_mut("target")?.as_table_like_mut()?;
	let key = targets
		.iter()
		.find(|(_, target)| target.get("dependencies").is_some_and(has_bridge))
		.map(|(key, _)| key.to_owned())?;
	targets
		.get_mut(&key)?
		.get_mut("dependencies")?
		.as_table_like_mut()
}

/// Grants the sign-in plugins in `leptoskit.toml`, and lets the HTTP plugin
/// reach the development server unless the app already chose its origins.
fn grant_plugins(root: &Path, report: &mut Report) -> Result<()> {
	let path = root.join(leptoskit_config::CONFIG_FILE);
	let contents = std::fs::read_to_string(&path)
		.with_context(|| format!("could not read {}", path.display()))?;
	let mut document = contents
		.parse::<toml_edit::DocumentMut>()
		.with_context(|| format!("could not parse {}", path.display()))?;
	let native = document
		.entry("native")
		.or_insert_with(toml_edit::table)
		.as_table_like_mut()
		.context("[native] in leptoskit.toml isn't a table")?;
	let plugins = native
		.entry("plugins")
		.or_insert_with(|| toml_edit::value(toml_edit::Array::new()))
		.as_array_mut()
		.context("`plugins` in [native] isn't a list")?;
	let mut changes = Vec::new();

	for (name, _) in crate::new_project::AUTH_PLUGINS {
		if !plugins.iter().any(|plugin| plugin.as_str() == Some(name)) {
			plugins.push(name);
			changes.push(format!("granted {name}"));
		}
	}

	let options = native.entry("plugin-options").or_insert_with(|| {
		let mut table = toml_edit::Table::new();
		table.set_implicit(true);
		toml_edit::Item::Table(table)
	});
	let options = options
		.as_table_like_mut()
		.context("`plugin-options` in [native] isn't a table")?;

	if !options.contains_key("http") {
		let mut http = toml_edit::Table::new();
		http.insert(
			"origins",
			toml_edit::value(toml_edit::Array::from_iter(
				crate::new_project::AUTH_DEV_ORIGINS,
			)),
		);
		options.insert("http", toml_edit::Item::Table(http));
		changes.push("let the HTTP plugin reach the development server".to_owned());
	}

	if changes.is_empty() {
		return Ok(());
	}

	let contents = document.to_string();
	LeptoskitConfig::from_toml(&contents, root).with_context(|| {
		format!(
			"granting the sign-in plugins would break {}",
			path.display()
		)
	})?;
	std::fs::write(&path, contents)
		.with_context(|| format!("could not write {}", path.display()))?;
	report.changed.push(format!(
		"updated {}: {}",
		leptoskit_config::CONFIG_FILE,
		changes.join(", ")
	));
	Ok(())
}

/// Appends a table, `[auth]` or `[auth.idp]`, to `leptoskit.toml`, checking
/// that the file still loads.
fn add_auth_table(root: &Path, table: &str, name: &str, report: &mut Report) -> Result<()> {
	let path = root.join(leptoskit_config::CONFIG_FILE);
	let mut contents = std::fs::read_to_string(&path)
		.with_context(|| format!("could not read {}", path.display()))?;

	if !contents.ends_with('\n') {
		contents.push('\n');
	}

	contents.push_str(table);
	LeptoskitConfig::from_toml(&contents, root)
		.with_context(|| format!("adding {name} would break {}", path.display()))?;
	std::fs::write(&path, contents)
		.with_context(|| format!("could not write {}", path.display()))?;
	report.changed.push(format!(
		"updated {}: added {name}",
		leptoskit_config::CONFIG_FILE
	));
	Ok(())
}

/// A file to write in `mode`, and how the same file reads in the other
/// modes.
struct Template<'a> {
	contents: &'a str,
	others: &'a [(Mode, String)],
	mode: Mode,
}

/// Writes a template like [`write_owned`], except that a file still exactly
/// as another mode wrote it isn't the app's own: a run that adds parts, such
/// as `--idp`, brings it up to date, and a run without them leaves it alone,
/// so a run never takes screens away. Returns whether it wrote.
fn write_template(
	root: &Path,
	path: &Path,
	template: &Template<'_>,
	force: bool,
	report: &mut Report,
) -> Result<bool> {
	let existing = if path.is_file() {
		std::fs::read_to_string(path).ok()
	} else {
		None
	};

	let earlier: Vec<Mode> = template
		.others
		.iter()
		.filter(|(_, other)| other != template.contents && existing.as_ref() == Some(other))
		.map(|(mode, _)| *mode)
		.collect();

	if earlier.is_empty() {
		return write_owned(root, path, template.contents, force, report);
	}

	// A file without one part's lines reads the same with or without it, so
	// the mode it matches with the most parts says what this run adds.
	let Some(earlier) = earlier
		.into_iter()
		.filter(|earlier| template.mode.covers(*earlier))
		.max_by_key(|earlier| u8::from(earlier.idp) + u8::from(earlier.passwords))
	else {
		return Ok(false);
	};
	let shown = relative(root, path);
	std::fs::write(path, template.contents).with_context(|| format!("could not write {shown}"))?;
	report.changed.push(format!(
		"updated {shown}: added {}",
		template.mode.added_to(earlier)
	));
	Ok(true)
}

/// Writes a file the app owns once it exists: creates it when missing, and
/// leaves it alone when it matches or, without `force`, when it differs.
/// Returns whether it wrote.
fn write_owned(
	root: &Path,
	path: &Path,
	contents: &str,
	force: bool,
	report: &mut Report,
) -> Result<bool> {
	let shown = relative(root, path);

	match std::fs::symlink_metadata(path) {
		Ok(metadata) => {
			anyhow::ensure!(metadata.is_file(), "{shown} isn't an ordinary file");

			if std::fs::read_to_string(path)? == contents {
				return Ok(false);
			}

			if !force {
				report.kept.push(format!(
					"kept {shown}: it differs from the template; --force replaces it"
				));
				return Ok(false);
			}

			std::fs::write(path, contents).with_context(|| format!("could not write {shown}"))?;
			report.changed.push(format!("replaced {shown}"));
		}

		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
			if let Some(parent) = path.parent() {
				std::fs::create_dir_all(parent)
					.with_context(|| format!("could not create {}", relative(root, parent)))?;
			}

			std::fs::write(path, contents).with_context(|| format!("could not write {shown}"))?;
			report.changed.push(format!("created {shown}"));
		}
		Err(error) => return Err(error).with_context(|| format!("could not inspect {shown}")),
	}

	Ok(true)
}

/// Appends `pub mod <name>;` to a crate root that doesn't declare the module.
fn declare_module(root: &Path, entry: &Path, name: &str, report: &mut Report) -> Result<()> {
	let mut contents = std::fs::read_to_string(entry)?;
	if source::Source::read(entry)?.module(name)? {
		return Ok(());
	}

	if !contents.ends_with('\n') {
		contents.push('\n');
	}

	let visibility = if name == "auth" { "pub " } else { "" };
	writeln!(
		contents,
		"\n/// Written by `leptoskit generate auth`.\n{visibility}mod {name};"
	)?;
	std::fs::write(entry, contents)?;
	report.changed.push(format!(
		"updated {}: declared `{visibility}mod {name};`",
		relative(root, entry)
	));
	Ok(())
}

/// Validate the dependency shape before writing any generated source.
fn check_feature_destination(root: &Path, manifest: &Path) -> Result<()> {
	let shown = relative(root, manifest);
	let mut document = std::fs::read_to_string(manifest)?.parse::<toml_edit::DocumentMut>()?;
	add_dependency_features(&mut document, "leptoskit", &[]).with_context(|| {
		format!(
			"{shown} needs a leptoskit entry under [dependencies] for generated auth; \
			 target-specific dependency tables must be integrated manually"
		)
	})?;
	Ok(())
}

/// Adds `features` to the `dependency` in a manifest, keeping its formatting.
fn add_features(
	root: &Path,
	manifest: &Path,
	features: &[&str],
	report: &mut Report,
) -> Result<()> {
	let shown = relative(root, manifest);
	let contents =
		std::fs::read_to_string(manifest).with_context(|| format!("could not read {shown}"))?;
	let mut document = contents
		.parse::<toml_edit::DocumentMut>()
		.with_context(|| format!("could not parse {shown}"))?;
	let added = add_dependency_features(&mut document, "leptoskit", features)
		.with_context(|| format!("{shown} doesn't depend on leptoskit"))?;

	if added.is_empty() {
		return Ok(());
	}

	std::fs::write(manifest, document.to_string())
		.with_context(|| format!("could not write {shown}"))?;
	report.changed.push(format!(
		"updated {shown}: added the leptoskit feature{} {}",
		if added.len() == 1 { "" } else { "s" },
		added.join(", ")
	));
	Ok(())
}

/// Adds the missing `features` to `name` under `[dependencies]`, turning a
/// bare version into a table. Returns the features it added, or `None`
/// without the dependency.
fn add_dependency_features(
	document: &mut toml_edit::DocumentMut,
	name: &str,
	features: &[&str],
) -> Option<Vec<String>> {
	let dependency = document.get_mut("dependencies")?.get_mut(name)?;

	if let Some(version) = dependency.as_str().map(str::to_owned) {
		let mut table = toml_edit::InlineTable::new();
		table.insert("version", version.into());
		*dependency = toml_edit::value(table);
	}

	let table = dependency.as_table_like_mut()?;
	let inserted = !table.contains_key("features");
	let list = table
		.entry("features")
		.or_insert_with(|| toml_edit::value(toml_edit::Array::new()))
		.as_array_mut()?;
	let mut added = Vec::new();

	for feature in features {
		if !list
			.iter()
			.any(|existing| existing.as_str() == Some(feature))
		{
			list.push(*feature);
			added.push((*feature).to_owned());
		}
	}

	// A key added to `{ … }` keeps the space that closed the table before
	// it, so lay the table out again.
	if inserted && let Some(inline) = dependency.as_inline_table_mut() {
		inline.fmt();
	}

	Some(added)
}

/// Whether `dependency` in a manifest turns on `feature`.
fn has_feature(manifest: &Path, dependency: &str, feature: &str) -> Result<bool> {
	let Ok(contents) = std::fs::read_to_string(manifest) else {
		return Ok(false);
	};
	let document: toml::Table = toml::from_str(&contents)
		.with_context(|| format!("could not parse {}", manifest.display()))?;
	Ok(document
		.get("dependencies")
		.and_then(|dependencies| dependencies.get(dependency))
		.and_then(|dependency| dependency.get("features"))
		.and_then(toml::Value::as_array)
		.is_some_and(|features| features.iter().any(|value| value.as_str() == Some(feature))))
}

/// `path` relative to the project root, with `/` separators.
fn relative(root: &Path, path: &Path) -> String {
	path.strip_prefix(root)
		.unwrap_or(path)
		.to_string_lossy()
		.replace(std::path::MAIN_SEPARATOR, "/")
}

#[cfg(test)]
mod tests;
