//! Real `OpenTofu` evaluation with all provider operations mocked. This checks
//! provider schemas and expanded expressions, not cloud provisioning.

use std::path::Path;
use std::process::Command;

use leptoskit_config::LeptoskitConfig;
use leptoskit_config::deployment;
use leptoskit_config::deployment::OriginFirewall;

use super::*;

fn configuration(variant: &str) -> LeptoskitConfig {
	let mut config: LeptoskitConfig = toml::from_str(include_str!(
		"../../../../tests/fixtures/website-graph.toml"
	))
	.unwrap();
	let leptoskit_config::infra::InfraConfig::Graph(infra) = config.infra.as_mut().unwrap() else {
		panic!("graph fixture required");
	};

	match variant {
		"mixed" => {
			infra.hosts.get_mut("web").unwrap().ssh_key_ids = vec![123];
			let compiler = infra.hosts.get_mut("compiler").unwrap();
			compiler.compute = ComputeProvider::Digitalocean;
			compiler.ssh_key_ids = vec![456];
			compiler.location = "ams3".into();
			compiler.server_type = "c-4".into();
		}
		"tunnel" => {
			let edge = infra.edge.as_mut().unwrap();
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
		}
		"static" => {
			config.services.clear();
			infra.frontends.get_mut("play").unwrap().calls.clear();
			infra.hosts.remove("compiler");
		}
		"private" => {
			infra.frontends.clear();
			config.artifacts.clear();
			infra.hosts.remove("web");
			infra.edge = None;
			let deploy = config
				.services
				.get_mut("play-api")
				.unwrap()
				.deploy
				.as_mut()
				.unwrap();
			deploy.domain = None;
			deploy.ingress = None;
		}
		"https" => {}
		_ => panic!("unknown provider fixture"),
	}

	config
}

fn command(tofu: &std::ffi::OsStr, directory: &Path, args: &[&str]) {
	let mut command = Command::new(tofu);
	command
		.current_dir(directory)
		.args(args)
		.env("TF_IN_AUTOMATION", "1");
	// Every provider is explicitly mocked in the fixture. Also withhold
	// credentials so this cannot accidentally become real cloud acceptance.
	for (name, _) in std::env::vars_os() {
		let name_text = name.to_string_lossy();
		if [
			"TOKEN",
			"SECRET",
			"PASSWORD",
			"AWS_",
			"CLOUDFLARE",
			"HCLOUD",
			"DIGITALOCEAN",
		]
		.iter()
		.any(|part| name_text.contains(part))
		{
			command.env_remove(name);
		}
	}

	let output = command
		.output()
		.expect("install OpenTofu or set LEPTOSKIT_TEST_TOFU");

	assert!(
		output.status.success(),
		"OpenTofu {args:?} failed:\n{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}

#[test]
#[ignore = "requires OpenTofu and provider downloads; all cloud operations are mocked"]
fn graph_provider_schemas_and_expanded_boundaries() {
	let tofu = std::env::var_os("LEPTOSKIT_TEST_TOFU").unwrap_or_else(|| "tofu".into());
	let root = tempfile::tempdir().unwrap();
	let ranges = Ranges::parse("173.245.48.0/20", "2400:cb00::/32").unwrap();
	for (variant, fixture) in [
		(
			"https",
			include_str!("../../../../tests/fixtures/graph-provider/https.tftest.hcl"),
		),
		(
			"mixed",
			include_str!("../../../../tests/fixtures/graph-provider/mixed.tftest.hcl"),
		),
		(
			"static",
			include_str!("../../../../tests/fixtures/graph-provider/static.tftest.hcl"),
		),
		(
			"private",
			include_str!("../../../../tests/fixtures/graph-provider/private.tftest.hcl"),
		),
		(
			"tunnel",
			include_str!("../../../../tests/fixtures/graph-provider/tunnel.tftest.hcl"),
		),
	] {
		let config = configuration(variant);
		config.validate().unwrap();
		let plan = deployment::compile(&config).unwrap().unwrap();
		let infra = config.infra.as_ref().unwrap().as_graph().unwrap();
		let public_https = infra
			.edge
			.as_ref()
			.is_some_and(|edge| edge.ingress == Ingress::CloudflareHttps);
		let resources = resources(infra, &plan, public_https.then_some(&ranges)).unwrap();
		let directory = root.path().join(variant);
		std::fs::create_dir(&directory).unwrap();
		std::fs::write(
			directory.join("main.tf.json"),
			serde_json::to_vec_pretty(&resources).unwrap(),
		)
		.unwrap();
		std::fs::write(directory.join("boundaries.tftest.hcl"), fixture).unwrap();
		command(
			&tofu,
			&directory,
			&["init", "-backend=false", "-input=false", "-no-color"],
		);
		command(&tofu, &directory, &["validate", "-no-color"]);
		command(&tofu, &directory, &["test", "-no-color"]);
	}
}
