//! The website packager: a complete artifact is assembled from a build the
//! CLI leaves behind, a public-only run drops the console, and a failed or
//! missing build stops packaging instead of shipping an incomplete site.
#![cfg(all(feature = "server", feature = "site", unix))]
use std::path::Path;
use std::process::Command;

/// A stand-in for the `leptoskit` CLI: it writes the page a web build would
/// leave in `--out`, with the base tag the book's players rewrite.
fn fake_cli(directory: &Path, status: i32) -> std::path::PathBuf {
	let cli = directory.join("fake-lk");
	std::fs::write(
		&cli,
		format!(
			"#!/bin/sh\nall=\"$*\"\nout=\nwhile [ \"$#\" -gt 0 ]; do\nif [ \"$1\" = --out ]; then \
			 shift; out=\"$1\"; fi\nshift\ndone\nif [ {status} -ne 0 ]; then\necho 'the web build \
			 failed' >&2\nexit {status}\nfi\nmkdir -p \"$out\"\nprintf '%s\\n' \"$all\" > \
			 \"$out/args.txt\"\nprintf '<!doctype html><base href=\"/\" /><title>built</title>' > \
			 \"$out/index.html\"\n"
		),
	)
	.unwrap();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
	}
	cli
}

fn packager() -> Command {
	Command::new(env!("CARGO_BIN_EXE_leptoskit-site"))
}

#[test]
fn a_complete_artifact_can_be_repackaged_for_the_public_domain() {
	let directory = tempfile::tempdir().unwrap();
	let out = directory.path().join("dist");
	let cli = fake_cli(directory.path(), 0);
	let hosted = packager()
		.args(["--url", "https://deploy.example.test"])
		.arg("--cli")
		.arg(&cli)
		.arg("--out")
		.arg(&out)
		.output()
		.unwrap();
	assert!(
		hosted.status.success(),
		"{}",
		String::from_utf8_lossy(&hosted.stderr)
	);
	assert!(
		String::from_utf8_lossy(&hosted.stdout).contains("Website, console and documentation"),
		"{}",
		String::from_utf8_lossy(&hosted.stdout)
	);
	// The client the fake CLI left becomes the console's entrypoint, and the
	// landing page offers it beside the real book.
	let console = std::fs::read_to_string(out.join("console/index.html")).unwrap();
	assert!(console.contains("<title>built</title>"));
	let landing = std::fs::read_to_string(out.join("index.html")).unwrap();
	assert!(landing.contains("href=\"/console/\""));
	assert!(landing.contains("https://deploy.example.test/"));
	assert!(out.join("docs/guide/01-installation.html").is_file());
	assert!(out.join("rive/rive.js").is_file());
	// Every played example is framed at where the site serves it.
	for example in leptoskit_docs_site::play::EXAMPLES {
		let page = out.join("docs/play").join(example.name).join("index.html");
		let framed = std::fs::read_to_string(&page).unwrap();
		assert!(
			framed.contains(&format!(r#"<base href="/docs/play/{}/" />"#, example.name)),
			"{}",
			framed
		);
		assert!(framed.contains("../../theme/play-frame.js"), "{}", framed);
	}

	// Public-only packaging reuses the directory and must not leave the
	// console entrypoint behind.
	let public = packager()
		.args(["--url", "https://leptoskit.dev", "--public-only"])
		.arg("--cli")
		.arg(&cli)
		.arg("--out")
		.arg(&out)
		.output()
		.unwrap();
	assert!(
		public.status.success(),
		"{}",
		String::from_utf8_lossy(&public.stderr)
	);
	assert!(
		String::from_utf8_lossy(&public.stdout).contains("Public website and documentation"),
		"{}",
		String::from_utf8_lossy(&public.stdout)
	);
	assert!(!out.join("console").exists());
	let landing = std::fs::read_to_string(out.join("index.html")).unwrap();
	assert!(!landing.contains("/console"));
	assert!(out.join("schema.json").is_file());
	assert!(out.join("schemas").is_dir());
}

#[test]
fn a_failed_or_missing_build_is_not_packaged() {
	let directory = tempfile::tempdir().unwrap();
	let out = directory.path().join("dist");
	let failing = fake_cli(directory.path(), 1);
	let output = packager()
		.arg("--cli")
		.arg(&failing)
		.arg("--out")
		.arg(&out)
		.output()
		.unwrap();
	assert!(!output.status.success());
	let report = String::from_utf8_lossy(&output.stderr);
	assert!(report.contains("could not build the website"), "{report}");

	let missing = directory.path().join("not-a-cli");
	let output = packager()
		.arg("--cli")
		.arg(&missing)
		.arg("--out")
		.arg(&out)
		.output()
		.unwrap();
	assert!(!output.status.success());
	let report = String::from_utf8_lossy(&output.stderr);
	assert!(
		report.contains("run the LeptosKit web build in"),
		"{report}"
	);
	assert!(!out.join("console").exists());
}

/// A stand-in CLI that fails, or writes a page without the base tag, inside
/// the book's played examples.
fn broken_cli(directory: &Path, failure: &str) -> std::path::PathBuf {
	let cli = directory.join("broken-lk");
	let body = match failure {
		"play" => {
			"case \"$out\" in *play*) echo 'the example build failed' >&2; exit 1;; esac\nprintf \
			 '<!doctype html><base href=\"/\" />' > \"$out/index.html\"\n"
		}
		_ => {
			"case \"$out\" in *play*) printf '<!doctype html>' > \"$out/index.html\";;\n*) printf \
			 '<!doctype html><base href=\"/\" />' > \"$out/index.html\";;\nesac\n"
		}
	};
	std::fs::write(
		&cli,
		format!(
			"#!/bin/sh\nout=\nwhile [ \"$#\" -gt 0 ]; do\nif [ \"$1\" = --out ]; then shift; \
			 out=\"$1\"; fi\nshift\ndone\nmkdir -p \"$out\"\n{body}"
		),
	)
	.unwrap();
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
	}
	cli
}

#[test]
fn a_relative_cli_is_resolved_and_a_release_build_is_asked_for() {
	let directory = tempfile::tempdir().unwrap();
	let nested = directory.path().join("nested");
	std::fs::create_dir_all(&nested).unwrap();
	fake_cli(&nested, 0);
	// A relative `--out` is taken from the crate's own directory, as the
	// packaging command documents.
	let relative = format!("../../target/website-relative-{}", std::process::id());
	let out = Path::new(env!("CARGO_MANIFEST_DIR")).join(&relative);
	let _ = std::fs::remove_dir_all(&out);
	let output = packager()
		.current_dir(directory.path())
		.args(["--cli", "nested/fake-lk", "--release"])
		.arg("--out")
		.arg(&relative)
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"{} {}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	let args = std::fs::read_to_string(out.join("args.txt")).unwrap();
	assert!(args.contains("build"), "{args}");
	assert!(args.contains("--release"), "{args}");
	let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_played_example_that_cannot_be_built_or_framed_stops_packaging() {
	let directory = tempfile::tempdir().unwrap();
	let out = directory.path().join("dist");
	let cli = broken_cli(directory.path(), "play");
	let output = packager()
		.arg("--cli")
		.arg(&cli)
		.arg("--out")
		.arg(&out)
		.output()
		.unwrap();
	assert!(!output.status.success());
	let report = String::from_utf8_lossy(&output.stderr);
	assert!(
		report.contains("could not build the gallery example to play"),
		"{report}"
	);

	let cli = broken_cli(directory.path(), "base");
	let output = packager()
		.arg("--cli")
		.arg(&cli)
		.arg("--out")
		.arg(&out)
		.output()
		.unwrap();
	assert!(!output.status.success());
	let report = String::from_utf8_lossy(&output.stderr);
	assert!(report.contains("prepare"), "{report}");
	assert!(
		report.contains("must declare"),
		"the frame document's refusal: {report}"
	);
}

#[test]
fn a_console_entrypoint_that_cannot_be_removed_stops_public_packaging() {
	let directory = tempfile::tempdir().unwrap();
	let out = directory.path().join("dist");
	// A previous packaging left `console` as a file rather than a directory.
	std::fs::create_dir_all(&out).unwrap();
	std::fs::write(out.join("console"), "not a directory").unwrap();
	let cli = fake_cli(directory.path(), 0);
	let output = packager()
		.arg("--public-only")
		.arg("--cli")
		.arg(&cli)
		.arg("--out")
		.arg(&out)
		.output()
		.unwrap();
	assert!(!output.status.success());
	let report = String::from_utf8_lossy(&output.stderr);
	assert!(report.contains("os error"), "{report}");
}

#[test]
fn a_rive_runtime_that_cannot_be_written_stops_packaging() {
	let directory = tempfile::tempdir().unwrap();
	let out = directory.path().join("dist");
	// The build left a directory where the runtime file belongs.
	std::fs::create_dir_all(out.join("rive/rive.js")).unwrap();
	let cli = fake_cli(directory.path(), 0);
	let output = packager()
		.arg("--cli")
		.arg(&cli)
		.arg("--out")
		.arg(&out)
		.output()
		.unwrap();
	assert!(!output.status.success());
	let report = String::from_utf8_lossy(&output.stderr);
	assert!(report.contains("os error"), "{report}");
}
