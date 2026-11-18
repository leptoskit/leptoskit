//! Pure OpenTofu JSON generation. Provider credentials use provider environment
//! variables.
use serde_json::Value;
use serde_json::json;

use crate::Generated;
use crate::PlatformError;
use crate::config::EdgeMode;
use crate::config::Node;
use crate::config::PlatformConfig;
use crate::config::Pool;
use crate::config::Role;
use crate::topology::Machine;

// Exact versions exercised by lk infra's provider acceptance, documented in
// docs/agents/infrastructure-exploration.md. lk infra emits broader ~>
// constraints; this offline generator pins those already validated versions for
// reproducibility.
const HCLOUD_VERSION: &str = "1.70.2";
const CLOUDFLARE_VERSION: &str = "5.28.1";

/// Generate an independent OpenTofu root per cluster; no provider is contacted.
pub fn generate(config: &PlatformConfig) -> Result<Generated, PlatformError> {
	config.validate()?;
	let mut files = Generated::new();
	for (region, entry) in &config.regions {
		for (name, cluster) in &entry.clusters {
			let machines = crate::topology::machines(cluster, None)?;
			let mut doc = json!({"terraform":{"required_version":">= 1.10, < 2.0","required_providers":{}},"provider":{},"resource":{},"output":{}});
			if machines.iter().any(|m| m.node.cloud()) {
				doc["terraform"]["required_providers"]["hcloud"] =
					json!({"source":"hetznercloud/hcloud","version":HCLOUD_VERSION});
				doc["provider"]["hcloud"] = json!({});
			}
			if let Some(cidr) = &cluster.private_cidr {
				doc["resource"]["hcloud_network"]["cluster"] = json!({"name":format!("{}-{region}-{name}",config.name),"ip_range":cidr,"lifecycle":{"prevent_destroy":true}});
				doc["resource"]["hcloud_network_subnet"]["cluster"] = json!({"network_id":"${hcloud_network.cluster.id}","type":"cloud","network_zone":cluster.network_zone,"ip_range":cidr});
			}
			let mut addresses = serde_json::Map::new();
			for machine in &machines {
				addresses.insert(machine.id.clone(), json!(machine.public));
				cloud_node(&mut doc, config, machine, &machines);
			}
			doc["output"]["node_addresses"] = json!({"value":addresses});
			if region == &config.default_region && name == &config.edge.cluster {
				edge(&mut doc, config, &machines)?;
			}
			files.insert(
				format!("{region}/{name}/main.tf.json"),
				format!("{}\n", serde_json::to_string_pretty(&doc)?),
			);
		}
	}
	Ok(files)
}

fn cloud_node(
	doc: &mut Value,
	config: &PlatformConfig,
	machine: &Machine<'_>,
	peers: &[Machine<'_>],
) {
	let Node::HetznerCloud {
		server_type,
		location,
		ssh_key_ids,
		..
	} = machine.node
	else {
		return;
	};
	let id = machine.id.replace('-', "_");
	// Attach the peer-dependent firewall separately, avoiding a server/firewall
	// dependency cycle. Initial cloud-init installs an admin-only host
	// firewall.
	doc["resource"]["hcloud_server"][&id] = json!({"name":machine.id,"server_type":server_type,"location":location,"image":"ubuntu-24.04","ssh_keys":ssh_key_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),"public_net":[{"ipv4_enabled":true,"ipv6_enabled":false}],"lifecycle":{"prevent_destroy":true},"user_data":initial_cloud_init(config)});
	if let Some(private) = &machine.private {
		doc["resource"]["hcloud_server"][&id]["network"] =
			json!([{"network_id":"${hcloud_network.cluster.id}","ip":private}]);
		doc["resource"]["hcloud_server"][&id]["depends_on"] =
			json!(["hcloud_network_subnet.cluster"]);
	}
	doc["resource"]["hcloud_firewall"][&id] =
		json!({"name":format!("{}-peers",machine.id),"rule":rules(config,machine,peers)});
	doc["resource"]["hcloud_firewall_attachment"][&id] = json!({"firewall_id":format!("${{hcloud_firewall.{id}.id}}"),"server_ids":[format!("${{hcloud_server.{id}.id}}") ]});
}

pub(crate) fn initial_cloud_init(config: &PlatformConfig) -> String {
	let nft = format!(
		"table inet leptoskit {{ chain input {{ type filter hook input priority 0; policy drop; \
		 ct state established,related accept; iifname lo accept; ip saddr {{ {} }} tcp dport 22 \
		 accept; }} }}",
		config
			.admin_cidrs
			.iter()
			.filter(|s| !s.contains(':'))
			.cloned()
			.collect::<Vec<_>>()
			.join(", ")
	);
	format!(
		"#cloud-config\n{}\n",
		json!({"ssh_pwauth":false,"package_update":true,"packages":["ca-certificates","curl","unattended-upgrades","nftables","zstd"],"write_files":[{"path":"/etc/ssh/sshd_config.d/00-leptoskit.conf","content":"PasswordAuthentication no\nPermitRootLogin prohibit-password\n"},{"path":"/etc/nftables.conf","content":nft}],"runcmd":["sshd -t","systemctl restart ssh","systemctl enable --now nftables","systemctl enable --now unattended-upgrades"]})
	)
}

pub(crate) fn rules(
	config: &PlatformConfig,
	machine: &Machine<'_>,
	peers: &[Machine<'_>],
) -> Vec<Value> {
	let mut rules = vec![rule("tcp", "22", config.admin_cidrs.clone())];
	if machine.node.role() == Role::Server {
		rules.push(rule("tcp", "6443", config.admin_cidrs.clone()));
	}
	for peer in peers.iter().filter(|p| p.id != machine.id) {
		rules.push(rule("udp", "51820", vec![host_cidr(&peer.public)]));
		if machine.node.role() == Role::Server {
			let address = if peer.node.role() == Role::Server {
				peer.private.as_ref().unwrap_or(&peer.public)
			} else {
				&peer.public
			};
			rules.push(rule("tcp", "6443", vec![host_cidr(address)]));
			if peer.node.role() == Role::Server {
				for port in ["2379-2380", "10250"] {
					rules.push(rule("tcp", port, vec![host_cidr(address)]));
				}
			}
		}
	}
	if config.edge.mode == EdgeMode::Https && machine.node.pool() == Pool::Ingress {
		rules.push(rule("tcp", "443", config.edge.cloudflare_cidrs.clone()));
	}
	rules
}

fn rule(protocol: &str, port: &str, sources: Vec<String>) -> Value {
	json!({"direction":"in","protocol":protocol,"port":port,"source_ips":sources})
}
fn host_cidr(address: &str) -> String {
	format!("{address}/32")
}

fn edge(
	doc: &mut Value,
	config: &PlatformConfig,
	machines: &[Machine<'_>],
) -> Result<(), PlatformError> {
	doc["terraform"]["required_providers"]["cloudflare"] =
		json!({"source":"cloudflare/cloudflare","version":CLOUDFLARE_VERSION});
	doc["provider"]["cloudflare"] = json!({});
	let control = url::Url::parse(&config.control_origin)
		.map_err(|_| PlatformError::Invalid("invalid origin".into()))?;
	let control = control
		.host_str()
		.ok_or_else(|| PlatformError::Invalid("origin requires a hostname".into()))?;
	let domains = [control.to_owned(), format!("*.{}", config.apps_domain)];
	let tunnel = config.edge.mode == EdgeMode::Tunnel;
	let content = if tunnel {
		doc["resource"]["cloudflare_zero_trust_tunnel_cloudflared"]["platform"] = json!({"account_id":config.edge.account_id,"name":config.name,"config_src":"cloudflare"});
		doc["resource"]["cloudflare_zero_trust_tunnel_cloudflared_config"]["platform"] = json!({"account_id":config.edge.account_id,"tunnel_id":"${cloudflare_zero_trust_tunnel_cloudflared.platform.id}","config":{"ingress":[{"hostname":control,"service":"http://traefik.kube-system.svc.cluster.local:80"},{"hostname":domains[1],"service":"http://traefik.kube-system.svc.cluster.local:80"},{"service":"http_status:404"}]}});
		doc["data"]["cloudflare_zero_trust_tunnel_cloudflared_token"]["platform"] = json!({"account_id":config.edge.account_id,"tunnel_id":"${cloudflare_zero_trust_tunnel_cloudflared.platform.id}"});
		doc["output"]["tunnel_token"] = json!({"value":"${data.cloudflare_zero_trust_tunnel_cloudflared_token.platform.token}","sensitive":true});
		"${cloudflare_zero_trust_tunnel_cloudflared.platform.id}.cfargotunnel.com".into()
	} else {
		machines
			.iter()
			.find(|m| m.node.pool() == Pool::Ingress)
			.ok_or_else(|| {
				PlatformError::Invalid(
					"HTTPS requires an ingress-pool node in the edge cluster".into(),
				)
			})?
			.public
			.clone()
	};
	for (index, domain) in domains.iter().enumerate() {
		doc["resource"]["cloudflare_dns_record"][format!("domain_{index}")] = json!({"zone_id":config.edge.zone_id,"name":domain,"type":if tunnel {"CNAME"} else {"A"},"content":content,"ttl":1,"proxied":true});
	}
	Ok(())
}
