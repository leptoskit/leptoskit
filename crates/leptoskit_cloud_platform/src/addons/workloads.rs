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

	// Serving Deployment.
	let mut serve_env = env;
	serve_env
		.push(json!({"name": "LEPTOSKIT_CLOUD_SWEEPER", "value": (cp.replicas == 1).to_string()}));
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
	cluster_pod["containers"] = json!([{
		"name": "cluster-worker",
		"image": workers.cluster_image,
		"args": [
			"run",
			"--state", "/var/lib/leptoskit-worker",
			"--profile-json", "/etc/leptoskit/profile.json",
			"--reconcile-on-start",
			"--runtime-class", config.isolation.runtime_class,
			"--postgres-namespace", "database",
			"--postgres-ca", "/etc/leptoskit/postgres-ca/ca.crt",
			"--status-interval-seconds", workers.status_interval_seconds.to_string()
		],
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
