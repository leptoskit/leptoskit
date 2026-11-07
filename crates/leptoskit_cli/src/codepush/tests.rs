//! The publish side of `leptoskit codepush`: release versioning and packing,
//! keys, and channels, offline and deterministic. The scenarios that need
//! the project under the current directory run in a child process, as
//! `publish::tests` does.

use std::path::Path;

use leptoskit_codepush::ReleaseSigningKey;
use leptoskit_codepush::ReleaseStore;
use leptoskit_config::Platform;
use semver::Version;

use super::load_key;
use super::next_version;
use super::selected_channel;
use super::validate_release_platform;

const COMMIT: &str = "3f9d5a1c2b8e7f604a9d5c3e2b1f0a8d7c6e5b4a";
const REPOSITORY: &str = "https://github.com/example/notes";
const HELPER: &str = "codepush::tests::subprocess_helper";

/// A bundle directory with one file, as a build leaves it.
fn bundle(root: &Path) -> std::path::PathBuf {
	let directory = root.join("bundle");
	std::fs::create_dir_all(&directory).unwrap();
	std::fs::write(directory.join("app.wasm"), b"a wasm bundle").unwrap();
	directory
}

/// A project trusting the `publisher` key and requiring two of `approver`
/// and `other`'s approvals.
fn project(
	root: &Path,
	publisher: &ReleaseSigningKey,
	approver: &ReleaseSigningKey,
	other: &ReleaseSigningKey,
) -> leptoskit_config::LeptoskitConfig {
	leptoskit_config::LeptoskitConfig::from_toml(
		&format!(
			"[app]\nname = \"Notes\"\nversion = \"1.0.0\"\nplatforms = \
			 [\"web\"]\n\n[codepush]\npublic_keys = [\"{}\"]\n\n[codepush.provenance]\nrepository \
			 = \"{REPOSITORY}\"\napprovers = [\"{}\", \"{}\"]\nthreshold = 2\n",
			publisher.public_key().to_hex(),
			approver.public_key().to_hex(),
			other.public_key().to_hex(),
		),
		root,
	)
	.unwrap()
}

#[test]
fn next_version_continues_builds_and_refuses_an_older_version() {
	let root = tempfile::tempdir().unwrap();
	let store = ReleaseStore::open(root.path().join("store")).unwrap();
	let project = Version::new(2, 0, 0);

	// An empty channel starts the project's version at build 1.
	assert_eq!(
		next_version(&store, "Notes", "production", &project).unwrap(),
		(project.clone(), 1)
	);

	let signer = ReleaseSigningKey::from_seed(&[0x5a; 32]);
	store
		.ingest_directory(
			&signer,
			"Notes",
			"production",
			&Version::new(1, 4, 0),
			7,
			&bundle(root.path()),
			false,
		)
		.unwrap();

	// The same version takes the next build; a newer one starts at 1.
	assert_eq!(
		next_version(&store, "Notes", "production", &Version::new(1, 4, 0)).unwrap(),
		(Version::new(1, 4, 0), 8)
	);
	assert_eq!(
		next_version(&store, "Notes", "production", &project).unwrap(),
		(project.clone(), 1)
	);

	// An older version would never be installed, so releasing it refuses.
	let error = next_version(&store, "Notes", "production", &Version::new(1, 0, 0))
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("1.0.0 is older than the latest production release 1.4.0+7"),
		"{error}"
	);
}

#[test]
fn keys_load_from_a_file_and_errors_never_quote_the_secret() {
	let root = tempfile::tempdir().unwrap();
	let file = root.path().join("signing.key");
	let signer = ReleaseSigningKey::from_seed(&[0x5a; 32]);
	std::fs::write(&file, format!("{}\n", signer.to_hex())).unwrap();

	let loaded = load_key(
		Some(file.to_str().unwrap()),
		"LEPTOSKIT_TEST_CODEPUSH_KEY",
		"signing key",
		|| "not reached".to_owned(),
	)
	.unwrap();
	assert_eq!(loaded.to_hex(), signer.to_hex());

	// A bad file is named but its contents are not.
	let bad = root.path().join("bad.key");
	std::fs::write(&bad, "not hex at all").unwrap();
	let error = load_key(
		Some(bad.to_str().unwrap()),
		"LEPTOSKIT_TEST_CODEPUSH_KEY",
		"signing key",
		|| "not reached".to_owned(),
	)
	.unwrap_err()
	.to_string();
	assert!(error.contains("could not load the signing key"), "{error}");
	assert!(!error.contains("not hex at all"), "{error}");

	let error = load_key(
		Some(root.path().join("absent.key").to_str().unwrap()),
		"LEPTOSKIT_TEST_CODEPUSH_KEY",
		"signing key",
		|| "not reached".to_owned(),
	)
	.unwrap_err()
	.to_string();
	assert!(error.contains("could not read the signing key"), "{error}");

	// Without a file, the environment is read; without the variable, the
	// caller's message says how to get a key.
	let error = load_key(
		None,
		"LEPTOSKIT_TEST_CODEPUSH_UNSET_KEY",
		"signing key",
		|| "pass --signing-key <PATH> or set the variable".to_owned(),
	)
	.unwrap_err()
	.to_string();
	assert!(error.contains("pass --signing-key <PATH>"), "{error}");
}

#[test]
fn keygen_writes_owner_only_keys_and_never_overwrites() {
	let root = tempfile::tempdir().unwrap();
	let out = root.path().join("signing.key");
	super::keygen(out.to_str().unwrap(), false).unwrap();

	let bytes = std::fs::read(&out).unwrap();
	let signer = ReleaseSigningKey::from_hex(&String::from_utf8(bytes).unwrap()).unwrap();
	assert_eq!(signer.public_key().to_hex().len(), 64);

	// Created owner-only, before any secret is written.
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		let mode = std::fs::metadata(&out).unwrap().permissions().mode();
		assert_eq!(mode & 0o777, 0o600);
	}

	let approver = root.path().join("approver.key");
	super::keygen(approver.to_str().unwrap(), true).unwrap();

	// An existing key is never overwritten.
	let error = super::keygen(out.to_str().unwrap(), false)
		.unwrap_err()
		.to_string();
	assert!(error.contains("could not create"), "{error}");
	assert!(
		error.contains("an existing key is never overwritten"),
		"{error}"
	);

	let unwritable = root.path().join("no-such-folder/signing.key");
	let error = super::keygen(unwritable.to_str().unwrap(), false)
		.unwrap_err()
		.to_string();
	assert!(error.contains("could not create"), "{error}");
}

#[test]
fn selected_channel_names_universal_platforms() {
	let root = tempfile::tempdir().unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		"[app]\nname = \"Notes\"\nversion = \"1.0.0\"\nplatforms = [\"web\", \"ios\"]\n",
		root.path(),
	)
	.unwrap();

	// No platform keeps the base channel.
	assert_eq!(
		selected_channel(root.path(), &config, "production", None).unwrap(),
		"production"
	);

	// Not a universal project: no per-platform channel yet.
	assert_eq!(
		selected_channel(root.path(), &config, "production", Some(Platform::Ios)).unwrap(),
		"production"
	);

	// A universal project gives every native destination its own channel.
	std::fs::create_dir_all(root.path().join("ios")).unwrap();
	std::fs::write(root.path().join("ios/Cargo.toml"), "").unwrap();
	assert_eq!(
		selected_channel(root.path(), &config, "production", Some(Platform::Ios)).unwrap(),
		"production-ios"
	);
	assert_eq!(
		selected_channel(root.path(), &config, "production", Some(Platform::Web)).unwrap(),
		"production"
	);

	// An aggregate build target is not an application platform.
	let error = selected_channel(root.path(), &config, "production", Some(Platform::Native))
		.unwrap_err()
		.to_string();
	assert!(error.contains("aggregate build target"), "{error}");

	// A platform the app doesn't ship is refused.
	let web_only = leptoskit_config::LeptoskitConfig::from_toml(
		"[app]\nname = \"Notes\"\nversion = \"1.0.0\"\nplatforms = [\"web\"]\n",
		root.path(),
	)
	.unwrap();
	let error = selected_channel(root.path(), &web_only, "production", Some(Platform::Ios))
		.unwrap_err()
		.to_string();
	assert!(error.contains("ios"), "{error}");
}

#[test]
fn release_platform_validation_matches_the_shell() {
	let root = tempfile::tempdir().unwrap();
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		"[app]\nname = \"Notes\"\nversion = \"1.0.0\"\nplatforms = [\"web\", \"ios\"]\n",
		root.path(),
	)
	.unwrap();

	// Not a universal project: nothing to validate.
	assert!(
		validate_release_platform(root.path(), &root.path().join("bundle"), &config, None).is_ok()
	);

	// Universal, no descriptor, and no platform: a web release.
	std::fs::create_dir_all(root.path().join("ios")).unwrap();
	std::fs::write(root.path().join("ios/Cargo.toml"), "").unwrap();
	let bundle = bundle(root.path());
	assert!(validate_release_platform(root.path(), &bundle, &config, None).is_ok());

	// A native release without a platform, or without the descriptor built
	// by `leptoskit native bundle`, is refused with the fix in the message.
	let error = validate_release_platform(root.path(), &bundle, &config, Some(Platform::Ios))
		.unwrap_err()
		.to_string();
	assert!(error.contains("native releases need"), "{error}");
	assert!(error.contains("runtime.json"), "{error}");
	assert!(error.contains("--target ios"), "{error}");

	let descriptor = bundle.join("runtime.json");
	std::fs::write(&descriptor, "not json").unwrap();
	let error = validate_release_platform(root.path(), &bundle, &config, Some(Platform::Ios))
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("invalid native runtime descriptor"),
		"{error}"
	);

	// The shell the release was built for must be the one this destination
	// runs.
	let expected = crate::native::config::shell(root.path(), &config, Platform::Ios);
	std::fs::write(
		&descriptor,
		serde_json::json!({ "abi": 1, "shell": format!("{expected}-but-wrong"), "plugins": [] })
			.to_string(),
	)
	.unwrap();
	let error = validate_release_platform(root.path(), &bundle, &config, Some(Platform::Ios))
		.unwrap_err()
		.to_string();
	assert!(
		error.contains(&format!(
			"does not match --platform ios (expected `{expected}`)"
		)),
		"{error}"
	);

	std::fs::write(
		&descriptor,
		serde_json::json!({ "abi": 1, "shell": expected, "plugins": [] }).to_string(),
	)
	.unwrap();
	assert!(validate_release_platform(root.path(), &bundle, &config, Some(Platform::Ios)).is_ok());
}

/// The child side of [`run_in_project`]: releases, approves, and attaches in
/// the project the parent made the current directory. A normal test run has
/// no scenario and does nothing.
fn run_scenario(scenario: &str) {
	match scenario {
		"release-approve-attach" => {
			// Packing the first release, then the second with its delta.
			super::run(release_command()).unwrap();
			super::run(release_command()).unwrap();

			// An independent rebuilder approves the first release from the
			// store, and the approval is attached.
			super::run(crate::cli::CodepushCommand::Approve {
				release: None,
				store: "store".to_owned(),
				channel: "production".to_owned(),
				platform: None,
				version: Some("1.0.0".to_owned()),
				build: Some(1),
				rebuilt: "bundle".to_owned(),
				repository: REPOSITORY.to_owned(),
				commit: COMMIT.to_owned(),
				approver_key: Some("approver.key".to_owned()),
				out: "approval.json".to_owned(),
			})
			.unwrap();

			super::run(crate::cli::CodepushCommand::Attach {
				store: "store".to_owned(),
				approvals: vec!["approval.json".to_owned()],
			})
			.unwrap();
		}

		// The publisher's own key may not approve its release.
		"approve-publisher" => {
			super::run(release_command()).unwrap();
			let error = super::run(crate::cli::CodepushCommand::Approve {
				release: None,
				store: "store".to_owned(),
				channel: "production".to_owned(),
				platform: None,
				version: Some("1.0.0".to_owned()),
				build: Some(1),
				rebuilt: "bundle".to_owned(),
				repository: REPOSITORY.to_owned(),
				commit: COMMIT.to_owned(),
				approver_key: Some("publisher.key".to_owned()),
				out: "approval.json".to_owned(),
			})
			.unwrap_err()
			.to_string();
			assert!(error.contains("is a publisher key"), "{error}");
		}

		// A release signed by a key no app trusts refuses before it is
		// stored.
		"release-untrusted" => {
			let error = super::run(release_command()).unwrap_err().to_string();
			assert!(error.contains("is not in codepush.public_keys"), "{error}");
		}

		other => panic!("unknown codepush scenario {other}"),
	}
}

/// The `codepush release` command the scenarios run, on the project's own
/// bundle and store with its publisher key.
fn release_command() -> crate::cli::CodepushCommand {
	crate::cli::CodepushCommand::Release {
		from: "bundle".to_owned(),
		store: "store".to_owned(),
		channel: "production".to_owned(),
		platform: None,
		mandatory: false,
		signing_key: Some("publisher.key".to_owned()),
	}
}

#[test]
fn subprocess_helper() {
	let Ok(scenario) = std::env::var("LEPTOSKIT_TEST_SCENARIO") else {
		return;
	};
	assert!(
		scenario.starts_with("release-") || scenario.starts_with("approve-"),
		"{scenario}"
	);
	run_scenario(&scenario);
}

/// Runs one helper scenario in a child process, whose current directory is
/// `root`: a project with its signing keys, store, and bundle.
fn run_in_project(scenario: &str, root: &Path) {
	let mut command = std::process::Command::new(std::env::current_exe().unwrap());
	command
		.args(["--exact", HELPER, "--nocapture", "--test-threads=1"])
		.env_clear()
		.env("LEPTOSKIT_TEST_SCENARIO", scenario)
		.env("PATH", std::env::var_os("PATH").unwrap_or_default())
		.current_dir(root);

	// Winsock loads its providers relative to SystemRoot on Windows.
	if let Some(system_root) = std::env::var_os("SystemRoot") {
		command.env("SystemRoot", system_root);
	}

	if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
		command.env("LLVM_PROFILE_FILE", profile);
	}

	let output = command.output().unwrap();
	assert!(
		output.status.success(),
		"scenario {scenario} failed\nstdout:\n{}\nstderr:\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}

#[test]
fn releases_approves_and_attaches_in_a_project() {
	let root = tempfile::tempdir().unwrap();
	let publisher = ReleaseSigningKey::from_seed(&[0x5a; 32]);
	let approver = ReleaseSigningKey::from_seed(&[1; 32]);
	let other = ReleaseSigningKey::from_seed(&[2; 32]);
	let config = project(root.path(), &publisher, &approver, &other);
	std::fs::write(
		root.path().join("leptoskit.toml"),
		config.to_toml_string().unwrap(),
	)
	.unwrap();
	std::fs::write(
		root.path().join("publisher.key"),
		format!("{}\n", publisher.to_hex()),
	)
	.unwrap();
	std::fs::write(
		root.path().join("approver.key"),
		format!("{}\n", approver.to_hex()),
	)
	.unwrap();
	bundle(root.path());

	run_in_project("release-approve-attach", root.path());
	run_in_project("approve-publisher", root.path());

	// The project ends with three releases (the publisher-refused scenario
	// packs one more) and one attached approval, on the release that was
	// approved: 1.0.0+1.
	let store = ReleaseStore::open(root.path().join("store")).unwrap();
	let latest = store
		.latest("Notes", "production")
		.unwrap()
		.expect("releases");
	assert_eq!(latest.manifest.build, 3);
	let first = store
		.find("Notes", "production", &Version::new(1, 0, 0), 1)
		.unwrap()
		.expect("the approved release");
	let approvals = store.approvals(&first.manifest).unwrap().expect("attached");
	assert_eq!(approvals.approvals.len(), 1);
}

#[test]
fn a_release_no_app_trusts_is_refused() {
	let root = tempfile::tempdir().unwrap();
	let publisher = ReleaseSigningKey::from_seed(&[0x5a; 32]);
	let approver = ReleaseSigningKey::from_seed(&[1; 32]);
	let other = ReleaseSigningKey::from_seed(&[2; 32]);
	// The project trusts a different key than the release carries.
	let trusted = ReleaseSigningKey::from_seed(&[3; 32]);
	let config = project(root.path(), &trusted, &approver, &other);
	std::fs::write(
		root.path().join("leptoskit.toml"),
		config.to_toml_string().unwrap(),
	)
	.unwrap();
	std::fs::write(
		root.path().join("publisher.key"),
		format!("{}\n", publisher.to_hex()),
	)
	.unwrap();
	bundle(root.path());

	run_in_project("release-untrusted", root.path());
}

/// An approve target that names no release at all is refused before any key
/// is read.
#[test]
fn approve_names_a_missing_target() {
	let error = super::run(crate::cli::CodepushCommand::Approve {
		release: None,
		store: "store".to_owned(),
		channel: "production".to_owned(),
		platform: None,
		version: Some("1.0.0".to_owned()),
		build: None,
		rebuilt: "bundle".to_owned(),
		repository: REPOSITORY.to_owned(),
		commit: COMMIT.to_owned(),
		approver_key: None,
		out: "approval.json".to_owned(),
	})
	.unwrap_err()
	.to_string();
	assert!(
		error.contains("pass --release <PATH>, or --version and --build"),
		"{error}"
	);
}
