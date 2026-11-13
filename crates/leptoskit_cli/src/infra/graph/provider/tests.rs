use leptoskit_config::LeptoskitConfig;
use leptoskit_config::deployment;
use leptoskit_config::deployment::OriginFirewall;

use super::*;

fn website() -> LeptoskitConfig {
	toml::from_str(include_str!(
		"../../../../tests/fixtures/website-graph.toml"
	))
	.unwrap()
}

fn graph(config: &mut LeptoskitConfig) -> &mut InfraGraph {
	match config.infra.as_mut().unwrap() {
		leptoskit_config::infra::InfraConfig::Graph(graph) => graph,
		leptoskit_config::infra::InfraConfig::Managed(_) => panic!("graph fixture required"),
	}
}

fn render(config: &LeptoskitConfig, ranges: Option<&Ranges>) -> Value {
	config.validate().unwrap();
	let plan = deployment::compile(config).unwrap().unwrap();
	resources(
		config.infra.as_ref().unwrap().as_graph().unwrap(),
		&plan,
		ranges,
	)
	.unwrap()
}

fn ranges() -> Ranges {
	Ranges::parse("173.245.48.0/20", "2400:cb00::/32").unwrap()
}

#[test]
fn https_places_domains_on_their_own_hosts_and_deduplicates_path_mounts() {
	let output = render(&website(), Some(&ranges()));
	let domains = output["locals"]["public_domains"].as_object().unwrap();
	assert_eq!(domains.len(), 2);
	assert_eq!(
		domains["leptoskit.dev"]["ipv4"],
		"${hcloud_server.host[\"web\"].ipv4_address}"
	);
	assert_eq!(
		domains["play-api.leptoskit.dev"]["ipv6"],
		"${hcloud_server.host[\"compiler\"].ipv6_address}"
	);
	assert_eq!(
		output["locals"]["hetzner_hosts"]["compiler"]["server_type"],
		"cpx32"
	);
	assert_eq!(
		output["locals"]["origin_cidrs"],
		json!(["173.245.48.0/20", "2400:cb00::/32"])
	);
	assert_eq!(
		output["resource"]["cloudflare_dns_record"]["ipv4"]["proxied"],
		true
	);

	assert_eq!(
		output["resource"]["cloudflare_dns_record"]["ipv6"]["type"],
		"AAAA"
	);
	assert_eq!(
		output["resource"]["hcloud_server"]["host"]["lifecycle"]["prevent_destroy"],
		true
	);
	assert!(output["resource"].get("cloudflare_ruleset").is_none());
	assert!(output["resource"].get("cloudflare_zone_setting").is_none());
	assert!(
		output["terraform"]["required_providers"]
			.get("digitalocean")
			.is_none()
	);
}

#[test]
fn https_requires_the_independent_range_snapshot() {
	let config = website();
	let plan = deployment::compile(&config).unwrap().unwrap();
	assert!(
		resources(
			config.infra.as_ref().unwrap().as_graph().unwrap(),
			&plan,
			None
		)
		.is_err()
	);
}

#[test]
fn mixed_providers_keep_keys_addresses_and_placement_separate() {
	let mut config = website();
	let infra = graph(&mut config);
	infra.hosts.get_mut("web").unwrap().ssh_key_ids = vec![123];
	let compiler = infra.hosts.get_mut("compiler").unwrap();
	compiler.compute = ComputeProvider::Digitalocean;
	compiler.location = "ams3".into();
	compiler.server_type = "c-4".into();
	compiler.ssh_key_ids = vec![456];

	let output = render(&config, Some(&ranges()));
	assert_eq!(
		output["locals"]["digitalocean_hosts"]["compiler"]["ssh_keys"],
		json!(["456"])
	);
	assert_eq!(
		output["locals"]["hetzner_hosts"]["web"]["ssh_keys"],
		json!(["123"])
	);
	assert!(output["locals"]["hetzner_hosts"].get("compiler").is_none());
	assert_eq!(
		output["locals"]["public_domains"]["play-api.leptoskit.dev"]["ipv4"],
		"${digitalocean_droplet.host[\"compiler\"].ipv4_address}"
	);
	assert_eq!(
		output["output"]["hosts"]["value"]["compiler"]["ipv6"],
		"${digitalocean_droplet.host[\"compiler\"].ipv6_address}"
	);
}

#[test]
fn tunnel_closes_public_https_and_marks_tokens_sensitive() {
	let mut config = website();
	let edge = graph(&mut config).edge.as_mut().unwrap();
	edge.ingress = Ingress::CloudflareTunnel;
	edge.origin_firewall = OriginFirewall::Closed;
	config
		.services
		.get_mut("play-api")
		.unwrap()
		.deploy
		.as_mut()
		.unwrap()
		.ingress = Some(Ingress::CloudflareTunnel);
	let output = render(&config, None);
	assert_eq!(
		output["locals"]["hetzner_hosts"]["compiler"]["public_https"],
		false
	);
	assert_eq!(
		output["locals"]["hetzner_hosts"]["web"]["public_https"],
		false
	);

	assert_eq!(output["locals"]["origin_cidrs"], json!([]));
	assert_eq!(output["output"]["tunnel_tokens"]["sensitive"], true);
	assert!(
		output["resource"]
			.get("cloudflare_origin_ca_certificate")
			.is_none()
	);
	assert_eq!(
		output["resource"]["cloudflare_dns_record"]["tunnel"]["type"],
		"CNAME"
	);
}

#[test]
fn static_only_has_no_application_database_or_backup_resources() {
	let mut config = website();
	config.services.clear();
	config
		.graph_infra()
		.context("graph infrastructure required")?
		.frontends
		.get_mut("play")
		.unwrap()
		.calls
		.clear();
	graph(&mut config).hosts.remove("compiler");
	let output = render(&config, Some(&ranges()));
	assert_eq!(
		output["locals"]["hetzner_hosts"].as_object().unwrap().len(),
		1
	);
	let resources = output["resource"].as_object().unwrap();
	assert_eq!(
		resources
			.keys()
			.map(String::as_str)
			.collect::<BTreeSet<_>>(),
		BTreeSet::from([
			"hcloud_server",
			"hcloud_firewall",
			"cloudflare_dns_record",
			"cloudflare_origin_ca_certificate"
		])
	);

	assert!(!CLOUD_INIT.contains("postgres"));
	assert!(!CLOUD_INIT.contains("restic"));
}

#[test]
fn private_service_has_no_cloudflare_resources_or_listener() {
	let mut config = website();
	config
		.graph_infra()
		.context("graph infrastructure required")?
		.frontends
		.clear();
	config.artifacts.clear();
	let service = config
		.services
		.get_mut("play-api")
		.unwrap()
		.deploy
		.as_mut()
		.unwrap();
	service.domain = None;
	service.ingress = None;
	let infra = graph(&mut config);
	// A retained project edge declaration does not make this service public.
	infra.hosts.remove("web");

	let output = render(&config, None);
	assert!(
		output["terraform"]["required_providers"]
			.get("cloudflare")
			.is_none()
	);
	assert_eq!(
		output["locals"]["hetzner_hosts"]["compiler"]["public_https"],
		false
	);
	assert_eq!(output["output"]["domains_by_host"]["value"], json!({}));
}

#[test]
fn provider_names_retain_identity_after_normalization_and_truncation() {
	let names = [
		resource_name("project", "API"),
		resource_name("project", "api"),
		resource_name("project", "a_b"),
		resource_name("project", "a-b"),
		resource_name(&"a".repeat(63), &"b".repeat(63)),
		resource_name(&"a".repeat(63), &format!("{}c", "b".repeat(62))),
	];
	assert_eq!(names.iter().collect::<BTreeSet<_>>().len(), names.len());

	for name in names {
		assert!(name.len() + "-origin".len() <= 63);
		assert!(
			name.bytes()
				.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
		);
	}
}

#[test]
fn generated_accounts_match_release_identity_and_stay_on_their_host() {
	let config = website();
	let output = render(&config, Some(&ranges()));
	let compiler = super::super::account(
		"leptoskit-web",
		&leptoskit_config::graph::Unit::Service("play-api".into()),
	);
	let site = super::super::account(
		"leptoskit-web",
		&leptoskit_config::graph::Unit::Frontend("site".into()),
	);
	let data = |host: &str| {
		output["locals"]["hetzner_hosts"][host]["user_data"]
			.as_str()
			.unwrap()
	};
	assert!(data("compiler").contains(&compiler));
	assert!(!data("web").contains(&compiler));
	assert!(data("web").contains(&site));
	assert!(!data("compiler").contains(&site));
	assert!(data("compiler").contains("leptoskit-edge"));
	let users: Value =
		serde_json::from_str(data("compiler").split_once("users: ").unwrap().1.trim()).unwrap();
	assert_eq!(users[0], "default");
	for user in users.as_array().unwrap().iter().skip(1) {
		assert_eq!(user["system"], true);
		assert_eq!(user["lock_passwd"], true);
		assert_eq!(user["no_create_home"], true);
		assert_eq!(user["shell"], "/usr/sbin/nologin");
	}
	assert_eq!(
		output["resource"]["hcloud_server"]["host"]["user_data"],
		"${each.value.user_data}"
	);
	assert_ne!(
		compiler,
		super::super::account(
			"other-project",
			&leptoskit_config::graph::Unit::Service("play-api".into())
		)
	);
}
