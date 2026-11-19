//! The build role: a `build_server` job's source, fetched and unpacked,
//! and whatever turns it into an image.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use leptoskit_cloud::archive;
use leptoskit_cloud::archive::Digest;
use leptoskit_cloud::workers::BuildOutcome;
use leptoskit_cloud::workers::BuildServerInput;
use sha2::Digest as _;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::watch;

use crate::logs::LogSink;

/// How long the archive's download may take.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(10);

/// Tells a build to stop: the deployment was cancelled, or the lease is
/// gone and nobody is waiting for the result.
#[derive(Clone, Debug)]
pub struct Stop(watch::Receiver<bool>);

impl Stop {
	pub(crate) fn new() -> (watch::Sender<bool>, Self) {
		let (sender, receiver) = watch::channel(false);
		(sender, Self(receiver))
	}

	/// A stop that never comes, for tests of a builder.
	#[must_use]
	pub fn never() -> Self {
		let (sender, stop) = Self::new();
		// The sender is kept alive for good, so `stopped` waits for ever.
		std::mem::forget(sender);
		stop
	}

	/// Whether the build was told to stop.
	#[must_use]
	pub fn is_stopped(&self) -> bool {
		*self.0.borrow()
	}

	/// Waits until the build is told to stop.
	pub async fn stopped(&mut self) {
		if self.0.wait_for(|stopped| *stopped).await.is_err() {
			// Nobody is left to say stop.
			std::future::pending::<()>().await;
		}
	}
}

/// One build, as a [`Builder`] is handed it.
#[derive(Debug)]
pub struct BuildJob<'a> {
	/// What to build. Its `archive` is already fetched: `source` holds it.
	pub input: &'a BuildServerInput,
	/// The unpacked source: a fresh directory, deleted after the build.
	pub source: &'a Path,
	/// An empty directory beside the source for the build's own files,
	/// deleted with it.
	pub scratch: &'a Path,
	/// The build's log.
	pub log: LogSink,
	/// When to give up.
	pub stop: Stop,
}

/// Turns a job's source into an image.
pub trait Builder: Send + Sync + 'static {
	/// Builds `job`. A build told to stop answers anything: its outcome is
	/// not used.
	fn build<'a>(
		&'a self,
		job: BuildJob<'a>,
	) -> Pin<Box<dyn Future<Output = BuildOutcome> + Send + 'a>>;
}

/// Why a job's source isn't in the workspace.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SourceError {
	/// The store didn't hand the archive over: worth another attempt.
	#[error("the source archive couldn't be downloaded: {0}")]
	Unavailable(String),
	/// The archive isn't the one the job names, or isn't one at all: no
	/// attempt will fare better.
	#[error("{0}")]
	Wrong(String),
	#[error("the workspace couldn't be written: {0}")]
	Workspace(#[from] std::io::Error),
}

/// A job's source archive: where to fetch it, and what it must be.
#[derive(Clone, Copy)]
pub(crate) struct Archive<'a> {
	pub(crate) download: &'a leptoskit_cloud::workers::Download,
	pub(crate) digest: &'a Digest,
	pub(crate) size: u64,
}

impl<'a> From<&'a BuildServerInput> for Archive<'a> {
	fn from(input: &'a BuildServerInput) -> Self {
		Self {
			download: &input.archive,
			digest: &input.digest,
			size: input.size,
		}
	}
}

impl<'a> From<&'a leptoskit_cloud::workers::PublishInput> for Archive<'a> {
	fn from(input: &'a leptoskit_cloud::workers::PublishInput) -> Self {
		Self {
			download: &input.archive,
			digest: &input.digest,
			size: input.size,
		}
	}
}

/// Downloads `input`'s archive to `file`, refusing a byte more than the
/// job says it has and anything whose SHA-256 isn't the job's.
async fn download(
	http: &reqwest::Client,
	input: Archive<'_>,
	file: &Path,
) -> Result<(), SourceError> {
	let unavailable = |error: reqwest::Error| {
		// The address is a credential, so the error is never printed with it.
		SourceError::Unavailable(if error.is_timeout() {
			"timed out".to_owned()
		} else {
			"the request failed".to_owned()
		})
	};
	// The source is the organisation's own: it travels encrypted, or
	// stays on this machine.
	if !url::Url::parse(&input.download.url).is_ok_and(|url| crate::client::private_transport(&url))
	{
		return Err(SourceError::Wrong(
			"the source archive's address isn't https".to_owned(),
		));
	}

	let mut response = http
		.get(&input.download.url)
		.timeout(DOWNLOAD_TIMEOUT)
		.send()
		.await
		.map_err(unavailable)?;

	if !response.status().is_success() {
		return Err(SourceError::Unavailable(format!(
			"the store answered {}",
			response.status().as_u16()
		)));
	}

	let mut out = tokio::fs::File::create(file).await?;
	let mut hasher = sha2::Sha256::new();
	let mut read = 0_u64;

	while let Some(chunk) = response.chunk().await.map_err(unavailable)? {
		read += chunk.len() as u64;

		if read > input.size {
			return Err(SourceError::Wrong(format!(
				"the source archive is larger than the {} bytes it was uploaded as",
				input.size
			)));
		}

		hasher.update(&chunk);
		out.write_all(&chunk).await?;
	}

	out.flush().await?;
	let found = format!("sha256:{}", hex::encode(hasher.finalize()));

	if read != input.size || found != input.digest.as_str() {
		return Err(SourceError::Wrong(
			"the source archive isn't the one that was uploaded: its size or SHA-256 differs"
				.to_owned(),
		));
	}

	Ok(())
}

/// Fetches `input`'s archive and unpacks it into `source`, which must not
/// exist yet, using `scratch` for the download.
pub(crate) async fn fetch(
	http: &reqwest::Client,
	input: Archive<'_>,
	source: &Path,
	scratch: &Path,
) -> Result<archive::Unpacked, SourceError> {
	// The digest is checked for its shape before anything is compared to it.
	Digest::parse(input.digest.as_str())
		.map_err(|_| SourceError::Wrong("the job's digest isn't a SHA-256".to_owned()))?;
	let file = scratch.join("source.tar.gz");
	download(http, input, &file).await?;
	let source = source.to_owned();
	let unpack = file.clone();
	let unpacked = tokio::task::spawn_blocking(move || {
		let reader = std::io::BufReader::new(std::fs::File::open(&unpack)?);
		archive::unpack(reader, &source, &archive::Limits::DEFAULT)
			.map_err(|error| SourceError::Wrong(format!("the source archive: {error}")))
	})
	.await
	.map_err(|error| SourceError::Workspace(std::io::Error::other(error)))??;
	tokio::fs::remove_file(&file).await?;
	Ok(unpacked)
}

#[cfg(test)]
mod tests {
	use leptoskit_cloud::deployments::AppFacts;
	use leptoskit_cloud::workers::DeploymentRef;
	use leptoskit_cloud::workers::Download;
	use tokio::io::AsyncReadExt as _;

	use super::*;

	/// Serves `body` with `status` to every request, and answers the
	/// address to ask.
	async fn store(status: u16, body: Vec<u8>) -> String {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
			.await
			.expect("a port");
		let address = listener.local_addr().expect("an address");
		tokio::spawn(async move {
			while let Ok((mut stream, _)) = listener.accept().await {
				let body = body.clone();
				tokio::spawn(async move {
					let mut request = [0_u8; 4096];
					let _ = stream.read(&mut request).await;
					let head = format!(
						"HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
						body.len()
					);
					let _ = stream.write_all(head.as_bytes()).await;
					let _ = stream.write_all(&body).await;
					let _ = stream.shutdown().await;
				});
			}
		});
		format!("http://{address}/sources/presigned")
	}

	fn input(url: String, bytes: &[u8]) -> BuildServerInput {
		BuildServerInput {
			deployment: DeploymentRef {
				organization: "ana".to_owned(),
				project: "notes".to_owned(),
				environment: "production".to_owned(),
				number: 1,
			},
			archive: Download {
				url,
				expires_at: "2026-11-19T00:00:00Z".parse().expect("a time"),
			},
			digest: Digest::of(bytes),
			size: bytes.len() as u64,
			toolchain: "stable".to_owned(),
			targets: Vec::new(),
			image_repository: "registry.example.test/ana/notes".to_owned(),
			app: AppFacts::default(),
		}
	}

	fn packed() -> Vec<u8> {
		let directory = tempfile::tempdir().expect("a directory");
		std::fs::write(directory.path().join("Cargo.toml"), "[package]\n").expect("a file");
		archive::pack(
			directory.path(),
			&archive::PackOptions {
				use_git: false,
				..archive::PackOptions::default()
			},
		)
		.expect("packs")
		.bytes
	}

	/// Fetches what `served` serves as the archive `promised` was uploaded
	/// as.
	async fn fetched(
		status: u16,
		served: Vec<u8>,
		promised: &[u8],
	) -> (Result<archive::Unpacked, SourceError>, tempfile::TempDir) {
		let directory = tempfile::tempdir().expect("a directory");
		let scratch = directory.path().join("scratch");
		std::fs::create_dir(&scratch).expect("a scratch");
		let input = input(store(status, served).await, promised);
		let result = fetch(
			&reqwest::Client::new(),
			(&input).into(),
			&directory.path().join("source"),
			&scratch,
		)
		.await;
		(result, directory)
	}

	#[tokio::test]
	async fn the_source_is_what_was_uploaded_or_nothing() {
		let bytes = packed();
		let (unpacked, directory) = fetched(200, bytes.clone(), &bytes).await;
		assert_eq!(unpacked.expect("unpacks").files, ["Cargo.toml"]);
		assert!(directory.path().join("source/Cargo.toml").is_file());
		// The download itself isn't left beside the source.
		assert!(!directory.path().join("scratch/source.tar.gz").exists());

		// One byte different, a byte more, and a byte less are all refused,
		// and nothing is unpacked.
		let mut flipped = bytes.clone();
		flipped[10] ^= 1;
		let mut longer = bytes.clone();
		longer.push(0);
		let shorter = bytes[..bytes.len() - 1].to_vec();

		for served in [flipped, longer, shorter] {
			let (refused, directory) = fetched(200, served, &bytes).await;
			assert!(matches!(refused, Err(SourceError::Wrong(_))), "{refused:?}");
			assert!(!directory.path().join("source").exists());
		}

		// Bytes that are what was uploaded, and aren't an archive.
		let nonsense = b"not an archive at all".to_vec();
		let (refused, _directory) = fetched(200, nonsense.clone(), &nonsense).await;
		assert!(
			matches!(&refused, Err(SourceError::Wrong(reason)) if reason.starts_with("the source archive:")),
			"{refused:?}"
		);
	}

	#[tokio::test]
	async fn a_store_that_does_not_answer_is_worth_another_attempt() {
		let bytes = packed();
		let (refused, _directory) = fetched(503, b"busy".to_vec(), &bytes).await;
		assert!(
			matches!(&refused, Err(SourceError::Unavailable(reason)) if reason.contains("503")),
			"{refused:?}"
		);
		// The address is a credential: no error repeats it.
		assert!(
			!refused
				.expect_err("refused")
				.to_string()
				.contains("presigned")
		);
	}

	#[tokio::test]
	async fn the_source_travels_encrypted_or_stays_on_this_machine() {
		let bytes = packed();
		let directory = tempfile::tempdir().expect("a directory");
		let scratch = directory.path().join("scratch");
		std::fs::create_dir(&scratch).expect("a scratch");
		let input = input("http://store.example.test/presigned".to_owned(), &bytes);
		let refused = fetch(
			&reqwest::Client::new(),
			(&input).into(),
			&directory.path().join("source"),
			&scratch,
		)
		.await;
		assert!(
			matches!(&refused, Err(SourceError::Wrong(reason)) if reason.contains("https")),
			"{refused:?}"
		);
	}
}
