//! Inspect declarations without running recipes or reading provider state.

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::deployment;
use leptoskit_config::graph::Unit;
use serde_json::Value;
use serde_json::json;

pub(crate) mod build;
mod edge;
pub(super) mod lifecycle;
mod network;
mod provider;
pub(crate) mod release;

/// Stable dedicated account, shared by provisioning and executable releases.
fn account(project: &str, unit: &Unit) -> String {
	use sha2::Digest as _;
	let identity = format!("{project}\0{unit}");
	let digest = hex::encode(sha2::Sha256::digest(identity.as_bytes()));
	format!("lk-{}", &digest[..24])
}

pub(super) fn generate(
	root: &std::path::Path,
	config: &LeptoskitConfig,
	force: bool,
) -> Result<()> {
	let directory = lifecycle::directory(root, config)?;
	let _lock = lifecycle::lock(&directory)?;
	lifecycle::invalidate(&directory)?;
	generate_locked(root, config, force)
}

fn generate_locked(root: &std::path::Path, config: &LeptoskitConfig, force: bool) -> Result<()> {
	config.validate()?;
	let plan = deployment::compile(config)
		.map_err(anyhow::Error::msg)?
		.context("graph declaration required")?;
	let infra = config
		.infra
		.as_ref()
		.and_then(leptoskit_config::infra::InfraConfig::as_graph)
		.context("graph infrastructure required")?;
	let ranges = if !plan.routes().entries().is_empty()
		&& infra
			.edge
			.as_ref()
			.is_some_and(|edge| edge.ingress == deployment::Ingress::CloudflareHttps)
	{
		Some(network::Ranges::fetch()?)
	} else {
		None
	};
	let files = rendered(config, &plan, ranges.as_ref())?;
	let files: Vec<_> = files
		.iter()
		.map(|(name, text)| (name.as_str(), text.clone()))
		.collect();
	let directory = root.join("infra").join(&infra.name);
	super::write_generated(&directory, &files, force)?;
	println!(
		"Graph provider configuration written to {}. Run `lk infra plan` to review provisioning. \
		 Application deployment requires the remaining lifecycle integration.",
		directory.display()
	);
	Ok(())
}

/// Reproduce generated inputs from declarations and their retained range
/// snapshot.
fn rendered(
	config: &LeptoskitConfig,
	plan: &leptoskit_config::graph::plan::Plan,
	ranges: Option<&network::Ranges>,
) -> Result<Vec<(String, String)>> {
	let infra = config
		.infra
		.as_ref()
		.and_then(leptoskit_config::infra::InfraConfig::as_graph)
		.context("graph infrastructure required")?;
	let resources = provider::resources(infra, plan, ranges)?;
	let edges = edge::configurations(config, plan, ranges)?;
	let mut declaration = report(config, &[])?;
	declaration["provisioning"] = json!({
		"cloudflare_ranges": ranges.map(network::Ranges::manifest),
		"origin_tls": if ranges.is_some() { "csr_required" } else { "not_applicable" },
		"runtime_installation": "not_checked",
		"zone_rule_ownership": "not_checked", "application_lifecycle": "not_checked",
		"edge": {"configuration":"generated", "caddy_version":edge::CADDY_VERSION,
			"installed_binary":"not_checked", "route_runtime":"not_checked"}
	});
	let mut files = vec![
		(
			"main.tf.json".to_owned(),
			serde_json::to_string_pretty(&resources)? + "\n",
		),
		(
			"graph.json".to_owned(),
			serde_json::to_string_pretty(&declaration)? + "\n",
		),
	];
	for (host, edge) in edges {
		files.push((
			format!("edge-{host}.json"),
			serde_json::to_string_pretty(&edge)? + "\n",
		));
	}
	Ok(files)
}

pub(super) fn print(config: &LeptoskitConfig, selected: &[String]) -> Result<()> {
	println!(
		"{}",
		serde_json::to_string_pretty(&report(config, selected)?)?
	);
	Ok(())
}

fn report(config: &LeptoskitConfig, selected: &[String]) -> Result<Value> {
	config.validate()?;
	let plan = deployment::compile(config)
		.map_err(anyhow::Error::msg)?
		.context(
			"infra graph requires infra.mode = 'graph'; existing managed projects retain their \
			 current commands",
		)?;
	let units = if selected.is_empty() {
		plan.placements().keys().cloned().collect()
	} else {
		selected
			.iter()
			.map(|value| unit(value))
			.collect::<Result<Vec<_>>>()?
	};

	let selection = plan.select(&units).map_err(anyhow::Error::msg)?;
	let placements: Vec<_> = plan
		.placements()
		.values()
		.map(|placement| {
			json!({
				"unit": placement.unit.to_string(), "host": placement.host,
				"artifacts": placement.artifacts, "depends_on": names(&placement.depends_on),
				"exclusive_host_required": placement.requires_exclusive_host,
			})
		})
		.collect();

	let routes: Vec<_> = plan
		.routes()
		.entries()
		.iter()
		.map(|route| {
			json!({
				"unit": route.unit.to_string(), "host": route.host,
				"domain": route.domain, "mount": route.mount,
			})
		})
		.collect();
	let trees: Vec<_> = plan
		.trees()
		.entries()
		.iter()
		.map(|tree| {
			let excluded = plan
				.trees()
				.exclusions(&tree.unit)
				.map_err(anyhow::Error::msg)?;
			Ok(
				json!({"unit": tree.unit.to_string(), "artifact": tree.artifact,
			"directory": tree.directory, "excluded_subtrees": excluded}),
			)
		})
		.collect::<Result<Vec<_>>>()?;
	let calls: Vec<_> = plan
		.calls()
		.iter()
		.map(|call| {
			json!({
				"caller": call.caller.to_string(), "target": call.target.to_string(),
			})
		})
		.collect();
	let stages: Vec<_> = selection.stages.iter().map(|stage| names(stage)).collect();
	let hosts = hosts(config, &plan)?;
	Ok(json!({
		"format_version": 1, "kind": "deployment_graph_declarations",
		"evidence": {
			"configuration": "checked", "artifacts": "not_checked",
			"providers": "not_checked", "runtime": "not_checked"
		},
		"hosts": hosts, "units": placements, "routes": routes, "trees": trees, "calls": calls,
		"selection": {"stages": stages, "artifacts": selection.artifacts, "readiness_only": names(&selection.readiness)},
	}))
}

fn hosts(
	config: &LeptoskitConfig,
	plan: &leptoskit_config::graph::plan::Plan,
) -> Result<Vec<Value>> {
	use leptoskit_config::graph::plan::ContainerRuntime;
	let infra = config
		.infra
		.as_ref()
		.and_then(leptoskit_config::infra::InfraConfig::as_graph)
		.context("graph infrastructure declaration is absent")?;
	Ok(plan
		.hosts()
		.values()
		.map(|host| {
			let declared = &infra.hosts[&host.name];
			let runtime = host.container_runtime.map(|runtime| {
				match runtime {
					ContainerRuntime::RootlessDocker => "rootless-docker",
					ContainerRuntime::RootlessPodman => "rootless-podman",
				}
			});
			json!({"name": host.name, "compute": declared.compute, "location": declared.location,
			"server_type": declared.server_type, "exclusive": host.exclusive,
			"requested_container_runtime": runtime})
		})
		.collect())
}

fn names(units: &[Unit]) -> Vec<String> {
	units.iter().map(ToString::to_string).collect()
}

fn unit(value: &str) -> Result<Unit> {
	match value.split_once('.') {
		Some(("frontends", name)) => Ok(Unit::Frontend(name.into())),
		Some(("services", name)) => Ok(Unit::Service(name.into())),
		_ => anyhow::bail!("unit {value:?} must be frontends.<name> or services.<name>"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn website() -> LeptoskitConfig {
		toml::from_str(include_str!("../../tests/fixtures/website-graph.toml")).unwrap()
	}

	#[test]
	fn tunnel_generation_preserves_edits_until_force_is_explicit() {
		let mut config = website();
		let leptoskit_config::infra::InfraConfig::Graph(infra) = config.infra.as_mut().unwrap()
		else {
			panic!("graph fixture required");
		};
		let edge = infra.edge.as_mut().unwrap();
		edge.ingress = deployment::Ingress::CloudflareTunnel;
		edge.origin_firewall = deployment::OriginFirewall::Closed;
		config
			.services
			.get_mut("play-api")
			.unwrap()
			.deploy
			.as_mut()
			.unwrap()
			.ingress = Some(deployment::Ingress::CloudflareTunnel);

		let root = tempfile::tempdir().unwrap();
		generate(root.path(), &config, false).unwrap();
		let directory = root.path().join("infra/leptoskit-web");
		let main = directory.join("main.tf.json");
		let manifest = directory.join("graph.json");
		let edge = directory.join("edge-web.json");
		let edge_original = std::fs::read(&edge).unwrap();
		assert!(directory.join("edge-compiler.json").is_file());
		let original = std::fs::read(&main).unwrap();
		let report: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
		assert_eq!(report["provisioning"]["origin_tls"], "not_applicable");
		assert_eq!(report["provisioning"]["edge"]["configuration"], "generated");
		assert_eq!(
			report["provisioning"]["edge"]["route_runtime"],
			"not_checked"
		);
		assert_eq!(
			report["provisioning"]["application_lifecycle"],
			"not_checked"
		);
		generate(root.path(), &config, false).unwrap();
		assert_eq!(std::fs::read(&main).unwrap(), original);
		assert_eq!(std::fs::read(&edge).unwrap(), edge_original);
		std::fs::write(&edge, "operator edge edits\n").unwrap();
		assert!(generate(root.path(), &config, false).is_err());
		assert_eq!(
			std::fs::read_to_string(&edge).unwrap(),
			"operator edge edits\n"
		);
		std::fs::write(&edge, &edge_original).unwrap();
		std::fs::write(&manifest, "operator edits\n").unwrap();
		config.frontends.get_mut("docs").unwrap().mount = "/reference/".into();
		assert!(generate(root.path(), &config, false).is_err());
		assert_eq!(std::fs::read(&main).unwrap(), original);
		assert_eq!(
			std::fs::read_to_string(&manifest).unwrap(),
			"operator edits\n"
		);
		generate(root.path(), &config, true).unwrap();
		assert!(
			std::fs::read_to_string(&manifest)
				.unwrap()
				.contains("/reference/")
		);
		assert!(!directory.join("plugins.json").exists());
		assert!(
			std::fs::read_to_string(&edge)
				.unwrap()
				.contains("/reference/")
		);
	}

	#[test]
	fn invalid_graph_writes_no_provider_configuration() {
		let mut config = website();
		config.frontends.get_mut("docs").unwrap().host = "undeclared".into();
		let root = tempfile::tempdir().unwrap();
		assert!(generate(root.path(), &config, false).is_err());
		assert!(!root.path().join("infra").exists());
	}

	#[test]
	fn docs_report_preserves_ownership_without_selecting_the_api() {
		let report = report(&website(), &["frontends.docs".into()]).unwrap();
		assert_eq!(report["selection"]["stages"], json!([["frontends.docs"]]));
		assert_eq!(report["selection"]["artifacts"], json!(["public-site"]));
		let site = report["trees"]
			.as_array()
			.unwrap()
			.iter()
			.find(|tree| tree["unit"] == "frontends.site")
			.unwrap();
		assert_eq!(site["excluded_subtrees"], json!(["docs", "play"]));
		assert_eq!(report["evidence"]["runtime"], "not_checked");
		let compiler = report["hosts"]
			.as_array()
			.unwrap()
			.iter()
			.find(|host| host["name"] == "compiler")
			.unwrap();
		assert_eq!(compiler["requested_container_runtime"], "rootless-podman");

		assert_eq!(compiler["exclusive"], true);
	}

	#[test]
	fn report_does_not_disclose_service_environment_values() {
		let mut config = website();
		config
			.services
			.get_mut("play-api")
			.unwrap()
			.env
			.insert("TEST_VALUE".into(), "do-not-print-this".into());
		let value = report(&config, &[]).unwrap().to_string();
		assert!(!value.contains("do-not-print-this"));
	}

	#[test]
	fn malformed_missing_and_duplicate_selections_fail() {
		for selected in [
			vec!["docs".into()],
			vec!["frontends.missing".into()],
			vec!["frontends.docs".into(), "frontends.docs".into()],
		] {
			assert!(report(&website(), &selected).is_err());
		}
	}
}
