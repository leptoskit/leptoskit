use super::*;

fn fixture() -> Window {
	let (value, _) = super::super::schema_window_fixture::evidence();
	Window::new(
		value["window"].as_str().unwrap(),
		value["window_sha256"].as_str().unwrap(),
	)
	.unwrap()
}

fn config() -> String {
	serde_json::to_string_pretty(&json!({
		"service": "notes", "env": {"clear": {"APP_SETTING": "keep"}, "secret": ["DATABASE_URL"]},
		"volumes": ["/var/lib/notes/uploads:/app/uploads:ro"],
		"servers": {"web": {"hosts": ["192.0.2.1"]}}
	}))
	.unwrap()
		+ "\n"
}

fn observation(window: &Window) -> Value {
	json!({"State": {"Running": true}, "Config": {"Env": [
		format!("{PATH_ENV}={CONTAINER_PATH}"), format!("{PIN_ENV}={}", window.pin), "APP_SETTING=keep",
	]}, "Mounts": [{"Type": "bind", "RW": false, "Source": window.host_path("notes").unwrap(), "Destination": CONTAINER_PATH}]})
}

#[test]
fn runtime_configuration_binds_one_window_without_discarding_app_settings() {
	let window = fixture();
	let original = config();
	let configured = configure(&original, "notes", Some(&window)).unwrap();
	let value: Value = serde_json::from_str(&configured).unwrap();
	assert_eq!(value["env"]["clear"][PATH_ENV], CONTAINER_PATH);
	assert_eq!(value["env"]["clear"][PIN_ENV], window.pin);
	assert_eq!(value["env"]["clear"]["APP_SETTING"], "keep");
	assert_eq!(value["env"]["secret"], json!(["DATABASE_URL"]));

	assert_eq!(value["volumes"].as_array().unwrap().len(), 2);
	assert_eq!(
		value["volumes"][0],
		"/var/lib/notes/uploads:/app/uploads:ro"
	);
	assert_eq!(
		configure(&configured, "notes", Some(&window)).unwrap(),
		configured
	);
	assert_eq!(configure(&configured, "notes", None).unwrap(), original);
}

#[test]
fn ordinary_generated_configuration_stays_byte_identical_without_a_window() {
	let mut value: Value = serde_json::from_str(&config()).unwrap();
	value.as_object_mut().unwrap().remove("volumes");
	let original = serde_json::to_string_pretty(&value).unwrap() + "\n";
	assert_eq!(configure(&original, "notes", None).unwrap(), original);
	let configured = configure(&original, "notes", Some(&fixture())).unwrap();
	assert_eq!(configure(&configured, "notes", None).unwrap(), original);
}

#[test]
fn recovery_selects_the_previous_window_instead_of_the_failed_candidates_pin() {
	let previous = fixture();
	// Different exact bytes remain a valid document but identify a different
	// approved input. The failed candidate's digest must disappear entirely.
	let candidate_json = format!("{}\n", previous.json);
	let candidate = Window::new(&candidate_json, &leptoskit_db::checksum(&candidate_json)).unwrap();
	let failed = configure(&config(), "notes", Some(&candidate)).unwrap();
	let restored = configure(&failed, "notes", Some(&previous)).unwrap();
	assert!(!restored.contains(&candidate.pin));
	assert_eq!(
		restored,
		configure(&config(), "notes", Some(&previous)).unwrap()
	);
}

#[test]
fn unapproved_secret_overrides_and_unknown_mounts_are_refused() {
	let window = fixture();

	for name in [PATH_ENV, PIN_ENV] {
		let mut value: Value = serde_json::from_str(&config()).unwrap();
		value["env"]["secret"]
			.as_array_mut()
			.unwrap()
			.push(json!(name));
		assert!(configure(&value.to_string(), "notes", Some(&window)).is_err());
		assert!(configure(&value.to_string(), "notes", None).is_err());
	}
	for mount in [
		format!("/unapproved.json:{CONTAINER_PATH}:ro"),
		format!("{}:{CONTAINER_PATH}:rw", window.host_path("notes").unwrap()),
	] {
		let mut value: Value = serde_json::from_str(&config()).unwrap();
		value["volumes"].as_array_mut().unwrap().push(json!(mount));
		assert!(configure(&value.to_string(), "notes", Some(&window)).is_err());
	}

	for name in ["../escape", "", "-option", "name/child", "name'quote"] {
		assert!(configure(&config(), name, Some(&window)).is_err());
	}

	assert!(configure(&config(), "App_2", Some(&window)).is_ok());
}

#[test]
fn running_container_requires_the_exact_pin_and_read_only_bind_source() {
	let window = fixture();
	let original = observation(&window);
	window.check_container("notes", &original).unwrap();
	let mut stopped = original.clone();
	stopped["State"]["Running"] = false.into();
	assert!(window.check_container("notes", &stopped).is_err());
	for environment in [
		json!([format!("{PATH_ENV}={CONTAINER_PATH}")]),
		json!([
			format!("{PATH_ENV}=/wrong"),
			format!("{PIN_ENV}={}", window.pin)
		]),
		json!([
			format!("{PATH_ENV}={CONTAINER_PATH}"),
			format!("{PIN_ENV}={}", "0".repeat(64))
		]),
		json!([
			format!("{PATH_ENV}={CONTAINER_PATH}"),
			format!("{PIN_ENV}={}", window.pin),
			format!("{PIN_ENV}={}", window.pin)
		]),
	] {
		let mut value = original.clone();
		value["Config"]["Env"] = environment;
		assert!(window.check_container("notes", &value).is_err());
	}
	for (key, value) in [
		("RW", json!(true)),
		("Type", json!("volume")),
		("Source", json!("/wrong")),
		("Destination", json!("/wrong")),
	] {
		let mut changed = original.clone();
		changed["Mounts"][0][key] = value;
		assert!(window.check_container("notes", &changed).is_err());
	}

	let mut duplicated = original.clone();
	duplicated["Mounts"]
		.as_array_mut()
		.unwrap()
		.push(original["Mounts"][0].clone());
	assert!(window.check_container("notes", &duplicated).is_err());
}

#[test]
fn runtime_input_cannot_replace_a_valid_document_against_the_original_pin() {
	let window = fixture();
	assert!(Window::new(&format!("{}\n", window.json), &window.pin).is_err());
	assert!(Window::new("{}", &leptoskit_db::checksum("{}")).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn installation_is_atomic_nonreplacing_and_rejects_changed_files() {
	use std::io::Write as _;
	use std::os::unix::fs::MetadataExt;
	use std::os::unix::fs::PermissionsExt;
	use std::process::Command;
	use std::process::Stdio;
	let window = fixture();
	let directory = tempfile::tempdir().unwrap();
	let path = directory.path().join("window.json");
	let script =
		window.install_script_at(directory.path().to_str().unwrap(), path.to_str().unwrap());
	let run = |bytes: &[u8]| {
		let mut child = Command::new("sh")
			.args(["-c", &script])
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.unwrap();
		child.stdin.take().unwrap().write_all(bytes).unwrap();
		child.wait_with_output().unwrap()
	};
	assert!(!run(b"substitution").status.success());
	assert!(!path.exists());

	assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
	let installed = run(window.json.as_bytes());
	assert!(
		installed.status.success(),
		"{}",
		String::from_utf8_lossy(&installed.stderr)
	);
	assert_eq!(installed.stdout, window.pin.as_bytes());
	assert_eq!(
		fs::metadata(&path).unwrap().permissions().mode() & 0o777,
		0o444
	);
	let inode = fs::metadata(&path).unwrap().ino();
	assert!(run(window.json.as_bytes()).status.success());
	assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
	fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
	fs::write(&path, b"changed").unwrap();
	assert!(!run(window.json.as_bytes()).status.success());
	assert_eq!(fs::read(&path).unwrap(), b"changed");
	fs::remove_file(&path).unwrap();
	std::os::unix::fs::symlink("missing", &path).unwrap();
	assert!(!run(window.json.as_bytes()).status.success());
	assert!(
		fs::symlink_metadata(&path)
			.unwrap()
			.file_type()
			.is_symlink()
	);
	fs::remove_file(&path).unwrap();
	fs::create_dir(&path).unwrap();
	assert!(!run(window.json.as_bytes()).status.success());
	assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
	fs::remove_dir(&path).unwrap();
	let destination = directory.path().join("another-directory");
	fs::create_dir(&destination).unwrap();
	std::os::unix::fs::symlink(&destination, &path).unwrap();
	assert!(!run(window.json.as_bytes()).status.success());
	assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}
