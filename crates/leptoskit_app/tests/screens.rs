use std::cell::RefCell;

use futures::executor::LocalPool;
use futures::executor::LocalSpawner;
use futures::task::LocalSpawnExt as _;
use leptoskit_app::Shell;
use leptoskit_app::screen;
use leptoskit_deploy_console::hosted_protocol::AccountView;
use leptoskit_deploy_console::hosted_protocol::Installation;
use leptoskit_deploy_console::hosted_protocol::Ownership;
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;
use serde_json::json;

thread_local! {
	static POOL: RefCell<LocalPool> = RefCell::new(LocalPool::new());
	static SPAWNER: LocalSpawner = POOL.with_borrow(LocalPool::spawner);
}

/// Each test thread's own task pool, run by [`settle`], as the console runs
/// its own. Requests reach no server, so each fails at once.
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

/// Runs every request the console started that can make progress, and
/// redraws the screen the way the request's answer does.
fn settle(tester: &mut WidgetTester) {
	POOL.with_borrow_mut(LocalPool::run_until_stalled);
	tester.pump();
}

/// A shell in its own reactive owner, with the test thread's executor
/// installed so the console's requests can run.
fn console(width: f32, height: f32, fill: impl Fn(&Shell)) -> (Owner, Shell, WidgetTester) {
	SPAWNER.with(|_| ());
	// Both halves of the executor trait: nothing waits on either here, but
	// the platform calls them.
	let pool = Pool;
	any_spawner::CustomExecutor::poll_local(&pool);
	any_spawner::CustomExecutor::spawn(&pool, Box::pin(async {}));
	let _ = any_spawner::Executor::init_local_custom_executor(pool);
	let owner = Owner::new();
	let shell = owner.with(Shell::new);
	fill(&shell);
	let drawing = owner.clone();
	let tester = WidgetTester::new(
		move || drawing.with(|| screen::screen(shell)),
		width,
		height,
	)
	.unwrap();
	(owner, shell, tester)
}

/// An account with one of each kind of installation: a customer-owned
/// one that has never connected, and `LeptosKit`-managed `DigitalOcean`
/// capacity that has.
fn two_installations() -> AccountView {
	AccountView {
		id: "example".into(),
		status: "active".into(),
		plan: "team".into(),
		installations: vec![
			Installation {
				id: "customer".into(),
				project: "notes".into(),
				environment: "production".into(),
				ownership: Ownership::Customer,
				last_seen: 0,
			},
			Installation {
				id: "managed".into(),
				project: "infra".into(),
				environment: "staging".into(),
				ownership: Ownership::LeptoskitDigitalocean,
				last_seen: 1_700_000_000,
			},
		],
	}
}

#[test]
fn shared_screens_render_on_phone_and_web_without_exposing_account_keys() {
	for width in [390.0, 1024.0] {
		let owner = Owner::new();
		let shell = owner.with(Shell::new);
		let tester =
			WidgetTester::new(move || owner.with(|| screen::screen(shell)), width, 1800.0).unwrap();
		tester.get_by_text("Connect your account").unwrap();
		tester.get_by_text("Documentation ↗").unwrap();
		let owner = Owner::new();

		let shell = owner.with(Shell::new);
		shell.key.set("never-render-the-account-secret".into());
		shell.account.set(Some(AccountView {
			id: "example".into(),
			status: "active".into(),
			plan: "team".into(),
			installations: vec![Installation {
				id: "example".into(),
				project: "notes".into(),
				environment: "production".into(),
				ownership: Ownership::Customer,
				last_seen: 0,
			}],
		}));
		let tester =
			WidgetTester::new(move || owner.with(|| screen::screen(shell)), width, 2400.0).unwrap();
		tester.get_by_text("Pair worker").unwrap();
		tester.get_by_text("Plan infrastructure").unwrap();
		assert!(
			tester
				.query_by_text("never-render-the-account-secret")
				.unwrap()
				.is_none()
		);
	}
}

#[test]
fn the_public_example_uses_working_shared_components() {
	use leptoskit_testing::Finder;
	use reactive_graph::signal::RwSignal;

	for width in [390.0, 1024.0] {
		let owner = Owner::new();
		let count = owner.with(|| RwSignal::new(0));
		let mut tester = WidgetTester::new(
			move || owner.with(|| leptoskit_app::landing::demo(count)),
			width,
			600.0,
		)
		.unwrap();
		tester.get_by_text("0 ideas shipped").unwrap();
		tester.tap(&Finder::test_id("demo-increment")).unwrap();
		tester.get_by_text("1 ideas shipped").unwrap();
		tester.tap(&Finder::test_id("demo-increment")).unwrap();

		tester.get_by_text("2 ideas shipped").unwrap();
	}
}

#[test]
fn the_console_shows_every_installation_its_plan_and_its_measurements() {
	let (_owner, _shell, tester) = console(1024.0, 4000.0, |shell| {
		shell.account.set(Some(two_installations()));
		shell.notice.set("Saved.".into());
		shell.output.set("one-use-code".into());
		shell.overview.set(json!({
			"projects": [
				{"id": "notes", "revision": 3},
				{"id": "infra", "revision": 1},
			],
			"operations": [
				{
					"id": "op-42",
					"project": "notes",
					"phase": "succeeded",
					"message": "planned",
					"intent": {"action": {"kind": "plan"}},
					"result": {"changes": [{"add": "droplet"}]},
				},
				{
					"id": "op-43",
					"project": "notes",
					"phase": "running",
					"message": "applying",
					"intent": {"action": {"kind": "apply"}},
				},
				{
					"id": "op-44",
					"project": "infra",
					"phase": "failed",
					"message": "provider refused",
					"intent": {"action": {"kind": "deploy"}},
				},
			],
		}));
		shell.measurement.set(json!({
			"instances": 3,
			"stale_instances": 1,
			"series": [
				{
					"key": {"source": "backend", "operation": "request", "outcome": "ok"},
					"release": "v1.2.3",
					"windows": [{"count": 3}, {"count": 4}, {"count": "not a count"}],
				},
				{
					"key": {"source": "worker", "operation": "deploy", "outcome": "error"},
					"release": "v1.2.2",
					"windows": [],
				},
			],
		}));
		shell.measured_project.set("notes".into());
	});
	let tree = tester.tree();
	for expected in [
		"example · team · active",
		"production · Your cloud account",
		"staging · LeptosKit-managed DigitalOcean",
		"Worker has not connected yet",
		"Worker has connected; refresh for deployment status",
		"Resource actions:",
		"Apply reviewed plan",
		"running · applying",
		"failed · provider refused",
		"Reporting instances: 3 · stale: 1",
		"backend · request · ok · 7 samples · v1.2.3",
		"worker · deploy · error · 0 samples · v1.2.2",
		"Browser observations are separate from backend measurements.",
		"Copy key, code or billing link",
		"Saved.",
	] {
		assert!(tree.contains(expected), "missing {expected}: {tree}");
	}
	// The account key itself never reaches the screen.
	assert!(!tree.contains("account-key"));
	assert!(!tree.contains("one-use-code"));
}

#[test]
fn the_console_buttons_send_their_requests_and_report_refusals() {
	let (_owner, shell, mut tester) = console(1024.0, 4000.0, |shell| {
		shell.account.set(Some(AccountView {
			id: "example".into(),
			status: "active".into(),
			plan: "control".into(),
			installations: vec![Installation {
				id: "customer".into(),
				project: "notes".into(),
				environment: "production".into(),
				ownership: Ownership::Customer,
				last_seen: 0,
			}],
		}));
		shell.output.set("one-use-code".into());
		shell.measured_project.set("notes".into());
		shell.overview.set(json!({
			"projects": [{"id": "notes", "revision": 3}],
			"operations": [{
				"id": "op-42",
				"project": "notes",
				"phase": "succeeded",
				"message": "planned",
				"intent": {"action": {"kind": "plan"}},
				"result": {"changes": [{"add": "droplet"}]},
			}],
		}));
	});

	// Every button builds its request and sends it; with no server, each
	// says so in the notice line. Copying the key comes first: a request
	// clears the output the button is offered for.
	tester
		.tap(&Finder::text("Copy key, code or billing link"))
		.unwrap();
	settle(&mut tester);
	assert!(tester.tree().contains("Run in the browser or native app"));

	tester.tap(&Finder::test_id("refresh")).unwrap();
	settle(&mut tester);
	assert!(tester.tree().contains("Run the app in a browser"));
	for button in [
		"Pair worker",
		"Revoke worker",
		"Plan infrastructure",
		"Deploy",
		"Roll back image",
		"Application measurements",
		"Create agent key",
		"Control plan",
		"Team plan",
		"Manage billing",
		"Apply reviewed plan",
	] {
		tester.tap(&Finder::text(button)).unwrap();
		settle(&mut tester);
	}

	// The release fields and flags the deploy request reads are settable.
	tester
		.enter_text(&Finder::label("Release version"), "sha-0123456789abcdef")
		.unwrap();
	tester
		.enter_text(&Finder::test_id("project"), "notes")
		.unwrap();
	tester
		.toggle(&Finder::label("First release"), true)
		.unwrap();
	tester.toggle(&Finder::label("Maintenance"), true).unwrap();
	settle(&mut tester);
	tester.tap(&Finder::test_id("add-installation")).unwrap();
	settle(&mut tester);
	assert!(shell.setup.get_untracked());

	assert!(shell.maintenance.get_untracked());
	assert_eq!(shell.version.get_untracked(), "sha-0123456789abcdef");
	assert_eq!(shell.project.get_untracked(), "notes");

	// Documentation opens from the screen's own link.
	tester.tap(&Finder::test_id("documentation")).unwrap();
	settle(&mut tester);
	assert!(tester.tree().contains("documentation in the browser"));

	// Signing out clears the account and returns to the connect card.
	tester.tap(&Finder::test_id("logout")).unwrap();
	settle(&mut tester);
	assert!(shell.account.get_untracked().is_none());
	let tree = tester.tree();
	assert!(tree.contains("Connect your account"), "{tree}");
	assert!(!tree.contains("Subscription"), "{tree}");

	// The connect card takes an account key and sends it.
	tester
		.enter_text(&Finder::test_id("account-key"), "account-key")
		.unwrap();
	assert_eq!(shell.key.get_untracked(), "account-key");
	tester.tap(&Finder::test_id("connect")).unwrap();
	settle(&mut tester);
	assert!(tester.tree().contains("Run the app in a browser"));
}
