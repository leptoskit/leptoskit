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

use std::collections::HashMap;
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
/// the encoded body.
#[derive(Clone, Debug)]
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

/// A host and the validated addresses a request to it may connect to.
#[derive(Clone, Debug)]
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
/// further than [`crate::push`]'s response bound.
#[derive(Clone, Debug)]
pub struct WireResponse {
	/// The HTTP status.
	pub status: u16,
	/// Header names, lower case, and their values.
	pub headers: Vec<(String, String)>,
	/// The body, as bytes, truncated at the read bound.
	pub body: Vec<u8>,
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
	/// One client per pinned host, holding its validated addresses.
	pinned: std::sync::Mutex<HashMap<String, reqwest::Client>>,
}

impl HttpBoundary {
	/// Builds the boundary. Fails only if reqwest can't initialize, which
	/// is a broken build rather than a configuration an operator can fix.
	///
	/// # Errors
	///
	/// A message when the client can't be built.
	pub fn new() -> Result<Self, String> {
		let client = reqwest::Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			.no_proxy()
			.connect_timeout(Duration::from_secs(5))
			.timeout(Duration::from_secs(10))
			.build()
			.map_err(|error| format!("the push HTTP client won't build: {error}"))?;

		Ok(Self {
			client,
			pinned: std::sync::Mutex::new(HashMap::new()),
		})
	}

	/// The client for a pinned host, built with its validated addresses
	/// once and reused. The addresses came from the caller's validator,
	/// so a connection here can't be talked into a private address after
	/// the fact.
	fn pinned_client(&self, pinned: &Pinned) -> Result<reqwest::Client, WireError> {
		let key = format!("{}:{}", pinned.host, pinned.port);
		let mut cached = self
			.pinned
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);

		if let Some(client) = cached.get(&key) {
			return Ok(client.clone());
		}

		let addrs: Vec<std::net::SocketAddr> = pinned
			.addrs
			.iter()
			.map(|addr| std::net::SocketAddr::new(*addr, pinned.port))
			.collect();
		let client = reqwest::Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			.no_proxy()
			.connect_timeout(Duration::from_secs(5))
			.timeout(Duration::from_secs(10))
			.resolve_to_addrs(&pinned.host, &addrs)
			.build()
			.map_err(|_| WireError::Unreachable)?;
		cached.insert(key, client.clone());

		Ok(client)
	}
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

/// What one transport's attempt came to, with nothing provider-shaped in
/// it: the classification is the whole answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
	/// The provider took the message.
	Accepted,
	/// Transient: wait `after` — the provider's `Retry-After`, when it
	/// gave one — or the engine's backoff when `None`.
	Transient { after: Option<Duration> },
	/// The delivery's moment isn't here yet: wait until then, with no
	/// attempt spent.
	WaitUntil(Timestamp),
	/// The delivery is over: the reason says why, and `invalidate` says
	/// the credential is no good at the provider, so the endpoint stops
	/// taking deliveries at all.
	Ended {
		reason: DeliveryReason,
		invalidate: bool,
	},
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
/// and when it stops being worth delivering.
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
