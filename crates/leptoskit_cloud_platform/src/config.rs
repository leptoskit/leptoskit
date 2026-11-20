//! Strict operator input. Credentials are references, never configuration
//! values.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::net::IpAddr;

use serde::Deserialize;
use serde::Serialize;

use crate::PlatformError;

/// Tested k3s release: <https://github.com/k3s-io/k3s/releases/tag/v1.35.10%2Bk3s1>
pub const K3S_VERSION: &str = "v1.35.10+k3s1";

/// Verified point release: <https://gvisor.dev/docs/user_guide/install/#point-release>.
pub const GVISOR_RELEASE: &str = "20261116.0";

/// Official x86-64 tarball digest, verified at
/// <https://storage.googleapis.com/gvisor/releases/release/20261116.0/x86_64/gvisor.tar.zstd.sha512>.
pub const GVISOR_SHA512: &str = "a34261504066142b6c1cc4d1813bb2d092761675416e350edc98c4cc66469b63b6f41acdeed09681b6c717c00e80e437f5f2f9c8d37705b8a9260632867c2b7d";

/// Official origin ranges, verified 2026-11-18 at
/// <https://www.cloudflare.com/ips-v4> and <https://www.cloudflare.com/ips-v6>.
pub const CLOUDFLARE_CIDRS: &[&str] = &[
	"173.245.48.0/20",
	"103.21.244.0/22",
	"103.22.200.0/22",
	"103.31.4.0/22",
	"141.101.64.0/18",
	"108.162.192.0/18",
	"190.93.240.0/20",
	"188.114.96.0/20",
	"197.234.240.0/22",
	"198.41.128.0/17",
	"162.158.0.0/15",
	"104.16.0.0/13",
	"104.24.0.0/14",
	"172.64.0.0/13",
	"131.0.72.0/22",
	"2400:cb00::/32",
	"2606:4700::/32",
	"2803:f800::/32",
	"2405:b500::/32",
	"2405:8100::/32",
	"2a06:98c0::/29",
	"2c0f:f248::/32",
];

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
	/// `ClusterIP` network.
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
	/// Ubuntu 24.04 x86-64 VMs created by `OpenTofu`.
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
		/// Optional per-group gVisor platform override (defaults to
		/// `isolation.platform`).
		#[serde(default, skip_serializing_if = "Option::is_none")]
		isolation_platform: Option<GvisorPlatform>,
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
		/// Optional per-group gVisor platform override (allows `kvm` on bare
		/// metal while cloud VMs stay on `systrap`).
		#[serde(default, skip_serializing_if = "Option::is_none")]
		isolation_platform: Option<GvisorPlatform>,
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

	/// Whether `OpenTofu` owns this node.
	pub fn cloud(&self) -> bool {
		matches!(self, Self::HetznerCloud { .. })
	}

	/// Effective gVisor platform for this node group.
	pub fn isolation_platform(&self, default: GvisorPlatform) -> GvisorPlatform {
		match self {
			Self::HetznerCloud {
				isolation_platform, ..
			}
			| Self::Existing {
				isolation_platform, ..
			} => isolation_platform.unwrap_or(default),
		}
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
	/// `RuntimeClass` name; currently gvisor only.
	#[serde(default = "runtime_class")]
	pub runtime_class: String,
	/// Kernel isolation backend.
	pub platform: GvisorPlatform,
	/// Immutable gVisor point release (YYYYMMDD.N).
	#[serde(default = "gvisor_release")]
	pub release: String,
	/// Verified official tarball SHA-512 for the supported point release.
	#[serde(default = "gvisor_sha512")]
	pub sha512: String,
	/// Destinations forbidden to tenants by the later add-on policy.
	pub denied_egress_cidrs: Vec<String>,
}

fn runtime_class() -> String {
	"gvisor".into()
}

fn gvisor_release() -> String {
	GVISOR_RELEASE.into()
}
fn gvisor_sha512() -> String {
	GVISOR_SHA512.into()
}

const fn default_max_connections() -> u32 {
	300
}

/// `CloudNativePG` shared database inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Postgres {
	/// Replica count.
	pub instances: u16,
	/// Per-instance PVC size, e.g. 100Gi.
	pub storage_size: String,
	/// Existing `StorageClass`.
	pub storage_class: String,
	/// Existing WAL archive bucket.
	pub wal_bucket: String,
	/// WAL retention days.
	pub retention_days: u16,
	/// Shared Postgres `max_connections` (defaults to 300 so multiple tenant
	/// roles with `CONNECTION LIMIT 20` do not exhaust Postgres' default 100).
	#[serde(default = "default_max_connections")]
	pub max_connections: u32,
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

/// Optional separate S3 endpoint for cold database WAL archives and base
/// backups (for example, Backblaze B2 while hot buckets stay on Cloudflare R2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColdObjectStorage {
	/// HTTPS S3 endpoint for backups and WAL archives.
	pub endpoint: String,
	/// S3 signing region for backups and WAL archives.
	pub region: String,
	/// Environment or Kubernetes secret reference for backups and WAL archives.
	pub credentials: CredentialRef,
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
	/// Optional separate cold storage target (endpoint, region, credentials)
	/// for `backups` and Postgres WAL archives.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cold: Option<ColdObjectStorage>,
}

impl ObjectStorage {
	/// Endpoint, region, and credential reference used for database backups
	/// and WAL archives (`cold` when configured, otherwise the primary store).
	#[must_use]
	pub fn backups_store(&self) -> (&str, &str, &CredentialRef) {
		if let Some(cold) = &self.cold {
			(&cold.endpoint, &cold.region, &cold.credentials)
		} else {
			(&self.endpoint, &self.region, &self.credentials)
		}
	}
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CustomDomains {
	/// Cloudflare for `SaaS`.
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
	/// Cluster in `default_region` owning public ingress.
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
	/// Zot backed by `object_storage.registry`.
	Zot {
		/// Existing authentication secret.
		auth_secret: String,
		/// Expose Zot externally at `registry.<apps_domain>` through Ingress.
		/// Defaults to `false` so pulls/pushes use the cluster-local Service
		/// and avoid Cloudflare's 100 MiB upload limit and hairpin loops.
		#[serde(default)]
		expose_ingress: bool,
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
	/// `CloudNativePG` operator chart.
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
		// A release and digest move together: accepting an overridden release
		// with the default checksum would produce a document that cannot boot.
		require(
			self.isolation.release == GVISOR_RELEASE
				&& self.isolation.sha512.eq_ignore_ascii_case(GVISOR_SHA512),
			"gVisor release and checksum must match the supported verified artifact",
		)?;
		require(
			self.admin_cidrs.iter().any(|cidr| !cidr.contains(':')),
			"IPv4-only node addressing requires at least one IPv4 admin CIDR",
		)?;
		validate_strings(&serde_json::to_value(self)?)?;
		require(
			[&self.edge.account_id, &self.edge.zone_id]
				.iter()
				.all(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())),
			"Cloudflare account and zone IDs must be 32 hex digits",
		)?;
		for pin in [
			&self.addons.cloudnative_pg,
			&self.addons.keda,
			&self.addons.keda_http,
			&self.addons.cert_manager,
			&self.addons.traefik,
			&self.addons.zot,
		] {
			let parts: Vec<_> = pin.trim_start_matches('v').split('.').collect();
			require(
				parts.len() == 3
					&& parts
						.iter()
						.all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())),
				"add-on charts require exact release versions",
			)?;
		}
		require(
			self.postgres
				.storage_size
				.strip_suffix("Gi")
				.is_some_and(|s| s.parse::<u32>().is_ok_and(|n| n > 0))
				&& slug(&self.postgres.storage_class),
			"postgres requires a positive Gi storage size and StorageClass slug",
		)?;
		for bucket in [
			&self.object_storage.sources,
			&self.object_storage.build_logs,
			&self.object_storage.web_assets,
			&self.object_storage.bundles,
			&self.object_storage.backups,
			&self.object_storage.registry,
		] {
			require(slug(bucket), "object storage bucket names must be slugs")?;
		}
		validate_credentials(&self.object_storage.credentials)?;
		if let Some(cold) = &self.object_storage.cold {
			require(
				https(&cold.endpoint),
				"cold object storage requires an HTTPS origin",
			)?;
			validate_credentials(&cold.credentials)?;
		}
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
		for value in self.admin_cidrs.iter().chain(&self.edge.cloudflare_cidrs) {
			cidr(value)?;
		}
		self.validate_isolation()?;
		require(
			https(&self.object_storage.endpoint),
			"object storage requires an HTTPS origin",
		)?;
		require(
			self.postgres.instances > 0
				&& self.postgres.retention_days > 0
				&& self.postgres.max_connections >= 100,
			"postgres instances, retention, and max_connections (>= 100) must be positive",
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
			let provided: BTreeSet<_> = self
				.edge
				.cloudflare_cidrs
				.iter()
				.map(String::as_str)
				.collect();
			require(
				provided == CLOUDFLARE_CIDRS.iter().copied().collect(),
				"HTTPS Cloudflare ranges must match the pinned official snapshot",
			)?;
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
				require(
					self.name.len() + region.len() + name.len() + 2 <= 53,
					"combined platform, region and cluster names must fit provider resource \
					 labels (53 characters)",
				)?;
				require(slug(name), "cluster name must be a slug")?;
				cluster.validate(self.isolation.platform)?;
			}
		}
		Ok(())
	}

	fn validate_isolation(&self) -> Result<(), PlatformError> {
		for value in &self.isolation.denied_egress_cidrs {
			let parsed = cidr(value)?;
			require(
				parsed.0.is_ipv4(),
				"isolation.denied_egress_cidrs must be IPv4 CIDRs",
			)?;
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
		Ok(())
	}
}

impl Cluster {
	fn validate(&self, platform: GvisorPlatform) -> Result<(), PlatformError> {
		let total: u32 = self.nodes.values().map(|n| u32::from(n.count())).sum();
		require(
			total <= 240,
			"clusters support at most 240 nodes to fit deterministic private addresses",
		)?;
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
				matches!(private.0,IpAddr::V4(ip) if ip.is_private()),
				"private_cidr must be an RFC1918 network",
			)?;
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
						node.isolation_platform(platform) != GvisorPlatform::Kvm,
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

fn validate_strings(value: &serde_json::Value) -> Result<(), PlatformError> {
	match value {
		serde_json::Value::String(s) => {
			require(
				!s.is_empty()
					&& s.len() <= 256
					&& !s.chars().any(char::is_control)
					&& !s.contains("${")
					&& !s.contains("%{"),
				"configuration strings must be nonempty, bounded literals without control \
				 characters or template expressions",
			)
		}
		serde_json::Value::Object(map) => {
			for value in map.values() {
				validate_strings(value)?;
			}
			Ok(())
		}
		serde_json::Value::Array(values) => {
			for value in values {
				validate_strings(value)?;
			}
			Ok(())
		}
		_ => Ok(()),
	}
}

fn validate_credentials(reference: &CredentialRef) -> Result<(), PlatformError> {
	match reference {
		CredentialRef::Environment {
			access_key,
			secret_key,
		} => {
			require(
				[access_key, secret_key].iter().all(|s| {
					s.bytes()
						.all(|b| b.is_ascii_uppercase() || b == b'_' || b.is_ascii_digit())
						&& !s.starts_with(|c: char| c.is_ascii_digit())
				}),
				"credentials must name environment variables, never values",
			)
		}
		CredentialRef::KubernetesSecret {
			name,
			access_key,
			secret_key,
		} => {
			require(
				slug(name)
					&& [access_key, secret_key].iter().all(|s| {
						s.bytes()
							.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
					}),
				"credentials must reference a Kubernetes secret and data keys",
			)
		}
	}
}
