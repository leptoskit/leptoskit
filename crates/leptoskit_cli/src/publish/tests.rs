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
			("DELETE", path)
				if path.ends_with("/listings/en-US/phoneScreenshots")
					|| path.ends_with("/listings/en-US/icon") =>
			{
				json(serde_json::json!({ "deleted": [] }))
			}
			("POST", path)
				if path.ends_with("/listings/en-US/phoneScreenshots")
					|| path.ends_with("/listings/en-US/icon") =>
			{
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
		.env("PATH", std::env::var_os("PATH").unwrap_or_default());

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

/// The server answers addons.mozilla.org's happy path, through the upload's
/// validation to a submitted version.
fn firefox_success() -> Answer {
	Box::new(|request, count| {
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
			("GET", "/addons/addon/notes@example.com/") => {
				json(serde_json::json!({ "id": 9, "previews": [] }))
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
	})
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
fn firefox_session_dry_runs_unlisted_and_names_a_missing_package() {
	// The unlisted channel sends no listing, so a dry run checks the package
	// only; and a build without the Firefox package is named.
	spawn_publish("firefox-unlisted", &[], None);
	spawn_publish("firefox-no-package", &[], None);
}

#[test]
fn firefox_session_reports_the_first_submission_and_other_refusals() {
	// A submitted version for an add-on addons.mozilla.org has never heard
	// of: the run asks for its first submission on the site.
	let requests = firefox_session(
		"firefox-first",
		Box::new(|request, _| {
			match (request.method.as_str(), request.path.as_str()) {
				("POST", "/addons/upload/") => {
					json(serde_json::json!({ "uuid": "u-1", "processed": false }))
				}
				("GET", "/addons/upload/u-1/") => {
					json(serde_json::json!({ "uuid": "u-1", "processed": true, "valid": true }))
				}
				_ => StatusCode::NOT_FOUND.into_response(),
			}
		}),
	);
	assert!(
		requests
			.iter()
			.any(|request| request.path == "/addons/addon/notes@example.com/versions/"),
		"{requests:?}"
	);

	// A version addons.mozilla.org refuses for another reason fails the run.
	let requests = firefox_session(
		"firefox-submit-refused",
		Box::new(|request, _| {
			match (request.method.as_str(), request.path.as_str()) {
				("POST", "/addons/upload/") => {
					json(serde_json::json!({ "uuid": "u-1", "processed": false }))
				}
				("GET", "/addons/upload/u-1/") => {
					json(serde_json::json!({ "uuid": "u-1", "processed": true, "valid": true }))
				}
				("POST", "/addons/addon/notes@example.com/versions/") => {
					(StatusCode::BAD_REQUEST, "the version is not newer").into_response()
				}
				_ => StatusCode::NOT_FOUND.into_response(),
			}
		}),
	);
	assert_eq!(
		requests.last().unwrap().path,
		"/addons/addon/notes@example.com/versions/"
	);
}

#[test]
fn firefox_session_uploads_the_listing_screenshots_as_previews() {
	let requests = firefox_session("firefox-listing", firefox_success());
	assert!(
		requests
			.iter()
			.any(|request| request.path == "/addons/addon/notes@example.com/previews/")
	);
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

/// The child side of [`spawn_publish`]: builds a [`super::Session`] from
/// packages and a manifest, or calls [`super::run`] in the project the
/// parent made the current directory. A normal test run has no scenario and
/// does nothing.
fn session_child(scenario: &str) {
	let (store_name, submit, dry_run) = match scenario {
		"chrome-draft" | "chrome-error" => ("chrome", false, false),
		"chrome-submit" => ("chrome", true, false),
		"edge-draft" | "edge-error" => ("edge", false, false),
		"edge-submit" => ("edge", true, false),
		"firefox-draft" | "firefox-error" => ("firefox", false, false),
		"firefox-submit" | "firefox-first" | "firefox-submit-refused" | "firefox-listing" => {
			("firefox", true, false)
		}
		"firefox-unlisted" | "firefox-no-package" => ("firefox", false, true),
		other => panic!("unknown session scenario {other}"),
	};

	let root = tempfile::tempdir().unwrap();

	for (name, bytes) in [
		("chromium.zip", b"PK chromium".as_slice()),
		("firefox.zip", b"PK firefox".as_slice()),
		("firefox-source.zip", b"PK source".as_slice()),
	] {
		std::fs::write(root.path().join(name), bytes).unwrap();
	}

	// The listing's screenshots, which a listed submit uploads as previews.
	if scenario == "firefox-listing" {
		let directory = root.path().join("store/screenshots/chrome-1280x800");
		std::fs::create_dir_all(&directory).unwrap();
		std::fs::write(
			directory.join("01-projects.png"),
			super::images::tests::png(1280, 800, 255),
		)
		.unwrap();
	}

	let mut config =
		leptoskit_config::LeptoskitConfig::from_toml(EXTENSION_PROJECT, root.path()).unwrap();

	if scenario == "firefox-unlisted" {
		config.publish.firefox.as_mut().unwrap().channel =
			leptoskit_config::publish::FirefoxChannel::Unlisted;
	}

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
	let mut packages = vec![package(leptoskit_config::Browser::Chromium, "chromium.zip")];

	if scenario != "firefox-no-package" {
		packages.push(package(leptoskit_config::Browser::Firefox, "firefox.zip"));
	}

	let extension = super::Extension {
		built: crate::extension::Built {
			packages,
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

	if scenario == "firefox-no-package" {
		let error = result.unwrap_err().to_string();
		assert!(
			error.contains("the build made no firefox package"),
			"{error}"
		);

		return;
	}

	if scenario.ends_with("-error") || scenario == "firefox-submit-refused" {
		let error = result.unwrap_err().to_string();

		let expected = match scenario {
			"chrome-error" => "manifest version must be newer",
			"edge-error" => "72 days",
			"firefox-error" => "manifest.json is invalid",
			"firefox-submit-refused" => "the version is not newer",
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
		"firefox-listing" => {
			"Notes 1.4.0 is submitted (listed channel). Its screenshots are uploaded: 1 image for \
			 chrome-1280x800."
		}
		"firefox-first" => {
			"Notes 1.4.0 passed validation; the add-on needs its first submission on the site."
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

	// The first-submission run asks for the site steps; the unlisted channel
	// has no listing to edit, so its steps never ask for the privacy policy.
	if scenario == "firefox-first" {
		assert!(
			steps
				.iter()
				.any(|step| step.what.contains("first version on the site"))
		);
		assert!(
			steps
				.iter()
				.any(|step| step.what.contains("Choose the add-on's license"))
		);
	}

	if scenario == "firefox-unlisted" {
		assert!(
			summary == expected
				&& !steps
					.iter()
					.any(|step| step.what.contains("privacy policy"))
		);
	}
}

/// The child side of [`run_scenario`]: `super::run` must reject each bad
/// configuration before it builds the extension or contacts a store. The
/// macOS dry runs go through [`run_macos_child`] instead: they succeed.
fn run_child(scenario: &str) {
	if scenario.starts_with("run-macos") {
		run_macos_child(scenario);

		return;
	}

	if scenario == "run-confirm-skipped" {
		confirm_child();

		return;
	}

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
		"run-missing-credential" => super::run(&args(vec!["chrome".to_owned()], None)).unwrap_err(),
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
	} else {
		session_child(&scenario);
	}
}

// The publish pipeline's own parsing, validation, and step writing, which
// the store clients and the dry runs above don't reach.

#[test]
fn build_numbers_parse_or_explain_themselves() {
	assert_eq!(
		"auto".parse::<super::BuildNumber>(),
		Ok(super::BuildNumber::Auto)
	);
	assert_eq!(
		"42".parse::<super::BuildNumber>(),
		Ok(super::BuildNumber::Number(42))
	);
	let error = "0".parse::<super::BuildNumber>().unwrap_err();
	assert!(error.contains("whole number above 0"), "{error}");

	let error = "next".parse::<super::BuildNumber>().unwrap_err();
	assert!(error.contains("`auto` or a whole number"), "{error}");
}

#[test]
fn the_api_override_variable_wins_over_the_default() {
	// A variable no test sets: the store's own address stands.
	assert_eq!(
		super::api("LEPTOSKIT_PUBLISH_TEST_UNSET_API", "https://store.example"),
		"https://store.example"
	);
}

#[test]
fn names_fall_back_from_the_extension_to_the_app() {
	let config = |toml: &str| {
		leptoskit_config::LeptoskitConfig::from_toml(toml, std::path::Path::new(".")).unwrap()
	};
	let named = config(
		"[app]\nname = \"Notes\"\nversion = \"1.4.0\"\nplatforms = [\"web\"]\n\n[extension]\nname \
		 = \"Notes for Chrome\"\nbrowsers = [\"chromium\"]\n\n[extension.popup]\nroute = \"/\"\n",
	);
	assert_eq!(super::display_name(&named), "Notes for Chrome");

	let auth = config(
		"[app]\nname = \"notes\"\nversion = \"1.4.0\"\nplatforms = [\"web\"]\n\n[auth]\napp_name \
		 = \"Notes\"\n",
	);
	assert_eq!(super::display_name(&auth), "Notes");

	let bare = config("[app]\nname = \"notes\"\nversion = \"1.4.0\"\nplatforms = [\"web\"]\n");
	assert_eq!(super::display_name(&bare), "notes");

	// Apple's version takes at most three integers, so a pre-release label
	// drops.
	let prerelease = config(
		"[app]\nname = \"notes\"\nversion = \"1.4.0-beta.2+local\"\nplatforms = [\"web\"]\n",
	);
	assert_eq!(super::apple_version(&prerelease), "1.4.0");
}

#[test]
fn confirm_skips_its_question_when_answered_up_front() {
	let args = |yes: bool| {
		super::PublishArgs {
			stores: vec!["chrome".to_owned()],
			dry_run: false,
			submit: true,
			staged: false,
			yes,
			json: false,
			credentials: None,
			build_number: super::BuildNumber::Auto,
		}
	};
	assert!(super::confirm(&args(true), "Notes", "1.4.0", &["chrome"]).unwrap());
}

/// The child side of the confirm scenario: its stdin is closed, so the
/// question is skipped and everything is submitted.
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

#[test]
fn play_summaries_name_each_rollout() {
	assert_eq!(
		super::play_summary(
			"Notes",
			"1.4.0",
			42,
			"internal",
			super::play::Rollout::Draft
		),
		"Notes 1.4.0 (versionCode 42) is on the internal track as a draft release."
	);
	assert_eq!(
		super::play_summary(
			"Notes",
			"1.4.0",
			42,
			"production",
			super::play::Rollout::Everyone
		),
		"Notes 1.4.0 (versionCode 42) is released on the production track."
	);
	assert_eq!(
		super::play_summary(
			"Notes",
			"1.4.0",
			42,
			"production",
			super::play::Rollout::Staged(0.1),
		),
		"Notes 1.4.0 (versionCode 42) is rolling out to 10% of the production track's users."
	);
}

/// A Google Play run with every piece of listing text and imagery present.
fn play_run<'a>(
	root: &'a std::path::Path,
	store: &'a super::listing::Store,
	rollout: super::play::Rollout,
	exists: bool,
	submitted: bool,
	icon: bool,
) -> super::PlayRun<'a> {
	super::PlayRun {
		package: "com.example.notes",
		track: "internal",
		aab: ".leptoskit/publish/android/app.aab",
		exists,
		rollout,
		submitted,
		store,
		root,
		screenshots: &[],
		icon,
	}
}

#[test]
fn play_steps_ask_for_what_the_api_cannot_set() {
	let root = tempfile::tempdir().unwrap();
	let empty = super::listing::Store::default();

	// A first upload: the app doesn't exist on Google Play yet.
	let steps = super::play_steps(&play_run(
		root.path(),
		&empty,
		super::play::Rollout::Draft,
		false,
		false,
		false,
	));
	assert!(
		steps[0].what.contains("Create the app com.example.notes"),
		"{steps:?}"
	);
	assert_eq!(
		steps[0].url.as_deref(),
		Some("https://play.google.com/console")
	);
	assert!(
		steps
			.iter()
			.any(|step| step.what.contains("Decide to release"))
	);

	// Every listing piece missing, before the first upload.
	let steps = super::play_steps(&play_run(
		root.path(),
		&empty,
		super::play::Rollout::Draft,
		true,
		false,
		false,
	));
	let listing = steps
		.iter()
		.find(|step| step.what.contains("main store listing"))
		.expect("a listing step");
	let labels: Vec<&str> = listing
		.enter
		.iter()
		.map(|field| field.label.as_str())
		.collect();
	assert_eq!(
		labels,
		[
			"Icon",
			"Feature graphic",
			"Phone screenshots",
			"Short description",
			"Full description"
		]
	);

	// With everything present and a submitted staged rollout: only the
	// rollout watch remains, and nothing asks for the listing.
	let full = super::listing::Store {
		summary: Some("Keep notes.".to_owned()),
		description: Some("Keep notes, everywhere.".to_owned()),
		..super::listing::Store::default()
	};
	let staged = super::play::Rollout::Staged(0.1);
	let steps = super::play_steps(&play_run(root.path(), &full, staged, true, true, true));
	assert_eq!(steps.len(), 1);
	assert!(
		steps[0].what.contains("Watch the first 10% of users"),
		"{steps:?}"
	);
	assert!(
		steps[0]
			.what
			.contains("raise the rollout in Play Console, or halt it")
	);

	// Submitted for everyone: the run ends at the release, with no rollout
	// to watch.
	let steps = super::play_steps(&play_run(
		root.path(),
		&full,
		super::play::Rollout::Everyone,
		true,
		true,
		true,
	));
	assert_eq!(steps.len(), 0);

	// A complete listing: no listing step, only the privacy policy, the
	// declarations, and the release decision.
	let phone = sets(&["android-phone", "play-feature-graphic"]);
	let run = super::PlayRun {
		package: "com.example.notes",
		track: "internal",
		aab: ".leptoskit/publish/android/app.aab",
		exists: true,
		rollout: super::play::Rollout::Draft,
		submitted: false,
		store: &full,
		root: root.path(),
		screenshots: &phone,
		icon: true,
	};
	let steps = super::play_steps(&run);
	assert_eq!(steps.len(), 3);
	assert!(
		!steps
			.iter()
			.any(|step| step.what.contains("main store listing"))
	);
	assert!(steps[0].what.contains("Set the privacy policy"));
}

#[test]
fn app_store_text_carries_the_versions_fields() {
	let mut store = super::listing::Store::default();
	assert_eq!(super::app_store_text(&store).len(), 0);

	store.description = Some("Keep notes, everywhere.".to_owned());
	store.listing.keywords = vec!["notes".to_owned(), "research".to_owned()];
	store.summary = Some("Keep notes.".to_owned());
	store.listing.support = Some("https://example.com/support".to_owned());
	store.listing.homepage = Some("https://example.com".to_owned());
	let text = super::app_store_text(&store);
	assert_eq!(text.len(), 5);
	assert_eq!(text["description"], "Keep notes, everywhere.");

	assert_eq!(text["keywords"], "notes,research");
	assert_eq!(text["promotionalText"], "Keep notes.");
	assert_eq!(text["supportUrl"], "https://example.com/support");
	assert_eq!(text["marketingUrl"], "https://example.com");
}

#[test]
fn encryption_compliance_is_answered_by_the_info_plist() {
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

	let garbage = root.path().join("garbage.plist");
	std::fs::write(&garbage, "not a plist").unwrap();
	assert!(!super::encryption_answered(&garbage));
}

/// An App Store run for `root` and its app-owned `Info.plist`.
fn app_store_run<'a>(
	root: &'a std::path::Path,
	info_plist: &'a std::path::Path,
	store: &'a super::listing::Store,
	screenshots: &'a [super::images::Set],
) -> super::AppStoreRun<'a> {
	super::AppStoreRun {
		app: "1234567890",
		version: "1.4.0",
		build: Some(42),
		root,
		info_plist,
		encryption_answered: super::encryption_answered(info_plist),
		beta_review: None,
		submitted: false,
		store,
		screenshots,
	}
}

#[test]
fn app_store_steps_report_what_is_still_left() {
	let root = tempfile::tempdir().unwrap();
	let empty = super::listing::Store::default();

	// Submitted, with beta review refused and compliance unanswered: exactly
	// those two steps, and nothing about the listing.
	let info_plist = root.path().join("ios/Info.plist");
	let mut run = app_store_run(root.path(), &info_plist, &empty, &[]);
	run.submitted = true;
	run.beta_review = Some("Missing export compliance");
	let steps = super::app_store_steps(&run);
	assert_eq!(steps.len(), 2);
	assert!(
		steps[0]
			.what
			.contains("Submit build 42 for TestFlight's beta review")
	);
	assert!(
		steps[0].enter[0]
			.value
			.contains("Missing export compliance")
	);

	assert!(
		steps[1]
			.what
			.contains("Answer the export compliance question for build 42")
	);
	assert!(
		steps[1].enter[0]
			.value
			.contains("ITSAppUsesNonExemptEncryption")
	);
	assert!(
		steps[1].enter[0]
			.value
			.contains("every later build carries the answer")
	);

	// Compliance answered in the plist: that step disappears.
	std::fs::create_dir_all(root.path().join("ios")).unwrap();
	std::fs::write(
		root.path().join("ios/Info.plist"),
		r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>ITSAppUsesNonExemptEncryption</key><false/></dict></plist>
"#,
	)
	.unwrap();
	let run = app_store_run(root.path(), &info_plist, &empty, &[]);
	let steps = super::app_store_steps(&run);
	assert!(
		!steps
			.iter()
			.any(|step| step.what.contains("export compliance"))
	);

	// Not submitted, an empty store, and no screenshots: the page step lists
	// everything, and the category step asks for the category and the
	// privacy policy.
	let steps = super::app_store_steps(&run);
	let page = steps
		.iter()
		.find(|step| step.what.contains("1.4.0's App Store page is missing"))
		.expect("a page step");
	assert!(
		page.enter
			.iter()
			.any(|field| field.label == "iPhone screenshots")
	);
	assert!(
		page.enter
			.iter()
			.any(|field| field.label == "iPad screenshots")
	);
	assert!(page.enter.iter().any(|field| field.label == "Description"));
	assert!(page.enter.iter().any(|field| field.label == "Keywords"));
	assert!(page.enter.iter().any(|field| field.label == "Support URL"));
	let category = steps
		.iter()
		.find(|step| step.what.contains("Choose the category"))
		.expect("a category step");

	assert!(
		category
			.enter
			.iter()
			.any(|field| field.value.contains("add category to store/listing.toml"))
	);
	assert!(category.enter.iter().any(|field| {
		field
			.value
			.contains("add privacy_policy to store/listing.toml")
	}));
	assert!(
		steps
			.iter()
			.any(|step| step.what.contains("App Privacy questions"))
	);
	assert!(steps.iter().any(|step| step.what.contains("age rating")));
	assert!(
		steps
			.iter()
			.any(|step| step.what.contains("Give App Review a contact"))
	);
	assert!(
		steps
			.iter()
			.any(|step| step.what.contains("Decide to release it"))
	);

	// A full store with both screenshot sizes has no page asks.
	let full = super::listing::Store {
		listing: super::listing::Listing {
			category: Some("Productivity".to_owned()),
			privacy_policy: Some("https://example.com/privacy".to_owned()),
			keywords: vec!["notes".to_owned()],
			support: Some("https://example.com/support".to_owned()),
			..super::listing::Listing::default()
		},
		description: Some("Keep notes, everywhere.".to_owned()),
		..super::listing::Store::default()
	};
	let both = sets(&["iphone-6.3", "ipad-13"]);
	let steps = super::app_store_steps(&app_store_run(root.path(), &info_plist, &full, &both));
	assert!(
		!steps
			.iter()
			.any(|step| step.what.contains("page is missing"))
	);
	let category = steps
		.iter()
		.find(|step| step.what.contains("Choose the category"))
		.expect("a category step");
	assert!(
		category
			.enter
			.iter()
			.any(|field| field.value == "Productivity")
	);
	assert!(
		!category
			.enter
			.iter()
			.any(|field| field.label == "Privacy Policy URL")
	);
}

/// One empty image set per store size, enough for the page and step
/// builders, which only look at folders.
fn sets(folders: &[&str]) -> Vec<super::images::Set> {
	folders
		.iter()
		.map(|folder| {
			super::images::Set {
				target: super::images::TARGETS
					.iter()
					.find(|target| target.folder == *folder)
					.unwrap(),
				images: Vec::new(),
			}
		})
		.collect()
}

#[test]
fn app_store_page_counts_screenshot_folders() {
	let root = tempfile::tempdir().unwrap();
	let store = super::listing::Store::default();
	let info_plist = root.path().join("ios/Info.plist");
	let iphone = sets(&["iphone-6.3"]);
	let run = app_store_run(root.path(), &info_plist, &store, &iphone);
	let page = super::app_store_page(&run);
	assert!(!page.iter().any(|field| field.label == "iPhone screenshots"));
	let ipad = page
		.iter()
		.find(|field| field.label == "iPad screenshots")
		.expect("an iPad ask");

	assert!(
		ipad.value.contains("add the ipad-13 size"),
		"{}",
		ipad.value
	);

	// Without screenshots, both sizes are asked for, the iPhone one naming
	// the size App Store Connect requires.
	let run = app_store_run(root.path(), &info_plist, &store, &[]);
	let page = super::app_store_page(&run);
	assert!(page.iter().any(|field| field.label == "iPhone screenshots"));
	assert!(
		page.iter()
			.any(|field| field.value.contains("add the iphone-6.3 size"))
	);

	let both = sets(&["iphone-6.3", "ipad-13"]);
	let run = app_store_run(root.path(), &info_plist, &store, &both);
	assert!(
		super::app_store_page(&run)
			.iter()
			.all(|field| field.label != "iPad screenshots")
	);
}

/// A Firefox run for `root`, with the store's add-on existing or not.
#[expect(
	clippy::too_many_arguments,
	reason = "each argument is an independent part of the run"
)]
fn firefox_run<'a>(
	settings: &'a leptoskit_config::publish::FirefoxStore,
	store: &'a super::listing::Store,
	zip: &'a std::path::Path,
	source: &'a std::path::Path,
	root: &'a std::path::Path,
	exists: bool,
	submitted: bool,
	screenshots: bool,
) -> super::FirefoxRun<'a> {
	super::FirefoxRun {
		guid: "notes@example.com",
		settings,
		store,
		submitted,
		exists,
		zip,
		source,
		root,
		screenshots,
	}
}

#[test]
fn firefox_steps_differ_by_channel_existence_and_submission() {
	let root = tempfile::tempdir().unwrap();
	let zip = root.path().join("firefox.zip");
	let source = root.path().join("firefox-source.zip");
	let store = super::listing::Store {
		summary: Some("Keep notes.".to_owned()),
		description: Some("Keep notes, everywhere.".to_owned()),
		..super::listing::Store::default()
	};

	// A first submission on the listed channel: the site steps, the listing
	// step with its screenshot ask, and the release decision.
	let listed = leptoskit_config::publish::FirefoxStore::default();
	let steps = super::firefox_steps(&firefox_run(
		&listed,
		&store,
		&zip,
		&source,
		root.path(),
		false,
		false,
		false,
	));
	assert!(steps[0].what.contains("first version on the site"));
	assert!(
		steps[0]
			.enter
			.iter()
			.any(|field| field.label == "Source code")
	);
	assert!(steps[1].what.contains("Choose the add-on's license"));
	assert!(steps[2].what.contains("Add the listing's privacy policy"));
	assert!(
		steps[2]
			.enter
			.iter()
			.any(|field| field.label == "Screenshots")
	);
	assert!(
		steps[2]
			.enter
			.iter()
			.any(|field| field.label == "Screenshots")
	);

	assert!(
		steps
			.last()
			.is_some_and(|step| step.what.contains("Decide to release it"))
	);

	// Once the add-on exists, the site steps go; screenshots present, no
	// screenshot ask.
	let steps = super::firefox_steps(&firefox_run(
		&listed,
		&store,
		&zip,
		&source,
		root.path(),
		true,
		false,
		true,
	));
	assert!(!steps.iter().any(|step| step.what.contains("first version")));
	assert!(!steps.iter().any(|step| step.what.contains("license")));
	let listing = steps
		.iter()
		.find(|step| step.what.contains("privacy policy"))
		.unwrap();
	assert!(
		!listing
			.enter
			.iter()
			.any(|field| field.label == "Screenshots")
	);
	assert!(
		steps
			.last()
			.is_some_and(|step| step.what.contains("Decide to release it"))
	);

	// Unlisted: no listing step at all.
	let unlisted = leptoskit_config::publish::FirefoxStore {
		channel: leptoskit_config::publish::FirefoxChannel::Unlisted,
	};
	let steps = super::firefox_steps(&firefox_run(
		&unlisted,
		&store,
		&zip,
		&source,
		root.path(),
		true,
		false,
		true,
	));
	assert!(
		!steps
			.iter()
			.any(|step| step.what.contains("privacy policy"))
	);
	assert!(
		steps
			.last()
			.is_some_and(|step| step.what.contains("Decide to release it"))
	);

	// Submitted: no release decision.
	let steps = super::firefox_steps(&firefox_run(
		&listed,
		&store,
		&zip,
		&source,
		root.path(),
		true,
		true,
		true,
	));
	assert!(
		!steps
			.iter()
			.any(|step| step.what.contains("Decide to release it"))
	);
}

#[test]
fn chrome_steps_ask_for_the_listing_and_justify_each_permission() {
	let root = tempfile::tempdir().unwrap();
	std::fs::create_dir_all(root.path().join("store/en-US")).unwrap();
	std::fs::write(
		root.path().join("store/permissions.toml"),
		"tabs = \"Reads your tabs.\"\n",
	)
	.unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		"[app]\nname = \"Notes\"\nversion = \"1.4.0\"\nplatforms = [\"web\", \
		 \"extension\"]\n\n[extension]\nname = \"Notes\"\ndescription = \"Keep notes.\"\nbrowsers \
		 = [\"chromium\"]\n\n\t	 [extension.popup]\nroute = \
		 \"/\"\n\n[publish.chrome]\npublisher_id = \"pub-1\"\nitem_id = \
		 \"abcdefghijklmnopabcdefghijklmnop\"\n",
		root.path(),
	)
	.unwrap();
	let store = super::listing::Store::load(root.path()).unwrap();
	let manifest = serde_json::json!({
		"permissions": ["tabs", "activeTab"],
		"host_permissions": ["https://example.com/*"],
	});

	let steps = super::chrome_steps(root.path(), &config, &store, &manifest, false);
	assert_eq!(steps.len(), 4);
	let listing = &steps[0];
	assert!(listing.what.contains("store listing"));
	assert!(
		listing
			.enter
			.iter()
			.any(|field| field.label == "Description" && field.value == "Keep notes.")
	);
	assert!(listing.enter.iter().any(|field| {
		field.label == "Screenshots" && field.value.contains("add the chrome-1280x800 size")
	}));

	let privacy = &steps[1];
	assert!(privacy.what.contains("privacy practices"));
	assert!(
		privacy
			.enter
			.iter()
			.any(|field| field.label == "Single purpose" && field.value == "Keep notes.")
	);
	assert!(
		privacy
			.enter
			.iter()
			.any(|field| field.label == "tabs" && field.value == "Reads your tabs.")
	);
	assert!(privacy.enter.iter().any(|field| {
		field.label == "activeTab" && field.value.contains("write one sentence for `activeTab`")
	}));
	assert!(
		privacy
			.enter
			.iter()
			.any(|field| field.label == "Host permission")
	);
	assert!(privacy.enter.iter().any(|field| {
		field.label == "Privacy policy"
			&& field.value == "(add privacy_policy to store/listing.toml)"
	}));
	assert!(steps[3].what.contains("Decide to release it"));

	// Submitted: the release decision goes.
	let steps = super::chrome_steps(root.path(), &config, &store, &manifest, true);
	assert_eq!(steps.len(), 3);

	// A manifest without permissions has no justifications for them.
	let steps = super::chrome_steps(root.path(), &config, &store, &serde_json::json!({}), true);
	let privacy = &steps[1];
	assert!(
		!privacy
			.enter
			.iter()
			.any(|field| field.label == "Host permission")
	);
	assert!(!privacy.enter.iter().any(|field| field.label == "tabs"));
}

#[test]
fn edge_steps_ask_for_properties_then_the_release() {
	let root = tempfile::tempdir().unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		"[app]\nname = \"Notes\"\nversion = \"1.4.0\"\nplatforms = [\"web\", \
		 \"extension\"]\n\n[extension]\nname = \"Notes\"\nbrowsers = [\"chromium\"]\n\n\t	 \
		 [extension.popup]\nroute = \"/\"\n\n[publish.edge]\nproduct_id = \"prod-1\"\n",
		root.path(),
	)
	.unwrap();
	let store = super::listing::Store::default();
	let steps = super::edge_steps(root.path(), &config, &store, false);
	assert_eq!(steps.len(), 2);
	assert!(steps[0].what.contains("Properties"));
	assert_eq!(
		steps[0].url.as_deref(),
		Some("https://partner.microsoft.com/dashboard/microsoftedge/prod-1/properties")
	);
	assert!(steps[0].enter.iter().any(|field| {
		field.label == "Description" && field.value == "(write it in store/en-US/description.md)"
	}));

	assert!(steps[1].what.contains("Decide to release it"));

	let steps = super::edge_steps(root.path(), &config, &store, true);
	assert_eq!(steps.len(), 1);
}

#[test]
fn credentials_listing_says_what_is_missing_and_who_makes_it() {
	let listed = super::list_credentials(&["chrome"], &super::Credentials::default());
	assert!(listed.contains("Credentials for chrome:"), "{listed}");
	assert!(listed.contains("missing"));
	assert!(listed.contains("To make it:"));
	assert!(listed.contains("Values are never printed."));
}

#[test]
fn capture_says_how_to_make_the_screenshots() {
	let root = tempfile::tempdir().unwrap();
	let text = super::capture(root.path(), &["android-phone"], "the android-phone size");
	assert!(
		text.contains("add the android-phone size to store/screenshots/screenshots.toml"),
		"{text}"
	);

	let plan = root.path().join("store/screenshots/screenshots.toml");
	std::fs::create_dir_all(plan.parent().unwrap()).unwrap();
	std::fs::write(
		&plan,
		"[devices.android-phone]\nandroid = \
		 \"emulator-5554\"\n\n[devices.\"iphone-6.9\"]\nsimulator = \"iPhone 17 Pro Max\"\n",
	)
	.unwrap();
	let text = super::capture(root.path(), &["android-phone"], "the android-phone size");
	assert!(
		text.contains("run `leptoskit publish screenshots`, which captures android-phone"),
		"{text}"
	);

	// A prefix matches the folder that starts with it.
	let text = super::capture(root.path(), &["iphone-"], "the iphone-6.9 size");
	assert!(text.contains("captures iphone-6.9"), "{text}");
}

#[test]
fn image_summaries_describe_what_was_checked_and_uploaded() {
	let root = super::images::tests::project(&[
		(
			"chrome-1280x800/01-projects.png",
			super::images::tests::png(1280, 800, 255),
		),
		(
			"chrome-1280x800/02-reading.png",
			super::images::tests::png(1280, 800, 255),
		),
	]);
	let sets = super::images::load(root.path(), "firefox").unwrap();

	assert_eq!(super::checked_images(&[]), "");
	assert_eq!(super::uploaded_images(&[]), "");
	assert_eq!(
		super::checked_images(&sets),
		" Its screenshots are checked: 2 images for chrome-1280x800."
	);
	assert_eq!(
		super::uploaded_images(&sets),
		" Its screenshots are uploaded: 2 images for chrome-1280x800."
	);

	// The browser stores' step counts the folder's files.
	let field = super::browser_screenshots(root.path());
	assert_eq!(field.label, "Screenshots");
	assert!(
		field
			.value
			.contains("upload store/screenshots/chrome-1280x800/*.png (2 files")
	);
	assert!(field.value.contains("`leptoskit publish screenshots`"));
}

#[test]
fn description_and_privacy_policy_fall_back_to_what_to_write() {
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		"[app]\nname = \"Notes\"\nversion = \"1.4.0\"\nplatforms = [\"web\", \
		 \"extension\"]\n\n[extension]\nname = \"Notes\"\ndescription = \"Keep notes.\"\nbrowsers \
		 = [\"chromium\"]\n\n\t	 [extension.popup]\nroute = \"/\"\n",
		std::path::Path::new("."),
	)
	.unwrap();
	let store = super::listing::Store::default();
	assert_eq!(super::description(&config, &store), "Keep notes.");
	assert_eq!(
		super::privacy_policy(&store),
		"(add privacy_policy to store/listing.toml)"
	);

	let written = super::listing::Store {
		listing: super::listing::Listing {
			privacy_policy: Some("https://example.com/privacy".to_owned()),
			..super::listing::Listing::default()
		},
		description: Some("Keep notes, everywhere.".to_owned()),
		..super::listing::Store::default()
	};
	assert_eq!(
		super::description(&config, &written),
		"Keep notes, everywhere."
	);
	assert_eq!(
		super::privacy_policy(&written),
		"https://example.com/privacy"
	);
}

#[test]
fn a_store_publish_does_not_know_is_refused() {
	let root = tempfile::tempdir().unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		"[app]\nname = \"Notes\"\nversion = \"1.4.0\"\nplatforms = [\"web\", \
		 \"macos\"]\n\n[publish.macos]\n",
		root.path(),
	)
	.unwrap();
	let store = super::listing::Store::load(root.path()).unwrap();
	let args = super::PublishArgs {
		stores: vec!["safari".to_owned()],
		dry_run: true,
		submit: false,
		staged: false,
		yes: true,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Auto,
	};
	let http = Http::default();
	let session = super::Session {
		args: &args,
		root: root.path(),
		config: &config,
		credentials: &super::Credentials::default(),
		store: &store,
		http: &http,
		runner: &super::tools::Recorder::default(),
		extension: None,
		name: "Notes".to_owned(),
		version: "1.4.0".to_owned(),
		submit: false,
	};
	let error = session.publish("safari").unwrap_err().to_string();
	assert!(error.contains("`safari` isn't a store"), "{error}");
}

#[test]
fn a_missing_package_is_named_with_its_browser() {
	let root = tempfile::tempdir().unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		"[app]\nname = \"Notes\"\nversion = \"1.4.0\"\nplatforms = [\"web\", \
		 \"extension\"]\n\n[extension]\nname = \"Notes\"\nbrowsers = [\"chromium\", \
		 \"firefox\"]\n\n[extension.popup]\nroute = \"/\"\n\n[extension.firefox]\nid = \
		 \"notes@example.com\"\n\n[publish.firefox]\n",
		root.path(),
	)
	.unwrap();
	let store = super::listing::Store::load(root.path()).unwrap();
	let package = crate::extension::Package {
		browser: leptoskit_config::Browser::Chromium,
		dir: root.path().join("chromium"),
		zip: Some(root.path().join("chromium.zip")),
	};
	let extension = super::Extension {
		built: crate::extension::Built {
			packages: vec![package],
			source: Some(root.path().join("firefox-source.zip")),
		},
		manifest: serde_json::json!({}),
	};
	let args = super::PublishArgs {
		stores: vec!["firefox".to_owned()],
		dry_run: true,
		submit: false,
		staged: false,
		yes: true,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Auto,
	};
	let http = Http::default();
	let session = super::Session {
		args: &args,
		root: root.path(),
		config: &config,
		credentials: &super::Credentials::default(),
		store: &store,
		http: &http,
		runner: &super::tools::Recorder::default(),
		extension: Some(&extension),
		name: "Notes".to_owned(),
		version: "1.4.0".to_owned(),
		submit: false,
	};
	let error = session.publish("firefox").unwrap_err().to_string();
	assert!(
		error.contains("the build made no firefox package"),
		"{error}"
	);
	assert!(error.contains("[extension].browsers"), "{error}");
}

/// A project whose Android host is the example's Gradle script, so Google
/// Play's checks pass and only the store's own refusals remain.
fn play_project(
	notes: Option<&str>,
	icon: bool,
) -> (tempfile::TempDir, leptoskit_config::LeptoskitConfig) {
	let root = tempfile::tempdir().unwrap();
	let example =
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/commonplace");
	std::fs::create_dir_all(root.path().join("android/app")).unwrap();
	std::fs::write(root.path().join("android/Cargo.toml"), "").unwrap();
	std::fs::copy(
		example.join("android/app/build.gradle.kts"),
		root.path().join("android/app/build.gradle.kts"),
	)
	.unwrap();

	if let Some(notes) = notes {
		let directory = root.path().join("store/en-US");
		std::fs::create_dir_all(&directory).unwrap();
		std::fs::write(directory.join("release_notes.md"), notes).unwrap();
	}

	if icon {
		std::fs::write(
			root.path().join("icon.svg"),
			r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect width="100" height="100" fill="black"/></svg>"#,
		)
		.unwrap();
	}

	let icon_line = if icon { "icon = \"icon.svg\"\n" } else { "" };
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		&format!(
			"[app]\nname = \"Notes\"\nversion = \"1.4.0\"\nplatforms = [\"web\", \
			 \"android\"]\n{icon_line}\n[native]\npackage_id = \
			 \"com.example.notes\"\n\n[publish.play]\ntrack = \"internal\"\n"
		),
		root.path(),
	)
	.unwrap();
	(root, config)
}

#[test]
fn play_refuses_oversized_store_text_before_anything_builds() {
	let (root, config) = play_project(Some("x".repeat(501).as_str()), true);
	let store = super::listing::Store::load(root.path()).unwrap();
	let args = super::PublishArgs {
		stores: vec!["play".to_owned()],
		dry_run: true,
		submit: false,
		staged: false,
		yes: true,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Auto,
	};
	let http = Http::default();
	let session = super::Session {
		args: &args,
		root: root.path(),
		config: &config,
		credentials: &super::Credentials::default(),
		store: &store,
		http: &http,
		runner: &super::tools::Recorder::default(),
		extension: None,
		name: "Notes".to_owned(),
		version: "1.4.0".to_owned(),
		submit: false,
	};
	let error = session.publish("play").unwrap_err().to_string();
	assert!(
		error.contains("release_notes.md has 501 characters"),
		"{error}"
	);
	assert!(error.contains("Google Play takes at most 500"), "{error}");

	// A store name too long for Google Play fails before the notes are
	// read, and an app without an icon gets past the icon load.
	let (root, config) = play_project(None, false);
	let mut store = super::listing::Store::default();
	store.listing.name =
		Some("A store listing name that is far too long for Google Play".to_owned());
	let http = Http::default();
	let session = super::Session {
		args: &args,
		root: root.path(),
		config: &config,
		credentials: &super::Credentials::default(),
		store: &store,
		http: &http,
		runner: &super::tools::Recorder::default(),
		extension: None,
		name: "Notes".to_owned(),
		version: "1.4.0".to_owned(),
		submit: false,
	};
	let error = session.publish("play").unwrap_err().to_string();
	assert!(
		error.contains("the store name has 57 characters"),
		"{error}"
	);
}

/// Publishes the `play` store on a session in `root`'s project, with no
/// credentials and a recorder instead of any tool.
fn play_publish(
	root: &std::path::Path,
	config: &leptoskit_config::LeptoskitConfig,
	store: &super::listing::Store,
	args: &super::PublishArgs,
) -> anyhow::Result<(String, Vec<super::Step>)> {
	let http = Http::default();
	let credentials = super::Credentials::default();
	let runner = super::tools::Recorder::default();
	let session = super::Session {
		args,
		root,
		config,
		credentials: &credentials,
		store,
		http: &http,
		runner: &runner,
		extension: None,
		name: "Notes".to_owned(),
		version: "1.4.0".to_owned(),
		submit: false,
	};
	session.publish("play")
}

#[test]
fn a_play_dry_run_names_an_explicit_build_number_without_an_icon() {
	let (root, config) = play_project(Some("Faster sync."), false);
	let store = super::listing::Store::load(root.path()).unwrap();
	let args = super::PublishArgs {
		stores: vec!["play".to_owned()],
		dry_run: true,
		submit: false,
		staged: false,
		yes: true,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Number(42),
	};
	let (summary, steps) = play_publish(root.path(), &config, &store, &args).unwrap();
	assert!(
		summary.contains("numbered 42") && summary.contains("internal track as a draft"),
		"{summary}"
	);
	assert!(steps.iter().all(|step| step.store == "play"));
}

#[test]
fn play_refuses_a_bundle_its_gradle_cannot_publish() {
	let (root, config) = play_project(None, false);
	std::fs::write(
		root.path().join("android/app/build.gradle.kts"),
		"not a gradle file at all",
	)
	.unwrap();
	let store = super::listing::Store::load(root.path()).unwrap();
	let args = super::PublishArgs {
		stores: vec!["play".to_owned()],
		dry_run: true,
		submit: false,
		staged: false,
		yes: true,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Auto,
	};
	let error = play_publish(root.path(), &config, &store, &args)
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("Google Play can't take this app's bundle yet"),
		"{error}"
	);
}

#[test]
fn the_play_listing_sends_nothing_without_text_an_icon_or_images() {
	let account = google_account();
	let app = "/androidpublisher/v3/applications/com.example.notes";
	let (base, seen) = store(Box::new(move |request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/token") => json(serde_json::json!({ "access_token": "t" })),
			("POST", path) if path == format!("{app}/edits") => {
				json(serde_json::json!({ "id": "e1" }))
			}

			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));

	let (root, config) = play_project(None, false);
	let store = super::listing::Store::load(root.path()).unwrap();
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
	let http = Http::default();
	let credentials = super::Credentials::default();
	let session = super::Session {
		args: &args,
		root: root.path(),
		config: &config,
		credentials: &credentials,
		store: &store,
		http: &http,
		runner: &super::tools::Recorder::default(),
		extension: None,
		name: "Notes".to_owned(),
		version: "1.4.0".to_owned(),
		submit: false,
	};
	let play = super::play::Play::connect(
		&http,
		&base,
		&format!("{base}/token"),
		&account,
		"com.example.notes",
	)
	.unwrap();
	let edit = play.edit().unwrap().unwrap();

	session.play_listing(&edit, &[], None).unwrap();

	// Only the sign-in and the edit: nothing to upload was uploaded.
	assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn the_play_listing_sends_text_images_and_the_icon() {
	let account = google_account();
	let app = "/androidpublisher/v3/applications/com.example.notes";
	let (base, seen) = store(Box::new(move |request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/token") => json(serde_json::json!({ "access_token": "t" })),
			("POST", path) if path == format!("{app}/edits") => {
				json(serde_json::json!({ "id": "e1" }))
			}
			("PUT", path) if path.ends_with("/listings/en-US") => json(serde_json::json!({})),
			("DELETE", path)
				if path.ends_with("/listings/en-US/phoneScreenshots")
					|| path.ends_with("/listings/en-US/icon") =>
			{
				json(serde_json::json!({ "deleted": [] }))
			}
			("POST", path)
				if path.ends_with("/listings/en-US/phoneScreenshots")
					|| path.ends_with("/listings/en-US/icon") =>
			{
				json(serde_json::json!({ "image": { "id": "i" } }))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));

	let (root, config) = play_project(None, true);
	let directory = root.path().join("store/en-US");
	std::fs::create_dir_all(&directory).unwrap();
	std::fs::write(directory.join("summary.md"), "Keep notes.").unwrap();
	std::fs::write(directory.join("description.md"), "Keep notes, everywhere.").unwrap();
	super::images::tests::png(1080, 1920, 255);

	for name in ["01-projects.png", "02-reading.png"] {
		let path = root
			.path()
			.join("store/screenshots/android-phone")
			.join(name);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, super::images::tests::png(1080, 1920, 255)).unwrap();
	}

	let store = super::listing::Store::load(root.path()).unwrap();
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
	let http = Http::default();
	let session = super::Session {
		args: &args,
		root: root.path(),
		config: &config,
		credentials: &super::Credentials::default(),
		store: &store,
		http: &http,
		runner: &super::tools::Recorder::default(),
		extension: None,
		name: "Notes".to_owned(),
		version: "1.4.0".to_owned(),
		submit: false,
	};
	let play = super::play::Play::connect(
		&http,
		&base,
		&format!("{base}/token"),
		&account,
		"com.example.notes",
	)
	.unwrap();
	let edit = play.edit().unwrap().unwrap();
	let icon = crate::icons::Icon::load(&root.path().join("icon.svg")).unwrap();
	let screenshots = super::images::load(root.path(), "play").unwrap();

	assert_eq!(screenshots.len(), 1);
	session
		.play_listing(&edit, &screenshots, Some(&icon))
		.unwrap();

	let seen = seen.lock().unwrap();
	let listing: serde_json::Value = serde_json::from_slice(&seen[2].body).unwrap();
	assert_eq!(
		listing,
		serde_json::json!({
			"language": "en-US",
			"title": "Notes",
			"shortDescription": "Keep notes.",
			"fullDescription": "Keep notes, everywhere.",
		})
	);
	// Two listing screenshots, then the app icon at 512×512.
	let images: Vec<&Vec<u8>> = seen[3..]
		.iter()
		.filter(|request| request.method == "POST")
		.map(|request| &request.body)
		.collect();
	assert_eq!(images.len(), 3, "{} uploads", images.len());
	let icon_body = &seen[seen.len() - 1].body;
	let icon = image::load_from_memory(icon_body).unwrap();
	assert_eq!((icon.width(), icon.height()), (512, 512));
}

/// A macOS-only project, so `leptoskit publish macos --dry-run` runs without
/// the extension build.
const MACOS_PROJECT: &str = r#"
[app]
name = "Notes"
version = "1.4.0"
platforms = ["web", "macos"]

[publish.macos]
github_release = "example/notes"
"#;

/// The child side of the macOS dry-run scenarios: `super::run` on the
/// project the parent made the current directory, then its saved report.
fn run_macos_child(scenario: &str) {
	let json = scenario == "run-macos-json";
	let args = super::PublishArgs {
		stores: vec!["macos".to_owned()],
		dry_run: true,
		submit: false,
		staged: false,
		yes: true,
		json,
		credentials: None,
		build_number: super::BuildNumber::Number(42),
	};
	super::run(&args).unwrap();

	let output = std::env::current_dir().unwrap().join(super::OUTPUT);

	if json {
		let json = std::fs::read_to_string(output.join("next-steps.json")).unwrap();
		let report: serde_json::Value = serde_json::from_str(&json).unwrap();
		assert_eq!(report["stores"][0]["store"], "macos");
		assert!(
			report["stores"][0]["summary"]
				.as_str()
				.unwrap()
				.contains("is checked")
		);
	}

	let report = std::fs::read_to_string(output.join("next-steps.md")).unwrap();
	assert!(
		report.contains("leptoskit publish macos --submit"),
		"{report}"
	);
	assert!(report.contains("example/notes"), "{report}");
}

#[test]
fn publish_run_dry_runs_macos_without_keys_or_sdks() {
	for scenario in ["run-macos", "run-macos-json"] {
		let configured = project(MACOS_PROJECT);
		run_scenario(scenario, configured.path(), &[]);
	}
}

#[test]
fn confirm_skips_its_question_without_a_terminal() {
	let configured = project(MACOS_PROJECT);
	run_scenario("run-confirm-skipped", configured.path(), &[]);
}
