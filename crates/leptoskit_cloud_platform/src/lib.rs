//! Offline, two-phase platform infrastructure generation.

pub mod config;

/// Invalid input or an unsuccessful generation operation.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
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
