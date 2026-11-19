//! Where source archives and logs are kept: an S3-compatible bucket in
//! production, or a directory on this machine for development and tests.
//!
//! Keys are the control plane's own, made from IDs and digests
//! ([`archive_key`], [`log_key`]), never from a path a client sent. Clients
//! and workers never get a credential for the store: they get a presigned
//! address for one object and one method, which expires.
//!
//! An upload's address is bound to the archive's size and SHA-256. On S3
//! the `PUT` signs `content-length` and `x-amz-checksum-sha256`, so the
//! store itself refuses other bytes; the local store hashes the body as it
//! streams it to disk and keeps it only when both match. Either way,
//! [`ObjectStore::verify`] checks what was stored before an archive can be
//! deployed: from the store's checksum when it reports one, and otherwise
//! by streaming the object through SHA-256 a chunk at a time, so a 512 MiB
//! archive is never held in memory.
//!
//! The local store's addresses are the control plane's own routes
//! (`/_leptoskit/storage/…`), signed with a key made when it starts, so they
//! stop working when it stops. It refuses an origin that isn't loopback.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use futures::StreamExt as _;
use hmac::Mac as _;
use leptoskit_axum::axum::Router;
use leptoskit_axum::axum::body::Body;
use leptoskit_axum::axum::extract::Path as UrlPath;
use leptoskit_axum::axum::extract::Query;
use leptoskit_axum::axum::extract::State;
use leptoskit_axum::axum::http::HeaderMap;
use leptoskit_axum::axum::http::StatusCode;
use leptoskit_axum::axum::response::IntoResponse;
use leptoskit_axum::axum::response::Response;
use leptoskit_axum::axum::routing::get;
use leptoskit_db::Timestamp;
use rusty_s3::S3Action as _;
use serde::Deserialize;
use sha2::Digest as _;
use subtle::ConstantTimeEq as _;
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;
use uuid::Uuid;

use crate::archive::Digest;

/// Where the local store's routes are served.
pub const LOCAL_STORAGE_PATH: &str = "/_leptoskit/storage";

/// How much of an object [`ObjectStore::get`] reads at most.
pub const MAX_SMALL_OBJECT: usize = 1024 * 1024;

/// How long a request to the store may take.
const REQUEST_TIMEOUT: Duration = Duration::from_mins(10);

/// How long the store's own presigned requests live.
const SELF_PRESIGN: Duration = Duration::from_mins(5);

type HmacSha256 = hmac::Hmac<sha2::Sha256>;

/// The two buckets: archives, and logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bucket {
	Sources,
	Logs,
}

impl Bucket {
	/// The bucket's name in the local store and on its routes.
	const fn name(self) -> &'static str {
		match self {
			Self::Sources => "sources",
			Self::Logs => "logs",
		}
	}

	fn parse(name: &str) -> Option<Self> {
		match name {
			"sources" => Some(Self::Sources),
			"logs" => Some(Self::Logs),
			_ => None,
		}
	}
}

/// The key an organisation's archive is kept under: one per archive row,
/// so bytes uploaded again after a row was deleted go under a new key, and
/// the deletion of the old row's object can never remove them.
#[must_use]
pub fn archive_key(organization: Uuid, archive: Uuid, digest: &Digest) -> String {
	format!("sources/{organization}/{archive}/{}", digest.hex())
}

/// The key a chunk of a log is kept under.
#[must_use]
pub fn log_key(organization: Uuid, environment: Uuid, chunk: Uuid) -> String {
	format!("logs/{organization}/{environment}/{chunk}")
}

/// Whether `key` is one this module made: segments of lower-case letters,
/// digits, and `-`, with no empty or dot segment.
fn is_key(key: &str) -> bool {
	!key.is_empty()
		&& key.len() <= 300
		&& key.split('/').all(|segment| {
			!segment.is_empty()
				&& segment
					.bytes()
					.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
		})
}

/// A secret value from the environment: its `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
	/// Wraps `value`.
	#[must_use]
	pub fn new(value: impl Into<String>) -> Self {
		Self(value.into())
	}

	/// The value, for the one place that uses it.
	#[must_use]
	pub fn expose(&self) -> &str {
		&self.0
	}
}

impl std::fmt::Debug for SecretValue {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str("[redacted]")
	}
}

/// An S3-compatible store: its address, region, buckets, and credentials.
#[derive(Clone, Debug)]
pub struct S3Config {
	/// Such as `https://<account>.r2.cloudflarestorage.com`.
	pub endpoint: url::Url,
	/// Such as `auto`.
	pub region: String,
	pub sources_bucket: String,
	pub logs_bucket: String,
	pub access_key_id: String,
	pub secret_access_key: SecretValue,
	/// Address buckets as `endpoint/bucket` rather than `bucket.endpoint`.
	pub path_style: bool,
	/// Sign `x-amz-checksum-sha256` on presigned PUTs and request
	/// `x-amz-checksum-mode: ENABLED` on HEAD. Disable for S3-compatible stores
	/// that reject checksum headers.
	pub checksum_headers: bool,
}

/// Where objects are kept.
#[derive(Clone, Debug)]
pub enum StorageConfig {
	/// An S3-compatible store.
	S3(S3Config),
	/// A directory on this machine, served by the control plane on
	/// loopback: for development and tests.
	Local {
		/// Where the buckets' directories go.
		directory: PathBuf,
	},
}

/// Why the store failed. The message never carries a presigned address.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
	/// The store's configuration can't be used.
	#[error("object store: {0}")]
	Config(String),
	/// A request didn't get an answer.
	#[error("object store: {0}")]
	Request(String),
	/// The store answered with an error.
	#[error("object store: {method} {key} answered {status}")]
	Status {
		method: &'static str,
		key: String,
		status: u16,
	},
	/// The local store's disk failed.
	#[error("object store: {0}")]
	Io(String),
}

impl From<StoreError> for leptoskit_rpc::Error {
	fn from(error: StoreError) -> Self {
		Self::retry(format!("storage_unavailable: {error}"))
	}
}

/// A presigned request: where to send it, the headers to send with it, and
/// when it stops working.
#[derive(Clone, PartialEq, Eq)]
pub struct Presigned {
	pub url: String,
	pub headers: BTreeMap<String, String>,
	pub expires_at: Timestamp,
}

impl std::fmt::Debug for Presigned {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Presigned")
			.field("url", &"[redacted]")
			.field("expires_at", &self.expires_at)
			.finish_non_exhaustive()
	}
}

/// What [`ObjectStore::verify`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verification {
	/// The object has the size and digest it should.
	Matches,
	/// There is no object.
	Missing,
	/// There is, and it isn't what was promised: why.
	Mismatch(String),
}

/// The object store, shared by every request.
#[derive(Clone)]
pub struct ObjectStore {
	inner: Arc<Inner>,
}

enum Inner {
	S3(Box<S3Store>),
	Local(LocalStore),
}

impl std::fmt::Debug for ObjectStore {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match &*self.inner {
			Inner::S3(_) => formatter.write_str("ObjectStore::S3"),
			Inner::Local(local) => {
				formatter
					.debug_struct("ObjectStore::Local")
					.field("root", &local.root)
					.finish()
			}
		}
	}
}

impl ObjectStore {
	/// The store `config` describes. A local store's addresses are on
	/// `origin`, which must be loopback.
	///
	/// # Errors
	///
	/// [`StoreError::Config`] for a configuration that can't be used.
	pub fn new(config: &StorageConfig, origin: &str) -> Result<Self, StoreError> {
		let inner = match config {
			StorageConfig::S3(s3) => Inner::S3(Box::new(S3Store::new(s3)?)),
			StorageConfig::Local { directory } => Inner::Local(LocalStore::new(directory, origin)?),
		};
		Ok(Self {
			inner: Arc::new(inner),
		})
	}

	/// The values the store was configured with that must never reach a
	/// log: its secret key.
	pub(crate) fn secrets(&self) -> Vec<String> {
		match &*self.inner {
			Inner::S3(s3) => vec![s3.secret.clone()],
			Inner::Local(_) => Vec::new(),
		}
	}

	/// The local store's routes, when it is one.
	pub(crate) fn routes(&self) -> Option<Router> {
		match &*self.inner {
			Inner::S3(_) => None,
			Inner::Local(_) => {
				Some(
					Router::new()
						.route(
							&format!("{LOCAL_STORAGE_PATH}/{{bucket}}/{{*key}}"),
							get(local_get).put(local_put),
						)
						.with_state(self.clone()),
				)
			}
		}
	}

	/// An address to `PUT` `size` bytes with SHA-256 `digest` to, for `ttl`.
	///
	/// # Errors
	///
	/// [`StoreError::Config`] for a key this module didn't make.
	pub fn presign_put(
		&self,
		bucket: Bucket,
		key: &str,
		size: u64,
		digest: &Digest,
		ttl: Duration,
	) -> Result<Presigned, StoreError> {
		check_key(key)?;
		let expires_at = expiry(ttl);

		match &*self.inner {
			Inner::S3(s3) => Ok(s3.presign_put(bucket, key, size, digest, ttl, expires_at)),
			Inner::Local(local) => {
				let expires = expires_at.timestamp();
				let message = signed_message("put", bucket, key, size, digest.hex(), expires);
				let signature = local.sign(&message);
				Ok(Presigned {
					url: format!(
						"{}{LOCAL_STORAGE_PATH}/{}/{key}?size={size}&digest={}&expires={expires}&\
						 sig={signature}",
						local.origin,
						bucket.name(),
						digest.hex()
					),
					headers: BTreeMap::from([("content-length".to_owned(), size.to_string())]),
					expires_at,
				})
			}
		}
	}

	/// An address to `GET` one object from, for `ttl`.
	///
	/// # Errors
	///
	/// [`StoreError::Config`] for a key this module didn't make.
	pub fn presign_get(
		&self,
		bucket: Bucket,
		key: &str,
		ttl: Duration,
	) -> Result<Presigned, StoreError> {
		check_key(key)?;
		let expires_at = expiry(ttl);

		match &*self.inner {
			Inner::S3(s3) => {
				let url = s3
					.bucket(bucket)
					.get_object(Some(&s3.credentials), key)
					.sign(ttl);
				Ok(Presigned {
					url: url.to_string(),
					headers: BTreeMap::new(),
					expires_at,
				})
			}
			Inner::Local(local) => {
				let expires = expires_at.timestamp();
				let message = signed_message("get", bucket, key, 0, "", expires);
				let signature = local.sign(&message);
				Ok(Presigned {
					url: format!(
						"{}{LOCAL_STORAGE_PATH}/{}/{key}?expires={expires}&sig={signature}",
						local.origin,
						bucket.name()
					),
					headers: BTreeMap::new(),
					expires_at,
				})
			}
		}
	}

	/// Whether the object at `key` has `size` bytes with SHA-256 `digest`.
	///
	/// # Errors
	///
	/// [`StoreError`] when the store can't be asked.
	pub async fn verify(
		&self,
		bucket: Bucket,
		key: &str,
		size: u64,
		digest: &Digest,
	) -> Result<Verification, StoreError> {
		check_key(key)?;

		match &*self.inner {
			Inner::S3(s3) => s3.verify(bucket, key, size, digest).await,
			Inner::Local(local) => local.verify(bucket, key, size, digest).await,
		}
	}

	/// Stores a small object.
	///
	/// # Errors
	///
	/// [`StoreError`] when the store refuses it.
	pub async fn put(&self, bucket: Bucket, key: &str, bytes: Vec<u8>) -> Result<(), StoreError> {
		check_key(key)?;

		match &*self.inner {
			Inner::S3(s3) => s3.put(bucket, key, bytes).await,
			Inner::Local(local) => local.put(bucket, key, &bytes).await,
		}
	}

	/// A small object, or `None` when there is none. At most
	/// [`MAX_SMALL_OBJECT`] bytes are read.
	///
	/// # Errors
	///
	/// [`StoreError`] when the store can't be read, or the object is larger.
	pub async fn get(&self, bucket: Bucket, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
		check_key(key)?;

		match &*self.inner {
			Inner::S3(s3) => s3.get(bucket, key).await,
			Inner::Local(local) => local.get(bucket, key).await,
		}
	}

	/// Deletes an object; deleting one that isn't there succeeds.
	///
	/// # Errors
	///
	/// [`StoreError`] when the store refuses.
	pub async fn delete(&self, bucket: Bucket, key: &str) -> Result<(), StoreError> {
		check_key(key)?;

		match &*self.inner {
			Inner::S3(s3) => s3.delete(bucket, key).await,
			Inner::Local(local) => local.delete(bucket, key).await,
		}
	}

	fn local(&self) -> Option<&LocalStore> {
		match &*self.inner {
			Inner::Local(local) => Some(local),
			Inner::S3(_) => None,
		}
	}
}

fn check_key(key: &str) -> Result<(), StoreError> {
	if is_key(key) {
		Ok(())
	} else {
		Err(StoreError::Config(
			"refused a key it didn't make".to_owned(),
		))
	}
}

fn expiry(ttl: Duration) -> Timestamp {
	leptoskit_db::now()
		+ chrono::TimeDelta::from_std(ttl).unwrap_or_else(|_| chrono::TimeDelta::minutes(15))
}

fn signed_message(
	method: &str,
	bucket: Bucket,
	key: &str,
	size: u64,
	digest: &str,
	expires: i64,
) -> String {
	format!(
		"{method}\n{}\n{key}\n{size}\n{digest}\n{expires}",
		bucket.name()
	)
}

/// What hashing a stream found.
struct Hashed {
	size: u64,
	digest: String,
}

fn finish_hash(hasher: sha2::Sha256, size: u64) -> Hashed {
	Hashed {
		size,
		digest: format!("sha256:{}", hex::encode(hasher.finalize())),
	}
}

fn compare(hashed: &Hashed, size: u64, digest: &Digest) -> Verification {
	if hashed.size != size {
		Verification::Mismatch(format!(
			"it has {} bytes, not the {size} promised",
			hashed.size
		))
	} else if hashed.digest != digest.as_str() {
		Verification::Mismatch(format!("its SHA-256 is {}, not {digest}", hashed.digest))
	} else {
		Verification::Matches
	}
}

struct S3Store {
	sources: rusty_s3::Bucket,
	logs: rusty_s3::Bucket,
	credentials: rusty_s3::Credentials,
	secret: String,
	client: reqwest::Client,
	checksum_headers: bool,
}

impl S3Store {
	fn new(config: &S3Config) -> Result<Self, StoreError> {
		let style = if config.path_style {
			rusty_s3::UrlStyle::Path
		} else {
			rusty_s3::UrlStyle::VirtualHost
		};
		let bucket = |name: &str| {
			rusty_s3::Bucket::new(
				config.endpoint.clone(),
				style,
				name.to_owned(),
				config.region.clone(),
			)
			.map_err(|_| StoreError::Config(format!("can't address the bucket `{name}`")))
		};
		let client = reqwest::Client::builder()
			.timeout(REQUEST_TIMEOUT)
			.build()
			.map_err(|_| StoreError::Config("can't make an HTTP client".to_owned()))?;

		Ok(Self {
			sources: bucket(&config.sources_bucket)?,
			logs: bucket(&config.logs_bucket)?,
			credentials: rusty_s3::Credentials::new(
				config.access_key_id.clone(),
				config.secret_access_key.expose().to_owned(),
			),
			secret: config.secret_access_key.expose().to_owned(),
			client,
			checksum_headers: config.checksum_headers,
		})
	}

	const fn bucket(&self, bucket: Bucket) -> &rusty_s3::Bucket {
		match bucket {
			Bucket::Sources => &self.sources,
			Bucket::Logs => &self.logs,
		}
	}

	fn presign_put(
		&self,
		bucket: Bucket,
		key: &str,
		size: u64,
		digest: &Digest,
		ttl: Duration,
		expires_at: Timestamp,
	) -> Presigned {
		let mut action = self.bucket(bucket).put_object(Some(&self.credentials), key);
		action
			.headers_mut()
			.insert("content-length", size.to_string());
		let mut headers = BTreeMap::from([("content-length".to_owned(), size.to_string())]);
		if self.checksum_headers {
			let checksum = checksum_header(digest);
			action
				.headers_mut()
				.insert("x-amz-checksum-sha256", checksum.clone());
			headers.insert("x-amz-checksum-sha256".to_owned(), checksum);
		}
		Presigned {
			url: action.sign(ttl).to_string(),
			headers,
			expires_at,
		}
	}

	async fn head_plain(&self, bucket: Bucket, key: &str) -> Result<reqwest::Response, StoreError> {
		let head = self
			.bucket(bucket)
			.head_object(Some(&self.credentials), key);
		self.client
			.head(head.sign(SELF_PRESIGN).as_str())
			.send()
			.await
			.map_err(|_| StoreError::Request(format!("HEAD {key} got no answer")))
	}

	async fn verify(
		&self,
		bucket: Bucket,
		key: &str,
		size: u64,
		digest: &Digest,
	) -> Result<Verification, StoreError> {
		let response = if self.checksum_headers {
			let mut head = self
				.bucket(bucket)
				.head_object(Some(&self.credentials), key);
			head.headers_mut().insert("x-amz-checksum-mode", "ENABLED");
			let resp = self
				.client
				.head(head.sign(SELF_PRESIGN).as_str())
				.header("x-amz-checksum-mode", "ENABLED")
				.send()
				.await
				.map_err(|_| StoreError::Request(format!("HEAD {key} got no answer")))?;
			// S3-compatible stores that don't support `x-amz-checksum-mode`
			// may answer 400 or 501; fall back to a plain HEAD and streaming
			// verification.
			if matches!(resp.status().as_u16(), 400 | 501) {
				self.head_plain(bucket, key).await?
			} else {
				resp
			}
		} else {
			self.head_plain(bucket, key).await?
		};

		match response.status().as_u16() {
			404 => return Ok(Verification::Missing),
			status if !(200..300).contains(&status) => {
				return Err(StoreError::Status {
					method: "HEAD",
					key: key.to_owned(),
					status,
				});
			}
			_ => {}
		}

		let length = response
			.headers()
			.get("content-length")
			.and_then(|value| value.to_str().ok())
			.and_then(|value| value.parse::<u64>().ok());

		if let Some(length) = length
			&& length != size
		{
			return Ok(Verification::Mismatch(format!(
				"it has {length} bytes, not the {size} promised"
			)));
		}

		// The store's own checksum, when it keeps one, settles it without
		// reading the object.
		if let Some(checksum) = response
			.headers()
			.get("x-amz-checksum-sha256")
			.and_then(|value| value.to_str().ok())
			.filter(|value| !value.contains('-'))
		{
			return Ok(if checksum == checksum_header(digest) {
				Verification::Matches
			} else {
				Verification::Mismatch("its SHA-256 isn't the one promised".to_owned())
			});
		}

		self.hash_by_reading(bucket, key, size, digest).await
	}

	/// Streams the object through SHA-256, a chunk at a time, giving up as
	/// soon as it is longer than promised.
	async fn hash_by_reading(
		&self,
		bucket: Bucket,
		key: &str,
		size: u64,
		digest: &Digest,
	) -> Result<Verification, StoreError> {
		let url = self
			.bucket(bucket)
			.get_object(Some(&self.credentials), key)
			.sign(SELF_PRESIGN);
		let mut response = self
			.client
			.get(url.as_str())
			.send()
			.await
			.map_err(|_| StoreError::Request(format!("GET {key} got no answer")))?;

		match response.status().as_u16() {
			404 => return Ok(Verification::Missing),
			status if !(200..300).contains(&status) => {
				return Err(StoreError::Status {
					method: "GET",
					key: key.to_owned(),
					status,
				});
			}
			_ => {}
		}

		let mut hasher = sha2::Sha256::new();
		let mut total: u64 = 0;

		while let Some(chunk) = response
			.chunk()
			.await
			.map_err(|_| StoreError::Request(format!("GET {key} stopped")))?
		{
			total = total.saturating_add(chunk.len() as u64);

			if total > size {
				return Ok(Verification::Mismatch(format!(
					"it has more than the {size} bytes promised"
				)));
			}

			hasher.update(&chunk);
		}

		Ok(compare(&finish_hash(hasher, total), size, digest))
	}

	async fn put(&self, bucket: Bucket, key: &str, bytes: Vec<u8>) -> Result<(), StoreError> {
		let url = self
			.bucket(bucket)
			.put_object(Some(&self.credentials), key)
			.sign(SELF_PRESIGN);
		let response = self
			.client
			.put(url.as_str())
			.body(bytes)
			.send()
			.await
			.map_err(|_| StoreError::Request(format!("PUT {key} got no answer")))?;
		let status = response.status().as_u16();

		if (200..300).contains(&status) {
			Ok(())
		} else {
			Err(StoreError::Status {
				method: "PUT",
				key: key.to_owned(),
				status,
			})
		}
	}

	async fn get(&self, bucket: Bucket, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
		let url = self
			.bucket(bucket)
			.get_object(Some(&self.credentials), key)
			.sign(SELF_PRESIGN);
		let mut response = self
			.client
			.get(url.as_str())
			.send()
			.await
			.map_err(|_| StoreError::Request(format!("GET {key} got no answer")))?;

		match response.status().as_u16() {
			404 => return Ok(None),
			status if !(200..300).contains(&status) => {
				return Err(StoreError::Status {
					method: "GET",
					key: key.to_owned(),
					status,
				});
			}
			_ => {}
		}

		let mut out = Vec::new();

		while let Some(chunk) = response
			.chunk()
			.await
			.map_err(|_| StoreError::Request(format!("GET {key} stopped")))?
		{
			if out.len() + chunk.len() > MAX_SMALL_OBJECT {
				return Err(StoreError::Request(format!("{key} is too large to read")));
			}

			out.extend_from_slice(&chunk);
		}

		Ok(Some(out))
	}

	async fn delete(&self, bucket: Bucket, key: &str) -> Result<(), StoreError> {
		let url = self
			.bucket(bucket)
			.delete_object(Some(&self.credentials), key)
			.sign(SELF_PRESIGN);
		let response = self
			.client
			.delete(url.as_str())
			.send()
			.await
			.map_err(|_| StoreError::Request(format!("DELETE {key} got no answer")))?;
		let status = response.status().as_u16();

		if (200..300).contains(&status) || status == 404 {
			Ok(())
		} else {
			Err(StoreError::Status {
				method: "DELETE",
				key: key.to_owned(),
				status,
			})
		}
	}
}

/// The `x-amz-checksum-sha256` value for `digest`: its 32 bytes in base64.
fn checksum_header(digest: &Digest) -> String {
	let bytes = hex::decode(digest.hex()).unwrap_or_default();
	base64::engine::general_purpose::STANDARD.encode(bytes)
}

struct LocalStore {
	root: PathBuf,
	origin: String,
	key: [u8; 32],
}

impl LocalStore {
	fn new(directory: &Path, origin: &str) -> Result<Self, StoreError> {
		let host = url::Url::parse(origin)
			.ok()
			.and_then(|url| url.host_str().map(str::to_owned))
			.unwrap_or_default();

		if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]") {
			return Err(StoreError::Config(format!(
				"the local store serves on loopback only, and the origin is `{origin}`"
			)));
		}

		let mut key = [0_u8; 32];
		getrandom::fill(&mut key)
			.map_err(|_| StoreError::Config("no randomness to sign addresses with".to_owned()))?;
		Ok(Self {
			root: directory.to_owned(),
			origin: origin.trim_end_matches('/').to_owned(),
			key,
		})
	}

	fn sign(&self, message: &str) -> String {
		let mut mac = <HmacSha256 as hmac::Mac>::new_from_slice(&self.key)
			.unwrap_or_else(|_| unreachable!("HMAC takes a key of any length"));
		mac.update(message.as_bytes());
		hex::encode(mac.finalize().into_bytes())
	}

	fn signature_matches(&self, message: &str, signature: &str) -> bool {
		bool::from(self.sign(message).as_bytes().ct_eq(signature.as_bytes()))
	}

	fn path(&self, bucket: Bucket, key: &str) -> PathBuf {
		let mut path = self.root.join(bucket.name());

		for segment in key.split('/') {
			path.push(segment);
		}

		path
	}

	async fn verify(
		&self,
		bucket: Bucket,
		key: &str,
		size: u64,
		digest: &Digest,
	) -> Result<Verification, StoreError> {
		let mut file = match tokio::fs::File::open(self.path(bucket, key)).await {
			Ok(file) => file,
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
				return Ok(Verification::Missing);
			}
			Err(error) => return Err(StoreError::Io(format!("can't open {key}: {error}"))),
		};
		let mut hasher = sha2::Sha256::new();
		let mut total: u64 = 0;
		let mut buffer = vec![0_u8; 64 * 1024];

		loop {
			let read = file
				.read(&mut buffer)
				.await
				.map_err(|error| StoreError::Io(format!("can't read {key}: {error}")))?;

			if read == 0 {
				break;
			}

			total = total.saturating_add(read as u64);

			if total > size {
				return Ok(Verification::Mismatch(format!(
					"it has more than the {size} bytes promised"
				)));
			}

			hasher.update(&buffer[..read]);
		}

		Ok(compare(&finish_hash(hasher, total), size, digest))
	}

	async fn put(&self, bucket: Bucket, key: &str, bytes: &[u8]) -> Result<(), StoreError> {
		let path = self.path(bucket, key);
		let temporary = self.temporary(bucket).await?;
		tokio::fs::write(&temporary, bytes)
			.await
			.map_err(|error| StoreError::Io(format!("can't write {key}: {error}")))?;
		place(&temporary, &path, key).await
	}

	async fn temporary(&self, bucket: Bucket) -> Result<PathBuf, StoreError> {
		let directory = self.root.join(bucket.name()).join(".incoming");
		tokio::fs::create_dir_all(&directory)
			.await
			.map_err(|error| {
				StoreError::Io(format!("can't make the store's directory: {error}"))
			})?;
		Ok(directory.join(Uuid::now_v7().to_string()))
	}

	async fn get(&self, bucket: Bucket, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
		match tokio::fs::read(self.path(bucket, key)).await {
			Ok(bytes) if bytes.len() > MAX_SMALL_OBJECT => {
				Err(StoreError::Io(format!("{key} is too large to read")))
			}
			Ok(bytes) => Ok(Some(bytes)),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
			Err(error) => Err(StoreError::Io(format!("can't read {key}: {error}"))),
		}
	}

	async fn delete(&self, bucket: Bucket, key: &str) -> Result<(), StoreError> {
		match tokio::fs::remove_file(self.path(bucket, key)).await {
			Ok(()) => Ok(()),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
			Err(error) => Err(StoreError::Io(format!("can't delete {key}: {error}"))),
		}
	}
}

/// Moves a finished temporary file into place.
async fn place(temporary: &Path, path: &Path, key: &str) -> Result<(), StoreError> {
	if let Some(parent) = path.parent() {
		tokio::fs::create_dir_all(parent)
			.await
			.map_err(|error| StoreError::Io(format!("can't make {key}'s directory: {error}")))?;
	}

	tokio::fs::rename(temporary, path)
		.await
		.map_err(|error| StoreError::Io(format!("can't keep {key}: {error}")))
}

/// The local store's query: a signature, its expiry, and for a `PUT` the
/// size and digest it is bound to.
#[derive(Deserialize)]
struct Signed {
	expires: i64,
	sig: String,
	#[serde(default)]
	size: Option<u64>,
	#[serde(default)]
	digest: Option<String>,
}

fn refusal(status: StatusCode, message: &str) -> Response {
	(status, message.to_owned()).into_response()
}

/// Checks a local address: a bucket, a key this module makes, a matching
/// signature, and an expiry in the future.
fn checked(
	store: &ObjectStore,
	bucket: &str,
	key: &str,
	method: &str,
	query: &Signed,
) -> Result<Bucket, (StatusCode, &'static str)> {
	let Some(local) = store.local() else {
		return Err((StatusCode::NOT_FOUND, "not found"));
	};
	let Some(bucket) = Bucket::parse(bucket).filter(|_| is_key(key)) else {
		return Err((StatusCode::NOT_FOUND, "not found"));
	};
	let message = signed_message(
		method,
		bucket,
		key,
		query.size.unwrap_or_default(),
		query.digest.as_deref().unwrap_or_default(),
		query.expires,
	);

	if !local.signature_matches(&message, &query.sig) {
		return Err((
			StatusCode::FORBIDDEN,
			"signature_mismatch: this address isn't one the control plane signed",
		));
	}

	if query.expires < leptoskit_db::now().timestamp() {
		return Err((
			StatusCode::FORBIDDEN,
			"expired: this address has expired; ask for a new one",
		));
	}

	Ok(bucket)
}

async fn local_get(
	State(store): State<ObjectStore>,
	UrlPath((bucket, key)): UrlPath<(String, String)>,
	Query(query): Query<Signed>,
) -> Response {
	let bucket = match checked(&store, &bucket, &key, "get", &query) {
		Ok(bucket) => bucket,
		Err((status, message)) => return refusal(status, message),
	};
	let Some(local) = store.local() else {
		return refusal(StatusCode::NOT_FOUND, "not found");
	};

	match tokio::fs::File::open(local.path(bucket, &key)).await {
		Ok(file) => {
			let stream = tokio_util_stream(file);
			Body::from_stream(stream).into_response()
		}
		Err(_) => refusal(StatusCode::NOT_FOUND, "not found"),
	}
}

/// A file as a stream of chunks, for a response body.
fn tokio_util_stream(
	file: tokio::fs::File,
) -> impl futures::Stream<Item = Result<Vec<u8>, std::io::Error>> + Send {
	futures::stream::unfold(Some(file), |file| {
		async move {
			let mut file = file?;
			let mut buffer = vec![0_u8; 64 * 1024];

			match file.read(&mut buffer).await {
				Ok(0) => None,
				Ok(read) => {
					buffer.truncate(read);
					Some((Ok(buffer), Some(file)))
				}
				Err(error) => Some((Err(error), None)),
			}
		}
	})
}

/// `PUT`: the body streams to a temporary file through SHA-256, and is
/// kept only when its size and digest are the ones the address is bound
/// to. Nothing larger than promised is written.
async fn local_put(
	State(store): State<ObjectStore>,
	UrlPath((bucket, key)): UrlPath<(String, String)>,
	Query(query): Query<Signed>,
	headers: HeaderMap,
	body: Body,
) -> Response {
	let bucket = match checked(&store, &bucket, &key, "put", &query) {
		Ok(bucket) => bucket,
		Err((status, message)) => return refusal(status, message),
	};
	let (Some(local), Some(size), Some(digest)) = (
		store.local(),
		query.size,
		query
			.digest
			.as_deref()
			.and_then(|hex| Digest::parse(&format!("sha256:{hex}")).ok()),
	) else {
		return refusal(
			StatusCode::BAD_REQUEST,
			"an upload address names its size and digest",
		);
	};

	if headers
		.get("content-length")
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.parse::<u64>().ok())
		.is_some_and(|length| length != size)
	{
		return refusal(
			StatusCode::BAD_REQUEST,
			"upload_mismatch: content-length isn't the size this address is bound to",
		);
	}

	match receive(local, bucket, &key, size, &digest, body).await {
		Ok(Verification::Matches) => StatusCode::OK.into_response(),
		Ok(Verification::Mismatch(why)) => {
			refusal(StatusCode::BAD_REQUEST, &format!("upload_mismatch: {why}"))
		}
		Ok(Verification::Missing) | Err(_) => {
			refusal(
				StatusCode::INTERNAL_SERVER_ERROR,
				"the upload couldn't be kept",
			)
		}
	}
}

async fn receive(
	local: &LocalStore,
	bucket: Bucket,
	key: &str,
	size: u64,
	digest: &Digest,
	body: Body,
) -> Result<Verification, StoreError> {
	let temporary = local.temporary(bucket).await?;
	let mut file = tokio::fs::File::create(&temporary)
		.await
		.map_err(|error| StoreError::Io(format!("can't receive {key}: {error}")))?;
	let mut stream = body.into_data_stream();
	let mut hasher = sha2::Sha256::new();
	let mut total: u64 = 0;
	let mut verdict = None;

	while let Some(chunk) = stream.next().await {
		let Ok(chunk) = chunk else {
			verdict = Some(Verification::Mismatch(
				"the upload stopped part-way".to_owned(),
			));
			break;
		};
		total = total.saturating_add(chunk.len() as u64);

		if total > size {
			verdict = Some(Verification::Mismatch(format!(
				"it has more than the {size} bytes promised"
			)));
			break;
		}

		hasher.update(&chunk);
		file.write_all(&chunk)
			.await
			.map_err(|error| StoreError::Io(format!("can't receive {key}: {error}")))?;
	}

	let verdict = verdict.unwrap_or_else(|| compare(&finish_hash(hasher, total), size, digest));
	file.flush()
		.await
		.map_err(|error| StoreError::Io(format!("can't receive {key}: {error}")))?;
	drop(file);

	if verdict == Verification::Matches {
		place(&temporary, &local.path(bucket, key), key).await?;
	} else {
		let _ = tokio::fs::remove_file(&temporary).await;
	}

	Ok(verdict)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn keys_are_the_control_planes_own() {
		let organization = Uuid::now_v7();
		let digest = Digest::parse(&format!("sha256:{}", "a".repeat(64))).unwrap();
		assert!(is_key(&archive_key(organization, Uuid::now_v7(), &digest)));
		assert!(is_key(&log_key(
			organization,
			Uuid::now_v7(),
			Uuid::now_v7()
		)));

		for bad in ["", "../x", "a//b", "a/./b", "a/B", "/a", "a/"] {
			assert!(!is_key(bad), "{bad}");
		}
	}

	#[test]
	fn the_local_store_is_loopback_only() {
		let directory = std::env::temp_dir();
		assert!(LocalStore::new(&directory, "http://127.0.0.1:8080").is_ok());
		assert!(LocalStore::new(&directory, "https://cloud.example.com").is_err());
	}

	#[test]
	fn a_secret_value_never_shows() {
		assert_eq!(format!("{:?}", SecretValue::new("hunter2")), "[redacted]");
	}

	#[test]
	fn the_checksum_header_is_the_digests_bytes_in_base64() {
		let digest = Digest::parse(
			"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
		)
		.unwrap();
		assert_eq!(
			checksum_header(&digest),
			"47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
		);
	}

	/// A fake S3 on loopback, path-style: it keeps objects in memory, notes
	/// each request's method, path and query, and reports a stored
	/// checksum only when `checksums` is on, as stores without checksum
	/// support don't.
	mod fake_s3 {
		use std::collections::HashMap;
		use std::sync::Arc;
		use std::sync::Mutex;

		use leptoskit_axum::axum;
		use leptoskit_axum::axum::body::Bytes;
		use leptoskit_axum::axum::extract::State;
		use leptoskit_axum::axum::http::HeaderMap;
		use leptoskit_axum::axum::http::Method;
		use leptoskit_axum::axum::http::StatusCode;
		use leptoskit_axum::axum::http::Uri;
		use leptoskit_axum::axum::response::IntoResponse;
		use leptoskit_axum::axum::response::Response;

		/// Each object's bytes, and the checksum it was stored with.
		type Objects = HashMap<String, (Vec<u8>, Option<String>)>;

		#[derive(Clone, Default)]
		pub struct Fake {
			pub objects: Arc<Mutex<Objects>>,
			pub requests: Arc<Mutex<Vec<String>>>,
			pub checksums: Arc<Mutex<bool>>,
		}

		async fn handle(
			State(fake): State<Fake>,
			method: Method,
			uri: Uri,
			headers: HeaderMap,
			body: Bytes,
		) -> Response {
			let path = uri.path().to_owned();
			fake.requests.lock().unwrap().push(format!(
				"{method} {path}?{}",
				uri.query().unwrap_or_default()
			));
			let mut objects = fake.objects.lock().unwrap();

			match method {
				Method::PUT => {
					let checksum = headers
						.get("x-amz-checksum-sha256")
						.and_then(|value| value.to_str().ok())
						.map(ToOwned::to_owned);
					objects.insert(path, (body.to_vec(), checksum));
					StatusCode::OK.into_response()
				}
				Method::HEAD => {
					let Some((bytes, checksum)) = objects.get(&path) else {
						return StatusCode::NOT_FOUND.into_response();
					};
					// hyper sends no body for `HEAD`, but keeps its length.
					let mut response = bytes.clone().into_response();

					if *fake.checksums.lock().unwrap()
						&& let Some(checksum) = checksum
					{
						response
							.headers_mut()
							.insert("x-amz-checksum-sha256", checksum.parse().unwrap());
					}

					response
				}
				Method::GET => {
					match objects.get(&path) {
						Some((bytes, _)) => bytes.clone().into_response(),
						None => StatusCode::NOT_FOUND.into_response(),
					}
				}
				Method::DELETE => {
					objects.remove(&path);
					StatusCode::NO_CONTENT.into_response()
				}
				_ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
			}
		}

		/// Serves the fake, and returns its address.
		pub async fn serve(fake: Fake) -> String {
			let router = axum::Router::new().fallback(handle).with_state(fake);
			let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
			let address = listener.local_addr().unwrap();
			tokio::spawn(async move {
				axum::serve(listener, router).await.unwrap();
			});
			format!("http://{address}")
		}
	}

	fn digest_of(bytes: &[u8]) -> Digest {
		Digest::parse(&format!(
			"sha256:{}",
			hex::encode(sha2::Sha256::digest(bytes))
		))
		.unwrap()
	}

	fn s3_store(endpoint: &str) -> ObjectStore {
		ObjectStore::new(
			&StorageConfig::S3(S3Config {
				endpoint: url::Url::parse(endpoint).unwrap(),
				region: "auto".to_owned(),
				sources_bucket: "sources".to_owned(),
				logs_bucket: "logs".to_owned(),
				access_key_id: "AKIDEXAMPLE".to_owned(),
				secret_access_key: SecretValue::new("wJalrXUtnFEMI-secret"),
				path_style: true,
				checksum_headers: true,
			}),
			"https://cloud.example.com",
		)
		.unwrap()
	}

	/// An upload address binds the size and the SHA-256 into its signature,
	/// and the store's answer to `HEAD`, or the object itself, decides
	/// whether what was uploaded is what was promised.
	#[tokio::test]
	async fn the_s3_store_signs_and_verifies_against_a_fake() {
		let fake = fake_s3::Fake::default();
		let endpoint = fake_s3::serve(fake.clone()).await;
		let store = s3_store(&endpoint);
		assert_eq!(store.secrets(), ["wJalrXUtnFEMI-secret".to_owned()]);
		assert!(
			store.routes().is_none(),
			"an S3 store serves nothing itself"
		);
		let bytes = b"an archive".to_vec();
		let digest = digest_of(&bytes);
		let key = archive_key(Uuid::now_v7(), Uuid::now_v7(), &digest);
		let presigned = store
			.presign_put(
				Bucket::Sources,
				&key,
				bytes.len() as u64,
				&digest,
				Duration::from_mins(15),
			)
			.unwrap();
		let url = url::Url::parse(&presigned.url).unwrap();
		let signed: String = url
			.query_pairs()
			.find(|(name, _)| name == "X-Amz-SignedHeaders")
			.map(|(_, value)| value.into_owned())
			.unwrap();
		assert!(signed.contains("content-length"), "{signed}");
		assert!(signed.contains("x-amz-checksum-sha256"), "{signed}");
		assert!(!format!("{presigned:?}").contains("X-Amz-Signature"));
		assert_eq!(
			presigned.headers["x-amz-checksum-sha256"],
			checksum_header(&digest)
		);

		// Nothing yet.
		assert_eq!(
			store
				.verify(Bucket::Sources, &key, bytes.len() as u64, &digest)
				.await
				.unwrap(),
			Verification::Missing
		);

		// A client's upload, through the presigned address.
		let response = reqwest::Client::new()
			.put(presigned.url.as_str())
			.header(
				"x-amz-checksum-sha256",
				&presigned.headers["x-amz-checksum-sha256"],
			)
			.body(bytes.clone())
			.send()
			.await
			.unwrap();
		assert_eq!(response.status().as_u16(), 200);

		// Checked by reading it, when the store keeps no checksum…
		assert_eq!(
			store
				.verify(Bucket::Sources, &key, bytes.len() as u64, &digest)
				.await
				.unwrap(),
			Verification::Matches
		);
		// …and by the store's checksum when it does.
		*fake.checksums.lock().unwrap() = true;
		assert_eq!(
			store
				.verify(Bucket::Sources, &key, bytes.len() as u64, &digest)
				.await
				.unwrap(),
			Verification::Matches
		);
		let other = digest_of(b"another archive");
		assert!(matches!(
			store
				.verify(Bucket::Sources, &key, bytes.len() as u64, &other)
				.await
				.unwrap(),
			Verification::Mismatch(_)
		));
		assert!(matches!(
			store
				.verify(Bucket::Sources, &key, 3, &digest)
				.await
				.unwrap(),
			Verification::Mismatch(_)
		));

		// The control plane's own reads and writes, signed.
		let log = log_key(Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
		store.put(Bucket::Logs, &log, b"[]".to_vec()).await.unwrap();
		assert_eq!(
			store.get(Bucket::Logs, &log).await.unwrap(),
			Some(b"[]".to_vec())
		);
		store.delete(Bucket::Logs, &log).await.unwrap();
		assert_eq!(store.get(Bucket::Logs, &log).await.unwrap(), None);
		store.delete(Bucket::Logs, &log).await.unwrap();

		let requests = fake.requests.lock().unwrap().clone();
		assert!(
			requests
				.iter()
				.all(|request| request.contains("X-Amz-Signature=")),
			"{requests:?}"
		);
		assert!(
			requests
				.iter()
				.any(|request| request.starts_with("PUT /logs/"))
		);
		assert!(
			requests
				.iter()
				.any(|request| request.starts_with("HEAD /sources/"))
		);

		// With checksum_headers disabled, presigned PUT omits
		// x-amz-checksum-sha256 and verify uses plain HEAD + streaming
		// GET cleanly.
		let no_checksum = ObjectStore::new(
			&StorageConfig::S3(S3Config {
				endpoint: url::Url::parse(&endpoint).unwrap(),
				region: "auto".to_owned(),
				sources_bucket: "sources".to_owned(),
				logs_bucket: "logs".to_owned(),
				access_key_id: "AKIDEXAMPLE".to_owned(),
				secret_access_key: SecretValue::new("wJalrXUtnFEMI-secret"),
				path_style: true,
				checksum_headers: false,
			}),
			"https://cloud.example.com",
		)
		.unwrap();
		let presigned = no_checksum.presign_put(
			Bucket::Sources,
			&key,
			bytes.len() as u64,
			&digest,
			Duration::from_mins(5),
		);
		assert!(
			!presigned
				.unwrap()
				.headers
				.contains_key("x-amz-checksum-sha256")
		);
		assert_eq!(
			no_checksum
				.verify(Bucket::Sources, &key, bytes.len() as u64, &digest)
				.await
				.unwrap(),
			Verification::Matches
		);
	}

	/// A store that answers with an error is a retryable failure that names
	/// the method and key, never the address.
	#[tokio::test]
	async fn a_failing_store_never_shows_its_addresses() {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let endpoint = format!("http://{}", listener.local_addr().unwrap());
		let router = Router::new().fallback(|| async { StatusCode::INTERNAL_SERVER_ERROR });
		tokio::spawn(async move {
			leptoskit_axum::axum::serve(listener, router).await.unwrap();
		});
		let store = s3_store(&endpoint);
		let key = log_key(Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
		let error = store.put(Bucket::Logs, &key, Vec::new()).await.unwrap_err();
		let message = leptoskit_rpc::Error::from(error).to_string();
		assert!(message.contains("storage_unavailable"), "{message}");
		assert!(!message.contains("X-Amz"), "{message}");
		assert!(!message.contains("wJalrXUtnFEMI"), "{message}");
	}
}
