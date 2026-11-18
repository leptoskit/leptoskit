//! An environment's managed Postgres database.
//!
//! An app whose `[db]` is Postgres gets one database per environment, made
//! the first time the environment is deployed: a database and a login role
//! of its own on its cluster's shared Postgres server. Its processes start
//! with `DATABASE_URL`, which holds the role's password, so it travels as a
//! secret does: named in the release's spec, and given by value only to the
//! cluster worker that claims the release.
//!
//! Nothing here answers the password or the address. The database stays
//! with the environment, whatever later deployments do, and goes when the
//! environment is deleted.

use leptoskit_db::Timestamp;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::size::Backups;
use crate::size::DatabaseSize;

/// An environment's database.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseSummary {
	/// The database's name on its server, which is its role's name too.
	pub name: String,
	pub size: DatabaseSize,
	/// What `[cloud.database] backups` asks for. Nothing takes backups
	/// yet.
	pub backups: Backups,
	/// Whether a release that uses it has been applied: until one has, the
	/// database may not exist on the server yet.
	pub ready: bool,
	pub created_at: Timestamp,
}

/// An environment's managed Postgres database, or `None` when it has none:
/// its app has no `[db]`, keeps SQLite, or hasn't been deployed.
///
/// # Errors
///
/// `NotFound` for an environment the caller can't see.
#[rpc::api(scope = crate::OAuthScope::Cloud, tag = "databases")]
#[rpc::query]
pub async fn database(
	cx: &rpc::QueryCx,
	organization: String,
	project: String,
	environment: String,
) -> rpc::Result<Option<DatabaseSummary>> {
	crate::server::databases::database(cx, &organization, &project, &environment).await
}
