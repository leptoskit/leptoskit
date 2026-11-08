#![cfg(feature = "server")]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;

use axum::Router;
use axum::body::Body;
use axum::body::to_bytes;
use axum::http::HeaderMap;
use axum::http::Request;
use axum::http::StatusCode;
use leptoskit::analytics::Analytics;
use leptoskit::analytics::ExportHealth;
use leptoskit::analytics::HostSample;
use leptoskit::analytics::Limits;
use leptoskit::analytics::Outcome;
use leptoskit::analytics::Resource;
use leptoskit::analytics::SeriesKey;
use leptoskit::analytics::Signal;
use leptoskit::analytics::Snapshot;
use leptoskit::analytics::Source;
use leptoskit_deploy_console::control::*;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt as _;

const OWNER: &str = "test-only-management-owner-key-0123456789";
const ORIGIN: &str = "http://localhost:4700";

fn clock() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_secs()
}

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

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Value) {
	let response = app.clone().oneshot(request).await.unwrap();
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
	let mut request = request(method, path, body);

	if let Some(bearer) = bearer {
		request
			.headers_mut()
			.insert("authorization", format!("Bearer {bearer}").parse().unwrap());
	}

	let (status, _, value) = send(app, request).await;
	(status, value)
}

fn service(store: &Store) -> Service {
	Service::new(store.clone(), OWNER, ORIGIN).unwrap()
}

fn finish(store: &Store, id: &str, succeeded: bool) {
	store
		.finish(
			id,
			&RunResult {
				succeeded,
				message: if succeeded {
					"Finished".into()
				} else {
					"Failed".into()
				},

				details: json!({"code":"test"}),
			},
		)
		.unwrap();
}

fn run(store: &Store, project: &str, revision: u64, key: &str, action: Action) -> Operation {
	let details = if matches!(action, Action::Plan) {
		json!({"changes":[
			{"address":"module.notes","actions":["create","update"]},
			{"actions":[1,"read"]}
		]})
	} else {
		json!({"code":"test"})
	};

	let operation = store
		.enqueue(project, &intent(revision, key, action))
		.unwrap();
	store.claim_project(Some(project)).unwrap().unwrap();
	store
		.finish(
			&operation.id,
			&RunResult {
				succeeded: true,
				message: "Finished".into(),
				details,
			},
		)
		.unwrap();
	store.operation(&operation.id).unwrap().unwrap()
}

#[test]
fn service_configuration_rejects_unsafe_keys_and_origins() {
	let (_root, store) = setup();
	let short = "x".repeat(31);
	let spaced = "word with space ".repeat(3);

	for key in [String::new(), "short".to_owned(), short, spaced] {
		assert!(Service::new(store.clone(), &key, ORIGIN).is_err());
	}
	for origin in [
		"http://deploy.example.com",
		"https://deploy.example.com/path",
		"https://deploy.example.com/?query=1",
		"https://deploy.example.com/#fragment",
		"https://user:secret@deploy.example.com",
		"not a url",
	] {
		assert!(
			Service::new(store.clone(), OWNER, origin).is_err(),
			"{origin}"
		);
	}

	for origin in ["https://deploy.example.com", ORIGIN] {
		let service = Service::new(store.clone(), OWNER, origin).unwrap();

		assert_eq!(
			service.origin(),
			if origin == ORIGIN {
				ORIGIN
			} else {
				"https://deploy.example.com"
			}
		);
	}
}

#[tokio::test]
async fn unauthenticated_requests_receive_security_headers_and_json_errors() {
	let (_root, store) = setup();
	let app = service(&store).router();
	let batch = serde_json::to_value(snapshot("notes", "unauth", 1)).unwrap();
	for (method, path, body) in [
		("GET", "/api/v1/overview", None),
		("GET", "/api/v1/operations/missing", None),
		(
			"POST",
			"/api/v1/tokens",
			Some(json!({"scope":"read","project":"notes"})),
		),
		(
			"POST",
			"/api/v1/tokens/revoke",
			Some(json!({"key":"missing"})),
		),
		("GET", "/api/v1/projects/notes/analytics", None),
		(
			"POST",
			"/api/v1/projects/notes/analytics/ingest",
			Some(batch),
		),
		(
			"POST",
			"/api/v1/projects/notes/operations",
			Some(json!(intent(0, "unauth", Action::Status))),
		),
		("POST", "/api/v1/login", Some(json!({"key":OWNER}))),
		("POST", "/api/v1/logout", None),
	] {
		if path == "/api/v1/login" {
			continue;
		}

		let (status, value) = call(&app, method, path, None, body).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
		assert_eq!(value["error"], "authentication_required");
	}
	let (status, value) = call(
		&app,
		"POST",
		"/api/v1/login",
		None,
		Some(json!({"key":OWNER})),
	)
	.await;
	assert_eq!(status, StatusCode::FORBIDDEN);
	assert_eq!(value["error"], "scope_or_origin_denied");

	for bearer in ["short", &"a".repeat(64)] {
		let (status, _) = call(&app, "GET", "/api/v1/overview", Some(bearer), None).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED);
	}

	let (status, ..) = send(&app, request("GET", "/healthz", None)).await;

	assert_eq!(status, StatusCode::OK);
	let (_, headers, _) = send(&app, request("GET", "/healthz", None)).await;
	assert_eq!(headers["cache-control"], "no-store");
	assert_eq!(headers["x-content-type-options"], "nosniff");
	assert_eq!(headers["referrer-policy"], "no-referrer");
	assert_eq!(headers["vary"], "Cookie, Authorization");
	assert!(
		headers["content-security-policy"]
			.to_str()
			.unwrap()
			.contains("frame-ancestors 'none'")
	);
	let (status, headers, _) = send(&app, request("GET", "/console.mjs", None)).await;
	assert_eq!(status, StatusCode::OK);
	assert!(
		headers["content-type"]
			.to_str()
			.unwrap()
			.contains("text/javascript")
	);
}

#[tokio::test]
async fn login_issues_origin_bound_sessions_and_logout_revokes_them() {
	let (_root, store) = setup();
	let app = service(&store).router();
	let (status, _, value) = send(
		&app,
		request("POST", "/api/v1/login", Some(json!({"key":OWNER}))),
	)
	.await;
	assert_eq!(status, StatusCode::FORBIDDEN);
	assert_eq!(value["error"], "scope_or_origin_denied");
	let mut oversized = request(
		"POST",
		"/api/v1/login",
		Some(json!({"key":"k".repeat(257)})),
	);
	oversized
		.headers_mut()
		.insert("origin", ORIGIN.parse().unwrap());
	assert_eq!(send(&app, oversized).await.0, StatusCode::FORBIDDEN);

	let mut unknown = request("POST", "/api/v1/login", Some(json!({"key":"a".repeat(64)})));
	unknown
		.headers_mut()
		.insert("origin", ORIGIN.parse().unwrap());
	assert_eq!(send(&app, unknown).await.0, StatusCode::UNAUTHORIZED);
	let read = store
		.issue(Scope::Read, Some("notes"), 3600, false)
		.unwrap();
	let mut scoped = request("POST", "/api/v1/login", Some(json!({"key":read})));
	scoped
		.headers_mut()
		.insert("origin", ORIGIN.parse().unwrap());
	assert_eq!(send(&app, scoped).await.0, StatusCode::FORBIDDEN);
	let mut login = request("POST", "/api/v1/login", Some(json!({"key":OWNER})));
	login
		.headers_mut()
		.insert("origin", ORIGIN.parse().unwrap());
	let (status, headers, _) = send(&app, login).await;
	assert_eq!(status, StatusCode::NO_CONTENT);
	let cookie = headers["set-cookie"].to_str().unwrap().to_owned();
	assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
	assert!(!cookie.contains("Secure"));
	let cookie = cookie.split(';').next().unwrap().to_owned();
	let mut overview = request("GET", "/api/v1/overview", None);
	overview
		.headers_mut()
		.insert("cookie", cookie.parse().unwrap());
	assert_eq!(send(&app, overview).await.0, StatusCode::OK);
	let mut cross_origin = request("POST", "/api/v1/logout", None);
	cross_origin
		.headers_mut()
		.insert("cookie", cookie.parse().unwrap());
	assert_eq!(send(&app, cross_origin).await.0, StatusCode::FORBIDDEN);
	let mut logout = request("POST", "/api/v1/logout", None);
	logout
		.headers_mut()
		.insert("cookie", cookie.parse().unwrap());
	logout
		.headers_mut()
		.insert("origin", ORIGIN.parse().unwrap());
	let (status, headers, _) = send(&app, logout).await;
	assert_eq!(status, StatusCode::NO_CONTENT);
	assert!(
		headers["set-cookie"]
			.to_str()
			.unwrap()
			.contains("Max-Age=0")
	);
	let mut revoked = request("GET", "/api/v1/overview", None);
	revoked
		.headers_mut()
		.insert("cookie", cookie.parse().unwrap());
	assert_eq!(send(&app, revoked).await.0, StatusCode::UNAUTHORIZED);
	assert_eq!(
		call(&app, "POST", "/api/v1/logout", Some(OWNER), None)
			.await
			.0,
		StatusCode::NO_CONTENT
	);
	let secure = Service::new(store, OWNER, "https://deploy.example.com").unwrap();
	let mut login = request("POST", "/api/v1/login", Some(json!({"key":OWNER})));
	login
		.headers_mut()
		.insert("origin", "https://deploy.example.com".parse().unwrap());
	let (status, headers, _) = send(&secure.router(), login).await;
	assert_eq!(status, StatusCode::NO_CONTENT);
	assert!(headers["set-cookie"].to_str().unwrap().contains("Secure"));
}

#[tokio::test]
async fn issued_tokens_are_scoped_to_projects_and_cannot_escalate() {
	let (_root, store) = setup();
	let app = service(&store).router();
	let (status, issued) = call(
		&app,
		"POST",
		"/api/v1/tokens",
		Some(OWNER),
		Some(json!({"scope":"read","project":"notes"})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(issued["expires_in"], 30 * 86400);
	let read = issued["key"].as_str().unwrap();
	assert_eq!(read.len(), 64);
	let (_, overview) = call(&app, "GET", "/api/v1/overview", Some(read), None).await;

	let projects: Vec<&str> = overview["projects"]
		.as_array()
		.unwrap()
		.iter()
		.map(|project| project["id"].as_str().unwrap())
		.collect();
	assert_eq!(projects, ["notes"]);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/projects/notes/operations",
			Some(read),
			Some(json!(intent(0, "read-denied", Action::Status)))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/tokens",
			Some(read),
			Some(json!({"scope":"read","project":"notes"}))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/tokens",
			Some(OWNER),
			Some(json!({"scope":"owner","project":"notes"}))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/tokens/revoke",
			Some(read),
			Some(json!({"key":"a".repeat(64)}))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	let (status, deploy) = call(
		&app,
		"POST",
		"/api/v1/tokens",
		Some(OWNER),
		Some(json!({"scope":"deploy","project":"notes"})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	let deploy = deploy["key"].as_str().unwrap();
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/projects/notes/operations",
			Some(deploy),
			Some(json!(intent(0, "deploy-denied", Action::Plan)))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	let (status, operation) = call(
		&app,
		"POST",
		"/api/v1/projects/notes/operations",
		Some(deploy),
		Some(json!(intent(0, "deploy-status", Action::Status))),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(
		call(
			&app,
			"GET",
			&format!("/api/v1/operations/{}", operation["id"].as_str().unwrap()),
			Some(deploy),
			None
		)
		.await
		.0,
		StatusCode::OK
	);
	assert_eq!(
		call(
			&app,
			"GET",
			"/api/v1/projects/other/analytics",
			Some(read),
			None
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/tokens/revoke",
			Some(OWNER),
			Some(json!({"key":read}))
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/overview", Some(read), None)
			.await
			.0,
		StatusCode::UNAUTHORIZED
	);
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn operation_errors_are_reported_with_stable_codes() {
	let (_root, store) = setup();
	let app = service(&store).router();
	let cases = [
		(
			"unknown",
			0,
			"unknown-project",
			Action::Status,
			"operation_rejected",
		),
		("notes", 0, "bad key", Action::Status, "invalid_intent"),
		(
			"notes",
			0,
			"empty-version",
			Action::Deploy {
				version: String::new(),
				setup: false,
				maintenance: false,
			},
			"invalid_intent",
		),
		(
			"notes",
			0,
			"flagged-version",
			Action::Deploy {
				version: "-latest".into(),
				setup: false,
				maintenance: false,
			},
			"invalid_intent",
		),
		(
			"notes",
			0,
			"bad-plan",
			Action::Apply {
				plan_id: "bad plan".into(),
			},
			"invalid_intent",
		),
		(
			"notes",
			4,
			"stale-revision",
			Action::Status,
			"revision_conflict",
		),
		(
			"notes",
			0,
			"queued",
			Action::Status,
			"operation_in_progress_placeholder",
		),
	];

	for (project, revision, key, action, expected) in cases {
		let (status, value) = call(
			&app,
			"POST",
			&format!("/api/v1/projects/{project}/operations"),
			Some(OWNER),
			Some(json!(intent(revision, key, action))),
		)
		.await;

		if expected == "operation_in_progress_placeholder" {
			assert_eq!(status, StatusCode::OK);
			assert_eq!(value["phase"], "queued");

			continue;
		}

		assert_eq!(status, StatusCode::CONFLICT, "{key}");
		assert_eq!(value["error"], expected, "{key}");
	}

	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/projects/notes/operations",
			Some(OWNER),
			Some(json!(intent(1, "duplicate", Action::Status)))
		)
		.await
		.1["error"],
		"operation_in_progress"
	);
	let queued = store.operations().unwrap().into_iter().next().unwrap();
	let (status, again) = call(
		&app,
		"POST",
		"/api/v1/projects/notes/operations",
		Some(OWNER),
		Some(json!(intent(0, "queued", Action::Status))),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(again["id"], queued.id);
	let (status, value) = call(
		&app,
		"POST",
		"/api/v1/projects/notes/operations",
		Some(OWNER),
		Some(json!(intent(0, "queued", Action::Plan))),
	)
	.await;
	assert_eq!(status, StatusCode::CONFLICT);
	assert_eq!(value["error"], "idempotency_conflict");
	let (status, value) = call(
		&app,
		"POST",
		"/api/v1/projects/notes/operations",
		Some(OWNER),
		Some(json!(intent(1, "busy", Action::Status))),
	)
	.await;
	assert_eq!(status, StatusCode::CONFLICT);
	assert_eq!(value["error"], "operation_in_progress");
	store.claim_project(Some("notes")).unwrap().unwrap();
	finish(&store, &queued.id, true);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/projects/notes/operations",
			Some(OWNER),
			Some(json!(intent(
				1,
				"apply-without-plan",
				Action::Apply {
					plan_id: "missing".into()
				}
			)))
		)
		.await
		.1["error"],
		"plan_is_not_current"
	);
	let plan = run(&store, "notes", 1, "plan", Action::Plan);
	let apply = run(
		&store,
		"notes",
		2,
		"apply",
		Action::Apply {
			plan_id: plan.id.clone(),
		},
	);
	assert_eq!(apply.phase, Phase::Succeeded);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/projects/notes/operations",
			Some(OWNER),
			Some(json!(intent(
				3,
				"reuse-plan",
				Action::Apply { plan_id: plan.id }
			)))
		)
		.await
		.1["error"],
		"plan_already_consumed"
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/operations/missing", Some(OWNER), None)
			.await
			.0,
		StatusCode::NOT_FOUND
	);
}

#[tokio::test]
async fn analytics_ingest_and_fleet_queries_enforce_scopes_and_ranges() {
	let (_root, store) = setup();
	let app = service(&store).router();
	let snapshot = snapshot("notes", "one", 2);
	let (status, _) = call(
		&app,
		"POST",
		"/api/v1/projects/notes/analytics/ingest",
		Some(OWNER),
		Some(serde_json::to_value(&snapshot).unwrap()),
	)
	.await;
	assert_eq!(status, StatusCode::NO_CONTENT);
	let (status, fleet) = call(
		&app,
		"GET",
		"/api/v1/projects/notes/analytics",
		Some(OWNER),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(fleet["instances"], 1);

	let to = clock();
	let (status, fleet) = call(
		&app,
		"GET",
		&format!("/api/v1/projects/notes/analytics?from={}&to={to}", to - 600),
		Some(OWNER),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(fleet["series"][0]["windows"][0]["count"], 2);

	for query in ["?from=0&to=1", "?from=0&to=999999999999"] {
		assert_eq!(
			call(
				&app,
				"GET",
				&format!("/api/v1/projects/notes/analytics{query}"),
				Some(OWNER),
				None
			)
			.await
			.0,
			StatusCode::BAD_REQUEST,
			"{query}"
		);
	}

	let read = store
		.issue(Scope::Read, Some("notes"), 3600, false)
		.unwrap();
	let ingest = store
		.issue(Scope::Ingest, Some("notes"), 3600, false)
		.unwrap();
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/projects/notes/analytics/ingest",
			Some(&ingest),
			Some(serde_json::to_value(&snapshot).unwrap())
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/projects/notes/analytics/ingest",
			Some(&read),
			Some(serde_json::to_value(&snapshot).unwrap())
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	let mut invalid = snapshot.clone();
	invalid.format_version = 2;
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/projects/notes/analytics/ingest",
			Some(OWNER),
			Some(serde_json::to_value(&invalid).unwrap())
		)
		.await
		.0,
		StatusCode::BAD_REQUEST
	);
	let mut mismatch = snapshot;
	mismatch.resource.environment = "another".into();
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/projects/notes/analytics/ingest",
			Some(OWNER),
			Some(serde_json::to_value(&mismatch).unwrap())
		)
		.await
		.0,
		StatusCode::BAD_REQUEST
	);
	assert_eq!(
		call(
			&app,
			"GET",
			"/api/v1/projects/notes/analytics",
			Some(&ingest),
			None
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
}

#[tokio::test]
async fn managed_app_keys_are_deterministic_and_revocation_blocks_reissue() {
	let (_root, store) = setup();
	let service = service(&store);
	let app = service.router();
	let read = service.managed_app_key("notes", false).unwrap();
	assert_eq!(service.managed_app_key("notes", false).unwrap(), read);
	let ingest = service.managed_app_key("notes", true).unwrap();
	assert_eq!(service.managed_app_key("notes", true).unwrap(), ingest);
	assert_ne!(read, ingest);

	assert!(service.managed_app_key("bad/project", false).is_err());
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/projects/notes/analytics/ingest",
			Some(&ingest),
			Some(serde_json::to_value(snapshot("notes", "managed", 1)).unwrap())
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	store.revoke(&ingest).unwrap();
	assert!(service.managed_app_key("notes", true).is_err());
}

#[tokio::test]
async fn worker_executes_queued_operations_and_reports_progress() {
	struct Recorder {
		service: Service,
		seen: Arc<Mutex<Vec<String>>>,
	}
	impl Executor for Recorder {
		fn run(
			&self,
			project: Project,
			operation: Operation,
		) -> Pin<Box<dyn Future<Output = RunResult> + Send>> {
			let service = self.service.clone();
			let seen = self.seen.clone();
			Box::pin(async move {
				service.progress(&operation.id, "working").unwrap();
				seen.lock()
					.unwrap()
					.push(format!("{}:{}", project.id, operation.id));
				RunResult {
					succeeded: true,
					message: "worker finished".into(),
					details: json!({"project":project.id}),
				}
			})
		}
	}
	let (_root, store) = setup();
	let service = service(&store);
	let seen = Arc::new(Mutex::new(Vec::new()));
	let handle = service
		.worker(Arc::new(Recorder {
			service: service.clone(),
			seen: seen.clone(),
		}))
		.unwrap();
	let first = store
		.enqueue("notes", &intent(0, "worker-1", Action::Status))
		.unwrap();
	let second = store
		.enqueue("other", &intent(0, "worker-2", Action::Backup))
		.unwrap();

	for id in [&first.id, &second.id] {
		tokio::time::timeout(std::time::Duration::from_secs(10), async {
			loop {
				let operation = store.operation(id).unwrap().unwrap();
				if operation.phase == Phase::Succeeded {
					assert_eq!(operation.message, "worker finished");
					assert_eq!(operation.result["project"], operation.project);
					break;
				}

				tokio::time::sleep(std::time::Duration::from_millis(10)).await;
			}
		})
		.await
		.unwrap();
	}

	assert_eq!(seen.lock().unwrap().len(), 2);
	handle.abort();
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn dashboard_assembles_history_plans_and_measurements() {
	let (root, store) = setup();
	let app = service(&store).router();
	let plan = run(&store, "notes", 0, "plan", Action::Plan);
	let apply = run(
		&store,
		"notes",
		1,
		"apply",
		Action::Apply {
			plan_id: plan.id.clone(),
		},
	);
	let deploy = run(
		&store,
		"notes",
		2,
		"deploy",
		Action::Deploy {
			version: "v1".into(),
			setup: true,
			maintenance: false,
		},
	);
	run(
		&store,
		"notes",
		3,
		"rollback",
		Action::Rollback {
			version: "v0".into(),
		},
	);
	run(&store, "notes", 4, "status", Action::Status);
	run(&store, "notes", 5, "backup", Action::Backup);

	store
		.enqueue("notes", &intent(6, "queued", Action::Status))
		.unwrap();
	let now = clock() as i64;
	let database = root.path().join("console.sqlite3");
	let db = rusqlite::Connection::open(database).unwrap();
	db.execute(
		"UPDATE operations SET updated=?1 WHERE id=?2",
		rusqlite::params![now - 7200, deploy.id],
	)
	.unwrap();
	db.execute(
		"UPDATE operations SET updated=?1 WHERE id=?2",
		rusqlite::params![now - 120, apply.id],
	)
	.unwrap();
	drop(db);
	let mut recent = snapshot_with_host(
		"notes",
		"one",
		2,
		HostSample {
			cgroup_memory_bytes: Some(2 * 1024 * 1024),
			..HostSample::default()
		},
	);
	// The dashboard's range ends exclusively at now. Use a completed minute
	// so this assertion also holds at the exact start of a wall-clock minute.
	for window in &mut recent.series[0].windows {
		window.start_unix_seconds = recent.observed_unix_seconds.saturating_sub(60) / 60 * 60;
	}

	store.ingest("notes", &recent).unwrap();
	let mut old = snapshot("notes", "two", 1);
	old.series[0].key.outcome = Outcome::ServerError;
	old.observed_unix_seconds = clock().saturating_sub(300);
	old.started_unix_seconds = old.observed_unix_seconds;

	for window in &mut old.series[0].windows {
		window.start_unix_seconds = old.observed_unix_seconds / 60 * 60;
	}

	old.host = Some(HostSample {
		process_resident_bytes: Some(1024 * 1024),
		filesystem_available_bytes: Some(2 * 1024 * 1024 * 1024),
		..HostSample::default()
	});
	old.export = Some(ExportHealth::default());
	store.ingest("notes", &old).unwrap();
	let (status, ..) = send(&app, request("GET", "/", None)).await;
	assert_eq!(status, StatusCode::OK);
	let login_page = String::from_utf8(
		to_bytes(
			app.clone()
				.oneshot(request("GET", "/", None))
				.await
				.unwrap()
				.into_body(),
			2 * 1024 * 1024,
		)
		.await
		.unwrap()
		.to_vec(),
	)
	.unwrap();
	assert!(login_page.contains("Connect to your deployment service"));
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
	for expected in [
		"Your applications",
		"notes",
		"other",
		"Plan infrastructure",
		"Infrastructure plan",
		"module.notes",
		"resource",
		"Apply this plan",
		"Apply infrastructure",
		"Deploy v1",
		"Rollback to v0",
		"Health check",
		"Backup verification",
		"Just now",
		"2m ago",
		"2h ago",
		"2 reporting instances · 1 stale",
		"memory 2.0 MiB · filesystem available unavailable",
		"memory 1.0 MiB · filesystem available 2.0 GiB",
		"2 samples",
		"Waiting for measurements",
		"text-red-600",
	] {
		assert!(html.contains(expected), "missing {expected}");
	}
	assert!(!html.contains(OWNER));
	let empty = tempfile::tempdir().unwrap();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(empty.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
	}
	let empty = Store::open(&empty.path().join("console.sqlite3")).unwrap();
	let empty_app = service(&empty).router();
	let mut page = request("GET", "/", None);
	page.headers_mut()
		.insert("authorization", format!("Bearer {OWNER}").parse().unwrap());
	let response = empty_app.oneshot(page).await.unwrap();
	let html = String::from_utf8(
		to_bytes(response.into_body(), 2 * 1024 * 1024)
			.await
			.unwrap()
			.to_vec(),
	)
	.unwrap();
	assert!(html.contains("No projects registered"));
}

fn snapshot(project: &str, instance: &str, count: usize) -> Snapshot {
	snapshot_with_host(project, instance, count, HostSample::default())
}

fn snapshot_with_host(project: &str, instance: &str, count: usize, host: HostSample) -> Snapshot {
	let analytics = Analytics::new(
		Resource {
			service: project.into(),
			environment: "production".into(),
			release: "v1".into(),
			instance: instance.into(),
		},
		Limits::default(),
		clock,
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

	let mut snapshot = analytics.snapshot();
	snapshot.host = Some(host);
	snapshot
}
