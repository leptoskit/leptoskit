mod ios;
pub(super) mod platform_sources;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
pub(super) use ios::apple_plugins;
#[cfg(test)]
pub(super) use ios::linked_frameworks as ios_linked_frameworks;

use super::Target;
use super::registry::Constructor;
use super::registry::Flavour;
use super::registry::Implementation;
use super::registry::Registry;
use super::registry::RustRegistration;

/// Validate destination support before writing any generated source.
pub fn generate(
	root: &Path,
	config: &leptoskit_config::LeptoskitConfig,
	registry: &Registry,
	target: Target,
	release: bool,
) -> Result<()> {
	for warning in registry.warnings() {
		eprintln!("{warning}");
	}

	if matches!(target, Target::Windows | Target::Linux | Target::Tui) {
		return super::rust_desktop::generate(root, config, registry, target);
	}

	registry.implementations(target)?;

	if matches!(target, Target::Ios | Target::IosSimulator) {
		validate_ios_usage_descriptions(root, registry)?;
	}

	if release
		&& target.is_android()
		&& let Some(warning) = android_keep_rules_warning(root)
	{
		crate::ui::warning(warning);
	}

	let generated = super::generated_dir(root, target);

	if let Ok(metadata) = std::fs::symlink_metadata(&generated) {
		anyhow::ensure!(
			metadata.is_dir() && !metadata.file_type().is_symlink(),
			".leptoskit must be an ordinary generated directory"
		);
	}

	std::fs::create_dir_all(&generated)?;
	let rust = rust_registrant(registry, target)?;
	write(&generated.join("registrant.rs"), &rust)?;

	match target {
		Target::Macos => super::desktop::generate(root, config, registry),
		Target::Windows | Target::Linux | Target::Tui => {
			unreachable!("desktop registrant generated above")
		}
		Target::Ios | Target::IosSimulator => ios::generate(root, config, registry),
		Target::Android => android(root, config, registry, "arm64-v8a"),
		Target::AndroidX86_64 => android(root, config, registry, "x86_64"),
		Target::Quest => {
			android_host(
				root,
				config,
				registry,
				"arm64-v8a",
				Some(&config.native.quest),
			)
		}
		Target::Harmonyos | Target::HarmonyosX86_64 => {
			super::harmonyos::generate(root, config, registry, target)
		}
		Target::Wearos | Target::Watchos => {
			anyhow::bail!("watch hosts are scaffolded before project::generate")
		}
	}
}

fn validate_ios_usage_descriptions(root: &Path, registry: &Registry) -> Result<()> {
	let required: std::collections::BTreeSet<_> = registry
		.plugins
		.iter()
		.flat_map(|plugin| &plugin.apple_usage_descriptions)
		.collect();

	if required.is_empty() {
		return Ok(());
	}

	let path = super::host_dir(root, Target::Ios).join("Info.plist");
	let plist = plist::Value::from_file(&path).with_context(|| {
		format!(
			"read app-owned iOS usage descriptions from {}",
			path.display()
		)
	})?;
	let dictionary = plist
		.as_dictionary()
		.context("iOS Info.plist must contain a property-list dictionary")?;

	for key in required {
		anyhow::ensure!(
			dictionary
				.get(key)
				.and_then(plist::Value::as_string)
				.is_some_and(|text| !text.trim().is_empty() && !text.contains('\0')),
			"selected iOS plugins require a nonempty app-owned string for `{key}` in {}; describe \
			 your app's actual use before building",
			path.display()
		);
	}

	Ok(())
}

/// Apps scaffolded before the CLI generated its R8 rules list only their own
/// `proguard-rules.pro`, so release minification strips the members the JNI
/// library calls and the app crashes at launch. The Gradle script is
/// app-owned, so the build names the fix rather than editing it. A missing
/// script has nothing to warn about: Gradle reports it.
fn android_keep_rules_warning(root: &Path) -> Option<String> {
	let path = super::host_dir(root, Target::Android).join("app/build.gradle.kts");
	let script = std::fs::read_to_string(&path).ok()?;

	(!script.contains(".leptoskit/android/proguard-rules.pro")).then(|| {
		format!(
			"warning: {} does not pass .leptoskit/android/proguard-rules.pro to proguardFiles; R8 \
			 will strip the members the JNI library calls and the release app will crash at \
			 launch. Add the generated file to the release proguardFiles list.",
			path.display()
		)
	})
}

/// Apps scaffolded before `app.icon` don't add the generated resources to
/// their Gradle script, so the launcher icons wouldn't reach the app.
fn android_res_warning(root: &Path) -> Option<String> {
	let path = super::host_dir(root, Target::Android).join("app/build.gradle.kts");
	let script = std::fs::read_to_string(&path).ok()?;
	(!script.contains(".leptoskit/android/res")).then(|| {
		format!(
			"warning: {} doesn't add .leptoskit/android/res to the main source set's res \
			 directories, so app.icon's launcher icons and app.launch_screen won't reach the app. \
			 Add it next to the assets line, as `leptoskit new` writes it.",
			path.display()
		)
	})
}

/// The version the app's Gradle script reads: `versionName` is
/// `app.version`, and `versionCode` is 1 until `leptoskit publish play`
/// passes the upload's own.
fn android_version(config: &leptoskit_config::LeptoskitConfig) -> String {
	format!(
		"# Generated by leptoskit from app.version.\nversionName={}\nversionCode=1\n",
		config.app.version
	)
}

/// Copy only ordinary package files so generated builds cannot traverse
/// symlinks.
pub(super) fn copy_package(source: &Path, destination: &Path) -> Result<()> {
	ensure_directory(destination)?;

	for entry in std::fs::read_dir(destination)? {
		let entry = entry?;

		if entry.file_type()?.is_dir() {
			std::fs::remove_dir_all(entry.path())?;
		} else {
			std::fs::remove_file(entry.path())?;
		}
	}

	for entry in walkdir::WalkDir::new(source)
		.into_iter()
		.filter_entry(|entry| {
			!matches!(
				entry.file_name().to_str(),
				Some("target" | ".build" | ".git" | ".swiftpm" | ".leptoskit" | "node_modules")
			)
		}) {
		let entry = entry?;
		anyhow::ensure!(
			!entry.file_type().is_symlink(),
			"native package source cannot be a symlink: {}",
			entry.path().display()
		);

		if entry.file_type().is_file() {
			copy(
				entry.path(),
				&destination.join(entry.path().strip_prefix(source)?),
			)?;
		}
	}
	Ok(())
}

fn android(
	root: &Path,
	config: &leptoskit_config::LeptoskitConfig,
	registry: &Registry,
	abi: &str,
) -> Result<()> {
	android_host(root, config, registry, abi, None)
}

/// The generated half of the Android host, for a phone or, with `quest`, for
/// a Meta Quest headset.
fn android_host(
	root: &Path,
	config: &leptoskit_config::LeptoskitConfig,
	registry: &Registry,
	abi: &str,
	quest: Option<&leptoskit_config::QuestConfig>,
) -> Result<()> {
	let dir = root.join(".leptoskit/android");
	write(&dir.join("version.properties"), &android_version(config))?;
	write(
		&dir.join("plugin-dependencies.txt"),
		&android_dependencies(registry),
	)?;
	write(&dir.join("abi.txt"), abi)?;
	copy(
		&registry.bridge.join("android/proguard-rules.pro"),
		&dir.join("proguard-rules.pro"),
	)?;
	let sources = dir.join("sources");
	prepare_android_sources(registry, &sources)?;
	let mut kotlin = String::from(
		"// Generated by leptoskit.\npackage dev.leptoskit.mobile\n\nimport \
		 android.content.Context\nimport dev.leptoskit.bridge.NativeDispatcher\n\nobject \
		 GeneratedPlugins {\n    fun register(\n        context: Context,\n        bridge: \
		 Long,\n        dispatcher: NativeDispatcher,\n    ) {\n",
	);

	android_registrant(registry, &sources, &mut kotlin)?;
	let manifest = std::fs::read_to_string(
		super::host_dir(root, Target::Android).join("app/src/main/AndroidManifest.xml"),
	)?;
	let mut permissions = std::collections::BTreeSet::new();
	let mut query_actions = std::collections::BTreeSet::new();
	let mut query_intents = std::collections::BTreeSet::new();

	for plugin in &registry.plugins {
		permissions.extend(plugin.android_permissions.iter().map(String::as_str));
		query_actions.extend(plugin.android_query_actions.iter().map(String::as_str));
		query_intents.extend(
			plugin
				.android_query_intents
				.iter()
				.map(|query| (query.action.as_str(), query.scheme.as_str())),
		);
	}

	let mut manifest =
		super::manifest::merge(&manifest, &permissions, &query_actions, &query_intents)?;
	manifest = super::manifest::services::app_components(
		&manifest,
		&android_services(registry),
		&android_receivers(registry),
	)?;
	// `app.icon` becomes the launcher icons, in resources only leptoskit
	// writes.
	let res = dir.join("res");

	if res.is_dir() {
		std::fs::remove_dir_all(&res)?;
	}

	let icon = crate::icons::write(root, config, crate::icons::Platform::Android, &res)?;

	if icon {
		manifest = super::manifest::with_icon(&manifest)?;
	}

	// `app.launch_screen` becomes a theme for the launcher activity, which
	// extends the application's own.
	let launch = crate::launch::Launch::load(root, config)?;

	if let Some(launch) = &launch
		&& launch.for_android()
	{
		let parent = super::manifest::application_theme(&manifest)?
			.unwrap_or_else(|| "@android:style/Theme.Material.Light.NoActionBar".to_owned());
		crate::launch::write_files(&res, launch.android_files(&parent)?)?;
		let (themed, set) =
			super::manifest::with_launch_theme(&manifest, crate::launch::ANDROID_THEME)?;

		if !set {
			crate::ui::warning(format!(
				"warning: the launcher activity in {} has a theme of its own, so 				 \
				 app.launch_screen isn't shown. Remove its android:theme, or extend @style/{} 				 \
				 from it.",
				super::host_dir(root, Target::Android)
					.join("app/src/main/AndroidManifest.xml")
					.display(),
				crate::launch::ANDROID_THEME
			));
		}

		manifest = themed;
	}

	let launch_assets = launch
		.as_ref()
		.is_some_and(crate::launch::Launch::for_android);
	if (icon || launch_assets)
		&& let Some(warning) = android_res_warning(root)
	{
		crate::ui::warning(warning);
	}

	manifest = android_file_providers(registry, &res, &manifest)?;

	// Quest packages the same host for the headset: its manifest says so.
	if let Some(quest) = quest {
		manifest = super::quest::manifest(&manifest, quest)?;
	}

	write(&dir.join("AndroidManifest.xml"), &manifest)?;
	write_android_cmake(root, registry, &[])
}

/// Writes the `CMake` project that builds the host's JNI library around the
/// Rust archive. `native` is what rustc reports for that archive with
/// `--print native-static-libs`: an archive carries no link directives, so the
/// plugins' system libraries (`aaudio`, `OpenSLES`, `mediandk`, …) have to be
/// named again here.
pub(super) fn write_android_cmake(
	root: &Path,
	registry: &Registry,
	native: &[String],
) -> Result<()> {
	let cmake = format!(
		"cmake_minimum_required(VERSION 3.22)\nproject(leptoskit_app LANGUAGES \
		 CXX)\nadd_library(runtime STATIC IMPORTED)\nset_target_properties(runtime PROPERTIES \
		 IMPORTED_LOCATION {})\nadd_library(leptoskit_bridge_jni SHARED {} \
		 {})\ntarget_compile_features(leptoskit_bridge_jni PRIVATE \
		 cxx_std_20)\ntarget_compile_options(leptoskit_bridge_jni PRIVATE -Wall -Wextra \
		 -Werror)\ntarget_compile_options(leptoskit_bridge_jni PRIVATE -ffunction-sections \
		 -fdata-sections)\ntarget_link_options(leptoskit_bridge_jni PRIVATE \
		 -Wl,--gc-sections)\ntarget_link_libraries(leptoskit_bridge_jni PRIVATE {})\n",
		quote(&super::generated_dir(root, Target::Android).join("lib/libleptoskit_bridge_ffi.a"))?,
		quote(&registry.bridge.join("android/bridge.cpp"))?,
		quote(&registry.bridge.join("mobile/app.cpp"))?,
		android_link_libraries(native)
	);
	write(&root.join(".leptoskit/android/CMakeLists.txt"), &cmake)
}

/// The C++ runtime, which the host project owns: `CMake` compiles the JNI
/// library's own C++ against the NDK's `c++_static`, so a dependency's
/// preference for `c++_shared` would only add an `libc++_shared.so` the APK
/// does not carry.
const ANDROID_CPP_RUNTIMES: [&str; 4] = ["c++", "c++_shared", "c++_static", "stdc++"];

/// The host's link line: the archive, the NDK libraries the JNI library already
/// names, then every library rustc reported, in its order, without repeats.
fn android_link_libraries(native: &[String]) -> String {
	let mut libraries = vec!["runtime", "log", "android", "dl", "m"];

	for library in native {
		let library = library.as_str();

		if libraries.contains(&library) || ANDROID_CPP_RUNTIMES.contains(&library) {
			continue;
		}

		libraries.push(library);
	}

	libraries.join(" ")
}

/// Copies each selected plugin's `FileProvider` paths XML into the generated
/// resources and declares the provider in the manifest.
fn android_file_providers(registry: &Registry, res: &Path, manifest: &str) -> Result<String> {
	let mut providers = Vec::new();

	for plugin in &registry.plugins {
		for provider in &plugin.android_file_providers {
			let source = plugin
				.directory
				.join("android/res/xml")
				.join(format!("{}.xml", provider.paths));
			let metadata = std::fs::symlink_metadata(&source)
				.with_context(|| format!("read file provider paths {}", source.display()))?;
			anyhow::ensure!(
				metadata.is_file() && !metadata.file_type().is_symlink(),
				"file provider paths must be a regular file: {}",
				source.display()
			);
			let xml = std::fs::read_to_string(&source)?;
			let document = roxmltree::Document::parse(&xml)
				.with_context(|| format!("invalid file provider paths {}", source.display()))?;
			anyhow::ensure!(
				document.root_element().has_tag_name("paths"),
				"file provider paths root must be <paths>: {}",
				source.display()
			);
			write(
				&res.join("xml").join(format!("{}.xml", provider.paths)),
				&xml,
			)?;
			providers.push(super::manifest::FileProvider {
				class: &provider.class,
				authority_suffix: &provider.authority_suffix,
				paths: &provider.paths,
			});
		}
	}

	super::manifest::with_file_providers(manifest, &providers)
}

fn android_dependencies(registry: &Registry) -> String {
	let dependencies: std::collections::BTreeSet<_> = registry
		.plugins
		.iter()
		.flat_map(|plugin| &plugin.android_dependencies)
		.collect();
	let mut output = String::new();

	for dependency in dependencies {
		output.push_str(dependency);
		output.push('\n');
	}

	output
}

fn android_services(registry: &Registry) -> Vec<super::manifest::services::ServiceComponent> {
	registry
		.plugins
		.iter()
		.flat_map(|plugin| &plugin.android_services)
		.map(|service| {
			super::manifest::services::ServiceComponent {
				name: service.name.clone(),
				foreground_service_type: service.foreground_service_type.clone(),
				exported: service.exported,
			}
		})
		.collect()
}

fn android_receivers(registry: &Registry) -> Vec<super::manifest::services::ReceiverComponent> {
	registry
		.plugins
		.iter()
		.flat_map(|plugin| &plugin.android_receivers)
		.map(|receiver| {
			super::manifest::services::ReceiverComponent {
				name: receiver.name.clone(),
				exported: receiver.exported,
				actions: receiver.actions.clone(),
			}
		})
		.collect()
}

fn copy_kotlin(package: &Path, destination: &Path, prefix: &str) -> Result<()> {
	let sources = package.join("android");
	anyhow::ensure!(
		!sources.is_symlink() && sources.is_dir(),
		"Android source directory must be ordinary: {}",
		sources.display()
	);

	for entry in std::fs::read_dir(sources)? {
		let path = entry?.path();

		if path.extension().is_some_and(|extension| extension == "kt") {
			anyhow::ensure!(
				path.is_file() && !path.is_symlink(),
				"Android source must be a regular file: {}",
				path.display()
			);
			let filename = path
				.file_name()
				.context("native source filename missing")?
				.to_str()
				.context("native source filename must be UTF-8")?;
			copy(&path, &destination.join(format!("{prefix}_{filename}")))?;
		}
	}

	Ok(())
}

fn quote(path: &Path) -> Result<String> {
	Ok(serde_json::to_string(
		path.to_str().context("native build path is not UTF-8")?,
	)?)
}

pub(super) fn write(path: &Path, content: &str) -> Result<()> {
	check_output(path)?;

	if std::fs::read(path).is_ok_and(|existing| existing == content.as_bytes()) {
		return Ok(());
	}

	std::fs::write(path, content).with_context(|| format!("write {}", path.display()))
}

pub fn copy(from: &Path, to: &Path) -> Result<()> {
	check_output(to)?;
	std::fs::copy(from, to)
		.with_context(|| format!("copy {} to {}", from.display(), to.display()))?;
	Ok(())
}

fn check_output(path: &Path) -> Result<()> {
	ensure_directory(path.parent().context("generated file has no parent")?)?;

	if let Ok(metadata) = std::fs::symlink_metadata(path) {
		anyhow::ensure!(
			metadata.is_file() && !metadata.file_type().is_symlink(),
			"generated output must be a regular file: {}",
			path.display()
		);
	}

	Ok(())
}

fn ensure_directory(path: &Path) -> Result<()> {
	for parent in path.ancestors() {
		if let Ok(metadata) = std::fs::symlink_metadata(parent) {
			anyhow::ensure!(
				metadata.is_dir() && !metadata.file_type().is_symlink(),
				"generated directory must not traverse a symlink: {}",
				parent.display()
			);
		}
	}

	std::fs::create_dir_all(path)?;
	Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod quality_tests;

/// Service registration stays inert while the foreign constructor owns
/// temporary buffers.
pub(super) fn rust_registrant(registry: &Registry, target: Target) -> Result<String> {
	let mut rust = String::from(
		"// Generated by leptoskit. Edit native.plugins and Cargo dependencies instead.\n",
	);

	let implementations = registry.implementations(target)?;
	rust.push_str(SERVICE_CONSTRUCTOR);
	rust.push_str(
		if implementations.iter().any(|(_, _, implementation)| {
			implementation.rust().is_some() || implementation.service().is_some()
		}) {
			"app"
		} else {
			"_app"
		},
	);
	rust.push_str(", _context| {\n");

	for (_, plugin, implementation) in &implementations {
		register_rust_plugin(plugin, implementation, &Installer::MOBILE, &mut rust)?;
	}

	rust.push_str("                Ok(())\n            },\n        )\n    }\n}\n");
	Ok(rust)
}

/// How a registrant spells the calls that install one plugin.
pub(super) struct Installer {
	/// The receiver and leading arguments of every install call.
	call: &'static str,
	/// The `&AppContext` that context factories take.
	context: &'static str,
	/// The same context, as a place whose directories can be borrowed.
	directories: &'static str,
	indent: &'static str,
	/// Whether the host runs plugin services.
	services: bool,
}

impl Installer {
	/// Inside a desktop runner's generated `plugins` function.
	pub(super) const DESKTOP: Self = Self {
		call: "plugins.{install}(host, ",
		context: "&context",
		directories: "context",
		indent: "    ",
		services: false,
	};
	/// Inside the mobile and macOS registrant's `create_with_context`
	/// closure.
	const MOBILE: Self = Self {
		call: "app.{install}(",
		context: "_context",
		directories: "_context",
		indent: "                ",
		services: true,
	};

	fn install(&self, install: &str) -> String {
		format!("{}{}", self.indent, self.call.replace("{install}", install))
	}
}

/// Keep synchronous plugin ownership separate from deferred asynchronous
/// service ownership.
pub(super) fn register_rust_plugin(
	plugin: &super::registry::Plugin,
	implementation: &Implementation<'_>,
	installer: &Installer,
	rust: &mut String,
) -> Result<()> {
	let Installer {
		context,
		directories,
		..
	} = installer;

	if let Some(factory) = implementation.service() {
		anyhow::ensure!(
			installer.services,
			"{} declares a plugin service, which this host can't run",
			plugin.name
		);
		writeln!(
			rust,
			"                app.defer_service({}::{factory}(\n                    \
			 &_context.data_dir,\n                    &_context.cache_dir,\n                )?)?;",
			plugin.crate_name
		)?;

		return Ok(());
	}

	let Some(RustRegistration {
		constructor,
		flavour,
	}) = implementation.rust()
	else {
		return Ok(());
	};

	let install = match flavour {
		Flavour::Worker => "install_worker",
		Flavour::MainThread => "install_main",
		Flavour::Sync | Flavour::Async => "install",
	};

	let crate_name = &plugin.crate_name;

	match constructor {
		Constructor::Type(kind) => {
			writeln!(
				rust,
				"{}Box::new({crate_name}::{kind}::default()))?;",
				installer.install(install)
			)?;
		}

		Constructor::Factory(factory) if flavour == Flavour::Async => {
			let options = plugin
				.options_json
				.as_ref()
				.context("validated async plugin options missing")?;
			writeln!(
				rust,
				"{}std::sync::Arc::new({crate_name}::{factory}({options:?})?))?;",
				installer.install("install_async")
			)?;
		}

		Constructor::ContextFactory(factory) if flavour == Flavour::Async => {
			let options = plugin
				.options_json
				.as_ref()
				.context("validated async plugin options missing")?;
			writeln!(
				rust,
				"{}std::sync::Arc::new({crate_name}::{factory}({context}, {options:?})?))?;",
				installer.install("install_async")
			)?;
		}

		Constructor::Factory(factory) => {
			writeln!(
				rust,
				"{}Box::new({crate_name}::{factory}(&{directories}.data_dir, \
				 &{directories}.cache_dir)?))?;",
				installer.install(install)
			)?;
		}

		Constructor::ContextFactory(factory) => {
			writeln!(
				rust,
				"{}Box::new({crate_name}::{factory}({context})?))?;",
				installer.install(install)
			)?;
		}
	}

	Ok(())
}

/// Native handlers retain order independently of Rust service installation.
fn android_registrant(registry: &Registry, sources: &Path, kotlin: &mut String) -> Result<()> {
	for (index, plugin, implementation) in registry.implementations(Target::Android)? {
		let Some(Implementation::Android { ty }) = implementation.native() else {
			continue;
		};

		writeln!(
			kotlin,
			"        dispatcher.register(bridge, {ty}({}))",
			if plugin.android_context {
				"context"
			} else {
				""
			}
		)?;
		copy_kotlin(&plugin.directory, sources, &format!("plugin_{index}"))?;
	}

	for (index, helper) in registry.support.iter().enumerate() {
		if helper.android {
			copy_kotlin(&helper.directory, sources, &format!("support_{index}"))?;
		}
	}

	kotlin.push_str("    }\n}\n");
	write(&sources.join("GeneratedPlugins.kt"), kotlin)?;
	Ok(())
}

fn prepare_android_sources(registry: &Registry, sources: &Path) -> Result<()> {
	ensure_directory(sources)?;
	// Replaced as one generated source tree; removed dependencies cannot leave
	// an old registrant or handler compiled into the next application.
	for entry in std::fs::read_dir(sources)? {
		let path = entry?.path();
		anyhow::ensure!(
			path.is_file() && !path.is_symlink(),
			"unexpected generated source entry {}",
			path.display()
		);

		std::fs::remove_file(path)?;
	}

	std::fs::copy(
		registry.bridge.join("android/NativePlugin.kt"),
		sources.join("NativePlugin.kt"),
	)?;
	std::fs::copy(
		registry.bridge.join("mobile/LeptosKitView.kt"),
		sources.join("LeptosKitView.kt"),
	)?;

	for source in platform_sources::ANDROID {
		copy(
			&registry.bridge.join(source),
			&sources.join(
				Path::new(source)
					.file_name()
					.context("native source filename missing")?,
			),
		)?;
	}

	Ok(())
}

/// The service constructor has an explicit safety contract and readable
/// generated parameters.
const SERVICE_CONSTRUCTOR: &str = r#"/// Creates an App with its inert service installer.
///
/// # Safety
/// Inputs and output must satisfy
/// `leptoskit_bridge_ffi::app::create_with_context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn leptoskit_app_create_with_context(
    wasm: *const u8,
    len: usize,
    context: *const leptoskit_bridge_ffi::app::NativeAppContext,
    error: *mut leptoskit_bridge_ffi::Buffer,
) -> *mut leptoskit_bridge_ffi::app::App {
    // SAFETY: The native app owns these input/output buffers for this call.
    unsafe {
        leptoskit_bridge_ffi::app::create_with_context(
            wasm,
            len,
            context,
            error,
            |"#;
