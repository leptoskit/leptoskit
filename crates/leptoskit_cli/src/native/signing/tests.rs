use std::collections::BTreeMap;
use std::path::Path;

use clap::Parser as _;
use leptoskit_config::LeptoskitConfig;

use super::*;

/// An environment of `pairs`, and nothing else.
fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
	let map: BTreeMap<String, String> = pairs
		.iter()
		.map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
		.collect();
	move |name: &str| map.get(name).cloned()
}

/// A keystore outside any repository, with its directory.
fn keystore() -> (tempfile::TempDir, String) {
	let directory = tempfile::tempdir().unwrap();
	let path = directory.path().join("upload.jks");
	std::fs::write(&path, b"keystore").unwrap();
	(directory, path.display().to_string())
}

const PASSWORDS: [(&str, &str); 3] = [
	(ANDROID_KEYSTORE_PASSWORD, "store-secret"),
	(ANDROID_KEY_ALIAS, "upload"),
	(ANDROID_KEY_PASSWORD, "key-secret"),
];

fn with_passwords<'a>(pairs: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
	let mut all = PASSWORDS.to_vec();
	all.extend_from_slice(pairs);
	all
}

fn project() -> tempfile::TempDir {
	tempfile::tempdir().unwrap()
}

#[test]
fn an_android_release_without_the_upload_key_names_every_missing_variable() {
	let root = project();
	let error = AndroidSigning::resolve(root.path(), &env(&[]))
		.err()
		.unwrap()
		.to_string();

	for name in [
		ANDROID_KEYSTORE,
		ANDROID_KEYSTORE_BASE64,
		ANDROID_KEYSTORE_PASSWORD,
		ANDROID_KEY_ALIAS,
		ANDROID_KEY_PASSWORD,
	] {
		assert!(error.contains(name), "{error}");
	}

	assert!(error.contains("--unsigned"), "{error}");

	// One missing password is named alone, and no value set is printed.
	let (_directory, path) = keystore();
	let error = AndroidSigning::resolve(
		root.path(),
		&env(&[
			(ANDROID_KEYSTORE, &path),
			(ANDROID_KEYSTORE_PASSWORD, "store-secret"),
			(ANDROID_KEY_ALIAS, "upload"),
		]),
	)
	.err()
	.unwrap()
	.to_string();
	assert!(error.contains(ANDROID_KEY_PASSWORD), "{error}");
	assert!(!error.contains(ANDROID_KEY_ALIAS), "{error}");
	assert!(!error.contains("store-secret"), "{error}");
}

#[test]
fn an_android_release_takes_the_keystore_by_path_outside_the_repository() {
	let root = project();
	let (_directory, path) = keystore();
	let signing = AndroidSigning::resolve(
		root.path(),
		&env(&with_passwords(&[(ANDROID_KEYSTORE, &path)])),
	)
	.unwrap();
	assert_eq!(
		signing.keystore,
		std::fs::canonicalize(&path).unwrap(),
		"Gradle gets the absolute path"
	);

	// A keystore inside the project is refused, ignored by git or not.
	let inside = root.path().join("signing/upload.jks");
	std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
	std::fs::write(&inside, b"keystore").unwrap();
	let error = AndroidSigning::resolve(
		root.path(),
		&env(&with_passwords(&[(
			ANDROID_KEYSTORE,
			&inside.display().to_string(),
		)])),
	)
	.err()
	.unwrap()
	.to_string();
	assert!(error.contains("inside the project's repository"), "{error}");
	assert!(error.contains(ANDROID_KEYSTORE_BASE64), "{error}");

	// So is a path to nothing.
	let error = AndroidSigning::resolve(
		root.path(),
		&env(&with_passwords(&[(
			ANDROID_KEYSTORE,
			"/nowhere/upload.jks",
		)])),
	)
	.err()
	.unwrap()
	.to_string();
	assert!(error.contains("doesn't exist"), "{error}");
}

#[test]
fn an_android_release_refuses_the_debug_key() {
	let root = project();
	let (_directory, path) = keystore();
	let error = AndroidSigning::resolve(
		root.path(),
		&env(&[
			(ANDROID_KEYSTORE, &path),
			(ANDROID_KEYSTORE_PASSWORD, "android"),
			(ANDROID_KEY_ALIAS, "AndroidDebugKey"),
			(ANDROID_KEY_PASSWORD, "android"),
		]),
	)
	.err()
	.unwrap()
	.to_string();
	assert!(error.contains("debug key"), "{error}");

	let directory = tempfile::tempdir().unwrap();
	let debug = directory.path().join("debug.keystore");
	std::fs::write(&debug, b"keystore").unwrap();
	let error = AndroidSigning::resolve(
		root.path(),
		&env(&with_passwords(&[(
			ANDROID_KEYSTORE,
			&debug.display().to_string(),
		)])),
	)
	.err()
	.unwrap()
	.to_string();
	assert!(error.contains("debug keystore"), "{error}");
}

#[test]
fn a_base64_keystore_is_written_privately_and_removed_after_the_build() {
	let root = project();
	let encoded = base64::engine::general_purpose::STANDARD.encode(b"keystore bytes");
	// Line breaks, as `base64` wraps its output, are ignored.
	let wrapped = format!("{}\n{}", &encoded[..8], &encoded[8..]);
	let signing = AndroidSigning::resolve(
		root.path(),
		&env(&with_passwords(&[(ANDROID_KEYSTORE_BASE64, &wrapped)])),
	)
	.unwrap();
	let path = signing.keystore.clone();
	assert_eq!(std::fs::read(&path).unwrap(), b"keystore bytes");
	assert!(!path.starts_with(root.path()), "never inside the project");
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		let mode = std::fs::metadata(&path).unwrap().permissions().mode();
		assert_eq!(mode & 0o777, 0o600, "readable only by this user");
	}
	drop(signing);
	assert!(!path.exists(), "removed when the build is done");

	let error = AndroidSigning::resolve(
		root.path(),
		&env(&with_passwords(&[(
			ANDROID_KEYSTORE_BASE64,
			"not base64 at all!",
		)])),
	)
	.err()
	.unwrap()
	.to_string();
	assert_eq!(error, format!("{ANDROID_KEYSTORE_BASE64} isn't base64"));

	let (_directory, keystore) = keystore();
	let error = AndroidSigning::resolve(
		root.path(),
		&env(&with_passwords(&[
			(ANDROID_KEYSTORE, &keystore),
			(ANDROID_KEYSTORE_BASE64, &encoded),
		])),
	)
	.err()
	.unwrap()
	.to_string();
	assert!(error.contains("set only one"), "{error}");
}

#[test]
fn gradle_gets_the_keystore_path_in_its_environment_and_no_secret_on_its_command_line() {
	let root = project();
	let (_directory, path) = keystore();
	let signing = AndroidSigning::resolve(
		root.path(),
		&env(&with_passwords(&[(ANDROID_KEYSTORE, &path)])),
	)
	.unwrap();
	let android = Path::new("/app/android");
	let tool = gradle_release(android, &signing, Some(42));

	assert_eq!(
		Path::new(&tool.program),
		android.join(if cfg!(windows) {
			"gradlew.bat"
		} else {
			"gradlew"
		})
	);
	assert_eq!(tool.dir.as_deref(), Some(android));
	assert_eq!(
		tool.args,
		[
			"--no-daemon",
			"--console=plain",
			"--quiet",
			"assembleRelease",
			"bundleRelease",
			"-Pleptoskit.versionCode=42",
		]
	);
	let names: Vec<_> = tool.env.iter().map(|(name, _)| *name).collect();
	assert_eq!(
		names,
		[ANDROID_KEYSTORE],
		"the passwords stay in the environment Gradle inherits"
	);
	assert_eq!(
		tool.env[0].1.expose(),
		std::fs::canonicalize(&path).unwrap().display().to_string()
	);
	let printed = format!("{tool:?}");

	for (_, secret) in PASSWORDS {
		assert!(!tool.args.iter().any(|arg| arg.contains(secret)));
		assert!(!printed.contains(secret), "{printed}");
	}

	let unnumbered = gradle_release(android, &signing, None);
	assert!(
		!unnumbered
			.args
			.iter()
			.any(|arg| arg.contains("versionCode"))
	);
}

#[test]
fn the_gradle_script_must_sign_the_release_with_the_upload_key() {
	let path = Path::new("android/app/build.gradle.kts");
	let template = include_str!("../templates/build.gradle.kts");
	assert!(
		android_script_problems(template, path).is_empty(),
		"`leptoskit new`'s script signs with the upload key"
	);

	let unsigned = "android { buildTypes { release { isMinifyEnabled = true } } }";
	let problems = android_script_problems(unsigned, path);
	assert_eq!(problems.len(), 1);
	assert!(problems[0].contains(ANDROID_KEYSTORE), "{problems:?}");

	let debug = format!(
		"{template}\nandroid {{ buildTypes {{ release {{ signingConfig = \
		 signingConfigs.getByName(\"debug\") }} }} }}"
	);
	let problems = android_script_problems(&debug, path);
	assert_eq!(problems.len(), 1);
	assert!(problems[0].contains("debug key"), "{problems:?}");
}

/// A ZIP of `entries`, with `before_directory` between the last entry and
/// the central directory, where an APK keeps its signing block.
fn zip(entries: &[&str], before_directory: &[u8]) -> Vec<u8> {
	let mut out = Vec::new();
	let mut directory = Vec::new();

	for name in entries {
		let offset = out.len() as u32;
		out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
		out.extend_from_slice(&[0; 22]);
		out.extend_from_slice(&(name.len() as u16).to_le_bytes());
		out.extend_from_slice(&0_u16.to_le_bytes());
		out.extend_from_slice(name.as_bytes());

		directory.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
		directory.extend_from_slice(&[0; 24]);
		directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
		directory.extend_from_slice(&[0; 12]);
		directory.extend_from_slice(&offset.to_le_bytes());
		directory.extend_from_slice(name.as_bytes());
	}

	out.extend_from_slice(before_directory);
	let start = out.len() as u32;
	out.extend_from_slice(&directory);
	out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
	out.extend_from_slice(&[0; 4]);
	out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
	out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
	out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
	out.extend_from_slice(&start.to_le_bytes());
	out.extend_from_slice(&0_u16.to_le_bytes());
	out
}

fn signing_block() -> Vec<u8> {
	let mut block = vec![0; 32];
	block.extend_from_slice(b"APK Sig Block 42");
	block
}

#[test]
fn only_a_signed_apk_and_app_bundle_count_as_the_release() {
	let android = tempfile::tempdir().unwrap();
	let apks = android.path().join(APK_DIR);
	let bundles = android.path().join(BUNDLE_DIR);
	std::fs::create_dir_all(&apks).unwrap();
	std::fs::create_dir_all(&bundles).unwrap();

	// Gradle names an APK it didn't sign `-unsigned`.
	std::fs::write(
		apks.join("app-release-unsigned.apk"),
		zip(&["AndroidManifest.xml"], &[]),
	)
	.unwrap();
	let error = signed_android_outputs(android.path())
		.unwrap_err()
		.to_string();
	assert!(error.contains("unsigned release APK"), "{error}");

	clear_android_outputs(android.path()).unwrap();
	assert!(!apks.exists() && !bundles.exists());
	std::fs::create_dir_all(&apks).unwrap();
	std::fs::create_dir_all(&bundles).unwrap();

	// An APK without a signing block isn't signed.
	let apk = apks.join("app-release.apk");
	std::fs::write(&apk, zip(&["AndroidManifest.xml"], &[])).unwrap();
	let error = signed_android_outputs(android.path())
		.unwrap_err()
		.to_string();
	assert!(error.contains("isn't signed"), "{error}");

	std::fs::write(&apk, zip(&["AndroidManifest.xml"], &signing_block())).unwrap();
	let error = signed_android_outputs(android.path())
		.unwrap_err()
		.to_string();
	assert!(error.contains("no App Bundle"), "{error}");

	// An App Bundle is signed as a JAR, with a block under META-INF.
	let bundle = bundles.join("app-release.aab");
	std::fs::write(
		&bundle,
		zip(
			&["base/manifest/AndroidManifest.xml", "META-INF/MANIFEST.MF"],
			&[],
		),
	)
	.unwrap();
	let error = signed_android_outputs(android.path())
		.unwrap_err()
		.to_string();
	assert!(error.contains("isn't signed"), "{error}");

	std::fs::write(
		&bundle,
		zip(
			&[
				"base/manifest/AndroidManifest.xml",
				"META-INF/MANIFEST.MF",
				"META-INF/UPLOAD.SF",
				"META-INF/UPLOAD.RSA",
			],
			&[],
		),
	)
	.unwrap();
	assert_eq!(
		signed_android_outputs(android.path()).unwrap(),
		(apk, bundle)
	);
}

fn config(toml: &str) -> LeptoskitConfig {
	config_for("\"ios\", \"android\"", toml)
}

fn config_for(platforms: &str, toml: &str) -> LeptoskitConfig {
	LeptoskitConfig::from_toml(
		&format!(
			"[app]\nname = \"notes\"\nversion = \"1.4.0\"\ndefault_platform = \"ios\"\nplatforms \
			 = [{platforms}]\n[native]\npackage_id = \"com.example.notes\"\n{toml}"
		),
		Path::new("notes"),
	)
	.unwrap()
}

#[test]
fn signed_artifacts_are_collected_by_target_name_and_version() {
	let root = project();
	let built = tempfile::tempdir().unwrap();
	let bundle = built.path().join("app-release.aab");
	let apk = built.path().join("app-release.apk");
	std::fs::write(&bundle, b"aab").unwrap();
	std::fs::write(&apk, b"apk").unwrap();

	let collected = collect(root.path(), &config(""), Target::Quest, &[&bundle, &apk]).unwrap();
	let release = root.path().join(".leptoskit/release/quest");
	assert_eq!(
		collected,
		[
			release.join("notes-1.4.0.aab"),
			release.join("notes-1.4.0.apk")
		]
	);
	assert_eq!(std::fs::read(&collected[1]).unwrap(), b"apk");
	assert_eq!(target_name(Target::AndroidX86_64), "android-x86_64");
}

#[test]
fn unsigned_releases_and_other_targets_need_no_signing_material() {
	let root = project();
	let config = config("");
	let unsigned = ReleaseOptions {
		unsigned: true,
		..ReleaseOptions::default()
	};

	for target in [Target::Android, Target::Quest, Target::Ios] {
		assert!(
			resolve(root.path(), &config, target, &unsigned, &env(&[]))
				.unwrap()
				.is_none()
		);
	}

	for target in [Target::Macos, Target::IosSimulator, Target::Linux] {
		assert!(
			resolve(
				root.path(),
				&config,
				target,
				&ReleaseOptions::default(),
				&env(&[])
			)
			.unwrap()
			.is_none()
		);
	}

	for target in [
		Target::Android,
		Target::AndroidX86_64,
		Target::Quest,
		Target::Ios,
	] {
		assert!(
			resolve(
				root.path(),
				&config,
				target,
				&ReleaseOptions::default(),
				&env(&[])
			)
			.is_err(),
			"{target:?} releases are signed or not built"
		);
	}
}

#[test]
fn the_release_flags_need_a_release_and_unsigned_takes_no_export_method() {
	let parse = |args: &[&str]| {
		crate::cli::Cli::try_parse_from(std::iter::once("leptoskit").chain(args.iter().copied()))
	};

	assert!(parse(&["build", "--platform", "android", "--unsigned"]).is_err());
	assert!(parse(&["build", "--platform", "ios", "--build-number", "3"]).is_err());
	assert!(
		parse(&[
			"build",
			"--release",
			"--platform",
			"ios",
			"--unsigned",
			"--export-method",
			"ad-hoc"
		])
		.is_err()
	);
	assert!(
		parse(&[
			"build",
			"--release",
			"--platform",
			"ios",
			"--export-method",
			"sideways"
		])
		.is_err()
	);
	assert!(
		parse(&[
			"build",
			"--release",
			"--platform",
			"ios",
			"--export-method",
			"ad-hoc",
			"--build-number",
			"7"
		])
		.is_ok()
	);
	assert!(parse(&["native", "build", "android", "--release", "--unsigned"]).is_ok());
}

#[test]
fn an_ios_release_needs_a_team_and_takes_the_environment_s_over_the_config_s() {
	let root = project();
	let options = ReleaseOptions::default();
	let error = IosSigning::resolve(root.path(), &config(""), &options, &env(&[]))
		.err()
		.unwrap()
		.to_string();
	assert!(error.contains(IOS_TEAM_ID), "{error}");
	assert!(error.contains("native.apple_team_id"), "{error}");

	let configured = config("apple_team_id = \"ABCDE12345\"");
	let signing = IosSigning::resolve(root.path(), &configured, &options, &env(&[])).unwrap();
	assert_eq!(signing.team, "ABCDE12345");
	assert_eq!(signing.method, ExportMethod::AppStoreConnect);
	assert!(matches!(signing.style, IosStyle::Automatic { key: None }));
	assert_eq!(signing.bundle_id, "com.example.notes");

	let signing = IosSigning::resolve(
		root.path(),
		&configured,
		&options,
		&env(&[(IOS_TEAM_ID, "ZYXWV98765")]),
	)
	.unwrap();
	assert_eq!(signing.team, "ZYXWV98765");

	let error = IosSigning::resolve(
		root.path(),
		&configured,
		&options,
		&env(&[(IOS_TEAM_ID, "abc")]),
	)
	.err()
	.unwrap()
	.to_string();
	assert!(error.contains("10 uppercase"), "{error}");
}

#[test]
fn the_export_method_comes_from_the_flag_over_the_config() {
	let root = project();
	let configured =
		config("apple_team_id = \"ABCDE12345\"\n[native.ios]\nexport-method = \"development\"\n");
	let signing = IosSigning::resolve(
		root.path(),
		&configured,
		&ReleaseOptions::default(),
		&env(&[]),
	)
	.unwrap();
	assert_eq!(signing.method, ExportMethod::Development);

	let signing = IosSigning::resolve(
		root.path(),
		&configured,
		&ReleaseOptions {
			export_method: Some(ExportMethod::AdHoc),
			..ReleaseOptions::default()
		},
		&env(&[]),
	)
	.unwrap();
	assert_eq!(signing.method, ExportMethod::AdHoc);
}

#[test]
fn manual_ios_signing_names_its_identity_and_profile() {
	let root = project();
	let options = ReleaseOptions::default();
	let manual =
		config("apple_team_id = \"ABCDE12345\"\n[native.ios]\nsigning-style = \"manual\"\n");
	let error = IosSigning::resolve(root.path(), &manual, &options, &env(&[]))
		.err()
		.unwrap()
		.to_string();
	assert!(error.contains(IOS_CODE_SIGN_IDENTITY), "{error}");
	assert!(error.contains(IOS_PROVISIONING_PROFILE), "{error}");

	let identity = [
		(IOS_CODE_SIGN_IDENTITY, "Apple Distribution"),
		(IOS_PROVISIONING_PROFILE, "Notes App Store"),
	];
	let signing = IosSigning::resolve(root.path(), &manual, &options, &env(&identity)).unwrap();
	assert!(matches!(
		&signing.style,
		IosStyle::Manual { identity, profile }
			if identity == "Apple Distribution" && profile == "Notes App Store"
	));

	// The environment chooses the style too.
	let automatic = config("apple_team_id = \"ABCDE12345\"");
	let mut pairs = identity.to_vec();
	pairs.push((IOS_SIGNING_STYLE, "manual"));
	let signing = IosSigning::resolve(root.path(), &automatic, &options, &env(&pairs)).unwrap();
	assert!(matches!(signing.style, IosStyle::Manual { .. }));

	let error = IosSigning::resolve(
		root.path(),
		&automatic,
		&options,
		&env(&[(IOS_SIGNING_STYLE, "sideways")]),
	)
	.err()
	.unwrap()
	.to_string();
	assert!(error.contains(IOS_SIGNING_STYLE), "{error}");

	// One profile can't sign the Safari extension as well as the app.
	let safari = config_for(
		"\"ios\", \"extension\"",
		"apple_team_id = \"ABCDE12345\"\n[native.ios]\nsigning-style = \
		 \"manual\"\n[extension]\nname = \"Notes\"\nbrowsers = \
		 [\"safari\"]\n[extension.popup]\nroute = \"/\"\n",
	);
	let error = IosSigning::resolve(root.path(), &safari, &options, &env(&identity))
		.err()
		.unwrap()
		.to_string();
	assert!(error.contains("Safari extension"), "{error}");
}

const P8: &str = "-----BEGIN PRIVATE \
                  KEY-----\nMIGTAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBHkwdwIBAQQg\n-----END PRIVATE \
                  KEY-----\n";

#[test]
fn automatic_ios_signing_signs_in_with_the_whole_app_store_connect_key() {
	let root = project();
	let configured = config("apple_team_id = \"ABCDE12345\"");
	let options = ReleaseOptions::default();
	let error = IosSigning::resolve(
		root.path(),
		&configured,
		&options,
		&env(&[(app_store::KEY_ID, "KEY123")]),
	)
	.err()
	.unwrap()
	.to_string();
	assert!(error.contains(app_store::ISSUER), "{error}");
	assert!(error.contains(app_store::KEY), "{error}");

	// A key given as text is written privately for Xcode, and removed.
	let signing = IosSigning::resolve(
		root.path(),
		&configured,
		&options,
		&env(&[
			(app_store::ISSUER, "issuer-1"),
			(app_store::KEY_ID, "KEY123"),
			(app_store::KEY, P8),
		]),
	)
	.unwrap();
	let IosStyle::Automatic { key: Some(key) } = &signing.style else {
		panic!("automatic signing with the key");
	};
	let path = key.path.clone();
	assert_eq!(std::fs::read_to_string(&path).unwrap(), P8);
	assert!(!path.starts_with(root.path()));
	let arguments = signing.authentication();
	assert_eq!(
		arguments,
		[
			"-allowProvisioningUpdates",
			"-authenticationKeyPath",
			&path.display().to_string(),
			"-authenticationKeyID",
			"KEY123",
			"-authenticationKeyIssuerID",
			"issuer-1",
		]
	);
	assert!(
		!arguments
			.iter()
			.any(|argument| argument.contains("PRIVATE KEY"))
	);
	drop(signing);
	assert!(!path.exists(), "removed when the build is done");

	// A key file inside the project is refused.
	let inside = root.path().join("AuthKey_KEY123.p8");
	std::fs::write(&inside, P8).unwrap();
	let error = IosSigning::resolve(
		root.path(),
		&configured,
		&options,
		&env(&[
			(app_store::ISSUER, "issuer-1"),
			(app_store::KEY_ID, "KEY123"),
			(app_store::KEY, &inside.display().to_string()),
		]),
	)
	.err()
	.unwrap()
	.to_string();
	assert!(error.contains("inside the project's repository"), "{error}");
}

fn automatic(method: ExportMethod) -> IosSigning {
	IosSigning {
		team: "ABCDE12345".into(),
		method,
		style: IosStyle::Automatic {
			key: Some(app_store::KeyFile::at(
				"/keys/AuthKey_KEY123.p8".into(),
				Secret::new("KEY123".into()),
				Secret::new("issuer-1".into()),
			)),
		},
		bundle_id: "com.example.notes".into(),
	}
}

fn manual(method: ExportMethod) -> IosSigning {
	IosSigning {
		team: "ABCDE12345".into(),
		method,
		style: IosStyle::Manual {
			identity: "Apple Distribution".into(),
			profile: "Notes Ad Hoc".into(),
		},
		bundle_id: "com.example.notes".into(),
	}
}

#[test]
fn a_signed_ios_release_archives_and_exports_with_its_signing() {
	let root = Path::new("/app");
	let arguments = |tool: &Tool| tool.args.join(" ").replace('\\', "/");
	let archive = ios_archive(root, &automatic(ExportMethod::AppStoreConnect), Some(42));
	assert_eq!(archive.program, "xcodebuild");
	assert_eq!(archive.dir.as_deref(), Some(root));
	assert_eq!(archive.unset, ["LD", "LDPLUSPLUS"]);
	insta::assert_snapshot!(arguments(&archive), @"-quiet archive -project .leptoskit/ios/Host.xcodeproj -scheme App -configuration Release -destination generic/platform=iOS -archivePath /app/.leptoskit/release/ios/App.xcarchive -derivedDataPath .leptoskit/ios/build -allowProvisioningUpdates -authenticationKeyPath /keys/AuthKey_KEY123.p8 -authenticationKeyID KEY123 -authenticationKeyIssuerID issuer-1 DEVELOPMENT_TEAM=ABCDE12345 CODE_SIGN_STYLE=Automatic CURRENT_PROJECT_VERSION=42");
	assert!(
		!archive
			.args
			.iter()
			.any(|arg| arg.contains("CODE_SIGNING_ALLOWED")),
		"a release is never built unsigned"
	);

	let export = ios_export(root, &automatic(ExportMethod::AppStoreConnect));
	insta::assert_snapshot!(arguments(&export), @"-quiet -exportArchive -archivePath /app/.leptoskit/release/ios/App.xcarchive -exportPath /app/.leptoskit/release/ios -exportOptionsPlist /app/.leptoskit/release/ios/ExportOptions.plist -allowProvisioningUpdates -authenticationKeyPath /keys/AuthKey_KEY123.p8 -authenticationKeyID KEY123 -authenticationKeyIssuerID issuer-1");

	let archive = ios_archive(root, &manual(ExportMethod::AdHoc), None);
	insta::assert_snapshot!(arguments(&archive), @"-quiet archive -project .leptoskit/ios/Host.xcodeproj -scheme App -configuration Release -destination generic/platform=iOS -archivePath /app/.leptoskit/release/ios/App.xcarchive -derivedDataPath .leptoskit/ios/build DEVELOPMENT_TEAM=ABCDE12345 CODE_SIGN_STYLE=Manual CODE_SIGN_IDENTITY=Apple Distribution PROVISIONING_PROFILE_SPECIFIER=Notes Ad Hoc");
	assert!(
		!ios_export(root, &manual(ExportMethod::AdHoc))
			.args
			.iter()
			.any(|arg| arg.starts_with("-allowProvisioningUpdates"))
	);

	let unattended = IosSigning {
		style: IosStyle::Automatic { key: None },
		..automatic(ExportMethod::Development)
	};
	assert!(
		ios_archive(root, &unattended, None)
			.args
			.contains(&"-allowProvisioningUpdates".to_owned()),
		"Xcode signs in with its own account"
	);
}

fn plist(text: &str) -> plist::Dictionary {
	plist::Value::from_reader_xml(text.as_bytes())
		.unwrap()
		.into_dictionary()
		.unwrap()
}

#[test]
fn export_options_follow_the_method_and_the_signing_style() {
	let options = plist(&export_options(&automatic(ExportMethod::AppStoreConnect)).unwrap());
	let get = |options: &plist::Dictionary, key: &str| options.get(key).cloned();
	assert_eq!(
		get(&options, "method").and_then(plist::Value::into_string),
		Some("app-store-connect".into())
	);
	assert_eq!(
		get(&options, "destination").and_then(plist::Value::into_string),
		Some("export".into()),
		"written to disk, never uploaded"
	);
	assert_eq!(
		get(&options, "teamID").and_then(plist::Value::into_string),
		Some("ABCDE12345".into())
	);
	assert_eq!(
		get(&options, "signingStyle").and_then(plist::Value::into_string),
		Some("automatic".into())
	);
	assert_eq!(
		get(&options, "manageAppVersionAndBuildNumber").and_then(|value| value.as_boolean()),
		Some(false)
	);
	assert!(get(&options, "provisioningProfiles").is_none());

	let options = plist(&export_options(&manual(ExportMethod::AdHoc)).unwrap());
	assert_eq!(
		get(&options, "method").and_then(plist::Value::into_string),
		Some("release-testing".into()),
		"Xcode's name for ad hoc"
	);
	assert_eq!(
		get(&options, "signingStyle").and_then(plist::Value::into_string),
		Some("manual".into())
	);
	assert_eq!(
		get(&options, "signingCertificate").and_then(plist::Value::into_string),
		Some("Apple Distribution".into())
	);
	let profiles = get(&options, "provisioningProfiles")
		.and_then(plist::Value::into_dictionary)
		.unwrap();
	assert_eq!(
		profiles
			.get("com.example.notes")
			.and_then(plist::Value::as_string),
		Some("Notes Ad Hoc")
	);
	assert!(get(&options, "uploadSymbols").is_none());

	let options = plist(&export_options(&automatic(ExportMethod::Development)).unwrap());
	assert_eq!(
		get(&options, "method").and_then(plist::Value::into_string),
		Some("debugging".into())
	);
}

#[test]
fn the_exported_ipa_is_named_for_the_app_and_its_version() {
	let root = project();
	clear_ios_output(root.path()).unwrap();
	let out = ios_output(root.path());
	let error = exported_ipa(root.path(), &config(""))
		.unwrap_err()
		.to_string();
	assert!(error.contains("no .ipa"), "{error}");

	std::fs::write(out.join("App.ipa"), b"ipa").unwrap();
	let ipa = exported_ipa(root.path(), &config("")).unwrap();
	assert_eq!(ipa, out.join("notes-1.4.0.ipa"));
	assert_eq!(std::fs::read(&ipa).unwrap(), b"ipa");

	clear_ios_output(root.path()).unwrap();
	assert!(!ipa.exists(), "an earlier release is cleared");
}
