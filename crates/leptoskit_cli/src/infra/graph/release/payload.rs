//! Bounded transport preparation inside the checked staging handle.
use std::collections::BTreeMap;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use serde_json::Value;

use crate::verify::assets::Root;

const LIMIT: usize = 64 * 1024 * 1024;

pub(super) struct Payload {
	pub files: BTreeMap<String, String>,
	pub backends: BTreeMap<String, String>,
}

pub(super) fn read(source: &Root, host: &Value, initial_bytes: usize) -> Result<Payload> {
	let mut total = initial_bytes;
	ensure!(total <= LIMIT, "Caddy exceeds release transport limit");
	let mut result = Payload {
		files: BTreeMap::new(),
		backends: BTreeMap::new(),
	};
	for (unit, tree) in host["units"].as_object().context("units missing")? {
		let root = source.beneath(unit)?;
		for (path, member) in tree.as_object().context("asset tree missing")? {
			if member["kind"] == "file" {
				let bytes = root
					.beneath("assets")?
					.release_bytes(path, (LIMIT - total) as u64)?;
				total += bytes.len();
				result
					.files
					.insert(format!("{unit}/{path}"), super::encode(&bytes));
			}
		}
		if let Some(contract) = host.get("executables").and_then(|map| map.get(unit)) {
			let bytes = root.release_bytes("backend", (LIMIT - total) as u64)?;
			ensure!(
				contract["backend"]["sha256"] == super::hash(&bytes)
					&& contract["backend"]["size"] == bytes.len() as u64,
				"staged executable differs from release contract"
			);
			total += bytes.len();
			result.backends.insert(unit.clone(), super::encode(&bytes));
		}
	}
	Ok(result)
}

#[cfg(test)]
mod tests;
