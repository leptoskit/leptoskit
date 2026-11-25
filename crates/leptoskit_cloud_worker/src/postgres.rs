//! Environments' managed databases on the cluster's shared Postgres
//! server, as [`DatabaseStep`] sets out.
//!
//! The worker connects as an administrator the operator names. Every name
//! is quoted as an identifier and every literal with `postgres-protocol`'s
//! escaping, never spliced in as it came, and a role's password is sent
//! only as a SCRAM verifier made here: the server, its logs, and the
//! network never see it in plain text.
//!
//! Each step checks that it took: a `REVOKE` the server answers with only a
//! warning, such as on a managed server where the administrator isn't the
//! owner, fails the step instead of passing as done.

use std::sync::Arc;
use std::time::Duration;

use leptoskit_cloud::workers::DatabaseStep;
use postgres_protocol::escape::escape_identifier;
use postgres_protocol::escape::escape_literal;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject as _;
use tokio::sync::Mutex;

use crate::cluster::Databases;
use crate::cluster::Pending;

/// How many connections an environment's role may hold at once.
pub const CONNECTION_LIMIT: u32 = 20;

/// How long one statement, or connecting, may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The databases, other than the environments' own and hosted
/// notifications ones, that are still open to
/// every role: ones that take connections and that `PUBLIC` may connect
/// to or make temporary tables in. A database with no ACL at all has the
/// defaults, which let `PUBLIC` do both.
const STILL_OPEN: &str = "SELECT datname FROM pg_database AS d WHERE datallowconn AND datname !~ \
                          '^(env|ntf)_[0-9a-f]{32}$' AND (datacl IS NULL OR EXISTS (SELECT 1 FROM \
                          aclexplode(d.datacl) AS acl WHERE acl.grantee = 0 AND \
                          acl.privilege_type IN ('CONNECT', 'TEMPORARY'))) ORDER BY datname";

/// The cluster's Postgres server, reached as an administrator.
pub struct Postgres {
	config: tokio_postgres::Config,
	tls: bool,
	/// Certificates trusted besides the webpki roots, for a server with a
	/// private authority.
	authorities: Vec<CertificateDer<'static>>,
	/// How many connections each environment's role may hold at once.
	connection_limit: u32,
	/// Whether to tolerate system databases (`postgres`, `template1`, etc.)
	/// that the administrator role does not own (for example on external
	/// managed Postgres providers without superuser access). Tenant
	/// databases (`env_*` and `ntf_*`) are still strictly locked down.
	allow_unowned_system_databases: bool,
	/// Whether the server's other databases are closed to everyone yet:
	/// once per worker is enough, and doing it again is harmless.
	prepared: Arc<Mutex<bool>>,
}

impl std::fmt::Debug for Postgres {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("Postgres")
			.field("tls", &self.tls)
			.field("authorities", &self.authorities.len())
			.field("connection_limit", &self.connection_limit)
			.field(
				"allow_unowned_system_databases",
				&self.allow_unowned_system_databases,
			)
			.finish_non_exhaustive()
	}
}

/// Why the administrator's address can't be used.
#[derive(Debug, thiserror::Error)]
#[error(
	"the Postgres administrator's address isn't one: it must be `postgres://…`, with `sslmode` \
	 `disable`, `prefer`, or `require` if at all"
)]
pub struct AddressError;

/// Why the certificates given for the server can't be used.
#[derive(Debug, thiserror::Error)]
#[error("the Postgres server's certificate authority isn't a PEM file of certificates")]
pub struct AuthorityError;

impl Postgres {
	/// The server at `address`, a `postgres://` URL with an administrator's
	/// name and password.
	///
	/// `sslmode=disable` connects without TLS, which is for a server on the
	/// same machine only. Any other mode, or none, requires TLS with a
	/// certificate the webpki roots, or an authority given to
	/// [`trust`](Self::trust), vouch for: `prefer` never falls back to
	/// plain text here.
	///
	/// # Errors
	///
	/// [`AddressError`] when `address` isn't a Postgres URL.
	pub fn new(address: &str) -> Result<Self, AddressError> {
		let mut config: tokio_postgres::Config = address.parse().map_err(|_| AddressError)?;
		let tls = config.get_ssl_mode() != tokio_postgres::config::SslMode::Disable;

		if tls {
			config.ssl_mode(tokio_postgres::config::SslMode::Require);
		} else if !all_hosts_loopback(&config) {
			return Err(AddressError);
		}

		config.connect_timeout(TIMEOUT);
		Ok(Self {
			config,
			tls,
			authorities: Vec::new(),
			connection_limit: CONNECTION_LIMIT,
			allow_unowned_system_databases: false,
			prepared: Arc::default(),
		})
	}

	/// Sets how many concurrent connections each environment's role may open
	/// (clamped to `1..=1000`; defaults to [`CONNECTION_LIMIT`]).
	#[must_use]
	pub fn connection_limit(mut self, limit: u32) -> Self {
		self.connection_limit = limit.clamp(1, 1000);
		self
	}

	/// Allows startup preparation to warn instead of failing when the
	/// administrator role does not own pre-existing system databases such as
	/// `postgres` or `template1`. Tenant databases remain strictly closed to
	/// `PUBLIC`.
	#[must_use]
	pub fn allow_unowned_system_databases(mut self, allow: bool) -> Self {
		self.allow_unowned_system_databases = allow;
		self
	}

	/// Also trusts the certificates in `pem` for the server's: for a server
	/// whose certificate a private authority signed.
	///
	/// # Errors
	///
	/// [`AuthorityError`] when `pem` holds no certificate, or one that
	/// can't be read.
	pub fn trust(mut self, pem: &[u8]) -> Result<Self, AuthorityError> {
		let certificates = CertificateDer::pem_slice_iter(pem)
			.collect::<Result<Vec<_>, _>>()
			.map_err(|_| AuthorityError)?;

		if certificates.is_empty() {
			return Err(AuthorityError);
		}

		self.authorities.extend(certificates);
		Ok(self)
	}

	async fn connect(&self) -> Result<tokio_postgres::Client, String> {
		let client = if self.tls {
			let (client, connection) = self
				.config
				.connect(tls(&self.authorities)?)
				.await
				.map_err(|error| describe_connect_error(&error, true))?;
			tokio::spawn(connection);
			client
		} else {
			let (client, connection) = self
				.config
				.connect(tokio_postgres::NoTls)
				.await
				.map_err(|error| describe_connect_error(&error, false))?;
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

	/// Closes every database that isn't an environment's, `template1` and
	/// `postgres` among them, to everyone the administrator didn't let in
	/// by name, and checks that it took.
	async fn prepare(&self, client: &tokio_postgres::Client) -> Result<(), String> {
		let mut prepared = self.prepared.lock().await;

		if *prepared {
			return Ok(());
		}

		for row in client
			.query(STILL_OPEN, &[])
			.await
			.map_err(|error| said(&error))?
		{
			let name: String = row.get(0);
			let result = client
				.batch_execute(&format!(
					"REVOKE CONNECT, TEMPORARY ON DATABASE {} FROM PUBLIC",
					escape_identifier(&name)
				))
				.await;

			if let Err(error) = result {
				let insufficient_privilege =
					error.code().is_some_and(|code| code.code() == "42501");
				if self.allow_unowned_system_databases
					&& insufficient_privilege
					&& is_allowed_unowned_system_db(&name)
				{
					eprintln!(
						"leptoskit-cloud-worker: warning: administrator cannot revoke PUBLIC on \
						 unowned system database `{name}` (42501); continuing because \
						 --postgres-allow-unowned-system-dbs is enabled"
					);
					continue;
				}
				return Err(said(&error));
			}
		}

		let open: Vec<String> = client
			.query(STILL_OPEN, &[])
			.await
			.map_err(|error| said(&error))?
			.iter()
			.map(|row| row.get(0))
			.collect();

		if !open.is_empty() {
			let disallowed: Vec<&str> = open
				.iter()
				.map(String::as_str)
				.filter(|name| !is_allowed_unowned_system_db(name))
				.collect();
			if self.allow_unowned_system_databases && disallowed.is_empty() {
				eprintln!(
					"leptoskit-cloud-worker: warning: system database(s) {} remain open to PUBLIC \
					 because the administrator does not own them; enable superuser access (e.g. \
					 CNPG enableSuperuserAccess: true) to harden them",
					open.join(", ")
				);
			} else {
				let blocked = if self.allow_unowned_system_databases {
					disallowed.join(", ")
				} else {
					open.join(", ")
				};
				return Err(format!(
					"the server's {blocked} database is still open to every role, and the \
					 administrator couldn't close it (grant superuser access, e.g. CNPG \
					 enableSuperuserAccess: true, or pass --postgres-allow-unowned-system-dbs for \
					 unowned postgres/template1 on managed Postgres)"
				));
			}
		}

		*prepared = true;
		Ok(())
	}

	async fn ensure_step(&self, step: &DatabaseStep, password: &str) -> Result<(), String> {
		ours(&step.name)?;

		if step.role != step.name {
			return Err("its role isn't named as its database is".to_owned());
		}

		let client = self.connect().await?;
		self.prepare(&client).await?;
		let role = escape_identifier(&step.role);
		let database = escape_identifier(&step.name);
		let verifier = escape_literal(&postgres_protocol::password::scram_sha_256(
			password.as_bytes(),
		));
		let connection_limit = self.connection_limit;
		let attributes = format!(
			"LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS NOINHERIT \
			 CONNECTION LIMIT {connection_limit} VALID UNTIL 'infinity' PASSWORD {verifier}"
		);

		let has_role = client
			.query_opt("SELECT 1 FROM pg_roles WHERE rolname = $1", &[&step.role])
			.await
			.map_err(|error| said(&error))?
			.is_some();

		if has_role {
			// Whatever it was, it is this again: no settings of its own and
			// no memberships either way.
			client
				.batch_execute(&format!("ALTER ROLE {role} WITH {attributes}"))
				.await
				.map_err(|error| format!("its role couldn't be updated: {}", said(&error)))?;
			reset_role(&client, &step.role).await?;

			// A hosted notifications tenant's password changes when a worker
			// that saw it is revoked; a session opened with the old one would
			// outlive it. The hub reconnects with the password it is given.
			if step.role.starts_with("ntf_") {
				end_sessions(&client, &step.role).await?;
			}
		} else {
			client
				.batch_execute(&format!("CREATE ROLE {role} WITH {attributes}"))
				.await
				.map_err(|error| format!("its role couldn't be made: {}", said(&error)))?;
		}

		let owner: Option<String> = client
			.query_opt(
				"SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname = $1",
				&[&step.name],
			)
			.await
			.map_err(|error| said(&error))?
			.map(|row| row.get(0));

		match owner {
			None => {
				client
					.batch_execute(&format!(
						"CREATE DATABASE {database} OWNER {role} TEMPLATE template0 \
						 ALLOW_CONNECTIONS false"
					))
					.await
					.map_err(|error| said(&error))?;
			}
			// Made by an earlier run of this step.
			Some(owner) if owner == step.role => {}
			// Never adopted: what is in it isn't known.
			Some(_) => {
				return Err("its database is there already, owned by another role".to_owned());
			}
		}

		client
			.batch_execute(&format!(
				"ALTER DATABASE {database} RESET ALL; REVOKE ALL ON DATABASE {database} FROM \
				 PUBLIC"
			))
			.await
			.map_err(|error| said(&error))?;

		let open = client
			.query_one(
				"SELECT EXISTS (SELECT 1 FROM pg_database AS d, aclexplode(d.datacl) AS acl WHERE \
				 d.datname = $1 AND acl.grantee = 0) OR (SELECT datacl IS NULL FROM pg_database \
				 WHERE datname = $1)",
				&[&step.name],
			)
			.await
			.map_err(|error| said(&error))?
			.get::<_, bool>(0);

		if open {
			return Err("its database is still open to every role".to_owned());
		}

		// Made closed to connections, so no role reached it before `PUBLIC`
		// was shut out; a step that failed before this leaves it closed.
		client
			.batch_execute(&format!(
				"ALTER DATABASE {database} WITH ALLOW_CONNECTIONS true"
			))
			.await
			.map_err(|error| said(&error))?;

		Ok(())
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

/// Clears `role`'s settings, everywhere, and every membership it has or
/// gives, then checks that none is left.
async fn reset_role(client: &tokio_postgres::Client, name: &str) -> Result<(), String> {
	let role = escape_identifier(name);
	let mut statements = vec![format!("ALTER ROLE {role} RESET ALL")];

	for row in client
		.query(
			"SELECT datname FROM pg_db_role_setting JOIN pg_roles ON pg_roles.oid = setrole JOIN \
			 pg_database ON pg_database.oid = setdatabase WHERE rolname = $1",
			&[&name],
		)
		.await
		.map_err(|error| said(&error))?
	{
		let database: String = row.get(0);
		statements.push(format!(
			"ALTER ROLE {role} IN DATABASE {} RESET ALL",
			escape_identifier(&database)
		));
	}

	for row in client
		.query(MEMBERSHIPS, &[&name])
		.await
		.map_err(|error| said(&error))?
	{
		let (parent, child): (String, String) = (row.get(0), row.get(1));
		statements.push(format!(
			"REVOKE {} FROM {}",
			escape_identifier(&parent),
			escape_identifier(&child)
		));
	}

	for statement in statements {
		client
			.batch_execute(&statement)
			.await
			.map_err(|error| said(&error))?;
	}

	let left = client
		.query_one(
			&format!(
				"SELECT (SELECT count(*) FROM ({MEMBERSHIPS}) AS m) + (SELECT count(*) FROM \
				 pg_db_role_setting JOIN pg_roles ON pg_roles.oid = setrole WHERE rolname = $1)"
			),
			&[&name],
		)
		.await
		.map_err(|error| said(&error))?
		.get::<_, i64>(0);

	if left == 0 {
		Ok(())
	} else {
		Err(
			"its role still has settings or memberships the administrator couldn't remove"
				.to_owned(),
		)
	}
}

/// The memberships `$1` has in other roles, and other roles have in it, as
/// (parent, member).
const MEMBERSHIPS: &str = "SELECT parent.rolname, child.rolname FROM pg_auth_members AS member \
                           JOIN pg_roles AS parent ON parent.oid = member.roleid JOIN pg_roles AS \
                           child ON child.oid = member.member WHERE child.rolname = $1 OR \
                           parent.rolname = $1";

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

/// System databases that an unprivileged administrator on a managed Postgres
/// provider may legitimately be unable to `REVOKE ... FROM PUBLIC` on. Any
/// other open database (such as an operator's `app` or `cloud` database) is
/// still treated as a hard error.
const ALLOWED_UNOWNED_SYSTEM_DBS: &[&str] = &["postgres", "template1"];

fn is_allowed_unowned_system_db(name: &str) -> bool {
	ALLOWED_UNOWNED_SYSTEM_DBS.contains(&name)
}

/// Returns true only when every configured host is loopback (or a local Unix
/// domain socket), enforcing the contract that `sslmode=disable` is accepted
/// for same-machine servers only.
fn all_hosts_loopback(config: &tokio_postgres::Config) -> bool {
	let hosts = config.get_hosts();
	if hosts.is_empty() {
		return false;
	}
	hosts.iter().all(|host| {
		match host {
			tokio_postgres::config::Host::Tcp(name) => {
				let trimmed = name.trim_matches(['[', ']']);
				trimmed.eq_ignore_ascii_case("localhost")
					|| trimmed
						.parse::<std::net::IpAddr>()
						.is_ok_and(|ip| ip.is_loopback())
			}
			#[cfg(unix)]
			tokio_postgres::config::Host::Unix(_) => true,
		}
	})
}

/// Refuses a name that isn't an environment's, `env_` or, for its hosted
/// notifications, `ntf_`, and 32 lower-case hex digits: whatever the
/// control plane says, the worker never alters, or drops, the
/// administrator or anything else on the server that isn't an
/// environment's.
fn ours(name: &str) -> Result<(), String> {
	let hex = name
		.strip_prefix("env_")
		.or_else(|| name.strip_prefix("ntf_"))
		.unwrap_or_default();

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

/// What the server said, by its SQLSTATE: a statement's text is never
/// repeated.
fn said(error: &tokio_postgres::Error) -> String {
	match error.code() {
		Some(code) => format!("the server refused a statement ({})", code.code()),
		None => "the server couldn't be reached".to_owned(),
	}
}

/// Classifies a Postgres connection error without ever printing connection
/// URLs or passwords: separates TLS certificate/handshake failures, auth
/// rejections, and network reachability failures by walking the full error
/// source chain.
fn describe_connect_error(error: &tokio_postgres::Error, tls_enabled: bool) -> String {
	if let Some(code) = error.code() {
		return match code.code() {
			"28P01" | "28000" => {
				format!(
					"the Postgres administrator failed authentication ({})",
					code.code()
				)
			}
			other => format!("the Postgres server rejected the connection ({other})"),
		};
	}

	let mut chain = String::new();
	let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
	while let Some(err) = current {
		if !chain.is_empty() {
			chain.push(' ');
		}
		chain.push_str(&err.to_string().to_ascii_lowercase());
		current = err.source();
	}

	if tls_enabled
		&& (chain.contains("tls")
			|| chain.contains("certificate")
			|| chain.contains("cert")
			|| chain.contains("handshake")
			|| chain.contains("invalid peer")
			|| chain.contains("unknown issuer"))
	{
		return "TLS verification with the Postgres server failed (mount the cluster CA with \
		        --postgres-ca / LEPTOSKIT_CLOUD_WORKER_POSTGRES_CA)"
			.to_owned();
	}

	if chain.contains("timed out") || chain.contains("timeout") {
		return "connecting to the Postgres server timed out".to_owned();
	}

	"the Postgres server couldn't be reached".to_owned()
}

/// TLS with the webpki roots and `authorities`.
fn tls(
	authorities: &[CertificateDer<'static>],
) -> Result<tokio_postgres_rustls::MakeRustlsConnect, String> {
	let mut roots = rustls::RootCertStore::empty();
	roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

	for authority in authorities {
		roots
			.add(authority.clone())
			.map_err(|_| "the server's certificate authority couldn't be trusted".to_owned())?;
	}

	let config = rustls::ClientConfig::builder_with_provider(Arc::new(
		rustls::crypto::ring::default_provider(),
	))
	.with_safe_default_protocol_versions()
	.map_err(|_| "TLS couldn't be set up".to_owned())?
	.with_root_certificates(roots)
	.with_no_client_auth();

	Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}

/// Ends every session `role` has open but this one's.
async fn end_sessions(client: &tokio_postgres::Client, role: &str) -> Result<(), String> {
	client
		.execute(
			"SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = $1 AND pid <> \
			 pg_backend_pid()",
			&[&role],
		)
		.await
		.map(drop)
		.map_err(|error| {
			format!(
				"its role's open sessions couldn't be ended (the administrator needs superuser \
				 access or the pg_signal_backend role): {}",
				said(&error)
			)
		})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn tls_is_required_unless_it_is_turned_off() {
		use tokio_postgres::config::SslMode;

		for (address, mode, tls) in [
			(
				"postgres://admin:pw@db.internal/postgres",
				SslMode::Require,
				true,
			),
			(
				"postgres://admin:pw@db.internal/postgres?sslmode=prefer",
				SslMode::Require,
				true,
			),
			(
				"postgres://admin:pw@db.internal/postgres?sslmode=require",
				SslMode::Require,
				true,
			),
			(
				"postgres://admin:pw@localhost/postgres?sslmode=disable",
				SslMode::Disable,
				false,
			),
			(
				"postgres://admin:pw@127.0.0.1/postgres?sslmode=disable",
				SslMode::Disable,
				false,
			),
			(
				"postgres://admin:pw@[::1]/postgres?sslmode=disable",
				SslMode::Disable,
				false,
			),
		] {
			let postgres = Postgres::new(address).expect("an address");
			assert_eq!(postgres.config.get_ssl_mode(), mode, "{address}");
			assert_eq!(postgres.tls, tls, "{address}");
		}
	}

	#[test]
	fn sslmode_disable_rejects_remote_hosts() {
		for address in [
			"postgres://admin:pw@db.internal/postgres?sslmode=disable",
			"postgres://admin:pw@10.0.0.5/postgres?sslmode=disable",
			"postgres://admin:pw@localhost,db.internal/postgres?sslmode=disable",
		] {
			assert!(
				Postgres::new(address).is_err(),
				"expected {address} to be rejected when sslmode=disable"
			);
		}
	}

	#[test]
	fn an_authority_must_be_certificates() {
		let postgres =
			Postgres::new("postgres://admin:pw@db.internal/postgres").expect("an address");
		assert!(postgres.trust(b"not a certificate").is_err());
	}

	#[test]
	fn connection_limit_and_unowned_system_dbs_are_configurable() {
		let postgres = Postgres::new("postgres://admin:pw@db.internal/postgres")
			.expect("an address")
			.connection_limit(0)
			.allow_unowned_system_databases(true);
		assert_eq!(postgres.connection_limit, 1);
		assert!(postgres.allow_unowned_system_databases);

		let postgres = postgres.connection_limit(40);
		assert_eq!(postgres.connection_limit, 40);

		let postgres = postgres.connection_limit(9_999);
		assert_eq!(postgres.connection_limit, 1_000);
	}

	#[test]
	fn only_an_environments_databases_are_its_own() {
		let hex = "0193a5b07c1d7e2f8a9b0c1d2e3f4a5b";
		assert_eq!(ours(&format!("env_{hex}")), Ok(()));
		assert_eq!(ours(&format!("ntf_{hex}")), Ok(()));

		for name in [
			"postgres".to_owned(),
			"template1".to_owned(),
			format!("ntf_{}", hex.to_uppercase()),
			format!("ntf_{hex}0"),
			format!("abc_{hex}"),
			format!("ntf{hex}"),
			"ntf_".to_owned(),
		] {
			assert!(ours(&name).is_err(), "{name}");
		}
	}

	#[test]
	fn only_known_system_databases_are_exempt_when_unowned() {
		assert!(is_allowed_unowned_system_db("postgres"));
		assert!(is_allowed_unowned_system_db("template1"));
		assert!(!is_allowed_unowned_system_db("app"));
		assert!(!is_allowed_unowned_system_db("cloud"));
		assert!(!is_allowed_unowned_system_db(
			"env_0123456789abcdef0123456789abcdef"
		));
	}
}
