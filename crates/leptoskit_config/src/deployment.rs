//! Explicit multi-unit deployment declarations. These do not inherit the
//! managed single-application preset's Postgres or backup defaults.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::infra::ComputeProvider;
use crate::infra::InfraBudget;
use crate::infra::InfraState;

mod compile;
mod validate;
pub use compile::compile;

/// The explicit discriminator keeps graph declarations separate from the
/// existing managed application preset during parsing and serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
	Graph,
}

/// Infrastructure for a graph of independently released units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct InfraGraph {
	pub mode: Mode,
	pub name: String,
	pub account_id: String,
	pub zone_id: String,
	#[serde(default)]
	pub ssh_key_ids: Vec<u64>,
	pub ssh_cidrs: Vec<String>,
	pub hosts: BTreeMap<String, Host>,
	/// Frontend artifact placement and routes owned by this graph.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub frontends: BTreeMap<String, Frontend>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub edge: Option<Edge>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub budget: Option<InfraBudget>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub state: Option<InfraState>,
}

/// A named host class. Several units can share it unless exclusive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Host {
	#[serde(default)]
	pub compute: ComputeProvider,
	pub location: String,
	pub server_type: String,
	#[serde(default)]
	pub exclusive: bool,
	/// Override the project's SSH keys for a different provider account.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub ssh_key_ids: Vec<u64>,
}

/// How traffic reaches an origin. Tunnel and public HTTPS are different
/// contracts, not interchangeable firewall presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum Ingress {
	CloudflareHttps,
	CloudflareTunnel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum EdgeProvider {
	#[default]
	Cloudflare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum OriginFirewall {
	/// Only the pinned Cloudflare ranges reach the public origin listener.
	CloudflareOnly,
	/// No public application listener; a tunnel connects outbound.
	Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Edge {
	#[serde(default)]
	pub provider: EdgeProvider,
	#[serde(default = "yes")]
	pub proxied: bool,
	pub ingress: Ingress,
	#[serde(rename = "origin-firewall")]
	pub origin_firewall: OriginFirewall,
}

/// A trusted repository build recipe with a named output directory. Its
/// command is an argument vector, not a shell fragment or user input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Artifact {
	pub command: Vec<String>,
	pub output: PathBuf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum FrontendKind {
	#[default]
	Static,
	Ssr,
}

/// A frontend's owned files, public route and host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Frontend {
	pub artifact: String,
	#[serde(default = "root_directory")]
	pub directory: String,
	#[serde(default)]
	pub kind: FrontendKind,
	pub host: String,
	pub domain: String,
	#[serde(default = "root_mount")]
	pub mount: String,
	/// SSR executable relative to the declared artifact output. Static
	/// frontends have no application executable.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub backend: Option<PathBuf>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub serve: Option<crate::ServeConfig>,
	#[serde(default, rename = "depends-on", skip_serializing_if = "Vec::is_empty")]
	pub depends_on: Vec<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub calls: Vec<String>,
}

/// Independent placement and exposure for an existing service source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ServiceDeploy {
	pub host: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub domain: Option<String>,
	#[serde(default = "root_mount")]
	pub mount: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub ingress: Option<Ingress>,
	#[serde(default, rename = "depends-on", skip_serializing_if = "Vec::is_empty")]
	pub depends_on: Vec<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub calls: Vec<String>,
}

/// The service implements the same health/readiness contract as an app
/// with `serve.probes`. A declaration is not a successful runtime probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ServiceServe {
	pub probes: bool,
	/// Variable consumed by this executable for its complete listen address,
	/// for example `PLAY_BIND`. Required when releasing an executable graph
	/// unit.
	#[serde(
		default,
		rename = "address-env",
		skip_serializing_if = "Option::is_none"
	)]
	pub address_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ServiceRuntime {
	pub container: Container,
}

/// A dedicated rootless runtime used by the trusted service supervisor.
/// The untrusted compilation workload must never receive its socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Container {
	pub engine: Engine,
	#[serde(default = "yes")]
	pub rootless: bool,
	pub socket_env: String,
	pub engine_env: String,
	pub image_env: String,
	/// Repository-relative file containing an immutable registry digest.
	/// Its exact bytes are captured in the concrete release plan.
	pub image_pin: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Engine {
	Docker,
	Podman,
}

impl InfraGraph {
	/// Checks declarations without provider access or resource creation.
	#[must_use]
	pub fn problem(&self) -> Option<String> {
		validate::check(self).err()
	}
}

const fn yes() -> bool {
	true
}
fn root_directory() -> String {
	".".into()
}
fn root_mount() -> String {
	"/".into()
}
