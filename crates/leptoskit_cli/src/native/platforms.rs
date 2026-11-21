//! In-tree platform extension crates registered with the CLI.
//!
//! Every `native platform` subcommand goes through [`builtin`] / [`find`] so
//! the registry stays live. HarmonyOS, Meta Horizon, and the experimental
//! Oculus stubs register here; first-party `quest` packaging stays outside this
//! table.

use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use clap::Subcommand;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;
use leptoskit_platform::PlatformExtension;
use leptoskit_platform::ScaffoldContext;
use leptoskit_platform_harmonyos::HarmonyOs;
use leptoskit_platform_horizon::Horizon;
use leptoskit_platform_horizon::bundle;
use leptoskit_platform_oculus::Oculus;

/// Builtin platform extensions shipped in this workspace.
#[must_use]
pub fn builtin() -> [&'static dyn PlatformExtension; 3] {
	[&HarmonyOs, &Horizon, &Oculus]
}

/// Looks up a builtin extension by destination id.
#[must_use]
#[cfg_attr(
	not(test),
	expect(dead_code, reason = "public registry lookup; tests cover it")
)]
pub fn find(id: &str) -> Option<&'static dyn PlatformExtension> {
	find_in(&builtin(), id)
}

fn find_in<'a>(
	extensions: &[&'a dyn PlatformExtension],
	id: &str,
) -> Option<&'a dyn PlatformExtension> {
	extensions
		.iter()
		.copied()
		.find(|extension| extension.id() == id)
}

/// `leptoskit native platform` subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum PlatformCommand {
	/// Lists in-tree platform extensions and their destination ids.
	List,
	/// Prints one extension's versioned capability manifest as JSON.
	Capabilities {
		/// Extension id (`harmonyos`, `horizon`, `oculus`, …).
		id: String,
	},
	/// Writes an extension's host stubs under `hosts/<id>/` (existing files
	/// stay).
	Scaffold {
		/// Extension id.
		id: String,
	},
	/// Packages a platform extension's deliverable (for `horizon`, validates
	/// and builds a self-contained offline MHCP `.zip` archive).
	Package {
		/// Extension id (`horizon`, …).
		id: String,
		/// Directory of built web files to package instead of running a fresh
		/// web release build.
		#[arg(long)]
		source: Option<PathBuf>,
		/// Destination path for the `.zip` archive (defaults to
		/// `.leptoskit/horizon/<app>-<version>-mhcp.zip`).
		#[arg(long)]
		out: Option<PathBuf>,
		/// Package `dist/` directly without rebuilding the web app first.
		#[arg(long)]
		skip_build: bool,
		/// Maximum archive size in MiB (defaults to 35 MiB for MHCP).
		#[arg(long, default_value_t = 35)]
		max_mb: usize,
	},
}

/// Dispatches `native platform` through the builtin registry.
pub fn run(command: PlatformCommand) -> Result<()> {
	run_with(&builtin(), command)
}

fn run_with(extensions: &[&dyn PlatformExtension], command: PlatformCommand) -> Result<()> {
	match command {
		PlatformCommand::List => list_extensions(extensions),
		PlatformCommand::Capabilities { id } => show_capabilities(extensions, &id),
		PlatformCommand::Scaffold { id } => scaffold_extension(extensions, &id),
		PlatformCommand::Package {
			id,
			source,
			out,
			skip_build,
			max_mb,
		} => {
			let extension = require_in(extensions, &id)?;
			package_extension(
				extension,
				source.as_deref(),
				out.as_deref(),
				skip_build,
				max_mb,
			)
		}
	}
}

fn list_extensions(extensions: &[&dyn PlatformExtension]) -> Result<()> {
	if extensions.is_empty() {
		println!(
			"no in-tree platform extensions registered yet; add a `leptoskit_platform_*` crate \
			 and list it in native/platforms.rs"
		);
		return Ok(());
	}
	for extension in extensions {
		let manifest = extension.manifest();
		manifest.validate().map_err(anyhow::Error::msg)?;
		let experimental = if manifest.experimental {
			"experimental"
		} else {
			"stable"
		};
		println!(
			"{}\t{}\t{}\t{experimental}",
			manifest.id, manifest.display_name, manifest.version
		);
	}
	Ok(())
}

fn show_capabilities(extensions: &[&dyn PlatformExtension], id: &str) -> Result<()> {
	let extension = require_in(extensions, id)?;
	let manifest = extension.manifest();
	manifest.validate().map_err(anyhow::Error::msg)?;
	println!("{}", serde_json::to_string_pretty(&manifest)?);
	Ok(())
}

fn scaffold_extension(extensions: &[&dyn PlatformExtension], id: &str) -> Result<()> {
	let extension = require_in(extensions, id)?;
	let (config, root) = LeptoskitConfig::load()
		.context("platform scaffold needs a leptoskit project (run from the app root)")?;
	let package = if config.native.package_id.is_empty() {
		leptoskit_config::default_package_id(&config.app.name)
	} else {
		config.native.package_id.clone()
	};
	let version = config.app.version.to_string();
	let context = ScaffoldContext {
		app_name: &config.app.name,
		package_id: &package,
		version: &version,
	};
	let host = root.join("hosts").join(extension.id());
	std::fs::create_dir_all(&host)
		.with_context(|| format!("could not create {}", host.display()))?;
	let mut written = 0usize;
	for file in extension.scaffold_files(context) {
		let path = host.join(&file.relative_path);
		if path.exists() {
			continue;
		}
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)
				.with_context(|| format!("could not create {}", parent.display()))?;
		}
		std::fs::write(&path, file.contents)
			.with_context(|| format!("could not write {}", path.display()))?;
		written += 1;
	}
	println!(
		"scaffolded {} new file(s) for {} under {}",
		written,
		extension.display_name(),
		host.display()
	);
	Ok(())
}

fn package_extension(
	extension: &dyn PlatformExtension,
	source: Option<&Path>,
	out: Option<&Path>,
	skip_build: bool,
	max_mb: usize,
) -> Result<()> {
	match extension.id() {
		Horizon::ID => package_horizon(source, out, skip_build, max_mb),
		Oculus::ID => anyhow::bail!("{}", Oculus::packaging_unavailable_message()),
		HarmonyOs::ID => {
			anyhow::bail!(
				"HarmonyOS HAP packaging runs through `leptoskit native build harmonyos`"
			);
		}
		other => anyhow::bail!("platform extension `{other}` does not implement packaging yet"),
	}
}

fn package_horizon(
	source: Option<&Path>,
	out: Option<&Path>,
	skip_build: bool,
	max_mb: usize,
) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()
		.context("platform package needs a leptoskit project (run from the app root)")?;
	let source_dir = if let Some(dir) = source {
		if dir.is_absolute() {
			dir.to_path_buf()
		} else {
			root.join(dir)
		}
	} else if skip_build {
		root.join("dist")
	} else {
		let staging = ".leptoskit/horizon/dist";
		crate::build::run(true, Some(Platform::Web), None, staging)?;
		root.join(staging)
	};

	let destination = match out {
		Some(path) if path.is_absolute() => path.to_path_buf(),
		Some(path) => root.join(path),
		None => {
			root.join(".leptoskit/horizon").join(format!(
				"{}-{}-mhcp.zip",
				config.app.name, config.app.version
			))
		}
	};
	let max_bytes = max_mb
		.checked_mul(1024 * 1024)
		.context("--max-mb is too large")?;
	let summary = package_horizon_dir(&source_dir, &destination, max_bytes)?;
	println!(
		"packaged Meta Horizon MHCP bundle at {} ({} file(s), {} bytes)",
		summary.output.display(),
		summary.file_count,
		summary.zip_bytes
	);
	Ok(())
}

/// Summary of a packaged Meta Horizon MHCP offline `.zip` archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HorizonPackageSummary {
	pub output: PathBuf,
	pub file_count: usize,
	pub zip_bytes: usize,
}

/// Reads `source_dir`, strips precompressed server sidecars (`.br`/`.gz`),
/// rewrites root-relative asset URLs in HTML to relative paths, validates every
/// MHCP offline rule, and writes a deterministic `.zip` to `destination`.
pub fn package_horizon_dir(
	source_dir: &Path,
	destination: &Path,
	max_bytes: usize,
) -> Result<HorizonPackageSummary> {
	anyhow::ensure!(
		source_dir.is_dir(),
		"Meta Horizon package source directory does not exist: {}",
		source_dir.display()
	);
	let raw_files = crate::extension::zip::files_under(source_dir)?;
	let mut files: Vec<(String, Vec<u8>)> = Vec::with_capacity(raw_files.len());
	for (path, bytes) in raw_files {
		if is_precompressed_sidecar(&path) {
			continue;
		}
		if Path::new(&path)
			.extension()
			.and_then(std::ffi::OsStr::to_str)
			.is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"))
			&& let Ok(text) = std::str::from_utf8(&bytes)
		{
			let rewritten = bundle::rewrite_html_relative(text);
			files.push((path, rewritten.into_bytes()));
		} else {
			files.push((path, bytes));
		}
	}

	let report = bundle::validate_files(&files, max_bytes);
	for warning in report
		.issues
		.iter()
		.filter(|issue| issue.severity == bundle::Severity::Warning)
	{
		eprintln!("warning [{}]: {}", warning.code, warning.message);
	}
	if !report.is_valid() {
		let messages: Vec<String> = report
			.errors()
			.map(|issue| {
				match &issue.path {
					Some(path) => format!("[{}] ({path}) {}", issue.code, issue.message),
					None => format!("[{}] {}", issue.code, issue.message),
				}
			})
			.collect();
		anyhow::bail!(
			"Meta Horizon MHCP bundle validation failed:\n- {}",
			messages.join("\n- ")
		);
	}

	let archive = crate::extension::zip::archive(&files)?;
	anyhow::ensure!(
		archive.len() <= max_bytes,
		"packaged MHCP zip is {} bytes, exceeding the {max_bytes}-byte limit",
		archive.len()
	);
	if let Some(parent) = destination.parent() {
		std::fs::create_dir_all(parent)
			.with_context(|| format!("could not create {}", parent.display()))?;
	}
	std::fs::write(destination, &archive)
		.with_context(|| format!("could not write {}", destination.display()))?;

	Ok(HorizonPackageSummary {
		output: destination.to_path_buf(),
		file_count: files.len(),
		zip_bytes: archive.len(),
	})
}

fn is_precompressed_sidecar(path: &str) -> bool {
	Path::new(path)
		.extension()
		.and_then(std::ffi::OsStr::to_str)
		.is_some_and(|ext| ext.eq_ignore_ascii_case("br") || ext.eq_ignore_ascii_case("gz"))
}

fn require_in<'a>(
	extensions: &[&'a dyn PlatformExtension],
	id: &str,
) -> Result<&'a dyn PlatformExtension> {
	find_in(extensions, id).with_context(|| {
		let known: Vec<_> = extensions.iter().map(|ext| ext.id()).collect();
		if known.is_empty() {
			format!("unknown platform extension `{id}`; no in-tree extensions are registered yet")
		} else {
			format!(
				"unknown platform extension `{id}`; known: {}",
				known.join(", ")
			)
		}
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn registers_harmonyos_horizon_and_oculus() {
		let extensions = builtin();
		assert_eq!(extensions.len(), 3);
		assert_eq!(find("harmonyos").unwrap().display_name(), "HarmonyOS");
		assert_eq!(find("horizon").unwrap().display_name(), "Meta Horizon");
		assert_eq!(find("oculus").unwrap().display_name(), "Meta Quest");
		assert!(find("quest").is_none());
		assert!(find("web").is_none());
		for extension in extensions {
			extension.manifest().validate().unwrap();
		}
	}

	#[test]
	fn require_lists_known_ids() {
		let error = match require_in(&builtin(), "missing") {
			Ok(_) => panic!("expected missing extension"),
			Err(error) => error.to_string(),
		};
		assert!(error.contains("missing"));
		assert!(error.contains("harmonyos"));
		assert!(error.contains("horizon"));
		assert!(error.contains("oculus"));
	}

	#[test]
	fn list_and_capabilities_include_horizon_and_oculus() {
		run(PlatformCommand::List).unwrap();
		run(PlatformCommand::Capabilities {
			id: "horizon".into(),
		})
		.unwrap();
		run(PlatformCommand::Capabilities {
			id: "oculus".into(),
		})
		.unwrap();
	}

	#[test]
	fn scaffold_horizon_and_oculus_write_host_files() {
		let root = tempfile::tempdir().unwrap();
		std::fs::write(
			root.path().join("leptoskit.toml"),
			"[app]\nname = \"horizon_probe\"\n",
		)
		.unwrap();
		let previous = std::env::current_dir().unwrap();
		std::env::set_current_dir(root.path()).unwrap();
		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			run(PlatformCommand::Scaffold {
				id: "oculus".into(),
			})
			.unwrap();
			assert!(
				root.path()
					.join("hosts/oculus/android/QuestActivity.kt")
					.is_file()
			);
			assert!(root.path().join("hosts/oculus/guest.rs").is_file());

			run(PlatformCommand::Scaffold {
				id: "horizon".into(),
			})
			.unwrap();
			assert!(
				root.path()
					.join("hosts/horizon/scripts/LeptosKitHost.ts")
					.is_file()
			);
			assert!(root.path().join("hosts/horizon/scripts/scene.ts").is_file());
			assert!(root.path().join("hosts/horizon/mhcp/index.html").is_file());
			assert!(
				root.path()
					.join("hosts/horizon/mhcp/BUILD_LOG.md")
					.is_file()
			);
		}));
		let _ = std::env::set_current_dir(previous);
		if let Err(payload) = result {
			std::panic::resume_unwind(payload);
		}
	}

	#[test]
	fn package_horizon_dir_rewrites_paths_strips_sidecars_and_builds_deterministic_zip() {
		let dir = tempfile::tempdir().unwrap();
		let src = dir.path().join("dist");
		std::fs::create_dir_all(src.join("vendor")).unwrap();
		std::fs::write(
			src.join("index.html"),
			r#"<!doctype html><html><head><meta name="viewport" content="width=device-width"><script type="module" src="/vendor/app.js"></script></head><body></body></html>"#,
		)
		.unwrap();
		std::fs::write(src.join("vendor/app.js"), "console.log('mhcp');").unwrap();
		std::fs::write(src.join("vendor/app.js.br"), [1, 2, 3]).unwrap();
		std::fs::write(src.join("vendor/app.js.gz"), [4, 5, 6]).unwrap();

		let zip_a = dir.path().join("out/a.zip");
		let zip_b = dir.path().join("out/b.zip");
		let summary_a = package_horizon_dir(&src, &zip_a, bundle::MAX_BUNDLE_BYTES).unwrap();
		let summary_b = package_horizon_dir(&src, &zip_b, bundle::MAX_BUNDLE_BYTES).unwrap();

		assert_eq!(summary_a.file_count, 2);
		assert_eq!(summary_a.zip_bytes, summary_b.zip_bytes);
		assert_eq!(
			std::fs::read(&zip_a).unwrap(),
			std::fs::read(&zip_b).unwrap()
		);
	}

	#[test]
	fn package_horizon_dir_rejects_external_cdn_and_missing_index() {
		let dir = tempfile::tempdir().unwrap();
		let bad_cdn = dir.path().join("cdn");
		std::fs::create_dir_all(&bad_cdn).unwrap();
		std::fs::write(
			bad_cdn.join("index.html"),
			r#"<meta name="viewport" content="width=device-width"><script src="https://cdn.example.com/three.min.js"></script>"#,
		)
		.unwrap();
		let error = package_horizon_dir(
			&bad_cdn,
			&dir.path().join("cdn.zip"),
			bundle::MAX_BUNDLE_BYTES,
		)
		.unwrap_err()
		.to_string();
		assert!(error.contains("external-url"), "{error}");

		let no_index = dir.path().join("empty");
		std::fs::create_dir_all(&no_index).unwrap();
		std::fs::write(no_index.join("app.js"), "1").unwrap();
		let error = package_horizon_dir(
			&no_index,
			&dir.path().join("empty.zip"),
			bundle::MAX_BUNDLE_BYTES,
		)
		.unwrap_err()
		.to_string();
		assert!(error.contains("missing-index-html"), "{error}");
	}

	#[test]
	fn package_oculus_explains_to_use_quest() {
		let error = package_extension(&Oculus, None, None, true, 35)
			.unwrap_err()
			.to_string();
		assert!(error.contains("--platform quest"), "{error}");
	}
}
