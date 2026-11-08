//! Files an app ships in its `assets/` folder, named one way on every
//! platform.
//!
//! An [`Image`](crate::components::Image) whose source is
//! `asset://portraits/ada.webp` shows `assets/portraits/ada.webp`: the browser
//! loads it from `/assets/portraits/ada.webp`, where the web build serves that
//! folder, and the native apps read it from the copy `leptoskit build` puts in
//! the app bundle.
//!
//! | Platform        | Where the folder is                         |
//! | --------------- | ------------------------------------------- |
//! | web             | `/assets/` on the page's origin             |
//! | iOS             | `assets/` in the app bundle                 |
//! | macOS           | `Contents/Resources/assets/`                |
//! | Android         | `assets/` inside the APK's assets           |
//! | Linux, Windows  | `assets/` beside the executable             |
//! | HarmonyOS       | `assets/` in the HAP's `rawfile` resources  |
//!
//! [`asset_path`] is what every native host uses to read a source, so a path
//! that would leave the folder is refused the same way everywhere.

use std::borrow::Cow;
use std::fmt;

/// What begins every asset source.
pub const SCHEME: &str = "asset://";

/// The folder of the app's project, of the web build's output, and of each
/// native bundle that holds the assets.
pub const FOLDER: &str = "assets";

/// Why an `asset://` source names no file in the assets folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidAsset {
	source: String,
	reason: &'static str,
}

impl InvalidAsset {
	/// The source as it was written.
	#[must_use]
	pub fn source(&self) -> &str {
		&self.source
	}
}

impl fmt::Display for InvalidAsset {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{} is not an asset path: {}", self.source, self.reason)
	}
}

impl std::error::Error for InvalidAsset {}

/// The path inside the assets folder that `source` names: `None` when
/// `source` isn't an `asset://` source, and an error when it is one but
/// doesn't name a file inside the folder.
///
/// The path is taken as it is written: it is not percent-decoded, and it is
/// case-sensitive, as it is on Android, Linux and the web.
///
/// ```
/// use leptoskit_native::asset::asset_path;
///
/// assert_eq!(
/// 	asset_path("asset://portraits/ada.webp"),
/// 	Some(Ok("portraits/ada.webp"))
/// );
/// assert_eq!(asset_path("file:///tmp/ada.webp"), None);
/// assert!(asset_path("asset://../secrets.txt").unwrap().is_err());
/// ```
///
/// # Errors
///
/// When the path is empty, absolute, holds a backslash or a NUL, or has an
/// empty, `.`, or `..` segment.
#[must_use]
pub fn asset_path(source: &str) -> Option<Result<&str, InvalidAsset>> {
	let path = source.strip_prefix(SCHEME)?;
	let invalid = |reason| {
		Err(InvalidAsset {
			source: source.to_owned(),
			reason,
		})
	};

	Some(if path.is_empty() {
		invalid("it names no file")
	} else if path.contains(['\\', '\0']) {
		invalid("separate folders with `/`")
	} else if path
		.split('/')
		.any(|segment| matches!(segment, "" | "." | ".."))
	{
		invalid("it must stay inside the assets folder, without empty, `.` or `..` segments")
	} else {
		Ok(path)
	})
}

/// The source of the file at `path` inside the assets folder:
/// `asset("portraits/ada.webp")` is `"asset://portraits/ada.webp"`.
#[must_use]
pub fn asset(path: &str) -> String {
	format!("{SCHEME}{path}")
}

/// What the browser loads for `source`: an asset becomes its address under
/// `/assets/`, where the web build serves the folder, and every other source
/// is left as it is.
#[must_use]
pub fn web_url(source: &str) -> Cow<'_, str> {
	match asset_path(source) {
		Some(Ok(path)) => Cow::Owned(format!("/{FOLDER}/{path}")),
		_ => Cow::Borrowed(source),
	}
}

/// The assets folder of the running app's bundle, found from its
/// executable: `Contents/Resources/assets` in a macOS app, and the
/// `assets` folder beside the executable everywhere else, which on iOS is
/// the app bundle's own. Android keeps its assets inside the APK, where
/// only its `AssetManager` reads them.
///
/// # Errors
///
/// When the executable's path is unknown.
#[cfg(not(target_arch = "wasm32"))]
pub fn bundled_folder() -> std::io::Result<std::path::PathBuf> {
	let executable = std::env::current_exe()?;
	let directory = executable
		.parent()
		.ok_or_else(|| std::io::Error::other("the executable has no folder"))?;

	Ok(match directory.parent() {
		Some(contents) if cfg!(target_os = "macos") && directory.ends_with("Contents/MacOS") => {
			contents.join("Resources").join(FOLDER)
		}
		_ => directory.join(FOLDER),
	})
}

/// The file `source` names in the running app's bundled assets folder
/// ([`bundled_folder`]): `None` when `source` isn't an `asset://` source.
///
/// # Errors
///
/// When the path leaves the assets folder, or the bundle can't be found.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn bundled_file(source: &str) -> Option<Result<std::path::PathBuf, InvalidAsset>> {
	Some(asset_path(source)?.and_then(|path| {
		bundled_folder()
			.map(|folder| folder.join(path))
			.map_err(|_| {
				InvalidAsset {
					source: source.to_owned(),
					reason: "the app's bundle can't be found",
				}
			})
	}))
}

/// Loads the bundled file `source` names with `load`, for a native host:
/// `None` when `source` isn't an `asset://` source, so the host reads it its
/// own way, and `Some` of what `load` made of the file otherwise. A refused
/// path, or a file `load` can't read, is passed to [`report_missing`].
#[cfg(not(target_arch = "wasm32"))]
#[track_caller]
pub fn load_bundled<T>(
	source: &str,
	load: impl FnOnce(&std::path::Path) -> Option<T>,
) -> Option<Option<T>> {
	Some(match bundled_file(source)? {
		Ok(path) => {
			let loaded = load(&path);

			if loaded.is_none() {
				report_missing(
					source,
					&format_args!("{} is missing or isn't an image", path.display()),
				);
			}

			loaded
		}
		Err(error) => {
			report_missing(source, &error);
			None
		}
	})
}

/// Reports an asset a host could not show: `problem` says why. A debug build
/// panics, so the mistake is seen at once; a release build writes it to
/// standard error and leaves the image empty.
///
/// # Panics
///
/// In a debug build, always.
#[track_caller]
pub fn report_missing(source: &str, problem: &dyn fmt::Display) {
	let message = format!(
		"Image {source} can't be shown: {problem}. Put a PNG, JPEG, or WebP at {FOLDER}/<path> in \
		 the app's project; `leptoskit build` bundles that folder."
	);

	#[cfg(debug_assertions)]
	panic!("{message}");
	#[cfg(not(debug_assertions))]
	eprintln!("leptoskit: {message}");
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_paths_inside_the_folder_are_assets() {
		assert_eq!(asset_path("asset://ada.png"), Some(Ok("ada.png")));
		assert_eq!(
			asset_path("asset://nfts/degenApes_1.webp"),
			Some(Ok("nfts/degenApes_1.webp"))
		);
		assert_eq!(asset_path("assets/ada.png"), None);
		assert_eq!(asset_path("/assets/ada.png"), None);
		assert_eq!(asset_path("https://example.com/ada.png"), None);

		for source in [
			"asset://",
			"asset:///ada.png",
			"asset://nfts//ada.png",
			"asset://nfts/",
			"asset://./ada.png",
			"asset://nfts/../../ada.png",
			"asset://nfts\\ada.png",
			"asset://ada\0.png",
		] {
			let error = asset_path(source).unwrap().unwrap_err();
			assert_eq!(error.source(), source);
			assert!(error.to_string().starts_with(source), "{error}");
		}
	}

	#[test]
	fn the_browser_loads_assets_from_the_served_folder() {
		assert_eq!(web_url("asset://nfts/ada.webp"), "/assets/nfts/ada.webp");
		assert_eq!(asset("nfts/ada.webp"), "asset://nfts/ada.webp");
		assert_eq!(web_url(&asset("ada.png")), "/assets/ada.png");
		assert!(matches!(web_url("/ada.png"), Cow::Borrowed("/ada.png")));
		// A refused path is left for the browser to fail on, visibly.
		assert_eq!(web_url("asset://../ada.png"), "asset://../ada.png");
	}

	#[cfg(not(target_arch = "wasm32"))]
	#[test]
	fn native_hosts_read_the_folder_beside_the_executable() {
		let executable = std::env::current_exe().unwrap();
		assert_eq!(
			bundled_file("asset://nfts/ada.webp").unwrap().unwrap(),
			executable.parent().unwrap().join("assets/nfts/ada.webp")
		);
		assert!(bundled_file("asset://../ada.png").unwrap().is_err());
		assert!(bundled_file("file:///ada.png").is_none());
	}

	#[cfg(debug_assertions)]
	#[test]
	#[should_panic(expected = "Image asset://ada.png can't be shown: not found")]
	fn a_missing_asset_stops_a_debug_build() {
		report_missing("asset://ada.png", &"not found");
	}
}
