//! The worker against a real control plane on a real port: it pairs,
//! claims a deployment's build, fetches and unpacks the source, runs a
//! build command, sends the log, and reports the outcome. The control
//! plane and the people in it come from `leptoskit_cloud`'s own tests.

#![cfg(unix)]
#![allow(
	clippy::cognitive_complexity,
	clippy::too_many_lines,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]

#[path = "../../leptoskit_cloud/tests/support/mod.rs"]
mod support;

use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use leptoskit_cloud::archive;
use leptoskit_cloud::workers::BuildOutcome;
use leptoskit_cloud::workers::JobKind;
use leptoskit_cloud::workers::JobOutcome;
use leptoskit_cloud_worker::CommandBuilder;
use leptoskit_cloud_worker::Control;
use leptoskit_cloud_worker::Ended;
use leptoskit_cloud_worker::Identity;
use leptoskit_cloud_worker::Key;
use leptoskit_cloud_worker::Marker;
use leptoskit_cloud_worker::RunError;
use leptoskit_cloud_worker::Saved;
use leptoskit_cloud_worker::Settings;
use leptoskit_cloud_worker::State;
use leptoskit_cloud_worker::Step;
use leptoskit_cloud_worker::Worker;
use serde_json::json;
use support::App;
use support::As;
use support::shipping;
use support::shipping::Shop;

/// A build command that succeeds: it shows it has the source, and answers
/// with an image in the repository it was told to push to.
const BUILDS: &str = r#"
echo "Compiling $(cat name.txt)"
repository=$(sed -n 's/.*"image_repository": "\(.*\)",*/\1/p' "$LEPTOSKIT_BUILD_INPUT")
cat > "$LEPTOSKIT_BUILD_OUTPUT" <<JSON
{
	"result": "succeeded",
	"image": {
		"repository": "$repository",
		"digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111"
	},
	"processes": [{
		"name": "app",
		"command": ["/app/server"],
		"port": 8080,
		"ready_path": "/readyz",
		"live_path": "/healthz"
	}],
	"migrations": [],
	"steps": [{ "name": "cargo build", "millis": 1200 }]
}
JSON
"#;

/// A project's source, packed as the CLI packs one.
fn source(name: &str) -> archive::Packed {
	let directory = tempfile::tempdir().expect("a directory");
	std::fs::write(directory.path().join("name.txt"), name).expect("a file");
	std::fs::create_dir(directory.path().join("src")).expect("a directory");
	std::fs::write(directory.path().join("src/main.rs"), "fn main() {}\n").expect("a file");
	archive::pack(
		directory.path(),
		&archive::PackOptions {
			use_git: false,
			..archive::PackOptions::default()
		},
	)
	.expect("packs")
}

/// A real worker's directories: its state and where it builds.
struct Bench {
	directory: tempfile::TempDir,
	identity: Identity,
}

impl Bench {
	/// Pairs a build worker called `name` with `app`, as an operator's
	/// code allows, and keeps its key in a fresh state directory.
	async fn pair(app: &App, ops: &str, name: &str) -> Self {
		let issued = app
			.call(
				"issue_pairing",
				json!({ "role": "build" }),
				As::Session(ops),
			)
			.await
			.expect(200)
			.json();
		let control = Control::new(&app.origin, "/rpc", None).expect("a client");
		let paired = control
			.pair(
				issued["code"].as_str().expect("a code"),
				name,
				vec!["linux".to_owned(), "x86_64".to_owned()],
			)
			.await
			.expect("pairs");
		let directory = tempfile::tempdir().expect("a directory");
		let identity = Identity {
			origin: app.origin.clone(),
			rpc: "/rpc".to_owned(),
			key: paired.key.clone(),
			worker: paired.worker.id,
			name: paired.worker.name.clone(),
			role: paired.worker.role,
		};
		let bench = Self {
			directory,
			identity,
		};
		bench.state().pair(&bench.identity).expect("keeps its key");
		bench
	}

	fn state_directory(&self) -> PathBuf {
		self.directory.path().join("state")
	}

	fn workspace(&self) -> PathBuf {
		self.directory.path().join("workspace")
	}

	fn state(&self) -> State {
		State::open(&self.state_directory()).expect("the state opens")
	}

	fn control(&self) -> Control {
		Control::new(
			&self.identity.origin,
			&self.identity.rpc,
			Some(Key::new(self.identity.key.clone())),
		)
		.expect("a client")
	}

	/// The worker, building with the shell script `script`, asking once
	/// without waiting and renewing its lease five times a second.
	fn worker(&self, script: &str) -> Worker<CommandBuilder> {
		let builder = CommandBuilder::new("/bin/sh", vec!["-c".to_owned(), script.to_owned()]);
		let settings = Settings {
			wait_seconds: 0,
			workspace: self.workspace(),
			renew_every: Some(Duration::from_millis(200)),
		};
		Worker::new(self.control(), self.state(), builder, settings).expect("a worker")
	}

	/// What is left where builds are made.
	fn left_in_workspace(&self) -> Vec<String> {
		entries(&self.workspace())
	}

	/// What the state directory holds beside the key and the lock.
	fn job_files(&self) -> Vec<String> {
		entries(&self.state_directory())
			.into_iter()
			.filter(|name| name != "worker.json" && name != "lock")
			.collect()
	}
}

fn entries(directory: &Path) -> Vec<String> {
	let Ok(entries) = std::fs::read_dir(directory) else {
		return Vec::new();
	};
	let mut names: Vec<String> = entries
		.map(|entry| {
			entry
				.expect("an entry")
				.file_name()
				.to_string_lossy()
				.into_owned()
		})
		.collect();
	names.sort();
	names
}

/// Deploys `packed` to production as Ana, and returns the deployment's
/// number.
async fn deploy(shop: &Shop<'_>, packed: &archive::Packed) -> u64 {
	let digest = shop.upload(&packed.bytes).await;
	assert_eq!(digest, packed.digest.as_str());
	shop.deploy("production", &digest, json!({}))
		.await
		.expect(200)
		.json()["number"]
		.as_u64()
		.expect("a number")
}

/// The build log of deployment `number`, as its lines' text.
async fn log_of(shop: &Shop<'_>, number: u64) -> Vec<String> {
	let logs = shop.on("build_logs", "production", number).await;
	logs.expect(200).json()["lines"]
		.as_array()
		.expect("lines")
		.iter()
		.map(|line| line["text"].as_str().expect("text").to_owned())
		.collect()
}

fn reported(step: &Step) -> &BuildOutcome {
	match step {
		Step::Ran {
			ended: Ended::Reported(JobOutcome::BuildServer(outcome)),
			..
		} => outcome,
		other => panic!("expected a reported build, got {other:?}"),
	}
}

/// A deployment's build is claimed, built from the source that was
/// uploaded, logged and reported; the deployment moves on to its release,
/// and nothing of the build is left on the worker.
#[tokio::test(flavor = "multi_thread")]
async fn a_worker_builds_what_was_deployed() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let bench = Bench::pair(&app, &shop.ops, "builder-real").await;
	let worker = bench.worker(BUILDS);

	// With nothing deployed there is nothing to do.
	assert_eq!(worker.step().await.expect("a step"), Step::Idle);

	let number = deploy(&shop, &source("notes v0.1.0")).await;
	let step = worker.step().await.expect("a step");
	let BuildOutcome::Succeeded(output) = reported(&step) else {
		panic!("{step:?}");
	};
	assert_eq!(output.processes[0].name, "app");
	assert!(
		output.image.repository.starts_with(support::REGISTRY),
		"{}",
		output.image.repository
	);

	// The control plane has the build: the release waits for the cluster.
	assert_eq!(
		shop.state("production", number).await,
		json!({ "state": "rolling_out" })
	);
	let log = log_of(&shop, number).await;
	assert_eq!(
		log,
		[
			"worker: fetching the source",
			"worker: unpacked 2 files, 25 bytes",
			"Compiling notes v0.1.0",
		]
	);
	let release = shop.cluster.take().await;
	shop.cluster
		.finish(&release, shipping::applied())
		.await
		.expect(200);
	assert_eq!(
		shop.state("production", number).await,
		json!({ "state": "live" })
	);

	// The source is gone, and so is every note of the job.
	assert_eq!(bench.left_in_workspace(), Vec::<String>::new());
	assert_eq!(bench.job_files(), Vec::<String>::new());
	assert_eq!(worker.step().await.expect("a step"), Step::Idle);
}

/// A build that fails says why, and the deployment fails with its reason.
#[tokio::test(flavor = "multi_thread")]
async fn a_build_that_fails_fails_the_deployment() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let bench = Bench::pair(&app, &shop.ops, "builder-real").await;
	let worker = bench.worker(
		r#"echo "error: expected ;" >&2
printf '%s' '{"result":"failed","reason":"src/main.rs:1: expected `;`"}' > "$LEPTOSKIT_BUILD_OUTPUT"
exit 101"#,
	);
	let number = deploy(&shop, &source("notes v0.1.0")).await;
	let step = worker.step().await.expect("a step");
	assert_eq!(
		reported(&step),
		&BuildOutcome::Failed {
			reason: "src/main.rs:1: expected `;`".to_owned(),
			steps: Vec::new(),
		}
	);
	assert_eq!(
		shop.state("production", number).await,
		json!({ "state": "failed", "stage": "building", "blocker": "build_failed" })
	);
	assert!(
		log_of(&shop, number)
			.await
			.contains(&"error: expected ;".to_owned())
	);
	assert_eq!(bench.left_in_workspace(), Vec::<String>::new());
	assert_eq!(bench.job_files(), Vec::<String>::new());
}

/// Cancelling a deployment reaches its build at the next renewal: the
/// build command is stopped, and the worker says it stopped.
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_deployment_stops_its_build() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let bench = Bench::pair(&app, &shop.ops, "builder-real").await;
	let worker = bench.worker("echo started; sleep 300");
	let number = deploy(&shop, &source("notes v0.1.0")).await;
	let cancel = async {
		// Once the build has said it started, it is running.
		for _ in 0..200 {
			if log_of(&shop, number).await.contains(&"started".to_owned()) {
				break;
			}

			tokio::time::sleep(Duration::from_millis(50)).await;
		}

		shop.on("cancel_deployment", "production", number)
			.await
			.expect(200);
	};
	let started = std::time::Instant::now();
	let (step, ()) = tokio::join!(worker.step(), cancel);
	assert_eq!(reported(&step.expect("a step")), &BuildOutcome::Stopped);
	assert!(started.elapsed() < Duration::from_secs(60));
	assert_eq!(
		shop.state("production", number).await,
		json!({ "state": "cancelled" })
	);
	assert_eq!(bench.left_in_workspace(), Vec::<String>::new());
	assert_eq!(bench.job_files(), Vec::<String>::new());
}

/// The successful outcome of the build `assignment`, as a worker saves it.
fn built(assignment: &leptoskit_cloud::workers::Assignment) -> JobOutcome {
	let assignment = serde_json::to_value(assignment).expect("an assignment");
	serde_json::from_value(shipping::built(&assignment, b"image", &[])).expect("an outcome")
}

/// A worker that died after saving an outcome sends it when it returns;
/// one that died mid-build drops its marker, waits out the lease, and is
/// given the build again.
#[tokio::test(flavor = "multi_thread")]
async fn a_worker_that_died_picks_up_where_it_can() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let bench = Bench::pair(&app, &shop.ops, "builder-real").await;
	let first = deploy(&shop, &source("notes v0.1.0")).await;

	// It claimed the build, built it, wrote the outcome down, and died
	// before the control plane heard.
	let assignment = bench
		.control()
		.claim(0)
		.await
		.expect("claims")
		.expect("a job");
	{
		let state = bench.state();
		state
			.start(&Marker {
				job: assignment.job,
				attempt: assignment.attempt,
				kind: JobKind::BuildServer,
			})
			.expect("marks");
		state
			.save(&Saved {
				job: assignment.job,
				attempt: assignment.attempt,
				outcome: built(&assignment),
			})
			.expect("saves");
	}
	assert_eq!(
		shop.state("production", first).await,
		json!({ "state": "building" })
	);
	let worker = bench.worker(BUILDS);
	// Back again, it reports what it saved before asking for anything.
	assert_eq!(worker.step().await.expect("a step"), Step::Idle);
	assert_eq!(
		shop.state("production", first).await,
		json!({ "state": "rolling_out" })
	);
	assert_eq!(bench.job_files(), Vec::<String>::new());
	drop(worker);
	let release = shop.cluster.take().await;
	shop.cluster
		.finish(&release, shipping::applied())
		.await
		.expect(200);

	// The next build it claimed and died in the middle of.
	let second = deploy(&shop, &source("notes v0.2.0")).await;
	let assignment = bench
		.control()
		.claim(0)
		.await
		.expect("claims")
		.expect("a job");
	assert_eq!(assignment.attempt, 1);
	bench
		.state()
		.start(&Marker {
			job: assignment.job,
			attempt: assignment.attempt,
			kind: JobKind::BuildServer,
		})
		.expect("marks");
	let worker = bench.worker(BUILDS);
	// The marker is dropped, and the control plane still counts the build
	// as this worker's until the lease runs out.
	assert_eq!(worker.step().await.expect("a step"), Step::Held);
	assert_eq!(bench.job_files(), Vec::<String>::new());
	shop.lapse_leases().await;
	shop.sweep().await;
	let step = worker.step().await.expect("a step");
	assert!(
		matches!(reported(&step), BuildOutcome::Succeeded(_)),
		"{step:?}"
	);
	assert_eq!(
		shop.state("production", second).await,
		json!({ "state": "rolling_out" })
	);
	assert!(
		log_of(&shop, second)
			.await
			.contains(&"Compiling notes v0.2.0".to_owned())
	);
}

/// A job that isn't a build, started and never reported, is never run
/// again: the worker refuses to go on until it is forgotten.
#[tokio::test(flavor = "multi_thread")]
async fn a_started_job_of_unknown_effect_stops_the_worker() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let bench = Bench::pair(&app, &shop.ops, "builder-real").await;
	let job = uuid::Uuid::from_u128(9);
	bench
		.state()
		.start(&Marker {
			job,
			attempt: 1,
			kind: JobKind::Release,
		})
		.expect("marks");
	let worker = bench.worker(BUILDS);
	let refused = worker.step().await.expect_err("refuses");
	assert!(
		matches!(refused, RunError::Unreconciled { job: found, kind: JobKind::Release } if found == job),
		"{refused}"
	);
	assert_eq!(bench.job_files(), ["job.json"]);
}

/// A revoked worker is told so, and stops.
#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_worker_stops() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let bench = Bench::pair(&app, &shop.ops, "builder-real").await;
	let worker = bench.worker(BUILDS);
	assert_eq!(worker.step().await.expect("a step"), Step::Idle);
	app.call(
		"revoke_worker",
		json!({ "worker": bench.identity.worker }),
		shop.as_ops(),
	)
	.await
	.expect(200);
	let (_never, shutdown) = tokio::sync::watch::channel(false);
	let stopped = tokio::time::timeout(Duration::from_secs(30), worker.run(shutdown))
		.await
		.expect("the loop ends");
	assert!(matches!(stopped, Err(RunError::Revoked)), "{stopped:?}");
}

/// The loop stops when it is asked to, and the key it kept never reaches
/// the database as text or the worker's own printed forms.
#[tokio::test(flavor = "multi_thread")]
async fn the_loop_stops_when_asked_and_the_key_stays_put() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let bench = Bench::pair(&app, &shop.ops, "builder-real").await;
	let worker = bench.worker(BUILDS);
	let number = deploy(&shop, &source("notes v0.1.0")).await;
	let (halt, shutdown) = tokio::sync::watch::channel(false);
	let asked = async {
		for _ in 0..400 {
			if shop.state("production", number).await == json!({ "state": "rolling_out" }) {
				break;
			}

			tokio::time::sleep(Duration::from_millis(50)).await;
		}

		halt.send(true).expect("the loop listens");
	};
	let (ran, ()) = tokio::join!(
		tokio::time::timeout(Duration::from_secs(60), worker.run(shutdown)),
		asked
	);
	ran.expect("the loop ends").expect("without an error");
	assert_eq!(
		shop.state("production", number).await,
		json!({ "state": "rolling_out" })
	);

	let secret = bench
		.identity
		.key
		.rsplit('_')
		.next()
		.expect("a secret")
		.to_owned();
	assert_eq!(app.find_in_every_table(&secret).await, Vec::<String>::new());
	assert!(!format!("{:?}", bench.identity).contains(&secret));
	assert!(!format!("{worker:?}").contains(&secret));
}
