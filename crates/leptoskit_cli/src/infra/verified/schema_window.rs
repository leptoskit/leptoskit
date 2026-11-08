//! Bind an exact expansion window to the reproduced source and executable.

use std::fs;
use std::io::Read as _;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::DbDialect;
use leptoskit_config::LeptoskitConfig;
use leptoskit_db::MigrationContract;
use leptoskit_db::MigrationWindow;
use leptoskit_db_schema::Dialect;
use serde::Deserialize;
use serde::Serialize;

use crate::verify::build::Engine;
use crate::verify::build::ReproducedBundle;

const MAX_BYTES: u64 = 8 * 1024 * 1024;

pub(super) struct Input {
	json: String,
	sha256: String,
	window: MigrationWindow,
}

impl Input {
	/// Freeze inputs before reproduction so later edits cannot replace them.
	pub(super) fn read(path: Option<&Path>, pin: Option<&str>) -> Result<Option<Self>> {
		let (path, pin) = match (path, pin) {
			(None, None) => return Ok(None),
			(Some(path), Some(pin)) => (path, pin),
			_ => {
				anyhow::bail!(
					"--migration-window requires --migration-window-sha256 and vice versa"
				)
			}
		};
		ensure!(super::valid_digest(pin), "invalid migration window SHA-256");
		let metadata = fs::symlink_metadata(path)?;
		ensure!(
			metadata.is_file() && metadata.len() <= MAX_BYTES,
			"migration window must be a regular file no larger than 8 MiB"
		);
		let mut json = String::new();
		fs::File::open(path)?
			.take(MAX_BYTES + 1)
			.read_to_string(&mut json)?;
		let window = MigrationWindow::from_json_pinned(&json, pin)?;
		Ok(Some(Self {
			json,
			sha256: pin.into(),
			window,
		}))
	}

	pub(super) fn package(
		self,
		engine: Engine,
		image: &str,
		bundle: &ReproducedBundle,
	) -> Result<Evidence> {
		let expected = source_contract(bundle)?;
		expected.check_window(&self.window)?;
		let bytes = super::export_document(
			engine,
			image,
			"LEPTOSKIT_DB_CONTRACT_OUT",
			"database.json",
			"migration contract",
			MAX_BYTES,
		)?;
		let contract = String::from_utf8(bytes).context("migration contract is not UTF-8")?;
		check_exported_contract(&expected, &self.window, &contract)?;
		Ok(Evidence {
			format_version: 1,
			window_sha256: self.sha256,
			window: self.json,
			contract_sha256: super::digest(contract.as_bytes()),
			contract,
		})
	}
}

pub(super) fn check_exported_contract(
	expected: &MigrationContract,
	window: &MigrationWindow,
	json: &str,
) -> Result<()> {
	let actual = MigrationContract::from_json(json)?;
	ensure!(
		actual == *expected,
		"executable's migration checkpoint differs from the reproduced source"
	);
	actual.check_window(window)?;
	Ok(())
}

fn source_contract(bundle: &ReproducedBundle) -> Result<MigrationContract> {
	let source = bundle.root().join("source");
	let config = LeptoskitConfig::load_from(&source)?;
	let db = config
		.db
		.as_ref()
		.context("migration window needs a database")?;
	ensure!(
		db.dialect == DbDialect::Postgres,
		"migration windows require PostgreSQL"
	);
	ensure!(
		!db.migrations.is_absolute()
			&& db
				.migrations
				.components()
				.all(|part| matches!(part, std::path::Component::Normal(_))),
		"migrations must stay inside reproduced source"
	);
	let directory = source.join(&db.migrations);
	let history = crate::db::History::read(&directory, Dialect::Postgres)?;
	ensure!(
		!history.names.is_empty(),
		"migration window needs an installed baseline"
	);
	let migrations = leptoskit_db::load_migrations(&directory)?;
	Ok(MigrationContract::new(
		&migrations,
		Some(&history.latest.to_json()),
	)?)
}

/// These exact bytes are covered by the complete descriptor's approvals and
/// policy digest. They are revalidated when stored history is consumed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Evidence {
	format_version: u32,
	window_sha256: String,
	window: String,
	contract_sha256: String,
	contract: String,
}

impl Evidence {
	pub(super) fn runtime(&self) -> Result<super::super::schema_window::Window> {
		super::super::schema_window::Window::new(&self.window, &self.window_sha256)
	}

	pub(super) fn validate(&self, migration_fingerprint: &str) -> Result<()> {
		ensure!(
			self.format_version == 1,
			"unsupported schema window evidence"
		);
		ensure!(
			self.window.len() <= MAX_BYTES as usize && self.contract.len() <= MAX_BYTES as usize,
			"oversized schema window evidence"
		);
		ensure!(
			super::digest(self.contract.as_bytes()) == self.contract_sha256,
			"migration contract digest differs"
		);
		let window = MigrationWindow::from_json_pinned(&self.window, &self.window_sha256)?;
		let contract = MigrationContract::from_json(&self.contract)?;
		contract.check_window(&window)?;
		let fingerprint = super::super::history_fingerprint(
			contract
				.migration_checksums()
				.map(|(name, sum)| (name.into(), sum.into()))
				.collect(),
		)?;
		ensure!(
			fingerprint == migration_fingerprint,
			"migration checkpoint differs from descriptor migration history"
		);
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fixture() -> (Evidence, String) {
		let (value, fingerprint) = super::super::super::schema_window_fixture::evidence();
		(serde_json::from_value(value).unwrap(), fingerprint)
	}

	#[test]
	fn stored_evidence_checks_bytes_checkpoint_and_release_history() {
		let (evidence, fingerprint) = fixture();
		evidence.validate(&fingerprint).unwrap();
		let roundtrip: Evidence =
			serde_json::from_str(&serde_json::to_string(&evidence).unwrap()).unwrap();
		roundtrip.validate(&fingerprint).unwrap();
		assert!(evidence.validate(&"0".repeat(64)).is_err());
		let mut changed = evidence.clone();
		changed.format_version = 2;
		assert!(changed.validate(&fingerprint).is_err());
		let mut changed = evidence.clone();
		changed.contract.push('\n');
		assert!(changed.validate(&fingerprint).is_err());
		let mut changed = evidence.clone();
		changed.window.push('\n');
		assert!(changed.validate(&fingerprint).is_err());
		let mut changed = evidence;
		let mut document: serde_json::Value = serde_json::from_str(&changed.contract).unwrap();
		document["migrations"][0]["sha256"] = "0".repeat(64).into();
		changed.contract = document.to_string();
		changed.contract_sha256 = super::super::digest(changed.contract.as_bytes());
		assert!(
			changed.validate(&fingerprint).is_err(),
			"re-pinning a substituted contract does not make it a window checkpoint"
		);
	}

	#[test]
	fn inputs_require_an_independent_pin_and_freeze_exact_bytes() {
		let (evidence, _) = fixture();
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("window.json");
		fs::write(&path, &evidence.window).unwrap();
		assert!(Input::read(None, None).unwrap().is_none());
		assert!(Input::read(Some(&path), None).is_err());
		assert!(Input::read(None, Some(&evidence.window_sha256)).is_err());
		assert!(Input::read(Some(&path), Some(&"0".repeat(64))).is_err());
		let frozen = Input::read(Some(&path), Some(&evidence.window_sha256))
			.unwrap()
			.unwrap();
		fs::write(&path, format!("{}\n", evidence.window)).unwrap();
		assert_eq!(frozen.json, evidence.window);
		assert!(Input::read(Some(&path), Some(&evidence.window_sha256)).is_err());
		assert!(Input::read(Some(directory.path()), Some(&evidence.window_sha256)).is_err());
	}
}
