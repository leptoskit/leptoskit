//! Calling the control plane: the worker protocol's operations over HTTP,
//! with the worker's key.

use std::time::Duration;

use leptoskit_cloud::workers;
use leptoskit_cloud::workers::Assignment;
use leptoskit_cloud::workers::Finished;
use leptoskit_cloud::workers::JobOutcome;
use leptoskit_cloud::workers::PairedWorker;
use leptoskit_cloud::workers::Renewed;
use leptoskit_rpc::Operation;
use uuid::Uuid;

/// The most of an answer that is read: an assignment is a few kilobytes.
const MAX_ANSWER: usize = 2 * 1024 * 1024;

/// How long a call may take, beyond any time it was asked to wait.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// A worker's key, `lkw_…`. Its `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Key(String);

impl Key {
	/// Wraps `value`.
	#[must_use]
	pub fn new(value: impl Into<String>) -> Self {
		Self(value.into())
	}

	/// The key itself, to send or to store.
	#[must_use]
	pub fn expose(&self) -> &str {
		&self.0
	}
}

impl std::fmt::Debug for Key {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str("Key([redacted])")
	}
}

/// Why an address isn't a control plane's.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
	"the control plane's address is an `https` origin, or `http` on this machine, with no path, \
	 user or password"
)]
pub struct OriginError;

/// Why a call didn't answer what was asked.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CallError {
	/// The control plane refused: its status, the error's code (such as
	/// `job_not_held`), and its message.
	#[error("{code} ({status}): {message}")]
	Rejected {
		status: u16,
		code: String,
		message: String,
	},
	/// No answer, or one that couldn't be read: worth asking again.
	#[error("the control plane didn't answer: {0}")]
	Unreachable(String),
}

impl CallError {
	/// Whether the control plane refused with `code`.
	#[must_use]
	pub fn is(&self, code: &str) -> bool {
		matches!(self, Self::Rejected { code: found, .. } if found == code)
	}

	/// Whether the key is nobody's: never paired here, or revoked.
	#[must_use]
	pub fn is_unauthorized(&self) -> bool {
		matches!(self, Self::Rejected { status: 401, .. })
	}

	/// Whether asking again later might be answered: no answer, a server's
	/// own failure, or being told to slow down.
	#[must_use]
	pub fn is_transient(&self) -> bool {
		match self {
			Self::Unreachable(_) => true,
			Self::Rejected { status, .. } => *status == 429 || *status >= 500,
		}
	}
}

/// `origin` as a control plane's address: `https`, or `http` on this
/// machine, and nothing but scheme, host and port.
///
/// # Errors
///
/// [`OriginError`] for anything else.
pub fn origin(value: &str) -> Result<String, OriginError> {
	let url = url::Url::parse(value).map_err(|_| OriginError)?;

	if !private_transport(&url)
		|| url.path() != "/"
		|| url.query().is_some()
		|| url.fragment().is_some()
		|| !url.username().is_empty()
		|| url.password().is_some()
	{
		return Err(OriginError);
	}

	Ok(url.origin().ascii_serialization())
}

/// Whether `url` is on this machine.
fn local(url: &url::Url) -> bool {
	match url.host() {
		Some(url::Host::Domain("localhost")) => true,
		Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
		Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
		_ => false,
	}
}

/// Whether what is sent to `url` stays private on the way: `https`, or
/// `http` to this machine.
pub(crate) fn private_transport(url: &url::Url) -> bool {
	url.scheme() == "https" || (url.scheme() == "http" && local(url))
}

/// `path` as the path operations are served under, such as `/rpc`.
fn rpc_path(path: &str) -> Result<String, OriginError> {
	let path = path.trim_end_matches('/');

	if !path.starts_with('/')
		|| path.starts_with("//")
		|| path.contains(['?', '#', '\\'])
		|| path.split('/').any(|part| part == "..")
	{
		return Err(OriginError);
	}

	Ok(path.to_owned())
}

/// A control plane, as a worker calls it.
#[derive(Clone, Debug)]
pub struct Control {
	http: reqwest::Client,
	/// The origin and the operations' path, with no `/` at the end.
	base: String,
	key: Option<Key>,
}

impl Control {
	/// The control plane at `origin`, whose operations are under `rpc`
	/// (`/rpc`), called with `key`. Without a key only [`pair`](Self::pair)
	/// is answered.
	///
	/// # Errors
	///
	/// [`OriginError`] when `origin` isn't `https` or this machine's, or
	/// `rpc` isn't a path.
	pub fn new(origin: &str, rpc: &str, key: Option<Key>) -> Result<Self, OriginError> {
		let origin = self::origin(origin)?;
		let rpc = rpc_path(rpc)?;
		// A redirect is never followed: the key goes only where it was told
		// to go. A proxy from the environment is used, as for any client,
		// except for a control plane on this machine: that is plain `http`,
		// and the key would cross the network to the proxy unencrypted.
		let mut http = reqwest::Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			.connect_timeout(Duration::from_secs(10));

		if origin.starts_with("http://") {
			http = http.no_proxy();
		}

		let http = http.build().map_err(|_| OriginError)?;
		Ok(Self {
			http,
			base: format!("{origin}{rpc}"),
			key,
		})
	}

	/// The same control plane, called with `key`.
	#[must_use]
	pub fn with_key(mut self, key: Key) -> Self {
		self.key = Some(key);
		self
	}

	/// Whatever a refusal's text held of the key, taken out: an error is
	/// printed, and the key never is.
	fn scrub(&self, text: &str) -> String {
		match &self.key {
			Some(key) if !key.expose().is_empty() => text.replace(key.expose(), "[redacted]"),
			_ => text.to_owned(),
		}
	}

	async fn call<O: Operation>(
		&self,
		operation: &O,
		wait: Duration,
	) -> Result<O::Output, CallError> {
		let mut request = self
			.http
			.post(format!("{}/{}", self.base, O::NAME))
			.timeout(CALL_TIMEOUT + wait)
			.json(operation);

		if let Some(key) = &self.key {
			request = request.bearer_auth(key.expose());
		}

		// The error's own text can name the address and nothing else; the
		// key is a header, which it never prints.
		let unreachable = |error: reqwest::Error| {
			CallError::Unreachable(if error.is_timeout() {
				"timed out".to_owned()
			} else if error.is_connect() {
				"couldn't connect".to_owned()
			} else {
				"the request failed".to_owned()
			})
		};
		let mut response = request.send().await.map_err(unreachable)?;
		let status = response.status().as_u16();
		let mut body = Vec::new();

		while let Some(chunk) = response.chunk().await.map_err(unreachable)? {
			if body.len() + chunk.len() > MAX_ANSWER {
				return Err(CallError::Unreachable("the answer was too long".to_owned()));
			}

			body.extend_from_slice(&chunk);
		}

		if (200..300).contains(&status) {
			return serde_json::from_slice(&body)
				.map_err(|_| CallError::Unreachable("the answer couldn't be read".to_owned()));
		}

		let text = self.scrub(&String::from_utf8_lossy(&body));
		// `Kind|code: message`, as `leptoskit_rpc` writes an error.
		let wire = text.split_once('|').map_or(text.as_str(), |(_, rest)| rest);
		let (code, message) = match wire.split_once(':') {
			Some((code, message))
				if !code.is_empty()
					&& code
						.bytes()
						.all(|byte| byte.is_ascii_lowercase() || byte == b'_') =>
			{
				(code, message.trim())
			}
			_ => ("request_rejected", wire.trim()),
		};
		let mut message = message.to_owned();

		if message.len() > 500 {
			let mut end = 500;

			while !message.is_char_boundary(end) {
				end -= 1;
			}

			message.truncate(end);
		}

		Err(CallError::Rejected {
			status,
			code: code.to_owned(),
			message,
		})
	}

	/// Trades a pairing code for a worker's key.
	///
	/// # Errors
	///
	/// [`CallError`]; `pairing_invalid` for a code that is unknown, used or
	/// expired.
	pub async fn pair(
		&self,
		code: &str,
		name: &str,
		capabilities: Vec<String>,
	) -> Result<PairedWorker, CallError> {
		self.call(
			&workers::Pair {
				code: code.to_owned(),
				name: name.to_owned(),
				capabilities,
			},
			Duration::ZERO,
		)
		.await
	}

	/// Asks for a job, waiting up to `wait_seconds` for one.
	///
	/// # Errors
	///
	/// [`CallError`]; `job_held` while the control plane still counts a job
	/// as this worker's.
	pub async fn claim(&self, wait_seconds: u32) -> Result<Option<Assignment>, CallError> {
		self.call(
			&workers::Claim {
				wait_seconds: Some(wait_seconds),
			},
			Duration::from_secs(u64::from(wait_seconds)),
		)
		.await
	}

	/// Extends the lease on `job`.
	///
	/// # Errors
	///
	/// [`CallError`]; `job_not_held` once the lease has run out.
	pub async fn renew(&self, job: Uuid) -> Result<Renewed, CallError> {
		self.call(&workers::Renew { job }, Duration::ZERO).await
	}

	/// Sends chunk `seq` of `job`'s log.
	///
	/// # Errors
	///
	/// [`CallError`].
	pub async fn append_log(
		&self,
		job: Uuid,
		attempt: u32,
		seq: u32,
		lines: Vec<String>,
	) -> Result<(), CallError> {
		self.call(
			&workers::AppendLog {
				job,
				attempt,
				seq,
				lines,
			},
			Duration::ZERO,
		)
		.await
	}

	/// Reports how `job` ended.
	///
	/// # Errors
	///
	/// [`CallError`]; `job_not_held` for an attempt whose lease ran out,
	/// `outcome_conflict` for an outcome other than the one recorded.
	pub async fn finish(
		&self,
		job: Uuid,
		attempt: u32,
		outcome: JobOutcome,
	) -> Result<Finished, CallError> {
		self.call(
			&workers::Finish {
				job,
				attempt,
				outcome,
			},
			Duration::ZERO,
		)
		.await
	}

	/// Reports what the cluster currently runs for `environment`, along with
	/// recent runtime log lines. Feeds `environment_status`, `runtime_logs`,
	/// and compute metering on the control plane.
	///
	/// # Errors
	///
	/// [`CallError`]; `environment_not_found` if the environment is no longer
	/// placed on this cluster.
	pub async fn report_status(
		&self,
		environment: Uuid,
		observed: leptoskit_cloud_runtime::Observed,
		logs: Vec<leptoskit_cloud_runtime::LogLine>,
	) -> Result<workers::StatusReceived, CallError> {
		self.call(
			&workers::ReportStatus {
				environment,
				observed: workers::RuntimeValue(observed),
				logs: logs.into_iter().map(workers::RuntimeValue).collect(),
			},
			Duration::ZERO,
		)
		.await
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn an_origin_is_https_or_this_machine() {
		assert_eq!(
			origin("https://cloud.example.com/").as_deref(),
			Ok("https://cloud.example.com")
		);
		assert_eq!(
			origin("http://127.0.0.1:4000").as_deref(),
			Ok("http://127.0.0.1:4000")
		);
		assert_eq!(
			origin("http://localhost:4000").as_deref(),
			Ok("http://localhost:4000")
		);

		for wrong in [
			"http://cloud.example.com",
			"https://cloud.example.com/rpc",
			"https://user:pass@cloud.example.com",
			"https://cloud.example.com/?a=b",
			"ftp://cloud.example.com",
			"cloud.example.com",
			"",
		] {
			assert_eq!(origin(wrong), Err(OriginError), "{wrong}");
		}

		assert_eq!(rpc_path("/rpc/").as_deref(), Ok("/rpc"));

		for wrong in ["rpc", "//other.example.com", "/rpc?x", "/a/../b"] {
			assert_eq!(rpc_path(wrong), Err(OriginError), "{wrong}");
		}
	}

	#[test]
	fn a_key_is_never_printed() {
		let key = Key::new("lkw_example");
		assert!(!format!("{key:?}").contains("secret"));
		let control = Control::new("http://127.0.0.1:1", "/rpc", Some(key)).expect("a client");
		assert!(!format!("{control:?}").contains("secret"));
		assert_eq!(
			control.scrub("no such key lkw_example here"),
			"no such key [redacted] here"
		);
	}
}
