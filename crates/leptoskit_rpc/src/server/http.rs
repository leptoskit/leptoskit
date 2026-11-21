//! The one-shot HTTP transport: `POST {path}/<name>` for every query and
//! mutation — `/rpc` unless `[rpc].path` says otherwise — and the same
//! calls in process while the server renders.

use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;

use axum::Router;
use axum::extract::Request;
use axum::http::HeaderValue;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::post;
use leptoskit_db::Db;

use super::Anonymous;
use super::Identify;
use super::Invocation;
use super::Registration;
pub(crate) use super::respond::failure;
use crate::Error;
use crate::Kind;

/// The header a mutation's request ID travels in.
pub const IDEMPOTENCY_KEY: &str = "idempotency-key";

/// The largest request body a call may send: 2 MiB.
const BODY_LIMIT: usize = 2 * 1024 * 1024;

/// JSON operation routes kept on `/api` while `json` is in `[rpc].transports`.
const LEGACY_ROUTE_PREFIX: &str = "/api";

/// The app's operations, served: the database they run on, and how to tell
/// who is calling.
///
/// ```ignore
/// let rpc = rpc::server::Server::new(db.clone()).identify(auth.clone());
/// let app = Router::new().merge(rpc.router()).layer(auth.layer());
/// ```
#[derive(Clone)]
pub struct Server {
	inner: Arc<Inner>,
}

#[derive(Clone)]
struct Inner {
	db: Db,
	identify: Arc<dyn Identify>,
	/// The service these operations belong to; `None` for the app's own.
	service: Option<&'static str>,
	transports: crate::wire::TransportSet,
	route_prefix: String,
	/// Where [`live`](Self::live) serves its socket: the `[rpc]` config's
	/// resolved live path, `/live` by default.
	live_path: String,
	/// What runs before every call a client sends.
	guards: super::Guards,
	/// The largest arguments a call may send, unless its operation says.
	max_body: usize,
}

impl Server {
	/// Operations on `db`, where every caller is anonymous until
	/// [`identify`](Self::identify) says otherwise.
	pub fn new(db: Db) -> Self {
		Self {
			inner: Arc::new(Inner {
				db,
				identify: Arc::new(Anonymous),
				service: None,
				transports: crate::wire::TransportSet::default(),
				route_prefix: crate::RPC_ROUTE_PREFIX.to_owned(),
				live_path: crate::live::LIVE_PATH.to_owned(),
				guards: super::Guards::default(),
				max_body: BODY_LIMIT,
			}),
		}
	}

	/// A copy with `change` made to its settings.
	fn with(self, change: impl FnOnce(&mut Inner)) -> Self {
		let mut inner = Arc::unwrap_or_clone(self.inner);
		change(&mut inner);
		Self {
			inner: Arc::new(inner),
		}
	}

	/// Applies `[rpc].path`, `[rpc].live_path`, and `[rpc].transports` from
	/// configuration: the operation routes mount at `path` (the JSON ones
	/// keep the legacy `/api` prefix while JSON is a transport), and
	/// [`live`](Self::live) serves at `live_path`, or `{path}/live`.
	#[must_use]
	pub fn rpc_config(self, config: &leptoskit_config::RpcConfig) -> Self {
		let route_prefix = config.path.trim_end_matches('/').to_owned();

		self.with(|inner| {
			inner.transports = crate::wire::TransportSet::from_names(&config.transports);
			inner.route_prefix = route_prefix;
			inner.live_path = super::mount_path(&config.resolved_live_path());
		})
	}

	/// Tells callers apart with `identify`, such as `leptoskit_auth`'s
	/// `Auth`.
	#[must_use]
	pub fn identify(self, identify: impl Identify) -> Self {
		self.with(|inner| inner.identify = Arc::new(identify))
	}

	/// Runs `guard` before every call a client sends, over HTTP and over
	/// the [live](Self::live) connections, after the caller is identified
	/// and before the operation's body runs. Guards run in the order they
	/// are added, and the first to refuse answers the call.
	///
	/// ```ignore
	/// let rpc = Server::new(db).identify(auth).guard(SignedInOnly);
	/// ```
	#[must_use]
	pub fn guard(self, guard: impl super::Guard) -> Self {
		self.with(|inner| inner.guards.push(Arc::new(guard)))
	}

	/// Limits how often each caller may call, counted before any
	/// [`guard`](Self::guard) runs: see [`RateLimit`](super::RateLimit). An
	/// operation's own `rate_limit` is counted on top, with this limit's
	/// keys and store. Calling it again replaces the limit.
	///
	/// ```ignore
	/// let rpc = Server::new(db).identify(auth).rate_limit(RateLimit::new());
	/// ```
	#[must_use]
	pub fn rate_limit(self, limit: super::RateLimit) -> Self {
		self.with(|inner| inner.guards.set_rate_limit(limit))
	}

	/// Reads where a call came from from `header`, which the proxy in
	/// front of the server sets, such as `fly-client-ip`, `x-real-ip`, or
	/// `cf-connecting-ip`, instead of the connection's own address. The
	/// last value of the header is read, since a proxy adds its own after
	/// any the client sent, and it must be one address. A request without
	/// it is counted by its connection.
	///
	/// Only set it when a proxy always sets the header: otherwise a client
	/// can write any address it likes there.
	///
	/// # Panics
	///
	/// When `header` isn't a header name.
	#[must_use]
	pub fn client_ip_header(self, header: &str) -> Self {
		let name = http::HeaderName::try_from(header)
			.unwrap_or_else(|_| panic!("`{header}` isn't a header name"));
		self.with(|inner| {
			inner
				.guards
				.set_client_ip(super::guard::ClientIp::Header(name));
		})
	}

	/// How long a call may run before it is answered `504` (`timed_out`),
	/// unless its operation says otherwise with `timeout`. No limit by
	/// default.
	#[must_use]
	pub fn timeout(self, timeout: std::time::Duration) -> Self {
		self.with(|inner| inner.guards.set_timeout(Some(timeout)))
	}

	/// The largest arguments, in bytes, a call over HTTP may send, unless
	/// its operation says otherwise with `max_body`: 2 MiB by default. A
	/// call over it is answered `413` (`too_large`).
	#[must_use]
	pub fn max_body(self, bytes: usize) -> Self {
		self.with(|inner| inner.max_body = bytes)
	}

	/// Serves the operations of the service `name`, declared with
	/// `#[rpc::query(service = "name")]` and the like, instead of the app's
	/// own: what a service's binary builds. Every server also serves the
	/// built-in `rpc.*` operations for its own jobs and inbox.
	///
	/// Nest its [`router`](Self::router) (and its live server's) under
	/// [`service_prefix(name)`](crate::service_prefix), where the app's
	/// clients send the service's operations:
	///
	/// ```ignore
	/// let rpc = Server::new(db).service("analytics").identify(ServiceCallers::for_service(Anonymous));
	/// let live = rpc.live().origin(public_origin);
	/// let app = Router::new().nest(&rpc::service_prefix("analytics"), rpc.router().merge(live.router()));
	/// ```
	///
	/// In this process, a call to one of the service's own operations runs
	/// in process, as the app's do in the app's server.
	///
	/// # Panics
	///
	/// When `name` isn't a service name: see
	/// [`is_service_name`](crate::is_service_name).
	#[must_use]
	pub fn service(self, name: &'static str) -> Self {
		assert!(
			crate::is_service_name(name),
			"`{name}` isn't a service name: a lowercase letter, then lowercase letters, digits, \
			 and `-`"
		);
		self.with(|inner| inner.service = Some(name))
	}

	/// The service these operations belong to, or `None` for the app's.
	#[must_use]
	pub fn service_name(&self) -> Option<&'static str> {
		self.inner.service
	}

	/// The database operations run on, before it acts for anyone.
	pub fn db(&self) -> &Db {
		&self.inner.db
	}

	/// Who `parts` is from, as this server's routes would identify them:
	/// how a page rendered for someone signs that person's calls to a
	/// service.
	pub(crate) fn identify_request<'a>(
		&'a self,
		parts: &'a http::request::Parts,
	) -> leptoskit_query::BoxFuture<'a, crate::Result<super::Identity>> {
		self.inner.identify.identify(parts)
	}

	/// The live server for these operations: the same database, the same way
	/// of telling callers apart, and the same `[rpc].path`/`live_path`
	/// mount. Configure it, then merge its
	/// [`router`](super::Live::router).
	#[cfg(feature = "live")]
	pub fn live(&self) -> super::Live {
		super::Live::new(self.inner.db.clone())
			.identify_arc(Arc::clone(&self.inner.identify))
			.for_service(self.inner.service)
			.with_live_path(self.inner.live_path.clone())
			.with_guards(self.inner.guards.clone())
	}

	/// Makes this the server that in-process calls use, such as
	/// `client::call` while a page renders. [`router`](Self::router) does it
	/// too.
	pub fn install(&self) {
		*INSTALLED.write().unwrap_or_else(PoisonError::into_inner) = Some(self.clone());
	}

	/// `POST {path}/<name>` for every registered query and mutation —
	/// `[rpc].path`, `/rpc` by default, with the legacy JSON routes kept on
	/// `/api` while JSON is a transport — relative to wherever the router is
	/// nested, and [installs](Self::install) this server for in-process
	/// calls.
	///
	/// A call sends its arguments as one JSON object, and gets back its
	/// result as JSON with status `200`, or a `Kind|message` error as
	/// `text/plain` with the status [`Error::status`] gives: the same as a
	/// `#[server]` function with the `Json` encoding, so every generated
	/// client speaks it. A mutation's `Idempotency-Key` header becomes its
	/// request ID.
	///
	/// # Panics
	///
	/// When two operations share a name; see
	/// [`registrations`](super::registrations).
	///
	/// Only this server's own operations get routes: the app's for a
	/// default server, or one service's after [`service`](Self::service).
	/// A service's operation linked into the app's build, as in a
	/// workspace-wide build, is never served by the app.
	pub fn router<S>(&self) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
	{
		let registrations =
			super::registrations_in(self.inner.service).unwrap_or_else(|error| panic!("{error}"));
		self.install();
		let mut router = Router::new();

		for registration in registrations {
			// Jobs run in the background; a worker claims them, not a route.
			if registration.kind == Kind::Job {
				continue;
			}

			let mut prefixes = vec![self.inner.route_prefix.clone()];
			if self.inner.transports.json && self.inner.route_prefix != LEGACY_ROUTE_PREFIX {
				prefixes.push(LEGACY_ROUTE_PREFIX.to_owned());
			}

			for prefix in prefixes {
				let server = self.clone();
				let path = format!("{prefix}/{}", registration.name);
				router = router.route(
					&path,
					post(move |request: Request| {
						let server = server.clone();
						async move { server.serve(registration, request).await }
					}),
				);
			}
		}

		router
	}

	async fn serve(&self, registration: &'static Registration, request: Request) -> Response {
		let (parts, body) = request.into_parts();

		// Who is calling, and whether they may, before the body is read: a
		// caller who is refused costs no more than their headers.
		let identity = match self.inner.identify.identify(&parts).await {
			Ok(identity) => identity,
			Err(error) => return failure(&error),
		};
		let guards = &self.inner.guards;
		let checked = guards
			.check(&super::Call {
				name: registration.name,
				kind: registration.kind,
				identity: &identity,
				address: guards.address(&parts),
				via: super::Via::Http,
				limits: &registration.limits,
			})
			.await;

		if let Err(error) = checked {
			return failure(&error);
		}

		let limit = registration.limits.max_body.unwrap_or(self.inner.max_body);
		let Ok(bytes) = axum::body::to_bytes(body, limit).await else {
			return failure(&super::guard::too_large(limit));
		};

		let content_type = parts
			.headers
			.get(header::CONTENT_TYPE)
			.and_then(|value| value.to_str().ok());
		let request_transport =
			match crate::wire::request_transport(content_type, self.inner.transports) {
				Ok(transport) => transport,
				Err(message) => return failure(&Error::invalid(message)),
			};
		let response_transport = crate::wire::response_transport(
			parts
				.headers
				.get(header::ACCEPT)
				.and_then(|value| value.to_str().ok()),
			self.inner.transports,
			request_transport,
		);

		let arguments = if bytes.is_empty() || bytes.iter().all(u8::is_ascii_whitespace) {
			crate::wire::WireArgs::empty(request_transport)
		} else {
			let crate::wire::Transport::Json = request_transport;
			match serde_json::from_slice(&bytes) {
				Ok(value @ serde_json::Value::Object(_)) => crate::wire::WireArgs::Json(value),
				Ok(_) | Err(_) => {
					return failure(&Error::invalid(
						"send the arguments as one JSON object, such as `{\"limit\": 10}`",
					));
				}
			}
		};

		let request_id = (registration.kind == Kind::Mutation)
			.then(|| {
				parts
					.headers
					.get(IDEMPOTENCY_KEY)
					.map(|value| value.to_str().map(str::to_owned))
			})
			.flatten()
			.transpose();

		let Ok(request_id) = request_id else {
			return failure(&Error::invalid(
				"the idempotency key must be printable ASCII",
			));
		};

		let run = Self::invoke(
			&self.inner.db,
			registration,
			identity,
			arguments,
			response_transport,
			request_id,
		);

		match guards.within(&registration.limits, run).await {
			Ok(body) => body.into_response(),
			Err(error) => failure(&error),
		}
	}

	/// Runs `registration` for the request `parts` came with, in process:
	/// a call the server makes while it renders that request, which was
	/// checked when it arrived, so no guard runs again.
	pub(crate) async fn call(
		&self,
		parts: &http::request::Parts,
		registration: &'static Registration,
		arguments: crate::wire::WireArgs,
		response: crate::wire::Transport,
		request_id: Option<String>,
	) -> crate::Result<crate::wire::WireBody> {
		let identity = self.inner.identify.identify(parts).await?;
		Self::invoke(
			&self.inner.db,
			registration,
			identity,
			arguments,
			response,
			request_id,
		)
		.await
	}

	async fn invoke(
		db: &Db,
		registration: &'static Registration,
		identity: super::Identity,
		arguments: crate::wire::WireArgs,
		response: crate::wire::Transport,
		request_id: Option<String>,
	) -> crate::Result<crate::wire::WireBody> {
		let db = db.with_context(identity.context.clone());
		(registration.handler)(Invocation {
			db,
			identity,
			request_id,
			arguments,
			response,
			reads: None,
		})
		.await
	}
}

impl crate::wire::WireBody {
	pub(crate) fn into_response(self) -> Response {
		let Self::Json(value) = self;
		(
			[(
				header::CONTENT_TYPE,
				HeaderValue::from_static(crate::JSON_CONTENT_TYPE),
			)],
			value.to_string(),
		)
			.into_response()
	}
}

impl std::fmt::Debug for Server {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Server").finish_non_exhaustive()
	}
}

static INSTALLED: RwLock<Option<Server>> = RwLock::new(None);

/// The server in-process calls use, once one is installed.
pub(crate) fn installed() -> Option<Server> {
	INSTALLED
		.read()
		.unwrap_or_else(PoisonError::into_inner)
		.clone()
}
