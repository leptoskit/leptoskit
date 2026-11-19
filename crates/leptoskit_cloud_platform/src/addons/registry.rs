//! Zot uses the distribution S3 driver and its AWS credential provider chain.
//! Chart keys verified in project-zot/helm-charts tag zot-0.1.134.
use serde_json::Value;
use serde_json::json;

use super::Documents;
use super::chart;
use super::object;
use crate::PlatformError;
use crate::config::CredentialRef;
use crate::config::PlatformConfig;
use crate::config::Registry;

pub(super) fn credentials(config: &PlatformConfig) -> (&str, &str, &str) {
	match &config.object_storage.credentials {
		CredentialRef::KubernetesSecret {
			name,
			access_key,
			secret_key,
		} => (name, access_key, secret_key),
		CredentialRef::Environment { .. } => {
			("object-storage", "ACCESS_KEY_ID", "SECRET_ACCESS_KEY")
		}
	}
}

pub(super) fn documents(
	config: &PlatformConfig,
	platform: &Value,
	edge: bool,
) -> Result<Documents, PlatformError> {
	let Registry::Zot { auth_secret } = &config.registry else {
		return Ok(Vec::new());
	};
	if !edge {
		return Ok(Vec::new());
	}
	let (secret, access, key) = credentials(config);
	let host = format!("registry.{}", config.apps_domain);
	// A persistent BoltDB cache is needed even with remote blob storage.
	// AWS_ACCESS_* are consumed by the distribution S3 driver provider chain;
	// htpasswd protects both reads and writes, with no anonymous policy.
	let configuration = json!({"distSpecVersion":"1.1.1","storage":{"rootDirectory":"/var/lib/registry","dedupe":false,"storageDriver":{"name":"s3","region":config.object_storage.region,"regionendpoint":config.object_storage.endpoint,"bucket":config.object_storage.registry,"secure":true,"skipverify":false,"forcepathstyle":true}},"http":{"address":"0.0.0.0","port":"5000","auth":{"htpasswd":{"path":"/auth/htpasswd"}}},"log":{"level":"info"}});
	let mut values = platform.clone();
	values["fullnameOverride"] = json!("zot");
	values["service"] = json!({"type":"ClusterIP","port":5000});
	values["mountConfig"] = json!(true);
	values["configFiles"] = json!({"config.json":serde_json::to_string_pretty(&configuration)?});
	values["externalSecrets"] = json!([{"secretName":auth_secret,"mountPath":"/auth"}]);
	values["persistence"] = json!(true);
	values["pvc"] = json!({"storageClassName":config.postgres.storage_class});
	values["env"] = json!([
		{"name":"AWS_ACCESS_KEY_ID","valueFrom":{"secretKeyRef":{"name":secret,"key":access}}},
		{"name":"AWS_SECRET_ACCESS_KEY","valueFrom":{"secretKeyRef":{"name":secret,"key":key}}}
	]);
	Ok(vec![
		(
			"20-charts",
			chart(
				"zot",
				"registry",
				"https://zotregistry.dev/helm-charts",
				"zot",
				&config.addons.zot,
				&values,
			)?,
		),
		(
			"30-registry",
			object(
				"cert-manager.io/v1",
				"Certificate",
				"registry",
				Some("registry"),
				json!({"spec":{"secretName":"registry-tls","issuerRef":{"name":"apps","kind":"ClusterIssuer"},"dnsNames":[host]}}),
			),
		),
		(
			"30-registry",
			object(
				"networking.k8s.io/v1",
				"Ingress",
				"registry",
				Some("registry"),
				json!({"spec":{"ingressClassName":"traefik","tls":[{"hosts":[host],"secretName":"registry-tls"}],"rules":[{"host":host,"http":{"paths":[{"path":"/","pathType":"Prefix","backend":{"service":{"name":"zot","port":{"number":5000}}}}]}}]}}),
			),
		),
	])
}
