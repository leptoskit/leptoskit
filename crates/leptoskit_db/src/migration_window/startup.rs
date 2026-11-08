//! Executable-owned checkpoints, exported before database or auth startup.

use std::collections::BTreeSet;
#[cfg(any(feature = "postgres", feature = "sqlite"))]
use std::io::Read;
use std::path::Path;

use super::ALGORITHM;
use super::Identity;
use super::MAX_BYTES;
use super::MAX_EXPANSIONS;
use super::MAX_HISTORY;
use super::Migration;
use super::MigrationWindow;
use super::MigrationWindowError;
use super::Schema;
use super::invalid;
use super::migration_identities;
use super::name;
use super::schema;
use crate::MigrationError;

/// A compiled server's checkpoint and implemented migration protocol.
///
/// Exported metadata is a statement by that executable. Verify its source
/// and artifact identity separately; a copied JSON document or image label
/// does not prove a running server uses this protocol.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "ContractDocument")]
pub struct MigrationContract {
	format_version: u32,
	window_protocol: Option<String>,
	migrations: Vec<Identity>,
	schema: Option<Schema>,
	consolidated_history: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ContractDocument {
	format_version: u32,
	window_protocol: Option<String>,
	migrations: Vec<Identity>,
	schema: Option<Schema>,
	consolidated_history: bool,
}

impl TryFrom<ContractDocument> for MigrationContract {
	type Error = MigrationWindowError;

	fn try_from(document: ContractDocument) -> Result<Self, Self::Error> {
		let contract = Self {
			format_version: document.format_version,
			window_protocol: document.window_protocol,
			migrations: document.migrations,
			schema: document.schema,
			consolidated_history: document.consolidated_history,
		};
		contract.validate()?;
		Ok(contract)
	}
}

impl MigrationContract {
	/// Derive a contract from this build's embedded migrations and snapshot.
	/// An empty registry has no snapshot. Consolidated histories, SQLite, and
	/// legacy snapshots remain strict and cannot opt into an expansion window.
	pub fn new(
		migrations: &[Migration],
		snapshot: Option<&str>,
	) -> Result<Self, MigrationWindowError> {
		let mut contract = Self {
			format_version: 1,
			window_protocol: None,
			migrations: migration_identities(migrations, true)?,
			schema: snapshot
				.map(schema)
				.transpose()?
				.map(|schema| schema.without_rename_hints()),
			consolidated_history: migrations
				.iter()
				.any(|migration| !migration.replaces.is_empty()),
		};

		if contract.can_expand() {
			contract.window_protocol = Some(ALGORITHM.into());
		}

		contract.validate()?;
		Ok(contract)
	}

	/// Decode an exported contract and validate its supported format.
	pub fn from_json(json: &str) -> Result<Self, MigrationWindowError> {
		if json.len() > MAX_BYTES {
			return invalid("migration contract exceeds 8 MiB");
		}

		let contract: Self = serde_json::from_str(json)
			.map_err(|error| MigrationWindowError::Invalid(error.to_string()))?;
		contract.validate()?;
		Ok(contract)
	}

	/// Serialize the executable's checkpoint, without deployment authority.
	pub fn to_json(&self) -> String {
		serde_json::to_string_pretty(self).expect("migration contract serializes")
	}

	/// Whether this checkpoint can participate in the current window protocol.
	pub fn supports_windows(&self) -> bool {
		self.window_protocol.as_deref() == Some(ALGORITHM)
	}

	/// Migration names and exact SQL checksums in this build's registry order.
	pub fn migration_checksums(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
		self.migrations
			.iter()
			.map(|entry| (entry.name.as_str(), entry.sha256.as_str()))
	}

	/// Check that this exported checkpoint belongs to a validated window.
	/// Release consumers must also bind the export to the checked executable.
	pub fn check_window(&self, window: &MigrationWindow) -> Result<(), MigrationWindowError> {
		self.validate()?;

		if !self.supports_windows() {
			return invalid("executable does not support the expansion window protocol");
		}

		window.checkpoint(
			self.migrations.clone(),
			self.schema.as_ref().expect("checked schema"),
		)?;
		Ok(())
	}

	fn can_expand(&self) -> bool {
		!self.migrations.is_empty()
			&& !self.consolidated_history
			&& self.schema.as_ref().is_some_and(|schema| {
				schema.dialect == leptoskit_db_schema::Dialect::Postgres
					&& schema.format == leptoskit_db_schema::FORMAT_VERSION
					&& schema.packs.is_some()
			})
	}

	fn validate(&self) -> Result<(), MigrationWindowError> {
		if self.format_version != 1
			|| self.migrations.len() > MAX_HISTORY + MAX_EXPANSIONS
			|| self.migrations.is_empty() != self.schema.is_none()
			|| self.to_json().len() > MAX_BYTES
		{
			return invalid("unsupported migration contract or missing embedded snapshot");
		}
		let expected = self.can_expand().then_some(ALGORITHM);

		if self.window_protocol.as_deref() != expected {
			return invalid("migration contract protocol contradicts its checkpoint");
		}

		if let Some(schema) = &self.schema {
			if schema.format != 1 && schema.format != leptoskit_db_schema::FORMAT_VERSION {
				return invalid("unsupported embedded schema format");
			}

			schema
				.validate()
				.map_err(|error| MigrationWindowError::Invalid(error.to_string()))?;
		}

		let mut names = BTreeSet::new();

		for identity in &self.migrations {
			if !name(&identity.name)
				|| !names.insert(&identity.name)
				|| identity.sha256.len() != 64
				|| !identity
					.sha256
					.bytes()
					.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
			{
				return invalid("invalid migration contract identity");
			}
		}

		Ok(())
	}
}

/// Write an executable's migration contract if requested, before startup.
///
/// Call this before configuration, authentication, database connections, or
/// sockets. When `LEPTOSKIT_DB_CONTRACT_OUT` is set, it creates that file and
/// returns `true`; return from `main` immediately. An existing file is not
/// overwritten. With no request this does no file access or database work.
pub fn export_migration_contract_if_requested(
	migrations: &[Migration],
	snapshot: Option<&str>,
) -> Result<bool, MigrationError> {
	let Some(path) = std::env::var_os("LEPTOSKIT_DB_CONTRACT_OUT") else {
		return Ok(false);
	};
	let path = std::path::PathBuf::from(path);
	let contract = MigrationContract::new(migrations, snapshot)?;
	write_contract(&path, &contract)?;
	Ok(true)
}

fn write_contract(path: &Path, contract: &MigrationContract) -> Result<(), MigrationError> {
	use std::io::Write as _;
	let result = (|| {
		let mut file = std::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(path)?;
		file.write_all(contract.to_json().as_bytes())?;
		file.sync_all()
	})();
	result.map_err(|source| {
		MigrationError::ContractExport {
			path: path.to_owned(),
			source,
		}
	})
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
pub(crate) fn startup_window(
	migrations: &[Migration],
	snapshot: Option<&str>,
) -> Result<Option<MigrationWindow>, MigrationError> {
	let contract = MigrationContract::new(migrations, snapshot)?;
	let path = std::env::var_os("LEPTOSKIT_MIGRATION_WINDOW");
	let pin = std::env::var_os("LEPTOSKIT_MIGRATION_WINDOW_SHA256");
	configured_window(&contract, path.as_deref(), pin.as_deref())
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
fn configured_window(
	contract: &MigrationContract,
	path: Option<&std::ffi::OsStr>,
	pin: Option<&std::ffi::OsStr>,
) -> Result<Option<MigrationWindow>, MigrationError> {
	let (path, pin) = match (path, pin) {
		(None, None) => return Ok(None),
		(Some(path), Some(pin)) => (path, pin),
		_ => {
			return Err(MigrationWindowError::Invalid(
				"set both LEPTOSKIT_MIGRATION_WINDOW and LEPTOSKIT_MIGRATION_WINDOW_SHA256".into(),
			)
			.into());
		}
	};

	let pin = pin.to_str().ok_or_else(|| {
		MigrationWindowError::Invalid("migration window digest is not UTF-8".into())
	})?;
	let window = read_window(Path::new(path), pin)?;
	contract.check_window(&window)?;
	Ok(Some(window))
}

#[cfg(any(feature = "postgres", feature = "sqlite"))]
fn read_window(path: &Path, pin: &str) -> Result<MigrationWindow, MigrationError> {
	let read = || -> std::io::Result<String> {
		if !std::fs::metadata(path)?.is_file() {
			return Err(std::io::Error::other(
				"migration window must be a regular file",
			));
		}

		let mut json = String::new();
		std::fs::File::open(path)?
			.take((MAX_BYTES + 1) as u64)
			.read_to_string(&mut json)?;
		Ok(json)
	};
	let json = read().map_err(|source| {
		MigrationError::Read {
			path: path.to_owned(),
			source,
		}
	})?;
	Ok(MigrationWindow::from_json_pinned(&json, pin)?)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::migration_window::tests::fixture;

	#[test]
	fn executable_contract_matches_only_its_own_checkpoint() {
		let (migrations, schemas, window) = fixture();

		for length in 1..=3 {
			let contract =
				MigrationContract::new(&migrations[..length], Some(&schemas[length - 1].to_json()))
					.unwrap();
			assert!(contract.supports_windows());
			let exported = MigrationContract::from_json(&contract.to_json()).unwrap();
			assert_eq!(exported, contract);
			exported.check_window(&window).unwrap();
		}

		let wrong = MigrationContract::new(&migrations[..1], Some(&schemas[2].to_json())).unwrap();
		assert!(matches!(
			wrong.check_window(&window),
			Err(MigrationWindowError::BuildMismatch)
		));
	}

	#[test]
	fn legacy_sqlite_empty_and_consolidated_checkpoints_remain_strict() {
		let (mut migrations, mut schemas, window) = fixture();
		let empty = MigrationContract::new(&[], None).unwrap();
		assert!(!empty.supports_windows());
		assert!(empty.check_window(&window).is_err());
		assert!(MigrationContract::new(&migrations, None).is_err());
		schemas[0].dialect = leptoskit_db_schema::Dialect::Sqlite;
		assert!(
			!MigrationContract::new(&migrations[..1], Some(&schemas[0].to_json()))
				.unwrap()
				.supports_windows()
		);
		schemas[0].dialect = leptoskit_db_schema::Dialect::Postgres;

		schemas[0].format = 1;
		schemas[0].packs = None;
		assert!(
			!MigrationContract::new(&migrations[..1], Some(&schemas[0].to_json()))
				.unwrap()
				.supports_windows()
		);
		migrations[0].replaces.push("older".into());
		assert!(
			!MigrationContract::new(&migrations, Some(&schemas[2].to_json()))
				.unwrap()
				.supports_windows()
		);
	}

	#[test]
	fn serde_consumers_cannot_skip_contract_validation() {
		let (migrations, schemas, _) = fixture();
		let contract =
			MigrationContract::new(&migrations[..1], Some(&schemas[0].to_json())).unwrap();
		let original = serde_json::to_value(contract).unwrap();
		for (field, value) in [
			("format_version", serde_json::json!(2)),
			("window_protocol", serde_json::json!("trust_me")),
			("migrations", serde_json::json!([])),
		] {
			let mut document = original.clone();
			document[field] = value;
			assert!(serde_json::from_value::<MigrationContract>(document).is_err());
		}
		let mut document = original;
		document["verified"] = true.into();
		assert!(serde_json::from_value::<MigrationContract>(document).is_err());
	}

	#[test]
	fn contract_export_does_not_overwrite_an_existing_result() {
		let (migrations, schemas, _) = fixture();
		let contract = MigrationContract::new(&migrations, Some(&schemas[2].to_json())).unwrap();
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("contract.json");
		write_contract(&path, &contract).unwrap();
		assert_eq!(
			MigrationContract::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap(),
			contract
		);
		std::fs::write(&path, "previous output").unwrap();
		assert!(write_contract(&path, &contract).is_err());

		assert_eq!(std::fs::read_to_string(path).unwrap(), "previous output");
	}

	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	#[test]
	fn startup_never_falls_back_after_a_partial_or_changed_window_input() {
		use std::ffi::OsStr;
		let (migrations, schemas, window) = fixture();
		let contract =
			MigrationContract::new(&migrations[..1], Some(&schemas[0].to_json())).unwrap();
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("window.json");
		let json = window.to_json();
		let pin = crate::checksum(&json);
		std::fs::write(&path, &json).unwrap();

		let file = Some(path.as_os_str());
		let digest = Some(OsStr::new(&pin));
		assert!(configured_window(&contract, None, None).unwrap().is_none());
		assert!(configured_window(&contract, file, None).is_err());
		assert!(configured_window(&contract, None, digest).is_err());
		assert!(
			configured_window(&contract, file, digest)
				.unwrap()
				.is_some()
		);
		std::fs::write(&path, format!("{json}\n")).unwrap();
		assert!(configured_window(&contract, file, digest).is_err());
		assert!(configured_window(&contract, Some(dir.path().as_os_str()), digest).is_err());
	}
}
