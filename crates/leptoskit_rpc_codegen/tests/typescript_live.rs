//! Run the generated live client through interrupted connection handshakes.
#![cfg(feature = "generators")]

use std::process::Command;

use leptoskit_rpc_codegen::ApiSpec;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::TargetLanguage;

#[test]
fn stalled_typescript_connections_retry_without_losing_work() {
	let dir = tempfile::tempdir().unwrap();
	let spec = ApiSpec::empty("Live", "1.0.0");
	for file in spec
		.generate(&GenerateOptions::new(
			TargetLanguage::TypeScript,
			"live-client",
		))
		.files
	{
		std::fs::write(dir.path().join(file.path), file.contents).unwrap();
	}
	std::fs::write(
		dir.path().join("test.mjs"),
		include_str!("fixtures/typescript_live.mjs"),
	)
	.unwrap();
	let output = Command::new("node")
		.args(["--experimental-strip-types", "--no-warnings", "test.mjs"])
		.current_dir(dir.path())
		.output()
		.expect("run Node from devenv");
	assert!(
		output.status.success(),
		"the generated live client failed:\n{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}
