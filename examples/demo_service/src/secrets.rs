//! The generation's own credentials, generated fresh each day and stored
//! once under the generation directory.
//!
//! Nothing here is ever a fixed literal: the site and admin signing keys,
//! the analytics token, the control service's owner key, and the identity
//! provider's first-party client id and secret are all random per
//! generation. A same-day restart reads them back, so sessions, the
//! admin's client, and the JWKS survive a crash unchanged; a daily reset
//! replaces them, so yesterday's cookies and client are unknown today.

use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use serde::Deserialize;
use serde::Serialize;

/// `secrets.json` inside the generation directory.
const FILE: &str = "secrets.json";

/// The credentials one generation runs on.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Secrets {
	/// The generation's UTC day, so a secrets file can never be accepted
	/// by a generation built for another day.
	pub day: String,
	/// 32 random bytes: the main site's signing key.
	#[serde(with = "hex")]
	pub site_key: [u8; 32],
	/// 32 random bytes: the admin's signing key.
	#[serde(with = "hex")]
	pub admin_key: [u8; 32],
	/// The analytics service's token for the app's own callers. At least
	/// 32 printable characters: the control service enforces that shape
	/// for its owner key, and this token guards the same kind of trust.
	pub analytics_token: String,
	/// The control service's owner credential, process-private.
	pub control_key: String,
	/// The identity provider client the admin signs in through.
	pub idp_client_id: String,
	pub idp_client_secret: String,
}

impl Secrets {
	/// Generates a day's credentials. Called once, at build time.
	///
	/// # Errors
	///
	/// When the OS random source fails.
	pub fn generate(day: &str) -> Result<Self> {
		Ok(Self {
			day: day.to_owned(),
			site_key: random_key()?,
			admin_key: random_key()?,
			analytics_token: random_token()?,
			control_key: random_token()?,
			idp_client_id: String::new(),
			idp_client_secret: String::new(),
		})
	}

	/// The path the credentials are stored at.
	#[must_use]
	pub fn path(dir: &Path) -> PathBuf {
		dir.join(FILE)
	}

	/// Writes the credentials once: `create_new`, mode 0600, fsynced, so a
	/// partially written file can never be mistaken for a real one.
	///
	/// # Errors
	///
	/// When the file exists (it must not: a build writes it exactly once)
	/// or the write or sync fails.
	pub fn create(&self, dir: &Path) -> Result<()> {
		let path = Self::path(dir);
		let mut options = std::fs::OpenOptions::new();
		options.write(true).create_new(true);
		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt as _;
			options.mode(0o600);
		}
		let mut file = options
			.open(&path)
			.with_context(|| format!("creating {}", path.display()))?;
		serde_json::to_writer_pretty(&mut file, self).context("writing the credentials")?;
		file.flush().context("flushing the credentials")?;
		file.sync_all()
			.with_context(|| format!("syncing {}", path.display()))?;
		Ok(())
	}

	/// Reads the credentials back.
	///
	/// # Errors
	///
	/// When the file is missing, unreadable, or not this day's.
	pub fn load(dir: &Path, day: &str) -> Result<Self> {
		let path = Self::path(dir);
		let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
		let secrets: Self = serde_json::from_slice(&bytes)
			.with_context(|| format!("parsing {}", path.display()))?;
		ensure!(
			secrets.day == day,
			"{} belongs to {}, not to {}",
			path.display(),
			secrets.day,
			day
		);
		Ok(secrets)
	}

	/// Fills in the identity provider client issued at registration,
	/// before the file is written.
	#[must_use]
	pub fn with_idp_client(mut self, client_id: String, client_secret: String) -> Self {
		self.idp_client_id = client_id;
		self.idp_client_secret = client_secret;
		self
	}
}

/// 32 random bytes from the OS.
fn random_key() -> Result<[u8; 32]> {
	let mut key = [0_u8; 32];
	getrandom::fill(&mut key).context("the OS random source failed")?;
	Ok(key)
}

/// A 48-character alphanumeric token: comfortably past the control
/// service's 32..=256 printable-character rule, with no punctuation a
/// header or form could mangle.
fn random_token() -> Result<String> {
	const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
	let mut bytes = [0_u8; 48];
	getrandom::fill(&mut bytes).context("the OS random source failed")?;
	let token: String = bytes
		.iter()
		.map(|byte| ALPHABET[usize::from(byte % (ALPHABET.len() as u8))] as char)
		.collect();
	Ok(token)
}

#[cfg(test)]
mod tests {
	use tempfile::TempDir;

	use super::*;

	#[test]
	fn tokens_are_long_printable_and_nondeterministic() -> Result<()> {
		let first = random_token()?;
		let second = random_token()?;
		ensure!(first.len() >= 32, "tokens must satisfy the owner-key rule");
		ensure!(first != second, "two tokens must differ");
		ensure!(
			first.bytes().all(|byte| (33..127).contains(&byte)),
			"tokens must be printable without spaces"
		);
		Ok(())
	}

	#[test]
	fn credentials_round_trip_and_refuse_other_days() -> Result<()> {
		let temp = TempDir::new()?;
		let dir = temp.path().to_path_buf();
		let day = "2026-11-15";
		let secrets = Secrets::generate(day)?.with_idp_client(
			"client-id".into(),
			"client-secret-with-enough-length".into(),
		);
		secrets.create(&dir)?;

		#[cfg(unix)]
		{
			use std::os::unix::fs::MetadataExt as _;
			let mode = std::fs::metadata(Secrets::path(&dir))?.mode();
			ensure!(mode & 0o777 == 0o600, "the credentials file must be 0600");
		}

		let read_back = Secrets::load(&dir, day)?;
		ensure!(read_back == secrets, "credentials must round-trip");
		ensure!(
			Secrets::load(&dir, "2026-11-16").is_err(),
			"another day's generation must refuse these credentials"
		);
		ensure!(
			secrets.create(&dir).is_err(),
			"a second create must refuse to overwrite"
		);
		Ok(())
	}
}
