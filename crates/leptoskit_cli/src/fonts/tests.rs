use std::path::Path;

use leptoskit_config::LeptoskitConfig;
use leptoskit_native::fonts::Face;
use leptoskit_native::fonts::Fonts;

use super::MANIFEST;
use super::STYLESHEET;
use super::check;
use super::link_stylesheet;
use super::manifest;
use super::stylesheet;
use super::url_path;
use super::write_manifest;
use super::write_web;

/// TrueType's version tag, then enough bytes to pass for a font file.
const TRUETYPE: &[u8] = b"\0\x01\0\0 a font";
const OPENTYPE: &[u8] = b"OTTO a font";

const DECLARED: &str = r#"
[app]
name = "fonts_fixture"

[theme.fonts.hand]
family = "Skribble Noto Sketch"
faces = [
  { file = "fonts/Sketch.ttf" },
  { file = "fonts/Sketch-Italic.ttf", style = "italic" },
]

[theme.fonts.--gentle]
faces = [{ file = "fonts/Gentle 100%.otf", weight = [300, 900] }]
"#;

fn config(toml: &str) -> LeptoskitConfig {
	LeptoskitConfig::from_toml(toml, Path::new("fixture")).expect("a valid configuration")
}

fn project(files: &[(&str, &[u8])]) -> tempfile::TempDir {
	let root = tempfile::tempdir().expect("a temporary folder");

	for (file, bytes) in files {
		let path = root.path().join("assets").join(file);
		std::fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
		std::fs::write(path, bytes).expect("a file");
	}

	root
}

fn declared_project() -> tempfile::TempDir {
	project(&[
		("fonts/Sketch.ttf", TRUETYPE),
		("fonts/Sketch-Italic.ttf", TRUETYPE),
		("fonts/Gentle 100%.otf", OPENTYPE),
	])
}

#[test]
fn native_hosts_read_the_manifest_the_build_writes() {
	let root = declared_project();
	let generated = root.path().join(".leptoskit/mobile");
	let path = write_manifest(root.path(), &config(DECLARED), &generated).expect("a manifest");

	assert_eq!(path, generated.join(MANIFEST));

	let fonts = Fonts::from_json(&std::fs::read(path).expect("the manifest")).expect("its shape");

	// Families in key order: `--gentle` before `hand`, without the dashes.
	assert_eq!(fonts.family("gentle"), Some(0));
	assert_eq!(fonts.family("hand"), Some(1));
	assert_eq!(fonts.families[0].family, "gentle");
	assert_eq!(fonts.families[1].family, "Skribble Noto Sketch");
	assert_eq!(fonts.families[0].faces[0].file, "fonts/Gentle 100%.otf");
	assert_eq!(fonts.families[0].faces[0].weight, [300, 900]);
	assert_eq!(
		fonts.select(1, 400, true),
		Some(Face {
			index: 2,
			weight: 400
		})
	);
	assert_eq!(
		fonts.select(0, 1000, false),
		Some(Face {
			index: 0,
			weight: 900
		})
	);
}

#[test]
fn an_app_without_fonts_writes_an_empty_manifest() {
	let config = config("[app]\nname = \"plain\"\n");
	let manifest = manifest(&config).expect("a manifest");

	assert_eq!(
		Fonts::from_json(&manifest).expect("its shape"),
		Fonts::default()
	);
	assert_eq!(stylesheet(&config), None);
}

#[test]
fn a_missing_face_fails_the_build_naming_it() {
	let root = project(&[("fonts/Sketch.ttf", TRUETYPE)]);
	let error = check(root.path(), &config(DECLARED)).expect_err("two faces are missing");
	let message = format!("{error:#}");

	assert!(
		message.contains("[theme.fonts.--gentle] faces[0] declares `fonts/Gentle 100%.otf`"),
		"{message}"
	);
	assert!(message.contains("no font file at"), "{message}");
	assert!(
		message.contains(&root.path().join("assets/fonts").display().to_string()),
		"{message}"
	);
	assert!(
		write_manifest(root.path(), &config(DECLARED), &root.path().join("out")).is_err(),
		"a manifest is written only for faces that exist"
	);
	assert!(!root.path().join("out").join(MANIFEST).exists());
}

#[test]
fn a_face_that_is_not_a_font_fails_the_build() {
	for (bytes, reason) in [
		(
			&b"wOF2 compressed"[..],
			"is not a TrueType or OpenType font",
		),
		(
			&b"ttcf collection"[..],
			"is not a TrueType or OpenType font",
		),
		(&b"\0\x01"[..], "is too short to be a font"),
	] {
		let root = project(&[
			("fonts/Sketch.ttf", bytes),
			("fonts/Sketch-Italic.ttf", TRUETYPE),
			("fonts/Gentle 100%.otf", OPENTYPE),
		]);
		let error = check(root.path(), &config(DECLARED)).expect_err("not a font");
		let message = format!("{error:#}");

		assert!(message.contains("[theme.fonts.hand] faces[0]"), "{message}");
		assert!(message.contains(reason), "{message}");
	}

	let root = project(&[
		("fonts/Sketch.ttf", b"true a font"),
		("fonts/Sketch-Italic.ttf", TRUETYPE),
		("fonts/Gentle 100%.otf", OPENTYPE),
	]);
	assert!(check(root.path(), &config(DECLARED)).is_ok());
}

#[test]
fn the_web_stylesheet_declares_every_face_and_names_every_family() {
	assert_eq!(
		stylesheet(&config(DECLARED)).expect("a stylesheet"),
		r#"@font-face {
	font-family: "gentle";
	src: url("assets/fonts/Gentle%20100%25.otf") format("opentype");
	font-weight: 300 900;
	font-style: normal;
	font-display: swap;
}
@font-face {
	font-family: "Skribble Noto Sketch";
	src: url("assets/fonts/Sketch.ttf") format("truetype");
	font-weight: 400;
	font-style: normal;
	font-display: swap;
}
@font-face {
	font-family: "Skribble Noto Sketch";
	src: url("assets/fonts/Sketch-Italic.ttf") format("truetype");
	font-weight: 400;
	font-style: italic;
	font-display: swap;
}
:root {
	--gentle: "gentle", ui-sans-serif, system-ui, sans-serif;
	--hand: "Skribble Noto Sketch", ui-sans-serif, system-ui, sans-serif;
}
"#
	);
}

#[test]
fn web_builds_write_and_link_the_stylesheet_once() {
	let root = declared_project();
	let out = root.path().join("dist");
	std::fs::create_dir_all(&out).expect("the output folder");
	std::fs::write(
		out.join("index.html"),
		"<html><head><script type=\"module\" src=\"./pkg/app.js\"></script></head></html>",
	)
	.expect("the page");

	write_web(root.path(), &config(DECLARED), &out).expect("the stylesheet");
	write_web(root.path(), &config(DECLARED), &out).expect("the stylesheet again");

	assert!(out.join(STYLESHEET).is_file());
	assert_eq!(
		std::fs::read_to_string(out.join("index.html")).expect("the page"),
		"<html><head><script type=\"module\" src=\"./pkg/app.js\"></script><link \
		 rel=\"stylesheet\" href=\"./leptoskit-fonts.css\" /></head></html>"
	);

	// Fonts the app no longer declares leave the build.
	write_web(root.path(), &config("[app]\nname = \"plain\"\n"), &out).expect("no stylesheet");
	assert!(!out.join(STYLESHEET).exists());
}

#[test]
fn the_stylesheet_links_from_the_root_unless_the_page_is_relative() {
	assert_eq!(
		link_stylesheet("<HEAD><!-- leptoskit-fonts.css --></HEAD>"),
		"<HEAD><!-- leptoskit-fonts.css --><link rel=\"stylesheet\" href=\"/leptoskit-fonts.css\" \
		 /></HEAD>"
	);
	assert_eq!(
		link_stylesheet("<head><base href=\"/docs/\" /></head>"),
		"<head><base href=\"/docs/\" /><link rel=\"stylesheet\" href=\"leptoskit-fonts.css\" \
		 /></head>"
	);
	assert_eq!(link_stylesheet("<body></body>"), "<body></body>");
}

#[test]
fn file_names_are_escaped_as_url_paths() {
	assert_eq!(
		url_path("fonts/a b#c?d%e.ttf"),
		"fonts/a%20b%23c%3Fd%25e.ttf"
	);
}
