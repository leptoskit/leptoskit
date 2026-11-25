//! The tenants a hub serves, how it learns of them, and how a request finds
//! one.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;
use std::time::Instant;

use leptoskit_axum::axum::Router;
use leptoskit_axum::axum::body::Body;
use leptoskit_axum::axum::extract::Request;
use leptoskit_axum::axum::http::StatusCode;
use leptoskit_axum::axum::http::Uri;
use leptoskit_axum::axum::response::IntoResponse as _;
use leptoskit_axum::axum::response::Response;
use leptoskit_cloud::notifications::HubState;
use leptoskit_cloud::notifications::KnownTenant;
use leptoskit_cloud::notifications::MAX_KNOWN_TENANTS;
use leptoskit_cloud::notifications::MAX_REPORTS;
use leptoskit_cloud::notifications::NotificationsReported;
use leptoskit_cloud::notifications::TenantChanges;
use leptoskit_cloud::notifications::TenantConfig;
use leptoskit_cloud::notifications::TenantReport;
use leptoskit_cloud::notifications::UsageReport;
use leptoskit_db::Db;
use leptoskit_db::PoolOptions;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::service;
use leptoskit_rpc as rpc;
use rpc::server::Anonymous;
use rpc::server::ServiceCallers;
use tower::ServiceExt as _;
use uuid::Uuid;

use crate::usage;
use crate::usage::Cursor;

/// A boxed future, so the traits here work as trait objects.
pub type Pending<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The service's routes, as the edge and the app reach them.
const PREFIX: &str = "/_leptoskit/services/notifications";

/// How many times one round asks again while the control plane says more
/// tenants are waiting.
const MOST_ASKS: usize = 20;

/// How long a tenant that failed waits before it is opened again, unless it
/// changes first.
const RETRY_AFTER: Duration = Duration::from_mins(5);

/// How long opening one tenant may take before it counts as failed, so one
/// slow database doesn't hold up every other tenant's round.
const OPEN_WITHIN: Duration = Duration::from_mins(2);

/// What stands in a reason for a tenant's secret.
const REDACTED: &str = "[redacted]";

/// The control plane, as a hub sees it.
pub trait Plane: Send + Sync {
	/// What changed among the hub's tenants, given the ones it holds.
	///
	/// # Errors
	///
	/// Why it couldn't be asked, in words.
	fn tenants(&self, known: Vec<KnownTenant>) -> Pending<'_, Result<TenantChanges, String>>;

	/// How each tenant is doing, and the pushes each sent.
	///
	/// # Errors
	///
	/// Why it couldn't be told, in words.
	fn report(
		&self,
		tenants: Vec<TenantReport>,
		usage: Vec<UsageReport>,
	) -> Pending<'_, Result<NotificationsReported, String>>;
}

impl Plane for leptoskit_cloud_worker::Control {
	fn tenants(&self, known: Vec<KnownTenant>) -> Pending<'_, Result<TenantChanges, String>> {
		Box::pin(async move {
			self.notification_tenants(known)
				.await
				.map_err(|error| error.to_string())
		})
	}

	fn report(
		&self,
		tenants: Vec<TenantReport>,
		usage: Vec<UsageReport>,
	) -> Pending<'_, Result<NotificationsReported, String>> {
		Box::pin(async move {
			self.report_notifications(tenants, usage)
				.await
				.map_err(|error| error.to_string())
		})
	}
}

/// How a tenant's database is opened from its address.
pub trait Open: Send + Sync {
	/// The database at `address`, not yet migrated.
	///
	/// # Errors
	///
	/// Why it couldn't be opened, in words that never hold the address.
	fn open<'a>(&'a self, address: &'a str) -> Pending<'a, Result<Db, String>>;
}

/// Opens each tenant's database with a pool of its own, of at most
/// `connections`.
#[derive(Clone, Copy, Debug)]
pub struct Connect {
	pub connections: usize,
}

impl Open for Connect {
	fn open<'a>(&'a self, address: &'a str) -> Pending<'a, Result<Db, String>> {
		Box::pin(async move {
			Db::connect_with(
				address,
				PoolOptions {
					max_connections: self.connections,
				},
			)
			.await
			// The driver's error can repeat the address, password and all.
			.map_err(|_| "its database couldn't be reached".to_owned())
		})
	}
}

/// One tenant the hub serves.
struct Served {
	version: i64,
	/// Kept so the tenant's live connections keep being followed, and so
	/// they can be closed when it stops being served.
	live: rpc::server::Live,
	jobs: rpc::server::JobsHandle,
	db: Db,
	/// Where `db` is, so a new version at the same address reuses its pool.
	address: String,
	/// What the control plane counts this tenant's pushes from.
	cursor: Option<String>,
}

/// A tenant that stops being served, or is served anew, stops its jobs and
/// closes its sockets: a client reconnects to what serves it now, and an
/// identity its old settings admitted is asked again.
impl Drop for Served {
	fn drop(&mut self) {
		self.jobs.stop();
		self.live.drain();
	}
}

/// How requests reach one tenant.
struct Route {
	router: Router,
	hostnames: Vec<String>,
	/// When it was opened, in the order of every tenant's: of two tenants
	/// that claim one hostname, the later has it.
	opened: u64,
}

/// A tenant the hub couldn't serve, at one version.
struct Failed {
	version: i64,
	reason: String,
	at: Instant,
}

#[derive(Default)]
struct Tenants {
	served: HashMap<Uuid, Arc<Mutex<Served>>>,
	routes: HashMap<Uuid, Route>,
	hosts: HashMap<String, Uuid>,
	failed: HashMap<Uuid, Failed>,
	opened: u64,
}

impl Tenants {
	/// Stops serving `tenant`, and gives its hostnames to whoever else
	/// claims them.
	fn forget(&mut self, tenant: Uuid) {
		self.served.remove(&tenant);
		self.failed.remove(&tenant);

		if self.routes.remove(&tenant).is_some() {
			self.rehost();
		}
	}

	/// Which tenant each hostname reaches, from every tenant's claims: the
	/// most recently opened wins one claimed twice, as the control plane
	/// gives a hostname to one environment at a time and a tenant that
	/// still claims one it lost hasn't been told yet.
	fn rehost(&mut self) {
		let mut routes: Vec<(&Uuid, &Route)> = self.routes.iter().collect();
		routes.sort_by_key(|(_, route)| route.opened);
		self.hosts.clear();

		for (tenant, route) in routes {
			for hostname in &route.hostnames {
				if let Some(earlier) = self.hosts.insert(hostname.clone(), *tenant) {
					eprintln!(
						"leptoskit-cloud-notifications: {hostname} is claimed by tenants \
						 {earlier} and {tenant}; {tenant}, opened later, has it"
					);
				}
			}
		}
	}
}

/// What one [`Hub::round`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Round {
	/// Tenants opened or reopened.
	pub opened: Vec<Uuid>,
	/// Tenants that couldn't be, and why.
	pub failed: Vec<(Uuid, String)>,
	/// Tenants dropped.
	pub removed: Vec<Uuid>,
	/// Pushes counted, by tenant.
	pub pushes: BTreeMap<Uuid, u64>,
	/// Why asking or telling the control plane failed, when it did.
	pub unreachable: Option<String>,
}

/// The tenants of one cluster, and their routes.
#[derive(Clone)]
pub struct Hub {
	tenants: Arc<Mutex<Tenants>>,
	open: Arc<dyn Open>,
}

impl Hub {
	/// A hub holding no tenants yet, opening their databases with `open`.
	pub fn new(open: Arc<dyn Open>) -> Self {
		Self {
			tenants: Arc::default(),
			open,
		}
	}

	fn tenants(&self) -> std::sync::MutexGuard<'_, Tenants> {
		self.tenants.lock().unwrap_or_else(PoisonError::into_inner)
	}

	/// The tenants it serves, at their versions.
	#[must_use]
	pub fn serving(&self) -> BTreeMap<Uuid, i64> {
		self.tenants()
			.served
			.iter()
			.map(|(tenant, served)| {
				let version = served
					.lock()
					.unwrap_or_else(PoisonError::into_inner)
					.version;
				(*tenant, version)
			})
			.collect()
	}

	/// What it tells the control plane it holds: the tenants it serves, and
	/// those that failed recently at their version, so they aren't sent
	/// again until they change or it is time to try again.
	fn known(&self) -> Vec<KnownTenant> {
		let tenants = self.tenants();
		let mut known: Vec<KnownTenant> = self
			.serving_locked(&tenants)
			.into_iter()
			.chain(
				tenants
					.failed
					.iter()
					.filter(|(_, failed)| failed.at.elapsed() < RETRY_AFTER)
					.map(|(tenant, failed)| (*tenant, failed.version)),
			)
			.map(|(tenant, version)| KnownTenant { tenant, version })
			.collect();
		known.truncate(MAX_KNOWN_TENANTS);
		known
	}

	fn serving_locked(&self, tenants: &Tenants) -> Vec<(Uuid, i64)> {
		let _ = self;
		tenants
			.served
			.iter()
			.map(|(tenant, served)| {
				(
					*tenant,
					served
						.lock()
						.unwrap_or_else(PoisonError::into_inner)
						.version,
				)
			})
			.collect()
	}

	/// Asks `plane` what changed, opens and drops tenants to match, and
	/// reports how each is doing and the pushes each sent up to `now`
	/// less [`usage::LAG`].
	pub async fn round(&self, plane: &dyn Plane, now: leptoskit_db::Timestamp) -> Round {
		let mut round = Round::default();

		for _ in 0..MOST_ASKS {
			let changes = match plane.tenants(self.known()).await {
				Ok(changes) => changes,
				Err(reason) => {
					round.unreachable = Some(reason);
					break;
				}
			};

			for tenant in changes.removed {
				self.tenants().forget(tenant);
				round.removed.push(tenant);
			}

			for config in changes.changed {
				let tenant = config.tenant;

				match self.serve(config).await {
					Ok(()) => round.opened.push(tenant),
					Err(reason) => round.failed.push((tenant, reason)),
				}
			}

			if !changes.more {
				break;
			}
		}

		self.report(plane, now, &mut round).await;
		round
	}

	/// Opens `config`'s tenant, and serves it in place of what it served
	/// for that tenant before; or, when it can't, stops serving it and
	/// remembers why.
	async fn serve(&self, config: TenantConfig) -> Result<(), String> {
		let tenant = config.tenant;
		let version = config.version;
		let secrets = secrets_of(&config);
		// The pool it holds for the same address, which a new version
		// reuses rather than opening another beside it.
		let reuse = self.tenants().served.get(&tenant).and_then(|served| {
			let served = served.lock().unwrap_or_else(PoisonError::into_inner);
			(served.address == config.secrets.database_url).then(|| served.db.clone())
		});
		let opening = open(self.open.as_ref(), config, reuse);
		let opened = match tokio::time::timeout(OPEN_WITHIN, opening).await {
			Ok(opened) => opened,
			Err(_) => {
				Err(format!(
					"it took more than {} seconds to open",
					OPEN_WITHIN.as_secs()
				))
			}
		};

		match opened {
			Ok((served, route)) => {
				let mut tenants = self.tenants();
				let previous = tenants.served.remove(&tenant);
				let previous_cursor = previous.as_ref().and_then(|served| {
					served
						.lock()
						.unwrap_or_else(PoisonError::into_inner)
						.cursor
						.clone()
				});
				tenants.failed.remove(&tenant);

				let mut served = served;
				// The control plane's cursor is what it held when it answered;
				// one it moved to since is at least as far.
				if served.cursor.is_none() {
					served.cursor = previous_cursor;
				}

				tenants.opened += 1;
				let opened = tenants.opened;
				tenants.routes.insert(
					tenant,
					Route {
						router: route.0,
						hostnames: route.1,
						opened,
					},
				);
				tenants.rehost();
				tenants.served.insert(tenant, Arc::new(Mutex::new(served)));
				drop(tenants);
				// Dropped outside the lock: it stops its jobs and closes its
				// sockets, which the new version's routes now take.
				drop(previous);
				Ok(())
			}
			Err(reason) => {
				let reason = redacted(&reason, &secrets);
				let mut tenants = self.tenants();
				tenants.forget(tenant);
				tenants.failed.insert(
					tenant,
					Failed {
						version,
						reason: reason.clone(),
						at: Instant::now(),
					},
				);
				Err(reason)
			}
		}
	}

	/// Tells `plane` how every tenant is doing, and each tenant's pushes
	/// since its cursor.
	async fn report(&self, plane: &dyn Plane, now: leptoskit_db::Timestamp, round: &mut Round) {
		let (states, counting) = {
			let tenants = self.tenants();
			let states: Vec<TenantReport> = self
				.serving_locked(&tenants)
				.into_iter()
				.map(|(tenant, version)| {
					TenantReport {
						tenant,
						version,
						state: HubState::Serving,
					}
				})
				.chain(tenants.failed.iter().map(|(tenant, failed)| {
					TenantReport {
						tenant: *tenant,
						version: failed.version,
						state: HubState::Failed {
							reason: failed.reason.clone(),
						},
					}
				}))
				.collect();
			let counting: Vec<(Uuid, Arc<Mutex<Served>>)> = tenants
				.served
				.iter()
				.map(|(tenant, served)| (*tenant, Arc::clone(served)))
				.collect();
			(states, counting)
		};

		let until = Cursor::at(now - usage::LAG);
		let mut usage = Vec::new();

		for (tenant, served) in counting {
			let (db, after) = {
				let served = served.lock().unwrap_or_else(PoisonError::into_inner);
				(served.db.clone(), served.cursor.clone())
			};
			let from = match after.as_deref().map(Cursor::parse) {
				Some(None) => {
					// Not a cursor this hub writes: counting from anywhere
					// would count twice or never.
					eprintln!(
						"leptoskit-cloud-notifications: tenant {tenant}'s usage cursor isn't one \
						 this hub writes; its pushes aren't counted"
					);
					continue;
				}
				Some(Some(cursor)) => Some(cursor),
				None => None,
			};

			if from.is_some_and(|from| from >= until) {
				continue;
			}

			match usage::count(&db, from, until).await {
				Ok(0) => {}
				Ok(pushes) => {
					round.pushes.insert(tenant, pushes);
					usage.push(UsageReport {
						tenant,
						after,
						until: until.to_string(),
						pushes,
					});
				}
				Err(reason) => {
					eprintln!("leptoskit-cloud-notifications: tenant {tenant}: {reason}");
				}
			}
		}

		let mut states = states.into_iter().peekable();
		let mut usage = usage.into_iter().peekable();

		while states.peek().is_some() || usage.peek().is_some() {
			let tenants: Vec<TenantReport> = states.by_ref().take(MAX_REPORTS).collect();
			let counted: Vec<UsageReport> = usage.by_ref().take(MAX_REPORTS).collect();

			match plane.report(tenants, counted).await {
				Ok(reported) => {
					let held = self.tenants();

					for (tenant, cursor) in reported.cursors {
						if let Some(served) = held.served.get(&tenant) {
							served.lock().unwrap_or_else(PoisonError::into_inner).cursor = cursor;
						}
					}
				}
				Err(reason) => {
					round.unreachable = Some(reason);
					round.pushes.clear();
					return;
				}
			}
		}
	}

	/// The hub's routes: a tenant's service by its hostname or its path,
	/// and `/healthz` and `/readyz` for the hub itself.
	pub fn router(&self) -> Router {
		let hub = self.clone();
		Router::new().fallback(move |request: Request| {
			let hub = hub.clone();
			async move { hub.dispatch(request).await }
		})
	}

	async fn dispatch(&self, mut request: Request) -> Response {
		let path = request.uri().path().to_owned();

		if path == "/healthz" || path == "/readyz" {
			return StatusCode::OK.into_response();
		}

		let router = if let Some(rest) = path.strip_prefix("/t/") {
			let (hex, rest) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
			let Some(tenant) = tenant_id(hex) else {
				return StatusCode::NOT_FOUND.into_response();
			};

			if !is_service_path(rest) {
				return StatusCode::NOT_FOUND.into_response();
			}

			let Some(uri) = rewritten(request.uri(), rest) else {
				return StatusCode::BAD_REQUEST.into_response();
			};
			*request.uri_mut() = uri;
			self.tenants()
				.routes
				.get(&tenant)
				.map(|route| route.router.clone())
		} else if is_service_path(&path) {
			let Some(host) = host_of(&request) else {
				return StatusCode::NOT_FOUND.into_response();
			};
			let tenants = self.tenants();
			tenants
				.hosts
				.get(&host)
				.and_then(|tenant| tenants.routes.get(tenant))
				.map(|route| route.router.clone())
		} else {
			None
		};

		match router {
			Some(router) => {
				match router.oneshot(request).await {
					Ok(response) => response,
					Err(never) => match never {},
				}
			}
			None => {
				Response::builder()
					.status(StatusCode::NOT_FOUND)
					.body(Body::from("no notifications are served here"))
					.unwrap_or_else(|_| StatusCode::NOT_FOUND.into_response())
			}
		}
	}
}

/// Whether `path` is under the service's routes.
fn is_service_path(path: &str) -> bool {
	path == PREFIX
		|| path
			.strip_prefix(PREFIX)
			.is_some_and(|rest| rest.starts_with('/'))
}

/// A tenant's ID as its path names it: 32 lower-case hex digits.
fn tenant_id(hex: &str) -> Option<Uuid> {
	(hex.len() == 32
		&& hex
			.bytes()
			.all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
	.then(|| Uuid::try_parse(hex).ok())
	.flatten()
}

/// `uri` with its path replaced by `path`, keeping its query.
fn rewritten(uri: &Uri, path: &str) -> Option<Uri> {
	let path_and_query = match uri.query() {
		Some(query) => format!("{path}?{query}"),
		None => path.to_owned(),
	};
	Uri::builder().path_and_query(path_and_query).build().ok()
}

/// The request's host, lower-cased and without its port: its `Host`
/// header, or over HTTP/2 the authority of its URI.
fn host_of(request: &Request) -> Option<String> {
	let host = match request
		.headers()
		.get(leptoskit_axum::axum::http::header::HOST)
	{
		Some(host) => host.to_str().ok()?,
		None => request.uri().host()?,
	};
	let host = match host.rsplit_once(':') {
		Some((name, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => name,
		_ => host,
	};
	(!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// What a tenant was given that its reasons must never show: its token, its
/// database's address and password, and the values of its settings.
fn secrets_of(config: &TenantConfig) -> Vec<String> {
	let address = &config.secrets.database_url;
	// `scheme://user:password@host/…`: the password alone, as an error
	// may quote it apart from the address.
	let password = address
		.split_once("://")
		.and_then(|(_, rest)| rest.rsplit_once('@'))
		.and_then(|(credentials, _)| credentials.split_once(':'))
		.map(|(_, password)| password.to_owned());
	let mut secrets: Vec<String> = [config.secrets.token.clone(), address.clone()]
		.into_iter()
		.chain(password)
		.chain(config.secrets.settings.values().cloned())
		// Shorter values are words a reason may use on its own account.
		.filter(|secret| secret.len() >= 4)
		.collect();
	// The longest first, so a secret that holds another goes whole.
	secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
	secrets
}

/// `reason` with each of `secrets` it holds replaced by [`REDACTED`].
fn redacted(reason: &str, secrets: &[String]) -> String {
	secrets.iter().fold(reason.to_owned(), |reason, secret| {
		reason.replace(secret.as_str(), REDACTED)
	})
}

/// Opens one tenant: its settings, its database (`reuse`, when it already
/// holds one at the same address), migrated, and its routes and job
/// workers. Returns what it serves, and its router and hostnames.
async fn open(
	open: &dyn Open,
	config: TenantConfig,
	reuse: Option<Db>,
) -> Result<(Served, (Router, Vec<String>)), String> {
	// Only the service's own settings: anything else the hub is given is
	// the operator's to say, not a tenant's.
	let settings: Vec<(String, String)> = config
		.secrets
		.settings
		.into_iter()
		.filter(|(name, _)| leptoskit_cloud::notifications::is_setting(name))
		.collect();
	let push = service::config::PushConfig::from_vars(settings.iter().cloned())?;
	let open_topics = service::config::OpenTopics::from_vars(settings.iter().cloned())?;
	let retention = service::config::Retention::from_vars(settings.iter().cloned())?;
	let runtime = if push.apns.is_none() && push.fcm.is_none() && push.web.is_none() {
		None
	} else {
		Some(Arc::new(service::transport::PushRuntime::build(&push)?))
	};
	let push = service::push::Push { runtime };

	let db = match reuse {
		Some(db) => db,
		None => open.open(&config.secrets.database_url).await?,
	};
	let migrations = match db.dialect() {
		leptoskit_db::Dialect::Postgres => leptoskit_notifications::POSTGRES_MIGRATIONS,
		leptoskit_db::Dialect::Sqlite => leptoskit_notifications::MIGRATIONS,
	};
	db.migrate(migrations)
		.await
		.map_err(|error| format!("its database couldn't be migrated: {error}"))?;

	let token = config.secrets.token;
	let callers = ServiceCallers::new(Anonymous)
		.caller(&token, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, &token);
	let (router, live) = service::routes_with_settings(
		db.clone(),
		config.origin.as_deref(),
		callers,
		push.clone(),
		open_topics,
	);
	live.start()
		.await
		.map_err(|error| format!("its live queries couldn't start: {error}"))?;
	let jobs = service::workers_with_settings(db.clone(), push, retention).start();

	let hostnames = config
		.hostnames
		.iter()
		.map(|hostname| hostname.to_ascii_lowercase())
		.collect();

	Ok((
		Served {
			version: config.version,
			live,
			jobs,
			db,
			address: config.secrets.database_url,
			cursor: config.usage_cursor,
		},
		(router, hostnames),
	))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_tenant_is_named_by_its_hex_alone() {
		let tenant = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
		assert_eq!(tenant_id("0123456789abcdef0123456789abcdef"), Some(tenant));

		for wrong in [
			"",
			"0123456789ABCDEF0123456789ABCDEF",
			"01234567-89ab-cdef-0123-456789abcdef",
			"0123456789abcdef0123456789abcde",
			"0123456789abcdef0123456789abcdef0",
		] {
			assert_eq!(tenant_id(wrong), None, "{wrong}");
		}
	}

	#[test]
	fn only_the_services_routes_are_its() {
		assert!(is_service_path("/_leptoskit/services/notifications"));
		assert!(is_service_path(
			"/_leptoskit/services/notifications/rpc/inbox"
		));
		assert!(!is_service_path("/_leptoskit/services/notificationsx"));
		assert!(!is_service_path("/_leptoskit/services/analytics/rpc"));
		assert!(!is_service_path("/"));
	}

	#[test]
	fn a_reason_never_shows_a_secret() {
		let secrets = {
			let mut secrets = vec![
				"postgres://ntf_ab:hunter22@db:5432/ntf_ab".to_owned(),
				"hunter22".to_owned(),
				"tok3n-value".to_owned(),
			];
			secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
			secrets
		};
		assert_eq!(
			redacted(
				"postgres://ntf_ab:hunter22@db:5432/ntf_ab refused hunter22 for tok3n-value",
				&secrets
			),
			"[redacted] refused [redacted] for [redacted]"
		);
	}

	#[test]
	fn a_path_is_rewritten_with_its_query() {
		let uri: Uri = "/t/abc/_leptoskit/services/notifications/live?x=1"
			.parse()
			.expect("a URI");
		assert_eq!(
			rewritten(&uri, "/_leptoskit/services/notifications/live")
				.expect("a URI")
				.to_string(),
			"/_leptoskit/services/notifications/live?x=1"
		);
	}
}
