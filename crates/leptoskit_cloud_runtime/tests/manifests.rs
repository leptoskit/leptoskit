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
use leptoskit_cloud_runtime::kubernetes::HttpScaledObject;
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
		ingress_class: "nginx".to_owned(),
		ingress_namespace: "ingress-nginx".to_owned(),
		wildcard_tls_secret: Some("leptoskit-wildcard-tls".to_owned()),
		cluster_issuer: Some("leptoskit-ca".to_owned()),
		dns_namespace: "kube-system".to_owned(),
		blocked_egress_cidrs: vec![
			"10.0.0.0/8".to_owned(),
			"169.254.0.0/16".to_owned(),
			"192.168.0.0/16".to_owned(),
		],
		image_pull_secret: None,
		keda_http: true,
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
				cpu_millis: 500,
				memory_mib: 128,
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
				cpu_millis: 12000,
				memory_mib: 4096,
				pods: 12,
				storage_gib: 10,
			},
			database: Some(DatabaseAccess {
				namespace: "postgres-shared".to_owned(),
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
			"HTTPScaledObject",
			"Ingress",
		]
	);
	insta::assert_snapshot!(as_yaml(&objects));
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
	cluster.keda_http = false;
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
				namespace: "postgres-shared".to_owned(),
				port: 5432,
			});
			spec.policy.quota = Quota {
				cpu_millis: 500,
				memory_mib: 128,
				pods: 2,
				storage_gib: 1,
			};
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
			cluster.keda_http = false;
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
				Object::HttpScaledObject(HttpScaledObject { spec: scaled, .. }) => Some(scaled),
				_ => None,
			}
		})
		.expect("the sleeper scales from zero");
	assert_eq!(keda.replicas_min, 0);
	assert_eq!(keda.replicas_max, 1);
	let trigger = &keda.triggers[0];
	assert_eq!(
		trigger.metadata.get("hosts").map(String::as_str),
		Some("notes.leptoskit.app")
	);
	assert_eq!(trigger.metadata.get("port").map(String::as_str), Some("80"));
	assert_eq!(
		trigger.metadata.get("service").map(String::as_str),
		Some("app")
	);
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
