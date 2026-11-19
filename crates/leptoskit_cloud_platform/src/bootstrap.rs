//! Address-dependent node documents. Tokens are supplied as root-only node
//! files.
use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::Value;
use serde_json::json;

use crate::Generated;
use crate::PlatformError;
use crate::config::Cluster;
use crate::config::GvisorPlatform;
use crate::config::PlatformConfig;
use crate::config::Pool;
use crate::config::Role;
use crate::topology::Machine;

// https://github.com/k3s-io/k3s/releases/download/v1.35.10%2Bk3s1/sha256sum-amd64.txt
const K3S_SHA256: &str = "5cd49f0faca5b2ea4355d4761e7b1445b8aa05cd50e612fd9c370551db49e5e2";

/// Render shell documents for existing nodes and cloud-init wrappers for cloud
/// nodes. A single cluster accepts normal `tofu output -json`; multiple
/// clusters accept an object keyed by `region/cluster`, each containing those
/// outputs.
pub fn generate(
	config: &PlatformConfig,
	outputs: Option<&str>,
) -> Result<Generated, PlatformError> {
	config.validate()?;
	// Never retain or echo arbitrary outputs: they may include tunnel tokens.
	let outputs: Option<Value> = outputs
		.map(serde_json::from_str)
		.transpose()
		.map_err(|_| PlatformError::Invalid("invalid OpenTofu outputs JSON".into()))?;
	let mut files = Generated::new();
	for (region, entry) in &config.regions {
		for (name, cluster) in &entry.clusters {
			let key = format!("{region}/{name}");
			let selected = outputs.as_ref().map(|v| v.get(&key).unwrap_or(v));
			let addresses: Option<BTreeMap<String, String>> = selected
				.map(|v| serde_json::from_value(v["node_addresses"]["value"].clone()))
				.transpose()
				.map_err(|_| {
					PlatformError::Invalid("outputs require node_addresses.value map".into())
				})?;
			if cluster.nodes.values().any(crate::config::Node::cloud) && addresses.is_none() {
				return Err(PlatformError::Invalid(
					"cloud nodes require tofu outputs; apply phase one first".into(),
				));
			}
			let machines = crate::topology::machines(cluster, addresses.as_ref())?;
			let first = machines
				.iter()
				.find(|m| m.node.role() == Role::Server)
				.ok_or_else(|| PlatformError::Invalid("missing server".into()))?;
			for machine in &machines {
				let script = script(config, cluster, machine, first, &machines)?;
				files.insert(format!("{key}/{}.sh", machine.id), script.clone());
				if machine.node.cloud() {
					files.insert(format!("{key}/{}.cloud-init.yaml",machine.id),format!("#cloud-config\n{}\n",serde_json::to_string_pretty(&json!({"write_files":[{"path":"/usr/local/sbin/leptoskit-bootstrap","permissions":"0700","content":script}],"runcmd":[["/usr/local/sbin/leptoskit-bootstrap"]]}))?));
				}
			}
		}
	}
	Ok(files)
}

fn script(
	config: &PlatformConfig,
	cluster: &Cluster,
	machine: &Machine<'_>,
	first: &Machine<'_>,
	peers: &[Machine<'_>],
) -> Result<String, PlatformError> {
	let server = machine.node.role() == Role::Server;
	let role = if server { "server" } else { "agent" };
	let pool = match machine.node.pool() {
		Pool::Tenant => "tenant",
		Pool::Platform => "platform",
		Pool::Build => "build",
		Pool::Ingress => "ingress",
	};
	let effective_platform = machine.node.isolation_platform(config.isolation.platform);
	let platform = match effective_platform {
		GvisorPlatform::Systrap => "systrap",
		GvisorPlatform::Kvm => "kvm",
	};
	// Build nodes are always NoSchedule. Platform nodes become NoSchedule when
	// dedicated tenant nodes exist so untrusted tenant pods cannot land on
	// control-plane VMs.
	let has_dedicated_tenant_pool = cluster.nodes.values().any(|n| n.pool() == Pool::Tenant);
	let effect = if machine.node.pool() == Pool::Build
		|| (machine.node.pool() == Pool::Platform && has_dedicated_tenant_pool)
	{
		"NoSchedule"
	} else {
		"PreferNoSchedule"
	};
	let mut settings = json!({"node-name":machine.id,"node-ip":machine.private.as_ref().unwrap_or(&machine.public),"node-external-ip":machine.public,"token-file":"/etc/rancher/k3s/join-token","node-label":[format!("leptoskit.dev/pool={pool}")],"node-taint":[format!("leptoskit.dev/pool={pool}:{effect}")]});
	let registration = cluster
		.registration_address
		.as_ref()
		.unwrap_or(&first.public);
	if server {
		// The add-on slice owns pinned Traefik and ingress placement. Disable
		// ServiceLB to avoid arbitrary host-port listeners. Keep bundled
		// metrics-server enabled so CPU HPAs (`max > min`) have metrics.k8s.io.
		settings["disable"] = json!(["traefik", "servicelb"]);
		settings["secrets-encryption"] = json!(true);
		settings["cluster-cidr"] = json!(cluster.pod_cidr);
		settings["service-cidr"] = json!(cluster.service_cidr);
		settings["flannel-backend"] = json!("wireguard-native");
		settings["flannel-external-ip"] = json!(true);
		// When existing agents outside the cloud private network join the
		// cluster, advertise the public address so kubernetes.default.svc is
		// routable from tenant and build nodes.
		let has_external_agents = peers
			.iter()
			.any(|peer| peer.node.role() == Role::Agent && !peer.node.cloud());
		let advertise = if has_external_agents {
			&machine.public
		} else {
			machine.private.as_ref().unwrap_or(&machine.public)
		};
		settings["advertise-address"] = json!(advertise);
		let mut sans = vec![registration.clone()];
		if !sans.contains(&machine.public) {
			sans.push(machine.public.clone());
		}
		if let Some(private) = &machine.private
			&& !sans.contains(private)
		{
			sans.push(private.clone());
		}
		settings["tls-san"] = json!(sans);
		if machine.id == first.id {
			settings["cluster-init"] = json!(true);
		}
	}
	if machine.id != first.id {
		// Server join uses the first private address, even with a public DNS
		// registration name; embedded etcd may never depend on the public
		// overlay.
		let target = if server
			|| (machine.private.is_some() && machine.node.cloud() && first.node.cloud())
		{
			first.private.as_ref().unwrap_or(registration)
		} else {
			registration
		};
		settings["server"] = json!(format!("https://{target}:6443"));
	}
	let firewall = firewall(config, machine, peers)?;
	let configuration = serde_json::to_string_pretty(&settings)?;
	let runtime = "{{ template \"base\" . \
	               }}\n[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runsc]\n  \
	               runtime_type = \
	               \"io.containerd.runsc.v1\"\n[plugins.'io.containerd.cri.v1.runtime'.containerd.\
	               runtimes.runsc.options]\n  TypeUrl = \"io.containerd.runsc.v1.options\"\n  \
	               ConfigPath = \"/etc/containerd/runsc.toml\"\n"
		.to_string();
	let runsc = format!("[runsc_config]\n  platform = \"{platform}\"\n");
	let unit = format!(
		r"[Unit]
Description=LeptosKit k3s {role}
Wants=network-online.target
After=network-online.target nftables.service
ConditionPathExists=/etc/rancher/k3s/join-token
[Service]
Type=notify
ExecStart=/usr/local/bin/k3s {role}
Restart=always
RestartSec=5
KillMode=process
Delegate=yes
LimitNOFILE=1048576
TasksMax=infinity
[Install]
WantedBy=multi-user.target
"
	);
	let release = &config.isolation.release;
	let checksum = &config.isolation.sha512;
	let version = &cluster.version;
	let mut script = String::from(
		"#!/bin/sh\nset -eu\numask 077\n# Supply a shared join token in \
		 /etc/rancher/k3s/join-token before running.\ntest -s /etc/rancher/k3s/join-token\ntest \
		 \"$(uname -m)\" = x86_64\n. /etc/os-release\ntest \"$ID:$VERSION_ID\" = \
		 ubuntu:24.04\nchmod 600 /etc/rancher/k3s/join-token\napt-get \
		 update\nDEBIAN_FRONTEND=noninteractive apt-get install -y ca-certificates curl nftables \
		 unattended-upgrades zstd\nmodprobe wireguard\nmodprobe br_netfilter\nprintf \
		 'net.ipv4.ip_forward=1\\n' > /etc/sysctl.d/90-leptoskit.conf\nsysctl --system\nmkdir -p \
		 /etc/rancher/k3s /var/lib/rancher/k3s/agent/etc/containerd /etc/containerd \
		 /etc/ssh/sshd_config.d\n",
	);
	// k3s loads this operator-owned file into containerd on startup:
	// https://docs.k3s.io/installation/private-registry. Never render auth.
	match &config.registry {
		crate::config::Registry::Zot { .. } => {
			script.push_str(
				"test -s /etc/rancher/k3s/registries.yaml\nchmod 600 \
				 /etc/rancher/k3s/registries.yaml\n",
			);
		}
		crate::config::Registry::External { .. } => {
			script.push_str(
				"if [ -s /etc/rancher/k3s/registries.yaml ]; then chmod 600 \
				 /etc/rancher/k3s/registries.yaml; fi\n",
			);
		}
	}
	if effective_platform == GvisorPlatform::Kvm {
		script.push_str("test -c /dev/kvm\n");
	}
	for (path, content) in [
		(
			"/etc/ssh/sshd_config.d/00-leptoskit.conf",
			"PasswordAuthentication no\nPermitRootLogin prohibit-password\n",
		),
		("/etc/nftables.conf", firewall.as_str()),
		("/etc/rancher/k3s/config.yaml", configuration.as_str()),
		(
			"/var/lib/rancher/k3s/agent/etc/containerd/config-v3.toml.tmpl",
			runtime.as_str(),
		),
		("/etc/containerd/runsc.toml", runsc.as_str()),
		("/etc/systemd/system/leptoskit-k3s.service", unit.as_str()),
	] {
		write!(
			script,
			"cat > {path} <<'LEPTOSKIT_DOCUMENT'\n{content}\nLEPTOSKIT_DOCUMENT\n"
		)?;
	}
	// Do not flush other nftables tables: kube-router owns NetworkPolicy rules.
	script.push_str(
		"nft -c -f /etc/nftables.conf\nnft -f /etc/nftables.conf\nsshd -t\nsystemctl restart \
		 ssh\nsystemctl enable nftables\nsystemctl enable --now \
		 unattended-upgrades\nwork=$(mktemp -d)\ntrap 'rm -rf \"$work\"' EXIT\ncd \"$work\"\n",
	);
	write!(script,"curl -fLsS 'https://github.com/k3s-io/k3s/releases/download/{version}/k3s' -o k3s\nprintf '%s  k3s\\n' '{K3S_SHA256}' | sha256sum -c -\ninstall -m 755 k3s /usr/local/bin/k3s\ncurl -fLsS 'https://storage.googleapis.com/gvisor/releases/release/{release}/x86_64/gvisor.tar.zstd' -o gvisor.tar.zstd\nprintf '%s  gvisor.tar.zstd\\n' '{checksum}' | sha512sum -c -\ntar --zstd -xf gvisor.tar.zstd -C /usr/local/bin\nsystemctl daemon-reload\nsystemctl enable leptoskit-k3s\nsystemctl restart leptoskit-k3s\n")?;
	Ok(script)
}

fn firewall(
	config: &PlatformConfig,
	machine: &Machine<'_>,
	peers: &[Machine<'_>],
) -> Result<String, PlatformError> {
	// Hetzner Cloud Firewalls do not filter private-network traffic, so host
	// input policy supplements them. Forwarding belongs to k3s/kube-router;
	// Flannel WireGuard and CNI input is already authenticated cluster traffic.
	// ICMP/ICMPv6 stays open so Path MTU Discovery works across WireGuard.
	let mut result = String::from(
		"add table inet leptoskit\nflush table inet leptoskit\ntable inet leptoskit {\n chain \
		 input {\n  type filter hook input priority 0; policy drop;\n  ct state \
		 established,related accept\n  iifname lo accept\n  ip protocol icmp accept\n  ip6 \
		 nexthdr icmpv6 accept\n  iifname { cni0, flannel-wg } accept\n",
	);
	for rule in crate::tofu::rules(config, machine, peers) {
		let protocol = rule["protocol"]
			.as_str()
			.ok_or_else(|| PlatformError::Invalid("missing rule protocol".into()))?;
		let port = rule["port"]
			.as_str()
			.ok_or_else(|| PlatformError::Invalid("missing rule port".into()))?;
		let sources = rule["source_ips"]
			.as_array()
			.ok_or_else(|| PlatformError::Invalid("missing rule peers".into()))?;
		for source in sources {
			let source = source
				.as_str()
				.ok_or_else(|| PlatformError::Invalid("missing peer address".into()))?;
			writeln!(
				result,
				"  {} saddr {source} {protocol} dport {port} accept",
				if source.contains(':') { "ip6" } else { "ip" }
			)?;
		}
	}
	result.push_str(" }\n}\n");
	Ok(result)
}
