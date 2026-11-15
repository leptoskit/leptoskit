//! The notifications service: it opens and migrates its own database, and
//! serves under `/_leptoskit/services/notifications`.
//!
//! `lk dev` and `lk serve` start it with everything it reads from the
//! environment: its address, its database, its token, and the app's origin.

use leptoskit_axum::axum::Router;
use leptoskit_axum::service::ServiceEnv;
use leptoskit_db::Db;
use leptoskit_db::PoolOptions;
use leptoskit_notifications::MIGRATIONS;
#[cfg(feature = "postgres")]
use leptoskit_notifications::POSTGRES_MIGRATIONS;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::service;

leptoskit_axum::service::main!(leptoskit_notifications, run);

type Error = Box<dyn std::error::Error + Send + Sync>;

async fn run(env: ServiceEnv) -> Result<Router, Error> {
	env.expect_name(SERVICE)?;

	let url = env
		.database_url()
		.ok_or("the notifications service needs a database: set `database = true`")?;

	let db = match env.database_pool() {
		Some(max_connections) => Db::connect_with(url, PoolOptions { max_connections }).await?,
		None => Db::connect(url).await?,
	};

	#[cfg(feature = "postgres")]
	let migrations = if url.starts_with("postgres") {
		POSTGRES_MIGRATIONS
	} else {
		MIGRATIONS
	};

	#[cfg(not(feature = "postgres"))]
	let migrations = if url.starts_with("postgres") {
		return Err(
			"this build has no Postgres support: add `postgres` to the service's features".into(),
		);
	} else {
		MIGRATIONS
	};

	for name in db.migrate(migrations).await? {
		println!("applied migration {name}");
	}

	let (router, live) = service::routes(db, env.public_origin());
	live.start().await?;
	Ok(router)
}
