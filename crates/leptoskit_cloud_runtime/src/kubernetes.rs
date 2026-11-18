//! Kubernetes manifests for one release.
//!
//! [`manifests`] turns a [`ReleaseSpec`] into the objects a cluster needs,
//! in one deterministic order: the same spec, the same cluster profile, and
//! the same secret values always produce the same bytes, so an apply can be
//! planned, shown, and repeated. The generator is pure — no clocks, no
//! randomness, no cluster access. What the cluster cannot honor is refused
//! here, before anything is applied.
//!
//! The objects are typed: built-in kinds come from `k8s-openapi`, and the
//! KEDA HTTP add-on's custom resource is a small local struct, since the
//! cluster installs the CRD itself.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use k8s_openapi::ByteString;
use k8s_openapi::api::apps::v1 as apps;
use k8s_openapi::api::autoscaling::v2 as autoscaling;
use k8s_openapi::api::core::v1 as core;
use k8s_openapi::api::networking::v1 as networking;
use k8s_openapi::api::policy::v1 as policy;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1 as meta;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use serde::Serialize;

use crate::spec::DatabaseAccess;
use crate::spec::Egress;
use crate::spec::Policy;
use crate::spec::Process;
use crate::spec::ReleaseSpec;
use crate::spec::Route;
use crate::spec::SpecError;
use crate::spec::Tls;
use crate::spec::Volume;

/// The standard label Kubernetes' own tooling reads a workload's name from.
/// It selects every object of one process.
const NAME_LABEL: &str = "app.kubernetes.io/name";

/// The label naming who built these objects.
const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";

/// The value of [`MANAGED_BY_LABEL`], so an operator can find and keep or
/// delete everything `LeptosKit` Cloud built.
const MANAGED_BY: &str = "leptoskit-cloud";

/// The prefix of this crate's own labels: organisation, project,
/// environment, and deployment number.
const LEPTOSKIT_LABEL_PREFIX: &str = "leptoskit.cloud/";

/// The label Kubernetes records a namespace's own name in, which
/// `NetworkPolicy` objects use to name other namespaces without a second
/// lookup.
const NAMESPACE_NAME_LABEL: &str = "kubernetes.io/metadata.name";

/// The name of the one service account every pod of the environment runs
/// as, with no token mounted.
const SERVICE_ACCOUNT: &str = "app";

/// The emptyDir that gives a read-only-root container a writable `/tmp`.
const TMP_VOLUME: &str = "tmp";

/// How a process's replicas are owned, decided once and read by both the
/// Deployment and the scaling stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scaling {
	/// KEDA's HTTP add-on: the process sleeps at zero and wakes on request.
	Sleeping,
	/// The horizontal pod autoscaler, on CPU use.
	Autoscaling,
	/// Nothing: the Deployment pins the release's minimum.
	Fixed,
}

/// What a cluster looks like to the manifest generator: which edge fronts
/// it, what certificates it can hand out, and which add-ons are installed.
/// Operator configuration, not customer data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterProfile {
	/// The Ingress class the edge serves, such as `nginx`.
	pub ingress_class: String,
	/// The namespace the edge's controllers run in, the one tenant pods
	/// accept traffic from.
	pub ingress_namespace: String,
	/// The wildcard certificate's secret name, copied into every tenant
	/// namespace for the default hostnames.
	pub wildcard_tls_secret: Option<String>,
	/// The cert-manager cluster issuer that issues per-host certificates.
	pub cluster_issuer: Option<String>,
	/// The namespace the cluster's DNS resolves from.
	pub dns_namespace: String,
	/// The ranges internet egress may not reach: the cluster's pod and
	/// service CIDRs, link-local, and the private ranges.
	pub blocked_egress_cidrs: Vec<String>,
	/// The image pull secret, for registries that ask for one.
	pub image_pull_secret: Option<String>,
	/// Whether KEDA's HTTP add-on is installed, which is what lets a
	/// process sleep at `min = 0` and wake on the first request.
	pub keda_http: bool,
}

impl Default for ClusterProfile {
	/// An edgeless cluster that resolves DNS in `kube-system`: every other
	/// field needs an operator's answer before a real apply, so there is no
	/// useful built-in profile.
	fn default() -> Self {
		Self {
			ingress_class: String::new(),
			ingress_namespace: String::new(),
			wildcard_tls_secret: None,
			cluster_issuer: None,
			dns_namespace: "kube-system".to_owned(),
			blocked_egress_cidrs: Vec::new(),
			image_pull_secret: None,
			keda_http: false,
		}
	}
}

/// The values behind a release spec's `secret_env` keys.
///
/// The map is deliberately not `Serialize`, so a value cannot be serialized
/// into anything but a Secret's data, and `Debug` and `Display` print
/// nothing but `[redacted]`, so no log line or error can carry one.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretValues {
	inner: BTreeMap<String, String>,
}

impl SecretValues {
	/// Values keyed the way the spec's `secret_env` names them.
	#[must_use]
	pub fn new(inner: BTreeMap<String, String>) -> Self {
		Self { inner }
	}

	/// The value one key holds, for the one place a Secret is written.
	#[must_use]
	pub fn get(&self, key: &str) -> Option<&str> {
		self.inner.get(key).map(String::as_str)
	}
}

impl std::fmt::Debug for SecretValues {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str("[redacted]")
	}
}

impl std::fmt::Display for SecretValues {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str("[redacted]")
	}
}

/// Why a release cannot be turned into manifests.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
	/// The spec is not applicable anywhere; fix it first.
	#[error("the release spec is invalid: {0}")]
	Invalid(#[from] SpecError),
	#[error(
		"the process {process:?} cannot sleep: this cluster has no HTTP scaler (`keda_http` is \
		 false), so min = 0 would take every replica away and never bring one back. Install the \
		 KEDA HTTP add-on on the cluster, or set the process's minimum to 1"
	)]
	SleepWithoutScaler { process: String },
	#[error(
		"the route {prefix:?} on {hostname} points at the process {process:?}, which strips \
		 cookies, but a plain Ingress cannot remove the app's Cookie header from requests or \
		 Set-Cookie from answers, so the process would receive every visitor's session. Make the \
		 route private, serve it from the cloud's edge, or trust the process with the app's \
		 cookies"
	)]
	StripCookies {
		hostname: String,
		prefix: String,
		process: String,
	},
	#[error(
		"no value for the secret key {key:?}, which the process {process:?} reads as \
		 {variable:?}. Give the release's secrets a value for it before applying"
	)]
	MissingSecret {
		process: String,
		variable: String,
		key: String,
	},
	#[error(
		"{hostname} asks for an issued certificate, but the cluster names no cert-manager cluster \
		 issuer (`cluster_issuer` is unset). Set one on the cluster profile, or use the wildcard \
		 certificate"
	)]
	NoIssuer { hostname: String },
	#[error(
		"{hostname} uses the wildcard certificate, but the cluster profile names no wildcard \
		 secret"
	)]
	NoWildcardSecret { hostname: String },
	#[error("the routes on {hostname} disagree about TLS: one host is served by one certificate")]
	MixedTls { hostname: String },
	#[error("the manifests could not be written as YAML: {0}")]
	Yaml(String),
}

/// One Kubernetes object of a release. Serializes to the exact JSON or YAML
/// the API server receives — `apiVersion`, `kind`, `metadata`, and the body
/// together, with no wrapper around them.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
#[expect(
	clippy::large_enum_variant,
	reason = "one enum holds whole Kubernetes objects, and a Deployment is larger than a \
	          Namespace by design"
)]
pub enum Object {
	Namespace(core::Namespace),
	ResourceQuota(core::ResourceQuota),
	LimitRange(core::LimitRange),
	NetworkPolicy(networking::NetworkPolicy),
	ServiceAccount(core::ServiceAccount),
	Secret(core::Secret),
	PersistentVolumeClaim(core::PersistentVolumeClaim),
	Deployment(apps::Deployment),
	Service(core::Service),
	HorizontalPodAutoscaler(autoscaling::HorizontalPodAutoscaler),
	PodDisruptionBudget(policy::PodDisruptionBudget),
	Ingress(networking::Ingress),
	HttpScaledObject(HttpScaledObject),
}

impl Object {
	/// The object's Kubernetes kind.
	#[must_use]
	pub fn kind(&self) -> &'static str {
		match self {
			Self::Namespace(_) => "Namespace",
			Self::ResourceQuota(_) => "ResourceQuota",
			Self::LimitRange(_) => "LimitRange",
			Self::NetworkPolicy(_) => "NetworkPolicy",
			Self::ServiceAccount(_) => "ServiceAccount",
			Self::Secret(_) => "Secret",
			Self::PersistentVolumeClaim(_) => "PersistentVolumeClaim",
			Self::Deployment(_) => "Deployment",
			Self::Service(_) => "Service",
			Self::HorizontalPodAutoscaler(_) => "HorizontalPodAutoscaler",
			Self::PodDisruptionBudget(_) => "PodDisruptionBudget",
			Self::Ingress(_) => "Ingress",
			Self::HttpScaledObject(_) => "HTTPScaledObject",
		}
	}

	/// The object's name. Every object this crate builds names itself, so
	/// this is always present for them.
	#[must_use]
	pub fn name(&self) -> Option<&str> {
		self.object_meta().name.as_deref()
	}

	/// The object's namespace. Only the Namespace object itself has none.
	#[must_use]
	pub fn namespace(&self) -> Option<&str> {
		self.object_meta().namespace.as_deref()
	}

	/// The object as one YAML document, ending in a newline.
	///
	/// # Errors
	/// [`ManifestError::Yaml`] when the object cannot be written as YAML,
	/// which the objects this crate builds never produce.
	pub fn to_yaml(&self) -> Result<String, ManifestError> {
		yaml_serde::to_string(self).map_err(|error| ManifestError::Yaml(error.to_string()))
	}

	fn object_meta(&self) -> &meta::ObjectMeta {
		match self {
			Self::Namespace(o) => &o.metadata,
			Self::ResourceQuota(o) => &o.metadata,
			Self::LimitRange(o) => &o.metadata,
			Self::NetworkPolicy(o) => &o.metadata,
			Self::ServiceAccount(o) => &o.metadata,
			Self::Secret(o) => &o.metadata,
			Self::PersistentVolumeClaim(o) => &o.metadata,
			Self::Deployment(o) => &o.metadata,
			Self::Service(o) => &o.metadata,
			Self::HorizontalPodAutoscaler(o) => &o.metadata,
			Self::PodDisruptionBudget(o) => &o.metadata,
			Self::Ingress(o) => &o.metadata,
			Self::HttpScaledObject(o) => &o.metadata,
		}
	}
}

/// KEDA's HTTP scaled object: the sleeping process's target, its replica
/// bounds, and the hosts and prefixes that wake it. The cluster installs
/// the CRD, so this is a plain serde struct, not a `k8s-openapi` type. The
/// fields are the object's body exactly as it serializes.
#[derive(Debug, Clone, Serialize)]
pub struct HttpScaledObject {
	#[serde(rename = "apiVersion")]
	pub api_version: &'static str,
	pub kind: &'static str,
	pub metadata: meta::ObjectMeta,
	pub spec: HttpScaledObjectSpec,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpScaledObjectSpec {
	#[serde(rename = "scaleTargetRef")]
	pub scale_target_ref: HttpScaleTargetRef,
	#[serde(rename = "replicasMin")]
	pub replicas_min: u32,
	#[serde(rename = "replicasMax")]
	pub replicas_max: u32,
	pub triggers: Vec<HttpTrigger>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpScaleTargetRef {
	pub name: String,
	#[serde(rename = "apiVersion")]
	pub api_version: &'static str,
	pub kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpTrigger {
	#[serde(rename = "type")]
	pub kind: &'static str,
	pub metadata: BTreeMap<String, String>,
}

/// The Kubernetes objects of one release, in one deterministic order: the
/// namespace first, then what shapes it (quota, limit range, network), then
/// what runs in it (account, secrets, claims, deployments, services,
/// scaling, disruption budgets), then what fronts it (ingresses).
///
/// # Errors
/// [`ManifestError`] when the spec cannot be honored: an invalid spec, a
/// secret with no value, sleep on a cluster that cannot wake a sleeping
/// process, cookies to strip where the edge cannot, or a certificate
/// nobody can issue.
pub fn manifests(
	spec: &ReleaseSpec,
	cluster: &ClusterProfile,
	secrets: &SecretValues,
) -> Result<Vec<Object>, ManifestError> {
	spec.validate()?;
	let namespace = spec.environment.namespace();
	let labels = labels(spec);
	let mut objects = Vec::new();
	objects.push(namespace_object(&namespace, &labels));
	objects.extend(quota_objects(spec, &namespace, &labels));
	objects.extend(network_policies(spec, cluster, &namespace, &labels));
	objects.push(service_account_object(&namespace, &labels));
	objects.extend(secret_objects(spec, secrets, &namespace, &labels)?);
	objects.extend(claim_objects(spec, &namespace, &labels));
	objects.extend(deployment_objects(spec, cluster, &namespace, &labels)?);
	objects.extend(service_objects(spec, &namespace, &labels));
	objects.extend(scaling_objects(spec, cluster, &namespace, &labels)?);
	objects.extend(budget_objects(spec, &namespace, &labels));
	objects.extend(ingress_objects(spec, cluster, &namespace, &labels)?);
	Ok(objects)
}

/// The labels every object of the release carries: who manages it and which
/// organisation, project, environment, and deployment it serves.
fn labels(spec: &ReleaseSpec) -> BTreeMap<String, String> {
	BTreeMap::from([
		(MANAGED_BY_LABEL.to_owned(), MANAGED_BY.to_owned()),
		(
			format!("{LEPTOSKIT_LABEL_PREFIX}organization"),
			spec.environment.organization.slug.clone(),
		),
		(
			format!("{LEPTOSKIT_LABEL_PREFIX}project"),
			spec.environment.project.slug.clone(),
		),
		(
			format!("{LEPTOSKIT_LABEL_PREFIX}environment"),
			spec.environment.environment.slug.clone(),
		),
		(
			format!("{LEPTOSKIT_LABEL_PREFIX}deployment"),
			spec.deployment.to_string(),
		),
	])
}

fn object_meta(
	namespace: Option<&str>,
	name: &str,
	labels: &BTreeMap<String, String>,
) -> meta::ObjectMeta {
	meta::ObjectMeta {
		name: Some(name.to_owned()),
		namespace: namespace.map(str::to_owned),
		labels: Some(labels.clone()),
		..Default::default()
	}
}

fn quantity(value: String) -> Quantity {
	Quantity(value)
}

fn cpu_millis(millis: u32) -> Quantity {
	quantity(format!("{millis}m"))
}

fn memory_mib(mib: u32) -> Quantity {
	quantity(format!("{mib}Mi"))
}

fn storage_gib(gib: u32) -> Quantity {
	quantity(format!("{gib}Gi"))
}

/// The namespace, held to the restricted Pod Security profile at every
/// level: nothing in the environment may run privileged.
fn namespace_object(namespace: &str, labels: &BTreeMap<String, String>) -> Object {
	let mut security = labels.clone();
	for level in ["enforce", "warn", "audit"] {
		security.insert(
			format!("pod-security.kubernetes.io/{level}"),
			"restricted".to_owned(),
		);
	}
	Object::Namespace(core::Namespace {
		metadata: object_meta(None, namespace, &security),
		spec: None,
		status: None,
	})
}

/// The quota and the limit range, from the policy's numbers: what the whole
/// environment may take, and the most any one container may ask for.
fn quota_objects(
	spec: &ReleaseSpec,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Vec<Object> {
	let quota = &spec.policy.quota;
	let mut objects = vec![Object::ResourceQuota(core::ResourceQuota {
		metadata: object_meta(Some(namespace), "quota", labels),
		spec: Some(core::ResourceQuotaSpec {
			hard: Some(BTreeMap::from([
				("requests.cpu".to_owned(), cpu_millis(quota.cpu_millis)),
				("limits.cpu".to_owned(), cpu_millis(quota.cpu_millis)),
				("requests.memory".to_owned(), memory_mib(quota.memory_mib)),
				("limits.memory".to_owned(), memory_mib(quota.memory_mib)),
				("pods".to_owned(), quantity(quota.pods.to_string())),
				(
					"requests.storage".to_owned(),
					storage_gib(quota.storage_gib),
				),
			])),
			scopes: None,
			scope_selector: None,
		}),
		status: None,
	})];
	objects.push(Object::LimitRange(core::LimitRange {
		metadata: object_meta(Some(namespace), "limits", labels),
		spec: Some(core::LimitRangeSpec {
			limits: vec![core::LimitRangeItem {
				type_: "Container".to_owned(),
				max: Some(BTreeMap::from([
					("cpu".to_owned(), cpu_millis(quota.cpu_millis)),
					("memory".to_owned(), memory_mib(quota.memory_mib)),
				])),
				min: None,
				default: None,
				default_request: None,
				max_limit_request_ratio: None,
			}],
		}),
	}));
	objects
}

/// Every pod of the environment starts with nothing allowed — no way in,
/// no way out — and the policies after the first one open exactly the doors
/// the release asks for, nothing more.
fn network_policies(
	spec: &ReleaseSpec,
	cluster: &ClusterProfile,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Vec<Object> {
	let mut policies = vec![
		network_policy(
			"default-deny",
			namespace,
			labels,
			&["Ingress", "Egress"],
			Some(Vec::new()),
			Some(Vec::new()),
		),
		network_policy(
			"allow-same-namespace",
			namespace,
			labels,
			&["Ingress", "Egress"],
			Some(vec![ingress_from_namespace_peer()]),
			Some(vec![egress_to_namespace_peer()]),
		),
	];
	policies.extend(ingress_policies(spec, cluster, namespace, labels));
	policies.push(dns_policy(cluster, namespace, labels));
	if spec.policy.egress == Egress::Internet {
		policies.push(internet_policy(cluster, namespace, labels));
	}
	if let Some(database) = &spec.policy.database {
		policies.push(database_policy(database, namespace, labels));
	}
	policies
}

fn network_policy(
	name: &str,
	namespace: &str,
	labels: &BTreeMap<String, String>,
	types: &[&str],
	ingress: Option<Vec<networking::NetworkPolicyIngressRule>>,
	egress: Option<Vec<networking::NetworkPolicyEgressRule>>,
) -> Object {
	Object::NetworkPolicy(networking::NetworkPolicy {
		metadata: object_meta(Some(namespace), name, labels),
		spec: Some(spec_for_types(types, ingress, egress)),
	})
}

fn spec_for_types(
	types: &[&str],
	ingress: Option<Vec<networking::NetworkPolicyIngressRule>>,
	egress: Option<Vec<networking::NetworkPolicyEgressRule>>,
) -> networking::NetworkPolicySpec {
	networking::NetworkPolicySpec {
		// Every pod of the namespace: the allows are additive on top of the
		// default deny.
		pod_selector: Some(meta::LabelSelector::default()),
		policy_types: Some(types.iter().map(|t| (*t).to_owned()).collect()),
		ingress,
		egress,
	}
}

/// Traffic between the environment's own pods, in both directions, on any
/// port.
fn same_namespace_peer() -> networking::NetworkPolicyPeer {
	networking::NetworkPolicyPeer {
		pod_selector: Some(meta::LabelSelector::default()),
		namespace_selector: None,
		ip_block: None,
	}
}

fn ingress_from_namespace_peer() -> networking::NetworkPolicyIngressRule {
	networking::NetworkPolicyIngressRule {
		ports: None,
		from: Some(vec![same_namespace_peer()]),
	}
}

fn egress_to_namespace_peer() -> networking::NetworkPolicyEgressRule {
	networking::NetworkPolicyEgressRule {
		ports: None,
		to: Some(vec![same_namespace_peer()]),
	}
}

fn peer_from_namespace(namespace: &str) -> networking::NetworkPolicyPeer {
	networking::NetworkPolicyPeer {
		namespace_selector: Some(meta::LabelSelector {
			match_labels: Some(BTreeMap::from([(
				NAMESPACE_NAME_LABEL.to_owned(),
				namespace.to_owned(),
			)])),
			match_expressions: None,
		}),
		pod_selector: None,
		ip_block: None,
	}
}

fn tcp_door(port: u16) -> networking::NetworkPolicyPort {
	networking::NetworkPolicyPort {
		protocol: Some("TCP".to_owned()),
		port: Some(IntOrString::Int(i32::from(port))),
		end_port: None,
	}
}

fn udp_door(port: u16) -> networking::NetworkPolicyPort {
	networking::NetworkPolicyPort {
		protocol: Some("UDP".to_owned()),
		port: Some(IntOrString::Int(i32::from(port))),
		end_port: None,
	}
}

/// One policy per routed process: the edge's namespace may reach that
/// process on its own port, and nothing else reaches it from outside. The
/// pod selector keeps each policy on its own process's pods.
fn ingress_policies(
	spec: &ReleaseSpec,
	cluster: &ClusterProfile,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Vec<Object> {
	let mut routed: Vec<&Process> = Vec::new();
	for route in &spec.routes {
		let Some(process) = spec.processes.iter().find(|p| p.name == route.process) else {
			// The spec was validated: a route always names a process.
			continue;
		};
		if !routed.contains(&process) {
			routed.push(process);
		}
	}
	routed
		.into_iter()
		.map(|process| ingress_policy_for(process, cluster, namespace, labels))
		.collect()
}

fn ingress_policy_for(
	process: &Process,
	cluster: &ClusterProfile,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Object {
	let mut policy_labels = labels.clone();
	policy_labels.insert(NAME_LABEL.to_owned(), process.name.clone());
	let from = networking::NetworkPolicyIngressRule {
		ports: Some(vec![tcp_door(process.port)]),
		from: Some(vec![peer_from_namespace(&cluster.ingress_namespace)]),
	};
	Object::NetworkPolicy(networking::NetworkPolicy {
		metadata: object_meta(
			Some(namespace),
			&format!("allow-from-ingress-{}", process.name),
			&policy_labels,
		),
		spec: Some(networking::NetworkPolicySpec {
			pod_selector: Some(meta::LabelSelector {
				match_labels: Some(BTreeMap::from([(
					NAME_LABEL.to_owned(),
					process.name.clone(),
				)])),
				match_expressions: None,
			}),
			policy_types: Some(vec!["Ingress".to_owned()]),
			ingress: Some(vec![from]),
			egress: None,
		}),
	})
}

/// DNS in the cluster's own namespace, over UDP and TCP 53. Even under
/// [`Egress::None`] the door stays open: a process with no other egress can
/// still resolve names.
fn dns_policy(
	cluster: &ClusterProfile,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Object {
	let to = vec![networking::NetworkPolicyEgressRule {
		to: Some(vec![peer_from_namespace(&cluster.dns_namespace)]),
		ports: Some(vec![udp_door(53), tcp_door(53)]),
	}];
	network_policy("allow-dns", namespace, labels, &["Egress"], None, Some(to))
}

/// The internet, except the ranges the cluster operator blocked: its own
/// pod and service CIDRs, link-local, and the private ranges. A cloud
/// metadata endpoint rides link-local, so a tenant pod cannot reach one.
fn internet_policy(
	cluster: &ClusterProfile,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Object {
	let to = vec![networking::NetworkPolicyEgressRule {
		to: Some(vec![networking::NetworkPolicyPeer {
			ip_block: Some(networking::IPBlock {
				cidr: "0.0.0.0/0".to_owned(),
				except: (!cluster.blocked_egress_cidrs.is_empty())
					.then(|| cluster.blocked_egress_cidrs.clone()),
			}),
			namespace_selector: None,
			pod_selector: None,
		}]),
		ports: None,
	}];
	network_policy(
		"allow-internet",
		namespace,
		labels,
		&["Egress"],
		None,
		Some(to),
	)
}

/// The database's namespace, on its port and nothing else.
fn database_policy(
	database: &DatabaseAccess,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Object {
	let to = vec![networking::NetworkPolicyEgressRule {
		to: Some(vec![peer_from_namespace(&database.namespace)]),
		ports: Some(vec![tcp_door(database.port)]),
	}];
	network_policy(
		"allow-database",
		namespace,
		labels,
		&["Egress"],
		None,
		Some(to),
	)
}

/// The one service account every pod runs as. No token is mounted: nothing
/// in a tenant environment talks to the API server.
fn service_account_object(namespace: &str, labels: &BTreeMap<String, String>) -> Object {
	Object::ServiceAccount(core::ServiceAccount {
		metadata: object_meta(Some(namespace), SERVICE_ACCOUNT, labels),
		automount_service_account_token: Some(false),
		image_pull_secrets: None,
		secrets: None,
	})
}

/// One Secret per process that reads any, with the values base64-encoded
/// into the data — the only place a secret value ever appears.
fn secret_objects(
	spec: &ReleaseSpec,
	secrets: &SecretValues,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Result<Vec<Object>, ManifestError> {
	let mut objects = Vec::new();
	for process in &spec.processes {
		if process.secret_env.is_empty() {
			continue;
		}
		let mut data = BTreeMap::new();
		for (variable, key) in &process.secret_env {
			let value = secrets.get(key).ok_or_else(|| {
				ManifestError::MissingSecret {
					process: process.name.clone(),
					variable: variable.clone(),
					key: key.clone(),
				}
			})?;
			// ByteString serializes as base64, the one form a secret value
			// ever takes outside the vault.
			data.insert(key.clone(), ByteString(value.as_bytes().to_vec()));
		}
		objects.push(Object::Secret(core::Secret {
			metadata: object_meta(Some(namespace), &secret_name(process), labels),
			type_: Some("Opaque".to_owned()),
			data: Some(data),
			immutable: None,
			string_data: None,
		}));
	}
	Ok(objects)
}

fn claim_objects(
	spec: &ReleaseSpec,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Vec<Object> {
	let mut objects = Vec::new();
	for process in &spec.processes {
		for volume in &process.volumes {
			objects.push(Object::PersistentVolumeClaim(core::PersistentVolumeClaim {
				metadata: object_meta(Some(namespace), &claim_name(process, volume), labels),
				spec: Some(core::PersistentVolumeClaimSpec {
					access_modes: Some(vec!["ReadWriteOnce".to_owned()]),
					resources: Some(core::VolumeResourceRequirements {
						requests: Some(BTreeMap::from([(
							"storage".to_owned(),
							storage_gib(volume.size_gib),
						)])),
						limits: None,
					}),
					..Default::default()
				}),
				status: None,
			}));
		}
	}
	objects
}

fn deployment_objects(
	spec: &ReleaseSpec,
	cluster: &ClusterProfile,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Result<Vec<Object>, ManifestError> {
	spec.processes
		.iter()
		.map(|process| deployment(spec, cluster, process, namespace, labels))
		.collect()
}

fn deployment(
	spec: &ReleaseSpec,
	cluster: &ClusterProfile,
	process: &Process,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Result<Object, ManifestError> {
	let pinned = matches!(scaling_for(process, cluster)?, Scaling::Fixed);
	let mut pod_labels = labels.clone();
	pod_labels.insert(NAME_LABEL.to_owned(), process.name.clone());
	let deployment = apps::Deployment {
		metadata: object_meta(Some(namespace), &process.name, labels),
		spec: Some(apps::DeploymentSpec {
			// An autoscaler owns the count when one is running; otherwise
			// the release's minimum pins it.
			replicas: pinned.then_some(process.scale.min as i32),
			selector: meta::LabelSelector {
				match_labels: Some(BTreeMap::from([(
					NAME_LABEL.to_owned(),
					process.name.clone(),
				)])),
				match_expressions: None,
			},
			strategy: Some(deployment_strategy(process)),
			template: core::PodTemplateSpec {
				metadata: Some(meta::ObjectMeta {
					labels: Some(pod_labels),
					..Default::default()
				}),
				spec: Some(pod_spec(&spec.policy, cluster, process)),
			},
			..Default::default()
		}),
		status: None,
	};
	Ok(Object::Deployment(deployment))
}

/// Rolling updates replace pods one at a time and never stop the last
/// serving pod before its replacement is ready. A process with volumes
/// cannot roll — two replicas would mount two copies of one claim — so it
/// is replaced instead.
fn deployment_strategy(process: &Process) -> apps::DeploymentStrategy {
	if process.volumes.is_empty() {
		apps::DeploymentStrategy {
			type_: Some("RollingUpdate".to_owned()),
			rolling_update: Some(apps::RollingUpdateDeployment {
				max_unavailable: Some(IntOrString::Int(0)),
				max_surge: Some(IntOrString::Int(1)),
			}),
		}
	} else {
		apps::DeploymentStrategy {
			type_: Some("Recreate".to_owned()),
			rolling_update: None,
		}
	}
}

fn pod_spec(policy: &Policy, cluster: &ClusterProfile, process: &Process) -> core::PodSpec {
	core::PodSpec {
		containers: vec![container(process)],
		service_account_name: Some(SERVICE_ACCOUNT.to_owned()),
		automount_service_account_token: Some(false),
		runtime_class_name: policy.runtime_class.clone(),
		image_pull_secrets: cluster
			.image_pull_secret
			.as_ref()
			.map(|name| vec![core::LocalObjectReference { name: name.clone() }]),
		security_context: Some(pod_security()),
		termination_grace_period_seconds: Some(30),
		volumes: Some(pod_volumes(process)),
		..Default::default()
	}
}

/// The pod runs as a non-root user under the runtime's default seccomp
/// filter. No user is pinned by number: that is the image's job, and
/// `runAsNonRoot` refuses an image that only offers root.
fn pod_security() -> core::PodSecurityContext {
	core::PodSecurityContext {
		run_as_non_root: Some(true),
		seccomp_profile: Some(core::SeccompProfile {
			type_: "RuntimeDefault".to_owned(),
			localhost_profile: None,
		}),
		..Default::default()
	}
}

fn container(process: &Process) -> core::Container {
	core::Container {
		name: process.name.clone(),
		image: Some(process.image.reference()),
		// An empty command or argument list keeps the image's own.
		command: (!process.command.is_empty()).then(|| process.command.clone()),
		args: (!process.args.is_empty()).then(|| process.args.clone()),
		ports: Some(vec![core::ContainerPort {
			container_port: i32::from(process.port),
			protocol: Some("TCP".to_owned()),
			..Default::default()
		}]),
		env: (!process.env.is_empty()).then(|| container_env(process)),
		env_from: container_env_from(process),
		readiness_probe: Some(http_probe(&process.probes.ready, process.port, 5, 5)),
		liveness_probe: Some(http_probe(&process.probes.live, process.port, 10, 10)),
		resources: Some(container_resources(process)),
		security_context: Some(container_security()),
		volume_mounts: Some(container_mounts(process)),
		..Default::default()
	}
}

/// A container that cannot escalate, writes nothing to its own root, and
/// keeps no Linux capability at all.
fn container_security() -> core::SecurityContext {
	core::SecurityContext {
		allow_privilege_escalation: Some(false),
		read_only_root_filesystem: Some(true),
		capabilities: Some(core::Capabilities {
			drop: Some(vec!["ALL".to_owned()]),
			add: None,
		}),
		..Default::default()
	}
}

fn http_probe(path: &str, port: u16, delay: i32, period: i32) -> core::Probe {
	core::Probe {
		http_get: Some(core::HTTPGetAction {
			path: Some(path.to_owned()),
			port: IntOrString::Int(i32::from(port)),
			scheme: Some("HTTP".to_owned()),
			host: None,
			http_headers: None,
		}),
		initial_delay_seconds: Some(delay),
		period_seconds: Some(period),
		..Default::default()
	}
}

fn container_env(process: &Process) -> Vec<core::EnvVar> {
	process
		.env
		.iter()
		.map(|(name, value)| {
			core::EnvVar {
				name: name.clone(),
				value: Some(value.clone()),
				value_from: None,
			}
		})
		.collect()
}

/// The process's secret variables come in whole from its Secret. A missing
/// Secret stops the pod at start, which is the honest answer to a half-set
/// release.
fn container_env_from(process: &Process) -> Option<Vec<core::EnvFromSource>> {
	(!process.secret_env.is_empty()).then(|| {
		vec![core::EnvFromSource {
			prefix: None,
			config_map_ref: None,
			secret_ref: Some(core::SecretEnvSource {
				name: secret_name(process),
				optional: None,
			}),
		}]
	})
}

/// Memory's request equals its limit, so a burst never steals memory a
/// neighbour counted on.
fn container_resources(process: &Process) -> core::ResourceRequirements {
	core::ResourceRequirements {
		limits: Some(BTreeMap::from([
			(
				"cpu".to_owned(),
				cpu_millis(process.resources.cpu_limit_millis),
			),
			(
				"memory".to_owned(),
				memory_mib(process.resources.memory_mib),
			),
		])),
		requests: Some(BTreeMap::from([
			(
				"cpu".to_owned(),
				cpu_millis(process.resources.cpu_request_millis),
			),
			(
				"memory".to_owned(),
				memory_mib(process.resources.memory_mib),
			),
		])),
		claims: None,
	}
}

/// The read-only root gets a writable `/tmp` — unless the process mounts
/// one of its own volumes there, which then serves the same purpose.
fn container_mounts(process: &Process) -> Vec<core::VolumeMount> {
	let mut mounts = Vec::new();
	if !mounts_tmp(process) {
		mounts.push(core::VolumeMount {
			name: TMP_VOLUME.to_owned(),
			mount_path: "/tmp".to_owned(),
			..Default::default()
		});
	}
	mounts.extend(process.volumes.iter().map(|volume| {
		core::VolumeMount {
			name: volume.name.clone(),
			mount_path: volume.mount_path.clone(),
			..Default::default()
		}
	}));
	mounts
}

fn pod_volumes(process: &Process) -> Vec<core::Volume> {
	let mut volumes = Vec::new();
	if !mounts_tmp(process) {
		volumes.push(core::Volume {
			name: TMP_VOLUME.to_owned(),
			empty_dir: Some(core::EmptyDirVolumeSource {
				medium: None,
				size_limit: None,
			}),
			..Default::default()
		});
	}
	volumes.extend(process.volumes.iter().map(|volume| {
		core::Volume {
			name: volume.name.clone(),
			persistent_volume_claim: Some(core::PersistentVolumeClaimVolumeSource {
				claim_name: claim_name(process, volume),
				read_only: None,
			}),
			..Default::default()
		}
	}));
	volumes
}

fn mounts_tmp(process: &Process) -> bool {
	process
		.volumes
		.iter()
		.any(|volume| volume.mount_path == "/tmp")
}

fn service_objects(
	spec: &ReleaseSpec,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Vec<Object> {
	spec.processes
		.iter()
		.map(|process| {
			Object::Service(core::Service {
				metadata: object_meta(Some(namespace), &process.name, labels),
				spec: Some(core::ServiceSpec {
					type_: Some("ClusterIP".to_owned()),
					selector: Some(BTreeMap::from([(
						NAME_LABEL.to_owned(),
						process.name.clone(),
					)])),
					ports: Some(vec![core::ServicePort {
						name: Some("http".to_owned()),
						port: 80,
						target_port: Some(IntOrString::Int(i32::from(process.port))),
						protocol: Some("TCP".to_owned()),
						..Default::default()
					}]),
					..Default::default()
				}),
				status: None,
			})
		})
		.collect()
}

/// Who owns each process's replicas, and what stands beside the Deployment
/// because of it: KEDA's scaled object for a sleeper, the autoscaler for an
/// elastic process, and nothing for one pinned at its minimum.
fn scaling_objects(
	spec: &ReleaseSpec,
	cluster: &ClusterProfile,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Result<Vec<Object>, ManifestError> {
	let mut objects = Vec::new();
	for process in &spec.processes {
		match scaling_for(process, cluster)? {
			Scaling::Sleeping => objects.push(keda_object(spec, process, namespace, labels)),
			Scaling::Autoscaling => objects.push(hpa_object(process, namespace, labels)),
			Scaling::Fixed => {}
		}
	}
	Ok(objects)
}

fn scaling_for(process: &Process, cluster: &ClusterProfile) -> Result<Scaling, ManifestError> {
	if process.scale.sleep {
		if cluster.keda_http {
			Ok(Scaling::Sleeping)
		} else {
			Err(ManifestError::SleepWithoutScaler {
				process: process.name.clone(),
			})
		}
	} else if process.scale.max > process.scale.min {
		Ok(Scaling::Autoscaling)
	} else {
		Ok(Scaling::Fixed)
	}
}

fn hpa_object(process: &Process, namespace: &str, labels: &BTreeMap<String, String>) -> Object {
	Object::HorizontalPodAutoscaler(autoscaling::HorizontalPodAutoscaler {
		metadata: object_meta(Some(namespace), &process.name, labels),
		spec: autoscaling::HorizontalPodAutoscalerSpec {
			scale_target_ref: autoscaling::CrossVersionObjectReference {
				api_version: Some("apps/v1".to_owned()),
				kind: "Deployment".to_owned(),
				name: process.name.clone(),
			},
			min_replicas: Some(process.scale.min as i32),
			max_replicas: process.scale.max as i32,
			metrics: Some(vec![autoscaling::MetricSpec {
				type_: "Resource".to_owned(),
				resource: Some(autoscaling::ResourceMetricSource {
					name: "cpu".to_owned(),
					target: autoscaling::MetricTarget {
						type_: "Utilization".to_owned(),
						average_utilization: Some(70),
						average_value: None,
						value: None,
					},
				}),
				..Default::default()
			}]),
			behavior: None,
		},
		status: None,
	})
}

/// The scaled object wakes the process when one of its routes is asked
/// for: KEDA watches the edge, sees a request for the process's hosts and
/// prefixes, and scales from zero up.
fn keda_object(
	spec: &ReleaseSpec,
	process: &Process,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Object {
	let routes: Vec<&Route> = spec
		.routes
		.iter()
		.filter(|route| route.process == process.name)
		.collect();
	let hosts = unique_sorted(routes.iter().map(|route| route.hostname.as_str()));
	let prefixes = unique_sorted(routes.iter().map(|route| route.path_prefix.as_str()));
	let trigger = HttpTrigger {
		kind: "http",
		metadata: BTreeMap::from([
			("hosts".to_owned(), hosts.join(",")),
			("pathPrefixes".to_owned(), prefixes.join(",")),
			("service".to_owned(), process.name.clone()),
			("port".to_owned(), "80".to_owned()),
		]),
	};
	Object::HttpScaledObject(HttpScaledObject {
		api_version: "http.keda.sh/v1alpha1",
		kind: "HTTPScaledObject",
		metadata: object_meta(Some(namespace), &process.name, labels),
		spec: HttpScaledObjectSpec {
			scale_target_ref: HttpScaleTargetRef {
				name: process.name.clone(),
				api_version: "apps/v1",
				kind: "Deployment",
			},
			replicas_min: 0,
			replicas_max: process.scale.max,
			triggers: vec![trigger],
		},
	})
}

/// A disruption budget for every process that always runs at least two:
/// one replica may go down for node work while the rest keep serving.
fn budget_objects(
	spec: &ReleaseSpec,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Vec<Object> {
	spec.processes
		.iter()
		.filter(|process| process.scale.min >= 2)
		.map(|process| {
			Object::PodDisruptionBudget(policy::PodDisruptionBudget {
				metadata: object_meta(Some(namespace), &process.name, labels),
				spec: Some(policy::PodDisruptionBudgetSpec {
					min_available: Some(IntOrString::Int(1)),
					selector: Some(meta::LabelSelector {
						match_labels: Some(BTreeMap::from([(
							NAME_LABEL.to_owned(),
							process.name.clone(),
						)])),
						match_expressions: None,
					}),
					max_unavailable: None,
					unhealthy_pod_eviction_policy: None,
				}),
				status: None,
			})
		})
		.collect()
}

/// One Ingress per hostname, its paths longest prefix first, so `/api`
/// answers before `/` and a service's prefix never falls through to the
/// app.
fn ingress_objects(
	spec: &ReleaseSpec,
	cluster: &ClusterProfile,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Result<Vec<Object>, ManifestError> {
	let mut by_host: BTreeMap<&str, Vec<&Route>> = BTreeMap::new();
	for route in &spec.routes {
		by_host
			.entry(route.hostname.as_str())
			.or_default()
			.push(route);
	}
	by_host
		.into_iter()
		.map(|(hostname, routes)| {
			ingress(hostname, routes, cluster, namespace, labels).map(Object::Ingress)
		})
		.collect()
}

fn ingress(
	hostname: &str,
	mut routes: Vec<&Route>,
	cluster: &ClusterProfile,
	namespace: &str,
	labels: &BTreeMap<String, String>,
) -> Result<networking::Ingress, ManifestError> {
	let (tls, annotations) = host_tls(hostname, &routes, cluster)?;
	for route in &routes {
		if route.strip_cookies {
			return Err(ManifestError::StripCookies {
				hostname: hostname.to_owned(),
				prefix: route.path_prefix.clone(),
				process: route.process.clone(),
			});
		}
	}
	routes.sort_by_key(|route| Reverse(route.path_prefix.len()));
	let paths = routes
		.iter()
		.map(|route| {
			networking::HTTPIngressPath {
				path: Some(route.path_prefix.clone()),
				path_type: "Prefix".to_owned(),
				backend: networking::IngressBackend {
					service: Some(networking::IngressServiceBackend {
						name: route.process.clone(),
						port: Some(networking::ServiceBackendPort {
							number: Some(80),
							name: None,
						}),
					}),
					resource: None,
				},
			}
		})
		.collect();
	Ok(networking::Ingress {
		metadata: meta::ObjectMeta {
			// A host name is already a valid object name: dots and all.
			name: Some(hostname.to_owned()),
			namespace: Some(namespace.to_owned()),
			labels: Some(labels.clone()),
			annotations,
			..Default::default()
		},
		spec: Some(networking::IngressSpec {
			ingress_class_name: Some(cluster.ingress_class.clone()),
			tls: tls.map(|entry| vec![entry]),
			rules: Some(vec![networking::IngressRule {
				host: Some(hostname.to_owned()),
				http: Some(networking::HTTPIngressRuleValue { paths }),
			}]),
			default_backend: None,
		}),
		status: None,
	})
}

/// The certificate one host is served with and the annotation that asks an
/// issuer for it.
type HostTls = (
	Option<networking::IngressTLS>,
	Option<BTreeMap<String, String>>,
);

/// The certificate one host is served with: the cluster's wildcard, or one
/// cert-manager issues, with the annotation that asks it to.
fn host_tls(
	hostname: &str,
	routes: &[&Route],
	cluster: &ClusterProfile,
) -> Result<HostTls, ManifestError> {
	let tls = routes[0].tls;
	if routes.iter().any(|route| route.tls != tls) {
		return Err(ManifestError::MixedTls {
			hostname: hostname.to_owned(),
		});
	}
	match tls {
		Tls::Wildcard => {
			let secret = cluster.wildcard_tls_secret.as_ref().ok_or_else(|| {
				ManifestError::NoWildcardSecret {
					hostname: hostname.to_owned(),
				}
			})?;
			Ok((
				Some(networking::IngressTLS {
					hosts: Some(vec![hostname.to_owned()]),
					secret_name: Some(secret.clone()),
				}),
				None,
			))
		}
		Tls::Issued => {
			let issuer = cluster.cluster_issuer.as_ref().ok_or_else(|| {
				ManifestError::NoIssuer {
					hostname: hostname.to_owned(),
				}
			})?;
			Ok((
				Some(networking::IngressTLS {
					hosts: Some(vec![hostname.to_owned()]),
					secret_name: Some(format!("{hostname}-tls")),
				}),
				Some(BTreeMap::from([(
					"cert-manager.io/cluster-issuer".to_owned(),
					issuer.clone(),
				)])),
			))
		}
	}
}

fn unique_sorted<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
	let mut seen: Vec<&str> = values.collect();
	seen.sort_unstable();
	seen.dedup();
	seen.into_iter().map(str::to_owned).collect()
}

fn secret_name(process: &Process) -> String {
	format!("{}-secrets", process.name)
}

fn claim_name(process: &Process, volume: &Volume) -> String {
	format!("{}-{}", process.name, volume.name)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_secret_value_prints_nothing_but_redacted() {
		let values = SecretValues::new(BTreeMap::from([
			("search-token".to_owned(), "hunter2-the-value".to_owned()),
			(
				"db-url".to_owned(),
				"postgres://test:test@db/test".to_owned(),
			),
		]));
		for printed in [format!("{values:?}"), format!("{values}")] {
			assert_eq!(printed, "[redacted]");
			assert!(!printed.contains("hunter2"), "{printed}");
			assert!(!printed.contains("postgres"), "{printed}");
			assert!(!printed.contains("search-token"), "{printed}");
		}
		let empty = SecretValues::default();
		assert_eq!(format!("{empty:?}"), "[redacted]");
	}

	#[test]
	fn a_lookup_names_the_key_it_needed() {
		let values = SecretValues::new(BTreeMap::from([("a".to_owned(), "one".to_owned())]));
		assert_eq!(values.get("a"), Some("one"));
		assert_eq!(values.get("b"), None);
	}
}
