//! One admin web app for a leptoskit site: its analytics, its
//! infrastructure, and lessons in both, signed in through the site's own
//! sign-in.
//!
//! The admin is its own deployable web app. People sign in on the main
//! site, which must run `leptoskit_idp` and register the admin as a
//! first-party app, and only accounts the main site gives the `admin` role
//! get in. Infrastructure actions run through the deploy console's control
//! [`Service`], recorded under the signed-in admin's name.
//!
//! ```no_run
//! # async fn run(service: leptoskit_deploy_console::control::Service) -> Result<(), Box<dyn std::error::Error>> {
//! use leptoskit_admin::Admin;
//! use leptoskit_admin::MainSite;
//!
//! let admin = Admin::builder("Notes")
//! 	.origin("https://admin.notes.example")
//! 	.main_site(MainSite::new(
//! 		"https://notes.example",
//! 		std::env::var("ADMIN_CLIENT_ID")?,
//! 		std::env::var("ADMIN_CLIENT_SECRET")?,
//! 	))
//! 	.keys(leptoskit_auth::Keys::from_env("LEPTOSKIT_ADMIN_KEYS")?)
//! 	.data_dir("/var/lib/notes-admin")
//! 	.control(service)
//! 	.build()
//! 	.await?;
//! let listener = tokio::net::TcpListener::bind("127.0.0.1:4801").await?;
//! axum::serve(listener, admin.router()).await?;
//! # Ok(())
//! # }
//! ```

mod access;
mod analytics;
mod charts;
mod components;
mod infra;
mod learn;
mod server;
mod shell;
mod signin;
mod style;
mod workings;

use std::path::PathBuf;
use std::sync::Arc;

use leptoskit_auth::Auth;
use leptoskit_auth::Keys;
use leptoskit_auth::Provider;
use leptoskit_auth::ProviderFuture;
use leptoskit_auth::ProviderHttp;
use leptoskit_auth::ProviderRequest;
use leptoskit_db::Db;
use leptoskit_deploy_console::control::Service;

/// Where the admin mounts: every page and endpoint lives under this path.
pub const BASE: &str = "/_leptoskit/admin";

/// Which environment the admin acts on, shown as the pill in the top bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Environment {
	/// Real people use it: the pill fills with ember.
	#[default]
	Production,
	/// A rehearsal of production.
	Staging,
	/// A developer's own machine.
	Development,
}

impl std::str::FromStr for Environment {
	type Err = AdminError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value {
			"production" => Ok(Self::Production),
			"staging" => Ok(Self::Staging),
			"development" => Ok(Self::Development),
			other => {
				Err(AdminError::Config(format!(
					"`{other}` isn't an environment: use production, staging, or development"
				)))
			}
		}
	}
}

/// The main site the admin signs people in through.
#[derive(Clone)]
pub struct MainSite {
	issuer: String,
	client_id: String,
	client_secret: String,
	name: Option<String>,
}

impl MainSite {
	/// The main site at `issuer` (its origin, such as
	/// `https://notes.example`), and the client ID and secret it gave the
	/// admin when you registered it with `leptoskit auth idp client add`.
	pub fn new(
		issuer: impl Into<String>,
		client_id: impl Into<String>,
		client_secret: impl Into<String>,
	) -> Self {
		Self {
			issuer: issuer.into(),
			client_id: client_id.into(),
			client_secret: client_secret.into(),
			name: None,
		}
	}

	/// The name on the sign-in button, **Continue with <name>**. The app's
	/// name by default.
	#[must_use]
	pub fn name(mut self, name: impl Into<String>) -> Self {
		self.name = Some(name.into());
		self
	}

	/// The main site's origin.
	#[must_use]
	pub fn issuer(&self) -> &str {
		&self.issuer
	}
}

impl std::fmt::Debug for MainSite {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("MainSite")
			.field("issuer", &self.issuer)
			.field("client_id", &self.client_id)
			.field("name", &self.name)
			.finish_non_exhaustive()
	}
}

/// The main site's analytics service, which the Analytics area reads.
///
/// The admin is its own app, so it calls the service the way the main
/// site's server does: at the service's private URL, signed with the
/// service's token (`LEPTOSKIT_SERVICE_ANALYTICS_TOKEN` on the main site),
/// as the caller the service knows as the app. Only the admin's server
/// holds the token; pages never see it.
#[derive(Clone)]
pub struct AnalyticsService {
	url: String,
	token: String,
}

impl AnalyticsService {
	/// The service at `url`, its private origin (such as
	/// `http://127.0.0.1:3110`), signed with `token`.
	pub fn new(url: impl Into<String>, token: impl Into<String>) -> Self {
		Self {
			url: url.into().trim_end_matches('/').to_owned(),
			token: token.into(),
		}
	}

	/// The service as the main site's server is told about it:
	/// `LEPTOSKIT_SERVICE_ANALYTICS_URL` and
	/// `LEPTOSKIT_SERVICE_ANALYTICS_TOKEN`. `None` unless both are set.
	#[must_use]
	pub fn from_env() -> Option<Self> {
		let read = |name: &str| {
			std::env::var(name)
				.ok()
				.filter(|value| !value.trim().is_empty())
		};
		Some(Self::new(
			read("LEPTOSKIT_SERVICE_ANALYTICS_URL")?,
			read("LEPTOSKIT_SERVICE_ANALYTICS_TOKEN")?,
		))
	}

	/// The service's origin.
	#[must_use]
	pub fn url(&self) -> &str {
		&self.url
	}
}

impl std::fmt::Debug for AnalyticsService {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("AnalyticsService")
			.field("url", &self.url)
			.finish_non_exhaustive()
	}
}

/// Why the admin couldn't start or answer.
#[derive(Clone, Debug, thiserror::Error)]
pub enum AdminError {
	/// Something it needs wasn't set, or was set wrong.
	#[error("{0}")]
	Config(String),
	/// Its database refused.
	#[error("the admin's database: {0}")]
	Database(String),
	/// A file it reads or writes.
	#[error("{0}")]
	Io(String),
	/// A request it can't act on.
	#[error("{0}")]
	Invalid(String),
}

impl AdminError {
	pub(crate) fn database(error: impl std::fmt::Display) -> Self {
		Self::Database(error.to_string())
	}
}

/// Sends the admin's requests to the main site through an embedder's
/// transport, for tests and for a main site in the same process.
#[derive(Clone)]
struct SharedHttp(Arc<dyn ProviderHttp>);

impl ProviderHttp for SharedHttp {
	fn send(&self, request: ProviderRequest) -> ProviderFuture<'_> {
		self.0.send(request)
	}
}

/// Sets up an [`Admin`].
pub struct AdminBuilder {
	app_name: String,
	origin: Option<String>,
	main_site: Option<MainSite>,
	roles: Vec<String>,
	environment: Environment,
	keys: Option<Keys>,
	data_dir: Option<PathBuf>,
	db: Option<Db>,
	control: Option<Service>,
	analytics: Option<AnalyticsService>,
	simulated_infrastructure: bool,
	http: Option<SharedHttp>,
}

impl AdminBuilder {
	/// The admin's own public origin, such as `https://admin.notes.example`.
	/// The main site sends people back here after they sign in.
	#[must_use]
	pub fn origin(mut self, origin: impl Into<String>) -> Self {
		self.origin = Some(origin.into().trim_end_matches('/').to_owned());
		self
	}

	/// The main site people sign in through.
	#[must_use]
	pub fn main_site(mut self, main_site: MainSite) -> Self {
		self.main_site = Some(main_site);
		self
	}

	/// The roles that let someone in, any one of which will do: `["admin"]`
	/// by default. The main site's `Roles` hook decides who has them.
	#[must_use]
	pub fn roles<S: Into<String>>(mut self, roles: impl IntoIterator<Item = S>) -> Self {
		self.roles = roles.into_iter().map(Into::into).collect();
		self
	}

	/// Which environment this admin acts on: production by default.
	#[must_use]
	pub fn environment(mut self, environment: Environment) -> Self {
		self.environment = environment;
		self
	}

	/// The keys that sign the admin's own sessions, as
	/// [`Keys::from_env`] reads them.
	#[must_use]
	pub fn keys(mut self, keys: Keys) -> Self {
		self.keys = Some(keys);
		self
	}

	/// Where the admin keeps its database: sessions, the roles the main
	/// site released, and each admin's preferences. Nothing else.
	#[must_use]
	pub fn data_dir(mut self, path: impl Into<PathBuf>) -> Self {
		self.data_dir = Some(path.into());
		self
	}

	/// The admin's database, instead of one in [`AdminBuilder::data_dir`].
	/// Its tables must already exist.
	#[must_use]
	pub fn database(mut self, db: Db) -> Self {
		self.db = Some(db);
		self
	}

	/// The deploy console's control service, whose projects the
	/// Infrastructure area shows and acts on. Run its worker yourself, as
	/// `lk admin serve` does.
	#[must_use]
	pub fn control(mut self, service: Service) -> Self {
		self.control = Some(service);
		self
	}

	/// The main site's analytics service, which the Analytics area reads
	/// and saves insights to. Without it, every Analytics screen says how
	/// to connect one.
	#[must_use]
	pub fn analytics(mut self, service: AnalyticsService) -> Self {
		self.analytics = Some(service);
		self
	}

	/// Says that the control service's worker runs against a stand-in
	/// cloud, as `admin_demo`'s simulated executor does, rather than real
	/// providers. Every screen then shows **Simulated** beside the
	/// environment pill, Overview and Changes say no provider is contacted,
	/// and plan and apply confirmations say they act on the stand-in.
	///
	/// Off by default, so a production admin never claims its actions are
	/// pretend. Only turn it on when the executor really is simulated.
	#[must_use]
	pub fn simulated_infrastructure(mut self, simulated: bool) -> Self {
		self.simulated_infrastructure = simulated;
		self
	}

	/// How the admin reaches the main site, instead of the network: for
	/// tests, and for a main site served from the same process.
	#[must_use]
	pub fn provider_http(mut self, http: impl ProviderHttp) -> Self {
		self.http = Some(SharedHttp(Arc::new(http)));
		self
	}

	/// Opens the database and sets up sign-in.
	///
	/// # Errors
	///
	/// When the origin, main site, or keys are missing, a role list is
	/// empty, or the database can't be opened or brought up to date.
	pub async fn build(self) -> Result<Admin, AdminError> {
		let origin = self
			.origin
			.ok_or_else(|| AdminError::Config("set the admin's origin with `.origin(…)`".into()))?;
		let main_site = self.main_site.ok_or_else(|| {
			AdminError::Config(
				"set the main site with `.main_site(MainSite::new(issuer, client_id, secret))`"
					.into(),
			)
		})?;
		let keys = self.keys.ok_or_else(|| {
			AdminError::Config("set the session keys with `.keys(Keys::from_env(…)?)`".into())
		})?;

		if self.roles.is_empty() {
			return Err(AdminError::Config(
				"`.roles(…)` needs at least one role, or no one could sign in".into(),
			));
		}

		let db = match (self.db, &self.data_dir) {
			(Some(db), _) => db,
			(None, Some(directory)) => {
				std::fs::create_dir_all(directory).map_err(|error| {
					AdminError::Io(format!("can't create {}: {error}", directory.display()))
				})?;
				let path = directory.join("admin.sqlite3");
				let db = Db::connect(&format!("sqlite://{}?mode=rwc", path.display()))
					.await
					.map_err(AdminError::database)?;
				access::migrate(&db, &directory.join("admin.schema.json")).await?;
				db
			}
			(None, None) => {
				return Err(AdminError::Config(
					"set where the admin keeps its sessions with `.data_dir(…)`".into(),
				));
			}
		};

		let button = main_site
			.name
			.clone()
			.unwrap_or_else(|| self.app_name.clone());
		let provider = Provider::oidc(
			access::PROVIDER,
			&button,
			&main_site.issuer,
			&main_site.client_id,
			Some(main_site.client_secret.clone()),
		)
		.map_err(|error| AdminError::Config(error.to_string()))?
		.userinfo(true)
		.scopes(["openid", "email", "profile", "roles"]);
		let mut auth = Auth::builder(&db, keys)
			.app_name(format!("{} admin", self.app_name))
			.origin(&origin)
			.mailer(leptoskit_auth::Mailbox::new())
			.provider(provider)
			.gate(access::AdminGate {
				roles: self.roles.clone(),
			})
			.roles::<access::AdminRoles>()
			.sign_in_methods([format!("oidc:{}", access::PROVIDER)]);

		if let Some(http) = self.http {
			auth = auth.provider_http(http);
		}

		let auth = auth
			.build()
			.map_err(|error| AdminError::Config(error.to_string()))?;

		Ok(Admin(Arc::new(Inner {
			app_name: self.app_name,
			origin,
			main_site,
			button,
			roles: self.roles,
			environment: self.environment,
			db,
			auth,
			control: self.control,
			analytics: self.analytics.map(analytics::Connection::new),
			simulated_infrastructure: self.simulated_infrastructure,
		})))
	}
}

/// A built admin: hand [`Admin::router`] to your server.
#[derive(Clone)]
pub struct Admin(Arc<Inner>);

pub(crate) struct Inner {
	pub app_name: String,
	pub origin: String,
	pub main_site: MainSite,
	/// The name on the sign-in button.
	pub button: String,
	pub roles: Vec<String>,
	pub environment: Environment,
	pub db: Db,
	pub auth: Auth,
	pub control: Option<Service>,
	pub analytics: Option<analytics::Connection>,
	/// Whether infrastructure actions run against a stand-in cloud.
	pub simulated_infrastructure: bool,
}

impl Admin {
	/// Starts an admin for the app called `app_name`.
	pub fn builder(app_name: impl Into<String>) -> AdminBuilder {
		AdminBuilder {
			app_name: app_name.into(),
			origin: None,
			main_site: None,
			roles: vec!["admin".to_owned()],
			environment: Environment::default(),
			keys: None,
			data_dir: None,
			db: None,
			control: None,
			analytics: None,
			simulated_infrastructure: false,
			http: None,
		}
	}

	/// Every page and endpoint: the admin under [`BASE`], sign-in under
	/// `/auth`, and `/` sending people to the admin.
	pub fn router(&self) -> axum::Router {
		server::router(self.clone())
	}

	/// The admin's sign-in, for embedders that add their own routes.
	#[must_use]
	pub fn auth(&self) -> &Auth {
		&self.0.auth
	}

	pub(crate) fn inner(&self) -> &Inner {
		&self.0
	}
}

impl std::fmt::Debug for Admin {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Admin")
			.field("app_name", &self.0.app_name)
			.field("origin", &self.0.origin)
			.field("main_site", &self.0.main_site)
			.field("roles", &self.0.roles)
			.field("simulated_infrastructure", &self.0.simulated_infrastructure)
			.finish_non_exhaustive()
	}
}
