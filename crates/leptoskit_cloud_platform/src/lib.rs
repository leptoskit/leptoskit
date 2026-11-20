//! Offline, two-phase platform infrastructure generation.

pub mod addons;
pub mod bootstrap;
pub mod cli;
pub mod config;
pub mod tofu;
mod topology;
pub mod up;

/// Deterministic relative paths and UTF-8 file contents; the library performs
/// no I/O.
pub type Generated = std::collections::BTreeMap<String, String>;

/// Invalid input or an unsuccessful generation operation.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
	/// Formatting into an output document failed.
	#[error("formatting failed")]
	Format(#[from] std::fmt::Error),
	/// Invalid operator configuration.
	#[error("{0}")]
	Invalid(String),
	/// Invalid TOML (values are omitted to avoid disclosing misplaced secrets).
	#[error("invalid platform TOML")]
	Toml,
	/// Serialization failed.
	#[error("serialization failed: {0}")]
	Json(#[from] serde_json::Error),
	/// Local file operation failed.
	#[error("file operation failed: {0}")]
	Io(#[from] std::io::Error),
}
