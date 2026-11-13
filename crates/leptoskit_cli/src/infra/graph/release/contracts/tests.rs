use super::*;

fn config() -> LeptoskitConfig {
	toml::from_str(include_str!(
		"../../../../../tests/fixtures/website-graph.toml"
	))
	.unwrap()
}

fn backend(unit: &str) -> Executable {
	Executable {
		unit: unit.into(),
		service: None,
		backend: Member::File {
			mode: 0o755,
			size: 8,
			sha256: "a".repeat(64),
		},
	}
}

fn ssr(config: &mut LeptoskitConfig) {
	let site = config
		.graph_infra_mut()
		.unwrap()
		.frontends
		.get_mut("site")
		.unwrap();
	site.kind = FrontendKind::Ssr;
	site.backend = Some("server".into());
	site.serve = Some(leptoskit_config::ServeConfig {
		probes: true,
		address_env: Some("LEPTOS_SITE_ADDR".into()),
		..Default::default()
	});
}

#[test]
fn ssr_requires_explicit_socket_contract_and_preserves_exact_backend_pin() {
	let mut config = config();
	ssr(&mut config);
	let executable = backend("frontends.site");
	let value = contract(&config, &executable).unwrap();
	assert_eq!(
		value["environment"],
		json!({"LEPTOS_SITE_ADDR":"127.0.0.1:3000"})
	);
	assert_eq!(
		value["backend"],
		json!({"mode":0o755,"size":8,"sha256":"a".repeat(64)})
	);
	assert_ne!(value["account"], "leptoskit-edge");
	let serve = config
		.graph_infra_mut()
		.unwrap()
		.frontends
		.get_mut("site")
		.unwrap()
		.serve
		.as_mut()
		.unwrap();
	serve.address_env = None;
	assert!(
		contract(&config, &executable)
			.unwrap_err()
			.to_string()
			.contains("serve.address-env")
	);
}

#[test]
fn refuses_unrepresentable_ssr_contracts_before_transport() {
	for field in ["variable", "address", "mount", "port", "probes"] {
		let mut config = config();
		ssr(&mut config);
		let site = config
			.graph_infra_mut()
			.unwrap()
			.frontends
			.get_mut("site")
			.unwrap();
		let serve = site.serve.as_mut().unwrap();
		match field {
			"variable" => serve.address_env = Some("X\nUser=root".into()),
			"address" => serve.address = "0.0.0.0".into(),
			"mount" => site.mount = "/site/".into(),
			"port" => serve.port = 18088,
			"probes" => serve.probes = false,
			_ => unreachable!(),
		}
		assert!(
			contract(&config, &backend("frontends.site")).is_err(),
			"{field}"
		);
	}
}

fn service(config: &mut LeptoskitConfig) -> Executable {
	let declared = config.services.get_mut("play-api").unwrap();
	declared.runtime = None;
	declared.serve.as_mut().unwrap().address_env = Some("PLAY_BIND".into());
	let settings = leptoskit_config::ResolvedService::merge("play-api", declared, None).unwrap();
	let mut executable = backend("services.play-api");
	executable.service = Some(crate::verify::graph::service::ServiceEvidence {
		settings,
		package: "leptoskit_playground_server".into(),
		binary: "server".into(),
		version: "1.0.0".into(),
		profile: "release".into(),
		default_features: true,
		package_manifest: "Cargo.toml".into(),
		workspace_manifest: "Cargo.toml".into(),
		lock: "Cargo.lock".into(),
		inputs: BTreeMap::new(),
		sources: BTreeMap::new(),
	});
	executable
}

#[test]
fn service_uses_declared_variable_and_resolved_settings_without_guessing() {
	let mut config = config();
	let mut executable = service(&mut config);
	let value = contract(&config, &executable).unwrap();
	assert_eq!(value["environment"], json!({"PLAY_BIND":"127.0.0.1:4702"}));
	executable.service.as_mut().unwrap().settings.port = Some(8000);
	assert!(contract(&config, &executable).is_err());
	executable.service = None;
	assert!(contract(&config, &executable).is_err());
}

#[test]
fn resolved_manifest_requirements_cannot_be_silently_dropped() {
	for field in ["database", "pool", "volumes", "bind", "secrets", "runtime"] {
		let mut config = config();
		let mut executable = service(&mut config);
		let settings = &mut executable.service.as_mut().unwrap().settings;
		match field {
			"database" => settings.database = true,
			"pool" => settings.pool = Some(2),
			"volumes" => settings.volumes.push("files".into()),
			"bind" => {
				settings.bind.insert("API".into(), "{url}".into());
			}
			"secrets" => settings.secrets.push("TOKEN".into()),
			"runtime" => {
				config.services.get_mut("play-api").unwrap().runtime =
					self::config().services["play-api"].runtime.clone();
			}
			_ => unreachable!(),
		}
		assert!(contract(&config, &executable).is_err(), "{field}");
	}
}

#[test]
fn package_metadata_cannot_smuggle_alongside_app_routes_into_graph_release() {
	for routing in [
		"public = true",
		"routes = [\"/_legacy\"]",
		"public = true\nroutes = [\"/_legacy\"]",
	] {
		let mut config = config();
		let mut executable = service(&mut config);
		// Exercise the consumer's merged-metadata check independently of the
		// capture filter, which already refuses rooted legacy route strings.
		let package: toml::Value = toml::from_str(&format!(
			"[package]\nname = \"leptoskit_playground_server\"\nversion = \
			 \"1.0.0\"\n[package.metadata.leptoskit.service]\nname = \"play-api\"\n{routing}\n"
		))
		.unwrap();
		let metadata: leptoskit_config::services::ServiceManifest = package["package"]["metadata"]
			["leptoskit"]["service"]
			.clone()
			.try_into()
			.unwrap();
		executable.service.as_mut().unwrap().settings = leptoskit_config::ResolvedService::merge(
			"play-api",
			&config.services["play-api"],
			Some(&metadata),
		)
		.unwrap();
		let error = contract(&config, &executable).unwrap_err();
		assert!(
			error.to_string().contains("inherited public/routes"),
			"{error:#}"
		);
	}
}

#[test]
fn literal_environment_merges_metadata_and_project_values_without_listen_override() {
	let mut config = config();
	config
		.services
		.get_mut("play-api")
		.unwrap()
		.env
		.insert("MODE".into(), "project".into());
	let mut executable = service(&mut config);
	let package = crate::verify::graph::service::manifest(
		br#"
[package.metadata.leptoskit.service]
name = "play-api"
[package.metadata.leptoskit.service.env]
MODE = "package"
LABEL = "hello %n $HOME"
EMPTY = ""
"#,
	)
	.unwrap();
	let metadata: leptoskit_config::services::ServiceManifest = package["package"]["metadata"]
		["leptoskit"]["service"]
		.clone()
		.try_into()
		.unwrap();
	executable.service.as_mut().unwrap().settings = leptoskit_config::ResolvedService::merge(
		"play-api",
		&config.services["play-api"],
		Some(&metadata),
	)
	.unwrap();
	let value = contract(&config, &executable).unwrap();
	assert_eq!(
		value["environment"],
		json!({"MODE":"project", "LABEL":"hello %n $HOME", "EMPTY":"", "PLAY_BIND":"127.0.0.1:4702"})
	);
	let settings = &mut executable.service.as_mut().unwrap().settings;
	settings
		.env
		.insert("PLAY_BIND".into(), "127.0.0.1:4702".into());
	assert!(
		contract(&config, &executable)
			.unwrap_err()
			.to_string()
			.contains("must not override")
	);
}

#[test]
fn combined_environment_limits_include_the_generated_listener() {
	let mut config = config();
	let mut executable = service(&mut config);
	executable.service.as_mut().unwrap().settings.env =
		(0..128).map(|i| (format!("V{i}"), String::new())).collect();
	assert!(
		contract(&config, &executable)
			.unwrap_err()
			.to_string()
			.contains("128 entries")
	);
}
