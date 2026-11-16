use std::path::PathBuf;

use super::*;
use crate::native::registry::Plugin;

#[test]
fn assembly_preserves_app_owned_usage_descriptions() {
	let root = std::env::temp_dir()
		.canonicalize()
		.unwrap()
		.join(format!("leptoskit-macos-assembly-{}", std::process::id()));
	let plist = "<plist><dict><key>NSMicrophoneUsageDescription</key><string>Record a voice \
	             note</string></dict></plist>";
	write(&root.join("hosts/macos/Info.plist"), plist).unwrap();
	write(&root.join("binary"), "executable").unwrap();
	write(&root.join(".leptoskit/assets/app.wasm"), "guest").unwrap();
	let bundle = assemble(&root, &LeptoskitConfig::default(), &root.join("binary")).unwrap();
	assert_eq!(
		std::fs::read_to_string(bundle.join("Contents/Info.plist")).unwrap(),
		plist
	);
	assert_eq!(
		std::fs::read_to_string(bundle.join("Contents/Resources/app.wasm")).unwrap(),
		"guest"
	);

	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn registers_only_selected_native_plugins_and_preserves_user_host() {
	let temporary = std::env::temp_dir()
		.canonicalize()
		.unwrap()
		.join(format!("leptoskit-macos-generation-{}", std::process::id()));
	let root = temporary.as_path();

	if root.exists() {
		std::fs::remove_dir_all(root).unwrap();
	}

	let mut registry = clipboard_registry(root);
	generate(root, &LeptoskitConfig::default(), &registry).unwrap();
	let generated = root.join(".leptoskit/macos");
	assert_eq!(
		std::fs::read_to_string(generated.join("Sources/App.swift")).unwrap(),
		"// customized window"
	);
	assert!(
		std::fs::read_to_string(generated.join("Sources/GeneratedPlugins.swift"))
			.unwrap()
			.contains("dispatcher.register(ClipboardPlugin(), on: bridge)")
	);
	assert!(
		std::fs::read_to_string(generated.join("Package.swift"))
			.unwrap()
			.contains(".package(path: \"packages/leptoskit_plugin_clipboard\")")
	);
	assert!(
		generated
			.join("packages/leptoskit_bridge_ffi/Package.swift")
			.is_file()
	);
	assert_eq!(
		std::fs::read_to_string(root.join("hosts/macos/App.swift")).unwrap(),
		"// customized window"
	);

	registry.plugins[0].apple_context = true;
	generate(root, &LeptoskitConfig::default(), &registry).unwrap();
	let context_registrant =
		std::fs::read_to_string(generated.join("Sources/GeneratedPlugins.swift")).unwrap();
	assert!(context_registrant.contains("_ context: NativePluginContext"));
	assert!(context_registrant.contains("ClipboardPlugin(context: context)"));
	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn assembly_falls_back_to_the_template_plist_for_older_apps() {
	let root = std::env::temp_dir()
		.canonicalize()
		.unwrap()
		.join(format!("leptoskit-macos-plist-{}", std::process::id()));
	write(&root.join("binary"), "executable").unwrap();
	write(&root.join(".leptoskit/assets/app.wasm"), "guest").unwrap();
	let mut config = LeptoskitConfig::default();
	config.native.package_id = "dev.example.legacy".into();
	let bundle = assemble(&root, &config, &root.join("binary")).unwrap();
	let info = std::fs::read_to_string(bundle.join("Contents/Info.plist")).unwrap();
	assert!(info.contains("dev.example.legacy"));

	assert!(!info.contains("__PACKAGE__") && !info.contains("__NAME__"));
	assert!(!info.contains("__VERSION__"));
	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn support_packages_link_and_incomplete_apple_metadata_fails() {
	let temporary = std::env::temp_dir()
		.canonicalize()
		.unwrap()
		.join(format!("leptoskit-macos-support-{}", std::process::id()));
	let root = temporary.as_path();

	if root.exists() {
		std::fs::remove_dir_all(root).unwrap();
	}

	let mut registry = support_registry(root);
	generate(root, &LeptoskitConfig::default(), &registry).unwrap();
	let package = std::fs::read_to_string(root.join(".leptoskit/macos/Package.swift")).unwrap();
	assert!(package.contains(".package(path: \"packages/leptoskit_sensor\")"));
	assert!(package.contains(".product(name: \"LeptosKitSensor\", package: \"leptoskit_sensor\")"));
	assert!(
		root.join(".leptoskit/macos/packages/leptoskit_sensor/Package.swift")
			.is_file()
	);

	// A native plugin without its Apple type cannot be registered.
	registry.plugins.push(
		serde_json::from_value(serde_json::json!({
			"name": "broken", "apple-module": "Broken"
		}))
		.unwrap(),
	);
	assert!(generate(root, &LeptoskitConfig::default(), &registry).is_err());
	std::fs::remove_dir_all(root).unwrap();
}

fn clipboard_registry(root: &Path) -> Registry {
	let bridge = root.join("sdk/leptoskit_bridge_ffi");
	let clipboard = root.join("sdk/leptoskit_plugin_clipboard");

	for source in crate::native::project::platform_sources::MACOS {
		write(&bridge.join(source), "// bridge source").unwrap();
	}

	write(&bridge.join("Package.swift"), "// bridge").unwrap();
	write(
		&bridge.join("desktop/LeptosKitMacViewController.swift"),
		"// renderer",
	)
	.unwrap();

	write(
		&bridge.join("mobile/RenderTransaction.swift"),
		"// transactions",
	)
	.unwrap();
	write(
		&bridge.join("mobile/NativeRustWorker.swift"),
		"// native worker",
	)
	.unwrap();
	write(&clipboard.join("Package.swift"), "// clipboard").unwrap();
	write(&root.join("hosts/macos/App.swift"), "// customized window").unwrap();
	let mut plugin: Plugin = serde_json::from_value(serde_json::json!({
		"name": "clipboard",
		"apple-module": "LeptosKitClipboard",
		"apple-type": "ClipboardPlugin"
	}))
	.unwrap();
	plugin.directory = clipboard;
	plugin.package_name = "leptoskit_plugin_clipboard".into();
	plugin.crate_name = "clipboard".into();

	Registry {
		package: "demo".into(),
		plugins: vec![plugin],
		support: vec![],
		bridge,
		target: root.join("target"),
		host_features: vec![],
		devtools_feature: None,
		missing: Vec::new(),
		scene3d: false,
	}
}

fn support_registry(root: &Path) -> Registry {
	let bridge = root.join("sdk/leptoskit_bridge_ffi");

	for source in crate::native::project::platform_sources::MACOS {
		write(&bridge.join(source), "// bridge source").unwrap();
	}
	for source in [
		"Package.swift",
		"desktop/LeptosKitMacViewController.swift",
		"mobile/RenderTransaction.swift",
		"mobile/NativeRustWorker.swift",
	] {
		write(&bridge.join(source), "// bridge").unwrap();
	}
	let sensor = root.join("sdk/leptoskit_sensor");
	write(&sensor.join("Package.swift"), "// shared sensor").unwrap();

	write(&root.join("hosts/macos/App.swift"), "// app").unwrap();

	Registry {
		package: "demo".into(),
		plugins: vec![],
		support: vec![crate::native::registry::NativeSupport {
			apple_module: Some("LeptosKitSensor".into()),
			android: false,
			directory: sensor,
			package_name: "leptoskit_sensor".into(),
		}],
		bridge,
		target: root.join("target"),
		host_features: vec![],
		devtools_feature: None,
		missing: Vec::new(),
		scene3d: false,
	}
}

#[test]
fn menu_bar_mode_replaces_only_the_stock_host() {
	let temporary = tempfile::tempdir().unwrap();
	// TMPDIR is /var/folders on macOS and /var is a symlink to
	// /private/var; the generator refuses paths that traverse symlinks, so
	// resolve the temp directory before generating into it.
	let root = std::fs::canonicalize(temporary.path()).unwrap();
	let registry = clipboard_registry(&root);
	write(
		&root.join("hosts/macos/App.swift"),
		include_str!("../templates/MacApp.swift"),
	)
	.unwrap();
	let mut config = LeptoskitConfig::default();
	config.native.mode = leptoskit_config::application::NativeMode::MenuBar;
	generate(&root, &config, &registry).unwrap();
	let generated = root.join(".leptoskit/macos/Sources/App.swift");
	assert_eq!(
		std::fs::read_to_string(&generated).unwrap(),
		include_str!("../templates/MacMenuBarApp.swift")
	);
	write(
		&root.join("hosts/macos/App.swift"),
		"// custom resident lifecycle",
	)
	.unwrap();
	generate(&root, &config, &registry).unwrap();
	assert_eq!(
		std::fs::read_to_string(generated).unwrap(),
		"// custom resident lifecycle"
	);
}

/// `[native.macos] aps-environment` is the push opt-in: it alone writes
/// `.leptoskit/macos/App.entitlements` with Apple's macOS key and exact
/// string, and dropping it removes the file again while every other
/// generated file stays byte-identical, so apps that never opted in
/// generate — and sign — exactly as before.
#[test]
fn the_push_entitlement_comes_and_goes_with_the_setting() {
	let temporary = tempfile::tempdir().unwrap();
	let root = std::fs::canonicalize(temporary.path()).unwrap();
	let registry = clipboard_registry(&root);
	let generated = |root: &Path| {
		[
			"Package.swift",
			"Sources/App.swift",
			"Sources/GeneratedPlugins.swift",
		]
		.into_iter()
		.map(|file| {
			(
				file,
				std::fs::read_to_string(root.join(".leptoskit/macos").join(file)).unwrap(),
			)
		})
		.collect::<Vec<_>>()
	};

	generate(&root, &LeptoskitConfig::default(), &registry).unwrap();
	let without = generated(&root);
	assert!(!root.join(".leptoskit/macos/App.entitlements").exists());

	// A stale entitlements file from an opted-in generation is removed, not
	// left to leak push into a local-only app.
	write(&root.join(".leptoskit/macos/App.entitlements"), "<stale/>").unwrap();
	let mut config = LeptoskitConfig::default();
	config.native.macos.aps_environment = Some(leptoskit_config::ApsEnvironment::Development);
	generate(&root, &config, &registry).unwrap();
	let with = generated(&root);
	assert_eq!(with, without, "the opt-in changes nothing else");

	let entitlements =
		plist::Value::from_file(root.join(".leptoskit/macos/App.entitlements")).unwrap();
	let entitlements = entitlements.as_dictionary().unwrap();
	assert_eq!(
		entitlements.get("com.apple.developer.aps-environment"),
		Some(&plist::Value::String("development".into()))
	);
	assert_eq!(entitlements.len(), 1, "nothing else is granted");
	assert!(entitlements_path(&root).is_some());

	config.native.macos.aps_environment = Some(leptoskit_config::ApsEnvironment::Production);
	generate(&root, &config, &registry).unwrap();
	let entitlements =
		plist::Value::from_file(root.join(".leptoskit/macos/App.entitlements")).unwrap();
	assert_eq!(
		entitlements
			.as_dictionary()
			.unwrap()
			.get("com.apple.developer.aps-environment"),
		Some(&plist::Value::String("production".into()))
	);

	config.native.macos.aps_environment = None;
	generate(&root, &config, &registry).unwrap();
	assert!(!root.join(".leptoskit/macos/App.entitlements").exists());
	assert_eq!(generated(&root), without);
	assert!(
		entitlements_path(&root).is_none(),
		"build signs nothing extra"
	);
}

// The generated macOS app's Swift compiles against the real modules: with
// the notifications plugin selected its package is built for this Mac and
// the stock host's conditional import resolves; without it the same host
// compiles with the plugin simply absent. Non-macOS machines skip: the
// generated project is still exercised textually by the tests everywhere
// else.
#[test]
fn generated_app_swift_typechecks_with_and_without_the_notifications_module() {
	let Some(sdk) = macos_sdk() else {
		eprintln!("skipped: no macOS SDK on this machine");
		return;
	};

	let temporary = tempfile::tempdir().unwrap();
	let root = std::fs::canonicalize(temporary.path()).unwrap();
	write(
		&root.join("hosts/macos/App.swift"),
		include_str!("../templates/MacApp.swift"),
	)
	.unwrap();

	// The real notifications plugin, with its real package directory.
	let mut plugin: Plugin = serde_json::from_value(serde_json::json!({
		"name": "notifications",
		"apple-module": "LeptosKitNotifications",
		"apple-type": "NotificationsPlugin",
	}))
	.unwrap();
	plugin.directory =
		Path::new(env!("CARGO_MANIFEST_DIR")).join("../leptoskit_plugin_notifications");
	plugin.package_name = "leptoskit_plugin_notifications".into();
	let registry = Registry {
		package: "app".into(),
		plugins: vec![plugin],
		support: Vec::new(),
		bridge: Path::new(env!("CARGO_MANIFEST_DIR")).join("../leptoskit_bridge_ffi"),
		target: root.join("target"),
		host_features: Vec::new(),
		devtools_feature: None,
		missing: Vec::new(),
		scene3d: false,
	};

	generate(&root, &LeptoskitConfig::default(), &registry).unwrap();
	let packages = root.join(".leptoskit/macos/packages");
	let bridge = registry.bridge.clone();
	let app = root.join(".leptoskit/macos/Sources");

	// Build the plugin package (and, through it, the bridge) for this Mac:
	// the modules the conditional import names.
	let plugin_build = root.join("plugin-build");
	swift(&[
		"build".into(),
		package_flag(&packages.join("leptoskit_plugin_notifications")),
		"--product=LeptosKitNotifications".into(),
		format!("--build-path={}", plugin_build.display()),
	]);
	let plugin_modules = modules(&plugin_build, "LeptosKitNotifications");
	assert!(
		plugin_modules
			.join("LeptosKitNotifications.swiftmodule")
			.exists(),
		"the plugin module was built for macOS"
	);

	typecheck(&sdk, &plugin_modules, &bridge, &sources(&app));

	// Without the plugin: the same host file, the module absent from the
	// search path, and a registrant that registers nothing.
	let without_plugin = Registry {
		plugins: Vec::new(),
		..registry
	};
	generate(&root, &LeptoskitConfig::default(), &without_plugin).unwrap();
	let bridge_build = root.join("bridge-build");
	swift(&[
		"build".into(),
		package_flag(&packages.join("leptoskit_bridge_ffi")),
		"--product=LeptosKitBridge".into(),
		format!("--build-path={}", bridge_build.display()),
	]);
	let bridge_modules = modules(&bridge_build, "LeptosKitBridge");
	assert!(
		!bridge_modules
			.join("LeptosKitNotifications.swiftmodule")
			.exists()
	);
	typecheck(&sdk, &bridge_modules, &bridge, &sources(&app));
}

/// The macOS SDK's path, when this machine has one.
fn macos_sdk() -> Option<PathBuf> {
	if !cfg!(target_os = "macos") {
		return None;
	}
	let output = Command::new("xcrun")
		.args(["--sdk", "macosx", "--show-sdk-path"])
		.output()
		.ok()?;
	if !output.status.success() {
		return None;
	}
	let path = String::from_utf8(output.stdout).ok()?.trim().to_owned();
	if path.is_empty() {
		return None;
	}
	Some(PathBuf::from(path))
}

fn package_flag(path: &Path) -> String {
	format!("--package-path={}", path.display())
}

fn swift(arguments: &[String]) {
	let output = Command::new("swift")
		.args(arguments)
		.output()
		.expect("swift runs");
	assert!(
		output.status.success(),
		"swift {} failed:\n{}",
		arguments.join(" "),
		String::from_utf8_lossy(&output.stderr)
	);
}

/// The directory a `swift build` exposes its built modules in — the products
/// directory `SwiftPM` itself passes as `-I` to consumers, falling back to
/// wherever the named module landed on an older layout.
fn modules(build: &Path, module: &str) -> PathBuf {
	let named = format!("{module}.swiftmodule");
	let mut fallback = None;
	for entry in walkdir::WalkDir::new(build)
		.into_iter()
		.filter_map(Result::ok)
		.filter(|entry| entry.file_name().to_string_lossy() == named)
	{
		let parent = entry.path().parent().unwrap().to_path_buf();
		if parent
			.ancestors()
			.any(|ancestor| ancestor.file_name().is_some_and(|name| name == "Products"))
		{
			return parent;
		}
		fallback.get_or_insert(parent);
	}

	fallback.unwrap_or_else(|| panic!("no {named} under {}", build.display()))
}

/// The generated executable target's own sources, exactly as `generate`
/// compiles them.
fn sources(directory: &Path) -> Vec<PathBuf> {
	let mut names: Vec<_> = std::iter::once("App.swift")
		.chain(std::iter::once("GeneratedPlugins.swift"))
		.chain(
			crate::native::project::platform_sources::MACOS
				.iter()
				.map(|path| Path::new(path).file_name().unwrap().to_str().unwrap()),
		)
		.map(str::to_owned)
		.collect();
	names.sort();
	names.iter().map(|name| directory.join(name)).collect()
}

fn typecheck(sdk: &Path, import: &Path, bridge: &Path, sources: &[PathBuf]) {
	let mut command = Command::new("swiftc");
	command
		.arg("-typecheck")
		.arg("-sdk")
		.arg(sdk)
		.arg("-I")
		.arg(import)
		// The bridge's C header module, which the generated registrant
		// imports beside the Swift one.
		.arg("-Xcc")
		.arg(format!(
			"-fmodule-map-file={}",
			bridge.join("include/module.modulemap").display()
		));
	command.args(sources.iter().map(|path| path as &Path));

	let output = command.output().expect("swiftc runs");
	assert!(
		output.status.success(),
		"typecheck failed:\n{}",
		String::from_utf8_lossy(&output.stderr)
	);
}
