//! Declaration and backward-compatibility tests, without provider or runtime
//! access.

use leptoskit_config::LeptoskitConfig;
use leptoskit_config::deployment;
use leptoskit_config::graph::Unit;
use leptoskit_config::infra::InfraConfig;
use leptoskit_config::infra::ManagedInfraConfig;

const WEBSITE: &str = include_str!("fixtures/website-graph.toml");

fn website() -> LeptoskitConfig {
	toml::from_str(WEBSITE).unwrap()
}

fn plan(config: &LeptoskitConfig) -> leptoskit_config::graph::plan::Plan {
	config.validate().unwrap();
	deployment::compile(config).unwrap().unwrap()
}

fn refuses(config: &LeptoskitConfig, message: &str) {
	let error = config.validate().unwrap_err().to_string();
	assert!(error.contains(message), "expected {message:?}: {error}");
}

#[test]
fn website_has_four_units_and_no_implicit_database() {
	let config = website();
	let graph = plan(&config);
	assert_eq!(graph.placements().len(), 4);
	assert_eq!(graph.hosts().len(), 2);
	assert!(config.managed_infra().is_none());
	assert!(config.db.is_none());
	let encoded = toml::to_string(&config).unwrap();
	for forbidden in [
		"[infra.postgres]",
		"[infra.backups]",
		"backup_bucket",
		"[db]",
	] {
		assert!(!encoded.contains(forbidden));
	}

	let decoded: LeptoskitConfig = toml::from_str(&encoded).unwrap();
	assert_eq!(config, decoded);
	assert_eq!(graph, plan(&decoded));
}

#[test]
fn graph_root_cannot_be_treated_as_a_single_application() {
	let config = website();
	let error = config.ensure_application_root().unwrap_err().to_string();
	assert!(error.contains("deployment graph root"));
	assert!(error.contains("lk infra graph"));
	config.validate().unwrap();
	LeptoskitConfig::default()
		.ensure_application_root()
		.unwrap();
}

#[test]
fn docs_and_compiler_release_selections_are_independent() {
	let graph = plan(&website());
	let docs = graph.select(&[Unit::Frontend("docs".into())]).unwrap();
	assert_eq!(docs.artifacts, ["public-site"]);
	assert_eq!(docs.stages, [vec![Unit::Frontend("docs".into())]]);
	let compiler = graph.select(&[Unit::Service("play-api".into())]).unwrap();
	assert_eq!(
		compiler.artifacts,
		["service-play-api", "toolchain-play-api"]
	);
	assert_eq!(compiler.stages, [vec![Unit::Service("play-api".into())]]);
}

#[test]
fn static_only_project_has_no_server_or_service_requirement() {
	let mut config = website();
	config.services.clear();
	config.frontends.get_mut("play").unwrap().calls.clear();
	let InfraConfig::Graph(infra) = config.infra.as_mut().unwrap() else {
		panic!()
	};
	infra.hosts.remove("compiler");
	assert_eq!(plan(&config).placements().len(), 3);
	assert!(!config.backend_enabled(false).unwrap());
}

#[test]
fn ssr_frontend_owns_its_backend_and_readiness() {
	let mut config = website();
	let site = config.frontends.get_mut("site").unwrap();
	site.kind = deployment::FrontendKind::Ssr;
	site.backend = Some("server".into());
	site.serve = Some(leptoskit_config::ServeConfig {
		probes: true,
		..Default::default()
	});
	plan(&config);
	config
		.frontends
		.get_mut("site")
		.unwrap()
		.serve
		.as_mut()
		.unwrap()
		.probes = false;
	refuses(&config, "readiness contract");
}

#[test]
fn static_frontend_rejects_inapplicable_process_fields() {
	let mut config = website();
	config.frontends.get_mut("site").unwrap().backend = Some("server".into());
	refuses(&config, "static and cannot declare");
}

#[test]
fn ports_are_scoped_to_the_placed_host() {
	let mut config = website();
	let mut second = config.services["play-api"].clone();
	second.runtime = None;
	second.deploy.as_mut().unwrap().host = "web".into();
	second.deploy.as_mut().unwrap().domain = Some("other.leptoskit.dev".into());
	config.services.insert("other".into(), second);
	plan(&config);
	config
		.services
		.get_mut("other")
		.unwrap()
		.deploy
		.as_mut()
		.unwrap()
		.host = "compiler".into();

	refuses(&config, "both use port 4702");
}

#[test]
fn rootless_compiler_requires_exclusive_placement() {
	let mut config = website();
	let InfraConfig::Graph(infra) = config.infra.as_mut().unwrap() else {
		panic!()
	};
	infra.hosts.get_mut("compiler").unwrap().exclusive = false;
	refuses(&config, "exclusive");
}

#[test]
fn runtime_bindings_cannot_be_shadowed_or_alias_each_other() {
	let mut config = website();
	config
		.services
		.get_mut("play-api")
		.unwrap()
		.env
		.insert("PLAY_IMAGE".into(), "mutable:latest".into());
	refuses(&config, "unshadowed");
	let service = config.services.get_mut("play-api").unwrap();
	service.env.clear();
	service.runtime.as_mut().unwrap().container.engine_env = "PLAY_IMAGE".into();
	refuses(&config, "distinct");
}

#[test]
fn service_data_and_root_application_defaults_are_not_inherited() {
	let mut config = website();
	config.services.get_mut("play-api").unwrap().database = None;
	refuses(&config, "database=false");
	config.services.get_mut("play-api").unwrap().database = Some(true);
	refuses(&config, "database=false");
	config = website();
	config.build.backend = None;
	refuses(&config, "build.backend = false");
}

#[test]
fn graph_fields_cannot_be_silently_ignored_by_legacy_mode() {
	let mut config = website();
	config.infra = None;
	refuses(&config, "infra.mode");
	assert!(
		toml::from_str::<LeptoskitConfig>(
			&WEBSITE.replace("mode = \"graph\"", "mode = \"managed\"")
		)
		.is_err()
	);
	assert!(toml::from_str::<LeptoskitConfig>(&WEBSITE.replace("mode = \"graph\"", "")).is_err());
	assert!(
		toml::from_str::<LeptoskitConfig>(&WEBSITE.replace(
			"mode = \"graph\"",
			"mode = \"graph\"\nbackup_bucket = \"legacy\""
		))
		.is_err()
	);
}

#[test]
fn tunnel_and_https_firewall_modes_cannot_be_conflated() {
	let mut config = website();
	let InfraConfig::Graph(infra) = config.infra.as_mut().unwrap() else {
		panic!()
	};
	infra.edge.as_mut().unwrap().ingress = deployment::Ingress::CloudflareTunnel;
	refuses(&config, "firewall");
}

#[test]
fn producers_cannot_overwrite_each_others_outputs() {
	let mut config = website();
	let mut recipe = config.artifacts["public-site"].clone();
	recipe.output = "crates/leptoskit_app/dist/docs".into();
	config.artifacts.insert("second".into(), recipe);
	refuses(&config, "output overlaps");
}

#[test]
fn output_and_pin_paths_must_have_unambiguous_project_spelling() {
	for invalid in [
		"../dist",
		"/tmp/dist",
		"dist/../other",
		"dist//docs",
		"dist/./docs",
		"dist\\docs",
		"dist\n",
	] {
		let mut config = website();
		config.artifacts.get_mut("public-site").unwrap().output = invalid.into();
		refuses(&config, "relative directory");
		let mut config = website();
		config
			.services
			.get_mut("play-api")
			.unwrap()
			.runtime
			.as_mut()
			.unwrap()
			.container
			.image_pin = invalid.into();
		refuses(&config, "image-pin");
	}
}

#[test]
fn derived_artifact_names_preserve_the_complete_service_name() {
	let mut config = website();
	let service = config.services.remove("play-api").unwrap();
	let name = "a".repeat(63);
	config.services.insert(name.clone(), service);
	config.frontends.get_mut("play").unwrap().calls = vec![format!("services.{name}")];
	let graph = plan(&config);
	assert!(graph.artifacts().contains(&format!("toolchain-{name}")));
}

#[test]
fn graph_references_and_readiness_cycles_fail_before_side_effects() {
	let mut config = website();
	config
		.frontends
		.get_mut("docs")
		.unwrap()
		.depends_on
		.push("services.missing".into());
	refuses(&config, "missing");
	config.frontends.get_mut("docs").unwrap().depends_on = vec!["frontends.site".into()];
	config
		.frontends
		.get_mut("site")
		.unwrap()
		.depends_on
		.push("frontends.docs".into());
	refuses(&config, "cycle");
}

#[test]
fn managed_configuration_serialization_has_no_new_wrapper_or_fields() {
	let original = r#"
name = "legacy"
domain = "example.com"
location = "hel1"
server_type = "cx23"
ssh_key_ids = [123]
ssh_cidrs = ["192.0.2.0/24"]
account_id = "00000000000000000000000000000001"
zone_id = "00000000000000000000000000000002"
updates_domain = "updates.example.com"
backup_bucket = "backup"
updates_bucket = "updates"
image = "ghcr.io/example/app"
"#;
	let managed: ManagedInfraConfig = toml::from_str(original).unwrap();
	let declaration: InfraConfig = toml::from_str(original).unwrap();
	LeptoskitConfig {
		infra: Some(declaration.clone()),
		..LeptoskitConfig::default()
	}
	.ensure_application_root()
	.unwrap();
	assert_eq!(declaration.as_managed(), Some(&managed));
	assert_eq!(
		toml::to_string(&declaration).unwrap(),
		toml::to_string(&managed).unwrap()
	);
	assert_eq!(declaration.clone().into_managed(), Some(managed));
	assert_eq!(
		toml::from_str::<InfraConfig>(&toml::to_string(&declaration).unwrap()).unwrap(),
		declaration
	);
}

#[test]
fn listen_variables_are_explicit_and_legacy_serialization_is_unchanged() {
	let mut config = website();
	assert!(!toml::to_string(&config).unwrap().contains("address-env"));
	let service = config.services.get_mut("play-api").unwrap();
	service.serve.as_mut().unwrap().address_env = Some("PLAY_BIND".into());
	plan(&config);
	let encoded = toml::to_string(&config).unwrap();
	assert!(encoded.contains("address-env = \"PLAY_BIND\""));
	assert_eq!(config, toml::from_str::<LeptoskitConfig>(&encoded).unwrap());
	config
		.services
		.get_mut("play-api")
		.unwrap()
		.serve
		.as_mut()
		.unwrap()
		.address_env = Some("PLAY_BIND\nUser=root".into());
	refuses(&config, "serve.address-env");
}
