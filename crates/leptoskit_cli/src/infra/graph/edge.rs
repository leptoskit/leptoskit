//! Managed edge configuration. Rendering does not install certificates,
//! publish a release tree, or attest to a running origin.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::deployment::FrontendKind;
use leptoskit_config::deployment::InfraGraph;
use leptoskit_config::deployment::Ingress;
use leptoskit_config::graph::Unit;
use leptoskit_config::graph::plan::Plan;
use leptoskit_config::services::ServiceTrust;
use serde_json::Value;
use serde_json::json;

use super::network::Ranges;

mod routes;
#[cfg(test)]
mod tests;

pub(super) const CADDY_VERSION: &str = "2.11.9";

enum Target {
	Static(PathBuf),
	Proxy { port: u16, isolated: bool },
}

/// Endpoints are explicit so a later release can switch a backend to a new
/// loopback port without guessing its container identity. Static roots must
/// contain only the checked public artifact tree.
struct Bindings {
	targets: BTreeMap<Unit, Target>,
	certificates: PathBuf,
}

pub(super) fn configurations(
	config: &LeptoskitConfig,
	plan: &Plan,
	ranges: Option<&Ranges>,
) -> Result<BTreeMap<String, Value>> {
	let infra = config
		.infra
		.as_ref()
		.and_then(leptoskit_config::infra::InfraConfig::as_graph)
		.context("graph infrastructure required")?;
	let hosts: BTreeSet<_> = plan
		.routes()
		.entries()
		.iter()
		.map(|route| &route.host)
		.collect();
	hosts
		.into_iter()
		.map(|host| {
			let bindings = declared_bindings(config, plan, infra, host)?;
			Ok((host.clone(), render(infra, plan, host, ranges, &bindings)?))
		})
		.collect()
}

fn declared_bindings(
	config: &LeptoskitConfig,
	plan: &Plan,
	infra: &InfraGraph,
	host: &str,
) -> Result<Bindings> {
	let mut targets = BTreeMap::new();
	for route in plan
		.routes()
		.entries()
		.iter()
		.filter(|route| route.host == host)
	{
		let target = match &route.unit {
			Unit::Frontend(name) => {
				let frontend = &infra.frontends[name];
				match frontend.kind {
					FrontendKind::Static => {
						Target::Static(PathBuf::from(format!(
							"/var/lib/leptoskit/{}/units/{}/current/assets",
							infra.name, route.unit
						)))
					}
					FrontendKind::Ssr => {
						Target::Proxy {
							port: frontend
								.serve
								.as_ref()
								.context("SSR serve contract missing")?
								.port,
							isolated: false,
						}
					}
				}
			}
			Unit::Service(name) => {
				Target::Proxy {
					port: config.services[name].port.context("service port missing")?,
					// Graph services have separate origins/ownership. Ambient browser
					// cookies require a deliberate first-party opt-in, not a package
					// location or inherited alongside-the-app default.
					isolated: config.services[name].trust != Some(ServiceTrust::FirstParty),
				}
			}
		};
		targets.insert(route.unit.clone(), target);
	}
	Ok(Bindings {
		targets,
		certificates: PathBuf::from(format!("/etc/leptoskit/{}/tls", infra.name)),
	})
}

fn literal_path(path: &std::path::Path) -> Result<&str> {
	let value = path.to_str().context("edge path must be UTF-8")?;
	ensure!(
		path.is_absolute()
			&& !value.contains(['{', '}'])
			&& !value.bytes().any(|byte| byte.is_ascii_control())
			&& path.components().all(|part| {
				matches!(
					part,
					std::path::Component::RootDir | std::path::Component::Normal(_)
				)
			}),
		"edge paths must be absolute literals without traversal or placeholders"
	);
	Ok(value)
}

fn render(
	infra: &InfraGraph,
	plan: &Plan,
	host: &str,
	ranges: Option<&Ranges>,
	bindings: &Bindings,
) -> Result<Value> {
	let ingress = infra
		.edge
		.as_ref()
		.context("public edge declaration missing")?
		.ingress;
	let https = ingress == Ingress::CloudflareHttps;
	let edge_port = if https { 443 } else { 8088 };
	ensure!(
		bindings
			.targets
			.values()
			.all(|target| !matches!(target, Target::Proxy {port, ..} if *port == edge_port)),
		"application port conflicts with the edge listener"
	);
	let trusted = if https {
		ranges
			.context("HTTPS edge requires the checked Cloudflare address snapshot")?
			.all()
	} else {
		vec!["127.0.0.1/32", "::1/128"]
	};
	let mut server = json!({
		"listen": [if https {":443"} else {"127.0.0.1:8088"}],
		"automatic_https": {"disable":true}, "protocols":["h1","h2"],
		"read_header_timeout":"10s", "idle_timeout":"60s", "max_header_bytes":65536,
		"trusted_proxies":{"source":"static", "ranges":trusted},
		"client_ip_headers":["CF-Connecting-IP"], "trusted_proxies_strict":1,
		"routes": routes::render(plan, host, bindings)?
	});
	let mut result = json!({"admin":{"disabled":true},"apps":{"http":{"servers":{}}}});
	if https {
		server["strict_sni_host"] = json!(true);
		server["tls_connection_policies"] = json!([{}]);
		let directory = literal_path(&bindings.certificates)?;
		let domains: BTreeSet<_> = plan
			.routes()
			.entries()
			.iter()
			.filter(|route| route.host == host)
			.map(|route| &route.domain)
			.collect();
		let files: Vec<_> = domains
			.into_iter()
			.map(|domain| {
				json!({
					"certificate":format!("{directory}/{domain}.crt"),
					"key":format!("{directory}/{domain}.key")
				})
			})
			.collect();
		result["apps"]["tls"] = json!({"certificates":{"load_files":files}});
	}
	result["apps"]["http"]["servers"]["edge"] = server;
	Ok(result)
}
