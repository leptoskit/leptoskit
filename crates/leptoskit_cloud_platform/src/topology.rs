//! Stable node expansion shared by both generation phases.
use std::collections::BTreeMap;

use crate::PlatformError;
use crate::config::Cluster;
use crate::config::Node;
use crate::config::require;

pub(crate) struct Machine<'a> {
	pub id: String,
	pub node: &'a Node,
	pub public: String,
	pub private: Option<String>,
}

pub(crate) fn machines<'a>(
	cluster: &'a Cluster,
	outputs: Option<&BTreeMap<String, String>>,
) -> Result<Vec<Machine<'a>>, PlatformError> {
	let cloud_servers = cluster
		.nodes
		.values()
		.any(|n| n.cloud() && n.role() == crate::config::Role::Server);
	let mut result = Vec::new();
	let mut index = 10_u32;
	for (group, node) in &cluster.nodes {
		for number in 0..node.count() {
			let id = format!("{group}-{number}");
			let (public, private) = match node {
				Node::Existing {
					public_address,
					private_address,
					..
				} => {
					(
						public_address.to_string(),
						private_address.map(|v| v.to_string()),
					)
				}
				Node::HetznerCloud { .. } => {
					let public = if let Some(outputs) = outputs {
						outputs.get(&id).cloned().ok_or_else(|| {
							PlatformError::Invalid(format!(
								"missing public address output for {id}"
							))
						})?
					} else {
						format!("${{hcloud_server.{}.ipv4_address}}", id.replace('-', "_"))
					};
					let private = if cloud_servers {
						let cidr = cluster.private_cidr.as_deref().ok_or_else(|| {
							PlatformError::Invalid("missing private network".into())
						})?;
						let (address, _) = crate::config::cidr(cidr)?;
						let std::net::IpAddr::V4(address) = address else {
							return Err(PlatformError::Invalid(
								"private network must be IPv4".into(),
							));
						};
						Some(std::net::Ipv4Addr::from(u32::from(address) + index).to_string())
					} else {
						None
					};
					(public, private)
				}
			};
			if outputs.is_some() {
				require(
					public.parse::<std::net::Ipv4Addr>().is_ok(),
					"node output must be an IPv4 address",
				)?;
			}
			result.push(Machine {
				id,
				node,
				public,
				private,
			});
			index += 1;
		}
	}
	Ok(result)
}
