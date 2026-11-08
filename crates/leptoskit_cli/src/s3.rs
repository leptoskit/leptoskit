//! An S3-compatible bucket for `leptoskit codepush publish` and `pull`:
//! Backblaze B2,
//! Cloudflare R2, AWS S3, `MinIO`, or anything else that speaks the S3 API.
//!
//! Requests are presigned with `SigV4` by `rusty-s3` and sent with `ureq`. The
//! credentials come only from the environment and never appear in output:
//! errors name the object key and the status, never the signed URL.

use std::io::Read;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use leptoskit_codepush::Bucket;
use leptoskit_codepush::CodePushError;
use leptoskit_codepush::ObjectKey;
use leptoskit_codepush::StaticObject;
use rusty_s3::Credentials;
use rusty_s3::S3Action;
use rusty_s3::UrlStyle;

/// The environment variable holding the S3 access key id.
pub const ACCESS_KEY_ID_ENV: &str = "AWS_ACCESS_KEY_ID";

/// The environment variable holding the S3 secret access key.
pub const SECRET_ACCESS_KEY_ENV: &str = "AWS_SECRET_ACCESS_KEY";

/// The environment variable holding an optional session token, for
/// temporary credentials.
pub const SESSION_TOKEN_ENV: &str = "AWS_SESSION_TOKEN";

/// How long each presigned request stays valid. Every request is signed just
/// before it is sent, so this only needs to cover one upload.
const PRESIGN_DURATION: Duration = Duration::from_mins(15);

/// The longest any one request may take, a large container upload included.
const REQUEST_TIMEOUT: Duration = Duration::from_mins(10);

/// Where a bucket lives and how to address it: the bucket flags of
/// `leptoskit codepush publish` and `pull`.
#[derive(Debug, Clone, clap::Args)]
pub struct S3Location {
	/// The S3 API endpoint, such as
	/// `https://<account id>.r2.cloudflarestorage.com` or
	/// `https://s3.us-west-004.backblazeb2.com`.
	#[arg(long, value_name = "URL")]
	pub endpoint: String,
	/// The bucket name.
	#[arg(long)]
	pub bucket: String,
	/// The bucket's region; `auto` suits R2.
	#[arg(long, default_value = "auto")]
	pub region: String,
	/// A key prefix inside the bucket, so one bucket can hold more than the
	/// registry.
	#[arg(long)]
	pub prefix: Option<String>,
	/// Addresses the bucket by path (`<endpoint>/<bucket>`) instead of by
	/// subdomain, for `MinIO` and other local servers.
	#[arg(long)]
	pub path_style: bool,
}

/// An S3-compatible bucket.
///
/// Deliberately not `Debug`: it holds the secret key.
pub struct S3Bucket {
	bucket: rusty_s3::Bucket,
	credentials: Credentials,
	prefix: String,
	agent: ureq::Agent,
}

impl S3Bucket {
	/// Streams an operator backup into a private bucket. This deliberately
	/// does not expose arbitrary object keys through the code-push API.
	pub fn upload_backup(&self, name: &str, path: &std::path::Path) -> Result<()> {
		anyhow::ensure!(
			!name.is_empty()
				&& name
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
			"invalid backup name"
		);
		let key = format!("backups/{name}");
		let mut action = self.bucket.put_object(Some(&self.credentials), &key);
		action
			.headers_mut()
			.insert("content-type", "application/octet-stream");
		action.headers_mut().insert("cache-control", "no-store");
		let file = std::fs::File::open(path)?;
		let response = self
			.agent
			.put(action.sign(PRESIGN_DURATION).as_str())
			.header("content-type", "application/octet-stream")
			.header("cache-control", "no-store")
			.send(file)
			.map_err(|error| request_error("PUT", &key, &error))?;
		anyhow::ensure!(
			response.status().is_success(),
			"backup upload failed: HTTP {}",
			response.status()
		);
		Ok(())
	}

	/// A bucket at `location`, signing with credentials from
	/// [`ACCESS_KEY_ID_ENV`], [`SECRET_ACCESS_KEY_ENV`], and optionally
	/// [`SESSION_TOKEN_ENV`].
	pub fn from_env(location: &S3Location) -> Result<Self> {
		let variable = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
		let (Some(key), Some(secret)) =
			(variable(ACCESS_KEY_ID_ENV), variable(SECRET_ACCESS_KEY_ENV))
		else {
			anyhow::bail!(
				"publishing needs the bucket's S3 credentials: set {ACCESS_KEY_ID_ENV} and \
				 {SECRET_ACCESS_KEY_ENV} (and {SESSION_TOKEN_ENV} for temporary credentials)"
			);
		};

		let credentials = match variable(SESSION_TOKEN_ENV) {
			Some(token) => Credentials::new_with_token(key, secret, token),
			None => Credentials::new(key, secret),
		};

		Self::new(location, credentials)
	}

	/// A bucket at `location`, signing with `credentials`.
	pub fn new(location: &S3Location, credentials: Credentials) -> Result<Self> {
		let endpoint: url::Url = location
			.endpoint
			.parse()
			.with_context(|| format!("{} is not a valid endpoint URL", location.endpoint))?;

		if endpoint.scheme() != "https" {
			// Presigned requests authenticate the request line, not the
			// payload, so a plain-http endpoint leaves uploads open to a
			// network attacker. Every reader verifies what it downloads,
			// so the damage is a failed release, not a bad one; this
			// mirrors `codepush.allow_http`'s friction and says so.
			eprintln!(
				"[codepush] endpoint {} is not https, so uploads can be tampered with in flight",
				location.endpoint
			);
		}
		let style = if location.path_style {
			UrlStyle::Path
		} else {
			UrlStyle::VirtualHost
		};

		let bucket = rusty_s3::Bucket::new(
			endpoint,
			style,
			location.bucket.clone(),
			location.region.clone(),
		)
		.with_context(|| format!("could not address bucket {}", location.bucket))?;

		let agent = ureq::Agent::config_builder()
			.http_status_as_error(false)
			.timeout_global(Some(REQUEST_TIMEOUT))
			.build()
			.into();
		Ok(Self {
			bucket,
			credentials,
			prefix: normalize_prefix(location.prefix.as_deref())?,
			agent,
		})
	}

	/// The bucket key of a registry file.
	fn object_name(&self, key: &ObjectKey) -> String {
		format!("{}{}", self.prefix, key.as_str())
	}
}

impl Bucket for S3Bucket {
	fn get(&mut self, key: &ObjectKey) -> Result<Option<Vec<u8>>, CodePushError> {
		let name = self.object_name(key);
		let url = self
			.bucket
			.get_object(Some(&self.credentials), &name)
			.sign(PRESIGN_DURATION);
		let mut response = self
			.agent
			.get(url.as_str())
			.call()
			.map_err(|error| request_error("GET", &name, &error))?;

		match response.status().as_u16() {
			200 => {
				let limit = key.kind().max_bytes();
				// Bound wire bytes and decoded output separately. Leave one
				// byte for an EOF probe: ureq rejects an exact fit otherwise.
				let mut bytes = Vec::new();
				response
					.body_mut()
					.with_config()
					.limit(limit + 1)
					.reader()
					.take(limit + 1)
					.read_to_end(&mut bytes)
					.map_err(|error| request_error("GET", &name, &ureq::Error::from(error)))?;

				if bytes.len() as u64 > limit {
					return Err(request_error(
						"GET",
						&name,
						&ureq::Error::BodyExceedsLimit(limit),
					));
				}

				Ok(Some(bytes))
			}
			404 => Ok(None),
			status => Err(status_error("GET", &name, status)),
		}
	}

	fn put(&mut self, object: &StaticObject) -> Result<(), CodePushError> {
		let name = self.object_name(&object.key);
		let kind = object.key.kind();
		let mut action = self.bucket.put_object(Some(&self.credentials), &name);
		// Signed, so a proxy cannot change how CDNs cache the file.
		action
			.headers_mut()
			.insert("content-type", kind.content_type());
		action
			.headers_mut()
			.insert("cache-control", kind.cache_control());
		let url = action.sign(PRESIGN_DURATION);
		let response = self
			.agent
			.put(url.as_str())
			.header("content-type", kind.content_type())
			.header("cache-control", kind.cache_control())
			.send(&object.bytes[..])
			.map_err(|error| request_error("PUT", &name, &error))?;

		match response.status().as_u16() {
			200..=299 => Ok(()),
			status => Err(status_error("PUT", &name, status)),
		}
	}
}

/// Trims a key prefix to `segment/.../` form, rejecting segments that are
/// empty or that could escape it.
fn normalize_prefix(prefix: Option<&str>) -> Result<String> {
	let Some(prefix) = prefix.map(|prefix| prefix.trim_matches('/')) else {
		return Ok(String::new());
	};

	if prefix.is_empty() {
		return Ok(String::new());
	}

	anyhow::ensure!(
		prefix
			.split('/')
			.all(|segment| !segment.is_empty() && segment != "." && segment != ".."),
		"the bucket prefix {prefix:?} must be plain path segments"
	);
	Ok(format!("{prefix}/"))
}

/// A failed request. The error is described by kind only: some `ureq` errors
/// quote the URL, and a presigned URL carries a signature.
fn request_error(method: &str, name: &str, error: &ureq::Error) -> CodePushError {
	let reason = match error {
		ureq::Error::Io(error) => error.to_string(),
		ureq::Error::Timeout(_) => "the request timed out".to_owned(),
		ureq::Error::HostNotFound => "the endpoint host was not found".to_owned(),
		ureq::Error::ConnectionFailed => "the connection failed".to_owned(),
		ureq::Error::BodyExceedsLimit(_) => "the object is too large".to_owned(),
		ureq::Error::Tls(error) => format!("TLS failed: {error}"),
		_ => "the request failed".to_owned(),
	};

	CodePushError::Transport(format!("{method} {name}: {reason}"))
}

fn status_error(method: &str, name: &str, status: u16) -> CodePushError {
	let hint = match status {
		401 | 403 => {
			"; check the credentials and their access to the bucket (on AWS a missing object also \
			 reads as 403 unless the credentials may list the bucket)"
		}
		_ => "",
	};

	CodePushError::Transport(format!("{method} {name}: HTTP {status}{hint}"))
}

#[cfg(test)]
mod tests {
	use std::io::BufRead;
	use std::io::Write;
	use std::net::TcpListener;
	use std::path::Path;
	use std::sync::Arc;
	use std::sync::Mutex;

	use axum::body::Bytes;
	use axum::http::Method;
	use axum::http::StatusCode;
	use axum::http::Uri;
	use axum::http::header;
	use axum::response::IntoResponse;
	use axum::response::Response;
	use leptoskit_codepush::ReleaseSigningKey;
	use leptoskit_codepush::ReleaseStore;
	use leptoskit_codepush::publish_channel;
	use semver::Version;

	use super::*;

	/// Serves generated bytes over loopback without allocating the response.
	fn generated_response(
		key: &ObjectKey,
		size: u64,
		chunked: bool,
	) -> Result<Option<Vec<u8>>, CodePushError> {
		let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
		let address = listener.local_addr().expect("address");
		let server = std::thread::spawn(move || {
			let (mut stream, _) = listener.accept().expect("accept");
			stream
				.set_read_timeout(Some(Duration::from_secs(10)))
				.expect("read timeout");
			stream
				.set_write_timeout(Some(Duration::from_secs(10)))
				.expect("write timeout");
			let mut reader = std::io::BufReader::new(&mut stream);
			let mut line = String::new();

			loop {
				line.clear();
				assert_ne!(reader.read_line(&mut line).expect("request"), 0);

				if line == "\r\n" {
					break;
				}
			}

			let header = if chunked {
				format!("Transfer-Encoding: chunked\r\n\r\n{size:x}\r\n")
			} else {
				format!("Content-Length: {size}\r\n\r\n")
			};

			write!(stream, "HTTP/1.1 200 OK\r\nConnection: close\r\n{header}").expect("head");
			let mut body = std::io::repeat(b'x').take(size);
			// The bounded client may close the connection before all bytes
			// arrive.
			if std::io::copy(&mut body, &mut stream).is_ok() && chunked {
				let _ = stream.write_all(b"\r\n0\r\n\r\n");
			}
		});
		let location = S3Location {
			endpoint: format!("http://{address}"),
			bucket: "test".to_owned(),
			region: "local".to_owned(),
			prefix: None,
			path_style: true,
		};
		let mut bucket =
			S3Bucket::new(&location, Credentials::new("test", "test")).expect("bucket");
		let result = bucket.get(key);
		server.join().expect("server");

		result
	}

	#[test]
	fn registry_body_limits_apply_to_content_length_and_chunked_responses() {
		let version = Version::new(1, 0, 0);
		let keys = [
			ObjectKey::index("app", "production"),
			ObjectKey::signature("app", "production", &version, 1),
			ObjectKey::release("app", "production", &version, 1),
			ObjectKey::delta("app", "production", &version, 1, &"a".repeat(64)).expect("delta key"),
		];

		for key in keys {
			for chunked in [false, true] {
				let result = generated_response(&key, key.kind().max_bytes() + 1, chunked);
				assert!(
					matches!(result, Err(CodePushError::Transport(message)) if message.ends_with("the object is too large")),
					"{} chunked={chunked}",
					key.as_str()
				);
			}
		}
	}

	#[test]
	fn registry_body_at_the_limit_is_accepted() {
		let key = ObjectKey::signature("app", "production", &Version::new(1, 0, 0), 1);

		for chunked in [false, true] {
			let bytes = generated_response(&key, key.kind().max_bytes(), chunked)
				.expect("read")
				.expect("body");
			assert_eq!(bytes, vec![b'x'; key.kind().max_bytes() as usize]);
		}
	}

	/// An S3-compatible bucket on loopback, without a store behind it: a PUT
	/// keeps its body, a GET serves it back or answers 404, and a DELETE
	/// forgets it. `fail_with` makes every request answer that status, as a
	/// refused or failing bucket does.
	struct FakeBucket {
		address: String,
		objects: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>>,
		requests: Arc<Mutex<Vec<(String, String)>>>,
		shutdown: Option<tokio::sync::oneshot::Sender<()>>,
		thread: Option<std::thread::JoinHandle<()>>,
	}

	impl FakeBucket {
		fn new(fail_with: Option<StatusCode>) -> Self {
			let objects: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>> = Arc::default();
			let requests: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
			let kept = Arc::clone(&objects);
			let seen = Arc::clone(&requests);
			let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
			let address = listener.local_addr().expect("address");
			listener.set_nonblocking(true).expect("nonblocking");
			let (shutdown, stopped) = tokio::sync::oneshot::channel();
			let thread = std::thread::spawn(move || {
				let runtime = tokio::runtime::Builder::new_current_thread()
					.enable_all()
					.build()
					.expect("runtime");
				runtime.block_on(serve(listener, kept, seen, stopped, fail_with));
			});
			Self {
				address: format!("http://{address}"),
				objects,
				requests,
				shutdown: Some(shutdown),
				thread: Some(thread),
			}
		}

		fn location(&self) -> S3Location {
			S3Location {
				endpoint: self.address.clone(),
				bucket: "test".to_owned(),
				region: "local".to_owned(),
				prefix: Some("/codepush/v1/".to_owned()),
				path_style: true,
			}
		}

		fn bucket(&self) -> S3Bucket {
			S3Bucket::new(&self.location(), Credentials::new("key", "secret")).expect("bucket")
		}
	}

	impl Drop for FakeBucket {
		fn drop(&mut self) {
			let _ = self.shutdown.take().expect("shutdown").send(());
			let result = self.thread.take().expect("thread").join();

			if !std::thread::panicking() {
				result.expect("server");
			}
		}
	}

	/// Answers the fake bucket's requests until the test drops it.
	async fn serve(
		listener: TcpListener,
		objects: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>>,
		requests: Arc<Mutex<Vec<(String, String)>>>,
		stopped: tokio::sync::oneshot::Receiver<()>,
		fail_with: Option<StatusCode>,
	) {
		let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
		let router = axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
			let objects = Arc::clone(&objects);
			let requests = Arc::clone(&requests);
			async move { answer(objects, requests, fail_with, method, uri, body) }
		});
		tokio::select! {
			() = async { axum::serve(listener, router).await.expect("serve"); } => {},
			_ = stopped => {},
		}
	}

	/// One request to the fake bucket: PUT stores the body, GET serves it or
	/// answers 404, DELETE forgets it, and any other method is refused.
	#[allow(
		clippy::needless_pass_by_value,
		reason = "the fallback handler owns each request's parts"
	)]
	fn answer(
		objects: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>>,
		requests: Arc<Mutex<Vec<(String, String)>>>,
		fail_with: Option<StatusCode>,
		method: Method,
		uri: Uri,
		body: Bytes,
	) -> Response {
		let path = uri.path().to_owned();
		requests
			.lock()
			.unwrap()
			.push((method.to_string(), path.clone()));

		if let Some(status) = fail_with {
			return status.into_response();
		}

		match method {
			Method::PUT => {
				objects.lock().unwrap().insert(path, body.to_vec());
				StatusCode::OK.into_response()
			}
			Method::GET => {
				match objects.lock().unwrap().get(&path).cloned() {
					Some(bytes) => {
						([(header::CONTENT_TYPE, "application/octet-stream")], bytes)
							.into_response()
					}
					None => StatusCode::NOT_FOUND.into_response(),
				}
			}
			Method::DELETE => {
				objects.lock().unwrap().remove(&path);
				StatusCode::NO_CONTENT.into_response()
			}
			_ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
		}
	}

	impl S3Bucket {
		/// Deletes an object, as the real-bucket test leaves nothing behind.
		fn delete(&mut self, key: &ObjectKey) {
			let name = self.object_name(key);
			let url = self
				.bucket
				.delete_object(Some(&self.credentials), &name)
				.sign(PRESIGN_DURATION);
			let response = self.agent.delete(url.as_str()).call().expect("delete");
			assert!(response.status().is_success(), "DELETE {name}");
		}
	}

	/// Publishes two releases through `bucket`, reads every file back,
	/// deletes them, and checks they are gone. A real bucket and the fake one
	/// run the same path.
	fn publish_read_back_delete(bucket: &mut S3Bucket, scratch: &Path) {
		let store = ReleaseStore::open(scratch.join("releases")).expect("store");
		let signer = ReleaseSigningKey::from_seed(&[0x5a; 32]);

		for build in 1..=2 {
			let dir = scratch.join(format!("bundle{build}"));
			std::fs::create_dir_all(&dir).expect("bundle");
			let mut wasm = vec![1u8; 32 * 1024];
			wasm.extend_from_slice(format!("build {build}").as_bytes());

			std::fs::write(dir.join("app.wasm"), wasm).expect("wasm");
			store
				.ingest_directory(
					&signer,
					"app",
					"production",
					&Version::new(1, 0, 0),
					build,
					&dir,
					false,
				)
				.expect("ingest");
		}

		let report = publish_channel(&store, "app", "production", bucket).expect("publish");
		assert_eq!(report.releases, ["1.0.0+1", "1.0.0+2"]);

		let index = ObjectKey::index("app", "production");
		let mut objects = Vec::new();

		for record in store.list("app", "production").expect("list") {
			objects.extend(store.release_objects(&record).expect("objects"));
		}

		objects.push(StaticObject {
			bytes: store.read_object(&index).expect("read").expect("index"),
			key: index,
		});

		for object in &objects {
			assert_eq!(
				bucket.get(&object.key).expect("get").as_ref(),
				Some(&object.bytes),
				"{}",
				object.key.as_str()
			);
		}

		for object in &objects {
			bucket.delete(&object.key);
		}

		for object in &objects {
			assert!(
				bucket.get(&object.key).expect("get after delete").is_none(),
				"{} still exists",
				object.key.as_str()
			);
		}
	}

	#[test]
	fn publishes_reads_back_and_deletes_through_a_bucket() {
		let server = FakeBucket::new(None);
		let mut bucket = server.bucket();
		let scratch = tempfile::tempdir().expect("scratch");
		publish_read_back_delete(&mut bucket, scratch.path());

		let requests = server.requests.lock().unwrap();
		let puts: Vec<&str> = requests
			.iter()
			.filter(|(method, _)| method == "PUT")
			.map(|(_, path)| path.as_str())
			.collect();
		// Everything lives under the configured prefix.
		assert!(
			puts.iter()
				.all(|path| path.starts_with("/test/codepush/v1/")),
			"{requests:?}"
		);
		// The index goes last, so a reader sees a release only once it is
		// whole.
		assert!(
			puts.last()
				.is_some_and(|path| path.ends_with(ObjectKey::index("app", "production").as_str())),
			"{puts:?}"
		);
	}

	#[test]
	fn backups_upload_and_refuse_unsafe_names() {
		let server = FakeBucket::new(None);
		let bucket = server.bucket();
		let scratch = tempfile::tempdir().expect("scratch");
		let file = scratch.path().join("app.db");
		std::fs::write(&file, b"sqlite bytes").expect("backup");

		for name in ["", "a/b", "with space", "backup?x"] {
			assert!(bucket.upload_backup(name, &file).is_err(), "{name:?}");
		}

		bucket.upload_backup("2026-11-08", &file).expect("upload");
		let objects = server.objects.lock().unwrap();
		// Backups live in their own `backups/` folder, outside the registry's
		// prefix.
		assert_eq!(
			objects.get("/test/backups/2026-11-08").map(Vec::as_slice),
			Some(b"sqlite bytes".as_slice())
		);
	}

	#[test]
	fn a_failed_backup_upload_reports_its_status() {
		let server = FakeBucket::new(Some(StatusCode::INTERNAL_SERVER_ERROR));
		let bucket = server.bucket();
		let scratch = tempfile::tempdir().expect("scratch");
		let file = scratch.path().join("app.db");
		std::fs::write(&file, b"sqlite bytes").expect("backup");

		let error = bucket
			.upload_backup("2026-11-08", &file)
			.unwrap_err()
			.to_string();
		assert!(error.contains("backup upload failed: HTTP 500"), "{error}");
	}

	#[test]
	fn a_missing_object_reads_as_none() {
		let server = FakeBucket::new(None);
		let mut bucket = server.bucket();
		assert!(
			bucket
				.get(&ObjectKey::index("app", "production"))
				.expect("get")
				.is_none()
		);
	}

	#[test]
	fn refused_objects_name_the_credentials_but_never_the_url() {
		let server = FakeBucket::new(Some(StatusCode::FORBIDDEN));
		let mut bucket = server.bucket();
		let key = ObjectKey::index("app", "production");
		let error = bucket.get(&key).unwrap_err().to_string();
		assert!(error.contains("HTTP 403"), "{error}");
		assert!(error.contains("check the credentials"), "{error}");
		assert!(!error.contains("X-Amz-Signature"), "{error}");

		let object = StaticObject {
			key: ObjectKey::signature("app", "production", &Version::new(1, 0, 0), 1),
			bytes: b"signature".to_vec(),
		};
		let error = bucket.put(&object).unwrap_err().to_string();
		assert!(error.contains("HTTP 403"), "{error}");
		assert!(error.contains("check the credentials"), "{error}");
		assert!(!error.contains("X-Amz-Signature"), "{error}");
	}

	#[test]
	fn a_server_error_on_put_has_no_credential_hint() {
		let server = FakeBucket::new(Some(StatusCode::INTERNAL_SERVER_ERROR));
		let mut bucket = server.bucket();
		let object = StaticObject {
			key: ObjectKey::index("app", "production"),
			bytes: b"index".to_vec(),
		};
		let error = bucket.put(&object).unwrap_err().to_string();
		assert!(error.contains("HTTP 500"), "{error}");
		assert!(!error.contains("check the credentials"), "{error}");
	}

	#[test]
	fn a_refused_connection_is_a_transport_error_without_the_url() {
		let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
		let address = listener.local_addr().expect("address");
		drop(listener);
		let location = S3Location {
			endpoint: format!("http://{address}"),
			bucket: "test".to_owned(),
			region: "local".to_owned(),
			prefix: None,
			path_style: true,
		};
		let mut bucket =
			S3Bucket::new(&location, Credentials::new("key", "secret")).expect("bucket");
		let error = bucket
			.get(&ObjectKey::index("app", "production"))
			.unwrap_err()
			.to_string();
		assert!(error.contains("transport error"), "{error}");
		assert!(!error.contains("X-Amz-Signature"), "{error}");

		let object = StaticObject {
			key: ObjectKey::index("app", "production"),
			bytes: b"index".to_vec(),
		};
		let error = bucket.put(&object).unwrap_err().to_string();
		assert!(error.contains("transport error"), "{error}");
		assert!(!error.contains("X-Amz-Signature"), "{error}");
	}

	#[test]
	fn transport_errors_name_their_cause() {
		let refused = ureq::Error::Io(std::io::Error::new(
			std::io::ErrorKind::ConnectionRefused,
			"connection refused",
		));
		let cases = [
			(
				ureq::Error::Timeout(ureq::Timeout::Global),
				"the request timed out",
			),
			(ureq::Error::HostNotFound, "the endpoint host was not found"),
			(ureq::Error::ConnectionFailed, "the connection failed"),
			(ureq::Error::BodyExceedsLimit(10), "the object is too large"),
			(
				ureq::Error::Tls("bad certificate"),
				"TLS failed: bad certificate",
			),
			(ureq::Error::StatusCode(500), "the request failed"),
			(refused, "connection refused"),
		];

		for (error, reason) in cases {
			let message = request_error("GET", "codepush/index.json", &error).to_string();
			assert!(message.contains(reason), "{message}");
		}

		assert!(
			status_error("GET", "codepush/index.json", 401)
				.to_string()
				.contains("check the credentials")
		);
		assert_eq!(
			status_error("GET", "codepush/index.json", 500).to_string(),
			"transport error: GET codepush/index.json: HTTP 500"
		);
	}

	#[test]
	fn endpoints_must_be_urls_and_buckets_must_be_addressable() {
		let credentials = || Credentials::new("key", "secret");
		let location = |endpoint: &str, path_style: bool| {
			S3Location {
				endpoint: endpoint.to_owned(),
				bucket: "test".to_owned(),
				region: "auto".to_owned(),
				prefix: None,
				path_style,
			}
		};

		let error = S3Bucket::new(&location("not a url", true), credentials())
			.err()
			.expect("invalid endpoint")
			.to_string();
		assert!(error.contains("is not a valid endpoint URL"), "{error}");

		let error = S3Bucket::new(&location("ftp://example.com", true), credentials())
			.err()
			.expect("unaddressable bucket")
			.to_string();
		assert!(error.contains("could not address bucket test"), "{error}");

		// Both addressing styles work; a plain-http endpoint only warns.
		let virtual_host = S3Bucket::new(&location("https://s3.example.com", false), credentials())
			.expect("virtual host");
		assert_eq!(
			virtual_host.bucket.base_url().host_str().expect("host"),
			"test.s3.example.com"
		);
		let path_style = S3Bucket::new(&location("http://127.0.0.1:1", true), credentials())
			.expect("path style");
		assert_eq!(path_style.bucket.base_url().path(), "/test/");
	}

	#[test]
	fn prefixes_are_normalized_and_validated() {
		assert_eq!(normalize_prefix(None).expect("none"), "");
		assert_eq!(normalize_prefix(Some("/")).expect("root"), "");
		assert_eq!(
			normalize_prefix(Some("/codepush/v1/")).expect("prefix"),
			"codepush/v1/"
		);

		for prefix in ["a//b", "../a", "a/./b"] {
			assert!(normalize_prefix(Some(prefix)).is_err(), "{prefix:?}");
		}
	}

	#[test]
	fn a_refused_backup_upload_is_a_transport_error() {
		let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
		let address = listener.local_addr().expect("address");
		drop(listener);
		let location = S3Location {
			endpoint: format!("http://{address}"),
			bucket: "test".to_owned(),
			region: "local".to_owned(),
			prefix: None,
			path_style: true,
		};
		let bucket = S3Bucket::new(&location, Credentials::new("key", "secret")).expect("bucket");
		let scratch = tempfile::tempdir().expect("scratch");
		let file = scratch.path().join("app.db");
		std::fs::write(&file, b"sqlite bytes").expect("backup");

		let error = bucket.upload_backup("app", &file).unwrap_err().to_string();
		assert!(error.contains("transport error"), "{error}");
		assert!(!error.contains("X-Amz-Signature"), "{error}");
	}

	#[test]
	fn a_response_that_ends_early_is_a_transport_error() {
		let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
		let address = listener.local_addr().expect("address");
		let server = std::thread::spawn(move || {
			let (mut stream, _) = listener.accept().expect("accept");
			let mut reader = std::io::BufReader::new(&mut stream);
			let mut line = String::new();

			loop {
				line.clear();
				let read = reader.read_line(&mut line).expect("request");

				if read == 0 || line == "\r\n" {
					break;
				}
			}

			stream
				.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 10\r\n\r\nabc")
				.expect("response");
		});
		let location = S3Location {
			endpoint: format!("http://{address}"),
			bucket: "test".to_owned(),
			region: "local".to_owned(),
			prefix: None,
			path_style: true,
		};
		let mut bucket =
			S3Bucket::new(&location, Credentials::new("key", "secret")).expect("bucket");
		let error = bucket
			.get(&ObjectKey::index("app", "production"))
			.unwrap_err()
			.to_string();
		server.join().expect("server");
		assert!(error.contains("transport error"), "{error}");
	}

	#[test]
	fn the_fake_bucket_refuses_methods_publishing_never_uses() {
		let server = FakeBucket::new(None);
		let bucket = server.bucket();
		let response = bucket
			.agent
			.head(format!("{}/test/anything", server.address))
			.call()
			.expect("head");
		assert_eq!(response.status().as_u16(), 405);
	}

	const ENV_HELPER: &str = "s3::tests::env_helper";

	/// Runs [`env_helper`] in a child process with only the environment the
	/// scenario needs, so the `AWS_*` variables a machine or another test
	/// sets never reach a bucket.
	fn spawn_env(scenario: &str, variables: &[(&str, &str)]) {
		let mut command = std::process::Command::new(std::env::current_exe().expect("test binary"));
		command
			.args(["--exact", ENV_HELPER, "--nocapture", "--test-threads=1"])
			.env_clear()
			.env("LEPTOSKIT_S3_SCENARIO", scenario)
			.env("PATH", std::env::var_os("PATH").unwrap_or_default())
			.stdin(std::process::Stdio::null());

		// Winsock loads its providers relative to SystemRoot on Windows.
		if let Some(system_root) = std::env::var_os("SystemRoot") {
			command.env("SystemRoot", system_root);
		}

		if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
			command.env("LLVM_PROFILE_FILE", profile);
		}

		for (name, value) in variables {
			command.env(name, value);
		}

		let output = command.output().expect("run helper");
		assert!(
			output.status.success(),
			"scenario {scenario} failed\nstdout:\n{}\nstderr:\n{}",
			String::from_utf8_lossy(&output.stdout),
			String::from_utf8_lossy(&output.stderr)
		);
	}

	#[test]
	fn credentials_come_from_the_environment_or_name_the_missing_ones() {
		spawn_env("missing", &[]);
		spawn_env(
			"plain",
			&[
				(ACCESS_KEY_ID_ENV, "key"),
				(SECRET_ACCESS_KEY_ENV, "secret"),
			],
		);
		spawn_env(
			"session",
			&[
				(ACCESS_KEY_ID_ENV, "key"),
				(SECRET_ACCESS_KEY_ENV, "secret"),
				(SESSION_TOKEN_ENV, "token"),
			],
		);
		spawn_env(
			"test-location",
			&[
				("LEPTOSKIT_S3_TEST_ENDPOINT", "https://s3.example.com"),
				("LEPTOSKIT_S3_TEST_BUCKET", "my-bucket"),
				("LEPTOSKIT_S3_TEST_PATH_STYLE", "1"),
			],
		);
	}

	#[test]
	fn env_helper() {
		let Ok(scenario) = std::env::var("LEPTOSKIT_S3_SCENARIO") else {
			return;
		};
		let location = S3Location {
			endpoint: "https://s3.example.com".to_owned(),
			bucket: "test".to_owned(),
			region: "auto".to_owned(),
			prefix: None,
			path_style: true,
		};

		match scenario.as_str() {
			"missing" => {
				let error = S3Bucket::from_env(&location)
					.err()
					.expect("no credentials")
					.to_string();
				assert!(error.contains(ACCESS_KEY_ID_ENV), "{error}");
				assert!(error.contains(SECRET_ACCESS_KEY_ENV), "{error}");
			}
			"plain" | "session" => {
				S3Bucket::from_env(&location).expect("credentials");
			}
			"test-location" => {
				let location = real_bucket_location();
				assert_eq!(location.endpoint, "https://s3.example.com");
				assert_eq!(location.bucket, "my-bucket");
				assert_eq!(location.region, "auto");
				assert!(location.path_style);
				assert!(
					location
						.prefix
						.as_deref()
						.is_some_and(|prefix| prefix.starts_with("leptoskit-test-")),
					"{:?}",
					location.prefix
				);
			}
			other => panic!("unknown s3 scenario {other}"),
		}
	}

	/// Publishes two releases to a real bucket under a fresh prefix, reads
	/// every file back, and deletes them again.
	///
	/// ```bash
	/// LEPTOSKIT_S3_TEST_ENDPOINT=https://s3.us-west-004.backblazeb2.com \
	/// LEPTOSKIT_S3_TEST_BUCKET=my-bucket LEPTOSKIT_S3_TEST_REGION=us-west-004 \
	/// AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... \
	/// cargo test -p leptoskit_cli publishes_to_a_real_bucket -- --ignored
	/// ```
	/// The bucket the ignored test uses, from the variables its doc comment
	/// names. A fresh prefix keeps concurrent runs apart.
	fn real_bucket_location() -> S3Location {
		let variable = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
		let unique = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.expect("clock")
			.as_nanos();
		S3Location {
			endpoint: variable("LEPTOSKIT_S3_TEST_ENDPOINT").expect("LEPTOSKIT_S3_TEST_ENDPOINT"),
			bucket: variable("LEPTOSKIT_S3_TEST_BUCKET").expect("LEPTOSKIT_S3_TEST_BUCKET"),
			region: variable("LEPTOSKIT_S3_TEST_REGION").unwrap_or_else(|| "auto".to_owned()),
			prefix: Some(format!("leptoskit-test-{unique}")),
			path_style: variable("LEPTOSKIT_S3_TEST_PATH_STYLE").is_some(),
		}
	}

	#[test]
	#[ignore = "needs a real S3-compatible bucket: set LEPTOSKIT_S3_TEST_ENDPOINT, \
	            LEPTOSKIT_S3_TEST_BUCKET, and the AWS_* credentials"]
	fn publishes_to_a_real_bucket() {
		let location = real_bucket_location();
		let mut bucket = S3Bucket::from_env(&location).expect("credentials");
		let scratch = std::env::temp_dir().join(
			location
				.prefix
				.clone()
				.expect("a disjoint prefix for the test's objects"),
		);

		publish_read_back_delete(&mut bucket, &scratch);

		let _ = std::fs::remove_dir_all(&scratch);
	}
}
