//! Embedding-surface tests: the typed facade, its SSE stream and a nested
//! router must behave exactly like the HTTP API embedders already know.
//!
//! Each test uses a fresh private temporary directory; nothing reads the
//! host's configuration or credentials.

#![cfg(feature = "server")]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::body::to_bytes;
use axum::http::HeaderMap;
use axum::http::Request;
use axum::http::StatusCode;
use leptoskit::apm::Analytics;
use leptoskit::apm::Limits;
use leptoskit::apm::Outcome;
use leptoskit::apm::Resource;
use leptoskit::apm::SeriesKey;
use leptoskit::apm::Signal;
use leptoskit::apm::Snapshot;
use leptoskit::apm::Source;
use leptoskit_deploy_console::control::*;
use serde_json::Value;
use serde_json::json;
use tokio_stream::StreamExt as _;
use tower::ServiceExt as _;

const OWNER: &str = "test-only-management-owner-key-0123456789";
const ORIGIN: &str = "http://localhost:4700";

fn setup() -> (tempfile::TempDir, Store) {
	let root = tempfile::tempdir().unwrap();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
	}
	let store = Store::open(&root.path().join("console.sqlite3")).unwrap();

	for id in ["notes", "other"] {
		store
			.register(&Project {
				id: id.into(),
				environment: "production".into(),
				root: root.path().canonicalize().unwrap(),
				revision: 0,
			})
			.unwrap();
	}

	(root, store)
}

fn service(store: &Store) -> Service {
	Service::new(store.clone(), OWNER, ORIGIN).unwrap()
}

fn intent(revision: u64, key: &str, action: Action) -> Intent {
	Intent {
		expected_revision: revision,
		idempotency_key: key.into(),
		action,
	}
}

fn request(method: &str, path: &str, body: Option<Value>) -> Request<Body> {
	Request::builder()
		.method(method)
		.uri(path)
		.header("content-type", "application/json")
		.body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
		.unwrap()
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Value) {
	let response = app.clone().oneshot(req).await.unwrap();
	let status = response.status();
	let headers = response.headers().clone();
	let body = to_bytes(response.into_body(), 10 * 1024 * 1024)
		.await
		.unwrap();
	(
		status,
		headers,
		serde_json::from_slice(&body).unwrap_or(Value::Null),
	)
}

async fn call(
	app: &Router,
	method: &str,
	path: &str,
	bearer: Option<&str>,
	body: Option<Value>,
) -> (StatusCode, Value) {
	let mut req = request(method, path, body);

	if let Some(bearer) = bearer {
		req.headers_mut()
			.insert("authorization", format!("Bearer {bearer}").parse().unwrap());
	}

	let (status, _, value) = send(app, req).await;
	(status, value)
}

fn finish(store: &Store, id: &str) {
	store
		.finish(
			id,
			&RunResult {
				succeeded: true,
				message: "Finished".into(),
				details: json!({"code":"test"}),
			},
		)
		.unwrap();
}

fn snapshot(project: &str, instance: &str, count: usize) -> Snapshot {
	let analytics = Analytics::new(
		Resource {
			service: project.into(),
			environment: "production".into(),
			release: "v1".into(),
			instance: instance.into(),
		},
		Limits::default(),
		|| {
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_secs()
		},
	)
	.unwrap();

	for _ in 0..count {
		analytics.record(
			SeriesKey {
				source: Source::Backend,
				signal: Signal::Http,
				operation: "/notes/{id}".into(),
				outcome: Outcome::Ok,
			},
			10.0,
			None,
		);
	}

	analytics.snapshot()
}

/// A harmless executor: one bounded progress message, then success.
struct FakeExecutor {
	service: Service,
}

impl Executor for FakeExecutor {
	fn run(
		&self,
		_project: Project,
		operation: Operation,
	) -> Pin<Box<dyn Future<Output = RunResult> + Send>> {
		let service = self.service.clone();
		Box::pin(async move {
			service.progress(&operation.id, "halfway").unwrap();
			RunResult {
				succeeded: true,
				message: "fake worker finished".into(),
				details: json!({"code":"test"}),
			}
		})
	}
}

#[tokio::test]
async fn facade_scope_enforcement_matches_the_http_api() {
	let (_root, store) = setup();
	let service = service(&store);
	let app = service.router();

	let deploy_key = store
		.issue(Scope::Deploy, Some("notes"), 3600, false)
		.unwrap();
	let read_key = store
		.issue(Scope::Read, Some("notes"), 3600, false)
		.unwrap();
	let ingest_key = store
		.issue(Scope::Ingest, Some("notes"), 3600, false)
		.unwrap();
	let elsewhere_key = store
		.issue(Scope::Deploy, Some("other"), 3600, false)
		.unwrap();

	// Overview content matches between the facade and the API.
	let read = Principal::scoped(Scope::Read, "notes");
	let overview = service.overview(&read).unwrap();
	let (status, value) = call(&app, "GET", "/api/v1/overview", Some(&read_key), None).await;
	assert_eq!(status, StatusCode::OK);
	let facade_projects: Vec<&str> = overview.projects.iter().map(|p| p.id.as_str()).collect();
	let http_projects: Vec<&str> = value["projects"]
		.as_array()
		.unwrap()
		.iter()
		.map(|p| p["id"].as_str().unwrap())
		.collect();
	assert_eq!(facade_projects, ["notes"]);
	assert_eq!(facade_projects, http_projects.as_slice());
	assert_eq!(
		overview.operations.len(),
		value["operations"].as_array().unwrap().len()
	);
	assert_eq!(
		service
			.overview(&Principal::owner())
			.unwrap()
			.projects
			.len(),
		2,
		"the owner sees every project"
	);

	// Writes the facade refuses are refused with the same HTTP outcome.
	let deploy = Principal::scoped(Scope::Deploy, "notes");
	let denied = [
		(read.clone(), read_key.clone(), Action::Status),
		(
			Principal::scoped(Scope::Ingest, "notes"),
			ingest_key.clone(),
			Action::Status,
		),
		(
			Principal::scoped(Scope::Deploy, "other"),
			elsewhere_key.clone(),
			Action::Status,
		),
		(deploy.clone(), deploy_key.clone(), Action::Plan),
	];
	for (principal, token, action) in &denied {
		let error = service
			.enqueue(principal, "notes", &intent(0, "facade", action.clone()))
			.unwrap_err();
		assert!(
			matches!(error, EnqueueError::Forbidden),
			"{action:?} must be forbidden"
		);
		let (status, value) = call(
			&app,
			"POST",
			"/api/v1/projects/notes/operations",
			Some(token),
			Some(json!(intent(0, "http", action.clone()))),
		)
		.await;
		assert_eq!(status, StatusCode::FORBIDDEN, "{action:?}");
		assert_eq!(value["error"], "scope_or_origin_denied");
	}

	// Writes the facade accepts are accepted over HTTP for the same
	// credential: an identical request replays idempotently with the same
	// operation.
	let owner = Principal::owner();
	for (revision, (principal, token, key, action)) in [
		(
			owner.clone(),
			OWNER.to_owned(),
			"owner-status",
			Action::Status,
		),
		(
			deploy.clone(),
			deploy_key.clone(),
			"deploy-status",
			Action::Status,
		),
	]
	.into_iter()
	.enumerate()
	{
		let revision = revision as u64;
		let facade_intent = intent(revision, key, action.clone());
		let operation = service
			.enqueue(&principal, "notes", &facade_intent.clone())
			.unwrap();
		let claimed = store.claim_project(Some("notes")).unwrap().unwrap();
		finish(&store, &claimed.id);

		let (status, value) = call(
			&app,
			"POST",
			"/api/v1/projects/notes/operations",
			Some(&token),
			Some(json!(facade_intent)),
		)
		.await;
		assert_eq!(status, StatusCode::OK, "{key}");
		assert_eq!(
			value["id"], operation.id,
			"{key} replays the same operation"
		);
	}
}

#[tokio::test]
async fn facade_operation_reads_match_the_http_api() {
	let (_root, store) = setup();
	let service = service(&store);
	let app = service.router();
	let other_read_key = store
		.issue(Scope::Read, Some("other"), 3600, false)
		.unwrap();

	// Operation reads enforce the same project visibility.
	let owner = Principal::owner();
	let revision = store
		.projects()
		.unwrap()
		.iter()
		.find(|p| p.id == "notes")
		.unwrap()
		.revision;
	let operation = service
		.enqueue(
			&owner,
			"notes",
			&intent(revision, "read-parity", Action::Status),
		)
		.unwrap();
	let claimed = store.claim_project(Some("notes")).unwrap().unwrap();
	finish(&store, &claimed.id);
	assert_eq!(
		service.operation(&owner, &operation.id).unwrap().id,
		operation.id
	);
	assert!(
		service
			.operation(&Principal::scoped(Scope::Read, "notes"), &operation.id)
			.is_ok()
	);
	let elsewhere_read = Principal::scoped(Scope::Read, "other");
	assert!(matches!(
		service.operation(&elsewhere_read, &operation.id),
		Err(FacadeError::Forbidden)
	));
	assert!(matches!(
		service.operation(&owner, "missing"),
		Err(FacadeError::NotFound)
	));
	assert_eq!(
		call(
			&app,
			"GET",
			&format!("/api/v1/operations/{}", operation.id),
			Some(&other_read_key),
			None
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/operations/missing", Some(OWNER), None)
			.await
			.0,
		StatusCode::NOT_FOUND
	);
}

#[tokio::test]
async fn facade_analytics_and_tokens_match_the_http_api() {
	let (_root, store) = setup();
	let service = service(&store);
	let app = service.router();
	let snapshot = snapshot("notes", "one", 2);
	store.ingest("notes", &snapshot).unwrap();

	let read_key = store
		.issue(Scope::Read, Some("notes"), 3600, false)
		.unwrap();
	let ingest_key = store
		.issue(Scope::Ingest, Some("notes"), 3600, false)
		.unwrap();
	let deploy_key = store
		.issue(Scope::Deploy, Some("notes"), 3600, false)
		.unwrap();

	// The facade reports the same fleet as the API for the same range.
	let read = Principal::scoped(Scope::Read, "notes");
	let to = snapshot.observed_unix_seconds + 1;
	let from = to.saturating_sub(600);
	let fleet = service.fleet(&read, "notes", from, to).unwrap();
	assert_eq!(fleet.instances, 1);
	let (status, value) = call(
		&app,
		"GET",
		&format!("/api/v1/projects/notes/analytics?from={from}&to={to}"),
		Some(&read_key),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(value["instances"], 1);

	let ingest = Principal::scoped(Scope::Ingest, "notes");
	assert!(matches!(
		service.fleet(&ingest, "notes", from, to),
		Err(FacadeError::Forbidden)
	));
	assert_eq!(
		call(
			&app,
			"GET",
			&format!("/api/v1/projects/notes/analytics?from={from}&to={to}"),
			Some(&ingest_key),
			None
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert!(matches!(
		service.fleet(&read, "notes", 0, 1),
		Err(FacadeError::Invalid(_))
	));
	assert_eq!(
		call(
			&app,
			"GET",
			"/api/v1/projects/notes/analytics?from=0&to=1",
			Some(&read_key),
			None
		)
		.await
		.0,
		StatusCode::BAD_REQUEST
	);

	// Token lifecycle: the facade and the API issue and revoke identically.
	let owner = Principal::owner();
	let issued = service.issue_token(&owner, Scope::Read, "other").unwrap();
	assert_eq!(issued.expires_in, 30 * 86400);
	let (status, value) = call(
		&app,
		"POST",
		"/api/v1/tokens",
		Some(OWNER),
		Some(json!({"scope":"read","project":"other"})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(value["expires_in"], 30 * 86400);
	assert_eq!(value["key"].as_str().unwrap().len(), 64);

	let deploy = Principal::scoped(Scope::Deploy, "notes");
	assert!(matches!(
		service.issue_token(&deploy, Scope::Read, "other"),
		Err(FacadeError::Forbidden)
	));
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/tokens",
			Some(&deploy_key),
			Some(json!({"scope":"read","project":"other"}))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert!(matches!(
		service.issue_token(&owner, Scope::Owner, "other"),
		Err(FacadeError::Forbidden)
	));
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/tokens",
			Some(OWNER),
			Some(json!({"scope":"owner","project":"other"}))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);

	service.revoke_token(&owner, &issued.key).unwrap();
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/tokens/revoke",
			Some(OWNER),
			Some(json!({"key": issued.key}))
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/overview", Some(&issued.key), None)
			.await
			.0,
		StatusCode::UNAUTHORIZED
	);
}

#[tokio::test]
async fn facade_enqueue_is_idempotent_and_records_the_actor() {
	let (_root, store) = setup();
	let service = service(&store);
	let app = service.router();
	let admin = Principal::scoped(Scope::Deploy, "notes").with_actor("admin@example.com");

	let first = service
		.enqueue(&admin, "notes", &intent(0, "same-key", Action::Status))
		.unwrap();
	let second = service
		.enqueue(&admin, "notes", &intent(0, "same-key", Action::Status))
		.unwrap();
	assert_eq!(first.id, second.id, "replay returns the same operation");
	assert_eq!(second.actor.as_deref(), Some("admin@example.com"));

	// The HTTP API sees the same idempotent operation for an identical
	// request from any caller that reaches the queue.
	let (status, value) = call(
		&app,
		"POST",
		"/api/v1/projects/notes/operations",
		Some(OWNER),
		Some(json!(intent(0, "same-key", Action::Status))),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(value["id"], first.id);
	assert_eq!(value["actor"], "admin@example.com");

	// A changed payload for the same key is a conflict on both surfaces.
	let conflict = service
		.enqueue(
			&Principal::owner(),
			"notes",
			&intent(1, "same-key", Action::Plan),
		)
		.unwrap_err();
	assert!(matches!(
		conflict,
		EnqueueError::Rejected(Rejection::IdempotencyConflict)
	));
	let (status, value) = call(
		&app,
		"POST",
		"/api/v1/projects/notes/operations",
		Some(OWNER),
		Some(json!(intent(1, "same-key", Action::Plan))),
	)
	.await;
	assert_eq!(status, StatusCode::CONFLICT);
	assert_eq!(value["error"], "idempotency_conflict");

	// An embedder cannot record an unbounded or non-printable actor.
	let hostile = Principal::owner().with_actor("\u{1F600} admin");
	assert!(matches!(
		service
			.enqueue(
				&hostile,
				"other",
				&intent(0, "hostile-actor", Action::Status)
			)
			.unwrap_err(),
		EnqueueError::Rejected(Rejection::InvalidActor)
	));
	let oversized = Principal::owner().with_actor("a".repeat(257));
	assert!(matches!(
		service
			.enqueue(
				&oversized,
				"other",
				&intent(0, "long-actor", Action::Status)
			)
			.unwrap_err(),
		EnqueueError::Rejected(Rejection::InvalidActor)
	));

	// The actor is shown in the dashboard's operation history.
	let mut page = request("GET", "/", None);
	page.headers_mut()
		.insert("authorization", format!("Bearer {OWNER}").parse().unwrap());
	let response = app.clone().oneshot(page).await.unwrap();
	assert_eq!(response.status(), StatusCode::OK);
	let html = String::from_utf8(
		to_bytes(response.into_body(), 2 * 1024 * 1024)
			.await
			.unwrap()
			.to_vec(),
	)
	.unwrap();
	assert!(html.contains("by admin@example.com"), "actor in history");
}

#[tokio::test]
async fn subscribe_streams_progress_and_the_terminal_operation() {
	let (_root, store) = setup();
	let service = service(&store);
	let admin = Principal::scoped(Scope::Deploy, "notes").with_actor("admin@example.com");

	let operation = service
		.enqueue(&admin, "notes", &intent(0, "subscribe", Action::Status))
		.unwrap();
	let mut subscription = service.subscribe(&operation.id);
	let handle = service
		.worker(Arc::new(FakeExecutor {
			service: service.clone(),
		}))
		.unwrap();

	let mut progress = Vec::new();
	let finished = loop {
		let update = tokio::time::timeout(Duration::from_secs(10), subscription.next())
			.await
			.expect("updates arrive within the deadline")
			.expect("the stream continues until the terminal update");

		match update {
			Update::Progress { event, .. } => progress.push((event.phase, event.message)),
			Update::Finished { operation } => break operation,
		}
	};

	assert_eq!(finished.phase, Phase::Succeeded);
	assert_eq!(finished.actor.as_deref(), Some("admin@example.com"));
	assert_eq!(
		progress,
		vec![
			(
				Phase::Running,
				"Running on the registered deployment worker".into()
			),
			(Phase::Running, "halfway".into()),
		],
		"claim and executor progress arrive in order"
	);
	assert_eq!(
		store.operation(&finished.id).unwrap().unwrap().phase,
		Phase::Succeeded
	);
	handle.abort();
}

#[tokio::test]
async fn api_router_nests_under_a_prefix() {
	let (_root, store) = setup();
	let service = service(&store);
	let app = Router::new().nest("/infra/api", service.api_router());

	let (status, value) = call(&app, "GET", "/infra/api/api/v1/overview", Some(OWNER), None).await;
	assert_eq!(status, StatusCode::OK);
	assert!(value["projects"].is_array());

	assert_eq!(
		call(&app, "GET", "/infra/api/healthz", None, None).await.0,
		StatusCode::OK
	);
	assert_eq!(
		call(&app, "GET", "/infra/api/api/v1/overview", None, None)
			.await
			.0,
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		call(&app, "GET", "/infra/api/", None, None).await.0,
		StatusCode::NOT_FOUND,
		"the dashboard page stays out of the API router"
	);
	assert_eq!(
		call(&app, "GET", "/infra/api/console.mjs", Some(OWNER), None)
			.await
			.0,
		StatusCode::NOT_FOUND,
		"the dashboard script stays out of the API router"
	);
}

#[tokio::test]
async fn the_events_route_streams_server_sent_events() {
	let (_root, store) = setup();
	let service = service(&store);
	let owner = Principal::owner();
	let operation = service
		.enqueue(&owner, "notes", &intent(0, "sse", Action::Status))
		.unwrap();

	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let router = service.router();
	let serving = tokio::spawn(async move {
		axum::serve(listener, router).await.unwrap();
	});

	let buffer = Arc::new(Mutex::new(String::new()));
	let reader = {
		let buffer = buffer.clone();
		let id = operation.id.clone();
		tokio::spawn(async move {
			let mut response = reqwest::Client::new()
				.get(format!("http://{address}/api/v1/operations/{id}/events"))
				.header("authorization", format!("Bearer {OWNER}"))
				.send()
				.await
				.unwrap();
			assert_eq!(response.status(), StatusCode::OK);
			assert!(
				response.headers()["content-type"]
					.to_str()
					.unwrap()
					.starts_with("text/event-stream")
			);

			while let Some(chunk) = response.chunk().await.unwrap() {
				buffer
					.lock()
					.unwrap()
					.push_str(&String::from_utf8_lossy(&chunk));
			}
		})
	};

	// The stream opens with the pre-worker snapshot.
	wait_for(&buffer, "event: operation").await;
	wait_for(&buffer, "\"phase\":\"queued\"").await;

	let handle = service
		.worker(Arc::new(FakeExecutor {
			service: service.clone(),
		}))
		.unwrap();
	wait_for(&buffer, "event: progress").await;
	wait_for(&buffer, "event: done").await;
	wait_for(&buffer, "\"phase\":\"succeeded\"").await;
	wait_for(&buffer, "fake worker finished").await;

	handle.abort();
	reader.abort();
	serving.abort();
}

async fn wait_for(buffer: &Mutex<String>, marker: &str) {
	tokio::time::timeout(Duration::from_secs(10), async {
		loop {
			if buffer.lock().unwrap().contains(marker) {
				return;
			}

			tokio::time::sleep(Duration::from_millis(20)).await;
		}
	})
	.await
	.unwrap_or_else(|_| panic!("expected {marker:?} to arrive"));
}
