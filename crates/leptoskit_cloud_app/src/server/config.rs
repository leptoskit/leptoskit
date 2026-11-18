//! The operator's environment, read and checked before anything starts.
//!
//! Nothing secret has a default, and no value is ever repeated in an
//! error: a message names the variable and what is wrong with it.

use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;

use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::SmtpMailer;
use leptoskit_cloud::pricing::Catalogue;
use leptoskit_cloud::server::ControlConfig;
use leptoskit_cloud::server::DEFAULT_REGION;
use leptoskit_cloud::server::DevMode;
use leptoskit_cloud::server::OAuthApp;
use leptoskit_cloud::server::S3Config;
use leptoskit_cloud::server::SecretValue;
use leptoskit_cloud::server::SharedMailer;
use leptoskit_cloud::server::ShippingSettings;
use leptoskit_cloud::server::StorageConfig;
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
	/// Whether the origin is the bind address, which `--dev` uses when
	/// [`ORIGIN`] isn't set: then a bind to port 0 is reached at the port it
	/// gets.
	origin_from_bind: bool,
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
				github: oauth_app(vars, GITHUB_CLIENT_ID, GITHUB_CLIENT_SECRET)?,
				google: oauth_app(vars, GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET)?,
				default_region,
				storage,
				apps_domain,
				registry,
				catalogue,
				shipping: ShippingSettings::default(),
				dev: mailbox.map(|mailbox| DevMode { mailbox }),
			},
			site: read(vars, SITE).map(std::path::PathBuf::from),
			origin_from_bind,
		})
	}
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

	Ok(StorageConfig::S3(S3Config {
		endpoint,
		region: read(vars, S3_REGION).unwrap_or_else(|| "auto".to_owned()),
		sources_bucket: required(vars, S3_SOURCES_BUCKET)?,
		logs_bucket: required(vars, S3_LOGS_BUCKET)?,
		access_key_id: required(vars, S3_ACCESS_KEY_ID)?,
		secret_access_key: SecretValue::new(required(vars, S3_SECRET_ACCESS_KEY)?),
		path_style,
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
}
