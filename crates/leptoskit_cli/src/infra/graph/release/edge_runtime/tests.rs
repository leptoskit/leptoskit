use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Parser as _;
use serde_json::json;

use super::*;

#[derive(clap::Parser)]
struct Parser {
	#[command(flatten)]
	args: Args,
}

fn arguments() -> Vec<String> {
	[
		"test",
		"--bundle",
		"bundle",
		"--sha256",
		"pin",
		"--trust",
		"trust.json",
		"--trust-sha256",
		"pin",
		"--version",
		"v1",
		"--unit",
		"services.worker",
		"--plan",
		"plan.json",
	]
	.into_iter()
	.map(str::to_owned)
	.collect()
}

fn trust() -> TrustedHost {
	TrustedHost {
		ipv4: "1.2.3.4".parse().unwrap(),
		port: 22,
		host_key: "ssh-ed25519 key".into(),
		identity: PathBuf::from("/operator/key"),
		architecture: crate::verify::graph::linux::Architecture::X86_64,
		prerequisites: BTreeMap::new(),
		tunnel: None,
	}
}

#[test]
fn private_cli_omits_caddy_and_public_inputs_are_a_pair() {
	let private = Parser::try_parse_from(arguments()).unwrap().args;
	assert!(private.caddy.is_none());
	assert!(bytes(&private).unwrap().is_none());

	for extra in [["--caddy", "binary"], ["--caddy-sha256", "pin"]] {
		let mut args = arguments();
		args.extend(extra.map(str::to_owned));
		assert!(Parser::try_parse_from(args).is_err());
	}

	let mut args = arguments();
	args.extend(["--caddy", "binary", "--caddy-sha256", "pin"].map(str::to_owned));
	assert!(Parser::try_parse_from(args).is_ok());
}

#[test]
fn private_host_needs_no_edge_file_binary_or_tls_but_rejects_prerequisites() {
	let directory = tempfile::tempdir().unwrap();
	let args = Parser::try_parse_from(arguments()).unwrap().args;
	let mut host = trust();
	let edge = bind(directory.path(), "worker", false, &args, &host, None).unwrap();
	assert!(edge.config.is_null());
	assert!(edge.sha256.is_none());
	assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
	assert!(bind(directory.path(), "public", true, &args, &host, None).is_err());
	host.prerequisites
		.insert("/etc/leptoskit/certificate.crt".into(), "pin".into());
	assert!(bind(directory.path(), "worker", false, &args, &host, None).is_err());
}

#[test]
fn mixed_hosts_send_caddy_only_to_public_hosts() {
	let private = json!({"edge":null});
	let public = json!({"edge":{"apps":{}}});
	let bytes = b"pinned-caddy".as_slice();
	assert!(payload(&private, None).unwrap().is_none());
	assert!(payload(&private, Some(bytes)).unwrap().is_none());
	assert!(payload(&public, None).is_err());
	assert_eq!(payload(&public, Some(bytes)).unwrap(), Some(bytes));
	let mut hosts = Map::new();
	hosts.insert("private".into(), private);
	assert!(check_usage(&hosts, None).is_ok());
	assert!(check_usage(&hosts, Some(bytes)).is_err());
	hosts.insert("public".into(), public);
	assert!(check_usage(&hosts, Some(bytes)).is_ok());
	assert!(check_usage(&hosts, None).is_err());
}
