//! Migration behaviour tests: a version-one journal upgrades in place.

use std::path::Path;

use tempfile::TempDir;

use super::*;

fn version_one_journal(database: &Path) {
	let db = rusqlite::Connection::open(database).unwrap();
	db.execute_batch(
		"CREATE TABLE _lk_migrations(version INTEGER PRIMARY KEY, checksum TEXT NOT NULL)",
	)
	.unwrap();
	let (_, up, _) = MIGRATIONS[0];
	db.execute_batch(up).unwrap();
	db.execute("INSERT INTO _lk_migrations VALUES(1, ?1)", [checksum(up)])
		.unwrap();
	db.execute(
		"INSERT INTO projects(id,environment,root,revision) VALUES('notes','production','/tmp',0)",
		[],
	)
	.unwrap();
	db.execute(
		"INSERT INTO \
		 operations(id,project,idempotency,request,revision,phase,created,updated,message,result) \
		 VALUES('op-1','notes','key','{\"expected_revision\":0,\"idempotency_key\":\"key\",\"\
		 action\":{\"kind\":\"status\"}}',1,'succeeded',1,1,'done','null')",
		[],
	)
	.unwrap();
}

#[test]
fn a_version_one_journal_upgrades_in_place_and_keeps_its_history() {
	let root = TempDir::new().unwrap();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
	}
	let database = root.path().join("console.sqlite3");
	version_one_journal(&database);

	let store = Store::open(&database).unwrap();
	let restored = store.operation("op-1").unwrap().unwrap();
	assert_eq!(restored.actor, None, "old operations have no actor");

	let recorded = store
		.enqueue_as(
			"notes",
			&Intent {
				expected_revision: 0,
				idempotency_key: "after-upgrade".into(),
				action: Action::Status,
			},
			Some("admin@example.com"),
		)
		.unwrap();
	assert_eq!(recorded.actor.as_deref(), Some("admin@example.com"));

	let backup = root.path().join("before-rollback.sqlite3");
	store.rollback(&backup).unwrap();
	assert!(
		!Store::open(&backup)
			.unwrap()
			.operations()
			.unwrap()
			.is_empty(),
		"the rollback backup stays readable"
	);
	let rolled_back = Store::open(&database).unwrap();
	assert!(rolled_back.projects().unwrap().is_empty());
}

/// The schema fingerprint is stable for one build and moves with its
/// migration list: an added migration, a changed version, and changed SQL
/// all alter it, and the framing keeps distinct lists from colliding.
#[test]
fn the_schema_fingerprint_tracks_the_migrations() {
	let current = Store::schema_fingerprint();
	assert_eq!(
		Store::schema_fingerprint(),
		current,
		"one build, one fingerprint"
	);
	assert_eq!(current.len(), 64, "a sha256 hex digest");

	// An added migration changes the fingerprint.
	let mut extended: Vec<(u32, &str, &str)> = MIGRATIONS.to_vec();
	extended.push((3, "CREATE TABLE added(x);", "DROP TABLE added;"));
	assert_ne!(
		migration_fingerprint(&extended),
		current,
		"a new migration must move the fingerprint"
	);

	// Changed up SQL changes it.
	let (_, up, down) = MIGRATIONS[0];
	let changed: Vec<(u32, &str, &str)> = std::iter::once((
		MIGRATIONS[0].0,
		"CREATE TABLE projects(id TEXT PRIMARY KEY);",
		down,
	))
	.chain(MIGRATIONS[1..].iter().copied())
	.collect();
	assert_ne!(
		migration_fingerprint(&changed),
		current,
		"changed migration SQL must move the fingerprint"
	);

	// Changed down SQL changes it too: rollback shape is schema too.
	let changed_down: Vec<(u32, &str, &str)> =
		std::iter::once((MIGRATIONS[0].0, up, "DROP TABLE everything;"))
			.chain(MIGRATIONS[1..].iter().copied())
			.collect();
	assert_ne!(
		migration_fingerprint(&changed_down),
		current,
		"changed rollback SQL must move the fingerprint"
	);

	// Changed version numbers change it.
	let reversioned: Vec<(u32, &str, &str)> = std::iter::once((MIGRATIONS[0].0 + 1, up, down))
		.chain(MIGRATIONS[1..].iter().copied())
		.collect();
	assert_ne!(
		migration_fingerprint(&reversioned),
		current,
		"a changed version must move the fingerprint"
	);

	// The length framing is unambiguous: two lists whose naive
	// concatenation is the same bytes hash differently, because the
	// boundaries differ.
	let framed: Vec<(u32, &str, &str)> = vec![(1, "AB", "")];
	let shifted: Vec<(u32, &str, &str)> = vec![(1, "A", "B")];
	assert_ne!(
		migration_fingerprint(&framed),
		migration_fingerprint(&shifted),
		"the framing must make boundary shifts distinguishable"
	);
}
