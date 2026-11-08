//! Signing a release build for a store.
//!
//! `leptoskit build --release` for Android, Meta Quest, and iOS makes what a
//! store takes: a signed App Bundle and APK, or an `.ipa`. The signing
//! material comes only from the environment, so a secret manager can hand it
//! to one build, and a file inside the project's repository is refused. A
//! release build never falls back to the debug key or to an unsigned build:
//! without the material it stops before anything is compiled, and names
//! what is missing. `--unsigned` asks for an unsigned build on purpose, for
//! measuring or testing.
//!
//! Secrets never reach a command line, where `ps` could show them: Gradle
//! reads the keystore's passwords from the environment itself, and only the
//! keystore's path is handed to it, in its environment.

use std::io::Read as _;
use std::io::Seek as _;
use std::io::SeekFrom;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use base64::Engine as _;
use leptoskit_config::ExportMethod;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::SigningStyle;

use super::Target;
use crate::publish::app_store;
use crate::publish::secrets::Secret;
use crate::publish::tools::Tool;

/// The Android upload keystore: its absolute path.
pub const ANDROID_KEYSTORE: &str = "LEPTOSKIT_ANDROID_KEYSTORE";
/// The Android upload keystore itself, base64-encoded, in place of its path.
pub const ANDROID_KEYSTORE_BASE64: &str = "LEPTOSKIT_ANDROID_KEYSTORE_BASE64";
/// The keystore's password.
pub const ANDROID_KEYSTORE_PASSWORD: &str = "LEPTOSKIT_ANDROID_KEYSTORE_PASSWORD";
/// The upload key's alias in the keystore.
pub const ANDROID_KEY_ALIAS: &str = "LEPTOSKIT_ANDROID_KEY_ALIAS";
/// The upload key's password.
pub const ANDROID_KEY_PASSWORD: &str = "LEPTOSKIT_ANDROID_KEY_PASSWORD";

/// The Apple Developer team that signs the iOS app; `native.apple_team_id`
/// when unset.
pub const IOS_TEAM_ID: &str = "LEPTOSKIT_IOS_TEAM_ID";
/// `automatic` or `manual`, over `native.ios.signing-style`.
pub const IOS_SIGNING_STYLE: &str = "LEPTOSKIT_IOS_SIGNING_STYLE";
/// Manual signing's identity, such as `Apple Distribution` or a
/// certificate's SHA-1.
pub const IOS_CODE_SIGN_IDENTITY: &str = "LEPTOSKIT_IOS_CODE_SIGN_IDENTITY";
/// Manual signing's provisioning profile: its name or UUID.
pub const IOS_PROVISIONING_PROFILE: &str = "LEPTOSKIT_IOS_PROVISIONING_PROFILE";

/// The alias Android's debug keystore signs with.
const DEBUG_ALIAS: &str = "androiddebugkey";

/// Where a signed release's artifacts are collected, under the app's root.
pub const OUTPUT: &str = ".leptoskit/release";

/// A release build's options on the command line.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
pub struct ReleaseOptions {
	/// Build the release without signing it: an APK or app no store takes,
	/// for measuring or testing. Without it, an Android, Quest, or iOS
	/// release is signed, or not built.
	#[arg(long, requires = "release")]
	pub unsigned: bool,
	/// Who an iOS release's `.ipa` is for: app-store-connect, ad-hoc, or
	/// development. Defaults to `native.ios.export-method`.
	#[arg(long, requires = "release", conflicts_with = "unsigned")]
	pub export_method: Option<ExportMethod>,
	/// The release's build number: Android's `versionCode`, and iOS's
	/// `CFBundleVersion`. Each store upload needs a higher one.
	#[arg(long, requires = "release")]
	pub build_number: Option<u64>,
}

/// Reads one environment variable. Tests pass their own.
pub(crate) type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The process's environment, with empty values treated as unset.
pub(crate) fn process_env(name: &str) -> Option<String> {
	std::env::var(name)
		.ok()
		.filter(|value| !value.trim().is_empty())
}

/// What signs a release build.
pub(crate) enum Signing {
	Android(AndroidSigning),
	Ios(IosSigning),
}

/// What signs `target`'s release, read before anything is compiled so a
/// missing variable fails in seconds. `None` for targets a store doesn't
/// sign through these variables, and for `--unsigned`.
///
/// # Errors
///
/// When the signing material is missing, inside the repository, or the
/// debug key.
pub(crate) fn resolve(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
	options: &ReleaseOptions,
	env: Env<'_>,
) -> Result<Option<Signing>> {
	if options.unsigned {
		return Ok(None);
	}

	match target {
		Target::Android | Target::AndroidX86_64 | Target::Quest => {
			AndroidSigning::resolve(root, env).map(|signing| Some(Signing::Android(signing)))
		}
		Target::Ios => {
			IosSigning::resolve(root, config, options, env)
				.map(|signing| Some(Signing::Ios(signing)))
		}
		_ => Ok(None),
	}
}

/// Refuses `path` inside the project's repository: signing material is
/// never read from it, so it can't be committed.
fn outside_repository(root: &Path, path: &Path, variable: &str, instead: &str) -> Result<()> {
	let repository = crate::git::command(root)
		.args(["rev-parse", "--show-toplevel"])
		.stderr(std::process::Stdio::null())
		.output()
		.ok()
		.filter(|output| output.status.success())
		.map_or_else(
			|| root.to_path_buf(),
			|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()),
		);
	let repository = std::fs::canonicalize(&repository).unwrap_or(repository);
	let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

	anyhow::ensure!(
		!path.starts_with(&repository),
		"{variable} names a file inside the project's repository; signing material is never read \
		 from it, so it can't be committed. Keep the file outside the repository, or {instead}"
	);
	Ok(())
}

/// Writes `bytes` to a new file at `path`, readable only by this user.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
	use std::io::Write as _;

	let mut options = std::fs::OpenOptions::new();
	options.write(true).create_new(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt as _;
		options.mode(0o600);
	}
	options
		.open(path)
		.and_then(|mut file| file.write_all(bytes))
		.context("could not write the signing material to a private temporary file")
}

/// The Android upload key, as Gradle's release `signingConfig` reads it.
pub(crate) struct AndroidSigning {
	/// The keystore's absolute path, which Gradle reads.
	keystore: PathBuf,
	/// The private directory a base64 keystore was written to, removed
	/// when the build is done.
	_temporary: Option<tempfile::TempDir>,
}

impl AndroidSigning {
	/// The upload key from the environment. The three passwords and the
	/// alias stay there: Gradle reads them itself.
	///
	/// # Errors
	///
	/// When a variable is missing, the keystore is unreadable, inside the
	/// repository, or the debug keystore.
	pub(crate) fn resolve(root: &Path, env: Env<'_>) -> Result<Self> {
		let path = env(ANDROID_KEYSTORE);
		let encoded = env(ANDROID_KEYSTORE_BASE64);
		let mut missing = Vec::new();

		if path.is_none() && encoded.is_none() {
			missing.push(format!("{ANDROID_KEYSTORE} (or {ANDROID_KEYSTORE_BASE64})"));
		}

		for name in [
			ANDROID_KEYSTORE_PASSWORD,
			ANDROID_KEY_ALIAS,
			ANDROID_KEY_PASSWORD,
		] {
			if env(name).is_none() {
				missing.push(name.to_owned());
			}
		}

		anyhow::ensure!(
			missing.is_empty(),
			"a release Android build is signed with your upload key, and {}. Set {} in the \
			 environment, for example with a secret manager around this one build, or pass \
			 --unsigned for a build no store takes. To make an upload key, {}",
			if missing.len() == 1 {
				format!("{} isn't set", missing[0])
			} else {
				format!("these aren't set: {}", missing.join(", "))
			},
			if missing.len() == 1 { "it" } else { "them" },
			crate::publish::play::KEYSTORE_HOW
		);
		anyhow::ensure!(
			path.is_none() || encoded.is_none(),
			"both {ANDROID_KEYSTORE} and {ANDROID_KEYSTORE_BASE64} are set; set only one"
		);
		anyhow::ensure!(
			!env(ANDROID_KEY_ALIAS)
				.unwrap_or_default()
				.trim()
				.eq_ignore_ascii_case(DEBUG_ALIAS),
			"{ANDROID_KEY_ALIAS} names Android's debug key, which no store takes; sign the \
			 release with your upload key"
		);

		if let Some(path) = path {
			let path = PathBuf::from(path.trim());
			anyhow::ensure!(
				path.is_file(),
				"{ANDROID_KEYSTORE} names a file that doesn't exist or can't be read; set it to \
				 the keystore's absolute path"
			);
			anyhow::ensure!(
				path.file_name().is_none_or(|name| name != "debug.keystore"),
				"{ANDROID_KEYSTORE} names Android's debug keystore, which no store takes; sign \
				 the release with your upload key"
			);
			outside_repository(
				root,
				&path,
				ANDROID_KEYSTORE,
				&format!("set {ANDROID_KEYSTORE_BASE64} to its contents instead"),
			)?;
			let keystore = std::fs::canonicalize(&path)
				.with_context(|| format!("{ANDROID_KEYSTORE} names a file that can't be read"))?;

			return Ok(Self {
				keystore,
				_temporary: None,
			});
		}

		let encoded: String = encoded
			.unwrap_or_default()
			.chars()
			.filter(|character| !character.is_ascii_whitespace())
			.collect();
		// The decoder's error quotes the offending byte, which here is a
		// secret's.
		let bytes = base64::engine::general_purpose::STANDARD
			.decode(encoded)
			.map_err(|_| anyhow::anyhow!("{ANDROID_KEYSTORE_BASE64} isn't base64"))?;
		anyhow::ensure!(!bytes.is_empty(), "{ANDROID_KEYSTORE_BASE64} is empty");
		let temporary = tempfile::Builder::new()
			.prefix("leptoskit-keystore-")
			.tempdir()
			.context("could not make a private temporary directory for the keystore")?;
		let keystore = temporary.path().join("upload.jks");
		write_private(&keystore, &bytes)?;

		Ok(Self {
			keystore,
			_temporary: Some(temporary),
		})
	}
}

/// What keeps the app's Gradle script from signing the release with the
/// upload key: it doesn't read the key, or signs with the debug key.
#[must_use]
pub(crate) fn android_script_problems(script: &str, path: &Path) -> Vec<String> {
	let mut problems = Vec::new();

	if !script.contains(ANDROID_KEYSTORE) {
		problems.push(format!(
			"{} doesn't sign the release with the upload key: add the signingConfigs block that \
			 reads {ANDROID_KEYSTORE}, as `leptoskit new` writes it, and set the release build \
			 type's signingConfig to it",
			path.display()
		));
	}

	if [
		"signingConfigs.getByName(\"debug\")",
		"signingConfigs.findByName(\"debug\")",
		"signingConfigs[\"debug\"]",
		"signingConfigs.debug",
	]
	.iter()
	.any(|debug| script.contains(debug))
	{
		problems.push(format!(
			"{} signs with the debug key, which no store takes; sign the release with the \
			 signingConfig that reads {ANDROID_KEYSTORE}",
			path.display()
		));
	}

	problems
}

/// Refuses the Android host's Gradle script when it wouldn't sign the
/// release with the upload key.
///
/// # Errors
///
/// When the script can't be read, or has a problem.
pub(crate) fn check_android_script(android: &Path) -> Result<()> {
	let script = android.join("app/build.gradle.kts");
	let text = std::fs::read_to_string(&script)
		.with_context(|| format!("could not read {}", script.display()))?;
	let problems = android_script_problems(&text, &script);
	anyhow::ensure!(problems.is_empty(), "{}", problems.join("\n"));
	Ok(())
}

/// `gradlew assembleRelease bundleRelease`: the signed APK, and the signed
/// App Bundle. The keystore's path reaches Gradle in its environment; the
/// passwords and alias are already there.
#[must_use]
pub(crate) fn gradle_release(
	android: &Path,
	signing: &AndroidSigning,
	build_number: Option<u64>,
) -> Tool {
	let mut tool = Tool::new(
		&android
			.join(if cfg!(windows) {
				"gradlew.bat"
			} else {
				"gradlew"
			})
			.display()
			.to_string(),
	)
	.dir(android)
	.args([
		"--no-daemon",
		"--console=plain",
		"--quiet",
		"assembleRelease",
		"bundleRelease",
	]);

	if let Some(build_number) = build_number {
		tool = tool.arg(format!("-Pleptoskit.versionCode={build_number}"));
	}

	tool.env(
		ANDROID_KEYSTORE,
		Secret::new(signing.keystore.display().to_string()),
	)
}

/// Where Gradle writes the release APK and App Bundle, under the host.
const APK_DIR: &str = "app/build/outputs/apk/release";
const BUNDLE_DIR: &str = "app/build/outputs/bundle/release";

/// Removes the release outputs of an earlier build, so the check after this
/// one reads what this one made.
pub(crate) fn clear_android_outputs(android: &Path) -> Result<()> {
	for directory in [APK_DIR, BUNDLE_DIR] {
		let directory = android.join(directory);

		if directory.exists() {
			std::fs::remove_dir_all(&directory)
				.with_context(|| format!("could not clear {}", directory.display()))?;
		}
	}

	Ok(())
}

/// The signed APK and App Bundle Gradle made, checked to be signed.
///
/// # Errors
///
/// When either is missing or unsigned.
pub(crate) fn signed_android_outputs(android: &Path) -> Result<(PathBuf, PathBuf)> {
	let apks = android.join(APK_DIR);
	anyhow::ensure!(
		!apks.join("app-release-unsigned.apk").exists(),
		"Gradle made an unsigned release APK: the app's Gradle script doesn't give the release \
		 build type the signingConfig that reads {ANDROID_KEYSTORE}"
	);
	let apk = apks.join("app-release.apk");
	anyhow::ensure!(
		apk.is_file(),
		"Gradle made no release APK at {}",
		apk.display()
	);
	anyhow::ensure!(
		has_apk_signing_block(&apk)?,
		"{} isn't signed with an APK signature",
		apk.display()
	);
	let bundle = android.join(BUNDLE_DIR).join("app-release.aab");
	anyhow::ensure!(
		bundle.is_file(),
		"Gradle made no App Bundle at {}",
		bundle.display()
	);
	anyhow::ensure!(
		is_jar_signed(&bundle)?,
		"{} isn't signed: it carries no signature under META-INF",
		bundle.display()
	);
	Ok((apk, bundle))
}

/// The ZIP's end of central directory: where its central directory starts,
/// and how long it is.
fn central_directory(file: &mut std::fs::File) -> Result<(u64, u64)> {
	const END: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
	let length = file.metadata()?.len();
	// The record is 22 bytes, followed by a comment of at most 65535.
	let tail = length.min(22 + 65_535);
	file.seek(SeekFrom::Start(length - tail))?;
	let mut buffer = Vec::new();
	file.take(tail).read_to_end(&mut buffer)?;
	let at = buffer
		.windows(4)
		.rposition(|window| window == END)
		.filter(|at| at + 22 <= buffer.len())
		.context("not a ZIP archive")?;
	let record = &buffer[at..at + 22];
	let size = u32::from_le_bytes([record[12], record[13], record[14], record[15]]);
	let offset = u32::from_le_bytes([record[16], record[17], record[18], record[19]]);
	anyhow::ensure!(
		size != u32::MAX && offset != u32::MAX,
		"the archive is a ZIP64 archive, which isn't checked"
	);
	Ok((u64::from(offset), u64::from(size)))
}

/// Whether the APK carries an APK Signing Block (signature scheme v2 or
/// later), which sits right before the central directory and ends with
/// its magic.
fn has_apk_signing_block(path: &Path) -> Result<bool> {
	let mut file =
		std::fs::File::open(path).with_context(|| format!("could not read {}", path.display()))?;
	let (offset, _) = central_directory(&mut file)
		.with_context(|| format!("could not read {}", path.display()))?;

	if offset < 16 {
		return Ok(false);
	}

	file.seek(SeekFrom::Start(offset - 16))?;
	let mut magic = [0_u8; 16];
	file.read_exact(&mut magic)?;
	Ok(&magic == b"APK Sig Block 42")
}

/// Whether the archive is signed as a JAR, as `jarsigner` signs an App
/// Bundle: a signature block under `META-INF`.
fn is_jar_signed(path: &Path) -> Result<bool> {
	const ENTRY: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
	let mut file =
		std::fs::File::open(path).with_context(|| format!("could not read {}", path.display()))?;
	let (offset, size) = central_directory(&mut file)
		.with_context(|| format!("could not read {}", path.display()))?;
	file.seek(SeekFrom::Start(offset))?;
	let mut directory = Vec::new();
	file.take(size).read_to_end(&mut directory)?;
	let mut at = 0;

	while at + 46 <= directory.len() && directory[at..at + 4] == ENTRY {
		let field = |index: usize| {
			usize::from(u16::from_le_bytes([
				directory[at + index],
				directory[at + index + 1],
			]))
		};
		let (length, extra, comment) = (field(28), field(30), field(32));
		let start = at + 46;
		let Some(name) = directory.get(start..start + length) else {
			break;
		};
		let name = String::from_utf8_lossy(name).to_ascii_uppercase();

		if name.starts_with("META-INF/")
			&& [".RSA", ".EC", ".DSA"]
				.iter()
				.any(|extension| name.ends_with(extension))
		{
			return Ok(true);
		}

		at = start + length + extra + comment;
	}

	Ok(false)
}

/// Copies a signed release's artifacts to `.leptoskit/release/<target>/`,
/// named for the app and its version, and returns where they went.
pub(crate) fn collect(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
	artifacts: &[&Path],
) -> Result<Vec<PathBuf>> {
	let directory = root.join(OUTPUT).join(target_name(target));
	std::fs::create_dir_all(&directory)?;
	let mut collected = Vec::new();

	for artifact in artifacts {
		let extension = artifact
			.extension()
			.and_then(|extension| extension.to_str())
			.unwrap_or_default();
		let destination = directory.join(format!(
			"{}-{}.{extension}",
			config.app.name, config.app.version
		));
		std::fs::copy(artifact, &destination).with_context(|| {
			format!(
				"could not copy {} to {}",
				artifact.display(),
				destination.display()
			)
		})?;
		collected.push(destination);
	}

	Ok(collected)
}

/// The target's name on the command line, such as `android-x86_64`.
fn target_name(target: Target) -> String {
	clap::ValueEnum::to_possible_value(&target).map_or_else(
		|| target.platform().to_string(),
		|value| value.get_name().to_owned(),
	)
}

/// How a release iOS build signs.
pub(crate) enum IosStyle {
	/// Xcode picks the certificate and makes the profile, signed in with
	/// the App Store Connect key when one is set, or the account in Xcode.
	Automatic { key: Option<app_store::KeyFile> },
	/// The identity and profile, installed on this machine, by name.
	Manual { identity: String, profile: String },
}

/// What signs a release iOS build, and how it is exported.
pub(crate) struct IosSigning {
	pub(crate) team: String,
	pub(crate) method: ExportMethod,
	pub(crate) style: IosStyle,
	pub(crate) bundle_id: String,
}

impl IosSigning {
	/// The iOS signing settings: the team, style, and identity from the
	/// environment, over `leptoskit.toml`'s non-secret settings.
	///
	/// # Errors
	///
	/// When the team, the manual identity or profile, or part of the App
	/// Store Connect key is missing.
	pub(crate) fn resolve(
		root: &Path,
		config: &LeptoskitConfig,
		options: &ReleaseOptions,
		env: Env<'_>,
	) -> Result<Self> {
		let team = env(IOS_TEAM_ID)
			.or_else(|| config.native.apple_team_id.clone())
			.map(|team| team.trim().to_owned())
			.with_context(|| {
				format!(
					"a release iOS build is signed for your Apple Developer team, and neither \
					 {IOS_TEAM_ID} nor native.apple_team_id is set. Set one to the team's \
					 10-character ID, from developer.apple.com under Membership, or pass \
					 --unsigned for a build no store takes"
				)
			})?;
		anyhow::ensure!(
			team.len() == 10
				&& team
					.bytes()
					.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()),
			"the Apple team ID must be 10 uppercase letters or digits, such as ABCDE12345"
		);
		let style = match env(IOS_SIGNING_STYLE) {
			Some(style) => {
				style
					.trim()
					.parse::<SigningStyle>()
					.with_context(|| format!("{IOS_SIGNING_STYLE} isn't automatic or manual"))?
			}
			None => config.native.ios.signing_style,
		};
		let method = options
			.export_method
			.unwrap_or(config.native.ios.export_method);

		let style = match style {
			SigningStyle::Manual => {
				let identity = env(IOS_CODE_SIGN_IDENTITY);
				let profile = env(IOS_PROVISIONING_PROFILE);
				let missing: Vec<_> = [
					(IOS_CODE_SIGN_IDENTITY, identity.is_none()),
					(IOS_PROVISIONING_PROFILE, profile.is_none()),
				]
				.into_iter()
				.filter_map(|(name, missing)| missing.then_some(name))
				.collect();
				anyhow::ensure!(
					missing.is_empty(),
					"manual iOS signing names the identity and the provisioning profile, and {} \
					 isn't set. Set the identity (such as `Apple Distribution`) and the profile's \
					 name or UUID, both installed on this machine, or use automatic signing",
					missing.join(" and ")
				);
				anyhow::ensure!(
					!super::safari::configured(config),
					"manual iOS signing names one provisioning profile, and this app also embeds \
					 its Safari extension, which needs its own; use automatic signing"
				);
				// The profile applies to every target Xcode builds, and the
				// pods' own targets refuse one.
				anyhow::ensure!(
					!super::ios_plugins_use_pods(root, config),
					"manual iOS signing names one provisioning profile for every target, and this \
					 app's plugins build CocoaPods targets, which refuse one; use automatic \
					 signing"
				);
				IosStyle::Manual {
					identity: identity.unwrap_or_default().trim().to_owned(),
					profile: profile.unwrap_or_default().trim().to_owned(),
				}
			}
			SigningStyle::Automatic => {
				IosStyle::Automatic {
					key: api_key(root, env)?,
				}
			}
		};

		Ok(Self {
			team,
			method,
			style,
			bundle_id: config.native.package_id.clone(),
		})
	}

	/// `xcodebuild`'s sign-in arguments, for the archive and the export.
	fn authentication(&self) -> Vec<String> {
		match &self.style {
			IosStyle::Automatic { key: Some(key) } => key.xcodebuild_args(),
			IosStyle::Automatic { key: None } => vec!["-allowProvisioningUpdates".to_owned()],
			IosStyle::Manual { .. } => Vec::new(),
		}
	}

	/// The build settings that sign the archive.
	fn build_settings(&self) -> Vec<String> {
		let mut settings = vec![format!("DEVELOPMENT_TEAM={}", self.team)];

		match &self.style {
			IosStyle::Automatic { .. } => settings.push("CODE_SIGN_STYLE=Automatic".to_owned()),
			IosStyle::Manual { identity, profile } => {
				settings.push("CODE_SIGN_STYLE=Manual".to_owned());
				settings.push(format!("CODE_SIGN_IDENTITY={identity}"));
				settings.push(format!("PROVISIONING_PROFILE_SPECIFIER={profile}"));
			}
		}

		settings
	}
}

/// The App Store Connect API key automatic signing signs in with, when
/// its three variables are set: the same ones `leptoskit publish` uses.
fn api_key(root: &Path, env: Env<'_>) -> Result<Option<app_store::KeyFile>> {
	let names = [app_store::ISSUER, app_store::KEY_ID, app_store::KEY];
	let values = names.map(env);

	if values.iter().all(Option::is_none) {
		return Ok(None);
	}

	let missing: Vec<_> = names
		.iter()
		.zip(&values)
		.filter_map(|(name, value)| value.is_none().then_some(*name))
		.collect();
	anyhow::ensure!(
		missing.is_empty(),
		"automatic iOS signing with an App Store Connect API key needs all three of its \
		 variables, and {} isn't set: {}",
		missing.join(" and "),
		app_store::HOW
	);
	let [issuer, id, key] = values.map(Option::unwrap_or_default);

	if !key.contains("-----BEGIN") {
		outside_repository(
			root,
			Path::new(key.trim()),
			app_store::KEY,
			&format!("set {} to the key's contents instead", app_store::KEY),
		)?;
	}

	let key = app_store::Key::new(Secret::new(issuer), Secret::new(id), &Secret::new(key))?;
	// A key given as text is written to a private file Xcode reads, and
	// removed when the build is done.
	key.file(&std::env::temp_dir()).map(Some)
}

/// `xcodebuild archive` for a signed release, into
/// `.leptoskit/release/ios/App.xcarchive`.
#[must_use]
pub(crate) fn ios_archive(root: &Path, signing: &IosSigning, build_number: Option<u64>) -> Tool {
	let (flag, container) = app_store::container(root);
	let mut tool = Tool::new("xcodebuild")
		.dir(root)
		// Xcode reads these as its linker drivers; a nix or devenv shell sets
		// `LD=ld`, which rejects the flags Xcode passes it.
		.unset("LD")
		.unset("LDPLUSPLUS")
		.args(["-quiet", "archive", flag, container, "-scheme", "App"])
		.args(["-configuration", "Release"])
		.args(["-destination", "generic/platform=iOS"])
		.arg("-archivePath")
		.path(&ios_output(root).join("App.xcarchive"))
		.args(["-derivedDataPath", ".leptoskit/ios/build"])
		.args(signing.authentication())
		.args(signing.build_settings());

	if let Some(build_number) = build_number {
		tool = tool.arg(format!("CURRENT_PROJECT_VERSION={build_number}"));
	}

	tool
}

/// `xcodebuild -exportArchive`, which signs the archive for its export
/// method and writes the `.ipa` to `.leptoskit/release/ios/`.
#[must_use]
pub(crate) fn ios_export(root: &Path, signing: &IosSigning) -> Tool {
	let out = ios_output(root);
	Tool::new("xcodebuild")
		.dir(root)
		.unset("LD")
		.unset("LDPLUSPLUS")
		.args(["-quiet", "-exportArchive"])
		.arg("-archivePath")
		.path(&out.join("App.xcarchive"))
		.arg("-exportPath")
		.path(&out)
		.arg("-exportOptionsPlist")
		.path(&out.join("ExportOptions.plist"))
		.args(signing.authentication())
}

/// Where a signed iOS release is archived and exported.
pub(crate) fn ios_output(root: &Path) -> PathBuf {
	root.join(OUTPUT).join("ios")
}

/// The `ExportOptions.plist` that exports the archive for its method,
/// signed as it was archived, keeping the build number it was given.
///
/// # Errors
///
/// When the property list can't be written.
pub(crate) fn export_options(signing: &IosSigning) -> Result<String> {
	use plist::Value;

	let mut options = plist::Dictionary::new();
	options.insert("destination".into(), Value::from("export"));
	options.insert("method".into(), Value::from(signing.method.xcode_method()));
	options.insert("teamID".into(), Value::from(signing.team.as_str()));

	match &signing.style {
		IosStyle::Automatic { .. } => {
			options.insert("signingStyle".into(), Value::from("automatic"));
		}
		IosStyle::Manual { identity, profile } => {
			options.insert("signingStyle".into(), Value::from("manual"));
			options.insert("signingCertificate".into(), Value::from(identity.as_str()));
			let mut profiles = plist::Dictionary::new();
			profiles.insert(signing.bundle_id.clone(), Value::from(profile.as_str()));
			options.insert("provisioningProfiles".into(), Value::Dictionary(profiles));
		}
	}

	if signing.method == ExportMethod::AppStoreConnect {
		options.insert("manageAppVersionAndBuildNumber".into(), Value::from(false));
		options.insert("uploadSymbols".into(), Value::from(true));
	}

	let mut xml = Vec::new();
	Value::Dictionary(options)
		.to_writer_xml(&mut xml)
		.context("could not write ExportOptions.plist")?;
	String::from_utf8(xml).context("ExportOptions.plist isn't UTF-8")
}

/// Clears the archive and `.ipa` of an earlier release, so what is left
/// after this one is its own.
pub(crate) fn clear_ios_output(root: &Path) -> Result<()> {
	let out = ios_output(root);

	if out.exists() {
		std::fs::remove_dir_all(&out)
			.with_context(|| format!("could not clear {}", out.display()))?;
	}

	std::fs::create_dir_all(&out)?;
	Ok(())
}

/// The `.ipa` the export wrote, renamed for the app and its version.
///
/// # Errors
///
/// When the export wrote none.
pub(crate) fn exported_ipa(root: &Path, config: &LeptoskitConfig) -> Result<PathBuf> {
	let out = ios_output(root);
	let ipa = std::fs::read_dir(&out)
		.with_context(|| format!("could not read {}", out.display()))?
		.filter_map(Result::ok)
		.map(|entry| entry.path())
		.find(|path| path.extension().is_some_and(|extension| extension == "ipa"))
		.with_context(|| format!("Xcode exported no .ipa to {}", out.display()))?;
	let named = out.join(format!("{}-{}.ipa", config.app.name, config.app.version));

	if ipa != named {
		std::fs::rename(&ipa, &named)
			.with_context(|| format!("could not rename {}", ipa.display()))?;
	}

	Ok(named)
}

#[cfg(test)]
mod tests;
