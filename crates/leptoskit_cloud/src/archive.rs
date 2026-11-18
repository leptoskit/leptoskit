//! The source archive: a project's files as one deterministic, gzip-compressed
//! tar, addressed by the SHA-256 of its bytes.
//!
//! The CLI packs it ([`pack`]) and uploads it by digest; a build worker
//! unpacks it ([`unpack`]) into a fresh sandbox. Both need the `archive`
//! feature. [`Digest`] and [`Limits`] are always available, so the dashboard
//! can show and check them.
//!
//! The rules, which both directions hold:
//!
//! - **What goes in.** In a Git checkout, the files `git ls-files --cached
//!   --others --exclude-standard` lists: tracked files and untracked ones
//!   `.gitignore` doesn't exclude. Elsewhere, a walk that honours `.gitignore`
//!   files. A `.leptoskitignore` file at the root, in `.gitignore`'s syntax,
//!   excludes more. `target/`, `.leptoskit/`, `.git/`, `dist/`, and
//!   `node_modules/` are always left out, at any depth. Files deleted from the
//!   working tree are left out even while Git still tracks them.
//! - **Deterministic.** Paths are sorted, times and owners are zero, modes are
//!   `0644`, or `0755` for executables, and the gzip header carries no time or
//!   operating system. Packing the same tree twice gives the same bytes, and so
//!   the same digest, with the same build of this crate; a different
//!   compression backend may compress differently.
//! - **Bounded.** At most 20,000 files and 512 MiB uncompressed; a file over 64
//!   MiB is refused with its path.
//! - **Contained.** Absolute paths, `..`, and symlinks that leave the root,
//!   followed through the archive's other symlinks, are refused when packing
//!   and when unpacking. Unpacking writes nothing through a symlink and runs no
//!   program.
//! - **No secrets.** `.env`, `.env.*`, `*.pem`, `*.p8`, `*.p12`, `*.jks`,
//!   `*.keystore`, and `credentials*.toml` are refused, naming the file and `lk
//!   cloud secret set`, unless `.leptoskitignore` (or `.gitignore`) already
//!   excludes them.

use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

#[cfg(feature = "archive")]
mod pack;
#[cfg(feature = "archive")]
mod paths;
#[cfg(feature = "archive")]
mod unpack;

#[cfg(feature = "archive")]
pub use pack::PackError;
#[cfg(feature = "archive")]
pub use pack::PackOptions;
#[cfg(feature = "archive")]
pub use pack::Packed;
#[cfg(feature = "archive")]
pub use pack::pack;
#[cfg(feature = "archive")]
pub use unpack::UnpackError;
#[cfg(feature = "archive")]
pub use unpack::Unpacked;
#[cfg(feature = "archive")]
pub use unpack::unpack;

/// The file that excludes more than `.gitignore` does, at the project's
/// root.
pub const IGNORE_FILE: &str = ".leptoskitignore";

/// Directories that never go into an archive, at any depth.
pub const ALWAYS_EXCLUDED: [&str; 5] = ["target", ".leptoskit", ".git", "dist", "node_modules"];

/// How much an archive may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
	/// The most files and symlinks.
	pub max_files: u64,
	/// The most bytes, uncompressed, across every file.
	pub max_bytes: u64,
	/// The most bytes one file may have.
	pub max_file_bytes: u64,
}

impl Limits {
	/// 20,000 files, 512 MiB in all, and 64 MiB for one file.
	pub const DEFAULT: Self = Self {
		max_files: 20_000,
		max_bytes: 512 * 1024 * 1024,
		max_file_bytes: 64 * 1024 * 1024,
	};
}

impl Default for Limits {
	fn default() -> Self {
		Self::DEFAULT
	}
}

/// The SHA-256 of an archive's compressed bytes, written `sha256:<hex>`
/// with 64 lower-case hex digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest(String);

/// Why a string isn't a [`Digest`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` isn't a digest: write `sha256:` and 64 lower-case hex digits")]
pub struct DigestError(String);

impl Digest {
	/// Checks `value`.
	///
	/// # Errors
	///
	/// [`DigestError`] when it isn't `sha256:` and 64 lower-case hex digits.
	pub fn parse(value: &str) -> Result<Self, DigestError> {
		let hex = value.strip_prefix("sha256:").filter(|hex| {
			hex.len() == 64
				&& hex
					.chars()
					.all(|character| matches!(character, '0'..='9' | 'a'..='f'))
		});

		match hex {
			Some(_) => Ok(Self(value.to_owned())),
			None => Err(DigestError(value.to_owned())),
		}
	}

	/// The digest of `bytes`.
	#[cfg(feature = "archive")]
	#[must_use]
	pub fn of(bytes: &[u8]) -> Self {
		use sha2::Digest as _;

		const HEX: &[u8; 16] = b"0123456789abcdef";
		let hash = sha2::Sha256::digest(bytes);
		let mut written = String::with_capacity("sha256:".len() + 64);
		written.push_str("sha256:");

		for byte in hash {
			written.push(char::from(HEX[usize::from(byte >> 4)]));
			written.push(char::from(HEX[usize::from(byte & 0x0f)]));
		}

		Self(written)
	}

	/// The digest, `sha256:<hex>`.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}

	/// The 64 hex digits alone.
	#[must_use]
	pub fn hex(&self) -> &str {
		&self.0["sha256:".len()..]
	}
}

impl FromStr for Digest {
	type Err = DigestError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		Self::parse(value)
	}
}

impl fmt::Display for Digest {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(&self.0)
	}
}

impl Serialize for Digest {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(&self.0)
	}
}

impl<'de> Deserialize<'de> for Digest {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let value = String::deserialize(deserializer)?;
		Self::parse(&value).map_err(serde::de::Error::custom)
	}
}

impl schemars::JsonSchema for Digest {
	fn schema_name() -> std::borrow::Cow<'static, str> {
		"Digest".into()
	}

	fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
		schemars::json_schema!({
			"type": "string",
			"description": "A SHA-256 digest: `sha256:` and 64 lower-case hex digits.",
			"pattern": "^sha256:[0-9a-f]{64}$"
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn digests_are_sha256_and_lower_case_hex() {
		let valid = format!("sha256:{}", "0123456789abcdef".repeat(4));
		let digest = Digest::parse(&valid).unwrap();
		assert_eq!(digest.as_str(), valid);
		assert_eq!(digest.hex(), "0123456789abcdef".repeat(4));
		assert_eq!(
			serde_json::from_str::<Digest>(&format!("\"{valid}\"")).unwrap(),
			digest
		);

		for invalid in [
			String::new(),
			"sha256:".into(),
			"0123456789abcdef".repeat(4),
			format!("sha512:{}", "a".repeat(64)),
			format!("sha256:{}", "A".repeat(64)),
			format!("sha256:{}", "a".repeat(63)),
			format!("sha256:{}", "a".repeat(65)),
			format!("sha256:{}g", "a".repeat(63)),
		] {
			assert!(Digest::parse(&invalid).is_err(), "{invalid}");
			assert!(serde_json::from_str::<Digest>(&format!("\"{invalid}\"")).is_err());
		}
	}

	#[cfg(feature = "archive")]
	#[test]
	fn digest_of_matches_a_known_hash() {
		assert_eq!(
			Digest::of(b"abc").as_str(),
			"sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
		);
	}

	#[test]
	fn the_default_limits_are_the_contract_s() {
		assert_eq!(
			Limits::default(),
			Limits {
				max_files: 20_000,
				max_bytes: 536_870_912,
				max_file_bytes: 67_108_864,
			}
		);
	}
}
