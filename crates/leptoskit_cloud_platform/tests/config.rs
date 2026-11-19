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
fn verified_gvisor_pin_defaults_are_coupled() {
	use leptoskit_cloud_platform::config::GVISOR_RELEASE;
	use leptoskit_cloud_platform::config::GVISOR_SHA512;
	let input = include_str!("../examples/launch.toml");
	let input = input
		.lines()
		.filter(|line| !line.starts_with("release =") && !line.starts_with("sha512 ="))
		.collect::<Vec<_>>()
		.join("\n");
	let mut config = PlatformConfig::parse(&input).unwrap();
	assert_eq!(config.isolation.release, GVISOR_RELEASE);
	assert_eq!(config.isolation.sha512, GVISOR_SHA512);
	config.isolation.sha512 = "a".repeat(128);
	assert!(config.validate().is_err());
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

#[test]
fn required_values_and_rendering_boundaries() {
	let config = PlatformConfig::parse(include_str!("../examples/launch.toml")).unwrap();
	let original = serde_json::to_value(config).unwrap();
	for (pointer, invalid) in [
		("/name", serde_json::json!("")),
		("/apps_domain", serde_json::json!("invalid")),
		("/default_region", serde_json::json!("missing")),
		("/admin_cidrs", serde_json::json!([])),
		("/admin_cidrs", serde_json::json!(["2001:db8::/32"])),
		("/admin_cidrs", serde_json::json!(["192.0.2.1/24"])),
		("/admin_cidrs", serde_json::json!(["192.0.2.0/33"])),
		("/isolation/runtime_class", serde_json::json!("runc")),
		("/isolation/sha512", serde_json::json!("unverified")),
		("/isolation/release", serde_json::json!("latest")),
		("/isolation/release", serde_json::json!("20261116.x")),
		(
			"/isolation/denied_egress_cidrs",
			serde_json::json!(["not-a-cidr"]),
		),
		(
			"/object_storage/endpoint",
			serde_json::json!("http://s3.example.com"),
		),
		("/object_storage/sources", serde_json::json!("${unsafe}")),
		(
			"/object_storage/credentials/access_key",
			serde_json::json!("bad/key"),
		),
		("/postgres/retention_days", serde_json::json!(0)),
		("/postgres/wal_bucket", serde_json::json!("different")),
		("/postgres/storage_size", serde_json::json!("0Gi")),
		("/edge/account_id", serde_json::json!("invalid")),
		("/edge/zone_id", serde_json::json!("invalid")),
		("/edge/cluster", serde_json::json!("missing")),
		("/addons/keda", serde_json::json!("latest")),
		("/addons/acme_email", serde_json::json!("injected\nline")),
		(
			"/regions/eu/clusters/launch/version",
			serde_json::json!("v0.0.0"),
		),
		(
			"/regions/eu/clusters/launch/service_cidr",
			serde_json::json!("2001:db8::/32"),
		),
		(
			"/regions/eu/clusters/launch/registration_address",
			serde_json::json!("https://bad"),
		),
		(
			"/regions/eu/clusters/launch/nodes/box/public_address",
			serde_json::json!("10.0.0.1"),
		),
		(
			"/regions/eu/clusters/launch/nodes/box/role",
			serde_json::json!("agent"),
		),
	] {
		let mut changed = original.clone();
		*changed.pointer_mut(pointer).unwrap() = invalid;
		let changed: PlatformConfig = serde_json::from_value(changed).unwrap();
		assert!(changed.validate().is_err(), "{pointer}");
	}
}

#[test]
fn cloud_private_network_and_node_groups_are_checked() {
	let config = PlatformConfig::parse(include_str!("../examples/growth.toml")).unwrap();
	let original = serde_json::to_value(config).unwrap();
	for (pointer, invalid) in [
		(
			"/regions/eu/clusters/growth/private_cidr",
			serde_json::json!(null),
		),
		(
			"/regions/eu/clusters/growth/private_cidr",
			serde_json::json!("10.42.0.0/16"),
		),
		(
			"/regions/eu/clusters/growth/private_cidr",
			serde_json::json!("192.0.2.0/24"),
		),
		(
			"/regions/eu/clusters/growth/network_zone",
			serde_json::json!(null),
		),
		(
			"/regions/eu/clusters/growth/nodes/control/count",
			serde_json::json!(0),
		),
		(
			"/regions/eu/clusters/growth/nodes/control/count",
			serde_json::json!(101),
		),
		(
			"/regions/eu/clusters/growth/nodes/control/ssh_key_ids",
			serde_json::json!([]),
		),
		(
			"/regions/eu/clusters/growth/nodes/control/server_type",
			serde_json::json!("bad/type"),
		),
		(
			"/regions/eu/clusters/growth/nodes/control/location",
			serde_json::json!("bad/location"),
		),
		(
			"/regions/eu/clusters/growth/nodes/tenant-b/public_address",
			serde_json::json!("203.0.113.10"),
		),
	] {
		let mut changed = original.clone();
		*changed.pointer_mut(pointer).unwrap() = invalid;
		let changed: PlatformConfig = serde_json::from_value(changed).unwrap();
		assert!(changed.validate().is_err(), "{pointer}");
	}
}

#[test]
fn existing_ha_requires_private_addresses_and_one_provider() {
	let mut config = PlatformConfig::parse(include_str!("../examples/launch.toml")).unwrap();
	let nodes = &mut config
		.regions
		.get_mut("eu")
		.unwrap()
		.clusters
		.get_mut("launch")
		.unwrap()
		.nodes;
	for (name, address) in [("second", "203.0.113.11"), ("third", "203.0.113.12")] {
		nodes.insert(
			name.into(),
			leptoskit_cloud_platform::config::Node::Existing {
				role: leptoskit_cloud_platform::config::Role::Server,
				pool: leptoskit_cloud_platform::config::Pool::Platform,
				public_address: address.parse().unwrap(),
				private_address: None,
				isolation_platform: None,
			},
		);
	}
	assert!(
		config
			.validate()
			.unwrap_err()
			.to_string()
			.contains("private network")
	);
	for (index, node) in config
		.regions
		.get_mut("eu")
		.unwrap()
		.clusters
		.get_mut("launch")
		.unwrap()
		.nodes
		.values_mut()
		.enumerate()
	{
		if let leptoskit_cloud_platform::config::Node::Existing {
			private_address, ..
		} = node
		{
			*private_address = Some(format!("10.44.0.{}", index + 10).parse().unwrap());
		}
	}
	config.validate().unwrap();
}

#[test]
fn cold_storage_max_connections_and_per_node_isolation() {
	use leptoskit_cloud_platform::config::ColdObjectStorage;
	use leptoskit_cloud_platform::config::CredentialRef;
	use leptoskit_cloud_platform::config::GvisorPlatform;
	use leptoskit_cloud_platform::config::Node;

	let mut config = PlatformConfig::parse(include_str!("../examples/growth.toml")).unwrap();
	// Valid hot/cold split (e.g. R2 hot + B2 cold for WAL/backups).
	config.object_storage.cold = Some(ColdObjectStorage {
		endpoint: "https://s3.us-west-004.backblazeb2.com".into(),
		region: "us-west-004".into(),
		credentials: CredentialRef::KubernetesSecret {
			name: "b2-backups".into(),
			access_key: "ACCESS_KEY_ID".into(),
			secret_key: "SECRET_ACCESS_KEY".into(),
		},
	});
	config.validate().unwrap();
	let (endpoint, region, _) = config.object_storage.backups_store();
	assert_eq!(endpoint, "https://s3.us-west-004.backblazeb2.com");
	assert_eq!(region, "us-west-004");

	// Cold endpoint must be HTTPS.
	config.object_storage.cold.as_mut().unwrap().endpoint = "http://insecure.example.com".into();
	assert!(config.validate().is_err());
	config.object_storage.cold = None;

	// max_connections below 100 is rejected.
	config.postgres.max_connections = 50;
	assert!(config.validate().is_err());
	config.postgres.max_connections = 300;
	config.validate().unwrap();

	// Cloud VM cannot override to kvm, while existing bare-metal nodes can.
	if let Node::HetznerCloud {
		isolation_platform, ..
	} = config
		.regions
		.get_mut("eu")
		.unwrap()
		.clusters
		.get_mut("growth")
		.unwrap()
		.nodes
		.get_mut("control")
		.unwrap()
	{
		*isolation_platform = Some(GvisorPlatform::Kvm);
	}
	assert!(config.validate().is_err());
}
