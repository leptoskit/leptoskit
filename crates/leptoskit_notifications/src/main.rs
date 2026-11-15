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
use leptoskit_rpc::server::Anonymous;
use leptoskit_rpc::server::ServiceCallers;

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

	// The push transports, from the operator's environment: none is a
	// normal way to run — the inbox is the whole service — and a
	// half-configured provider refuses to start, named in full. Routes and
	// workers share the runtime; job arguments never store it.
	let push = service::config::PushConfig::from_env()
		.and_then(|config| {
			if config.apns.is_none() && config.fcm.is_none() && config.web.is_none() {
				return Ok(None);
			}

			println!(
				"push transports: {}",
				[
					config.apns.as_ref().map(|_| "apns"),
					config.fcm.as_ref().map(|_| "fcm"),
					config.web.as_ref().map(|_| "web_push"),
				]
				.into_iter()
				.flatten()
				.collect::<Vec<_>>()
				.join(", ")
			);
			service::transport::PushRuntime::build(&config).map(Some)
		})
		.map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
	let push = service::push::Push {
		runtime: push.map(std::sync::Arc::new),
	};

	let (router, live) = service::routes_with_push(
		db.clone(),
		env.public_origin(),
		ServiceCallers::for_service(Anonymous),
		push.clone(),
	);
	live.start().await?;
	// The workers run the wake jobs a scheduled or expiring send enqueues,
	// the push deliveries' rounds, and keep the change log the live
	// queries follow pruned.
	service::workers_with_push(db, push).start();
	Ok(router)
}
