//! Deterministic compute, origin firewall and DNS resources for named hosts.
//! Application installation and shared-zone rule adoption are separate gates.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::deployment::InfraGraph;
use leptoskit_config::deployment::Ingress;
use leptoskit_config::graph::plan::Plan;
use leptoskit_config::infra::ComputeProvider;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;
use sha2::Digest as _;

use super::network::Ranges;

const CLOUD_INIT: &str =
	"#cloud-config\nssh_pwauth: false\ndisable_root: false\npackage_update: true\npackages:\n  - \
	 ca-certificates\n  - unattended-upgrades\n  - python3\nwrite_files:\n  - path: \
	 /etc/ssh/sshd_config.d/00-leptoskit.conf\n    content: |\n      PasswordAuthentication no\n      \
	 PermitRootLogin prohibit-password\nruncmd:\n  - sshd -t\n  - systemctl restart ssh\n";

#[cfg(test)]
mod acceptance;
#[cfg(test)]
mod tests;

pub(super) fn resources(
	config: &InfraGraph,
	plan: &Plan,
	ranges: Option<&Ranges>,
) -> Result<Value> {
	let domains = domains(plan);
	let https = !domains.is_empty()
		&& config
			.edge
			.as_ref()
			.is_some_and(|edge| edge.ingress == Ingress::CloudflareHttps);
	ensure!(
		!https || ranges.is_some(),
		"HTTPS origins require a checked dual-stack Cloudflare range snapshot"
	);
	let mut result = json!({
		"terraform": {"required_version": ">= 1.10, < 2.0", "required_providers": {}},
		"provider": {}, "locals": {}, "resource": {}, "output": {}
	});
	let mut used_names = BTreeSet::new();
	let mut hcloud = Map::new();
	let mut digitalocean = Map::new();

	for (name, host) in &config.hosts {
		let resource_name = resource_name(&config.name, name);
		ensure!(
			used_names.insert(resource_name.clone()),
			"derived host name collision"
		);
		let keys = if host.ssh_key_ids.is_empty() {
			&config.ssh_key_ids
		} else {
			&host.ssh_key_ids
		};

		let public = domains.contains_key(name);
		let value = json!({"name": resource_name, "location": host.location,
			"server_type": host.server_type, "ssh_keys": keys.iter().map(ToString::to_string).collect::<Vec<_>>(),
			"public_https": public && https, "user_data":cloud_init(&config.name, name, plan, public)?});

		match host.compute {
			ComputeProvider::Hetzner => {
				hcloud.insert(name.clone(), value);
			}
			ComputeProvider::Digitalocean => {
				digitalocean.insert(name.clone(), value);
			}
		}
	}

	result["locals"]["ssh_cidrs"] = json!(config.ssh_cidrs);
	result["locals"]["origin_cidrs"] = json!(ranges.map_or_else(Vec::new, Ranges::all));

	if !hcloud.is_empty() {
		hetzner(&mut result, hcloud);
	}

	if !digitalocean.is_empty() {
		digitalocean_resources(&mut result, digitalocean);
	}

	if !domains.is_empty() {
		edge(&mut result, config, &domains, https)?;
	}

	if let Some(state) = &config.state {
		result["terraform"]["backend"] = json!({"s3": {"bucket": state.bucket,
			"key": state.key, "region": state.region, "encrypt": true, "use_lockfile": true}});
	}

	outputs(&mut result, config, &domains)?;
	Ok(result)
}

fn cloud_init(project: &str, host: &str, plan: &Plan, edge: bool) -> Result<String> {
	let mut accounts = BTreeSet::new();
	if edge {
		accounts.insert("leptoskit-edge".to_owned());
	}
	for placement in plan
		.placements()
		.values()
		.filter(|placement| placement.host == host)
	{
		accounts.insert(super::account(project, &placement.unit));
	}
	// JSON flow values are valid YAML; no unit name is interpolated as shell
	// code.
	let mut users = vec![json!("default")];
	users.extend(accounts.into_iter().map(|name| {
		json!({
			"name":name,"system":true,"lock_passwd":true,"no_create_home":true,
			"shell":"/usr/sbin/nologin"
		})
	}));
	Ok(format!(
		"{CLOUD_INIT}users: {}\n",
		serde_json::to_string(&users)?
	))
}

fn domains(plan: &Plan) -> BTreeMap<String, BTreeSet<String>> {
	let mut domains = BTreeMap::<String, BTreeSet<String>>::new();

	for route in plan.routes().entries() {
		domains
			.entry(route.host.clone())
			.or_default()
			.insert(route.domain.clone());
	}

	domains
}

fn provider(value: &mut Value, name: &str, source: &str, version: &str) {
	value["terraform"]["required_providers"][name] = json!({"source": source, "version": version});
	value["provider"][name] = json!({});
}

fn hetzner(value: &mut Value, hosts: Map<String, Value>) {
	provider(value, "hcloud", "hetznercloud/hcloud", "~> 1.0");
	value["locals"]["hetzner_hosts"] = Value::Object(hosts);
	value["resource"]["hcloud_firewall"] = json!({"host": {
		"for_each": "${local.hetzner_hosts}", "name": "${each.value.name}-origin",
		"dynamic": {"rule": {
			"for_each": "${concat([{port = \"22\", source_ips = local.ssh_cidrs}], each.value.public_https ? [{port = \"443\", source_ips = local.origin_cidrs}] : [])}",
			"content": {"direction": "in", "protocol": "tcp", "port": "${rule.value.port}", "source_ips": "${rule.value.source_ips}"}
		}}
	}});
	value["resource"]["hcloud_server"] = json!({"host": {
		"for_each": "${local.hetzner_hosts}", "name": "${each.value.name}", "image": "ubuntu-24.04",
		"location": "${each.value.location}", "server_type": "${each.value.server_type}",
		"ssh_keys": "${each.value.ssh_keys}", "firewall_ids": ["${hcloud_firewall.host[each.key].id}"],
		"public_net": [{"ipv4_enabled": true, "ipv6_enabled": true}],
		"lifecycle": {"prevent_destroy": true}, "user_data": "${each.value.user_data}"
	}});
}

fn digitalocean_resources(value: &mut Value, hosts: Map<String, Value>) {
	provider(value, "digitalocean", "digitalocean/digitalocean", "~> 2.0");
	value["locals"]["digitalocean_hosts"] = Value::Object(hosts);
	value["resource"]["digitalocean_droplet"] = json!({"host": {
		"for_each": "${local.digitalocean_hosts}", "name": "${each.value.name}", "image": "ubuntu-24-04-x64",
		"region": "${each.value.location}", "size": "${each.value.server_type}",
		"ssh_keys": "${each.value.ssh_keys}", "ipv6": true, "monitoring": true,
		"lifecycle": {"prevent_destroy": true}, "user_data": "${each.value.user_data}"
	}});
	value["resource"]["digitalocean_firewall"] = json!({"host": {
		"for_each": "${local.digitalocean_hosts}", "name": "${each.value.name}-origin",
		"droplet_ids": ["${digitalocean_droplet.host[each.key].id}"],
		"dynamic": {"inbound_rule": {
			"for_each": "${concat([{port = \"22\", addresses = local.ssh_cidrs}], each.value.public_https ? [{port = \"443\", addresses = local.origin_cidrs}] : [])}",
			"content": {"protocol": "tcp", "port_range": "${inbound_rule.value.port}", "source_addresses": "${inbound_rule.value.addresses}"}
		}},
		"outbound_rule": [
			{"protocol": "tcp", "port_range": "1-65535", "destination_addresses": ["0.0.0.0/0", "::/0"]},
			{"protocol": "udp", "port_range": "1-65535", "destination_addresses": ["0.0.0.0/0", "::/0"]},
			{"protocol": "icmp", "destination_addresses": ["0.0.0.0/0", "::/0"]}
		]
	}});
}

fn edge(
	value: &mut Value,
	config: &InfraGraph,
	domains: &BTreeMap<String, BTreeSet<String>>,
	https: bool,
) -> Result<()> {
	ensure!(
		config.edge.is_some(),
		"public graph routes require edge settings"
	);
	provider(value, "cloudflare", "cloudflare/cloudflare", "~> 5.0");
	value["locals"]["public_hosts"] = json!(domains);

	if https {
		https_edge(value, config, domains)?;
	} else {
		tunnel_edge(value, config, domains);
	}

	Ok(())
}

fn https_edge(
	value: &mut Value,
	config: &InfraGraph,
	domains: &BTreeMap<String, BTreeSet<String>>,
) -> Result<()> {
	let mut records = Map::new();

	for (host, domains) in domains {
		for domain in domains {
			records.insert(
				domain.clone(),
				json!({"ipv4": address(config, host, false)?, "ipv6": address(config, host, true)?}),
			);
		}
	}

	value["locals"]["public_domains"] = json!(records);

	for (name, kind, address) in [("ipv4", "A", "ipv4"), ("ipv6", "AAAA", "ipv6")] {
		value["resource"]["cloudflare_dns_record"][name] = json!({
			"for_each": "${local.public_domains}", "zone_id": config.zone_id,
			"name": "${each.key}", "type": kind, "ttl": 1, "proxied": true,
			"content": format!("${{each.value.{address}}}")
		});
	}

	// Only CSRs enter Terraform. Private keys belong to the CLI's private
	// origin-key store and must never be embedded in user_data or state.
	value["variable"]["origin_csrs"] = json!({"type": "map(string)",
		"description": "Public ECC CSRs prepared by lk for each public graph host; private keys stay outside Terraform."});
	value["resource"]["cloudflare_origin_ca_certificate"] = json!({"host": {
		"for_each": "${local.public_hosts}", "csr": "${var.origin_csrs[each.key]}",
		"hostnames": "${each.value}", "request_type": "origin-ecc", "requested_validity": 365
	}});
	value["output"]["origin_certificates"] = json!({"value": "${{for name, cert in cloudflare_origin_ca_certificate.host : name => {certificate = cert.certificate, expires_on = cert.expires_on}}}"});
	Ok(())
}

fn tunnel_edge(
	value: &mut Value,
	config: &InfraGraph,
	domains: &BTreeMap<String, BTreeSet<String>>,
) {
	let mut records = Map::new();

	for (host, domains) in domains {
		for domain in domains {
			records.insert(domain.clone(), json!(host));
		}
	}

	value["locals"]["public_domains"] = json!(records);
	value["resource"]["cloudflare_zero_trust_tunnel_cloudflared"] = json!({"host": {
		"for_each": "${local.public_hosts}", "account_id": config.account_id,
		"name": format!("{}-${{each.key}}", config.name), "config_src": "cloudflare"
	}});
	value["resource"]["cloudflare_zero_trust_tunnel_cloudflared_config"] = json!({"host": {
		"for_each": "${local.public_hosts}", "account_id": config.account_id,
		"tunnel_id": "${cloudflare_zero_trust_tunnel_cloudflared.host[each.key].id}",
		"config": {"ingress": "${concat([for domain in each.value : {hostname = domain, service = \"http://127.0.0.1:8088\"}], [{service = \"http_status:404\"}])}"}
	}});
	value["resource"]["cloudflare_dns_record"] = json!({"tunnel": {
		"for_each": "${local.public_domains}", "zone_id": config.zone_id, "name": "${each.key}",
		"type": "CNAME", "ttl": 1, "proxied": true,
		"content": "${cloudflare_zero_trust_tunnel_cloudflared.host[each.value].id}.cfargotunnel.com"
	}});

	value["data"]["cloudflare_zero_trust_tunnel_cloudflared_token"] = json!({"host": {
		"for_each": "${local.public_hosts}", "account_id": config.account_id,
		"tunnel_id": "${cloudflare_zero_trust_tunnel_cloudflared.host[each.key].id}"
	}});
	value["output"]["tunnel_tokens"] = json!({"value": "${{for name, token in data.cloudflare_zero_trust_tunnel_cloudflared_token.host : name => token.token}}", "sensitive": true});
}

fn address(config: &InfraGraph, name: &str, ipv6: bool) -> Result<String> {
	let host = config
		.hosts
		.get(name)
		.context("route names an undeclared host")?;

	let resource = match host.compute {
		ComputeProvider::Hetzner => "hcloud_server",
		ComputeProvider::Digitalocean => "digitalocean_droplet",
	};
	let attribute = if ipv6 { "ipv6_address" } else { "ipv4_address" };
	Ok(format!(
		"${{{resource}.host[{}].{attribute}}}",
		serde_json::to_string(name)?
	))
}

fn outputs(
	value: &mut Value,
	config: &InfraGraph,
	domains: &BTreeMap<String, BTreeSet<String>>,
) -> Result<()> {
	let hosts = config
		.hosts
		.iter()
		.map(|(name, host)| {
			Ok((
				name.clone(),
				json!({
					"provider": host.compute, "location": host.location, "server_type": host.server_type,
					"ipv4": address(config, name, false)?, "ipv6": address(config, name, true)?
				}),
			))
		})
		.collect::<Result<Map<_, _>>>()?;
	value["output"]["hosts"] = json!({"value": hosts});
	value["output"]["domains_by_host"] = json!({"value": domains});
	Ok(())
}

fn resource_name(project: &str, host: &str) -> String {
	let digest = sha2::Sha256::digest(format!("{project}\0{host}").as_bytes());
	let short = |value: &str, limit| {
		value
			.chars()
			.take(limit)
			.map(|c| {
				if c == '_' {
					'-'
				} else {
					c.to_ascii_lowercase()
				}
			})
			.collect::<String>()
	};
	format!(
		"{}-{}-{}",
		short(project, 16),
		short(host, 24),
		hex::encode(&digest[..6])
	)
}
