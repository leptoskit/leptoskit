//! An environment's managed database on a private, throwaway Postgres,
//! made and dropped as a cluster's worker does.
//!
//! Without Postgres binaries the tests are skipped, unless
//! `LEPTOSKIT_REQUIRE_POSTGRES` is set.

#![cfg(unix)]

#[path = "../../leptoskit_cloud/tests/support/postgres.rs"]
mod server;

use leptoskit_cloud::size::Backups;
use leptoskit_cloud::size::DatabaseSize;
use leptoskit_cloud::workers::DatabaseStep;
use leptoskit_cloud_worker::Postgres;
use leptoskit_cloud_worker::cluster::Databases as _;

const NAME: &str = "env_0123456789abcdef0123456789abcdef";

/// The administrator's address on `server`, and a client for it.
async fn administrator(server: &server::Postgres) -> (String, tokio_postgres::Client) {
	let mut address = url::Url::parse(&server.database()).expect("an address");
	address.set_path("/postgres");
	address.set_query(Some("sslmode=disable"));
	let address = address.to_string();
	let (client, connection) = tokio_postgres::connect(&address, tokio_postgres::NoTls)
		.await
		.expect("connects");
	tokio::spawn(connection);
	(address, client)
}

fn step(name: &str) -> DatabaseStep {
	DatabaseStep {
		name: name.to_owned(),
		role: name.to_owned(),
		size: DatabaseSize::Shared,
		backups: Backups::default(),
	}
}

#[tokio::test]
async fn a_database_is_made_kept_to_its_role_and_dropped() {
	let Some(server) = server::Postgres::start() else {
		return;
	};
	let (address, admin) = administrator(&server).await;
	let postgres = Postgres::new(&address).expect("an address");

	postgres
		.ensure(&step(NAME), "first-password")
		.await
		.expect("made");
	// Again, with a new password: nothing else changes.
	postgres
		.ensure(&step(NAME), "second'password")
		.await
		.expect("made again");

	let role = admin
		.query_one(
			"SELECT rolcanlogin, rolsuper, rolcreatedb, rolcreaterole, rolreplication, \
			 rolbypassrls, rolconnlimit FROM pg_roles WHERE rolname = $1",
			&[&NAME],
		)
		.await
		.expect("the role");
	assert!(role.get::<_, bool>(0), "logs in");
	for column in 1..=5 {
		assert!(!role.get::<_, bool>(column), "attribute {column}");
	}
	assert_eq!(
		role.get::<_, i32>(6),
		i32::try_from(leptoskit_cloud_worker::postgres::CONNECTION_LIMIT).expect("small")
	);

	let owner: String = admin
		.query_one(
			"SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname = $1",
			&[&NAME],
		)
		.await
		.expect("the database")
		.get(0);
	assert_eq!(owner, NAME);

	// Only its role connects to it, and it connects to nothing every
	// later database is copied from.
	for (database, allowed) in [(NAME, true), ("template1", false), ("postgres", false)] {
		let may: bool = admin
			.query_one(
				"SELECT has_database_privilege($1, $2, 'CONNECT')",
				&[&NAME, &database],
			)
			.await
			.expect("asks")
			.get(0);
		assert_eq!(may, allowed, "{database}");
	}
	let public: bool = admin
		.query_one(
			"SELECT bool_or(acl::text LIKE '=%') FROM pg_database, unnest(datacl) AS acl WHERE \
			 datname = $1",
			&[&NAME],
		)
		.await
		.expect("asks")
		.get::<_, Option<bool>>(0)
		.unwrap_or(false);
	assert!(!public, "PUBLIC may not connect");

	postgres.remove(NAME).await.expect("dropped");
	postgres
		.remove(NAME)
		.await
		.expect("dropping again is nothing");
	let left = admin
		.query(
			"SELECT 1 FROM pg_database WHERE datname = $1 UNION ALL SELECT 1 FROM pg_roles WHERE \
			 rolname = $1",
			&[&NAME],
		)
		.await
		.expect("asks");
	assert!(left.is_empty());
}

#[tokio::test]
async fn only_an_environments_names_are_touched() {
	let Some(server) = server::Postgres::start() else {
		return;
	};
	let (address, admin) = administrator(&server).await;
	let postgres = Postgres::new(&address).expect("an address");

	for name in [
		"postgres",
		"env_x",
		"ENV_0123456789ABCDEF0123456789ABCDEF",
		"env_0123\"; DROP",
	] {
		assert_eq!(
			postgres.ensure(&step(name), "password").await,
			Err("its name isn't an environment's".to_owned()),
			"{name}"
		);
		assert_eq!(
			postgres.remove(name).await,
			Err("its name isn't an environment's".to_owned()),
			"{name}"
		);
	}

	let superuser: bool = admin
		.query_one(
			"SELECT rolsuper FROM pg_roles WHERE rolname = 'postgres'",
			&[],
		)
		.await
		.expect("asks")
		.get(0);
	assert!(superuser);
}
