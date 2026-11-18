//! Strict operator input. Credentials are references, never configuration
//! values.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::net::IpAddr;

use serde::Deserialize;
use serde::Serialize;

use crate::PlatformError;

/// Tested k3s release: https://github.com/k3s-io/k3s/releases/tag/v1.35.10%2Bk3s1
pub const K3S_VERSION: &str = "v1.35.10+k3s1";

/// Complete offline platform declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformConfig {
	/// Resource name prefix.
	pub name: String,
	/// HTTPS console/API origin, without a path.
	pub control_origin: String,
	/// Base DNS domain for wildcard application hosts.
	pub apps_domain: String,
	/// Region used for the platform edge.
	pub default_region: String,
	/// Clusters grouped by operator region slug.
	pub regions: BTreeMap<String, Region>,
	/// Tenant runtime and forbidden egress.
	pub isolation: Isolation,
	/// Shared database settings, consumed by the add-on phase.
	pub postgres: Postgres,
	/// Existing S3-compatible buckets and credential references.
	pub object_storage: ObjectStorage,
	/// Cloudflare origin and custom-domain strategy.
	pub edge: Edge,
	/// Registry deployment strategy.
	pub registry: Registry,
	/// Explicit chart pins and add-on settings for the next slice.
	pub addons: Addons,
	/// SSH/API administrators, never an unrestricted default.
	pub admin_cidrs: Vec<String>,
}

/// Region's cluster map.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
	/// Independent Kubernetes clusters.
	pub clusters: BTreeMap<String, Cluster>,
}

/// One IPv4 k3s cluster. IPv6 pod networking is deliberately not implied.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cluster {
	/// Only k3s is supported.
	pub distribution: String,
	/// Exact release supported by this generator.
	#[serde(default = "k3s_version")]
	pub version: String,
	/// Pod network, disjoint from services and node networks.
	pub pod_cidr: String,
	/// ClusterIP network.
	pub service_cidr: String,
	/// Optional operator-managed DNS name resolving to reachable servers.
	pub registration_address: Option<String>,
	/// Hetzner private subnet, required when cloud servers exist.
	pub private_cidr: Option<String>,
	/// Hetzner network zone shared by every attached VM.
	pub network_zone: Option<String>,
	/// Node groups; cloud counts expand into stable numbered identities.
	pub nodes: BTreeMap<String, Node>,
}

fn k3s_version() -> String {
	K3S_VERSION.into()
}

/// Control-plane or worker process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
	/// k3s server (embedded etcd for HA).
	Server,
	/// k3s agent.
	Agent,
}

/// Scheduling boundary encoded as labels and taints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Pool {
	/// Tenant workloads.
	Tenant,
	/// Platform services.
	Platform,
	/// Untrusted build jobs.
	Build,
	/// Edge ingress.
	Ingress,
}

/// Provider-specific node inputs reject mixed and unknown fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Node {
	/// Ubuntu 24.04 x86-64 VMs created by OpenTofu.
	HetznerCloud {
		/// Kubernetes role.
		role: Role,
		/// Scheduling pool.
		pool: Pool,
		/// Hetzner machine size (must support x86-64).
		server_type: String,
		/// Datacenter location in the declared network zone.
		location: String,
		/// Number of identical machines.
		count: u16,
		/// Existing provider SSH key IDs.
		ssh_key_ids: Vec<u64>,
	},
	/// Operator-owned Ubuntu 24.04 x86-64 host; never provisioned.
	Existing {
		/// Kubernetes role.
		role: Role,
		/// Scheduling pool.
		pool: Pool,
		/// Reachable public IPv4 address.
		public_address: std::net::Ipv4Addr,
		/// Existing private IPv4 link; mandatory for multiple servers.
		private_address: Option<std::net::Ipv4Addr>,
	},
}

impl Node {
	/// Kubernetes role.
	pub fn role(&self) -> Role {
		match self {
			Self::HetznerCloud { role, .. } | Self::Existing { role, .. } => *role,
		}
	}

	/// Scheduling pool.
	pub fn pool(&self) -> Pool {
		match self {
			Self::HetznerCloud { pool, .. } | Self::Existing { pool, .. } => *pool,
		}
	}

	/// Number of machines represented by the group.
	pub fn count(&self) -> u16 {
		match self {
			Self::HetznerCloud { count, .. } => *count,
			Self::Existing { .. } => 1,
		}
	}

	/// Whether OpenTofu owns this node.
	pub fn cloud(&self) -> bool {
		matches!(self, Self::HetznerCloud { .. })
	}
}

/// gVisor execution platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GvisorPlatform {
	/// Works without nested virtualization.
	Systrap,
	/// Requires operator-provided /dev/kvm.
	Kvm,
}

/// Runtime policy and artifact integrity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Isolation {
	/// RuntimeClass name; currently gvisor only.
	#[serde(default = "runtime_class")]
	pub runtime_class: String,
	/// Kernel isolation backend.
	pub platform: GvisorPlatform,
	/// Immutable gVisor point release (YYYYMMDD.N).
	pub release: String,
	/// Required official tarball SHA-512, with no invented default.
	pub sha512: String,
	/// Destinations forbidden to tenants by the later add-on policy.
	pub denied_egress_cidrs: Vec<String>,
}

fn runtime_class() -> String {
	"gvisor".into()
}

/// CloudNativePG shared database inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Postgres {
	/// Replica count.
	pub instances: u16,
	/// Per-instance PVC size, e.g. 100Gi.
	pub storage_size: String,
	/// Existing StorageClass.
	pub storage_class: String,
	/// Existing WAL archive bucket.
	pub wal_bucket: String,
	/// WAL retention days.
	pub retention_days: u16,
}

/// A reference to credentials; secret material is never accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CredentialRef {
	/// Resolve at application time from the operator environment.
	Environment {
		/// Access key variable name.
		access_key: String,
		/// Secret key variable name.
		secret_key: String,
	},
	/// Existing Kubernetes secret in the consumer namespace.
	KubernetesSecret {
		/// Secret name.
		name: String,
		/// Access key data key.
		access_key: String,
		/// Secret key data key.
		secret_key: String,
	},
}

/// Existing buckets; generation creates no object-store credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectStorage {
	/// HTTPS S3 endpoint.
	pub endpoint: String,
	/// S3 signing region.
	pub region: String,
	/// Source archive bucket.
	pub sources: String,
	/// Build log bucket.
	pub build_logs: String,
	/// Web assets bucket.
	pub web_assets: String,
	/// OTA bundle bucket.
	pub bundles: String,
	/// Database backup bucket.
	pub backups: String,
	/// Registry blob bucket.
	pub registry: String,
	/// Environment or Kubernetes secret reference.
	pub credentials: CredentialRef,
}

/// Public origin strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeMode {
	/// Outbound cloudflared connector, no inbound application ports.
	Tunnel,
	/// Port 443 admitted only from an explicit official range snapshot.
	Https,
}

/// Certificate ownership for customer domains.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CustomDomains {
	/// Cloudflare for SaaS.
	CloudflareSaas,
	/// cert-manager certificates.
	CertManager,
}

/// Edge account, placement and transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
	/// Cloudflare account identifier (not a credential).
	pub account_id: String,
	/// Cloudflare zone identifier.
	pub zone_id: String,
	/// Edge transport.
	pub mode: EdgeMode,
	/// Cluster in default_region owning public ingress.
	pub cluster: String,
	/// Customer certificate strategy.
	pub custom_domains: CustomDomains,
	/// Operator-supplied official Cloudflare IPv4 and IPv6 snapshot for HTTPS.
	#[serde(default)]
	pub cloudflare_cidrs: Vec<String>,
}

/// Registry source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Registry {
	/// Zot backed by object_storage.registry.
	Zot {
		/// Existing authentication secret.
		auth_secret: String,
	},
	/// Operator-managed registry.
	External {
		/// Registry DNS hostname.
		host: String,
		/// Kubernetes image pull secret.
		pull_secret: String,
	},
}

/// Pins are required inputs so the later add-on task can select compatible
/// charts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Addons {
	/// CloudNativePG operator chart.
	pub cloudnative_pg: String,
	/// KEDA chart.
	pub keda: String,
	/// KEDA HTTP add-on chart.
	pub keda_http: String,
	/// cert-manager chart.
	pub cert_manager: String,
	/// Traefik chart (bundled ingress is disabled).
	pub traefik: String,
	/// Zot chart.
	pub zot: String,
	/// ACME account contact supplied by the operator.
	pub acme_email: String,
	/// Existing Cloudflare DNS API secret.
	pub cloudflare_api_secret: String,
}

pub(crate) fn require(ok: bool, message: &str) -> Result<(), PlatformError> {
	if !ok {
		return Err(PlatformError::Invalid(message.into()));
	}
	Ok(())
}

pub(crate) fn slug(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 32
		&& value
			.bytes()
			.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
		&& !value.starts_with('-')
		&& !value.ends_with('-')
}

pub(crate) fn cidr(value: &str) -> Result<(IpAddr, u8), PlatformError> {
	let invalid = || PlatformError::Invalid("expected a canonical CIDR".into());
	let (address, prefix) = value.split_once('/').ok_or_else(invalid)?;
	let address: IpAddr = address.parse().map_err(|_| invalid())?;
	let prefix: u8 = prefix.parse().map_err(|_| invalid())?;
	let (bits, raw) = match address {
		IpAddr::V4(a) => (32, u128::from(u32::from(a))),
		IpAddr::V6(a) => (128, u128::from(a)),
	};
	require(
		prefix > 0 && prefix <= bits,
		"CIDR must have a restricted prefix",
	)?;
	let host_mask = (1_u128 << (bits - prefix)) - 1;
	require(raw & host_mask == 0, "CIDR must be canonical")?;
	Ok((address, prefix))
}

fn https(value: &str) -> bool {
	url::Url::parse(value).is_ok_and(|u| {
		u.scheme() == "https"
			&& u.host_str().is_some()
			&& u.username().is_empty()
			&& u.password().is_none()
			&& u.query().is_none()
			&& u.fragment().is_none()
			&& u.path() == "/"
	})
}

fn hostname(value: &str) -> bool {
	value.contains('.') && value.split('.').all(slug)
}

impl PlatformConfig {
	/// Parse and validate without retaining TOML error text that may contain
	/// secrets.
	pub fn parse(input: &str) -> Result<Self, PlatformError> {
		let config: Self = toml::from_str(input).map_err(|_| PlatformError::Toml)?;
		config.validate()?;
		Ok(config)
	}

	/// Enforce supported topology, safe rendering and explicit operational
	/// inputs.
	pub fn validate(&self) -> Result<(), PlatformError> {
		require(slug(&self.name), "name must be a lowercase slug")?;
		require(
			https(&self.control_origin),
			"control_origin must be an HTTPS origin without credentials or path",
		)?;
		require(
			hostname(&self.apps_domain),
			"apps_domain must be a DNS name",
		)?;
		require(
			!self.admin_cidrs.is_empty(),
			"admin_cidrs must not be empty",
		)?;
		for value in self
			.admin_cidrs
			.iter()
			.chain(&self.isolation.denied_egress_cidrs)
			.chain(&self.edge.cloudflare_cidrs)
		{
			cidr(value)?;
		}
		require(
			self.isolation.runtime_class == "gvisor",
			"only the gvisor runtime class is supported",
		)?;
		require(
			self.isolation.sha512.len() == 128
				&& self.isolation.sha512.bytes().all(|b| b.is_ascii_hexdigit()),
			"isolation.sha512 must be the official tarball SHA-512 (128 hex digits)",
		)?;
		let (date, point) =
			self.isolation.release.split_once('.').ok_or_else(|| {
				PlatformError::Invalid("gVisor release must be YYYYMMDD.N".into())
			})?;
		require(
			date.len() == 8
				&& date.bytes().all(|b| b.is_ascii_digit())
				&& !point.is_empty()
				&& point.bytes().all(|b| b.is_ascii_digit()),
			"gVisor release must be YYYYMMDD.N",
		)?;
		require(
			https(&self.object_storage.endpoint),
			"object storage requires an HTTPS origin",
		)?;
		require(
			self.postgres.instances > 0 && self.postgres.retention_days > 0,
			"postgres instances and retention must be positive",
		)?;
		require(
			self.postgres.wal_bucket == self.object_storage.backups,
			"postgres WAL bucket must match the backups bucket",
		)?;
		require(
			self.regions
				.get(&self.default_region)
				.is_some_and(|r| r.clusters.contains_key(&self.edge.cluster)),
			"edge cluster must exist in default_region",
		)?;
		if self.edge.mode == EdgeMode::Https {
			require(
				self.edge.cloudflare_cidrs.iter().any(|s| s.contains(':'))
					&& self.edge.cloudflare_cidrs.iter().any(|s| !s.contains(':')),
				"HTTPS requires an official dual-stack Cloudflare range snapshot",
			)?;
		}
		for (region, config) in &self.regions {
			require(
				slug(region) && !config.clusters.is_empty(),
				"regions require slug names and clusters",
			)?;
			for (name, cluster) in &config.clusters {
				require(slug(name), "cluster name must be a slug")?;
				cluster.validate(self.isolation.platform)?;
			}
		}
		Ok(())
	}
}

impl Cluster {
	fn validate(&self, platform: GvisorPlatform) -> Result<(), PlatformError> {
		require(
			self.distribution == "k3s" && self.version == K3S_VERSION,
			"distribution/version must match the pinned k3s release",
		)?;
		let pod = cidr(&self.pod_cidr)?;
		let service = cidr(&self.service_cidr)?;
		require(
			pod.0.is_ipv4() && service.0.is_ipv4() && !overlaps(pod, service),
			"pod/service CIDRs must be disjoint IPv4 networks",
		)?;
		let servers: Vec<_> = self
			.nodes
			.values()
			.filter(|n| n.role() == Role::Server)
			.collect();
		let count: u32 = servers.iter().map(|n| u32::from(n.count())).sum();
		require(
			count == 1 || (count >= 3 && count % 2 == 1),
			"use one server or an odd number of at least three servers; two servers cannot \
			 maintain etcd quorum",
		)?;
		require(
			servers.iter().all(|n| n.cloud() == servers[0].cloud()),
			"k3s embedded etcd is not supported across the multicloud overlay: servers must share \
			 one provider and private network",
		)?;
		if count > 1 {
			require(
				servers.iter().all(|n| {
					!matches!(
						n,
						Node::Existing {
							private_address: None,
							..
						}
					)
				}),
				"k3s embedded etcd is not supported across the multicloud overlay: existing \
				 servers require an operator-built private network and private_address",
			)?;
		}
		if servers[0].cloud() {
			let private = cidr(self.private_cidr.as_deref().ok_or_else(|| {
				PlatformError::Invalid("cloud servers require private_cidr".into())
			})?)?;
			require(
				private.0.is_ipv4()
					&& private.1 <= 24
					&& !overlaps(private, pod)
					&& !overlaps(private, service),
				"private_cidr must be IPv4 /24 or larger and disjoint from pod/services",
			)?;
			require(
				self.network_zone.as_deref().is_some_and(slug),
				"cloud servers require network_zone",
			)?;
		}
		if let Some(address) = &self.registration_address {
			require(
				hostname(address),
				"registration_address must be an operator-managed DNS name",
			)?;
		}
		let mut addresses = BTreeSet::new();
		for (name, node) in &self.nodes {
			require(
				slug(name) && node.count() > 0 && node.count() <= 100,
				"node names must be slugs; counts must be 1..100",
			)?;
			match node {
				Node::HetznerCloud {
					server_type,
					location,
					ssh_key_ids,
					..
				} => {
					require(
						platform != GvisorPlatform::Kvm,
						"gVisor kvm is allowed only on existing nodes: cloud VMs do not guarantee \
						 nested virtualization",
					)?;
					require(
						slug(server_type)
							&& slug(location)
							&& !ssh_key_ids.is_empty()
							&& ssh_key_ids.iter().all(|id| *id > 0),
						"cloud nodes require server_type, location and SSH key IDs",
					)?;
				}
				Node::Existing {
					public_address,
					private_address,
					..
				} => {
					require(
						!public_address.is_unspecified()
							&& !public_address.is_loopback()
							&& !public_address.is_private()
							&& addresses.insert(*public_address),
						"existing public addresses must be unique, public IPv4 addresses",
					)?;
					if let Some(private) = private_address {
						require(
							private.is_private() && addresses.insert(*private),
							"private addresses must be unique RFC1918 addresses",
						)?;
					}
				}
			}
		}
		Ok(())
	}
}

fn overlaps(a: (IpAddr, u8), b: (IpAddr, u8)) -> bool {
	match (a.0, b.0) {
		(IpAddr::V4(a_ip), IpAddr::V4(b_ip)) => {
			u32::from(a_ip) >> (32 - a.1.min(b.1)) == u32::from(b_ip) >> (32 - a.1.min(b.1))
		}
		_ => true,
	}
}
