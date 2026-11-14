use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use anyhow::Result;
use anyhow::ensure;
use axum::Json;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::response::sse::Event as ServerEvent;
use axum::response::sse::KeepAlive;
use axum::response::sse::Sse;
use axum::routing::get;
use axum::routing::post;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
use subtle::ConstantTimeEq as _;
use tokio_stream::Stream;

use super::*;

/// Broadcast capacity per service; subscribers that fall further behind skip
/// updates and re-read the operation.
const EVENTS: usize = 256;

/// One owner, one durable store, one worker. Put TLS/Cloudflare in front when
/// serving beyond localhost. Provider credentials stay on the worker machine.
#[derive(Clone)]
pub struct Service(Arc<App>);

struct App {
	store: Store,
	owner_hash: [u8; 32],
	origin: String,
	secure: bool,
	wake: Arc<tokio::sync::Notify>,
	events: tokio::sync::broadcast::Sender<Update>,
}

impl Service {
	pub fn progress(&self, id: &str, message: &str) -> Result<()> {
		if let Some(event) = self.0.store.progress(id, message)? {
			self.publish(Update::Progress {
				operation_id: id.to_owned(),
				event,
			});
		}

		Ok(())
	}

	#[must_use]
	pub fn origin(&self) -> &str {
		&self.0.origin
	}

	/// App keys are deterministic per owner/project and remain separate from
	/// management credentials. They cannot authorize deployment operations.
	pub fn managed_app_key(&self, project: &str, ingest: bool) -> Result<String> {
		use std::fmt::Write as _;

		use hmac::Mac as _;
		ensure!(identifier(project), "invalid project ID");
		let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&self.0.owner_hash)
			.map_err(|_| anyhow::anyhow!("invalid key material"))?;
		mac.update(if ingest {
			b"leptoskit-ingest-v1:"
		} else {
			b"leptoskit-app-read-v1:"
		});

		mac.update(project.as_bytes());
		let mut key = String::new();

		for byte in mac.finalize().into_bytes() {
			write!(&mut key, "{byte:02x}")?;
		}

		if ingest {
			self.0.store.install_ingest_key(project, &key)?;
		}

		Ok(key)
	}

	pub fn new(store: Store, owner_key: &str, origin: &str) -> Result<Self> {
		ensure!(
			(32..=256).contains(&owner_key.len())
				&& owner_key.bytes().all(|b| (33..127).contains(&b)),
			"management owner key needs 32..=256 printable non-space characters"
		);
		let url = url::Url::parse(origin)?;
		let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
		ensure!(
			(url.scheme() == "https" || (local && url.scheme() == "http"))
				&& url.path() == "/"
				&& url.query().is_none()
				&& url.fragment().is_none()
				&& url.username().is_empty()
				&& url.password().is_none(),
			"management public URL must be an HTTPS origin or localhost HTTP"
		);
		Ok(Self(Arc::new(App {
			store,
			owner_hash: sha2::Sha256::digest(owner_key.as_bytes()).into(),
			origin: url.origin().ascii_serialization(),
			secure: url.scheme() == "https",
			wake: Arc::new(tokio::sync::Notify::new()),
			events: tokio::sync::broadcast::channel(EVENTS).0,
		})))
	}

	/// The API routes only, for embedding under another axum app's path
	/// prefix with its own authentication. The dashboard page and script stay
	/// on [`Service::router`].
	pub fn api_router(&self) -> Router {
		self.routes(false)
	}

	pub fn router(&self) -> Router {
		self.routes(true)
	}

	fn routes(&self, dashboard: bool) -> Router {
		let router = Router::new()
			.route("/healthz", get(|| async { StatusCode::OK }))
			.route("/api/v1/login", post(login))
			.route("/api/v1/logout", post(logout))
			.route("/api/v1/overview", get(overview))
			.route("/api/v1/tokens", post(issue))
			.route("/api/v1/tokens/revoke", post(revoke))
			.route("/api/v1/projects/{project}/operations", post(enqueue))
			.route("/api/v1/operations/{id}", get(operation))
			.route("/api/v1/operations/{id}/events", get(operation_events))
			.route("/api/v1/projects/{project}/analytics", get(fleet))
			.route("/api/v1/projects/{project}/analytics/ingest", post(ingest));
		let router = if dashboard {
			router
				.route("/", get(page))
				.route("/console.mjs", get(script))
		} else {
			router
		};

		router
			.layer(DefaultBodyLimit::max(8 * 1024 * 1024))
			.layer(axum::middleware::from_fn(headers))
			.with_state(self.clone())
	}

	/// Start only after taking the service directory's exclusive lock. Running
	/// jobs left by a previous process become interrupted, never auto-replayed.
	pub fn worker(&self, executor: Arc<dyn Executor>) -> Result<tokio::task::JoinHandle<()>> {
		self.0.store.interrupt_running()?;
		Ok(tokio::spawn(worker::work(
			self.0.store.clone(),
			executor,
			self.0.wake.clone(),
			self.0.events.clone(),
		)))
	}

	// The typed seam embedders call in-process; the HTTP handlers below call
	// exactly these methods, so scope, revision and idempotency rules cannot
	// drift between the two surfaces.

	/// The projects and recent operations this principal may read.
	pub fn overview(&self, principal: &Principal) -> Result<Overview, FacadeError> {
		let projects = self.0.store.projects().map_err(FacadeError::Storage)?;
		let operations = self.0.store.operations().map_err(FacadeError::Storage)?;

		Ok(Overview {
			format_version: 1,
			projects: projects
				.into_iter()
				.filter(|p| principal.allows(&p.id, Scope::Read))
				.collect(),
			operations: operations
				.into_iter()
				.filter(|o| principal.allows(&o.project, Scope::Read))
				.collect(),
		})
	}

	/// Enqueues an intent under the principal's scope and the queue's
	/// revision, idempotency, one-operation-per-project and plan-ownership
	/// rules, then wakes the worker.
	pub fn enqueue(
		&self,
		principal: &Principal,
		project: &str,
		intent: &Intent,
	) -> Result<Operation, EnqueueError> {
		let reviewed_plan = matches!(intent.action, Action::Plan | Action::Apply { .. });
		if !principal.allows(project, Scope::Deploy)
			|| (reviewed_plan && principal.scope != Scope::Owner)
		{
			return Err(EnqueueError::Forbidden);
		}

		let operation = self
			.0
			.store
			.enqueue_as(project, intent, principal.actor.as_deref())?;
		self.0.wake.notify_one();
		if let Some(event) = operation.events.last() {
			self.publish(Update::Progress {
				operation_id: operation.id.clone(),
				event: event.clone(),
			});
		}

		Ok(operation)
	}

	/// One operation, if it exists and the principal may read its project.
	pub fn operation(&self, principal: &Principal, id: &str) -> Result<Operation, FacadeError> {
		let operation = self
			.0
			.store
			.operation(id)
			.map_err(FacadeError::Storage)?
			.ok_or(FacadeError::NotFound)?;

		if !principal.allows(&operation.project, Scope::Read) {
			return Err(FacadeError::Forbidden);
		}

		Ok(operation)
	}

	/// One project's analytics history over a bounded range of at most a day.
	pub fn fleet(
		&self,
		principal: &Principal,
		project: &str,
		from: u64,
		to: u64,
	) -> Result<Fleet, FacadeError> {
		if !principal.allows(project, Scope::Read) {
			return Err(FacadeError::Forbidden);
		}

		self.0
			.store
			.fleet(project, from, to)
			.map_err(FacadeError::Invalid)
	}

	/// Issues a long-lived project credential; owner authority is required
	/// and cannot be delegated.
	pub fn issue_token(
		&self,
		principal: &Principal,
		scope: Scope,
		project: &str,
	) -> Result<IssuedToken, FacadeError> {
		if principal.scope != Scope::Owner || principal.project.is_some() || scope == Scope::Owner {
			return Err(FacadeError::Forbidden);
		}

		let key = self
			.0
			.store
			.issue(scope, Some(project), 30 * 86400, false)
			.map_err(FacadeError::Storage)?;

		Ok(IssuedToken {
			key,
			expires_in: 30 * 86400,
		})
	}

	/// Revokes one management credential; owner authority is required.
	pub fn revoke_token(&self, principal: &Principal, key: &str) -> Result<(), FacadeError> {
		if principal.scope != Scope::Owner || principal.project.is_some() {
			return Err(FacadeError::Forbidden);
		}

		self.0.store.revoke(key).map_err(FacadeError::Storage)
	}

	/// Subscribes to one operation's journal updates: progress events, then
	/// the terminal operation. It does not authorize; pair it with
	/// [`Service::operation`] to enforce read scope.
	#[must_use]
	pub fn subscribe(&self, operation_id: &str) -> Subscription {
		Subscription::new(operation_id.to_owned(), self.0.events.subscribe())
	}

	fn publish(&self, update: Update) {
		let _ = self.0.events.send(update);
	}

	async fn auth(&self, headers: &HeaderMap, write: bool) -> ApiResult<Principal> {
		if let Some(key) = headers
			.get("authorization")
			.and_then(|h| h.to_str().ok())
			.and_then(|h| h.strip_prefix("Bearer "))
		{
			if key.len() <= 256
				&& bool::from(
					self.0
						.owner_hash
						.ct_eq(&sha2::Sha256::digest(key.as_bytes())),
				) {
				return Ok(Principal {
					scope: Scope::Owner,
					project: None,
					session: false,
					actor: None,
				});
			}

			return self.token(key.to_owned(), false).await;
		}
		let key = headers
			.get("cookie")
			.and_then(|h| h.to_str().ok())
			.and_then(|cookies| {
				cookies
					.split(';')
					.find_map(|cookie| cookie.trim().strip_prefix("lk_session="))
			})
			.ok_or_else(unauthorized)?;
		let principal = self.token(key.to_owned(), true).await?;

		if write && !self.same_origin(headers) {
			return Err(forbidden());
		}

		Ok(principal)
	}

	async fn token(&self, key: String, session: bool) -> ApiResult<Principal> {
		let store = self.0.store.clone();
		tokio::task::spawn_blocking(move || store.authenticate(&key, session))
			.await
			.map_err(|_| {
				ApiError(
					StatusCode::SERVICE_UNAVAILABLE,
					"authentication_store_unavailable",
				)
			})?
			.map_err(storage)?
			.ok_or_else(unauthorized)
	}

	fn same_origin(&self, headers: &HeaderMap) -> bool {
		headers.get("origin").and_then(|h| h.to_str().ok()) == Some(self.0.origin.as_str())
	}
}

type ApiResult<T> = std::result::Result<T, ApiError>;
struct ApiError(StatusCode, &'static str);
impl IntoResponse for ApiError {
	fn into_response(self) -> Response {
		(self.0, Json(serde_json::json!({"error":self.1}))).into_response()
	}
}
fn unauthorized() -> ApiError {
	ApiError(StatusCode::UNAUTHORIZED, "authentication_required")
}
fn forbidden() -> ApiError {
	ApiError(StatusCode::FORBIDDEN, "scope_or_origin_denied")
}
fn storage(_: anyhow::Error) -> ApiError {
	ApiError(
		StatusCode::SERVICE_UNAVAILABLE,
		"management_store_unavailable",
	)
}
fn facade_error(error: FacadeError) -> ApiError {
	match error {
		FacadeError::Forbidden => forbidden(),
		FacadeError::NotFound => ApiError(StatusCode::NOT_FOUND, "operation_not_found"),
		FacadeError::Invalid(_) => {
			ApiError(StatusCode::BAD_REQUEST, "analytics_range_or_storage_error")
		}
		FacadeError::Storage(error) => storage(error),
	}
}
fn enqueue_error(error: EnqueueError) -> ApiError {
	match error {
		EnqueueError::Forbidden => forbidden(),
		EnqueueError::Rejected(rejection) => ApiError(StatusCode::CONFLICT, rejection.code()),
		EnqueueError::Storage(error) => storage(error),
	}
}

async fn headers(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
	let mut response = next.run(request).await;
	for (name, value) in [
		("cache-control", "no-store"),
		("x-content-type-options", "nosniff"),
		("referrer-policy", "no-referrer"),
		(
			"content-security-policy",
			"default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; connect-src \
			 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
		),
		("vary", "Cookie, Authorization"),
	] {
		response
			.headers_mut()
			.insert(name, HeaderValue::from_static(value));
	}
	response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
	key: String,
}
async fn login(
	State(service): State<Service>,
	headers: HeaderMap,
	Json(body): Json<Login>,
) -> ApiResult<Response> {
	if !service.same_origin(&headers) || body.key.len() > 256 {
		return Err(forbidden());
	}

	let mut auth = HeaderMap::new();
	auth.insert(
		"authorization",
		HeaderValue::from_str(&format!("Bearer {}", body.key)).map_err(|_| unauthorized())?,
	);
	let principal = service.auth(&auth, false).await?;

	if principal.scope != Scope::Owner {
		return Err(forbidden());
	}

	let key = service
		.0
		.store
		.issue(Scope::Owner, principal.project.as_deref(), 8 * 3600, true)
		.map_err(storage)?;
	let cookie = format!(
		"lk_session={key}; HttpOnly; SameSite=Strict; Path=/; Max-Age=28800{}",
		if service.0.secure { "; Secure" } else { "" }
	);
	Ok(([("set-cookie", cookie)], StatusCode::NO_CONTENT).into_response())
}

async fn logout(State(service): State<Service>, headers: HeaderMap) -> ApiResult<Response> {
	let principal = service.auth(&headers, true).await?;

	if principal.session
		&& let Some(cookie) = headers
			.get("cookie")
			.and_then(|v| v.to_str().ok())
			.and_then(|v| {
				v.split(';')
					.find_map(|s| s.trim().strip_prefix("lk_session="))
			}) {
		service.0.store.revoke(cookie).map_err(storage)?;
	}
	Ok((
		[(
			"set-cookie",
			"lk_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
		)],
		StatusCode::NO_CONTENT,
	)
		.into_response())
}

async fn overview(State(service): State<Service>, headers: HeaderMap) -> ApiResult<Json<Overview>> {
	let principal = service.auth(&headers, false).await?;
	Ok(Json(service.overview(&principal).map_err(facade_error)?))
}

async fn enqueue(
	State(service): State<Service>,
	Path(project): Path<String>,
	headers: HeaderMap,
	Json(intent): Json<Intent>,
) -> ApiResult<Json<Operation>> {
	let principal = service.auth(&headers, true).await?;
	Ok(Json(
		service
			.enqueue(&principal, &project, &intent)
			.map_err(enqueue_error)?,
	))
}

async fn operation(
	State(service): State<Service>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> ApiResult<Json<Operation>> {
	let principal = service.auth(&headers, false).await?;
	Ok(Json(
		service.operation(&principal, &id).map_err(facade_error)?,
	))
}

/// Live operation progress as server-sent events: the current state, then
/// journal updates until the operation finishes.
async fn operation_events(
	State(service): State<Service>,
	Path(id): Path<String>,
	headers: HeaderMap,
) -> ApiResult<Response> {
	let principal = service.auth(&headers, false).await?;
	// Subscribe before reading the snapshot, so an update published in
	// between is delivered (at worst twice) instead of lost.
	let subscription = service.subscribe(&id);
	let current = service.operation(&principal, &id).map_err(facade_error)?;
	let terminal = matches!(
		current.phase,
		Phase::Succeeded | Phase::Failed | Phase::Interrupted
	);
	let stream = EventStream {
		snapshot: Some(current),
		terminal,
		subscription,
	};

	Ok(Sse::new(stream)
		.keep_alive(KeepAlive::default())
		.into_response())
}

/// The current state followed by [`Service::subscribe`] updates.
struct EventStream {
	snapshot: Option<Operation>,
	terminal: bool,
	subscription: Subscription,
}

impl Stream for EventStream {
	type Item = Result<ServerEvent, serde_json::Error>;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		if let Some(operation) = self.snapshot.take() {
			return Poll::Ready(Some(frame("operation", &operation)));
		}

		if self.terminal {
			return Poll::Ready(None);
		}

		match Pin::new(&mut self.subscription).poll_next(cx) {
			Poll::Ready(Some(Update::Progress { event, .. })) => {
				Poll::Ready(Some(frame("progress", &event)))
			}
			Poll::Ready(Some(Update::Finished { operation })) => {
				self.terminal = true;

				Poll::Ready(Some(frame("done", &operation)))
			}
			Poll::Ready(None) => Poll::Ready(None),
			Poll::Pending => Poll::Pending,
		}
	}
}

fn frame<T: Serialize>(name: &str, value: &T) -> Result<ServerEvent, serde_json::Error> {
	Ok(ServerEvent::default()
		.event(name)
		.data(serde_json::to_string(value)?))
}

#[derive(Deserialize)]
struct Range {
	from: Option<u64>,
	to: Option<u64>,
}
async fn fleet(
	State(service): State<Service>,
	Path(project): Path<String>,
	Query(range): Query<Range>,
	headers: HeaderMap,
) -> ApiResult<Json<Fleet>> {
	let principal = service.auth(&headers, false).await?;
	let to = range.to.unwrap_or_else(now);
	let from = range.from.unwrap_or_else(|| to.saturating_sub(3600));
	// The analytics aggregation can touch a hundred thousand windows; keep it
	// off the runtime's worker threads, as before.
	let fleet = tokio::task::spawn_blocking(move || service.fleet(&principal, &project, from, to))
		.await
		.map_err(|_| {
			ApiError(
				StatusCode::SERVICE_UNAVAILABLE,
				"analytics_store_unavailable",
			)
		})?
		.map_err(facade_error)?;
	Ok(Json(fleet))
}

async fn ingest(
	State(service): State<Service>,
	Path(project): Path<String>,
	headers: HeaderMap,
	Json(snapshot): Json<leptoskit::apm::Snapshot>,
) -> ApiResult<StatusCode> {
	let principal = service.auth(&headers, true).await?;

	if principal.session || !principal.allows(&project, Scope::Ingest) {
		return Err(forbidden());
	}

	let store = service.0.store.clone();
	tokio::task::spawn_blocking(move || store.ingest(&project, &snapshot))
		.await
		.map_err(|_| {
			ApiError(
				StatusCode::SERVICE_UNAVAILABLE,
				"analytics_store_unavailable",
			)
		})?
		.map_err(|_| ApiError(StatusCode::BAD_REQUEST, "analytics_batch_rejected"))?;
	Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Issue {
	scope: Scope,
	project: String,
}
async fn issue(
	State(service): State<Service>,
	headers: HeaderMap,
	Json(body): Json<Issue>,
) -> ApiResult<Json<IssuedToken>> {
	let principal = service.auth(&headers, true).await?;
	Ok(Json(
		service
			.issue_token(&principal, body.scope, &body.project)
			.map_err(facade_error)?,
	))
}
async fn revoke(
	State(service): State<Service>,
	headers: HeaderMap,
	Json(body): Json<Login>,
) -> ApiResult<StatusCode> {
	let principal = service.auth(&headers, true).await?;
	service
		.revoke_token(&principal, &body.key)
		.map_err(facade_error)?;
	Ok(StatusCode::NO_CONTENT)
}

async fn page(State(service): State<Service>, headers: HeaderMap) -> Response {
	let principal = service.auth(&headers, false).await.ok();
	let store = service.0.store.clone();
	match tokio::task::spawn_blocking(move || {
		let overview = principal.and_then(|principal| service.overview(&principal).ok());
		dashboard::html(overview.as_ref(), &store)
	})
	.await
	{
		Ok(html) => axum::response::Html(html).into_response(),
		Err(_) => {
			ApiError(StatusCode::SERVICE_UNAVAILABLE, "dashboard_unavailable").into_response()
		}
	}
}
async fn script() -> impl IntoResponse {
	(
		[("content-type", "text/javascript; charset=utf-8")],
		include_str!("dashboard.mjs"),
	)
}
