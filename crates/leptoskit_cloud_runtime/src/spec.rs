//! The desired state of one cloud environment, as plain data.
//!
//! [`ReleaseSpec`] is the whole release a runtime applies: which processes
//! run which images at which scale, who reaches them, and the policy they
//! live under. It carries no plans, no prices, and no cluster detail — the
//! control plane builds it, a [`Runtime`](crate::Runtime) applies it.
//! [`ReleaseSpec::validate`] refuses a spec no runtime could honor, with
//! one typed error per reason.
//!
//! Names a customer chose never reach Kubernetes through this module: the
//! namespace comes from [`EnvironmentRef::namespace`], which hashes the
//! three ids. Slugs are labels, never names.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use uuid::Uuid;

/// A name in the cloud's address space: the slug people chose, and the id
/// the cloud assigned. Manifests build labels from the slug and names from
/// the id, so a rename moves nothing a runtime applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedRef {
	/// The slug, unique within its parent.
	pub slug: String,
	/// The identifier, a UUID v7 the control plane assigned.
	pub id: Uuid,
}

/// Which environment a release goes to: the organisation, the project, and
/// the environment inside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRef {
	pub organization: NamedRef,
	pub project: NamedRef,
	pub environment: NamedRef,
}

impl EnvironmentRef {
	/// The Kubernetes namespace for this environment: `lk-` followed by the
	/// first twenty lower-case hex characters of SHA-256 over the three ids,
	/// in order. The slugs are never hashed, so a rename moves nothing, and
	/// no two environments can share a namespace.
	#[must_use]
	pub fn namespace(&self) -> String {
		let mut hasher = Sha256::new();
		hasher.update(self.organization.id.as_bytes());
		hasher.update(self.project.id.as_bytes());
		hasher.update(self.environment.id.as_bytes());
		let digest = hasher.finalize();
		// Ten bytes are exactly twenty hex characters, which keeps the
		// namespace a short DNS label.
		let mut name = String::with_capacity(23);
		name.push_str("lk-");
		for byte in &digest[..10] {
			let _ = write!(name, "{byte:02x}");
		}
		name
	}
}

/// What a process is: the app itself, or a service beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcessKind {
	/// The app's own server. Exactly one per release.
	App,
	/// One of the app's `[services.<name>]`.
	Service,
}

/// The exact image a process runs: a repository and an immutable digest.
/// A tag would drift between two applies of one spec, so it is not
/// representable here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
	pub repository: String,
	/// `sha256:` followed by 64 lower-case hex characters.
	pub digest: String,
}

impl Image {
	/// The reference a container runtime pulls: `repository@sha256:…`.
	#[must_use]
	pub fn reference(&self) -> String {
		format!("{}@{}", self.repository, self.digest)
	}
}

/// The HTTP paths a process answers its health checks on. Both are probed
/// on the process's own port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probes {
	/// The readiness path: traffic reaches the process once it answers.
	pub ready: String,
	/// The liveness path: a process that stops answering is restarted.
	pub live: String,
}

/// CPU and memory a process asks for. Memory's request equals its limit —
/// a process that bursts past its request steals memory its neighbours
/// counted on, so the two are one number here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resources {
	pub cpu_request_millis: u32,
	pub cpu_limit_millis: u32,
	pub memory_mib: u32,
}

/// How many replicas a process runs. `min = 0` with `sleep` means the
/// process sleeps when idle and starts on the first request; a process
/// that holds live connections keeps `min` at least one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scale {
	pub min: u32,
	pub max: u32,
	/// Sleep when idle. Requires `min = 0`, and a cluster that can wake a
	/// sleeping process; the manifests refuse one that cannot.
	pub sleep: bool,
}

/// A named persistent directory a process owns, kept across restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Volume {
	/// The volume's name, a DNS label like a process's.
	pub name: String,
	/// The absolute path the directory appears at inside the process.
	pub mount_path: String,
	pub size_gib: u32,
}

/// One process of the release: the app, or a service beside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Process {
	/// A lower-case DNS label, unique in the release: it becomes every
	/// object's name for the process in the namespace.
	pub name: String,
	pub kind: ProcessKind,
	pub image: Image,
	/// The command to run. Empty keeps the image's own.
	pub command: Vec<String>,
	/// The command's arguments.
	pub args: Vec<String>,
	/// The port the process serves on, never zero.
	pub port: u16,
	pub probes: Probes,
	/// Plain environment variables, sorted on the way into a manifest.
	pub env: BTreeMap<String, String>,
	/// Variables read from the release's secrets: the variable's name, then
	/// the secret key that holds its value. The key must have a value when
	/// the manifests are built; the value never appears anywhere but the
	/// Secret's data.
	pub secret_env: BTreeMap<String, String>,
	pub resources: Resources,
	pub scale: Scale,
	/// Named persistent directories. A process with any runs at most one
	/// replica, so two replicas never write to two copies of one claim.
	pub volumes: Vec<Volume>,
}

/// The TLS a route's hostname ends its paths with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tls {
	/// The cluster's wildcard certificate, shared by every default hostname.
	Wildcard,
	/// A certificate cert-manager issues for this one hostname.
	Issued,
}

/// One path on one hostname, served by one process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
	/// A lower-case DNS name with at least two labels.
	pub hostname: String,
	/// The path prefix the route covers, starting with `/`.
	pub path_prefix: String,
	/// The process that serves it. Must name a process of the release.
	pub process: String,
	/// Strip the app's cookies from requests and answers. A plain Ingress
	/// cannot do this, so the manifests refuse a route that asks for it;
	/// only the cloud's own edge strips cookies.
	pub strip_cookies: bool,
	pub tls: Tls,
}

/// Which outside addresses a process may reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Egress {
	/// The internet, except the cluster's own and private ranges.
	Internet,
	/// Nothing outside the namespace but DNS and the database.
	None,
}

/// The most an environment may take from the cluster, over all its
/// processes at full scale plus one surge pod per process without volumes.
/// Processes with volumes use `Recreate` and require no surge capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quota {
	pub cpu_millis: u32,
	pub memory_mib: u32,
	pub pods: u32,
	pub storage_gib: u32,
}

impl Quota {
	/// Minimum quota for full scale and simultaneous rolling updates.
	/// CPU counts limits, memory counts requests (which equal limits), and
	/// storage counts each claim once. Arithmetic saturates at `u32::MAX`;
	/// validation still rejects totals that exceed the representable quota.
	#[must_use]
	pub fn for_processes(processes: &[Process]) -> Self {
		let [cpu, memory, pods, storage] = quota_totals(processes);
		let bounded = |value: u64| u32::try_from(value).unwrap_or(u32::MAX);
		Self {
			cpu_millis: bounded(cpu),
			memory_mib: bounded(memory),
			pods: bounded(pods),
			storage_gib: bounded(storage),
		}
	}
}

fn quota_totals(processes: &[Process]) -> [u64; 4] {
	let mut totals = [0u64; 4];
	for process in processes {
		let replicas = u64::from(process.scale.max) + u64::from(process.volumes.is_empty());
		totals[0] =
			totals[0].saturating_add(replicas * u64::from(process.resources.cpu_limit_millis));
		totals[1] = totals[1].saturating_add(replicas * u64::from(process.resources.memory_mib));
		totals[2] = totals[2].saturating_add(replicas);
		for volume in &process.volumes {
			totals[3] = totals[3].saturating_add(u64::from(volume.size_gib));
		}
	}
	totals
}

/// A database the processes may reach: the namespace it runs in and the
/// port to reach it on. The database itself is managed elsewhere; this is
/// only the network door to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseAccess {
	pub namespace: String,
	pub port: u16,
}

/// The policy every process of the release lives under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
	/// The `RuntimeClass` pods run under, such as a sandboxed one. None keeps
	/// the cluster's own default.
	pub runtime_class: Option<String>,
	pub egress: Egress,
	pub quota: Quota,
	/// The database's door, when the environment has one.
	pub database: Option<DatabaseAccess>,
}

/// The whole desired state of one environment: the processes, their routes,
/// and the policy they live under. Immutable once applied — a change is a
/// new deployment number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseSpec {
	pub environment: EnvironmentRef,
	/// The deployment this release is, one number per environment.
	pub deployment: u64,
	pub processes: Vec<Process>,
	pub routes: Vec<Route>,
	pub policy: Policy,
}

impl ReleaseSpec {
	/// Refuses a spec no runtime could honor, naming the first reason in a
	/// fixed order: processes, then quota, then routes.
	///
	/// # Errors
	/// A [`SpecError`] naming the rule the spec breaks.
	pub fn validate(&self) -> Result<(), SpecError> {
		self.validate_processes()?;
		self.validate_quota()?;
		self.validate_routes()?;
		Ok(())
	}

	fn validate_processes(&self) -> Result<(), SpecError> {
		for process in &self.processes {
			if !is_dns_label(&process.name) {
				return Err(SpecError::ProcessName {
					name: process.name.clone(),
				});
			}
		}

		for (index, process) in self.processes.iter().enumerate() {
			if self.processes[..index]
				.iter()
				.any(|other| other.name == process.name)
			{
				return Err(SpecError::DuplicateProcess {
					name: process.name.clone(),
				});
			}
		}

		let mut apps = self.processes.iter().filter(|p| p.kind == ProcessKind::App);
		if apps.next().is_none() {
			return Err(SpecError::NoApp);
		}
		if apps.next().is_some() {
			let names = self
				.processes
				.iter()
				.filter(|p| p.kind == ProcessKind::App)
				.map(|p| p.name.as_str())
				.collect::<Vec<_>>()
				.join(", ");
			return Err(SpecError::TooManyApps { names });
		}

		for process in &self.processes {
			Self::validate_one_process(process)?;
		}
		Ok(())
	}

	fn validate_one_process(process: &Process) -> Result<(), SpecError> {
		let name = &process.name;
		if process.port == 0 {
			return Err(SpecError::InvalidPort {
				process: name.clone(),
			});
		}

		if process.image.repository.is_empty() {
			return Err(SpecError::ImageRepository {
				process: name.clone(),
			});
		}
		if !is_digest(&process.image.digest) {
			return Err(SpecError::InvalidDigest {
				process: name.clone(),
				digest: process.image.digest.clone(),
			});
		}

		for variable in process.env.keys().chain(process.secret_env.keys()) {
			if !is_env_name(variable) {
				return Err(SpecError::InvalidEnvName {
					process: name.clone(),
					variable: variable.clone(),
				});
			}
		}
		for variable in process.env.keys() {
			if process.secret_env.contains_key(variable) {
				return Err(SpecError::EnvNameConflict {
					process: name.clone(),
					variable: variable.clone(),
				});
			}
		}

		for volume in &process.volumes {
			if !is_dns_label(&volume.name) {
				return Err(SpecError::VolumeName {
					process: name.clone(),
					name: volume.name.clone(),
				});
			}
		}

		let scale = &process.scale;
		if scale.min > scale.max {
			return Err(SpecError::ScaleInverted {
				process: name.clone(),
				min: scale.min,
				max: scale.max,
			});
		}
		if scale.sleep && scale.min != 0 {
			return Err(SpecError::SleepNeedsMinZero {
				process: name.clone(),
				min: scale.min,
			});
		}
		if !process.volumes.is_empty() && scale.max != 1 {
			return Err(SpecError::VolumesNeedOneReplica {
				process: name.clone(),
				max: scale.max,
			});
		}
		Ok(())
	}

	/// The totals are what the environment may take at full scale: every
	/// process at its maximum replica count plus its rolling surge. Storage
	/// counts each volume once, not per replica — a claim is shared, and a
	/// process with volumes cannot scale past one replica anyway.
	fn validate_quota(&self) -> Result<(), SpecError> {
		let [cpu_millis, memory_mib, pods, storage_gib] = quota_totals(&self.processes);

		let quota = &self.policy.quota;
		if cpu_millis > u64::from(quota.cpu_millis) {
			return Err(SpecError::QuotaCpu {
				used: cpu_millis,
				allowed: u64::from(quota.cpu_millis),
			});
		}
		if memory_mib > u64::from(quota.memory_mib) {
			return Err(SpecError::QuotaMemory {
				used: memory_mib,
				allowed: u64::from(quota.memory_mib),
			});
		}
		if pods > u64::from(quota.pods) {
			return Err(SpecError::QuotaPods {
				used: pods,
				allowed: u64::from(quota.pods),
			});
		}
		if storage_gib > u64::from(quota.storage_gib) {
			return Err(SpecError::QuotaStorage {
				used: storage_gib,
				allowed: u64::from(quota.storage_gib),
			});
		}
		Ok(())
	}

	fn validate_routes(&self) -> Result<(), SpecError> {
		for route in &self.routes {
			if !is_hostname(&route.hostname) {
				return Err(SpecError::Hostname {
					hostname: route.hostname.clone(),
				});
			}
			if !route.path_prefix.starts_with('/') {
				return Err(SpecError::PathPrefix {
					process: route.process.clone(),
					prefix: route.path_prefix.clone(),
				});
			}
			if !self.processes.iter().any(|p| p.name == route.process) {
				return Err(SpecError::UnknownRouteProcess {
					process: route.process.clone(),
				});
			}
		}
		Ok(())
	}
}

/// Why a [`ReleaseSpec`] cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpecError {
	#[error(
		"process names must be lower-case DNS labels: a letter first, then letters, digits and \
		 hyphens, ending in a letter or digit, at most 63 characters — not {name:?}"
	)]
	ProcessName { name: String },
	#[error("two processes are named {name:?}")]
	DuplicateProcess { name: String },
	#[error("exactly one process of kind `app` is required; this spec has none")]
	NoApp,
	#[error("exactly one process of kind `app` is allowed; this spec names several: {names}")]
	TooManyApps { names: String },
	#[error("process {process:?} listens on port 0, but 0 is not a port")]
	InvalidPort { process: String },
	#[error("process {process:?} has no image repository")]
	ImageRepository { process: String },
	#[error(
		"process {process:?} has image digest {digest:?}: a digest is `sha256:` followed by 64 \
		 lower-case hex characters, never a tag"
	)]
	InvalidDigest { process: String, digest: String },
	#[error(
		"process {process:?} sets the environment variable {variable:?}: names are C identifiers, \
		 a letter or underscore first, then letters, digits and underscores"
	)]
	InvalidEnvName { process: String, variable: String },
	#[error(
		"process {process:?} sets {variable:?} both as a plain and as a secret environment \
		 variable"
	)]
	EnvNameConflict { process: String, variable: String },
	#[error(
		"process {process:?} has a volume named {name:?}: volume names follow the same rules as \
		 process names"
	)]
	VolumeName { process: String, name: String },
	#[error("process {process:?} scales from {min} to {max} replicas: min must not exceed max")]
	ScaleInverted { process: String, min: u32, max: u32 },
	#[error("process {process:?} sleeps when idle, so its minimum must be 0, not {min}")]
	SleepNeedsMinZero { process: String, min: u32 },
	#[error(
		"process {process:?} has volumes, so every replica would write to its own copy of the \
		 same claim; its maximum must be 1, not {max}"
	)]
	VolumesNeedOneReplica { process: String, max: u32 },
	#[error(
		"at full scale the environment wants {used} millicores of CPU, more than the quota's \
		 {allowed}"
	)]
	QuotaCpu { used: u64, allowed: u64 },
	#[error(
		"at full scale the environment wants {used} MiB of memory, more than the quota's {allowed}"
	)]
	QuotaMemory { used: u64, allowed: u64 },
	#[error("at full scale the environment runs {used} pods, more than the quota's {allowed}")]
	QuotaPods { used: u64, allowed: u64 },
	#[error("the environment's volumes hold {used} GiB, more than the quota's {allowed}")]
	QuotaStorage { used: u64, allowed: u64 },
	#[error(
		"{hostname:?} is not a usable host name: a lower-case name of letters, digits and hyphens \
		 in labels separated by dots, at least two labels, at most 253 characters"
	)]
	Hostname { hostname: String },
	#[error("the route to {process:?} has the path prefix {prefix:?}: it must start with `/`")]
	PathPrefix { process: String, prefix: String },
	#[error("a route names the process {process:?}, but no process with that name exists")]
	UnknownRouteProcess { process: String },
}

/// An RFC 1035 label, the shape every name this crate builds needs: a
/// lower-case letter first, then letters, digits and hyphens, ending in a
/// letter or digit, at most 63 characters.
fn is_dns_label(name: &str) -> bool {
	let bytes = name.as_bytes();
	if bytes.is_empty() || bytes.len() > 63 || !bytes[0].is_ascii_lowercase() {
		return false;
	}
	if !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
		return false;
	}
	bytes
		.iter()
		.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// One label of a host name. Unlike a process name it may start with a
/// digit: `9notes.example.com` is a legal host.
fn is_hostname_label(label: &str) -> bool {
	let bytes = label.as_bytes();
	if bytes.is_empty() || bytes.len() > 63 {
		return false;
	}
	if !bytes[0].is_ascii_alphanumeric() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
		return false;
	}
	bytes
		.iter()
		.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// A usable public host name: lower-case labels separated by dots, at
/// least two of them, at most 253 characters in all.
fn is_hostname(name: &str) -> bool {
	if name.is_empty() || name.len() > 253 {
		return false;
	}
	let labels: Vec<&str> = name.split('.').collect();
	labels.len() >= 2 && labels.iter().all(|label| is_hostname_label(label))
}

/// An environment variable name, as a C identifier: a letter or underscore
/// first, then letters, digits and underscores.
fn is_env_name(name: &str) -> bool {
	let mut chars = name.chars();
	let Some(first) = chars.next() else {
		return false;
	};
	(first.is_ascii_alphabetic() || first == '_')
		&& chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// An image digest: `sha256:` followed by 64 lower-case hex characters.
fn is_digest(digest: &str) -> bool {
	let Some(hex) = digest.strip_prefix("sha256:") else {
		return false;
	};
	hex.len() == 64
		&& hex
			.bytes()
			.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn named(slug: &str, suffix: u8) -> NamedRef {
		NamedRef {
			slug: slug.to_owned(),
			id: Uuid::from_u128(u128::from_be_bytes([
				0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, suffix,
			])),
		}
	}

	fn environment() -> EnvironmentRef {
		EnvironmentRef {
			organization: named("acme", 1),
			project: named("notes", 2),
			environment: named("production", 3),
		}
	}

	/// The digest of the word `test`, in the form a release names.
	fn digest() -> String {
		"sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".to_owned()
	}

	fn base_spec() -> ReleaseSpec {
		ReleaseSpec {
			environment: environment(),
			deployment: 1,
			processes: vec![Process {
				name: "app".to_owned(),
				kind: ProcessKind::App,
				image: Image {
					repository: "ghcr.io/acme/notes".to_owned(),
					digest: digest(),
				},
				command: vec!["/app/server".to_owned()],
				args: vec![],
				port: 8080,
				probes: Probes {
					ready: "/readyz".to_owned(),
					live: "/healthz".to_owned(),
				},
				env: BTreeMap::new(),
				secret_env: BTreeMap::new(),
				resources: Resources {
					cpu_request_millis: 100,
					cpu_limit_millis: 500,
					memory_mib: 128,
				},
				scale: Scale {
					min: 0,
					max: 1,
					sleep: true,
				},
				volumes: vec![],
			}],
			routes: vec![Route {
				hostname: "notes.leptoskit.app".to_owned(),
				path_prefix: "/".to_owned(),
				process: "app".to_owned(),
				strip_cookies: false,
				tls: Tls::Wildcard,
			}],
			policy: Policy {
				runtime_class: Some("gvisor".to_owned()),
				egress: Egress::Internet,
				quota: Quota {
					cpu_millis: 1000,
					memory_mib: 256,
					pods: 2,
					storage_gib: 1,
				},
				database: None,
			},
		}
	}

	fn assert_fails(spec: &ReleaseSpec, which: &str) -> SpecError {
		spec.validate().expect_err(which)
	}

	fn assert_passes(spec: &ReleaseSpec, which: &str) {
		assert_eq!(spec.validate(), Ok(()), "{which}");
	}

	#[test]
	fn the_namespace_is_built_from_the_ids_alone() {
		let first = environment();
		assert_eq!(first.namespace(), "lk-6211248c257b072bf4f6");
		let mut renamed = first.clone();
		renamed.project.slug = "renamed".to_owned();
		assert_eq!(
			renamed.namespace(),
			first.namespace(),
			"a slug rename moves nothing"
		);
		let mut different = first;
		different.environment.id = named("staging", 4).id;
		assert_ne!(different.namespace(), "lk-6211248c257b072bf4f6");
		assert!(different.namespace().starts_with("lk-"));
		assert_eq!(different.namespace().len(), 23);
	}

	#[test]
	fn a_valid_spec_passes() {
		assert_passes(&base_spec(), "the base spec");
	}

	#[test]
	fn minimum_quota_includes_surge_but_not_for_recreate() {
		let mut spec = base_spec();
		spec.processes[0].scale.min = 1;
		spec.processes[0].scale.sleep = false;
		spec.policy.quota = Quota::for_processes(&spec.processes);
		assert_eq!(
			spec.policy.quota,
			Quota {
				cpu_millis: 1000,
				memory_mib: 256,
				pods: 2,
				storage_gib: 0,
			}
		);
		assert_passes(&spec, "one xs plus its surge pod");
		spec.policy.quota.pods -= 1;
		assert!(matches!(spec.validate(), Err(SpecError::QuotaPods { .. })));
		spec.processes[0].volumes.push(Volume {
			name: "data".to_owned(),
			mount_path: "/data".to_owned(),
			size_gib: 1,
		});
		spec.policy.quota = Quota::for_processes(&spec.processes);
		assert_eq!(spec.policy.quota.pods, 1);
		assert_eq!(spec.policy.quota.cpu_millis, 500);
		assert_passes(&spec, "recreate does not surge");
	}

	#[test]
	fn process_names_must_be_dns_labels() {
		let mut spec = base_spec();
		spec.processes[0].name = "App".to_owned();
		assert!(matches!(
			assert_fails(&spec, "uppercase"),
			SpecError::ProcessName { .. }
		));

		let mut spec = base_spec();
		spec.processes[0].name = "-app".to_owned();
		assert!(matches!(
			assert_fails(&spec, "leading hyphen"),
			SpecError::ProcessName { .. }
		));

		let mut spec = base_spec();
		spec.processes[0].name = "1app".to_owned();
		assert!(matches!(
			assert_fails(&spec, "leading digit"),
			SpecError::ProcessName { .. }
		));

		let mut spec = base_spec();
		spec.processes[0].name = "a".repeat(64);
		assert!(matches!(
			assert_fails(&spec, "too long"),
			SpecError::ProcessName { .. }
		));

		let mut spec = base_spec();
		spec.processes[0].name = "app-1".to_owned();
		spec.routes[0].process = "app-1".to_owned();
		assert_passes(&spec, "digits and hyphens inside a name");
	}

	#[test]
	fn process_names_must_be_unique() {
		let mut spec = base_spec();
		let mut second = spec.processes[0].clone();
		second.kind = ProcessKind::Service;
		spec.processes.push(second);
		assert!(matches!(
			assert_fails(&spec, "duplicate"),
			SpecError::DuplicateProcess { .. }
		));
	}

	#[test]
	fn exactly_one_app_process() {
		let mut spec = base_spec();
		spec.processes.clear();
		assert!(matches!(assert_fails(&spec, "no app"), SpecError::NoApp));

		let mut spec = base_spec();
		let mut second = spec.processes[0].clone();
		second.name = "twin".to_owned();
		spec.processes.push(second);
		assert!(matches!(
			assert_fails(&spec, "two apps"),
			SpecError::TooManyApps { .. }
		));
	}

	#[test]
	fn routes_must_name_a_process() {
		let mut spec = base_spec();
		spec.routes[0].process = "search".to_owned();
		assert!(matches!(
			assert_fails(&spec, "unknown"),
			SpecError::UnknownRouteProcess { .. }
		));
	}

	#[test]
	fn scale_needs_min_at_most_max() {
		let mut spec = base_spec();
		spec.processes[0].scale = Scale {
			min: 2,
			max: 1,
			sleep: false,
		};
		assert!(matches!(
			assert_fails(&spec, "inverted"),
			SpecError::ScaleInverted { .. }
		));

		let mut spec = base_spec();
		spec.processes[0].scale = Scale {
			min: 2,
			max: 5,
			sleep: false,
		};
		spec.policy.quota = Quota {
			cpu_millis: 3000,
			memory_mib: 768,
			pods: 6,
			storage_gib: 1,
		};
		assert_passes(&spec, "min below max");
	}

	#[test]
	fn sleep_needs_min_zero() {
		let mut spec = base_spec();
		spec.processes[0].scale = Scale {
			min: 1,
			max: 1,
			sleep: true,
		};
		assert!(matches!(
			assert_fails(&spec, "sleeping at one"),
			SpecError::SleepNeedsMinZero { .. }
		));
	}

	#[test]
	fn volumes_need_one_replica() {
		let mut spec = base_spec();
		spec.processes[0].volumes = vec![Volume {
			name: "data".to_owned(),
			mount_path: "/data".to_owned(),
			size_gib: 1,
		}];
		spec.processes[0].scale = Scale {
			min: 1,
			max: 2,
			sleep: false,
		};
		assert!(matches!(
			assert_fails(&spec, "scaling a volume"),
			SpecError::VolumesNeedOneReplica { .. }
		));

		let mut spec = base_spec();
		spec.processes[0].volumes = vec![Volume {
			name: "data".to_owned(),
			mount_path: "/data".to_owned(),
			size_gib: 1,
		}];
		spec.processes[0].scale = Scale {
			min: 1,
			max: 1,
			sleep: false,
		};
		assert_passes(&spec, "one replica with a volume");
	}

	#[test]
	fn the_quota_holds_the_full_scale() {
		// The checks run in a fixed order — CPU, memory, pods, storage — so
		// each case fits everything before the resource it names.
		let mut spec = base_spec();
		spec.processes[0].scale = Scale {
			min: 1,
			max: 2,
			sleep: false,
		};
		spec.policy.quota.cpu_millis = 999;
		assert!(matches!(
			assert_fails(&spec, "cpu"),
			SpecError::QuotaCpu {
				used: 1500,
				allowed: 999
			}
		));

		let mut spec = base_spec();
		spec.processes[0].scale = Scale {
			min: 1,
			max: 2,
			sleep: false,
		};
		spec.policy.quota.cpu_millis = 1500;
		spec.policy.quota.memory_mib = 255;
		assert!(matches!(
			assert_fails(&spec, "memory"),
			SpecError::QuotaMemory {
				used: 384,
				allowed: 255
			}
		));

		let mut spec = base_spec();
		spec.processes[0].scale = Scale {
			min: 1,
			max: 2,
			sleep: false,
		};
		spec.policy.quota.cpu_millis = 1500;
		spec.policy.quota.memory_mib = 384;
		spec.policy.quota.pods = 1;
		assert!(matches!(
			assert_fails(&spec, "pods"),
			SpecError::QuotaPods {
				used: 3,
				allowed: 1
			}
		));

		let mut spec = base_spec();
		spec.processes[0].volumes = vec![Volume {
			name: "data".to_owned(),
			mount_path: "/data".to_owned(),
			size_gib: 2,
		}];
		spec.processes[0].scale = Scale {
			min: 1,
			max: 1,
			sleep: false,
		};
		spec.policy.quota.storage_gib = 1;
		assert!(matches!(
			assert_fails(&spec, "storage"),
			SpecError::QuotaStorage {
				used: 2,
				allowed: 1
			}
		));

		let mut spec = base_spec();
		spec.processes[0].scale = Scale {
			min: 1,
			max: 2,
			sleep: false,
		};
		spec.policy.quota = Quota {
			cpu_millis: 1500,
			memory_mib: 384,
			pods: 3,
			storage_gib: 0,
		};
		assert_passes(&spec, "an exact fit");
	}

	#[test]
	fn hostnames_must_be_valid() {
		for hostname in [
			"notes",
			"Notes.example.com",
			"notes..com",
			"-notes.example.com",
			"notes.exa mple.com",
		] {
			let mut spec = base_spec();
			spec.routes[0].hostname = hostname.to_owned();
			assert!(matches!(
				assert_fails(&spec, hostname),
				SpecError::Hostname { .. }
			));
		}

		let mut spec = base_spec();
		spec.routes[0].hostname = "9notes.example.com".to_owned();
		assert_passes(&spec, "a leading digit is a legal host label");
	}

	#[test]
	fn path_prefixes_must_start_with_a_slash() {
		let mut spec = base_spec();
		spec.routes[0].path_prefix = "notes".to_owned();
		assert!(matches!(
			assert_fails(&spec, "no slash"),
			SpecError::PathPrefix { .. }
		));
	}

	#[test]
	fn environment_names_must_be_identifiers() {
		let mut spec = base_spec();
		spec.processes[0]
			.env
			.insert("9LOG".to_owned(), "info".to_owned());
		assert!(matches!(
			assert_fails(&spec, "digit first"),
			SpecError::InvalidEnvName { .. }
		));

		let mut spec = base_spec();
		spec.processes[0]
			.secret_env
			.insert("LOG-LEVEL".to_owned(), "log-level".to_owned());
		assert!(matches!(
			assert_fails(&spec, "hyphen"),
			SpecError::InvalidEnvName { .. }
		));

		let mut spec = base_spec();
		spec.processes[0]
			.secret_env
			.insert("_PRIVATE".to_owned(), "private".to_owned());
		assert_passes(&spec, "underscore-prefixed secret variable");
	}

	#[test]
	fn a_variable_cannot_be_plain_and_secret() {
		let mut spec = base_spec();
		spec.processes[0]
			.env
			.insert("TOKEN".to_owned(), "plain".to_owned());
		spec.processes[0]
			.secret_env
			.insert("TOKEN".to_owned(), "token".to_owned());
		assert!(matches!(
			assert_fails(&spec, "conflict"),
			SpecError::EnvNameConflict { .. }
		));
	}

	#[test]
	fn ports_must_be_nonzero() {
		let mut spec = base_spec();
		spec.processes[0].port = 0;
		assert!(matches!(
			assert_fails(&spec, "zero port"),
			SpecError::InvalidPort { .. }
		));
	}

	#[test]
	fn digests_must_be_well_formed() {
		let mut spec = base_spec();
		spec.processes[0].image.digest = "v1.2.3".to_owned();
		assert!(matches!(
			assert_fails(&spec, "a tag"),
			SpecError::InvalidDigest { .. }
		));

		let mut spec = base_spec();
		spec.processes[0].image.digest = format!("sha256:{}", "a".repeat(63));
		assert!(matches!(
			assert_fails(&spec, "short hex"),
			SpecError::InvalidDigest { .. }
		));

		let mut spec = base_spec();
		spec.processes[0].image.digest = format!("sha256:{}", "A".repeat(64));
		assert!(matches!(
			assert_fails(&spec, "uppercase"),
			SpecError::InvalidDigest { .. }
		));

		let mut spec = base_spec();
		spec.processes[0].image.digest = format!("sha512:{}", "a".repeat(64));
		assert!(matches!(
			assert_fails(&spec, "wrong algorithm"),
			SpecError::InvalidDigest { .. }
		));
	}

	#[test]
	fn volumes_need_dns_label_names() {
		let mut spec = base_spec();
		spec.processes[0].volumes = vec![Volume {
			name: "data/data".to_owned(),
			mount_path: "/data".to_owned(),
			size_gib: 1,
		}];
		spec.processes[0].scale = Scale {
			min: 1,
			max: 1,
			sleep: false,
		};
		assert!(matches!(
			assert_fails(&spec, "slash in name"),
			SpecError::VolumeName { .. }
		));
	}

	#[test]
	fn the_name_helpers_match_the_rules_they_document() {
		assert!(is_dns_label("app-1"));
		assert!(!is_dns_label(""));
		assert!(!is_dns_label("App"));
		assert!(!is_dns_label("app-"));
		assert!(!is_dns_label(&"a".repeat(64)));

		assert!(is_hostname("notes.example.com"));
		assert!(!is_hostname("localhost"));
		assert!(!is_hostname("notes.example.com-"));
		assert!(!is_hostname(&format!("{}.com", "a".repeat(253))));
	}
}
