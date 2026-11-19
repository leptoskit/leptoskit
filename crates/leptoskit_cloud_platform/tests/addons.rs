use std::collections::BTreeSet;

use leptoskit_cloud_platform::addons;
use leptoskit_cloud_platform::config::PlatformConfig;
use serde_json::Value;

fn snapshot(name: &str, input: &str) {
	let config = PlatformConfig::parse(input).unwrap();
	let files = addons::generate(&config).unwrap();
	assert_eq!(files, addons::generate(&config).unwrap());
	insta::assert_snapshot!(
		name,
		files
			.iter()
			.map(|(path, content)| format!("# {path}\n{content}"))
			.collect::<Vec<_>>()
			.join("---\n")
	);
}

#[test]
fn launch() {
	snapshot("addons_launch", include_str!("../examples/launch.toml"));
}

fn refs(value: &Value, namespace: &str, found: &mut BTreeSet<(String, String)>) {
	match value {
		Value::Object(map) => {
			for (key, value) in map {
				if [
					"secretKeyRef",
					"apiTokenSecretRef",
					"privateKeySecretRef",
					"accessKeyId",
					"secretAccessKey",
				]
				.contains(&key.as_str())
				{
					found.insert((namespace.into(), value["name"].as_str().unwrap().into()));
				} else if key == "secretName" {
					found.insert((namespace.into(), value.as_str().unwrap().into()));
				}
				refs(value, namespace, found);
			}
		}
		Value::Array(items) => {
			for item in items {
				refs(item, namespace, found);
			}
		}
		_ => {}
	}
}

#[test]
fn documents_and_secret_inventory_agree() {
	for input in [
		include_str!("../examples/launch.toml"),
		include_str!("../examples/growth.toml"),
	] {
		let config = PlatformConfig::parse(input).unwrap();
		let files = addons::generate(&config).unwrap();
		let docs: Vec<Value> = files
			.values()
			.map(|s| serde_json::from_str(s).unwrap())
			.collect();
		let namespaces: BTreeSet<_> = docs
			.iter()
			.filter(|d| d["kind"] == "Namespace")
			.map(|d| d["metadata"]["name"].as_str().unwrap())
			.collect();
		let mut found = BTreeSet::new();
		let mut listed = BTreeSet::new();
		for doc in &docs {
			for pointer in ["/apiVersion", "/kind", "/metadata/name"] {
				assert!(doc.pointer(pointer).unwrap().as_str().is_some());
			}
			let ns = doc["metadata"]["namespace"]
				.as_str()
				.unwrap_or("cert-manager");
			if doc["metadata"].get("namespace").is_some() {
				assert!(namespaces.contains(ns));
			}
			if doc["kind"] == "ConfigMap" {
				let entries: Vec<Value> =
					serde_json::from_str(doc["data"]["requirements.json"].as_str().unwrap())
						.unwrap();
				for entry in entries {
					assert!(listed.insert((
						entry["namespace"].as_str().unwrap().to_string(),
						entry["name"].as_str().unwrap().to_string()
					)));
				}
			} else {
				refs(doc, ns, &mut found);
			}
			if doc["kind"] == "HelmChart" {
				let pins = serde_json::to_value(&config.addons).unwrap();
				assert!(
					pins.as_object()
						.unwrap()
						.values()
						.any(|pin| pin == &doc["spec"]["version"])
				);
				assert!(namespaces.contains(doc["spec"]["targetNamespace"].as_str().unwrap()));
				let values: Value =
					serde_json::from_str(doc["spec"]["valuesContent"].as_str().unwrap()).unwrap();
				refs(
					&values,
					doc["spec"]["targetNamespace"].as_str().unwrap(),
					&mut found,
				);
			}
		}
		assert_eq!(found, listed);
		let tofu = leptoskit_cloud_platform::tofu::generate(&config).unwrap();
		assert!(tofu.values().any(|s| s.contains(addons::TUNNEL_ORIGIN)));
		let chart = docs
			.iter()
			.find(|d| d["kind"] == "HelmChart" && d["metadata"]["name"] == "traefik")
			.unwrap();
		assert_eq!(chart["spec"]["targetNamespace"], "kube-system");
		let values: Value =
			serde_json::from_str(chart["spec"]["valuesContent"].as_str().unwrap()).unwrap();
		assert_eq!(values["fullnameOverride"], "traefik");
		for content in files.values() {
			for forbidden in [
				"secret-sentinel",
				"BEGIN PRIVATE KEY",
				"AKIA",
				"stringData",
				"password:",
			] {
				assert!(!content.contains(forbidden));
			}
		}
	}
}

#[test]
fn invalid_input_is_refused() {
	let mut config = PlatformConfig::parse(include_str!("../examples/launch.toml")).unwrap();
	config.addons.keda = "latest".into();
	assert!(addons::generate(&config).is_err());
}

#[test]
fn https_external_registry_and_environment_credentials() {
	use leptoskit_cloud_platform::config::CLOUDFLARE_CIDRS;
	use leptoskit_cloud_platform::config::CredentialRef;
	use leptoskit_cloud_platform::config::EdgeMode;
	use leptoskit_cloud_platform::config::Node;
	use leptoskit_cloud_platform::config::Pool;
	use leptoskit_cloud_platform::config::Registry;
	let mut config = PlatformConfig::parse(include_str!("../examples/launch.toml")).unwrap();
	config.edge.mode = EdgeMode::Https;
	config.edge.cloudflare_cidrs = CLOUDFLARE_CIDRS.iter().map(|s| (*s).into()).collect();
	if let Node::Existing { pool, .. } = config
		.regions
		.get_mut("eu")
		.unwrap()
		.clusters
		.get_mut("launch")
		.unwrap()
		.nodes
		.get_mut("box")
		.unwrap()
	{
		*pool = Pool::Ingress;
	}
	config.registry = Registry::External {
		host: "registry.example.com".into(),
		pull_secret: "pull-credentials".into(),
	};
	config.object_storage.credentials = CredentialRef::Environment {
		access_key: "S3_ACCESS_KEY".into(),
		secret_key: "S3_SECRET_KEY".into(),
	};
	let files = addons::generate(&config).unwrap();
	let docs: Vec<Value> = files
		.values()
		.map(|s| serde_json::from_str(s).unwrap())
		.collect();
	assert!(
		!docs
			.iter()
			.any(|d| d["kind"] == "Deployment" || d["metadata"]["name"] == "zot")
	);
	assert!(
		docs.iter()
			.any(|d| d["metadata"]["name"] == "cloudflare-https")
	);
	let traefik = docs
		.iter()
		.find(|d| d["kind"] == "HelmChart" && d["metadata"]["name"] == "traefik")
		.unwrap();
	let values: Value =
		serde_json::from_str(traefik["spec"]["valuesContent"].as_str().unwrap()).unwrap();
	assert_eq!(values["deployment"]["kind"], "DaemonSet");
	assert_eq!(values["ports"]["websecure"]["hostPort"], 443);
	assert_eq!(values["nodeSelector"]["leptoskit.dev/pool"], "ingress");
}

#[test]
fn growth() {
	snapshot("addons_growth", include_str!("../examples/growth.toml"));
	let config = PlatformConfig::parse(include_str!("../examples/growth.toml")).unwrap();
	for content in addons::generate(&config).unwrap().values() {
		let doc: Value = serde_json::from_str(content).unwrap();
		if let Some(values) = doc["spec"]["valuesContent"].as_str() {
			let values: Value = serde_json::from_str(values).unwrap();
			if let Some(selector) = values.get("nodeSelector") {
				assert_eq!(selector["leptoskit.dev/pool"], "platform");
			}
		}
	}
}
