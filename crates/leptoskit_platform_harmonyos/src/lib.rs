//! <!-- {=harmonyosPlatformSummary|trim|linePrefix:"//! ":true} -->
//!
//! HarmonyOS platform extension: ArkUI host templates and the capability
//! manifest the CLI and docs use for the experimental destination.
//!
//! <!-- {/harmonyosPlatformSummary} -->

#![forbid(unsafe_code)]

use leptoskit_platform::CapabilityMatrix;
use leptoskit_platform::CodepushSupport;
use leptoskit_platform::Manifest;
use leptoskit_platform::PROTOCOL;
use leptoskit_platform::PlatformExtension;
use leptoskit_platform::PluginHostCapabilities;
use leptoskit_platform::RenderStrategy;
use leptoskit_platform::ScaffoldContext;
use leptoskit_platform::ScaffoldFile;
use leptoskit_platform::WidgetCapability;

/// Embedded host templates formerly shipped inside `leptoskit_cli`.
pub mod templates {
	pub const APP_ICON: &str = include_str!("../templates/app_icon.svg");
	pub const BUILD_PROFILE: &str = include_str!("../templates/build-profile.json5");
	pub const CMAKE_LISTS: &str = include_str!("../templates/CMakeLists.txt");
	pub const COUNTER_TEST: &str = include_str!("../templates/Counter.test.ets");
	pub const ENTRY_ABILITY: &str = include_str!("../templates/EntryAbility.ets");
	pub const ENTRY_BUILD_PROFILE: &str = include_str!("../templates/entry-build-profile.json5");
	pub const GUEST_RS: &str = include_str!("../templates/guest.rs");
	pub const INDEX_D_TS: &str = include_str!("../templates/index.d.ts");
	pub const INDEX_ETS: &str = include_str!("../templates/Index.ets");
	pub const MODULE_JSON5: &str = include_str!("../templates/module.json5");
	pub const OH_PACKAGE: &str = include_str!("../templates/oh-package.json5");
	pub const OPEN_HARMONY_TEST_RUNNER: &str =
		include_str!("../templates/OpenHarmonyTestRunner.ets");
	pub const PLUGINS_TS: &str = include_str!("../templates/plugins.ts");
	pub const README: &str = include_str!("../templates/readme.md");
	pub const SCENE_TS: &str = include_str!("../templates/scene.ts");
	pub const TEST_MODULE: &str = include_str!("../templates/test-module.json5");
	pub const TESTING_MD: &str = include_str!("../templates/testing.md");
}

/// The HarmonyOS / ArkUI platform extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct HarmonyOs;

impl HarmonyOs {
	/// Destination id used in `app.platforms` and host directories.
	pub const ID: &'static str = "harmonyos";

	/// Entry `build-profile.json5` with `__ABI__` substituted.
	#[must_use]
	pub fn entry_build_profile(abi: &str) -> String {
		templates::ENTRY_BUILD_PROFILE.replace("__ABI__", abi)
	}

	/// `TESTING.md` with the bundle name filled in.
	#[must_use]
	pub fn testing_markdown(package_id: &str) -> String {
		templates::TESTING_MD.replace("__BUNDLE_NAME__", package_id)
	}

	/// Instrumentation test with the bundle name filled in.
	#[must_use]
	pub fn counter_test(package_id: &str) -> String {
		templates::COUNTER_TEST.replace("__BUNDLE_NAME__", package_id)
	}
}

impl PlatformExtension for HarmonyOs {
	fn id(&self) -> &'static str {
		Self::ID
	}

	fn display_name(&self) -> &'static str {
		"HarmonyOS"
	}

	fn manifest(&self) -> Manifest {
		Manifest {
			protocol: PROTOCOL,
			id: Self::ID.into(),
			display_name: self.display_name().into(),
			version: env!("CARGO_PKG_VERSION").into(),
			experimental: true,
			render: RenderStrategy::ForeignHeadless,
			capabilities: CapabilityMatrix {
				widgets: [
					"view",
					"text",
					"button",
					"text_input",
					"password_input",
					"scroll_view",
					"image",
					"rive",
				]
				.into_iter()
				.map(|kind| {
					WidgetCapability {
						kind: kind.into(),
						support: "full".into(),
						note: None,
					}
				})
				.chain([WidgetCapability {
					kind: "vector".into(),
					support: "unsupported".into(),
					note: Some("ArkUI scene has no Vector path yet".into()),
				}])
				.collect(),
				plugins: PluginHostCapabilities {
					sync: true,
					worker: false,
					asynchronous: false,
					service: false,
					native: Some("arkts".into()),
				},
				codepush: CodepushSupport::Unsupported,
				text_measurer: "none".into(),
				packaging: vec!["hap".into()],
			},
			docs_url: Some("https://leptoskit.dev/docs/platforms/harmonyos.html".into()),
		}
	}

	fn scaffold_files(&self, context: ScaffoldContext<'_>) -> Vec<ScaffoldFile> {
		let package = context.package_id;
		let app = serde_json::json!({"app": {
			"bundleName": package, "vendor": "leptoskit", "versionCode": 1,
			"versionName": context.version, "icon": "$media:app_icon", "label": "$string:app_name"
		}});
		let resources =
			serde_json::json!({"string": [{"name":"app_name", "value": context.app_name}]});
		let app = serde_json::to_string_pretty(&app).expect("app.json5 is valid JSON");
		let resources =
			serde_json::to_string_pretty(&resources).expect("string.json is valid JSON");

		vec![
			file("AppScope/app.json5", app),
			file("AppScope/resources/base/element/string.json", resources),
			file(
				"AppScope/resources/base/media/app_icon.svg",
				templates::APP_ICON,
			),
			file("build-profile.json5", templates::BUILD_PROFILE),
			file("oh-package.json5", templates::OH_PACKAGE),
			file("TESTING.md", Self::testing_markdown(package)),
			file("entry/src/ohosTest/module.json5", templates::TEST_MODULE),
			file(
				"entry/src/ohosTest/ets/testrunner/OpenHarmonyTestRunner.ets",
				templates::OPEN_HARMONY_TEST_RUNNER,
			),
			file(
				"entry/src/ohosTest/ets/test/Counter.test.ets",
				Self::counter_test(package),
			),
			file(
				"hvigorfile.ts",
				"import { appTasks } from '@ohos/hvigor-ohos-plugin';\nexport default { system: \
				 appTasks, plugins: [] };\n",
			),
			file(
				"hvigor/hvigor-config.json5",
				"{\"modelVersion\": \"5.0.0\", \"dependencies\": {}, \"execution\": {}, \
				 \"logging\": {}, \"debugging\": {}, \"nodeOptions\": {}}\n",
			),
			file(
				"entry/hvigorfile.ts",
				"import { hapTasks } from '@ohos/hvigor-ohos-plugin';\nexport default { system: \
				 hapTasks, plugins: [] };\n",
			),
			file(
				"entry/oh-package.json5",
				"{\"name\": \"entry\", \"version\": \"1.0.0\", \"dependencies\": \
				 {\"libleptoskit.so\": \"file:./src/main/cpp/types/libleptoskit\"}}\n",
			),
			file(
				"entry/build-profile.json5",
				Self::entry_build_profile("arm64-v8a"),
			),
			file("entry/src/main/module.json5", templates::MODULE_JSON5),
			file(
				"entry/src/main/resources/base/profile/main_pages.json",
				"{\"src\": [\"pages/Index\"]}\n",
			),
			file(
				"entry/src/main/resources/base/element/color.json",
				"{\"color\": [{\"name\": \"start_window_background\", \"value\": \"#FFFFFF\"}]}\n",
			),
			file(
				"entry/src/main/ets/entryability/EntryAbility.ets",
				templates::ENTRY_ABILITY,
			),
			file("entry/src/main/ets/pages/Index.ets", templates::INDEX_ETS),
			file("entry/src/main/ets/scene.ts", templates::SCENE_TS),
			file("entry/src/main/cpp/CMakeLists.txt", templates::CMAKE_LISTS),
			file(
				"entry/src/main/cpp/types/libleptoskit/index.d.ts",
				templates::INDEX_D_TS,
			),
			file(
				"entry/src/main/cpp/types/libleptoskit/oh-package.json5",
				"{\"name\":\"libleptoskit.so\",\"version\":\"1.0.0\",\"types\":\"./index.d.ts\"}\n",
			),
			file(
				".gitignore",
				"/.hvigor\n/oh_modules\n/entry/oh_modules\n/entry/build\n/entry/.test\n/entry/.\
				 cxx\n/entry/build-profile.json5\n/entry/src/main/ets/leptoskit\n/local.\
				 properties\n/entry/src/main/resources/rawfile/app.wasm\n",
			),
		]
	}
}

fn file(relative_path: impl Into<String>, contents: impl Into<String>) -> ScaffoldFile {
	ScaffoldFile {
		relative_path: relative_path.into(),
		contents: contents.into(),
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_platform::PlatformExtension;

	use super::*;

	#[test]
	fn manifest_is_valid() {
		HarmonyOs.manifest().validate().unwrap();
	}

	#[test]
	fn scaffold_includes_the_arkui_scene() {
		let files = HarmonyOs.scaffold_files(ScaffoldContext {
			app_name: "Demo",
			package_id: "dev.leptoskit.demo",
			version: "0.1.0",
		});
		let scene = files
			.iter()
			.find(|file| file.relative_path == "entry/src/main/ets/scene.ts")
			.expect("scene.ts");
		assert!(scene.contents.contains("applyTransaction") || !scene.contents.is_empty());
		assert!(files.iter().any(|file| {
			file.relative_path == "TESTING.md" && file.contents.contains("dev.leptoskit.demo")
		}));
	}

	#[test]
	fn entry_profile_substitutes_abi() {
		assert!(HarmonyOs::entry_build_profile("x86_64").contains("x86_64"));
	}
}
