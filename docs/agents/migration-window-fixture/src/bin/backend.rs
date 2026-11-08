use std::io::BufRead;
use std::io::Write;

use leptoskit_db::Connection;
use leptoskit_db::Db;
use leptoskit_db::Read;
use leptoskit_db::Statement;

#[cfg(not(feature = "expanded"))]
static MIGRATIONS: &[leptoskit_db::Migration] = leptoskit_db::migrations!("fixtures/old");
#[cfg(feature = "expanded")]
static MIGRATIONS: &[leptoskit_db::Migration] = leptoskit_db::migrations!("fixtures/new");
#[cfg(not(feature = "expanded"))]
static SCHEMA: Option<&str> = leptoskit_db::migration_schema!("fixtures/old");
#[cfg(feature = "expanded")]
static SCHEMA: Option<&str> = leptoskit_db::migration_schema!("fixtures/new");

fn statement(sql: String) -> Statement {
	Statement { sql, args: vec![] }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	if leptoskit_db::export_migration_contract_if_requested(MIGRATIONS, SCHEMA)? {
		return Ok(());
	}
	let db = Db::connect(&std::env::var("DATABASE_URL")?).await?;
	if std::env::args().any(|arg| arg == "--bootstrap") {
		db.migrate(MIGRATIONS).await?;
	} else {
		db.migrate_startup(MIGRATIONS, SCHEMA).await?;
	}
	println!(
		"ready:{}",
		if cfg!(feature = "expanded") {
			"new"
		} else {
			"old"
		}
	);
	std::io::stdout().flush()?;
	for line in std::io::stdin().lock().lines() {
		let line = line?;
		if line == "quit" {
			break;
		}
		if let Some(sql) = line.strip_prefix("query ") {
			println!("{:?}", db.fetch(statement(sql.into())).await?);
		} else {
			println!("{}", db.execute(statement(line)).await?);
		}
		std::io::stdout().flush()?;
	}
	Ok(())
}
