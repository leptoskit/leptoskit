//! A worker that rehearses operations without a cloud provider.
//!
//! Plans are real as far as they can be without credentials: the executor
//! runs the same generator `lk infra generate` does, then compares the
//! resources with the ones it last "applied". Everything after that is a
//! simulation, and each operation says so in its first step.
//!
//! With [`Simulated::persisted`], Apply also records the applied
//! `leptoskit.toml` durably, so a restarted worker recovers what the
//! simulated cloud last applied.

use std::future::Future;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context as _;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;
use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::Executor;
use leptoskit_deploy_console::control::Operation;
use leptoskit_deploy_console::control::Project;
use leptoskit_deploy_console::control::RunResult;
use leptoskit_deploy_console::control::Service;
use serde_json::Value;
use serde_json::json;

const NOTICE: &str = "Demo executor: simulated, no cloud provider is contacted.";

pub struct Simulated {
	service: Service,
	/// The `leptoskit.toml` the simulated cloud last applied.
	applied: Arc<Mutex<String>>,
	/// When set, Apply durably replaces this file with the applied text.
	persistence: Option<PathBuf>,
}

impl Simulated {
	pub fn new(service: Service, applied: &str) -> Self {
		Self {
			service,
			applied: Arc::new(Mutex::new(applied.to_owned())),
			persistence: None,
		}
	}

	/// Builds the executor from the applied configuration last written to
	/// `path`, so a restarted worker recovers what the simulated cloud last
	/// applied.
	///
	/// A missing file falls back to `committed`, and construction writes
	/// nothing: the file appears with the first persisted Apply. A missing
	/// file is the only fallback — a file that exists but can't be read,
	/// through permissions or bytes that aren't UTF-8, is an error, never a
	/// silent reset to `committed`.
	///
	/// Apply replaces `path` atomically and durably, and reports success
	/// only once the text is on disk; see [`persist_applied`].
	///
	/// The caller must hold the generation's exclusive worker file lock
	/// before constructing the executor and keep holding it for as long as
	/// the executor runs: this worker is the only reader and writer of
	/// `path`, and no locking happens here.
	pub fn persisted(service: Service, committed: &str, path: &Path) -> Result<Self> {
		let applied = match std::fs::read_to_string(path) {
			Ok(text) => text,
			Err(error) if error.kind() == io::ErrorKind::NotFound => committed.to_owned(),
			Err(error) => {
				return Err(anyhow::Error::new(error).context(format!(
					"the simulated cloud's applied state at {} can't be read",
					path.display()
				)));
			}
		};

		Ok(Self {
			service,
			applied: Arc::new(Mutex::new(applied)),
			persistence: Some(path.to_owned()),
		})
	}
}

impl Executor for Simulated {
	fn run(
		&self,
		project: Project,
		operation: Operation,
	) -> Pin<Box<dyn Future<Output = RunResult> + Send>> {
		let service = self.service.clone();
		let applied = self.applied.clone();
		let persistence = self.persistence.clone();
		Box::pin(async move {
			let step = |message: &str| {
				let _ = service.progress(&operation.id, message);
			};
			step(NOTICE);

			match rehearse(
				&project.root,
				&operation.intent.action,
				&applied,
				persistence.as_deref(),
				step,
			)
			.await
			{
				Ok(result) => result,
				Err(error) => {
					RunResult {
						succeeded: false,
						message: format!("The demo couldn't rehearse this: {error:#}"),
						details: json!({ "code": "demo_failed" }),
					}
				}
			}
		})
	}
}

async fn pause() {
	tokio::time::sleep(Duration::from_millis(1500)).await;
}

async fn rehearse(
	root: &Path,
	action: &Action,
	applied: &Mutex<String>,
	persistence: Option<&Path>,
	step: impl Fn(&str),
) -> Result<RunResult> {
	let working = std::fs::read_to_string(root.join("leptoskit.toml"))?;
	let ok = |message: &str, details: Value| {
		Ok(RunResult {
			succeeded: true,
			message: message.to_owned(),
			details,
		})
	};

	match action {
		Action::Plan => {
			step("Generating infra/notes-production/main.tf.json from leptoskit.toml");
			let wanted = generate(root, root)?;
			pause().await;
			let scratch = root.join(".git/admin-demo-applied");
			std::fs::create_dir_all(&scratch)?;
			std::fs::write(scratch.join("leptoskit.toml"), applied_text(applied))?;
			let existing = generate(&scratch, &scratch)?;
			step("Comparing with what the simulated cloud has");
			pause().await;
			let changes = compare(&existing, &wanted);
			step(&match changes.len() {
				0 => "Nothing would change".to_owned(),
				1 => "1 resource would change".to_owned(),
				count => format!("{count} resources would change"),
			});
			ok(
				"Completed. The plan was compared with the simulated cloud.",
				json!({ "code": "completed", "changes": changes }),
			)
		}
		Action::Apply { .. } => {
			for message in [
				"Locking the simulated state",
				"Applying the reviewed plan",
				"Waiting for the new host to answer /health",
			] {
				step(message);
				pause().await;
			}

			let details = match persistence {
				// Memory-only: the demo binary's behaviour, nothing on disk.
				None => {
					set_applied(applied, &working)?;
					json!({ "code": "completed" })
				}
				Some(path) => {
					let digest = persist_applied(path, &working, applied)?;
					json!({ "code": "completed", "applied_sha256": digest })
				}
			};

			ok(
				"Completed. The simulated cloud matches leptoskit.toml.",
				details,
			)
		}
		Action::Deploy {
			version,
			setup,
			maintenance,
		} => {
			if *setup {
				step("Creating the database and running every migration");
				pause().await;
			}

			if *maintenance {
				step("Telling live connections to come back later");
				pause().await;
			}

			for message in [
				format!("Building the image for {version}"),
				format!("Starting {version} on web-0"),
				"Waiting for web-0 to answer /health".to_owned(),
			] {
				step(&message);
				pause().await;
			}

			// `bad` stands in for a release that fails its health check.
			if version == "bad" {
				return Ok(RunResult {
					succeeded: false,
					message: "web-0 didn't answer /health with the new image, so traffic stayed \
					          on the previous release."
						.into(),
					details: json!({ "code": "health_check_failed" }),
				});
			}

			step(&format!("Moving traffic to {version}"));
			pause().await;
			ok(
				&format!("Completed. {version} is serving."),
				json!({ "code": "completed" }),
			)
		}
		Action::Rollback { version } => {
			step(&format!("Moving traffic back to {version}"));
			pause().await;
			ok(
				&format!("Completed. {version} is serving again."),
				json!({ "code": "completed" }),
			)
		}
		Action::Status => {
			let hosts = hosts(&applied_text(applied));
			step(&format!("Probing {hosts} hosts"));
			pause().await;
			ok(
				&format!("{hosts} of {hosts} hosts answered /health (simulated)."),
				json!({ "code": "completed" }),
			)
		}
		Action::Backup => {
			for message in [
				"Fetching the newest backup from notes-backups",
				"Restoring it into a scratch database",
			] {
				step(message);
				pause().await;
			}

			ok(
				"The newest backup restored cleanly (simulated).",
				json!({ "code": "completed" }),
			)
		}
	}
}

fn applied_text(applied: &Mutex<String>) -> String {
	applied.lock().map(|text| text.clone()).unwrap_or_default()
}

/// Sets the in-memory applied text, guarding against a poisoned lock.
fn set_applied(applied: &Mutex<String>, text: &str) -> Result<()> {
	let mut guard = applied
		.lock()
		.map_err(|_| anyhow::anyhow!("the applied state is poisoned"))?;
	text.clone_into(&mut guard);

	Ok(())
}

/// Replaces the applied file at `path` with `text` atomically and durably,
/// and returns the SHA-256 of the bytes now in it.
///
/// The text goes to a uniquely named temporary file in the same directory,
/// is written and synced, and is then renamed over the target; finally the
/// directory itself is synced, so the rename survives a power cut. Only
/// after all of that does the caller report success.
///
/// A failure before the rename leaves the previous file and the in-memory
/// state untouched. A failure after it cannot: the file already holds the
/// new text, so the memory is brought in step with it and the error says
/// the replacement happened but its durability is unconfirmed — the
/// operation reports failure without claiming the old state survived.
///
/// The caller must already hold the worker's exclusive file lock; no
/// concurrent writer is expected or handled here.
fn persist_applied(path: &Path, text: &str, applied: &Mutex<String>) -> Result<String> {
	use sha2::Digest as _;

	let parent = parent_of(path);
	let mut temporary = tempfile::Builder::new()
		.prefix(".applied.")
		.suffix(".tmp")
		.tempfile_in(parent)
		.with_context(|| {
			format!(
				"the applied state at {} couldn't be written",
				path.display()
			)
		})?;

	temporary
		.write_all(text.as_bytes())
		.and_then(|()| temporary.as_file().sync_all())
		.with_context(|| {
			format!(
				"the applied state at {} couldn't be written",
				path.display()
			)
		})?;
	let digest = hex::encode(sha2::Sha256::digest(text.as_bytes()));

	// From a successful rename on, the file holds the new text whatever
	// happens next, so the memory is brought in step before anything else
	// is reported.
	if let Err(error) = temporary.persist(path) {
		return Err(anyhow::Error::new(error.error).context(format!(
			"the applied state at {} couldn't be replaced",
			path.display()
		)));
	}

	set_applied(applied, text).context(
		"the applied state was replaced, but the executor's memory of it couldn't be updated",
	)?;

	if let Err(error) = sync_directory(parent) {
		return Err(anyhow::anyhow!(
			"the applied state at {} was replaced with the new configuration, but confirming it \
			 is durable failed, so it may not survive a restart: {error}",
			path.display()
		));
	}

	Ok(digest)
}

/// The directory holding `path`; `.` for a bare file name.
fn parent_of(path: &Path) -> &Path {
	path.parent()
		.filter(|parent| !parent.as_os_str().is_empty())
		.unwrap_or(Path::new("."))
}

/// Makes a rename or new entry in `directory` durable. Unix needs an fsync
/// of the directory itself; Windows cannot open a directory as a file
/// through `std`, and NTFS journals the metadata change — the same
/// trade-off the codepush store makes, stated here rather than silently
/// skipped.
fn sync_directory(directory: &Path) -> io::Result<()> {
	#[cfg(test)]
	if injected_directory_sync_failure(directory) {
		return Err(io::Error::other("the directory sync failed"));
	}

	if cfg!(unix) {
		std::fs::File::open(directory)?.sync_all()?;
	}

	Ok(())
}

/// Whether the test seam has armed a directory-sync failure for this
/// directory. Only a matching directory consumes the arming, so parallel
/// tests with their own directories stay unaffected.
#[cfg(test)]
fn injected_directory_sync_failure(directory: &Path) -> bool {
	FAIL_NEXT_DIRECTORY_SYNC.lock().is_ok_and(|mut pending| {
		let matches = pending.as_deref() == Some(directory);

		if matches {
			*pending = None;
		}

		matches
	})
}

fn hosts(config: &str) -> usize {
	config
		.lines()
		.find_map(|line| line.trim().strip_prefix("instances = "))
		.and_then(|count| count.trim().parse().ok())
		.unwrap_or(1)
}

/// Runs the real generator on `config_root`'s `leptoskit.toml`, writing
/// under `out`, and returns the resources.
fn generate(config_root: &Path, out: &Path) -> Result<Value> {
	let config = LeptoskitConfig::load_from(config_root)?;
	let infra = config
		.managed_infra()
		.context("the demo project needs a managed [infra]")?;
	let dir = out.join("infra").join(&infra.name);
	leptoskit_cli::infra::generate(&dir, infra, true)?;
	let text = std::fs::read_to_string(dir.join("main.tf.json"))?;
	Ok(serde_json::from_str::<Value>(&text)?["resource"].clone())
}

/// What a plan would report: each resource instance created, deleted, or
/// updated between the two generated files.
fn compare(existing: &Value, wanted: &Value) -> Vec<Value> {
	let instances = |resources: &Value| {
		let mut found = std::collections::BTreeMap::new();

		for (kind, named) in resources.as_object().into_iter().flatten() {
			for (name, body) in named.as_object().into_iter().flatten() {
				let mut body = body.clone();
				let count = body.get("count").cloned();
				let each = body.get("for_each").cloned();

				if let Some(fields) = body.as_object_mut() {
					fields.remove("count");
					fields.remove("for_each");
				}

				// Addresses as OpenTofu writes them: `web[0]` for `count`,
				// `replica["1"]` for `for_each`.
				let addresses: Vec<String> = match (count, each) {
					(Some(count), _) => {
						(0..count.as_u64().unwrap_or(1))
							.map(|index| format!("{kind}.{name}[{index}]"))
							.collect()
					}
					(None, Some(Value::Object(keys))) => {
						keys.keys()
							.map(|key| format!("{kind}.{name}[\"{key}\"]"))
							.collect()
					}
					_ => vec![format!("{kind}.{name}")],
				};

				for address in addresses {
					found.insert(address, body.clone());
				}
			}
		}

		found
	};
	let before = instances(existing);
	let after = instances(wanted);
	let mut changes = Vec::new();

	for (address, body) in &after {
		match before.get(address) {
			None => changes.push(json!({ "address": address, "actions": ["create"] })),
			Some(old) if old != body => {
				changes.push(json!({ "address": address, "actions": ["update"] }));
			}
			Some(_) => {}
		}
	}

	for address in before
		.keys()
		.filter(|address| !after.contains_key(*address))
	{
		changes.push(json!({ "address": address, "actions": ["delete"] }));
	}

	changes
}

/// The directory whose next `sync_directory` should fail, so the
/// post-replacement error path can be exercised deterministically.
#[cfg(test)]
static FAIL_NEXT_DIRECTORY_SYNC: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
mod tests {
	use leptoskit_deploy_console::control::Intent;
	use leptoskit_deploy_console::control::Phase;
	use leptoskit_deploy_console::control::Store;

	use super::*;

	/// The demo project's configuration as committed: one host.
	const COMMITTED: &str = include_str!("../project/leptoskit.toml");
	/// The test service's owner credential. Nothing outside the test's own
	/// in-process store accepts it.
	const OWNER_KEY: &str = "admin-demo-test-owner-key-not-a-secret-0123456789";

	/// The committed configuration edited to three hosts, as the admin
	/// stages it for the demo's plan.
	fn wanted() -> String {
		COMMITTED.replace("instances = 1", "instances = 3")
	}

	/// An in-process control service over this directory's own store.
	fn service(root: &Path) -> Result<Service> {
		let store = Store::open(&root.join("console.sqlite3"))?;
		Service::new(store, OWNER_KEY, "http://127.0.0.1:9")
	}

	fn project(root: &Path) -> Project {
		Project {
			id: "notes-production".into(),
			environment: "production".into(),
			root: root.to_path_buf(),
			revision: 0,
		}
	}

	fn operation(id: &str, action: Action) -> Operation {
		Operation {
			events: Vec::new(),
			id: id.to_owned(),
			project: "notes-production".into(),
			actor: None,
			revision: 0,
			intent: Intent {
				expected_revision: 0,
				idempotency_key: format!("{id}-idempotency"),
				action,
			},
			phase: Phase::Running,
			created: 0,
			updated: 0,
			message: String::new(),
			result: Value::Null,
		}
	}

	async fn run(executor: &Simulated, root: &Path, id: &str, action: Action) -> RunResult {
		executor.run(project(root), operation(id, action)).await
	}

	/// The host count a Status operation reports, read from its message.
	async fn status_hosts(executor: &Simulated, root: &Path, id: &str) -> usize {
		let result = run(executor, root, id, Action::Status).await;
		assert!(result.succeeded, "{}", result.message);
		result
			.message
			.split_once(" of ")
			.and_then(|(hosts, _)| hosts.parse::<usize>().ok())
			.unwrap_or_else(|| panic!("unexpected status message: {}", result.message))
	}

	/// Every entry of `directory`, sorted.
	fn directory_entries(directory: &Path) -> Vec<String> {
		let mut names: Vec<_> = std::fs::read_dir(directory)
			.expect("read the state directory")
			.map(|entry| {
				entry
					.expect("entry")
					.file_name()
					.to_string_lossy()
					.into_owned()
			})
			.collect();
		names.sort();
		names
	}

	/// Arms the seam that fails the next sync of this directory.
	fn fail_next_directory_sync(directory: &Path) {
		*FAIL_NEXT_DIRECTORY_SYNC
			.lock()
			.expect("the sync failure flag isn't poisoned") = Some(directory.to_path_buf());
	}

	/// A project directory and the state directory that holds the applied
	/// file, both private to the test.
	fn workspace() -> Result<(tempfile::TempDir, PathBuf, PathBuf)> {
		let directory = tempfile::tempdir()?;
		let project_root = directory.path().join("notes");
		let state = directory.path().join("state");
		std::fs::create_dir_all(&project_root)?;
		std::fs::create_dir_all(&state)?;

		// The control store refuses a directory any other account can read.
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
		}

		std::fs::write(project_root.join("leptoskit.toml"), wanted())?;
		Ok((directory, project_root, state))
	}

	#[tokio::test(start_paused = true)]
	async fn a_restarted_worker_recovers_the_last_applied_configuration() -> Result<()> {
		let workspace = workspace()?;
		let (directory, project_root, state) = (&workspace.0, &workspace.1, &workspace.2);
		let applied_path = state.join("applied.toml");
		let service = service(directory.path())?;

		let worker = Simulated::persisted(service.clone(), COMMITTED, &applied_path)?;
		let apply = run(
			&worker,
			project_root,
			"apply-1",
			Action::Apply {
				plan_id: "plan-1".into(),
			},
		)
		.await;
		assert!(apply.succeeded, "{}", apply.message);

		// A fresh executor, as a restarted worker builds one, recovers what
		// the simulated cloud last applied.
		let restarted = Simulated::persisted(service, COMMITTED, &applied_path)?;
		assert_eq!(status_hosts(&restarted, project_root, "status-1").await, 3);

		Ok(())
	}

	#[tokio::test(start_paused = true)]
	async fn a_persisted_apply_reports_the_digest_of_the_file_it_wrote() -> Result<()> {
		let workspace = workspace()?;
		let (directory, project_root, state) = (&workspace.0, &workspace.1, &workspace.2);
		let applied_path = state.join("applied.toml");

		let worker = Simulated::persisted(service(directory.path())?, COMMITTED, &applied_path)?;
		let apply = run(
			&worker,
			project_root,
			"apply-1",
			Action::Apply {
				plan_id: "plan-1".into(),
			},
		)
		.await;
		assert!(apply.succeeded, "{}", apply.message);

		use sha2::Digest as _;
		assert_eq!(
			apply.details["applied_sha256"],
			Value::String(hex::encode(sha2::Sha256::digest(std::fs::read(
				&applied_path
			)?))),
		);

		Ok(())
	}

	#[cfg(unix)]
	#[tokio::test(start_paused = true)]
	async fn a_failed_apply_before_replacement_preserves_the_file_and_the_memory() -> Result<()> {
		let workspace = workspace()?;
		let (directory, project_root, state) = (&workspace.0, &workspace.1, &workspace.2);
		let applied_path = state.join("applied.toml");
		std::fs::write(&applied_path, COMMITTED)?;
		let worker = Simulated::persisted(service(directory.path())?, COMMITTED, &applied_path)?;
		assert_eq!(
			status_hosts(&worker, project_root, "status-before").await,
			1
		);

		// An unwritable state directory fails the temporary file's creation
		// before anything is replaced. This boundary is reported, not
		// pretended identical everywhere: root ignores directory modes, and
		// Windows' directory permissions mean something else.
		use std::os::unix::fs::PermissionsExt as _;
		let writable =
			|mode: u32| std::fs::set_permissions(state, std::fs::Permissions::from_mode(mode));
		writable(0o500)?;

		if std::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(state.join("canary"))
			.is_ok()
		{
			// This account ignores directory modes, so the failure can't be
			// reproduced here; leave the boundary untested.
			writable(0o700)?;
			return Ok(());
		}

		let apply = run(
			&worker,
			project_root,
			"apply-1",
			Action::Apply {
				plan_id: "plan-1".into(),
			},
		)
		.await;
		assert!(!apply.succeeded, "{}", apply.message);
		assert_eq!(apply.details["code"], "demo_failed");
		assert_eq!(std::fs::read_to_string(&applied_path)?, COMMITTED);
		assert_eq!(status_hosts(&worker, project_root, "status-after").await, 1);
		// No temporary file was left behind to block later applies.
		assert_eq!(directory_entries(state), ["applied.toml"]);

		writable(0o700)?;
		Ok(())
	}

	#[tokio::test(start_paused = true)]
	async fn a_missing_file_uses_the_committed_configuration_until_the_first_apply() -> Result<()> {
		let workspace = workspace()?;
		let (directory, project_root, state) = (&workspace.0, &workspace.1, &workspace.2);
		let applied_path = state.join("applied.toml");

		let worker = Simulated::persisted(service(directory.path())?, COMMITTED, &applied_path)?;
		assert!(!applied_path.exists());
		assert_eq!(status_hosts(&worker, project_root, "status-1").await, 1);
		assert!(!applied_path.exists(), "status wrote nothing");

		let apply = run(
			&worker,
			project_root,
			"apply-1",
			Action::Apply {
				plan_id: "plan-1".into(),
			},
		)
		.await;
		assert!(apply.succeeded, "{}", apply.message);
		assert_eq!(std::fs::read_to_string(&applied_path)?, wanted());

		Ok(())
	}

	#[tokio::test(start_paused = true)]
	async fn the_memory_only_executor_never_creates_persistence_files() -> Result<()> {
		let workspace = workspace()?;
		let (directory, project_root, state) = (&workspace.0, &workspace.1, &workspace.2);

		let worker = Simulated::new(service(directory.path())?, COMMITTED);
		let apply = run(
			&worker,
			project_root,
			"apply-1",
			Action::Apply {
				plan_id: "plan-1".into(),
			},
		)
		.await;
		assert!(apply.succeeded, "{}", apply.message);
		assert_eq!(apply.details["code"], "completed");
		assert_eq!(apply.details["applied_sha256"], Value::Null);
		assert!(
			directory_entries(state).is_empty(),
			"the state directory stayed untouched: {:?}",
			directory_entries(state)
		);
		assert_eq!(status_hosts(&worker, project_root, "status-1").await, 3);

		Ok(())
	}

	#[tokio::test(start_paused = true)]
	async fn unreadable_or_malformed_applied_state_is_an_error_not_a_reset() -> Result<()> {
		let workspace = workspace()?;
		let (directory, _project_root, state) = (&workspace.0, &workspace.1, &workspace.2);
		let service = service(directory.path())?;

		// Malformed: the file holds bytes that aren't UTF-8.
		let malformed = state.join("applied.toml");
		std::fs::write(&malformed, [0xFF, 0xFE])?;

		let Err(error) = Simulated::persisted(service.clone(), COMMITTED, &malformed) else {
			panic!("malformed applied state must be an error");
		};
		assert!(
			error.to_string().contains("applied.toml"),
			"the error names the file: {error}"
		);
		assert_eq!(std::fs::read(&malformed)?, [0xFF, 0xFE]);

		// Unreadable in another way: the path is a directory.
		let occupied = state.join("occupied");
		std::fs::create_dir(&occupied)?;

		let Err(error) = Simulated::persisted(service, COMMITTED, &occupied) else {
			panic!("a directory at the applied path must be an error");
		};
		assert!(
			error.to_string().contains("occupied"),
			"the error names the path: {error}"
		);
		assert!(occupied.is_dir());

		Ok(())
	}

	#[tokio::test(start_paused = true)]
	async fn a_failed_directory_sync_reports_failure_with_memory_matching_the_file() -> Result<()> {
		let workspace = workspace()?;
		let (directory, project_root, state) = (&workspace.0, &workspace.1, &workspace.2);
		let applied_path = state.join("applied.toml");
		std::fs::write(&applied_path, COMMITTED)?;
		let worker = Simulated::persisted(service(directory.path())?, COMMITTED, &applied_path)?;
		fail_next_directory_sync(state);

		let apply = run(
			&worker,
			project_root,
			"apply-1",
			Action::Apply {
				plan_id: "plan-1".into(),
			},
		)
		.await;
		assert!(!apply.succeeded, "{}", apply.message);
		assert_eq!(apply.details["code"], "demo_failed");
		assert!(
			apply.message.contains("durable"),
			"the uncertainty must be explicit: {}",
			apply.message
		);

		// The file was replaced, and the memory says so too.
		assert_eq!(std::fs::read_to_string(&applied_path)?, wanted());
		assert_eq!(status_hosts(&worker, project_root, "status-1").await, 3);
		assert_eq!(directory_entries(state), ["applied.toml"]);

		Ok(())
	}
}
