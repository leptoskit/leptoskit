//! Each store client against a local server that answers as the store does,
//! from its documentation, and records what it was sent.

use std::sync::Arc;
use std::sync::Mutex;

use axum::body::Bytes;
use axum::http::HeaderMap;
use axum::http::Method;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::response::IntoResponse;
use axum::response::Response;

use super::app_store::AppStore;
use super::app_store::Key;
use super::chrome::Chrome;
use super::edge::Edge;
use super::firefox::Firefox;
use super::http::Http;
use super::secrets::Secret;

/// One request the fake store received.
#[derive(Clone, Debug)]
struct Seen {
	method: String,
	path: String,
	query: String,
	authorization: String,
	content_type: String,
	body: Vec<u8>,
}

type Answer = Box<dyn Fn(&Seen, usize) -> Response + Send + Sync>;

struct StoreServer {
	requests: Arc<Mutex<Vec<Seen>>>,
	shutdown: Option<tokio::sync::oneshot::Sender<()>>,
	thread: Option<std::thread::JoinHandle<()>>,
}

impl std::ops::Deref for StoreServer {
	type Target = Mutex<Vec<Seen>>;

	fn deref(&self) -> &Self::Target {
		&self.requests
	}
}

impl Drop for StoreServer {
	fn drop(&mut self) {
		let _ = self.shutdown.take().unwrap().send(());
		let result = self.thread.take().unwrap().join();
		if !std::thread::panicking() {
			result.unwrap();
		}
	}
}

/// A store on loopback: `answer` gets each request and how many came
/// before it. Dropping the server stops and joins its thread.
fn store(answer: Answer) -> (String, StoreServer) {
	let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
	let recorded = Arc::clone(&seen);
	let answer = Arc::new(answer);
	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let address = listener.local_addr().unwrap();
	listener.set_nonblocking(true).unwrap();
	let (shutdown, stopped) = tokio::sync::oneshot::channel();
	let thread = std::thread::spawn(move || {
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap();
		runtime.block_on(async move {
			let listener = tokio::net::TcpListener::from_std(listener).unwrap();
			let router = axum::Router::new().fallback(
				move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
					let recorded = Arc::clone(&recorded);
					let answer = Arc::clone(&answer);
					async move {
						let header = |name: &str| {
							headers
								.get(name)
								.and_then(|value| value.to_str().ok())
								.unwrap_or_default()
								.to_owned()
						};
						let request = Seen {
							method: method.to_string(),
							path: uri.path().to_owned(),
							query: uri.query().unwrap_or_default().to_owned(),
							authorization: header("authorization"),
							content_type: header("content-type"),
							body: body.to_vec(),
						};
						let count = {
							let mut seen = recorded.lock().unwrap();
							seen.push(request.clone());
							seen.len() - 1
						};
						answer(&request, count)
					}
				},
			);
			tokio::select! {
				() = async { axum::serve(listener, router).await.unwrap(); } => {},
				_ = stopped => {},
			}
		});
	});
	(
		format!("http://{address}"),
		StoreServer {
			requests: seen,
			shutdown: Some(shutdown),
			thread: Some(thread),
		},
	)
}

fn json(value: serde_json::Value) -> Response {
	axum::Json(value).into_response()
}

/// A fresh RSA key in PKCS#8 PEM, so no private key is ever committed.
fn rsa_key() -> String {
	let program = std::env::var_os("LEPTOSKIT_TEST_OPENSSL").unwrap_or_else(|| "openssl".into());
	let output = std::process::Command::new(program)
		.args([
			"genpkey",
			"-algorithm",
			"RSA",
			"-pkeyopt",
			"rsa_keygen_bits:2048",
		])
		.output()
		.expect("publish tests require OpenSSL; set LEPTOSKIT_TEST_OPENSSL to its executable");
	assert!(
		output.status.success(),
		"test RSA key generation failed: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	String::from_utf8(output.stdout).unwrap()
}

#[test]
fn chrome_signs_in_uploads_and_submits() {
	let key = rsa_key();
	let (base, seen) = store(Box::new(|request, _| {
		match request.path.as_str() {
			"/token" => json(serde_json::json!({ "access_token": "token-1", "expires_in": 3600 })),
			path if path.ends_with(":upload") => {
				json(serde_json::json!({ "uploadState": "SUCCEEDED", "crxVersion": "1.4.0" }))
			}
			path if path.ends_with(":publish") => {
				json(serde_json::json!({ "state": "PENDING_REVIEW" }))
			}

			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let account = Secret::new(serde_json::json!({ "client_email": "ci@project.iam.gserviceaccount.com", "private_key": key }).to_string());
	let http = Http::default();
	let chrome = Chrome::connect(
		&http,
		&base,
		&format!("{base}/token"),
		&account,
		"pub-1",
		"abcdefghijklmnopabcdefghijklmnop",
	)
	.unwrap();
	assert_eq!(chrome.upload(b"PK zip".to_vec()).unwrap(), "1.4.0");
	assert_eq!(chrome.submit(false).unwrap(), "PENDING_REVIEW");

	let seen = seen.lock().unwrap();
	let form = String::from_utf8(seen[0].body.clone()).unwrap();
	assert!(form.starts_with(
		"grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion="
	));
	let assertion = form.rsplit('=').next().unwrap();
	assert_eq!(assertion.split('.').count(), 3, "a signed JWT");
	assert_eq!(
		seen[1].path,
		"/upload/v2/publishers/pub-1/items/abcdefghijklmnopabcdefghijklmnop:upload"
	);
	assert_eq!(seen[1].authorization, "Bearer token-1");
	assert_eq!(seen[1].content_type, "application/zip");

	assert_eq!(seen[1].body, b"PK zip");
	assert_eq!(
		seen[2].path,
		"/v2/publishers/pub-1/items/abcdefghijklmnopabcdefghijklmnop:publish"
	);
	let publish: serde_json::Value = serde_json::from_slice(&seen[2].body).unwrap();
	assert_eq!(publish["publishType"], "DEFAULT_PUBLISH");
}

#[test]
fn chrome_reports_a_refused_package() {
	let key = rsa_key();
	let (base, _server) = store(Box::new(|request, _| {
		match request.path.as_str() {
			"/token" => json(serde_json::json!({ "access_token": "t" })),
			_ => (StatusCode::BAD_REQUEST, "manifest version must be newer").into_response(),
		}
	}));
	let account =
		Secret::new(serde_json::json!({ "client_email": "a@b", "private_key": key }).to_string());
	let http = Http::default();
	let chrome = Chrome::connect(
		&http,
		&base,
		&format!("{base}/token"),
		&account,
		"p",
		"abcdefghijklmnopabcdefghijklmnop",
	)
	.unwrap();
	let error = chrome.upload(b"zip".to_vec()).unwrap_err().to_string();
	assert!(
		error.contains("400") && error.contains("must be newer"),
		"{error}"
	);
}

#[test]
fn edge_uploads_waits_for_its_operations_and_submits() {
	let (base, seen) = store(Box::new(|request, count| {
		let accepted = |location: &str| {
			let mut response = StatusCode::ACCEPTED.into_response();
			response
				.headers_mut()
				.insert("location", location.parse().unwrap());
			response
		};
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/v1/products/prod-1/submissions/draft/package") => accepted("op-upload"),
			("GET", "/v1/products/prod-1/submissions/draft/package/operations/op-upload") => {
				// In progress once, then done.
				json(
					serde_json::json!({ "status": if count < 2 { "InProgress" } else { "Succeeded" } }),
				)
			}
			("POST", "/v1/products/prod-1/submissions") => {
				accepted("/v1/products/prod-1/submissions/operations/op-submit")
			}
			("GET", "/v1/products/prod-1/submissions/operations/op-submit") => {
				json(serde_json::json!({ "status": "Succeeded" }))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let edge = Edge::new(
		&http,
		&base,
		"prod-1",
		Secret::new("client".into()),
		Secret::new("key".into()),
	);
	edge.upload(b"zip".to_vec()).unwrap();
	edge.submit("Version 1.4.0.").unwrap();
	let seen = seen.lock().unwrap();
	assert_eq!(seen[0].authorization, "ApiKey key");
	assert_eq!(seen[0].content_type, "application/zip");

	assert_eq!(
		seen.iter()
			.filter(|request| request.path.ends_with("op-upload"))
			.count(),
		2
	);
	let notes: serde_json::Value =
		serde_json::from_slice(&seen.last().unwrap().body.clone()).unwrap_or_default();
	assert!(notes.is_null() || notes.is_object());
}

#[test]
fn edge_explains_an_expired_key() {
	let (base, _server) = store(Box::new(|_, _| StatusCode::UNAUTHORIZED.into_response()));
	let http = Http::default();
	let edge = Edge::new(
		&http,
		&base,
		"p",
		Secret::new("c".into()),
		Secret::new("old".into()),
	);
	let error = edge.upload(b"zip".to_vec()).unwrap_err().to_string();
	assert!(error.contains("72 days"), "{error}");
}

#[test]
fn firefox_validates_then_submits_with_the_source() {
	let (base, seen) = store(Box::new(|request, count| {
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/addons/upload/") => {
				json(serde_json::json!({ "uuid": "u-1", "processed": false }))
			}
			("GET", "/addons/upload/u-1/") => {
				json(serde_json::json!({ "uuid": "u-1", "processed": count > 1, "valid": true }))
			}
			("POST", "/addons/addon/notes@example.com/versions/") => {
				json(serde_json::json!({ "id": 7, "version": "1.4.0" }))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let firefox = Firefox::new(
		&http,
		&base,
		"notes@example.com",
		Secret::new("user:1".into()),
		Secret::new("secret".into()),
	);
	let upload = firefox.upload(b"PK package", "listed").unwrap();
	assert_eq!(upload, "u-1");
	let version = firefox.submit(&upload, b"PK source").unwrap();
	assert_eq!(version["version"], "1.4.0");

	let seen = seen.lock().unwrap();
	assert!(seen[0].authorization.starts_with("JWT "));
	assert!(
		seen[0]
			.content_type
			.starts_with("multipart/form-data; boundary=")
	);
	let upload_body = String::from_utf8_lossy(&seen[0].body);
	assert!(upload_body.contains("name=\"channel\"\r\n\r\nlisted"));
	assert!(upload_body.contains("PK package"));
	let version_body = String::from_utf8_lossy(&seen.last().unwrap().body);
	assert!(version_body.contains("name=\"upload\"\r\n\r\nu-1"));

	assert!(version_body.contains("filename=\"firefox-source.zip\""));
	assert!(version_body.contains("PK source"));
}

#[test]
fn firefox_reports_failed_validation_and_a_missing_add_on() {
	let (base, _server) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/addons/upload/") => json(serde_json::json!({ "uuid": "u-2" })),
			("GET", "/addons/upload/u-2/") => {
				json(serde_json::json!({
					"processed": true, "valid": false,
					"validation": { "messages": [{ "type": "error", "message": "manifest.json is invalid" }] }
				}))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let firefox = Firefox::new(
		&http,
		&base,
		"new@example.com",
		Secret::new("i".into()),
		Secret::new("s".into()),
	);
	let error = firefox.upload(b"zip", "listed").unwrap_err().to_string();
	assert!(error.contains("manifest.json is invalid"), "{error}");
	let error = firefox.submit("u-2", b"source").unwrap_err().to_string();
	assert!(
		error.contains("has no add-on new@example.com yet"),
		"{error}"
	);
}

fn apple_key() -> Option<Key> {
	let pem = super::secrets::tests::ec_key()?;
	Some(
		Key::new(
			Secret::new("issuer-1".into()),
			Secret::new("KEY123".into()),
			&Secret::new(pem),
		)
		.unwrap(),
	)
}

/// The claims of a bearer token.
fn claims(authorization: &str) -> serde_json::Value {
	use base64::Engine as _;

	let token = authorization.strip_prefix("Bearer ").unwrap();
	let claims = token.split('.').nth(1).unwrap();
	serde_json::from_slice(
		&base64::engine::general_purpose::URL_SAFE_NO_PAD
			.decode(claims)
			.unwrap(),
	)
	.unwrap()
}

#[test]
fn app_store_numbers_the_next_build_and_waits_for_processing() {
	let Some(key) = apple_key() else { return };
	let (base, seen) = store(Box::new(|request, count| {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/v1/builds") if request.query.contains("sort=-uploadedDate") => {
				json(serde_json::json!({ "data": [
					{ "id": "b1", "attributes": { "version": "7" } },
					{ "id": "b2", "attributes": { "version": "41" } },
					{ "id": "b3", "attributes": { "version": "1.0.3" } },
				]}))
			}

			("GET", "/v1/builds") => {
				// Not listed yet, then processing, then done.
				match count {
					1 => json(serde_json::json!({ "data": [] })),
					2 => {
						json(serde_json::json!({ "data": [
							{ "id": "b4", "attributes": { "processingState": "PROCESSING" } },
						]}))
					}
					_ => {
						json(serde_json::json!({ "data": [
							{ "id": "b4", "attributes": { "processingState": "VALID" } },
						]}))
					}
				}
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let client = AppStore::new(&http, &base, &key);
	assert_eq!(client.highest_build("123").unwrap(), Some(41));
	assert_eq!(client.wait_for_build("123", "1.4.0", 42).unwrap(), "b4");

	let seen = seen.lock().unwrap();
	assert_eq!(
		seen[0].query,
		"filter%5Bapp%5D=123&sort=-uploadedDate&limit=200&fields%5Bbuilds%5D=version"
	);
	assert_eq!(
		seen[1].query,
		"filter%5Bapp%5D=123&filter%5Bversion%5D=42&filter%5BpreReleaseVersion.version%5D=1.4.0&\
		 limit=1"
	);
	let claims = claims(&seen[0].authorization);
	assert_eq!(claims["iss"], "issuer-1");
	assert_eq!(claims["aud"], "appstoreconnect-v1");
	assert_eq!(
		claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap(),
		1200
	);
}

#[test]
fn app_store_reports_a_build_that_failed_processing() {
	let Some(key) = apple_key() else { return };
	let (base, _server) = store(Box::new(|_, _| {
		json(serde_json::json!({ "data": [
			{ "id": "b4", "attributes": { "processingState": "INVALID" } },
		]}))
	}));
	let http = Http::default();
	let error = AppStore::new(&http, &base, &key)
		.wait_for_build("123", "1.4.0", 42)
		.unwrap_err()
		.to_string();
	assert!(error.contains("couldn't process build 42"), "{error}");
}

#[test]
fn app_store_adds_builds_to_testflight_groups() {
	let Some(key) = apple_key() else { return };
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/v1/apps/123/betaGroups") => {
				json(serde_json::json!({ "data": [
					{ "id": "g1", "attributes": { "name": "Team", "isInternalGroup": true, "hasAccessToAllBuilds": true } },
					{ "id": "g2", "attributes": { "name": "QA", "isInternalGroup": true, "hasAccessToAllBuilds": false } },
					{ "id": "g3", "attributes": { "name": "Beta", "isInternalGroup": false } },
				]}))
			}
			("POST", "/v1/builds/b4/relationships/betaGroups") => {
				StatusCode::NO_CONTENT.into_response()
			}
			("POST", "/v1/betaAppReviewSubmissions") => {
				(StatusCode::CONFLICT, "Missing export compliance").into_response()
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let client = AppStore::new(&http, &base, &key);
	let names = ["Team", "QA", "Beta"].map(str::to_owned);
	let groups = client.add_to_groups("123", "b4", &names).unwrap();
	assert_eq!(groups.len(), 3);
	let error = client.submit_for_beta_review("b4").unwrap_err().to_string();

	assert!(error.contains("Missing export compliance"), "{error}");
	let error = client
		.add_to_groups("123", "b4", &["Friends".to_owned()])
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("no TestFlight group `Friends`; it has `Team`, `QA`, `Beta`"),
		"{error}"
	);

	let seen = seen.lock().unwrap();
	let added: serde_json::Value = serde_json::from_slice(&seen[1].body).unwrap();
	// The group that gets every build already has it.
	assert_eq!(
		added,
		serde_json::json!({ "data": [
			{ "type": "betaGroups", "id": "g2" },
			{ "type": "betaGroups", "id": "g3" },
		]})
	);
	let review: serde_json::Value = serde_json::from_slice(&seen[2].body).unwrap();
	assert_eq!(review["data"]["relationships"]["build"]["data"]["id"], "b4");
}

#[test]
fn app_store_submits_a_version_for_review() {
	let Some(key) = apple_key() else { return };
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/v1/apps/123/appStoreVersions") => {
				json(serde_json::json!({ "data": [
					{ "id": "v1", "attributes": { "versionString": "1.0.0", "appStoreState": "READY_FOR_SALE" } },
					{ "id": "v2", "attributes": { "versionString": "1.1.0", "appStoreState": "PREPARE_FOR_SUBMISSION" } },
				]}))
			}
			("PATCH", "/v1/appStoreVersions/v2") => {
				json(serde_json::json!({ "data": { "id": "v2" } }))
			}
			("GET", "/v1/appStoreVersions/v2/appStoreVersionLocalizations") => {
				json(serde_json::json!({ "data": [
					{ "id": "l1", "attributes": { "locale": "en-US" } },
				]}))
			}
			("PATCH", "/v1/appStoreVersionLocalizations/l1") => json(serde_json::json!({})),
			("PATCH", "/v1/appStoreVersions/v2/relationships/build") => {
				StatusCode::NO_CONTENT.into_response()
			}
			("GET", "/v1/reviewSubmissions") => json(serde_json::json!({ "data": [] })),
			("POST", "/v1/reviewSubmissions") => {
				json(serde_json::json!({ "data": { "id": "s1" } }))
			}
			("POST", "/v1/reviewSubmissionItems") => {
				(
					StatusCode::CREATED,
					axum::Json(serde_json::json!({ "data": { "id": "i1" } })),
				)
					.into_response()
			}
			("PATCH", "/v1/reviewSubmissions/s1") => {
				json(
					serde_json::json!({ "data": { "id": "s1", "attributes": { "state": "WAITING_FOR_REVIEW" } } }),
				)
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let client = AppStore::new(&http, &base, &key);
	// The version in preparation takes the new number.
	let (version, first) = client.version("123", "1.2.0").unwrap();
	assert_eq!((version.as_str(), first), ("v2", false));
	client
		.localize(
			&version,
			"en-US",
			&serde_json::json!({ "whatsNew": "Clip from the side panel." }),
		)
		.unwrap();
	client.attach(&version, "b4").unwrap();

	assert_eq!(
		client.submit_for_review("123", &version).unwrap(),
		"waiting for review"
	);

	let seen = seen.lock().unwrap();
	let body =
		|index: usize| -> serde_json::Value { serde_json::from_slice(&seen[index].body).unwrap() };
	assert_eq!(seen[0].query, "filter%5Bplatform%5D=IOS&limit=200");
	assert_eq!(body(1)["data"]["attributes"]["versionString"], "1.2.0");
	assert_eq!(
		body(3)["data"]["attributes"]["whatsNew"],
		"Clip from the side panel."
	);
	assert_eq!(
		body(4),
		serde_json::json!({ "data": { "type": "builds", "id": "b4" } })
	);
	assert_eq!(
		seen[5].query,
		"filter%5Bapp%5D=123&filter%5Bplatform%5D=IOS&filter%5Bstate%5D=READY_FOR_REVIEW"
	);
	assert_eq!(body(6)["data"]["attributes"]["platform"], "IOS");

	let item = body(7);
	assert_eq!(
		item["data"]["relationships"]["reviewSubmission"]["data"]["id"],
		"s1"
	);
	assert_eq!(
		item["data"]["relationships"]["appStoreVersion"]["data"]["id"],
		"v2"
	);
	assert_eq!(body(8)["data"]["attributes"]["submitted"], true);
}

#[test]
fn app_store_creates_a_first_version_and_refuses_a_released_one() {
	let Some(key) = apple_key() else { return };
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/v1/apps/1/appStoreVersions") => json(serde_json::json!({ "data": [] })),
			("POST", "/v1/appStoreVersions") => json(serde_json::json!({ "data": { "id": "v9" } })),
			("GET", "/v1/apps/2/appStoreVersions") => {
				json(serde_json::json!({ "data": [
					{ "id": "v1", "attributes": { "versionString": "1.0.0", "appStoreState": "READY_FOR_SALE" } },
				]}))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let client = AppStore::new(&http, &base, &key);
	assert_eq!(
		client.version("1", "1.0.0").unwrap(),
		("v9".to_owned(), true)
	);
	let created: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[1].body).unwrap();
	assert_eq!(created["data"]["attributes"]["platform"], "IOS");
	assert_eq!(created["data"]["relationships"]["app"]["data"]["id"], "1");

	let error = client.version("2", "1.0.0").unwrap_err().to_string();
	assert!(error.contains("already ready for sale"), "{error}");
}

#[test]
fn app_store_explains_a_refused_key() {
	let Some(key) = apple_key() else { return };
	let (base, _server) = store(Box::new(|_, _| StatusCode::UNAUTHORIZED.into_response()));
	let http = Http::default();
	let error = AppStore::new(&http, &base, &key)
		.highest_build("1")
		.unwrap_err()
		.to_string();
	assert!(error.contains("LEPTOSKIT_PUBLISH_APPLE_KEY_ID"), "{error}");
}

/// Every credential, set to a value no output may contain.
#[test]
fn no_credential_value_reaches_any_output() {
	use std::fmt::Write as _;

	let dir = tempfile::tempdir().unwrap();
	let project = tempfile::tempdir().unwrap();
	let file = dir.path().join("credentials.toml");
	let value = |name: &str| format!("value-of-{name}");
	let mut text = String::new();
	let mut names: Vec<&str> = super::NEEDED.iter().map(|needed| needed.name).collect();
	names.sort_unstable();
	names.dedup();

	for name in names {
		let _ = writeln!(text, "{name} = \"{}\"", value(name));
	}

	std::fs::write(&file, text).unwrap();
	let credentials = super::Credentials::load(Some(&file), project.path()).unwrap();
	let stores: Vec<&str> = leptoskit_config::publish::STORES.to_vec();

	let mut outputs = vec![
		super::list_credentials(&stores, &credentials),
		format!("{credentials:?}"),
	];

	for needed in &super::NEEDED {
		let secret = credentials.require(needed.name, needed.how).unwrap();
		outputs.push(format!("{secret} {secret:?}"));
	}

	// The stores' refusals, and keys that can't be used.
	let (base, _server) = store(Box::new(|_, _| StatusCode::UNAUTHORIZED.into_response()));
	let http = Http::default();
	let get = |name: &str| credentials.get(name).unwrap();
	let edge = Edge::new(
		&http,
		&base,
		"p",
		get(super::edge::CLIENT_ID),
		get(super::edge::API_KEY),
	);

	outputs.push(format!("{:#}", edge.upload(b"zip".to_vec()).unwrap_err()));
	let firefox = Firefox::new(
		&http,
		&base,
		"a@b",
		get(super::firefox::ISSUER),
		get(super::firefox::SECRET),
	);
	outputs.push(format!(
		"{:#}",
		firefox.upload(b"zip", "listed").unwrap_err()
	));
	outputs.push(format!(
		"{:#}",
		Chrome::connect(
			&http,
			&base,
			&base,
			&get(super::chrome::CREDENTIAL),
			"p",
			"i"
		)
		.err()
		.unwrap()
	));
	let apple = Key::new(
		get(super::app_store::ISSUER),
		get(super::app_store::KEY_ID),
		&get(super::app_store::KEY),
	);
	outputs.push(format!("{:#}", apple.err().unwrap()));

	for output in &outputs {
		for needed in &super::NEEDED {
			assert!(
				!output.contains(&value(needed.name)),
				"{} leaked into: {output}",
				needed.name
			);
		}
	}

	assert!(outputs[0].contains("LEPTOSKIT_PUBLISH_APPLE_KEY "));
}

/// `leptoskit publish --dry-run` on Commonplace: every store's checks and
/// remaining steps, from its real configuration and `store/` folder, with
/// the packages standing in for a build.
#[test]
fn commonplace_dry_run() {
	let mut settings = insta::Settings::clone_current();
	settings.add_filter(r"\\", "/");
	let _guard = settings.bind_to_scope();
	let out = tempfile::tempdir().unwrap();
	let report = commonplace(out.path(), &[], true).unwrap();
	assert!(
		out.path()
			.join(".leptoskit/publish/ios/ExportOptions.plist")
			.is_file()
	);
	insta::assert_snapshot!(report.text());
}

/// Publishes Commonplace's configuration and `store/` text from `out`, where
/// the packages stand in for a build, to `stores` (every configured one when
/// empty). Asserts that no tool ran.
fn commonplace(
	out: &std::path::Path,
	stores: &[&str],
	dry_run: bool,
) -> anyhow::Result<super::Report> {
	use leptoskit_config::Browser;

	let example =
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/commonplace");
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		&std::fs::read_to_string(example.join("leptoskit.toml")).unwrap(),
		&example,
	)
	.unwrap();
	let store = super::listing::Store::load(&example).unwrap();
	let extension_config = config.extension.clone().unwrap();
	let manifest =
		crate::extension::manifest::manifest(&crate::extension::manifest::ManifestInput {
			browser: Browser::Chromium,
			config: &config,
			extension: &extension_config,
			plugin_permissions: &[],
			icons: &[16, 32, 48, 128],
			dev: None,
		});
	// Commonplace's icon, which Google Play's listing gets.
	let icon = config.app.icon.clone().unwrap();
	std::fs::create_dir_all(out.join(&icon).parent().unwrap()).unwrap();
	std::fs::copy(example.join(&icon), out.join(&icon)).unwrap();
	// And its screenshot plan, unless the test brought one.
	let plan = out.join("store/screenshots/screenshots.toml");

	if !plan.exists() {
		std::fs::create_dir_all(plan.parent().unwrap()).unwrap();
		std::fs::copy(example.join("store/screenshots/screenshots.toml"), &plan).unwrap();
	}

	// Commonplace's iOS host is `ios/`, as in the example.
	std::fs::create_dir_all(out.join("ios")).unwrap();
	std::fs::write(out.join("ios/Cargo.toml"), "").unwrap();
	// And its Android host is `android/`, with its own Gradle script.
	std::fs::create_dir_all(out.join("android/app")).unwrap();
	std::fs::write(out.join("android/Cargo.toml"), "").unwrap();
	std::fs::copy(
		example.join("android/app/build.gradle.kts"),
		out.join("android/app/build.gradle.kts"),
	)
	.unwrap();
	let package = |browser: Browser| {
		crate::extension::Package {
			browser,
			dir: out.join(browser.name()),
			zip: Some(out.join(format!("{}.zip", browser.name()))),
		}
	};
	let extension = super::Extension {
		built: crate::extension::Built {
			packages: vec![package(Browser::Chromium), package(Browser::Firefox)],
			source: Some(out.join("firefox-source.zip")),
		},
		manifest,
	};
	let args = super::PublishArgs {
		stores: Vec::new(),
		dry_run,
		submit: false,
		staged: false,
		yes: false,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Auto,
	};
	let credentials = super::Credentials::default();
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let session = super::Session {
		args: &args,
		root: out,
		config: &config,
		credentials: &credentials,
		store: &store,
		http: &http,
		runner: &runner,
		extension: Some(&extension),
		name: super::display_name(&config),
		version: super::apple_version(&config),
		submit: false,
	};
	let mut report = super::Report::default();
	let configured = config.publish.stores();
	let stores = if stores.is_empty() {
		configured
	} else {
		stores.to_vec()
	};

	let published = stores.iter().try_for_each(|name| {
		let (summary, steps) = session.publish(name)?;
		report.steps.extend(steps);
		report.stores.push(super::Status {
			store: (*name).to_owned(),
			summary,
		});
		anyhow::Ok(())
	});
	// A dry run runs no tool and contacts no store.
	assert!(runner.seen.borrow().is_empty());
	published.map(|()| report)
}

fn google_account() -> Secret {
	let key = rsa_key();
	Secret::new(
		serde_json::json!({ "client_email": "ci@project.iam.gserviceaccount.com", "private_key": key })
			.to_string(),
	)
}

#[test]
fn play_numbers_uploads_and_releases_through_an_edit() {
	use super::play::Play;
	use super::play::Rollout;

	let account = google_account();
	let app = "/androidpublisher/v3/applications/com.example.notes";
	let (base, seen) = store(Box::new(move |request, count| {
		let path = request.path.as_str();
		match (request.method.as_str(), path) {
			("POST", "/token") => json(serde_json::json!({ "access_token": "token-1" })),
			("POST", path) if path == format!("{app}/edits") => {
				json(serde_json::json!({ "id": format!("edit-{count}") }))
			}
			("GET", path) if path.ends_with("/tracks") => {
				json(serde_json::json!({ "tracks": [
					{ "track": "internal", "releases": [{ "versionCodes": ["40", "41"] }] },
					{ "track": "production", "releases": [{ "versionCodes": ["39"] }] },
				]}))
			}
			("GET", path) if path.ends_with("/bundles") => {
				json(serde_json::json!({ "bundles": [{ "versionCode": 38 }] }))
			}

			("DELETE", _) => StatusCode::NO_CONTENT.into_response(),
			("POST", path) if path.ends_with("/bundles") => {
				json(serde_json::json!({ "versionCode": 42, "sha256": "…" }))
			}
			("PUT", path) if path.ends_with("/tracks/production") => {
				json(serde_json::json!({ "track": "production" }))
			}
			("POST", path) if path.ends_with(":commit") => {
				json(serde_json::json!({ "id": "edit" }))
			}

			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let play = Play::connect(
		&http,
		&base,
		&format!("{base}/token"),
		&account,
		"com.example.notes",
	)
	.unwrap();
	let edit = play.edit().unwrap().unwrap();
	assert_eq!(edit.highest_version_code().unwrap(), Some(41));

	edit.discard();
	let edit = play.edit().unwrap().unwrap();
	assert_eq!(edit.upload(b"PK aab".to_vec()).unwrap(), 42);
	edit.release(
		"production",
		&super::play::release("1.4.0", 42, Rollout::Staged(0.1), Some("Faster sync.")),
	)
	.unwrap();
	edit.commit().unwrap();

	let seen = seen.lock().unwrap();
	let form = String::from_utf8(seen[0].body.clone()).unwrap();
	let assertion = form.rsplit('=').next().unwrap();
	let claims: serde_json::Value = {
		use base64::Engine as _;
		serde_json::from_slice(
			&base64::engine::general_purpose::URL_SAFE_NO_PAD
				.decode(assertion.split('.').nth(1).unwrap())
				.unwrap(),
		)
		.unwrap()
	};
	assert_eq!(
		claims["scope"],
		"https://www.googleapis.com/auth/androidpublisher"
	);
	assert_eq!(seen[1].authorization, "Bearer token-1");
	assert_eq!(seen[4].method, "DELETE");
	assert_eq!(seen[4].path, format!("{app}/edits/edit-1"));
	let upload = &seen[6];
	assert_eq!(
		upload.path,
		"/upload/androidpublisher/v3/applications/com.example.notes/edits/edit-5/bundles"
	);
	assert_eq!(upload.query, "uploadType=media");
	assert_eq!(upload.content_type, "application/octet-stream");
	assert_eq!(upload.body, b"PK aab");
	let track: serde_json::Value = serde_json::from_slice(&seen[7].body).unwrap();
	assert_eq!(track["track"], "production");

	assert_eq!(track["releases"][0]["status"], "inProgress");
	assert_eq!(track["releases"][0]["userFraction"], 0.1);
	assert_eq!(track["releases"][0]["versionCodes"][0], "42");
	assert_eq!(seen[8].path, format!("{app}/edits/edit-5:commit"));
}

#[test]
fn play_reports_a_missing_app_and_a_service_account_without_access() {
	let account = google_account();
	let (base, _server) = store(Box::new(|request, _| {
		match request.path.as_str() {
			"/token" => json(serde_json::json!({ "access_token": "t" })),
			path if path.contains("com.example.new") => StatusCode::NOT_FOUND.into_response(),
			_ => StatusCode::FORBIDDEN.into_response(),
		}
	}));
	let http = Http::default();
	let connect = |package: &str| {
		super::play::Play::connect(&http, &base, &format!("{base}/token"), &account, package)
			.unwrap()
	};
	assert!(connect("com.example.new").edit().unwrap().is_none());
	let error = connect("com.example.notes")
		.edit()
		.err()
		.unwrap()
		.to_string();
	assert!(error.contains("Users and permissions"), "{error}");
}

#[test]
fn app_store_sets_the_privacy_policy_on_app_information() {
	let Some(key) = apple_key() else { return };
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/v1/apps/123/appInfos") => {
				json(serde_json::json!({ "data": [
					{ "id": "live", "attributes": { "appStoreState": "READY_FOR_SALE" } },
					{ "id": "next", "attributes": { "appStoreState": "PREPARE_FOR_SUBMISSION" } },
				]}))
			}
			("GET", "/v1/appInfos/next/appInfoLocalizations") => {
				json(serde_json::json!({ "data": [
					{ "id": "fr", "attributes": { "locale": "fr-FR" } },
					{ "id": "en", "attributes": { "locale": "en-US" } },
				]}))
			}
			("PATCH", "/v1/appInfoLocalizations/en") => json(serde_json::json!({})),
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	assert!(
		AppStore::new(&http, &base, &key)
			.privacy_policy("123", "https://example.com/privacy")
			.unwrap()
	);
	let seen = seen.lock().unwrap();
	let body: serde_json::Value = serde_json::from_slice(&seen[2].body).unwrap();
	assert_eq!(
		body["data"]["attributes"]["privacyPolicyUrl"],
		"https://example.com/privacy"
	);
}

#[test]
fn play_sets_the_store_listing() {
	let account = google_account();
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/token") => json(serde_json::json!({ "access_token": "t" })),
			("POST", path) if path.ends_with("/edits") => json(serde_json::json!({ "id": "e1" })),
			("PUT", path) if path.ends_with("/edits/e1/listings/en-US") => {
				json(serde_json::json!({}))
			}

			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let play = super::play::Play::connect(&http, &base, &format!("{base}/token"), &account, "a.b")
		.unwrap();
	play.edit()
		.unwrap()
		.unwrap()
		.listing("en-US", "Notes", "Keep notes.", "Keep notes, everywhere.")
		.unwrap();
	let body: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[2].body).unwrap();
	assert_eq!(
		body,
		serde_json::json!({
			"language": "en-US", "title": "Notes",
			"shortDescription": "Keep notes.", "fullDescription": "Keep notes, everywhere.",
		})
	);
}

#[test]
fn firefox_sets_the_listing_text_it_has() {
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("PATCH", "/addons/addon/notes@example.com/") => json(serde_json::json!({})),
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let firefox = |guid: &str| {
		Firefox::new(
			&http,
			&base,
			guid,
			Secret::new("i".into()),
			Secret::new("s".into()),
		)
	};
	assert!(
		firefox("notes@example.com")
			.listing(None, Some("Keep notes."), Some("Keep notes, everywhere."))
			.unwrap()
	);
	// An add-on that doesn't exist yet has no listing to set.
	assert!(
		!firefox("new@example.com")
			.listing(None, Some("Keep notes."), None)
			.unwrap()
	);
	let body: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0].body).unwrap();
	assert_eq!(
		body,
		serde_json::json!({
			"summary": { "en-US": "Keep notes." },
			"description": { "en-US": "Keep notes, everywhere." },
		})
	);
}

#[test]
fn a_store_without_its_icon_gets_a_step() {
	let root = tempfile::tempdir().unwrap();
	let config = |icon: &str| {
		leptoskit_config::LeptoskitConfig::from_toml(
			&format!("[app]\nplatforms = [\"web\", \"ios\"]\n{icon}"),
			root.path(),
		)
		.unwrap()
	};
	let without = config("");
	let step = super::icon_step(root.path(), &without, "app_store").unwrap();
	assert!(step.what.contains("set app.icon"));
	assert!(super::icon_step(root.path(), &config("icon = \"icon.png\"\n"), "play").is_none());
	// An extension may bring its own sizes instead.
	assert!(super::icon_step(root.path(), &without, "chrome").is_some());
	std::fs::create_dir_all(root.path().join("assets")).unwrap();

	std::fs::write(root.path().join("assets/icon-128.png"), "png").unwrap();
	assert!(super::icon_step(root.path(), &without, "chrome").is_none());
}

/// The release workflow passes every credential `leptoskit publish` reads,
/// so a new store's credential can't be forgotten there.
#[test]
fn the_release_workflow_passes_every_credential() {
	let workflow = crate::new_project::release_workflow();

	for needed in &super::NEEDED {
		// The keystore is a file, decoded from its own secret.
		if needed.name == super::play::KEYSTORE {
			assert!(workflow.contains("secrets.LEPTOSKIT_ANDROID_KEYSTORE_BASE64"));
			assert!(workflow.contains("LEPTOSKIT_ANDROID_KEYSTORE=$RUNNER_TEMP/upload.jks"));
			continue;
		}

		assert!(
			workflow.contains(&format!("{0}: ${{{{ secrets.{0} }}}}", needed.name)),
			"the release workflow doesn't pass {}",
			needed.name
		);
	}

	for store in leptoskit_config::publish::STORES {
		assert!(
			workflow.contains(store),
			"the workflow doesn't look for {store}"
		);
	}

	assert!(workflow.contains(&format!(
		"cargo install leptoskit_cli --version {} --locked",
		env!("CARGO_PKG_VERSION")
	)));

	let root = tempfile::tempdir().unwrap();
	let config = leptoskit_config::LeptoskitConfig::default();
	super::write_workflow(root.path(), &config).unwrap();
	let path = root.path().join(crate::new_project::RELEASE_WORKFLOW);
	assert_eq!(std::fs::read_to_string(&path).unwrap(), workflow);
	// An existing workflow is the app's; it isn't replaced.
	let error = super::write_workflow(root.path(), &config).unwrap_err();
	assert!(error.to_string().contains("remove it first"), "{error}");
}

/// Two screenshots for a set: real PNGs of `width`×`height`.
fn shots(width: u32, height: u32) -> Vec<super::images::Image> {
	["01-projects.png", "02-reading.png"]
		.into_iter()
		.map(|name| {
			super::images::Image {
				name: name.to_owned(),
				bytes: super::images::tests::png(width, height, 255),
				width,
				height,
				content_type: "image/png",
			}
		})
		.collect()
}

#[test]
fn app_store_replaces_a_screenshot_set_in_parts() {
	use md5::Digest as _;

	let Some(key) = apple_key() else { return };
	let base: Arc<std::sync::OnceLock<String>> = Arc::default();
	let address = Arc::clone(&base);
	let (url, seen) = store(Box::new(move |request, count| {
		let base = address.get().unwrap();
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/v1/appStoreVersionLocalizations/l1/appScreenshotSets") => {
				// Another display type's set only.
				json(serde_json::json!({ "data": [
					{ "id": "set-ipad", "attributes": { "screenshotDisplayType": "APP_IPAD_PRO_3GEN_129" } },
				]}))
			}
			("POST", "/v1/appScreenshotSets") => {
				(
					StatusCode::CREATED,
					axum::Json(serde_json::json!({ "data": { "id": "set-1" } })),
				)
					.into_response()
			}
			("GET", "/v1/appScreenshotSets/set-1/appScreenshots") => {
				json(serde_json::json!({ "data": [{ "id": "old-1" }, { "id": "old-2" }] }))
			}
			("DELETE", path) if path.starts_with("/v1/appScreenshots/old-") => {
				StatusCode::NO_CONTENT.into_response()
			}

			("POST", "/v1/appScreenshots") => {
				let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
				let size = body["data"]["attributes"]["fileSize"].as_u64().unwrap();
				let half = size / 2;
				let id = format!("shot-{count}");
				// Two parts, as Apple splits large files.
				json(
					serde_json::json!({ "data": { "id": id, "attributes": { "uploadOperations": [
						{ "method": "PUT", "url": format!("{base}/assets/{id}/1"), "offset": 0, "length": half,
						  "requestHeaders": [{ "name": "Content-Type", "value": "image/png" }, { "name": "x-apple-part", "value": "1" }] },
						{ "method": "PUT", "url": format!("{base}/assets/{id}/2"), "offset": half, "length": size - half,
						  "requestHeaders": [{ "name": "Content-Type", "value": "image/png" }] },
					]}}}),
				)
			}
			("PUT", path) if path.starts_with("/assets/") => StatusCode::OK.into_response(),
			("PATCH", path) if path.starts_with("/v1/appScreenshots/shot-") => {
				json(serde_json::json!({}))
			}
			("GET", path) if path.starts_with("/v1/appScreenshots/shot-") => {
				// Processing once, then done.
				let state = if count % 2 == 0 {
					"UPLOAD_COMPLETE"
				} else {
					"COMPLETE"
				};

				json(
					serde_json::json!({ "data": { "attributes": { "assetDeliveryState": { "state": state } } } }),
				)
			}

			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	base.set(url.clone()).unwrap();
	let http = Http::default();
	let client = AppStore::new(&http, &url, &key);
	let images = shots(1320, 2868);

	client
		.replace_screenshots("l1", "APP_IPHONE_67", &images)
		.unwrap();

	let seen = seen.lock().unwrap();
	let body =
		|request: &Seen| -> serde_json::Value { serde_json::from_slice(&request.body).unwrap() };
	assert_eq!(
		seen[0].query,
		"filter%5BscreenshotDisplayType%5D=APP_IPHONE_67"
	);
	let set = body(&seen[1]);
	assert_eq!(
		set["data"]["attributes"]["screenshotDisplayType"],
		"APP_IPHONE_67"
	);
	assert_eq!(
		set["data"]["relationships"]["appStoreVersionLocalization"]["data"]["id"],
		"l1"
	);
	let deleted: Vec<&str> = seen
		.iter()
		.filter(|request| request.method == "DELETE")
		.map(|request| request.path.as_str())
		.collect();
	assert_eq!(
		deleted,
		["/v1/appScreenshots/old-1", "/v1/appScreenshots/old-2"]
	);

	// Each image: reserved with its name and size, sent in its parts without
	// the API key, then committed with its checksum.
	for image in &images {
		let reserve = seen
			.iter()
			.find(|request| {
				request.method == "POST"
					&& request.path == "/v1/appScreenshots"
					&& body(request)["data"]["attributes"]["fileName"] == image.name.as_str()
			})
			.unwrap();
		assert_eq!(
			body(reserve)["data"]["attributes"]["fileSize"],
			image.bytes.len()
		);
		assert_eq!(
			body(reserve)["data"]["relationships"]["appScreenshotSet"]["data"]["id"],
			"set-1"
		);
	}

	let parts: Vec<&Seen> = seen
		.iter()
		.filter(|request| request.method == "PUT")
		.collect();
	assert_eq!(parts.len(), 4);
	assert!(parts.iter().all(|part| part.authorization.is_empty()));
	assert!(parts.iter().all(|part| part.content_type == "image/png"));
	let first: Vec<u8> = [parts[0].body.clone(), parts[1].body.clone()].concat();
	assert_eq!(first, images[0].bytes);
	let commits: Vec<serde_json::Value> = seen
		.iter()
		.filter(|request| request.method == "PATCH")
		.map(body)
		.collect();
	assert_eq!(commits.len(), 2);

	assert_eq!(commits[0]["data"]["attributes"]["uploaded"], true);
	assert_eq!(
		commits[0]["data"]["attributes"]["sourceFileChecksum"],
		hex::encode(md5::Md5::digest(&images[0].bytes))
	);
	assert!(
		seen.iter()
			.any(|request| request.query == "fields%5BappScreenshots%5D=assetDeliveryState")
	);
}

#[test]
fn app_store_reports_a_screenshot_apple_could_not_process() {
	let Some(key) = apple_key() else { return };
	let (base, _server) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", path) if path.ends_with("/appScreenshotSets") => {
				json(serde_json::json!({ "data": [
					{ "id": "set-1", "attributes": { "screenshotDisplayType": "APP_IPHONE_67" } },
				]}))
			}

			("GET", "/v1/appScreenshotSets/set-1/appScreenshots") => {
				json(serde_json::json!({ "data": [] }))
			}
			("POST", "/v1/appScreenshots") => {
				json(
					serde_json::json!({ "data": { "id": "shot-1", "attributes": { "uploadOperations": [] } } }),
				)
			}
			("PATCH", "/v1/appScreenshots/shot-1") => json(serde_json::json!({})),
			("GET", "/v1/appScreenshots/shot-1") => {
				json(
					serde_json::json!({ "data": { "attributes": { "assetDeliveryState": {
						"state": "FAILED",
						"errors": [{ "code": "IMAGE_INCORRECT_DIMENSIONS", "description": "The dimensions of one or more screenshots are wrong." }],
					}}}}),
				)
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let error = AppStore::new(&http, &base, &key)
		.replace_screenshots("l1", "APP_IPHONE_67", &shots(1320, 2868)[..1])
		.unwrap_err()
		.to_string();
	assert!(
		error.contains(
			"couldn't process the screenshot 01-projects.png: The dimensions of one or more \
			 screenshots are wrong. (IMAGE_INCORRECT_DIMENSIONS)"
		),
		"{error}"
	);
}

#[test]
fn play_replaces_the_listing_images_inside_the_edit() {
	let account = google_account();
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/token") => json(serde_json::json!({ "access_token": "t" })),
			("POST", path) if path.ends_with("/edits") => json(serde_json::json!({ "id": "e1" })),
			("DELETE", path) if path.ends_with("/listings/en-US/phoneScreenshots") => {
				json(serde_json::json!({ "deleted": [] }))
			}
			("POST", path) if path.ends_with("/listings/en-US/phoneScreenshots") => {
				json(serde_json::json!({ "image": { "id": "i" } }))
			}

			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let play = super::play::Play::connect(&http, &base, &format!("{base}/token"), &account, "a.b")
		.unwrap();
	let images = shots(1080, 1920);
	play.edit()
		.unwrap()
		.unwrap()
		.replace_images("en-US", "phoneScreenshots", &images)
		.unwrap();
	let seen = seen.lock().unwrap();
	assert_eq!(seen[2].method, "DELETE");
	assert_eq!(
		seen[2].path,
		"/androidpublisher/v3/applications/a.b/edits/e1/listings/en-US/phoneScreenshots"
	);

	for (request, image) in seen[3..].iter().zip(&images) {
		assert_eq!(
			request.path,
			"/upload/androidpublisher/v3/applications/a.b/edits/e1/listings/en-US/phoneScreenshots"
		);
		assert_eq!(request.query, "uploadType=media");
		assert_eq!(request.content_type, "image/png");
		assert_eq!(request.body, image.bytes);
	}

	assert_eq!(seen.len(), 5);
}

#[test]
fn firefox_replaces_the_listing_screenshots() {
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/addons/addon/notes@example.com/") => {
				json(serde_json::json!({ "id": 9, "previews": [{ "id": 11 }, { "id": 12 }] }))
			}
			("DELETE", path) if path.starts_with("/addons/addon/notes@example.com/previews/") => {
				StatusCode::NO_CONTENT.into_response()
			}

			("POST", "/addons/addon/notes@example.com/previews/") => {
				(
					StatusCode::CREATED,
					axum::Json(serde_json::json!({ "id": 13 })),
				)
					.into_response()
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let firefox = |guid: &str| {
		Firefox::new(
			&http,
			&base,
			guid,
			Secret::new("i".into()),
			Secret::new("s".into()),
		)
	};
	let images = shots(1280, 800);
	assert!(
		firefox("notes@example.com")
			.replace_previews(&images)
			.unwrap()
	);
	// An add-on that doesn't exist yet has no listing to show them.
	assert!(
		!firefox("new@example.com")
			.replace_previews(&images)
			.unwrap()
	);

	let seen = seen.lock().unwrap();
	assert_eq!(seen[1].path, "/addons/addon/notes@example.com/previews/11/");
	assert_eq!(seen[2].path, "/addons/addon/notes@example.com/previews/12/");

	for (index, request) in seen[3..5].iter().enumerate() {
		assert!(request.authorization.starts_with("JWT "));
		let form = String::from_utf8_lossy(&request.body);
		assert!(
			form.contains(&format!(
				"name=\"image\"; filename=\"{}\"\r\nContent-Type: image/png\r\n",
				images[index].name
			)),
			"{form}"
		);
		assert!(form.contains(&format!("name=\"position\"\r\n\r\n{index}\r\n")));
	}
}

/// A dry run checks every store's screenshots, and refuses one the store
/// would, before anything builds or uploads.
#[test]
fn a_dry_run_checks_the_screenshots_and_refuses_a_wrong_size() {
	let write = |out: &std::path::Path, path: &str, bytes: Vec<u8>| {
		let path = out.join("store/screenshots").join(path);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, bytes).unwrap();
	};
	let png = super::images::tests::png;
	let out = tempfile::tempdir().unwrap();
	write(
		out.path(),
		"iphone-6.3/01-projects.png",
		png(1206, 2622, 255),
	);
	write(
		out.path(),
		"android-phone/01-projects.png",
		png(1080, 1920, 255),
	);
	write(
		out.path(),
		"android-phone/02-reading.png",
		png(1080, 1920, 255),
	);
	write(
		out.path(),
		"chrome-1280x800/01-projects.png",
		png(1280, 800, 255),
	);
	let report = commonplace(
		out.path(),
		&["app_store", "play", "firefox", "chrome"],
		true,
	)
	.unwrap();

	let text = report.text();
	assert!(
		text.contains("Its screenshots are checked: 1 image for iphone-6.3."),
		"{text}"
	);
	assert!(
		text.contains("Its screenshots are checked: 2 images for android-phone."),
		"{text}"
	);
	assert!(
		text.contains("Its screenshots are checked: 1 image for chrome-1280x800."),
		"{text}"
	);
	assert!(
		text.contains("upload store/screenshots/chrome-1280x800/*.png (1 file"),
		"{text}"
	);
	// The iPhone screenshots are there; the iPad ones aren't.
	assert!(!text.contains("iPhone screenshots:"), "{text}");
	assert!(
		text.contains(
			"iPad screenshots: (the app runs on iPad: add the ipad-13 size to \
			 store/screenshots/screenshots.toml"
		),
		"{text}"
	);

	write(
		out.path(),
		"iphone-6.9/01-projects.png",
		png(1206, 2622, 255),
	);

	for dry_run in [true, false] {
		let error = commonplace(out.path(), &["app_store"], dry_run)
			.err()
			.unwrap()
			.to_string();
		assert!(
			error.contains("iphone-6.9/01-projects.png: it is 1206×2622; the App Store takes"),
			"{error}"
		);
		assert!(error.contains("nothing was uploaded"), "{error}");
	}
}

/// A minimal extension project with the three browser stores, so a
/// [`super::Session`] can be built from packages and a manifest without
/// compiling or packaging anything.
const EXTENSION_PROJECT: &str = r#"
[app]
name = "Notes"
version = "1.4.0"
platforms = ["web", "extension"]
icon = "icon.png"

[extension]
name = "Notes"
browsers = ["chromium", "firefox"]

[extension.popup]
route = "/popup"

[extension.firefox]
id = "notes@example.com"

[publish.chrome]
publisher_id = "pub-1"
item_id = "abcdefghijklmnopabcdefghijklmnop"

[publish.edge]
product_id = "prod-1"

[publish.firefox]
channel = "listed"
"#;

const NO_STORE_PROJECT: &str = r#"
[app]
name = "Notes"
version = "1.4.0"
platforms = ["web"]
"#;

fn browser_credentials() -> Vec<(String, String)> {
	[
		("LEPTOSKIT_PUBLISH_EDGE_CLIENT_ID", "client"),
		("LEPTOSKIT_PUBLISH_EDGE_API_KEY", "key"),
		("LEPTOSKIT_PUBLISH_FIREFOX_JWT_ISSUER", "issuer"),
		("LEPTOSKIT_PUBLISH_FIREFOX_JWT_SECRET", "secret"),
	]
	.into_iter()
	.map(|(name, value)| (name.to_owned(), value.to_owned()))
	.collect()
}

const HELPER: &str = "publish::tests::subprocess_helper";

/// Runs one helper test in a child process with only the environment the
/// scenario needs, so `LEPTOSKIT_PUBLISH_*` overrides never leak into the
/// test process.
fn spawn_publish(scenario: &str, environment: &[(String, String)], cwd: Option<&std::path::Path>) {
	let mut command = std::process::Command::new(std::env::current_exe().unwrap());
	command
		.args(["--exact", HELPER, "--nocapture", "--test-threads=1"])
		.env_clear()
		.env("LEPTOSKIT_TEST_SCENARIO", scenario)
		.env("PATH", std::env::var_os("PATH").unwrap_or_default())
		// No terminal, so `confirm` takes the prompt's non-interactive path
		// instead of waiting for an answer.
		.stdin(std::process::Stdio::null());

	// Winsock loads its providers relative to SystemRoot on Windows.
	if let Some(system_root) = std::env::var_os("SystemRoot") {
		command.env("SystemRoot", system_root);
	}

	if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
		command.env("LLVM_PROFILE_FILE", profile);
	}

	for (name, value) in environment {
		command.env(name, value);
	}

	if let Some(cwd) = cwd {
		command.current_dir(cwd);
	}

	let output = command.output().unwrap();
	assert!(
		output.status.success(),
		"scenario {scenario} failed\nstdout:\n{}\nstderr:\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}

fn chrome_session(scenario: &str, answer: Answer, account: &Secret) -> Vec<Seen> {
	let (base, seen) = store(answer);
	let environment = vec![
		("LEPTOSKIT_PUBLISH_CHROME_API".to_owned(), base.clone()),
		(
			"LEPTOSKIT_PUBLISH_GOOGLE_OAUTH".to_owned(),
			format!("{base}/token"),
		),
		(
			"LEPTOSKIT_PUBLISH_CHROME_SERVICE_ACCOUNT".to_owned(),
			account.expose().to_owned(),
		),
	];
	spawn_publish(scenario, &environment, None);
	seen.lock().unwrap().clone()
}

fn firefox_session(scenario: &str, answer: Answer) -> Vec<Seen> {
	let (base, seen) = store(answer);
	let mut environment = browser_credentials();
	environment.push(("LEPTOSKIT_PUBLISH_FIREFOX_API".to_owned(), base));
	spawn_publish(scenario, &environment, None);
	seen.lock().unwrap().clone()
}

/// Edge Add-ons answers `202 Accepted` with a `Location` header, so these
/// scenarios use the existing `store` fixture, which can carry headers.
fn edge_session(scenario: &str, answer: Answer) -> Vec<Seen> {
	let (base, seen) = store(answer);
	let mut environment = browser_credentials();
	environment.push(("LEPTOSKIT_PUBLISH_EDGE_API".to_owned(), base));
	spawn_publish(scenario, &environment, None);
	seen.lock().unwrap().clone()
}

fn accepted(location: &str) -> Response {
	let mut response = StatusCode::ACCEPTED.into_response();
	response
		.headers_mut()
		.insert("location", location.parse().unwrap());
	response
}

fn edge_answer(submit: bool) -> Answer {
	Box::new(move |request, count| {
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/v1/products/prod-1/submissions/draft/package") => accepted("op-upload"),
			("GET", "/v1/products/prod-1/submissions/draft/package/operations/op-upload") => {
				json(
					serde_json::json!({ "status": if count < 2 { "InProgress" } else { "Succeeded" } }),
				)
			}
			("POST", "/v1/products/prod-1/submissions") if submit => {
				accepted("/v1/products/prod-1/submissions/operations/op-submit")
			}
			("GET", "/v1/products/prod-1/submissions/operations/op-submit") if submit => {
				json(serde_json::json!({ "status": "Succeeded" }))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	})
}

fn project(config: &str) -> tempfile::TempDir {
	let directory = tempfile::tempdir().unwrap();
	std::fs::write(directory.path().join("leptoskit.toml"), config).unwrap();
	directory
}

fn run_scenario(scenario: &str, root: &std::path::Path, environment: &[(String, String)]) {
	spawn_publish(scenario, environment, Some(root));
}

#[test]
fn chrome_session_uploads_a_draft_and_submits_only_when_asked() {
	let account = google_account();
	let answer = || {
		Box::new(|request: &Seen, _: usize| {
			match request.path.as_str() {
				"/token" => {
					json(serde_json::json!({ "access_token": "token-1", "expires_in": 3600 }))
				}
				path if path.ends_with(":upload") => {
					json(serde_json::json!({ "uploadState": "SUCCEEDED", "crxVersion": "1.4.0" }))
				}
				path if path.ends_with(":publish") => {
					json(serde_json::json!({ "state": "PENDING_REVIEW" }))
				}
				_ => StatusCode::NOT_FOUND.into_response(),
			}
		})
	};

	let draft = chrome_session("chrome-draft", answer(), &account);
	assert_eq!(draft.len(), 2);
	assert_eq!(draft[0].path, "/token");
	assert_eq!(
		draft[1].path,
		"/upload/v2/publishers/pub-1/items/abcdefghijklmnopabcdefghijklmnop:upload"
	);
	assert_eq!(draft[1].authorization, "Bearer token-1");
	assert_eq!(draft[1].body, b"PK chromium");

	let submitted = chrome_session("chrome-submit", answer(), &account);
	assert_eq!(submitted.len(), 3);
	assert_eq!(
		submitted[2].path,
		"/v2/publishers/pub-1/items/abcdefghijklmnopabcdefghijklmnop:publish"
	);
	let publish: serde_json::Value = serde_json::from_slice(&submitted[2].body).unwrap();
	assert_eq!(publish["publishType"], "DEFAULT_PUBLISH");
}

#[test]
fn chrome_session_reports_a_refused_package() {
	let account = google_account();
	let requests = chrome_session(
		"chrome-error",
		Box::new(|request, _| {
			match request.path.as_str() {
				"/token" => json(serde_json::json!({ "access_token": "token-1" })),
				_ => (StatusCode::BAD_REQUEST, "manifest version must be newer").into_response(),
			}
		}),
		&account,
	);
	assert_eq!(requests.len(), 2);
	assert_eq!(
		requests[1].path,
		"/upload/v2/publishers/pub-1/items/abcdefghijklmnopabcdefghijklmnop:upload"
	);
}

#[test]
fn edge_session_uploads_a_draft_and_submits_only_when_asked() {
	let draft = edge_session("edge-draft", edge_answer(false));
	assert_eq!(draft.len(), 3);
	assert_eq!(
		draft[0].path,
		"/v1/products/prod-1/submissions/draft/package"
	);
	assert_eq!(draft[0].authorization, "ApiKey key");
	assert_eq!(draft[0].body, b"PK chromium");

	let submitted = edge_session("edge-submit", edge_answer(true));
	assert_eq!(submitted.len(), 5);
	assert_eq!(submitted[3].path, "/v1/products/prod-1/submissions");
	let notes: serde_json::Value = serde_json::from_slice(&submitted[3].body).unwrap();
	assert_eq!(notes["notes"], "Version 1.4.0.");
}

#[test]
fn edge_session_explains_an_expired_key() {
	let seen = edge_session(
		"edge-error",
		Box::new(|_, _| StatusCode::UNAUTHORIZED.into_response()),
	);
	assert_eq!(seen.len(), 1);
	assert_eq!(
		seen[0].path,
		"/v1/products/prod-1/submissions/draft/package"
	);
}

#[test]
fn firefox_session_validates_a_draft_and_submits_the_source_only_when_asked() {
	let answer = || {
		Box::new(|request: &Seen, count: usize| {
			match (request.method.as_str(), request.path.as_str()) {
				("POST", "/addons/upload/") => {
					json(serde_json::json!({ "uuid": "u-1", "processed": false }))
				}
				("GET", "/addons/upload/u-1/") => {
					json(
						serde_json::json!({ "uuid": "u-1", "processed": count > 1, "valid": true }),
					)
				}
				("POST", "/addons/addon/notes@example.com/versions/") => {
					json(serde_json::json!({ "id": 7, "version": "1.4.0" }))
				}
				_ => StatusCode::NOT_FOUND.into_response(),
			}
		})
	};

	let draft = firefox_session("firefox-draft", answer());
	assert_eq!(draft.len(), 3);
	assert_eq!(draft[0].path, "/addons/upload/");
	assert!(draft[0].authorization.starts_with("JWT "));
	assert!(
		draft[0]
			.content_type
			.starts_with("multipart/form-data; boundary=")
	);
	assert!(String::from_utf8_lossy(&draft[0].body).contains("PK firefox"));
	assert!(String::from_utf8_lossy(&draft[0].body).contains("name=\"channel\"\r\n\r\nlisted"));
	assert_eq!(draft[2].path, "/addons/upload/u-1/");

	let submitted = firefox_session("firefox-submit", answer());
	assert_eq!(submitted.len(), 4);
	assert_eq!(
		submitted[3].path,
		"/addons/addon/notes@example.com/versions/"
	);
	let body = String::from_utf8_lossy(&submitted[3].body);
	assert!(body.contains("name=\"upload\"\r\n\r\nu-1"));
	assert!(body.contains("filename=\"firefox-source.zip\""));
	assert!(body.contains("PK source"));
}

#[test]
fn firefox_session_reports_failed_validation() {
	let requests = firefox_session(
		"firefox-error",
		Box::new(|request, _| {
			match (request.method.as_str(), request.path.as_str()) {
				("POST", "/addons/upload/") => json(serde_json::json!({ "uuid": "u-1" })),
				("GET", "/addons/upload/u-1/") => {
					json(serde_json::json!({
						"processed": true,
						"valid": false,
						"validation": {
							"messages": [{ "type": "error", "message": "manifest.json is invalid" }]
						}
					}))
				}
				_ => StatusCode::NOT_FOUND.into_response(),
			}
		}),
	);
	assert_eq!(requests.len(), 2);
	assert_eq!(requests[1].path, "/addons/upload/u-1/");
}

#[test]
fn firefox_session_explains_a_first_submission() {
	let requests = firefox_session(
		"firefox-first-submission",
		Box::new(|request, _| {
			match (request.method.as_str(), request.path.as_str()) {
				("POST", "/addons/upload/") => json(serde_json::json!({ "uuid": "u-1" })),
				("GET", "/addons/upload/u-1/") => {
					json(serde_json::json!({ "processed": true, "valid": true }))
				}
				_ => StatusCode::NOT_FOUND.into_response(),
			}
		}),
	);
	assert_eq!(requests.len(), 3);
	assert_eq!(
		requests[2].path,
		"/addons/addon/notes@example.com/versions/"
	);
}

#[test]
fn firefox_session_replaces_the_listing_screenshots() {
	let requests = firefox_session(
		"firefox-screenshots",
		Box::new(|request, _| {
			match (request.method.as_str(), request.path.as_str()) {
				("POST", "/addons/upload/") => json(serde_json::json!({ "uuid": "u-1" })),
				("GET", "/addons/upload/u-1/") => {
					json(serde_json::json!({ "processed": true, "valid": true }))
				}
				("GET", "/addons/addon/notes@example.com/") => {
					json(serde_json::json!({ "id": 9, "previews": [{ "id": 11 }] }))
				}
				("DELETE", "/addons/addon/notes@example.com/previews/11/") => {
					StatusCode::NO_CONTENT.into_response()
				}
				("POST", "/addons/addon/notes@example.com/previews/") => {
					(
						StatusCode::CREATED,
						axum::Json(serde_json::json!({ "id": 13 })),
					)
						.into_response()
				}
				_ => StatusCode::NOT_FOUND.into_response(),
			}
		}),
	);
	let deleted = requests
		.iter()
		.filter(|request| request.method == "DELETE")
		.map(|request| request.path.as_str())
		.collect::<Vec<_>>();
	assert_eq!(deleted, ["/addons/addon/notes@example.com/previews/11/"]);
	let uploaded = requests.last().unwrap();
	assert_eq!(uploaded.path, "/addons/addon/notes@example.com/previews/");
	assert!(
		String::from_utf8_lossy(&uploaded.body).contains("01-projects.png"),
		"{}",
		String::from_utf8_lossy(&uploaded.body)
	);
}

#[test]
fn firefox_session_reports_a_refused_submission() {
	let requests = firefox_session(
		"firefox-submit-error",
		Box::new(|request, _| {
			match (request.method.as_str(), request.path.as_str()) {
				("POST", "/addons/upload/") => json(serde_json::json!({ "uuid": "u-1" })),
				("GET", "/addons/upload/u-1/") => {
					json(serde_json::json!({ "processed": true, "valid": true }))
				}
				_ => (StatusCode::INTERNAL_SERVER_ERROR, "server exploded").into_response(),
			}
		}),
	);
	assert_eq!(requests.len(), 3);
	assert_eq!(
		requests[2].path,
		"/addons/addon/notes@example.com/versions/"
	);
}

#[test]
fn firefox_session_checks_an_unlisted_draft_without_contacting_the_site() {
	let requests = firefox_session(
		"firefox-unlisted",
		Box::new(|_, _| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
	);
	assert!(requests.is_empty(), "{requests:#?}");
}

/// The app stores' uploads reach a native build, which needs the project's
/// own configuration. From a directory without one they fail before a tool
/// runs, naming what is missing.
#[test]
fn native_store_uploads_fail_before_a_tool_runs() {
	let account = google_account();
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/token") => json(serde_json::json!({ "access_token": "t" })),
			("POST", path) if path.ends_with("/edits") => json(serde_json::json!({ "id": "e1" })),
			("GET", path) if path.ends_with("/tracks") => {
				json(serde_json::json!({ "tracks": [
					{ "track": "internal", "releases": [{ "versionCodes": ["40"] }] },
				]}))
			}
			("GET", path) if path.ends_with("/bundles") => {
				json(serde_json::json!({ "bundles": [{ "versionCode": 39 }] }))
			}
			("GET", "/v1/builds") => {
				json(serde_json::json!({ "data": [
					{ "id": "b7", "attributes": { "version": "7" } },
				]}))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let environment = vec![
		(
			"LEPTOSKIT_PUBLISH_APPLE_ISSUER_ID".to_owned(),
			"issuer-1".to_owned(),
		),
		(
			"LEPTOSKIT_PUBLISH_APPLE_KEY_ID".to_owned(),
			"KEY123".to_owned(),
		),
		(
			"LEPTOSKIT_PUBLISH_APPLE_KEY".to_owned(),
			super::secrets::tests::ec_key().expect("publish tests require OpenSSL"),
		),
		(
			"LEPTOSKIT_PUBLISH_MACOS_IDENTITY".to_owned(),
			"Developer ID Application: Example".to_owned(),
		),
		(
			"LEPTOSKIT_PUBLISH_PLAY_SERVICE_ACCOUNT".to_owned(),
			account.expose().to_owned(),
		),
		("LEPTOSKIT_PUBLISH_APPLE_API".to_owned(), base.clone()),
		("LEPTOSKIT_PUBLISH_PLAY_API".to_owned(), base.clone()),
		(
			"LEPTOSKIT_PUBLISH_GOOGLE_OAUTH".to_owned(),
			format!("{base}/token"),
		),
		(
			"LEPTOSKIT_ANDROID_KEYSTORE".to_owned(),
			"upload.jks".to_owned(),
		),
		(
			"LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD".to_owned(),
			"keystore-password".to_owned(),
		),
		(
			"LEPTOSKIT_ANDROID_KEY_ALIAS".to_owned(),
			"upload".to_owned(),
		),
		(
			"LEPTOSKIT_ANDROID_KEY_PASSWORD".to_owned(),
			"key-password".to_owned(),
		),
	];
	spawn_publish("native-failures", &environment, None);
	// Play read the app's tracks before the build it couldn't start.
	assert!(!seen.lock().unwrap().is_empty());
}

#[test]
fn play_works_without_an_app_until_the_build_it_cannot_start() {
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/token") => json(serde_json::json!({ "access_token": "t" })),
			// Play Console hasn't got the app yet, so opening an edit 404s.
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let environment = vec![
		(
			"LEPTOSKIT_PUBLISH_PLAY_SERVICE_ACCOUNT".to_owned(),
			google_account().expose().to_owned(),
		),
		("LEPTOSKIT_PUBLISH_PLAY_API".to_owned(), base.clone()),
		(
			"LEPTOSKIT_PUBLISH_GOOGLE_OAUTH".to_owned(),
			format!("{base}/token"),
		),
		(
			"LEPTOSKIT_ANDROID_KEYSTORE".to_owned(),
			"upload.jks".to_owned(),
		),
		(
			"LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD".to_owned(),
			"keystore-password".to_owned(),
		),
		(
			"LEPTOSKIT_ANDROID_KEY_ALIAS".to_owned(),
			"upload".to_owned(),
		),
		(
			"LEPTOSKIT_ANDROID_KEY_PASSWORD".to_owned(),
			"key-password".to_owned(),
		),
	];
	spawn_publish("play-missing-app", &environment, None);
	let seen = seen.lock().unwrap();
	assert_eq!(seen.len(), 2, "{seen:#?}");
	assert_eq!(seen[1].method, "POST");
	assert!(seen[1].path.ends_with("/edits"), "{}", seen[1].path);
}

#[test]
fn app_store_and_macos_name_a_key_file_they_cannot_read() {
	spawn_publish(
		"app-store-bad-key",
		&[
			(
				"LEPTOSKIT_PUBLISH_APPLE_ISSUER_ID".to_owned(),
				"issuer-1".to_owned(),
			),
			(
				"LEPTOSKIT_PUBLISH_APPLE_KEY_ID".to_owned(),
				"KEY123".to_owned(),
			),
			(
				"LEPTOSKIT_PUBLISH_APPLE_KEY".to_owned(),
				"/nonexistent/AuthKey.p8".to_owned(),
			),
			(
				"LEPTOSKIT_PUBLISH_MACOS_IDENTITY".to_owned(),
				"Developer ID Application: Example".to_owned(),
			),
		],
		None,
	);
}

#[test]
fn submitting_without_a_terminal_needs_no_answer() {
	spawn_publish("confirm", &[], None);
}

/// With a terminal on stdin, `--submit` asks before it reaches users. Only
/// Unix lets a test hand its child a terminal, through `script`, so the
/// Windows run takes the prompt as covered by the others.
#[cfg(unix)]
#[test]
fn a_terminal_asks_before_submitting() {
	use std::io::Write as _;
	use std::process::Stdio;
	use std::time::Duration;

	let exe = std::env::current_exe().expect("test binary");
	let helper: Vec<std::ffi::OsString> = vec![
		exe.into_os_string(),
		"--exact".into(),
		HELPER.into(),
		"--nocapture".into(),
		"--test-threads=1".into(),
	];
	let mut command = std::process::Command::new("script");

	if cfg!(target_os = "macos") {
		command.args(["-q", "/dev/null"]).args(&helper);
	} else {
		let line = helper
			.iter()
			.map(|argument| format!("'{}'", argument.to_string_lossy().replace('\'', "'\\''")))
			.collect::<Vec<_>>()
			.join(" ");
		command.args(["-q", "-e", "-c", &line, "/dev/null"]);
	}

	command
		.env_clear()
		.env("LEPTOSKIT_TEST_SCENARIO", "confirm-prompt")
		.env("PATH", std::env::var_os("PATH").unwrap_or_default())
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped());

	if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
		command.env("LLVM_PROFILE_FILE", profile);
	}

	let mut child = command.spawn().expect("script");
	{
		let mut stdin = child.stdin.take().expect("stdin");
		stdin.write_all(b"y\n").expect("answer");
		// Keep the pseudo-terminal open long enough for the child to read
		// the answer; closing immediately delivers an end-of-file first.
		std::thread::sleep(Duration::from_millis(500));
	}
	let output = child.wait_with_output().expect("script");
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(
		stdout.contains("Submit Notes 1.4.0 to chrome for review?"),
		"{stdout}\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
	assert!(stdout.contains("test result: ok"), "{stdout}");
}

#[test]
fn publish_run_rejects_no_store_and_an_unknown_store() {
	let no_store = project(NO_STORE_PROJECT);
	run_scenario("run-no-store", no_store.path(), &[]);

	let configured = project(EXTENSION_PROJECT);
	run_scenario("run-unknown-store", configured.path(), &[]);
}

#[test]
fn publish_run_names_a_missing_credential() {
	let configured = project(EXTENSION_PROJECT);
	run_scenario("run-missing-credential", configured.path(), &[]);
}

#[test]
fn publish_run_refuses_credentials_inside_the_project() {
	let configured = project(EXTENSION_PROJECT);
	let credentials = configured.path().join("credentials.toml");
	std::fs::write(
		&credentials,
		"LEPTOSKIT_PUBLISH_CHROME_SERVICE_ACCOUNT = \"value\"\n",
	)
	.unwrap();
	run_scenario(
		"run-credentials-inside",
		configured.path(),
		&[(
			"LEPTOSKIT_TEST_CREDENTIALS".to_owned(),
			credentials.display().to_string(),
		)],
	);
}

#[test]
fn publish_run_refuses_to_replace_an_existing_workflow() {
	let configured = project(EXTENSION_PROJECT);
	let workflow = configured.path().join(crate::new_project::RELEASE_WORKFLOW);
	std::fs::create_dir_all(workflow.parent().unwrap()).unwrap();
	std::fs::write(&workflow, "name: existing\n").unwrap();
	run_scenario("run-existing-workflow", configured.path(), &[]);
}

#[test]
fn publish_run_reports_an_extension_build_it_cannot_start() {
	let configured = project(EXTENSION_PROJECT);
	run_scenario(
		"run-extension-build",
		configured.path(),
		&[(
			"LEPTOSKIT_PUBLISH_CHROME_SERVICE_ACCOUNT".to_owned(),
			"{}".to_owned(),
		)],
	);
}

/// The child side of [`spawn_publish`]: builds a [`super::Session`] from
/// packages and a manifest, or calls [`super::run`] in the project the
/// parent made the current directory. A normal test run has no scenario and
/// does nothing.
fn session_child(scenario: &str) {
	let (store_name, submit) = match scenario {
		"chrome-draft" | "chrome-error" => ("chrome", false),
		"chrome-submit" => ("chrome", true),
		"edge-draft" | "edge-error" => ("edge", false),
		"edge-submit" => ("edge", true),
		"firefox-draft" | "firefox-error" | "firefox-screenshots" | "firefox-unlisted" => {
			("firefox", false)
		}
		"firefox-submit" | "firefox-first-submission" | "firefox-submit-error" => ("firefox", true),
		other => panic!("unknown session scenario {other}"),
	};
	let dry_run = scenario == "firefox-unlisted";
	let root = tempfile::tempdir().unwrap();

	for (name, bytes) in [
		("chromium.zip", b"PK chromium".as_slice()),
		("firefox.zip", b"PK firefox".as_slice()),
		("firefox-source.zip", b"PK source".as_slice()),
	] {
		std::fs::write(root.path().join(name), bytes).unwrap();
	}

	if scenario == "firefox-screenshots" {
		let shots = root.path().join("store/screenshots/chrome-1280x800");
		std::fs::create_dir_all(&shots).unwrap();
		std::fs::write(
			shots.join("01-projects.png"),
			super::images::tests::png(1280, 800, 255),
		)
		.unwrap();
	}

	// An unlisted add-on isn't on the site, so it has no listing page.
	let project = if dry_run {
		EXTENSION_PROJECT.replace("channel = \"listed\"", "channel = \"unlisted\"")
	} else {
		EXTENSION_PROJECT.to_owned()
	};
	let config = leptoskit_config::LeptoskitConfig::from_toml(&project, root.path()).unwrap();
	let store = super::listing::Store::load(root.path()).unwrap();
	let credentials = super::Credentials::default();
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let package = |browser: leptoskit_config::Browser, zip: &str| {
		crate::extension::Package {
			browser,
			dir: root.path().join(browser.name()),
			zip: Some(root.path().join(zip)),
		}
	};
	let extension = super::Extension {
		built: crate::extension::Built {
			packages: vec![
				package(leptoskit_config::Browser::Chromium, "chromium.zip"),
				package(leptoskit_config::Browser::Firefox, "firefox.zip"),
			],
			source: Some(root.path().join("firefox-source.zip")),
		},
		manifest: serde_json::json!({ "permissions": [] }),
	};
	let args = super::PublishArgs {
		stores: vec![store_name.to_owned()],
		dry_run,
		submit,
		staged: false,
		yes: true,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Auto,
	};
	let session = super::Session {
		args: &args,
		root: root.path(),
		config: &config,
		credentials: &credentials,
		store: &store,
		http: &http,
		runner: &runner,
		extension: Some(&extension),
		name: "Notes".to_owned(),
		version: "1.4.0".to_owned(),
		submit,
	};
	let result = session.publish(store_name);

	if scenario.ends_with("-error") {
		let error = result.unwrap_err().to_string();
		let expected = match store_name {
			"chrome" => "manifest version must be newer",
			"edge" => "72 days",
			"firefox" if scenario == "firefox-submit-error" => "server exploded",
			"firefox" => "manifest.json is invalid",
			other => panic!("no expected error for {other}"),
		};
		assert!(error.contains(expected), "{error}");
		return;
	}

	let (summary, steps) = result.unwrap();
	let expected = match scenario {
		"chrome-draft" | "edge-draft" => "Notes 1.4.0 is uploaded as a draft.",
		"chrome-submit" => "Notes 1.4.0 is submitted for review (PENDING_REVIEW).",
		"edge-submit" => "Notes 1.4.0 is submitted for certification.",
		"firefox-draft" => {
			"Notes 1.4.0 passed addons.mozilla.org's validation; nothing is submitted yet."
		}
		"firefox-submit" => "Notes 1.4.0 is submitted (listed channel).",
		"firefox-first-submission" => {
			"Notes 1.4.0 passed validation; the add-on needs its first submission on the site."
		}
		"firefox-screenshots" => {
			"Notes 1.4.0 passed addons.mozilla.org's validation; nothing is submitted yet. Its \
			 screenshots are uploaded: 1 image for chrome-1280x800."
		}
		"firefox-unlisted" => "Notes 1.4.0 is built and checked; nothing was uploaded.",
		other => panic!("no expected summary for {other}"),
	};
	assert_eq!(summary, expected);
	assert!(steps.iter().all(|step| step.store == store_name));
	assert_eq!(
		steps
			.iter()
			.any(|step| step.what.contains("Decide to release it")),
		!submit
	);
}

/// The child side of [`run_scenario`]: `super::run` must reject each bad
/// configuration before it builds the extension or contacts a store.
fn run_child(scenario: &str) {
	let args = |stores: Vec<String>, credentials: Option<std::path::PathBuf>| {
		super::PublishArgs {
			stores,
			dry_run: false,
			submit: false,
			staged: false,
			yes: true,
			json: false,
			credentials,
			build_number: super::BuildNumber::Auto,
		}
	};
	let error = match scenario {
		"run-no-store" => super::run(&args(Vec::new(), None)).unwrap_err(),
		"run-unknown-store" => super::run(&args(vec!["play".to_owned()], None)).unwrap_err(),
		"run-missing-credential" | "run-extension-build" => {
			super::run(&args(vec!["chrome".to_owned()], None)).unwrap_err()
		}
		"run-credentials-inside" => {
			let credentials = std::env::var("LEPTOSKIT_TEST_CREDENTIALS").unwrap();
			super::run(&args(
				vec!["credentials".to_owned()],
				Some(std::path::PathBuf::from(credentials)),
			))
			.unwrap_err()
		}
		"run-existing-workflow" => {
			super::run(&args(vec!["workflow".to_owned()], None)).unwrap_err()
		}
		other => panic!("unknown run scenario {other}"),
	};
	let text = format!("{error:#}");
	let expected = match scenario {
		"run-no-store" => "no store is configured",
		"run-unknown-store" => "`play` isn't configured",
		"run-missing-credential" => "LEPTOSKIT_PUBLISH_CHROME_SERVICE_ACCOUNT isn't set",
		"run-credentials-inside" => "inside the project and git doesn't ignore it",
		"run-existing-workflow" => "remove it first",
		"run-extension-build" => "cargo metadata failed",
		other => panic!("no expected error for {other}"),
	};
	assert!(text.contains(expected), "{text}");
}

#[test]
fn subprocess_helper() {
	let Ok(scenario) = std::env::var("LEPTOSKIT_TEST_SCENARIO") else {
		return;
	};

	if scenario.starts_with("run-") {
		run_child(&scenario);
	} else if scenario == "native-failures" {
		native_failure_child();
	} else if scenario == "play-missing-app" {
		play_missing_app_child();
	} else if scenario == "app-store-bad-key" {
		bad_key_child();
	} else if scenario == "confirm" {
		confirm_child();
	} else if scenario == "confirm-prompt" {
		confirm_prompt_child();
	} else {
		session_child(&scenario);
	}
}

/// The child side of the native store scenarios: `app_store`, `macos`, and
/// `play` reach their native build from a directory with no project, so each
/// fails before a tool runs or an upload starts.
fn native_failure_child() {
	let empty = tempfile::tempdir().unwrap();
	std::env::set_current_dir(empty.path()).unwrap();
	let root = tempfile::tempdir().unwrap();
	write_android_host(root.path());
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		r#"
[app]
name = "Notes"
version = "1.4.0"
default_platform = "ios"
platforms = ["ios", "android", "macos"]

[native]
package_id = "com.example.notes"
apple_team_id = "ABCDE12345"

[publish.app_store]
app_id = "1234567890"
team_id = "ABCDE12345"

[publish.macos]
distribution = "developer_id"

[publish.play]
track = "internal"
"#,
		root.path(),
	)
	.unwrap();
	let store = super::listing::Store::load(root.path()).unwrap();
	let credentials = super::Credentials::default();
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let args = super::PublishArgs {
		stores: Vec::new(),
		dry_run: false,
		submit: false,
		staged: false,
		yes: true,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Auto,
	};
	let session = super::Session {
		args: &args,
		root: root.path(),
		config: &config,
		credentials: &credentials,
		store: &store,
		http: &http,
		runner: &runner,
		extension: None,
		name: super::display_name(&config),
		version: super::apple_version(&config),
		submit: false,
	};

	for store in ["app_store", "macos", "play"] {
		let error = session.publish(store).unwrap_err().to_string();
		assert!(
			error.contains("no leptoskit.toml found"),
			"{store}: {error}"
		);
	}

	// No tool ran, and only Play reached its mock store before the build.
	assert!(runner.seen.borrow().is_empty());
}

/// The child side of the missing-app scenario: an app Play Console hasn't
/// seen yet opens no edit, so Play gets version code 1 and has nothing to
/// upload.
fn play_missing_app_child() {
	let empty = tempfile::tempdir().unwrap();
	std::env::set_current_dir(empty.path()).unwrap();
	let root = tempfile::tempdir().unwrap();
	write_android_host(root.path());
	let config = leptoskit_config::LeptoskitConfig::from_toml(PLAY_PROJECT, root.path()).unwrap();
	let store = super::listing::Store::load(root.path()).unwrap();
	let credentials = super::Credentials::default();
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let args = super::PublishArgs {
		stores: vec!["play".to_owned()],
		dry_run: false,
		submit: false,
		staged: false,
		yes: true,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Auto,
	};
	let session = super::Session {
		args: &args,
		root: root.path(),
		config: &config,
		credentials: &credentials,
		store: &store,
		http: &http,
		runner: &runner,
		extension: None,
		name: super::display_name(&config),
		version: super::apple_version(&config),
		submit: false,
	};
	let error = session.publish("play").unwrap_err().to_string();
	assert!(error.contains("no leptoskit.toml found"), "{error}");
}

/// The child side of the bad-key scenarios: each app store reports the key
/// it can't sign with before any build or upload.
fn bad_key_child() {
	let root = tempfile::tempdir().unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		r#"
[app]
name = "Notes"
version = "1.4.0"
default_platform = "ios"
platforms = ["ios", "macos"]

[native]
package_id = "com.example.notes"
apple_team_id = "ABCDE12345"

[publish.app_store]
app_id = "1234567890"
team_id = "ABCDE12345"

[publish.macos]
distribution = "developer_id"
"#,
		root.path(),
	)
	.unwrap();
	let store = super::listing::Store::load(root.path()).unwrap();
	let credentials = super::Credentials::default();
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let args = super::PublishArgs {
		stores: Vec::new(),
		dry_run: false,
		submit: false,
		staged: false,
		yes: true,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Number(9),
	};
	let session = super::Session {
		args: &args,
		root: root.path(),
		config: &config,
		credentials: &credentials,
		store: &store,
		http: &http,
		runner: &runner,
		extension: None,
		name: super::display_name(&config),
		version: super::apple_version(&config),
		submit: false,
	};

	for store in ["app_store", "macos"] {
		let error = session.publish(store).unwrap_err().to_string();
		assert!(
			error.contains("names a file that can't be read"),
			"{store}: {error}"
		);
	}
}

/// The child side of the prompt scenario: a terminal asks, and `y` answers.
fn confirm_prompt_child() {
	let args = super::PublishArgs {
		stores: vec!["chrome".to_owned()],
		dry_run: false,
		submit: true,
		staged: false,
		yes: false,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Auto,
	};
	assert!(super::confirm(&args, "Notes", "1.4.0", &["chrome"]).unwrap());
}

/// The child side of the confirmation scenario: with no terminal, `--submit`
/// takes the non-interactive answer, so scripts never hang waiting.
fn confirm_child() {
	let args = super::PublishArgs {
		stores: vec!["chrome".to_owned()],
		dry_run: false,
		submit: true,
		staged: false,
		yes: false,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Auto,
	};
	assert!(super::confirm(&args, "Notes", "1.4.0", &["chrome"]).unwrap());
}

/// `leptoskit publish` arguments for a test: every store, or the ones named,
/// with no credentials file.
fn arguments(
	stores: &[&str],
	dry_run: bool,
	build_number: super::BuildNumber,
) -> super::PublishArgs {
	super::PublishArgs {
		stores: stores.iter().map(|store| (*store).to_owned()).collect(),
		dry_run,
		submit: false,
		staged: false,
		yes: true,
		json: false,
		credentials: None,
		build_number,
	}
}

/// A [`super::Session`] over `root` with no extension packages, for the
/// stores whose dry runs don't need one.
fn session<'a>(
	root: &'a std::path::Path,
	config: &'a leptoskit_config::LeptoskitConfig,
	store: &'a super::listing::Store,
	credentials: &'a super::Credentials,
	args: &'a super::PublishArgs,
	http: &'a Http,
	runner: &'a super::tools::Recorder,
) -> super::Session<'a> {
	super::Session {
		args,
		root,
		config,
		credentials,
		store,
		http,
		runner,
		extension: None,
		name: super::display_name(config),
		version: super::apple_version(config),
		submit: false,
	}
}

/// One screenshot set for `folder`, as `store/screenshots/` would load it.
fn one_image_set(folder: &str) -> super::images::Set {
	let target = super::images::TARGETS
		.iter()
		.find(|target| target.folder == folder)
		.unwrap_or_else(|| panic!("no store size {folder}"));
	super::images::Set {
		target,
		images: vec![super::images::Image {
			name: "01-projects.png".to_owned(),
			bytes: super::images::tests::png(1280, 800, 255),
			width: 1280,
			height: 800,
			content_type: "image/png",
		}],
	}
}

/// `publish play`'s local Android host: a Gradle script Google Play can
/// take, and the `Cargo.toml` that marks `android/` as the host directory.
fn write_android_host(root: &std::path::Path) {
	let host = root.join("android");
	std::fs::create_dir_all(host.join("app")).unwrap();
	std::fs::write(host.join("Cargo.toml"), "").unwrap();
	std::fs::write(
		host.join("app/build.gradle.kts"),
		r#"
android {
    defaultConfig {
        targetSdk = 36
    }
    signingConfigs {
        create("release") {
            storeFile = file(System.getenv("LEPTOSKIT_ANDROID_KEYSTORE")!!)
        }
    }
}
val versionCode = (findProperty("leptoskit.versionCode") as String).toInt()
"#,
	)
	.unwrap();
}

const PLAY_PROJECT: &str = r#"
[app]
name = "Notes"
version = "1.4.0"
platforms = ["web", "android"]

[native]
package_id = "com.example.notes"

[publish.play]
track = "internal"
"#;

#[test]
fn build_numbers_are_auto_or_positive_whole_numbers() {
	use std::str::FromStr as _;

	assert_eq!(
		super::BuildNumber::from_str("auto").unwrap(),
		super::BuildNumber::Auto
	);
	assert_eq!(
		super::BuildNumber::from_str("42").unwrap(),
		super::BuildNumber::Number(42)
	);
	assert_eq!(
		super::BuildNumber::from_str("0007").unwrap(),
		super::BuildNumber::Number(7)
	);

	for text in ["", "0", "-1", "1.5", "later"] {
		let error = super::BuildNumber::from_str(text).unwrap_err();
		assert!(error.contains("`auto`"), "{text:?}: {error}");
	}
}

#[test]
fn display_names_and_versions_prefer_the_name_people_see() {
	let root = tempfile::tempdir().unwrap();
	let config =
		|text: &str| leptoskit_config::LeptoskitConfig::from_toml(text, root.path()).unwrap();

	let plain = config("[app]\nname = \"Notes\"\nversion = \"1.4.0-beta.2\"\n");
	assert_eq!(super::display_name(&plain), "Notes");
	// Apple's version takes three integers and drops the pre-release label.
	assert_eq!(super::apple_version(&plain), "1.4.0");

	let auth = config("[app]\nname = \"Notes\"\n[auth]\napp_name = \"Commonplace\"\n");
	assert_eq!(super::display_name(&auth), "Commonplace");

	let extension = config(
		"[app]\nname = \"Notes\"\n[extension]\nname = \"Clipper\"\nbrowsers = \
		 [\"chromium\"]\n[extension.popup]\nroute = \"/popup\"\n",
	);
	assert_eq!(super::display_name(&extension), "Clipper");
}

#[test]
fn store_text_falls_back_to_the_extension_and_a_placeholder() {
	let root = tempfile::tempdir().unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		"[app]\nname = \"Notes\"\n[extension]\nname = \"Notes\"\ndescription = \"Clip \
		 quotes.\"\nbrowsers = [\"chromium\"]\n[extension.popup]\nroute = \"/popup\"\n",
		root.path(),
	)
	.unwrap();
	let with_text = super::listing::Store {
		description: Some("Store text.".to_owned()),
		listing: super::listing::Listing {
			privacy_policy: Some("https://example.com/privacy".to_owned()),
			..super::listing::Listing::default()
		},
		..super::listing::Store::default()
	};
	assert_eq!(super::description(&config, &with_text), "Store text.");
	assert_eq!(
		super::privacy_policy(&with_text),
		"https://example.com/privacy"
	);

	let empty = super::listing::Store::default();
	assert_eq!(super::description(&config, &empty), "Clip quotes.");
	assert_eq!(
		super::privacy_policy(&empty),
		"(add privacy_policy to store/listing.toml)"
	);
	let plain =
		leptoskit_config::LeptoskitConfig::from_toml("[app]\nname = \"Notes\"\n", root.path())
			.unwrap();
	assert_eq!(
		super::description(&plain, &empty),
		"(write it in store/en-US/description.md)"
	);
}

#[test]
fn browser_screenshots_count_files_or_point_at_the_capture() {
	let root = tempfile::tempdir().unwrap();
	let none = super::browser_screenshots(root.path());
	assert_eq!(none.label, "Screenshots");
	assert!(
		none.value.contains("the chrome-1280x800 size"),
		"{}",
		none.value
	);

	let folder = root.path().join("store/screenshots/chrome-1280x800");
	std::fs::create_dir_all(&folder).unwrap();
	std::fs::write(
		folder.join("01-projects.png"),
		super::images::tests::png(1280, 800, 255),
	)
	.unwrap();
	let one = super::browser_screenshots(root.path());
	assert!(one.value.contains("(1 file"), "{}", one.value);

	std::fs::write(
		folder.join("02-reading.png"),
		super::images::tests::png(1280, 800, 255),
	)
	.unwrap();
	let two = super::browser_screenshots(root.path());
	assert!(two.value.contains("(2 files"), "{}", two.value);
}

#[test]
fn image_summaries_say_what_each_store_receives() {
	assert_eq!(super::uploaded_images(&[]), "");
	assert_eq!(
		super::uploaded_images(&[one_image_set("android-phone")]),
		" Its screenshots are uploaded: 1 image for android-phone."
	);
	assert!(
		super::checked_images(&[one_image_set("iphone-6.3")])
			.contains("Its screenshots are checked: 1 image for iphone-6.3.")
	);
}

#[test]
fn app_store_text_carries_only_the_listing_it_has() {
	let store = super::listing::Store {
		description: Some("Keep notes.".to_owned()),
		summary: Some("A notebook for everything.".to_owned()),
		listing: super::listing::Listing {
			keywords: vec!["notes".to_owned(), "research".to_owned()],
			support: Some("https://example.com/support".to_owned()),
			homepage: Some("https://example.com".to_owned()),
			..super::listing::Listing::default()
		},
		..super::listing::Store::default()
	};
	insta::assert_snapshot!(serde_json::to_string_pretty(&super::app_store_text(&store)).unwrap());
	assert!(super::app_store_text(&super::listing::Store::default()).is_empty());
}

#[test]
fn export_compliance_is_read_from_the_apps_info_plist() {
	let root = tempfile::tempdir().unwrap();
	let answered = root.path().join("answered.plist");
	std::fs::write(
		&answered,
		r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>ITSAppUsesNonExemptEncryption</key><false/></dict></plist>
"#,
	)
	.unwrap();
	assert!(super::encryption_answered(&answered));

	let unanswered = root.path().join("unanswered.plist");
	std::fs::write(
		&unanswered,
		r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleName</key><string>Notes</string></dict></plist>
"#,
	)
	.unwrap();
	assert!(!super::encryption_answered(&unanswered));
	assert!(!super::encryption_answered(
		&root.path().join("missing.plist")
	));
}

#[test]
fn app_store_steps_name_the_gaps_the_api_leaves() {
	let root = tempfile::tempdir().unwrap();
	let store = super::listing::Store::default();
	let run = super::AppStoreRun {
		app: "1234567890",
		version: "1.4.0",
		build: Some(42),
		root: root.path(),
		info_plist: &root.path().join("ios/Info.plist"),
		encryption_answered: false,
		beta_review: Some("Missing export compliance"),
		submitted: false,
		store: &store,
		screenshots: &[],
	};
	let report = super::steps::Report {
		stores: vec![super::steps::Status {
			store: "app_store".to_owned(),
			summary: "Notes 1.4.0 (build 42) is in TestFlight.".to_owned(),
		}],
		steps: super::app_store_steps(&run),
	};
	insta::assert_snapshot!(report.text());
}

#[test]
fn a_submitted_app_store_run_asks_for_no_page_work() {
	let root = tempfile::tempdir().unwrap();
	let store = super::listing::Store::default();
	let run = super::AppStoreRun {
		app: "1234567890",
		version: "1.4.0",
		build: Some(42),
		root: root.path(),
		info_plist: &root.path().join("ios/Info.plist"),
		encryption_answered: true,
		beta_review: None,
		submitted: true,
		store: &store,
		screenshots: &[],
	};
	// Once submitted, the version is in review and the page's work is done.
	assert_eq!(super::app_store_steps(&run).len(), 0);
}

#[test]
fn a_complete_app_store_page_asks_for_no_text() {
	let root = tempfile::tempdir().unwrap();
	let store = super::listing::Store {
		description: Some("Keep notes.".to_owned()),
		summary: Some("A notebook for everything.".to_owned()),
		listing: super::listing::Listing {
			keywords: vec!["notes".to_owned()],
			support: Some("https://example.com/support".to_owned()),
			..super::listing::Listing::default()
		},
		..super::listing::Store::default()
	};
	let screenshots = [one_image_set("iphone-6.3"), one_image_set("ipad-13")];
	let run = super::AppStoreRun {
		app: "1234567890",
		version: "1.4.0",
		build: None,
		root: root.path(),
		info_plist: &root.path().join("ios/Info.plist"),
		encryption_answered: true,
		beta_review: None,
		submitted: false,
		store: &store,
		screenshots: &screenshots,
	};
	let steps = super::app_store_steps(&run);
	assert!(
		steps
			.iter()
			.all(|step| !step.what.contains("App Store page is missing")),
		"{steps:#?}"
	);
	assert!(
		steps
			.iter()
			.any(|step| step.what.contains("Decide to release it"))
	);
}

#[test]
fn play_steps_name_a_missing_app_and_its_listing_gaps() {
	let root = tempfile::tempdir().unwrap();
	let store = super::listing::Store::default();
	let run = super::PlayRun {
		package: "com.example.notes",
		track: "internal",
		aab: "android/app/build/outputs/bundle/release/app-release.aab",
		exists: false,
		rollout: super::play::Rollout::Draft,
		submitted: false,
		store: &store,
		root: root.path(),
		screenshots: &[],
		icon: false,
	};
	let report = super::steps::Report {
		stores: vec![super::steps::Status {
			store: "play".to_owned(),
			summary: "Notes 1.4.0 (versionCode 42) is built.".to_owned(),
		}],
		steps: super::play_steps(&run),
	};
	insta::assert_snapshot!(report.text());
}

#[test]
fn play_steps_ask_for_a_listing_only_while_it_is_missing() {
	let root = tempfile::tempdir().unwrap();
	let complete = super::listing::Store {
		summary: Some("Keep notes.".to_owned()),
		description: Some("Keep notes, everywhere.".to_owned()),
		..super::listing::Store::default()
	};
	let screenshots = [
		one_image_set("android-phone"),
		one_image_set("play-feature-graphic"),
	];
	let complete_run = super::PlayRun {
		package: "com.example.notes",
		track: "internal",
		aab: "app-release.aab",
		exists: true,
		rollout: super::play::Rollout::Draft,
		submitted: false,
		store: &complete,
		root: root.path(),
		screenshots: &screenshots,
		icon: true,
	};
	assert!(
		super::play_steps(&complete_run)
			.iter()
			.all(|step| !step.what.contains("main store listing is missing"))
	);
	assert!(
		super::play_steps(&complete_run)
			.iter()
			.any(|step| step.what.contains("Decide to release it"))
	);
}

#[test]
fn play_steps_watch_a_staged_rollout_after_submitting() {
	let root = tempfile::tempdir().unwrap();
	let store = super::listing::Store::default();
	let mut run = super::PlayRun {
		package: "com.example.notes",
		track: "production",
		aab: "app-release.aab",
		exists: true,
		rollout: super::play::Rollout::Staged(0.1),
		submitted: true,
		store: &store,
		root: root.path(),
		screenshots: &[],
		icon: true,
	};
	let steps = super::play_steps(&run);
	assert_eq!(steps.len(), 1);
	assert!(steps[0].what.contains("first 10%"), "{}", steps[0].what);

	run.rollout = super::play::Rollout::Everyone;
	assert_eq!(super::play_steps(&run).len(), 0);
}

#[test]
fn play_summaries_name_the_rollout() {
	use super::play::Rollout;

	assert_eq!(
		super::play_summary("Notes", "1.4.0", 42, "internal", Rollout::Draft),
		"Notes 1.4.0 (versionCode 42) is on the internal track as a draft release."
	);
	assert_eq!(
		super::play_summary("Notes", "1.4.0", 42, "internal", Rollout::Everyone),
		"Notes 1.4.0 (versionCode 42) is released on the internal track."
	);
	assert_eq!(
		super::play_summary("Notes", "1.4.0", 42, "production", Rollout::Staged(0.1)),
		"Notes 1.4.0 (versionCode 42) is rolling out to 10% of the production track's users."
	);
}

#[test]
fn browser_store_steps_ask_for_release_only_while_unsubmitted() {
	let root = tempfile::tempdir().unwrap();
	let config =
		leptoskit_config::LeptoskitConfig::from_toml(EXTENSION_PROJECT, root.path()).unwrap();
	let store = super::listing::Store::default();
	let manifest = serde_json::json!({
		"permissions": ["tabs", "storage"],
		"optional_host_permissions": ["https://*/*"],
	});

	let chrome = super::chrome_steps(root.path(), &config, &store, &manifest, false);
	assert!(
		chrome
			.iter()
			.any(|step| step.what.contains("Decide to release it"))
	);
	assert!(
		chrome
			.iter()
			.any(|step| step.enter.iter().any(|field| field.label == "tabs"))
	);
	assert!(chrome.iter().any(|step| {
		step.enter
			.iter()
			.any(|field| field.label == "Host permission")
	}));
	assert!(
		super::chrome_steps(root.path(), &config, &store, &manifest, true)
			.iter()
			.all(|step| !step.what.contains("Decide to release it"))
	);

	let edge = super::edge_steps(root.path(), &config, &store, false);
	assert!(
		edge.iter()
			.any(|step| step.what.contains("Decide to release it"))
	);
	assert!(
		super::edge_steps(root.path(), &config, &store, true)
			.iter()
			.all(|step| !step.what.contains("Decide to release it"))
	);
}

#[test]
fn firefox_steps_cover_a_first_submission_and_the_listing() {
	let root = tempfile::tempdir().unwrap();
	let config =
		leptoskit_config::LeptoskitConfig::from_toml(EXTENSION_PROJECT, root.path()).unwrap();
	let settings = config.publish.firefox.clone().unwrap();
	let store = super::listing::Store::default();
	// Relative paths keep the snapshot stable across checkouts.
	let zip = std::path::Path::new("firefox.zip");
	let source = std::path::Path::new("firefox-source.zip");
	let first = super::FirefoxRun {
		guid: "notes@example.com",
		settings: &settings,
		store: &store,
		submitted: false,
		exists: false,
		zip,
		source,
		root: root.path(),
		screenshots: false,
	};
	let report = super::steps::Report {
		stores: vec![super::steps::Status {
			store: "firefox".to_owned(),
			summary: "Notes 1.4.0 passed validation.".to_owned(),
		}],
		steps: super::firefox_steps(&first),
	};
	insta::assert_snapshot!(report.text());

	let exists = super::FirefoxRun {
		exists: true,
		..first
	};
	let listing = super::firefox_steps(&exists);
	assert!(
		listing
			.iter()
			.any(|step| step.what.contains("privacy policy")),
		"{listing:#?}"
	);
	assert!(
		listing
			.iter()
			.any(|step| step.what.contains("Decide to release it"))
	);

	let submitted = super::FirefoxRun {
		submitted: true,
		..exists
	};
	assert!(
		super::firefox_steps(&submitted)
			.iter()
			.all(|step| !step.what.contains("Decide to release it"))
	);

	// An unlisted add-on has no listing page at all.
	let unlisted_settings = leptoskit_config::publish::FirefoxStore {
		channel: leptoskit_config::publish::FirefoxChannel::Unlisted,
	};
	let unlisted = super::FirefoxRun {
		settings: &unlisted_settings,
		..exists
	};
	assert!(
		super::firefox_steps(&unlisted)
			.iter()
			.all(|step| !step.what.contains("privacy policy"))
	);
}

#[test]
fn a_macos_dry_run_uses_the_build_number_and_names_the_release_step() {
	let root = tempfile::tempdir().unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		"[app]\nname = \"Notes\"\nversion = \"1.4.0\"\n[publish.macos]\ngithub_release = \
		 \"example/notes\"\n",
		root.path(),
	)
	.unwrap();
	let store = super::listing::Store::default();
	let credentials = super::Credentials::default();
	let args = arguments(&["macos"], true, super::BuildNumber::Number(9));
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let session = session(
		root.path(),
		&config,
		&store,
		&credentials,
		&args,
		&http,
		&runner,
	);
	let (summary, steps) = session.macos().unwrap();
	assert!(
		summary.contains("Nothing was built or notarized"),
		"{summary}"
	);
	assert_eq!(steps.len(), 1);
	assert!(
		steps[0]
			.what
			.contains("run `leptoskit publish macos --submit`"),
		"{}",
		steps[0].what
	);
	assert!(steps[0].url.is_none());
}

#[test]
fn a_play_dry_run_explains_its_build_number() {
	let root = tempfile::tempdir().unwrap();
	write_android_host(root.path());
	let config = leptoskit_config::LeptoskitConfig::from_toml(PLAY_PROJECT, root.path()).unwrap();
	let store = super::listing::Store::default();
	let credentials = super::Credentials::default();
	let args = arguments(&["play"], true, super::BuildNumber::Number(42));
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let session = session(
		root.path(),
		&config,
		&store,
		&credentials,
		&args,
		&http,
		&runner,
	);
	let (summary, steps) = session.play().unwrap();
	assert!(summary.contains("numbered 42"), "{summary}");
	assert!(
		summary.contains("Nothing was built or uploaded"),
		"{summary}"
	);
	assert!(steps.iter().any(|step| step.store == "play"));
}

#[test]
fn a_play_dry_run_refuses_a_host_without_a_gradle_script() {
	let root = tempfile::tempdir().unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(PLAY_PROJECT, root.path()).unwrap();
	let store = super::listing::Store::default();
	let credentials = super::Credentials::default();
	let args = arguments(&["play"], true, super::BuildNumber::Auto);
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let session = session(
		root.path(),
		&config,
		&store,
		&credentials,
		&args,
		&http,
		&runner,
	);
	let error = session.play().unwrap_err().to_string();
	assert!(error.contains("could not read"), "{error}");
	assert!(error.contains("build.gradle.kts"), "{error}");
}

#[test]
fn a_play_dry_run_refuses_a_gradle_script_play_would_reject() {
	let root = tempfile::tempdir().unwrap();
	write_android_host(root.path());
	std::fs::write(
		root.path().join("android/app/build.gradle.kts"),
		"android { defaultConfig { targetSdk = 28 } }
",
	)
	.unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(PLAY_PROJECT, root.path()).unwrap();
	let store = super::listing::Store::default();
	let credentials = super::Credentials::default();
	let args = arguments(&["play"], true, super::BuildNumber::Auto);
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let session = session(
		root.path(),
		&config,
		&store,
		&credentials,
		&args,
		&http,
		&runner,
	);
	let error = session.play().unwrap_err().to_string();
	assert!(
		error.contains("Google Play can't take this app's bundle yet"),
		"{error}"
	);
	assert!(error.contains("targetSdk"), "{error}");
}

#[test]
fn a_play_dry_run_refuses_release_notes_play_would_cut() {
	let root = tempfile::tempdir().unwrap();
	write_android_host(root.path());
	std::fs::create_dir_all(root.path().join("store/en-US")).unwrap();
	std::fs::write(
		root.path().join("store/en-US/release_notes.md"),
		"x".repeat(super::play::NOTES_LIMIT + 1),
	)
	.unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(PLAY_PROJECT, root.path()).unwrap();
	let store = super::listing::Store::load(root.path()).unwrap();
	let credentials = super::Credentials::default();
	let args = arguments(&["play"], true, super::BuildNumber::Auto);
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let session = session(
		root.path(),
		&config,
		&store,
		&credentials,
		&args,
		&http,
		&runner,
	);
	let error = session.play().unwrap_err().to_string();
	assert!(error.contains("Google Play takes at most 500"), "{error}");
}

#[test]
fn play_listing_sets_the_text_images_and_icon() {
	let root = tempfile::tempdir().unwrap();
	std::fs::write(
		root.path().join("icon.png"),
		super::images::tests::png(1024, 1024, 255),
	)
	.unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		"[app]\nname = \"Notes\"\nversion = \"1.4.0\"\nicon = \"icon.png\"\n[native]\npackage_id \
		 = \"com.example.notes\"\n[publish.play]\ntrack = \"internal\"\n",
		root.path(),
	)
	.unwrap();
	let listing_store = super::listing::Store {
		summary: Some("Keep notes.".to_owned()),
		description: Some("Keep notes, everywhere.".to_owned()),
		listing: super::listing::Listing {
			name: Some("Notes Pro".to_owned()),
			..super::listing::Listing::default()
		},
		..super::listing::Store::default()
	};
	let screenshots = [one_image_set("android-phone")];
	let icon = crate::icons::Icon::load(&root.path().join("icon.png")).unwrap();

	let app = "/androidpublisher/v3/applications/com.example.notes";
	let (base, seen) = store(Box::new(move |request, count| {
		let path = request.path.as_str();
		match (request.method.as_str(), path) {
			("POST", "/token") => json(serde_json::json!({ "access_token": "t" })),
			("POST", path) if path.ends_with("/edits") => json(serde_json::json!({ "id": "e1" })),
			("PUT", path) if path.ends_with("/listings/en-US") => json(serde_json::json!({})),
			("DELETE", path) if path.ends_with("/phoneScreenshots") || path.ends_with("/icon") => {
				json(serde_json::json!({ "deleted": [] }))
			}
			("POST", path) if path.ends_with("/icon") && count > 6 => {
				(StatusCode::INTERNAL_SERVER_ERROR, "icon refused").into_response()
			}
			("POST", path) if path.ends_with("/phoneScreenshots") || path.ends_with("/icon") => {
				json(serde_json::json!({ "image": { "id": "i" } }))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let account = google_account();
	let play = super::play::Play::connect(
		&http,
		&base,
		&format!("{base}/token"),
		&account,
		"com.example.notes",
	)
	.unwrap();
	let edit = play.edit().unwrap().unwrap();
	let credentials = super::Credentials::default();
	let args = arguments(&["play"], false, super::BuildNumber::Auto);
	let runner = super::tools::Recorder::default();
	let publisher = session(
		root.path(),
		&config,
		&listing_store,
		&credentials,
		&args,
		&http,
		&runner,
	);
	publisher
		.play_listing(&edit, &screenshots, Some(&icon))
		.unwrap();
	// Without both descriptions, the listing's text isn't touched.
	let untitled_store = super::listing::Store::default();
	let untitled = session(
		root.path(),
		&config,
		&untitled_store,
		&credentials,
		&args,
		&http,
		&runner,
	);
	untitled.play_listing(&edit, &[], None).unwrap();
	let error = publisher
		.play_listing(&edit, &[], Some(&icon))
		.unwrap_err()
		.to_string();
	assert!(error.contains("icon refused"), "{error}");
	drop(edit);

	let seen = seen.lock().unwrap();
	assert_eq!(seen[2].path, format!("{app}/edits/e1/listings/en-US"));
	let listing: serde_json::Value = serde_json::from_slice(&seen[2].body).unwrap();
	assert_eq!(listing["title"], "Notes Pro");
	assert_eq!(listing["shortDescription"], "Keep notes.");
	assert_eq!(seen[3].method, "DELETE");
	assert_eq!(
		seen[3].path,
		format!("{app}/edits/e1/listings/en-US/phoneScreenshots")
	);
	assert_eq!(seen[4].body, screenshots[0].images[0].bytes);
	assert_eq!(seen[5].method, "DELETE");
	assert!(seen[5].path.ends_with("/listings/en-US/icon"));
	assert_eq!(seen[6].content_type, "image/png");
	assert_eq!(seen[6].body, icon.png(512).unwrap());
}

#[test]
fn an_unknown_store_is_not_published() {
	let root = tempfile::tempdir().unwrap();
	let config =
		leptoskit_config::LeptoskitConfig::from_toml("[app]\nname = \"Notes\"\n", root.path())
			.unwrap();
	let store = super::listing::Store::default();
	let credentials = super::Credentials::default();
	let args = arguments(&[], true, super::BuildNumber::Auto);
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let session = session(
		root.path(),
		&config,
		&store,
		&credentials,
		&args,
		&http,
		&runner,
	);
	let error = session.publish("figma").unwrap_err().to_string();
	assert!(
		error.contains("`figma` isn't a store leptoskit publish publishes to"),
		"{error}"
	);
}

#[test]
fn a_missing_browser_package_names_the_build() {
	let root = tempfile::tempdir().unwrap();
	let config =
		leptoskit_config::LeptoskitConfig::from_toml(EXTENSION_PROJECT, root.path()).unwrap();
	let store = super::listing::Store::default();
	let credentials = super::Credentials::default();
	let args = arguments(&["chrome"], true, super::BuildNumber::Auto);
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let built = super::Extension {
		built: crate::extension::Built {
			packages: vec![crate::extension::Package {
				browser: leptoskit_config::Browser::Chromium,
				dir: root.path().join("chromium"),
				zip: Some(root.path().join("chromium.zip")),
			}],
			source: None,
		},
		manifest: serde_json::json!({}),
	};
	let session = super::Session {
		args: &args,
		root: root.path(),
		config: &config,
		credentials: &credentials,
		store: &store,
		http: &http,
		runner: &runner,
		extension: Some(&built),
		name: "Notes".to_owned(),
		version: "1.4.0".to_owned(),
		submit: false,
	};
	let error = session
		.package(leptoskit_config::Browser::Firefox)
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("the build made no firefox package"),
		"{error}"
	);
	assert!(error.contains("[extension].browsers"), "{error}");
}
