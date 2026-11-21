//! The Meta Horizon Store, for Meta Quest headsets: the release-signed Quest
//! APK, uploaded through Meta's `ovr-platform-util upload-quest-build`.
//!
//! Without `--submit`, uploads carry `--draft` so nothing reaches testers or
//! Production users. With `--submit`, uploads go live on the configured channel
//! (`alpha`, `beta`, `rc`, or `store`); first-time Store review, the Data Use
//! Checkup, store listing assets, and age rating remain Developer Dashboard
//! steps printed at the end of every run.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::publish::QuestAgeGroup;
use leptoskit_config::publish::QuestStore;

use super::secrets::Credentials;
use super::secrets::Secret;
use super::tools::Tool;

/// The app's secret from the Meta Horizon Developer Dashboard (API tab).
pub const APP_SECRET: &str = "LEPTOSKIT_PUBLISH_QUEST_APP_SECRET";
/// Alternative credential: an Oculus platform utility user token.
pub const TOKEN: &str = "LEPTOSKIT_PUBLISH_QUEST_TOKEN";
/// Where to obtain the Meta Horizon Store upload credential.
pub const AUTH_HOW: &str =
	"open the Meta Horizon Developer Dashboard (https://developers.meta.com/horizon/manage/), \
	 select your app, open Development -> API, and copy the App Secret — or set \
	 LEPTOSKIT_PUBLISH_QUEST_TOKEN to an Oculus platform utility user token";
/// Optional environment override for the `ovr-platform-util` executable path.
pub const TOOL_ENV: &str = "LEPTOSKIT_OVR_PLATFORM_UTIL";
/// Default executable name looked up on `PATH`.
pub const TOOL_DEFAULT: &str = "ovr-platform-util";
/// The upload keystore variables shared with `leptoskit build --release
/// --platform quest`.
pub const KEYSTORE: &str = crate::native::signing::ANDROID_KEYSTORE;
pub const KEYSTORE_PASSWORD: &str = crate::native::signing::ANDROID_KEYSTORE_PASSWORD;
pub const KEY_ALIAS: &str = crate::native::signing::ANDROID_KEY_ALIAS;
pub const KEY_PASSWORD: &str = crate::native::signing::ANDROID_KEY_PASSWORD;
pub const KEYSTORE_HOW: &str = super::play::KEYSTORE_HOW;
/// Quest headsets run `arm64-v8a` binaries.
pub const REQUIRED_ABI: &str = "arm64-v8a";

/// Authentication passed to `ovr-platform-util`.
#[derive(Debug, Clone)]
pub enum Auth {
	/// App Secret (`-s`).
	AppSecret(Secret),
	/// Oculus platform utility user token (`-t`).
	UserToken(Secret),
}

impl Auth {
	/// Resolves `LEPTOSKIT_PUBLISH_QUEST_APP_SECRET` or
	/// `LEPTOSKIT_PUBLISH_QUEST_TOKEN`.
	///
	/// # Errors
	///
	/// When neither credential is set.
	pub fn resolve(credentials: &Credentials) -> Result<Self> {
		if let Some(secret) = credentials.get(APP_SECRET) {
			return Ok(Self::AppSecret(secret));
		}
		if let Some(token) = credentials.get(TOKEN) {
			return Ok(Self::UserToken(token));
		}
		credentials
			.require(APP_SECRET, AUTH_HOW)
			.map(Self::AppSecret)
	}

	fn secret(&self) -> &Secret {
		match self {
			Self::AppSecret(secret) | Self::UserToken(secret) => secret,
		}
	}
}

/// What Gradle signs the Quest APK with.
pub struct Signing {
	pub keystore: Secret,
	pub keystore_password: Secret,
	pub key_alias: Secret,
	pub key_password: Secret,
}

/// Parameters for one `ovr-platform-util upload-quest-build` invocation.
pub struct Upload<'a> {
	pub tool: &'a str,
	pub app_id: &'a str,
	pub auth: &'a Auth,
	pub apk: &'a Path,
	pub channel: &'a str,
	pub draft: bool,
	pub age_group: Option<QuestAgeGroup>,
	pub notes: Option<&'a str>,
	pub obb: Option<&'a Path>,
	pub assets_dir: Option<&'a Path>,
	pub debug_symbols_dir: Option<&'a Path>,
}

/// Resolves the `ovr-platform-util` program path from [`TOOL_ENV`] or `PATH`.
#[must_use]
pub fn tool_program(override_from: Option<String>) -> String {
	override_from
		.filter(|value| !value.trim().is_empty())
		.unwrap_or_else(|| TOOL_DEFAULT.to_owned())
}

/// `gradlew assembleRelease` in the Android host for the Quest build.
#[must_use]
pub fn apk(android: &Path, signing: &Signing, build_number: Option<u64>) -> Tool {
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
	]);

	if let Some(build_number) = build_number {
		tool = tool.arg(format!("-Pleptoskit.versionCode={build_number}"));
	}

	tool.env(KEYSTORE, signing.keystore.clone())
		.env(KEYSTORE_PASSWORD, signing.keystore_password.clone())
		.env(KEY_ALIAS, signing.key_alias.clone())
		.env(KEY_PASSWORD, signing.key_password.clone())
}

/// Builds the `ovr-platform-util upload-quest-build` command.
#[must_use]
pub fn upload(params: &Upload<'_>) -> Tool {
	let mut tool = Tool::new(params.tool).args(["upload-quest-build", "-a", params.app_id]);

	tool = match params.auth {
		Auth::AppSecret(secret) => tool.args(["-s", secret.expose()]),
		Auth::UserToken(token) => tool.args(["-t", token.expose()]),
	};

	tool = tool
		.arg("--apk")
		.path(params.apk)
		.args(["-c", params.channel]);

	if let Some(age_group) = params.age_group {
		tool = tool.args(["--age-group", age_group.as_cli_value()]);
	}

	if params.draft {
		tool = tool.arg("--draft");
	}

	if let Some(notes) = params.notes {
		tool = tool.args(["-n", notes]);
	}

	if let Some(obb) = params.obb {
		tool = tool.arg("--obb").path(obb);
	}

	if let Some(assets_dir) = params.assets_dir {
		tool = tool.arg("--assets-dir").path(assets_dir);
	}

	if let Some(debug_symbols_dir) = params.debug_symbols_dir {
		tool = tool.arg("--debug-symbols-dir").path(debug_symbols_dir);
	}

	tool
}

/// Scrubs the secret or user token from any tool error before it reaches the
/// terminal.
#[must_use]
pub fn redact_error(error: anyhow::Error, auth: &Auth) -> anyhow::Error {
	let needle = auth.secret().expose();
	let message = format!("{error:#}");
	if needle.is_empty() || !message.contains(needle) {
		return error;
	}
	anyhow::anyhow!("{}", message.replace(needle, "[redacted]"))
}

/// Checks that the Android Gradle script accepts `leptoskit.versionCode` and
/// signs release APKs with the upload key.
#[must_use]
pub fn gradle_problems(script: &str, path: &Path) -> Vec<String> {
	let mut problems = Vec::new();
	if !script.contains("leptoskit.versionCode") {
		problems.push(format!(
			"{} doesn't take the build number leptoskit chooses: set versionCode from the \
			 `leptoskit.versionCode` property, as `leptoskit new` writes it",
			path.display()
		));
	}
	if !script.contains(KEYSTORE) {
		problems.push(format!(
			"{} doesn't sign the release with the upload key: add the signingConfigs block that \
			 reads {KEYSTORE}, as `leptoskit new` writes it",
			path.display()
		));
	}
	problems
}

/// Refuses an x86-only prepared Android host: Meta Quest headsets run
/// `arm64-v8a`.
pub fn check_abis(root: &Path) -> Result<()> {
	let path = root.join(".leptoskit/android/abi.txt");
	let abi = match std::fs::read_to_string(&path) {
		Ok(abi) => Some(abi.trim().to_owned()),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
		Err(error) => {
			return Err(error).with_context(|| format!("could not read {}", path.display()));
		}
	};

	anyhow::ensure!(
		abi.as_deref().is_none_or(|abi| abi == REQUIRED_ABI),
		"{} names the Android build's only ABI `{}`; the Meta Horizon Store takes a Quest APK \
		 that supports {REQUIRED_ABI}. Prepare the Quest build (`leptoskit native prepare \
		 quest`), not an x86_64 emulator build",
		path.display(),
		abi.as_deref().unwrap_or_default()
	);
	Ok(())
}

/// Verifies optional OBB, assets, and debug-symbols paths exist before
/// compiling.
pub fn check_optional_paths(root: &Path, settings: &QuestStore) -> Result<()> {
	if let Some(obb) = &settings.obb {
		let path = root.join(obb);
		anyhow::ensure!(
			path.is_file(),
			"publish.quest.obb `{obb}` does not exist at {}",
			path.display()
		);
	}
	if let Some(assets_dir) = &settings.assets_dir {
		let path = root.join(assets_dir);
		anyhow::ensure!(
			path.is_dir(),
			"publish.quest.assets_dir `{assets_dir}` is not a directory at {}",
			path.display()
		);
	}
	if let Some(debug_symbols_dir) = &settings.debug_symbols_dir {
		let path = root.join(debug_symbols_dir);
		anyhow::ensure!(
			path.is_dir(),
			"publish.quest.debug_symbols_dir `{debug_symbols_dir}` is not a directory at {}",
			path.display()
		);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn upload_builds_expected_ovr_platform_util_arguments() {
		let auth = Auth::AppSecret(Secret::new("super-secret".into()));
		let tool = upload(&Upload {
			tool: "ovr-platform-util",
			app_id: "5159709737372459",
			auth: &auth,
			apk: Path::new("/out/app-release.apk"),
			channel: "alpha:quest3only",
			draft: true,
			age_group: Some(QuestAgeGroup::TeensAndAdults),
			notes: Some("First Quest release"),
			obb: Some(Path::new("/proj/main.obb")),
			assets_dir: Some(Path::new("/proj/dlc")),
			debug_symbols_dir: Some(Path::new("/proj/symbols")),
		});
		assert_eq!(tool.program, "ovr-platform-util");
		assert_eq!(
			tool.args,
			[
				"upload-quest-build",
				"-a",
				"5159709737372459",
				"-s",
				"super-secret",
				"--apk",
				"/out/app-release.apk",
				"-c",
				"alpha:quest3only",
				"--age-group",
				"TEENS_AND_ADULTS",
				"--draft",
				"-n",
				"First Quest release",
				"--obb",
				"/proj/main.obb",
				"--assets-dir",
				"/proj/dlc",
				"--debug-symbols-dir",
				"/proj/symbols",
			]
		);
	}

	#[test]
	fn upload_supports_user_token_and_redacts_errors() {
		let auth = Auth::UserToken(Secret::new("OC|1234|secret-token".into()));
		let tool = upload(&Upload {
			tool: "/custom/ovr-platform-util",
			app_id: "999",
			auth: &auth,
			apk: Path::new("app.apk"),
			channel: "store",
			draft: false,
			age_group: None,
			notes: None,
			obb: None,
			assets_dir: None,
			debug_symbols_dir: None,
		});
		assert_eq!(tool.program, "/custom/ovr-platform-util");
		assert!(
			tool.args
				.windows(2)
				.any(|pair| pair == ["-t", "OC|1234|secret-token"])
		);
		assert!(!tool.args.iter().any(|arg| arg == "--draft"));

		let scrubbed = redact_error(
			anyhow::anyhow!("failed with token OC|1234|secret-token in stderr"),
			&auth,
		);
		let text = scrubbed.to_string();
		assert!(!text.contains("secret-token"), "{text}");
		assert!(text.contains("[redacted]"), "{text}");
	}
}
