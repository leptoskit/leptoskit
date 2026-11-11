//! The client compiles no large dependencies: §16 of the design keeps JSON,
//! UUID, time, compression, regex, and async-runtime crates out of apps'
//! bundles. `cargo deny` can't check this, because the service uses some of
//! them legitimately.

use std::process::Command;

/// Crates the client must never pull in, directly or through another.
const BANNED: &[&str] = &[
	"serde_json",
	"uuid",
	"chrono",
	"time",
	"flate2",
	"zstd",
	"brotli",
	"miniz_oxide",
	"regex",
	"semver",
	"tokio",
];

#[test]
fn the_client_compiles_none_of_the_banned_crates() {
	let output = Command::new(env!("CARGO"))
		.args([
			"tree",
			"--offline",
			"--package",
			"leptoskit_analytics",
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
	assert!(found.is_empty(), "the client depends on {found:?}:\n{tree}");
	assert_eq!(
		tree.lines()
			.next()
			.and_then(|line| line.split_whitespace().next()),
		Some("leptoskit_analytics")
	);
}
