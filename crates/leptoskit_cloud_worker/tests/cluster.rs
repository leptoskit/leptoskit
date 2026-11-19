//! A cluster's worker against a real control plane on a real port, with a
//! recording fake for the cluster: it applies a release, says when one
//! never became ready, and removes an environment.

#![cfg(unix)]
#![allow(
	clippy::cognitive_complexity,
	clippy::too_many_lines,
	reason = "a scenario is one story, read top to bottom; its asserts are what count"
)]

#[path = "../../leptoskit_cloud/tests/support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use leptoskit_cloud::workers::DestroyOutcome;
use leptoskit_cloud::workers::JobOutcome;
use leptoskit_cloud::workers::ReleaseOutcome;
use leptoskit_cloud::workers::WorkerRole;
use leptoskit_cloud_runtime::Applied;
use leptoskit_cloud_runtime::EnvironmentRef;
use leptoskit_cloud_runtime::LogLine;
use leptoskit_cloud_runtime::LogQuery;
use leptoskit_cloud_runtime::Observed;
use leptoskit_cloud_runtime::ReleaseSpec;
use leptoskit_cloud_runtime::Runtime;
use leptoskit_cloud_runtime::RuntimeError;
use leptoskit_cloud_runtime::RuntimeFuture;
use leptoskit_cloud_runtime::SecretValues;
use leptoskit_cloud_runtime::testing::Call;
use leptoskit_cloud_runtime::testing::Fake;
use leptoskit_cloud_worker::CommandBuilder;
use leptoskit_cloud_worker::Control;
use leptoskit_cloud_worker::Ended;
use leptoskit_cloud_worker::Identity;
use leptoskit_cloud_worker::Key;
use leptoskit_cloud_worker::Settings;
use leptoskit_cloud_worker::State;
use leptoskit_cloud_worker::Step;
use leptoskit_cloud_worker::Worker;
use leptoskit_cloud_worker::cluster::Bounds;
use leptoskit_cloud_worker::cluster::ClusterRole;
use leptoskit_cloud_worker::cluster::Pending;
use leptoskit_cloud_worker::cluster::Place;
use serde_json::json;
use support::shipping::Shop;

/// A runtime that hands every call to a shared fake, or answers every
/// apply that a process never became ready.
#[derive(Clone)]
struct Recorded {
	fake: Arc<Fake>,
	never_ready: bool,
}

impl Runtime for Recorded {
	fn apply(&self, spec: &ReleaseSpec) -> RuntimeFuture<'_, Result<Applied, RuntimeError>> {
		if self.never_ready {
			let process = spec.processes[0].name.clone();
			return Box::pin(async move {
				Err(RuntimeError::NotReady {
					process,
					reason: "the image never started".to_owned(),
				})
			});
		}

		self.fake.apply(spec)
	}

	fn status(
		&self,
		environment: &EnvironmentRef,
	) -> RuntimeFuture<'_, Result<Observed, RuntimeError>> {
		self.fake.status(environment)
	}

	fn logs(
		&self,
		environment: &EnvironmentRef,
		query: &LogQuery,
	) -> RuntimeFuture<'_, Result<Vec<LogLine>, RuntimeError>> {
		self.fake.logs(environment, query)
	}

	fn destroy(&self, environment: &EnvironmentRef) -> RuntimeFuture<'_, Result<(), RuntimeError>> {
		self.fake.destroy(environment)
	}
}

impl Place for Recorded {
	fn runtime(&self, _: SecretValues) -> Pending<'_, Result<Box<dyn Runtime>, RuntimeError>> {
		let runtime = self.clone();
		Box::pin(async move { Ok(Box::new(runtime) as Box<dyn Runtime>) })
	}
}

/// The shop's cluster worker, as the real worker, applying to `place`.
fn cluster_worker(
	shop: &Shop<'_>,
	place: Recorded,
	bounds: Bounds,
	directory: &tempfile::TempDir,
) -> Worker<CommandBuilder> {
	let identity = Identity {
		origin: shop.app.origin.clone(),
		rpc: "/rpc".to_owned(),
		key: shop.cluster.key.clone(),
		worker: shop.cluster.id.parse().expect("an ID"),
		name: "eu-one-worker".to_owned(),
		role: WorkerRole::Cluster,
	};
	let state = State::open(&directory.path().join("state")).expect("the state opens");
	state.pair(&identity).expect("keeps its key");
	let control = Control::new(
		&identity.origin,
		&identity.rpc,
		Some(Key::new(identity.key)),
	)
	.expect("a client");
	let settings = Settings {
		wait_seconds: 0,
		workspace: directory.path().join("workspace"),
		renew_every: Some(Duration::from_millis(200)),
	};
	Worker::new(
		control,
		state,
		CommandBuilder::new("false", Vec::new()),
		settings,
	)
	.expect("a worker")
	.with_cluster(ClusterRole::new(Arc::new(place)).bounds(bounds))
}

fn reported(step: &Step) -> &JobOutcome {
	match step {
		Step::Ran {
			ended: Ended::Reported(outcome),
			..
		} => outcome,
		other => panic!("expected a reported job, got {other:?}"),
	}
}

/// A deployment's release is applied to the cluster, and the deployment
/// goes live; its log says what the worker did.
#[tokio::test(flavor = "multi_thread")]
async fn a_release_is_applied_and_goes_live() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let fake = Arc::new(Fake::new());
	let directory = tempfile::tempdir().expect("a directory");
	let worker = cluster_worker(
		&shop,
		Recorded {
			fake: Arc::clone(&fake),
			never_ready: false,
		},
		Bounds::default(),
		&directory,
	);

	// Nothing to release yet.
	assert_eq!(worker.step().await.expect("a step"), Step::Idle);

	let digest = shop.upload(b"notes").await;
	let number = shop
		.deploy("production", &digest, json!({}))
		.await
		.expect(200)
		.json()["number"]
		.as_u64()
		.expect("a number");
	let build = shop.build.take().await;
	shop.build
		.finish(&build, support::shipping::built(&build, b"image", &[]))
		.await
		.expect(200);

	let step = worker.step().await.expect("a step");
	assert_eq!(
		reported(&step),
		&JobOutcome::Release(ReleaseOutcome::Applied)
	);
	assert_eq!(
		shop.state("production", number).await,
		json!({ "state": "live" })
	);
	let applied = fake.applied();
	assert_eq!(applied.len(), 1);
	assert_eq!(applied[0].deployment, number);
	assert_eq!(applied[0].environment.environment.slug, "production");
	assert!(!directory.path().join("state/job.json").exists());
}

/// A release that never becomes ready fails its deployment, and the
/// process it names is the one that didn't start.
#[tokio::test(flavor = "multi_thread")]
async fn a_release_that_never_becomes_ready_fails() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let directory = tempfile::tempdir().expect("a directory");
	let worker = cluster_worker(
		&shop,
		Recorded {
			fake: Arc::new(Fake::new()),
			never_ready: true,
		},
		Bounds::default(),
		&directory,
	);
	let digest = shop.upload(b"notes").await;
	let number = shop
		.deploy("production", &digest, json!({}))
		.await
		.expect(200)
		.json()["number"]
		.as_u64()
		.expect("a number");
	let build = shop.build.take().await;
	shop.build
		.finish(&build, support::shipping::built(&build, b"image", &[]))
		.await
		.expect(200);

	let step = worker.step().await.expect("a step");
	assert_eq!(
		reported(&step),
		&JobOutcome::Release(ReleaseOutcome::NotReady {
			process: "app".to_owned(),
			reason: "the image never started".to_owned(),
		})
	);
	assert_eq!(
		shop.state("production", number).await,
		json!({ "state": "failed", "stage": "rolling_out", "blocker": "not_ready" })
	);
}

/// A release outside the bounds the operator set is refused before the
/// cluster is touched, whatever the control plane sent.
#[tokio::test(flavor = "multi_thread")]
async fn a_release_outside_the_operators_bounds_is_refused() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let fake = Arc::new(Fake::new());
	let directory = tempfile::tempdir().expect("a directory");
	let worker = cluster_worker(
		&shop,
		Recorded {
			fake: Arc::clone(&fake),
			never_ready: false,
		},
		Bounds {
			runtime_class: Some("kata".to_owned()),
			database: None,
		},
		&directory,
	);
	let digest = shop.upload(b"notes").await;
	let number = shop
		.deploy("production", &digest, json!({}))
		.await
		.expect(200)
		.json()["number"]
		.as_u64()
		.expect("a number");
	let build = shop.build.take().await;
	shop.build
		.finish(&build, support::shipping::built(&build, b"image", &[]))
		.await
		.expect(200);

	let step = worker.step().await.expect("a step");
	assert_eq!(
		reported(&step),
		&JobOutcome::Release(ReleaseOutcome::NotReady {
			process: "app".to_owned(),
			reason: "the release is refused: its processes don't run under the `kata` runtime \
			         class this cluster requires"
				.to_owned(),
		})
	);
	assert_eq!(
		shop.state("production", number).await,
		json!({ "state": "failed", "stage": "rolling_out", "blocker": "not_ready" })
	);
	assert_eq!(fake.calls(), Vec::<Call>::new());
}

/// Deleting an environment that ran removes it from the cluster.
#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_environment_is_removed_from_the_cluster() {
	let app = support::app().await;
	let shop = Shop::open(&app).await;
	let fake = Arc::new(Fake::new());
	let directory = tempfile::tempdir().expect("a directory");
	let worker = cluster_worker(
		&shop,
		Recorded {
			fake: Arc::clone(&fake),
			never_ready: false,
		},
		Bounds::default(),
		&directory,
	);
	app.call(
		"create_environment",
		json!({ "organization": shop.org, "project": "notes", "name": "staging", "kind": "staging" }),
		shop.as_ana(),
	)
	.await
	.expect(200);
	let digest = shop.upload(b"short-lived").await;
	shop.deploy("staging", &digest, json!({})).await.expect(200);
	let build = shop.build.take().await;
	shop.build
		.finish(&build, support::shipping::built(&build, b"image", &[]))
		.await
		.expect(200);
	worker.step().await.expect("the release");

	app.call(
		"delete_environment",
		json!({
			"organization": shop.org,
			"project": "notes",
			"environment": "staging",
			"confirm": "staging",
		}),
		shop.as_ana(),
	)
	.await
	.expect(200);
	let step = worker.step().await.expect("a step");
	assert_eq!(
		reported(&step),
		&JobOutcome::DestroyEnvironment(DestroyOutcome::Destroyed)
	);
	assert!(
		fake.calls().iter().any(
			|call| matches!(call, Call::Destroy { environment } if environment.environment.slug == "staging")
		),
		"{:?}",
		fake.calls()
	);
	assert_eq!(fake.applied(), Vec::<ReleaseSpec>::new());
	let clusters = app.call("clusters", json!({}), shop.as_ops()).await;
	assert_eq!(clusters.expect(200).json()[0]["environments"], 0);
}
