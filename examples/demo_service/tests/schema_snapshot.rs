//! The generation format's schema snapshot.
//!
//! This is evidence for the fingerprint contract, not the protection
//! itself: the runtime refuses a published marker whose fingerprint
//! doesn't match before anything opens (see `generation::open_or_build`).
//! The snapshot pins what format 1's databases actually look like, so a
//! schema change shows up in review as a diff that demands a
//! `GENERATION_FORMAT` bump alongside it.

use std::path::Path;

use anyhow::Result;
use demo_service::clock;
use demo_service::config::Config;
use demo_service::generation;
use demo_service::marker::GENERATION_FORMAT;
use demo_service::shutdown;
use rusqlite::OpenFlags;
use sha2::Digest as _;

/// Builds a fresh generation in a temporary volume and snapshots the
/// format with a sha256 of each database's `sqlite_master` SQL plus the
/// admin's schema snapshot file.
#[tokio::test]
async fn format_one_is_pinned() -> Result<()> {
	let root = tempfile::tempdir()?.keep();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
	}

	let config = Config {
		bind: "127.0.0.1:0".parse().expect("a bind address"),
		volume: root.clone(),
		site_origin: "http://localhost".to_owned(),
		admin_origin: "http://127.0.0.1".to_owned(),
		authorities: test_authorities(),
	};
	let terminated = shutdown::init();
	let readiness = leptoskit_axum::Readiness::new();

	let day = clock::day_of(clock::now_millis());
	let built =
		generation::open_or_build(&config, &day, clock::now_millis(), &terminated, &readiness)
			.await
			.expect("a fresh generation builds in process");
	let dir = built.dir.clone();
	drop(built);

	let fingerprint = demo_service::marker::Published::current_fingerprint()?;
	let snapshot = serde_json::json!({
		"format": GENERATION_FORMAT,
		"fingerprint": fingerprint,
		"site_sqlite_master": sqlite_master(&dir.join("site.sqlite3"))?,
		"analytics_sqlite_master": sqlite_master(&dir.join("analytics.sqlite3"))?,
		"console_sqlite_master": sqlite_master(&dir.join("console.sqlite3"))?,
		"admin_sqlite_master": sqlite_master(&demo_service::marker::admin_database(&dir)
			.expect("the admin's database is present"))?,
		"admin_schema_json": file_sha256(&dir.join("admin/admin.schema.json"))?,
	});

	insta::assert_json_snapshot!("generation_format", snapshot);
	Ok(())
}

/// The `sqlite_master` SQL of a database, read-only, ordered by object
/// name.
fn sqlite_master(path: &Path) -> Result<serde_json::Value> {
	let connection = rusqlite::Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
	let mut statement = connection.prepare(
		"SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, \
		 name",
	)?;
	let rows = statement.query_map([], |row| {
		Ok(serde_json::json!({
			"type": row.get::<_, String>(0)?,
			"name": row.get::<_, String>(1)?,
			"sql_sha256": row.get::<_, Option<String>>(2)?.map(|sql| {
				format!("{:x}", sha2::Sha256::digest(sql.as_bytes()))
			}),
		}))
	})?;
	let objects: Vec<serde_json::Value> = rows.flatten().collect();
	Ok(serde_json::json!({
		"objects": objects.len(),
		"sql_sha256": format!(
			"{:x}",
			sha2::Sha256::digest(
				objects
					.iter()
					.map(|object| object["sql_sha256"].as_str().unwrap_or_default())
					.collect::<Vec<_>>()
					.join("\n")
					.as_bytes(),
			)
		),
		"names": objects
			.iter()
			.map(|object| object["name"].clone())
			.collect::<Vec<_>>(),
	}))
}

fn file_sha256(path: &Path) -> Result<String> {
	let bytes = std::fs::read(path)?;
	Ok(format!("{:x}", sha2::Sha256::digest(&bytes)))
}

fn test_authorities() -> demo_service::config::Authorities {
	let authority = |host: &str| {
		demo_service::config::Authority::of_host_header(host).expect("a test authority")
	};
	demo_service::config::Authorities {
		site: authority("localhost"),
		admin: authority("127.0.0.1"),
		// The header grammar refuses port 0; this test never dispatches
		// on the bind, so the port only has to be a valid one.
		bind: authority("127.0.0.1:8080"),
	}
}
