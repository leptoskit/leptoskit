use leptoskit_db::Dialect;
use uuid::Uuid;

/// The few places SQLite and Postgres spell the same literal differently,
/// for tests that seed rows with plain SQL instead of one RPC call per row.
#[derive(Clone, Copy)]
pub struct Sql(pub Dialect);

impl Sql {
	/// A UUID column's literal.
	pub fn uuid(self, id: Uuid) -> String {
		match self.0 {
			Dialect::Sqlite => format!("x'{}'", id.simple()),
			Dialect::Postgres => format!("'{id}'::uuid"),
		}
	}

	/// An expression that makes a fresh row id, once per row.
	pub fn new_id(self) -> &'static str {
		match self.0 {
			Dialect::Sqlite => "randomblob(16)",
			Dialect::Postgres => "gen_random_uuid()",
		}
	}

	/// A timestamp column's literal, from an RFC 3339 string.
	pub fn timestamp(self, rfc3339: &str) -> String {
		match self.0 {
			Dialect::Sqlite => format!("'{rfc3339}'"),
			Dialect::Postgres => format!("'{rfc3339}'::timestamptz"),
		}
	}
}
