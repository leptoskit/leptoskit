//! Local macOS development bundles; `leptoskit publish macos` signs them
//! for distribution.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LeptoskitConfig;

use super::Target;
use super::project::copy;
use super::project::copy_package;
use super::project::write;
use super::registry::Registry;

/// The environment variable naming the certificate a push-enabled local
/// build is signed with, as `security find-identity -v -p codesigning`
/// lists it.
pub const IDENTITY: &str = "LEPTOSKIT_MACOS_IDENTITY";

/// Where a macOS bundle carries its provisioning profile.
const EMBEDDED_PROFILE: &str = "Contents/embedded.provisionprofile";

pub fn generate(root: &Path, config: &LeptoskitConfig, registry: &Registry) -> Result<()> {
	// Checked before any write, so a refused configuration leaves the
	// generated project exactly as it was.
	provisioning_profile(root, config)?;
	let directory = root.join(".leptoskit/macos");
	let packages = directory.join("packages");
	copy_package(&registry.bridge, &packages.join("leptoskit_bridge_ffi"))?;
	let mut dependencies = String::from(".package(path: \"packages/leptoskit_bridge_ffi\"),\n");
	let mut products =
		String::from(".product(name: \"LeptosKitBridge\", package: \"leptoskit_bridge_ffi\"),\n");

	support_packages(registry, &packages, &mut dependencies, &mut products)?;
	let mut registrant = String::from("import CLeptosKitBridge\nimport LeptosKitBridge\n");
	let mut registrations = String::new();

	plugin_packages(
		registry,
		&packages,
		&mut dependencies,
		&mut products,
		&mut registrant,
		&mut registrations,
	)?;
	writeln!(
		registrant,
		"@MainActor\nenum GeneratedPlugins {{\n    static func register(\n        _ bridge: \
		 OpaquePointer,\n        _ dispatcher: NativeDispatcher,\n        _ context: \
		 NativePluginContext\n    ) throws {{\n{registrations}    }}\n}}"
	)?;
	write(
		&directory.join("Sources/GeneratedPlugins.swift"),
		&registrant,
	)?;
	let app_source = super::host_dir(root, Target::Macos).join("App.swift");
	let app = std::fs::read_to_string(&app_source)?;
	// App-owned custom hosts keep their lifecycle. The stock host gains a
	// status item with a lazy window; it starts no guest interpreter at launch.
	let app = if config.native.mode == leptoskit_config::application::NativeMode::MenuBar
		&& app == include_str!("templates/MacApp.swift")
	{
		include_str!("templates/MacMenuBarApp.swift")
	} else {
		&app
	};
	write(&directory.join("Sources/App.swift"), app)?;
	write_macos_entitlements(root, config)?;
	let mut sources = vec!["App.swift".to_owned(), "GeneratedPlugins.swift".to_owned()];

	for source in super::project::platform_sources::MACOS {
		let filename = Path::new(source)
			.file_name()
			.context("native source filename missing")?;
		copy(
			&registry.bridge.join(source),
			&directory.join("Sources").join(filename),
		)?;
		sources.push(
			filename
				.to_str()
				.context("native source filename is not UTF-8")?
				.to_owned(),
		);
	}

	let sources = sources
		.iter()
		.map(|source| format!("{source:?}"))
		.collect::<Vec<_>>()
		.join(",\n");
	write(
		&directory.join("Package.swift"),
		&format!(
			"// swift-tools-version:6.0\nimport PackageDescription\nlet package = Package(\nname: \
			 \"App\", platforms: [.macOS(.v14)],\ndependencies: [{dependencies}],\ntargets: \
			 [.executableTarget(\nname: \"App\",\ndependencies: [{products}],\npath: \
			 \"Sources\",\nsources: [{sources}]\n)])\n"
		),
	)?;

	Ok(())
}

pub fn build(root: &Path, config: &LeptoskitConfig, release: bool, unsigned: bool) -> Result<()> {
	// What signs the bundle is settled before Swift compiles anything, so a
	// missing identity doesn't cost the Swift build first.
	let signing = local_signing(root, config, &super::signing::process_env, unsigned)?;
	let directory = root.join(".leptoskit/macos");
	let profile = if release { "release" } else { "debug" };
	let library = super::generated_dir(root, Target::Macos).join("lib");
	super::run_tool(
		Command::new("swift")
			.current_dir(&directory)
			.args(["build", "-c", profile, "-Xlinker", "-L"])
			.arg("-Xlinker")
			.arg(&library)
			.args(["-Xlinker", "-dead_strip"]),
	)?;
	let output = Command::new("swift")
		.current_dir(&directory)
		.args(["build", "-c", profile, "--show-bin-path"])
		.output()?;
	anyhow::ensure!(
		output.status.success(),
		"could not find the Swift executable"
	);
	let binary = String::from_utf8(output.stdout)?;
	let binary = Path::new(binary.trim()).join("App");

	if release {
		super::run_tool(Command::new("strip").arg(&binary))?;
	}

	let bundle = assemble(root, config, &binary)?;

	if super::safari::configured(config) {
		super::safari::assemble_macos(
			root,
			config,
			&bundle,
			&root.join(crate::extension::OUTPUT).join("safari"),
		)?;
	}

	// The profile goes in before the signature seals the bundle.
	embed_profile(&bundle, signing.profile())?;

	if unsigned && config.native.macos.aps_environment.is_some() {
		crate::ui::warning(
			"warning: this build is ad-hoc signed without the push entitlement, as --unsigned \
			 asks. It launches, and registering for push answers `aps-environment`; a \
			 distribution signs it again with the entitlement and the provisioning profile.",
		);
	}

	let mut signature = Command::new("codesign");
	signature.args(signing.codesign_arguments());
	super::run_tool(signature.arg(&bundle))?;
	super::run_tool(
		Command::new("codesign")
			.args(["--verify", "--strict"])
			.arg(&bundle),
	)?;
	println!("built application at {}", bundle.display());
	Ok(())
}

/// The provisioning profile a push-enabled macOS app embeds: the file
/// `[native.macos] provisioning-profile` names, under the project. `None`
/// for an app without `aps-environment`, which needs none and whose setting
/// is not read.
///
/// # Errors
///
/// When the app is push-enabled and the profile isn't configured or isn't
/// there. Apple's push entitlement is restricted: macOS kills an app that
/// claims it at launch unless a matching profile is embedded.
pub(crate) fn provisioning_profile(
	root: &Path,
	config: &LeptoskitConfig,
) -> Result<Option<std::path::PathBuf>> {
	if config.native.macos.aps_environment.is_none() {
		return Ok(None);
	}

	let configured = config
		.native
		.macos
		.provisioning_profile
		.as_deref()
		.map(str::trim)
		.filter(|path| !path.is_empty())
		.context(
			"native.macos.aps-environment makes this a push-enabled Mac app, and macOS refuses to \
			 launch one without a provisioning profile that grants the push entitlement. Download \
			 the profile for this app's bundle id from your Apple developer account and name it \
			 with `provisioning-profile` under [native.macos]; the build embeds it as \
			 Contents/embedded.provisionprofile",
		)?;
	let path = root.join(configured);
	anyhow::ensure!(
		path.is_file(),
		"native.macos.provisioning-profile names {configured}, and there is no file at {}; a \
		 push-enabled Mac app can't launch without the profile embedded as \
		 Contents/embedded.provisionprofile",
		path.display()
	);

	Ok(Some(path))
}

/// How a local macOS build is signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LocalSigning {
	/// Ad-hoc, with no entitlements: every app without push, and an
	/// `--unsigned` build of one with it.
	AdHoc,
	/// A push-enabled app: a real identity, the push entitlement, and the
	/// provisioning profile that grants it.
	Push {
		identity: String,
		entitlements: std::path::PathBuf,
		profile: std::path::PathBuf,
	},
}

impl LocalSigning {
	/// The provisioning profile the bundle embeds, when it embeds one.
	fn profile(&self) -> Option<&Path> {
		match self {
			Self::AdHoc => None,
			Self::Push { profile, .. } => Some(profile),
		}
	}

	/// `codesign`'s arguments, up to the bundle's path.
	fn codesign_arguments(&self) -> Vec<std::ffi::OsString> {
		let mut arguments: Vec<std::ffi::OsString> = vec!["--force".into(), "--sign".into()];

		match self {
			Self::AdHoc => arguments.push("-".into()),
			Self::Push { identity, .. } => arguments.push(identity.into()),
		}

		arguments.push("--timestamp=none".into());

		if let Self::Push { entitlements, .. } = self {
			arguments.push("--entitlements".into());
			arguments.push(entitlements.into());
		}

		arguments
	}
}

/// Decides how a local build is signed. An app without push is signed
/// ad-hoc, as it always was. A push-enabled app is never signed ad-hoc
/// with its entitlement: an ad-hoc signature can't carry a restricted
/// entitlement, and macOS would kill the app as it starts. It needs the
/// provisioning profile and the identity [`IDENTITY`] names, or `unsigned`,
/// which builds it ad-hoc without the entitlement — an app that launches
/// and can't register for push — for measuring, or for a distribution to
/// sign again.
///
/// # Errors
///
/// When the app is push-enabled and its profile or identity is missing.
pub(crate) fn local_signing(
	root: &Path,
	config: &LeptoskitConfig,
	env: super::signing::Env<'_>,
	unsigned: bool,
) -> Result<LocalSigning> {
	let Some(profile) = provisioning_profile(root, config)? else {
		return Ok(LocalSigning::AdHoc);
	};

	if unsigned {
		return Ok(LocalSigning::AdHoc);
	}

	let identity = env(IDENTITY).with_context(|| {
		format!(
			"native.macos.aps-environment makes this a push-enabled Mac app, and leptoskit won't \
			 ad-hoc sign one: macOS refuses to launch an app that claims the push entitlement \
			 without a real signature and its provisioning profile. Set {IDENTITY} to your \
			 signing certificate's name, as `security find-identity -v -p codesigning` lists it \
			 (an Apple Development certificate for a development profile), or pass --release \
			 --unsigned to build without the push entitlement"
		)
	})?;

	let entitlements = entitlements_path(root).context(
		"the push entitlements at .leptoskit/macos/App.entitlements are missing; generation \
		 writes them, so run the build again",
	)?;

	Ok(LocalSigning::Push {
		identity,
		entitlements,
		profile,
	})
}

/// Puts the provisioning profile in the bundle, or takes a stale one out:
/// the bundle directory outlives a build, and a profile left behind by an
/// earlier push-enabled build has no place in an app without push.
pub(crate) fn embed_profile(bundle: &Path, profile: Option<&Path>) -> Result<()> {
	let embedded = bundle.join(EMBEDDED_PROFILE);

	if let Some(profile) = profile {
		return copy(profile, &embedded);
	}

	if std::fs::symlink_metadata(&embedded).is_ok() {
		std::fs::remove_file(&embedded)
			.with_context(|| format!("remove {}", embedded.display()))?;
	}

	Ok(())
}

/// Where the macOS app's entitlements live: `.leptoskit/macos/
/// App.entitlements`, present only while `[native.macos] aps-environment`
/// opts the app into push.
pub(crate) fn entitlements_path(root: &Path) -> Option<std::path::PathBuf> {
	let path = root.join(".leptoskit/macos/App.entitlements");
	path.is_file().then_some(path)
}

/// Writes `.leptoskit/macos/App.entitlements` with the APNs environment when
/// `[native.macos] aps-environment` opts the app into push, and removes a
/// stale one when it doesn't, so apps that never opted in generate — and
/// sign — exactly as before.
fn write_macos_entitlements(root: &Path, config: &LeptoskitConfig) -> Result<()> {
	let path = root.join(".leptoskit/macos/App.entitlements");
	let Some(environment) = config.native.macos.aps_environment else {
		if std::fs::symlink_metadata(&path).is_ok() {
			std::fs::remove_file(&path)?;
		}
		return Ok(());
	};

	let mut dictionary = plist::Dictionary::new();
	dictionary.insert(
		"com.apple.developer.aps-environment".into(),
		plist::Value::String(environment.as_str().into()),
	);
	let mut xml = Vec::new();
	plist::Value::Dictionary(dictionary).to_writer_xml(&mut xml)?;
	write(&path, &String::from_utf8(xml)?)?;
	Ok(())
}

fn assemble(root: &Path, config: &LeptoskitConfig, binary: &Path) -> Result<std::path::PathBuf> {
	let application = root.join(".leptoskit/macos/App.app");
	let bundle = application.join("Contents");
	copy(binary, &bundle.join("MacOS/App"))?;
	copy(
		&super::generated_dir(root, Target::Macos).join("assets/app.wasm"),
		&bundle.join("Resources/app.wasm"),
	)?;
	// The typefaces `[theme.fonts]` declares, which the host registers from
	// the staged `assets/` before its first layout.
	copy(
		&super::generated_dir(root, Target::Macos).join("mobile/fonts.json"),
		&bundle.join("Resources/fonts.json"),
	)?;
	// The app's `assets/`, staged for the bundle, where `asset://` images,
	// 3D models, and sounds read it.
	super::sync_folder(
		&super::generated_dir(root, Target::Macos).join("mobile/assets"),
		&bundle.join("Resources/assets"),
	)?;

	let info =
		match std::fs::read_to_string(super::host_dir(root, Target::Macos).join("Info.plist")) {
			Ok(info) => info,
			// Older apps lack an app-owned plist.
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
				include_str!("templates/MacInfo.plist")
					.replace("__PACKAGE__", &config.native.package_id)
					.replace("__NAME__", &config.app.name)
					.replace(
						"__VERSION__",
						&format!(
							"{}.{}.{}",
							config.app.version.major,
							config.app.version.minor,
							config.app.version.patch
						),
					)
			}
			Err(error) => return Err(error).context("read hosts/macos/Info.plist"),
		};
	// `app.icon` becomes the bundle's `.icns`, which Info.plist names.
	let resources = bundle.join("Resources");
	let info = if crate::icons::write(root, config, crate::icons::Platform::Macos, &resources)? {
		let mut plist = plist::Value::from_reader_xml(info.as_bytes())
			.context("hosts/macos/Info.plist isn't a property list")?;
		plist
			.as_dictionary_mut()
			.context("hosts/macos/Info.plist must contain a dictionary")?
			.insert("CFBundleIconFile".into(), "AppIcon".into());
		let mut xml = Vec::new();
		plist.to_writer_xml(&mut xml)?;

		String::from_utf8(xml)?
	} else {
		info
	};

	write(&bundle.join("Info.plist"), &info)?;
	Ok(application)
}

#[cfg(test)]
mod tests;

fn support_packages(
	registry: &Registry,
	packages: &Path,
	dependencies: &mut String,
	products: &mut String,
) -> Result<()> {
	for support in &registry.support {
		if let Some(module) = &support.apple_module {
			copy_package(&support.directory, &packages.join(&support.package_name))?;
			writeln!(
				dependencies,
				".package(path: \"packages/{}\"),",
				support.package_name
			)?;
			writeln!(
				products,
				".product(name: {module:?}, package: {:?}),",
				support.package_name
			)?;
		}
	}

	Ok(())
}

fn plugin_packages(
	registry: &Registry,
	packages: &Path,
	dependencies: &mut String,
	products: &mut String,
	registrant: &mut String,
	registrations: &mut String,
) -> Result<()> {
	for (plugin, module, kind) in super::project::apple_plugins(registry, Target::Macos)? {
		// CocoaPods integrates into Xcode projects, not Swift packages: the
		// macOS host builds without the plugin's pods, so its Swift must not
		// import them here.
		if plugin.uses_pods() {
			crate::ui::warning(format!(
				"{} declares apple-pods, which the macOS host cannot install: it builds without \
				 them; use a Swift package for macOS or mark the plugin unavailable there",
				plugin.name
			));
		}

		copy_package(&plugin.directory, &packages.join(&plugin.package_name))?;
		writeln!(
			dependencies,
			".package(path: \"packages/{}\"),",
			plugin.package_name
		)?;
		writeln!(
			products,
			".product(name: {module:?}, package: {:?}),",
			plugin.package_name
		)?;
		writeln!(registrant, "import {module}")?;
		writeln!(
			registrations,
			"        try dispatcher.register({kind}({}), on: bridge)",
			if plugin.apple_context {
				"context: context"
			} else {
				""
			}
		)?;
	}

	Ok(())
}
