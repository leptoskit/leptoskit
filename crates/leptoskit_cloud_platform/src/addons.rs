//! Offline Kubernetes add-ons. JSON is a YAML 1.2 subset, so the existing
//! serializer provides escaping and stable ordering without another parser.
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::Generated;
use crate::PlatformError;
use crate::config::Cluster;
use crate::config::CredentialRef;
use crate::config::EdgeMode;
use crate::config::PlatformConfig;
use crate::config::Pool;

mod registry;

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
	let pool = if cluster.nodes.values().any(|n| n.pool() == desired) {
		desired
	} else if cluster.nodes.values().any(|n| n.pool() == Pool::Platform) {
		Pool::Platform
	} else if cluster.nodes.values().any(|n| n.pool() == Pool::Tenant) {
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
	json!({"nodeSelector":{"leptoskit.dev/pool":name},"tolerations":[{"key":"leptoskit.dev/pool","operator":"Equal","value":name,"effect":"PreferNoSchedule"}]})
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
				"http",
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

fn namespaces_and_worker() -> Result<Documents, PlatformError> {
	let mut documents = namespaces_and_runtime();
	let (role, binding) =
		leptoskit_cloud_runtime::kubernetes::worker_rbac("kube-system", "leptoskit-cloud-worker");
	documents.push(("10-worker-rbac", serde_json::to_value(role)?));
	documents.push(("10-worker-rbac", serde_json::to_value(binding)?));
	Ok(documents)
}

fn namespaces_and_runtime() -> Documents {
	let mut documents = Vec::new();
	for ns in [
		"kube-system",
		"cnpg-system",
		"database",
		"keda",
		"cert-manager",
		"registry",
	] {
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

/// Generate independent, ordered YAML documents per cluster without resolving
/// secrets or contacting chart repositories.
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
			let mut documents = namespaces_and_worker()?;
			let platform = placement(cluster, Pool::Platform);
			documents.extend(common_charts(config, &platform)?);
			documents.extend(registry::documents(
				config,
				&platform,
				region == &config.default_region && name == &config.edge.cluster,
			)?);
			if region == &config.default_region && name == &config.edge.cluster {
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
					// same source boundary at the
					// destination pod, on its target port.
					documents.push(("10-policies",object("networking.k8s.io/v1","NetworkPolicy","cloudflare-https",Some("kube-system"),json!({"spec":{"podSelector":{"matchLabels":{"app.kubernetes.io/name":"traefik"}},"policyTypes":["Ingress"],"ingress":[{"from":sources,"ports":[{"protocol":"TCP","port":8443}]}]}}))));
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
					pod["containers"] = json!([{"name":"cloudflared","image":"cloudflare/cloudflared:2026.11.0","args":["tunnel","--no-autoupdate","run"],"env":[{"name":"TUNNEL_TOKEN","valueFrom":{"secretKeyRef":{"name":"cloudflare-tunnel","key":"token"}}}]}]);
					documents.push(("40-tunnel",object("apps/v1","Deployment","cloudflared",Some("kube-system"),json!({"spec":{"replicas":2,"selector":{"matchLabels":{"app":"cloudflared"}},"template":{"metadata":{"labels":{"app":"cloudflared"}},"spec":pod}}}))));
				}
			}
			let (secret, access, key) = match &config.object_storage.credentials {
				CredentialRef::KubernetesSecret {
					name,
					access_key,
					secret_key,
				} => (name.as_str(), access_key.as_str(), secret_key.as_str()),
				CredentialRef::Environment { .. } => {
					("object-storage", "ACCESS_KEY_ID", "SECRET_ACCESS_KEY")
				}
			};
			let mut required = vec![
				json!({"namespace":"database","name":secret,"keys":[access,key],"purpose":"WAL archives and base backups","source":config.object_storage.credentials,"createdBy":"operator"}),
			];
			if region == &config.default_region && name == &config.edge.cluster {
				if let crate::config::Registry::Zot { auth_secret } = &config.registry {
					required.push(json!({"namespace":"registry","name":auth_secret,"keys":["htpasswd"],"purpose":"Authenticated registry push and pull","source":"operator bcrypt htpasswd file","createdBy":"operator"}));
					required.push(json!({"namespace":"registry","name":secret,"keys":[access,key],"purpose":"Registry S3 storage","source":config.object_storage.credentials,"createdBy":"operator"}));
					required.push(json!({"namespace":"registry","name":"registry-tls","keys":["tls.crt","tls.key"],"purpose":"Registry TLS","source":"Certificate registry","createdBy":"cert-manager"}));
				}
				required.push(json!({"namespace":"cert-manager","name":config.addons.cloudflare_api_secret,"keys":["api-token"],"purpose":"DNS-01 challenge","source":"Cloudflare token with zone DNS edit and zone read","createdBy":"operator"}));
				required.push(json!({"namespace":"cert-manager","name":"apps-acme-account","keys":["tls.key"],"purpose":"ACME account key","source":"cert-manager generates this; do not precreate","createdBy":"cert-manager"}));
				required.push(json!({"namespace":"kube-system","name":"apps-wildcard-tls","keys":["tls.crt","tls.key"],"purpose":"Ingress default wildcard certificate","source":"Certificate apps-wildcard generates this; do not precreate","createdBy":"cert-manager"}));
				if config.edge.mode == EdgeMode::Tunnel {
					required.push(json!({"namespace":"kube-system","name":"cloudflare-tunnel","keys":["token"],"purpose":"Tunnel connector","source":"sensitive OpenTofu tunnel_token output","createdBy":"operator"}));
				}
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
			// API.
			let mut affinity = platform.clone();
			affinity["podAntiAffinityType"] = json!("preferred");
			documents.push(("30-database",object("postgresql.cnpg.io/v1","Cluster","shared",Some("database"),json!({"spec":{"instances":config.postgres.instances,"storage":{"size":config.postgres.storage_size,"storageClass":config.postgres.storage_class},"affinity":affinity,"backup":{"retentionPolicy":format!("{}d",config.postgres.retention_days),"barmanObjectStore":{"destinationPath":format!("s3://{}/{region}/{name}",config.postgres.wal_bucket),"endpointURL":config.object_storage.endpoint,"s3Credentials":{"accessKeyId":{"name":secret,"key":access},"secretAccessKey":{"name":secret,"key":key}},"wal":{"compression":"gzip"}}}}}))));
			documents.push(("30-database",object("postgresql.cnpg.io/v1","ScheduledBackup","shared-daily",Some("database"),json!({"spec":{"schedule":"0 0 2 * * *","backupOwnerReference":"self","cluster":{"name":"shared"},"method":"barmanObjectStore"}}))));
			for ns in [
				"kube-system",
				"cnpg-system",
				"database",
				"keda",
				"cert-manager",
				"registry",
			] {
				// The runtime owns tenant default-deny and egress policies.
				// Here protect the destination namespaces;
				// policies are additive.
				let mut ingress = vec![
					json!({"from":[{"namespaceSelector":{"matchExpressions":[{"key":"app.kubernetes.io/managed-by","operator":"NotIn","values":["leptoskit-cloud"]}]}}]}),
				];
				if ns == "database" {
					ingress.push(json!({"from":[{"namespaceSelector":{"matchLabels":{"app.kubernetes.io/managed-by":"leptoskit-cloud"}}}],"ports":[{"protocol":"TCP","port":5432}]}));
				}
				documents.push((
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
			documents.push(("10-policies",object("networking.k8s.io/v1","NetworkPolicy","tenant-dns",Some("kube-system"),json!({"spec":{"podSelector":{"matchLabels":{"k8s-app":"kube-dns"}},"policyTypes":["Ingress"],"ingress":[{"from":[{"namespaceSelector":{"matchLabels":{"app.kubernetes.io/managed-by":"leptoskit-cloud"}}}],"ports":[{"protocol":"UDP","port":53},{"protocol":"TCP","port":53}]}]}}))));
			for (index, (phase, mut document)) in documents.into_iter().enumerate() {
				if document["kind"] == "Cluster" {
					document["spec"]["env"] =
						json!([{"name":"AWS_DEFAULT_REGION","value":config.object_storage.region}]);
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
