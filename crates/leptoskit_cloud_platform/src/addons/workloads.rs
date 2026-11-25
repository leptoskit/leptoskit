//! In-cluster control-plane (`50-control-plane`) and worker (`60-workers`)
//! Kubernetes manifests.

use serde_json::Value;
use serde_json::json;

use super::Documents;
use super::credential_keys;
use super::object;
use super::placement;
use super::runtime_profile;
use crate::PlatformError;
use crate::config::Cluster;
use crate::config::ControlPlaneConfig;
use crate::config::PlatformConfig;
use crate::config::Pool;
use crate::config::Registry;
use crate::config::WorkersConfig;

/// Namespace running the `LeptosKit` Cloud control plane and its dedicated
/// Postgres database.
pub const CONTROL_NAMESPACE: &str = "leptoskit-cloud-control";

/// Namespace running a cluster's notifications hub, when
/// `workers.notifications_image` is set.
pub const NOTIFICATIONS_NAMESPACE: &str = "leptoskit-notifications";

/// The name the hub's Service answers on, as the cluster's `notifications`
/// facts and its worker's `--notifications-host` must give it.
pub const NOTIFICATIONS_HOST: &str = "hub.leptoskit-notifications.svc.cluster.local";

/// The port the hub serves on, and its Service's port: the edge's bridge,
/// the apps' calls and the hub's own `NetworkPolicy` all use this one.
pub const NOTIFICATIONS_PORT: u16 = 8080;

/// Control-plane documents (`50-control-plane`) and their secret-inventory
/// entries. Emitted only on the edge cluster when `[control_plane]` is
/// configured.
#[expect(
	clippy::too_many_lines,
	reason = "declarative Kubernetes YAML manifests for the control plane"
)]
pub(super) fn control_plane_documents(
	config: &PlatformConfig,
	cluster: &Cluster,
	platform: &Value,
	cp: &ControlPlaneConfig,
	region: &str,
	cluster_name: &str,
) -> Result<(Documents, Vec<Value>), PlatformError> {
	let mut documents = Vec::new();
	let mut required = Vec::new();

	let (hot_secret, hot_access, hot_key) =
		credential_keys(&config.object_storage.credentials, false);
	let (backups_endpoint, _backups_region, backups_cred_ref) =
		config.object_storage.backups_store();
	let uses_cold = config.object_storage.cold.is_some();
	let (db_secret, db_access, db_key) = credential_keys(backups_cred_ref, uses_cold);

	let registry_url = match &config.registry {
		Registry::Zot { .. } => "zot.registry.svc.cluster.local:5000/leptoskit".to_owned(),
		Registry::External { host, .. } => format!("{host}/leptoskit"),
	};

	let control_host = url::Url::parse(&config.control_origin)
		.ok()
		.and_then(|u| u.host_str().map(str::to_owned))
		.ok_or_else(|| PlatformError::Invalid("control_origin must contain a valid host".into()))?;

	// Dedicated CNPG cluster for control-plane metadata.
	let mut affinity = platform.clone();
	affinity["podAntiAffinityType"] = json!("preferred");
	documents.push((
		"50-control-plane",
		object(
			"postgresql.cnpg.io/v1",
			"Cluster",
			"control",
			Some(CONTROL_NAMESPACE),
			json!({
				"spec": {
					"instances": cp.db_instances,
					"postgresql": {
						"pg_hba": ["hostssl all all 0.0.0.0/0 scram-sha-256"]
					},
					"storage": {
						"size": cp.db_storage_size,
						"storageClass": config.postgres.storage_class
					},
					"affinity": affinity,
					"backup": {
						"retentionPolicy": format!("{}d", config.postgres.retention_days),
						"barmanObjectStore": {
							"destinationPath": format!("s3://{}/{region}/{cluster_name}/control", config.postgres.wal_bucket),
							"endpointURL": backups_endpoint,
							"s3Credentials": {
								"accessKeyId": {"name": db_secret, "key": db_access},
								"secretAccessKey": {"name": db_secret, "key": db_key}
							},
							"wal": {"compression": "gzip"}
						}
					}
				}
			}),
		),
	));

	let mut env = vec![
		json!({"name": "LEPTOSKIT_CLOUD_ORIGIN", "value": config.control_origin}),
		json!({"name": "LEPTOSKIT_CLOUD_APPS_DOMAIN", "value": config.apps_domain}),
		json!({"name": "LEPTOSKIT_CLOUD_DEFAULT_REGION", "value": config.default_region}),
		json!({"name": "LEPTOSKIT_CLOUD_REGISTRY", "value": registry_url}),
		json!({"name": "LEPTOSKIT_CLOUD_BIND", "value": "0.0.0.0:8080"}),
		json!({"name": "LEPTOSKIT_CLOUD_SITE", "value": "/srv/site"}),
		json!({"name": "LEPTOSKIT_CLOUD_CLIENT_IP_HEADER", "value": cp.client_ip_header}),
		json!({"name": "LEPTOSKIT_CLOUD_S3_ENDPOINT", "value": config.object_storage.endpoint}),
		json!({"name": "LEPTOSKIT_CLOUD_S3_REGION", "value": config.object_storage.region}),
		json!({"name": "LEPTOSKIT_CLOUD_S3_SOURCES_BUCKET", "value": config.object_storage.sources}),
		json!({"name": "LEPTOSKIT_CLOUD_S3_LOGS_BUCKET", "value": config.object_storage.build_logs}),
		json!({"name": "LEPTOSKIT_CLOUD_S3_CHECKSUM_HEADERS", "value": cp.s3_checksum_headers.to_string()}),
		json!({"name": "LEPTOSKIT_CLOUD_S3_ACCESS_KEY_ID", "valueFrom": {"secretKeyRef": {"name": hot_secret, "key": hot_access}}}),
		json!({"name": "LEPTOSKIT_CLOUD_S3_SECRET_ACCESS_KEY", "valueFrom": {"secretKeyRef": {"name": hot_secret, "key": hot_key}}}),
		json!({"name": "LEPTOSKIT_CLOUD_DATABASE_URL", "valueFrom": {"secretKeyRef": {"name": "control-app", "key": "uri"}}}),
		json!({"name": "LEPTOSKIT_CLOUD_AUTH_KEYS", "valueFrom": {"secretKeyRef": {"name": cp.secret_name, "key": "auth-keys"}}}),
		json!({"name": "LEPTOSKIT_CLOUD_VAULT_KEYS", "valueFrom": {"secretKeyRef": {"name": cp.secret_name, "key": "vault-keys"}}}),
	];
	if let Some(email) = &cp.bootstrap_operator_email {
		env.push(json!({"name": "LEPTOSKIT_CLOUD_BOOTSTRAP_OPERATOR_EMAIL", "value": email}));
	}

	let security_context = json!({
		"allowPrivilegeEscalation": false,
		"readOnlyRootFilesystem": true,
		"capabilities": {"drop": ["ALL"]}
	});
	let pod_security = json!({
		"runAsNonRoot": true,
		"runAsUser": 65532,
		"runAsGroup": 65532,
		"seccompProfile": {"type": "RuntimeDefault"}
	});

	// One-shot migration Job (idempotent; also bootstraps the initial operator
	// account when `bootstrap_operator_email` is set).
	let mut migrate_pod = platform.clone();
	migrate_pod["automountServiceAccountToken"] = json!(false);
	migrate_pod["restartPolicy"] = json!("OnFailure");
	migrate_pod["securityContext"] = pod_security.clone();
	migrate_pod["containers"] = json!([{
		"name": "migrate",
		"image": cp.image,
		"args": ["migrate"],
		"env": env,
		"resources": {
			"requests": {"cpu": "100m", "memory": "128Mi"},
			"limits": {"cpu": "500m", "memory": "512Mi"}
		},
		"securityContext": security_context
	}]);
	documents.push((
		"50-control-plane",
		object(
			"batch/v1",
			"Job",
			"control-migrate",
			Some(CONTROL_NAMESPACE),
			json!({
				"spec": {
					"backoffLimit": 5,
					"template": {
						"metadata": {"labels": {"app": "control-migrate"}},
						"spec": migrate_pod
					}
				}
			}),
		),
	));

	// Serving Deployment. Every replica runs the sweeper: a Deployment gives
	// its environment to all of them, and the sweeper's advisory lock lets
	// only one sweep at a time, whatever `replicas` is.
	let mut serve_env = env;
	serve_env.push(json!({"name": "LEPTOSKIT_CLOUD_SWEEPER", "value": "true"}));
	let mut serve_pod = platform.clone();
	serve_pod["automountServiceAccountToken"] = json!(false);
	serve_pod["securityContext"] = pod_security;
	serve_pod["containers"] = json!([{
		"name": "control",
		"image": cp.image,
		"args": ["serve"],
		"ports": [{"name": "http", "containerPort": 8080}],
		"env": serve_env,
		"readinessProbe": {"httpGet": {"path": "/readyz", "port": 8080}, "initialDelaySeconds": 5, "periodSeconds": 10},
		"livenessProbe": {"httpGet": {"path": "/healthz", "port": 8080}, "initialDelaySeconds": 15, "periodSeconds": 20},
		"resources": {
			"requests": {"cpu": "100m", "memory": "256Mi"},
			"limits": {"cpu": "1000m", "memory": "1Gi"}
		},
		"securityContext": security_context
	}]);
	documents.push((
		"50-control-plane",
		object(
			"apps/v1",
			"Deployment",
			"control",
			Some(CONTROL_NAMESPACE),
			json!({
				"spec": {
					"replicas": cp.replicas,
					"selector": {"matchLabels": {"app": "leptoskit-cloud-control"}},
					"template": {
						"metadata": {"labels": {"app": "leptoskit-cloud-control"}},
						"spec": serve_pod
					}
				}
			}),
		),
	));

	documents.push((
		"50-control-plane",
		object(
			"v1",
			"Service",
			"control",
			Some(CONTROL_NAMESPACE),
			json!({
				"spec": {
					"selector": {"app": "leptoskit-cloud-control"},
					"ports": [{"name": "http", "port": 80, "targetPort": 8080}]
				}
			}),
		),
	));

	documents.push((
		"50-control-plane",
		object(
			"networking.k8s.io/v1",
			"Ingress",
			"control",
			Some(CONTROL_NAMESPACE),
			json!({
				"spec": {
					"ingressClassName": "traefik",
					"rules": [{
						"host": control_host,
						"http": {
							"paths": [{
								"path": "/",
								"pathType": "Prefix",
								"backend": {
									"service": {"name": "control", "port": {"number": 80}}
								}
							}]
						}
					}]
				}
			}),
		),
	));

	// Secret inventory entries (must match every secret referenced above).
	required.push(json!({
		"namespace": CONTROL_NAMESPACE,
		"name": cp.secret_name,
		"keys": ["auth-keys", "vault-keys"],
		"purpose": "Control plane session signing and vault master keys",
		"source": "operator generated random 32-byte base64url keys",
		"createdBy": "operator"
	}));
	required.push(json!({
		"namespace": CONTROL_NAMESPACE,
		"name": "control-app",
		"keys": ["uri"],
		"purpose": "Control plane CNPG database connection URI",
		"source": "CNPG Cluster/control generates this; do not precreate",
		"createdBy": "cnpg"
	}));
	required.push(json!({
		"namespace": CONTROL_NAMESPACE,
		"name": hot_secret,
		"keys": [hot_access, hot_key],
		"purpose": "Control plane source archive and log S3 storage",
		"source": config.object_storage.credentials,
		"createdBy": "operator"
	}));
	if db_secret != hot_secret {
		required.push(json!({
			"namespace": CONTROL_NAMESPACE,
			"name": db_secret,
			"keys": [db_access, db_key],
			"purpose": "Control plane CNPG WAL and base backups",
			"source": backups_cred_ref,
			"createdBy": "operator"
		}));
	}

	let _ = cluster;
	Ok((documents, required))
}

/// Worker documents (`60-workers`) and their secret-inventory entries.
#[expect(
	clippy::too_many_lines,
	reason = "declarative Kubernetes YAML manifests for cluster and build workers"
)]
pub(super) fn worker_documents(
	config: &PlatformConfig,
	cluster: &Cluster,
	platform: &Value,
	workers: &WorkersConfig,
	worker_name: &str,
) -> Result<(Documents, Vec<Value>), PlatformError> {
	let mut documents = Vec::new();
	let mut required = Vec::new();

	let profile = runtime_profile(config, cluster);
	let profile_json = serde_json::to_string_pretty(&profile)?;

	// ServiceAccount bound by `10-worker-rbac` and constrained by the
	// `ValidatingAdmissionPolicy` to `lk-*` tenant namespaces.
	documents.push((
		"60-workers",
		object(
			"v1",
			"ServiceAccount",
			"leptoskit-cloud-worker",
			Some("kube-system"),
			json!({}),
		),
	));

	// Serialized ClusterProfile mounted into the cluster worker pod.
	documents.push((
		"60-workers",
		object(
			"v1",
			"ConfigMap",
			"cluster-profile",
			Some("kube-system"),
			json!({"data": {"profile.json": profile_json}}),
		),
	));

	// Persistent state PVC for cluster worker identity + job markers.
	documents.push((
		"60-workers",
		object(
			"v1",
			"PersistentVolumeClaim",
			"cluster-worker-state",
			Some("kube-system"),
			json!({
				"spec": {
					"accessModes": ["ReadWriteOnce"],
					"storageClassName": config.postgres.storage_class,
					"resources": {"requests": {"storage": "1Gi"}}
				}
			}),
		),
	));

	let mut cluster_pod = platform.clone();
	cluster_pod["serviceAccountName"] = json!("leptoskit-cloud-worker");
	cluster_pod["securityContext"] = json!({
		"runAsNonRoot": true,
		"runAsUser": 65532,
		"runAsGroup": 65532,
		"fsGroup": 65532,
		"seccompProfile": {"type": "RuntimeDefault"}
	});
	cluster_pod["volumes"] = json!([
		{"name": "state", "persistentVolumeClaim": {"claimName": "cluster-worker-state"}},
		{"name": "profile", "configMap": {"name": "cluster-profile"}},
		{"name": "postgres-ca", "secret": {"secretName": "cluster-worker-postgres-ca"}},
		{"name": "tmp", "emptyDir": {"sizeLimit": "128Mi"}}
	]);
	let mut cluster_args = vec![
		"run".to_owned(),
		"--state".to_owned(),
		"/var/lib/leptoskit-worker".to_owned(),
		"--profile-json".to_owned(),
		"/etc/leptoskit/profile.json".to_owned(),
		"--reconcile-on-start".to_owned(),
		"--runtime-class".to_owned(),
		config.isolation.runtime_class.clone(),
		"--postgres-namespace".to_owned(),
		"database".to_owned(),
		"--postgres-ca".to_owned(),
		"/etc/leptoskit/postgres-ca/ca.crt".to_owned(),
		"--status-interval-seconds".to_owned(),
		workers.status_interval_seconds.to_string(),
	];
	if workers.notifications_image.is_some() {
		// The worker refuses a release that bridges to anything but this
		// hub, and makes each tenant's database in a release.
		cluster_args.extend([
			"--notifications-namespace".to_owned(),
			NOTIFICATIONS_NAMESPACE.to_owned(),
			"--notifications-host".to_owned(),
			NOTIFICATIONS_HOST.to_owned(),
			"--notifications-port".to_owned(),
			NOTIFICATIONS_PORT.to_string(),
		]);
	}
	cluster_pod["containers"] = json!([{
		"name": "cluster-worker",
		"image": workers.cluster_image,
		"args": cluster_args,
		"env": [
			{"name": "LEPTOSKIT_CLOUD_WORKER_POSTGRES_URL", "valueFrom": {"secretKeyRef": {"name": "cluster-worker-postgres", "key": "uri"}}}
		],
		"volumeMounts": [
			{"name": "state", "mountPath": "/var/lib/leptoskit-worker"},
			{"name": "profile", "mountPath": "/etc/leptoskit/profile.json", "subPath": "profile.json", "readOnly": true},
			{"name": "postgres-ca", "mountPath": "/etc/leptoskit/postgres-ca", "readOnly": true},
			{"name": "tmp", "mountPath": "/tmp"}
		],
		"resources": {
			"requests": {"cpu": "100m", "memory": "128Mi"},
			"limits": {"cpu": "500m", "memory": "512Mi"}
		},
		"securityContext": {
			"allowPrivilegeEscalation": false,
			"readOnlyRootFilesystem": true,
			"capabilities": {"drop": ["ALL"]}
		}
	}]);
	documents.push((
		"60-workers",
		object(
			"apps/v1",
			"Deployment",
			"cluster-worker",
			Some("kube-system"),
			json!({
				"spec": {
					"replicas": 1,
					"strategy": {"type": "Recreate"},
					"selector": {"matchLabels": {"app": "leptoskit-cloud-cluster-worker"}},
					"template": {
						"metadata": {"labels": {"app": "leptoskit-cloud-cluster-worker"}},
						"spec": cluster_pod
					}
				}
			}),
		),
	));

	required.push(json!({
		"namespace": "kube-system",
		"name": "cluster-worker-postgres",
		"keys": ["uri"],
		"purpose": "Shared CNPG superuser connection URI for tenant database provisioning",
		"source": "copied from database/shared-superuser secret",
		"createdBy": "operator"
	}));
	required.push(json!({
		"namespace": "kube-system",
		"name": "cluster-worker-postgres-ca",
		"keys": ["ca.crt"],
		"purpose": "Private CA certificate signing the shared CNPG Postgres server",
		"source": "copied from database/shared-ca secret",
		"createdBy": "operator"
	}));

	if let Some(image) = &workers.notifications_image {
		let (hub_docs, hub_secrets) = notifications_documents(config, platform, image, worker_name);
		documents.extend(hub_docs);
		required.extend(hub_secrets);
	}

	let has_build_pool = cluster.nodes.values().any(|n| n.pool() == Pool::Build);
	if let Some(build_image) = &workers.build_image
		&& (has_build_pool || workers.allow_build_on_shared_node)
	{
		documents.push((
			"60-workers",
			object(
				"v1",
				"PersistentVolumeClaim",
				"build-worker-state",
				Some("kube-system"),
				json!({
					"spec": {
						"accessModes": ["ReadWriteOnce"],
						"storageClassName": config.postgres.storage_class,
						"resources": {"requests": {"storage": "1Gi"}}
					}
				}),
			),
		));
		documents.push((
			"60-workers",
			object(
				"v1",
				"PersistentVolumeClaim",
				"build-worker-cache",
				Some("kube-system"),
				json!({
					"spec": {
						"accessModes": ["ReadWriteOnce"],
						"storageClassName": config.postgres.storage_class,
						"resources": {"requests": {"storage": workers.build_cache_size}}
					}
				}),
			),
		));

		let mut build_pod = placement(cluster, Pool::Build);
		build_pod["automountServiceAccountToken"] = json!(false);
		build_pod["securityContext"] = json!({
			"runAsNonRoot": true,
			"runAsUser": 1000,
			"runAsGroup": 1000,
			"fsGroup": 1000,
			"seccompProfile": {"type": "Unconfined"}
		});
		build_pod["volumes"] = json!([
			{"name": "state", "persistentVolumeClaim": {"claimName": "build-worker-state"}},
			{"name": "cache", "persistentVolumeClaim": {"claimName": "build-worker-cache"}}
		]);
		let mut build_args = vec![
			"run".to_owned(),
			"--state".to_owned(),
			"/var/lib/leptoskit-worker".to_owned(),
			"--build-cache-dir".to_owned(),
			"/var/cache/leptoskit-build".to_owned(),
		];
		if matches!(config.registry, Registry::Zot { .. }) {
			build_args.push("--build-arg".to_owned());
			build_args.push("--insecure-registry".to_owned());
		}
		build_pod["containers"] = json!([{
			"name": "build-worker",
			"image": build_image,
			"args": build_args,
			"volumeMounts": [
				{"name": "state", "mountPath": "/var/lib/leptoskit-worker"},
				{"name": "cache", "mountPath": "/var/cache/leptoskit-build"}
			],
			"resources": {
				"requests": {"cpu": "500m", "memory": "1Gi"},
				"limits": {"cpu": "4000m", "memory": "8Gi"}
			}
		}]);
		documents.push((
			"60-workers",
			object(
				"apps/v1",
				"Deployment",
				"build-worker",
				Some("kube-system"),
				json!({
					"spec": {
						"replicas": 1,
						"strategy": {"type": "Recreate"},
						"selector": {"matchLabels": {"app": "leptoskit-cloud-build-worker"}},
						"template": {
							"metadata": {"labels": {"app": "leptoskit-cloud-build-worker"}},
							"spec": build_pod
						}
					}
				}),
			),
		));
	}

	Ok((documents, required))
}

/// The cluster's notifications hub: its namespace (`00-namespaces`), the
/// policies around it (`10-policies`), its state, `Deployment/hub` and
/// `Service/hub` (`60-workers`), and the one Secret it reads, the pairing
/// code that pairs it on its first start.
///
/// The hub's namespace isn't a platform namespace: the hub answers tenants
/// and the internet, so the platform's namespaces don't admit it. It is
/// admitted to the shared Postgres on 5432 and to the cluster's DNS, and
/// admits only Traefik and the apps' namespaces, on its port.
#[expect(
	clippy::too_many_lines,
	reason = "declarative Kubernetes manifests for the notifications hub"
)]
fn notifications_documents(
	config: &PlatformConfig,
	platform: &Value,
	image: &str,
	worker_name: &str,
) -> (Documents, Vec<Value>) {
	let mut documents = Vec::new();

	// The hub's pod meets `restricted`; the namespace enforces it.
	let mut namespace = object("v1", "Namespace", NOTIFICATIONS_NAMESPACE, None, json!({}));
	namespace["metadata"]["labels"] = json!({
		"pod-security.kubernetes.io/enforce": "restricted",
		"pod-security.kubernetes.io/enforce-version": "v1.35"
	});
	documents.push(("00-namespaces", namespace));

	let hub = json!({"namespaceSelector": {"matchLabels": {"kubernetes.io/metadata.name": NOTIFICATIONS_NAMESPACE}}});
	documents.push((
		"10-policies",
		object(
			"networking.k8s.io/v1",
			"NetworkPolicy",
			"hub-ingress",
			Some(NOTIFICATIONS_NAMESPACE),
			json!({"spec": {
				"podSelector": {},
				"policyTypes": ["Ingress"],
				"ingress": [
					{
						"from": [{
							"namespaceSelector": {"matchLabels": {"kubernetes.io/metadata.name": "kube-system"}},
							"podSelector": {"matchLabels": {"app.kubernetes.io/name": "traefik"}}
						}],
						"ports": [{"protocol": "TCP", "port": NOTIFICATIONS_PORT}]
					},
					{
						"from": [{"namespaceSelector": {"matchLabels": {"app.kubernetes.io/managed-by": "leptoskit-cloud"}}}],
						"ports": [{"protocol": "TCP", "port": NOTIFICATIONS_PORT}]
					}
				]
			}}),
		),
	));
	documents.push((
		"10-policies",
		object(
			"networking.k8s.io/v1",
			"NetworkPolicy",
			"notifications-hub",
			Some("database"),
			json!({"spec": {
				"podSelector": {"matchLabels": {"cnpg.io/cluster": "shared"}},
				"policyTypes": ["Ingress"],
				"ingress": [{"from": [hub.clone()], "ports": [{"protocol": "TCP", "port": 5432}]}]
			}}),
		),
	));
	documents.push((
		"10-policies",
		object(
			"networking.k8s.io/v1",
			"NetworkPolicy",
			"notifications-dns",
			Some("kube-system"),
			json!({"spec": {
				"podSelector": {"matchLabels": {"k8s-app": "kube-dns"}},
				"policyTypes": ["Ingress"],
				"ingress": [{"from": [hub], "ports": [{"protocol": "UDP", "port": 53}, {"protocol": "TCP", "port": 53}]}]
			}}),
		),
	));

	documents.push((
		"60-workers",
		object(
			"v1",
			"PersistentVolumeClaim",
			"hub-state",
			Some(NOTIFICATIONS_NAMESPACE),
			json!({
				"spec": {
					"accessModes": ["ReadWriteOnce"],
					"storageClassName": config.postgres.storage_class,
					"resources": {"requests": {"storage": "1Gi"}}
				}
			}),
		),
	));

	// The hub runs no tenant code and needs no Kubernetes API: it holds
	// every tenant's values on the cluster in memory, so its pod gets no
	// service-account token.
	let mut pod = platform.clone();
	pod["automountServiceAccountToken"] = json!(false);
	pod["securityContext"] = json!({
		"runAsNonRoot": true,
		"runAsUser": 65532,
		"runAsGroup": 65532,
		"fsGroup": 65532,
		"seccompProfile": {"type": "RuntimeDefault"}
	});
	pod["volumes"] = json!([
		{"name": "state", "persistentVolumeClaim": {"claimName": "hub-state"}},
		{"name": "tmp", "emptyDir": {"sizeLimit": "64Mi"}}
	]);
	pod["containers"] = json!([{
		"name": "hub",
		"image": image,
		"args": ["--state", "/var/lib/leptoskit-notifications"],
		"ports": [{"name": "http", "containerPort": NOTIFICATIONS_PORT}],
		"env": [
			{"name": "LEPTOSKIT_CLOUD_NOTIFICATIONS_LISTEN", "value": format!("0.0.0.0:{NOTIFICATIONS_PORT}")},
			{"name": "LEPTOSKIT_CLOUD_NOTIFICATIONS_URL", "value": config.control_origin},
			{"name": "LEPTOSKIT_CLOUD_NOTIFICATIONS_NAME", "value": worker_name},
			{"name": "LEPTOSKIT_CLOUD_NOTIFICATIONS_PAIRING_CODE", "valueFrom": {"secretKeyRef": {"name": "hub-pairing", "key": "code", "optional": true}}}
		],
		"readinessProbe": {"httpGet": {"path": "/readyz", "port": NOTIFICATIONS_PORT}, "initialDelaySeconds": 5, "periodSeconds": 10},
		"livenessProbe": {"httpGet": {"path": "/healthz", "port": NOTIFICATIONS_PORT}, "initialDelaySeconds": 15, "periodSeconds": 20},
		"volumeMounts": [
			{"name": "state", "mountPath": "/var/lib/leptoskit-notifications"},
			{"name": "tmp", "mountPath": "/tmp"}
		],
		"resources": {
			"requests": {"cpu": "100m", "memory": "256Mi"},
			"limits": {"cpu": "2000m", "memory": "2Gi"}
		},
		"securityContext": {
			"allowPrivilegeEscalation": false,
			"readOnlyRootFilesystem": true,
			"capabilities": {"drop": ["ALL"]}
		}
	}]);
	// One replica: its state volume is `ReadWriteOnce`, and the paired key
	// is locked by the process that holds it.
	documents.push((
		"60-workers",
		object(
			"apps/v1",
			"Deployment",
			"hub",
			Some(NOTIFICATIONS_NAMESPACE),
			json!({
				"spec": {
					"replicas": 1,
					"strategy": {"type": "Recreate"},
					"selector": {"matchLabels": {"app": "leptoskit-cloud-notifications"}},
					"template": {
						"metadata": {"labels": {"app": "leptoskit-cloud-notifications"}},
						"spec": pod
					}
				}
			}),
		),
	));

	documents.push((
		"60-workers",
		object(
			"v1",
			"Service",
			"hub",
			Some(NOTIFICATIONS_NAMESPACE),
			json!({
				"spec": {
					"selector": {"app": "leptoskit-cloud-notifications"},
					"ports": [{"name": "http", "port": NOTIFICATIONS_PORT, "targetPort": NOTIFICATIONS_PORT}]
				}
			}),
		),
	));

	let required = vec![json!({
		"namespace": NOTIFICATIONS_NAMESPACE,
		"name": "hub-pairing",
		"keys": ["code"],
		"purpose": "One-time pairing code for the notifications hub, read only while its state is unpaired; delete it once the hub has paired",
		"source": "issue_pairing for the notifications role and this cluster",
		"createdBy": "operator"
	})];

	(documents, required)
}
