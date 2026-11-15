//! The unsupported platform's proof: on a non-Unix host the binary
//! refuses immediately — a nonzero exit, its reason on stderr, nothing
//! served — because this daily-reset service requires Unix `flock`/`exec`
//! and is deployed on Linux. It reads no configuration, binds nothing,
//! and creates no state before refusing. On Unix this test compiles to
//! nothing; it exists for a Windows run, where it executes naturally.

#![cfg(not(unix))]

use std::process::Command;
use std::process::Stdio;

#[test]
fn an_unsupported_platform_refuses_before_serving() {
	let output = Command::new(env!("CARGO_BIN_EXE_demo_service"))
		.env_remove("DEMO_BIND")
		.env_remove("DEMO_SITE_ORIGIN")
		.env_remove("DEMO_ADMIN_ORIGIN")
		.env_remove("LEPTOSKIT_SERVICE_VOLUME_STATE")
		.stdin(Stdio::null())
		.output()
		.expect("the binary starts and refuses");

	assert!(
		!output.status.success(),
		"the binary refuses with a nonzero exit: {output:?}"
	);
	assert!(
		output.stdout.is_empty(),
		"nothing serves, so stdout stays empty: {:?}",
		String::from_utf8_lossy(&output.stdout)
	);
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		stderr.contains("requires Unix flock/exec") && stderr.contains("deployed on Linux"),
		"the refusal names the Unix requirement and the Linux deployment: {stderr}"
	);
}
