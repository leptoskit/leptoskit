#![cfg(feature = "server")]

use std::fmt::Write as _;

use axum::Router;
use axum::body::Body;
use axum::body::to_bytes;
use axum::http::HeaderMap;
use axum::http::Request;
use axum::http::StatusCode;
use hmac::Mac as _;
use leptoskit_deploy_console::control::Action;
use leptoskit_deploy_console::control::Intent;
use leptoskit_deploy_console::hosted::Billing;
use leptoskit_deploy_console::hosted::Hosted;
use leptoskit_deploy_console::hosted::Ownership;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt as _;

const SECRET: &str = "test-only-hosted-encryption-secret-0123456789";

fn clock() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_secs()
}

fn setup(with_billing: bool) -> (tempfile::TempDir, Hosted, Option<Billing>, String, String) {
	let dir = tempfile::tempdir().unwrap();
	let billing = with_billing.then(|| {
		Billing::new(
			"test".into(),
			"test-hook".into(),
			"price_control".into(),
			"price_team".into(),
		)
		.unwrap()
	});
	let host = Hosted::open(
		&dir.path().join("cloud"),
		SECRET.into(),
		"http://localhost:4701",
		billing.clone(),
	)
	.unwrap();
	let alice = host.accounts().create("alice", "cus_alice").unwrap();
	let bob = host.accounts().create("bob", "cus_bob").unwrap();
	(dir, host, billing, alice, bob)
}

fn activate(billing: &Billing, host: &Hosted, account: &str, status: &str, price: &str) {
	billing
		.apply_subscription(
			host,
			account,
			&json!({
				"id":format!("sub_{account}"),
				"customer":format!("cus_{account}"),
				"status":status,
				"items":{"data":[{"quantity":1,"price":{"id":price},"current_period_end":clock()+3600}]}
			}),
		)
		.unwrap();
}

fn request(method: &str, path: &str, body: Option<Vec<u8>>) -> Request<Body> {
	Request::builder()
		.method(method)
		.uri(path)
		.header("content-type", "application/json")
		.body(body.map_or_else(Body::empty, Body::from))
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
	key: Option<&str>,
	body: Option<Value>,
) -> (StatusCode, Value) {
	let mut request = request(
		method,
		path,
		body.map(|value| serde_json::to_vec(&value).unwrap()),
	);
	if let Some(key) = key {
		request
			.headers_mut()
			.insert("authorization", format!("Bearer {key}").parse().unwrap());
	}
	let (status, _, value) = send(app, request).await;
	(status, value)
}

async fn pair(app: &Router, host: &Hosted, account: &str, project: &str) -> String {
	let code = host.accounts().pairing(account, project).unwrap();
	let (status, value) = call(
		app,
		"POST",
		"/api/v1/worker/pair",
		None,
		Some(json!({"code":code})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	value["key"].as_str().unwrap().into()
}

async fn revision(app: &Router, account: &str, project: &str) -> u64 {
	let (_, overview) = call(app, "GET", "/api/v1/overview", Some(account), None).await;
	overview["projects"]
		.as_array()
		.unwrap()
		.iter()
		.find(|value| value["id"] == project)
		.unwrap()["revision"]
		.as_u64()
		.unwrap()
}

async fn enqueue(
	app: &Router,
	account: &str,
	project: &str,
	revision: u64,
	key: &str,
	action: Action,
) -> Value {
	let (status, operation) = call(
		app,
		"POST",
		&format!("/api/v1/projects/{project}/operations"),
		Some(account),
		Some(json!(Intent {
			expected_revision: revision,
			idempotency_key: key.into(),
			action,
		})),
	)
	.await;
	assert_eq!(status, StatusCode::OK, "{operation}");
	operation
}

fn signed(secret: &str, event: &Value) -> (String, Vec<u8>) {
	let bytes = serde_json::to_vec(event).unwrap();
	let timestamp = clock();
	let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
	mac.update(format!("{timestamp}.").as_bytes());
	mac.update(&bytes);
	let mut hex = String::new();
	for byte in mac.finalize().into_bytes() {
		write!(&mut hex, "{byte:02x}").unwrap();
	}
	(format!("t={timestamp},v1={hex}"), bytes)
}

async fn deliver(app: &Router, event: Value) -> (StatusCode, Value) {
	let (signature, bytes) = signed("test-hook", &event);
	let mut request = request("POST", "/api/v1/webhooks/stripe", Some(bytes));
	request
		.headers_mut()
		.insert("stripe-signature", signature.parse().unwrap());
	let (status, _, value) = send(app, request).await;
	(status, value)
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn accounts_install_plans_only_after_entitlement_and_within_plan_limits() {
	let (_dir, host, billing, alice, _bob) = setup(true);
	let billing = billing.unwrap();
	let app = host.router();
	assert_eq!(
		call(&app, "GET", "/healthz", None, None).await.0,
		StatusCode::OK
	);
	let (_, headers, _) = send(&app, request("GET", "/healthz", None)).await;
	assert_eq!(headers["cache-control"], "no-store");
	assert_eq!(headers["x-content-type-options"], "nosniff");
	assert_eq!(headers["referrer-policy"], "no-referrer");
	assert_eq!(
		call(&app, "GET", "/api/v1/account", None, None).await.0,
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/account", Some(&"a".repeat(64)), None)
			.await
			.0,
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/account", Some(&"a".repeat(257)), None)
			.await
			.0,
		StatusCode::UNAUTHORIZED
	);
	let (status, account) = call(&app, "GET", "/api/v1/account", Some(&alice), None).await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(account["status"], "inactive");
	assert_eq!(account["plan"], "control");
	assert_eq!(account["installations"].as_array().unwrap().len(), 0);
	let install = |project: &str, environment: &str, ownership: &str| {
		json!({
			"project":project,
			"environment":environment,
			"ownership":ownership
		})
	};
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/installations",
			Some(&alice),
			Some(install("notes", "production", "customer"))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	activate(&billing, &host, "alice", "active", "price_control");
	let (status, notes) = call(
		&app,
		"POST",
		"/api/v1/installations",
		Some(&alice),
		Some(install("notes", "production", "customer")),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(notes["ownership"], "customer");
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/installations",
			Some(&alice),
			Some(install("notes", "production", "customer"))
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/installations",
			Some(&alice),
			Some(install("managed", "production", "leptoskit_digitalocean"))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/installations",
			Some(&alice),
			Some(install("invalid/id", "production", "customer"))
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/installations",
			Some(&alice),
			Some(install("other", "", "customer"))
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/installations",
			Some(&alice),
			Some(
				json!({"project":"other","environment":"production","ownership":"customer","extra":true})
			)
		)
		.await
		.0,
		StatusCode::UNPROCESSABLE_ENTITY
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/installations",
			Some(&alice),
			Some(install("other", "production", "customer"))
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	activate(&billing, &host, "alice", "active", "price_team");
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/installations",
			Some(&alice),
			Some(install("other", "production", "customer"))
		)
		.await
		.0,
		StatusCode::OK
	);
	let (_, account) = call(&app, "GET", "/api/v1/account", Some(&alice), None).await;
	assert_eq!(account["plan"], "team");
	assert_eq!(account["installations"].as_array().unwrap().len(), 2);
	assert_eq!(account["installations"][0]["last_seen"], 0);
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn pairing_is_single_use_and_workers_only_finish_their_own_project() {
	let (_dir, host, billing, alice, _bob) = setup(true);
	let billing = billing.unwrap();
	activate(&billing, &host, "alice", "active", "price_team");
	host.register("alice", "notes", "production", Ownership::Customer)
		.unwrap();
	host.register("alice", "other", "production", Ownership::Customer)
		.unwrap();
	host.register(
		"alice",
		"managed",
		"production",
		Ownership::LeptoskitDigitalocean,
	)
	.unwrap();
	let app = host.router();
	let (status, value) = call(
		&app,
		"POST",
		"/api/v1/installations/notes/pair",
		Some(&alice),
		None,
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(value["expires_in"], 600);
	assert_eq!(value["code"].as_str().unwrap().len(), 70);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/installations/missing/pair",
			Some(&alice),
			None
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/installations/managed/pair",
			Some(&alice),
			None
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(&app, "POST", "/api/v1/worker/pair", None, Some(json!({})))
			.await
			.0,
		StatusCode::BAD_REQUEST
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/pair",
			None,
			Some(json!({"code":"a".repeat(257)}))
		)
		.await
		.0,
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/pair",
			None,
			Some(json!({"code":"missing"}))
		)
		.await
		.0,
		StatusCode::UNAUTHORIZED
	);
	let code = host.accounts().pairing("alice", "notes").unwrap();
	let (status, paired) = call(
		&app,
		"POST",
		"/api/v1/worker/pair",
		None,
		Some(json!({"code":code.clone()})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(paired["installation"]["project"], "notes");
	let worker = paired["key"].as_str().unwrap().to_owned();
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/pair",
			None,
			Some(json!({"code":code}))
		)
		.await
		.0,
		StatusCode::UNAUTHORIZED
	);
	let other = pair(&app, &host, "alice", "other").await;
	let operation = enqueue(&app, &alice, "notes", 0, "first", Action::Status).await;
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/claim",
			Some(&alice),
			Some(json!({}))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/claim",
			Some(&"a".repeat(257)),
			Some(json!({}))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	let (status, job) = call(
		&app,
		"POST",
		"/api/v1/worker/claim",
		Some(&worker),
		Some(json!({})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(job["operation"]["id"], operation["id"]);
	assert!(
		job["analytics_ingest_key"]
			.as_str()
			.unwrap()
			.starts_with("alice.")
	);
	assert!(
		call(
			&app,
			"POST",
			"/api/v1/worker/claim",
			Some(&worker),
			Some(json!({}))
		)
		.await
		.1
		.is_null()
	);
	assert!(
		host.accounts()
			.installations("alice")
			.unwrap()
			.into_iter()
			.find(|installation| installation.project == "notes")
			.unwrap()
			.last_seen
			> 0
	);
	let finish = |succeeded: bool, details: Value| {
		json!({
			"id":operation["id"],
			"succeeded":succeeded,
			"message":"Checked",
			"details":details
		})
	};
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/finish",
			Some(&other),
			Some(finish(true, json!({})))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/finish",
			Some(&worker),
			Some(json!({"id":"missing","succeeded":true,"message":"x","details":{}}))
		)
		.await
		.0,
		StatusCode::NOT_FOUND
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/finish",
			Some(&worker),
			Some(json!({"id":operation["id"]}))
		)
		.await
		.0,
		StatusCode::UNPROCESSABLE_ENTITY
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/finish",
			Some(&worker),
			Some(finish(false, json!({"code":"checked"})))
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/finish",
			Some(&worker),
			Some(finish(false, json!({"code":"checked"})))
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/finish",
			Some(&worker),
			Some(finish(false, json!({"code":"different"})))
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/installations/notes/revoke",
			Some(&alice),
			None
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/installations/missing/revoke",
			Some(&alice),
			None
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/claim",
			Some(&worker),
			Some(json!({}))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
}

#[tokio::test]
async fn expired_pairing_grants_and_missing_installations_are_rejected() {
	let (dir, host, billing, _alice, _bob) = setup(true);
	let billing = billing.unwrap();
	activate(&billing, &host, "alice", "active", "price_control");
	host.register("alice", "notes", "production", Ownership::Customer)
		.unwrap();
	let app = host.router();
	let code = host.accounts().pairing("alice", "notes").unwrap();
	let db = rusqlite::Connection::open(dir.path().join("cloud/accounts.sqlite3")).unwrap();
	db.execute(
		"UPDATE hosted_installations SET pair_until=0 WHERE account='alice' AND project='notes'",
		[],
	)
	.unwrap();
	drop(db);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/pair",
			None,
			Some(json!({"code":code}))
		)
		.await
		.0,
		StatusCode::UNAUTHORIZED
	);
	drop(app);
	assert!(host.accounts().pairing("alice", "missing").is_err());
	assert!(host.accounts().revoke_worker("alice", "missing").is_err());
	assert!(host.accounts().view("missing").is_err());
	assert!(host.accounts().rotate("missing").is_err());
	assert!(host.accounts().enable_managed("missing", true).is_err());
	assert!(host.accounts().create("invalid/id", "cus_bad").is_err());
	assert!(host.accounts().create("third", "invalid").is_err());
	assert!(host.accounts().create("alice", "cus_other").is_err());
	assert!(host.accounts().installations("missing").unwrap().is_empty());
	assert!(
		host.register("bad/id", "notes", "production", Ownership::Customer)
			.is_err()
	);
	assert!(
		host.register("alice", "bad/id", "production", Ownership::Customer)
			.is_err()
	);
	assert!(
		host.register("alice", "notes", "production", Ownership::Customer)
			.is_err()
	);
}

#[tokio::test]
async fn billing_endpoints_degrade_without_configuration_and_reject_bad_webhooks() {
	let (_dir, host, _, alice, _bob) = setup(false);
	let app = host.router();
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/billing/checkout",
			Some(&alice),
			Some(json!({"plan":"control"}))
		)
		.await
		.0,
		StatusCode::SERVICE_UNAVAILABLE
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/webhooks/stripe",
			None,
			Some(json!({}))
		)
		.await
		.0,
		StatusCode::SERVICE_UNAVAILABLE
	);
	drop(app);
	let (_dir, host, billing, alice, _bob) = setup(true);
	let billing = billing.unwrap();
	activate(&billing, &host, "alice", "active", "price_control");
	let app = host.router();
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/billing/unknown",
			Some(&alice),
			Some(json!({}))
		)
		.await
		.0,
		StatusCode::NOT_FOUND
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/billing/checkout",
			Some(&alice),
			Some(json!({"plan":"unknown"}))
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/webhooks/stripe",
			None,
			Some(json!({}))
		)
		.await
		.0,
		StatusCode::BAD_REQUEST
	);
	assert_eq!(deliver(&app, json!({})).await.0, StatusCode::BAD_REQUEST);
	assert_eq!(
		deliver(
			&app,
			json!({"id":"evt_unknown","type":"customer.created","data":{}})
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		deliver(
			&app,
			json!({"id":"evt_unknown","type":"customer.created","data":{}})
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		deliver(
			&app,
			json!({"id":"evt_unrelated","type":"customer.subscription.updated","data":{"object":{"id":"sub_x"}}})
		)
		.await
		.0,
		StatusCode::BAD_REQUEST
	);
	assert_eq!(
		deliver(
			&app,
			json!({"id":"evt_other","type":"customer.subscription.updated","data":{"object":{"id":"sub_x","customer":"cus_missing"}}})
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		deliver(
			&app,
			json!({"id":"evt_checkout","type":"checkout.session.completed","data":{"object":{"customer":"cus_missing"}}})
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	let bytes = serde_json::to_vec(&json!({"id":"evt_stale"})).unwrap();
	let mut stale = request("POST", "/api/v1/webhooks/stripe", Some(bytes));
	stale
		.headers_mut()
		.insert("stripe-signature", "t=1,v1=invalid".parse().unwrap());
	assert_eq!(send(&app, stale).await.0, StatusCode::BAD_REQUEST);
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn state_endpoint_accepts_basic_and_bearer_credentials() {
	let (dir, host, billing, alice, _bob) = setup(true);
	let billing = billing.unwrap();
	activate(&billing, &host, "alice", "active", "price_team");
	for project in ["notes", "other"] {
		host.register("alice", project, "production", Ownership::Customer)
			.unwrap();
	}
	let app = host.router();
	let worker = pair(&app, &host, "alice", "notes").await;
	let path = "/api/v1/state/notes";
	assert_eq!(
		call(&app, "GET", path, None, None).await.0,
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		call(&app, "GET", path, Some(&alice), None).await.0,
		StatusCode::NOT_FOUND
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/state/missing", Some(&alice), None)
			.await
			.0,
		StatusCode::CONFLICT
	);
	let basic = |text: &str| {
		use base64::Engine as _;
		format!(
			"Basic {}",
			base64::engine::general_purpose::STANDARD.encode(text)
		)
	};
	for (header, expected) in [
		("Basic !!!".to_owned(), StatusCode::UNAUTHORIZED),
		(
			format!("Basic {}", {
				use base64::Engine as _;
				base64::engine::general_purpose::STANDARD.encode([0xff_u8, 0xfe])
			}),
			StatusCode::UNAUTHORIZED,
		),
		(basic("nocolon"), StatusCode::UNAUTHORIZED),
		(basic("ignored:missing"), StatusCode::FORBIDDEN),
	] {
		let mut attempt = request("GET", path, None);
		attempt
			.headers_mut()
			.insert("authorization", header.parse().unwrap());
		assert_eq!(send(&app, attempt).await.0, expected);
	}
	let mut attempt = request("GET", path, None);
	attempt.headers_mut().insert(
		"authorization",
		basic(&format!("ignored:{worker}")).parse().unwrap(),
	);
	assert_eq!(send(&app, attempt).await.0, StatusCode::NOT_FOUND);
	assert_eq!(
		call(
			&app,
			"LOCK",
			path,
			Some(&worker),
			Some(json!({"Operation":"OperationTypeApply"}))
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	let mut invalid = request("LOCK", path, Some(b"not json".to_vec()));
	invalid
		.headers_mut()
		.insert("authorization", format!("Bearer {worker}").parse().unwrap());
	assert_eq!(send(&app, invalid).await.0, StatusCode::CONFLICT);
	let lock = json!({"ID":"lock-1","Operation":"OperationTypeApply"});
	assert_eq!(
		call(
			&app,
			"LOCK",
			path,
			Some(&worker),
			Some(json!({"ID":"x".repeat(257)}))
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(&app, "LOCK", path, Some(&worker), Some(lock.clone()))
			.await
			.0,
		StatusCode::OK
	);
	assert_eq!(
		call(
			&app,
			"LOCK",
			path,
			Some(&worker),
			Some(json!({"ID":"other"}))
		)
		.await
		.0,
		StatusCode::LOCKED
	);
	let state = json!({"version":4,"serial":1,"lineage":"fixture","secret":"value"});
	assert_eq!(
		call(&app, "POST", path, Some(&worker), Some(state.clone()))
			.await
			.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			&format!("{path}?ID=lock-1"),
			Some(&worker),
			Some(state.clone())
		)
		.await
		.0,
		StatusCode::OK
	);
	assert_eq!(call(&app, "GET", path, Some(&alice), None).await.1, state);
	assert_eq!(
		call(&app, "PATCH", path, Some(&worker), None).await.0,
		StatusCode::METHOD_NOT_ALLOWED
	);
	assert_eq!(
		call(&app, "UNLOCK", path, Some(&worker), Some(lock.clone()))
			.await
			.0,
		StatusCode::OK
	);
	assert_eq!(
		call(&app, "UNLOCK", path, Some(&worker), Some(lock.clone()))
			.await
			.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(
			&app,
			"POST",
			&format!("{path}?ID=lock-1"),
			Some(&worker),
			Some(json!({"version":4,"serial":1,"lineage":"fixture","secret":"value"}))
		)
		.await
		.0,
		StatusCode::CONFLICT
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/state/other", Some(&worker), None)
			.await
			.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/state/other?ID=lock-1",
			Some(&worker),
			Some(state)
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	drop(app);
	assert!(host.state_read("alice", "missing").is_err());
	host.state_lock("alice", "notes", &lock, false).unwrap();
	assert!(host.state_lock("alice", "missing", &lock, false).is_err());
	assert!(
		host.state_lock(
			"alice",
			"notes",
			&json!({"Operation":"OperationTypeApply"}),
			false
		)
		.is_err()
	);
	assert!(
		host.state_lock("alice", "notes", &json!({"ID":"x".repeat(257)}), false)
			.is_err()
	);
	assert!(
		host.state_lock(
			"alice",
			"notes",
			&json!({"ID":"lock-2","padding":"x".repeat(17_000)}),
			false
		)
		.is_err()
	);
	assert!(
		host.state_write("alice", "missing", "lock-1", b"{}")
			.is_err()
	);
	assert!(
		host.state_write("alice", "notes", "lock-1", &vec![b'{'; 8 * 1024 * 1024 + 1])
			.is_err()
	);
	assert!(host.state_write("alice", "notes", "lock-1", b"{}").is_err());
	assert!(
		host.state_write("alice", "notes", "lock-1", br#"{"serial":1}"#)
			.is_err()
	);
	let state_bytes =
		serde_json::to_vec(&json!({"version":4,"serial":1,"lineage":"fixture","secret":"value"}))
			.unwrap();
	assert!(
		host.state_write("alice", "notes", "lock-1", &state_bytes)
			.is_ok()
	);
	assert!(
		host.state_write("alice", "notes", "lock-1", &state_bytes)
			.is_ok()
	);
	assert!(
		host.state_write(
			"alice",
			"notes",
			"lock-1",
			br#"{"serial":1,"lineage":"fixture","changed":true}"#
		)
		.is_err()
	);
	assert!(
		host.state_write(
			"alice",
			"notes",
			"lock-1",
			br#"{"serial":0,"lineage":"fixture"}"#
		)
		.is_err()
	);
	assert!(
		host.state_write(
			"alice",
			"notes",
			"lock-1",
			br#"{"serial":2,"lineage":"another"}"#
		)
		.is_err()
	);
	let db = rusqlite::Connection::open(dir.path().join("cloud/accounts.sqlite3")).unwrap();
	db.execute(
		"UPDATE hosted_states SET body=?1 WHERE account='alice' AND project='notes'",
		[vec![0_u8; 20]],
	)
	.unwrap();
	drop(db);
	assert!(host.state_read("alice", "notes").is_err());
	assert!(
		host.state_write(
			"alice",
			"notes",
			"lock-1",
			br#"{"serial":3,"lineage":"fixture"}"#
		)
		.is_err()
	);
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn proxy_enforces_tenant_tokens_and_operation_policy() {
	let (_dir, host, billing, alice, _bob) = setup(true);
	let billing = billing.unwrap();
	activate(&billing, &host, "alice", "active", "price_control");
	host.register("alice", "notes", "production", Ownership::Customer)
		.unwrap();
	let app = host.router();
	assert_eq!(
		call(&app, "GET", "/api/v1/overview", None, None).await.0,
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/overview", Some(&alice), None)
			.await
			.0,
		StatusCode::OK
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/overview", Some("alice"), None)
			.await
			.0,
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		call(
			&app,
			"GET",
			"/api/v1/overview",
			Some(&format!("unknown.{}", "a".repeat(64))),
			None
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(
			&app,
			"GET",
			"/api/v1/overview",
			Some(&format!("alice.{}", "a".repeat(64))),
			None
		)
		.await
		.0,
		StatusCode::UNAUTHORIZED
	);
	for path in ["/api/v1/login", "/api/v1/logout"] {
		assert_eq!(
			call(&app, "POST", path, Some(&alice), Some(json!({})))
				.await
				.0,
			StatusCode::NOT_FOUND,
			"{path}"
		);
	}
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/tokens",
			Some(&alice),
			Some(json!({"scope":"owner","project":"notes"}))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	let mut invalid = request("POST", "/api/v1/tokens", Some(b"not json".to_vec()));
	invalid
		.headers_mut()
		.insert("authorization", format!("Bearer {alice}").parse().unwrap());
	assert_eq!(send(&app, invalid).await.0, StatusCode::BAD_REQUEST);
	let (status, issued) = call(
		&app,
		"POST",
		"/api/v1/tokens",
		Some(&alice),
		Some(json!({"scope":"deploy","project":"notes"})),
	)
	.await;
	assert_eq!(status, StatusCode::OK);
	let agent = issued["key"].as_str().unwrap();
	assert!(agent.starts_with("alice."));
	assert_eq!(
		call(&app, "GET", "/api/v1/overview", Some(agent), None)
			.await
			.0,
		StatusCode::OK
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/account", Some(agent), None)
			.await
			.0,
		StatusCode::UNAUTHORIZED
	);
	let operation = enqueue(&app, &alice, "notes", 0, "proxy", Action::Status).await;
	assert_eq!(
		call(
			&app,
			"GET",
			&format!("/api/v1/operations/{}", operation["id"].as_str().unwrap()),
			Some(agent),
			None
		)
		.await
		.0,
		StatusCode::OK
	);
	let mut malformed = request(
		"POST",
		"/api/v1/projects/notes/operations",
		Some(b"not json".to_vec()),
	);
	malformed
		.headers_mut()
		.insert("authorization", format!("Bearer {alice}").parse().unwrap());
	assert_eq!(send(&app, malformed).await.0, StatusCode::BAD_REQUEST);
	let mut missing = request(
		"POST",
		"/api/v1/projects/notes/operations",
		Some(br#"{"expected_revision":1}"#.to_vec()),
	);
	missing
		.headers_mut()
		.insert("authorization", format!("Bearer {alice}").parse().unwrap());
	assert_eq!(send(&app, missing).await.0, StatusCode::BAD_REQUEST);
	let mut oversized = request(
		"POST",
		"/api/v1/projects/notes/operations",
		Some(vec![b' '; 70_000]),
	);
	oversized
		.headers_mut()
		.insert("authorization", format!("Bearer {alice}").parse().unwrap());
	assert_eq!(send(&app, oversized).await.0, StatusCode::BAD_REQUEST);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/operations",
			Some(&alice),
			Some(json!(Intent {
				expected_revision: 1,
				idempotency_key: "wrong-path".into(),
				action: Action::Status,
			}))
		)
		.await
		.0,
		StatusCode::BAD_REQUEST
	);
	let mut revoke_invalid = request("POST", "/api/v1/tokens/revoke", Some(b"not json".to_vec()));
	revoke_invalid
		.headers_mut()
		.insert("authorization", format!("Bearer {alice}").parse().unwrap());
	assert_eq!(send(&app, revoke_invalid).await.0, StatusCode::BAD_REQUEST);
	let mut revoke_oversized = request("POST", "/api/v1/tokens/revoke", Some(vec![b' '; 5_000]));
	revoke_oversized
		.headers_mut()
		.insert("authorization", format!("Bearer {alice}").parse().unwrap());
	assert_eq!(
		send(&app, revoke_oversized).await.0,
		StatusCode::BAD_REQUEST
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/tokens/revoke",
			Some(&alice),
			Some(json!({"key":agent}))
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/overview", Some(agent), None)
			.await
			.0,
		StatusCode::UNAUTHORIZED
	);
	assert_eq!(
		call(&app, "GET", "/api/v1/unknown", Some(&alice), None)
			.await
			.0,
		StatusCode::NOT_FOUND
	);
	let worker = pair(&app, &host, "alice", "notes").await;
	let (_, job) = call(
		&app,
		"POST",
		"/api/v1/worker/claim",
		Some(&worker),
		Some(json!({})),
	)
	.await;
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/worker/finish",
			Some(&worker),
			Some(json!({
				"id":job["operation"]["id"],
				"succeeded":true,
				"message":"Checked",
				"details":{}
			}))
		)
		.await
		.0,
		StatusCode::NO_CONTENT
	);
	activate(&billing, &host, "alice", "past_due", "price_control");
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/projects/notes/operations",
			Some(&alice),
			Some(json!(Intent {
				expected_revision: 1,
				idempotency_key: "denied-deploy".into(),
				action: Action::Deploy {
					version: "v1".into(),
					setup: false,
					maintenance: false,
				},
			}))
		)
		.await
		.0,
		StatusCode::FORBIDDEN
	);
	assert_eq!(
		call(
			&app,
			"POST",
			"/api/v1/projects/notes/operations",
			Some(&alice),
			Some(json!(Intent {
				expected_revision: 1,
				idempotency_key: "allowed-status".into(),
				action: Action::Status,
			}))
		)
		.await
		.0,
		StatusCode::OK
	);
}

#[tokio::test]
async fn claims_finish_operations_when_the_subscription_or_capacity_changed() {
	let (_dir, host, billing, alice, _bob) = setup(true);
	let billing = billing.unwrap();
	activate(&billing, &host, "alice", "active", "price_team");
	for (project, ownership) in [
		("notes", Ownership::Customer),
		("managed", Ownership::LeptoskitDigitalocean),
	] {
		host.register("alice", project, "production", ownership)
			.unwrap();
	}
	let app = host.router();
	let worker = pair(&app, &host, "alice", "notes").await;
	let managed = {
		let code = host.accounts().pairing("alice", "managed").unwrap();
		let (status, value) = call(
			&app,
			"POST",
			"/api/v1/worker/pair",
			None,
			Some(json!({"code":code})),
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		value["key"].as_str().unwrap().to_owned()
	};
	let deploy = enqueue(
		&app,
		&alice,
		"notes",
		0,
		"deploy",
		Action::Deploy {
			version: "v1".into(),
			setup: false,
			maintenance: false,
		},
	)
	.await;
	activate(&billing, &host, "alice", "past_due", "price_team");
	assert!(
		call(
			&app,
			"POST",
			"/api/v1/worker/claim",
			Some(&worker),
			Some(json!({}))
		)
		.await
		.1
		.is_null()
	);
	let (_, finished) = call(
		&app,
		"GET",
		&format!("/api/v1/operations/{}", deploy["id"].as_str().unwrap()),
		Some(&alice),
		None,
	)
	.await;
	assert_eq!(finished["phase"], "failed");
	assert_eq!(finished["result"]["code"], "policy_changed");
	enqueue(&app, &alice, "notes", 1, "status", Action::Status).await;
	let (_, job) = call(
		&app,
		"POST",
		"/api/v1/worker/claim",
		Some(&worker),
		Some(json!({})),
	)
	.await;
	assert!(!job.is_null());
	activate(&billing, &host, "alice", "active", "price_team");
	host.accounts().enable_managed("alice", true).unwrap();
	let revision = revision(&app, &alice, "managed").await;
	let first = enqueue(
		&app,
		&alice,
		"managed",
		revision,
		"managed-1",
		Action::Status,
	)
	.await;
	host.accounts().enable_managed("alice", false).unwrap();
	assert!(
		call(
			&app,
			"POST",
			"/api/v1/worker/claim",
			Some(&managed),
			Some(json!({}))
		)
		.await
		.1
		.is_null()
	);
	let (_, finished) = call(
		&app,
		"GET",
		&format!("/api/v1/operations/{}", first["id"].as_str().unwrap()),
		Some(&alice),
		None,
	)
	.await;
	assert_eq!(finished["result"]["code"], "policy_changed");
	host.accounts().enable_managed("alice", true).unwrap();
	enqueue(
		&app,
		&alice,
		"managed",
		revision + 1,
		"managed-2",
		Action::Status,
	)
	.await;
	let (_, job) = call(
		&app,
		"POST",
		"/api/v1/worker/claim",
		Some(&managed),
		Some(json!({})),
	)
	.await;
	assert!(!job.is_null());
}

#[tokio::test]
async fn operator_reconciliation_requires_a_stopped_worker() {
	let (dir, host, billing, alice, _bob) = setup(true);
	let billing = billing.unwrap();
	activate(&billing, &host, "alice", "active", "price_control");
	host.register("alice", "notes", "production", Ownership::Customer)
		.unwrap();
	let app = host.router();
	let worker = pair(&app, &host, "alice", "notes").await;
	let operation = enqueue(&app, &alice, "notes", 0, "reconcile", Action::Status).await;
	assert!(
		host.interrupt("alice", operation["id"].as_str().unwrap())
			.is_err()
	);
	assert!(host.interrupt("alice", "missing").is_err());
	let (_, claimed) = call(
		&app,
		"POST",
		"/api/v1/worker/claim",
		Some(&worker),
		Some(json!({})),
	)
	.await;
	assert_eq!(claimed["operation"]["phase"], "running");
	host.accounts().revoke_worker("alice", "notes").unwrap();
	host.interrupt("alice", operation["id"].as_str().unwrap())
		.unwrap();
	let (_, finished) = call(
		&app,
		"GET",
		&format!("/api/v1/operations/{}", operation["id"].as_str().unwrap()),
		Some(&alice),
		None,
	)
	.await;
	assert_eq!(finished["phase"], "interrupted");
	assert!(
		host.interrupt("alice", operation["id"].as_str().unwrap())
			.is_err()
	);
	assert!(host.backup(&dir.path().join("cloud")).is_err());
	assert!(
		Hosted::open(
			&dir.path().join("cloud"),
			"short".into(),
			"http://localhost:4701",
			None
		)
		.is_err()
	);
	assert!(
		Hosted::open(
			&dir.path().join("cloud"),
			"s".repeat(300),
			"http://localhost:4701",
			None
		)
		.is_err()
	);
	assert!(
		Hosted::open(
			&dir.path().join("cloud"),
			SECRET.into(),
			"http://deploy.example.com",
			None
		)
		.is_err()
	);
	assert!(
		host.accounts()
			.installations("alice")
			.unwrap()
			.iter()
			.any(|installation| installation.project == "notes")
	);
}
