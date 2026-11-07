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
	use std::sync::Arc;
	use std::sync::Mutex;
	use std::thread::JoinHandle;

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

	impl S3Bucket {
		/// Deletes an object, so the real-bucket test leaves nothing behind.
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

	/// Publishes two releases to a real bucket under a fresh prefix, reads
	/// every file back, and deletes them again.
	///
	/// ```bash
	/// LEPTOSKIT_S3_TEST_ENDPOINT=https://s3.us-west-004.backblazeb2.com \
	/// LEPTOSKIT_S3_TEST_BUCKET=my-bucket LEPTOSKIT_S3_TEST_REGION=us-west-004 \
	/// AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... \
	/// cargo test -p leptoskit_cli publishes_to_a_real_bucket -- --ignored
	/// ```
	#[test]
	#[ignore = "needs a real S3-compatible bucket: set LEPTOSKIT_S3_TEST_ENDPOINT, \
	            LEPTOSKIT_S3_TEST_BUCKET, and the AWS_* credentials"]
	fn publishes_to_a_real_bucket() {
		let variable = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
		let unique = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.expect("clock")
			.as_nanos();
		let location = S3Location {
			endpoint: variable("LEPTOSKIT_S3_TEST_ENDPOINT").expect("LEPTOSKIT_S3_TEST_ENDPOINT"),
			bucket: variable("LEPTOSKIT_S3_TEST_BUCKET").expect("LEPTOSKIT_S3_TEST_BUCKET"),
			region: variable("LEPTOSKIT_S3_TEST_REGION").unwrap_or_else(|| "auto".to_owned()),
			prefix: Some(format!("leptoskit-test-{unique}")),
			path_style: variable("LEPTOSKIT_S3_TEST_PATH_STYLE").is_some(),
		};
		let mut bucket = S3Bucket::from_env(&location).expect("credentials");

		let scratch = std::env::temp_dir().join(format!("leptoskit_s3_test_{unique}"));
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

		let report = publish_channel(&store, "app", "production", &mut bucket).expect("publish");
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

		let _ = std::fs::remove_dir_all(&scratch);
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

	/// Answers `count` requests on loopback with a status line, extra header
	/// lines, and a body, recording each request's head. One connection is
	/// served at a time; dropping the result waits for the thread.
	struct Served {
		address: String,
		head: Arc<Mutex<Vec<String>>>,
		bodies: Arc<Mutex<Vec<Vec<u8>>>>,
		join: Option<JoinHandle<()>>,
	}

	impl Served {
		/// What each connection received: the request line and header lines.
		fn requests(&self) -> Vec<String> {
			self.head.lock().unwrap().clone()
		}

		/// The bytes each connection's request body carried.
		fn bodies(&self) -> Vec<Vec<u8>> {
			self.bodies.lock().unwrap().clone()
		}
	}

	impl Drop for Served {
		fn drop(&mut self) {
			let _ = self.join.take().map(JoinHandle::join);
		}
	}

	/// Serves `count` connections that each get `status` (a full status
	/// line), `extra` header lines, and `body`.
	fn answering(count: usize, status: &str, extra: &str, body: &[u8]) -> Served {
		let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
		let address = listener.local_addr().expect("address").to_string();
		let head: Arc<Mutex<Vec<String>>> = Arc::default();
		let bodies: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
		let recorded_head = Arc::clone(&head);
		let recorded_bodies = Arc::clone(&bodies);
		let (status, extra, body) = (status.to_owned(), extra.to_owned(), body.to_vec());
		let join = std::thread::spawn(move || {
			for _ in 0..count {
				let Ok((mut stream, _)) = listener.accept() else {
					break;
				};
				stream
					.set_read_timeout(Some(Duration::from_secs(10)))
					.expect("read timeout");
				let mut reader = std::io::BufReader::new(&mut stream);
				let mut lines = Vec::new();

				loop {
					let mut line = String::new();
					assert_ne!(reader.read_line(&mut line).expect("request"), 0);

					if line == "\r\n" {
						break;
					}

					lines.push(line.trim_end().to_owned());
				}

				let length = lines
					.iter()
					.find_map(|line| {
						line.split_once(':')
							.filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
							.and_then(|(_, value)| value.trim().parse::<usize>().ok())
					})
					.unwrap_or(0);
				let mut bytes = vec![0u8; length];
				reader.read_exact(&mut bytes).expect("body");
				recorded_head.lock().unwrap().push(lines.join("\n"));
				recorded_bodies.lock().unwrap().push(bytes);
				let mut response = format!(
					"{status}\r\nConnection: close\r\nContent-Length: {}\r\n",
					body.len()
				);

				if !extra.is_empty() {
					response.push_str(&extra);
					response.push_str("\r\n");
				}

				write!(stream, "{response}\r\n").expect("head");
				stream.write_all(&body).expect("body");
			}
		});
		Served {
			address,
			head,
			bodies,
			join: Some(join),
		}
	}

	/// A bucket against `server`, addressing by path.
	fn bucket_at(server: &Served, prefix: Option<&str>) -> S3Bucket {
		let location = S3Location {
			endpoint: format!("http://{}", server.address),
			bucket: "test".to_owned(),
			region: "local".to_owned(),
			prefix: prefix.map(str::to_owned),
			path_style: true,
		};
		S3Bucket::new(&location, Credentials::new("key", "secret")).expect("bucket")
	}

	#[test]
	fn put_writes_the_object_and_reports_a_failed_status() {
		let index = ObjectKey::index("app", "production");
		let object = StaticObject {
			key: index.clone(),
			bytes: b"the index".to_vec(),
		};

		let ok = answering(1, "HTTP/1.1 200 OK", "", b"");
		bucket_at(&ok, None).put(&object).expect("put");
		let requests = ok.requests();
		assert_eq!(requests.len(), 1);
		assert!(
			requests[0].starts_with("PUT /test/app/production/index.json?"),
			"{}",
			requests[0]
		);
		let lower = requests[0].to_lowercase();
		assert!(
			lower.contains(&format!("content-type: {}", index.kind().content_type())),
			"{}",
			requests[0]
		);
		assert!(
			lower.contains(&format!("cache-control: {}", index.kind().cache_control())),
			"{}",
			requests[0]
		);
		assert_eq!(ok.bodies(), vec![b"the index".to_vec()]);

		let refused = answering(1, "HTTP/1.1 403 Forbidden", "", b"");
		let error = bucket_at(&refused, None)
			.put(&object)
			.expect_err("403")
			.to_string();
		assert!(
			error.contains("PUT app/production/index.json: HTTP 403; check the credentials"),
			"{error}"
		);

		let broken = answering(1, "HTTP/1.1 500 Internal Server Error", "", b"");
		let error = bucket_at(&broken, None)
			.put(&object)
			.expect_err("500")
			.to_string();
		assert!(
			error.contains("PUT app/production/index.json: HTTP 500"),
			"{error}"
		);
	}

	#[test]
	fn get_treats_404_as_absent_and_names_other_statuses() {
		let index = ObjectKey::index("app", "production");

		let missing = answering(1, "HTTP/1.1 404 Not Found", "", b"");
		assert_eq!(bucket_at(&missing, None).get(&index).unwrap(), None);

		// A response with extra headers, as a proxy adds, is fine.
		let broken = answering(
			1,
			"HTTP/1.1 503 Service Unavailable",
			"Retry-After: 30",
			b"",
		);
		let error = bucket_at(&broken, None)
			.get(&index)
			.expect_err("503")
			.to_string();
		assert!(
			error.contains("GET app/production/index.json: HTTP 503"),
			"{error}"
		);
	}

	#[test]
	fn a_prefix_joins_every_object_key() {
		let index = ObjectKey::index("app", "production");
		let server = answering(0, "", "", b"");
		let bucket = bucket_at(&server, Some("codepush/v1"));
		assert_eq!(
			bucket.object_name(&index),
			"codepush/v1/app/production/index.json"
		);
		// Path style keeps the bucket in the request line's path.
		let path = bucket
			.bucket
			.get_object(Some(&bucket.credentials), &bucket.object_name(&index))
			.sign(PRESIGN_DURATION);
		assert!(
			path.path().starts_with("/test/codepush/v1/app/"),
			"{}",
			path
		);

		// Virtual-host style keeps the bucket in the host instead. It needs
		// a hostname endpoint: a bucket can't be a subdomain of an address.
		let location = S3Location {
			endpoint: "http://localhost:9".to_owned(),
			bucket: "test".to_owned(),
			region: "local".to_owned(),
			prefix: None,
			path_style: false,
		};
		let bucket = S3Bucket::new(&location, Credentials::new("key", "secret")).expect("bucket");
		let host = bucket
			.bucket
			.get_object(Some(&bucket.credentials), &bucket.object_name(&index))
			.sign(PRESIGN_DURATION);
		assert!(host.path().starts_with("/app/production/"), "{}", host);
		assert!(host.as_str().contains("test.localhost"), "{}", host);
	}

	/// The name of the child test this module runs its environment
	/// scenarios in, so `AWS_*` variables are set only in the child.
	const HELPER: &str = "s3::tests::subprocess_helper";

	/// Runs one helper scenario in a child process with only the environment
	/// the scenario needs, so credentials never leak into this process.
	fn spawn_scenario(scenario: &str, environment: &[(&str, &str)]) -> String {
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

		let output = command.output().unwrap();
		assert!(
			output.status.success(),
			"scenario {scenario} failed\nstdout:\n{}\nstderr:\n{}",
			String::from_utf8_lossy(&output.stdout),
			String::from_utf8_lossy(&output.stderr)
		);
		String::from_utf8_lossy(&output.stdout).into_owned()
	}

	/// A bucket location nothing listens on, enough for `from_env`, which
	/// only builds the bucket.
	fn env_location() -> S3Location {
		S3Location {
			endpoint: "http://127.0.0.1:9".to_owned(),
			bucket: "test".to_owned(),
			region: "local".to_owned(),
			prefix: None,
			path_style: true,
		}
	}

	#[test]
	fn credentials_come_from_the_environment() {
		spawn_scenario("from-env-missing", &[]);

		// An empty value counts as unset.
		spawn_scenario(
			"from-env-empty",
			&[(ACCESS_KEY_ID_ENV, ""), (SECRET_ACCESS_KEY_ENV, "secret")],
		);

		spawn_scenario(
			"from-env-set",
			&[
				(ACCESS_KEY_ID_ENV, "key"),
				(SECRET_ACCESS_KEY_ENV, "secret"),
			],
		);
	}

	#[test]
	fn a_session_token_signs_with_the_token() {
		let stdout = spawn_scenario(
			"from-env-token",
			&[
				(ACCESS_KEY_ID_ENV, "key"),
				(SECRET_ACCESS_KEY_ENV, "secret"),
				(SESSION_TOKEN_ENV, "token"),
			],
		);
		assert!(stdout.contains("X-Amz-Security-Token="), "{stdout}");
	}

	/// The child side of [`spawn_scenario`]: checks one environment shape of
	/// [`S3Bucket::from_env`]. A normal test run has no scenario and does
	/// nothing.
	fn run_scenario(scenario: &str) {
		match scenario {
			"from-env-missing" => {
				let Err(error) = S3Bucket::from_env(&env_location()) else {
					panic!("no credentials are set")
				};
				let error = error.to_string();
				assert!(
					error.contains(ACCESS_KEY_ID_ENV)
						&& error.contains(SECRET_ACCESS_KEY_ENV)
						&& error.contains(SESSION_TOKEN_ENV),
					"{error}"
				);
			}
			"from-env-empty" => {
				assert!(S3Bucket::from_env(&env_location()).is_err());
			}
			"from-env-set" => {
				S3Bucket::from_env(&env_location()).expect("credentials");
			}
			"from-env-token" => {
				let bucket = S3Bucket::from_env(&env_location()).expect("credentials");
				let name = bucket.object_name(&ObjectKey::index("app", "production"));
				println!(
					"{}",
					bucket
						.bucket
						.get_object(Some(&bucket.credentials), &name)
						.sign(PRESIGN_DURATION)
				);
			}
			other => panic!("unknown s3 scenario {other}"),
		}
	}

	#[test]
	fn subprocess_helper() {
		let Ok(scenario) = std::env::var("LEPTOSKIT_TEST_SCENARIO") else {
			return;
		};
		assert!(scenario.starts_with("from-env-"), "{scenario}");
		run_scenario(&scenario);
	}

	#[test]
	fn a_broken_endpoint_is_named() {
		let location = S3Location {
			endpoint: "127.0.0.1:1 without a scheme".to_owned(),
			bucket: "test".to_owned(),
			region: "local".to_owned(),
			prefix: None,
			path_style: true,
		};
		let Err(error) = S3Bucket::new(&location, Credentials::new("key", "secret")) else {
			panic!("not a URL")
		};
		let error = error.to_string();
		assert!(error.contains("is not a valid endpoint URL"), "{error}");
	}

	#[test]
	fn upload_backup_validates_its_name_and_streams_the_file() {
		let scratch = tempfile::tempdir().unwrap();
		let file = scratch.path().join("backup-1.tar");
		std::fs::write(&file, b"backup bytes").unwrap();

		for name in ["", "a/b", "a b", "a\tb", "héllo"] {
			let server = answering(0, "", "", b"");
			let error = bucket_at(&server, None)
				.upload_backup(name, &file)
				.expect_err(name);
			assert_eq!(error.to_string(), "invalid backup name", "{name}");
		}

		let ok = answering(1, "HTTP/1.1 200 OK", "", b"");
		bucket_at(&ok, None)
			.upload_backup("backup-1.tar", &file)
			.expect("upload");
		assert!(
			ok.requests()[0].starts_with("PUT /test/backups/backup-1.tar?"),
			"{}",
			ok.requests()[0]
		);
		let lower = ok.requests()[0].to_lowercase();
		assert!(
			lower.contains("content-type: application/octet-stream"),
			"{}",
			ok.requests()[0]
		);
		assert!(
			lower.contains("cache-control: no-store"),
			"{}",
			ok.requests()[0]
		);
		assert_eq!(ok.bodies(), vec![b"backup bytes".to_vec()]);

		let refused = answering(1, "HTTP/1.1 500 Internal Server Error", "", b"");
		let error = bucket_at(&refused, None)
			.upload_backup("backup-1.tar", &file)
			.expect_err("500")
			.to_string();
		assert!(error.contains("backup upload failed: HTTP 500"), "{error}");

		// A missing file, and a directory, fail before anything is sent.
		let missing = scratch.path().join("absent.tar");
		let server = answering(0, "", "", b"");
		assert!(
			bucket_at(&server, None)
				.upload_backup("backup-1.tar", &missing)
				.is_err()
		);
		let server = answering(0, "", "", b"");
		assert!(
			bucket_at(&server, None)
				.upload_backup("backup-1.tar", scratch.path())
				.is_err()
		);
	}

	#[test]
	fn request_errors_name_the_kind_never_a_url() {
		let reasons: [(ureq::Error, &str); 7] = [
			(
				ureq::Error::Io(std::io::Error::new(
					std::io::ErrorKind::UnexpectedEof,
					"the socket closed",
				)),
				"the socket closed",
			),
			(
				ureq::Error::Timeout(ureq::Timeout::Global),
				"the request timed out",
			),
			(ureq::Error::HostNotFound, "the endpoint host was not found"),
			(ureq::Error::ConnectionFailed, "the connection failed"),
			(ureq::Error::BodyExceedsLimit(1), "the object is too large"),
			(
				ureq::Error::Tls("the certificate expired"),
				"the certificate expired",
			),
			// The kinds the match doesn't name stay undescribed, and the
			// URL, which carries the signature, is never quoted.
			(ureq::Error::StatusCode(418), "the request failed"),
		];

		for (error, reason) in reasons {
			let message = request_error("GET", "app/production/index.json", &error).to_string();
			assert!(
				message.contains("GET app/production/index.json: "),
				"{message}"
			);
			assert!(message.ends_with(reason), "{message}");
			assert!(!message.contains("https://"), "{message}");
		}
	}

	/// A bucket whose endpoint has stopped listening.
	fn bucket_at_gone() -> S3Bucket {
		let gone = answering(0, "", "", b"");
		let address = gone.address.clone();
		drop(gone);
		let location = S3Location {
			endpoint: format!("http://{address}"),
			bucket: "test".to_owned(),
			region: "local".to_owned(),
			prefix: None,
			path_style: true,
		};
		S3Bucket::new(&location, Credentials::new("key", "secret")).expect("bucket")
	}

	/// Serves a head promising more bytes than it sends, then closes, so
	/// reading the body fails.
	fn truncated_body_server() -> String {
		let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
		let address = listener.local_addr().expect("address").to_string();
		std::thread::spawn(move || {
			let (mut stream, _) = listener.accept().expect("accept");
			let mut reader = std::io::BufReader::new(&mut stream);
			let mut line = String::new();

			loop {
				line.clear();
				assert_ne!(reader.read_line(&mut line).expect("request"), 0);

				if line == "\r\n" {
					break;
				}
			}

			write!(
				stream,
				"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 64\r\n\r\nshort"
			)
			.expect("head");
		});
		address
	}

	#[test]
	fn a_failed_connection_and_a_truncated_body_name_the_object() {
		let index = ObjectKey::index("app", "production");
		let object = StaticObject {
			key: index.clone(),
			bytes: b"the index".to_vec(),
		};

		// The PUT never connects.
		let mut bucket = bucket_at_gone();
		let error = bucket.put(&object).expect_err("refused").to_string();
		assert!(error.contains("PUT app/production/index.json: "), "{error}");
		assert!(!error.contains("http://"), "{error}");

		// The GET's body is shorter than its Content-Length promises.
		let address = truncated_body_server();
		let location = S3Location {
			endpoint: format!("http://{address}"),
			bucket: "test".to_owned(),
			region: "local".to_owned(),
			prefix: None,
			path_style: true,
		};
		let mut bucket =
			S3Bucket::new(&location, Credentials::new("key", "secret")).expect("bucket");
		let error = bucket.get(&index).expect_err("truncated").to_string();
		assert!(error.contains("GET app/production/index.json: "), "{error}");
		assert!(!error.contains("http://"), "{error}");
	}

	#[test]
	fn status_errors_hint_about_the_credentials_on_401_and_403() {
		for status in [401u16, 403] {
			let message = status_error("PUT", "app/production/index.json", status).to_string();
			assert!(
				message.ends_with(
					"check the credentials and their access to the bucket (on AWS a missing \
					 object also reads as 403 unless the credentials may list the bucket)"
				),
				"{message}"
			);
		}

		let message = status_error("PUT", "app/production/index.json", 503).to_string();
		assert!(message.ends_with("HTTP 503"), "{message}");
	}

	#[test]
	fn a_plain_http_endpoint_where_tls_is_expected_is_described_safely() {
		// The endpoint expects TLS, so this listener answers the ClientHello
		// with a fatal `handshake_failure` alert and closes.
		let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
		let address = listener.local_addr().expect("address").to_string();
		let server = std::thread::spawn(move || {
			let (mut stream, _) = listener.accept().expect("accept");
			let _ = stream.write_all(&[0x15, 0x03, 0x03, 0x00, 0x02, 0x02, 0x28]);
		});
		let location = S3Location {
			endpoint: format!("https://{address}"),
			bucket: "test".to_owned(),
			region: "local".to_owned(),
			prefix: None,
			path_style: true,
		};
		let mut bucket =
			S3Bucket::new(&location, Credentials::new("key", "secret")).expect("bucket");
		let message = bucket
			.get(&ObjectKey::index("app", "production"))
			.expect_err("the handshake is refused")
			.to_string();
		server.join().expect("server");
		assert!(
			message.contains("GET app/production/index.json: "),
			"{message}"
		);
		assert!(!message.contains("https://"), "{message}");
	}
}
