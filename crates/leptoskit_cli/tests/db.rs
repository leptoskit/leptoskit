//! `leptoskit db` end to end: a throwaway app whose db crate depends on this
//! repository's `leptoskit_db`, migrated through the real binary.
//!
//! Ignored by default because it compiles the app; CI runs it with the other
//! end-to-end tests (`--ignored`).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error as BridgeError;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;
use leptoskit_native::EventOutcome;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_plugin_http::Method as HttpMethod;
use leptoskit_plugin_http::Request as HttpRequest;
use leptoskit_plugin_http::protocol;
use leptoskit_yoga::NodeId;

const TABLES: &str = r#"
use leptoskit_db::Id;
use leptoskit_db::table;

/// Someone with an account.
#[table(name = "users", public)]
pub struct User {
	pub id: Id<Self>,
	#[db(unique, max_len = 32)]
	pub handle: String,
	// EXTRA
}

leptoskit_db::export_schema!();
"#;

fn repository() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR"))
		.join("../..")
		.canonicalize()
		.unwrap()
}

/// A workspace with one db crate, `app_db`, and a SQLite `[db]` table.
fn app(root: &Path) {
	let leptoskit_db = repository().join("crates/leptoskit_db");
	fs::write(
		root.join("Cargo.toml"),
		"[workspace]\nmembers = [\"app_db\"]\nresolver = \"2\"\n",
	)
	.unwrap();
	fs::create_dir_all(root.join("app_db/src")).unwrap();
	fs::write(
		root.join("app_db/Cargo.toml"),
		format!(
			"[package]\nname = \"app_db\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = \
			 false\n\n[dependencies]\nleptoskit_db = {{ path = {:?} }}\n\n[features]\nserver = \
			 [\"leptoskit_db/server\"]\n",
			leptoskit_db.display().to_string()
		),
	)
	.unwrap();
	fs::write(root.join("app_db/src/lib.rs"), TABLES).unwrap();
	fs::write(
		root.join("leptoskit.toml"),
		"schema_version = 0\n\n[app]\nname = \"app\"\nversion = \"0.1.0\"\n\n[db]\ndialect = \
		 \"sqlite\"\n",
	)
	.unwrap();
	// The repository's lock file pins the same versions it tests with.
	fs::copy(repository().join("Cargo.lock"), root.join("Cargo.lock")).unwrap();
}

fn leptoskit(root: &Path, args: &[&str]) -> Output {
	let target = std::env::var_os("LEPTOSKIT_E2E_TARGET_DIR")
		.map_or_else(|| root.join("target"), PathBuf::from);
	Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(args)
		.current_dir(root)
		.env("CARGO_TARGET_DIR", target)
		.env_remove("DATABASE_URL")
		.output()
		.expect("run leptoskit")
}

fn succeeds(output: &Output) -> String {
	let stdout = String::from_utf8_lossy(&output.stdout).to_string();
	assert!(
		output.status.success(),
		"leptoskit failed:\n{stdout}\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	stdout
}

fn fails(output: &Output) -> String {
	assert!(!output.status.success(), "leptoskit should have failed");
	String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
#[ignore = "compiles a throwaway app; run with --ignored"]
fn migrations_follow_the_tables() {
	let root = tempfile::tempdir().unwrap();
	app(root.path());

	let created = succeeds(&leptoskit(
		root.path(),
		&["db", "migration", "create", "init"],
	));
	assert!(created.contains("+ table users (2 columns)"), "{created}");
	assert!(succeeds(&leptoskit(root.path(), &["db", "check"])).contains("the tables match"));

	let grown = TABLES.replace("// EXTRA", "pub bio: Option<String>,");
	fs::write(root.path().join("app_db/src/lib.rs"), grown).unwrap();
	let drifted = fails(&leptoskit(root.path(), &["db", "check"]));
	assert!(drifted.contains("+ column users.bio (text)"), "{drifted}");

	std::thread::sleep(std::time::Duration::from_secs(1));
	succeeds(&leptoskit(
		root.path(),
		&["db", "migration", "create", "add bio"],
	));
	succeeds(&leptoskit(root.path(), &["db", "check"]));

	let url = format!("sqlite://{}", root.path().join("app.sqlite3").display());
	let migrated = succeeds(&leptoskit(root.path(), &["db", "migrate", "--url", &url]));
	assert_eq!(
		migrated
			.lines()
			.filter(|line| line.starts_with("applied "))
			.count(),
		2,
		"{migrated}"
	);
	assert!(
		succeeds(&leptoskit(root.path(), &["db", "migrate", "--url", &url])).contains("up to date")
	);
	let status = succeeds(&leptoskit(root.path(), &["db", "status", "--url", &url]));
	assert!(
		status.contains("the database has every migration"),
		"{status}"
	);

	assert!(fails(&leptoskit(root.path(), &["db", "reset", "--url", &url])).contains("pass --yes"));
	assert!(
		succeeds(&leptoskit(
			root.path(),
			&["db", "reset", "--url", &url, "--yes"]
		))
		.contains("applied 2")
	);
	assert!(fails(&leptoskit(root.path(), &["db", "migrate"])).contains("set DATABASE_URL"));
}

/// One HTTP/1.1 request to the scaffold's server, returning the status line's
/// code and the body.
fn request(address: &str, method: &str, path: &str, body: &str) -> (u16, String) {
	let (status, _, body) = exchange(address, method, path, "", body);
	(status, body)
}

/// One HTTP/1.1 request with extra header lines, each ending in `\r\n`,
/// returning the status line's code, the response headers, and the body.
fn exchange(
	address: &str,
	method: &str,
	path: &str,
	headers: &str,
	body: &str,
) -> (u16, String, String) {
	use std::io::Read;
	use std::io::Write;

	let mut stream = std::net::TcpStream::connect(address).expect("connect");
	write!(
		stream,
		"{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: \
		 application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
		body.len()
	)
	.unwrap();
	let mut response = String::new();
	stream.read_to_string(&mut response).unwrap();
	let status = response[9..12].parse().unwrap();
	let (head, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
	(status, head.to_owned(), body.to_owned())
}

/// A scaffolded server, killed if the test ends while it runs.
struct Server {
	child: std::process::Child,
}

impl Server {
	/// Starts the scaffold's debug server on `address`, with `envs`, and
	/// waits for it to listen.
	fn start(root: &Path, target: &Path, address: &str, envs: &[(&str, &str)]) -> Self {
		let mut child = Command::new(target.join("debug/notes"));
		child
			.current_dir(root)
			.env("LEPTOS_SITE_ADDR", address)
			.env_remove("DATABASE_URL")
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null());

		for (name, value) in envs {
			child.env(name, value);
		}

		let server = Self {
			child: child.spawn().unwrap(),
		};
		let ready = (0..240).any(|_| {
			std::thread::sleep(std::time::Duration::from_millis(250));
			std::net::TcpStream::connect(address).is_ok()
		});
		assert!(ready, "the server never listened on {address}");
		server
	}

	/// Stops the server the way a container runtime does, with `SIGTERM`, and
	/// returns whether it shut down cleanly: it fails `/readyz`, tells live
	/// sockets to reconnect, and finishes what's in flight.
	fn terminate(mut self) -> bool {
		#[cfg(unix)]
		{
			let signalled = Command::new("kill")
				.args(["-TERM", &self.child.id().to_string()])
				.status()
				.unwrap();
			assert!(signalled.success());
		}
		#[cfg(not(unix))]
		self.child.kill().unwrap();

		let stopped = (0..240).find_map(|_| {
			std::thread::sleep(std::time::Duration::from_millis(250));
			self.child.try_wait().unwrap()
		});
		stopped.is_some_and(|status| cfg!(not(unix)) || status.success())
	}
}

impl Drop for Server {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

/// A free port on this machine, as `127.0.0.1:port`.
fn free_address() -> String {
	std::net::TcpListener::bind("127.0.0.1:0")
		.unwrap()
		.local_addr()
		.unwrap()
		.to_string()
}

/// Whether `program` runs; otherwise the part of the test that needs it is
/// skipped, unless `LEPTOSKIT_REQUIRE_SDK_TOOLCHAINS=1`.
fn have(program: &str) -> bool {
	let found = Command::new(program)
		.arg("--version")
		.output()
		.is_ok_and(|output| output.status.success());

	if !found {
		assert!(
			std::env::var("LEPTOSKIT_REQUIRE_SDK_TOOLCHAINS").as_deref() != Ok("1"),
			"`{program}` is required but isn't on PATH"
		);
		eprintln!("skipped: `{program}` isn't on PATH");
	}

	found
}

/// `leptoskit rpc spec` and `leptoskit rpc generate --target typescript`
/// with the server's environment broken: no sign-in keys, and a database
/// URL that can't connect. Both still succeed, because the server writes
/// the spec before it reads either, and nothing touches the database. The
/// spec lists `operations`, each at `POST /api/<name>`, and the backend's
/// `GET /api/health`; the client is written to `client`.
fn exports_without_the_environment(root: &Path, operations: &[&str], client: &Path) -> String {
	let target = std::env::var_os("LEPTOSKIT_E2E_TARGET_DIR")
		.map_or_else(|| root.join("target"), PathBuf::from);
	let broken = |args: &[&str]| {
		let output = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
			.args(args)
			.current_dir(root)
			.env("CARGO_TARGET_DIR", &target)
			.env("DATABASE_URL", "postgres://nobody@127.0.0.1:1/never")
			.env_remove("AUTH_ENCRYPTION_KEYS")
			.output()
			.expect("run leptoskit");
		succeeds(&output)
	};

	let spec = broken(&["rpc", "spec"]);
	let parsed: serde_json::Value = serde_json::from_str(&spec).expect("the spec is JSON");
	let functions = parsed["functions"]
		.as_array()
		.expect("the spec lists functions");
	let route = |name: &str| {
		functions
			.iter()
			.find(|function| function["name"] == name)
			.map(|function| {
				format!(
					"{} {}",
					function["method"].as_str().unwrap(),
					function["path"].as_str().unwrap()
				)
			})
	};
	assert_eq!(
		route("health").as_deref(),
		Some("GET /api/health"),
		"{spec}"
	);

	for name in operations {
		assert_eq!(
			route(name),
			Some(format!("POST /api/{name}")),
			"{name} isn't published: {spec}"
		);
	}

	broken(&[
		"rpc",
		"generate",
		"--target",
		"typescript",
		"--out",
		client.to_str().unwrap(),
	]);
	let index = fs::read_to_string(client.join("index.ts")).unwrap();
	assert!(index.contains("health(): Promise<Health>"), "{index}");

	for name in operations {
		let member = name
			.split('_')
			.enumerate()
			.map(|(index, part)| {
				if index == 0 {
					part.to_owned()
				} else {
					part[..1].to_uppercase() + &part[1..]
				}
			})
			.collect::<String>();
		assert!(
			index.contains(&format!("readonly {member} ")),
			"{member}: {index}"
		);
	}

	assert!(
		!root.join("notes.sqlite3").exists(),
		"exporting the spec opened the database"
	);
	spec
}

/// The built executable must export its own checkpoint even when every
/// normal startup input is unusable. An existing export is not replaced.
fn exports_migration_checkpoint_without_startup(root: &Path, target: &Path) {
	let directory = tempfile::tempdir().unwrap();
	let out = directory.path().join("contract.json");
	let database = directory.path().join("must-not-exist.sqlite3");
	let mut command = Command::new(target.join("debug/notes"));
	command
		.current_dir(root)
		.env("LEPTOSKIT_DB_CONTRACT_OUT", &out)
		.env("DATABASE_URL", format!("sqlite:{}", database.display()))
		.env("LEPTOS_SITE_ADDR", "not-a-socket")
		.env("LEPTOSKIT_MIGRATION_WINDOW", "missing-window.json")
		.env_remove("LEPTOSKIT_MIGRATION_WINDOW_SHA256")
		.env_remove("LEPTOSKIT_RPC_SPEC_OUT")
		.env_remove("AUTH_ENCRYPTION_KEYS")
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null());
	let run = |command: &mut Command| {
		let mut server = Server {
			child: command.spawn().unwrap(),
		};
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);

		loop {
			if let Some(status) = server.child.try_wait().unwrap() {
				break status;
			}

			assert!(
				std::time::Instant::now() < deadline,
				"migration export entered normal startup"
			);
			std::thread::sleep(std::time::Duration::from_millis(20));
		}
	};
	assert!(run(&mut command).success());
	let json = fs::read_to_string(&out).unwrap();
	let exported = leptoskit_db::MigrationContract::from_json(&json).unwrap();
	let migrations = leptoskit_db::load_migrations(&root.join("migrations")).unwrap();
	let snapshot = fs::read_to_string(
		root.join("migrations")
			.join(migrations.last().unwrap().name.as_ref())
			.join("schema.json"),
	)
	.unwrap();
	assert_eq!(
		exported,
		leptoskit_db::MigrationContract::new(&migrations, Some(&snapshot)).unwrap()
	);
	assert!(!database.exists(), "migration export created a database");
	assert!(!run(&mut command).success());
	assert_eq!(fs::read_to_string(out).unwrap(), json);
}

/// The generated notes screen really uses the shared typed operations, keeps
/// the browser and native paths apart, and, with sign-in, only shows one
/// account's notes.
fn assert_generated_notes_screen(root: &Path, auth: bool) {
	let app = root.join("notes_app/src");
	let notes = fs::read_to_string(app.join("notes.rs")).expect("the notes screen exists");
	assert!(
		notes.contains("_shared::AddNote;") && notes.contains("_shared::Notes as NotesQuery;"),
		"the screen doesn't use the shared types: {notes}"
	);
	assert!(
		notes.contains("mutation_scope::<AddNote>()")
			&& notes.contains("invalidate_scope(&query_scope::<NotesQuery>())"),
		"the screen doesn't send AddNote and refresh the list: {notes}"
	);
	assert!(
		notes.contains("use_live(|| NotesQuery {})")
			&& notes.contains("use_rpc_query(|| NotesQuery {})"),
		"the browser live path and the native HTTP path must both exist: {notes}"
	);
	for state in [
		"Loading notes…",
		"No notes yet.",
		"The notes couldn't be loaded",
		"The note couldn't be saved",
		"Saved.",
	] {
		assert!(notes.contains(state), "missing state {state:?}: {notes}");
	}
	assert!(
		notes.contains("MAX_BODY") && notes.contains("validate(&self.draft.get_untracked())"),
		"the body isn't validated before sending: {notes}"
	);
	assert!(
		notes.contains("remove_queries")
			&& notes.contains("QueryFilter::scope")
			&& notes.contains("query_scope::<")
			&& notes.contains("NotesQuery"),
		"an account change must drop the previous list: {notes}"
	);

	let lib = fs::read_to_string(app.join("lib.rs")).expect("the app lib exists");
	let home = fs::read_to_string(app.join("routes/index.rs")).expect("the home route exists");
	let routes = fs::read_to_string(app.join("routes.rs")).expect("the generated routes exist");

	assert!(
		routes.contains("@generated") && routes.contains("Navigation"),
		"the scaffold must commit a generated file-router module: {routes}"
	);

	if auth {
		assert!(
			lib.contains("Notes::new(identity.clone())")
				&& lib.contains("child.cleanup()")
				&& lib.contains("notes.owns(&identity)"),
			"the signed-in screen isn't rebuilt per account: {lib}"
		);
		assert!(lib.contains("auth::screens().gate"), "{lib}");
		assert!(home.contains("shell.notes()"), "{home}");
	} else {
		assert!(
			lib.contains("Notes::shared()") && lib.contains("pub fn provide() -> Notes"),
			"the database scaffold must keep the query cache and notes state: {lib}"
		);

		assert!(lib.contains("origin(SERVER)"), "{lib}");
		assert!(home.contains("notes.screen()"), "{home}");
	}

	assert!(
		!lib.contains("Counter::default()"),
		"a database scaffold still shows the counter: {lib}"
	);
}

/// `leptoskit new --db sqlite`, its first migration, and the server it
/// generates: the published API exports without a database, the server
/// migrates at startup and serves the shared crate's typed operations, and
/// the TypeScript client generated from the spec, run outside this
/// repository, adds a note live, sees it in its subscription, and follows
/// the server through a restart.
#[test]
#[ignore = "compiles a scaffolded server; run with --ignored"]
#[allow(
	clippy::too_many_lines,
	reason = "one scaffold's API, native journey, and browser journey read best top to bottom"
)]
#[expect(
	clippy::cognitive_complexity,
	reason = "one scaffold's API, native journey, and browser journey read best top to bottom"
)]
fn the_database_scaffold_migrates_and_serves() {
	use std::io::BufRead;
	use std::io::Write as _;

	let scratch = tempfile::tempdir().unwrap();
	let sdk = repository();
	succeeds(&leptoskit(
		scratch.path(),
		&[
			"new",
			"notes",
			"--db",
			"sqlite",
			"--sdk-path",
			sdk.to_str().unwrap(),
		],
	));
	let root = scratch.path().join("notes");
	fs::copy(sdk.join("Cargo.lock"), root.join("Cargo.lock")).unwrap();

	let created = succeeds(&leptoskit(&root, &["db", "migration", "create", "init"]));
	assert!(created.contains("+ table notes (3 columns)"), "{created}");
	assert!(created.contains("+ table rpc.receipts"), "{created}");
	succeeds(&leptoskit(&root, &["db", "check"]));
	assert_generated_notes_screen(&root, false);

	let client = scratch.path().join("client");
	exports_without_the_environment(&root, &["notes", "add_note"], &client);

	let target = std::env::var_os("LEPTOSKIT_E2E_TARGET_DIR")
		.map_or_else(|| root.join("target"), PathBuf::from);
	let cargo = |args: &[&str]| {
		let status = Command::new(env!("CARGO"))
			.args(args)
			.current_dir(&root)
			.env("CARGO_TARGET_DIR", &target)
			.status()
			.unwrap();
		assert!(status.success(), "cargo {}", args.join(" "));
	};
	let address = free_address();
	point_native_at(&root, &format!("http://{address}"));
	cargo(&["build", "--locked", "--package", "notes"]);
	exports_migration_checkpoint_without_startup(&root, &target);
	// The shared contracts build for the browser without the server's
	// driver.
	cargo(&[
		"check",
		"--locked",
		"--package",
		"notes_app",
		"--target",
		"wasm32-unknown-unknown",
		"--features",
		"hydrate",
	]);
	cargo(&["test", "--locked", "--package", "notes_app"]);
	// The native guest is optimized, as `leptoskit build` makes it.
	cargo(&[
		"build",
		"--locked",
		"--release",
		"--package",
		"notes_app",
		"--target",
		"wasm32-unknown-unknown",
		"--features",
		"native",
	]);

	let origins = native_origins(&root);
	assert_eq!(origins, [format!("http://{address}")]);

	// The real journey runs against a fresh database, so its empty and
	// loaded states are the scaffold's own rather than the API probes'.
	let native_server = Server::start(&root, &target, &address, &[]);
	native_database_journey(
		&target.join("wasm32-unknown-unknown/release/notes_app.wasm"),
		&address,
		&origins,
	);
	assert!(
		native_server.terminate(),
		"the native journey's server didn't shut down cleanly"
	);
	remove_database(&root);

	let server = Server::start(&root, &target, &address, &[]);
	// Rendering a page loads its query in process; it must not call the
	// server through its own relative URL.
	let page = request(&address, "GET", "/", "");
	assert_eq!(page.0, 200, "{}", page.1);
	assert!(page.1.contains("Notes"), "{}", page.1);
	assert!(
		!page.1.contains("The notes couldn't be loaded"),
		"{}",
		page.1
	);
	let initially_empty = request(&address, "POST", "/api/notes", "{}");
	let health = request(&address, "GET", "/api/health", "");
	let added = request(&address, "POST", "/api/add_note", r#"{"body":"hello"}"#);

	let empty = request(&address, "POST", "/api/add_note", r#"{"body":""}"#);
	let blank = request(&address, "POST", "/api/add_note", r#"{"body":"   "}"#);
	let too_long = request(
		&address,
		"POST",
		"/api/add_note",
		&format!(r#"{{"body":"{}"}}"#, "x".repeat(501)),
	);
	let listed = request(&address, "POST", "/api/notes", "{}");
	let ready = request(&address, "GET", "/readyz", "");

	assert_eq!(initially_empty, (200, "[]".to_owned()));
	assert_eq!(health, (200, r#"{"ready":true}"#.to_owned()));
	let (status, body) = added;
	assert_eq!(status, 200, "{body}");
	assert!(body.contains(r#""body":"hello""#), "{body}");
	let (status, body) = empty;
	assert_eq!(status, 400, "an empty note breaks min_len: {body}");
	assert!(body.starts_with("Args|"), "{body}");

	let (status, body) = blank;
	assert_eq!(status, 400, "a blank note breaks min_len: {body}");
	assert!(body.starts_with("Args|"), "{body}");
	let (status, body) = too_long;
	assert_eq!(status, 400, "a 501-character note breaks max_len: {body}");
	assert!(body.starts_with("Args|"), "{body}");
	let (status, body) = listed;
	assert_eq!(status, 200);
	assert!(
		body.starts_with('[') && body.contains(r#""body":"hello""#),
		"{body}"
	);
	assert_eq!(ready.0, 200, "{}", ready.1);

	if have("node") {
		fs::copy(
			Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes_sdk/journey.ts"),
			client.join("journey.ts"),
		)
		.unwrap();
		let mut journey = Command::new("node")
			.args(["--experimental-strip-types", "--no-warnings", "journey.ts"])
			.current_dir(&client)
			.env("NOTES_URL", format!("http://{address}"))
			.stdin(std::process::Stdio::piped())
			.stdout(std::process::Stdio::piped())
			.stderr(std::process::Stdio::inherit())
			.spawn()
			.unwrap();
		let mut lines = std::io::BufReader::new(journey.stdout.take().unwrap()).lines();
		let mut said = Vec::new();
		let mut server = Some(server);

		for line in lines.by_ref() {
			let line = line.unwrap();
			said.push(line.clone());

			if line == "ready for restart" {
				assert!(
					server.take().unwrap().terminate(),
					"the server didn't shut down cleanly"
				);
				server = Some(Server::start(&root, &target, &address, &[]));
				let (status, body) = request(
					&address,
					"POST",
					"/api/add_note",
					r#"{"body":"after restart"}"#,
				);
				assert_eq!(status, 200, "{body}");
				// Shutdown and startup each have their own minute. Only now
				// start the client's deadline for the resumed subscription.
				journey
					.stdin
					.as_mut()
					.unwrap()
					.write_all(b"restarted\n")
					.unwrap();
			}
		}

		let finished = journey.wait().unwrap();
		assert!(finished.success(), "the journey failed after {said:?}");
		assert_eq!(
			said,
			[
				"ok subscribed",
				"ok mutation",
				"ready for restart",
				"ok reconnect"
			]
		);
		assert!(server.unwrap().terminate());
	} else {
		assert!(server.terminate(), "the server didn't shut down cleanly");
	}

	assert!(root.join("notes.sqlite3").is_file());
}

/// `leptoskit db studio` on a throwaway app: it builds the studio into the
/// db crate, prints an address on this machine with a token, and serves
/// the app's tables there, only with the token.
#[test]
#[ignore = "compiles a throwaway app with the studio; run with --ignored"]
fn the_studio_serves_the_apps_tables() {
	use std::io::BufRead;

	let root = tempfile::tempdir().unwrap();
	app(root.path());
	succeeds(&leptoskit(
		root.path(),
		&["db", "migration", "create", "init"],
	));
	let database = root.path().join("app.sqlite3");
	let url = format!("sqlite://{}", database.display());
	succeeds(&leptoskit(root.path(), &["db", "migrate", "--url", &url]));

	let target = std::env::var_os("LEPTOSKIT_E2E_TARGET_DIR")
		.map_or_else(|| root.path().join("target"), PathBuf::from);
	let mut studio = Command::new(env!("CARGO_BIN_EXE_leptoskit"))
		.args(["db", "studio", "--url", &url])
		.current_dir(root.path())
		.env("CARGO_TARGET_DIR", target)
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.spawn()
		.unwrap();
	let stdout = std::io::BufReader::new(studio.stdout.take().unwrap());
	let address = stdout.lines().map_while(Result::ok).find_map(|line| {
		line.strip_prefix("leptoskit db studio: ")
			.map(ToString::to_string)
	});
	let pages = address.as_ref().map(|address| {
		let (host, path) = address
			.strip_prefix("http://")
			.and_then(|rest| rest.split_once('/'))
			.unwrap();
		(
			request(host, "GET", &format!("/{path}"), ""),
			request(host, "GET", "/", ""),
			request(
				host,
				"GET",
				&format!("/{}", path.replacen('?', "schema?", 1)),
				"",
			),
		)
	});
	studio.kill().unwrap();
	let output = studio.wait_with_output().unwrap();

	let address = address.unwrap_or_else(|| {
		panic!(
			"the studio never printed its address:\n{}",
			String::from_utf8_lossy(&output.stderr)
		)
	});
	assert!(address.starts_with("http://127.0.0.1:"), "{address}");
	assert!(address.contains("/?token="), "{address}");
	let (tables, without_token, schema) = pages.unwrap();
	assert_eq!(tables.0, 200);
	assert!(tables.1.contains(">users</a>"), "{}", tables.1);
	assert_eq!(without_token.0, 403);
	assert!(schema.1.contains("<h2>users</h2>"), "{}", schema.1);
}

/// `leptoskit new --auth`: its tables migrate, its published API exports
/// without sign-in keys or a database, the server and the app build for the
/// server, the browser, and a native guest that loads in the native runtime,
/// the generated tests pass, the server signs someone in with an emailed code
/// from the development mailbox, and each person's notes are their own.
#[test]
#[ignore = "compiles a scaffolded server; run with --ignored"]
#[allow(
	clippy::too_many_lines,
	reason = "one scaffold's sign-in, native journey, and per-person probes read best top to \
	          bottom"
)]
fn the_auth_scaffold_signs_people_in() {
	let scratch = tempfile::tempdir().unwrap();
	let sdk = repository();
	succeeds(&leptoskit(
		scratch.path(),
		&[
			"new",
			"notes",
			"--auth",
			"--sdk-path",
			sdk.to_str().unwrap(),
		],
	));
	let root = scratch.path().join("notes");
	fs::copy(sdk.join("Cargo.lock"), root.join("Cargo.lock")).unwrap();

	let created = succeeds(&leptoskit(&root, &["db", "migration", "create", "init"]));
	assert!(created.contains("+ table users (3 columns)"), "{created}");
	assert!(created.contains("+ table notes (4 columns)"), "{created}");
	assert!(created.contains("+ table auth.sessions"), "{created}");
	assert_generated_notes_screen(&root, true);
	exports_without_the_environment(
		&root,
		&["notes", "add_note"],
		&scratch.path().join("client"),
	);

	let target = std::env::var_os("LEPTOSKIT_E2E_TARGET_DIR")
		.map_or_else(|| root.join("target"), PathBuf::from);
	let cargo = |args: &[&str]| {
		let status = Command::new(env!("CARGO"))
			.args(args)
			.current_dir(&root)
			.env("CARGO_TARGET_DIR", &target)
			.status()
			.unwrap();
		assert!(status.success(), "cargo {}", args.join(" "));
	};
	let address = free_address();
	point_native_at(&root, &format!("http://{address}"));
	cargo(&["build", "--locked", "--package", "notes"]);
	exports_migration_checkpoint_without_startup(&root, &target);
	cargo(&["test", "--locked", "--package", "notes_app"]);
	cargo(&[
		"check",
		"--locked",
		"--package",
		"notes_app",
		"--target",
		"wasm32-unknown-unknown",
		"--features",
		"hydrate",
	]);
	// The native guest is optimized, as `leptoskit build` makes it.
	cargo(&[
		"build",
		"--locked",
		"--release",
		"--package",
		"notes_app",
		"--target",
		"wasm32-unknown-unknown",
		"--features",
		"native",
	]);

	let origins = native_origins(&root);
	assert_eq!(origins, [format!("http://{address}")]);

	let origin = format!("Origin: http://{address}\r\n");
	let mut server = Command::new(target.join("debug/notes"))
		.current_dir(&root)
		.env("LEPTOS_SITE_ADDR", &address)
		.env(
			"AUTH_ENCRYPTION_KEYS",
			"test:BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc",
		)
		.env_remove("DATABASE_URL")
		.stdout(std::process::Stdio::null())
		.stderr(std::process::Stdio::null())
		.spawn()
		.unwrap();
	let ready = (0..120).any(|_| {
		std::thread::sleep(std::time::Duration::from_millis(250));
		std::net::TcpStream::connect(&address).is_ok()
	});
	let page = ready.then(|| request(&address, "GET", "/", ""));
	let sent = ready.then(|| {
		request(
			&address,
			"POST",
			"/auth/email/send",
			r#"{"email":"ada@example.com"}"#,
		)
	});
	let mail = ready.then(|| request(&address, "GET", "/_leptoskit/mail", ""));
	let code = mail.as_ref().and_then(|(_, body)| {
		let start = body.find("sign-in code: ")? + "sign-in code: ".len();
		Some(body[start..start + 9].to_owned())
	});

	let verified = code.as_ref().map(|code| {
		exchange(
			&address,
			"POST",
			"/auth/email/verify",
			&origin,
			&format!(r#"{{"email":"ada@example.com","code":"{code}"}}"#),
		)
	});
	let cookie = verified.as_ref().and_then(|(_, head, _)| {
		let line = head
			.lines()
			.find(|line| line.to_ascii_lowercase().starts_with("set-cookie:"))?;
		Some(
			line["set-cookie:".len()..]
				.split(';')
				.next()?
				.trim()
				.to_owned(),
		)
	});
	let session = cookie.as_ref().map(|cookie| {
		exchange(
			&address,
			"GET",
			"/auth/session",
			&format!("Cookie: {cookie}\r\n"),
			"",
		)
	});
	// The native journey runs before the direct probes, so its accounts start
	// with empty note lists.
	if ready {
		native_auth_journey(
			&target.join("wasm32-unknown-unknown/release/notes_app.wasm"),
			&address,
			&origins,
		);
	}

	// Each person's notes are their own: the operations run as the caller,
	// and the table's policy shows them only their rows.
	let notes = cookie
		.as_ref()
		.map(|ada| each_persons_notes(&address, &origin, ada));
	// The probes answer without a session.
	let probe = ready.then(|| request(&address, "GET", "/readyz", ""));
	server.kill().unwrap();

	server.wait().unwrap();

	assert!(ready, "the server never listened on {address}");
	let (status, body) = page.unwrap();
	assert_eq!(status, 200);
	assert!(body.contains("Checking whether you"), "{body}");
	assert_eq!(sent.unwrap().0, 204);
	let code = code.unwrap_or_else(|| panic!("no code in the mailbox: {mail:?}"));
	let (status, _, body) = verified.unwrap();
	assert_eq!(status, 200, "{body}");

	assert!(body.contains(r#""signed_up":true"#), "{body}");
	assert!(cookie.is_some(), "no session cookie for {code}");
	let (status, _, body) = session.unwrap();
	assert_eq!(status, 200);
	assert!(body.contains(r#""account_id""#), "{body}");
	assert_their_own(notes.unwrap());
	assert_eq!(probe.unwrap().0, 200);

	assert_eq!(
		succeeds(&leptoskit(&root, &["generate", "auth"])),
		"nothing changed: the sign-in screens are already in place\n"
	);
}

/// What Ada, signed in with `ada`, Bob, signed in here, and someone signed
/// out get from the notes: the signed-out calls, Ada's and Bob's adds, their
/// lists, and Ada's list after she signs out.
fn each_persons_notes(address: &str, origin: &str, ada: &str) -> Option<[(u16, String); 7]> {
	let (_, head, _) = exchange(
		address,
		"POST",
		"/auth/dev/sign-in",
		origin,
		r#"{"email":"bob@example.com"}"#,
	);
	let line = head
		.lines()
		.find(|line| line.to_ascii_lowercase().starts_with("set-cookie:"))?;
	let bob = line["set-cookie:".len()..]
		.split(';')
		.next()?
		.trim()
		.to_owned();
	let call = |cookie: Option<&str>, path: &str, body: &str| {
		let headers = match cookie {
			Some(cookie) => format!("{origin}Cookie: {cookie}\r\n"),
			None => origin.to_owned(),
		};

		let (status, _, body) = exchange(address, "POST", path, &headers, body);
		(status, body)
	};
	let anonymous = call(None, "/api/add_note", r#"{"body":"nobody's"}"#);
	let anonymous_list = call(None, "/api/notes", "{}");
	let added = call(Some(ada), "/api/add_note", r#"{"body":"ada's"}"#);
	let bobs = call(Some(&bob), "/api/add_note", r#"{"body":"bob's"}"#);

	let adas_list = call(Some(ada), "/api/notes", "{}");
	let bobs_list = call(Some(&bob), "/api/notes", "{}");
	// Signing out ends Ada's session; her old cookie must not read the list.
	let (..) = exchange(
		address,
		"POST",
		"/auth/sign-out",
		&format!("{origin}Cookie: {ada}\r\n"),
		"",
	);
	let after_sign_out = call(Some(ada), "/api/notes", "{}");

	Some([
		anonymous,
		anonymous_list,
		added,
		bobs,
		adas_list,
		bobs_list,
		after_sign_out,
	])
}

/// Someone signed out is refused, Ada and Bob each see only their own notes,
/// and a signed-out session's old cookie reads nothing.
fn assert_their_own(notes: Option<[(u16, String); 7]>) {
	let [
		anonymous,
		anonymous_list,
		added,
		bobs,
		adas_list,
		bobs_list,
		after_sign_out,
	] = notes.expect("dev sign-in sets Bob's cookie");
	let (status, body) = anonymous;
	assert_eq!(status, 401, "{body}");
	let (status, body) = anonymous_list;
	assert_eq!(status, 401, "{body}");
	let (status, body) = added;
	assert_eq!(status, 200, "{body}");
	assert!(body.contains(r#""body":"ada's""#), "{body}");

	assert_eq!(bobs.0, 200, "{}", bobs.1);
	let (status, body) = adas_list;
	assert_eq!(status, 200, "{body}");
	assert!(
		body.contains("ada's") && !body.contains("bob's"),
		"Ada sees only hers: {body}"
	);
	let (status, body) = bobs_list;
	assert_eq!(status, 200, "{body}");
	assert!(
		body.contains("bob's") && !body.contains("ada's"),
		"Bob sees only his: {body}"
	);
	let (status, body) = after_sign_out;
	assert_eq!(status, 401, "a signed-out cookie still read notes: {body}");
}

/// The generated native guest imports none of the browser APIs the query
/// cache and sign-in client use there (timers, the clock, `navigator`, the
/// window, `fetch`, event listeners), and takes the host's clock.
///
/// `leptos` itself still links js-sys's futures and stream glue; the
/// interpreter links those to traps, since nothing a host calls reaches them.
fn assert_native_guest_module(wasm: &[u8]) {
	let engine = wasmi::Engine::default();
	let module = wasmi::Module::new(&engine, wasm).unwrap();
	let browser: Vec<String> = module
		.imports()
		.map(|import| format!("{}::{}", import.module(), import.name()))
		.filter(|import| {
			[
				"setTimeout",
				"clearTimeout",
				"Date",
				"navigator",
				"Window",
				"fetch",
				"addEventListener",
				"visibilityState",
				"BroadcastChannel",
			]
			.iter()
			.any(|api| import.contains(api))
		})
		.collect();
	assert!(browser.is_empty(), "browser imports: {browser:?}");
	let modules: std::collections::BTreeSet<String> = module
		.imports()
		.map(|import| import.module().to_owned())
		.collect();
	assert!(
		modules.iter().all(|module| {
			["leptoskit", "leptoskit_plugins_v1"].contains(&module.as_str())
				|| module.starts_with("__wbindgen")
		}),
		"imports from {modules:?}"
	);
	assert!(
		module
			.exports()
			.any(|export| export.name() == "leptoskit_clock"),
		"the guest takes the host's clock"
	);
}

/// What the host does with one HTTP request the guest submitted.
#[derive(Clone, Copy)]
enum Fate {
	/// Run it now and deliver the answer.
	Run,
	/// Run it now, but keep the answer until the test releases it.
	Hold,
	/// Fail it without reaching the server.
	Fail(BridgeError),
}

/// One HTTP answer the test ran but has not delivered to the guest yet.
struct Held {
	id: leptoskit_bridge::CallId,
	reply: Result<Bytes, BridgeError>,
}

/// What the test does with each HTTP request the guest submits.
type FateFor = Box<dyn FnMut(&HttpRequest) -> Fate>;

/// A scaffolded native guest in the real interpreter, driven through the same
/// bridge and surface events a native host uses. HTTP requests go through the
/// generated guest's `http` plugin codec to the real scaffolded server, and
/// the secure store is an in-memory host implementation of the same protocol,
/// so no device or credential store is touched.
struct NativeApp {
	host: leptoskit_wasm::WasmHost<leptoskit_native::HeadlessBackend>,
	runtime: tokio::runtime::Runtime,
	http: leptoskit_plugin_http::Client,
	http_plugin: leptoskit_bridge::PluginId,
	secure_plugin: Option<leptoskit_bridge::PluginId>,
	secrets: BTreeMap<(String, String), Vec<u8>>,
	held: Vec<Held>,
	seen: Vec<String>,
	fate: Option<FateFor>,
}

impl NativeApp {
	fn new(wasm: &[u8], origins: &[String], secure_store: bool) -> Self {
		let mut host = leptoskit_wasm::WasmHost::headless(wasm).expect("the generated guest loads");
		let http_plugin = host
			.plugins()
			.register("http", [MethodId(0)])
			.expect("register the HTTP plugin");
		host.plugins()
			.grant(http_plugin, MethodId(0), true)
			.expect("grant the HTTP plugin");
		let secure_plugin = secure_store.then(|| {
			let methods = [MethodId(0), MethodId(1), MethodId(2)];
			let id = host
				.plugins()
				.register("secure-store", methods)
				.expect("register the secure store");
			for method in methods {
				host.plugins()
					.grant(id, method, true)
					.expect("grant the secure store");
			}

			id
		});
		let config = leptoskit_plugin_http::Config::new(origins.iter().map(String::as_str))
			.expect("the generated HTTP origins");
		let http = leptoskit_plugin_http::Client::new(config).expect("the HTTP client");
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.expect("the test runtime");
		Self {
			host,
			runtime,
			http,
			http_plugin,
			secure_plugin,
			secrets: BTreeMap::new(),
			held: Vec::new(),
			seen: Vec::new(),
			fate: None,
		}
	}

	fn start(&mut self) {
		self.host
			.start(390.0, 844.0)
			.expect("the generated guest starts");
	}

	/// Runs every task that can make progress and answers every request the
	/// test is not holding, until the guest is quiet.
	fn settle(&mut self) {
		let mut quiet = 0;

		for _ in 0..512 {
			let mut worked = self.drain();
			self.host.wake_plugins().expect("the generated guest wakes");
			// The wake may have queued requests the guest submitted while it
			// ran, so take them before deciding the guest is quiet.
			worked |= self.drain();

			if worked {
				quiet = 0;
			} else {
				quiet += 1;

				if quiet == 2 {
					return;
				}
			}
		}

		panic!("the generated guest did not settle");
	}

	/// Handles every queued request and cancellation, saying whether there was
	/// any.
	fn drain(&mut self) -> bool {
		let mut worked = false;

		loop {
			let request = self.host.plugins().next_request();
			let Some(request) = request else {
				break;
			};
			worked = true;
			self.handle(&request);
		}

		loop {
			if self.host.plugins().next_cancelled().is_none() {
				break;
			}

			worked = true;
		}

		worked
	}

	/// Settles with `fate` deciding what happens to each HTTP request.
	fn settle_with(&mut self, fate: impl FnMut(&HttpRequest) -> Fate + 'static) {
		self.fate = Some(Box::new(fate));
		self.settle();
		self.fate = None;
	}

	/// Delivers an answer the test held, reporting what the bridge did with a
	/// call the guest may already have cancelled.
	fn release(&mut self, index: usize) -> Result<(), BridgeError> {
		let held = self.held.remove(index);
		self.host.plugins().complete(held.id, held.reply)
	}

	fn handle(&mut self, request: &leptoskit_bridge::Request) {
		if request.plugin == self.http_plugin {
			let decoded = protocol::decode_request(&request.payload).expect("an HTTP frame");
			self.seen.push(format!(
				"{} {}",
				decoded.method.as_str(),
				url_path(&decoded.url)
			));
			let fate = self.fate.as_mut().map_or(Fate::Run, |fate| fate(&decoded));

			match fate {
				Fate::Run => {
					let reply = self.http_reply(&decoded);
					self.host
						.plugins()
						.complete(request.id, reply)
						.expect("the bridge accepts a live reply");
				}
				Fate::Hold => {
					let reply = self.http_reply(&decoded);
					self.held.push(Held {
						id: request.id,
						reply,
					});
				}
				Fate::Fail(error) => {
					self.host
						.plugins()
						.complete(request.id, Err(error))
						.expect("the bridge accepts an error reply");
				}
			}
		} else if Some(request.plugin) == self.secure_plugin {
			let reply = self.secure_reply(request.method, &request.payload);
			self.host
				.plugins()
				.complete(request.id, reply)
				.expect("the bridge accepts a secure-store reply");
		} else {
			panic!("the guest called an unregistered plugin");
		}
	}

	fn http_reply(&self, request: &HttpRequest) -> Result<Bytes, BridgeError> {
		let response = self
			.runtime
			.block_on(self.http.execute(request.clone()))
			.map_err(http_error)?;
		protocol::encode_response(&response)
	}

	/// The generated secure-store protocol, kept in this process: a GET is a
	/// presence flag and the secret, SET and DELETE are empty replies.
	fn secure_reply(&mut self, method: MethodId, payload: &[u8]) -> Result<Bytes, BridgeError> {
		let mut input = Decoder::new(payload);
		let service = input.string()?.to_owned();
		let key = input.string()?.to_owned();
		let secret = if method == MethodId(1) {
			Some(input.bytes()?.to_vec())
		} else {
			None
		};

		input.finish()?;

		match method.0 {
			0 => {
				let mut output = Encoder::default();

				if let Some(secret) = self.secrets.get(&(service, key)) {
					output.u32(1);
					output.bytes(secret)?;
				} else {
					output.u32(0);
				}

				Ok(output.finish())
			}
			1 => {
				self.secrets
					.insert((service, key), secret.unwrap_or_default());
				Ok(Bytes::new())
			}
			2 => {
				self.secrets.remove(&(service, key));
				Ok(Bytes::new())
			}
			_ => Err(BridgeError::Unavailable),
		}
	}

	fn nodes(&self) -> Vec<NodeId> {
		let surface = self.host.surface();
		let mut nodes = Vec::new();
		let mut stack: Vec<NodeId> = surface.root().into_iter().collect();

		while let Some(node) = stack.pop() {
			nodes.push(node);
			stack.extend(surface.children(node).iter().copied());
		}

		nodes
	}

	fn texts(&self) -> Vec<String> {
		let surface = self.host.surface();
		self.nodes()
			.into_iter()
			.filter_map(|node| surface.props(node).text.clone())
			.collect()
	}

	fn text(&self) -> String {
		self.texts().join("\n")
	}

	fn assert_text(&self, needle: &str) {
		let text = self.text();
		assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
	}

	fn assert_no_text(&self, needle: &str) {
		let text = self.text();
		assert!(!text.contains(needle), "unexpected {needle:?} in:\n{text}");
	}

	fn button(&self, label: &str) -> NodeId {
		let surface = self.host.surface();
		self.nodes()
			.into_iter()
			.find(|&node| {
				surface.kind(node) == NativeKind::Button
					&& surface.props(node).text.as_deref() == Some(label)
			})
			.unwrap_or_else(|| panic!("no {label:?} button in:\n{}", self.text()))
	}

	fn input(&self, placeholder: &str) -> NodeId {
		let surface = self.host.surface();
		self.nodes()
			.into_iter()
			.find(|&node| {
				surface.kind(node) == NativeKind::TextInput
					&& surface.props(node).placeholder.as_deref() == Some(placeholder)
			})
			.unwrap_or_else(|| panic!("no {placeholder:?} input in:\n{}", self.text()))
	}

	fn value(&self, placeholder: &str) -> Option<String> {
		let node = self.input(placeholder);
		self.host.surface().props(node).value.clone()
	}

	fn enabled(&self, label: &str) -> bool {
		let node = self.button(label);
		self.host.surface().props(node).enabled != Some(false)
	}

	fn press(&mut self, label: &str) {
		let node = self.button(label);
		let outcome = self
			.host
			.dispatch(node, &NativeEvent::Press)
			.expect("the generated guest handles a press");
		assert_eq!(outcome, EventOutcome::Handled, "the {label:?} handler ran");
	}

	fn type_into(&mut self, placeholder: &str, text: &str) {
		let node = self.input(placeholder);
		let outcome = self
			.host
			.dispatch(
				node,
				&NativeEvent::Change {
					value: text.to_owned(),
				},
			)
			.expect("the generated guest handles a change");
		assert_eq!(
			outcome,
			EventOutcome::Handled,
			"the {placeholder:?} handler ran"
		);
	}

	fn saw(&self, method: HttpMethod, path: &str) -> bool {
		self.seen
			.iter()
			.any(|seen| seen == &format!("{} {path}", method.as_str()))
	}
}

/// The path of an absolute HTTP URL, with its query.
fn url_path(url: &str) -> &str {
	let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
	rest.find('/').map_or("/", |index| &rest[index..])
}

/// The bridge error for an HTTP plugin failure, matching the host's mapping.
#[expect(
	clippy::needless_pass_by_value,
	reason = "the plugin returns an owned error, which map_err hands over"
)]
fn http_error(error: leptoskit_plugin_http::Error) -> BridgeError {
	match error {
		leptoskit_plugin_http::Error::Invalid(_) => BridgeError::InvalidArgument,
		leptoskit_plugin_http::Error::DestinationDenied => BridgeError::PermissionDenied,
		leptoskit_plugin_http::Error::TooLarge => BridgeError::TooLarge,
		leptoskit_plugin_http::Error::Backpressure => BridgeError::Busy,
		leptoskit_plugin_http::Error::Unavailable => BridgeError::Unavailable,
		leptoskit_plugin_http::Error::Timeout | leptoskit_plugin_http::Error::Transport(_) => {
			BridgeError::Native
		}
	}
}

/// Points the generated app's `SERVER` at `origin` before it compiles, and
/// grants the HTTP plugin exactly that origin in `leptoskit.toml`.
fn point_native_at(root: &Path, origin: &str) {
	let library = root.join("notes_app/src/lib.rs");
	let source = fs::read_to_string(&library).expect("the generated app library");
	let constant = "pub const SERVER: &str = \"http://localhost:3000\";";
	assert!(
		source.contains(constant),
		"the generated SERVER constant moved: {source}"
	);
	fs::write(
		&library,
		source.replace(constant, &format!("pub const SERVER: &str = \"{origin}\";")),
	)
	.expect("point the generated app at the fixture");

	let path = root.join("leptoskit.toml");
	let text = fs::read_to_string(&path).expect("the generated config");
	let mut document: toml_edit::DocumentMut = text.parse().expect("the config is TOML");
	let native = document["native"]
		.as_table_mut()
		.expect("the config has [native]");
	let options = native["plugin-options"]
		.as_table_mut()
		.expect("the config has [native.plugin-options]");
	let http = options["http"]
		.as_table_mut()
		.expect("the config grants the HTTP plugin");
	http["origins"] = toml_edit::value(toml_edit::Array::from_iter([origin]));
	fs::write(&path, document.to_string()).expect("write the generated config");
}

/// Removes the scaffold's SQLite database and its journal, with the server
/// stopped, so the next one starts empty.
fn remove_database(root: &Path) {
	for entry in fs::read_dir(root).expect("the generated project") {
		let entry = entry.expect("a project entry");

		if entry
			.file_name()
			.to_string_lossy()
			.starts_with("notes.sqlite3")
		{
			fs::remove_file(entry.path()).expect("remove the generated database");
		}
	}
}

/// The origins the generated config grants the HTTP plugin, after
/// [`point_native_at`] made it exactly the fixture.
fn native_origins(root: &Path) -> Vec<String> {
	let text = fs::read_to_string(root.join("leptoskit.toml")).expect("the generated config");
	let config: toml::Value = toml::from_str(&text).expect("the config is TOML");
	config["native"]["plugin-options"]["http"]["origins"]
		.as_array()
		.expect("the HTTP plugin lists its origins")
		.iter()
		.map(|origin| origin.as_str().expect("an origin string").to_owned())
		.collect()
}

/// The newest sign-in code in the development mailbox.
fn latest_code(address: &str) -> String {
	let (status, body) = request(address, "GET", "/_leptoskit/mail", "");
	assert_eq!(status, 200, "{body}");
	let marker = "sign-in code: ";
	let start = body
		.find(marker)
		.unwrap_or_else(|| panic!("no sign-in code in the mailbox: {body}"))
		+ marker.len();
	body[start..]
		.chars()
		.take_while(|character| character.is_ascii_alphanumeric() || *character == '-')
		.collect()
}

/// Signs in through the generated screen, the real server, and the
/// development mailbox.
fn sign_in(app: &mut NativeApp, address: &str, email: &str) {
	app.type_into("you@example.com", email);
	app.press("Email me a code");
	app.settle();
	app.assert_text(&format!("Check {email} for a code."));

	let code = latest_code(address);
	app.type_into("ABCD-EFGH", &code);
	app.press("Sign in");
	app.settle();
	app.assert_no_text("Checking whether you're signed in…");
}

/// A rate-limited address on a guest with no timer: the screen must keep the
/// server's message and leave a manual retry usable, not count down time it
/// cannot measure.
fn exercise_rate_limit(app: &mut NativeApp, _address: &str) {
	let email = "rate-limit@example.com";
	app.type_into("you@example.com", email);
	app.press("Email me a code");
	app.settle();
	app.assert_text(&format!("Check {email} for a code."));

	// The address may ask for five codes; the sixth is refused.
	for _ in 0..4 {
		app.press("Send a new code");
		app.settle();
	}

	app.press("Send a new code");
	app.settle();
	app.assert_text("Too many attempts");
	app.assert_text("try again in");
	assert!(
		app.enabled("Send a new code"),
		"a host without a timer must leave the manual retry usable"
	);

	app.assert_no_text("Try again in");

	// Pressing it reaches the server again; the server, not the screen, is
	// still the authority on when the window opens.
	app.press("Send a new code");
	app.settle();
	app.assert_text("Too many attempts");
	assert!(app.enabled("Send a new code"));

	app.press("Use a different address");
	app.settle();
	app.assert_text("Email me a code");
}

/// The database scaffold's actual journey: the real optimized guest loads
/// notes over the generated HTTP plugin, and the generated screen shows the
/// loading, empty, failed, pending, saved, and refreshed states.
fn native_database_journey(wasm: &Path, address: &str, origins: &[String]) {
	let wasm = fs::read(wasm).expect("the optimized native guest");
	assert_native_guest_module(&wasm);
	let mut app = NativeApp::new(&wasm, origins, false);
	app.start();
	app.assert_text("Loading notes…");

	// The first load fails, and the manual Refresh recovers to the empty list.
	app.settle_with(|request| {
		if request.method == HttpMethod::Post && url_path(&request.url) == "/api/notes" {
			Fate::Fail(BridgeError::Native)
		} else {
			Fate::Run
		}
	});
	app.assert_text("The notes couldn't be loaded");
	app.press("Refresh");
	app.settle();
	app.assert_text("No notes yet. The first one goes here.");
	app.assert_text("0 notes");

	// Validation happens before anything is sent.
	let before = app.seen.len();
	app.press("Add note");
	app.assert_text("Write a note before adding it.");
	assert_eq!(app.seen.len(), before, "an empty note reached the server");
	app.type_into("Write a note", &"x".repeat(501));
	app.press("Add note");
	app.assert_text("Keep the note to 500 characters.");
	assert_eq!(
		app.seen.len(),
		before,
		"an overlong note reached the server"
	);

	// Clear the overlong draft the validation left behind.
	app.type_into("Write a note", "");

	// A save stays pending until its answer arrives, then clears the draft.
	app.type_into("Write a note", "first note");
	app.press("Add note");
	app.settle_with(|request| {
		if request.method == HttpMethod::Post && url_path(&request.url) == "/api/add_note" {
			Fate::Hold
		} else {
			Fate::Run
		}
	});
	app.assert_text("Saving…");
	assert_eq!(app.held.len(), 1);
	app.release(0).expect("the pending save is still live");
	app.settle();
	app.assert_text("Saved.");

	app.assert_text("1 note");
	app.assert_text("first note");
	assert_eq!(
		app.value("Write a note").as_deref(),
		Some(""),
		"a saved note clears the draft"
	);

	// Refresh shows a note another client added.
	let (status, body) = request(
		address,
		"POST",
		"/api/add_note",
		r#"{"body":"second note"}"#,
	);
	assert_eq!(status, 200, "{body}");
	app.press("Refresh");
	app.settle();
	app.assert_text("2 notes");
	app.assert_text("second note");

	// A failed save is reported and leaves the draft in place.
	app.type_into("Write a note", "will fail");
	app.press("Add note");
	app.settle_with(|request| {
		if request.method == HttpMethod::Post && url_path(&request.url) == "/api/add_note" {
			Fate::Fail(BridgeError::Native)
		} else {
			Fate::Run
		}
	});
	app.assert_text("The note couldn't be saved");
	assert_eq!(app.value("Write a note").as_deref(), Some("will fail"));
	assert!(app.saw(HttpMethod::Post, "/api/add_note"));
}

/// The auth scaffold's actual journey: session resolution, an emailed code
/// from the development mailbox, notes over HTTP, sign-out, a second account
/// with no previous rows, and late replies after their screen was disposed.
fn native_auth_journey(wasm: &Path, address: &str, origins: &[String]) {
	let wasm = fs::read(wasm).expect("the optimized native guest");
	assert_native_guest_module(&wasm);
	let mut app = NativeApp::new(&wasm, origins, true);
	app.start();
	app.assert_text("Checking whether you're signed in…");
	app.settle();
	app.assert_text("Email me a code");

	exercise_rate_limit(&mut app, address);

	sign_in(&mut app, address, "ada@example.com");
	app.assert_text("No notes yet. The first one goes here.");
	app.assert_text("0 notes");
	app.type_into("Write a note", "ada native note");
	app.press("Add note");
	app.settle();
	app.assert_text("1 note");
	app.assert_text("ada native note");

	// Signing out drops the screen and its rows.
	app.press("Sign out");
	app.settle();
	app.assert_text("Email me a code");
	app.assert_no_text("ada native note");

	// Another account starts empty, not with the previous account's rows.
	sign_in(&mut app, address, "bob@example.com");
	app.assert_text("No notes yet. The first one goes here.");
	app.assert_no_text("ada native note");
	app.type_into("Write a note", "bob native note");
	app.press("Add note");
	app.settle();
	app.assert_text("bob native note");
	app.assert_no_text("ada native note");

	// Hold a list refresh, sign out, then deliver the old answer. It must
	// neither trap nor bring Bob's rows back on the sign-in screen.
	app.press("Refresh");
	app.settle_with(|request| {
		if request.method == HttpMethod::Post && url_path(&request.url) == "/api/notes" {
			Fate::Hold
		} else {
			Fate::Run
		}
	});
	assert_eq!(app.held.len(), 1);
	app.press("Sign out");
	app.settle();
	let late = app.release(0);
	assert!(
		matches!(late, Ok(()) | Err(BridgeError::InvalidCall)),
		"a late list reply: {late:?}"
	);
	app.settle();

	app.assert_text("Email me a code");
	app.assert_no_text("bob native note");

	// Back as Ada: only Ada's rows, never Bob's.
	sign_in(&mut app, address, "ada@example.com");
	app.assert_text("ada native note");
	app.assert_no_text("bob native note");

	// A save that completes after its screen is gone must not trap or
	// restore the old account's rows.
	app.type_into("Write a note", "late ada note");
	app.press("Add note");
	app.settle_with(|request| {
		if request.method == HttpMethod::Post && url_path(&request.url) == "/api/add_note" {
			Fate::Hold
		} else {
			Fate::Run
		}
	});
	app.assert_text("Saving…");
	assert_eq!(app.held.len(), 1);
	app.press("Sign out");
	app.settle();
	let late = app.release(0);

	assert!(
		matches!(late, Ok(()) | Err(BridgeError::InvalidCall)),
		"a late save reply: {late:?}"
	);
	app.settle();
	app.assert_text("Email me a code");
	app.assert_no_text("late ada note");

	// Bob again: no Ada rows, and the app still handles events after the late
	// completions.
	sign_in(&mut app, address, "bob@example.com");
	app.assert_text("bob native note");
	app.assert_no_text("ada native note");
	app.assert_no_text("late ada note");
	app.type_into("Write a note", "bob again");
	app.press("Add note");
	app.settle();
	app.assert_text("bob again");
}

#[test]
fn the_database_must_be_configured() {
	let root = tempfile::tempdir().unwrap();
	fs::write(
		root.path().join("leptoskit.toml"),
		"schema_version = 0\n\n[app]\nname = \"app\"\nversion = \"0.1.0\"\n",
	)
	.unwrap();
	let error = fails(&leptoskit(root.path(), &["db", "check"]));
	assert!(error.contains("add a `[db]` table"), "{error}");
}

/// The directory holding an installed Postgres's `initdb`, found where the
/// database crate's tests and `dev_server::test_binaries` look:
/// `LEPTOSKIT_POSTGRES_BIN` or `PGBIN`, the path, or the newest of Debian's
/// `/usr/lib/postgresql/*/bin`, which is where CI's Ubuntu runners keep it.
/// `None` skips the test, unless CI requires Postgres.
fn postgres_binaries() -> Option<PathBuf> {
	let has_initdb = |directory: &Path| {
		directory.join("initdb").exists() || directory.join("initdb.exe").exists()
	};
	let from_env = ["LEPTOSKIT_POSTGRES_BIN", "PGBIN"]
		.into_iter()
		.filter_map(std::env::var_os)
		.map(PathBuf::from);
	let from_path = std::env::var_os("PATH")
		.into_iter()
		.flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>());
	let mut debian: Vec<PathBuf> = fs::read_dir("/usr/lib/postgresql")
		.into_iter()
		.flatten()
		.filter_map(|entry| Some(entry.ok()?.path().join("bin")))
		.collect();
	debian.sort_by_key(|path| {
		path.parent()
			.and_then(|version| version.file_name()?.to_str()?.parse::<u32>().ok())
			.unwrap_or(0)
	});
	debian.reverse();
	let found = from_env
		.chain(from_path)
		.chain(debian)
		.find(|directory| has_initdb(directory));

	if found.is_none() {
		assert!(
			std::env::var_os("LEPTOSKIT_REQUIRE_POSTGRES").is_none(),
			"LEPTOSKIT_REQUIRE_POSTGRES is set, but no Postgres binaries were found"
		);
	}

	found
}

/// A Postgres project with no `DATABASE_URL`: `leptoskit db` runs the
/// development database itself, keeps its data between commands, and stops
/// it afterwards.
#[test]
fn postgres_commands_use_the_development_database() {
	let Some(binaries) = postgres_binaries() else {
		return;
	};
	let root = tempfile::tempdir().unwrap();
	let migration = root.path().join("migrations/20261030T000000_init");
	fs::create_dir_all(&migration).unwrap();
	fs::write(
		migration.join("migration.sql"),
		"CREATE TABLE \"notes\" (\"id\" uuid PRIMARY KEY);\n",
	)
	.unwrap();
	let snapshot = leptoskit_db_schema::Schema::empty(leptoskit_db_schema::Dialect::Postgres);
	fs::write(migration.join("schema.json"), snapshot.to_json()).unwrap();

	fs::write(
		root.path().join("migrations/registry.txt"),
		"20261030T000000_init\n",
	)
	.unwrap();
	let config = "schema_version = 0\n\n[app]\nname = \"my-app\"\nversion = \"0.1.0\"\n\n[db]\n";
	fs::write(root.path().join("leptoskit.toml"), config).unwrap();
	let run = |args: &[&str]| {
		Command::new(env!("CARGO_BIN_EXE_leptoskit"))
			.args(args)
			.current_dir(root.path())
			.env_remove("DATABASE_URL")
			.env("LEPTOSKIT_POSTGRES_BIN", &binaries)
			.env("XDG_CACHE_HOME", root.path().join("cache"))
			.output()
			.expect("run leptoskit")
	};

	let migrated = succeeds(&run(&["db", "migrate"]));
	assert!(
		migrated.contains("using the development database postgres://postgres:***@localhost:"),
		"{migrated}"
	);
	assert!(migrated.contains("/my_app?"), "{migrated}");
	assert!(
		migrated.contains("applied 20261030T000000_init"),
		"{migrated}"
	);
	let data = root.path().join(".leptoskit/db/postgres/data");
	assert!(!data.join("postmaster.pid").exists(), "the server stopped");

	assert!(succeeds(&run(&["db", "migrate"])).contains("up to date"));
	assert!(succeeds(&run(&["db", "reset", "--yes"])).contains("applied 1"));
	assert!(!root.path().join("cache").exists());

	fs::write(
		root.path().join("leptoskit.toml"),
		format!("{config}dev_server = \"off\"\n"),
	)
	.unwrap();
	assert!(fails(&run(&["db", "migrate"])).contains("set DATABASE_URL"));
}
