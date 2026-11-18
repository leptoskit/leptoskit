//! The transports push delivery speaks: one per provider, behind one
//! process-wide runtime, over one injected HTTP boundary.
//!
//! Every transport builds its real provider request — the APNs URL, the
//! FCM v1 path, the Web Push endpoint — and hands it to a
//! [`Boundary`], the only thing that touches a socket. The service
//! installs the real boundary, reqwest with redirects refused, proxies
//! ignored, TLS verified, timeouts set, and response reads bounded; the
//! tests install a mock that receives the same requests. No
//! configuration — operator-provided or caller-provided — reaches the
//! boundary choice: production code calls the same constructors the
//! tests do, pointed at the same provider URLs, and only the boundary
//! differs.

pub(crate) mod apns;
pub(crate) mod fcm;
pub(crate) mod host;
pub(crate) mod webpush;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use leptoskit_db::Timestamp;
use uuid::Uuid;

use crate::push::DeliveryReason;
use crate::push::PushPlatform;
use crate::service::config;

/// One outbound request, exactly as the transport means to send it: real
/// provider URL, headers with their signed authorization material, and
/// the encoded body. Secret material rides in all three — bearer tokens
/// in the headers, device tokens in the URL, signed assertions and
/// registration tokens in the body — so the `Debug` print shows the
/// destination's host and the sizes, and none of the values.
#[derive(Clone)]
pub struct WireRequest {
	/// The URL, with no credentials in it.
	pub url: String,
	/// Header names, lower case, and their values.
	pub headers: Vec<(String, String)>,
	/// The request body, encoded as the provider asks.
	pub body: Vec<u8>,
	/// The host a DNS-validated request is pinned to, with the addresses
	/// the resolver answered and a validator cleared. `None` for a
	/// request whose host needs no pinning — a shipped provider host, or
	/// a boundary that resolves its own way.
	pub pinned: Option<Pinned>,
}

impl std::fmt::Debug for WireRequest {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let host = self
			.url
			.split_once("://")
			.map_or("", |(_, rest)| rest)
			.split(['/', '?', '#'])
			.next()
			.unwrap_or_default();

		f.debug_struct("WireRequest")
			.field("url", &format!("…://{host}/<redacted>"))
			.field(
				"headers",
				&self
					.headers
					.iter()
					.map(|(name, _)| (name.clone(), "<redacted>"))
					.collect::<Vec<_>>(),
			)
			.field("body", &format!("<{} bytes redacted>", self.body.len()))
			.field("pinned", &self.pinned)
			.finish()
	}
}

/// A host and the validated addresses a request to it may connect to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pinned {
	/// The host, as it names itself to the resolver and in TLS.
	pub host: String,
	/// The port to connect to.
	pub port: u16,
	/// Every address the connection may use, each cleared by the
	/// validator.
	pub addrs: Vec<std::net::IpAddr>,
}

/// One provider answer, already bounded: the status, the headers the
/// classification reads (`retry-after` among them), and a body read no
/// further than [`crate::push`]'s response bound. A body can carry
/// access tokens, so the `Debug` print shows the sizes, not the bytes.
#[derive(Clone)]
pub struct WireResponse {
	/// The HTTP status.
	pub status: u16,
	/// Header names, lower case, and their values.
	pub headers: Vec<(String, String)>,
	/// The body, as bytes, truncated at the read bound.
	pub body: Vec<u8>,
}

impl std::fmt::Debug for WireResponse {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("WireResponse")
			.field("status", &self.status)
			.field(
				"headers",
				&self
					.headers
					.iter()
					.map(|(name, _)| (name.clone(), "<redacted>"))
					.collect::<Vec<_>>(),
			)
			.field("body", &format!("<{} bytes redacted>", self.body.len()))
			.finish()
	}
}

/// Why a request never got an answer to classify: the socket said no.
/// Every one of these is transient by nature — the provider was
/// unreachable, slow, or answered more than this service will read — so
/// the delivery retries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
	/// The connection or the answer didn't finish inside the timeouts.
	TimedOut,
	/// The connection failed: DNS, refused, reset, TLS.
	Unreachable,
	/// The answer's body ran past the read bound and was refused.
	TooLarge,
}

/// The only place a push request touches a socket. The real
/// implementation is [`HttpBoundary`]; tests install their own.
pub trait Boundary: Send + Sync {
	/// Posts `request` and answers with a bounded response, or a
	/// [`WireError`] when no answer came.
	fn post(
		&self,
		request: WireRequest,
	) -> Pin<Box<dyn Future<Output = Result<WireResponse, WireError>> + Send + '_>>;
}

/// The real boundary: reqwest, with redirects refused so a provider can't
/// walk a request to another host, proxies ignored so the environment's
/// `HTTP_PROXY` never carries a push, TLS verified against public roots,
/// timeouts on the connection and the whole request, and every response
/// read only up to the bound.
pub struct HttpBoundary {
	client: reqwest::Client,
	/// One client per pinned host **and address set**, least recently
	/// used first: a re-resolution with new addresses builds a fresh
	/// client rather than connecting to stale ones, and a host whose
	/// address set keeps changing can't grow the cache without bound.
	pinned: std::sync::Mutex<PinnedClients>,
}

/// How many pinned clients are kept before the oldest is dropped. Each
/// holds a connection pool; a few hosts with rotating addresses are
/// normal, an unbounded set is not.
const PINNED_CLIENT_LIMIT: usize = 32;

/// A bounded insertion-ordered map of pinned clients.
struct PinnedClients {
	entries: Vec<(String, reqwest::Client)>,
}

impl PinnedClients {
	fn get(&mut self, key: &str) -> Option<reqwest::Client> {
		let at = self.entries.iter().position(|(held, _)| held == key)?;
		let (held, client) = self.entries.remove(at);

		// The hit goes to the back: the front is the least recently used.
		self.entries.push((held, client.clone()));

		Some(client)
	}

	fn insert(&mut self, key: String, client: reqwest::Client) {
		self.entries.retain(|(held, _)| *held != key);

		if self.entries.len() >= PINNED_CLIENT_LIMIT {
			self.entries
				.drain(..self.entries.len() + 1 - PINNED_CLIENT_LIMIT);
		}

		self.entries.push((key, client));
	}
}

impl HttpBoundary {
	/// Builds the boundary. Fails only if reqwest can't initialize, which
	/// is a broken build rather than a configuration an operator can fix.
	///
	/// # Errors
	///
	/// A message when the client can't be built.
	pub fn new() -> Result<Self, String> {
		Ok(Self::over_client(boundary_client(
			reqwest::Client::builder(),
		)?))
	}

	/// The same boundary over an already-built client: the crate's own
	/// integration tests use it to prove production HTTP behavior against
	/// local servers. Nothing in the service calls it.
	#[doc(hidden)]
	#[must_use]
	pub fn over_client(client: reqwest::Client) -> Self {
		Self {
			client,
			pinned: std::sync::Mutex::new(PinnedClients {
				entries: Vec::new(),
			}),
		}
	}

	/// The client for a pinned host and address set, built with exactly
	/// those addresses. The addresses came from the caller's validator,
	/// so a connection here can't be talked into a private address after
	/// the fact, and a fresh resolution that answers new addresses gets a
	/// fresh client rather than the stale one.
	fn pinned_client(&self, pinned: &Pinned) -> Result<reqwest::Client, WireError> {
		let mut key = format!("{}:{}", pinned.host, pinned.port);

		for addr in &pinned.addrs {
			key.push(' ');
			key.push_str(&addr.to_string());
		}

		let mut cached = self
			.pinned
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);

		if let Some(client) = cached.get(&key) {
			return Ok(client);
		}

		let addrs: Vec<std::net::SocketAddr> = pinned
			.addrs
			.iter()
			.map(|addr| std::net::SocketAddr::new(*addr, pinned.port))
			.collect();
		let client =
			boundary_client(reqwest::Client::builder().resolve_to_addrs(&pinned.host, &addrs))
				.map_err(|_| WireError::Unreachable)?;
		cached.insert(key, client.clone());

		Ok(client)
	}
}

/// Finishes a boundary client: redirects refused, proxies ignored, TLS
/// verified, and the timeouts set.
///
/// # Errors
///
/// A message when the client can't be built.
fn boundary_client(builder: reqwest::ClientBuilder) -> Result<reqwest::Client, String> {
	builder
		.redirect(reqwest::redirect::Policy::none())
		.no_proxy()
		.connect_timeout(Duration::from_secs(5))
		.timeout(Duration::from_secs(10))
		.build()
		.map_err(|error| format!("the push HTTP client won't build: {error}"))
}

impl Boundary for HttpBoundary {
	fn post(
		&self,
		request: WireRequest,
	) -> Pin<Box<dyn Future<Output = Result<WireResponse, WireError>> + Send + '_>> {
		let read = async move {
			let client = match &request.pinned {
				Some(pinned) => self.pinned_client(pinned)?,
				None => self.client.clone(),
			};
			let mut built = client.post(&request.url);

			for (name, value) in &request.headers {
				built = built.header(name, value);
			}

			let response = built
				.body(request.body)
				.send()
				.await
				.map_err(|error| wire_error(&error))?;
			let status = response.status().as_u16();
			let headers = response
				.headers()
				.iter()
				.map(|(name, value)| {
					(
						name.as_str().to_owned(),
						value.to_str().unwrap_or_default().to_owned(),
					)
				})
				.collect();
			let body = read_bounded(response).await?;

			Ok(WireResponse {
				status,
				headers,
				body,
			})
		};

		Box::pin(read)
	}
}

/// Reads a response body up to the bound, refusing a longer one: the
/// providers answer with small status objects, and a large or endless
/// answer is refused rather than read.
async fn read_bounded(response: reqwest::Response) -> Result<Vec<u8>, WireError> {
	if let Some(length) = response.content_length()
		&& length > crate::push::MAX_RESPONSE_BYTES as u64
	{
		return Err(WireError::TooLarge);
	}

	let mut body = Vec::new();
	let mut reader = response;

	loop {
		let chunk = tokio::time::timeout(Duration::from_secs(10), reader.chunk())
			.await
			.map_err(|_| WireError::TimedOut)?
			.map_err(|error| wire_error(&error))?;
		let Some(chunk) = chunk else {
			return Ok(body);
		};

		if body.len() + chunk.len() > crate::push::MAX_RESPONSE_BYTES {
			return Err(WireError::TooLarge);
		}

		body.extend_from_slice(&chunk);
	}
}

/// Which socket failure an error was.
fn wire_error(error: &reqwest::Error) -> WireError {
	if error.is_timeout() {
		WireError::TimedOut
	} else if error.is_decode() {
		WireError::TooLarge
	} else {
		WireError::Unreachable
	}
}

/// Parses provider delays without shortening them. An overflowing decimal
/// becomes the largest duration, which the scheduler terminalizes safely.
fn retry_after(response: &WireResponse, now: std::time::SystemTime) -> Option<Duration> {
	let value = response
		.headers
		.iter()
		.find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))?
		.1
		.trim();

	if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
		return Some(Duration::from_secs(value.parse().unwrap_or(u64::MAX)));
	}

	httpdate::parse_http_date(value)
		.ok()
		.map(|date| date.duration_since(now).unwrap_or_default())
}

#[cfg(test)]
mod retry_tests {
	use super::*;

	#[test]
	fn provider_delays_are_never_shortened() {
		let now = std::time::UNIX_EPOCH + Duration::from_secs(784_111_777);
		for (value, expected) in [
			("42", Some(42)),
			("25200", Some(25_200)),
			("Sun, 06 Nov 1994 08:49:57 GMT", Some(20)),
			("Sun, 06 Nov 1994 08:49:17 GMT", Some(0)),
			("nonsense", None),
			("-1", None),
			("999999999999999999999999999999", Some(u64::MAX)),
		] {
			let response = WireResponse {
				status: 429,
				headers: vec![("Retry-After".into(), value.into())],
				body: vec![],
			};
			assert_eq!(
				retry_after(&response, now),
				expected.map(Duration::from_secs),
				"{value}"
			);
		}
	}
}

/// What one transport's attempt came to, with nothing provider-shaped in
/// it: the classification is the whole answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
	/// The provider took the message.
	Accepted,
	/// Transient: wait `after` — the provider's `Retry-After`, when it
	/// gave one — or the engine's backoff when `None`. A network attempt
	/// was spent.
	Transient { after: Option<Duration> },
	/// The delivery's moment isn't here yet: wait until then, with no
	/// attempt spent.
	WaitUntil(Timestamp),
	/// OAuth failed before a push request. Retry within the delivery horizon
	/// without spending a push attempt; status names operator configuration.
	Stalled { after: Option<Duration> },
	/// A push request reached the provider but its operator credentials were
	/// refused.
	AuthRejected { after: Option<Duration> },
	/// The delivery is over: the reason says why, and `invalidate` says
	/// the credential is no good at the provider, so the endpoint stops
	/// taking deliveries at all.
	Ended {
		reason: DeliveryReason,
		invalidate: bool,
	},
}

impl Outcome {
	/// Whether a provider was actually asked: only a network attempt may
	/// spend a delivery's attempt count or stamp its `attempted_at`.
	/// Local decisions — schedule, cancellation, expiry, opt-outs,
	/// liveness, size — decided before any wire moved, and an
	/// operator-configuration stall is not the delivery's attempt.
	#[must_use]
	pub(crate) fn network_attempted(self) -> bool {
		match self {
			Self::Accepted | Self::Transient { .. } | Self::AuthRejected { .. } => true,
			Self::Stalled { .. } | Self::WaitUntil(_) => false,
			// Only a provider's answer ends a delivery for these reasons;
			// every other ending is decided before a request.
			Self::Ended { reason, .. } => {
				matches!(
					reason,
					DeliveryReason::Unregistered
						| DeliveryReason::ProviderRejected
						| DeliveryReason::OperatorConfiguration
				)
			}
		}
	}
}

/// One provider's sender. `credential` is the stored, canonical
/// credential string for this transport's platform.
pub(crate) trait Transport: Send + Sync {
	/// Pushes `message` to one endpoint's credential. The boundary call
	/// is the only awaited work; no lock or database handle is held
	/// across it.
	fn attempt<'a>(
		&'a self,
		credential: &'a str,
		message: &'a Message,
	) -> Pin<Box<dyn Future<Output = Outcome> + Send + 'a>>;
}

/// What a transport pushes: the notification's stable id — the collapse
/// and topic key on every provider — its visible text, its `data` object,
/// when it stops being worth delivering, and the [options] that shape
/// the notification on each platform, with the badge already resolved to
/// the number this recipient's push carries.
///
/// [options]: crate::MessageOptions
#[derive(Clone, Debug)]
pub struct Message {
	/// The notification's id: the collapse id at APNs, the collapse key
	/// at FCM, the topic at Web Push.
	pub id: Uuid,
	/// The notification's title, as the send named it.
	pub title: String,
	/// The notification's body, as the send named it.
	pub body: String,
	/// The send's `data`, a JSON object's text.
	pub data: String,
	/// When the notification stops being visible: the providers' expiry,
	/// so a late push doesn't wake someone for something already gone.
	pub expires_at: Option<Timestamp>,
	/// How this send shapes the push: a silent data message, priority,
	/// an image, a sound, a link, and the per-platform blocks. Empty is
	/// the send exactly as it was before options existed.
	pub options: crate::MessageOptions,
	/// The badge this push sets, already resolved per recipient — a
	/// number the transports put in the payload. `None` touches no
	/// badge.
	pub badge: Option<u32>,
}

/// The process's push transports, as the operator configured them. A
/// platform without a transport is `None`, and its endpoints are refused
/// at enrollment and skipped at delivery, loudly, in both places.
pub struct PushRuntime {
	pub(crate) apns: Option<apns::Apns>,
	pub(crate) fcm: Option<fcm::Fcm>,
	pub(crate) web: Option<webpush::WebPush>,
}

impl PushRuntime {
	/// Builds the runtime from an operator's configuration, over the real
	/// boundary. Fails when a provider's configuration doesn't hold — a
	/// key that won't parse, a field missing beside its siblings — so the
	/// service refuses to start rather than discovering this at the
	/// first delivery.
	///
	/// # Errors
	///
	/// A message naming the provider and what's wrong.
	pub fn build(config: &config::PushConfig) -> Result<Self, String> {
		let boundary = Arc::new(HttpBoundary::new()?);
		Self::build_over(config, boundary)
	}

	/// Builds the runtime over an injected boundary: the tests' path, and
	/// the only way anything but the real HTTP boundary carries a push.
	///
	/// # Errors
	///
	/// As [`build`](Self::build).
	pub fn build_over(
		config: &config::PushConfig,
		boundary: Arc<dyn Boundary>,
	) -> Result<Self, String> {
		Ok(Self {
			apns: config
				.apns
				.as_ref()
				.map(|config| apns::Apns::new(config, boundary.clone()))
				.transpose()?,
			fcm: config
				.fcm
				.as_ref()
				.map(|config| fcm::Fcm::new(config, boundary.clone()))
				.transpose()?,
			web: config
				.web
				.as_ref()
				.map(|config| webpush::WebPush::new(config, boundary))
				.transpose()?,
		})
	}

	/// Whether the platform has a transport.
	#[must_use]
	pub fn configured(&self, platform: PushPlatform) -> bool {
		match platform {
			PushPlatform::Apns => self.apns.is_some(),
			PushPlatform::Fcm => self.fcm.is_some(),
			PushPlatform::WebPush => self.web.is_some(),
		}
	}

	/// The transport for `platform`, if configured.
	pub(crate) fn transport(&self, platform: PushPlatform) -> Option<&dyn Transport> {
		match platform {
			PushPlatform::Apns => self.apns.as_ref().map(|t| t as &dyn Transport),
			PushPlatform::Fcm => self.fcm.as_ref().map(|t| t as &dyn Transport),
			PushPlatform::WebPush => self.web.as_ref().map(|t| t as &dyn Transport),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::Pinned;
	use super::WireRequest;
	use super::WireResponse;

	/// The wire types' `Debug` prints show the destination's host, the
	/// header names, and sizes — never the tokens in the URL, the
	/// authorization headers, or the bodies carrying assertions and
	/// access tokens.
	#[test]
	fn wire_debug_prints_no_secrets() {
		let request = WireRequest {
			url: "https://api.sandbox.push.apple.com/3/device/a1b2c3d4e5f6".to_owned(),
			headers: vec![
				(
					"authorization".to_owned(),
					"Bearer eyJhbGc.secret.part".to_owned(),
				),
				("apns-topic".to_owned(), "dev.leptoskit.app".to_owned()),
			],
			body: b"grant_type=assertion=eyJ.very.secret".to_vec(),
			pinned: None,
		};
		let printed = format!("{request:?}");
		assert!(
			printed.contains("api.sandbox.push.apple.com"),
			"the host shows: {printed}"
		);
		assert!(!printed.contains("a1b2c3"), "no URL tokens: {printed}");
		assert!(!printed.contains("Bearer"), "no auth headers: {printed}");
		assert!(!printed.contains("secret"), "no bodies: {printed}");
		assert!(printed.contains("redacted"), "the print says what it hides");

		let response = WireResponse {
			status: 200,
			headers: vec![("retry-after".to_owned(), "1".to_owned())],
			body: br#"{"access_token":"ya29.the-token"}"#.to_vec(),
		};
		let printed = format!("{response:?}");
		assert!(!printed.contains("ya29"), "no access tokens: {printed}");
		assert!(printed.contains("200"), "the status shows: {printed}");
		assert!(
			printed.contains("retry-after"),
			"the header names show: {printed}"
		);
	}

	/// A pinned destination with new addresses is a new cache entry, so a
	/// re-resolution can't keep connecting to addresses the resolver no
	/// longer answers, and the cache stays bounded however often they
	/// change.
	#[test]
	fn a_pinned_host_with_new_addresses_is_a_new_client() {
		use super::PINNED_CLIENT_LIMIT;

		let mut clients = super::PinnedClients {
			entries: Vec::new(),
		};
		let key = |pinned: &Pinned| {
			let mut key = format!("{}:{} ", pinned.host, pinned.port);
			key.push_str(
				&pinned
					.addrs
					.iter()
					.map(std::net::IpAddr::to_string)
					.collect::<Vec<_>>()
					.join(" "),
			);
			key
		};
		let pin = |addrs: &[std::net::IpAddr]| {
			Pinned {
				host: "push.example.net".to_owned(),
				port: 443,
				addrs: addrs.to_vec(),
			}
		};

		let one = pin(&["1.1.1.1".parse().unwrap()]);
		let two = pin(&["1.1.1.1".parse().unwrap(), "8.8.8.8".parse().unwrap()]);
		clients.insert(key(&one), build());
		assert!(clients.get(&key(&one)).is_some(), "the first set is kept");
		assert!(
			clients.get(&key(&two)).is_none(),
			"a new address set is a new client, not the stale one"
		);

		// However many address sets come and go, the cache stays bounded.
		for change in 0..(PINNED_CLIENT_LIMIT * 2) {
			clients.insert(
				key(&pin(&[format!("9.9.9.{change}").parse().unwrap()])),
				build(),
			);
		}

		assert!(clients.entries.len() <= PINNED_CLIENT_LIMIT);

		fn build() -> reqwest::Client {
			reqwest::Client::new()
		}
	}
}
