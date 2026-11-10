//! Compile-fail tests: what `#[api]` cannot publish, and operations shaped
//! wrongly, must fail the build with an error that names the problem and
//! points at its cause. Pass tests: what must compile without a warning.
#![cfg(feature = "ssr")]

#[test]
fn compile_outcomes() {
	let cases = trybuild::TestCases::new();
	cases.compile_fail("tests/ui/*.rs");
	cases.pass("tests/ui_pass/*.rs");
}
