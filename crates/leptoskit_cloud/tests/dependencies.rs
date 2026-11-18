//! The library without features compiles no server dependencies and none of
//! the archive's filesystem crates: the dashboard compiles it to wasm. `cargo
//! deny` can't check this, because other crates use these legitimately. The
//! tree is read for `wasm32-unknown-unknown`, which is what the dashboard
//! builds, without proc macros: they run in the compiler and never reach a
//! bundle (`leptoskit_db_macros` and leptos's code-splitting macros build
//! with `sha2`).

#![cfg(not(target_arch = "wasm32"))]

use std::process::Command;

/// Crates no build of this library may pull in, directly or through
/// another: the server, its runtime, and its databases.
const SERVER: &[&str] = &[
	"axum",
	"deadpool-postgres",
	"hyper",
	"leptoskit_auth",
	"leptoskit_axum",
	"reqwest",
	"rusqlite",
	"tokio",
	"tokio-postgres",
];

/// Crates only the `archive` feature may pull in: they read and write the
/// filesystem. `walkdir` isn't here: leptos brings it (through
/// `leptos_hot_reload`) into every leptoskit wasm build, and the tables and
/// operations this library declares need leptos's query client. Nor is
/// `sha2`: it is pure computation, and the shared types need it for the
/// release spec (`leptoskit_cloud_runtime` hashes IDs into namespaces) and
/// for shortening hostnames.
const ARCHIVE: &[&str] = &["flate2", "ignore", "tar"];

#[test]
fn the_shared_types_compile_neither_server_nor_filesystem_crates() {
	let tree = tree(&[]);
	assert_absent(&tree, SERVER);
	assert_absent(&tree, ARCHIVE);
}

#[test]
fn the_archive_feature_adds_the_archive_crates_and_no_server() {
	let tree = tree(&["--features", "archive"]);
	assert_absent(&tree, SERVER);

	for name in ["flate2", "tar", "sha2", "ignore"] {
		assert!(
			crates(&tree).any(|found| found == name),
			"the archive build should compile {name}:\n{tree}"
		);
	}
}

/// `cargo tree` for this package on the wasm target, normal edges only and
/// no proc macros, one package per line.
fn tree(features: &[&str]) -> String {
	let output = Command::new(env!("CARGO"))
		.arg("tree")
		.args(features)
		.args([
			"--offline",
			"--package",
			"leptoskit_cloud",
			"--target",
			"wasm32-unknown-unknown",
			"--edges",
			"normal,no-proc-macro",
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
	let tree = String::from_utf8(output.stdout).expect("cargo tree prints UTF-8");
	assert_eq!(crates(&tree).next(), Some("leptoskit_cloud"), "{tree}");
	tree
}

fn crates(tree: &str) -> impl Iterator<Item = &str> {
	tree.lines()
		.filter_map(|line| line.split_whitespace().next())
}

fn assert_absent(tree: &str, banned: &[&str]) {
	let found: Vec<&str> = crates(tree).filter(|name| banned.contains(name)).collect();
	assert!(found.is_empty(), "this build depends on {found:?}:\n{tree}");
}

/// A client may share a build with a database user without enabling Cloud's
/// server. Check each backend separately so one cannot mask the other.
#[test]
fn shared_types_compile_with_unified_database_backends() {
	let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
	let directory = tempfile::tempdir().expect("temporary client");
	std::fs::create_dir(directory.path().join("src")).expect("source directory");
	std::fs::write(directory.path().join("src/lib.rs"), "").expect("client source");
	for backend in [
		&[][..],
		&["sqlite"][..],
		&["postgres"][..],
		&["sqlite", "postgres"][..],
	] {
		let manifest = format!(
			"[package]\nname = \"cloud-feature-client\"\nversion = \"0.0.0\"\nedition = \
			 \"2024\"\n[dependencies]\nleptoskit_cloud = {{ path = {:?}, default-features = false \
			 }}\nleptoskit_db = {{ path = {:?}, default-features = false, features = {backend:?} \
			 }}\n",
			root,
			root.join("../leptoskit_db")
		);
		std::fs::write(directory.path().join("Cargo.toml"), manifest).expect("manifest");
		let output = Command::new("cargo")
			.args(["check", "--offline", "--manifest-path"])
			.arg(directory.path().join("Cargo.toml"))
			.current_dir(root)
			.output()
			.expect("cargo check runs");
		assert!(
			output.status.success(),
			"{backend:?}:\n{}",
			String::from_utf8_lossy(&output.stderr)
		);
	}
	let output = Command::new("cargo")
		.args([
			"check",
			"--offline",
			"--package",
			"leptoskit_cloud",
			"--no-default-features",
			"--target",
			"wasm32-unknown-unknown",
		])
		.current_dir(root)
		.output()
		.expect("wasm cargo check runs");
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
}
