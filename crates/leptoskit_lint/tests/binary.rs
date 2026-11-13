//! The `leptoskit-lint` binary over a scratch project.
#![cfg(feature = "cli")]

use std::path::Path;
use std::process::Command;
use std::process::Output;

fn leptoskit_lint(dir: &Path, args: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_leptoskit-lint"))
		.args(args)
		.current_dir(dir)
		.output()
		.expect("run leptoskit-lint")
}

fn text(bytes: &[u8]) -> String {
	String::from_utf8_lossy(bytes).into_owned()
}

/// A project with one builder tree, which the default rules warn about, and
/// one file that does not parse.
fn project() -> tempfile::TempDir {
	let project = tempfile::tempdir().expect("scratch project");
	std::fs::write(
		project.path().join("screen.rs"),
		"fn screen() -> Element {\n\tView().class(\"p-4\").child(Text(\"Hi\"))\n}\n",
	)
	.expect("write source");
	std::fs::write(project.path().join("broken.rs"), "fn broken( {").expect("write source");
	project
}

#[test]
fn reports_findings_and_skipped_files_and_passes_with_only_warnings() {
	let project = project();
	let output = leptoskit_lint(project.path(), &[]);
	let stdout = text(&output.stdout);
	assert!(output.status.success(), "{stdout}");
	assert!(
		stdout.contains("warning[leptoskit::markup_style]"),
		"{stdout}"
	);
	assert!(
		stdout.contains("skipped broken.rs: it does not parse"),
		"{stdout}"
	);
	assert!(
		stdout.contains("2 files checked: 1 warning, 0 errors"),
		"{stdout}"
	);
}

#[test]
fn deny_warnings_fails_on_a_warning() {
	let project = project();
	let output = leptoskit_lint(project.path(), &["--deny-warnings", "screen.rs"]);
	let stdout = text(&output.stdout);
	assert_eq!(output.status.code(), Some(1), "{stdout}");
	assert!(
		stdout.contains("1 file checked: 1 warning, 0 errors"),
		"{stdout}"
	);
}

#[test]
fn a_path_that_cannot_be_read_exits_with_two() {
	let project = project();
	let output = leptoskit_lint(project.path(), &["missing"]);
	let stderr = text(&output.stderr);
	assert_eq!(output.status.code(), Some(2), "{stderr}");
	assert!(stderr.starts_with("error: "), "{stderr}");
	assert!(stderr.contains("missing"), "{stderr}");
}

#[test]
fn selected_frontend_enforces_its_style_and_skips_other_frontends() {
	let project = tempfile::tempdir().unwrap();
	let root = project.path();
	std::fs::create_dir(root.join("site")).unwrap();
	std::fs::create_dir(root.join("product")).unwrap();
	let builder = "fn screen() -> Element { View().child(Text(\"Hi\")) }";
	std::fs::write(root.join("product/screen.rs"), builder).unwrap();
	std::fs::write(
		root.join("site/screen.rs"),
		"fn screen() -> Element { view! { <View /> } }",
	)
	.unwrap();
	std::fs::write(
		root.join("leptoskit.toml"),
		r#"
        [app]
        default_target = "desktop"
        [lint]
        markup = "builder"
        exclude = ["site/**"]
        [lint.rules]
        markup_style = "deny"
        [frontends.site.lint]
        markup = "view"
        exclude = ["product/**"]
        [frontends.site.lint.rules]
        markup_style = "deny"
        [targets.landing]
        frontend = "site"
        [targets.desktop]
    "#,
	)
	.unwrap();
	let run = |target: Option<&str>| {
		let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit-lint"));
		command.current_dir(root).env_remove("LEPTOSKIT_APP_TARGET");
		if let Some(target) = target {
			command.env("LEPTOSKIT_APP_TARGET", target);
		}
		command.output().unwrap()
	};
	for target in [None, Some("landing")] {
		let output = run(target);
		assert!(
			output.status.success(),
			"{}{}",
			text(&output.stdout),
			text(&output.stderr)
		);
		assert!(text(&output.stdout).contains("1 file checked"));
	}
	std::fs::write(root.join("site/screen.rs"), builder).unwrap();
	let output = run(Some("landing"));
	assert_eq!(output.status.code(), Some(1));
	assert!(text(&output.stdout).contains("error[leptoskit::markup_style]"));
}
