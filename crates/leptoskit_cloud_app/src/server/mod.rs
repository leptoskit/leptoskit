//! `leptoskit-cloud-control`'s commands.
//!
//! `serve` runs the control plane, `migrate` brings its database up to
//! date, and `operator` grants, revokes, and lists the people who run the
//! cloud. Every command reads its configuration from the environment
//! ([`config`]), and refuses to start when it is incomplete or unsafe.

pub mod config;
#[cfg(feature = "ssr")]
pub mod console;
mod pages;

use std::net::SocketAddr;

use leptoskit_axum::GracefulShutdown;
use leptoskit_axum::Readiness;
use leptoskit_axum::axum::Router;
use leptoskit_axum::axum::middleware::map_response;
use leptoskit_cloud::server::Control;
use leptoskit_db::Db;
use leptoskit_db::Migration;

use self::config::ConfigError;
use self::config::DatabaseUrl;
use self::config::ServeConfig;
use self::config::Vars;

/// The commands.
#[derive(Debug, clap::Parser)]
#[command(
	name = "leptoskit-cloud-control",
	version,
	about = "LeptosKit Cloud's control plane."
)]
pub enum Command {
	/// Serves the control plane.
	Serve {
		/// Development: sign in to any address without a code, keep mail
		/// in memory (listed at `/_leptoskit/mail`), make a signing key for
		/// this run, and migrate on start. Listens on, and is reached at,
		/// this machine only. Development builds only.
		#[arg(long)]
		dev: bool,
	},
	/// Applies the database's pending migrations.
	Migrate,
	/// Manages operators: the people who run the cloud.
	#[command(subcommand)]
	Operator(OperatorCommand),
}

/// `operator`'s commands. A person signs in once before they can be made an
/// operator: the address must be one an account has verified.
#[derive(Debug, clap::Subcommand)]
pub enum OperatorCommand {
	/// Makes the person who verified `email` an operator.
	Grant {
		/// Their verified address.
		email: String,
	},
	/// Stops the person who verified `email` being an operator.
	Revoke {
		/// Their verified address.
		email: String,
	},
	/// Lists operators, by address.
	List,
}

/// Why a command failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// The environment is incomplete or unsafe.
	#[error(transparent)]
	Config(#[from] ConfigError),
	/// The database couldn't be opened. The message never repeats its URL,
	/// which may hold a password.
	#[error("couldn't open the database named by {}", config::DATABASE_URL)]
	Connect,
	/// The database failed.
	#[error("database: {0}")]
	Db(#[from] leptoskit_db::DbError),
	/// A migration failed, or the database's history doesn't match this
	/// build's.
	#[error("migrations: {0}")]
	Migration(#[from] leptoskit_db::MigrationError),
	/// The database has migrations to apply first.
	#[error(
		"the database has {0} pending migration(s): run `leptoskit-cloud-control migrate` first"
	)]
	Pending(usize),
	/// The database has had migrations this build doesn't know: it is newer
	/// than this build.
	#[error(
		"the database has migrations this build doesn't know, such as `{0}`: run a newer build"
	)]
	Unknown(String),
	/// The control plane didn't start.
	#[error(transparent)]
	Control(#[from] leptoskit_cloud::server::ControlError),
	/// An operator command failed.
	#[error(transparent)]
	Operator(#[from] leptoskit_cloud::server::OperatorError),
	/// The server couldn't listen, or stopped serving.
	#[error("serve: {0}")]
	Serve(String),
}

/// Runs `command`, reading the environment through `vars`.
///
/// # Errors
///
/// [`Error`], which never repeats a secret.
pub async fn run(command: Command, vars: Vars<'_>) -> Result<(), Error> {
	match command {
		Command::Serve { dev } => serve(ServeConfig::from_vars(vars, dev)?).await,
		Command::Migrate => {
			let database = DatabaseUrl::from_vars(vars)?;
			let db = open(&database).await?;
			let applied = db.migrate(migrations(&database)).await?;

			if applied.is_empty() {
				println!("the database is up to date");
			}

			for name in applied {
				println!("applied {name}");
			}

			if let Some(email) = config::bootstrap_operator_email(vars)?
				&& leptoskit_cloud::server::bootstrap_needed(&db).await?
			{
				leptoskit_cloud::server::bootstrap_operator(&db, &email).await?;
				println!("bootstrapped operator {email}");
			}

			Ok(())
		}
		Command::Operator(command) => {
			let database = DatabaseUrl::from_vars(vars)?;
			let db = open(&database).await?;
			ready(&db, &database).await?;
			operator(&db, command, vars).await
		}
	}
}

async fn operator(db: &Db, command: OperatorCommand, vars: Vars<'_>) -> Result<(), Error> {
	use leptoskit_cloud::server::grant_operator;
	use leptoskit_cloud::server::operators;
	use leptoskit_cloud::server::revoke_operator;

	match command {
		OperatorCommand::Grant { email } => {
			grant_operator(db, &email).await?;
			println!("{email} is an operator");
		}
		OperatorCommand::Revoke { email } => {
			if revoke_operator(db, &email).await? {
				println!("{email} is no longer an operator");

				// A variable that isn't an address names nobody: `migrate` and
				// `serve` refuse it.
				let configured = matches!(
					config::bootstrap_operator_email(vars),
					Ok(Some(configured)) if configured == email.trim().to_ascii_lowercase()
				);
				if configured && operators(db).await?.is_empty() {
					println!(
						"{email} is the address {} names, and no operator remains: the next start \
						 doesn't promote it again",
						config::BOOTSTRAP_OPERATOR_EMAIL
					);
				}
			} else {
				println!("{email} wasn't an operator");
			}
		}
		OperatorCommand::List => {
			for email in operators(db).await? {
				println!("{email}");
			}
		}
	}

	Ok(())
}

async fn serve(config: ServeConfig) -> Result<(), Error> {
	let db = open(&config.database).await?;

	if config.control.dev.is_some() {
		db.migrate(migrations(&config.database)).await?;
	} else {
		ready(&db, &config.database).await?;
	}

	if let Some(email) = &config.bootstrap_operator_email
		&& leptoskit_cloud::server::bootstrap_needed(&db).await?
	{
		leptoskit_cloud::server::bootstrap_operator(&db, email).await?;
	}

	let listener = tokio::net::TcpListener::bind(config.bind)
		.await
		.map_err(|error| Error::Serve(format!("couldn't listen on {}: {error}", config.bind)))?;
	let address = listener
		.local_addr()
		.map_err(|error| Error::Serve(error.to_string()))?;
	let dev = config.control.dev.is_some();
	#[cfg(feature = "ssr")]
	let site = config.site.clone();
	let control = Control::start(&db, config.control_for(address)).await?;
	let readiness = Readiness::new();
	let own = Router::new().merge(readiness.routes());
	#[cfg(feature = "ssr")]
	let own = own.merge(console::routes(site));
	let router = router(&control, own);
	let live = control.live().clone();
	let shutdown = GracefulShutdown::new()
		.readiness(&readiness)
		.signal(async move {
			leptoskit_axum::shutdown_signal().await;
			live.drain();
		});
	announce(address, dev);
	leptoskit_axum::serve_listener_with(listener, router, shutdown)
		.await
		.map_err(|error| Error::Serve(error.to_string()))
}

/// Everything the binary answers: the control plane's routes, then `own`,
/// the routes this binary adds, for whatever those don't claim.
///
/// The OAuth provider serves discovery, tokens and the device flow from its
/// layer rather than from routes, so they answer only through a fallback
/// inside that layer. `Control::start` builds its routes and layers them
/// last, sign-in inside the provider; `own` is built in full and layered
/// the same way, last, and nothing is merged afterwards. Merging into a
/// layered router instead would swap its fallback for the merged router's
/// unlayered one, and those endpoints would answer 404.
///
/// The outer layer is the one every page passes, including the control
/// plane's own: no answer may be framed, whichever layer rendered it.
pub fn router(control: &Control, own: Router) -> Router {
	let own = own
		.layer(control.auth().layer())
		.layer(control.idp().layer());
	control
		.router()
		.fallback_service(own)
		.layer(map_response(pages::refuse_framing))
}

/// Says where the server listens: the one line a supervisor, or a test,
/// waits for.
fn announce(address: SocketAddr, dev: bool) {
	let mode = if dev { " (development)" } else { "" };
	println!("listening on {address}{mode}");
}

/// Opens the database without ever repeating its URL.
async fn open(database: &DatabaseUrl) -> Result<Db, Error> {
	Db::connect(database.as_str())
		.await
		.map_err(|_| Error::Connect)
}

/// This build's migrations for the database `database` names.
fn migrations(database: &DatabaseUrl) -> &'static [Migration] {
	#[cfg(feature = "postgres")]
	if database.is_postgres() {
		return leptoskit_cloud::POSTGRES_MIGRATIONS;
	}

	let _ = database;
	leptoskit_cloud::MIGRATIONS
}

/// Refuses a database that isn't exactly at this build's migrations.
async fn ready(db: &Db, database: &DatabaseUrl) -> Result<(), Error> {
	let applied = db.applied_migrations().await?;
	let known = migrations(database);

	if let Some(unknown) = applied.iter().find(|name| {
		!known
			.iter()
			.any(|migration| migration.name == name.as_str())
	}) {
		return Err(Error::Unknown(unknown.clone()));
	}

	let pending = known
		.iter()
		.filter(|migration| !applied.iter().any(|name| *name == migration.name))
		.count();

	if pending > 0 {
		return Err(Error::Pending(pending));
	}

	Ok(())
}
