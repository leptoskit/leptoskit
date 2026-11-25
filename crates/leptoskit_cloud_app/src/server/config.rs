//! The operator's environment, read and checked before anything starts.
//!
//! Nothing secret has a default, and no value is ever repeated in an
//! error: a message names the variable and what is wrong with it.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::sync::Arc;

use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::SmtpMailer;
use leptoskit_cloud::pricing::Catalogue;
use leptoskit_cloud::server::ControlConfig;
use leptoskit_cloud::server::DEFAULT_REGION;
use leptoskit_cloud::server::DevMode;
use leptoskit_cloud::server::DnsOverHttps;
use leptoskit_cloud::server::OAuthApp;
use leptoskit_cloud::server::PaymentsConfig;
use leptoskit_cloud::server::S3Config;
use leptoskit_cloud::server::SecretValue;
use leptoskit_cloud::server::SharedMailer;
use leptoskit_cloud::server::SharedResolver;
use leptoskit_cloud::server::ShippingSettings;
use leptoskit_cloud::server::StorageConfig;
use leptoskit_cloud::server::Stripe;
use leptoskit_cloud::server::catalogue_from_json;
use leptoskit_cloud::size::Region;

/// The public origin, such as `https://cloud.leptoskit.com`.
pub const ORIGIN: &str = "LEPTOSKIT_CLOUD_ORIGIN";
/// The database: `sqlite://path` or `postgres://…`.
pub const DATABASE_URL: &str = "LEPTOSKIT_CLOUD_DATABASE_URL";
/// The keys sessions and codes are signed with: `id:key` pairs, newest
/// first, each key 32 random bytes in unpadded base64url.
pub const AUTH_KEYS: &str = "LEPTOSKIT_CLOUD_AUTH_KEYS";
/// The vault's master keys, which wrap each organisation's data key: `id:key`
/// pairs, newest first, in the form of [`AUTH_KEYS`] and never the same
/// keys.
pub const VAULT_KEYS: &str = "LEPTOSKIT_CLOUD_VAULT_KEYS";
/// The DNS-over-HTTPS resolver custom domains are verified through, which
/// answers `?name=…&type=TXT` in JSON. [`DEFAULT_DNS_RESOLVER`] without it.
pub const DNS_RESOLVER: &str = "LEPTOSKIT_CLOUD_DNS_RESOLVER";
/// Cloudflare's public resolver.
pub const DEFAULT_DNS_RESOLVER: &str = "https://cloudflare-dns.com/dns-query";
/// Where mail is sent through, such as `smtps://user:password@host`.
pub const SMTP_URL: &str = "LEPTOSKIT_CLOUD_SMTP_URL";
/// Who mail is from, such as `LeptosKit Cloud <cloud@leptoskit.com>`.
pub const MAIL_FROM: &str = "LEPTOSKIT_CLOUD_MAIL_FROM";
/// GitHub sign-in's OAuth app.
pub const GITHUB_CLIENT_ID: &str = "LEPTOSKIT_CLOUD_GITHUB_CLIENT_ID";
pub const GITHUB_CLIENT_SECRET: &str = "LEPTOSKIT_CLOUD_GITHUB_CLIENT_SECRET";
/// Google sign-in's OAuth client.
pub const GOOGLE_CLIENT_ID: &str = "LEPTOSKIT_CLOUD_GOOGLE_CLIENT_ID";
pub const GOOGLE_CLIENT_SECRET: &str = "LEPTOSKIT_CLOUD_GOOGLE_CLIENT_SECRET";
/// The address to listen on.
pub const BIND: &str = "LEPTOSKIT_CLOUD_BIND";
/// The directory the console's client build wrote, holding `pkg/`. Without
/// it the console's pages render but don't hydrate.
pub const SITE: &str = "LEPTOSKIT_CLOUD_SITE";
/// Where projects run when they don't say.
pub const DEFAULT_REGION_VAR: &str = "LEPTOSKIT_CLOUD_DEFAULT_REGION";
/// The domain environments are served under, such as `apps.example.com`.
pub const APPS_DOMAIN: &str = "LEPTOSKIT_CLOUD_APPS_DOMAIN";
/// Where built images are pushed, such as `registry.example.com/leptoskit`.
pub const REGISTRY: &str = "LEPTOSKIT_CLOUD_REGISTRY";
/// A JSON price catalogue's path. Without it, previews say there is no price
/// catalogue.
pub const PRICE_CATALOGUE: &str = "LEPTOSKIT_CLOUD_PRICE_CATALOGUE";
/// The header the proxy in front of the control plane puts the client's
/// address in, such as `x-real-ip`: one the proxy sets itself, whatever the
/// client sent, to one address. `x-forwarded-for` and `forwarded` are
/// refused: they are lists the client writes the start of. Rate limits
/// count by it. Without it, and for a request the header is missing from,
/// they count by the connection's address, which behind a proxy is the
/// proxy's.
pub const CLIENT_IP_HEADER: &str = "LEPTOSKIT_CLOUD_CLIENT_IP_HEADER";
/// A restricted Stripe key, to take payment for plans: it writes customers,
/// checkout sessions and customer portal sessions, and reads subscriptions
/// and charges. With it, [`STRIPE_WEBHOOK_SECRET`], [`STRIPE_PRICES`] and
/// [`PRICE_CATALOGUE`] are required. Without it nobody can subscribe.
pub const STRIPE_KEY: &str = "LEPTOSKIT_CLOUD_STRIPE_KEY";
/// The secret Stripe signs the events it posts to `/webhooks/stripe` with.
pub const STRIPE_WEBHOOK_SECRET: &str = "LEPTOSKIT_CLOUD_STRIPE_WEBHOOK_SECRET";
/// Stripe's price for each plan that is sold, as `pro=price_…,team=price_…`.
pub const STRIPE_PRICES: &str = "LEPTOSKIT_CLOUD_STRIPE_PRICES";
/// `true` to have Stripe work out tax at checkout. It collects none until
/// the Stripe account is registered where tax is owed.
pub const STRIPE_AUTOMATIC_TAX: &str = "LEPTOSKIT_CLOUD_STRIPE_AUTOMATIC_TAX";
/// `true` to take a Stripe test key (`rk_test_…`) outside `--dev`, for a
/// staging control plane: nobody pays, and anyone with a test card gets a
/// paid plan. Without it a test key is refused outside `--dev`.
pub const STRIPE_TEST_MODE: &str = "LEPTOSKIT_CLOUD_STRIPE_TEST_MODE";
/// The S3-compatible store source archives and logs are kept in, such as
/// `https://<account>.r2.cloudflarestorage.com`.
pub const S3_ENDPOINT: &str = "LEPTOSKIT_CLOUD_S3_ENDPOINT";
/// The store's region; `auto` when not set.
pub const S3_REGION: &str = "LEPTOSKIT_CLOUD_S3_REGION";
/// The bucket source archives go in.
pub const S3_SOURCES_BUCKET: &str = "LEPTOSKIT_CLOUD_S3_SOURCES_BUCKET";
/// The bucket build and runtime logs go in.
pub const S3_LOGS_BUCKET: &str = "LEPTOSKIT_CLOUD_S3_LOGS_BUCKET";
/// The store's access key.
pub const S3_ACCESS_KEY_ID: &str = "LEPTOSKIT_CLOUD_S3_ACCESS_KEY_ID";
pub const S3_SECRET_ACCESS_KEY: &str = "LEPTOSKIT_CLOUD_S3_SECRET_ACCESS_KEY";
/// `true` to address buckets as `endpoint/bucket`, as `MinIO` does.
pub const S3_PATH_STYLE: &str = "LEPTOSKIT_CLOUD_S3_PATH_STYLE";
/// `false` to disable `x-amz-checksum-sha256` / `x-amz-checksum-mode` headers
/// for S3-compatible gateways that reject checksum headers. Defaults to `true`.
pub const S3_CHECKSUM_HEADERS: &str = "LEPTOSKIT_CLOUD_S3_CHECKSUM_HEADERS";
/// `false` on a control-plane replica that should never run the background
/// sweeper loop, which takes expired leases back and deletes expired archives
/// and logs. Defaults to `true`: every replica of a Deployment gets the same
/// value, and on Postgres the sweeper's advisory lock lets only one replica
/// sweep at a time.
pub const SWEEPER: &str = "LEPTOSKIT_CLOUD_SWEEPER";
/// Optional email address made an operator by `migrate` or `serve` while the
/// database has never had one, without requiring interactive email sign-in
/// first.
pub const BOOTSTRAP_OPERATOR_EMAIL: &str = "LEPTOSKIT_CLOUD_BOOTSTRAP_OPERATOR_EMAIL";
/// Optional maximum bytes per database backup dump (defaults to 20 GiB).
pub const BACKUP_CAP_BYTES: &str = "LEPTOSKIT_CLOUD_BACKUP_CAP_BYTES";
/// Optional maximum unexpired runtime log bytes per organisation (defaults to
/// 512 MiB).
pub const RUNTIME_LOG_CAP_BYTES: &str = "LEPTOSKIT_CLOUD_RUNTIME_LOG_CAP_BYTES";
/// In `--dev`, the directory the control plane keeps objects in and serves
/// on loopback, instead of an S3 store.
pub const STORAGE_DIR: &str = "LEPTOSKIT_CLOUD_STORAGE_DIR";

/// `--dev`'s apps domain, unless [`APPS_DOMAIN`] says otherwise.
pub const DEV_APPS_DOMAIN: &str = "apps.localhost";
/// `--dev`'s registry, unless [`REGISTRY`] says otherwise.
pub const DEV_REGISTRY: &str = "registry.localhost/leptoskit";

/// Where the server listens unless [`BIND`] says otherwise: this machine
/// only.
pub const DEFAULT_BIND: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);

/// Reads one environment variable.
pub type Vars<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Why the environment can't run the server.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
	/// A required variable isn't set.
	#[error("set {0}")]
	Missing(&'static str),
	/// A variable is set to something it can't be.
	#[error("{name} {problem}")]
	Invalid { name: &'static str, problem: String },
	/// Development mode can't run here.
	#[error("--dev {0}")]
	Dev(String),
}

fn invalid(name: &'static str, problem: impl Into<String>) -> ConfigError {
	ConfigError::Invalid {
		name,
		problem: problem.into(),
	}
}

/// A set, non-blank variable.
fn read(vars: Vars<'_>, name: &str) -> Option<String> {
	vars(name)
		.map(|value| value.trim().to_owned())
		.filter(|value| !value.is_empty())
}

fn required(vars: Vars<'_>, name: &'static str) -> Result<String, ConfigError> {
	read(vars, name).ok_or(ConfigError::Missing(name))
}

/// The database URL. Its `Debug` hides it: it may hold a password.
#[derive(Clone)]
pub struct DatabaseUrl(String);

impl DatabaseUrl {
	/// [`DATABASE_URL`], checked for a scheme this build can open.
	///
	/// # Errors
	///
	/// When it isn't set, or names a database this build can't open.
	pub fn from_vars(vars: Vars<'_>) -> Result<Self, ConfigError> {
		let url = required(vars, DATABASE_URL)?;

		if url.starts_with("sqlite:") {
			return Ok(Self(url));
		}

		if url.starts_with("postgres://") || url.starts_with("postgresql://") {
			if cfg!(feature = "postgres") {
				return Ok(Self(url));
			}

			return Err(invalid(
				DATABASE_URL,
				"names Postgres, which this build can't open: build with the `postgres` feature",
			));
		}

		Err(invalid(
			DATABASE_URL,
			"must start with `sqlite:` or `postgres://`",
		))
	}

	/// The URL.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// Whether it names Postgres.
	#[must_use]
	pub fn is_postgres(&self) -> bool {
		self.0.starts_with("postgres")
	}
}

impl std::fmt::Debug for DatabaseUrl {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str("DatabaseUrl([redacted])")
	}
}

/// What `serve` runs with.
#[derive(Debug)]
pub struct ServeConfig {
	/// Where to listen: [`BIND`].
	pub bind: SocketAddr,
	/// The database: [`DATABASE_URL`].
	pub database: DatabaseUrl,
	/// The control plane's configuration, before the origin follows the
	/// bound port: use [`control_for`](Self::control_for).
	pub control: ControlConfig,
	/// The console's client build: [`SITE`].
	pub site: Option<std::path::PathBuf>,
	/// Optional operator email, made an operator while the database has never
	/// had one: [`BOOTSTRAP_OPERATOR_EMAIL`].
	pub bootstrap_operator_email: Option<String>,
	/// Whether the origin is the bind address, which `--dev` uses when
	/// [`ORIGIN`] isn't set: then a bind to port 0 is reached at the port it
	/// gets.
	origin_from_bind: bool,
}

/// Optional operator email read and validated from
/// [`BOOTSTRAP_OPERATOR_EMAIL`].
///
/// # Errors
///
/// [`ConfigError::Invalid`] when set to a value that is not a valid email
/// address.
pub fn bootstrap_operator_email(vars: Vars<'_>) -> Result<Option<String>, ConfigError> {
	let Some(raw) = read(vars, BOOTSTRAP_OPERATOR_EMAIL) else {
		return Ok(None);
	};
	let normalized = raw.to_ascii_lowercase();
	leptoskit_db::Email::parse(&normalized)
		.map_err(|_| invalid(BOOTSTRAP_OPERATOR_EMAIL, "must be a valid email address"))?;
	Ok(Some(normalized))
}

impl ServeConfig {
	/// The control plane's configuration, once the server listens on
	/// `address`.
	#[must_use]
	pub fn control_for(self, address: SocketAddr) -> ControlConfig {
		let mut control = self.control;

		if self.origin_from_bind {
			control.origin = format!("http://{address}");
		}

		control
	}

	/// Reads and checks everything `serve` needs. With `dev`, the keys may
	/// be left out (a random key is made for this run), mail is kept in
	/// memory, and the server must listen on, and be reached at, this
	/// machine only.
	///
	/// Call it inside a Tokio runtime: the SMTP mailer starts its connection
	/// pool as it is made.
	///
	/// # Errors
	///
	/// [`ConfigError`], naming the variable.
	pub fn from_vars(vars: Vars<'_>, dev: bool) -> Result<Self, ConfigError> {
		if dev && !cfg!(debug_assertions) {
			return Err(ConfigError::Dev(
				"is for development builds only; this is a release build".to_owned(),
			));
		}

		let database = DatabaseUrl::from_vars(vars)?;
		let bind = match read(vars, BIND) {
			Some(bind) => {
				bind.parse::<SocketAddr>().map_err(|_| {
					invalid(BIND, "must be an address and port, such as 0.0.0.0:8080")
				})?
			}
			None => DEFAULT_BIND,
		};

		if dev && !bind.ip().is_loopback() {
			return Err(ConfigError::Dev(format!(
				"listens on this machine only: set {BIND} to a loopback address, such as \
				 127.0.0.1:8080, not {}",
				bind.ip()
			)));
		}

		let origin_from_bind = dev && read(vars, ORIGIN).is_none();
		let origin = origin(vars, dev, bind)?;
		let keys = keys(vars, dev)?;
		let vault = vault_keys(vars, Some(&keys), dev)?;
		// Everything else is checked before the SMTP mailer starts its pool.
		let default_region =
			read(vars, DEFAULT_REGION_VAR).unwrap_or_else(|| DEFAULT_REGION.into());
		Region::parse(&default_region)
			.map_err(|error| invalid(DEFAULT_REGION_VAR, error.to_string()))?;
		let storage = storage(vars, dev)?;
		let apps_domain = read(vars, APPS_DOMAIN)
			.or_else(|| dev.then(|| DEV_APPS_DOMAIN.to_owned()))
			.ok_or(ConfigError::Missing(APPS_DOMAIN))?;
		let registry = read(vars, REGISTRY)
			.or_else(|| dev.then(|| DEV_REGISTRY.to_owned()))
			.ok_or(ConfigError::Missing(REGISTRY))?;
		let catalogue = catalogue(vars)?;
		let payments = payments(vars, dev)?;
		let resolver = resolver(vars, dev)?;
		let shipping = shipping_settings(vars)?;
		let bootstrap_operator_email = bootstrap_operator_email(vars)?;
		let (mailer, mailbox) = if dev {
			let mailbox = Mailbox::new();
			(SharedMailer::new(mailbox.clone()), Some(mailbox))
		} else {
			let url = required(vars, SMTP_URL)?;
			let from = required(vars, MAIL_FROM)?;
			let mailer = SmtpMailer::new(&url, &from).map_err(|error| {
				// The error never repeats the URL, which holds a password.
				invalid(SMTP_URL, format!("(or {MAIL_FROM}) is unusable: {error}"))
			})?;
			(SharedMailer::new(mailer), None)
		};

		Ok(Self {
			bind,
			database,
			control: ControlConfig {
				origin,
				keys,
				vault,
				mailer,
				resolver,
				github: oauth_app(vars, GITHUB_CLIENT_ID, GITHUB_CLIENT_SECRET)?,
				google: oauth_app(vars, GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET)?,
				default_region,
				storage,
				apps_domain,
				registry,
				catalogue,
				payments,
				rate_limits: leptoskit_cloud::server::RateLimits::default(),
				client_ip_header: client_ip_header(vars)?,
				shipping,
				dev: mailbox.map(|mailbox| DevMode { mailbox }),
			},
			site: read(vars, SITE).map(std::path::PathBuf::from),
			bootstrap_operator_email,
			origin_from_bind,
		})
	}
}

fn shipping_settings(vars: Vars<'_>) -> Result<ShippingSettings, ConfigError> {
	let run_sweeper = match read(vars, SWEEPER).as_deref() {
		None | Some("true") => true,
		Some("false") => false,
		Some(_) => return Err(invalid(SWEEPER, "must be `true` or `false`")),
	};
	let mut settings = ShippingSettings {
		run_sweeper,
		..ShippingSettings::default()
	};
	let max_bytes = i64::MAX as u64;
	if let Some(raw) = read(vars, BACKUP_CAP_BYTES) {
		settings.backup_cap = raw
			.parse::<u64>()
			.ok()
			.filter(|&v| (1..=max_bytes).contains(&v))
			.ok_or_else(|| {
				invalid(
					BACKUP_CAP_BYTES,
					"must be a positive byte count within 63-bit range",
				)
			})?;
	}
	if let Some(raw) = read(vars, RUNTIME_LOG_CAP_BYTES) {
		settings.organization_runtime_log_cap = raw
			.parse::<u64>()
			.ok()
			.filter(|&v| (1..=max_bytes).contains(&v))
			.ok_or_else(|| {
				invalid(
					RUNTIME_LOG_CAP_BYTES,
					"must be a positive byte count within 63-bit range",
				)
			})?;
	}
	Ok(settings)
}

/// What reads a custom domain's challenge record: [`DNS_RESOLVER`], over
/// `https` except in `--dev`.
fn resolver(vars: Vars<'_>, dev: bool) -> Result<SharedResolver, ConfigError> {
	let endpoint = read(vars, DNS_RESOLVER).unwrap_or_else(|| DEFAULT_DNS_RESOLVER.to_owned());

	if !dev && !endpoint.starts_with("https://") {
		return Err(invalid(DNS_RESOLVER, "must be https"));
	}

	let resolver = DnsOverHttps::new(&endpoint)
		.map_err(|error| invalid(DNS_RESOLVER, format!("is unusable: {error}")))?;
	Ok(Arc::new(resolver))
}

/// The public origin: `https` with no path, or in `--dev`, this machine
/// over `http` (by default, the bind address).
fn origin(vars: Vars<'_>, dev: bool, bind: SocketAddr) -> Result<String, ConfigError> {
	let Some(origin) = read(vars, ORIGIN) else {
		return if dev {
			Ok(format!("http://{bind}"))
		} else {
			Err(ConfigError::Missing(ORIGIN))
		};
	};
	let parsed = url::Url::parse(&origin).map_err(|_| {
		invalid(
			ORIGIN,
			"must be an origin, such as https://cloud.example.com",
		)
	})?;
	let bare = parsed.path() == "/"
		&& parsed.query().is_none()
		&& parsed.fragment().is_none()
		&& parsed.username().is_empty()
		&& parsed.password().is_none();

	if !bare || parsed.host().is_none() {
		return Err(invalid(
			ORIGIN,
			"must be a scheme and host with no path, such as https://cloud.example.com",
		));
	}

	let local = match parsed.host() {
		Some(url::Host::Domain(domain)) => domain == "localhost",
		Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
		Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
		None => false,
	};

	if dev && !local {
		return Err(ConfigError::Dev(format!(
			"is reached on this machine only: set {ORIGIN} to http://localhost:<port> or a \
			 loopback address"
		)));
	}

	if !dev && parsed.scheme() != "https" {
		return Err(invalid(
			ORIGIN,
			"must be https outside --dev: sessions and passkeys need it",
		));
	}

	Ok(origin.trim_end_matches('/').to_owned())
}

/// The signing keys: [`AUTH_KEYS`], or in `--dev` without it, a random key
/// for this run only.
fn keys(vars: Vars<'_>, dev: bool) -> Result<Keys, ConfigError> {
	match read(vars, AUTH_KEYS) {
		Some(text) => {
			Keys::parse(&text).map_err(|error| {
				// The error names a key's ID, never the key.
				invalid(AUTH_KEYS, format!("is invalid: {error}"))
			})
		}
		None if dev => {
			let mut secret = [0_u8; 32];
			getrandom::fill(&mut secret).map_err(|_| {
				ConfigError::Dev("couldn't make a key: the system has no randomness".to_owned())
			})?;
			Ok(Keys::single("dev", secret))
		}
		None => Err(ConfigError::Missing(AUTH_KEYS)),
	}
}

/// The vault's master keys: [`VAULT_KEYS`], or in `--dev` without it, a
/// random key for this run only, so a secret stored in one run can't be
/// read in the next.
fn vault_keys(vars: Vars<'_>, auth: Option<&Keys>, dev: bool) -> Result<Keys, ConfigError> {
	match read(vars, VAULT_KEYS) {
		Some(text) => {
			let keys = Keys::parse(&text).map_err(|error| {
				// The error names a key's ID, never the key.
				invalid(VAULT_KEYS, format!("is invalid: {error}"))
			})?;

			// One leaked key must not open both sessions and secrets,
			// whatever each variable calls it.
			if auth.is_some_and(|auth| auth.shares_key_with(&keys)) {
				return Err(invalid(
					VAULT_KEYS,
					format!("must not hold a key that is in {AUTH_KEYS}: make it its own"),
				));
			}

			Ok(keys)
		}
		None if dev => {
			let mut secret = [0_u8; 32];
			getrandom::fill(&mut secret).map_err(|_| {
				ConfigError::Dev("couldn't make a key: the system has no randomness".to_owned())
			})?;
			Ok(Keys::single("dev", secret))
		}
		None => Err(ConfigError::Missing(VAULT_KEYS)),
	}
}

/// The object store: S3 when [`S3_ENDPOINT`] is set, and in `--dev` without
/// it, a directory: [`STORAGE_DIR`], or one under the system's temporary
/// directory.
fn storage(vars: Vars<'_>, dev: bool) -> Result<StorageConfig, ConfigError> {
	if !dev && read(vars, STORAGE_DIR).is_some() {
		return Err(invalid(
			STORAGE_DIR,
			format!("is for --dev only: set {S3_ENDPOINT} and its buckets"),
		));
	}

	let Some(endpoint) = read(vars, S3_ENDPOINT) else {
		return if dev {
			let directory = read(vars, STORAGE_DIR).map_or_else(
				|| std::env::temp_dir().join("leptoskit-cloud-dev-storage"),
				std::path::PathBuf::from,
			);
			Ok(StorageConfig::Local { directory })
		} else {
			Err(ConfigError::Missing(S3_ENDPOINT))
		};
	};
	let endpoint = url::Url::parse(&endpoint)
		.ok()
		.filter(|url| matches!(url.scheme(), "https" | "http") && url.host().is_some())
		.ok_or_else(|| {
			invalid(
				S3_ENDPOINT,
				"must be an http(s) address, such as https://s3.example.com",
			)
		})?;
	let path_style = match read(vars, S3_PATH_STYLE).as_deref() {
		None | Some("false") => false,
		Some("true") => true,
		Some(_) => return Err(invalid(S3_PATH_STYLE, "must be `true` or `false`")),
	};
	let checksum_headers = match read(vars, S3_CHECKSUM_HEADERS).as_deref() {
		None | Some("true") => true,
		Some("false") => false,
		Some(_) => return Err(invalid(S3_CHECKSUM_HEADERS, "must be `true` or `false`")),
	};

	Ok(StorageConfig::S3(S3Config {
		endpoint,
		region: read(vars, S3_REGION).unwrap_or_else(|| "auto".to_owned()),
		sources_bucket: required(vars, S3_SOURCES_BUCKET)?,
		logs_bucket: required(vars, S3_LOGS_BUCKET)?,
		access_key_id: required(vars, S3_ACCESS_KEY_ID)?,
		secret_access_key: SecretValue::new(required(vars, S3_SECRET_ACCESS_KEY)?),
		path_style,
		checksum_headers,
	}))
}

/// The price catalogue at [`PRICE_CATALOGUE`]'s path, if it names one.
fn catalogue(vars: Vars<'_>) -> Result<Option<Catalogue>, ConfigError> {
	let Some(path) = read(vars, PRICE_CATALOGUE) else {
		return Ok(None);
	};
	let text = std::fs::read_to_string(&path)
		.map_err(|error| invalid(PRICE_CATALOGUE, format!("can't be read: {error}")))?;

	catalogue_from_json(&text)
		.map(Some)
		.map_err(|error| invalid(PRICE_CATALOGUE, error.to_string()))
}

/// [`CLIENT_IP_HEADER`], when it is a header's name.
fn client_ip_header(vars: Vars<'_>) -> Result<Option<String>, ConfigError> {
	let Some(header) = read(vars, CLIENT_IP_HEADER) else {
		return Ok(None);
	};

	if header.is_empty()
		|| !header
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
	{
		return Err(invalid(
			CLIENT_IP_HEADER,
			"is a header's name, such as `x-real-ip`".to_owned(),
		));
	}

	let header = header.to_ascii_lowercase();

	// These are lists a client starts and each proxy adds to, so part of
	// their value is whatever the client sent.
	if matches!(header.as_str(), "x-forwarded-for" | "forwarded") {
		return Err(invalid(
			CLIENT_IP_HEADER,
			format!(
				"is a header holding one address that the proxy sets itself, such as `x-real-ip`; \
				 `{header}` is a list the client can write the start of"
			),
		));
	}

	Ok(Some(header))
}

/// How payment is taken: [`STRIPE_KEY`], [`STRIPE_WEBHOOK_SECRET`] and
/// [`STRIPE_PRICES`] together, or none of them. A test key is taken in
/// `--dev`, and elsewhere only with [`STRIPE_TEST_MODE`]: with one, a made
/// up card buys a plan.
fn payments(vars: Vars<'_>, dev: bool) -> Result<Option<PaymentsConfig>, ConfigError> {
	let Some(key) = read(vars, STRIPE_KEY) else {
		for name in [
			STRIPE_WEBHOOK_SECRET,
			STRIPE_PRICES,
			STRIPE_AUTOMATIC_TAX,
			STRIPE_TEST_MODE,
		] {
			if read(vars, name).is_some() {
				return Err(invalid(name, format!("is set without {STRIPE_KEY}")));
			}
		}

		return Ok(None);
	};
	// The error never repeats the key.
	let stripe =
		Stripe::new(&key).map_err(|error| invalid(STRIPE_KEY, format!("is unusable: {error}")))?;
	let livemode = Stripe::is_live_key(&key);
	let test_mode = match read(vars, STRIPE_TEST_MODE).as_deref() {
		None | Some("false") => false,
		Some("true") => true,
		Some(_) => return Err(invalid(STRIPE_TEST_MODE, "is `true` or `false`".to_owned())),
	};

	if livemode && test_mode {
		return Err(invalid(
			STRIPE_TEST_MODE,
			format!("is `true`, and {STRIPE_KEY} is a live key"),
		));
	}

	let webhook_secret = required(vars, STRIPE_WEBHOOK_SECRET)?;
	let mut prices = BTreeMap::new();

	for pair in required(vars, STRIPE_PRICES)?.split(',') {
		let Some((plan, price)) = pair.trim().split_once('=') else {
			return Err(invalid(
				STRIPE_PRICES,
				"is a list of `plan=price` pairs, such as `pro=price_123`".to_owned(),
			));
		};

		if prices
			.insert(plan.trim().to_owned(), price.trim().to_owned())
			.is_some()
		{
			return Err(invalid(
				STRIPE_PRICES,
				format!("names `{}` twice", plan.trim()),
			));
		}
	}

	let automatic_tax = match read(vars, STRIPE_AUTOMATIC_TAX).as_deref() {
		None | Some("false") => false,
		Some("true") => true,
		Some(_) => {
			return Err(invalid(
				STRIPE_AUTOMATIC_TAX,
				"is `true` or `false`".to_owned(),
			));
		}
	};
	// Last, so what else is wrong with the variables is said first.
	if !livemode && !test_mode && !dev {
		return Err(invalid(
			STRIPE_KEY,
			format!(
				"is a test key: with one, a made up card buys a plan. Use a live key, or set \
				 {STRIPE_TEST_MODE}=true for a control plane nobody pays"
			),
		));
	}

	Ok(Some(PaymentsConfig {
		provider: Arc::new(stripe),
		webhook_secret,
		livemode,
		prices,
		automatic_tax,
	}))
}

/// An OAuth app: both its ID and secret, or neither.
fn oauth_app(
	vars: Vars<'_>,
	id: &'static str,
	secret: &'static str,
) -> Result<Option<OAuthApp>, ConfigError> {
	match (read(vars, id), read(vars, secret)) {
		(Some(client_id), Some(client_secret)) => {
			Ok(Some(OAuthApp {
				client_id,
				client_secret,
			}))
		}
		(None, None) => Ok(None),
		(Some(_), None) => Err(ConfigError::Missing(secret)),
		(None, Some(_)) => Err(ConfigError::Missing(id)),
	}
}

#[cfg(test)]
mod tests {
	use std::collections::HashMap;

	use super::*;

	const KEY: &str = "k1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
	const VAULT_KEY: &str = "v1:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA";
	const SMTP: &str = "smtp://user:hunter2@127.0.0.1:2525";

	fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
		let map: HashMap<String, String> = pairs
			.iter()
			.map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
			.collect();
		move |name| map.get(name).cloned()
	}

	fn production() -> Vec<(&'static str, &'static str)> {
		vec![
			(DATABASE_URL, "sqlite://cloud.db"),
			(ORIGIN, "https://cloud.example.com"),
			(AUTH_KEYS, KEY),
			(VAULT_KEYS, VAULT_KEY),
			(SMTP_URL, SMTP),
			(MAIL_FROM, "Cloud <cloud@example.com>"),
			(APPS_DOMAIN, "apps.example.com"),
			(REGISTRY, "registry.example.com/leptoskit"),
			(S3_ENDPOINT, "https://s3.example.com"),
			(S3_SOURCES_BUCKET, "sources"),
			(S3_LOGS_BUCKET, "logs"),
			(S3_ACCESS_KEY_ID, "AKIDEXAMPLE"),
			(S3_SECRET_ACCESS_KEY, "wJalrXUtnFEMI-hunter3"), // gitleaks:allow (fake key)
		]
	}

	fn without(name: &str) -> Vec<(&'static str, &'static str)> {
		production()
			.into_iter()
			.filter(|(variable, _)| *variable != name)
			.collect()
	}

	fn error(pairs: &[(&str, &str)], dev: bool) -> String {
		ServeConfig::from_vars(&vars(pairs), dev)
			.expect_err("refused")
			.to_string()
	}

	#[tokio::test]
	async fn a_complete_environment_serves() {
		let config = ServeConfig::from_vars(&vars(&production()), false).expect("valid");
		assert_eq!(config.bind, DEFAULT_BIND);
		assert_eq!(config.control.origin, "https://cloud.example.com");
		assert_eq!(config.control.default_region, DEFAULT_REGION);
		assert!(config.control.dev.is_none());
		assert!(config.control.github.is_none());
		assert!(!format!("{config:?}").contains("hunter2"));
		assert!(!format!("{config:?}").contains("hunter3"));
		assert_eq!(config.control.apps_domain, "apps.example.com");
		assert!(config.control.catalogue.is_none());
		let StorageConfig::S3(s3) = &config.control.storage else {
			panic!("an S3 store");
		};
		assert_eq!((s3.region.as_str(), s3.path_style), ("auto", false));
		assert_eq!(s3.secret_access_key.expose(), "wJalrXUtnFEMI-hunter3");
	}

	#[test]
	fn nothing_secret_is_defaulted() {
		for name in [
			AUTH_KEYS,
			VAULT_KEYS,
			SMTP_URL,
			MAIL_FROM,
			ORIGIN,
			DATABASE_URL,
			APPS_DOMAIN,
			REGISTRY,
			S3_ENDPOINT,
			S3_SOURCES_BUCKET,
			S3_LOGS_BUCKET,
			S3_ACCESS_KEY_ID,
			S3_SECRET_ACCESS_KEY,
		] {
			assert_eq!(error(&without(name), false), format!("set {name}"));
		}
	}

	#[tokio::test]
	async fn bad_values_are_named_without_repeating_them() {
		let mut pairs = without(AUTH_KEYS);
		pairs.push((AUTH_KEYS, "k1:not-a-key-at-all-secretish"));
		let message = error(&pairs, false);
		assert!(message.starts_with(AUTH_KEYS), "{message}");
		assert!(!message.contains("secretish"), "{message}");

		let mut pairs = without(VAULT_KEYS);
		pairs.push((VAULT_KEYS, "v1:not-a-key-at-all-secretish"));
		let message = error(&pairs, false);
		assert!(message.starts_with(VAULT_KEYS), "{message}");
		assert!(!message.contains("secretish"), "{message}");

		// The vault's keys are its own: one leaked variable mustn't open
		// both sessions and secrets.
		let mut pairs = without(VAULT_KEYS);
		pairs.push((VAULT_KEYS, KEY));
		let message = error(&pairs, false);
		assert!(
			message.contains("must not hold a key that is in"),
			"{message}"
		);
		assert!(!message.contains("AAAA"), "{message}");

		// The same key under another name, beside one of its own, is still
		// the same key.
		let renamed = format!(
			"{VAULT_KEY},other-name:{}",
			KEY.split_once(':').expect("id:key").1
		);
		let mut pairs = without(VAULT_KEYS);
		pairs.push((VAULT_KEYS, &renamed));
		let message = error(&pairs, false);
		assert!(
			message.contains("must not hold a key that is in"),
			"{message}"
		);

		for (name, value, expected) in [
			(ORIGIN, "http://cloud.example.com", "must be https"),
			(
				DNS_RESOLVER,
				"http://dns.example.com/dns-query",
				"must be https",
			),
			(
				DNS_RESOLVER,
				"https://dns.example.com/q?name=x",
				"is unusable",
			),
			(ORIGIN, "https://cloud.example.com/path", "no path"),
			(DATABASE_URL, "mysql://x", "must start with"),
			(BIND, "everywhere", "must be an address"),
			(DEFAULT_REGION_VAR, "Mars!", "isn't a region name"),
			(S3_ENDPOINT, "s3.example.com", "must be an http(s) address"),
			(S3_PATH_STYLE, "yes", "must be `true` or `false`"),
			(STORAGE_DIR, "/var/objects", "is for --dev only"),
			(PRICE_CATALOGUE, "/nonexistent/prices.json", "can't be read"),
		] {
			let mut pairs = without(name);
			pairs.push((name, value));
			let message = error(&pairs, false);
			assert!(message.contains(expected), "{name}: {message}");
		}
	}

	#[tokio::test]
	async fn an_oauth_app_needs_both_halves() {
		let mut pairs = production();
		pairs.push((GITHUB_CLIENT_ID, "abc"));
		assert_eq!(error(&pairs, false), format!("set {GITHUB_CLIENT_SECRET}"));
		pairs.push((GITHUB_CLIENT_SECRET, "shh"));
		let config = ServeConfig::from_vars(&vars(&pairs), false).expect("valid");
		let github = config.control.github.expect("GitHub");
		assert_eq!(github.client_id, "abc");
		assert!(!format!("{github:?}").contains("shh"));
	}

	#[test]
	fn development_stays_on_this_machine() {
		let local = [(DATABASE_URL, "sqlite::memory:")];
		let config = ServeConfig::from_vars(&vars(&local), true).expect("valid");
		assert_eq!(config.control.origin, "http://127.0.0.1:8080");
		assert!(config.control.dev.is_some());
		assert_eq!(config.control.apps_domain, DEV_APPS_DOMAIN);
		assert_eq!(config.control.registry, DEV_REGISTRY);
		assert!(matches!(
			config.control.storage,
			StorageConfig::Local { .. }
		));

		// A free port is reached at the port it gets.
		let any_port = [(DATABASE_URL, "sqlite::memory:"), (BIND, "127.0.0.1:0")];
		let config = ServeConfig::from_vars(&vars(&any_port), true).expect("valid");
		let bound = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4321);
		assert_eq!(config.control_for(bound).origin, "http://127.0.0.1:4321");

		let everywhere = [(DATABASE_URL, "sqlite::memory:"), (BIND, "0.0.0.0:8080")];
		assert!(error(&everywhere, true).contains("loopback"));
		let remote = [
			(DATABASE_URL, "sqlite::memory:"),
			(ORIGIN, "https://cloud.example.com"),
		];
		assert!(error(&remote, true).contains("this machine only"));
	}

	#[tokio::test]
	async fn a_price_catalogue_is_read_from_its_file() {
		let directory = tempfile::tempdir().expect("a directory");
		let path = directory.path().join("prices.json");
		let path_text = path.to_string_lossy().into_owned();
		let mut pairs: Vec<(&str, &str)> = production();
		pairs.push((PRICE_CATALOGUE, &path_text));

		std::fs::write(&path, r#"{"currency": "usd", "plans": []}"#).expect("written");
		let message = error(&pairs, false);
		assert!(
			message.starts_with(PRICE_CATALOGUE) && message.contains("at least one plan"),
			"{message}"
		);

		std::fs::write(
			&path,
			r#"{"currency": "usd", "plans": [{"id": "hobby", "name": "Hobby", "monthly": 0}]}"#,
		)
		.expect("written");
		let config = ServeConfig::from_vars(&vars(&pairs), false).expect("valid");
		let catalogue = config.control.catalogue.expect("a catalogue");
		assert_eq!(catalogue.plans().len(), 1);
	}

	#[tokio::test]
	async fn a_proxys_header_is_a_headers_name() {
		let mut pairs = production();
		let config = ServeConfig::from_vars(&vars(&pairs), false).expect("valid");
		assert_eq!(config.control.client_ip_header, None);
		pairs.push((CLIENT_IP_HEADER, "X-Real-IP"));
		let config = ServeConfig::from_vars(&vars(&pairs), false).expect("valid");
		assert_eq!(
			config.control.client_ip_header.as_deref(),
			Some("x-real-ip")
		);
		pairs.pop();
		pairs.push((CLIENT_IP_HEADER, "x-real-ip: 1.2.3.4"));
		let refused = error(&pairs, false);
		assert!(refused.starts_with(CLIENT_IP_HEADER), "{refused}");

		// A list the client writes the start of is nobody's word.
		for list in ["X-Forwarded-For", "forwarded"] {
			pairs.pop();
			pairs.push((CLIENT_IP_HEADER, list));
			let refused = error(&pairs, false);
			assert!(
				refused.starts_with(CLIENT_IP_HEADER) && refused.contains("is a list"),
				"{refused}"
			);
		}
	}

	#[tokio::test]
	async fn stripe_is_all_of_its_variables_or_none() {
		let directory = tempfile::tempdir().expect("a directory");
		let path = directory.path().join("prices.json");
		let path_text = path.to_string_lossy().into_owned();
		std::fs::write(
			&path,
			r#"{"currency": "usd", "plans": [
				{"id": "hobby", "name": "Hobby", "monthly": 0},
				{"id": "pro", "name": "Pro", "monthly": 100, "spend_cap": 100}
			]}"#,
		)
		.expect("written");
		let mut pairs: Vec<(&str, &str)> = production();
		pairs.push((PRICE_CATALOGUE, &path_text));
		let with = |extra: &[(&'static str, &'static str)]| {
			let mut pairs = pairs.clone();
			pairs.extend_from_slice(extra);
			pairs
		};
		let config = ServeConfig::from_vars(&vars(&pairs), false).expect("valid");
		assert!(config.control.payments.is_none());

		let alone = error(&with(&[(STRIPE_PRICES, "pro=price_1")]), false);
		assert!(alone.starts_with(STRIPE_PRICES), "{alone}");
		let key = (STRIPE_KEY, "rk_test_made_up"); // gitleaks:allow (fake key)
		let no_secret = error(&with(&[key, (STRIPE_PRICES, "pro=price_1")]), false);
		assert!(no_secret.contains(STRIPE_WEBHOOK_SECRET), "{no_secret}");
		let secret = (STRIPE_WEBHOOK_SECRET, "whsec_made_up_for_the_tests");
		let publishable = error(
			&with(&[
				(STRIPE_KEY, "pk_test_made_up"),
				secret,
				(STRIPE_PRICES, "pro=price_1"),
			]),
			false,
		);
		assert!(
			publishable.starts_with(STRIPE_KEY) && !publishable.contains("pk_test_made_up"),
			"{publishable}"
		);
		let unpaired = error(&with(&[key, secret, (STRIPE_PRICES, "pro")]), false);
		assert!(unpaired.starts_with(STRIPE_PRICES), "{unpaired}");
		let twice = error(
			&with(&[key, secret, (STRIPE_PRICES, "pro=price_1,pro=price_2")]),
			false,
		);
		assert!(twice.contains("twice"), "{twice}");

		let complete = with(&[
			key,
			secret,
			(STRIPE_PRICES, "pro=price_1"),
			(STRIPE_AUTOMATIC_TAX, "true"),
		]);
		// A test key buys a plan with a made up card, so it is asked for.
		let test_key = error(&complete, false);
		assert!(
			test_key.starts_with(STRIPE_KEY) && test_key.contains(STRIPE_TEST_MODE),
			"{test_key}"
		);
		let mut staging = complete.clone();
		staging.push((STRIPE_TEST_MODE, "true"));
		let config = ServeConfig::from_vars(&vars(&staging), false).expect("valid");
		let payments = config.control.payments.expect("payments");
		assert_eq!(payments.prices["pro"], "price_1");
		assert!(payments.automatic_tax);
		assert!(!payments.livemode);
		assert!(!format!("{payments:?}").contains("whsec_made_up"));

		let live = (STRIPE_KEY, "rk_live_made_up"); // gitleaks:allow (fake key)
		let mut paid = with(&[live, secret, (STRIPE_PRICES, "pro=price_1")]);
		let config = ServeConfig::from_vars(&vars(&paid), false).expect("valid");
		assert!(config.control.payments.expect("payments").livemode);
		paid.push((STRIPE_TEST_MODE, "true"));
		let mixed = error(&paid, false);
		assert!(mixed.starts_with(STRIPE_TEST_MODE), "{mixed}");
	}

	#[tokio::test]
	async fn path_style_buckets_for_stores_that_need_them() {
		let mut pairs = production();
		pairs.push((S3_PATH_STYLE, "true"));
		pairs.push((S3_REGION, "us-east-1"));
		let config = ServeConfig::from_vars(&vars(&pairs), false).expect("valid");
		let StorageConfig::S3(s3) = config.control.storage else {
			panic!("an S3 store");
		};
		assert_eq!((s3.region.as_str(), s3.path_style), ("us-east-1", true));
	}

	#[tokio::test]
	async fn validates_bootstrap_operator_email_and_byte_caps() {
		let mut pairs = production();
		pairs.push((BOOTSTRAP_OPERATOR_EMAIL, "not-an-email"));
		let bad_email = error(&pairs, false);
		assert!(
			bad_email.starts_with(BOOTSTRAP_OPERATOR_EMAIL),
			"{bad_email}"
		);

		let mut pairs = production();
		pairs.push((BACKUP_CAP_BYTES, "9223372036854775808"));
		let bad_backup = error(&pairs, false);
		assert!(bad_backup.starts_with(BACKUP_CAP_BYTES), "{bad_backup}");

		let mut pairs = production();
		pairs.push((RUNTIME_LOG_CAP_BYTES, "0"));
		let bad_log_cap = error(&pairs, false);
		assert!(
			bad_log_cap.starts_with(RUNTIME_LOG_CAP_BYTES),
			"{bad_log_cap}"
		);

		let mut pairs = production();
		pairs.push((BOOTSTRAP_OPERATOR_EMAIL, "OPS@Example.COM"));
		pairs.push((BACKUP_CAP_BYTES, "1048576"));
		pairs.push((RUNTIME_LOG_CAP_BYTES, "2097152"));
		let ok = ServeConfig::from_vars(&vars(&pairs), false).expect("valid");
		assert_eq!(
			ok.bootstrap_operator_email.as_deref(),
			Some("ops@example.com")
		);
		assert_eq!(ok.control.shipping.backup_cap, 1_048_576);
		assert_eq!(ok.control.shipping.organization_runtime_log_cap, 2_097_152);
	}
}
