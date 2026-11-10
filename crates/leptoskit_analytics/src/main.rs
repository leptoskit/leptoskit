//! The analytics service: it opens and migrates its own database, starts
//! the batch writer, and serves under `/_leptoskit/services/analytics`.
//!
//! `lk dev` and `lk serve` start it with everything it reads from the
//! environment: its address, its database, its token, and the app's origin.

use leptoskit_analytics::SERVICE;
use leptoskit_analytics::service;
use leptoskit_axum::axum::Router;
use leptoskit_axum::service::ServiceEnv;
use leptoskit_db::Db;
use leptoskit_db::PoolOptions;

leptoskit_axum::service::main!(leptoskit_analytics, run);

type Error = Box<dyn std::error::Error + Send + Sync>;

async fn run(env: ServiceEnv) -> Result<Router, Error> {
	env.expect_name(SERVICE)?;

	let url = env
		.database_url()
		.ok_or("the analytics service needs a database: set `database = true`")?;

	let db = match env.database_pool() {
		Some(max_connections) => Db::connect_with(url, PoolOptions { max_connections }).await?,
		None => Db::connect(url).await?,
	};

	#[cfg(feature = "postgres")]
	let migrations = if url.starts_with("postgres") {
		service::POSTGRES_MIGRATIONS
	} else {
		service::MIGRATIONS
	};

	#[cfg(not(feature = "postgres"))]
	let migrations = if url.starts_with("postgres") {
		return Err(
			"this build has no Postgres support: add `postgres` to the service's features".into(),
		);
	} else {
		service::MIGRATIONS
	};

	for name in db.migrate(migrations).await? {
		println!("applied migration {name}");
	}

	Ok(service::routes(
		db,
		env.public_origin(),
		service::IngestConfig::default(),
	))
}
