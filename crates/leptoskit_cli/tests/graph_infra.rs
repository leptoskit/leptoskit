//! CLI dispatch and saved-plan refusal without cloud credentials.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;
use std::process::Output;

fn project() -> tempfile::TempDir {
	let root = tempfile::tempdir().unwrap();
	let mut config: leptoskit_config::LeptoskitConfig =
		toml::from_str(include_str!("fixtures/website-graph.toml")).unwrap();
	let leptoskit_config::infra::InfraConfig::Graph(infra) = config.infra.as_mut().unwrap() else {
		panic!("graph fixture required");
	};
	infra.budget = None;
	infra.edge.as_mut().unwrap().ingress = leptoskit_config::deployment::Ingress::CloudflareTunnel;
	infra.edge.as_mut().unwrap().origin_firewall =
		leptoskit_config::deployment::OriginFirewall::Closed;
	config
		.services
		.get_mut("play-api")
		.unwrap()
		.deploy
		.as_mut()
		.unwrap()
		.ingress = Some(leptoskit_config::deployment::Ingress::CloudflareTunnel);
	fs::write(
		root.path().join("leptoskit.toml"),
		toml::to_string(&config).unwrap(),
	)
	.unwrap();
	let bin = root.path().join("bin");
	fs::create_dir(&bin).unwrap();
	let tofu = bin.join("tofu");
	fs::write(&tofu, r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> tofu-calls
if [ "${MOCK_FAIL:-}" = "$1" ]; then exit 1; fi
case "$1" in
version) printf '%s' '{"terraform_version":"1.13.9","platform":"fixture"}' ;;
workspace) printf default ;;
init) printf lock > .terraform.lock.hcl ;;
validate) ;;
plan) printf saved-plan > leptoskit.plan ;;
show) printf '%s' '{"format_version":"1.2","resource_changes":[]}' ;;
apply) ;;
output)
 case "$3" in
 hosts) printf '%s' '{"web":{"provider":"hetzner","location":"hel1","server_type":"cx23","ipv4":"192.0.2.10","ipv6":"2001:db8::10"},"compiler":{"provider":"hetzner","location":"hel1","server_type":"cpx32","ipv4":"192.0.2.11","ipv6":"2001:db8::11"}}' ;;
 domains_by_host) printf '%s' '{"web":["leptoskit.dev"],"compiler":["play-api.leptoskit.dev"]}' ;;
 *) printf fixture-private-token; exit 1 ;;
 esac ;;
*) exit 1 ;;
esac
"#).unwrap();
	fs::set_permissions(tofu, fs::Permissions::from_mode(0o755)).unwrap();
	root
}

fn command(root: &Path, args: &[&str]) -> Command {
	let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit"));
	command.current_dir(root).arg("infra").args(args);
	let mut paths = vec![root.join("bin")];
	paths.extend(std::env::split_paths(
		&std::env::var_os("PATH").unwrap_or_default(),
	));
	command.env("PATH", std::env::join_paths(paths).unwrap());
	for (key, _) in std::env::vars_os() {
		let text = key.to_string_lossy();
		if text.starts_with("TF_CLI_ARGS")
			|| matches!(text.as_ref(), "TF_DATA_DIR" | "TF_WORKSPACE")
		{
			command.env_remove(key);
		}
	}
	command
}

fn success(output: Output) -> Output {
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	output
}

#[test]
fn graph_commands_plan_apply_once_and_return_only_public_observations() {
	let root = project();
	let root = root.path();
	success(command(root, &["plan"]).output().unwrap());
	let dir = root.join("infra/leptoskit-web");
	assert!(dir.join("plan.sha256").is_file());
	success(command(root, &["apply"]).output().unwrap());
	assert!(!dir.join("plan.sha256").exists());
	assert!(!command(root, &["apply"]).output().unwrap().status.success());
	let output = success(command(root, &["output"]).output().unwrap());
	let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
	assert_eq!(value["evidence"]["application_runtime"], "not_checked");
	let calls = fs::read_to_string(dir.join("tofu-calls")).unwrap();
	assert!(calls.contains("apply -input=false -lock=true leptoskit.plan\n"));
	assert!(calls.contains("output -json hosts\noutput -json domains_by_host\n"));
	assert!(!calls.lines().any(|line| line == "output -json"));
	assert!(!String::from_utf8_lossy(&output.stdout).contains("fixture-private-token"));
}

#[test]
fn substitutions_and_failed_replanning_never_reuse_a_saved_plan() {
	let root = project();
	let root = root.path();
	success(command(root, &["plan"]).output().unwrap());
	let dir = root.join("infra/leptoskit-web");
	fs::write(dir.join("leptoskit.plan"), b"changed").unwrap();
	assert!(!command(root, &["apply"]).output().unwrap().status.success());
	assert!(
		!fs::read_to_string(dir.join("tofu-calls"))
			.unwrap()
			.contains("apply ")
	);
	success(command(root, &["plan"]).output().unwrap());
	assert!(
		!command(root, &["plan"])
			.env("MOCK_FAIL", "init")
			.output()
			.unwrap()
			.status
			.success()
	);
	assert!(!dir.join("plan.sha256").exists());
	success(command(root, &["plan"]).output().unwrap());
	assert!(
		!command(root, &["plan"])
			.env("TF_CLI_ARGS_plan", "-destroy")
			.output()
			.unwrap()
			.status
			.success()
	);
	assert!(!dir.join("plan.sha256").exists());
	success(command(root, &["plan"]).output().unwrap());
	assert!(
		!command(root, &["apply"])
			.env("MOCK_FAIL", "apply")
			.output()
			.unwrap()
			.status
			.success()
	);
	assert!(!dir.join("plan.sha256").exists());
}
