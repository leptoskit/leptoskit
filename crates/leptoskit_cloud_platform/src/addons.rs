//! Offline Kubernetes add-ons. JSON is a YAML 1.2 subset, so the existing
//! serializer provides escaping and stable ordering without another parser.
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::Generated;
use crate::PlatformError;
use crate::config::Cluster;
use crate::config::CredentialRef;
use crate::config::CustomDomains;
use crate::config::EdgeMode;
use crate::config::PlatformConfig;
use crate::config::Pool;
use crate::config::Registry;

mod registry;
mod workloads;

/// Origin shared with the `OpenTofu` tunnel configuration.
pub const TUNNEL_ORIGIN: &str = "http://traefik.kube-system.svc.cluster.local:80";

/// Runtime profile for one configured cluster. The database is the shared
/// CNPG cluster in namespace `database`, TCP 5432; releases name that door.
/// No runtime dependency on this operator crate is needed.
#[must_use]
pub fn runtime_profile(
	config: &PlatformConfig,
	cluster: &Cluster,
) -> leptoskit_cloud_runtime::ClusterProfile {
	let mut profile = leptoskit_cloud_runtime::ClusterProfile::platform_defaults();
	profile
		.blocked_egress_cidrs
		.extend(config.isolation.denied_egress_cidrs.clone());
	profile
		.blocked_egress_cidrs
		.extend([cluster.pod_cidr.clone(), cluster.service_cidr.clone()]);
	profile.blocked_egress_cidrs.sort();
	profile.blocked_egress_cidrs.dedup();
	match config.edge.custom_domains {
		CustomDomains::CloudflareSaas => {
			profile.edge_terminated_custom_domains = true;
			profile.cluster_issuer = None;
		}
		CustomDomains::CertManager => {
			profile.edge_terminated_custom_domains = false;
			profile.cluster_issuer = Some("apps-http01".to_owned());
		}
	}
	if let Registry::External { pull_secret, .. } = &config.registry {
		profile.image_pull_secret = Some(pull_secret.clone());
	}
	if cluster.nodes.values().any(|n| n.pool() == Pool::Tenant) {
		let effect = if cluster.nodes.values().any(|n| n.pool() == Pool::Platform) {
			"NoSchedule"
		} else {
			"PreferNoSchedule"
		};
		profile
			.tenant_node_selector
			.insert("leptoskit.dev/pool".to_owned(), "tenant".to_owned());
		profile
			.tenant_tolerations
			.push(leptoskit_cloud_runtime::TolerationSpec {
				key: "leptoskit.dev/pool".to_owned(),
				operator: "Equal".to_owned(),
				value: Some("tenant".to_owned()),
				effect: effect.to_owned(),
			});
	}
	profile
}

#[derive(Serialize)]
struct Metadata<'a> {
	name: &'a str,
	#[serde(skip_serializing_if = "Option::is_none")]
	namespace: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest<'a> {
	api_version: &'a str,
	kind: &'a str,
	metadata: Metadata<'a>,
	#[serde(flatten)]
	body: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HelmChartSpec<'a> {
	repo: &'a str,
	chart: &'a str,
	version: &'a str,
	target_namespace: &'a str,
	values_content: String,
}

fn object(api: &str, kind: &str, name: &str, namespace: Option<&str>, body: Value) -> Value {
	// This concrete envelope contains only strings and JSON values: its
	// serialization has no custom fallible serializer.
	json!(Manifest {
		api_version: api,
		kind,
		metadata: Metadata { name, namespace },
		body
	})
}

fn placement(cluster: &Cluster, desired: Pool) -> Value {
	// The launch box has only a tenant label. Hard selectors must name a
	// pool that exists. Build nodes remain exclusive.
	let has_tenant = cluster.nodes.values().any(|n| n.pool() == Pool::Tenant);
	let pool = if cluster.nodes.values().any(|n| n.pool() == desired) {
		desired
	} else if cluster.nodes.values().any(|n| n.pool() == Pool::Platform) {
		Pool::Platform
	} else if has_tenant {
		Pool::Tenant
	} else {
		Pool::Ingress
	};
	let name = match pool {
		Pool::Platform => "platform",
		Pool::Ingress => "ingress",
		Pool::Tenant => "tenant",
		Pool::Build => "build",
	};
	let effect = if pool == Pool::Build || (pool == Pool::Platform && has_tenant) {
		"NoSchedule"
	} else {
		"PreferNoSchedule"
	};
	json!({"nodeSelector":{"leptoskit.dev/pool":name},"tolerations":[{"key":"leptoskit.dev/pool","operator":"Equal","value":name,"effect":effect}]})
}

fn chart(
	name: &str,
	ns: &str,
	repo: &str,
	package: &str,
	version: &str,
	values: &Value,
) -> Result<Value, PlatformError> {
	let spec = HelmChartSpec {
		repo,
		chart: package,
		version,
		target_namespace: ns,
		values_content: serde_json::to_string_pretty(values)?,
	};
	Ok(object(
		"helm.cattle.io/v1",
		"HelmChart",
		name,
		Some("kube-system"),
		json!({"spec":spec}),
	))
}

type Documents = Vec<(&'static str, Value)>;

fn common_charts(config: &PlatformConfig, platform: &Value) -> Result<Documents, PlatformError> {
	// Verified values: see the versioned upstream sources in the runbook.
	let mut keda = platform.clone();
	for component in ["operator", "metricsServer", "webhooks"] {
		keda[component] = platform.clone();
	}
	let mut http = json!({});
	for component in ["operator", "scaler", "interceptor"] {
		http[component] = platform.clone();
	}
	http["interceptor"]["directPodRouting"] = json!(false);
	let mut cert = platform.clone();
	cert["crds"] = json!({"enabled":true});
	for component in ["webhook", "cainjector", "startupapicheck"] {
		cert[component] = platform.clone();
	}
	Ok(vec![
		(
			"20-charts",
			chart(
				"cloudnative-pg",
				"cnpg-system",
				"https://cloudnative-pg.github.io/charts",
				"cloudnative-pg",
				&config.addons.cloudnative_pg,
				platform,
			)?,
		),
		(
			"20-charts",
			chart(
				"keda",
				"keda",
				"https://kedacore.github.io/charts",
				"keda",
				&config.addons.keda,
				&keda,
			)?,
		),
		(
			"21-http",
			chart(
				"keda-add-ons-http",
				"keda",
				"https://kedacore.github.io/charts",
				"keda-add-ons-http",
				&config.addons.keda_http,
				&http,
			)?,
		),
		(
			"20-charts",
			chart(
				"cert-manager",
				"cert-manager",
				"https://charts.jetstack.io",
				"cert-manager",
				&config.addons.cert_manager,
				&cert,
			)?,
		),
	])
}

fn platform_namespaces(config: &PlatformConfig, edge: bool) -> Vec<&'static str> {
	let mut namespaces = vec![
		"kube-system",
		"cnpg-system",
		"database",
		"keda",
		"cert-manager",
	];
	if edge && matches!(config.registry, Registry::Zot { .. }) {
		namespaces.push("registry");
	}
	if edge && config.control_plane.is_some() {
		namespaces.push(workloads::CONTROL_NAMESPACE);
	}
	namespaces
}

fn namespaces_and_worker(namespaces: &[&'static str]) -> Result<Documents, PlatformError> {
	let mut documents = namespaces_and_runtime(namespaces);
	let (role, binding) =
		leptoskit_cloud_runtime::kubernetes::worker_rbac("kube-system", "leptoskit-cloud-worker");
	documents.push(("10-worker-rbac", serde_json::to_value(role)?));
	documents.push(("10-worker-rbac", serde_json::to_value(binding)?));
	let (vap, vap_binding) = leptoskit_cloud_runtime::kubernetes::worker_admission_policy(
		"kube-system",
		"leptoskit-cloud-worker",
	);
	documents.push(("10-worker-rbac", serde_json::to_value(vap)?));
	documents.push(("10-worker-rbac", serde_json::to_value(vap_binding)?));
	Ok(documents)
}

fn namespaces_and_runtime(namespaces: &[&'static str]) -> Documents {
	let mut documents = Vec::new();
	for &ns in namespaces {
		let level = if ns == "kube-system" {
			"privileged"
		} else {
			"baseline"
		};
		let mut doc = object("v1", "Namespace", ns, None, json!({}));
		doc["metadata"]["labels"] = json!({"pod-security.kubernetes.io/enforce":level,"pod-security.kubernetes.io/enforce-version":"v1.35"});
		documents.push(("00-namespaces", doc));
	}
	documents.push((
		"10-runtime",
		object(
			"node.k8s.io/v1",
			"RuntimeClass",
			"gvisor",
			None,
			json!({"handler":"runsc"}),
		),
	));
	documents
}

pub(super) fn credential_keys(reference: &CredentialRef, cold: bool) -> (&str, &str, &str) {
	match reference {
		CredentialRef::KubernetesSecret {
			name,
			access_key,
			secret_key,
		} => (name.as_str(), access_key.as_str(), secret_key.as_str()),
		CredentialRef::Environment { .. } => {
			if cold {
				("object-storage-cold", "ACCESS_KEY_ID", "SECRET_ACCESS_KEY")
			} else {
				("object-storage", "ACCESS_KEY_ID", "SECRET_ACCESS_KEY")
			}
		}
	}
}

/// Host CIDRs for control-plane servers so host-network `kube-apiserver`
/// webhooks and `APIService` probes (cert-manager, CNPG, KEDA, metrics-server)
/// can reach platform pods across nodes.
fn server_host_cidrs(cluster: &Cluster) -> Vec<String> {
	let mut cidrs = Vec::new();
	if let Some(private_cidr) = &cluster.private_cidr
		&& cluster
			.nodes
			.values()
			.any(|node| node.role() == crate::config::Role::Server && node.cloud())
	{
		cidrs.push(private_cidr.clone());
	}
	for node in cluster.nodes.values() {
		if let crate::config::Node::Existing {
			role: crate::config::Role::Server,
			public_address,
			private_address,
			..
		} = node
		{
			cidrs.push(format!("{public_address}/32"));
			if let Some(private) = private_address {
				cidrs.push(format!("{private}/32"));
			}
		}
	}
	cidrs.sort();
	cidrs.dedup();
	cidrs
}

/// Generate independent, ordered YAML documents per cluster without resolving
/// secrets or contacting chart repositories.
#[expect(
	clippy::too_many_lines,
	reason = "declarative phase-ordered cluster add-on manifest generation"
)]
pub fn generate(config: &PlatformConfig) -> Result<Generated, PlatformError> {
	config.validate()?;
	let mut files = Generated::new();
	for (region, entry) in &config.regions {
		for (name, cluster) in &entry.clusters {
			crate::config::require(
				cluster
					.nodes
					.values()
					.any(|node| node.pool() != Pool::Build),
				"add-ons require a non-build node",
			)?;
			let edge = region == &config.default_region && name == &config.edge.cluster;
			let namespaces = platform_namespaces(config, edge);
			let mut documents = namespaces_and_worker(&namespaces)?;
			let platform = placement(cluster, Pool::Platform);
			documents.extend(common_charts(config, &platform)?);
			documents.extend(registry::documents(config, &platform, edge)?);
			if edge {
				let mut traefik = placement(cluster, Pool::Ingress);
				traefik["fullnameOverride"] = json!("traefik");
				// Tenant Ingress uses the HTTP add-on's documented ExternalName
				// bridge.
				traefik["providers"] =
					json!({"kubernetesIngress":{"allowExternalNameServices":true}});
				traefik["service"] = json!({"type":"ClusterIP"});
				// Tunnel requests originate from connector pods, not Cloudflare
				// IPs.
				let trusted = if config.edge.mode == EdgeMode::Tunnel {
					vec![cluster.pod_cidr.clone()]
				} else {
					config.edge.cloudflare_cidrs.clone()
				};
				traefik["ports"] = json!({"web":{"forwardedHeaders":{"trustedIPs":trusted}},"websecure":{"forwardedHeaders":{"trustedIPs":config.edge.cloudflare_cidrs}}});
				if config.edge.mode == EdgeMode::Https {
					traefik["deployment"] = json!({"kind":"DaemonSet"});
					traefik["ports"]["websecure"]["hostPort"] = json!(443);
					let sources: Vec<_> = config
						.edge
						.cloudflare_cidrs
						.iter()
						.map(|cidr| json!({"ipBlock":{"cidr":cidr}}))
						.collect();
					// HostPort DNAT can bypass host INPUT rules. Enforce the
					// same source boundary at the destination pod, on its
					// target port.
					let mut traefik_ingress =
						vec![json!({"from":sources,"ports":[{"protocol":"TCP","port":8443}]})];
					if config.edge.custom_domains == CustomDomains::CertManager {
						traefik["ports"]["web"]["hostPort"] = json!(80);
						traefik_ingress.push(json!({"from":[{"ipBlock":{"cidr":"0.0.0.0/0"}},{"ipBlock":{"cidr":"::/0"}}],"ports":[{"protocol":"TCP","port":8000}]}));
					}
					documents.push(("10-policies",object("networking.k8s.io/v1","NetworkPolicy","cloudflare-https",Some("kube-system"),json!({"spec":{"podSelector":{"matchLabels":{"app.kubernetes.io/name":"traefik"}},"policyTypes":["Ingress"],"ingress":traefik_ingress}}))));
				}
				documents.push((
					"20-charts",
					chart(
						"traefik",
						"kube-system",
						"https://traefik.github.io/charts",
						"traefik",
						&config.addons.traefik,
						&traefik,
					)?,
				));
				documents.push(("30-certificates",object("cert-manager.io/v1","ClusterIssuer","apps",None,json!({"spec":{"acme":{"server":"https://acme-v02.api.letsencrypt.org/directory","email":config.addons.acme_email,"privateKeySecretRef":{"name":"apps-acme-account"},"solvers":[{"dns01":{"cloudflare":{"apiTokenSecretRef":{"name":config.addons.cloudflare_api_secret,"key":"api-token"}}}}]}}}))));
				if config.edge.custom_domains == CustomDomains::CertManager {
					documents.push(("30-certificates",object("cert-manager.io/v1","ClusterIssuer","apps-http01",None,json!({"spec":{"acme":{"server":"https://acme-v02.api.letsencrypt.org/directory","email":config.addons.acme_email,"privateKeySecretRef":{"name":"apps-http01-acme-account"},"solvers":[{"http01":{"ingress":{"ingressClassName":"traefik"}}}]}}}))));
				}
				documents.push(("30-certificates",object("cert-manager.io/v1","Certificate","apps-wildcard",Some("kube-system"),json!({"spec":{"secretName":"apps-wildcard-tls","issuerRef":{"name":"apps","kind":"ClusterIssuer"},"dnsNames":[format!("*.{}",config.apps_domain)]}}))));
				documents.push((
					"30-certificates",
					object(
						"traefik.io/v1alpha1",
						"TLSStore",
						"default",
						Some("kube-system"),
						json!({"spec":{"defaultCertificate":{"secretName":"apps-wildcard-tls"}}}),
					),
				));
				if config.edge.mode == EdgeMode::Tunnel {
					let mut pod = platform.clone();
					pod["automountServiceAccountToken"] = json!(false);
					pod["securityContext"] = json!({"runAsNonRoot":true,"runAsUser":65532,"runAsGroup":65532,"seccompProfile":{"type":"RuntimeDefault"}});
					pod["containers"] = json!([{"name":"cloudflared","image":"cloudflare/cloudflared:2026.11.0","args":["tunnel","--no-autoupdate","run"],"env":[{"name":"TUNNEL_TOKEN","valueFrom":{"secretKeyRef":{"name":"cloudflare-tunnel","key":"token"}}}],"resources":{"requests":{"cpu":"50m","memory":"64Mi"},"limits":{"cpu":"500m","memory":"256Mi"}},"securityContext":{"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}}}]);
					documents.push(("40-tunnel",object("apps/v1","Deployment","cloudflared",Some("kube-system"),json!({"spec":{"replicas":2,"selector":{"matchLabels":{"app":"cloudflared"}},"template":{"metadata":{"labels":{"app":"cloudflared"}},"spec":pod}}}))));
				}
			}
			let (hot_secret, hot_access, hot_key) =
				credential_keys(&config.object_storage.credentials, false);
			let (backups_endpoint, backups_region, backups_cred_ref) =
				config.object_storage.backups_store();
			let uses_cold = config.object_storage.cold.is_some();
			let (db_secret, db_access, db_key) = credential_keys(backups_cred_ref, uses_cold);
			let mut required = vec![
				json!({"namespace":"database","name":db_secret,"keys":[db_access,db_key],"purpose":"WAL archives and base backups","source":backups_cred_ref,"createdBy":"operator"}),
			];
			if edge {
				if let Registry::Zot {
					auth_secret,
					expose_ingress,
				} = &config.registry
				{
					required.push(json!({"namespace":"registry","name":auth_secret,"keys":["htpasswd"],"purpose":"Authenticated registry push and pull","source":"operator bcrypt htpasswd file","createdBy":"operator"}));
					required.push(json!({"namespace":"registry","name":hot_secret,"keys":[hot_access,hot_key],"purpose":"Registry S3 storage","source":config.object_storage.credentials,"createdBy":"operator"}));
					if *expose_ingress {
						required.push(json!({"namespace":"registry","name":"registry-tls","keys":["tls.crt","tls.key"],"purpose":"Registry TLS","source":"Certificate registry","createdBy":"cert-manager"}));
					}
				}
				required.push(json!({"namespace":"cert-manager","name":config.addons.cloudflare_api_secret,"keys":["api-token"],"purpose":"DNS-01 challenge","source":"Cloudflare token with zone DNS edit and zone read","createdBy":"operator"}));
				required.push(json!({"namespace":"cert-manager","name":"apps-acme-account","keys":["tls.key"],"purpose":"ACME account key","source":"cert-manager generates this; do not precreate","createdBy":"cert-manager"}));
				if config.edge.custom_domains == CustomDomains::CertManager {
					required.push(json!({"namespace":"cert-manager","name":"apps-http01-acme-account","keys":["tls.key"],"purpose":"ACME account key for customer HTTP-01 domains","source":"cert-manager generates this; do not precreate","createdBy":"cert-manager"}));
				}
				required.push(json!({"namespace":"kube-system","name":"apps-wildcard-tls","keys":["tls.crt","tls.key"],"purpose":"Ingress default wildcard certificate","source":"Certificate apps-wildcard generates this; do not precreate","createdBy":"cert-manager"}));
				if config.edge.mode == EdgeMode::Tunnel {
					required.push(json!({"namespace":"kube-system","name":"cloudflare-tunnel","keys":["token"],"purpose":"Tunnel connector","source":"sensitive OpenTofu tunnel_token output","createdBy":"operator"}));
				}
				if let Some(cp) = &config.control_plane {
					let (cp_docs, cp_secrets) = workloads::control_plane_documents(
						config, cluster, &platform, cp, region, name,
					)?;
					documents.extend(cp_docs);
					required.extend(cp_secrets);
				}
			}
			if let Some(workers) = &config.workers {
				let (worker_docs, worker_secrets) =
					workloads::worker_documents(config, cluster, &platform, workers)?;
				documents.extend(worker_docs);
				required.extend(worker_secrets);
			}
			documents.push((
				"01-secrets",
				object(
					"v1",
					"ConfigMap",
					"required-secrets",
					Some("kube-system"),
					json!({"data":{"requirements.json":serde_json::to_string_pretty(&required)?}}),
				),
			));
			// Native Barman and affinity fields verified against the v1.30.2
			// API. Superuser access is enabled so the cluster worker can
			// provision isolated tenant databases and roles; pg_hba explicitly
			// rejects plaintext TCP connections before CNPG's defaults.
			let mut affinity = platform.clone();
			affinity["podAntiAffinityType"] = json!("preferred");
			documents.push(("30-database",object("postgresql.cnpg.io/v1","Cluster","shared",Some("database"),json!({"spec":{"instances":config.postgres.instances,"enableSuperuserAccess":true,"postgresql":{"parameters":{"max_connections":config.postgres.max_connections.to_string()},"pg_hba":["hostnossl all all 0.0.0.0/0 reject","hostnossl all all ::/0 reject","hostssl all all 0.0.0.0/0 scram-sha-256","hostssl all all ::/0 scram-sha-256"]},"storage":{"size":config.postgres.storage_size,"storageClass":config.postgres.storage_class},"affinity":affinity,"backup":{"retentionPolicy":format!("{}d",config.postgres.retention_days),"barmanObjectStore":{"destinationPath":format!("s3://{}/{region}/{name}",config.postgres.wal_bucket),"endpointURL":backups_endpoint,"s3Credentials":{"accessKeyId":{"name":db_secret,"key":db_access},"secretAccessKey":{"name":db_secret,"key":db_key}},"wal":{"compression":"gzip"}}}}}))));
			documents.push(("30-database",object("postgresql.cnpg.io/v1","ScheduledBackup","shared-daily",Some("database"),json!({"spec":{"schedule":"0 0 2 * * *","backupOwnerReference":"self","cluster":{"name":"shared"},"method":"barmanObjectStore"}}))));
			documents.extend(platform_network_policies(cluster, &namespaces));
			for (index, (phase, mut document)) in documents.into_iter().enumerate() {
				if document["kind"] == "Cluster" {
					document["spec"]["env"] =
						json!([{"name":"AWS_DEFAULT_REGION","value":backups_region}]);
				}
				files.insert(
					format!("{region}/{name}/addons/{phase}-{index:02}.yaml"),
					format!("{}\n", serde_json::to_string_pretty(&document)?),
				);
			}
		}
	}
	Ok(files)
}

fn platform_network_policies(cluster: &Cluster, namespaces: &[&str]) -> Documents {
	let mut docs = Vec::new();
	let control_plane_cidrs: Vec<Value> = server_host_cidrs(cluster)
		.into_iter()
		.map(|cidr| json!({"ipBlock":{"cidr":cidr}}))
		.collect();
	for &ns in namespaces {
		// The runtime owns tenant default-deny and egress policies. Protect
		// each destination namespace with an explicit allowlist of trusted
		// platform namespaces instead of a denylist.
		let mut ingress = vec![
			json!({"from":[{"namespaceSelector":{"matchExpressions":[{"key":"kubernetes.io/metadata.name","operator":"In","values":namespaces}]}}]}),
		];
		if !control_plane_cidrs.is_empty()
			&& matches!(ns, "kube-system" | "cnpg-system" | "keda" | "cert-manager")
		{
			// Host-network kube-apiserver calls webhooks and APIServices from
			// server node IPs on multi-node clusters.
			ingress.push(json!({"from":control_plane_cidrs}));
		}
		if ns == "database" {
			ingress.push(json!({"from":[{"namespaceSelector":{"matchLabels":{"app.kubernetes.io/managed-by":"leptoskit-cloud"}}}],"ports":[{"protocol":"TCP","port":5432}]}));
		}
		docs.push((
			"10-policies",
			object(
				"networking.k8s.io/v1",
				"NetworkPolicy",
				"platform-ingress",
				Some(ns),
				json!({"spec":{"podSelector":{},"policyTypes":["Ingress"],"ingress":ingress}}),
			),
		));
	}
	docs.push(("10-policies",object("networking.k8s.io/v1","NetworkPolicy","tenant-dns",Some("kube-system"),json!({"spec":{"podSelector":{"matchLabels":{"k8s-app":"kube-dns"}},"policyTypes":["Ingress"],"ingress":[{"from":[{"namespaceSelector":{"matchLabels":{"app.kubernetes.io/managed-by":"leptoskit-cloud"}}}],"ports":[{"protocol":"UDP","port":53},{"protocol":"TCP","port":53}]}]}}))));
	docs
}
