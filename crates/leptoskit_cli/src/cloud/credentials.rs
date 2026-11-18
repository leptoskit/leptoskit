//! Private, atomic session storage, with a separate inode for the refresh lock.

use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;

use crate::publish::secrets::Secret;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Session {
	pub client_id: String,
	#[serde(with = "secret")]
	pub access: Secret,
	#[serde(with = "secret")]
	pub refresh: Secret,
	pub expires_at: u64,
}

pub(super) mod secret {
	use super::*;
	pub fn serialize<S: serde::Serializer>(
		value: &Secret,
		serializer: S,
	) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(value.expose())
	}
	pub fn deserialize<'de, D: serde::Deserializer<'de>>(
		deserializer: D,
	) -> Result<Secret, D::Error> {
		String::deserialize(deserializer).map(Secret::new)
	}
}

#[derive(Clone, Debug)]
pub(super) struct Store {
	path: PathBuf,
}

impl Store {
	pub fn new(root: &Path, origin: &str) -> Self {
		Self {
			path: root.join(format!(
				"{:x}.json",
				sha2::Sha256::digest(origin.as_bytes())
			)),
		}
	}

	pub fn default_root() -> Result<PathBuf> {
		let root = match std::env::var_os("XDG_CONFIG_HOME") {
			Some(root) => PathBuf::from(root),
			None => {
				PathBuf::from(std::env::var_os("HOME").context("Set HOME or XDG_CONFIG_HOME.")?)
					.join(".config")
			}
		};
		Ok(root.join("leptoskit/cloud"))
	}

	fn directory(&self) -> Result<()> {
		let parent = self
			.path
			.parent()
			.context("Credential file needs a parent.")?;
		if !parent.exists() {
			#[cfg(unix)]
			{
				use std::os::unix::fs::DirBuilderExt as _;
				std::fs::DirBuilder::new()
					.recursive(true)
					.mode(0o700)
					.create(parent)?;
			}
			#[cfg(not(unix))]
			std::fs::create_dir_all(parent)?;
		}
		private(parent, 0o700)
	}

	pub fn lock(&self) -> Result<File> {
		self.directory()?;
		let path = self.path.with_extension("lock");
		let mut options = OpenOptions::new();
		options.read(true).write(true).create(true).truncate(false);
		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt as _;
			options.mode(0o600);
		}
		let file = options.open(&path)?;
		private(&path, 0o600)?;
		fs4::FileExt::lock(&file)?;
		Ok(file)
	}

	pub fn load(&self) -> Result<Option<Session>> {
		if !self.path.try_exists()? {
			return Ok(None);
		}
		self.directory()?;
		private(&self.path, 0o600)?;
		serde_json::from_slice(&std::fs::read(&self.path)?)
			.map(Some)
			.map_err(|_| {
				anyhow::anyhow!(
					"Cloud credentials are damaged. Run `lk cloud logout`, then `lk cloud login`."
				)
			})
	}

	pub fn save(&self, session: &Session) -> Result<()> {
		self.directory()?;
		if self.path.try_exists()? {
			private(&self.path, 0o600)?;
		}
		let parent = self
			.path
			.parent()
			.context("Credential file needs a parent.")?;
		let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
		temporary.write_all(&serde_json::to_vec(session)?)?;
		temporary.as_file().sync_all()?;
		temporary.persist(&self.path)?;
		#[cfg(unix)]
		File::open(parent)?.sync_all()?;
		Ok(())
	}

	pub fn delete(&self) -> Result<()> {
		match std::fs::remove_file(&self.path) {
			Ok(()) => Ok(()),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
			Err(error) => Err(error.into()),
		}
	}
}

fn private(path: &Path, mode: u32) -> Result<()> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		let metadata = std::fs::symlink_metadata(path)?;
		ensure!(
			!metadata.file_type().is_symlink(),
			"Cloud credentials must not be a symbolic link."
		);
		ensure!(
			metadata.permissions().mode() & 0o777 == mode,
			"Cloud credentials must be private. Run: chmod {mode:o} '{}'",
			path.display()
		);
	}
	Ok(())
}
