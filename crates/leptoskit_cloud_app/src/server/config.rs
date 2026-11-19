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
use leptoskit_cloud::server::ControlConfig;
use leptoskit_cloud::server::DEFAULT_REGION;
use leptoskit_cloud::server::DevMode;
use leptoskit_cloud::server::OAuthApp;
use leptoskit_cloud::server::SharedMailer;
use leptoskit_cloud::size::Region;

/// The public origin, such as `https://cloud.leptoskit.com`.
pub const ORIGIN: &str = "LEPTOSKIT_CLOUD_ORIGIN";
/// The database: `sqlite://path` or `postgres://…`.
pub const DATABASE_URL: &str = "LEPTOSKIT_CLOUD_DATABASE_URL";
/// The keys sessions and codes are signed with: `id:key` pairs, newest
/// first, each key 32 random bytes in unpadded base64url.
pub const AUTH_KEYS: &str = "LEPTOSKIT_CLOUD_AUTH_KEYS";
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
/// Where projects run when they don't say.
pub const DEFAULT_REGION_VAR: &str = "LEPTOSKIT_CLOUD_DEFAULT_REGION";

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
		let default_region =
			read(vars, DEFAULT_REGION_VAR).unwrap_or_else(|| DEFAULT_REGION.into());
		Region::parse(&default_region)
			.map_err(|error| invalid(DEFAULT_REGION_VAR, error.to_string()))?;

		Ok(Self {
			bind,
			database,
			control: ControlConfig {
				origin,
				keys,
				mailer,
				github: oauth_app(vars, GITHUB_CLIENT_ID, GITHUB_CLIENT_SECRET)?,
				google: oauth_app(vars, GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET)?,
				default_region,
				dev: mailbox.map(|mailbox| DevMode { mailbox }),
			},
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
			(SMTP_URL, SMTP),
			(MAIL_FROM, "Cloud <cloud@example.com>"),
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
	}

	#[test]
	fn nothing_secret_is_defaulted() {
		for name in [AUTH_KEYS, SMTP_URL, MAIL_FROM, ORIGIN, DATABASE_URL] {
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

		for (name, value, expected) in [
			(ORIGIN, "http://cloud.example.com", "must be https"),
			(ORIGIN, "https://cloud.example.com/path", "no path"),
			(DATABASE_URL, "mysql://x", "must start with"),
			(BIND, "everywhere", "must be an address"),
			(DEFAULT_REGION_VAR, "Mars!", "isn't a region name"),
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
}
