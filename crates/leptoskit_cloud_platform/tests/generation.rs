use leptoskit_cloud_platform::config::PlatformConfig;
use leptoskit_cloud_platform::tofu;

#[test]
fn tofu_examples_are_deterministic() {
	for (name, input) in [
		("launch", include_str!("../examples/launch.toml")),
		("growth", include_str!("../examples/growth.toml")),
	] {
		let config = PlatformConfig::parse(input).unwrap();
		let files = tofu::generate(&config).unwrap();
		assert_eq!(files, tofu::generate(&config).unwrap());
		for content in files.values() {
			insta::assert_snapshot!(format!("tofu_{name}"), content);
		}
		for content in files.values() {
			assert!(!content.contains("secret-sentinel"));
		}
	}
}

#[test]
fn https_only_opens_ingress_and_uses_official_ranges() {
	use leptoskit_cloud_platform::config::CLOUDFLARE_CIDRS;
	use leptoskit_cloud_platform::config::EdgeMode;
	use leptoskit_cloud_platform::config::Node;
	use leptoskit_cloud_platform::config::Pool;
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
	let files = tofu::generate(&config).unwrap();
	let doc: serde_json::Value = serde_json::from_str(files.values().next().unwrap()).unwrap();
	assert!(
		doc["resource"]
			.get("cloudflare_zero_trust_tunnel_cloudflared")
			.is_none()
	);
	assert_eq!(
		doc["resource"]["cloudflare_dns_record"]["domain_0"]["type"],
		"A"
	);
	let boot = leptoskit_cloud_platform::bootstrap::generate(&config, None).unwrap();
	assert!(boot["eu/launch/box-0.sh"].contains("ip saddr 173.245.48.0/20 tcp dport 443"));
	config.edge.cloudflare_cidrs = vec!["0.0.0.0/1".into(), "8000::/1".into()];
	assert!(config.validate().is_err());
}

#[test]
#[ignore = "requires OpenTofu and provider downloads, never provisions resources"]
fn tofu_validate() {
	let tofu = std::env::var_os("LEPTOSKIT_TEST_TOFU").unwrap_or_else(|| "tofu".into());
	if std::process::Command::new(&tofu)
		.arg("version")
		.output()
		.is_err()
	{
		eprintln!("OpenTofu unavailable; acceptance skipped");
		return;
	}
	for input in [
		include_str!("../examples/launch.toml"),
		include_str!("../examples/growth.toml"),
	] {
		let root = tempfile::tempdir().unwrap();
		for (path, content) in tofu::generate(&PlatformConfig::parse(input).unwrap()).unwrap() {
			let path = root.path().join(path);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(&path, content).unwrap();
			for args in [
				vec!["init", "-backend=false", "-input=false", "-no-color"],
				vec!["validate", "-no-color"],
			] {
				let output = std::process::Command::new(&tofu)
					.args(&args)
					.current_dir(path.parent().unwrap())
					.output()
					.unwrap();
				assert!(
					output.status.success(),
					"{args:?}\n{}\n{}",
					String::from_utf8_lossy(&output.stdout),
					String::from_utf8_lossy(&output.stderr)
				);
			}
		}
	}
}
