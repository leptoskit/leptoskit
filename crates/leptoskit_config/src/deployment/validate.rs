//! Check each independent infrastructure contract before graph normalization.

use super::Host;
use super::InfraGraph;
use super::Ingress;
use super::OriginFirewall;

const MAX_HOSTS: usize = 64;
const CLOUDFLARE_ID_LENGTH: usize = 32;
const CURRENCY_LENGTH: usize = 3;

pub(super) fn check(graph: &InfraGraph) -> Result<(), String> {
	identity(graph)?;
	hosts(graph)?;
	edge(graph)?;
	budget(graph)?;
	state(graph)
}

fn identity(graph: &InfraGraph) -> Result<(), String> {
	if !crate::graph::identifier(&graph.name) {
		return Err("infra.name must be a valid graph resource name".into());
	}

	for value in [&graph.account_id, &graph.zone_id] {
		if value.len() != CLOUDFLARE_ID_LENGTH
			|| !value.bytes().all(|byte| byte.is_ascii_hexdigit())
		{
			return Err("infra.account_id and zone_id must be 32 hexadecimal characters".into());
		}
	}

	Ok(())
}

fn hosts(graph: &InfraGraph) -> Result<(), String> {
	if graph.hosts.is_empty() || graph.hosts.len() > MAX_HOSTS {
		return Err(format!(
			"infra.hosts must declare between 1 and {MAX_HOSTS} hosts"
		));
	}

	if graph.ssh_cidrs.is_empty() || graph.ssh_cidrs.iter().any(|value| !ssh_cidr(value)) {
		return Err("infra.ssh_cidrs needs administrator CIDRs with nonzero prefixes".into());
	}

	let first = graph.hosts.values().next().map(|host| host.compute);
	let mixed = graph.hosts.values().any(|host| Some(host.compute) != first);

	for (name, host) in &graph.hosts {
		host_settings(name, host, graph, mixed)?;
	}

	Ok(())
}

// Names are interpolated into provider resources; SSH identifiers must also
// stay in their provider namespace when the project mixes compute providers.
fn host_settings(name: &str, host: &Host, graph: &InfraGraph, mixed: bool) -> Result<(), String> {
	if !crate::graph::identifier(name)
		|| !provider_name(&host.location)
		|| !provider_name(&host.server_type)
	{
		return Err(format!(
			"infra.hosts.{name} needs a valid name, location and server_type"
		));
	}

	// Provider key IDs are separate namespaces. Reusing the common numeric
	// IDs across providers could authorize a different key on another host.
	if host.ssh_key_ids.is_empty() && (graph.ssh_key_ids.is_empty() || mixed) {
		return Err(format!(
			"infra.hosts.{name} needs SSH key IDs for its provider; mixed providers require \
			 per-host ssh_key_ids"
		));
	}

	if host
		.ssh_key_ids
		.iter()
		.chain(&graph.ssh_key_ids)
		.any(|key| *key == 0)
	{
		return Err(format!("infra.hosts.{name} SSH key IDs must be positive"));
	}

	Ok(())
}

fn edge(graph: &InfraGraph) -> Result<(), String> {
	let Some(edge) = &graph.edge else {
		return Ok(());
	};

	let expected = match edge.ingress {
		Ingress::CloudflareHttps => OriginFirewall::CloudflareOnly,
		Ingress::CloudflareTunnel => OriginFirewall::Closed,
	};

	if !edge.proxied || edge.origin_firewall != expected {
		return Err(
			"infra.edge must use proxied DNS and the firewall matching its HTTPS or tunnel ingress"
				.into(),
		);
	}

	Ok(())
}

fn budget(graph: &InfraGraph) -> Result<(), String> {
	let Some(budget) = &graph.budget else {
		return Ok(());
	};
	let valid_amount = budget.monthly_compute.is_finite() && budget.monthly_compute > 0.0;
	let valid_currency = budget.currency.len() == CURRENCY_LENGTH
		&& budget
			.currency
			.bytes()
			.all(|byte| byte.is_ascii_uppercase());

	if !valid_amount || !valid_currency {
		return Err(
			"infra.budget needs a positive compute amount and uppercase three-letter currency"
				.into(),
		);
	}

	Ok(())
}

fn state(graph: &InfraGraph) -> Result<(), String> {
	let Some(state) = &graph.state else {
		return Ok(());
	};
	let fields = [&state.bucket, &state.key, &state.region];

	if fields
		.iter()
		.any(|value| value.is_empty() || value.bytes().any(|byte| byte.is_ascii_control()))
	{
		return Err("infra.state needs a bucket, key and region without control characters".into());
	}

	Ok(())
}

fn ssh_cidr(value: &str) -> bool {
	let Some((address, prefix)) = value.split_once('/') else {
		return false;
	};
	let (Ok(address), Ok(prefix)) = (address.parse::<std::net::IpAddr>(), prefix.parse::<u8>())
	else {
		return false;
	};

	let bits = match address {
		std::net::IpAddr::V4(_) => u32::BITS,
		std::net::IpAddr::V6(_) => u128::BITS,
	};

	prefix > 0 && u32::from(prefix) <= bits
}

fn provider_name(value: &str) -> bool {
	!value.is_empty()
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || b"-._".contains(&byte))
}
