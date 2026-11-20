//! The manifests a release builds, snapshot-tested, and the refusals a
//! plain cluster cannot get past.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::process::Command;
use std::process::Stdio;

use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1 as core;
use leptoskit_cloud_runtime::ClusterProfile;
use leptoskit_cloud_runtime::DatabaseAccess;
use leptoskit_cloud_runtime::Egress;
use leptoskit_cloud_runtime::EnvironmentRef;
use leptoskit_cloud_runtime::Image;
use leptoskit_cloud_runtime::ManifestError;
use leptoskit_cloud_runtime::NamedRef;
use leptoskit_cloud_runtime::Object;
use leptoskit_cloud_runtime::Policy;
use leptoskit_cloud_runtime::Probes;
use leptoskit_cloud_runtime::Process;
use leptoskit_cloud_runtime::ProcessKind;
use leptoskit_cloud_runtime::Quota;
use leptoskit_cloud_runtime::ReleaseSpec;
use leptoskit_cloud_runtime::Resources;
use leptoskit_cloud_runtime::Route;
use leptoskit_cloud_runtime::Scale;
use leptoskit_cloud_runtime::SecretValues;
use leptoskit_cloud_runtime::Tls;
use leptoskit_cloud_runtime::Volume;
use leptoskit_cloud_runtime::kubernetes::HttpAddon;
use leptoskit_cloud_runtime::kubernetes::ScaledObject;
use leptoskit_cloud_runtime::manifests;

/// The digest of the word `test`, in the form a release names.
const DIGEST: &str = "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

fn id(number: u64) -> uuid::Uuid {
	format!("00000000-0000-0000-0000-{number:012x}")
		.parse()
		.expect("a fixture id")
}

fn named(slug: &str, number: u64) -> NamedRef {
	NamedRef {
		slug: slug.to_owned(),
		id: id(number),
	}
}

fn environment() -> EnvironmentRef {
	EnvironmentRef {
		organization: named("acme", 1),
		project: named("notes", 2),
		environment: named("production", 3),
	}
}

fn image(repository: &str) -> Image {
	Image {
		repository: repository.to_owned(),
		digest: DIGEST.to_owned(),
	}
}

/// Size xs: the hobby app's share.
fn resources_xs() -> Resources {
	Resources {
		cpu_request_millis: 100,
		cpu_limit_millis: 500,
		memory_mib: 128,
	}
}

/// Size s.
fn resources_s() -> Resources {
	Resources {
		cpu_request_millis: 250,
		cpu_limit_millis: 1000,
		memory_mib: 256,
	}
}

/// Size m.
fn resources_m() -> Resources {
	Resources {
		cpu_request_millis: 500,
		cpu_limit_millis: 2000,
		memory_mib: 512,
	}
}

/// A cluster with the edge, the wildcard, and an issuer: what the cloud's
/// own clusters look like.
fn cluster() -> ClusterProfile {
	ClusterProfile {
		ingress_class: "traefik".to_owned(),
		ingress_namespace: "kube-system".to_owned(),
		wildcard_tls_secret: Some("apps-wildcard-tls".to_owned()),
		cluster_issuer: Some("apps".to_owned()),
		dns_namespace: "kube-system".to_owned(),
		blocked_egress_cidrs: vec![
			"10.0.0.0/8".to_owned(),
			"169.254.0.0/16".to_owned(),
			"192.168.0.0/16".to_owned(),
		],
		image_pull_secret: None,
		edge_terminated_custom_domains: false,
		keda_http: Some(HttpAddon {
			namespace: "keda".to_owned(),
			interceptor_service: "keda-add-ons-http-interceptor-proxy".to_owned(),
			interceptor_port: 8080,
			scaler_service: "keda-add-ons-http-external-scaler".to_owned(),
			scaler_port: 9090,
			interceptor_labels: BTreeMap::from([
				(
					"app.kubernetes.io/component".to_owned(),
					"interceptor".to_owned(),
				),
				(
					"app.kubernetes.io/name".to_owned(),
					"http-add-on".to_owned(),
				),
			]),
		}),
		tenant_node_selector: BTreeMap::new(),
		tenant_tolerations: Vec::new(),
		tmp_size_limit_mib: leptoskit_cloud_runtime::kubernetes::DEFAULT_TMP_SIZE_LIMIT_MIB,
	}
}

/// A minimal app: size xs, sleeping when idle, the shared wildcard
/// certificate, KEDA's HTTP add-on on the cluster.
fn minimal_sleeping_app() -> (ReleaseSpec, ClusterProfile, SecretValues) {
	let spec = ReleaseSpec {
		environment: environment(),
		deployment: 1,
		processes: vec![Process {
			name: "app".to_owned(),
			kind: ProcessKind::App,
			image: image("ghcr.io/acme/notes"),
			command: vec!["/app/server".to_owned()],
			args: vec![],
			port: 8080,
			probes: Probes {
				ready: "/readyz".to_owned(),
				live: "/healthz".to_owned(),
			},
			env: BTreeMap::from([("RUST_LOG".to_owned(), "info".to_owned())]),
			secret_env: BTreeMap::new(),
			resources: resources_xs(),
			scale: Scale {
				min: 0,
				max: 1,
				sleep: true,
			},
			volumes: vec![],
		}],
		routes: vec![Route {
			hostname: "notes.leptoskit.app".to_owned(),
			path_prefix: "/".to_owned(),
			process: "app".to_owned(),
			strip_cookies: false,
			tls: Tls::Wildcard,
		}],
		policy: Policy {
			runtime_class: Some("gvisor".to_owned()),
			egress: Egress::Internet,
			quota: Quota {
				cpu_millis: 1000,
				memory_mib: 256,
				pods: 2,
				storage_gib: 1,
			},
			database: None,
		},
	};
	(spec, cluster(), SecretValues::default())
}

/// An always-on app with two services: a public search service on a custom
/// domain cert-manager issues for, and a private file service with a
/// volume. The app autoscales from two to five.
fn app_with_services() -> (ReleaseSpec, ClusterProfile, SecretValues) {
	let spec = ReleaseSpec {
		environment: environment(),
		deployment: 2,
		processes: vec![
			Process {
				name: "app".to_owned(),
				kind: ProcessKind::App,
				image: image("ghcr.io/acme/notes"),
				command: vec!["/app/server".to_owned()],
				args: vec![],
				port: 8080,
				probes: Probes {
					ready: "/readyz".to_owned(),
					live: "/healthz".to_owned(),
				},
				env: BTreeMap::new(),
				secret_env: BTreeMap::new(),
				resources: resources_s(),
				scale: Scale {
					min: 2,
					max: 5,
					sleep: false,
				},
				volumes: vec![],
			},
			Process {
				name: "search".to_owned(),
				kind: ProcessKind::Service,
				image: image("ghcr.io/acme/search"),
				command: vec!["/app/services/search".to_owned()],
				args: vec!["--port".to_owned(), "3110".to_owned()],
				port: 3110,
				probes: Probes {
					ready: "/readyz".to_owned(),
					live: "/healthz".to_owned(),
				},
				env: BTreeMap::from([("SEARCH_MODE".to_owned(), "full".to_owned())]),
				secret_env: BTreeMap::from([(
					"SEARCH_TOKEN".to_owned(),
					"search-token".to_owned(),
				)]),
				resources: resources_m(),
				scale: Scale {
					min: 1,
					max: 2,
					sleep: false,
				},
				volumes: vec![],
			},
			Process {
				name: "files".to_owned(),
				kind: ProcessKind::Service,
				image: image("ghcr.io/acme/files"),
				command: vec!["/app/services/files".to_owned()],
				args: vec![],
				port: 3111,
				probes: Probes {
					ready: "/readyz".to_owned(),
					live: "/healthz".to_owned(),
				},
				env: BTreeMap::new(),
				secret_env: BTreeMap::new(),
				resources: resources_m(),
				scale: Scale {
					min: 1,
					max: 1,
					sleep: false,
				},
				volumes: vec![Volume {
					name: "data".to_owned(),
					mount_path: "/srv/files".to_owned(),
					size_gib: 5,
				}],
			},
		],
		routes: vec![
			Route {
				hostname: "notes.acme.dev".to_owned(),
				path_prefix: "/".to_owned(),
				process: "app".to_owned(),
				strip_cookies: false,
				tls: Tls::Wildcard,
			},
			Route {
				hostname: "notes.acme.dev".to_owned(),
				path_prefix: "/docs".to_owned(),
				process: "app".to_owned(),
				strip_cookies: false,
				tls: Tls::Wildcard,
			},
			Route {
				hostname: "search.acme.dev".to_owned(),
				path_prefix: "/_leptoskit/services/search/".to_owned(),
				process: "search".to_owned(),
				strip_cookies: false,
				tls: Tls::Issued,
			},
		],
		policy: Policy {
			runtime_class: Some("gvisor".to_owned()),
			egress: Egress::Internet,
			quota: Quota {
				cpu_millis: 14000,
				memory_mib: 4096,
				pods: 12,
				storage_gib: 10,
			},
			database: Some(DatabaseAccess {
				namespace: "database".to_owned(),
				port: 5432,
			}),
		},
	};
	let secrets = SecretValues::new(BTreeMap::from([(
		"search-token".to_owned(),
		"hunter2-the-value".to_owned(),
	)]));
	(spec, cluster(), secrets)
}

/// The whole release as one multi-document YAML string, exactly as an
/// apply would read it.
fn as_yaml(objects: &[Object]) -> String {
	let mut out = String::new();
	for (index, object) in objects.iter().enumerate() {
		if index > 0 {
			out.push_str("---\n");
		}
		out.push_str(&object.to_yaml().expect("the fixture serializes"));
	}
	out
}

fn build(spec: &ReleaseSpec, cluster: &ClusterProfile, secrets: &SecretValues) -> Vec<Object> {
	manifests(spec, cluster, secrets).expect("the fixture manifests")
}

#[test]
fn a_minimal_sleeping_app() {
	let (spec, cluster, secrets) = minimal_sleeping_app();
	let objects = build(&spec, &cluster, &secrets);

	let kinds: Vec<&str> = objects.iter().map(Object::kind).collect();
	assert_eq!(
		kinds,
		vec![
			"Namespace",
			"ResourceQuota",
			"LimitRange",
			"NetworkPolicy",
			"NetworkPolicy",
			"NetworkPolicy",
			"NetworkPolicy",
			"NetworkPolicy",
			"ServiceAccount",
			"Deployment",
			"Service",
			"Service",
			"InterceptorRoute",
			"ScaledObject",
			"Ingress",
		]
	);
	insta::assert_snapshot!(as_yaml(&objects));
}

#[test]
fn one_xs_app_has_room_for_its_rolling_surge() {
	let (mut spec, cluster, secrets) = minimal_sleeping_app();
	spec.processes[0].scale.min = 1;
	spec.processes[0].scale.sleep = false;
	spec.policy.quota = Quota::for_processes(&spec.processes);
	let objects = build(&spec, &cluster, &secrets);
	let quota = objects
		.iter()
		.find_map(|object| {
			match object {
				Object::ResourceQuota(quota) => quota.spec.as_ref()?.hard.as_ref(),
				_ => None,
			}
		})
		.expect("quota");
	assert_eq!(quota["pods"].0, "2");
	assert_eq!(quota["limits.cpu"].0, "1000m");
	assert_eq!(quota["limits.memory"].0, "256Mi");
	let deployment = objects
		.iter()
		.find_map(|object| {
			match object {
				Object::Deployment(deployment) => deployment.spec.as_ref(),
				_ => None,
			}
		})
		.expect("deployment");
	let rolling = deployment
		.strategy
		.as_ref()
		.and_then(|strategy| strategy.rolling_update.as_ref())
		.expect("rolling update");
	assert_eq!(
		rolling.max_surge,
		Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(1))
	);
}

#[test]
fn an_app_with_services() {
	let (spec, cluster, secrets) = app_with_services();
	let objects = build(&spec, &cluster, &secrets);
	assert_eq!(
		objects
			.iter()
			.filter(|o| o.kind() == "NetworkPolicy")
			.count(),
		7
	);
	assert_eq!(
		objects.iter().filter(|o| o.kind() == "Deployment").count(),
		3
	);
	assert_eq!(
		objects
			.iter()
			.filter(|o| o.kind() == "HorizontalPodAutoscaler")
			.count(),
		2
	);
	assert_eq!(objects.iter().filter(|o| o.kind() == "Ingress").count(), 2);
	insta::assert_snapshot!(as_yaml(&objects));
}

#[test]
fn the_refusals() {
	let (spec, mut cluster, secrets) = minimal_sleeping_app();

	// Sleep on a cluster without the HTTP add-on.
	cluster.keda_http = None;
	let error = manifests(&spec, &cluster, &secrets).expect_err("no add-on");
	assert!(matches!(error, ManifestError::SleepWithoutScaler { ref process } if process == "app"));
	assert!(error.to_string().contains("cannot sleep"), "{}", error);

	// Cookies the plain Ingress cannot strip.
	let (spec, cluster, secrets) = minimal_sleeping_app();
	let mut stripped = spec.clone();
	stripped.routes[0].strip_cookies = true;
	let error = manifests(&stripped, &cluster, &secrets).expect_err("strips cookies");
	assert!(
		matches!(error, ManifestError::StripCookies { .. }),
		"{error}"
	);
	assert!(error.to_string().contains("Ingress"), "{}", error);

	// A secret key with no value.
	let mut missing = spec.clone();
	missing.processes[0]
		.secret_env
		.insert("DATABASE_URL".to_owned(), "db-url".to_owned());
	let error = manifests(&missing, &cluster, &secrets).expect_err("no value");
	assert!(
		matches!(error, ManifestError::MissingSecret { ref key, .. } if key == "db-url"),
		"{error}"
	);

	// A custom domain with no issuer on the cluster.
	let mut no_issuer = cluster.clone();
	no_issuer.cluster_issuer = None;
	let mut issued = spec.clone();
	issued.routes[0].tls = Tls::Issued;
	let error = manifests(&issued, &no_issuer, &secrets).expect_err("no issuer");
	assert!(matches!(error, ManifestError::NoIssuer { .. }), "{error}");

	// The wildcard with no wildcard secret.
	let mut no_wildcard = cluster.clone();
	no_wildcard.wildcard_tls_secret = None;
	let error = manifests(&spec, &no_wildcard, &secrets).expect_err("no wildcard");
	assert!(
		matches!(error, ManifestError::NoWildcardSecret { .. }),
		"{error}"
	);

	// One host asked for both kinds of certificate.
	let mut mixed = spec.clone();
	mixed.routes.push(Route {
		hostname: "notes.leptoskit.app".to_owned(),
		path_prefix: "/docs".to_owned(),
		process: "app".to_owned(),
		strip_cookies: false,
		tls: Tls::Issued,
	});
	let error = manifests(&mixed, &cluster, &secrets).expect_err("mixed tls");
	assert!(matches!(error, ManifestError::MixedTls { .. }), "{error}");

	// An invalid spec is refused before any object is built.
	let mut invalid = spec;
	invalid.processes[0].port = 0;
	let error = manifests(&invalid, &cluster, &secrets).expect_err("invalid");
	assert!(matches!(error, ManifestError::Invalid(_)), "{error}");
}

/// A handful of generated specs, each built twice: the bytes agree, the
/// namespacing is right, the pods are hardened, and no secret value leaves
/// the Secret's data.
#[test]
fn every_spec_builds_the_same_manifests_the_same_way() {
	let cases: Vec<(
		String,
		ReleaseSpec,
		ClusterProfile,
		SecretValues,
		Vec<String>,
	)> = vec![
		{
			let (spec, cluster, secrets) = minimal_sleeping_app();
			("sleeping".to_owned(), spec, cluster, secrets, vec![])
		},
		{
			let (spec, cluster, secrets) = app_with_services();
			(
				"services".to_owned(),
				spec,
				cluster,
				secrets,
				vec!["hunter2-the-value".to_owned()],
			)
		},
		{
			// No routes, no internet, a database, one fixed replica.
			let (mut spec, cluster, secrets) = minimal_sleeping_app();
			spec.routes.clear();
			spec.processes[0].scale = Scale {
				min: 1,
				max: 1,
				sleep: false,
			};
			spec.policy.egress = Egress::None;
			spec.policy.database = Some(DatabaseAccess {
				namespace: "database".to_owned(),
				port: 5432,
			});
			spec.policy.quota = Quota::for_processes(&spec.processes);
			("private".to_owned(), spec, cluster, secrets, vec![])
		},
		{
			// A secret, a volume mounted where the temporary one would go,
			// an issued certificate, and no add-on to sleep with.
			let (mut spec, mut cluster, _) = minimal_sleeping_app();
			spec.processes[0].secret_env =
				BTreeMap::from([("DATABASE_URL".to_owned(), "db-url".to_owned())]);
			spec.processes[0].volumes = vec![Volume {
				name: "data".to_owned(),
				mount_path: "/tmp".to_owned(),
				size_gib: 1,
			}];
			spec.processes[0].scale = Scale {
				min: 1,
				max: 1,
				sleep: false,
			};
			spec.routes[0].tls = Tls::Issued;
			cluster.keda_http = None;
			spec.policy.quota = Quota {
				cpu_millis: 500,
				memory_mib: 128,
				pods: 2,
				storage_gib: 2,
			};
			let secrets = SecretValues::new(BTreeMap::from([(
				"db-url".to_owned(),
				"postgres://test:test@db/test".to_owned(),
			)]));
			(
				"stateful".to_owned(),
				spec,
				cluster,
				secrets,
				vec!["postgres://test:test@db/test".to_owned()],
			)
		},
	];

	for (name, spec, cluster, secrets, values) in cases {
		let first = build(&spec, &cluster, &secrets);
		let second = build(&spec, &cluster, &secrets);
		assert_eq!(
			as_yaml(&first),
			as_yaml(&second),
			"{name} builds the same bytes twice"
		);

		let namespace = spec.environment.namespace();
		for object in &first {
			if object.kind() == "Namespace" {
				assert_eq!(object.name(), Some(namespace.as_str()), "{name}");
				assert_eq!(object.namespace(), None, "{name}");
			} else {
				assert_eq!(
					object.namespace(),
					Some(namespace.as_str()),
					"{name}: {}",
					object.kind()
				);
			}
		}
		assert!(pods_are_hardened(&first), "{name}: a pod is not hardened");
		assert_secrets_stay_in_data(&first, &values, &secrets, &name);
	}
}

fn pods_are_hardened(objects: &[Object]) -> bool {
	objects.iter().all(|object| {
		match object {
			Object::Deployment(deployment) => {
				deployment
					.spec
					.as_ref()
					.map(|spec| &spec.template)
					.and_then(|template| template.spec.as_ref())
					.is_some_and(pod_is_hardened)
			}
			_ => true,
		}
	})
}

fn pod_is_hardened(pod: &core::PodSpec) -> bool {
	let pod_hardened = pod.security_context.as_ref().is_some_and(|security| {
		security.run_as_non_root == Some(true)
			&& security
				.seccomp_profile
				.as_ref()
				.is_some_and(|profile| profile.type_ == "RuntimeDefault")
	});
	let containers_hardened = pod.containers.iter().all(container_security_is_hardened);
	pod_hardened && containers_hardened && pod.automount_service_account_token == Some(false)
}

fn container_security_is_hardened(container: &core::Container) -> bool {
	let Some(context) = container.security_context.as_ref() else {
		return false;
	};
	let drops_all = context
		.capabilities
		.as_ref()
		.and_then(|capabilities| capabilities.drop.as_ref())
		.is_some_and(|drop| drop.iter().any(|capability| capability == "ALL"));
	context.allow_privilege_escalation == Some(false)
		&& context.read_only_root_filesystem == Some(true)
		&& drops_all
}

fn assert_secrets_stay_in_data(
	objects: &[Object],
	values: &[String],
	secrets: &SecretValues,
	name: &str,
) {
	let printed = format!("{secrets:?}");
	for value in values {
		assert!(
			!printed.contains(value.as_str()),
			"{name}: Debug prints a value"
		);
	}
	for object in objects {
		let yaml = as_yaml(std::slice::from_ref(object));
		for value in values {
			assert!(
				!yaml.contains(value.as_str()),
				"{name}: {} carries a secret value",
				object.kind()
			);
		}
		if let Object::Secret(secret) = object {
			let data = secret.data.as_ref().expect("the secret has data");
			for value in values {
				assert!(
					data.values()
						.any(|ByteString(bytes)| bytes.as_slice() == value.as_bytes()),
					"{name}: the secret's data is missing the value"
				);
			}
		}
	}
}

/// A sleeping process's scaled object names the hosts that wake it.
#[test]
fn the_sleeper_wakes_on_its_routes() {
	let (spec, cluster, secrets) = minimal_sleeping_app();
	let objects = build(&spec, &cluster, &secrets);
	let keda = objects
		.iter()
		.find_map(|object| {
			match object {
				Object::ScaledObject(ScaledObject { spec: scaled, .. }) => Some(scaled),
				_ => None,
			}
		})
		.expect("the sleeper scales from zero");
	assert_eq!(keda.replicas_min, 0);
	assert_eq!(keda.replicas_max, 1);
	let trigger = &keda.triggers[0];
	assert_eq!(
		trigger.metadata.get("interceptorRoute").map(String::as_str),
		Some("app")
	);
	assert_eq!(trigger.kind, "external-push");
	let route = objects
		.iter()
		.find_map(|object| {
			match object {
				Object::InterceptorRoute(route) => Some(route),
				_ => None,
			}
		})
		.unwrap();
	assert_eq!(route.api_version, "http.keda.sh/v1beta1");
	assert_eq!(route.spec.target.service, "app");
	assert_eq!(route.spec.target.port, 80);
	assert_eq!(route.spec.rules[0].hosts, ["notes.leptoskit.app"]);
	assert_eq!(route.spec.rules[0].paths[0].value, "/");
	let ingress = objects
		.iter()
		.find_map(|object| {
			match object {
				Object::Ingress(ingress) => ingress.spec.as_ref(),
				_ => None,
			}
		})
		.unwrap();
	let backend = ingress.rules.as_ref().unwrap()[0]
		.http
		.as_ref()
		.unwrap()
		.paths[0]
		.backend
		.service
		.as_ref()
		.unwrap();
	assert_eq!(backend.name, "http-interceptor");
	assert_eq!(backend.port.as_ref().unwrap().number, Some(8080));
	let external = objects
		.iter()
		.find_map(|object| {
			match object {
				Object::Service(service)
					if service.metadata.name.as_deref() == Some("http-interceptor") =>
				{
					service.spec.as_ref()
				}
				_ => None,
			}
		})
		.unwrap();
	assert_eq!(external.type_.as_deref(), Some("ExternalName"));
	assert_eq!(
		external.external_name.as_deref(),
		Some("keda-add-ons-http-interceptor-proxy.keda.svc.cluster.local")
	);
	let policy = objects
		.iter()
		.find_map(|object| {
			match object {
				Object::NetworkPolicy(policy)
					if policy.metadata.name.as_deref() == Some("allow-from-ingress-app") =>
				{
					policy.spec.as_ref()
				}
				_ => None,
			}
		})
		.unwrap();
	let peers = policy.ingress.as_ref().unwrap()[0].from.as_ref().unwrap();
	assert_eq!(peers.len(), 1);
	assert_eq!(
		peers[0]
			.namespace_selector
			.as_ref()
			.unwrap()
			.match_labels
			.as_ref()
			.unwrap()["kubernetes.io/metadata.name"],
		"keda"
	);
	assert_eq!(
		peers[0]
			.pod_selector
			.as_ref()
			.unwrap()
			.match_labels
			.as_ref()
			.unwrap()["app.kubernetes.io/component"],
		"interceptor"
	);
}

#[test]
fn secret_changes_only_roll_processes_that_use_them() {
	let (spec, cluster, secrets) = app_with_services();
	let before = build(&spec, &cluster, &secrets);
	let changed = SecretValues::new(BTreeMap::from([(
		"search-token".to_owned(),
		"changed-secret".to_owned(),
	)]));
	let after = build(&spec, &cluster, &changed);
	for name in ["app", "search", "files"] {
		let template = |objects: &[Object]| {
			objects
				.iter()
				.find_map(|object| {
					match object {
						Object::Deployment(deployment)
							if deployment.metadata.name.as_deref() == Some(name) =>
						{
							Some(deployment.spec.as_ref().unwrap().template.clone())
						}
						_ => None,
					}
				})
				.unwrap()
		};
		assert_eq!(template(&before) == template(&after), name != "search");
	}
}

#[test]
fn platform_defaults_validate_and_use_the_controller_certificate() {
	let profile = ClusterProfile::platform_defaults();
	profile.validate().unwrap();
	let (spec, _, secrets) = minimal_sleeping_app();
	for object in build(&spec, &profile, &secrets) {
		if let Object::Ingress(ingress) = object {
			let spec = ingress.spec.unwrap();
			assert_eq!(spec.ingress_class_name.as_deref(), Some("traefik"));
			assert_eq!(spec.tls.unwrap()[0].secret_name, None);
		}
	}
	assert!(ClusterProfile::default().validate().is_err());
	let mut invalid = profile;
	invalid.keda_http.as_mut().unwrap().interceptor_port = 0;
	assert!(invalid.validate().is_err());
	let mut zero_tmp = ClusterProfile::platform_defaults();
	zero_tmp.tmp_size_limit_mib = 0;
	assert!(zero_tmp.validate().is_err());
}

#[test]
fn edge_terminated_custom_domains_skip_cert_manager_issuer() {
	let (mut spec, mut profile, secrets) = minimal_sleeping_app();
	spec.routes[0].hostname = "shop.example.com".to_owned();
	spec.routes[0].tls = Tls::Issued;
	profile.cluster_issuer = None;
	profile.edge_terminated_custom_domains = true;
	let objects = build(&spec, &profile, &secrets);
	let ingress = objects
		.iter()
		.find_map(|object| {
			match object {
				Object::Ingress(ingress) => Some(ingress),
				_ => None,
			}
		})
		.expect("ingress present");
	assert!(ingress.metadata.annotations.is_none());
	let tls = ingress.spec.as_ref().unwrap().tls.as_ref().unwrap();
	assert_eq!(tls[0].secret_name, None);
	assert_eq!(
		tls[0].hosts.as_deref(),
		Some(&["shop.example.com".to_owned()][..])
	);
}

#[test]
fn tenant_placement_tmp_limit_and_profile_json_round_trip() {
	let (spec, mut profile, secrets) = minimal_sleeping_app();
	profile
		.tenant_node_selector
		.insert("leptoskit.dev/pool".to_owned(), "tenant".to_owned());
	profile
		.tenant_tolerations
		.push(leptoskit_cloud_runtime::TolerationSpec {
			key: "leptoskit.dev/pool".to_owned(),
			operator: "Equal".to_owned(),
			value: Some("tenant".to_owned()),
			effect: "PreferNoSchedule".to_owned(),
		});
	profile.tmp_size_limit_mib = 256;
	let encoded = serde_json::to_string(&profile).expect("profile serializes");
	let decoded: ClusterProfile = serde_json::from_str(&encoded).expect("profile deserializes");
	assert_eq!(decoded, profile);

	let objects = build(&spec, &decoded, &secrets);
	let deployment = objects
		.iter()
		.find_map(|object| {
			match object {
				Object::Deployment(deployment) => Some(deployment),
				_ => None,
			}
		})
		.expect("deployment present");
	let pod_spec = deployment
		.spec
		.as_ref()
		.unwrap()
		.template
		.spec
		.as_ref()
		.unwrap();
	assert_eq!(
		pod_spec
			.node_selector
			.as_ref()
			.unwrap()
			.get("leptoskit.dev/pool")
			.map(String::as_str),
		Some("tenant")
	);
	assert_eq!(pod_spec.tolerations.as_ref().unwrap().len(), 1);
	let tmp_vol = pod_spec
		.volumes
		.as_ref()
		.unwrap()
		.iter()
		.find(|v| v.name == "tmp")
		.unwrap();
	assert_eq!(
		tmp_vol
			.empty_dir
			.as_ref()
			.unwrap()
			.size_limit
			.as_ref()
			.unwrap()
			.0,
		"256Mi"
	);
	let (vap, binding) = leptoskit_cloud_runtime::kubernetes::worker_admission_policy(
		"kube-system",
		"leptoskit-cloud-worker",
	);
	assert_eq!(
		vap.metadata.name.as_deref(),
		Some("leptoskit-cloud-worker-scope")
	);
	assert_eq!(binding.spec.policy_name, "leptoskit-cloud-worker-scope");
	let spec = vap.spec.expect("vap spec");
	assert_eq!(spec.failure_policy.as_deref(), Some("Fail"));
	let conditions = spec.match_conditions.expect("matchConditions");
	assert_eq!(conditions.len(), 1);
	assert!(
		conditions[0]
			.expression
			.contains("system:serviceaccount:kube-system:leptoskit-cloud-worker")
	);
	let rules = spec
		.match_constraints
		.expect("matchConstraints")
		.resource_rules
		.expect("resourceRules");
	assert_eq!(rules[0].resources.as_deref(), Some(&["*/*".to_owned()][..]));
	let validations = spec.validations.expect("validations");
	assert_eq!(validations.len(), 2);
	assert!(validations[0].expression.contains("^lk-[0-9a-f]{20}$"));
	assert!(
		validations[1]
			.expression
			.contains("pod-security.kubernetes.io/enforce")
			&& validations[1]
				.expression
				.contains("app.kubernetes.io/managed-by")
	);
}

#[test]
fn cert_manager_issued_route_emits_acme_http01_solver_policy() {
	let (mut spec, mut profile, secrets) = minimal_sleeping_app();
	profile.edge_terminated_custom_domains = false;
	profile.cluster_issuer = Some("apps-http01".to_owned());
	spec.routes.push(Route {
		hostname: "shop.example.com".to_owned(),
		path_prefix: "/".to_owned(),
		process: "app".to_owned(),
		tls: Tls::Issued,
		strip_cookies: false,
	});
	let objects = build(&spec, &profile, &secrets);
	let solver = objects
		.iter()
		.find_map(|object| {
			match object {
				Object::NetworkPolicy(policy)
					if policy.metadata.name.as_deref() == Some("allow-acme-http01-solver") =>
				{
					Some(policy)
				}
				_ => None,
			}
		})
		.expect("HTTP-01 solver policy emitted for cert-manager Issued routes");
	let labels = solver
		.spec
		.as_ref()
		.unwrap()
		.pod_selector
		.as_ref()
		.unwrap()
		.match_labels
		.as_ref()
		.unwrap();
	assert_eq!(
		labels
			.get("acme.cert-manager.io/http01-solver")
			.map(String::as_str),
		Some("true")
	);
}

#[test]
fn cluster_profile_validation_rejects_invalid_tolerations_selectors_and_names() {
	let mut bad_toleration = ClusterProfile::platform_defaults();
	bad_toleration
		.tenant_tolerations
		.push(leptoskit_cloud_runtime::TolerationSpec {
			key: "leptoskit.dev/pool".to_owned(),
			operator: "Exists".to_owned(),
			value: Some("tenant".to_owned()),
			effect: "NoSchedule".to_owned(),
		});
	assert!(bad_toleration.validate().is_err());

	let mut bad_operator = ClusterProfile::platform_defaults();
	bad_operator
		.tenant_tolerations
		.push(leptoskit_cloud_runtime::TolerationSpec {
			key: "leptoskit.dev/pool".to_owned(),
			operator: "Bogus".to_owned(),
			value: None,
			effect: "NoSchedule".to_owned(),
		});
	assert!(bad_operator.validate().is_err());

	let mut bad_selector = ClusterProfile::platform_defaults();
	bad_selector
		.tenant_node_selector
		.insert("invalid key with spaces".to_owned(), "tenant".to_owned());
	assert!(bad_selector.validate().is_err());

	let mut bad_issuer = ClusterProfile::platform_defaults();
	bad_issuer.cluster_issuer = Some("INVALID_ISSUER".to_owned());
	assert!(bad_issuer.validate().is_err());
}

/// Where a `kubectl` is installed, the snapshots are real Kubernetes. Run
/// with `cargo nextest run --ignored` on a machine that has one.
#[test]
#[ignore = "only where kubectl is installed"]
fn the_manifests_pass_kubectl_dry_run() {
	let Ok(_) = Command::new("kubectl")
		.arg("version")
		.arg("--client")
		.output()
	else {
		return;
	};
	let cases = [
		to_yaml_of(minimal_sleeping_app()),
		to_yaml_of(app_with_services()),
	];
	for yaml in cases {
		let mut child = Command::new("kubectl")
			.args(["apply", "--dry-run=client", "-f", "-"])
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.expect("kubectl spawns");
		child
			.stdin
			.as_mut()
			.expect("stdin is piped")
			.write_all(yaml.as_bytes())
			.expect("kubectl reads stdin");
		let result = child.wait_with_output().expect("kubectl finishes");
		assert!(
			result.status.success(),
			"kubectl refused the manifests:\n{}",
			String::from_utf8_lossy(&result.stderr)
		);
	}
}

fn to_yaml_of((spec, cluster, secrets): (ReleaseSpec, ClusterProfile, SecretValues)) -> String {
	as_yaml(&build(&spec, &cluster, &secrets))
}
