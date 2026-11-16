//! The library without features compiles no server dependencies: an app
//! that only names the notification types doesn't build the service, the
//! HTTP client, or the database. `cargo deny` can't check this, because the
//! service uses some of them legitimately. The tree is read for the browser
//! and native targets, which are what an app's client builds compile.

#![cfg(not(target_arch = "wasm32"))]

use std::process::Command;

/// Crates the shared types must never pull in, directly or through another.
/// `sha2` isn't here: proc-macro dependencies (`leptoskit_db_macros`, and
/// leptos's code-splitting macros through `leptoskit_rpc`'s query client)
/// build with it at compile time, and never reach a bundle. `dbus` is: the
/// notifications plugin's Linux backend is a platform plugin's host
/// dependency, and no SDK, service, or guest tree may inherit it.
const BANNED: &[&str] = &[
	"axum",
	"dbus",
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
	check(&[], &["leptoskit_plugin_notifications"]);
}

#[test]
fn the_client_build_compiles_none_of_the_banned_crates() {
	check(
		&["--no-default-features"],
		&["leptoskit_plugin_notifications"],
	);
}

#[test]
fn the_browser_client_build_compiles_none_of_the_banned_crates() {
	check(
		&["--no-default-features", "--features", "browser"],
		&["leptoskit_plugin_notifications"],
	);
}

/// The native guest build compiles the plugin — that is its point — but
/// only its guest side: the server and host backends stay out, and so does
/// the browser implementation, whose imports would break the interpreter's
/// browser-free ABI. Crate names alone cannot prove that (`chrono` and
/// `leptos` bring their own wasm bindings into every tree), so the check
/// reads the plugin's resolved features instead: `guest` alone, never
/// `browser`.
#[test]
fn the_native_client_build_compiles_the_plugin_guest_alone() {
	let output = Command::new(env!("CARGO"))
		.arg("tree")
		.args(["--no-default-features", "--features", "native"])
		.args(TREE_ARGS)
		.args(["--format", "{p} [{f}]"])
		.current_dir(env!("CARGO_MANIFEST_DIR"))
		.output()
		.expect("cargo runs");
	assert!(
		output.status.success(),
		"{}",
		String::from_utf8_lossy(&output.stderr)
	);
	let tree = String::from_utf8(output.stdout).unwrap();
	assert_banned_absent(&tree);

	let plugin = tree
		.lines()
		.find(|line| line.starts_with("leptoskit_plugin_notifications"));
	let Some(plugin) = plugin else {
		panic!("the native build must compile the plugin:\n{tree}");
	};
	let features = plugin
		.rsplit_once('[')
		.expect("the tree is formatted with features");
	assert!(
		!features.1.contains("browser"),
		"the plugin's browser implementation leaked into the guest build:\n{plugin}"
	);
}

/// The arguments every tree read shares: the package under check, the
/// guest's own target, and no dev or build edges.
const TREE_ARGS: &[&str] = &[
	"--offline",
	"--package",
	"leptoskit_notifications",
	"--target",
	"wasm32-unknown-unknown",
	"--edges",
	"normal",
	"--prefix",
	"none",
];

fn check(extra: &[&str], absent: &[&str]) {
	let output = Command::new(env!("CARGO"))
		.arg("tree")
		.args(extra)
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
	let tree = String::from_utf8(output.stdout).unwrap();
	assert_banned_absent(&tree);
	let strays: Vec<&str> = tree
		.lines()
		.filter_map(|line| line.split_whitespace().next())
		.filter(|name| absent.contains(name))
		.collect();
	assert!(
		strays.is_empty(),
		"this build pulls in crates it does not use: {strays:?}\n{tree}"
	);
	assert_eq!(
		tree.lines()
			.next()
			.and_then(|line| line.split_whitespace().next()),
		Some("leptoskit_notifications")
	);
}

fn assert_banned_absent(tree: &str) {
	let found: Vec<&str> = tree
		.lines()
		.filter_map(|line| line.split_whitespace().next())
		.filter(|name| BANNED.contains(name))
		.collect();
	assert!(
		found.is_empty(),
		"the shared types depend on {found:?}:\n{tree}"
	);
}
