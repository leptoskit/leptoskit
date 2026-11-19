//! The control plane's routes: the operations, the live socket, sign-in,
//! the device flow, the OpenAPI document, and what the CLI needs to find
//! its client.

use std::sync::Arc;

use leptoskit_auth::Auth;
use leptoskit_auth::Keys;
use leptoskit_auth::Mail;
use leptoskit_auth::MailFuture;
use leptoskit_auth::Mailbox;
use leptoskit_auth::Mailer;
use leptoskit_auth::Provider;
use leptoskit_auth::SolanaOptions;
use leptoskit_axum::axum::Json;
use leptoskit_axum::axum::Router;
use leptoskit_axum::axum::routing::get;
use leptoskit_db::Db;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::GrantType;
use leptoskit_idp::Idp;
use leptoskit_idp::NewClientSpec;
use leptoskit_rpc::server::Live;
use leptoskit_rpc::server::Server;
use leptoskit_rpc::spec::SpecOptions;
use serde::Serialize;

use super::Services;
use super::Shipping;
use super::ShippingSettings;
use super::identity::CloudIdentify;
use super::jobs;
use super::jobs::Swept;
use super::storage::ObjectStore;
use super::storage::StorageConfig;
use crate::OAuthScope;
use crate::pricing::Catalogue;

/// The name the CLI's OAuth client is registered under, and found by.
pub const CLI_CLIENT_NAME: &str = "LeptosKit CLI";

/// The scopes the CLI asks for: who the person is, and the cloud's
/// operations. The device flow gives it a refresh token as well, to stay
/// signed in.
pub const CLI_SCOPES: [&str; 3] = ["openid", "email", "cloud"];

/// Where projects run when `create_project` names no region.
pub const DEFAULT_REGION: &str = "eu-central";

/// Where the CLI learns its client ID and scopes before it starts the
/// device flow.
pub const DISCOVERY_PATH: &str = "/.well-known/leptoskit-cloud";

/// Where the operations are served.
pub const RPC_PATH: &str = "/rpc";

/// A mailer that can be shared: by `leptoskit_auth` for codes, and by the
/// control plane for invitations.
#[derive(Clone)]
pub struct SharedMailer(Arc<dyn Mailer>);

impl SharedMailer {
	/// Shares `mailer`.
	pub fn new(mailer: impl Mailer) -> Self {
		Self(Arc::new(mailer))
	}
}

impl Mailer for SharedMailer {
	fn send(&self, mail: Mail) -> MailFuture<'_> {
		self.0.send(mail)
	}
}

impl std::fmt::Debug for SharedMailer {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str("SharedMailer")
	}
}

/// A third-party sign-in's OAuth app: its client ID and secret. Its `Debug`
/// never shows the secret.
#[derive(Clone)]
pub struct OAuthApp {
	pub client_id: String,
	pub client_secret: String,
}

impl std::fmt::Debug for OAuthApp {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("OAuthApp")
			.field("client_id", &self.client_id)
			.field("client_secret", &"[redacted]")
			.finish()
	}
}

/// Development mode: `POST /auth/dev/sign-in` signs anyone in without a
/// code, and `GET /_leptoskit/mail` lists every email `mailbox` kept,
/// sign-in codes and invitations alike. Only debug builds have it.
#[derive(Clone, Debug)]
pub struct DevMode {
	/// The mailbox the control plane's mailer keeps mail in.
	pub mailbox: Mailbox,
}

/// How the control plane runs. Nothing secret has a default: the keys come
/// from the operator, and a sign-in provider is on only when its app is
/// given.
#[derive(Clone, Debug)]
pub struct ControlConfig {
	/// The public origin, such as `https://cloud.leptoskit.com`: what
	/// sessions, passkeys, wallet messages, and mail links name.
	pub origin: String,
	/// The keys sessions and codes are signed with.
	pub keys: Keys,
	/// The vault's master keys, newest first: each organisation's data key
	/// is wrapped by the newest, and any of them unwraps one. Never the
	/// sign-in keys.
	pub vault: Keys,
	/// How mail is sent.
	pub mailer: SharedMailer,
	/// What reads a custom domain's challenge record from DNS.
	pub resolver: super::dns::SharedResolver,
	/// GitHub sign-in, when configured.
	pub github: Option<OAuthApp>,
	/// Google sign-in, when configured.
	pub google: Option<OAuthApp>,
	/// Where projects run when they don't say.
	pub default_region: String,
	/// Where source archives and logs are kept.
	pub storage: StorageConfig,
	/// The domain environments are served under, such as
	/// `apps.leptoskit.dev`: a production environment at
	/// `<project>-<organization>.<domain>`.
	pub apps_domain: String,
	/// Where built images are pushed, such as `registry.example.com/leptoskit`:
	/// each project's under `<registry>/<organization id>/<project id>`.
	pub registry: String,
	/// The prices `preview_release` estimates with, when there are any.
	pub catalogue: Option<Catalogue>,
	/// Leases, addresses, retention, and the storage cap.
	pub shipping: ShippingSettings,
	/// Development mode, in debug builds only.
	pub dev: Option<DevMode>,
}

/// Why the control plane didn't start.
#[derive(Debug, thiserror::Error)]
pub enum ControlError {
	/// The configuration is wrong.
	#[error("{0}")]
	Config(String),
	/// `leptoskit_auth` refused its configuration.
	#[error("sign-in: {0}")]
	Auth(#[from] leptoskit_auth::AuthError),
	/// `leptoskit_idp` refused its configuration, or couldn't register the
	/// CLI's client.
	#[error("OAuth provider: {0}")]
	Idp(#[from] leptoskit_idp::IdpError),
	/// The OpenAPI document couldn't be collected.
	#[error("OpenAPI: {0}")]
	Spec(#[from] leptoskit_rpc::spec::SpecError),
	/// The live socket couldn't start.
	#[error("live: {0}")]
	Live(String),
}

/// The control plane, built: its sign-in, its OAuth provider, and its
/// routes.
#[derive(Clone)]
pub struct Control {
	auth: Auth,
	idp: Idp,
	live: Live,
	router: Router,
	cli_client_id: String,
	services: Services,
	_sweeper: Arc<Sweeper>,
}

/// The sweeper's task, stopped when the last [`Control`] goes.
struct Sweeper(tokio::task::JoinHandle<()>);

impl Drop for Sweeper {
	fn drop(&mut self) {
		self.0.abort();
	}
}

impl std::fmt::Debug for Control {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Control")
			.field("cli_client_id", &self.cli_client_id)
			.finish_non_exhaustive()
	}
}

/// What [`DISCOVERY_PATH`] answers.
#[derive(Clone, Debug, Serialize)]
struct Discovery {
	cli_client_id: String,
	scope: String,
	rpc: &'static str,
}

impl Control {
	/// Builds sign-in and the OAuth provider over `db`, which must be
	/// migrated, registers the CLI's client unless it is, and starts the
	/// live socket.
	///
	/// # Errors
	///
	/// [`ControlError`] when the configuration is refused, or the database
	/// fails.
	pub async fn start(db: &Db, config: ControlConfig) -> Result<Self, ControlError> {
		let origin = config.origin.trim_end_matches('/').to_owned();
		let auth = auth(db, &origin, &config)?;
		let idp = Idp::builder(db, &auth)
			.issuer(&origin)
			.scopes::<OAuthScope>()
			.build()?;
		let cli_client_id = cli_client(&idp).await?;
		check_apps_domain(&config.apps_domain)?;
		check_registry(&config.registry)?;
		let store = ObjectStore::new(&config.storage, &origin)
			.map_err(|error| ControlError::Config(error.to_string()))?;
		let services = Services::new(
			db,
			config.mailer.clone(),
			origin.clone(),
			config.default_region.clone(),
			Shipping {
				redactions: store.secrets().into(),
				vault: super::vault::Vault::new(config.vault.clone()),
				resolver: config.resolver.clone(),
				store: store.clone(),
				apps_domain: config.apps_domain.clone(),
				registry: config.registry.trim_end_matches('/').to_owned(),
				catalogue: config.catalogue.clone(),
				settings: config.shipping,
				jobs: Arc::new(tokio::sync::Notify::new()),
			},
		);
		let identify = CloudIdentify::new(auth.clone(), services.clone(), cli_client_id.clone());
		let server = Server::new(db.clone()).identify(identify);
		let live = server.live().origin(origin.clone());
		live.start()
			.await
			.map_err(|error| ControlError::Live(error.to_string()))?;
		let mut spec = SpecOptions::new("LeptosKit Cloud", env!("CARGO_PKG_VERSION"));
		spec.description = Some("The LeptosKit Cloud control plane.".to_owned());
		spec.servers = vec![origin.clone()];
		RPC_PATH.clone_into(&mut spec.path);
		let discovery = Discovery {
			cli_client_id: cli_client_id.clone(),
			scope: CLI_SCOPES.join(" "),
			rpc: RPC_PATH,
		};
		let mut router = Router::new()
			.merge(server.router())
			.merge(live.router())
			.merge(auth.router())
			.merge(idp.router())
			.merge(leptoskit_rpc::spec::openapi_router(&spec)?)
			.route(
				DISCOVERY_PATH,
				get(move || {
					let discovery = discovery.clone();
					async move { Json(discovery) }
				}),
			);

		if let Some(dev) = &config.dev {
			router = router.merge(dev_routes(dev));
		}

		if let Some(routes) = store.routes() {
			router = router.merge(routes);
		}

		// The session layer, then the OAuth provider's outside it: the
		// provider turns the CLI's access token into a delegation the
		// session layer then carries.
		let router = router.layer(auth.layer()).layer(idp.layer());

		let sweeper = Sweeper(tokio::spawn(sweep_forever(services.clone())));

		Ok(Self {
			auth,
			idp,
			live,
			router,
			cli_client_id,
			services,
			_sweeper: Arc::new(sweeper),
		})
	}

	/// Every route, layered.
	pub fn router(&self) -> Router {
		self.router.clone()
	}

	/// Sign-in.
	#[must_use]
	pub fn auth(&self) -> &Auth {
		&self.auth
	}

	/// The OAuth provider.
	#[must_use]
	pub fn idp(&self) -> &Idp {
		&self.idp
	}

	/// The live socket's server, to drain on shutdown.
	#[must_use]
	pub fn live(&self) -> &Live {
		&self.live
	}

	/// The CLI's OAuth client ID.
	#[must_use]
	pub fn cli_client_id(&self) -> &str {
		&self.cli_client_id
	}

	/// Sweeps now, as the sweeper does on its own every
	/// [`ShippingSettings::sweep`]: lapsed leases, builds nobody claimed,
	/// expired archives, and expired logs.
	///
	/// # Errors
	///
	/// When the database or the store fails.
	pub async fn sweep(&self) -> leptoskit_rpc::Result<Swept> {
		jobs::sweep(&self.services).await
	}
}

/// The price catalogue in `text`, a JSON object of `currency`, `plans`, and
/// `meters`, as [`Catalogue`] reads it.
///
/// # Errors
///
/// `Config` when `text` isn't a catalogue, or the catalogue can't work.
pub fn catalogue_from_json(text: &str) -> Result<Catalogue, ControlError> {
	serde_json::from_str(text)
		.map_err(|error| ControlError::Config(format!("isn't a price catalogue: {error}")))
}

/// Runs the sweeper every [`ShippingSettings::sweep`]. A failed sweep is
/// reported and tried again at the next.
async fn sweep_forever(services: Services) {
	let mut interval = tokio::time::interval(services.shipping().settings.sweep);
	interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

	loop {
		interval.tick().await;

		if let Err(error) = jobs::sweep(&services).await {
			eprintln!("leptoskit-cloud: the sweep failed: {error}");
		}
	}
}

/// `Config` unless `domain` is a lower-case DNS name of two labels or more,
/// short enough to put a 63-character label before.
fn check_apps_domain(domain: &str) -> Result<(), ControlError> {
	let labels: Vec<&str> = domain.split('.').collect();
	let valid = domain.len() <= 253 - 64
		&& labels.len() >= 2
		&& labels.iter().all(|label| {
			(1..=63).contains(&label.len())
				&& label
					.bytes()
					.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
				&& !label.starts_with('-')
				&& !label.ends_with('-')
		});

	if valid {
		Ok(())
	} else {
		Err(ControlError::Config(format!(
			"the apps domain must be a lower-case DNS name, such as apps.example.com, of at most \
			 {} characters; `{domain}` isn't",
			253 - 64
		)))
	}
}

/// `Config` unless `registry` is a host and path, such as
/// `registry.example.com/leptoskit`.
fn check_registry(registry: &str) -> Result<(), ControlError> {
	let registry = registry.trim_end_matches('/');
	let valid = !registry.is_empty()
		&& !registry.contains("://")
		&& registry.bytes().all(|byte| {
			byte.is_ascii_lowercase()
				|| byte.is_ascii_digit()
				|| matches!(byte, b'.' | b'-' | b'_' | b'/' | b':')
		});

	if valid {
		Ok(())
	} else {
		Err(ControlError::Config(
			"the registry must be a host and path with no scheme, such as \
			 registry.example.com/leptoskit"
				.to_owned(),
		))
	}
}

/// Sign-in: emailed codes, passkeys, Solana wallets, and GitHub and Google
/// when they're configured. No passwords.
fn auth(db: &Db, origin: &str, config: &ControlConfig) -> Result<Auth, ControlError> {
	let mut methods = vec!["email", "passkey", "wallet"];
	let mut builder = Auth::builder(db, config.keys.clone())
		.app_name("LeptosKit Cloud")
		.origin(origin)
		.mailer(config.mailer.clone())
		.solana(SolanaOptions {
			sign_in: true,
			chain_id: "mainnet".to_owned(),
		});

	if let Some(github) = &config.github {
		builder = builder.provider(Provider::github(&github.client_id, &github.client_secret));
		methods.push("github");
	}

	if let Some(google) = &config.google {
		builder = builder.provider(Provider::google(&google.client_id, &google.client_secret));
		methods.push("google");
	}

	builder = builder.sign_in_methods(methods);

	if config.dev.is_some() {
		builder = dev_sign_in(builder)?;
	}

	Ok(builder.build()?)
}

#[cfg(debug_assertions)]
#[allow(
	clippy::unnecessary_wraps,
	reason = "release builds refuse development mode here"
)]
fn dev_sign_in(
	builder: leptoskit_auth::AuthBuilder,
) -> Result<leptoskit_auth::AuthBuilder, ControlError> {
	Ok(builder.dev_sign_in())
}

#[cfg(not(debug_assertions))]
fn dev_sign_in(
	_: leptoskit_auth::AuthBuilder,
) -> Result<leptoskit_auth::AuthBuilder, ControlError> {
	Err(ControlError::Config(
		"release builds have no development mode".to_owned(),
	))
}

/// `GET /_leptoskit/mail`: every email kept, newest first, as plain text.
fn dev_routes(dev: &DevMode) -> Router {
	let mailbox = dev.mailbox.clone();
	Router::new().route(
		"/_leptoskit/mail",
		get(move || {
			let mailbox = mailbox.clone();
			async move {
				let mut out = String::new();

				for mail in mailbox.messages().iter().rev() {
					for part in [
						"To: ",
						mail.to.as_str(),
						"\nSubject: ",
						mail.subject.as_str(),
						"\n\n",
						mail.text.as_str(),
						"\n\n----\n\n",
					] {
						out.push_str(part);
					}
				}

				out
			}
		}),
	)
}

/// The CLI's client ID: the oldest first-party client named
/// [`CLI_CLIENT_NAME`], registered now when there is none. Two servers
/// starting at once may both register one; each then keeps the oldest and
/// removes its own.
async fn cli_client(idp: &Idp) -> Result<String, ControlError> {
	if let Some(existing) = oldest_cli_client(idp).await? {
		return Ok(existing);
	}

	let (registered, _) = idp
		.register_client(NewClientSpec {
			name: CLI_CLIENT_NAME.to_owned(),
			redirect_uris: Vec::new(),
			post_logout_redirect_uris: Vec::new(),
			scopes: CLI_SCOPES.iter().map(|&scope| scope.to_owned()).collect(),
			auth_method: ClientAuth::None,
			grant_types: vec![GrantType::DeviceCode],
			jwks: None,
			id_token_alg: leptoskit_idp::SigningAlgorithm::Es256,
			first_party: true,
			introspection: false,
			dpop_bound: false,
		})
		.await?;
	let oldest = oldest_cli_client(idp)
		.await?
		.unwrap_or_else(|| registered.client_id.clone());

	if oldest != registered.client_id {
		idp.remove_client(&registered.client_id).await?;
	}

	Ok(oldest)
}

async fn oldest_cli_client(idp: &Idp) -> Result<Option<String>, ControlError> {
	Ok(idp
		.clients()
		.await?
		.into_iter()
		.find(|client| {
			client.name == CLI_CLIENT_NAME
				&& client.first_party
				&& client.auth_method == ClientAuth::None
				&& client.grant_types.contains(&GrantType::DeviceCode)
		})
		.map(|client| client.client_id))
}
