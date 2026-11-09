//! Generated native hosts and dependency-driven plugin registration.

use std::io::IsTerminal as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use clap::Subcommand;
use clap::ValueEnum;
use leptoskit_codepush::capability::HostCapabilityDescriptor;
use leptoskit_codepush::capability::RUNTIME_CONTRACT_VERSION;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Optimize;
use leptoskit_config::Platform;
use sha2::Digest;
use sha2::Sha256;

pub(crate) mod config;
mod desktop;
mod guest_profile;
mod harmonyos;
mod lock;
mod manifest;
mod platforms;
mod project;
mod quest;

#[cfg(test)]
mod dev_client_host;
mod registry;
pub mod signing;
mod watch;

mod rust_desktop;
pub(crate) mod safari;

pub(crate) use rust_desktop::application;
pub mod scaffold;
mod size;

/// Native host tooling. Generated files live in `.leptoskit`; platform
/// directories contain app-owned integration sources.
#[derive(Debug, Clone, Subcommand)]
pub enum NativeCommand {
	/// Compiles the native Wasm guest and compatibility descriptor for code
	/// push.
	Bundle {
		#[arg(long, default_value = "dist")]
		out: String,
		/// Destination whose platform feature the bundle enables.
		#[arg(long, value_enum)]
		target: Option<Target>,
	},
	/// Regenerates host projects and registrants and compiles their Rust
	/// inputs.
	Prepare {
		#[arg(value_enum)]
		target: Target,
		#[arg(long)]
		release: bool,
	},
	/// Produces an Apple, Windows, or Linux application, Android APK, or
	/// HarmonyOS HAP.
	///
	/// A release for Android, Meta Quest, or an iOS device is signed for its
	/// store, with the signing material from the environment.
	Build {
		#[arg(value_enum)]
		target: Target,
		#[arg(long)]
		release: bool,
		#[command(flatten)]
		options: signing::ReleaseOptions,
	},
	/// Lists and scaffolds in-tree platform extensions.
	Platform {
		#[command(subcommand)]
		command: platforms::PlatformCommand,
	},
}

/// The native compilation target, including simulator architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Target {
	/// A native AppKit application for this Mac's architecture.
	Macos,
	IosSimulator,
	Ios,
	Android,
	/// Android x86-64 devices and hosted CI emulators.
	#[value(name = "android-x86_64")]
	AndroidX86_64,
	/// HarmonyOS arm64 devices and arm64 emulators (DevEco Studio).
	Harmonyos,
	/// HarmonyOS x86-64 emulators.
	#[value(name = "harmonyos-x86_64")]
	HarmonyosX86_64,
	/// Experimental Android Wear OS watch host.
	Wearos,
	/// Experimental Apple watchOS watch host.
	Watchos,
	/// Meta Quest headsets (Horizon OS): the Android host, packaged as a VR
	/// app.
	Quest,
	/// A native Windows application for this machine's architecture.
	Windows,
	/// A native GTK application for this machine's architecture.
	Linux,
	/// A terminal application for this machine's own terminal.
	Tui,
}

impl Target {
	fn rust(self) -> &'static str {
		match self {
			Self::Macos if cfg!(target_arch = "x86_64") => "x86_64-apple-darwin",
			Self::Macos => "aarch64-apple-darwin",

			Self::IosSimulator if cfg!(target_arch = "x86_64") => "x86_64-apple-ios",
			Self::IosSimulator => "aarch64-apple-ios-sim",
			Self::Ios => "aarch64-apple-ios",
			Self::Android | Self::Wearos | Self::Quest => "aarch64-linux-android",
			Self::AndroidX86_64 => "x86_64-linux-android",
			Self::Harmonyos => "aarch64-unknown-linux-ohos",
			Self::HarmonyosX86_64 => "x86_64-unknown-linux-ohos",
			// watchOS packaging is experimental; the stub guest still targets
			// the device arm64 watchOS triple for future hosts.
			Self::Watchos => "aarch64-apple-watchos",

			Self::Windows if cfg!(target_arch = "aarch64") => "aarch64-pc-windows-msvc",
			Self::Windows => "x86_64-pc-windows-msvc",

			Self::Linux if cfg!(target_arch = "aarch64") => "aarch64-unknown-linux-gnu",
			Self::Linux => "x86_64-unknown-linux-gnu",
			// The terminal runner compiles for whichever machine runs it.
			Self::Tui => host_triple(),
		}
	}

	/// The stable destination flavour a capability contract names.
	pub(crate) fn flavor(self) -> &'static str {
		match self {
			Self::Macos => "macos",
			Self::IosSimulator => "ios-simulator",
			Self::Ios => "ios",
			Self::Android => "android",
			Self::AndroidX86_64 => "android-x86_64",
			Self::Harmonyos => "harmonyos",
			Self::HarmonyosX86_64 => "harmonyos-x86_64",
			Self::Wearos => "wearos",
			Self::Watchos => "watchos",
			Self::Quest => "quest",
			Self::Windows => "windows",
			Self::Linux => "linux",
			Self::Tui => "tui",
		}
	}

	/// Whether the destination itself is experimental.
	pub(crate) fn experimental(self) -> bool {
		matches!(
			self,
			Self::Harmonyos | Self::HarmonyosX86_64 | Self::Wearos | Self::Watchos
		)
	}

	pub(crate) fn platform(self) -> Platform {
		match self {
			Self::Macos => Platform::Macos,
			Self::Ios | Self::IosSimulator => Platform::Ios,
			Self::Android | Self::AndroidX86_64 => Platform::Android,
			Self::Harmonyos | Self::HarmonyosX86_64 => Platform::Harmonyos,
			Self::Wearos => Platform::Wearos,
			Self::Watchos => Platform::Watchos,
			Self::Quest => Platform::Quest,
			Self::Windows => Platform::Windows,
			Self::Linux => Platform::Linux,
			Self::Tui => Platform::Tui,
		}
	}

	/// Whether the runner builds for the host without a `--target` triple.
	pub(super) fn builds_on_host(self) -> bool {
		matches!(self, Self::Tui)
	}

	/// Whether a debug app for this target reports to the `DevTools` hub
	/// under `leptoskit dev`: the hosts that run the shared app runtime.
	pub(crate) fn reports_to_devtools(self) -> bool {
		matches!(
			self,
			Self::Macos
				| Self::Ios
				| Self::IosSimulator
				| Self::Android
				| Self::AndroidX86_64
				| Self::Quest
		)
	}

	/// Whether this target builds the Android host: the phones, and Quest.
	pub(crate) fn is_android(self) -> bool {
		matches!(self, Self::Android | Self::AndroidX86_64 | Self::Quest)
	}

	/// The platform whose host directory this target builds. Quest packages
	/// the Android host rather than keeping a project of its own.
	fn host_platform(self) -> Platform {
		match self {
			Self::Quest => Platform::Android,
			other => other.platform(),
		}
	}

	pub(crate) fn from_platform(platform: Platform) -> Result<Self> {
		match platform {
			Platform::Ios => Ok(Self::Ios),
			Platform::Android => Ok(Self::Android),
			Platform::Macos => Ok(Self::Macos),
			Platform::Windows => Ok(Self::Windows),
			Platform::Linux => Ok(Self::Linux),
			Platform::Harmonyos => Ok(Self::Harmonyos),
			Platform::Wearos => Ok(Self::Wearos),
			Platform::Watchos => Ok(Self::Watchos),
			Platform::Quest => Ok(Self::Quest),
			Platform::Tui => Ok(Self::Tui),
			_ => anyhow::bail!("{platform} is not a concrete native platform"),
		}
	}
}

/// The triple of the machine running the build, for the host-built terminal
/// runner.
fn host_triple() -> &'static str {
	match (std::env::consts::OS, std::env::consts::ARCH) {
		("macos", "aarch64") => "aarch64-apple-darwin",
		("macos", _) => "x86_64-apple-darwin",
		("windows", "aarch64") => "aarch64-pc-windows-msvc",
		("windows", _) => "x86_64-pc-windows-msvc",
		("linux", "aarch64") => "aarch64-unknown-linux-gnu",
		_ => "x86_64-unknown-linux-gnu",
	}
}

/// App-owned platform integration, with support for existing native projects.
pub(crate) fn host_dir(root: &Path, target: Target) -> PathBuf {
	let platform = target.host_platform().to_string();
	let directory = root.join(&platform);

	if directory.join("Cargo.toml").is_file() {
		return directory;
	}

	root.join("hosts").join(platform)
}

/// Platform-local generated state prevents one destination replacing another.
pub(crate) fn generated_dir(root: &Path, target: Target) -> PathBuf {
	let generated = root.join(".leptoskit");

	if root
		.join(target.host_platform().to_string())
		.join("Cargo.toml")
		.is_file()
	{
		return generated.join(target.host_platform().to_string());
	}

	generated
}

/// Runs native generation and optionally the platform build tool.
pub fn run(command: NativeCommand) -> Result<()> {
	let (target, release, build, options) = match command {
		NativeCommand::Bundle { out, target } => return run_bundle(&out, target),
		NativeCommand::Platform { command } => return platforms::run(command),
		NativeCommand::Prepare { target, release } => {
			(target, release, false, signing::ReleaseOptions::default())
		}
		NativeCommand::Build {
			target,
			release,
			options,
		} => (target, release, true, options),
	};

	let (config, root) = LeptoskitConfig::load()?;
	config.ensure_application_root()?;

	if matches!(target, Target::Wearos | Target::Watchos) {
		ensure_target_enabled(&config, target)?;
		registry::enforce_required(&config, target, &[], &[])?;
		let contract = registry::descriptor(&config, target, &[], &[]);
		contract.validate()?;
		let native = if target == Target::Wearos {
			leptoskit_config::NativeTarget::Wearos
		} else {
			leptoskit_config::NativeTarget::Watchos
		};

		watch::scaffold(&root, &config, native)?;
		write_capabilities(
			&root
				.join(".leptoskit")
				.join(target.platform().to_string())
				.join("capabilities.json"),
			&contract,
		)?;

		for warning in registry::optional_warnings(&config, target, &[], &[]) {
			eprintln!("{warning}");
		}

		if build {
			return watch::build(target);
		}

		println!(
			"prepared experimental {} host stubs under hosts/{}",
			target.platform(),
			target.platform()
		);

		return Ok(());
	}

	let ohos_ndk = check_host(&config, target, release)?;
	// A store release's signing material is read before anything compiles,
	// so a missing variable fails in seconds, not after the build.
	let signing = if build && release {
		signing::resolve(&root, &config, target, &options, &signing::process_env)?
	} else {
		None
	};

	if let Some(signing::Signing::Android(_)) = &signing {
		signing::check_android_script(&host_dir(&root, target))?;
	}
	let _guard = lock::acquire(&root)?;
	let plugins = crate::build::resolve_plugins(&config)?;
	plugins.generate(&root, target.platform(), release)?;
	crate::l10n::sync(&root, &config)?;
	let registry = registry::resolve(&root, &config, target)?;

	let build_dir = native_build_directory(&root, &registry.target, target.rust())?;
	// The Apple apps carry the Safari extension, so its package comes first.
	if matches!(target, Target::Ios | Target::IosSimulator | Target::Macos) {
		safari::prepare(&config, release)?;
	}

	project::generate(&root, &config, &registry, target, release)?;
	let generated = generated_dir(&root, target);
	let bundle_dir = generated.join("bundle");
	let contract = bundle(
		&root,
		&config,
		&registry.plugins,
		&registry.missing,
		target,
		release,
		Some(&build_dir),
		&bundle_dir,
	)?;
	write_capabilities(&generated.join("capabilities.json"), &contract)?;

	if matches!(target, Target::Linux | Target::Windows | Target::Tui) {
		write_assets(&root, &config, target, &bundle_dir, release, &contract)?;

		// The terminal shows no pictures, so it ships none.
		if target != Target::Tui {
			stage_assets(&root, &assets_destination(&root, target))?;
		}

		rust_desktop::build(&root, &config, target, release, &registry.host_features)?;

		return Ok(());
	}

	let mut host = host_library_command(
		&root,
		&config,
		&registry,
		target,
		release,
		&build_dir,
		ohos_ndk.as_deref(),
	)?;
	if target.is_android() {
		// The archive is linked from CMake, which never sees Cargo's link
		// directives, so the libraries rustc reports for it are written into
		// the host's CMake project before Gradle links the JNI library.
		let native_libraries = run_host_library(&mut host)?;
		project::write_android_cmake(&root, &registry, &native_libraries)?;
	} else {
		run_tool(&mut host)?;
	}

	let profile = profile_directory(&config, true, release);
	project::copy(
		&build_dir
			.join(target.rust())
			.join(profile)
			.join("libleptoskit_app.a"),
		&generated.join("lib").join("libleptoskit_bridge_ffi.a"),
	)?;
	write_assets(&root, &config, target, &bundle_dir, release, &contract)?;
	stage_assets(&root, &assets_destination(&root, target))?;

	if build {
		match &signing {
			Some(signing) => build_signed(&root, &config, target, signing, &options)?,
			None => build_application(&root, &config, target, release, options.build_number)?,
		}

		if !matches!(signing, Some(signing::Signing::Ios(_))) {
			report_application_size(&root, target, release);
		}
	}

	Ok(())
}

/// Builds a release signed for its store, and checks that it is.
fn build_signed(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
	signing: &signing::Signing,
	options: &signing::ReleaseOptions,
) -> Result<()> {
	use crate::publish::tools::Runner as _;

	match signing {
		signing::Signing::Android(key) => {
			let android = host_dir(root, target);
			signing::clear_android_outputs(&android)?;
			crate::publish::tools::System.run(&signing::gradle_release(
				&android,
				key,
				options.build_number,
			))?;
			let (apk, bundle) = signing::signed_android_outputs(&android)?;
			let collected = signing::collect(root, config, target, &[&bundle, &apk])?;

			for artifact in collected {
				crate::ui::success(format_args!(
					"signed {}",
					artifact.strip_prefix(root).unwrap_or(&artifact).display()
				));
			}
		}
		signing::Signing::Ios(ios) => {
			install_ios_pods(root)?;
			signing::clear_ios_output(root)?;
			let out = signing::ios_output(root);
			std::fs::write(
				out.join("ExportOptions.plist"),
				signing::export_options(ios)?,
			)?;
			crate::publish::tools::System.run(&signing::ios_archive(
				root,
				ios,
				options.build_number,
			))?;
			crate::publish::tools::System.run(&signing::ios_export(root, ios))?;
			let ipa = signing::exported_ipa(root, config)?;
			crate::ui::success(format_args!(
				"signed {} for {}",
				ipa.strip_prefix(root).unwrap_or(&ipa).display(),
				ios.method
			));
		}
	}

	Ok(())
}

/// `native bundle`: compiles only the guest and its runtime descriptor into
/// `out`, for the destination `target` names or the project's default.
fn run_bundle(out: &str, target: Option<Target>) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	config.ensure_application_root()?;
	let target = target
		.map_or_else(
			|| {
				match config.app.default_platform {
					Platform::Native => Ok(legacy_target(&root, &config)),
					platform => Target::from_platform(platform),
				}
			},
			Ok,
		)
		.context("native bundle requires --target when app.default_platform is web or server")?;
	ensure_target_enabled(&config, target)?;
	// A guest with no runtime plugins needs no dependency resolution; the
	// host descriptor still covers the destination's built-in capabilities.
	let (plugins, missing) = if config.native.plugins.is_empty() {
		registry::enforce_required(&config, target, &[], &[])?;
		(Vec::new(), Vec::new())
	} else {
		let registry = registry::resolve(&root, &config, target)?;

		(registry.plugins, registry.missing)
	};

	let output = root.join(out);
	let contract = bundle(
		&root, &config, &plugins, &missing, target, true, None, &output,
	)?;
	write_capabilities(&output.with_extension("capabilities.json"), &contract)?;
	Ok(())
}

/// Saves the resolved host contract next to the guest, never inside the signed
/// guest bundle: the host descriptor is local build evidence, not a guest
/// claim.
fn write_capabilities(path: &Path, contract: &HostCapabilityDescriptor) -> Result<()> {
	contract.validate()?;

	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent)?;
	}

	std::fs::write(path, serde_json::to_vec_pretty(contract)?)?;
	Ok(())
}

/// Rejects a destination this machine or configuration cannot build, before
/// anything is generated. Returns the HarmonyOS NDK for an OHOS target.
fn check_host(config: &LeptoskitConfig, target: Target, release: bool) -> Result<Option<PathBuf>> {
	ensure_target_enabled(config, target)?;
	anyhow::ensure!(
		!matches!(target, Target::Macos | Target::Ios | Target::IosSimulator)
			|| cfg!(target_os = "macos"),
		"Apple applications require macOS and the Xcode command-line tools"
	);
	anyhow::ensure!(
		!matches!(target, Target::Windows) || cfg!(target_os = "windows"),
		"Windows applications require Windows and the MSVC build tools"
	);
	anyhow::ensure!(
		!matches!(target, Target::Linux) || cfg!(target_os = "linux"),
		"Linux applications require Linux and GTK 4 development libraries"
	);
	let ohos_ndk = if matches!(target, Target::Harmonyos | Target::HarmonyosX86_64) {
		harmonyos::validate(config)?;
		Some(harmonyos::ndk()?)
	} else {
		None
	};

	if config.codepush.enabled {
		let server = &config.codepush.server;
		anyhow::ensure!(
			server.starts_with("https://")
				|| (!release && config.codepush.allow_http && server.starts_with("http://")),
			"native code push requires HTTPS; debug builds may opt into codepush.allow_http"
		);
	}

	Ok(ohos_ndk)
}

/// The cargo invocation that compiles the host static library a mobile or
/// macOS shell links, with the target's C toolchain configured for the
/// host's C dependencies.
///
/// The Android hosts link that archive from `CMake`, outside Cargo, so the
/// build asks rustc for the native libraries it needs: `cargo rustc` is `cargo
/// build` plus a flag for the last crate, and the note it prints is the only
/// place those libraries are named.
fn host_library_command(
	root: &Path,
	config: &LeptoskitConfig,
	registry: &registry::Registry,
	target: Target,
	release: bool,
	build_dir: &Path,
	ohos_ndk: Option<&Path>,
) -> Result<Command> {
	let mut host = Command::new("cargo");
	host.current_dir(root).args([
		if target.is_android() {
			"rustc"
		} else {
			"build"
		},
		"--package",
		&registry.package,
		"--lib",
		"--target",
		target.rust(),
	]);
	host.arg("--target-dir").arg(build_dir);

	if release {
		// Only release builds pick up `bundle.optimize`'s profile.
		let (_, host_arguments) = release_arguments(config, true);
		host.args(&host_arguments);
	} else {
		host.args(DEBUG_HOST_ARGUMENTS);
	}

	if let Some(ndk) = ohos_ndk {
		harmonyos::configure_cargo(&mut host, ndk, target);
	}

	let features = host_features(
		registry,
		target,
		release,
		crate::build::devtools::hub().is_some(),
	);

	if !features.is_empty() {
		host.args(["--features", &features.join(",")]);
	}

	set_log_level(&mut host, config);

	if target.is_android() {
		configure_android_toolchain(&mut host, root, target, &android_ndk()?)?;

		// `run_host_library` reads the build's stderr, so Cargo no longer sees
		// a terminal; keep its colour when the build prints to one.
		if std::io::stderr().is_terminal() {
			host.args(["--color", "always"]);
		}

		host.args(["--", "--print", "native-static-libs"]);
	} else if matches!(target, Target::Ios | Target::IosSimulator | Target::Macos) {
		for (key, value) in apple_cc_environment(target) {
			host.env(key, value);
		}
	}

	Ok(host)
}

/// The features a host build turns on: the plugins', and the `DevTools`
/// agent's in a debug build that `leptoskit dev` runs with a hub. Release
/// builds never get the agent (plan D4).
fn host_features(
	registry: &registry::Registry,
	target: Target,
	release: bool,
	devtools: bool,
) -> Vec<String> {
	let mut features = registry.host_features.clone();

	if devtools
		&& !release
		&& target.reports_to_devtools()
		&& let Some(feature) = &registry.devtools_feature
	{
		features.push(feature.clone());
	}

	features
}

/// The NDK's prebuilt toolchain directory for the machine running the build.
const NDK_HOST: &str = if cfg!(target_os = "macos") {
	"darwin-x86_64"
} else if cfg!(target_os = "windows") {
	"windows-x86_64"
} else {
	"linux-x86_64"
};

/// The extension of the NDK's clang launchers on this machine.
const NDK_LAUNCHER_SUFFIX: &str = if cfg!(target_os = "windows") {
	".cmd"
} else {
	""
};

/// Points the host build's linker and cc-rs at the clang of the NDK at `ndk`
/// for an Android target, and records the NDK for Gradle.
fn configure_android_toolchain(
	host: &mut Command,
	root: &Path,
	target: Target,
	ndk: &Path,
) -> Result<()> {
	let architecture = if matches!(target, Target::AndroidX86_64) {
		"x86_64"
	} else {
		"aarch64"
	};

	let tools = ndk.join(format!("toolchains/llvm/prebuilt/{NDK_HOST}/bin"));
	let compiler = tools.join(format!(
		"{architecture}-linux-android26-clang{NDK_LAUNCHER_SUFFIX}"
	));
	anyhow::ensure!(
		compiler.is_file(),
		"selected NDK does not contain a compiler for this host: {}",
		compiler.display()
	);
	project::write(
		&root.join(".leptoskit/android/ndk-path.txt"),
		&gradle_ndk_path(ndk)?,
	)?;
	// C dependencies of the host (ring, through the code push transport's
	// TLS) compile with cc-rs, which needs the NDK compiler and archiver
	// too.
	let target_key = target.rust().replace('-', "_");
	host.env(
		format!(
			"CARGO_TARGET_{}_LINUX_ANDROID_LINKER",
			architecture.to_ascii_uppercase()
		),
		&compiler,
	)
	.env(format!("CC_{target_key}"), compiler)
	.env(
		format!("CXX_{target_key}"),
		tools.join(format!(
			"{architecture}-linux-android26-clang++{NDK_LAUNCHER_SUFFIX}"
		)),
	)
	.env(
		format!("AR_{target_key}"),
		tools.join(if cfg!(windows) {
			"llvm-ar.exe"
		} else {
			"llvm-ar"
		}),
	);
	Ok(())
}

/// Runs the platform build tool that packages the prepared host into an
/// application: Xcode, Gradle, or DevEco's hvigor.
fn build_application(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
	release: bool,
	build_number: Option<u64>,
) -> Result<()> {
	match target {
		Target::Macos => desktop::build(root, config, release),
		Target::Windows | Target::Linux | Target::Tui => unreachable!("desktop hosts built above"),
		Target::Harmonyos | Target::HarmonyosX86_64 => harmonyos::build(root, release),
		Target::Wearos | Target::Watchos => watch::build(target),
		Target::Ios | Target::IosSimulator => {
			install_ios_pods(root)?;
			run_tool(&mut ios_build(root, target, release, build_number))
		}
		Target::Android | Target::AndroidX86_64 | Target::Quest => {
			run_tool(&mut gradle_build(root, target, release, build_number))
		}
	}
}

/// The generated Podfile, present while a selected plugin declares pods.
pub(crate) fn ios_podfile(root: &Path) -> Option<PathBuf> {
	let podfile = root.join(".leptoskit/ios/Podfile");
	podfile.is_file().then_some(podfile)
}

/// Whether the project's selected iOS plugins install `CocoaPods`, which makes
/// `pod` a required tool. Resolution problems surface in the build itself,
/// so callers treat them as "no pods".
pub(crate) fn ios_plugins_use_pods(root: &Path, config: &LeptoskitConfig) -> bool {
	registry::resolve(root, config, Target::Ios)
		.is_ok_and(|registry| registry.apple_pods().is_ok_and(|pods| !pods.is_empty()))
}

/// Runs `pod install` on the generated Podfile, so apps never manage
/// `CocoaPods` by hand. A no-op without pods: `CocoaPods` is needed only when a
/// plugin uses it, which `leptoskit doctor` reports.
pub(crate) fn install_ios_pods(root: &Path) -> Result<()> {
	let Some(podfile) = ios_podfile(root) else {
		return Ok(());
	};
	let directory = podfile
		.parent()
		.context("the generated Podfile has no directory")?;
	let mut install = Command::new("pod");
	install.current_dir(directory).arg("install");

	match install.status() {
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
			anyhow::bail!(
			"CocoaPods is required while a selected plugin declares apple-pods; install it from \
			 https://cocoapods.org and run `leptoskit doctor`"
		)
		}

		Err(error) => {
			return Err(error).context("could not run `pod install`");
		}
		Ok(status) => anyhow::ensure!(status.success(), "`pod install` failed: {status}"),
	}

	Ok(())
}

/// The Gradle wrapper invocation that assembles the generated Android host,
/// and for an unsigned release, its App Bundle too.
fn gradle_build(root: &Path, target: Target, release: bool, build_number: Option<u64>) -> Command {
	let android = host_dir(root, target);
	let mut command = Command::new(android.join(if cfg!(windows) {
		"gradlew.bat"
	} else {
		"gradlew"
	}));

	command
		.current_dir(&android)
		.args(["--no-daemon", "--console=plain", "--quiet"]);

	if release {
		// The APK installs directly; Google Play takes the App Bundle.
		command.args(["assembleRelease", "bundleRelease"]);
	} else {
		command.arg("assembleDebug");
	}

	if let Some(build_number) = build_number {
		command.arg(format!("-Pleptoskit.versionCode={build_number}"));
	}

	command
}

/// The `xcodebuild` invocation that builds the generated iOS host project,
/// unsigned for a device: a signed release is archived and exported
/// instead. `-quiet` leaves warnings and errors only; the rest is build
/// chatter.
fn ios_build(root: &Path, target: Target, release: bool, build_number: Option<u64>) -> Command {
	let architecture = if matches!(target, Target::IosSimulator) && cfg!(target_arch = "x86_64") {
		"ARCHS=x86_64"
	} else {
		"ARCHS=arm64"
	};

	// Pods integrate the project into a workspace, which the build uses
	// instead of the project itself.
	let container: [&str; 2] = if ios_podfile(root).is_some() {
		["-workspace", ".leptoskit/ios/Host.xcworkspace"]
	} else {
		["-project", ".leptoskit/ios/Host.xcodeproj"]
	};

	// Naming the destination keeps xcodebuild from printing every one it
	// knows when the scheme leaves the choice open.
	let destination = if matches!(target, Target::Ios) {
		"generic/platform=iOS"
	} else {
		"generic/platform=iOS Simulator"
	};

	let mut command = Command::new("xcodebuild");
	// Xcode reads `LD` and `LDPLUSPLUS` from the environment as its linker
	// drivers. A nix or devenv shell sets `LD=ld`, which rejects the
	// clang-style flags Xcode passes it.
	command.env_remove("LD").env_remove("LDPLUSPLUS");
	command
		.current_dir(root)
		.arg("-quiet")
		.args(container)
		.args([
			"-scheme",
			"App",
			"-destination",
			destination,
			"-configuration",
			if release { "Release" } else { "Debug" },
			"-derivedDataPath",
			".leptoskit/ios/build",
			architecture,
			"build",
		]);

	if matches!(target, Target::IosSimulator) {
		// Simulator Keychain access needs an ad-hoc signed app.
		// Disabling signing produces a launchable app without this
		// capability.
		command.args(["CODE_SIGNING_ALLOWED=YES", "CODE_SIGN_IDENTITY=-"]);
	} else {
		command.arg("CODE_SIGNING_ALLOWED=NO");
	}

	if let Some(build_number) = build_number {
		command.arg(format!("CURRENT_PROJECT_VERSION={build_number}"));
	}

	command
}

/// Rebuilds only the Wasm guest a debug build of `target` embeds, exactly as
/// `native build` does, and returns it. `leptoskit dev` sends it to the apps
/// it launched instead of rebuilding and reinstalling them.
pub(crate) fn rebuild_guest(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
) -> Result<Vec<u8>> {
	let _guard = lock::acquire(root)?;
	let plugins = crate::build::resolve_plugins(config)?;
	plugins.generate(root, target.platform(), false)?;
	crate::l10n::sync(root, config)?;
	let registry = registry::resolve(root, config, target)?;
	let build_dir = native_build_directory(root, &registry.target, target.rust())?;
	let generated = generated_dir(root, target);
	let bundle_dir = generated.join("bundle");
	let contract = bundle(
		root,
		config,
		&registry.plugins,
		&registry.missing,
		target,
		false,
		Some(&build_dir),
		&bundle_dir,
	)?;
	write_capabilities(&generated.join("capabilities.json"), &contract)?;
	built_guest(root, target)
}

/// The guest the last build of `target` produced.
pub(crate) fn built_guest(root: &Path, target: Target) -> Result<Vec<u8>> {
	let path = generated_dir(root, target).join("bundle/app.wasm");
	std::fs::read(&path).with_context(|| format!("could not read {}", path.display()))
}

/// The inferred destination used by pre-universal native projects.
pub(crate) fn legacy_target(root: &Path, config: &LeptoskitConfig) -> Target {
	use leptoskit_config::NativeTarget;

	match config.native.targets.as_slice() {
		[NativeTarget::Harmonyos] => Target::Harmonyos,
		[NativeTarget::Wearos] => Target::Wearos,
		[NativeTarget::Watchos] => Target::Watchos,
		[NativeTarget::Quest] => Target::Quest,
		[NativeTarget::Macos] => Target::Macos,
		[NativeTarget::Ios] => Target::IosSimulator,
		[NativeTarget::Android] => Target::Android,
		[NativeTarget::Windows] => Target::Windows,
		[NativeTarget::Linux] => Target::Linux,

		_ if cfg!(target_os = "macos") && host_dir(root, Target::Macos).is_dir() => Target::Macos,

		_ if cfg!(target_os = "macos") => Target::IosSimulator,
		_ => Target::Android,
	}
}

/// Prints the size of the artifact a finished platform build produced, so a
/// size change surfaces in the build that caused it. A missing artifact or a
/// measurement failure never fails the build: the report is information, not
/// a gate.
fn report_application_size(root: &Path, target: Target, release: bool) {
	let report = match target {
		Target::Macos => size::macos(root).map(Some),
		Target::Ios | Target::IosSimulator => size::ios(root, release),
		Target::Android | Target::AndroidX86_64 | Target::Quest => size::android(root, release),
		// The desktop hosts and the harmonyos packaging report through their
		// own tools; a measurement for them is a follow-up.
		Target::Windows
		| Target::Linux
		| Target::Harmonyos
		| Target::HarmonyosX86_64
		| Target::Wearos
		| Target::Watchos
		| Target::Tui => Ok(None),
	};

	match report {
		Ok(Some(report)) => {
			println!("application size ({}):\n{}", report.label, report.render());
		}
		Ok(None) => crate::ui::skipped("built; no artifact found to measure"),
		Err(error) => crate::ui::warning(format_args!("could not measure the artifact: {error}")),
	}
}

/// The cc-rs compiler variables for an Apple target, pointing at the SDK's
/// own clang: C dependencies of the host (ring, through the code push
/// transport's TLS) compile with cc-rs, and a wrapped cc on PATH (nix, a
/// sysroot prefix) rejects Apple targets. Empty off macOS, where there is no
/// SDK to find.
fn apple_cc_environment(target: Target) -> Vec<(String, String)> {
	let sdk = match target {
		Target::Ios => "iphoneos",
		Target::IosSimulator => "iphonesimulator",
		_ => "macosx",
	};

	let target_key = target.rust().replace('-', "_");
	let mut environment = Vec::new();

	if let Some(compiler) = xcrun_find(sdk, "clang") {
		environment.push((format!("CC_{target_key}"), compiler.display().to_string()));

		if let Some(plusplus) = xcrun_find(sdk, "clang++") {
			environment.push((format!("CXX_{target_key}"), plusplus.display().to_string()));
		}
	}

	environment
}

/// Copies the app's `assets/` folder to `destination`, where the native app
/// bundles it, so `asset://` images, 3D models, and sounds read the same
/// files on every platform. The copy follows the folder: a file whose size
/// and modification time match is left alone, a changed one is copied
/// again, and one the app removed is removed from the copy. Hidden files,
/// such as `.DS_Store`, and symbolic links stay behind.
///
/// Assets ship with the store build. A code push update carries the guest
/// alone, so a release that shows a new asset needs a new store build.
fn stage_assets(root: &Path, destination: &Path) -> Result<()> {
	sync_folder(&root.join(ASSETS), destination)
}

/// Makes `destination` a copy of `source`'s ordinary, visible files, as
/// [`stage_assets`] describes; a missing `source` leaves it empty.
pub(crate) fn sync_folder(source: &Path, destination: &Path) -> Result<()> {
	let wanted: Vec<(PathBuf, std::fs::Metadata)> = if source.is_dir() {
		walkdir::WalkDir::new(source)
			.follow_links(false)
			.into_iter()
			.filter_entry(|entry| entry.depth() == 0 || !hidden(entry.file_name()))
			.filter_map(|entry| {
				entry
					.map_err(anyhow::Error::from)
					.and_then(|entry| {
						if entry.file_type().is_file() {
							let relative = entry.path().strip_prefix(source)?.to_path_buf();
							Ok(Some((relative, entry.metadata()?)))
						} else {
							Ok(None)
						}
					})
					.transpose()
			})
			.collect::<Result<_>>()?
	} else {
		Vec::new()
	};

	std::fs::create_dir_all(destination)
		.with_context(|| format!("create {}", destination.display()))?;
	let keep: std::collections::BTreeSet<&Path> =
		wanted.iter().map(|(path, _)| path.as_path()).collect();

	// What the app no longer has leaves the copy, deepest first, so emptied
	// folders go too.
	for entry in walkdir::WalkDir::new(destination)
		.min_depth(1)
		.contents_first(true)
	{
		let entry = entry?;
		let relative = entry.path().strip_prefix(destination)?;

		if entry.file_type().is_dir() {
			if std::fs::read_dir(entry.path())?.next().is_none() {
				std::fs::remove_dir(entry.path())?;
			}
		} else if !keep.contains(relative) {
			std::fs::remove_file(entry.path())
				.with_context(|| format!("remove {}", entry.path().display()))?;
		}
	}

	for (relative, metadata) in wanted {
		let target = destination.join(&relative);
		let current = std::fs::metadata(&target).ok();
		let fresh = current.is_some_and(|current| {
			current.len() == metadata.len()
				&& matches!(
					(metadata.modified(), current.modified()),
					(Ok(source), Ok(copy)) if source <= copy
				)
		});

		if !fresh {
			project::copy(&source.join(&relative), &target)?;
		}
	}

	Ok(())
}

/// The folder in the app's project, the web build, and each native bundle
/// that holds the files the app ships: `leptoskit_native::asset::FOLDER`.
pub(crate) const ASSETS: &str = "assets";

fn hidden(name: &std::ffi::OsStr) -> bool {
	name.to_str().is_some_and(|name| name.starts_with('.'))
}

/// Where `target`'s app bundles the staged `assets/` folder: the generated
/// `mobile/assets`, which Android packs under the APK's `assets/`, iOS
/// references as the bundle's `assets` folder, and the macOS app copies
/// into `Contents/Resources/assets`; the folder beside the Linux and Windows
/// executable; and HarmonyOS's `rawfile/assets`.
fn assets_destination(root: &Path, target: Target) -> PathBuf {
	match target {
		Target::Linux | Target::Windows => generated_dir(root, target).join("app").join(ASSETS),
		Target::Harmonyos | Target::HarmonyosX86_64 => {
			root.join("hosts/harmonyos/entry/src/main/resources/rawfile")
				.join(ASSETS)
		}
		_ => generated_dir(root, target).join("mobile").join(ASSETS),
	}
}

/// Writes what the target's shell bundles from the built guest. Desktop gets
/// the raw Wasm. Mobile gets the guest inside its store container, which the
/// update store verifies, and the update configuration, but never the Wasm a
/// second time.
fn write_assets(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
	bundle_dir: &Path,
	release: bool,
	contract: &HostCapabilityDescriptor,
) -> Result<()> {
	let generated = generated_dir(root, target);
	if matches!(
		target,
		Target::Macos | Target::Windows | Target::Linux | Target::Tui
	) {
		return project::copy(
			&bundle_dir.join("app.wasm"),
			&generated.join("assets/app.wasm"),
		);
	}

	if matches!(target, Target::Harmonyos | Target::HarmonyosX86_64) {
		return project::copy(
			&bundle_dir.join("app.wasm"),
			&root.join("hosts/harmonyos/entry/src/main/resources/rawfile/app.wasm"),
		);
	}

	let assets = generated.join("mobile");
	std::fs::create_dir_all(&assets)?;
	let channel = config::channel(root, &config.codepush.channel, target.platform());
	leptoskit_codepush::manifest::bundle_directory_as(
		bundle_dir,
		&assets.join("app.lkb"),
		&config.app.name,
		&channel,
		config.app.version.clone(),
		0,
	)?;
	let mut update_config = serde_json::json!({
		"enabled": config.codepush.enabled, "app": config.app.name,
		"channel": channel, "server": config.codepush.server,
		"public_keys": config.codepush.public_keys,
		"runtime": host_runtime_descriptor(root, config, target, contract),
		"allow_http": config.codepush.allow_http && !release,
	});
	// The host's own trust policy: shipped with the store build, never with a
	// download.
	if let Some(provenance) = &config.codepush.provenance {
		update_config["provenance"] = serde_json::to_value(provenance)?;
	}

	std::fs::write(
		assets.join("codepush.json"),
		serde_json::to_vec_pretty(&update_config)?,
	)?;
	Ok(())
}

/// Builds the native ABI guest without a platform shell or JavaScript glue.
/// The guest always compiles for release — it is interpreted by wasmi, where a
/// debug build is both slower and larger — with [`bundle.optimize`] picking
/// the profile exactly as it does for the web client. `wasm-opt -Oz` runs
/// only when the enclosing build is a release build, like the web client, and
/// warns rather than fails when binaryen is missing: a machine that only
/// builds for devices still prepares, bundles, and debugs native apps.
#[allow(clippy::too_many_arguments)]
fn bundle(
	root: &Path,
	config: &LeptoskitConfig,
	plugins: &[registry::Plugin],
	missing: &[registry::MissingPlugin],
	target: Target,
	release: bool,
	// Isolates same-named applications that share a Cargo target directory.
	target_dir: Option<&Path>,
	out: &Path,
) -> Result<HostCapabilityDescriptor> {
	anyhow::ensure!(
		!config.native.update_contract.trim().is_empty(),
		"native.update_contract cannot be empty"
	);
	let contract = registry::descriptor(config, target, plugins, missing);
	contract.validate()?;
	registry::requirements(config).validate()?;

	for warning in registry::optional_warnings(config, target, plugins, missing) {
		eprintln!("{warning}");
	}

	let metadata = crate::wasm::cargo_metadata(root)?;
	let portable = root
		.join(target.platform().to_string())
		.join("Cargo.toml")
		.is_file();
	let package = if portable {
		config
			.build
			.client_package
			.clone()
			.context("portable native builds require build.client_package")?
	} else {
		format!("{}_guest", config.app.name)
	};

	let client = crate::wasm::detect_client_with(Some(&package), None, &metadata)?
		.context("native application has no cdylib")?;
	let (plugins, context) = crate::build::plugin_context(config, target.platform(), release)?;
	let mut rustflags = config.build.rustflags.clone();
	rustflags.extend(context.rustflags);
	let mut options = crate::wasm::CargoOptions {
		release,
		features: context.cargo_features,
		rustflags,
		env: context.env,
		..Default::default()
	};

	if portable {
		let feature = target
			.platform()
			.feature()
			.context("native platform feature missing")?;
		let package = metadata
			.package(&package)
			.context("native application package missing")?;
		anyhow::ensure!(
			package.declares_feature(feature),
			"native app package `{}` must declare feature `{feature}`",
			package.name
		);
		options = options.with_platform(target.platform())?;
		options.target_directory = Some(generated_dir(root, target).join("target"));
	} else if let Some(target_dir) = target_dir {
		options.target_directory = Some(target_dir.to_path_buf());
	}

	if target.reports_to_devtools() {
		options =
			crate::build::devtools::guest_options(options, metadata.package(&package), release);
	}

	let options = guest_profile::select(config, options);

	let wasm = crate::wasm::compile_wasm(root, &client, &options)?;
	std::fs::create_dir_all(out)?;
	let module = out.join("app.wasm");
	project::copy(&wasm, &module)?;

	if release && config.bundle.wasm_opt {
		// Native builds cannot require binaryen the way the web client does:
		// `native prepare` and `native bundle` also run on machines that only
		// build for devices. Without it the guest ships unoptimized, with a
		// warning saying so; the size report makes the difference visible.
		match crate::wasm::wasm_opt_version() {
			Ok(_) => crate::wasm::optimize_guest(&module)?,
			Err(_) => {
				crate::ui::warning(
					"wasm-opt is not installed, so the guest wasm ships unoptimized; install \
					 binaryen for the smaller build",
				);
			}
		}
	}

	std::fs::write(
		out.join("runtime.json"),
		serde_json::to_vec_pretty(&guest_runtime_descriptor(root, config, target))?,
	)?;
	plugins.after_build(&mut leptoskit_plugin::PluginPostBuildContext {
		config: config.clone(),
		platform: target.platform(),
		release,
		output_dir: out.to_path_buf(),
		wasm_bundle: Some(out.join("app.wasm")),
	})?;
	crate::ui::success(format_args!(
		"native Wasm bundle: {} (capability contract {})",
		out.display(),
		contract.digest()
	));
	Ok(contract)
}

/// What a debug host build adds to cargo's default profile: its dependencies,
/// the Wasm interpreter, layout engine, and renderer among them, compile
/// optimized. Unoptimized, the interpreter makes every tap take a visible
/// fraction of a second. The app's own crates stay unoptimized so rebuilding
/// them stays fast, and debug assertions stay on, which is what lets a debug
/// build follow `leptoskit dev`.
///
/// The interpreter is the exception: its debug assertions are off. Optimized,
/// it dispatches each instruction with a tail call, and the assertions keep
/// LLVM from turning those calls into jumps. Every instruction would then
/// stay on the native stack until the guest returns, and a guest loop
/// overflows it.
pub(crate) const DEBUG_HOST_ARGUMENTS: &[&str] = &[
	"--config",
	"profile.dev.package.\"*\".opt-level=3",
	"--config",
	"profile.dev.package.wasmi.debug-assertions=false",
];

/// Hands `[native] log-level` to the runtime's logger, which reads it when
/// it is compiled. Unset, the runtime logs `info` in a debug build and
/// `warn` in a release build.
pub(crate) fn set_log_level(command: &mut Command, config: &LeptoskitConfig) {
	if let Some(level) = config.native.log_level {
		command.env(leptoskit_config::LogLevel::VARIABLE, level.as_str());
	}
}

/// The profile arguments a release cargo build takes: `--release`, or
/// `--profile <name>` with the `--config` entries defining the profile.
/// `host` picks between the two size profiles: the wasm one can
/// `panic = "abort"`, while the host's `catch_unwind` boundary cannot.
fn release_arguments(config: &LeptoskitConfig, host: bool) -> (String, Vec<String>) {
	match &config.bundle.cargo_profile {
		Some(profile) => (profile.clone(), vec!["--profile".into(), profile.clone()]),

		None if config.bundle.optimize == Optimize::Smallest && host => {
			(
				crate::wasm::HOST_SMALLEST_PROFILE.into(),
				profile_arguments(crate::wasm::HOST_SMALLEST_PROFILE),
			)
		}

		None if config.bundle.optimize == Optimize::Smallest => {
			(
				crate::wasm::SMALLEST_PROFILE.into(),
				profile_arguments(crate::wasm::SMALLEST_PROFILE),
			)
		}

		_ => ("release".into(), vec!["--release".into()]),
	}
}

/// `--profile <name>` plus the `--config` entries defining one of leptoskit's
/// size profiles, taken from the wasm module that owns them.
fn profile_arguments(profile: &str) -> Vec<String> {
	let config = if profile == crate::wasm::HOST_SMALLEST_PROFILE {
		crate::wasm::host_smallest_profile_config()
	} else {
		crate::wasm::smallest_profile_config()
	};

	let mut arguments = vec!["--profile".to_owned(), profile.to_owned()];

	for entry in config {
		arguments.push("--config".into());
		arguments.push(entry);
	}

	arguments
}

/// The directory under `target/<triple>` cargo writes a profile into: the
/// profile's own name, except the dev profile's `debug`.
fn profile_directory(config: &LeptoskitConfig, host: bool, release: bool) -> String {
	if !release {
		return "debug".into();
	}

	let (profile, _) = release_arguments(config, host);

	match profile.as_str() {
		"release" => "release".into(),
		name => name.to_owned(),
	}
}

/// The ordinary bundled file makes compatibility part of the signed content.
/// A guest carries requirements only; it never declares what the host can do.
fn guest_runtime_descriptor(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
) -> serde_json::Value {
	let shell = config::shell(root, config, target.platform());
	let requirements = registry::requirements(config);
	// Optional plugins must not become mandatory through the legacy list.
	let plugins: Vec<_> = config
		.native
		.plugins
		.iter()
		.filter(|name| {
			!requirements
				.optional
				.contains(&leptoskit_codepush::capability::plugin_capability(name))
		})
		.collect();

	serde_json::json!({
		"abi": leptoskit_wasm::ABI_VERSION,
		"shell": shell,
		"plugins": plugins,
		"format_version": RUNTIME_CONTRACT_VERSION,
		"requirements": requirements,
	})
}

/// The store build's `codepush.json` carries the host descriptor; it is never
/// read from a download.
fn host_runtime_descriptor(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
	contract: &HostCapabilityDescriptor,
) -> serde_json::Value {
	let shell = config::shell(root, config, target.platform());

	serde_json::json!({
		"abi": leptoskit_wasm::ABI_VERSION,
		"shell": shell,
		"plugins": config.native.plugins,
		"format_version": RUNTIME_CONTRACT_VERSION,
		"host": contract,
	})
}

fn ensure_target_enabled(config: &LeptoskitConfig, target: Target) -> Result<()> {
	// Legacy HarmonyOS projects selected their only destination through
	// native.targets before app.platforms existed.
	if target.platform() == Platform::Harmonyos
		&& config.app.default_platform == Platform::Native
		&& config.native.targets == [leptoskit_config::NativeTarget::Harmonyos]
		&& config.app.platforms == Platform::APPLICATION
	{
		return Ok(());
	}

	config.ensure_platform_enabled(target.platform())?;

	Ok(())
}

fn native_build_directory(root: &Path, target_dir: &Path, target: &str) -> Result<PathBuf> {
	let root = root
		.canonicalize()
		.context("canonicalize native application directory")?;
	let identity = hex::encode(Sha256::digest(root.as_os_str().as_encoded_bytes()));
	Ok(target_dir
		.join("leptoskit-native")
		.join(identity)
		.join(target))
}

pub(crate) fn run_tool(command: &mut Command) -> Result<()> {
	let status = command
		.status()
		.with_context(|| format!("could not execute {command:?}"))?;
	anyhow::ensure!(status.success(), "{command:?} failed: {status}");
	Ok(())
}

/// Runs the host library build, printing what it prints, and returns the native
/// libraries rustc reported for the archive, in the order it reported them.
fn run_host_library(command: &mut Command) -> Result<Vec<String>> {
	use std::io::BufRead as _;

	let mut child = command
		.stdout(std::process::Stdio::inherit())
		.stderr(std::process::Stdio::piped())
		.spawn()
		.with_context(|| format!("could not execute {command:?}"))?;
	let stderr = child.stderr.take().context("host build has no stderr")?;
	let mut output = String::new();

	for line in std::io::BufReader::new(stderr).lines() {
		let line = line?;
		eprintln!("{line}");
		output.push_str(&line);
		output.push('\n');
	}

	let status = child.wait()?;
	anyhow::ensure!(status.success(), "{command:?} failed: {status}");
	Ok(native_static_libs(&output))
}

/// The libraries in rustc's `note: native-static-libs:` line, with `-l`
/// stripped, in rustc's order and without the repeats it keeps for linkers that
/// care about them. Anything that is not a `-l` entry is left to the C linker.
fn native_static_libs(output: &str) -> Vec<String> {
	let mut libraries: Vec<String> = Vec::new();

	for line in output.lines() {
		let Some(list) = line.trim().strip_prefix("note: native-static-libs:") else {
			continue;
		};

		for entry in list.split_whitespace() {
			let Some(name) = entry.strip_prefix("-l") else {
				continue;
			};

			if !name.is_empty() && !libraries.iter().any(|known| known == name) {
				libraries.push(name.to_owned());
			}
		}
	}

	libraries
}

/// The path of `tool` inside an SDK, from the `xcrun` on `PATH`: Xcode's own
/// compiler, not whatever `cc` resolves to on the surrounding machine.
fn xcrun_find(sdk: &str, tool: &str) -> Option<PathBuf> {
	xcrun_find_in(
		std::env::var_os("PATH").as_deref().unwrap_or_default(),
		sdk,
		tool,
	)
}

/// [`xcrun_find`] over an explicit `PATH`, so the lookup is testable without
/// mutating the environment.
fn xcrun_find_in(search: &std::ffi::OsStr, sdk: &str, tool: &str) -> Option<PathBuf> {
	let mut xcrun = None;

	for directory in std::env::split_paths(&search) {
		let candidate = directory.join("xcrun");

		if candidate.is_file() {
			xcrun = Some(candidate);
			break;
		}
	}

	let output = Command::new(xcrun?)
		.args(["--sdk", sdk, "--find", tool])
		.output()
		.ok()?;

	if !output.status.success() {
		return None;
	}

	let path = String::from_utf8(output.stdout).ok()?;
	let path = PathBuf::from(path.trim());
	path.is_file().then_some(path)
}

/// The NDK Gradle and the host build use: `ANDROID_NDK_HOME`, or the newest
/// under the SDK `ANDROID_HOME` names.
pub(crate) fn android_ndk() -> Result<PathBuf> {
	if let Some(path) = std::env::var_os("ANDROID_NDK_HOME") {
		return checked_ndk(Path::new(&path));
	}

	let sdk = std::env::var_os("ANDROID_HOME")
		.or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
		.context("set ANDROID_HOME and install an NDK, or set ANDROID_NDK_HOME")?;
	let mut versions = std::fs::read_dir(Path::new(&sdk).join("ndk"))?
		.map(|entry| entry.map(|entry| entry.path()))
		.collect::<std::io::Result<Vec<_>>>()?;
	versions.retain(|path| path.join("source.properties").is_file());
	versions.sort_by_cached_key(|path| {
		path.file_name()
			.and_then(|name| name.to_str())
			.unwrap_or_default()
			.split('.')
			.map(|part| part.parse::<u64>().unwrap_or(0))
			.collect::<Vec<_>>()
	});
	checked_ndk(&versions.pop().context("no Android NDK is installed")?)
}

fn checked_ndk(path: &Path) -> Result<PathBuf> {
	let path = path.canonicalize().context("resolve Android NDK path")?;
	anyhow::ensure!(
		path.join("source.properties").is_file(),
		"Android NDK is missing source.properties: {}",
		path.display()
	);
	Ok(path)
}

fn gradle_ndk_path(path: &Path) -> Result<String> {
	let path = path.to_str().context("Android NDK path must be UTF-8")?;
	// Rust canonicalization uses verbatim Windows prefixes; Gradle/Java expect
	// an ordinary drive or UNC path in their Android DSL.
	Ok(if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
		format!(r"\\{unc}")
	} else {
		path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn generated_contract_reports_unresolved_grants_without_claiming_support() {
		use leptoskit_codepush::capability::ContractCompatibility;
		use leptoskit_codepush::installed::RuntimeContract;

		let root = Path::new(".");
		let mut config = LeptoskitConfig::default();
		config.native.plugins = vec!["http".into()];
		let descriptor = registry::descriptor(&config, Target::Ios, &[], &[]);
		let host: RuntimeContract = serde_json::from_value(host_runtime_descriptor(
			root,
			&config,
			Target::Ios,
			&descriptor,
		))
		.unwrap();
		let guest = |config: &LeptoskitConfig| -> RuntimeContract {
			serde_json::from_value(guest_runtime_descriptor(root, config, Target::Ios)).unwrap()
		};
		assert_eq!(
			guest(&config).verify_guest(&host).unwrap(),
			ContractCompatibility::Incomplete
		);
		config.native.required_plugins.push("http".into());

		assert!(guest(&config).verify_guest(&host).is_err());
		config.native.required_plugins.clear();
		config.native.optional_plugins.push("http".into());
		assert_eq!(
			guest(&config).verify_guest(&host).unwrap(),
			ContractCompatibility::Verified
		);
	}

	#[test]
	fn optional_plugins_do_not_become_legacy_update_requirements() {
		let mut config = LeptoskitConfig::default();
		config.native.plugins = vec!["http".into(), "haptics".into(), "clipboard".into()];
		config.native.optional_plugins = vec!["haptics".into()];
		config.native.optional_capabilities = vec!["plugin.clipboard".into()];
		let runtime: leptoskit_codepush::installed::RuntimeContract = serde_json::from_value(
			guest_runtime_descriptor(Path::new("."), &config, Target::Ios),
		)
		.unwrap();
		assert_eq!(runtime.plugins, vec!["http"]);
		let required = runtime.requirements.unwrap();
		assert_eq!(
			required.optional,
			vec!["plugin.clipboard", "plugin.haptics"]
		);

		assert_eq!(required.protocol.bridge, leptoskit_bridge::VERSION);
	}

	#[test]
	fn universal_platforms_isolate_generated_state_and_update_contracts() {
		let root =
			std::env::temp_dir().join(format!("leptoskit-platform-state-{}", std::process::id()));
		std::fs::create_dir_all(root.join("ios")).unwrap();
		std::fs::create_dir_all(root.join("android")).unwrap();
		std::fs::write(root.join("ios/Cargo.toml"), "").unwrap();
		std::fs::write(root.join("android/Cargo.toml"), "").unwrap();
		let mut config = LeptoskitConfig::default();

		assert_eq!(host_dir(&root, Target::IosSimulator), root.join("ios"));
		assert_ne!(
			generated_dir(&root, Target::Ios),
			generated_dir(&root, Target::Android)
		);
		assert_eq!(
			generated_dir(&root, Target::Ios),
			generated_dir(&root, Target::IosSimulator)
		);
		let ios_contract = registry::descriptor(&config, Target::Ios, &[], &[]);
		let android_contract = registry::descriptor(&config, Target::Android, &[], &[]);
		let ios: leptoskit_codepush::installed::RuntimeContract = serde_json::from_value(
			host_runtime_descriptor(&root, &config, Target::Ios, &ios_contract),
		)
		.unwrap();
		let android: leptoskit_codepush::installed::RuntimeContract = serde_json::from_value(
			host_runtime_descriptor(&root, &config, Target::Android, &android_contract),
		)
		.unwrap();
		assert_ne!(ios.shell, android.shell);

		assert_ne!(
			ios.host.as_ref().unwrap().platform,
			android.host.as_ref().unwrap().platform
		);

		let bundle = root.join("bundle");
		std::fs::create_dir_all(&bundle).unwrap();
		std::fs::write(bundle.join("app.wasm"), b"\0asm\x01\0\0\0").unwrap();
		config.app.name = "platform_app".into();

		for (target, contract) in [
			(Target::Ios, &ios_contract),
			(Target::Android, &android_contract),
		] {
			std::fs::write(
				bundle.join("runtime.json"),
				serde_json::to_vec(&guest_runtime_descriptor(&root, &config, target)).unwrap(),
			)
			.unwrap();
			write_assets(&root, &config, target, &bundle, false, contract).unwrap();
			let mobile = generated_dir(&root, target).join("mobile");
			let container = std::fs::read(mobile.join("app.lkb")).unwrap();
			let manifest = leptoskit_codepush::read_container(&container)
				.unwrap()
				.manifest;
			let updates: serde_json::Value =
				serde_json::from_slice(&std::fs::read(mobile.join("codepush.json")).unwrap())
					.unwrap();

			assert_eq!(
				manifest.channel,
				format!("production-{}", target.platform())
			);
			assert_eq!(updates["channel"].as_str(), Some(manifest.channel.as_str()));
		}

		config.app.platforms = vec![Platform::Web];
		assert!(ensure_target_enabled(&config, Target::Ios).is_err());
	}

	#[test]
	fn report_application_size_prints_the_artifact_it_found() {
		let root = std::env::temp_dir()
			.canonicalize()
			.unwrap()
			.join(format!("leptoskit-size-print-{}", std::process::id()));
		let contents = root.join(".leptoskit/macos/App.app/Contents/MacOS");
		std::fs::create_dir_all(&contents).unwrap();
		std::fs::write(contents.join("App"), vec![0_u8; 1024]).unwrap();
		report_application_size(&root, Target::Macos, true);
		// A target with no artifact to measure still reports cleanly.
		report_application_size(&root, Target::Android, true);
		report_application_size(&root, Target::Harmonyos, true);
		// An iOS build reports its application bundle; an unreadable
		// products directory surfaces as a warning, not a
		// failure.
		let app = root.join(".leptoskit/ios/build/Build/Products/Release-iphonesimulator/App.app");

		std::fs::create_dir_all(&app).unwrap();
		std::fs::write(app.join("App"), vec![0_u8; 512]).unwrap();
		report_application_size(&root, Target::IosSimulator, true);
		let blocked = root.join(".leptoskit/ios/build/Build/Products");
		std::fs::remove_dir_all(&blocked).unwrap();
		std::fs::write(&blocked, b"file").unwrap();
		report_application_size(&root, Target::IosSimulator, true);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn legacy_native_layouts_enforce_enabled_platforms_and_preserve_harmonyos_opt_in() {
		let mut config = LeptoskitConfig::default();
		config.app.default_platform = Platform::Native;
		config.app.platforms = vec![Platform::Web];
		assert!(ensure_target_enabled(&config, Target::Android).is_err());
		config.native.targets = vec![leptoskit_config::NativeTarget::Harmonyos];
		assert!(ensure_target_enabled(&config, Target::Harmonyos).is_err());
		config.app.platforms = Platform::APPLICATION.to_vec();
		ensure_target_enabled(&config, Target::Harmonyos).unwrap();

		config.app.default_platform = Platform::Web;
		assert!(ensure_target_enabled(&config, Target::Harmonyos).is_err());
	}

	#[test]
	fn apple_cc_environment_names_the_sdk_and_offers_nothing_without_one() {
		for target in [Target::Ios, Target::IosSimulator, Target::Macos] {
			// On macOS the SDK compilers resolve; elsewhere the build cannot
			// depend on Xcode, so the environment is simply empty.
			let environment = apple_cc_environment(target);

			if cfg!(target_os = "macos") {
				assert!(
					environment.iter().any(|(key, _)| {
						key.starts_with("CC_") && key.ends_with(&target.rust().replace('-', "_"))
					}),
					"the SDK compiler is named for {target:?}: {environment:?}"
				);
			} else {
				assert!(environment.is_empty(), "no SDK off macOS: {environment:?}");
			}
		}
	}

	#[test]
	fn release_arguments_follow_optimize_and_the_cargo_profile_escape_hatch() {
		let mut config = LeptoskitConfig::default();
		let (profile, arguments) = release_arguments(&config, true);
		assert_eq!(profile, "release");
		assert_eq!(arguments, vec!["--release".to_owned()]);

		config.bundle.optimize = Optimize::Smallest;
		let (profile, arguments) = release_arguments(&config, true);
		assert_eq!(profile, crate::wasm::HOST_SMALLEST_PROFILE);
		assert_eq!(arguments[0], "--profile");
		assert_eq!(arguments[1], profile);
		assert!(arguments.iter().any(|argument| {
			argument == "profile.leptoskit-host-smallest.build-override.strip = \"none\""
		}));
		assert!(
			!arguments
				.iter()
				.any(|argument| argument.contains("panic = \"abort\"")),
			"the host keeps unwinding for catch_unwind: {arguments:?}"
		);

		let (profile, arguments) = release_arguments(&config, false);
		assert_eq!(profile, crate::wasm::SMALLEST_PROFILE);
		assert!(
			arguments
				.iter()
				.any(|argument| argument == "profile.leptoskit-smallest.panic = \"abort\""),
			"the wasm guest may abort: {arguments:?}"
		);

		config.bundle.cargo_profile = Some("wasm-release".into());

		for host in [true, false] {
			let (profile, arguments) = release_arguments(&config, host);
			assert_eq!(profile, "wasm-release");
			assert_eq!(
				arguments,
				vec!["--profile".to_owned(), "wasm-release".to_owned()]
			);
		}
	}

	#[test]
	fn profile_directory_follows_the_build_and_its_profile() {
		let mut config = LeptoskitConfig::default();
		assert_eq!(profile_directory(&config, true, false), "debug");
		assert_eq!(profile_directory(&config, true, true), "release");
		config.bundle.optimize = Optimize::Smallest;
		assert_eq!(
			profile_directory(&config, true, true),
			crate::wasm::HOST_SMALLEST_PROFILE
		);
		config.bundle.cargo_profile = Some("wasm-release".into());
		assert_eq!(profile_directory(&config, true, true), "wasm-release");
	}

	#[test]
	fn debug_hosts_optimize_dependencies_and_keep_debug_assertions_outside_the_interpreter() {
		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		let dependency = root.join("dependency");
		let interpreter = root.join("wasmi");
		let app = root.join("app");
		std::fs::create_dir_all(dependency.join("src")).unwrap();
		std::fs::create_dir_all(interpreter.join("src")).unwrap();
		std::fs::create_dir_all(app.join("src")).unwrap();

		std::fs::write(
			dependency.join("Cargo.toml"),
			"[package]\nname = \"dependency\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
		)
		.unwrap();
		// Cargo tells a build script the optimization level it compiles with.
		std::fs::write(
			dependency.join("build.rs"),
			"fn main() { println!(\"cargo:rustc-env=DEPENDENCY_OPT_LEVEL={}\", \
			 std::env::var(\"OPT_LEVEL\").unwrap()); }",
		)
		.unwrap();
		std::fs::write(
			dependency.join("src/lib.rs"),
			"pub const OPT_LEVEL: &str = env!(\"DEPENDENCY_OPT_LEVEL\");\npub const \
			 DEBUG_ASSERTIONS: bool = cfg!(debug_assertions);\n",
		)
		.unwrap();
		// A stand-in with the interpreter's package name.
		std::fs::write(
			interpreter.join("Cargo.toml"),
			"[package]\nname = \"wasmi\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
		)
		.unwrap();
		std::fs::write(
			interpreter.join("src/lib.rs"),
			"pub const DEBUG_ASSERTIONS: bool = cfg!(debug_assertions);\n",
		)
		.unwrap();
		std::fs::write(
			app.join("Cargo.toml"),
			"[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \
			 \"2021\"\n[dependencies]\ndependency = { path = \"../dependency\" }\nwasmi = { path \
			 = \"../wasmi\" }\n[workspace]\n",
		)
		.unwrap();
		std::fs::write(
			app.join("src/main.rs"),
			"fn main() { println!(\"{} {} {} {}\", dependency::OPT_LEVEL, \
			 dependency::DEBUG_ASSERTIONS, wasmi::DEBUG_ASSERTIONS, cfg!(debug_assertions)); }",
		)
		.unwrap();
		let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
			.current_dir(&app)
			.env_remove("CARGO_BUILD_TARGET")
			.args(["run", "--offline", "--quiet", "--target-dir"])
			.arg(root.join("target"))
			.args(DEBUG_HOST_ARGUMENTS)
			.output()
			.unwrap();

		assert!(
			output.status.success(),
			"{}",
			String::from_utf8_lossy(&output.stderr)
		);
		assert_eq!(
			String::from_utf8(output.stdout).unwrap().trim(),
			"3 true false true",
			"dependencies optimize; debug assertions stay on outside the interpreter"
		);
	}

	#[test]
	fn same_named_cargo_apps_preserve_their_artifacts_in_a_shared_target_directory() {
		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		let shared = root.join("shared-target");
		let mut executables = Vec::new();

		for name in ["first", "second"] {
			let app = root.join(name);
			std::fs::create_dir_all(app.join("src")).unwrap();
			std::fs::write(
				app.join("Cargo.toml"),
				"[package]\nname = \"same-name\"\nversion = \"0.1.0\"\nedition = \
				 \"2021\"\n[workspace]\n",
			)
			.unwrap();

			std::fs::write(
				app.join("src/main.rs"),
				format!("fn main() {{ println!({name:?}); }}"),
			)
			.unwrap();
			let destination = native_build_directory(&app, &shared, "host").unwrap();
			let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
				.current_dir(&app)
				.env("CARGO_TARGET_DIR", &shared)
				.env_remove("CARGO_BUILD_TARGET")
				.args(["build", "--offline", "--target-dir"])
				.arg(&destination)
				.output()
				.unwrap();
			assert!(
				output.status.success(),
				"{}",
				String::from_utf8_lossy(&output.stderr)
			);
			executables.push(
				destination
					.join("debug")
					.join(format!("same-name{}", std::env::consts::EXE_SUFFIX)),
			);
			assert_eq!(
				native_build_directory(&app.join("src/.."), &shared, "host").unwrap(),
				destination
			);
			assert_ne!(
				native_build_directory(&app, &shared, "another-target").unwrap(),
				destination
			);
		}

		// The first artifact is consumed only after the second same-named app
		// has built, reproducing the native host build/copy interleaving.
		for (executable, expected) in executables.iter().zip(["first", "second"]) {
			let output = Command::new(executable).output().unwrap();
			assert!(output.status.success());
			assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
		}
	}

	#[cfg(unix)]
	#[test]
	fn xcrun_find_resolves_through_a_fake_sdk() {
		use std::os::unix::fs::PermissionsExt as _;

		let root = std::env::temp_dir()
			.canonicalize()
			.unwrap()
			.join(format!("leptoskit-xcrun-find-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		std::fs::write(root.join("clang"), b"").unwrap();
		std::fs::write(
			root.join("xcrun"),
			format!("#!/bin/sh\nprintf '%s\\n' \"{}/clang\"\n", root.display()),
		)
		.unwrap();
		std::fs::set_permissions(root.join("xcrun"), std::fs::Permissions::from_mode(0o755))
			.unwrap();
		let found = xcrun_find_in(root.as_os_str(), "macosx", "clang");
		assert_eq!(found, Some(root.join("clang")));
		assert!(
			xcrun_find_in(std::ffi::OsStr::new("/nonexistent"), "macosx", "clang").is_none(),
			"no xcrun on the search path finds nothing"
		);

		std::fs::remove_dir_all(root).unwrap();
	}

	/// A directory alias: a symlink on Unix, a junction (which needs no
	/// privilege) on Windows.
	fn directory_alias(target: &Path, alias: &Path) {
		#[cfg(unix)]
		std::os::unix::fs::symlink(target, alias).unwrap();
		#[cfg(windows)]
		assert!(
			Command::new("cmd")
				.args(["/C", "mklink", "/J"])
				.arg(alias)
				.arg(target)
				.status()
				.unwrap()
				.success()
		);
	}

	#[cfg(any(unix, windows))]
	#[test]
	fn native_cache_identity_resolves_directory_aliases() {
		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		let first = root.join("first");
		let second = root.join("second");
		std::fs::create_dir_all(&first).unwrap();
		std::fs::create_dir_all(&second).unwrap();
		let alias = root.join("alias");
		directory_alias(&first, &alias);

		let first_cache = native_build_directory(&first, root, "host").unwrap();
		assert_eq!(
			native_build_directory(&alias, root, "host").unwrap(),
			first_cache
		);
		assert_ne!(
			native_build_directory(&second, root, "host").unwrap(),
			first_cache
		);
	}

	// Each platform's own non-Unicode names: arbitrary bytes on Linux,
	// unpaired UTF-16 surrogates on Windows. APFS rejects non-UTF-8 names.
	#[cfg(any(target_os = "linux", windows))]
	#[test]
	fn native_cache_identity_preserves_non_utf8_paths() {
		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		#[cfg(target_os = "linux")]
		let (first, second) = {
			use std::os::unix::ffi::OsStringExt;
			(
				root.join(std::ffi::OsString::from_vec(vec![b'a', 0xfe])),
				root.join(std::ffi::OsString::from_vec(vec![b'a', 0xff])),
			)
		};
		#[cfg(windows)]
		let (first, second) = {
			use std::os::windows::ffi::OsStringExt;
			(
				root.join(std::ffi::OsString::from_wide(&[u16::from(b'a'), 0xD800])),
				root.join(std::ffi::OsString::from_wide(&[u16::from(b'a'), 0xD801])),
			)
		};
		std::fs::create_dir_all(&first).unwrap();
		std::fs::create_dir_all(&second).unwrap();

		assert_ne!(
			native_build_directory(&first, root, "host").unwrap(),
			native_build_directory(&second, root, "host").unwrap()
		);
	}

	#[test]
	fn ndk_paths_are_canonical_and_require_an_installation_manifest() {
		let root = std::env::temp_dir().join(format!("leptoskit-ndk-path-{}", std::process::id()));
		std::fs::create_dir_all(root.join("child")).unwrap();
		assert!(checked_ndk(&root).is_err());
		std::fs::write(root.join("source.properties"), "Pkg.Revision = 28.0.0").unwrap();
		assert_eq!(
			checked_ndk(&root.join("child/..")).unwrap(),
			root.canonicalize().unwrap()
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn assets_are_staged_fresh_for_the_native_bundle() {
		// Generated files are never written through a symlink, and macOS's
		// temporary directory sits behind one.
		let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
		let root = tempfile::tempdir_in(base).unwrap();
		let root = root.path();
		std::fs::create_dir_all(root.join("assets/models")).unwrap();
		std::fs::create_dir_all(root.join("assets/nfts")).unwrap();
		std::fs::create_dir_all(root.join("assets/.cache")).unwrap();
		std::fs::write(root.join("assets/models/rig.glb"), b"glTF").unwrap();
		std::fs::write(root.join("assets/nfts/ada.webp"), b"RIFF").unwrap();
		std::fs::write(root.join("assets/.DS_Store"), b"finder").unwrap();
		std::fs::write(root.join("assets/.cache/blob"), b"cache").unwrap();
		let staged = assets_destination(root, Target::Android);
		assert_eq!(staged, root.join(".leptoskit/mobile/assets"));
		std::fs::create_dir_all(staged.join("gone")).unwrap();
		std::fs::write(staged.join("gone/stale.glb"), b"old").unwrap();

		stage_assets(root, &staged).unwrap();

		assert_eq!(
			std::fs::read(staged.join("models/rig.glb")).unwrap(),
			b"glTF"
		);
		assert_eq!(
			std::fs::read(staged.join("nfts/ada.webp")).unwrap(),
			b"RIFF"
		);
		assert!(
			!staged.join("gone").exists(),
			"a removed asset, and its emptied folder, are not shipped"
		);
		assert!(
			!staged.join(".DS_Store").exists() && !staged.join(".cache").exists(),
			"hidden files stay behind"
		);

		// A changed file is copied again; an unchanged one is left alone.
		std::fs::write(root.join("assets/nfts/ada.webp"), b"RIFF-changed").unwrap();
		stage_assets(root, &staged).unwrap();
		assert_eq!(
			std::fs::read(staged.join("nfts/ada.webp")).unwrap(),
			b"RIFF-changed"
		);

		std::fs::remove_dir_all(root.join("assets")).unwrap();
		stage_assets(root, &staged).unwrap();
		assert!(staged.is_dir() && std::fs::read_dir(&staged).unwrap().next().is_none());
	}

	#[test]
	fn every_native_bundle_has_its_assets_folder() {
		let root = Path::new("/app");
		assert_eq!(
			assets_destination(root, Target::Ios),
			root.join(".leptoskit/mobile/assets")
		);
		assert_eq!(
			assets_destination(root, Target::Macos),
			root.join(".leptoskit/mobile/assets")
		);
		// Beside the executable, `.leptoskit/app/<name>`.
		assert_eq!(
			assets_destination(root, Target::Linux),
			root.join(".leptoskit/app/assets")
		);
		assert_eq!(
			assets_destination(root, Target::Harmonyos),
			root.join("hosts/harmonyos/entry/src/main/resources/rawfile/assets")
		);
	}

	#[test]
	fn desktop_bundles_raw_wasm_and_mobile_only_the_verified_container() {
		// Generated paths refuse symlinks, and the macOS temporary directory is
		// one.
		let root = std::env::temp_dir()
			.canonicalize()
			.unwrap()
			.join(format!("leptoskit-assets-{}", std::process::id()));
		let bundle = root.join(".leptoskit/bundle");
		std::fs::create_dir_all(&bundle).unwrap();
		let wasm = b"\0asm\x01\0\0\0";
		std::fs::write(bundle.join("app.wasm"), wasm).unwrap();
		std::fs::write(bundle.join("runtime.json"), b"{}").unwrap();
		let mut config = LeptoskitConfig::default();
		config.app.name = "demo".into();

		config.codepush.enabled = true;
		config.codepush.allow_http = true;

		let android_contract = registry::descriptor(&config, Target::Android, &[], &[]);
		let macos_contract = registry::descriptor(&config, Target::Macos, &[], &[]);
		write_assets(
			&root,
			&config,
			Target::Android,
			&bundle,
			true,
			&android_contract,
		)
		.unwrap();
		let mobile = root.join(".leptoskit/mobile");
		let container = std::fs::read(mobile.join("app.lkb")).unwrap();
		assert_eq!(
			leptoskit_codepush::installed::entry_module(&container).unwrap(),
			wasm
		);
		let updates: serde_json::Value =
			serde_json::from_slice(&std::fs::read(mobile.join("codepush.json")).unwrap()).unwrap();

		assert_eq!(updates["enabled"], true);
		assert_eq!(updates["app"], "demo");

		assert!(updates.get("provenance").is_none(), "no policy, no table");

		assert_eq!(updates["channel"], "production");
		assert_eq!(
			updates["allow_http"], false,
			"release builds never allow HTTP"
		);
		assert!(!root.join(".leptoskit/assets/app.wasm").exists());

		write_assets(
			&root,
			&config,
			Target::Macos,
			&bundle,
			false,
			&macos_contract,
		)
		.unwrap();
		assert_eq!(
			std::fs::read(root.join(".leptoskit/assets/app.wasm")).unwrap(),
			wasm
		);

		// A provenance policy ships with the store build, in the host's own
		// update configuration.
		config.codepush.provenance = Some(leptoskit_config::CodePushProvenance {
			repository: "https://github.com/example/demo".into(),
			approvers: vec!["b".repeat(64)],
			threshold: 1,
		});
		write_assets(
			&root,
			&config,
			Target::Android,
			&bundle,
			true,
			&android_contract,
		)
		.unwrap();
		let updates: leptoskit_codepush::installed::UpdateConfiguration =
			serde_json::from_slice(&std::fs::read(mobile.join("codepush.json")).unwrap()).unwrap();
		assert_eq!(
			updates.provenance,
			Some(leptoskit_codepush::ProvenanceConfiguration {
				repository: "https://github.com/example/demo".into(),
				approvers: vec!["b".repeat(64)],
				threshold: 1,
			})
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn gradle_ndk_paths_preserve_spaces_and_normalize_windows_prefixes() {
		assert_eq!(
			gradle_ndk_path(Path::new("/opt/Android NDK")).unwrap(),
			"/opt/Android NDK"
		);
		assert_eq!(
			gradle_ndk_path(Path::new(r"\\?\C:\Android NDK")).unwrap(),
			r"C:\Android NDK"
		);
		assert_eq!(
			gradle_ndk_path(Path::new(r"\\?\UNC\server\Android NDK")).unwrap(),
			r"\\server\Android NDK"
		);
	}

	/// The values a command sets in its environment, by name.
	fn environment(command: &Command) -> Vec<(String, String)> {
		command
			.get_envs()
			.filter_map(|(key, value)| {
				Some((
					key.to_string_lossy().into_owned(),
					value?.to_string_lossy().into_owned(),
				))
			})
			.collect()
	}

	/// A command's arguments as strings.
	fn arguments(command: &Command) -> Vec<String> {
		command
			.get_args()
			.map(|argument| argument.to_string_lossy().into_owned())
			.collect()
	}

	#[test]
	fn android_hosts_link_and_compile_with_the_ndk_clang() {
		// Generated files refuse symlinked paths, and macOS links its temp dir.
		let root = std::env::temp_dir()
			.canonicalize()
			.unwrap()
			.join(format!("leptoskit-ndk-{}", std::process::id()));
		let ndk = root.join("ndk");
		let tools = ndk.join(format!("toolchains/llvm/prebuilt/{NDK_HOST}/bin"));
		std::fs::create_dir_all(&tools).unwrap();
		std::fs::create_dir_all(root.join(".leptoskit/android")).unwrap();
		let clang = tools.join(format!(
			"aarch64-linux-android26-clang{NDK_LAUNCHER_SUFFIX}"
		));
		std::fs::write(&clang, "").unwrap();

		let mut host = Command::new("cargo");
		configure_android_toolchain(&mut host, &root, Target::Android, &ndk).unwrap();
		let environment = environment(&host);
		let value = |name: &str| {
			environment
				.iter()
				.find(|(key, _)| key == name)
				.map(|(_, value)| PathBuf::from(value))
		};
		assert_eq!(
			value("CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"),
			Some(clang.clone())
		);
		assert_eq!(value("CC_aarch64_linux_android"), Some(clang));
		assert_eq!(
			value("CXX_aarch64_linux_android"),
			Some(tools.join(format!(
				"aarch64-linux-android26-clang++{NDK_LAUNCHER_SUFFIX}"
			)))
		);
		assert!(
			value("AR_aarch64_linux_android").is_some_and(|archiver| archiver.starts_with(&tools)),
			"the NDK's llvm-ar archives: {environment:?}"
		);

		assert_eq!(
			std::fs::read_to_string(root.join(".leptoskit/android/ndk-path.txt")).unwrap(),
			gradle_ndk_path(&ndk).unwrap(),
			"Gradle reads the same NDK"
		);

		// This NDK has no compiler for the x86_64 emulator.
		let error = configure_android_toolchain(
			&mut Command::new("cargo"),
			&root,
			Target::AndroidX86_64,
			&ndk,
		)
		.unwrap_err();
		assert!(
			error.to_string().contains("does not contain a compiler"),
			"{error}"
		);
		std::fs::remove_dir_all(&root).unwrap();
	}

	#[test]
	fn android_hosts_assemble_with_the_gradle_wrapper() {
		let root = Path::new("project");
		let android = host_dir(root, Target::Android);
		let release = gradle_build(root, Target::Android, true, None);
		let wrapper = Path::new(release.get_program());
		assert_eq!(wrapper.parent(), Some(android.as_path()));
		assert_eq!(wrapper.file_stem(), Some("gradlew".as_ref()));
		assert_eq!(release.get_current_dir(), Some(android.as_path()));
		assert_eq!(
			arguments(&release),
			[
				"--no-daemon",
				"--console=plain",
				"--quiet",
				"assembleRelease",
				"bundleRelease"
			]
		);

		let debug = gradle_build(root, Target::AndroidX86_64, false, None);
		assert_eq!(
			arguments(&debug),
			["--no-daemon", "--console=plain", "--quiet", "assembleDebug"]
		);
	}

	#[test]
	fn only_development_hosts_with_an_agent_compile_devtools_in() {
		let registry = registry::Registry {
			package: "app_macos".into(),
			plugins: Vec::new(),
			support: Vec::new(),
			bridge: PathBuf::from("bridge"),
			target: PathBuf::from("target"),
			host_features: vec!["leptoskit_bridge_ffi/async-host".into()],
			devtools_feature: Some("leptoskit_bridge_ffi/devtools".into()),
			missing: Vec::new(),
			scene3d: false,
		};
		let features =
			|target, release, devtools| host_features(&registry, target, release, devtools);
		assert_eq!(
			features(Target::Macos, false, true),
			[
				"leptoskit_bridge_ffi/async-host",
				"leptoskit_bridge_ffi/devtools"
			]
		);

		for target in [
			Target::IosSimulator,
			Target::Ios,
			Target::Android,
			Target::AndroidX86_64,
		] {
			assert!(
				features(target, false, true).contains(&"leptoskit_bridge_ffi/devtools".into())
			);
		}

		assert_eq!(
			features(Target::Macos, true, true),
			["leptoskit_bridge_ffi/async-host"],
			"release builds never report"
		);
		assert_eq!(
			features(Target::Macos, false, false),
			["leptoskit_bridge_ffi/async-host"],
			"only `leptoskit dev` runs a hub"
		);
		assert_eq!(
			features(Target::Harmonyos, false, true),
			["leptoskit_bridge_ffi/async-host"],
			"the ArkUI host has no agent yet"
		);
	}

	#[test]
	fn ios_hosts_build_for_their_sdk_and_sign_only_for_the_simulator() {
		let root = Path::new("project");
		let pair = |arguments: &[String], flag: &str| {
			arguments
				.windows(2)
				.find(|pair| pair[0] == flag)
				.map(|pair| pair[1].clone())
		};

		let simulator = ios_build(root, Target::IosSimulator, false, None);
		assert_eq!(simulator.get_program(), "xcodebuild");
		assert_eq!(simulator.get_current_dir(), Some(root));
		let simulator = arguments(&simulator);
		assert_eq!(
			pair(&simulator, "-destination").as_deref(),
			Some("generic/platform=iOS Simulator")
		);
		assert_eq!(pair(&simulator, "-configuration").as_deref(), Some("Debug"));
		assert!(
			simulator.ends_with(&[
				"build".to_owned(),
				"CODE_SIGNING_ALLOWED=YES".to_owned(),
				"CODE_SIGN_IDENTITY=-".to_owned(),
			]),
			"ad-hoc signed for the simulator's keychain: {simulator:?}"
		);

		let device = arguments(&ios_build(root, Target::Ios, true, None));
		assert_eq!(
			pair(&device, "-destination").as_deref(),
			Some("generic/platform=iOS")
		);
		assert_eq!(pair(&device, "-configuration").as_deref(), Some("Release"));
		assert!(device.contains(&"ARCHS=arm64".to_owned()));
		assert!(device.ends_with(&["build".to_owned(), "CODE_SIGNING_ALLOWED=NO".to_owned()]));
	}

	#[test]
	fn ios_builds_stay_quiet_and_use_the_pods_workspace_only_with_pods() {
		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		std::fs::create_dir_all(root.join(".leptoskit/ios")).unwrap();

		let plain = arguments(&ios_build(root, Target::IosSimulator, false, None));
		assert!(plain.contains(&"-quiet".to_owned()));
		assert!(plain.contains(&"-project".to_owned()));
		assert!(!plain.contains(&"-workspace".to_owned()));

		std::fs::write(root.join(".leptoskit/ios/Podfile"), "# pods").unwrap();
		let pods = arguments(&ios_build(root, Target::IosSimulator, false, None));
		assert!(pods.contains(&"-quiet".to_owned()));
		assert!(pods.contains(&"-workspace".to_owned()));
		assert!(!pods.contains(&"-project".to_owned()));
	}

	#[test]
	fn every_target_names_its_platform_and_rust_triple() {
		for (target, platform) in [
			(Target::Macos, Platform::Macos),
			(Target::IosSimulator, Platform::Ios),
			(Target::Ios, Platform::Ios),
			(Target::Android, Platform::Android),
			(Target::AndroidX86_64, Platform::Android),
			(Target::Harmonyos, Platform::Harmonyos),
			(Target::HarmonyosX86_64, Platform::Harmonyos),
			(Target::Windows, Platform::Windows),
			(Target::Linux, Platform::Linux),
			(Target::Tui, Platform::Tui),
		] {
			assert_eq!(target.platform(), platform);
			assert!(!target.rust().is_empty(), "{target:?}");
		}

		assert_eq!(Target::from_platform(Platform::Ios).unwrap(), Target::Ios);
		assert_eq!(
			Target::from_platform(Platform::Android).unwrap(),
			Target::Android
		);
		assert_eq!(
			Target::from_platform(Platform::Harmonyos).unwrap(),
			Target::Harmonyos
		);
		assert_eq!(
			Target::from_platform(Platform::Macos).unwrap(),
			Target::Macos
		);
		assert_eq!(
			Target::from_platform(Platform::Windows).unwrap(),
			Target::Windows
		);
		assert_eq!(
			Target::from_platform(Platform::Linux).unwrap(),
			Target::Linux
		);
		assert_eq!(Target::from_platform(Platform::Tui).unwrap(), Target::Tui);

		assert_eq!(Target::Android.rust(), "aarch64-linux-android");
		assert_eq!(Target::AndroidX86_64.rust(), "x86_64-linux-android");
		assert_eq!(Target::Harmonyos.rust(), "aarch64-unknown-linux-ohos");
		assert_eq!(Target::HarmonyosX86_64.rust(), "x86_64-unknown-linux-ohos");
		assert_eq!(Target::Ios.rust(), "aarch64-apple-ios");
		assert_eq!(Target::Tui.rust(), host_triple());

		for platform in [
			Platform::Web,
			Platform::Server,
			Platform::Extension,
			Platform::Native,
		] {
			assert!(
				Target::from_platform(platform).is_err(),
				"{platform} is not a concrete native platform"
			);
		}

		assert!(Target::Tui.builds_on_host());
		assert!(!Target::Macos.builds_on_host());
		for target in [
			Target::Macos,
			Target::Ios,
			Target::IosSimulator,
			Target::Android,
			Target::AndroidX86_64,
		] {
			assert!(target.reports_to_devtools(), "{target:?}");
		}

		for target in [
			Target::Windows,
			Target::Linux,
			Target::Harmonyos,
			Target::HarmonyosX86_64,
			Target::Tui,
		] {
			assert!(!target.reports_to_devtools(), "{target:?}");
		}
	}

	#[test]
	fn host_and_code_push_rules_are_checked_before_anything_builds() {
		let mut config = LeptoskitConfig::default();
		config.app.platforms = vec![Platform::Web];
		let error = check_host(&config, Target::Android, false)
			.unwrap_err()
			.to_string();
		assert!(error.contains("android"), "{error}");

		config.app.platforms = Platform::APPLICATION.to_vec();
		config.codepush.enabled = true;
		config.codepush.server = "http://push.example.com".into();
		let error = check_host(&config, Target::Android, false)
			.unwrap_err()
			.to_string();
		assert!(error.contains("requires HTTPS"), "{error}");
		let error = check_host(&config, Target::Android, true)
			.unwrap_err()
			.to_string();
		assert!(error.contains("requires HTTPS"), "{error}");

		// A debug build may opt into cleartext, a release build never does.
		config.codepush.allow_http = true;
		assert!(
			check_host(&config, Target::Android, false)
				.unwrap()
				.is_none()
		);
		assert!(check_host(&config, Target::Android, true).is_err());

		config.codepush.enabled = false;

		for (target, supported) in [
			(Target::Macos, cfg!(target_os = "macos")),
			(Target::Ios, cfg!(target_os = "macos")),
			(Target::IosSimulator, cfg!(target_os = "macos")),
			(Target::Windows, cfg!(target_os = "windows")),
			(Target::Linux, cfg!(target_os = "linux")),
		] {
			assert_eq!(
				check_host(&config, target, true).is_ok(),
				supported,
				"{target:?} on this host"
			);
		}
	}

	fn host_registry() -> registry::Registry {
		registry::Registry {
			package: "app_host".into(),
			plugins: Vec::new(),
			support: Vec::new(),
			bridge: PathBuf::from("bridge"),
			target: PathBuf::from("target"),
			host_features: vec!["leptoskit_bridge_ffi/async-host".into()],
			devtools_feature: Some("leptoskit_bridge_ffi/devtools".into()),
			missing: Vec::new(),
			scene3d: false,
		}
	}

	#[test]
	fn host_library_builds_take_the_profile_features_and_ohos_toolchain() {
		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		let mut config = LeptoskitConfig::default();
		config.bundle.optimize = Optimize::Smallest;
		let build_dir = root.join("target");
		let registry = host_registry();

		let debug = host_library_command(
			root,
			&config,
			&registry,
			Target::Macos,
			false,
			&build_dir,
			None,
		)
		.unwrap();
		let debug_arguments = arguments(&debug);
		assert!(debug_arguments.contains(&"build".to_owned()));
		assert!(debug_arguments.contains(&"app_host".to_owned()));
		assert!(debug_arguments.contains(&"leptoskit_bridge_ffi/async-host".to_owned()));
		assert!(
			debug_arguments
				.iter()
				.any(|argument| argument == "profile.dev.package.\"*\".opt-level=3"),
			"{debug_arguments:?}"
		);

		let release = host_library_command(
			root,
			&config,
			&registry,
			Target::Macos,
			true,
			&build_dir,
			None,
		)
		.unwrap();
		let release_arguments = arguments(&release);
		assert!(release_arguments.contains(&crate::wasm::HOST_SMALLEST_PROFILE.to_owned()));
		assert!(
			!release_arguments
				.iter()
				.any(|argument| argument.contains("opt-level=3")),
			"{release_arguments:?}"
		);
		#[cfg(target_os = "macos")]
		assert!(
			environment(&release).iter().any(|(key, _)| {
				key == "CC_aarch64_apple_darwin" || key == "CC_x86_64_apple_darwin"
			}),
			"the SDK compiler reaches cc-rs: {:?}",
			environment(&release)
		);

		let ohos = root.join("ohos-ndk");
		let harmony = host_library_command(
			root,
			&config,
			&registry,
			Target::Harmonyos,
			false,
			&build_dir,
			Some(&ohos),
		)
		.unwrap();
		let harmony_environment = environment(&harmony);
		assert!(
			harmony_environment.iter().any(|(key, value)| {
				key == "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_OHOS_LINKER"
					&& value.starts_with(&ohos.display().to_string())
			}),
			"{harmony_environment:?}"
		);
	}

	#[test]
	fn host_library_builds_carry_the_configured_log_level() {
		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		let build_dir = root.join("target");
		let registry = host_registry();
		let level = |config: &LeptoskitConfig| {
			let host = host_library_command(
				root,
				config,
				&registry,
				Target::Macos,
				false,
				&build_dir,
				None,
			)
			.unwrap();
			environment(&host)
				.into_iter()
				.find(|(key, _)| key == leptoskit_config::LogLevel::VARIABLE)
				.map(|(_, value)| value)
		};

		// Unset, the runtime picks the level from the build.
		assert_eq!(level(&LeptoskitConfig::default()), None);

		let mut config = LeptoskitConfig::default();
		config.native.log_level = Some(leptoskit_config::LogLevel::Debug);
		assert_eq!(level(&config).as_deref(), Some("debug"));
	}

	#[test]
	fn tools_report_whether_they_ran() {
		#[cfg(unix)]
		{
			let mut succeeded = Command::new("sh");
			succeeded.args(["-c", "exit 0"]);
			run_tool(&mut succeeded).unwrap();

			let mut failed = Command::new("sh");
			failed.args(["-c", "exit 7"]);
			let error = run_tool(&mut failed).unwrap_err().to_string();
			assert!(error.contains("failed"), "{error}");
		}

		let mut missing = Command::new("leptoskit-tool-that-does-not-exist");
		let error = run_tool(&mut missing).unwrap_err().to_string();
		assert!(error.contains("could not execute"), "{error}");
	}

	#[test]
	fn rustcs_note_names_the_archives_libraries_once_in_its_order() {
		let output = "   Compiling app v0.1.0\nnote: link against the following native \
		              artifacts\n\nnote: native-static-libs: -ldl -llog -lunwind -ldl -lm -lc \
		              -laaudio -lOpenSLES\n\n    Finished `dev` profile [unoptimized + debuginfo] \
		              target(s) in 0.42s\n";

		assert_eq!(
			native_static_libs(output),
			["dl", "log", "unwind", "m", "c", "aaudio", "OpenSLES"]
		);
		assert_eq!(
			native_static_libs("nothing was printed"),
			Vec::<String>::new()
		);
		assert_eq!(
			native_static_libs("note: native-static-libs: -framework Security -lAudioToolbox"),
			["AudioToolbox"],
			"a framework is the C linker's to name, not a library to link"
		);
	}

	#[cfg(unix)]
	#[test]
	fn a_host_build_reports_the_libraries_it_printed() {
		let mut build = Command::new("sh");
		build.args([
			"-c",
			"echo 'note: native-static-libs: -laaudio -ldl -laaudio' >&2; echo compiling",
		]);
		assert_eq!(run_host_library(&mut build).unwrap(), ["aaudio", "dl"]);

		let mut failed = Command::new("sh");
		failed.args(["-c", "exit 3"]);
		let error = run_host_library(&mut failed).unwrap_err().to_string();
		assert!(error.contains("failed"), "{error}");
	}

	#[test]
	fn the_built_guest_is_read_back_or_names_the_missing_bundle() {
		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		let bundle = generated_dir(root, Target::Android).join("bundle");
		std::fs::create_dir_all(&bundle).unwrap();
		std::fs::write(bundle.join("app.wasm"), b"\0asm").unwrap();
		assert_eq!(built_guest(root, Target::Android).unwrap(), b"\0asm");

		std::fs::remove_file(bundle.join("app.wasm")).unwrap();
		let error = built_guest(root, Target::Android).unwrap_err().to_string();
		assert!(error.contains("could not read"), "{error}");
	}

	#[test]
	fn legacy_projects_infer_their_single_destination() {
		use leptoskit_config::NativeTarget;

		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		let mut config = LeptoskitConfig::default();

		for (native, target) in [
			(NativeTarget::Harmonyos, Target::Harmonyos),
			(NativeTarget::Macos, Target::Macos),
			(NativeTarget::Ios, Target::IosSimulator),
			(NativeTarget::Android, Target::Android),
			(NativeTarget::Windows, Target::Windows),
			(NativeTarget::Linux, Target::Linux),
		] {
			config.native.targets = vec![native];
			assert_eq!(legacy_target(root, &config), target);
		}

		config.native.targets = Vec::new();
		let inferred = legacy_target(root, &config);

		if cfg!(target_os = "macos") {
			assert_eq!(inferred, Target::IosSimulator);
			std::fs::create_dir_all(root.join("macos")).unwrap();
			std::fs::write(root.join("macos/Cargo.toml"), "").unwrap();
			assert_eq!(legacy_target(root, &config), Target::Macos);
		} else {
			assert_eq!(inferred, Target::Android);
		}
	}

	#[test]
	fn bundling_refuses_an_empty_update_contract() {
		let temporary = tempfile::tempdir().unwrap();
		let mut config = LeptoskitConfig::default();
		config.native.update_contract = String::new();
		let error = bundle(
			temporary.path(),
			&config,
			&[],
			&[],
			Target::Macos,
			false,
			None,
			&temporary.path().join("bundle"),
		)
		.unwrap_err()
		.to_string();
		assert!(error.contains("update_contract cannot be empty"), "{error}");
	}

	#[test]
	fn portable_projects_must_name_the_client_package() {
		let temporary = tempfile::tempdir().unwrap();
		let root = temporary.path();
		std::fs::write(
			root.join("Cargo.toml"),
			"[workspace]\nmembers = [\"app\"]\nresolver = \"2\"\n",
		)
		.unwrap();
		std::fs::create_dir_all(root.join("app/src")).unwrap();
		std::fs::write(
			root.join("app/Cargo.toml"),
			"[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
		)
		.unwrap();
		std::fs::write(root.join("app/src/lib.rs"), "").unwrap();
		std::fs::create_dir_all(root.join("macos")).unwrap();
		std::fs::write(root.join("macos/Cargo.toml"), "").unwrap();

		let error = bundle(
			root,
			&LeptoskitConfig::default(),
			&[],
			&[],
			Target::Macos,
			false,
			None,
			&root.join("bundle"),
		)
		.unwrap_err()
		.to_string();
		assert!(
			error.contains("portable native builds require build.client_package"),
			"{error}"
		);
	}

	#[test]
	fn a_missing_podfile_needs_no_cocoapods() {
		let temporary = tempfile::tempdir().unwrap();
		install_ios_pods(temporary.path()).unwrap();
	}

	/// Runs one native-host situation in a child test process, so `cwd`,
	/// `PATH`, and the Android variables are the scenario's alone.
	#[cfg(unix)]
	mod child {
		use std::os::unix::fs::PermissionsExt as _;
		use std::path::PathBuf;
		use std::process::Command;
		use std::process::Output;

		use super::*;
		use crate::native::NativeCommand;
		use crate::native::run;

		const HELPER: &str = "native::tests::child::helper";
		const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

		fn pod_script() -> &'static str {
			r#"#!/bin/sh
set -eu
exit "${LEPTOSKIT_TEST_POD_STATUS:-0}"
"#
		}

		pub(super) struct Fixture {
			pub(super) directory: tempfile::TempDir,
		}

		impl Fixture {
			pub(super) fn new() -> Self {
				let directory = tempfile::tempdir().unwrap();
				let bin = directory.path().join("bin");
				std::fs::create_dir(&bin).unwrap();
				let pod = bin.join("pod");
				std::fs::write(&pod, pod_script()).unwrap();
				std::fs::set_permissions(&pod, std::fs::Permissions::from_mode(0o755)).unwrap();
				Self { directory }
			}

			pub(super) fn project(&self) -> PathBuf {
				let project = self.directory.path().join("project");
				std::fs::create_dir_all(&project).unwrap();
				std::fs::write(
					project.join("leptoskit.toml"),
					"schema_version = 0\n\n[app]\nname = \"app\"\nversion = \"0.1.0\"\nplatforms \
					 = [\"web\"]\n",
				)
				.unwrap();
				project
			}

			pub(super) fn root(&self) -> PathBuf {
				self.directory.path().join("app")
			}

			pub(super) fn empty_path(&self) -> PathBuf {
				let empty = self.directory.path().join("empty");
				std::fs::create_dir_all(&empty).unwrap();
				empty
			}

			fn run(&self, scenario: &str, environment: &[(&str, &str)]) -> Output {
				let mut command = Command::new(std::env::current_exe().unwrap());
				command
					.args(["--exact", HELPER, "--nocapture", "--test-threads=1"])
					.env_clear()
					.env("PATH", SYSTEM_PATH)
					.env("TMPDIR", self.directory.path())
					.env("LEPTOSKIT_TEST_NATIVE", scenario)
					.current_dir(self.project());

				for (name, value) in environment {
					command.env(name, value);
				}

				if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
					command.env("LLVM_PROFILE_FILE", profile);
				}

				command.output().unwrap()
			}

			pub(super) fn run_ok(&self, scenario: &str, environment: &[(&str, &str)]) {
				let output = self.run(scenario, environment);
				assert!(
					output.status.success(),
					"{scenario} failed\nstdout:\n{}\nstderr:\n{}",
					String::from_utf8_lossy(&output.stdout),
					String::from_utf8_lossy(&output.stderr),
				);
			}
		}

		fn expect<T>(result: Result<T>, expected: &str) {
			let Err(error) = result else {
				panic!("the scenario should have failed");
			};
			assert!(
				format!("{error:#}").contains(expected),
				"expected {expected:?} in {error:#}"
			);
		}

		#[test]
		fn helper() {
			let Ok(scenario) = std::env::var("LEPTOSKIT_TEST_NATIVE") else {
				return;
			};
			let root = std::env::var_os("LEPTOSKIT_TEST_ROOT")
				.map_or_else(|| std::env::current_dir().unwrap(), PathBuf::from);
			let pod_status = std::env::var("LEPTOSKIT_TEST_POD_STATUS").unwrap_or_default();

			match scenario.as_str() {
				"bundle-default" => {
					expect(
						run(NativeCommand::Bundle {
							out: "dist".into(),
							target: None,
						}),
						"native bundle requires --target",
					);
				}
				"prepare-disabled" => {
					expect(
						run(NativeCommand::Prepare {
							target: Target::Linux,
							release: false,
						}),
						"linux",
					);
				}
				"build-disabled" => {
					expect(
						run(NativeCommand::Build {
							target: Target::Macos,
							release: true,
							options: signing::ReleaseOptions::default(),
						}),
						"macos",
					);
				}
				"pods" => {
					let result = install_ios_pods(&root);
					match pod_status.as_str() {
						"0" => result.expect("a present pod installs the Podfile"),
						"1" => expect(result, "`pod install` failed"),
						other => panic!("unknown pod status {other}"),
					}
				}
				"pods-missing" => expect(install_ios_pods(&root), "CocoaPods is required"),
				"ndk-missing" => expect(android_ndk(), "set ANDROID_HOME"),
				"ndk-invalid" => expect(android_ndk(), "missing source.properties"),
				"ndk-unreadable" => expect(android_ndk(), "No such file or directory"),
				"ndk-newest" => {
					let ndk = android_ndk().unwrap();
					assert!(
						ndk.ends_with("ndk/27.1.1"),
						"the newest NDK is used: {}",
						ndk.display()
					);
				}
				other => panic!("unknown native scenario {other}"),
			}
		}
	}

	#[cfg(unix)]
	#[test]
	fn native_commands_refuse_disabled_destinations_before_building() {
		let fixture = child::Fixture::new();
		fixture.run_ok("bundle-default", &[]);
		fixture.run_ok("prepare-disabled", &[]);
		fixture.run_ok("build-disabled", &[]);
	}

	#[cfg(unix)]
	#[test]
	fn pod_install_runs_only_with_a_podfile_and_needs_the_tool() {
		let fixture = child::Fixture::new();
		std::fs::create_dir_all(fixture.root().join(".leptoskit/ios")).unwrap();
		std::fs::write(fixture.root().join(".leptoskit/ios/Podfile"), "# pods").unwrap();
		let root = fixture.root().display().to_string();
		let path = format!(
			"{}:/usr/bin:/bin:/usr/sbin:/sbin",
			fixture.directory.path().join("bin").display()
		);

		fixture.run_ok(
			"pods",
			&[
				("LEPTOSKIT_TEST_ROOT", &root),
				("LEPTOSKIT_TEST_POD_STATUS", "0"),
				("PATH", &path),
			],
		);
		fixture.run_ok(
			"pods",
			&[
				("LEPTOSKIT_TEST_ROOT", &root),
				("LEPTOSKIT_TEST_POD_STATUS", "1"),
				("PATH", &path),
			],
		);
		fixture.run_ok(
			"pods-missing",
			&[
				("LEPTOSKIT_TEST_ROOT", &root),
				("PATH", &fixture.empty_path().display().to_string()),
			],
		);
	}

	#[cfg(unix)]
	#[test]
	fn the_android_ndk_comes_from_the_variable_or_the_newest_sdk_installation() {
		let fixture = child::Fixture::new();
		fixture.run_ok("ndk-missing", &[]);
		fixture.run_ok(
			"ndk-unreadable",
			&[(
				"ANDROID_HOME",
				&fixture
					.directory
					.path()
					.join("absent-sdk")
					.display()
					.to_string(),
			)],
		);
		let invalid = fixture.directory.path().join("not-an-ndk");
		std::fs::create_dir_all(&invalid).unwrap();
		fixture.run_ok(
			"ndk-invalid",
			&[("ANDROID_NDK_HOME", &invalid.display().to_string())],
		);

		let sdk = fixture.directory.path().join("sdk");

		for version in ["25.0.0", "27.1.1"] {
			let ndk = sdk.join("ndk").join(version);
			std::fs::create_dir_all(&ndk).unwrap();
			std::fs::write(ndk.join("source.properties"), "Pkg.Revision = 1").unwrap();
		}

		// A directory without an installation manifest is not an NDK.
		std::fs::create_dir_all(sdk.join("ndk/28.0.0")).unwrap();
		fixture.run_ok(
			"ndk-newest",
			&[("ANDROID_HOME", &sdk.display().to_string())],
		);
	}

	#[test]
	fn quest_builds_the_android_host_for_the_headset() {
		assert_eq!(Target::Quest.rust(), "aarch64-linux-android");
		assert_eq!(Target::Quest.platform(), Platform::Quest);
		assert_eq!(
			Target::from_platform(Platform::Quest).unwrap(),
			Target::Quest
		);
		assert!(Target::Quest.is_android());
		assert!(Target::Quest.reports_to_devtools());
		let root = Path::new("/project");
		assert_eq!(
			host_dir(root, Target::Quest),
			host_dir(root, Target::Android)
		);
		assert_eq!(Platform::Quest.feature(), Some("android"));

		let mut config = LeptoskitConfig::default();
		config.native.targets = vec![leptoskit_config::NativeTarget::Quest];
		assert_eq!(legacy_target(root, &config), Target::Quest);
	}

	#[test]
	fn watch_targets_map_triples_platforms_and_legacy_layouts() {
		assert_eq!(Target::Wearos.rust(), "aarch64-linux-android");
		assert_eq!(Target::Watchos.rust(), "aarch64-apple-watchos");
		assert_eq!(Target::Wearos.platform(), Platform::Wearos);
		assert_eq!(Target::Watchos.platform(), Platform::Watchos);
		assert_eq!(
			Target::from_platform(Platform::Wearos).unwrap(),
			Target::Wearos
		);
		assert_eq!(
			Target::from_platform(Platform::Watchos).unwrap(),
			Target::Watchos
		);

		let root =
			std::env::temp_dir().join(format!("leptoskit-legacy-watch-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).unwrap();
		let mut config = LeptoskitConfig::default();
		config.native.targets = vec![leptoskit_config::NativeTarget::Wearos];
		assert_eq!(legacy_target(&root, &config), Target::Wearos);
		config.native.targets = vec![leptoskit_config::NativeTarget::Watchos];

		assert_eq!(legacy_target(&root, &config), Target::Watchos);
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn watch_prepare_and_build_use_the_experimental_host_path() {
		let previous = std::env::current_dir().unwrap();
		for (native, target) in [
			(leptoskit_config::NativeTarget::Wearos, Target::Wearos),
			(leptoskit_config::NativeTarget::Watchos, Target::Watchos),
		] {
			let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
				"leptoskit-watch-run-{}-{}",
				target.platform(),
				std::process::id()
			));
			let _ = std::fs::remove_dir_all(&root);
			scaffold::run_for_target(root.to_str().unwrap(), Some("steps"), None, Some(native))
				.unwrap();
			std::env::set_current_dir(&root).unwrap();
			run(NativeCommand::Prepare {
				target,
				release: false,
			})
			.unwrap();
			assert!(
				root.join(format!("hosts/{}/guest.rs", target.platform()))
					.is_file()
			);
			let build_error = run(NativeCommand::Build {
				target,
				release: false,
				options: signing::ReleaseOptions::default(),
			})
			.unwrap_err()
			.to_string();
			assert!(build_error.contains("experimental"), "{build_error}");
			let packaging =
				build_application(&root, &LeptoskitConfig::default(), target, false, None)
					.unwrap_err()
					.to_string();
			assert!(packaging.contains("experimental"), "{packaging}");
			std::env::set_current_dir(&previous).unwrap();
			let _ = std::fs::remove_dir_all(&root);
		}
	}
}
