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
	// A push-enabled Mac app generates only with its provisioning profile.
	config.native.macos.provisioning_profile = Some("push.provisionprofile".into());
	write(&root.join("push.provisionprofile"), "profile").unwrap();
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

/// Apple's push entitlement is a restricted one: macOS kills an app that
/// claims it at launch unless a matching provisioning profile is embedded in
/// the bundle. So `aps-environment` without a configured profile stops
/// before anything is generated, naming the setting, instead of building an
/// app that cannot start.
#[test]
fn a_push_enabled_mac_app_generates_only_with_its_provisioning_profile() {
	let temporary = tempfile::tempdir().unwrap();
	let root = std::fs::canonicalize(temporary.path()).unwrap();
	let registry = clipboard_registry(&root);
	let mut config = LeptoskitConfig::default();
	config.native.macos.aps_environment = Some(leptoskit_config::ApsEnvironment::Development);

	let error = generate(&root, &config, &registry)
		.expect_err("push without a provisioning profile must not generate")
		.to_string();
	assert!(error.contains("provisioning-profile"), "{error}");
	assert!(error.contains("embedded.provisionprofile"), "{error}");
	assert!(
		!root.join(".leptoskit").exists(),
		"the refusal precedes every generated write"
	);

	// A configured profile that isn't there is named by its path.
	config.native.macos.provisioning_profile = Some("signing/push.provisionprofile".into());
	let error = generate(&root, &config, &registry).unwrap_err().to_string();
	assert!(error.contains("signing/push.provisionprofile"), "{error}");
	assert!(!root.join(".leptoskit").exists());

	write(&root.join("signing/push.provisionprofile"), "profile").unwrap();
	generate(&root, &config, &registry).unwrap();
	assert!(entitlements_path(&root).is_some());

	// The profile is a push setting: without `aps-environment` it is
	// neither required nor read.
	config.native.macos.aps_environment = None;
	config.native.macos.provisioning_profile = Some("missing.provisionprofile".into());
	generate(&root, &config, &registry).unwrap();
}

/// A push-enabled Mac app is never ad-hoc signed with its entitlement: the
/// local build asks for a real identity and embeds the profile, an app
/// without push signs exactly as before, and `--unsigned` builds the push
/// app ad-hoc without the entitlement.
#[test]
fn a_push_enabled_mac_app_is_never_ad_hoc_signed_with_its_entitlement() {
	let temporary = tempfile::tempdir().unwrap();
	let root = std::fs::canonicalize(temporary.path()).unwrap();
	let nothing = |_: &str| None;
	let identity = |name: &str| {
		(name == IDENTITY).then(|| "Apple Development: Example (ABCDE12345)".to_owned())
	};
	let arguments = |signing: &LocalSigning| {
		signing
			.codesign_arguments()
			.iter()
			.map(|argument| argument.to_string_lossy().into_owned())
			.collect::<Vec<_>>()
			.join(" ")
	};

	// No push: ad-hoc, no entitlements, whatever the environment holds.
	let plain = LeptoskitConfig::default();
	for env in [&nothing as crate::native::signing::Env<'_>, &identity] {
		let signing = local_signing(&root, &plain, env, false).unwrap();
		assert_eq!(signing, LocalSigning::AdHoc);
		assert_eq!(arguments(&signing), "--force --sign - --timestamp=none");
		assert_eq!(signing.profile(), None);
	}

	let mut config = LeptoskitConfig::default();
	config.native.macos.aps_environment = Some(leptoskit_config::ApsEnvironment::Development);
	config.native.macos.provisioning_profile = Some("push.provisionprofile".into());
	write(&root.join("push.provisionprofile"), "profile").unwrap();
	write(&root.join(".leptoskit/macos/App.entitlements"), "<plist/>").unwrap();

	// Push without an identity: refused, with the variable and the reason.
	let error = local_signing(&root, &config, &nothing, false)
		.unwrap_err()
		.to_string();
	assert!(error.contains(IDENTITY), "{error}");
	assert!(error.contains("ad-hoc"), "{error}");
	assert!(error.contains("--unsigned"), "{error}");

	// Push with an identity: that identity, the entitlements, the profile.
	let signing = local_signing(&root, &config, &identity, false).unwrap();
	assert_eq!(
		arguments(&signing).replace(&root.display().to_string(), "<root>"),
		"--force --sign Apple Development: Example (ABCDE12345) --timestamp=none --entitlements \
		 <root>/.leptoskit/macos/App.entitlements"
	);
	assert_eq!(
		signing.profile(),
		Some(root.join("push.provisionprofile").as_path())
	);

	// `--unsigned`: ad-hoc and without the entitlement, identity or not.
	for env in [&nothing as crate::native::signing::Env<'_>, &identity] {
		assert_eq!(
			local_signing(&root, &config, env, true).unwrap(),
			LocalSigning::AdHoc
		);
	}

	// A missing profile is refused before the identity is even read.
	std::fs::remove_file(root.join("push.provisionprofile")).unwrap();
	let error = local_signing(&root, &config, &identity, true)
		.unwrap_err()
		.to_string();
	assert!(error.contains("push.provisionprofile"), "{error}");
}

/// The profile is copied into the bundle where macOS looks for it, and a
/// bundle that no longer needs one loses the copy an earlier build left.
#[test]
fn the_provisioning_profile_is_embedded_and_a_stale_one_removed() {
	let temporary = tempfile::tempdir().unwrap();
	let root = std::fs::canonicalize(temporary.path()).unwrap();
	let bundle = root.join("App.app");
	let embedded = bundle.join("Contents/embedded.provisionprofile");
	write(&root.join("push.provisionprofile"), "profile").unwrap();

	embed_profile(&bundle, None).unwrap();
	assert!(!embedded.exists());

	embed_profile(&bundle, Some(&root.join("push.provisionprofile"))).unwrap();
	assert_eq!(std::fs::read_to_string(&embedded).unwrap(), "profile");

	embed_profile(&bundle, None).unwrap();
	assert!(!embedded.exists(), "a stale profile is removed");
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
	// the modules the conditional import names. Targets, not products:
	// `--product` cannot name an automatic library, and SwiftPM then builds
	// the package's default targets instead.
	let plugin_build = root.join("plugin-build");
	swift(&[
		"build".into(),
		package_flag(&packages.join("leptoskit_plugin_notifications")),
		"--target=LeptosKitNotifications".into(),
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
		"--target=LeptosKitBridge".into(),
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
	// The compiler's diagnostics go to stdout under `swift build`; stderr
	// carries only SwiftPM's own warnings.
	assert!(
		output.status.success(),
		"swift {} failed:\nstdout:\n{}\nstderr:\n{}",
		arguments.join(" "),
		String::from_utf8_lossy(&output.stdout),
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
