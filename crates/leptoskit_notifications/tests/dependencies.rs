//! The library without features compiles no server dependencies: an app
//! that only names the notification types doesn't build the service, the
//! HTTP client, or the database. `cargo deny` can't check this, because the
//! service uses some of them legitimately. The tree is read for the browser
//! target, which is what an app's client build compiles.

#![cfg(not(target_arch = "wasm32"))]

use std::process::Command;

/// Crates the shared types must never pull in, directly or through another.
/// `sha2` isn't here: proc-macro dependencies (`leptoskit_db_macros`, and
/// leptos's code-splitting macros through `leptoskit_rpc`'s query client)
/// build with it at compile time, and never reach a bundle.
const BANNED: &[&str] = &[
	"axum",
	"deadpool-postgres",
	"hex",
	"hyper",
	"leptoskit_axum",
	"reqwest",
	"rusqlite",
	"tokio",
	"tokio-postgres",
];

#[test]
fn the_shared_types_compile_none_of_the_banned_crates() {
	check(&[]);
}

#[test]
fn the_client_build_compiles_none_of_the_banned_crates() {
	check(&["--no-default-features"]);
}

fn check(extra: &[&str]) {
	let output = Command::new(env!("CARGO"))
		.arg("tree")
		.args(extra)
		.args([
			"--offline",
			"--package",
			"leptoskit_notifications",
			"--target",
			"wasm32-unknown-unknown",
			"--edges",
			"normal",
			"--prefix",
			"none",
			"--format",
			"{p}",
		])
		.current_dir(env!("CARGO_MANIFEST_DIR"))
		.output()
		.expect("cargo runs");
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let tree = String::from_utf8(output.stdout).unwrap();
	let found: Vec<&str> = tree
		.lines()
		.filter_map(|line| line.split_whitespace().next())
		.filter(|name| BANNED.contains(name))
		.collect();
	assert!(
		found.is_empty(),
		"the shared types depend on {found:?}:\n{tree}"
	);
	assert_eq!(
		tree.lines()
			.next()
			.and_then(|line| line.split_whitespace().next()),
		Some("leptoskit_notifications")
	);
}
