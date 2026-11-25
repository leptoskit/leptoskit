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

#[test]
fn runtime_profile_and_platform_selectors_agree() {
	let config = PlatformConfig::parse(include_str!("../examples/launch.toml")).unwrap();
	let cluster = config
		.regions
		.values()
		.next()
		.unwrap()
		.clusters
		.values()
		.next()
		.unwrap();
	let profile = addons::runtime_profile(&config, cluster);
	profile.validate().unwrap();
	assert_eq!(profile.ingress_class, "traefik");
	assert_eq!(profile.ingress_namespace, "kube-system");
	assert_eq!(profile.dns_namespace, "kube-system");
	assert_eq!(
		profile.wildcard_tls_secret.as_deref(),
		Some("apps-wildcard-tls")
	);
	assert!(profile.blocked_egress_cidrs.contains(&cluster.pod_cidr));
	assert!(profile.blocked_egress_cidrs.contains(&cluster.service_cidr));
	let spec: leptoskit_cloud_runtime::ReleaseSpec = serde_json::from_value(serde_json::json!({
		"environment":{"organization":{"slug":"org","id":"00000000-0000-0000-0000-000000000001"},"project":{"slug":"app","id":"00000000-0000-0000-0000-000000000002"},"environment":{"slug":"test","id":"00000000-0000-0000-0000-000000000003"}},
		"deployment":1,"processes":[{"name":"app","kind":"app","image":{"repository":"example.invalid/app","digest":format!("sha256:{}","a".repeat(64))},"command":[],"args":[],"port":8080,"probes":{"ready":"/","live":"/"},"env":{},"secret_env":{},"resources":{"cpu_request_millis":100,"cpu_limit_millis":500,"memory_mib":128},"scale":{"min":1,"max":1,"sleep":false},"volumes":[]}],"routes":[],"policy":{"runtime_class":null,"egress":"none","quota":{"cpu_millis":1000,"memory_mib":256,"pods":2,"storage_gib":0},"database":{"namespace":"database","port":5432}}
	})).unwrap();
	let objects = leptoskit_cloud_runtime::manifests(
		&spec,
		&profile,
		&leptoskit_cloud_runtime::SecretValues::default(),
	)
	.unwrap();
	let tenant = serde_json::to_value(&objects[0]).unwrap();
	let labels = tenant["metadata"]["labels"].as_object().unwrap();
	let files = addons::generate(&config).unwrap();
	let documents: Vec<Value> = files
		.values()
		.map(|content| serde_json::from_str(content).unwrap())
		.collect();
	let policies = documents
		.iter()
		.filter(|document| document["kind"] == "NetworkPolicy");

	for policy in policies {
		for ingress in policy["spec"]["ingress"].as_array().unwrap() {
			// The peers that select tenant namespaces, by the label the runtime
			// puts on them.
			let tenant_selectors = ingress["from"]
				.as_array()
				.unwrap()
				.iter()
				.filter_map(|peer| peer["namespaceSelector"]["matchLabels"].as_object())
				.filter(|selector| selector.contains_key("app.kubernetes.io/managed-by"));

			for selector in tenant_selectors {
				assert!(
					selector
						.iter()
						.all(|(key, value)| labels.get(key) == Some(value))
				);
				assert!(
					policy["metadata"]["namespace"] != "database"
						|| ingress["ports"][0]["port"] == 5432
				);
			}
		}
	}
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
		include_str!("../examples/full-stack.toml"),
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
			if doc["kind"] == "ConfigMap" && doc["metadata"]["name"] == "required-secrets" {
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
	// External registry skips creating the registry namespace and wires
	// image_pull_secret.
	assert!(
		!docs
			.iter()
			.any(|d| d["kind"] == "Namespace" && d["metadata"]["name"] == "registry")
	);
	let cluster = config
		.regions
		.values()
		.next()
		.unwrap()
		.clusters
		.values()
		.next()
		.unwrap();
	let profile = addons::runtime_profile(&config, cluster);
	assert_eq!(
		profile.image_pull_secret.as_deref(),
		Some("pull-credentials")
	);
}

#[test]
fn cert_manager_http01_cold_s3_and_zot_ingress_opt_in() {
	use leptoskit_cloud_platform::config::ColdObjectStorage;
	use leptoskit_cloud_platform::config::CredentialRef;
	use leptoskit_cloud_platform::config::CustomDomains;
	use leptoskit_cloud_platform::config::Registry;

	let mut config = PlatformConfig::parse(include_str!("../examples/launch.toml")).unwrap();
	config.edge.custom_domains = CustomDomains::CertManager;
	config.registry = Registry::Zot {
		auth_secret: "registry-auth".into(),
		expose_ingress: true,
	};
	config.object_storage.cold = Some(ColdObjectStorage {
		endpoint: "https://s3.us-west-004.backblazeb2.com".into(),
		region: "us-west-004".into(),
		credentials: CredentialRef::KubernetesSecret {
			name: "cold-backups".into(),
			access_key: "B2_KEY_ID".into(),
			secret_key: "B2_APP_KEY".into(),
		},
	});
	let files = addons::generate(&config).unwrap();
	let docs: Vec<Value> = files
		.values()
		.map(|s| serde_json::from_str(s).unwrap())
		.collect();
	assert!(
		docs.iter()
			.any(|d| d["kind"] == "ClusterIssuer" && d["metadata"]["name"] == "apps-http01")
	);
	assert!(
		docs.iter()
			.any(|d| d["kind"] == "Ingress" && d["metadata"]["name"] == "registry")
	);
	let cnpg = docs
		.iter()
		.find(|d| d["kind"] == "Cluster" && d["metadata"]["name"] == "shared")
		.unwrap();
	assert_eq!(
		cnpg["spec"]["backup"]["barmanObjectStore"]["endpointURL"],
		"https://s3.us-west-004.backblazeb2.com"
	);
	assert_eq!(
		cnpg["spec"]["backup"]["barmanObjectStore"]["s3Credentials"]["accessKeyId"]["name"],
		"cold-backups"
	);
	assert_eq!(cnpg["spec"]["env"][0]["value"], "us-west-004");
	assert_eq!(cnpg["spec"]["enableSuperuserAccess"], true);
	assert_eq!(
		cnpg["spec"]["postgresql"]["parameters"]["max_connections"],
		"300"
	);
	let pg_hba = cnpg["spec"]["postgresql"]["pg_hba"].as_array().unwrap();
	assert!(
		pg_hba
			.iter()
			.any(|rule| rule.as_str() == Some("hostnossl all all 0.0.0.0/0 reject"))
	);

	let cluster = config
		.regions
		.values()
		.next()
		.unwrap()
		.clusters
		.values()
		.next()
		.unwrap();
	let profile = addons::runtime_profile(&config, cluster);
	assert!(!profile.edge_terminated_custom_domains);
	assert_eq!(profile.cluster_issuer.as_deref(), Some("apps-http01"));

	let tofu = leptoskit_cloud_platform::tofu::generate(&config).unwrap();
	assert!(
		!tofu.values().any(|s| s.contains("http_status:404")),
		"tunnel must forward custom domains and HTTP-01 challenges to Traefik"
	);
}

#[test]
fn growth() {
	snapshot("addons_growth", include_str!("../examples/growth.toml"));
	let config = PlatformConfig::parse(include_str!("../examples/growth.toml")).unwrap();
	let cluster = config
		.regions
		.values()
		.next()
		.unwrap()
		.clusters
		.values()
		.next()
		.unwrap();
	let profile = addons::runtime_profile(&config, cluster);
	assert!(profile.edge_terminated_custom_domains);
	assert_eq!(
		profile.tenant_tolerations[0].effect, "NoSchedule",
		"dedicated tenant pool uses NoSchedule so k3s system pods remain on platform servers"
	);
	assert_eq!(profile.cluster_issuer, None);
	assert_eq!(
		profile
			.tenant_node_selector
			.get("leptoskit.dev/pool")
			.map(String::as_str),
		Some("tenant")
	);
	assert_eq!(profile.tenant_tolerations.len(), 1);
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

#[test]
fn full_stack_emits_control_plane_workers_and_ordered_up_plan() {
	let config = PlatformConfig::parse(include_str!("../examples/full-stack.toml")).unwrap();
	let files = addons::generate(&config).unwrap();
	let docs: Vec<Value> = files
		.values()
		.map(|s| serde_json::from_str(s).unwrap())
		.collect();

	// 50-control-plane documents.
	assert!(docs.iter().any(|d| {
		d["kind"] == "Cluster"
			&& d["metadata"]["name"] == "control"
			&& d["metadata"]["namespace"] == "leptoskit-cloud-control"
	}));
	assert!(
		docs.iter()
			.any(|d| d["kind"] == "Job" && d["metadata"]["name"] == "control-migrate")
	);
	assert!(
		docs.iter()
			.any(|d| d["kind"] == "Deployment" && d["metadata"]["name"] == "control")
	);
	assert!(
		docs.iter()
			.any(|d| d["kind"] == "Ingress" && d["metadata"]["name"] == "control")
	);

	// 60-workers documents, including valid ClusterProfile ConfigMap JSON.
	assert!(docs.iter().any(|d| {
		d["kind"] == "ServiceAccount" && d["metadata"]["name"] == "leptoskit-cloud-worker"
	}));
	let profile_cm = docs
		.iter()
		.find(|d| d["kind"] == "ConfigMap" && d["metadata"]["name"] == "cluster-profile")
		.expect("cluster-profile ConfigMap");
	let parsed_profile: leptoskit_cloud_runtime::ClusterProfile =
		serde_json::from_str(profile_cm["data"]["profile.json"].as_str().unwrap())
			.expect("valid ClusterProfile JSON");
	parsed_profile.validate().unwrap();
	assert!(parsed_profile.edge_terminated_custom_domains);

	assert!(
		docs.iter()
			.any(|d| d["kind"] == "Deployment" && d["metadata"]["name"] == "cluster-worker")
	);
	assert!(
		docs.iter()
			.any(|d| d["kind"] == "Deployment" && d["metadata"]["name"] == "build-worker")
	);

	// One-click `up` dry run orders 50-control-plane before 60-workers.
	let dir = tempfile::tempdir().unwrap();
	let mut runner = leptoskit_cloud_platform::up::DryRunRunner::default();
	let options = leptoskit_cloud_platform::up::UpOptions {
		out: dir.path().to_path_buf(),
		dry_run: true,
		skip_tofu: true,
		skip_bootstrap: true,
		skip_apply: false,
		kubeconfig: None,
		ssh_user: "root".into(),
	};
	leptoskit_cloud_platform::up::execute(&config, &options, &mut runner).unwrap();
	let kubectl: Vec<String> = runner
		.commands
		.iter()
		.filter(|(p, ..)| p == "kubectl")
		.map(|(_, a, _)| a.join(" "))
		.collect();
	let pos = |needle: &str| {
		kubectl
			.iter()
			.position(|line| line.contains(needle))
			.unwrap()
	};
	assert!(pos("50-control-plane-") < pos("job/control-migrate"));
	assert!(pos("job/control-migrate") < pos("deployment/control"));
	assert!(pos("deployment/control") < pos("60-workers-"));
	assert!(pos("60-workers-") < pos("deployment/cluster-worker"));
}

#[test]
fn every_control_plane_replica_runs_the_sweeper() {
	// A Deployment hands its pod template's environment to every replica, so
	// the replica count can't choose which of them runs the sweeper. Each one
	// is told to run it, and the sweeper's advisory lock lets one sweep at a
	// time.
	let rendered: Vec<(u64, Option<String>)> = [1_u16, 2, 3]
		.into_iter()
		.map(|replicas| {
			let mut config =
				PlatformConfig::parse(include_str!("../examples/full-stack.toml")).unwrap();
			config.control_plane.as_mut().unwrap().replicas = replicas;
			let docs: Vec<Value> = addons::generate(&config)
				.unwrap()
				.values()
				.map(|content| serde_json::from_str(content).unwrap())
				.collect();
			let control = docs
				.iter()
				.find(|d| d["kind"] == "Deployment" && d["metadata"]["name"] == "control")
				.expect("the control Deployment");
			let env = control["spec"]["template"]["spec"]["containers"][0]["env"]
				.as_array()
				.unwrap();
			(
				control["spec"]["replicas"].as_u64().unwrap(),
				env.iter()
					.find(|entry| entry["name"] == "LEPTOSKIT_CLOUD_SWEEPER")
					.and_then(|entry| entry["value"].as_str())
					.map(str::to_owned),
			)
		})
		.collect();
	assert_eq!(
		rendered,
		[
			(1, Some("true".to_owned())),
			(2, Some("true".to_owned())),
			(3, Some("true".to_owned())),
		]
	);
}
