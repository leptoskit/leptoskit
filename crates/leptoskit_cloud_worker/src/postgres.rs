//! Environments' managed databases on the cluster's shared Postgres
//! server, as [`DatabaseStep`] sets out.
//!
//! The worker connects as an administrator the operator names. Every name
//! is quoted as an identifier and the password as a literal, with
//! `postgres-protocol`'s escaping, never spliced in as it came.

use std::sync::Arc;
use std::time::Duration;

use leptoskit_cloud::workers::DatabaseStep;
use postgres_protocol::escape::escape_identifier;
use postgres_protocol::escape::escape_literal;
use tokio::sync::Mutex;

use crate::cluster::Databases;
use crate::cluster::Pending;

/// How many connections an environment's role may hold at once.
pub const CONNECTION_LIMIT: u32 = 20;

/// How long one statement, or connecting, may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The cluster's Postgres server, reached as an administrator.
pub struct Postgres {
	config: tokio_postgres::Config,
	tls: bool,
	/// Whether `template1` and `postgres` are closed to everyone else yet:
	/// once per worker is enough, and doing it again is harmless.
	prepared: Arc<Mutex<bool>>,
}

impl std::fmt::Debug for Postgres {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Postgres")
			.field("tls", &self.tls)
			.finish_non_exhaustive()
	}
}

/// Why the administrator's address can't be used.
#[derive(Debug, thiserror::Error)]
#[error("the Postgres administrator's address isn't one: it must be `postgres://…`")]
pub struct AddressError;

impl Postgres {
	/// The server at `address`, a `postgres://` URL with an administrator's
	/// name and password. `sslmode=disable` connects without TLS, which is
	/// for a server on the same machine or network only; anything else
	/// requires TLS with a certificate the webpki roots trust.
	///
	/// # Errors
	///
	/// [`AddressError`] when `address` isn't a Postgres URL.
	pub fn new(address: &str) -> Result<Self, AddressError> {
		let config: tokio_postgres::Config = address.parse().map_err(|_| AddressError)?;
		let tls = config.get_ssl_mode() != tokio_postgres::config::SslMode::Disable;
		let mut config = config;
		config.connect_timeout(TIMEOUT);
		Ok(Self {
			config,
			tls,
			prepared: Arc::default(),
		})
	}

	async fn connect(&self) -> Result<tokio_postgres::Client, String> {
		let unreachable = |_| "the server couldn't be reached".to_owned();

		let client = if self.tls {
			let (client, connection) = self.config.connect(tls()?).await.map_err(unreachable)?;
			tokio::spawn(connection);
			client
		} else {
			let (client, connection) = self
				.config
				.connect(tokio_postgres::NoTls)
				.await
				.map_err(unreachable)?;
			tokio::spawn(connection);
			client
		};

		// No statement waits for ever on a lock a tenant's session holds.
		client
			.batch_execute(&format!(
				"SET statement_timeout = {}; SET lock_timeout = {}",
				TIMEOUT.as_millis(),
				TIMEOUT.as_millis()
			))
			.await
			.map_err(|error| said(&error))?;
		Ok(client)
	}

	/// Closes the databases every new one is copied from, and the default
	/// one, to everyone the administrator didn't let in.
	async fn prepare(&self, client: &tokio_postgres::Client) -> Result<(), String> {
		let mut prepared = self.prepared.lock().await;

		if !*prepared {
			client
				.batch_execute(
					"REVOKE CONNECT ON DATABASE template1 FROM PUBLIC; REVOKE CONNECT ON DATABASE \
					 postgres FROM PUBLIC",
				)
				.await
				.map_err(|error| said(&error))?;
			*prepared = true;
		}

		Ok(())
	}

	async fn ensure_step(&self, step: &DatabaseStep, password: &str) -> Result<(), String> {
		ours(&step.name)?;
		ours(&step.role)?;
		let client = self.connect().await?;
		self.prepare(&client).await?;
		let role = escape_identifier(&step.role);
		let database = escape_identifier(&step.name);
		let secret = escape_literal(password);

		let has_role = client
			.query_opt("SELECT 1 FROM pg_roles WHERE rolname = $1", &[&step.role])
			.await
			.map_err(|error| said(&error))?
			.is_some();
		let attributes = format!(
			"LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS NOINHERIT \
			 CONNECTION LIMIT {CONNECTION_LIMIT} PASSWORD {secret}"
		);

		if has_role {
			// Whatever it was, it is this again, and a member of nothing.
			client
				.batch_execute(&format!("ALTER ROLE {role} WITH {attributes}"))
				.await
				.map_err(|_| "its role couldn't be updated".to_owned())?;
			let memberships = client
				.query(
					"SELECT parent.rolname FROM pg_auth_members AS member JOIN pg_roles AS parent \
					 ON parent.oid = member.roleid JOIN pg_roles AS child ON child.oid = \
					 member.member WHERE child.rolname = $1",
					&[&step.role],
				)
				.await
				.map_err(|error| said(&error))?;

			for membership in memberships {
				let parent: String = membership.get(0);
				client
					.batch_execute(&format!(
						"REVOKE {} FROM {role}",
						escape_identifier(&parent)
					))
					.await
					.map_err(|error| said(&error))?;
			}
		} else {
			client
				.batch_execute(&format!("CREATE ROLE {role} WITH {attributes}"))
				.await
				.map_err(|_| "its role couldn't be made".to_owned())?;
		}

		let has_database = client
			.query_opt(
				"SELECT 1 FROM pg_database WHERE datname = $1",
				&[&step.name],
			)
			.await
			.map_err(|error| said(&error))?
			.is_some();

		if !has_database {
			client
				.batch_execute(&format!(
					"CREATE DATABASE {database} OWNER {role} TEMPLATE template0"
				))
				.await
				.map_err(|error| said(&error))?;
		}

		client
			.batch_execute(&format!(
				"ALTER DATABASE {database} OWNER TO {role}; REVOKE ALL ON DATABASE {database} \
				 FROM PUBLIC"
			))
			.await
			.map_err(|error| said(&error))
	}

	async fn remove_named(&self, name: &str) -> Result<(), String> {
		ours(name)?;
		let client = self.connect().await?;
		let quoted = escape_identifier(name);
		client
			.batch_execute(&format!("DROP DATABASE IF EXISTS {quoted} WITH (FORCE)"))
			.await
			.map_err(|error| said(&error))?;
		client
			.batch_execute(&format!("DROP ROLE IF EXISTS {quoted}"))
			.await
			.map_err(|error| said(&error))
	}
}

impl Databases for Postgres {
	fn ensure<'a>(
		&'a self,
		step: &'a DatabaseStep,
		password: &'a str,
	) -> Pending<'a, Result<(), String>> {
		Box::pin(self.ensure_step(step, password))
	}

	fn remove<'a>(&'a self, name: &'a str) -> Pending<'a, Result<(), String>> {
		Box::pin(self.remove_named(name))
	}
}

/// Refuses a name that isn't an environment's, `env_` and 32 lower-case
/// hex digits: whatever the control plane says, the worker never alters,
/// or drops, the administrator or anything else on the server.
fn ours(name: &str) -> Result<(), String> {
	let hex = name.strip_prefix("env_").unwrap_or_default();

	if hex.len() == 32
		&& hex
			.bytes()
			.all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
	{
		Ok(())
	} else {
		Err("its name isn't an environment's".to_owned())
	}
}

/// What the server said, by its SQLSTATE: a statement's text, which may
/// hold a password, is never repeated.
fn said(error: &tokio_postgres::Error) -> String {
	match error.code() {
		Some(code) => format!("the server refused a statement ({})", code.code()),
		None => "the server couldn't be reached".to_owned(),
	}
}

/// TLS with the webpki roots.
fn tls() -> Result<tokio_postgres_rustls::MakeRustlsConnect, String> {
	let mut roots = rustls::RootCertStore::empty();
	roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
	let config = rustls::ClientConfig::builder_with_provider(Arc::new(
		rustls::crypto::ring::default_provider(),
	))
	.with_safe_default_protocol_versions()
	.map_err(|_| "TLS couldn't be set up".to_owned())?
	.with_root_certificates(roots)
	.with_no_client_auth();

	Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}
