use std::path::Path;
use std::process::Command;

fn command(config: &Path, operation: &str, out: Option<&Path>) -> std::process::Output {
	let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit-cloud-platform"));
	command.arg(operation).arg("--config").arg(config);
	if let Some(out) = out {
		command.arg("--out").arg(out);
	}
	command.output().unwrap()
}

#[test]
fn four_commands_and_ownership() {
	let root = tempfile::tempdir().unwrap();
	let config = root.path().join("config.toml");
	let out = root.path().join("generated");
	std::fs::write(&config, include_str!("../examples/launch.toml")).unwrap();
	assert!(command(&config, "check", None).status.success());
	let summary = command(&config, "plan-summary", None);
	assert!(summary.status.success());
	assert!(String::from_utf8_lossy(&summary.stdout).contains("single point of failure"));
	assert!(command(&config, "generate", Some(&out)).status.success());
	let unchanged = command(&config, "generate", Some(&out));
	assert!(unchanged.status.success());
	assert_eq!(String::from_utf8_lossy(&unchanged.stdout), "unchanged\n");
	assert!(command(&config, "bootstrap", Some(&out)).status.success());
	let file = out.join("eu/launch/main.tf.json");
	std::fs::write(&file, "foreign-sentinel").unwrap();
	let failed = command(&config, "generate", Some(&out));
	assert!(!failed.status.success());
	assert!(String::from_utf8_lossy(&failed.stderr).contains("foreign"));
	assert_eq!(std::fs::read_to_string(file).unwrap(), "foreign-sentinel");
}

#[test]
fn refuses_unowned_files_and_invalid_configuration() {
	let root = tempfile::tempdir().unwrap();
	let config = root.path().join("config.toml");
	std::fs::write(&config, include_str!("../examples/launch.toml")).unwrap();
	std::fs::create_dir_all(root.path().join("eu/launch")).unwrap();
	let file = root.path().join("eu/launch/main.tf.json");
	std::fs::write(&file, "foreign").unwrap();
	assert!(
		!command(&config, "generate", Some(root.path()))
			.status
			.success()
	);
	assert_eq!(std::fs::read_to_string(file).unwrap(), "foreign");
	std::fs::write(&config, "invalid secret-sentinel").unwrap();
	let result = command(&config, "check", None);
	assert!(!result.status.success());
	assert!(!String::from_utf8_lossy(&result.stderr).contains("secret-sentinel"));
}

#[test]
fn bootstrap_accepts_real_output_shape() {
	let root = tempfile::tempdir().unwrap();
	let config = root.path().join("config.toml");
	let outputs = root.path().join("outputs.json");
	std::fs::write(&config, include_str!("../examples/growth.toml")).unwrap();
	std::fs::write(&outputs, include_str!("growth-outputs.json")).unwrap();
	assert!(
		!command(&config, "bootstrap", Some(root.path()))
			.status
			.success()
	);
	let result = Command::new(env!("CARGO_BIN_EXE_leptoskit-cloud-platform"))
		.args(["bootstrap", "--config"])
		.arg(config)
		.arg("--outputs")
		.arg(outputs)
		.arg("--out")
		.arg(root.path())
		.output()
		.unwrap();
	assert!(
		result.status.success(),
		"{}",
		String::from_utf8_lossy(&result.stderr)
	);
}

#[cfg(unix)]
#[test]
fn refuses_symlink_paths() {
	let root = tempfile::tempdir().unwrap();
	let other = tempfile::tempdir().unwrap();
	let config = root.path().join("config.toml");
	std::fs::write(&config, include_str!("../examples/launch.toml")).unwrap();
	std::os::unix::fs::symlink(other.path(), root.path().join("eu")).unwrap();
	assert!(
		!command(&config, "generate", Some(root.path()))
			.status
			.success()
	);
	assert_eq!(std::fs::read_dir(other.path()).unwrap().count(), 0);
}
