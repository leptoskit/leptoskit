use std::fmt::Write as _;

use leptoskit_cloud_platform::bootstrap;
use leptoskit_cloud_platform::config::PlatformConfig;

#[test]
fn snapshots_and_shell_syntax() {
	for (name, input, outputs) in [
		("launch", include_str!("../examples/launch.toml"), None),
		(
			"growth",
			include_str!("../examples/growth.toml"),
			Some(include_str!("growth-outputs.json")),
		),
	] {
		let config = PlatformConfig::parse(input).unwrap();
		let generated = bootstrap::generate(&config, outputs).unwrap();
		assert_eq!(generated, bootstrap::generate(&config, outputs).unwrap());
		let mut snapshot = String::new();
		for (path, content) in &generated {
			if std::path::Path::new(path)
				.extension()
				.is_some_and(|e| e == "sh")
			{
				assert!(content.contains("[Service]\nType=notify\n"));
				assert!(content.contains("Delegate=yes\nLimitNOFILE=1048576\n"));
			}
			writeln!(snapshot, "=== {path} ===\n{content}\n").unwrap();
			assert!(!content.contains("secret-sentinel"));
			if std::path::Path::new(path)
				.extension()
				.is_some_and(|e| e == "sh")
			{
				let directory = tempfile::tempdir().unwrap();
				let file = directory.path().join("bootstrap.sh");
				std::fs::write(&file, content).unwrap();
				assert!(
					std::process::Command::new("sh")
						.arg("-n")
						.arg(file)
						.status()
						.unwrap()
						.success()
				);
			}
		}
		insta::assert_snapshot!(format!("bootstrap_{name}"), snapshot);
	}
}

#[test]
fn cloud_outputs_are_required_and_checked() {
	let config = PlatformConfig::parse(include_str!("../examples/growth.toml")).unwrap();
	for output in [
		None,
		Some("{}"),
		Some("secret-sentinel"),
		Some("{\"node_addresses\":{\"value\":{\"control-0\":\"invalid\"}}}"),
	] {
		let error = bootstrap::generate(&config, output).unwrap_err();
		assert!(!error.to_string().contains("secret-sentinel"));
	}
}

#[test]
fn firewall_has_exact_public_wireguard_peers() {
	let config = PlatformConfig::parse(include_str!("../examples/growth.toml")).unwrap();
	let files = bootstrap::generate(&config, Some(include_str!("growth-outputs.json"))).unwrap();
	let peers = [
		"192.0.2.10",
		"192.0.2.11",
		"192.0.2.12",
		"198.51.100.10",
		"203.0.113.10",
		"203.0.113.11",
		"203.0.113.12",
	];
	let script = &files["eu/growth/control-0.sh"];
	let actual: Vec<_> = script
		.lines()
		.filter(|s| s.contains("udp dport 51820"))
		.collect();
	assert_eq!(actual.len(), peers.len() - 1);
	for peer in peers.iter().skip(1) {
		assert!(
			actual
				.iter()
				.any(|line| line.contains(&format!("{peer}/32")))
		);
	}
	assert!(!script.contains("tcp dport 443"));
	assert!(files["eu/growth/control-1.sh"].contains("https://10.44.0.11:6443"));
	assert!(files["eu/growth/tenant-a-0.sh"].contains("https://192.0.2.10:6443"));
}
