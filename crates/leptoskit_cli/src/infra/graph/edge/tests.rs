use super::*;

fn website() -> LeptoskitConfig {
	toml::from_str(include_str!(
		"../../../../tests/fixtures/website-graph.toml"
	))
	.unwrap()
}

fn tunnel(config: &mut LeptoskitConfig) {
	let leptoskit_config::infra::InfraConfig::Graph(infra) = config.infra.as_mut().unwrap() else {
		panic!("graph")
	};
	let edge = infra.edge.as_mut().unwrap();
	edge.ingress = Ingress::CloudflareTunnel;
	edge.origin_firewall = leptoskit_config::deployment::OriginFirewall::Closed;
	config
		.services
		.get_mut("play-api")
		.unwrap()
		.deploy
		.as_mut()
		.unwrap()
		.ingress = Some(Ingress::CloudflareTunnel);
}

#[test]
fn tunnel_edges_keep_host_ownership_and_loopback_ingress() {
	let mut config = website();
	tunnel(&mut config);
	let plan = leptoskit_config::deployment::compile(&config)
		.unwrap()
		.unwrap();
	let edges = configurations(&config, &plan, None).unwrap();
	assert_eq!(edges.len(), 2);
	for edge in edges.values() {
		assert_eq!(edge["admin"]["disabled"], true);
		assert_eq!(
			edge["apps"]["http"]["servers"]["edge"]["listen"],
			json!(["127.0.0.1:8088"])
		);
		assert!(edge["apps"].get("tls").is_none());
	}
	let web = serde_json::to_string(&edges["web"]).unwrap();
	assert!(!web.contains("play-api.leptoskit.dev"));
	assert!(web.contains("frontends.docs/current/assets"));
	assert!(!web.contains("reverse_proxy"));
	let api = serde_json::to_string(&edges["compiler"]).unwrap();
	assert!(api.contains("127.0.0.1:4702"));
	assert!(api.contains("Set-Cookie"));
	assert!(!api.contains("file_server"));
}

#[test]
fn https_needs_ranges_and_explicit_certificate_files() {
	let config = website();
	let plan = leptoskit_config::deployment::compile(&config)
		.unwrap()
		.unwrap();
	assert!(configurations(&config, &plan, None).is_err());
	let ranges = Ranges::parse("173.245.48.0/20\n", "2400:cb00::/32\n").unwrap();
	let edges = configurations(&config, &plan, Some(&ranges)).unwrap();
	let server = &edges["web"]["apps"]["http"]["servers"]["edge"];
	assert_eq!(server["listen"], json!([":443"]));
	assert_eq!(server["strict_sni_host"], true);
	assert_eq!(server["trusted_proxies"]["ranges"], json!(ranges.all()));
	assert_eq!(
		edges["web"]["apps"]["tls"]["certificates"]["load_files"]
			.as_array()
			.unwrap()
			.len(),
		1
	);
}

#[test]
fn public_services_require_deliberate_ambient_cookie_trust() {
	let mut config = website();
	tunnel(&mut config);
	let plan = leptoskit_config::deployment::compile(&config)
		.unwrap()
		.unwrap();
	let isolated = configurations(&config, &plan, None).unwrap();
	assert!(isolated["compiler"].to_string().contains("Set-Cookie"));
	config.services.get_mut("play-api").unwrap().trust = Some(ServiceTrust::FirstParty);
	let trusted = configurations(&config, &plan, None).unwrap();
	assert!(!trusted["compiler"].to_string().contains("Set-Cookie"));
}

#[test]
fn reserved_listener_port_and_placeholder_mounts_are_refused() {
	let mut config = website();
	tunnel(&mut config);
	config.services.get_mut("play-api").unwrap().port = Some(8088);
	let plan = leptoskit_config::deployment::compile(&config)
		.unwrap()
		.unwrap();
	assert!(configurations(&config, &plan, None).is_err());
	config.services.get_mut("play-api").unwrap().port = Some(4702);
	config
		.graph_infra_mut()
		.unwrap()
		.frontends
		.get_mut("docs")
		.unwrap()
		.mount = "/{env.VALUE}/".into();
	let plan = leptoskit_config::deployment::compile(&config)
		.unwrap()
		.unwrap();
	assert!(configurations(&config, &plan, None).is_err());
}

#[test]
fn private_hosts_do_not_gain_an_edge_listener() {
	let mut config = website();
	config.graph_infra_mut().unwrap().frontends.clear();
	config.artifacts.clear();
	let service = config.services.get_mut("play-api").unwrap();
	service.deploy.as_mut().unwrap().domain = None;
	service.deploy.as_mut().unwrap().ingress = None;
	let plan = leptoskit_config::deployment::compile(&config)
		.unwrap()
		.unwrap();
	assert!(configurations(&config, &plan, None).unwrap().is_empty());
}

#[test]
#[ignore = "runs the real pinned Caddy executable and local HTTP/TLS upstreams"]
fn generated_edges_serve_only_owned_routes_and_verified_proxy_headers() {
	let caddy = std::env::var_os("LEPTOSKIT_TEST_CADDY")
		.expect("set LEPTOSKIT_TEST_CADDY to the checked Caddy executable");
	let root = tempfile::tempdir().unwrap();
	let mut config = website();
	let plan = leptoskit_config::deployment::compile(&config)
		.unwrap()
		.unwrap();
	let ranges = Ranges::parse("173.245.48.0/20\n", "2400:cb00::/32\n").unwrap();
	let https = configurations(&config, &plan, Some(&ranges)).unwrap();
	tunnel(&mut config);
	let plan = leptoskit_config::deployment::compile(&config)
		.unwrap()
		.unwrap();
	let http = configurations(&config, &plan, None).unwrap();
	for (name, port) in [("site", 3000), ("play", 3001)] {
		let frontend = config
			.graph_infra_mut()
			.unwrap()
			.frontends
			.get_mut(name)
			.unwrap();
		frontend.kind = FrontendKind::Ssr;
		frontend.backend = Some(
			if name == "site" {
				"server"
			} else {
				"play/server"
			}
			.into(),
		);
		frontend.serve = Some(leptoskit_config::ServeConfig {
			port,
			probes: true,
			..Default::default()
		});
	}
	let plan = leptoskit_config::deployment::compile(&config)
		.unwrap()
		.unwrap();
	let ssr = configurations(&config, &plan, None).unwrap();
	let document = json!({"caddy_version":CADDY_VERSION,"https":https,"tunnel":http,"ssr":ssr});
	let path = root.path().join("edges.json");
	std::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
	let status = std::process::Command::new("python3")
		.arg(concat!(
			env!("CARGO_MANIFEST_DIR"),
			"/tests/fixtures/graph-edge-acceptance.py"
		))
		.arg(caddy)
		.arg(&path)
		.status()
		.unwrap();
	assert!(status.success(), "real generated edge acceptance failed");
}
