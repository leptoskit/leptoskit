//! Exact structural expansion evidence for applications that opt into it.

use std::collections::BTreeSet;

use leptoskit_db_schema::Schema;
use serde::Deserialize;
use serde::Serialize;

use crate::Migration;
use crate::checksum;

const ALGORITHM: &str = "leptoskit-generated-expansion-v1";
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_HISTORY: usize = 4096;
const MAX_EXPANSIONS: usize = 64;

mod startup;
pub use startup::MigrationContract;
pub use startup::export_migration_contract_if_requested;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) use startup::startup_window;

/// Invalid or inapplicable expansion evidence. No migration is run on error.
#[derive(Debug, thiserror::Error)]
pub enum MigrationWindowError {
	/// A bounded document could not be decoded or validated.
	#[error("invalid migration window: {0}")]
	Invalid(String),
	/// A snapshot transition or its SQL is not in the supported subset.
	#[error(transparent)]
	Expansion(#[from] leptoskit_db_schema::ExpansionError),
	/// This executable does not embed an exact checkpoint of the window.
	#[error("this build's migration history or schema is not a checkpoint of the window")]
	BuildMismatch,
	/// The exact document bytes do not match the operator's independent pin.
	#[error("migration window does not match the independently supplied SHA-256")]
	DigestMismatch,
	/// The database history is not an exact prefix within the window.
	#[error("database history is outside the migration window; investigate or recover explicitly")]
	HistoryMismatch,
	/// This online migration protocol currently requires PostgreSQL.
	#[error("online migration windows require PostgreSQL")]
	UnsupportedDialect,
	/// Development pushes are not evidence of a deployed schema checkpoint.
	#[error(
		"database has unrecorded development pushes; adopt or recover before an online rollout"
	)]
	PushedState,
	/// The attempt exceeded its total deadline; its outcome must be inspected.
	#[error("online migration deadline exceeded; inspect migration history before retrying")]
	Timeout,
}

/// A checked sequence of additive schema transitions, with exact SQL bytes.
///
/// Both the old and the new executable must explicitly opt into this
/// protocol and supply their own embedded schema snapshot. A window only
/// establishes structural compatibility for column-named reads and writes.
/// It does not authorize a release, prove the live database's physical
/// schema, or establish compatibility of arbitrary SQL or business logic.
/// Ordinary `Db::migrate` continues to reject unknown history.
#[derive(Clone, Debug, Serialize)]
#[serde(transparent)]
pub struct MigrationWindow(Document);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
	name: String,
	sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expansion {
	name: String,
	sql: String,
	schema: Schema,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
	format_version: u32,
	algorithm: String,
	baseline: Vec<Identity>,
	schema: Schema,
	expansions: Vec<Expansion>,
}

impl MigrationWindow {
	/// Bind a baseline to one or more generated expansion migrations.
	///
	/// Snapshots are the committed `schema.json` files. Every forward SQL
	/// checksum is checked; consolidation and custom SQL are refused. No
	/// checksum or SQL is rewritten to make a migration fit the window.
	pub fn new(
		baseline: &[Migration],
		baseline_snapshot: &str,
		expansions: impl IntoIterator<Item = (Migration, String)>,
	) -> Result<Self, MigrationWindowError> {
		let baseline = identities(baseline)?;
		let mut steps = Vec::new();
		for (migration, snapshot) in expansions {
			if steps.len() >= MAX_EXPANSIONS {
				return invalid("too many expansion migrations");
			}
			identities(std::slice::from_ref(&migration))?;
			steps.push(Expansion {
				name: migration.name.into_owned(),
				sql: migration.sql.into_owned(),
				schema: schema(&snapshot)?,
			});
		}
		let window = Self(Document {
			format_version: 1,
			algorithm: ALGORITHM.into(),
			baseline,
			schema: schema(baseline_snapshot)?,
			expansions: steps,
		});
		window.validate()?;
		Ok(window)
	}

	/// Decode a bounded document and recheck every generated SQL transition.
	pub fn from_json(json: &str) -> Result<Self, MigrationWindowError> {
		if json.len() > MAX_BYTES {
			return invalid("document exceeds 8 MiB");
		}
		let window = Self(
			serde_json::from_str(json)
				.map_err(|error| MigrationWindowError::Invalid(error.to_string()))?,
		);
		window.validate()?;
		Ok(window)
	}

	/// Decode only when the exact document bytes match an independent pin.
	///
	/// The expected digest must come from an approved release input, not from
	/// the same untrusted download as the document. This checks integrity;
	/// signatures and operator authorization are separate release policy.
	pub fn from_json_pinned(json: &str, sha256: &str) -> Result<Self, MigrationWindowError> {
		if json.len() > MAX_BYTES {
			return invalid("document exceeds 8 MiB");
		}
		if checksum(json) != sha256 {
			return Err(MigrationWindowError::DigestMismatch);
		}
		Self::from_json(json)
	}

	/// The versioned evidence document. This is not a signature or approval.
	pub fn to_json(&self) -> String {
		serde_json::to_string_pretty(self).expect("validated window serializes")
	}

	fn validate(&self) -> Result<(), MigrationWindowError> {
		let doc = &self.0;
		if doc.format_version != 1 || doc.algorithm != ALGORITHM {
			return invalid("unsupported format or expansion algorithm");
		}
		if doc.baseline.is_empty()
			|| doc.baseline.len() > MAX_HISTORY
			|| doc.expansions.is_empty()
			|| doc.expansions.len() > MAX_EXPANSIONS
			|| self.to_json().len() > MAX_BYTES
		{
			return invalid("empty or oversized migration history");
		}
		let mut names = BTreeSet::new();
		for identity in &doc.baseline {
			if !name(&identity.name)
				|| !names.insert(&identity.name)
				|| identity.sha256.len() != 64
				|| !identity
					.sha256
					.bytes()
					.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
			{
				return invalid("invalid or repeated migration identity");
			}
		}
		let mut previous = &doc.schema;
		for expansion in &doc.expansions {
			if !name(&expansion.name) || !names.insert(&expansion.name) {
				return invalid("invalid or repeated expansion name");
			}
			leptoskit_db_schema::verify_expansion(previous, &expansion.schema, &expansion.sql)?;
			previous = &expansion.schema;
		}
		Ok(())
	}

	/// Check this build against its own migrations and embedded schema.
	///
	/// This never derives the build's schema from the window itself.
	pub fn check_build(
		&self,
		migrations: &[Migration],
		snapshot: &str,
	) -> Result<(), MigrationWindowError> {
		self.checked(migrations, snapshot).map(|_| ())
	}

	pub(crate) fn checked(
		&self,
		migrations: &[Migration],
		snapshot: &str,
	) -> Result<CheckedWindow, MigrationWindowError> {
		let own = identities(migrations)?;
		let own_schema = schema(snapshot)?.without_rename_hints();
		self.checkpoint(own, &own_schema)
	}

	fn checkpoint(
		&self,
		own: Vec<Identity>,
		own_schema: &Schema,
	) -> Result<CheckedWindow, MigrationWindowError> {
		let mut history = self.0.baseline.clone();
		let mut matches = own == history && *own_schema == self.0.schema.without_rename_hints();
		for expansion in &self.0.expansions {
			history.push(Identity {
				name: expansion.name.clone(),
				sha256: checksum(&expansion.sql),
			});
			matches |= own == history && *own_schema == expansion.schema.without_rename_hints();
		}
		if !matches {
			return Err(MigrationWindowError::BuildMismatch);
		}
		Ok(CheckedWindow {
			history,
			baseline: self.0.baseline.len(),
			own,
		})
	}
}

#[cfg_attr(not(feature = "postgres"), allow(dead_code))]
#[derive(Clone)]
pub(crate) struct CheckedWindow {
	history: Vec<Identity>,
	baseline: usize,
	own: Vec<Identity>,
}

#[cfg_attr(not(feature = "postgres"), allow(dead_code))]
impl CheckedWindow {
	pub(crate) fn pending<'a>(
		&self,
		applied: &[(String, String)],
		migrations: &'a [Migration],
	) -> Result<&'a [Migration], MigrationWindowError> {
		if identities(migrations)? != self.own {
			return Err(MigrationWindowError::BuildMismatch);
		}
		// Drivers do not promise history row ordering. Require exact set
		// membership of the registry prefix, including length and uniqueness.
		let actual: BTreeSet<_> = applied.iter().map(|(name, sum)| (name, sum)).collect();
		if applied.len() < self.baseline
			|| applied.len() > self.history.len()
			|| actual.len() != applied.len()
			|| !self.history[..applied.len()]
				.iter()
				.all(|entry| actual.contains(&(&entry.name, &entry.sha256)))
		{
			return Err(MigrationWindowError::HistoryMismatch);
		}
		Ok(&migrations[applied.len().min(migrations.len())..])
	}
}

fn identities(migrations: &[Migration]) -> Result<Vec<Identity>, MigrationWindowError> {
	migration_identities(migrations, false)
}

fn migration_identities(
	migrations: &[Migration],
	allow_consolidation: bool,
) -> Result<Vec<Identity>, MigrationWindowError> {
	if migrations.len() > MAX_HISTORY + MAX_EXPANSIONS {
		return invalid("too many migrations");
	}
	let mut names = BTreeSet::new();
	migrations
		.iter()
		.map(|migration| {
			if migration.sql.len() > MAX_BYTES
				|| !name(&migration.name)
				|| !names.insert(&migration.name)
				|| migration.checksum != checksum(&migration.sql)
				|| (!allow_consolidation && !migration.replaces.is_empty())
			{
				return invalid("changed checksum, repeated name or consolidated migration");
			}
			Ok(Identity {
				name: migration.name.to_string(),
				sha256: migration.checksum.to_string(),
			})
		})
		.collect()
}

fn schema(text: &str) -> Result<Schema, MigrationWindowError> {
	if text.len() > MAX_BYTES {
		return invalid("schema exceeds 8 MiB");
	}
	Schema::from_json(text).map_err(|error| MigrationWindowError::Invalid(error.to_string()))
}

fn name(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 255
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

fn invalid<T>(reason: &str) -> Result<T, MigrationWindowError> {
	Err(MigrationWindowError::Invalid(reason.into()))
}

#[cfg(test)]
mod tests;
