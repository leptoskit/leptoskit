//! The library without features compiles none of the runtime's machinery:
//! an app that only names the spec types builds no Kubernetes bindings, no
//! async runtime, and no HTTP client. `cargo deny` cannot check this,
//! because the `kubernetes` feature uses some of them legitimately.

#![cfg(not(target_arch = "wasm32"))]

use std::process::Command;

/// Crates the base library must never pull in, directly or through
/// another. The YAML writers and the Kubernetes bindings join this list
/// through the `kubernetes` feature, not the base: the base never writes
/// YAML or builds an object.
const BANNED: &[&str] = &["k8s-openapi", "kube", "reqwest", "tokio", "yaml_serde"];

#[test]
fn the_base_library_pulls_none_of_the_runtime_machinery() {
	let output = Command::new(env!("CARGO"))
		.arg("tree")
		.args(TREE_ARGS)
		.args(["--format", "{p}"])
		.current_dir(env!("CARGO_MANIFEST_DIR"))
		.output()
		.expect("cargo runs");
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let tree = String::from_utf8(output.stdout).expect("the tree is UTF-8");
	let found: Vec<&str> = tree
		.lines()
		.filter_map(|line| line.split_whitespace().next())
		.filter(|name| BANNED.contains(name))
		.collect();
	assert!(
		found.is_empty(),
		"the base library depends on {found:?}:\n{tree}"
	);
	assert_eq!(
		tree.lines()
			.next()
			.and_then(|line| line.split_whitespace().next()),
		Some("leptoskit_cloud_runtime")
	);
}

/// The package under check, no dev or build edges, on the guest target a
/// browser build compiles for.
const TREE_ARGS: &[&str] = &[
	"--offline",
	"--no-default-features",
	"--package",
	"leptoskit_cloud_runtime",
	"--target",
	"wasm32-unknown-unknown",
	"--edges",
	"normal",
	"--prefix",
	"none",
];
