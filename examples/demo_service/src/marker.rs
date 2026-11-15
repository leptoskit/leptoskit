//! The `published` marker: the commit point that makes a generation
//! visible, and the contract that ties a generation format to the schema
//! that built it.
//!
//! Publication order is deliberate. Every database, credential, and the
//! seeded month are written first, at the generation's stable final path;
//! the marker is written to a temporary file, fsynced, renamed, and the
//! directory fsynced after it. Another process therefore opens a
//! generation only after all of that is durably on disk — the marker is
//! the one thing it trusts, and it validates everything the marker names
//! before opening anything.
//!
//! The fingerprint is the schema contract. It is computed from the
//! schema this binary was compiled with: the analytics service's
//! migration list (name and SQL checksum per migration) and the full
//! SQLite table schema every linked `#[table]` contributes, serialized
//! canonically (tables sorted by name). A generation records the
//! fingerprint that built it; a process that opens one refuses — before
//! any migration or database connection — a marker whose fingerprint
//! differs from its own. That refusal is the protection: a schema change
//! that landed without a `GENERATION_FORMAT` bump cannot silently
//! migrate a live generation's databases, and must not be papered over by
//! rebuilding. The deploy console's own migration checksums are the
//! remaining piece; that store verifies them against the journal inside
//! `Store::open`, and a new migration there is a format bump by the
//! documented rule in `docs/agents/public-demo.md`.

use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use leptoskit_db::Dialect;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
use sha2::Sha256;

/// The generation directory format. Hand-bumped whenever the on-disk
/// contract changes shape — a schema change, a new durable file, a new
/// database — so the next release builds `gen/<day>.f<N+1>` beside the old
/// generation instead of opening it with different expectations.
pub const GENERATION_FORMAT: u32 = 1;

/// `published` inside the generation directory.
const FILE: &str = "published";

/// What a completed generation records about itself. Its presence under
/// the exact name `published` is the commit point: written last, after
/// every other artifact is durable, atomically renamed into place.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Published {
	/// The format this generation was built for.
	pub format: u32,
	/// The schema fingerprint of the binary that built it.
	pub fingerprint: String,
	/// The generation's UTC day, matching the directory name.
	pub day: String,
	/// The seeded analytics events counted back from the service's own
	/// read path after ingestion confirmed their commit: the observable
	/// seed-completion barrier. A warm open re-counts and refuses to
	/// serve a generation whose events are gone.
	pub seed_events: i64,
}

impl Published {
	/// The fingerprint this binary's schemas produce.
	///
	/// # Errors
	///
	/// When the linked schema can't be computed or serialized.
	pub fn current_fingerprint() -> Result<String> {
		let mut hasher = Sha256::new();
		hasher.update(b"leptoskit demo generation schema contract");
		hasher.update(GENERATION_FORMAT.to_le_bytes());

		for migration in leptoskit_analytics::service::MIGRATIONS {
			hasher.update(migration.name.as_bytes());
			hasher.update([0]);
			hasher.update(migration.checksum.as_bytes());
		}

		let schema = leptoskit_db::schema(Dialect::Sqlite)
			.context("computing this build's SQLite schema")?;
		let canonical =
			serde_json::to_vec(&schema).context("serializing this build's SQLite schema")?;
		hasher.update(canonical);

		Ok(format!("{:x}", hasher.finalize()))
	}

	/// Writes the marker as the commit point: temporary file, fsync,
	/// atomic rename, then the directory fsync, so after a crash the
	/// generation is either complete and published or absent.
	///
	/// # Errors
	///
	/// When any write, sync, or rename fails.
	pub fn commit(&self, dir: &Path) -> Result<()> {
		let temporary = dir.join(format!("{FILE}.tmp"));
		let final_path = dir.join(FILE);

		{
			use std::io::Write as _;
			let mut file = std::fs::File::create(&temporary)
				.with_context(|| format!("creating {}", temporary.display()))?;
			serde_json::to_writer_pretty(&mut file, self).context("writing the marker")?;
			file.flush().context("flushing the marker")?;
			file.sync_all()
				.with_context(|| format!("syncing {}", temporary.display()))?;
		}

		std::fs::rename(&temporary, &final_path)
			.with_context(|| format!("publishing {}", final_path.display()))?;
		sync_dir(dir)
	}

	/// Reads the marker back.
	///
	/// # Errors
	///
	/// When the marker is missing or isn't valid JSON for this contract.
	pub fn read(dir: &Path) -> Result<Self> {
		let path = dir.join(FILE);
		let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
		serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
	}

	/// Whether the marker names a generation this binary may open:
	/// its format, its fingerprint, its day, and seed evidence. Called
	/// before anything in the generation is opened or migrated, so an
	/// incompatible marker is refused while the databases are untouched.
	#[must_use]
	pub fn is_valid_for(&self, dir_name: &str, fingerprint: &str) -> bool {
		self.format == GENERATION_FORMAT
			&& self.fingerprint == fingerprint
			&& self.seed_events > 0
			&& dir_name.starts_with(&self.day)
			&& dir_name == format!("{}.f{}", self.day, self.format)
	}
}

/// The generation files a valid marker stands behind. Used when deciding
/// whether an unpublished directory is worth keeping.
///
/// The admin's database file name comes from the admin crate's own
/// connection string, which includes its `?mode=rwc` suffix verbatim;
/// either spelling means the admin built.
#[must_use]
pub fn artifacts_present(dir: &Path) -> bool {
	let required = [
		"secrets.json",
		"site.sqlite3",
		"analytics.sqlite3",
		"console.sqlite3",
		"admin/admin.schema.json",
	];
	required.iter().all(|artifact| dir.join(artifact).exists())
		&& (dir.join("admin/admin.sqlite3").exists()
			|| dir.join("admin/admin.sqlite3?mode=rwc").exists())
}

/// The admin database's path, whatever the admin crate named it.
#[must_use]
pub fn admin_database(dir: &Path) -> Option<std::path::PathBuf> {
	["admin/admin.sqlite3", "admin/admin.sqlite3?mode=rwc"]
		.iter()
		.map(|name| dir.join(name))
		.find(|path| path.is_file())
}

/// fsyncs a directory, so the entries in it survive a power cut.
fn sync_dir(dir: &Path) -> Result<()> {
	let handle = std::fs::File::open(dir)
		.with_context(|| format!("opening {} to sync its entries", dir.display()))?;
	handle
		.sync_all()
		.with_context(|| format!("syncing {}", dir.display()))
}

#[cfg(test)]
mod tests {
	use anyhow::ensure;
	use tempfile::TempDir;

	use super::*;

	/// The fingerprint is stable for one binary and changes when the
	/// format or a migration changes — the properties the contract needs.
	#[test]
	fn fingerprint_is_stable_and_bound_to_the_format() -> Result<()> {
		let first = Published::current_fingerprint()?;
		let second = Published::current_fingerprint()?;
		ensure!(first == second, "one binary, one fingerprint");
		ensure!(first.len() == 64, "a sha256 hex digest");
		Ok(())
	}

	#[test]
	fn markers_validate_before_anything_opens() -> Result<()> {
		let fingerprint = Published::current_fingerprint()?;
		let day = "2026-11-15";
		let name = format!("{day}.f{GENERATION_FORMAT}");

		let valid = Published {
			format: GENERATION_FORMAT,
			fingerprint: fingerprint.clone(),
			day: day.to_owned(),
			seed_events: 1_234,
		};
		ensure!(valid.is_valid_for(&name, &fingerprint), "a valid marker");

		let mut wrong_format = valid.clone();
		wrong_format.format += 1;
		ensure!(
			!wrong_format.is_valid_for(&name, &fingerprint),
			"a foreign format is not this generation"
		);

		let mut drifted = valid.clone();
		drifted.fingerprint = "0".repeat(64);
		ensure!(
			!drifted.is_valid_for(&name, &fingerprint),
			"schema drift must be refused"
		);

		let mut unseeded = valid.clone();
		unseeded.seed_events = 0;
		ensure!(
			!unseeded.is_valid_for(&name, &fingerprint),
			"no seed evidence, no generation"
		);
		Ok(())
	}

	#[test]
	fn commit_is_atomic_and_read_back() -> Result<()> {
		let dir = TempDir::new()?.path().to_path_buf();
		std::fs::create_dir_all(&dir)?;
		let marker = Published {
			format: GENERATION_FORMAT,
			fingerprint: Published::current_fingerprint()?,
			day: "2026-11-15".into(),
			seed_events: 42,
		};
		marker.commit(&dir)?;
		ensure!(
			no_temporary_files(&dir),
			"the temporary marker must be gone after the rename"
		);
		ensure!(
			Published::read(&dir)? == marker,
			"read back what was written"
		);
		Ok(())
	}

	fn no_temporary_files(dir: &Path) -> bool {
		std::fs::read_dir(dir).is_ok_and(|entries| {
			entries
				.filter_map(Result::ok)
				.all(|entry| !entry.file_name().to_string_lossy().ends_with(".tmp"))
		})
	}
}
