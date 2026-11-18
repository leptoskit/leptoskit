use serde_json::json;

use super::*;

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
