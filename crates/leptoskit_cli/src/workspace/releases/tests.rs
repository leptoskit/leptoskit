use super::*;

fn fixture() -> (tempfile::TempDir, PathBuf, plan::Plan) {
	let directory = tempfile::tempdir().unwrap();
	let root = directory.path();
	let manifest = root.join("leptoskit.workspace.toml");
	fs::write(
		&manifest,
		"format_version=1\n[applications.api]\npath='apps/api'\n[applications.web]\npath='apps/\
		 web'\ndepends-on=['api']\n",
	)
	.unwrap();
	let mut applications = serde_json::Map::new();
	for name in ["api", "web"] {
		let app = root.join("apps").join(name);
		fs::create_dir_all(app.join("migrations")).unwrap();
		fs::write(app.join("migrations/registry.txt"), "").unwrap();
		fs::write(
			app.join("leptoskit.toml"),
			format!(
				r#"
[app]
name = "{name}"
platforms = ["server"]
default_platform = "server"
[serve]
probes = true
[db]
dialect = "postgres"
[infra]
name = "{name}-production"
domain = "{name}.example.com"
location = "hel1"
server_type = "cx23"
ssh_key_ids = [123]
ssh_cidrs = ["192.0.2.1/32"]
account_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
zone_id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
updates_domain = "updates-{name}.example.com"
updates_bucket = "{name}-releases"
backup_bucket = "{name}-backups"
image = "ghcr.io/team/{name}"
registry = "ghcr.io"
registry_username = "team"
secrets = ["API_TOKEN", "PATH"]
"#
			),
		)
		.unwrap();
		let descriptor = serde_json::json!({
			"format_version":1,"kind":"leptoskit-verified-backend-image","inventory_sha256":"a".repeat(64),
			"recipe":{"repository":"https://github.com/example/backend","commit":"a".repeat(40),
				"builder_image":format!("builder@sha256:{}","a".repeat(64)),"target":"aarch64-unknown-linux-musl",
				"package":name,"binary":name,"features":[],"no_default_features":false,"rustflags":[]},
			"executable_sha256":"b".repeat(64),"executable_mode":493,"migrations":hash(b"[]"),
			"runtime_image":format!("runtime@sha256:{}","a".repeat(64)),"runtime_config_sha256":"c".repeat(64),
			"engine_image_id":format!("sha256:{}","a".repeat(64)),"image_config_sha256":hash(b"{}"),"platform":"linux/arm64",
			"assets":"not_applicable","services":"unsupported","rpc_contract":{"state":"not_applicable","declaration":"operator_declares_no_rpc"},
			"native_guest_compatibility":"not_applicable","runtime_attestation":"absent"});
		let bytes = serde_json::to_vec(&descriptor).unwrap();
		fs::write(app.join("descriptor.json"), &bytes).unwrap();
		applications.insert(name.into(), serde_json::json!({"version":"v1","descriptor":format!("apps/{name}/descriptor.json"),"descriptor-sha256":hash(&bytes),"setup":true}));
	}
	let releases =
		serde_json::from_value(serde_json::json!({"format_version":1,"applications":applications}))
			.unwrap();
	let manifest = manifest.canonicalize().unwrap();
	let plan = plan::create(&manifest, releases).unwrap();
	(directory, manifest, plan)
}

#[test]
fn plan_checks_order_membership_and_configuration_before_execution() {
	let (_directory, manifest, mut plan) = fixture();
	assert_eq!(plan.startup_order, ["api", "web"]);
	plan.startup_order.reverse();
	assert!(plan::validate(&manifest, &plan).is_err());
	plan.startup_order.reverse();
	plan.applications
		.get_mut("web")
		.unwrap()
		.configuration_sha256 = "0".repeat(64);
	assert!(plan::validate(&manifest, &plan).is_err());
}

#[test]
fn changed_manifest_descriptor_and_escaping_path_fail() {
	let (directory, manifest, mut plan) = fixture();
	let app = plan.applications.get_mut("web").unwrap();
	app.descriptor = PathBuf::from("../outside");
	assert!(plan::validate(&manifest, &plan).is_err());
	app_restore(&mut plan);
	fs::write(directory.path().join("apps/web/descriptor.json"), "{}").unwrap();
	assert!(plan::validate(&manifest, &plan).is_err());
	fs::write(&manifest, "format_version=2").unwrap();
	assert!(plan::unchanged(&manifest, &plan, "api").is_err());
}

fn app_restore(plan: &mut plan::Plan) {
	plan.applications.get_mut("web").unwrap().descriptor =
		PathBuf::from("apps/web/descriptor.json");
}

#[test]
fn readiness_dependencies_must_be_selected_for_fresh_checks() {
	let (_directory, manifest, mut plan) = fixture();
	plan.applications.remove("api");
	plan.startup_order = vec!["web".into()];
	let error = plan::validate(&manifest, &plan).unwrap_err();
	assert!(
		error
			.to_string()
			.contains("include its current verified artifact")
	);
}

#[test]
fn credential_references_never_grant_control_over_release_tools() {
	let (_directory, manifest, mut plan) = fixture();
	plan.applications
		.get_mut("api")
		.unwrap()
		.environment
		.insert("PATH".into(), "API_PATH".into());
	assert!(plan::validate(&manifest, &plan).is_err());
	let environment = &mut plan.applications.get_mut("api").unwrap().environment;
	environment.clear();
	environment.insert(
		"API_TOKEN".into(),
		"WORKSPACE_TEST_MISSING_API_TOKEN".into(),
	);
	plan::validate(&manifest, &plan).unwrap();
	assert!(resolved_environment("api", &plan.applications["api"]).is_err());
}

#[test]
fn staging_freezes_bytes_and_rejects_post_plan_substitution() {
	let (directory, _manifest, plan) = fixture();
	let staging = tempfile::tempdir().unwrap();
	let root = directory.path().canonicalize().unwrap();
	plan::stage(&root, &plan, staging.path()).unwrap();
	let original = fs::read(staging.path().join("api.json")).unwrap();
	fs::write(root.join("apps/api/descriptor.json"), "{}").unwrap();
	assert_eq!(fs::read(staging.path().join("api.json")).unwrap(), original);
	assert!(plan::stage(&root, &plan, staging.path()).is_err());
}

#[test]
fn journal_retains_interrupted_progress_but_rejects_another_plan() {
	let (directory, _manifest, plan) = fixture();
	let path = directory.path().join("journal.json");
	let mut state = journal(&path, &plan, "pin").unwrap();
	state.applications.get_mut("api").unwrap().state = State::Running;
	state.applications.get_mut("api").unwrap().attempts = 1;
	save(&path, &state).unwrap();
	let recovered = journal(&path, &plan, "pin").unwrap();
	assert!(matches!(
		recovered.applications["api"].state,
		State::Running
	));
	assert_eq!(recovered.applications["api"].attempts, 1);
	assert!(journal(&path, &plan, "other").is_err());
}

#[cfg(unix)]
#[test]
fn command_preserves_pins_app_directory_and_explicit_environment() {
	use std::os::unix::fs::PermissionsExt as _;
	let (directory, _manifest, plan) = fixture();
	let script = directory.path().join("release-fixture");
	fs::write(
		&script,
		"#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" > arguments\nprintf '%s' \"$API_TOKEN\" > \
		 mapped-token\nprintf '%s' \"$LEPTOSKIT_CONTROL_LOCAL\" > local-mode\n",
	)
	.unwrap();
	fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
	let staging = tempfile::tempdir().unwrap();
	execute(
		&script,
		directory.path(),
		"api",
		&plan.applications["api"],
		staging.path(),
		&BTreeMap::from([("API_TOKEN".into(), "test-scoped-value".into())]),
		Duration::from_secs(2),
	)
	.unwrap();
	let app = directory.path().join("apps/api");
	let args = fs::read_to_string(app.join("arguments")).unwrap();
	assert!(args.contains("--reuse-current\n"));
	assert!(args.contains(&format!(
		"--configuration-sha256\n{}\n",
		plan.applications["api"].configuration_sha256
	)));
	assert!(args.contains(&format!(
		"--descriptor-sha256\n{}\n",
		plan.applications["api"].descriptor_sha256
	)));
	assert!(args.contains("--setup\n"));
	assert_eq!(
		fs::read_to_string(app.join("mapped-token")).unwrap(),
		"test-scoped-value"
	);
	assert_eq!(fs::read_to_string(app.join("local-mode")).unwrap(), "1");
}

#[cfg(unix)]
#[test]
fn deadline_terminates_the_owned_process_tree() {
	use std::os::unix::fs::PermissionsExt as _;
	let (directory, _manifest, plan) = fixture();
	let script = directory.path().join("release-fixture");
	fs::write(
		&script,
		"#!/bin/sh\n(sleep 1; echo late > late-marker) &\nwait\n",
	)
	.unwrap();
	fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
	let result = execute(
		&script,
		directory.path(),
		"api",
		&plan.applications["api"],
		directory.path(),
		&BTreeMap::new(),
		Duration::from_millis(100),
	);
	assert!(
		result
			.unwrap_err()
			.to_string()
			.contains("deadline exceeded")
	);
	std::thread::sleep(Duration::from_millis(1200));
	assert!(!directory.path().join("apps/api/late-marker").exists());
}

#[cfg(unix)]
#[test]
fn batch_stops_after_failure_and_retry_rechecks_completed_apps() {
	use std::os::unix::fs::PermissionsExt as _;
	let (directory, manifest, plan) = fixture();
	let root = directory.path();
	let script = root.join("release-fixture");
	fs::write(
		&script,
		"#!/bin/sh\nset -eu\nname=$(basename \"$PWD\")\nprintf '%s\\n' \"$name\" >> \
		 ../../calls\nif [ -e \"../../fail-$name\" ]; then exit 1; fi\n",
	)
	.unwrap();
	fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
	let bytes = serde_json::to_vec(&plan).unwrap();
	let pin = hash(&bytes);
	let plan_path = root.join("plan.json");
	fs::write(&plan_path, &bytes).unwrap();
	let args = DeployArgs {
		manifest: Some(manifest),
		plan: plan_path,
		sha256: pin.clone(),
		timeout_seconds: 2,
	};
	fs::write(root.join("fail-api"), "").unwrap();
	assert!(deploy_with(&args, &script).is_err());
	assert_eq!(fs::read_to_string(root.join("calls")).unwrap(), "api\n");
	fs::remove_file(root.join("fail-api")).unwrap();
	fs::write(root.join("fail-web"), "").unwrap();
	assert!(deploy_with(&args, &script).is_err());
	let journal_path = root.join(format!(".leptoskit/workspace/{pin}.json"));
	let state = journal(&journal_path, &plan, &pin).unwrap();
	assert!(matches!(
		state.applications["api"].state,
		State::CommandCompleted
	));
	assert!(matches!(state.applications["web"].state, State::Failed));
	fs::remove_file(root.join("fail-web")).unwrap();
	deploy_with(&args, &script).unwrap();
	assert_eq!(
		fs::read_to_string(root.join("calls")).unwrap(),
		"api\napi\nweb\napi\nweb\n"
	);
	let state = journal(&journal_path, &plan, &pin).unwrap();
	assert_eq!(state.applications["api"].attempts, 3);
	assert_eq!(state.applications["web"].attempts, 2);
	assert!(
		state
			.applications
			.values()
			.all(|progress| matches!(progress.state, State::CommandCompleted))
	);
}
