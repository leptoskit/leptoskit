use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use serde_json::Value;
use sha2::Digest as _;

use crate::verify::assets::Root;

fn hash(bytes: &[u8]) -> String {
	hex::encode(sha2::Sha256::digest(bytes))
}

pub(super) fn configuration(config: &LeptoskitConfig) -> Result<String> {
	Ok(hash(&serde_json::to_vec(config)?))
}

/// The generated provider and edge inputs must still describe this graph.
/// Reuse the retained CIDRs; applying a plan never silently refreshes them.
pub(super) fn generated(dir: &Path, config: &LeptoskitConfig) -> Result<BTreeMap<String, String>> {
	let root = Root::open(dir)?;
	let graph: Value = serde_json::from_slice(&root.metadata("graph.json")?)?;
	let snapshot = &graph["provisioning"]["cloudflare_ranges"];
	let ranges = (!snapshot.is_null())
		.then(|| super::super::network::Ranges::from_manifest(snapshot))
		.transpose()?;
	let plan = leptoskit_config::deployment::compile(config)
		.map_err(anyhow::Error::msg)?
		.context("graph declaration required")?;
	let files = super::super::rendered(config, &plan, ranges.as_ref())?;
	let mut result = BTreeMap::new();

	for (name, expected) in files {
		let actual = root.metadata(&name)?;
		ensure!(
			actual == expected.as_bytes(),
			"{name} no longer matches graph declarations; review and run `lk infra generate \
			 --force`, then plan again"
		);
		result.insert(name, hash(&actual));
	}

	no_extra_inputs(&root)?;

	Ok(result)
}

fn no_extra_inputs(root: &Root) -> Result<()> {
	for name in root.names()? {
		let input = [
			".tf",
			".tofu",
			".tf.json",
			".tofu.json",
			".tfvars",
			".tfvars.json",
			".tofuvars",
			".tofuvars.json",
		]
		.iter()
		.any(|suffix| name.ends_with(suffix));
		ensure!(
			!input || name == "main.tf.json",
			"unmanaged OpenTofu input {name:?}; graph provisioning consumes only generated \
			 resources"
		);
	}

	Ok(())
}

/// Recovery consumes a separately pinned historical plan. This checks its
/// exact file membership and bytes, not reproduction by today's generator.
pub(super) fn historical(
	dir: &Path,
	config: &LeptoskitConfig,
	pins: &Value,
) -> Result<BTreeMap<String, String>> {
	let pins: BTreeMap<String, String> = serde_json::from_value(pins.clone())?;
	let graph = leptoskit_config::deployment::compile(config)
		.map_err(anyhow::Error::msg)?
		.context("graph declaration required")?;
	let mut names =
		std::collections::BTreeSet::from(["main.tf.json".to_owned(), "graph.json".to_owned()]);
	names.extend(
		graph
			.routes()
			.entries()
			.iter()
			.map(|route| format!("edge-{}.json", route.host)),
	);
	ensure!(
		pins.keys().eq(names.iter()),
		"historical generated input membership differs"
	);
	let root = Root::open(dir)?;
	no_extra_inputs(&root)?;
	for (name, pin) in &pins {
		ensure!(
			crate::verify::digest_is_valid(pin) && hash(&root.metadata(name)?) == *pin,
			"historical input {name} differs from independently pinned recovery plan"
		);
	}
	Ok(pins)
}

pub(super) fn fingerprint(
	dir: &Path,
	config: &LeptoskitConfig,
) -> Result<BTreeMap<String, String>> {
	let mut files = generated(dir, config)?;
	let root = Root::open(dir)?;
	files.insert(
		".terraform.lock.hcl".into(),
		hash(&root.read_file(".terraform.lock.hcl", 1024 * 1024)?),
	);
	files.insert(super::PLAN.into(), hash(&root.metadata(super::PLAN)?));

	if root.names()?.iter().any(|name| name == ".terraform") {
		let state = root.beneath(".terraform")?;
		let names = state.names()?;

		for name in ["terraform.tfstate", "environment"] {
			if names.iter().any(|entry| entry == name) {
				files.insert(format!(".terraform/{name}"), hash(&state.metadata(name)?));
			}
		}
	}

	Ok(files)
}
