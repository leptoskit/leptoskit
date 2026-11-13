//! A newer worker can recover historical intent only through an independently
//! pinned original plan. The original operation and host bindings stay exact.
use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use serde_json::Value;

pub(super) fn load(args: &super::Args) -> Result<Option<Value>> {
	args.recover_plan_sha256
		.as_ref()
		.map(|pin| {
			let bytes = super::pinned(&args.plan, pin, 16 * 1024 * 1024)?;
			Ok(serde_json::from_slice(&bytes)?)
		})
		.transpose()
}

pub(super) fn inputs(
	dir: &Path,
	config: &LeptoskitConfig,
	historical: Option<&Value>,
) -> Result<Value> {
	match historical {
		Some(plan) => super::super::lifecycle::recovery_inputs(dir, config, &plan["inputs"]),
		None => super::super::lifecycle::release_inputs(dir, config),
	}
}

pub(super) fn saved(path: &Path, historical: Option<&Value>) -> Result<Value> {
	if let Some(plan) = historical {
		return Ok(plan.clone());
	}

	let name = path
		.file_name()
		.and_then(|name| name.to_str())
		.context("plan filename missing")?;
	Ok(serde_json::from_slice(
		&super::input_root(path)?.metadata(name)?,
	)?)
}

pub(super) fn select(mut current: Value, historical: &Value) -> Result<Value> {
	ensure!(
		historical["format_version"] == 1,
		"unsupported historical release format"
	);
	ensure!(
		matches!(
			historical["kind"].as_str(),
			Some("graph-static-release" | "graph-release")
		),
		"unsupported historical release kind"
	);

	for field in ["installer_sha256", "transport_sha256"] {
		ensure!(
			historical[field]
				.as_str()
				.is_some_and(crate::verify::digest_is_valid),
			"invalid historical worker identity"
		);
		// These values identify the historical operation. No historical code is
		// executed: transport() always embeds the current checked-in worker.
		current[field] = historical[field].clone();
	}

	if historical["kind"] == "graph-static-release" {
		current["kind"] = historical["kind"].clone();
		static_hosts(&mut current, historical)?;
	}

	ensure!(
		current == *historical,
		"historical plan differs from checked artifacts, configuration, selection, provider or \
		 host trust; recovery cannot change intent"
	);
	Ok(historical.clone())
}

fn static_hosts(current: &mut Value, historical: &Value) -> Result<()> {
	for (name, host) in current["hosts"]
		.as_object_mut()
		.context("current hosts missing")?
	{
		static_host(host, &historical["hosts"][name])?;
	}
	Ok(())
}

fn static_host(host: &mut Value, historical: &Value) -> Result<()> {
	for field in ["executables", "linux_formats"] {
		ensure!(
			host[field]
				.as_object()
				.is_some_and(serde_json::Map::is_empty),
			"legacy static plan cannot authorize executable releases"
		);

		if historical.get(field).is_none() {
			host.as_object_mut()
				.context("current host missing")?
				.remove(field);
		}
	}

	Ok(())
}

#[cfg(test)]
mod tests;
