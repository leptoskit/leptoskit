//! The typefaces an app declares under `[theme.fonts]`.
//!
//! Every build checks that each declared face is a font file inside the
//! app's assets folder, which the build already ships. Native builds then
//! write `fonts.json`, the manifest each host installs before its first
//! layout (the shape `leptoskit_native::fonts::Fonts` reads), and web builds
//! write `leptoskit-fonts.css`, which declares each face with `@font-face`
//! and names each family as the custom property `font-[var(--name)]` reads.

use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::FontConfig;
use leptoskit_config::FontStyleConfig;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::ThemeConfig;
use serde::Serialize;

/// The manifest native hosts read, beside the app bundle.
pub(crate) const MANIFEST: &str = "fonts.json";

/// The stylesheet web builds write beside `index.html`.
pub(crate) const STYLESHEET: &str = "leptoskit-fonts.css";

/// What a family falls back to while its file loads, or when it can't.
const FALLBACK: &str = "ui-sans-serif, system-ui, sans-serif";

#[derive(Serialize)]
struct Manifest<'a> {
	families: Vec<ManifestFamily<'a>>,
}

#[derive(Serialize)]
struct ManifestFamily<'a> {
	name: &'a str,
	family: &'a str,
	faces: Vec<ManifestFace<'a>>,
}

#[derive(Serialize)]
struct ManifestFace<'a> {
	file: &'a str,
	weight: [u16; 2],
	italic: bool,
}

/// The declared families in manifest order, by custom property name.
fn families(config: &LeptoskitConfig) -> impl Iterator<Item = (&str, &str, &FontConfig)> {
	config
		.theme
		.iter()
		.flat_map(|theme| theme.fonts.iter())
		.map(|(key, font)| {
			let name = ThemeConfig::property_name(key);
			let family = font.family.as_deref().map_or(name, str::trim);

			(name, family, font)
		})
}

/// Fails unless every declared face is a TrueType or OpenType file in the
/// app's assets folder, naming the face and where it looked.
pub(crate) fn check(root: &Path, config: &LeptoskitConfig) -> Result<()> {
	let assets = root.join(&config.build.inputs.assets);

	for (key, font) in config.theme.iter().flat_map(|theme| theme.fonts.iter()) {
		for (index, face) in font.faces.iter().enumerate() {
			let path = face
				.file
				.split('/')
				.fold(assets.clone(), |path, segment| path.join(segment));

			check_face(&path).with_context(|| {
				format!(
					"[theme.fonts.{key}] faces[{index}] declares `{}`, which must be a font file \
					 in the app's assets folder",
					face.file
				)
			})?;
		}
	}

	Ok(())
}

/// Whether `path` is a file that starts as a TrueType or OpenType font does.
fn check_face(path: &Path) -> Result<()> {
	use std::io::Read as _;

	let mut file =
		std::fs::File::open(path).with_context(|| format!("no font file at {}", path.display()))?;
	let mut tag = [0; 4];

	file.read_exact(&mut tag)
		.with_context(|| format!("{} is too short to be a font", path.display()))?;
	anyhow::ensure!(
		matches!(&tag, b"\0\x01\0\0" | b"true" | b"OTTO"),
		"{} is not a TrueType or OpenType font (it starts {tag:02x?}); a font collection or a \
		 WOFF file renamed .ttf doesn't load on every platform",
		path.display()
	);

	Ok(())
}

/// The manifest native hosts install: every declared family in key order,
/// each face's file relative to the bundled assets folder. An app that
/// declares no fonts gets an empty manifest, so hosts never look further.
pub(crate) fn manifest(config: &LeptoskitConfig) -> Result<Vec<u8>> {
	let families = families(config)
		.map(|(name, family, font)| {
			ManifestFamily {
				name,
				family,
				faces: font
					.faces
					.iter()
					.map(|face| {
						ManifestFace {
							file: &face.file,
							weight: face.weight.range(),
							italic: face.style == FontStyleConfig::Italic,
						}
					})
					.collect(),
			}
		})
		.collect();

	Ok(serde_json::to_vec_pretty(&Manifest { families })?)
}

/// Checks the declared faces and writes the manifest into `folder`, the
/// generated folder the native host bundles beside its update configuration.
pub(crate) fn write_manifest(
	root: &Path,
	config: &LeptoskitConfig,
	folder: &Path,
) -> Result<PathBuf> {
	check(root, config)?;
	std::fs::create_dir_all(folder).with_context(|| format!("create {}", folder.display()))?;
	let path = folder.join(MANIFEST);
	std::fs::write(&path, manifest(config)?)
		.with_context(|| format!("write {}", path.display()))?;

	Ok(path)
}

/// The web stylesheet: an `@font-face` rule per declared face, its file
/// under the build's `assets/`, and a `:root` rule naming each family as its
/// custom property. `None` when the app declares no fonts.
pub(crate) fn stylesheet(config: &LeptoskitConfig) -> Option<String> {
	let mut css = String::new();
	let mut root = String::new();

	for (name, family, font) in families(config) {
		for face in &font.faces {
			let [min, max] = face.weight.range();
			let weight = if min == max {
				min.to_string()
			} else {
				format!("{min} {max}")
			};
			let style = match face.style {
				FontStyleConfig::Normal => "normal",
				FontStyleConfig::Italic => "italic",
			};
			// The configuration accepts only TrueType and OpenType files.
			let format = face
				.format()
				.map(|format| format!(" format(\"{format}\")"))
				.unwrap_or_default();
			let _ = writeln!(
				css,
				"@font-face {{\n\tfont-family: \"{family}\";\n\tsrc: \
				 url(\"assets/{}\"){format};\n\tfont-weight: {weight};\n\tfont-style: \
				 {style};\n\tfont-display: swap;\n}}",
				url_path(&face.file)
			);
		}

		let _ = writeln!(root, "\t--{name}: \"{family}\", {FALLBACK};");
	}

	if root.is_empty() {
		return None;
	}

	let _ = write!(css, ":root {{\n{root}}}\n");

	Some(css)
}

/// `file` as a URL path: the characters that end a path or start an escape
/// are escaped.
fn url_path(file: &str) -> String {
	let mut path = String::with_capacity(file.len());

	for ch in file.chars() {
		match ch {
			'%' => path.push_str("%25"),
			'#' => path.push_str("%23"),
			'?' => path.push_str("%3F"),
			' ' => path.push_str("%20"),
			_ => path.push(ch),
		}
	}

	path
}

/// Checks the declared faces and writes the web stylesheet into `out_dir`,
/// linking it from `index.html`; removes a stylesheet an earlier build wrote
/// when the app declares no fonts any more.
pub(crate) fn write_web(root: &Path, config: &LeptoskitConfig, out_dir: &Path) -> Result<()> {
	check(root, config)?;
	let path = out_dir.join(STYLESHEET);

	let Some(css) = stylesheet(config) else {
		if path.is_file() {
			std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
		}

		return Ok(());
	};

	std::fs::write(&path, css).with_context(|| format!("write {}", path.display()))?;
	let index = out_dir.join("index.html");

	if index.is_file() {
		let html =
			std::fs::read_to_string(&index).with_context(|| format!("read {}", index.display()))?;
		let linked = link_stylesheet(&html);

		if linked != html {
			std::fs::write(&index, linked).with_context(|| format!("write {}", index.display()))?;
		}
	}

	Ok(())
}

/// `html` with the font stylesheet linked before `</head>`: against the
/// page's `<base>` when it has one, beside the page when it loads its bundle
/// relatively (`./pkg/`), and from the site's root otherwise, as the bundle's
/// preloads are. Idempotent.
fn link_stylesheet(html: &str) -> String {
	let lower = html.to_ascii_lowercase();

	let Some(head_end) = lower.find("</head>") else {
		return html.to_owned();
	};

	let prefix = if lower.contains("<base ") {
		""
	} else if html.contains("./pkg/") {
		"./"
	} else {
		"/"
	};
	let link = format!("<link rel=\"stylesheet\" href=\"{prefix}{STYLESHEET}\" />");

	if html.contains(&link) {
		return html.to_owned();
	}

	format!("{}{link}{}", &html[..head_end], &html[head_end..])
}

#[cfg(test)]
mod tests;
