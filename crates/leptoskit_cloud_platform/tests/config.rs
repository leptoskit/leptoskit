use leptoskit_cloud_platform::config::PlatformConfig;

#[test]
fn examples_and_defaults_round_trip() {
	for input in [
		include_str!("../examples/launch.toml"),
		include_str!("../examples/growth.toml"),
	] {
		let config = PlatformConfig::parse(input).unwrap();
		let encoded = toml::to_string(&config).unwrap();
		assert_eq!(
			encoded,
			toml::to_string(&PlatformConfig::parse(&encoded).unwrap()).unwrap()
		);
	}
}

#[test]
fn invalid_inputs_fail_without_echoing_values() {
	let original = include_str!("../examples/launch.toml");
	for (from, to) in [
		("name = \"cloud-example\"", "name = \"../bad\""),
		("https://console.example.com", "http://console.example.com"),
		("10.43.0.0/16", "10.42.0.0/16"),
		("192.0.2.0/24", "0.0.0.0/0"),
		("distribution = \"k3s\"", "distribution = \"other\""),
		("203.0.113.10", "127.0.0.1"),
		("platform = \"systrap\"", "platform = \"unsafe\""),
		("instances = 1", "instances = 0"),
		("mode = \"tunnel\"", "mode = \"https\""),
		("kind = \"kubernetes-secret\"", "kind = \"plaintext\""),
	] {
		assert!(
			PlatformConfig::parse(&original.replace(from, to)).is_err(),
			"{from}"
		);
	}
	let error =
		PlatformConfig::parse(&format!("password = 'secret-sentinel'\n{original}")).unwrap_err();
	assert!(!format!("{error:?}").contains("secret-sentinel"));
}

#[test]
fn ha_and_virtualization_constraints() {
	let original = include_str!("../examples/growth.toml");
	assert!(PlatformConfig::parse(&original.replace("count = 3", "count = 2")).is_err());
	assert!(
		PlatformConfig::parse(&original.replace("platform = \"systrap\"", "platform = \"kvm\""))
			.unwrap_err()
			.to_string()
			.contains("nested virtualization")
	);
}
