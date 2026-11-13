//! Public hosts consume a pinned Caddy binary. Private hosts carry explicit
//! absence and never receive edge bytes or TLS/tunnel prerequisites.
use std::path::Path;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::deployment::Ingress;
use serde_json::Map;
use serde_json::Value;

use super::Args;
use super::TrustedHost;

const LIMIT: u64 = 64 * 1024 * 1024;

pub(super) struct Edge {
	pub config: Value,
	pub sha256: Option<String>,
}

pub(super) fn bytes(args: &Args) -> Result<Option<Vec<u8>>> {
	match (&args.caddy, &args.caddy_sha256) {
		(Some(path), Some(pin)) => Ok(Some(super::pinned(path, pin, LIMIT)?)),
		(None, None) => Ok(None),
		_ => anyhow::bail!("provide both --caddy and --caddy-sha256"),
	}
}

pub(super) fn check_usage(hosts: &Map<String, Value>, bytes: Option<&[u8]>) -> Result<()> {
	let public = hosts.values().any(|host| !host["edge"].is_null());
	ensure!(
		public == bytes.is_some(),
		"supply pinned Caddy only when the selection includes a public host"
	);
	Ok(())
}

pub(super) fn payload<'a>(host: &Value, bytes: Option<&'a [u8]>) -> Result<Option<&'a [u8]>> {
	if host["edge"].is_null() {
		return Ok(None);
	}

	Ok(Some(bytes.context("public host requires pinned Caddy")?))
}

pub(super) fn bind(
	dir: &Path,
	name: &str,
	public: bool,
	args: &Args,
	host: &TrustedHost,
	ingress: Option<Ingress>,
) -> Result<Edge> {
	if !public {
		ensure!(
			host.prerequisites.is_empty() && host.tunnel.is_none(),
			"private host cannot consume TLS/tunnel prerequisites"
		);
		return Ok(Edge {
			config: Value::Null,
			sha256: None,
		});
	}

	let binary = args
		.caddy
		.as_ref()
		.context("public host requires --caddy and --caddy-sha256")?;
	let root = super::input_root(binary)?;
	let filename = binary
		.file_name()
		.and_then(|name| name.to_str())
		.context("Caddy filename missing")?;
	crate::verify::graph::linux::inspect(root.open_executable(filename)?, host.architecture)?;
	let bytes = crate::verify::assets::Root::open(dir)?.metadata(&format!("edge-{name}.json"))?;
	let config = serde_json::from_slice(&bytes)?;
	check_ingress(
		host,
		&config,
		ingress.context("public edge declaration missing")?,
	)?;
	Ok(Edge {
		config,
		sha256: Some(super::hash(&bytes)),
	})
}

/// Trust pins describe prerequisites already installed by the operator.
/// They never authorize downloading certificates or starting an unpinned
/// tunnel.
fn check_ingress(host: &TrustedHost, edge: &Value, ingress: Ingress) -> Result<()> {
	if ingress == Ingress::CloudflareTunnel {
		ensure!(
			host.tunnel.is_some(),
			"configure and pin a running cloudflared systemd tunnel before release"
		);

		return Ok(());
	}

	ensure!(
		host.tunnel.is_none(),
		"HTTPS ingress cannot consume tunnel prerequisites"
	);

	for cert in edge["apps"]["tls"]["certificates"]["load_files"]
		.as_array()
		.context("TLS certificate bindings missing")?
	{
		let path = cert["certificate"]
			.as_str()
			.context("certificate path missing")?;
		ensure!(
			host.prerequisites.contains_key(path),
			"pin each configured public TLS certificate; install matching private keys outside \
			 receipts first"
		);
	}
	Ok(())
}

#[cfg(test)]
mod tests;
