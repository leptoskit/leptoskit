use std::sync::Arc;
use std::sync::Mutex;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::Method;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::any;
use serde_json::Value;
use serde_json::json;

use super::*;

fn fixture() -> ReleaseSpec {
	serde_json::from_value(json!({
		"environment": {
			"organization":{"slug":"org","id":"00000000-0000-0000-0000-000000000001"},
			"project":{"slug":"app","id":"00000000-0000-0000-0000-000000000002"},
			"environment":{"slug":"test","id":"00000000-0000-0000-0000-000000000003"}
		},"deployment":1,
		"processes":[{"name":"app","kind":"app","image":{"repository":"example.invalid/app","digest":format!("sha256:{}", "a".repeat(64))},"command":[],"args":[],"port":8080,"probes":{"ready":"/","live":"/"},"env":{},"secret_env":{"TOKEN":"token"},"resources":{"cpu_request_millis":100,"cpu_limit_millis":500,"memory_mib":128},"scale":{"min":1,"max":1,"sleep":false},"volumes":[]}],
		"routes":[],"policy":{"runtime_class":null,"egress":"none","quota":{"cpu_millis":1000,"memory_mib":256,"pods":2,"storage_gib":0},"database":null}
	})).unwrap()
}

fn deployment() -> Deployment {
	serde_json::from_value(json!({"metadata":{"name":"app","uid":"deployment","generation":2,"annotations":{"deployment.kubernetes.io/revision":"2"}},"spec":{"replicas":1,"selector":{"matchLabels":{"app":"app"}},"template":{"metadata":{"labels":{"app":"app"}},"spec":{"containers":[{"name":"app","image":"app@sha256:a"}]}}},"status":{"observedGeneration":2}})).unwrap()
}

fn set(ready: bool) -> ReplicaSet {
	let mut set: ReplicaSet = serde_json::from_value(json!({"metadata":{"name":"app-new","uid":"set","ownerReferences":[{"apiVersion":"apps/v1","kind":"Deployment","name":"app","uid":"deployment"}],"annotations":{"deployment.kubernetes.io/revision":"2"}},"spec":{"replicas":1,"selector":{"matchLabels":{"app":"app"}}},"status":{"replicas":1,"availableReplicas":i32::from(ready)}})).unwrap();
	set.spec.as_mut().unwrap().template = Some(deployment().spec.unwrap().template);
	set
}

fn pod(reason: Option<&str>) -> Pod {
	let mut value = json!({"metadata":{"name":"app-pod","labels":{"app.kubernetes.io/name":"app",MANAGED:OWNER},"ownerReferences":[{"apiVersion":"apps/v1","kind":"ReplicaSet","name":"app-new","uid":"set"}]},"status":{"containerStatuses":[{"name":"app","image":"app","imageID":"containerd://sha256:abc","ready":true,"restartCount":3,"state":{"running":{"startedAt":"2026-01-01T00:00:00Z"}},"lastState":{"terminated":{"exitCode":137,"reason":"OOMKilled","finishedAt":"2026-01-01T00:00:00Z"}}}]}});
	if let Some(reason) = reason {
		if matches!(reason, "Unschedulable" | "ReadinessProbeFailed") {
			value["status"]["conditions"] = json!([{"type":if reason=="Unschedulable" {"PodScheduled"} else {"Ready"},"status":"False","reason":reason,"message":"precise diagnostic"}]);
		} else {
			value["status"]["containerStatuses"][0]["state"] =
				json!({"waiting":{"reason":reason,"message":"container start failed"}});
		}
	}
	serde_json::from_value(value).unwrap()
}

#[test]
fn observations_use_current_set_pods_and_retained_oom_evidence() {
	let mut old = set(true);
	old.metadata
		.annotations
		.as_mut()
		.unwrap()
		.insert("deployment.kubernetes.io/revision".into(), "1".into());
	let actual = observe_process(
		&deployment(),
		&[old, set(false)],
		&[pod(Some("CrashLoopBackOff"))],
	);
	assert_eq!(actual.ready_replicas, 0);
	assert_eq!(actual.observed_restarts, 3);
	assert_eq!(actual.last_oom_kill, Some(1_767_225_600_000));
	assert_eq!(actual.image, "containerd://sha256:abc");
	assert!(
		matches!(actual.state, ProcessState::Failed { reason } if reason.contains("CrashLoopBackOff") && reason.contains("137"))
	);
	let actual = observe_process(&deployment(), &[set(true)], &[pod(None)]);
	assert_eq!(actual.state, ProcessState::Ready);
	let mut sleeping = deployment();
	sleeping.spec.as_mut().unwrap().replicas = Some(0);
	assert_eq!(
		observe_process(&sleeping, &[], &[]).state,
		ProcessState::Sleeping
	);
	assert_eq!(
		observe_process(&deployment(), &[set(false)], &[]).observed_restarts,
		0
	);
}

#[test]
fn old_template_and_unobserved_generation_cannot_complete_a_rollout() {
	let mut old = set(true);
	old.spec
		.as_mut()
		.unwrap()
		.template
		.as_mut()
		.unwrap()
		.spec
		.as_mut()
		.unwrap()
		.containers[0]
		.image = Some("old-image".into());
	assert_eq!(
		observe_process(&deployment(), &[old], &[pod(None)]).state,
		ProcessState::Pending
	);
	let mut deployment = deployment();
	deployment.metadata.generation = Some(3);
	assert_eq!(
		observe_process(&deployment, &[set(true)], &[pod(None)]).state,
		ProcessState::Pending
	);
}

#[test]
fn scheduler_and_readiness_conditions_are_preserved() {
	for (kind, reason) in [
		("PodScheduled", "Unschedulable"),
		("Ready", "ReadinessProbeFailed"),
	] {
		let mut pod = pod(None);
		pod.status.as_mut().unwrap().conditions = Some(
			serde_json::from_value(
				json!([{"type":kind,"status":"False","reason":reason,"message":"precise diagnostic"}]),
			)
			.unwrap(),
		);
		assert!(
			matches!(observe_process(&deployment(), &[set(false)], &[pod]).state, ProcessState::Failed { reason: actual } if actual.contains(reason) && actual.contains("precise diagnostic"))
		);
	}
}

#[test]
fn prune_protects_claims_unlabelled_and_controller_owned_objects() {
	let object = |kind: &str, name: &str, managed: bool| {
		serde_json::from_value::<DynamicObject>(json!({"apiVersion":"v1","kind":kind,"metadata":{"name":name,"labels":if managed {json!({MANAGED:OWNER})}else{json!({})}}})).unwrap()
	};
	let mut child = object("HorizontalPodAutoscaler", "child", true);
	child.metadata.owner_references = Some(
		serde_json::from_value(
			json!([{"apiVersion":"keda.sh/v1alpha1","kind":"ScaledObject","name":"app","uid":"scaler"}]),
		)
		.unwrap(),
	);
	let existing = vec![
		object("Service", "removed", true),
		object("Deployment", "app", true),
		object("PersistentVolumeClaim", "data", true),
		object("Secret", "operator", false),
		child,
	];
	assert_eq!(
		prune_set(
			&existing,
			&BTreeSet::from([("Deployment".into(), "app".into())])
		),
		["removed"]
	);
}

#[test]
fn errors_discard_api_bodies() {
	for (code, expected) in [
		(403, "forbidden"),
		(409, "conflict"),
		(503, "unavailable"),
		(422, "other"),
	] {
		let error = api_error(kube::Error::Api(Box::new(
			serde_json::from_value(
				json!({"code":code,"message":"secret-value","reason":"secret-value","status":"Failure"}),
			)
			.unwrap(),
		)));
		assert!(!error.to_string().contains("secret-value"));
		assert!(match expected {
			"forbidden" => matches!(error, RuntimeError::Forbidden),
			"conflict" => matches!(error, RuntimeError::Conflict),
			"unavailable" => matches!(error, RuntimeError::Unavailable),
			_ => matches!(error, RuntimeError::Other(_)),
		});
	}
}

#[test]
fn logs_are_sorted_and_bounded() {
	let line = |at_millis| {
		LogLine {
			process: "app".into(),
			pod: Some("pod".into()),
			at_millis,
			message: "hello".into(),
		}
	};
	let lines = bound_logs(vec![line(1), line(3), line(2)], 2, LOG_BYTES);
	assert_eq!(
		lines.iter().map(|line| line.at_millis).collect::<Vec<_>>(),
		[3, 2]
	);
	assert_eq!(bound_logs(vec![line(1)], 1, 1), []);
}

#[derive(Default)]
struct Script {
	requests: Vec<(Method, String, HeaderMap, Value)>,
	code: Option<u16>,
	reason: Option<String>,
	prune: bool,
	absent: bool,
	polls: usize,
}

async fn handler(
	State(script): State<Arc<Mutex<Script>>>,
	method: Method,
	uri: Uri,
	headers: HeaderMap,
	body: Bytes,
) -> Response {
	let mut script = script.lock().unwrap();
	let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
	script
		.requests
		.push((method.clone(), uri.to_string(), headers, value.clone()));
	let response = |value: Value| axum::Json(value).into_response();
	if let Some(code) = script.code {
		return (StatusCode::from_u16(code).unwrap(), axum::Json(json!({"kind":"Status","apiVersion":"v1","status":"Failure","message":"secret-value","reason":"failure","code":code}))).into_response();
	}
	if method == Method::PATCH {
		return response(value);
	}
	if method == Method::DELETE {
		script.absent = true;
		return response(json!({"kind":"Status","apiVersion":"v1","status":"Success"}));
	}
	let path = uri.path();
	if path.ends_with("/log") {
		return "2026-01-01T00:00:01Z newest secret-value\n2026-01-01T00:00:00Z older\n"
			.into_response();
	}
	if path.ends_with(&fixture().environment.namespace()) {
		if script.absent {
			return (StatusCode::NOT_FOUND, axum::Json(json!({"kind":"Status","apiVersion":"v1","code":404,"reason":"NotFound","message":"gone","status":"Failure"}))).into_response();
		}
		return response(
			json!({"kind":"Namespace","apiVersion":"v1","metadata":{"name":fixture().environment.namespace()}}),
		);
	}
	let items = if path.ends_with("/deployments") {
		script.polls += 1;
		vec![serde_json::to_value(deployment()).unwrap()]
	} else if path.ends_with("/replicasets") {
		vec![serde_json::to_value(set(script.reason.is_none() && script.polls >= 3)).unwrap()]
	} else if path.ends_with("/pods") {
		vec![serde_json::to_value(pod(script.reason.as_deref())).unwrap()]
	} else if path.ends_with("/services") && script.prune {
		vec![
			json!({"kind":"Service","apiVersion":"v1","metadata":{"name":"removed","labels":{MANAGED:OWNER}}}),
		]
	} else {
		vec![]
	};
	response(json!({"kind":"List","apiVersion":"v1","metadata":{},"items":items}))
}

async fn server(script: Script) -> (Cluster, Arc<Mutex<Script>>, tokio::task::JoinHandle<()>) {
	let script = Arc::new(Mutex::new(script));
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let app = Router::new()
		.fallback(any(handler))
		.with_state(script.clone());
	let task = tokio::spawn(async move {
		axum::serve(listener, app).await.unwrap();
	});
	let mut config = Config::new(format!("http://{address}").parse().unwrap());
	config.default_retry = false;
	let client = Client::try_from(config).unwrap();
	let cluster = Cluster::new(
		client,
		ClusterProfile::platform_defaults(),
		SecretValues::new(std::collections::BTreeMap::from([(
			"token".into(),
			"secret-value".into(),
		)])),
		Duration::from_millis(700),
	)
	.unwrap();
	(cluster, script, task)
}

#[tokio::test]
async fn apply_uses_ssa_in_order_prunes_and_waits_for_new_set() {
	let (cluster, script, task) = server(Script {
		prune: true,
		..Default::default()
	})
	.await;
	cluster.apply(&fixture()).await.unwrap();
	let script = script.lock().unwrap();
	let patches: Vec<_> = script
		.requests
		.iter()
		.filter(|request| request.0 == Method::PATCH)
		.collect();
	assert!(patches[0].1.starts_with("/api/v1/namespaces/lk-"));
	let secret = patches
		.iter()
		.position(|request| request.1.contains("/secrets/"))
		.unwrap();
	let deployment = patches
		.iter()
		.position(|request| request.1.contains("/deployments/"))
		.unwrap();
	assert!(secret < deployment);
	for (method, path, headers, body) in &script.requests {
		assert!(!path.contains("secret-value"));
		assert!(!body.to_string().contains("secret-value"));
		if body["kind"] != "Secret" {
			assert!(!body.to_string().contains("c2VjcmV0LXZhbHVl"));
		}
		if *method == Method::PATCH {
			assert_eq!(headers["content-type"], "application/apply-patch+yaml");
			assert!(path.contains(&format!("fieldManager={FIELD_MANAGER}")));
		}
	}
	assert!(
		script.requests.iter().any(|request| {
			request.0 == Method::DELETE
				&& request
					.1
					.split('?')
					.next()
					.is_some_and(|path| path.ends_with("/services/removed"))
		}),
		"requests: {:?}",
		script
			.requests
			.iter()
			.map(|request| (&request.0, &request.1))
			.collect::<Vec<_>>()
	);
	assert!(script.polls >= 2);
	task.abort();
}

#[tokio::test]
async fn first_apply_does_not_prune_desired_objects() {
	let (cluster, script, task) = server(Script::default()).await;
	let spec = fixture();
	cluster.apply(&spec).await.unwrap();
	let script = script.lock().unwrap();
	assert!(
		!script
			.requests
			.iter()
			.any(|request| request.0 == Method::DELETE)
	);
	let kinds: Vec<_> = script
		.requests
		.iter()
		.filter(|request| request.0 == Method::PATCH)
		.map(|request| request.3["kind"].as_str().unwrap())
		.collect();
	let objects = manifests(&spec, &cluster.profile, &cluster.secrets).unwrap();
	assert_eq!(
		kinds,
		objects
			.iter()
			.map(crate::kubernetes::Object::kind)
			.collect::<Vec<_>>()
	);
	assert!(script.polls >= 3);
	task.abort();
}

#[tokio::test]
async fn status_logs_destroy_and_already_absent_destroy() {
	let (cluster, script, task) = server(Script::default()).await;
	let environment = fixture().environment;
	let observed = cluster.status(&environment).await.unwrap();
	assert_eq!(observed.processes[0].observed_restarts, 3);
	assert!(observed.processes[0].last_oom_kill.is_some());
	let logs = cluster
		.logs(
			&environment,
			&LogQuery {
				limit: Some(1),
				..Default::default()
			},
		)
		.await
		.unwrap();
	assert_eq!(logs.len(), 1);
	assert_eq!(logs[0].pod.as_deref(), Some("app-pod"));
	assert_eq!(logs[0].message, "newest [redacted]");
	cluster.destroy(&environment).await.unwrap();
	cluster.destroy(&environment).await.unwrap();
	assert_eq!(
		script
			.lock()
			.unwrap()
			.requests
			.iter()
			.filter(|request| request.0 == Method::DELETE)
			.count(),
		1
	);
	task.abort();
}

#[tokio::test]
async fn rollout_failures_name_process_and_real_reason() {
	for reason in [
		"ImagePullBackOff",
		"CrashLoopBackOff",
		"Unschedulable",
		"ReadinessProbeFailed",
	] {
		let (cluster, _, task) = server(Script {
			reason: Some(reason.into()),
			..Default::default()
		})
		.await;
		assert!(
			matches!(cluster.apply(&fixture()).await,Err(RuntimeError::NotReady { process, reason: actual }) if process=="app" && actual.contains(reason))
		);
		task.abort();
	}
}

#[tokio::test]
async fn api_outage_and_forbidden_are_typed() {
	for code in [503, 403] {
		let (cluster, _, task) = server(Script {
			code: Some(code),
			..Default::default()
		})
		.await;
		let error = cluster.apply(&fixture()).await.unwrap_err();
		assert!(matches!(
			(code, error),
			(503, RuntimeError::Unavailable) | (403, RuntimeError::Forbidden)
		));
		task.abort();
	}
}

/// Opt in only with an explicitly named disposable cluster. Uses a small
/// multi-architecture public http-echo image, pinned by its manifest-list
/// digest.
#[tokio::test]
#[ignore = "requires LEPTOSKIT_TEST_KUBECONFIG pointing at a disposable local cluster"]
async fn real_cluster_apply_status_logs_prune_destroy() {
	let path =
		std::env::var_os("LEPTOSKIT_TEST_KUBECONFIG").expect("name an explicit test kubeconfig");
	let mut profile = ClusterProfile::platform_defaults();
	profile.keda_http = None;
	let cluster = Cluster::from_kubeconfig(
		Path::new(&path),
		profile,
		SecretValues::default(),
		Duration::from_secs(120),
	)
	.await
	.unwrap();
	let mut spec = fixture();
	spec.environment.environment.id = uuid::Uuid::now_v7();
	let app = &mut spec.processes[0];
	app.image.repository = "docker.io/hashicorp/http-echo".into();
	// https://hub.docker.com/v2/repositories/hashicorp/http-echo/tags/1.0.0
	app.image.digest =
		"sha256:fcb75f691c8b0414d670ae570240cbf95502cc18a9ba57e982ecac589760a186".into();
	app.args = vec!["-listen=:8080".into(), "-text=ready".into()];
	app.secret_env.clear();
	let mut service = app.clone();
	service.name = "removed".into();
	service.kind = crate::ProcessKind::Service;
	spec.processes.push(service);
	spec.policy.quota = crate::Quota::for_processes(&spec.processes);
	let outcome = async {
		cluster.apply(&spec).await?;
		let status = cluster.status(&spec.environment).await?;
		assert_eq!(status.processes.len(), 2);
		assert!(
			status
				.processes
				.iter()
				.all(|process| process.ready_replicas == 1)
		);
		let _logs = cluster
			.logs(&spec.environment, &LogQuery::default())
			.await?;
		spec.processes.pop();
		spec.deployment += 1;
		cluster.apply(&spec).await?;
		let deployments =
			Api::<Deployment>::namespaced(cluster.client.clone(), &spec.environment.namespace());
		assert!(
			deployments
				.get_opt("removed")
				.await
				.map_err(api_error)?
				.is_none()
		);
		let services = Api::<k8s_openapi::api::core::v1::Service>::namespaced(
			cluster.client.clone(),
			&spec.environment.namespace(),
		);
		assert!(
			services
				.get_opt("removed")
				.await
				.map_err(api_error)?
				.is_none()
		);
		Ok::<(), RuntimeError>(())
	}
	.await;
	let destroyed = cluster.destroy(&spec.environment).await;
	outcome.unwrap();
	destroyed.unwrap();
}
