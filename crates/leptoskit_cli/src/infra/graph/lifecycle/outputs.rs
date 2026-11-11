use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Host {
	provider: leptoskit_config::infra::ComputeProvider,
	location: String,
	server_type: String,
	ipv4: Ipv4Addr,
	ipv6: Ipv6Addr,
}

pub(super) fn read(
	dir: &Path,
	config: &LeptoskitConfig,
	engine: &impl super::Engine,
) -> Result<Value> {
	super::inputs::generated(dir, config)?;
	read_bound(dir, config, engine)
}

/// Caller has checked either current generated inputs or independently pinned
/// historical recovery inputs. Keep provider identity/output checks identical.
pub(super) fn read_bound(
	dir: &Path,
	config: &LeptoskitConfig,
	engine: &impl super::Engine,
) -> Result<Value> {
	super::identity(dir, engine)?;
	super::workspace(dir, engine)?;
	let hosts = engine.invoke(dir, &["output", "-json", "hosts"], true)?;
	let domains = engine.invoke(dir, &["output", "-json", "domains_by_host"], true)?;
	checked(config, &hosts, &domains)
}

fn checked(config: &LeptoskitConfig, hosts: &[u8], domains: &[u8]) -> Result<Value> {
	let infra = super::settings(config)?;
	let hosts: BTreeMap<String, Host> = serde_json::from_slice(hosts)
		.map_err(|_| anyhow::anyhow!("invalid public host outputs; raw values were not logged"))?;
	let domains: BTreeMap<String, BTreeSet<String>> =
		serde_json::from_slice(domains).map_err(|_| {
			anyhow::anyhow!("invalid public domain outputs; raw values were not logged")
		})?;
	ensure!(
		hosts.keys().eq(infra.hosts.keys()),
		"applied host names differ from the graph declaration"
	);
	let plan = leptoskit_config::deployment::compile(config)
		.map_err(anyhow::Error::msg)?
		.context("graph declaration required")?;
	let mut expected = BTreeMap::<String, BTreeSet<String>>::new();

	for route in plan.routes().entries() {
		expected
			.entry(route.host.clone())
			.or_default()
			.insert(route.domain.clone());
	}

	ensure!(
		domains == expected,
		"applied domains differ from the graph declaration"
	);
	let mut public = BTreeMap::new();

	for (name, host) in hosts {
		let declared = &infra.hosts[&name];
		ensure!(
			host.provider == declared.compute
				&& host.location == declared.location
				&& host.server_type == declared.server_type,
			"applied placement for host {name:?} differs from the graph declaration"
		);
		ensure!(
			public_ipv4(host.ipv4) && public_ipv6(host.ipv6),
			"invalid applied address for host {name:?}"
		);
		public.insert(
			name,
			json!({
				"provider": host.provider,
				"location": host.location,
				"server_type": host.server_type,
				"ipv4": host.ipv4,
				"ipv6": host.ipv6
			}),
		);
	}

	Ok(json!({
		"format_version": 1,
		"kind": "leptoskit-graph-infrastructure-output",
		"hosts": public,
		"domains_by_host": domains,
		"evidence": {
			"declarations": "checked",
			"provider_state": "observed",
			"host_reachability": "not_checked",
			"application_runtime": "not_checked",
			"runtime_attestation": "absent"
		}
	}))
}

fn public_ipv4(address: Ipv4Addr) -> bool {
	!address.is_unspecified()
		&& !address.is_loopback()
		&& !address.is_multicast()
		&& !address.is_broadcast()
		&& !address.is_private()
		&& !address.is_link_local()
}

fn public_ipv6(address: Ipv6Addr) -> bool {
	!address.is_unspecified()
		&& !address.is_loopback()
		&& !address.is_multicast()
		&& !address.is_unique_local()
		&& !address.is_unicast_link_local()
		&& address.to_ipv4().is_none()
}

#[cfg(test)]
mod tests;
