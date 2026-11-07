//! Recompute structural compatibility from exact pinned contract bytes.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read as _;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_rpc_codegen::ApiSpec;
use serde::Deserialize;
use serde::Serialize;

use super::digest;
use super::valid_digest;

const ALGORITHM: &str = "leptoskit-rpc-structural-v1";
const MAX_SPEC_BYTES: usize = 16 * 1024 * 1024;
const MAX_BASELINES: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Baseline {
	sha256: String,
	/// Exact bytes are retained so approval/history readers can recompute the
	/// check without fetching mutable URLs or trusting a stored boolean.
	specification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Compatibility {
	NotChecked,
	Checked {
		algorithm: String,
		specification: String,
		baselines: Vec<Baseline>,
	},
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Contract {
	Exported {
		sha256: String,
		compatibility: Compatibility,
	},
	NotApplicable {
		declaration: String,
	},
}

pub(super) fn validate_pins(pins: &[String]) -> Result<()> {
	ensure!(
		pins.len() <= MAX_BASELINES,
		"at most {MAX_BASELINES} RPC baselines are supported"
	);
	ensure!(
		pins.iter().all(|pin| valid_digest(pin)),
		"invalid RPC baseline SHA-256"
	);
	ensure!(
		pins.iter().collect::<BTreeSet<_>>().len() == pins.len(),
		"duplicate RPC baseline pins"
	);
	Ok(())
}

/// Read once before reproduction: later filesystem changes cannot replace
/// the contract whose independent pin the operator selected.
pub(super) fn baselines(paths: &[PathBuf], pins: &[String]) -> Result<Vec<Baseline>> {
	ensure!(
		paths.len() == pins.len(),
		"each --rpc-baseline requires one --rpc-baseline-sha256, in the same order"
	);
	validate_pins(pins)?;
	paths
		.iter()
		.zip(pins)
		.map(|(path, pin)| {
			let metadata = fs::symlink_metadata(path)
				.with_context(|| format!("read RPC baseline {}", path.display()))?;
			ensure!(
				metadata.is_file() && metadata.len() <= MAX_SPEC_BYTES as u64,
				"RPC baseline must be a regular file of at most {MAX_SPEC_BYTES} bytes"
			);
			let mut bytes = Vec::new();
			fs::File::open(path)?
				.take(MAX_SPEC_BYTES as u64 + 1)
				.read_to_end(&mut bytes)?;
			ensure!(
				bytes.len() <= MAX_SPEC_BYTES,
				"RPC baseline exceeds its size limit"
			);
			ensure!(
				digest(&bytes) == *pin,
				"RPC baseline differs from independently selected SHA-256"
			);
			let specification = String::from_utf8(bytes)?;
			ApiSpec::from_json(&specification)?;
			Ok(Baseline {
				sha256: pin.clone(),
				specification,
			})
		})
		.collect()
}

impl Contract {
	pub(super) fn exported(bytes: Vec<u8>, baselines: &[Baseline]) -> Result<Self> {
		ensure!(
			bytes.len() <= MAX_SPEC_BYTES,
			"exported RPC specification exceeds its size limit"
		);
		let sha256 = digest(&bytes);
		let specification = String::from_utf8(bytes)?;
		ApiSpec::from_json(&specification)?;
		let compatibility = if baselines.is_empty() {
			Compatibility::NotChecked
		} else {
			Compatibility::Checked {
				algorithm: ALGORITHM.into(),
				specification,
				baselines: baselines.to_vec(),
			}
		};
		let contract = Self::Exported {
			sha256,
			compatibility,
		};
		contract.validate()?;
		Ok(contract)
	}

	pub(super) fn checked(&self) -> bool {
		matches!(
			self,
			Self::Exported {
				compatibility: Compatibility::Checked { .. },
				..
			}
		)
	}

	pub(super) fn validate(&self) -> Result<()> {
		match self {
			Self::NotApplicable { declaration } => {
				ensure!(
					declaration == "operator_declares_no_rpc",
					"unsupported RPC declaration"
				)
			}
			Self::Exported {
				sha256,
				compatibility,
			} => {
				ensure!(valid_digest(sha256), "invalid exported RPC SHA-256");
				if let Compatibility::Checked {
					algorithm,
					specification,
					baselines,
				} = compatibility
				{
					ensure!(
						algorithm == ALGORITHM,
						"unsupported RPC compatibility algorithm"
					);
					check(sha256, specification, baselines)?;
				}
			}
		}
		Ok(())
	}

	/// The required window comes from operator policy, never from the
	/// candidate descriptor or a publisher-selected baseline.
	pub(super) fn require(&self, pins: &[String]) -> Result<()> {
		validate_pins(pins)?;
		self.validate()?;
		if pins.is_empty() {
			return Ok(());
		}
		let Self::Exported {
			compatibility: Compatibility::Checked { baselines, .. },
			..
		} = self
		else {
			anyhow::bail!(
				"operator RPC policy requires checked compatibility; absent and not_checked \
				 evidence cannot authorize this release"
			);
		};
		for pin in pins {
			ensure!(
				baselines.iter().any(|baseline| &baseline.sha256 == pin),
				"release lacks operator-required RPC baseline {pin}"
			);
		}
		Ok(())
	}
}

fn check(sha256: &str, specification: &str, baselines: &[Baseline]) -> Result<()> {
	ensure!(
		specification.len() <= MAX_SPEC_BYTES,
		"exported RPC specification exceeds its size limit"
	);
	ensure!(
		digest(specification.as_bytes()) == sha256,
		"RPC specification differs from the executable export digest"
	);
	ensure!(
		!baselines.is_empty(),
		"checked RPC compatibility requires a baseline"
	);
	let pins: Vec<_> = baselines
		.iter()
		.map(|baseline| baseline.sha256.clone())
		.collect();
	validate_pins(&pins)?;
	let current = ApiSpec::from_json(specification)?;
	for baseline in baselines {
		ensure!(
			baseline.specification.len() <= MAX_SPEC_BYTES,
			"RPC baseline exceeds its size limit"
		);
		ensure!(
			digest(baseline.specification.as_bytes()) == baseline.sha256,
			"stored RPC baseline differs from its pin"
		);
		let previous = ApiSpec::from_json(&baseline.specification)?;
		let changes = previous.breaking_changes(&current);
		ensure!(
			changes.is_empty(),
			"RPC baseline {} is not structurally compatible:\n{}",
			baseline.sha256,
			changes.join("\n")
		);
	}
	Ok(())
}

#[cfg(test)]
mod tests;
