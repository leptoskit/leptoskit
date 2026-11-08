//! Launch screens from `[app.launch_screen]`: what iOS and Android show from
//! the moment the app is opened until its first screen is drawn.
//!
//! - iOS: a `LaunchBackground` colour and a `LaunchImage` in the app's asset
//!   catalog, named by `UILaunchScreen` in the generated `Info.plist`.
//! - Android: a `LeptosKitLaunchTheme` for the launcher activity. Android 12
//!   and later draw its splash screen attributes; earlier versions draw its
//!   window background, a layer list of the colour and the image.
//!
//! The image is read the way [`crate::icons`] reads `app.icon`: one square
//! PNG or SVG, 1024 pixels or larger.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::LaunchScreen;
use leptoskit_config::LeptoskitConfig;
use serde_json::json;

use crate::icons::Icon;

/// The iOS colour set and image set names `UILaunchScreen` refers to.
pub const IOS_COLOR: &str = "LaunchBackground";
pub const IOS_IMAGE: &str = "LaunchImage";

/// The theme leptoskit generates for the launcher activity.
pub const ANDROID_THEME: &str = "LeptosKitLaunchTheme";

/// Android's splash icon canvas and the circle it is shown in, in dp.
const SPLASH_CANVAS: u32 = 288;
const SPLASH_CIRCLE: u32 = 192;

/// The launch screen, with its image read.
pub struct Launch<'a> {
	config: &'a LaunchScreen,
	image: Option<Icon>,
}

impl<'a> Launch<'a> {
	/// Reads `[app.launch_screen]`, or `None` when the app has none.
	///
	/// # Errors
	///
	/// When the image can't be read, isn't a PNG or SVG, isn't square, or is
	/// too small.
	pub fn load(root: &Path, config: &'a LeptoskitConfig) -> Result<Option<Self>> {
		let Some(launch) = &config.app.launch_screen else {
			return Ok(None);
		};
		let image = launch
			.image
			.as_ref()
			.map(|image| {
				Icon::load(&root.join(image))
					.with_context(|| format!("app.launch_screen.image {}", image.display()))
			})
			.transpose()?;

		Ok(Some(Self {
			config: launch,
			image,
		}))
	}

	/// Whether there is an image to draw.
	#[must_use]
	pub fn has_image(&self) -> bool {
		self.image.is_some()
	}

	/// The iOS asset catalog's colour set and image set, by path relative to
	/// the generated directory.
	///
	/// # Errors
	///
	/// When encoding fails.
	pub fn ios_files(&self) -> Result<Vec<(String, Vec<u8>)>> {
		let catalog = "Assets.xcassets";
		let mut colors = vec![json!({
			"color": ios_color(&self.config.background)?,
			"idiom": "universal",
		})];

		if let Some(dark) = &self.config.background_dark {
			colors.push(json!({
				"appearances": [{ "appearance": "luminosity", "value": "dark" }],
				"color": ios_color(dark)?,
				"idiom": "universal",
			}));
		}

		let mut files = vec![
			(
				format!("{catalog}/Contents.json"),
				crate::icons::CATALOG.as_bytes().to_vec(),
			),
			(
				format!("{catalog}/{IOS_COLOR}.colorset/Contents.json"),
				pretty(&json!({ "colors": colors, "info": info() }))?,
			),
		];

		if let Some(image) = &self.image {
			let mut images = Vec::new();

			for scale in 1..=3 {
				let name = format!("launch@{scale}x.png");
				images.push(json!({
					"filename": name,
					"idiom": "universal",
					"scale": format!("{scale}x"),
				}));
				files.push((
					format!("{catalog}/{IOS_IMAGE}.imageset/{name}"),
					image.png(self.config.image_size * scale)?,
				));
			}

			files.push((
				format!("{catalog}/{IOS_IMAGE}.imageset/Contents.json"),
				pretty(&json!({ "images": images, "info": info() }))?,
			));
		}

		Ok(files)
	}

	/// The `UILaunchScreen` dictionary that names the asset catalog's sets.
	#[must_use]
	pub fn ios_plist(&self) -> plist::Dictionary {
		let mut screen = plist::Dictionary::new();
		screen.insert("UIColorName".into(), IOS_COLOR.into());

		if self.image.is_some() {
			screen.insert("UIImageName".into(), IOS_IMAGE.into());
		}

		screen
	}

	/// Android's resources, by path relative to the generated `res`
	/// directory. `parent` is the theme the launch theme extends: the
	/// application's own, so the app looks the same once it has started.
	///
	/// # Errors
	///
	/// When encoding fails.
	pub fn android_files(&self, parent: &str) -> Result<Vec<(String, Vec<u8>)>> {
		let background = android_color(&self.config.background)?;
		let mut files = vec![
			(
				"values/leptoskit_launch.xml".to_owned(),
				format!(
					"{HEADER}<resources>\n    <color \
					 name=\"leptoskit_launch_background\">{background}</color>\n    <style \
					 name=\"{ANDROID_THEME}\" parent=\"{parent}\">\n        <item \
					 name=\"android:windowBackground\">@drawable/leptoskit_launch</item>\n    \
					 </style>\n</resources>\n"
				)
				.into_bytes(),
			),
			(
				"values-v31/leptoskit_launch.xml".to_owned(),
				format!(
					"{HEADER}<resources>\n    <style name=\"{ANDROID_THEME}\" \
					 parent=\"{parent}\">\n        <item \
					 name=\"android:windowSplashScreenBackground\">@color/\
					 leptoskit_launch_background</item>\n{}    </style>\n</resources>\n",
					if self.image.is_some() {
						"        <item \
						 name=\"android:windowSplashScreenAnimatedIcon\">@drawable/\
						 leptoskit_launch_icon</item>\n"
					} else {
						""
					}
				)
				.into_bytes(),
			),
		];

		if let Some(dark) = &self.config.background_dark {
			files.push((
				"values-night/leptoskit_launch_colors.xml".to_owned(),
				format!(
					"{HEADER}<resources>\n    <color \
					 name=\"leptoskit_launch_background\">{}</color>\n</resources>\n",
					android_color(dark)?
				)
				.into_bytes(),
			));
		}

		let image_layer = if let Some(image) = &self.image {
			let size = self.config.image_size;
			// One picture at the densest screens' scale, drawn at its size in
			// dp everywhere.
			files.push((
				"drawable-nodpi/leptoskit_launch_image.png".to_owned(),
				image.png(size * 4)?,
			));
			// Android 12's splash icon: a 288dp canvas, shown inside a 192dp
			// circle.
			files.push((
				"drawable-nodpi/leptoskit_launch_icon.png".to_owned(),
				image.padded_png(SPLASH_CANVAS * 4, SPLASH_CIRCLE * 4)?,
			));
			format!(
				"    <item android:gravity=\"center\" android:width=\"{size}dp\" \
				 android:height=\"{size}dp\" \
				 android:drawable=\"@drawable/leptoskit_launch_image\" />\n"
			)
		} else {
			String::new()
		};

		files.push((
			"drawable/leptoskit_launch.xml".to_owned(),
			format!(
				"{HEADER}<layer-list \
				 xmlns:android=\"http://schemas.android.com/apk/res/android\">\n    <item \
				 android:drawable=\"@color/leptoskit_launch_background\" />\n{image_layer}</layer-list>\n"
			)
			.into_bytes(),
		));

		Ok(files)
	}
}

const HEADER: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!-- Generated by leptoskit \
                      from [app.launch_screen]. -->\n";

fn info() -> serde_json::Value {
	json!({ "author": "leptoskit", "version": 1 })
}

fn rgb(colour: &str) -> Result<[u8; 3]> {
	leptoskit_config::launch_colour(colour).with_context(|| {
		format!("app.launch_screen colour `{colour}` must be opaque hex such as \"#0b0c10\"")
	})
}

fn ios_color(colour: &str) -> Result<serde_json::Value> {
	let [red, green, blue] = rgb(colour)?;

	Ok(json!({
		"color-space": "srgb",
		"components": {
			"alpha": "1.000",
			"red": format!("0x{red:02X}"),
			"green": format!("0x{green:02X}"),
			"blue": format!("0x{blue:02X}"),
		},
	}))
}

fn android_color(colour: &str) -> Result<String> {
	let [red, green, blue] = rgb(colour)?;
	Ok(format!("#{red:02X}{green:02X}{blue:02X}"))
}

fn pretty(value: &serde_json::Value) -> Result<Vec<u8>> {
	let mut bytes = serde_json::to_vec_pretty(value)?;
	bytes.push(b'\n');
	Ok(bytes)
}

/// Writes `files` under `dir`.
///
/// # Errors
///
/// When a file can't be written.
pub fn write_files(dir: &Path, files: Vec<(String, Vec<u8>)>) -> Result<()> {
	for (name, bytes) in files {
		let path = dir.join(name);

		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent)?;
		}

		std::fs::write(&path, bytes)
			.with_context(|| format!("could not write {}", path.display()))?;
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use super::*;

	fn project(image: bool, dark: bool) -> (tempfile::TempDir, LeptoskitConfig) {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(
			dir.path().join("mark.svg"),
			r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><circle cx="32" cy="32" r="30" fill="#c33"/></svg>"##,
		)
		.unwrap();
		let mut config = LeptoskitConfig::default();
		config.app.launch_screen = Some(LaunchScreen {
			background: "#0b0c10".into(),
			background_dark: dark.then(|| "#000".into()),
			image: image.then(|| PathBuf::from("mark.svg")),
			image_size: 100,
		});
		(dir, config)
	}

	fn width(png: &[u8]) -> u32 {
		image::load_from_memory(png).unwrap().width()
	}

	#[test]
	fn ios_gets_a_colour_set_an_image_set_and_the_plist_naming_them() {
		let (dir, config) = project(true, true);
		let launch = Launch::load(dir.path(), &config).unwrap().unwrap();
		let files = launch.ios_files().unwrap();
		let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
		assert_eq!(
			names,
			[
				"Assets.xcassets/Contents.json",
				"Assets.xcassets/LaunchBackground.colorset/Contents.json",
				"Assets.xcassets/LaunchImage.imageset/launch@1x.png",
				"Assets.xcassets/LaunchImage.imageset/launch@2x.png",
				"Assets.xcassets/LaunchImage.imageset/launch@3x.png",
				"Assets.xcassets/LaunchImage.imageset/Contents.json",
			]
		);
		let colors: serde_json::Value = serde_json::from_slice(&files[1].1).unwrap();
		assert_eq!(colors["colors"][0]["color"]["components"]["red"], "0x0B");
		assert_eq!(colors["colors"][1]["appearances"][0]["value"], "dark");
		assert_eq!(colors["colors"][1]["color"]["components"]["blue"], "0x00");
		assert_eq!(
			[&files[2], &files[3], &files[4]].map(|(_, png)| width(png)),
			[100, 200, 300]
		);
		let images: serde_json::Value = serde_json::from_slice(&files[5].1).unwrap();
		assert_eq!(images["images"][2]["filename"], "launch@3x.png");

		let plist = launch.ios_plist();
		assert_eq!(
			plist.get("UIColorName").and_then(plist::Value::as_string),
			Some("LaunchBackground")
		);
		assert_eq!(
			plist.get("UIImageName").and_then(plist::Value::as_string),
			Some("LaunchImage")
		);
	}

	#[test]
	fn android_gets_a_splash_theme_and_an_earlier_window_background() {
		let (dir, config) = project(true, true);
		let launch = Launch::load(dir.path(), &config).unwrap().unwrap();
		let files = launch.android_files("@style/LeptosKitTheme").unwrap();
		let file = |name: &str| {
			files
				.iter()
				.find(|(path, _)| path == name)
				.map_or_else(|| panic!("{name} is missing"), |(_, bytes)| bytes.clone())
		};
		let text = |name: &str| String::from_utf8(file(name)).unwrap();

		let values = text("values/leptoskit_launch.xml");
		assert!(values.contains(">#0B0C10</color>"), "{values}");
		assert!(
			values.contains("name=\"LeptosKitLaunchTheme\" parent=\"@style/LeptosKitTheme\""),
			"{values}"
		);
		assert!(values.contains("@drawable/leptoskit_launch<"), "{values}");
		let splash = text("values-v31/leptoskit_launch.xml");
		assert!(splash.contains("windowSplashScreenBackground"), "{splash}");
		assert!(
			splash.contains("@drawable/leptoskit_launch_icon"),
			"{splash}"
		);
		assert!(!splash.contains("windowBackground\""), "{splash}");
		assert!(text("values-night/leptoskit_launch_colors.xml").contains(">#000000<"));
		let layers = text("drawable/leptoskit_launch.xml");
		assert!(
			layers.contains("android:width=\"100dp\"") && layers.contains("leptoskit_launch_image"),
			"{layers}"
		);
		assert_eq!(
			width(&file("drawable-nodpi/leptoskit_launch_image.png")),
			400
		);
		let icon = image::load_from_memory(&file("drawable-nodpi/leptoskit_launch_icon.png"))
			.unwrap()
			.to_rgba8();
		assert_eq!(icon.width(), 1152);
		// The canvas's corners stay clear of the circle Android shows.
		assert_eq!(icon.get_pixel(0, 0)[3], 0);

		for (_, bytes) in files.iter().filter(|(name, _)| {
			Path::new(name)
				.extension()
				.is_some_and(|extension| extension == "xml")
		}) {
			roxmltree::Document::parse(std::str::from_utf8(bytes).unwrap()).unwrap();
		}
	}

	#[test]
	fn a_colour_alone_is_a_launch_screen_and_no_table_is_none() {
		let (dir, config) = project(false, false);
		let launch = Launch::load(dir.path(), &config).unwrap().unwrap();
		assert!(!launch.has_image());
		assert_eq!(launch.ios_files().unwrap().len(), 2);
		assert!(launch.ios_plist().get("UIImageName").is_none());
		let android = launch.android_files("@style/LeptosKitTheme").unwrap();
		assert!(android.iter().all(|(name, _)| {
			Path::new(name)
				.extension()
				.is_none_or(|extension| extension != "png")
				&& !name.contains("night")
		}));
		assert!(
			!String::from_utf8_lossy(&android[1].1).contains("AnimatedIcon"),
			"Android 12 shows the app icon when there's no image"
		);

		assert!(
			Launch::load(dir.path(), &LeptoskitConfig::default())
				.unwrap()
				.is_none()
		);
	}

	#[test]
	fn a_missing_image_names_the_key() {
		let (dir, mut config) = project(true, false);
		config.app.launch_screen.as_mut().unwrap().image = Some(PathBuf::from("gone.png"));
		let error = Launch::load(dir.path(), &config).err().unwrap();
		assert!(
			format!("{error:#}").contains("app.launch_screen.image gone.png"),
			"{error:#}"
		);
	}
}
