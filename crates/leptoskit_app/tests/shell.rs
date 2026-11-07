//! The console's shell: what it sends, how a refusal lands, and the
//! management operations it builds from the account's state. Nothing
//! listens on a server, so every request fails at once and the shell says
//! so; the state it builds before sending is what these tests read.
use std::cell::RefCell;

use futures::executor::LocalPool;
use futures::executor::LocalSpawner;
use futures::task::LocalSpawnExt as _;
use leptoskit_app::Shell;
use leptoskit_app::client;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
use serde_json::json;

thread_local! {
	static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
	static SPAWNER: LocalSpawner = POOL.with_borrow(LocalPool::spawner);
}

/// Each test thread's own task pool, run by [`settle`], as a native app
/// runs its own.
struct Pool;

impl any_spawner::CustomExecutor for Pool {
	fn spawn(&self, task: any_spawner::PinnedFuture<()>) {
		self.spawn_local(task);
	}

	fn spawn_local(&self, task: any_spawner::PinnedLocalFuture<()>) {
		let _ = SPAWNER.try_with(|spawner| spawner.spawn_local(task));
	}

	fn poll_local(&self) {}
}

/// Runs every task the shell started that can make progress.
fn settle() {
	POOL.with_borrow_mut(LocalPool::run_until_stalled);
}

/// A shell in its own reactive owner, with the test thread's executor
/// installed so its requests can run.
fn shell() -> (Owner, Shell) {
	SPAWNER.with(|_| ());
	// Both halves of the executor trait: nothing waits on either here, but
	// the platform calls them.
	let pool = Pool;
	any_spawner::CustomExecutor::poll_local(&pool);
	any_spawner::CustomExecutor::spawn(&pool, Box::pin(async {}));
	let _ = any_spawner::Executor::init_local_custom_executor(pool);
	let owner = Owner::new();
	let shell = owner.with(Shell::new);
	(owner, shell)
}

#[test]
fn a_request_says_when_it_cannot_reach_a_service() {
	let (_owner, shell) = shell();
	shell.notice.set("stale".into());
	shell.output.set("stale".into());
	shell.request("/api/v1/overview".into(), None);
	assert!(shell.busy.get_untracked());
	assert_eq!(shell.notice.get_untracked(), "Working…");
	assert_eq!(shell.output.get_untracked(), "");
	settle();
	assert!(!shell.busy.get_untracked());
	assert_eq!(
		shell.notice.get_untracked(),
		"Run the app in a browser or native host"
	);
	assert_eq!(shell.output.get_untracked(), "");
}

#[test]
fn a_request_while_one_is_running_is_dropped() {
	let (_owner, shell) = shell();
	shell.busy.set(true);
	shell.request("/api/v1/overview".into(), None);
	assert_eq!(shell.notice.get_untracked(), "");
	settle();
	// The dropped request left the shell busy and said nothing.
	assert!(shell.busy.get_untracked());
	assert_eq!(shell.notice.get_untracked(), "");
}

#[test]
fn a_management_operation_waits_for_the_projects_revision() {
	let (_owner, shell) = shell();
	shell.operation("notes", &json!({"kind": "plan"}));
	assert_eq!(
		shell.notice.get_untracked(),
		"Refresh application status before deploying."
	);
	assert!(!shell.busy.get_untracked());

	shell.overview.set(json!({
		"projects": [
			{"id": "other", "revision": 2},
			{"id": "notes", "revision": 7},
		]
	}));
	shell.operation("notes", &json!({"kind": "deploy"}));
	// The request it built is on its way; no server answers it.
	assert!(shell.busy.get_untracked());
	settle();
	assert_eq!(
		shell.notice.get_untracked(),
		"Run the app in a browser or native host"
	);
}

#[test]
fn signing_out_clears_everything_the_screen_holds() {
	let (_owner, shell) = shell();
	shell.key.set("account-key".into());
	shell.output.set("one-use code".into());
	shell.notice.set("notice".into());
	shell.account.set(Some(
		leptoskit_deploy_console::hosted_protocol::AccountView {
			id: "example".into(),
			status: "active".into(),
			plan: "team".into(),
			installations: Vec::new(),
		},
	));
	shell.overview.set(json!({"projects": []}));
	shell.measurement.set(json!({"instances": 1}));
	shell.measured_project.set("notes".into());
	shell.logout();
	assert_eq!(shell.key.get_untracked(), "");
	assert_eq!(shell.output.get_untracked(), "");
	assert_eq!(shell.notice.get_untracked(), "");
	assert!(shell.account.get_untracked().is_none());
	assert!(shell.overview.get_untracked().is_null());
	assert!(shell.measurement.get_untracked().is_null());
	assert_eq!(shell.measured_project.get_untracked(), "");
}

#[test]
fn a_default_shell_starts_empty() {
	let owner = Owner::new();
	let shell = owner.with(Shell::default);
	assert_eq!(shell.key.get_untracked(), "");
	assert!(shell.account.get_untracked().is_none());
	assert!(shell.overview.get_untracked().is_null());
	assert_eq!(shell.notice.get_untracked(), "");
	assert_eq!(shell.output.get_untracked(), "");
	assert!(shell.measurement.get_untracked().is_null());
	assert_eq!(shell.measured_project.get_untracked(), "");
	assert_eq!(shell.project.get_untracked(), "");
	assert_eq!(shell.version.get_untracked(), "");
	assert!(!shell.busy.get_untracked());
	assert!(!shell.setup.get_untracked());
	assert!(!shell.maintenance.get_untracked());
}

#[test]
fn outside_a_host_the_client_says_where_it_runs() {
	use futures::FutureExt as _;
	// The fallbacks are plain ready futures; polling each once is enough.
	assert_eq!(
		client::request("key", "/api/v1/overview", None).now_or_never(),
		Some(Err("Run the app in a browser or native host".to_owned()))
	);
	assert_eq!(
		client::copy("value").now_or_never(),
		Some(Err("Run in the browser or native app".to_owned()))
	);
	assert_eq!(
		client::open_documentation().now_or_never(),
		Some(Err(
			"Open the documentation in the browser or native app".to_owned()
		))
	);
}
