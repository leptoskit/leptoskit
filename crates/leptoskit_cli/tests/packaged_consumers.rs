//! Checks the external package acceptance harness without starting Cargo.

#[test]
fn packaged_consumer_harness_rejects_incomplete_evidence() {
	let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
	let output = std::process::Command::new("python3")
		.args([
			"-m",
			"unittest",
			"discover",
			"-s",
			"setup/scripts/consumer_acceptance",
			"-v",
		])
		.current_dir(root)
		.output()
		.expect("python3 is required for package acceptance harness tests");
	assert!(
		output.status.success(),
		"{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}
