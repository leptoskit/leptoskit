//! Offline Kubernetes add-ons. JSON is a YAML 1.2 subset, so the existing
//! serializer provides escaping and stable ordering without another parser.
use serde_json::Value;
use serde_json::json;

use crate::Generated;
use crate::PlatformError;
use crate::config::Cluster;
use crate::config::CredentialRef;
use crate::config::EdgeMode;
use crate::config::PlatformConfig;
use crate::config::Pool;

/// Origin shared with the OpenTofu tunnel configuration.
pub const TUNNEL_ORIGIN: &str = "http://traefik.kube-system.svc.cluster.local:80";

fn object(api: &str, kind: &str, name: &str, namespace: Option<&str>, body: Value) -> Value {
	let mut doc = body;
	doc["apiVersion"] = json!(api);
	doc["kind"] = json!(kind);
	doc["metadata"] = json!({"name":name});
	if let Some(ns) = namespace {
		doc["metadata"]["namespace"] = json!(ns);
	}
	doc
}

fn placement(cluster: &Cluster, desired: Pool) -> Value {
	// The launch box has only a tenant label. Hard selectors must name a
	// pool that exists. Build nodes remain exclusive.
	let pool = if cluster.nodes.values().any(|n| n.pool() == desired) {
		desired
	} else if cluster.nodes.values().any(|n| n.pool() == Pool::Platform) {
		Pool::Platform
	} else {
		Pool::Tenant
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
	Ok(object(
		"helm.cattle.io/v1",
		"HelmChart",
		name,
		Some("kube-system"),
		json!({"spec":{"repo":repo,"chart":package,"version":version,"targetNamespace":ns,"valuesContent":serde_json::to_string_pretty(values)?}}),
	))
}

/// Generate independent, ordered YAML documents per cluster without resolving
/// secrets or contacting chart repositories.
pub fn generate(config: &PlatformConfig) -> Result<Generated, PlatformError> {
	config.validate()?;
	let mut files = Generated::new();
	for (region, entry) in &config.regions {
		for (name, cluster) in &entry.clusters {
			let mut documents = Vec::new();
			for ns in [
				"kube-system",
				"cnpg-system",
				"database",
				"keda",
				"cert-manager",
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
			let platform = placement(cluster, Pool::Platform);
			// Values verified in the official versioned chart sources listed in
			// the runbook. Recheck these sources whenever a pin changes.
			documents.push((
				"20-charts",
				chart(
					"cloudnative-pg",
					"cnpg-system",
					"https://cloudnative-pg.github.io/charts",
					"cloudnative-pg",
					&config.addons.cloudnative_pg,
					&platform,
				)?,
			));
			let mut keda = platform.clone();
			for component in ["operator", "metricsServer", "webhooks"] {
				keda[component] = platform.clone();
			}
			documents.push((
				"20-charts",
				chart(
					"keda",
					"keda",
					"https://kedacore.github.io/charts",
					"keda",
					&config.addons.keda,
					&keda,
				)?,
			));
			let mut http = json!({});
			for component in ["operator", "scaler", "interceptor"] {
				http[component] = platform.clone();
			}
			http["interceptor"]["directPodRouting"] = json!(false);
			documents.push((
				"21-http",
				chart(
					"http",
					"keda",
					"https://kedacore.github.io/charts",
					"keda-add-ons-http",
					&config.addons.keda_http,
					&http,
				)?,
			));
			let mut cert = platform.clone();
			cert["crds"] = json!({"enabled":true});
			for component in ["webhook", "cainjector", "startupapicheck"] {
				cert[component] = platform.clone();
			}
			documents.push((
				"20-charts",
				chart(
					"cert-manager",
					"cert-manager",
					"https://charts.jetstack.io",
					"cert-manager",
					&config.addons.cert_manager,
					&cert,
				)?,
			));
			if region == &config.default_region && name == &config.edge.cluster {
				let mut traefik = placement(cluster, Pool::Ingress);
				traefik["fullnameOverride"] = json!("traefik");
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
			for (index, (phase, document)) in documents.into_iter().enumerate() {
				files.insert(
					format!("{region}/{name}/addons/{phase}-{index:02}.yaml"),
					format!("{}\n", serde_json::to_string_pretty(&document)?),
				);
			}
		}
	}
	Ok(files)
}
