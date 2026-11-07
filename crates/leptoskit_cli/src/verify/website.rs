//! Versioned production website inputs and exact asset-tree evidence.
//!
//! An inventory is only integrity evidence. Only the container build path may
//! construct a reproduced website descriptor after comparing fresh outputs.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;
use leptoskit_config::RenderMode;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest as _;
use sha2::Sha256;

pub(crate) const PROFILE: &str = "leptoskit-web-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Recipe {
	pub(crate) profile: String,
	/// SHA-256 of the complete raw leptoskit.toml in the source commit.
	pub(crate) config_sha256: String,
	/// SHA-256 of the ordered rustc/cargo/leptoskit/wasm-opt version report.
	pub(crate) toolchain_sha256: String,
	/// The client library stem used by wasm-bindgen and the running server.
	pub(crate) output_name: String,
}

impl Recipe {
	pub(crate) fn validate(&self, recipe: &super::BuildRecipe) -> Result<()> {
		ensure!(
			self.profile == PROFILE,
			"unsupported website production profile"
		);
		ensure!(
			super::digest_is_valid(&self.config_sha256)
				&& super::digest_is_valid(&self.toolchain_sha256),
			"website configuration and toolchain pins are required"
		);
		ensure!(
			super::is_name(&self.output_name, b"_"),
			"invalid website output name"
		);
		// The production pipeline selects features and flags from the pinned
		// app configuration. Never silently ignore backend recipe overrides.
		ensure!(
			recipe.features.is_empty()
				&& !recipe.no_default_features
				&& recipe.rustflags.is_empty(),
			"website features and rustflags must be configured in the pinned source, not backend \
			 recipe overrides"
		);
		ensure!(
			matches!(
				recipe.target.as_str(),
				"x86_64-unknown-linux-gnu" | "aarch64-unknown-linux-gnu"
			),
			"website v1 requires a native Linux GNU builder target"
		);
		Ok(())
	}

	pub(crate) fn source(&self, source: &Path, config: &LeptoskitConfig) -> Result<()> {
		Self::configuration(config)?;
		reject_services(source)?;
		ensure!(
			hash(&fs::read(source.join("leptoskit.toml"))?) == self.config_sha256,
			"website build configuration differs from recipe"
		);

		for reserved in ["dist", "target", ".leptoskit", "public/pkg", "web/pkg"] {
			ensure!(
				!source.join(reserved).exists(),
				"website source contains reserved generated directory {reserved}"
			);
		}

		Ok(())
	}

	pub(crate) fn configuration(config: &LeptoskitConfig) -> Result<()> {
		ensure!(
			config.app.platforms.contains(&Platform::Web) && config.build.backend != Some(false),
			"website profile requires web and an application backend"
		);
		ensure!(
			matches!(config.app.render, RenderMode::Ssr | RenderMode::Csr),
			"website v1 supports SSR and CSR; other render modes need their own acceptance"
		);
		// Built-in code generation is supplied by the pinned CLI. External
		// plugins and localization require additional input profiles.
		ensure!(
			config.plugins.is_empty() && config.l10n.is_none(),
			"website v1 does not support explicit plugins or localization"
		);
		Ok(())
	}
}

/// Infrastructure cache/relay services are not application executables.
pub(crate) fn reject_services(source: &Path) -> Result<()> {
	let raw: toml::Value = toml::from_str(&fs::read_to_string(source.join("leptoskit.toml"))?)?;
	// Application services add executables that need their own evidence.
	// infra.services instead configures separate cache/relay containers.
	ensure!(
		raw.get("services").is_none()
			&& raw.get("service").is_none()
			&& !source.join("services").exists(),
		"service-bearing releases need evidence for every executable and schema; unsupported"
	);
	Ok(())
}

pub(crate) fn hash(bytes: &[u8]) -> String {
	hex::encode(Sha256::digest(bytes))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Member {
	Directory {
		mode: u32,
	},
	File {
		mode: u32,
		size: u64,
		sha256: String,
	},
}

pub(crate) type Tree = BTreeMap<String, Member>;

fn safe_path(path: &str) -> bool {
	!path.is_empty()
		&& !path.contains('\\')
		&& !path.chars().any(char::is_control)
		&& path.split('/').all(|p| !matches!(p, "" | "." | ".."))
}

/// Validate every member and its parent before checking mandatory generated
/// assets. Parent membership prevents a flat inventory from hiding an
/// incomplete tree.
pub(crate) fn validate_tree(tree: &Tree, output_name: &str) -> Result<()> {
	ensure!(
		!tree.is_empty() && tree.len() <= 100_000,
		"website asset tree is absent or too large"
	);

	for (path, member) in tree {
		ensure!(safe_path(path), "unsafe website asset path");

		if let Some((parent, _)) = path.rsplit_once('/') {
			ensure!(
				matches!(tree.get(parent), Some(Member::Directory { .. })),
				"missing website parent directory"
			);
		}

		match member {
			Member::Directory { mode } => {
				ensure!(*mode == 0o755, "website directories require mode 0755");
			}
			Member::File { mode, sha256, .. } => {
				ensure!(
					*mode == 0o644 && super::digest_is_valid(sha256),
					"website files require mode 0644 and SHA-256"
				);
			}
		}
	}
	for path in [
		format!("pkg/{output_name}.js"),
		format!("pkg/{output_name}_bg.wasm"),
	] {
		ensure!(
			matches!(tree.get(&path), Some(Member::File { size, .. }) if *size > 0),
			"website profile is missing required generated asset {path}"
		);
	}
	Ok(())
}

fn unix_mode(metadata: &fs::Metadata) -> u32 {
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		metadata.permissions().mode() & 0o7777
	}
	#[cfg(not(unix))]
	{
		let _ = metadata;
		// Missing mode metadata can never satisfy the required 0755/0644 modes.
		0
	}
}

pub(crate) fn tree(root: &Path, output_name: &str) -> Result<Tree> {
	ensure!(
		fs::symlink_metadata(root)?.is_dir(),
		"website root must be a directory"
	);
	ensure!(
		unix_mode(&fs::metadata(root)?) == 0o755,
		"website root requires mode 0755"
	);
	let mut result = Tree::new();

	for entry in walkdir::WalkDir::new(root).min_depth(1).follow_links(false) {
		let entry = entry?;
		let path = entry
			.path()
			.strip_prefix(root)?
			.to_str()
			.context("website paths must be UTF-8")?
			.to_owned();
		let metadata = fs::symlink_metadata(entry.path())?;
		let mode = unix_mode(&metadata);
		let member = if metadata.is_dir() {
			Member::Directory { mode }
		} else {
			ensure!(
				metadata.is_file(),
				"website links and special files are unsupported"
			);
			let mut hasher = Sha256::new();
			let size = std::io::copy(&mut fs::File::open(entry.path())?, &mut hasher)?;
			Member::File {
				mode,
				size,
				sha256: hex::encode(hasher.finalize()),
			}
		};

		result.insert(path, member);
	}

	validate_tree(&result, output_name)?;
	Ok(result)
}

/// Copy only inventoried members, then compare the copy, catching mutation
/// between inventory and copying. No links, source or arbitrary build scripts.
pub(crate) fn copy(root: &Path, dest: &Path, output_name: &str) -> Result<Tree> {
	let expected = tree(root, output_name)?;
	fs::create_dir(dest)?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		fs::set_permissions(dest, fs::Permissions::from_mode(0o755))?;
	}

	for (path, member) in &expected {
		let target = dest.join(path);

		match member {
			Member::Directory { .. } => fs::create_dir(&target)?,
			Member::File { .. } => {
				fs::copy(root.join(path), &target)?;
			}
		}

		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			fs::set_permissions(
				&target,
				fs::Permissions::from_mode(match member {
					Member::Directory { mode } | Member::File { mode, .. } => *mode,
				}),
			)?;
		}
	}

	ensure!(
		tree(dest, output_name)? == expected,
		"website assets changed while copying"
	);
	Ok(expected)
}

/// Read image export directly: never unpack an untrusted tar onto the host.
/// Includes all members under /app/dist, so inherited extra assets fail too.
pub(crate) fn image_tree(reader: impl std::io::Read, expected: Option<&Tree>) -> Result<()> {
	let mut actual = Tree::new();
	let mut root_seen = false;

	for entry in tar::Archive::new(reader).entries()? {
		let mut entry = entry?;
		let path = entry
			.path()?
			.to_str()
			.context("non-UTF8 image path")?
			.to_owned();
		let path = path
			.strip_prefix("./")
			.unwrap_or(&path)
			.trim_end_matches('/');

		if path == "." && entry.header().entry_type().is_dir() {
			continue;
		}

		ensure!(safe_path(path), "unsafe image export path");
		ensure!(
			path != "app/services" && !path.starts_with("app/services/"),
			"runtime contains unsupported application services"
		);

		if path == "app/dist" {
			ensure!(
				!root_seen
					&& entry.header().entry_type().is_dir()
					&& entry.header().mode()? == 0o755,
				"invalid image website root"
			);
			root_seen = true;
			continue;
		}

		let Some(relative) = path.strip_prefix("app/dist/") else {
			continue;
		};
		let mode = entry.header().mode()?;
		let member = if entry.header().entry_type().is_dir() {
			Member::Directory { mode }
		} else {
			ensure!(
				entry.header().entry_type().is_file(),
				"image assets contain a link or special file"
			);
			let mut hasher = Sha256::new();
			let size = std::io::copy(&mut entry, &mut hasher)?;
			Member::File {
				mode,
				size,
				sha256: hex::encode(hasher.finalize()),
			}
		};

		ensure!(
			actual.insert(relative.to_owned(), member).is_none(),
			"duplicate image asset path"
		);
	}

	match expected {
		Some(expected) => {
			ensure!(
				root_seen && &actual == expected,
				"image website assets differ from reproduced tree"
			);
		}
		None => {
			ensure!(
				!root_seen && actual.is_empty(),
				"backend-only image contains website assets"
			);
		}
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn recipe_and_source_profiles_reject_unsupported_or_changed_inputs() {
		let dir = tempfile::tempdir().unwrap();
		let raw = concat!(
			"[app]\nname='site'\nplatforms=['web','server']\n",
			"default_platform='web'\nrender='ssr'\n[bundle]\nsplit=false\n",
		);
		fs::write(dir.path().join("leptoskit.toml"), raw).unwrap();
		let config = LeptoskitConfig::from_toml(raw, dir.path()).unwrap();
		let recipe: super::super::BuildRecipe = serde_json::from_value(serde_json::json!({
            "repository":"https://github.com/example/site", "commit":"a".repeat(40),
            "builder_image":format!("builder@sha256:{}", "b".repeat(64)),
            "target":"aarch64-unknown-linux-gnu", "package":"site", "binary":"site",
            "features":[], "no_default_features":false, "rustflags":[],
            "website":{"profile":PROFILE,"config_sha256":hash(raw.as_bytes()),"toolchain_sha256":"c".repeat(64),"output_name":"site"}
        })).unwrap();
		let website = recipe.website.as_ref().unwrap();
		assert!(super::super::validate(&recipe).is_ok());
		assert!(website.source(dir.path(), &config).is_ok());
		let mut changed = recipe.clone();
		changed.features.push("ignored-override".into());
		assert!(super::super::validate(&changed).is_err());
		changed = recipe.clone();
		changed.website.as_mut().unwrap().profile = "unknown-v2".into();
		assert!(super::super::validate(&changed).is_err());
		changed = recipe.clone();
		changed.website.as_mut().unwrap().toolchain_sha256.clear();
		assert!(super::super::validate(&changed).is_err());
		let mut configuration = config.clone();
		configuration.bundle.split = true;
		assert!(Recipe::configuration(&configuration).is_ok());
		configuration = config.clone();
		configuration.app.render = RenderMode::Islands;
		assert!(Recipe::configuration(&configuration).is_err());
		fs::write(
			dir.path().join("leptoskit.toml"),
			format!("{raw}\n# changed"),
		)
		.unwrap();
		assert!(website.source(dir.path(), &config).is_err());
		fs::write(dir.path().join("leptoskit.toml"), raw).unwrap();

		for reserved in ["dist", "target", ".leptoskit", "public/pkg", "web/pkg"] {
			fs::create_dir_all(dir.path().join(reserved)).unwrap();
			assert!(website.source(dir.path(), &config).is_err());
			fs::remove_dir_all(dir.path().join(reserved)).unwrap();
		}

		fs::write(
			dir.path().join("leptoskit.toml"),
			format!("{raw}\n[services.worker]\npackage='worker'\n"),
		)
		.unwrap();
		assert!(website.source(dir.path(), &config).is_err());
	}

	#[cfg(unix)]
	fn fixture() -> tempfile::TempDir {
		let dir = tempfile::tempdir().unwrap();
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
		}
		fs::create_dir(dir.path().join("pkg")).unwrap();
		fs::write(dir.path().join("pkg/site.js"), b"generated JavaScript").unwrap();
		fs::write(dir.path().join("pkg/site_bg.wasm"), b"\0asm").unwrap();
		fs::write(dir.path().join("style.css"), b"body {}").unwrap();
		dir
	}

	#[cfg(unix)]
	fn archive(tree: &Tree, dir: &Path) -> Vec<u8> {
		let mut builder = tar::Builder::new(Vec::new());
		let mut header = tar::Header::new_gnu();
		header.set_entry_type(tar::EntryType::Directory);
		header.set_mode(0o755);
		header.set_size(0);
		header.set_cksum();
		builder
			.append_data(&mut header, "app/dist", &[][..])
			.unwrap();

		for (path, member) in tree {
			let mut header = tar::Header::new_gnu();

			let bytes = match member {
				Member::Directory { mode } => {
					header.set_entry_type(tar::EntryType::Directory);
					header.set_mode(*mode);
					Vec::new()
				}
				Member::File { mode, .. } => {
					header.set_mode(*mode);
					fs::read(dir.join(path)).unwrap()
				}
			};

			header.set_size(bytes.len() as u64);
			header.set_cksum();
			builder
				.append_data(&mut header, format!("app/dist/{path}"), bytes.as_slice())
				.unwrap();
		}

		builder.into_inner().unwrap()
	}

	#[cfg(unix)]
	#[test]
	fn image_membership_bytes_modes_and_missing_profile_fail_closed() {
		let dir = fixture();
		let expected = tree(dir.path(), "site").unwrap();
		let bytes = archive(&expected, dir.path());
		assert!(image_tree(bytes.as_slice(), Some(&expected)).is_ok());
		assert!(image_tree(bytes.as_slice(), None).is_err());

		for path in ["pkg/site.js", "pkg/site_bg.wasm", "style.css"] {
			let original = fs::read(dir.path().join(path)).unwrap();
			fs::write(dir.path().join(path), b"substitution").unwrap();
			assert!(
				image_tree(archive(&expected, dir.path()).as_slice(), Some(&expected)).is_err()
			);
			fs::write(dir.path().join(path), original).unwrap();
		}

		for path in expected.keys() {
			let mut changed = expected.clone();
			changed.remove(path);
			assert!(image_tree(bytes.as_slice(), Some(&changed)).is_err());
		}

		let mut changed = expected.clone();
		changed.insert("extra".into(), Member::Directory { mode: 0o755 });
		assert!(image_tree(bytes.as_slice(), Some(&changed)).is_err());

		for member in changed.values_mut() {
			match member {
				Member::File { mode, .. } | Member::Directory { mode } => *mode = 0o777,
			}
		}

		assert!(image_tree(bytes.as_slice(), Some(&changed)).is_err());
	}

	#[cfg(unix)]
	#[test]
	fn incomplete_and_escaping_trees_are_not_website_evidence() {
		let dir = fixture();
		let expected = tree(dir.path(), "site").unwrap();
		for path in [
			"../outside",
			"/absolute",
			"pkg/../escape",
			"pkg//file",
			"pkg\\file",
			"pkg/line\n",
		] {
			let mut changed = expected.clone();
			changed.insert(path.into(), Member::Directory { mode: 0o755 });
			assert!(validate_tree(&changed, "site").is_err(), "{path}");
		}

		for path in ["pkg", "pkg/site.js", "pkg/site_bg.wasm"] {
			let mut changed = expected.clone();
			changed.remove(path);
			assert!(validate_tree(&changed, "site").is_err());
		}
	}

	#[cfg(unix)]
	#[test]
	fn links_and_mutated_copies_cannot_pass() {
		use std::os::unix::fs::PermissionsExt as _;
		use std::os::unix::fs::symlink;
		let dir = fixture();
		let dest = tempfile::tempdir().unwrap();
		let expected = copy(dir.path(), &dest.path().join("site"), "site").unwrap();
		fs::write(dest.path().join("site/style.css"), "changed").unwrap();
		assert_ne!(tree(&dest.path().join("site"), "site").unwrap(), expected);
		fs::set_permissions(dir.path().join("pkg"), fs::Permissions::from_mode(0o777)).unwrap();
		assert!(tree(dir.path(), "site").is_err());
		fs::set_permissions(dir.path().join("pkg"), fs::Permissions::from_mode(0o755)).unwrap();
		symlink("/etc/passwd", dir.path().join("link")).unwrap();
		assert!(tree(dir.path(), "site").is_err());
	}
}
