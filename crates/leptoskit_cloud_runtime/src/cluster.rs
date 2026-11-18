//! Server-side application of tenant releases. Credentials belong to the
//! worker, never to a release. API error bodies are deliberately never
//! displayed.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::apps::v1::ReplicaSet;
use k8s_openapi::api::core::v1::Namespace;
use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use kube::Config;
use kube::ResourceExt;
use kube::api::Api;
use kube::api::DeleteParams;
use kube::api::ListParams;
use kube::api::LogParams;
use kube::api::Patch;
use kube::api::PatchParams;
use kube::core::ApiResource;
use kube::core::DynamicObject;
use kube::core::GroupVersionKind;
use tokio::time::Instant;

use crate::Applied;
use crate::ClusterProfile;
use crate::EnvironmentRef;
use crate::LogLine;
use crate::LogQuery;
use crate::Observed;
use crate::ObservedProcess;
use crate::ProcessState;
use crate::ReleaseSpec;
use crate::Runtime;
use crate::RuntimeError;
use crate::RuntimeFuture;
use crate::SecretValues;
use crate::manifests;

const MANAGED: &str = "app.kubernetes.io/managed-by";
const OWNER: &str = "leptoskit-cloud";
const SELECTOR: &str = "app.kubernetes.io/managed-by=leptoskit-cloud";
/// Field ownership is stable across worker restarts and releases.
pub const FIELD_MANAGER: &str = "leptoskit-cloud-runtime";
/// Hard cap on the sum of returned log message and pod-name bytes.
pub const LOG_BYTES: usize = 256 * 1024;

/// One authenticated cluster connection and its operator-owned profile.
/// Secret leases are supplied separately from the immutable release spec.
#[derive(Clone)]
pub struct Cluster {
	client: Client,
	profile: ClusterProfile,
	secrets: SecretValues,
	deadline: Duration,
}

impl Cluster {
	/// Use an already constructed client, also useful for a loopback fake API.
	/// # Errors
	/// Refuses an invalid profile or a zero operation deadline.
	pub fn new(
		client: Client,
		profile: ClusterProfile,
		secrets: SecretValues,
		deadline: Duration,
	) -> Result<Self, RuntimeError> {
		profile
			.validate()
			.map_err(|error| RuntimeError::Other(error.to_string()))?;
		if deadline.is_zero() {
			return Err(RuntimeError::Other(
				"operation deadline must be positive".to_owned(),
			));
		}
		Ok(Self {
			client,
			profile,
			secrets,
			deadline,
		})
	}

	/// Use the worker's mounted `ServiceAccount`; never falls back to a
	/// developer config. # Errors
	/// Returns a redacted configuration or connection error.
	pub fn in_cluster(
		profile: ClusterProfile,
		secrets: SecretValues,
		deadline: Duration,
	) -> Result<Self, RuntimeError> {
		let mut config = Config::incluster()
			.map_err(|_| RuntimeError::Other("in-cluster configuration unavailable".to_owned()))?;
		configure_deadline(&mut config, deadline);
		Self::new(
			Client::try_from(config).map_err(api_error)?,
			profile,
			secrets,
			deadline,
		)
	}

	/// Read only the explicitly named kubeconfig for development.
	/// # Errors
	/// Returns a redacted configuration error; credentials and paths are not
	/// displayed.
	pub async fn from_kubeconfig(
		path: &Path,
		profile: ClusterProfile,
		secrets: SecretValues,
		deadline: Duration,
	) -> Result<Self, RuntimeError> {
		let kubeconfig = kube::config::Kubeconfig::read_from(path)
			.map_err(|_| RuntimeError::Other("cannot read kubeconfig".to_owned()))?;
		let mut config =
			Config::from_custom_kubeconfig(kubeconfig, &kube::config::KubeConfigOptions::default())
				.await
				.map_err(|_| RuntimeError::Other("invalid kubeconfig".to_owned()))?;
		configure_deadline(&mut config, deadline);
		Self::new(
			Client::try_from(config).map_err(api_error)?,
			profile,
			secrets,
			deadline,
		)
	}

	fn api(&self, resource: Resource, namespace: &str) -> Api<DynamicObject> {
		Api::namespaced_with(self.client.clone(), namespace, &resource.api_resource())
	}

	fn redact(&self, text: &str) -> String {
		self.secrets.redact(text)
	}

	async fn apply_release(&self, spec: &ReleaseSpec) -> Result<Applied, RuntimeError> {
		spec.validate()?;
		let objects = manifests(spec, &self.profile, &self.secrets)
			.map_err(|error| RuntimeError::Other(error.to_string()))?;
		let namespace = spec.environment.namespace();
		let mut desired = BTreeSet::new();
		for object in &objects {
			let value = serde_json::to_value(object)
				.map_err(|_| RuntimeError::Other("cannot serialize manifest".to_owned()))?;
			let name = object
				.name()
				.ok_or_else(|| RuntimeError::Other("manifest has no name".to_owned()))?;
			if object.kind() == "Namespace" {
				Api::<Namespace>::all(self.client.clone())
					.patch(
						name,
						&PatchParams::apply(FIELD_MANAGER),
						&Patch::Apply(&value),
					)
					.await
					.map_err(api_error)?;
			} else {
				let resource = RESOURCES
					.iter()
					.copied()
					.find(|resource| resource.kind == object.kind())
					.ok_or_else(|| RuntimeError::Other("unsupported manifest kind".to_owned()))?;
				self.api(resource, &namespace)
					.patch(
						name,
						&PatchParams::apply(FIELD_MANAGER),
						&Patch::Apply(&value),
					)
					.await
					.map_err(api_error)?;
				desired.insert((resource.kind.to_owned(), name.to_owned()));
			}
		}
		for resource in RESOURCES {
			if resource.kind == "PersistentVolumeClaim" {
				continue;
			}
			if self.profile.keda_http.is_none()
				&& matches!(resource.group, "http.keda.sh" | "keda.sh")
			{
				continue;
			}
			let existing = self
				.api(*resource, &namespace)
				.list(&ListParams::default().labels(SELECTOR))
				.await
				.map_err(api_error)?;
			for object in prune_set(&existing.items, &desired) {
				deleted(
					self.api(*resource, &namespace)
						.delete(&object, &DeleteParams::default())
						.await,
				)?;
			}
		}
		let until = Instant::now() + self.deadline;
		let mut last_pending = (
			spec.processes
				.first()
				.map_or_else(String::new, |process| process.name.clone()),
			"new ReplicaSet has not become available".to_owned(),
		);
		loop {
			let observed =
				match tokio::time::timeout_at(until, self.observe(&spec.environment)).await {
					Ok(result) => result?,
					Err(_) => {
						return Err(RuntimeError::NotReady {
							process: last_pending.0,
							reason: last_pending.1,
						});
					}
				};
			let pending = spec.processes.iter().find(|process| {
				!observed.processes.iter().any(|actual| {
					actual.name == process.name
						&& matches!(actual.state, ProcessState::Ready | ProcessState::Sleeping)
				})
			});
			let Some(process) = pending else {
				break;
			};
			last_pending = (
				process.name.clone(),
				observed
					.processes
					.iter()
					.find(|actual| actual.name == process.name)
					.and_then(|actual| {
						match &actual.state {
							ProcessState::Failed { reason } => Some(reason.clone()),
							_ => None,
						}
					})
					.unwrap_or_else(|| "new ReplicaSet has not become available".to_owned()),
			);
			if Instant::now() >= until {
				let reason = observed
					.processes
					.iter()
					.find(|actual| actual.name == process.name)
					.and_then(|actual| {
						match &actual.state {
							ProcessState::Failed { reason } => Some(reason.clone()),
							_ => None,
						}
					})
					.unwrap_or_else(|| "new ReplicaSet has not become available".to_owned());
				return Err(RuntimeError::NotReady {
					process: process.name.clone(),
					reason,
				});
			}
			tokio::time::sleep(
				Duration::from_millis(250).min(until.saturating_duration_since(Instant::now())),
			)
			.await;
		}
		Ok(Applied {
			environment: spec.environment.clone(),
			deployment: spec.deployment,
			processes: spec
				.processes
				.iter()
				.map(|process| process.name.clone())
				.collect(),
		})
	}

	async fn observe(&self, environment: &EnvironmentRef) -> Result<Observed, RuntimeError> {
		let namespace = environment.namespace();
		let params = ListParams::default().labels(SELECTOR);
		let deployments = Api::<Deployment>::namespaced(self.client.clone(), &namespace)
			.list(&params)
			.await
			.map_err(api_error)?;
		let sets = Api::<ReplicaSet>::namespaced(self.client.clone(), &namespace)
			.list(&params)
			.await
			.map_err(api_error)?;
		let pods = Api::<Pod>::namespaced(self.client.clone(), &namespace)
			.list(&params)
			.await
			.map_err(api_error)?;
		let processes = deployments
			.items
			.iter()
			.map(|deployment| {
				let mut observed = observe_process(deployment, &sets.items, &pods.items);
				if let ProcessState::Failed { reason } = &mut observed.state {
					*reason = self.redact(reason);
				}
				observed
			})
			.collect();
		Ok(Observed {
			environment: environment.clone(),
			processes,
		})
	}

	async fn read_logs(
		&self,
		environment: &EnvironmentRef,
		query: &LogQuery,
	) -> Result<Vec<LogLine>, RuntimeError> {
		let pods = Api::<Pod>::namespaced(self.client.clone(), &environment.namespace());
		let mut lines = Vec::new();
		let limit = query.limit.unwrap_or(100).min(10_000);
		if limit == 0 {
			return Ok(lines);
		}
		for pod in pods
			.list(&ListParams::default().labels(SELECTOR))
			.await
			.map_err(api_error)?
			.items
		{
			let process = pod
				.labels()
				.get("app.kubernetes.io/name")
				.cloned()
				.unwrap_or_default();
			if query.process.as_ref().is_some_and(|name| name != &process) {
				continue;
			}
			let name = pod.name_any();
			let text = pods
				.logs(
					&name,
					&LogParams {
						timestamps: true,
						tail_lines: Some(i64::from(limit)),
						limit_bytes: Some(LOG_BYTES as i64),
						since_seconds: query
							.since_seconds
							.map(|seconds| i64::try_from(seconds).unwrap_or(i64::MAX)),
						..Default::default()
					},
				)
				.await
				.map_err(api_error)?;
			for line in text.lines() {
				let Some((time, message)) = line.split_once(' ') else {
					continue;
				};
				let time = chrono::DateTime::parse_from_rfc3339(time)
					.map_err(|_| RuntimeError::Other("pod log has invalid timestamp".to_owned()))?;
				lines.push(LogLine {
					pod: Some(name.clone()),
					process: process.clone(),
					at_millis: time.timestamp_millis(),
					message: self.redact(message),
				});
			}
		}
		Ok(bound_logs(lines, limit as usize, LOG_BYTES))
	}

	async fn destroy_environment(&self, environment: &EnvironmentRef) -> Result<(), RuntimeError> {
		let namespaces = Api::<Namespace>::all(self.client.clone());
		let name = environment.namespace();
		if namespaces
			.get_opt(&name)
			.await
			.map_err(api_error)?
			.is_none()
		{
			return Ok(());
		}
		deleted(namespaces.delete(&name, &DeleteParams::default()).await)?;
		let until = Instant::now() + self.deadline;
		loop {
			if namespaces
				.get_opt(&name)
				.await
				.map_err(api_error)?
				.is_none()
			{
				return Ok(());
			}
			if Instant::now() >= until {
				return Err(RuntimeError::Other(
					"namespace deletion deadline exceeded".to_owned(),
				));
			}
			tokio::time::sleep(
				Duration::from_millis(250).min(until.saturating_duration_since(Instant::now())),
			)
			.await;
		}
	}
}

impl Runtime for Cluster {
	fn apply(&self, spec: &ReleaseSpec) -> RuntimeFuture<'_, Result<Applied, RuntimeError>> {
		let spec = spec.clone();
		Box::pin(async move { self.apply_release(&spec).await })
	}

	fn status(
		&self,
		environment: &EnvironmentRef,
	) -> RuntimeFuture<'_, Result<Observed, RuntimeError>> {
		let environment = environment.clone();
		Box::pin(async move { self.observe(&environment).await })
	}

	fn logs(
		&self,
		environment: &EnvironmentRef,
		query: &LogQuery,
	) -> RuntimeFuture<'_, Result<Vec<LogLine>, RuntimeError>> {
		let environment = environment.clone();
		let query = query.clone();
		Box::pin(async move { self.read_logs(&environment, &query).await })
	}

	fn destroy(&self, environment: &EnvironmentRef) -> RuntimeFuture<'_, Result<(), RuntimeError>> {
		let environment = environment.clone();
		Box::pin(async move {
			tokio::time::timeout(self.deadline, self.destroy_environment(&environment))
				.await
				.map_err(|_| {
					RuntimeError::Other("namespace deletion deadline exceeded".to_owned())
				})?
		})
	}
}

fn configure_deadline(config: &mut Config, deadline: Duration) {
	config.default_retry = false;
	config.connect_timeout = Some(deadline);
	config.read_timeout = Some(deadline);
	config.write_timeout = Some(deadline);
}

fn deleted<T>(result: Result<T, kube::Error>) -> Result<(), RuntimeError> {
	match result {
		Ok(_) => Ok(()),
		Err(kube::Error::Api(response)) if response.code == 404 => Ok(()),
		Err(error) => Err(api_error(error)),
	}
}

fn api_error(error: kube::Error) -> RuntimeError {
	match error {
		kube::Error::Api(response) => {
			match response.code {
				401 | 403 => RuntimeError::Forbidden,
				409 => RuntimeError::Conflict,
				500..=599 => RuntimeError::Unavailable,
				code => RuntimeError::Other(format!("Kubernetes API returned HTTP {code}")),
			}
		}
		kube::Error::HyperError(_)
		| kube::Error::Service(_)
		| kube::Error::ReadEvents(_)
		| kube::Error::RustlsTls(_) => RuntimeError::Unavailable,
		_ => {
			RuntimeError::Other(
				"Kubernetes client could not process the request or response".to_owned(),
			)
		}
	}
}

#[derive(Clone, Copy)]
struct Resource {
	group: &'static str,
	version: &'static str,
	kind: &'static str,
	plural: &'static str,
}
impl Resource {
	fn api_resource(self) -> ApiResource {
		ApiResource::from_gvk_with_plural(
			&GroupVersionKind::gvk(self.group, self.version, self.kind),
			self.plural,
		)
	}
}

const RESOURCES: &[Resource] = &[
	Resource {
		group: "",
		version: "v1",
		kind: "ResourceQuota",
		plural: "resourcequotas",
	},
	Resource {
		group: "",
		version: "v1",
		kind: "LimitRange",
		plural: "limitranges",
	},
	Resource {
		group: "networking.k8s.io",
		version: "v1",
		kind: "NetworkPolicy",
		plural: "networkpolicies",
	},
	Resource {
		group: "",
		version: "v1",
		kind: "ServiceAccount",
		plural: "serviceaccounts",
	},
	Resource {
		group: "",
		version: "v1",
		kind: "Secret",
		plural: "secrets",
	},
	Resource {
		group: "",
		version: "v1",
		kind: "PersistentVolumeClaim",
		plural: "persistentvolumeclaims",
	},
	Resource {
		group: "apps",
		version: "v1",
		kind: "Deployment",
		plural: "deployments",
	},
	Resource {
		group: "",
		version: "v1",
		kind: "Service",
		plural: "services",
	},
	Resource {
		group: "autoscaling",
		version: "v2",
		kind: "HorizontalPodAutoscaler",
		plural: "horizontalpodautoscalers",
	},
	Resource {
		group: "http.keda.sh",
		version: "v1beta1",
		kind: "InterceptorRoute",
		plural: "interceptorroutes",
	},
	Resource {
		group: "keda.sh",
		version: "v1alpha1",
		kind: "ScaledObject",
		plural: "scaledobjects",
	},
	Resource {
		group: "policy",
		version: "v1",
		kind: "PodDisruptionBudget",
		plural: "poddisruptionbudgets",
	},
	Resource {
		group: "networking.k8s.io",
		version: "v1",
		kind: "Ingress",
		plural: "ingresses",
	},
];

fn prune_set(existing: &[DynamicObject], desired: &BTreeSet<(String, String)>) -> Vec<String> {
	existing
		.iter()
		.filter(|object| {
			object
				.labels()
				.get(MANAGED)
				.is_some_and(|value| value == OWNER)
				&& object.owner_references().is_empty()
		})
		.filter_map(|object| {
			let kind = object.types.as_ref()?.kind.clone();
			let name = object.name_any();
			(kind != "PersistentVolumeClaim" && !desired.contains(&(kind, name.clone())))
				.then_some(name)
		})
		.collect()
}

fn observe_process(deployment: &Deployment, sets: &[ReplicaSet], pods: &[Pod]) -> ObservedProcess {
	let desired = deployment
		.spec
		.as_ref()
		.and_then(|spec| spec.replicas)
		.unwrap_or(1)
		.max(0) as u32;
	let revision = deployment
		.annotations()
		.get("deployment.kubernetes.io/revision");
	let set = sets.iter().find(|set| {
		set.owner_references()
			.iter()
			.any(|owner| Some(&owner.uid) == deployment.metadata.uid.as_ref())
			&& set.annotations().get("deployment.kubernetes.io/revision") == revision
			&& set
				.spec
				.as_ref()
				.and_then(|spec| spec.template.as_ref())
				.is_some_and(|template| {
					let mut template = template.clone();
					if let Some(labels) = template
						.metadata
						.as_mut()
						.and_then(|metadata| metadata.labels.as_mut())
					{
						labels.remove("pod-template-hash");
					}
					deployment
						.spec
						.as_ref()
						.is_some_and(|spec| spec.template == template)
				})
	});
	let current: Vec<&Pod> = pods
		.iter()
		.filter(|pod| {
			set.is_some_and(|set| {
				pod.owner_references()
					.iter()
					.any(|owner| Some(&owner.uid) == set.metadata.uid.as_ref())
			})
		})
		.collect();
	let available = set
		.and_then(|set| set.status.as_ref())
		.and_then(|status| status.available_replicas)
		.unwrap_or(0)
		.max(0) as u32;
	let mut observed_restarts = 0;
	let mut last_oom_kill = None;
	let mut image = String::new();
	let mut failure = None;
	for pod in current {
		if let Some(status) = &pod.status {
			for condition in status.conditions.as_deref().unwrap_or_default() {
				if condition.status == "False"
					&& matches!(
						condition.type_.as_str(),
						"PodScheduled" | "Ready" | "ContainersReady"
					) {
					failure = Some(format!(
						"{}: {}",
						condition.reason.as_deref().unwrap_or(&condition.type_),
						condition
							.message
							.as_deref()
							.unwrap_or("readiness probe has not passed")
					));
				}
			}
			for container in status.container_statuses.as_deref().unwrap_or_default() {
				observed_restarts += u64::try_from(container.restart_count).unwrap_or(0);
				if image.is_empty() {
					image.clone_from(&container.image_id);
				}
				for state in [container.last_state.as_ref(), container.state.as_ref()]
					.into_iter()
					.flatten()
				{
					if let Some(terminated) = &state.terminated
						&& terminated.reason.as_deref() == Some("OOMKilled")
						&& let Some(time) = &terminated.finished_at
					{
						last_oom_kill = last_oom_kill.max(Some(time.0.as_millisecond()));
					}
				}
				if let Some(waiting) = container
					.state
					.as_ref()
					.and_then(|state| state.waiting.as_ref())
				{
					let reason = waiting.reason.as_deref().unwrap_or("Waiting");
					let exit = container
						.last_state
						.as_ref()
						.and_then(|state| state.terminated.as_ref())
						.map(|termination| format!("; last exit code {}", termination.exit_code))
						.unwrap_or_default();
					failure = Some(format!(
						"{reason}: {}{exit}",
						waiting
							.message
							.as_deref()
							.unwrap_or("container has not started")
					));
				}
			}
		}
	}
	let observed_generation = deployment
		.status
		.as_ref()
		.and_then(|status| status.observed_generation)
		.unwrap_or(0);
	let current_generation = deployment.metadata.generation.unwrap_or(1);
	let state = if desired == 0 {
		ProcessState::Sleeping
	} else if available >= desired && observed_generation >= current_generation {
		ProcessState::Ready
	} else if let Some(reason) = failure {
		ProcessState::Failed { reason }
	} else {
		ProcessState::Pending
	};
	ObservedProcess {
		name: deployment.name_any(),
		ready_replicas: available,
		desired_replicas: desired,
		image,
		observed_restarts,
		last_oom_kill,
		state,
	}
}

fn bound_logs(mut lines: Vec<LogLine>, limit: usize, bytes: usize) -> Vec<LogLine> {
	lines.sort_by(|left, right| {
		right
			.at_millis
			.cmp(&left.at_millis)
			.then_with(|| left.pod.cmp(&right.pod))
	});
	let mut remaining = bytes.saturating_sub(2);
	lines
		.into_iter()
		.take(limit)
		.take_while(|line| {
			// These concrete strings/integers cannot fail serialization. Count
			// escaped JSON bytes and a comma as well as payload bytes.
			let cost =
				serde_json::to_vec(line).map_or(usize::MAX, |json| json.len().saturating_add(1));
			if cost > remaining {
				false
			} else {
				remaining -= cost;
				true
			}
		})
		.collect()
}

#[cfg(test)]
mod tests;
