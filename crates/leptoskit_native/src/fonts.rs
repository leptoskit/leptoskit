//! The typefaces an app bundles.
//!
//! An app declares its fonts once, in `leptoskit.toml`:
//!
//! ```toml
//! [theme.fonts.hand]
//! family = "Skribble Noto Sketch"
//! faces = [
//!   { file = "fonts/NotoSketch-Regular.ttf" },
//!   { file = "fonts/NotoSketch-Bold.ttf", weight = 700 },
//! ]
//! ```
//!
//! and chooses one with the class it would use on the web,
//! `font-[var(--hand)]`. The build checks every file and writes the
//! declarations as a [`Fonts`] manifest beside the app; each native host
//! registers the files and [installs](install) the manifest before its first
//! layout, so the Tailwind compiler can resolve `var(--hand)` to
//! [`FontFamily::Declared`](crate::FontFamily::Declared), and
//! [`NativeProps::text_style`](crate::NativeProps::text_style) can choose the
//! [`Face`] that both measurement and drawing use.
//!
//! The manifest is process-wide, like the files a platform registers: it is
//! installed once, and a second install of the same fonts changes nothing.

use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Every font an app declares, in declaration order.
///
/// A face is named by its position in [`faces`](Self::faces): the families'
/// faces, one family after another.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Fonts {
	/// The declared families.
	pub families: Vec<DeclaredFamily>,
}

/// One `[theme.fonts.<name>]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DeclaredFamily {
	/// The custom property's name without its leading `--`: `hand` for
	/// `font-[var(--hand)]`.
	pub name: String,
	/// The family name the web stylesheet declares.
	pub family: String,
	/// The family's files, at least one.
	pub faces: Vec<FontFace>,
}

/// One file of a [`DeclaredFamily`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FontFace {
	/// The file's path inside the app's `assets` folder, such as
	/// `fonts/NotoSketch-Regular.ttf`.
	pub file: String,
	/// The weights the file draws, `[min, max]`: equal for a static face, the
	/// `wght` axis of a variable one.
	pub weight: [u16; 2],
	/// Whether the file is italic.
	#[cfg_attr(feature = "serde", serde(default))]
	pub italic: bool,
}

impl FontFace {
	/// Whether the file is a variable font whose `wght` axis the host sets.
	#[must_use]
	pub const fn is_variable(&self) -> bool {
		self.weight[0] != self.weight[1]
	}
}

/// The face a run of text draws with: which declared file, and the weight to
/// draw it at.
///
/// `weight` is the file's own weight for a static face, and the requested
/// weight clamped to the axis for a variable one. Hosts never synthesize
/// bold or italic from a face: the face chosen is what draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Face {
	/// The face's position in [`Fonts::faces`].
	pub index: u16,
	/// The weight to draw, from 1 to 1000.
	pub weight: u16,
}

impl Fonts {
	/// Every face, in declaration order: the position of a face here is its
	/// [`Face::index`].
	pub fn faces(&self) -> impl Iterator<Item = &FontFace> {
		self.families.iter().flat_map(|family| &family.faces)
	}

	/// The face at `index`.
	#[must_use]
	pub fn face(&self, index: u16) -> Option<&FontFace> {
		self.faces().nth(usize::from(index))
	}

	/// The family `var(--name)` chooses. `name` may carry its leading `--`.
	#[must_use]
	pub fn family(&self, name: &str) -> Option<u16> {
		let name = name.trim().trim_start_matches('-');
		let index = self
			.families
			.iter()
			.position(|family| family.name == name)?;

		u16::try_from(index).ok()
	}

	/// Chooses the face of `family` that draws `weight` and `italic`, as a
	/// browser's font matching does.
	///
	/// Style first: the faces in the requested style, or the other style
	/// when the family has none. Then weight: a face that draws it; else, for
	/// a weight from 400 to 500, the nearest heavier face up to 500, then
	/// the nearest lighter, then the nearest heavier; below 400 the nearest
	/// lighter, then heavier; above 500 the nearest heavier, then lighter.
	/// Equal candidates go to the one declared first.
	///
	/// `None` when `family` names no declared family.
	#[must_use]
	pub fn select(&self, family: u16, weight: u16, italic: bool) -> Option<Face> {
		let declared = self.families.get(usize::from(family))?;
		let first = self.families[..usize::from(family)]
			.iter()
			.map(|family| family.faces.len())
			.sum::<usize>();
		let has_style = declared.faces.iter().any(|face| face.italic == italic);
		let (position, face) = declared
			.faces
			.iter()
			.enumerate()
			.filter(|(_, face)| !has_style || face.italic == italic)
			.min_by_key(|(_, face)| weight_rank(weight, face.weight))?;
		let index = u16::try_from(first + position).ok()?;

		Some(Face {
			index,
			weight: weight.clamp(face.weight[0], face.weight[1]),
		})
	}

	/// Parses a manifest the build wrote.
	///
	/// # Errors
	///
	/// Returns the JSON error when `json` is not a manifest.
	#[cfg(feature = "serde")]
	pub fn from_json(json: &[u8]) -> Result<Self, serde_json::Error> {
		serde_json::from_slice(json)
	}
}

/// How far a face drawing `range` is from the `requested` weight, in the
/// order CSS font matching prefers: lower is better.
fn weight_rank(requested: u16, [min, max]: [u16; 2]) -> (u8, u16) {
	if (min..=max).contains(&requested) {
		return (0, 0);
	}

	let heavier = min > requested;
	let distance = if heavier {
		min - requested
	} else {
		requested - max
	};
	let tier = match (requested, heavier) {
		(400..=500, true) if min <= 500 => 1,
		(400..=500, true) => 3,
		(..400, false) | (501.., true) => 1,
		(400..=500 | 501.., false) | (..400, true) => 2,
	};

	(tier, distance)
}

/// The fonts this process has installed, and where their files are.
#[derive(Debug, PartialEq)]
struct Installed {
	fonts: Fonts,
	root: PathBuf,
}

static INSTALLED: OnceLock<Installed> = OnceLock::new();

/// Installs the app's fonts for this process.
///
/// `root` is the folder the manifest's paths are relative to: the app's
/// bundled `assets` folder. Hosts call this once, before their first layout;
/// installing the same fonts again changes nothing.
///
/// # Errors
///
/// Returns [`InstallError`] when different fonts were installed already,
/// since a platform cannot unregister the faces it has drawn with.
pub fn install(fonts: Fonts, root: impl Into<PathBuf>) -> Result<&'static Fonts, InstallError> {
	let refused = INSTALLED
		.set(Installed {
			fonts,
			root: root.into(),
		})
		.err();
	// Set by this call or an earlier one.
	let installed = INSTALLED.get().ok_or(InstallError)?;

	match refused {
		Some(offered) if offered != *installed => Err(InstallError),
		_ => Ok(&installed.fonts),
	}
}

/// The fonts this process installed, if any.
#[must_use]
pub fn installed() -> Option<&'static Fonts> {
	INSTALLED.get().map(|installed| &installed.fonts)
}

/// Where the file of the installed face at `index` is.
#[must_use]
pub fn path(index: u16) -> Option<PathBuf> {
	let installed = INSTALLED.get()?;
	let face = installed.fonts.face(index)?;

	Some(join(&installed.root, &face.file))
}

fn join(root: &Path, file: &str) -> PathBuf {
	file.split('/')
		.fold(root.to_path_buf(), |path, segment| path.join(segment))
}

/// Fonts were installed twice, differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallError;

impl fmt::Display for InstallError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str("this process installed different fonts already")
	}
}

impl std::error::Error for InstallError {}

#[cfg(test)]
mod tests {
	use super::*;

	fn face(file: &str, weight: [u16; 2], italic: bool) -> FontFace {
		FontFace {
			file: file.to_owned(),
			weight,
			italic,
		}
	}

	fn family(name: &str, faces: Vec<FontFace>) -> DeclaredFamily {
		DeclaredFamily {
			name: name.to_owned(),
			family: name.to_owned(),
			faces,
		}
	}

	fn statics() -> Fonts {
		Fonts {
			families: vec![
				family("mono", vec![face("mono.ttf", [400, 400], false)]),
				family(
					"hand",
					vec![
						face("light.ttf", [300, 300], false),
						face("regular.ttf", [400, 400], false),
						face("bold.ttf", [700, 700], false),
						face("black.ttf", [900, 900], false),
						face("italic.ttf", [400, 400], true),
					],
				),
			],
		}
	}

	#[test]
	fn faces_are_numbered_across_families() {
		let fonts = statics();

		assert_eq!(fonts.faces().count(), 6);
		assert_eq!(
			fonts.face(0).map(|face| face.file.as_str()),
			Some("mono.ttf")
		);
		assert_eq!(
			fonts.face(3).map(|face| face.file.as_str()),
			Some("bold.ttf")
		);
		assert_eq!(fonts.face(6), None);
		assert_eq!(fonts.family("--hand"), Some(1));
		assert_eq!(fonts.family("mono"), Some(0));
		assert_eq!(fonts.family("--body"), None);
	}

	#[test]
	fn an_exact_weight_and_style_wins() {
		let fonts = statics();

		assert_eq!(
			fonts.select(1, 700, false),
			Some(Face {
				index: 3,
				weight: 700
			})
		);
		assert_eq!(
			fonts.select(1, 400, true),
			Some(Face {
				index: 5,
				weight: 400
			})
		);
		assert_eq!(fonts.select(2, 400, false), None);
	}

	#[test]
	fn missing_weights_fall_back_the_way_browsers_do() {
		let fonts = statics();
		let index = |weight| fonts.select(1, weight, false).map(|face| face.index);

		// From 400 to 500: heavier up to 500, then lighter, then heavier.
		assert_eq!(index(500), Some(2));
		assert_eq!(index(450), Some(2));
		// Below 400: lighter first, then heavier.
		assert_eq!(index(350), Some(1));
		assert_eq!(index(200), Some(1));
		assert_eq!(index(100), Some(1));
		// Above 500: heavier first, then lighter.
		assert_eq!(index(600), Some(3));
		assert_eq!(index(800), Some(4));
		assert_eq!(index(1000), Some(4));
	}

	#[test]
	fn a_missing_style_falls_back_to_the_other() {
		let fonts = statics();

		// The only italic face draws every italic weight.
		assert_eq!(fonts.select(1, 900, true).map(|face| face.index), Some(5));
		// The mono family has no italic, so its upright face draws italic.
		assert_eq!(fonts.select(0, 400, true).map(|face| face.index), Some(0));
		let italic_only = Fonts {
			families: vec![family("slant", vec![face("slant.ttf", [400, 400], true)])],
		};
		assert_eq!(
			italic_only.select(0, 700, false),
			Some(Face {
				index: 0,
				weight: 400
			})
		);
	}

	#[test]
	fn variable_faces_draw_the_requested_weight_within_their_axis() {
		let fonts = Fonts {
			families: vec![family(
				"variable",
				vec![
					face("upright.ttf", [300, 800], false),
					face("heavy.ttf", [900, 1000], false),
				],
			)],
		};

		assert_eq!(
			fonts.select(0, 650, false),
			Some(Face {
				index: 0,
				weight: 650
			})
		);
		assert_eq!(
			fonts.select(0, 100, false),
			Some(Face {
				index: 0,
				weight: 300
			})
		);
		assert_eq!(
			fonts.select(0, 850, false),
			Some(Face {
				index: 1,
				weight: 900
			})
		);
		assert!(fonts.face(0).is_some_and(FontFace::is_variable));
		assert!(!statics().face(0).is_some_and(FontFace::is_variable));
	}

	#[test]
	fn ties_go_to_the_face_declared_first() {
		let fonts = Fonts {
			families: vec![family(
				"twins",
				vec![
					face("first.ttf", [400, 400], false),
					face("second.ttf", [400, 400], false),
				],
			)],
		};

		assert_eq!(fonts.select(0, 400, false).map(|face| face.index), Some(0));
	}

	#[test]
	fn the_ranking_orders_weights_as_css_does() {
		assert_eq!(weight_rank(400, [400, 400]), (0, 0));
		assert_eq!(weight_rank(400, [500, 500]), (1, 100));
		assert_eq!(weight_rank(400, [300, 300]), (2, 100));
		assert_eq!(weight_rank(400, [600, 600]), (3, 200));
		assert_eq!(weight_rank(300, [200, 200]), (1, 100));
		assert_eq!(weight_rank(300, [400, 400]), (2, 100));
		assert_eq!(weight_rank(600, [700, 700]), (1, 100));
		assert_eq!(weight_rank(600, [500, 500]), (2, 100));
	}

	#[test]
	fn paths_join_the_root_one_segment_at_a_time() {
		assert_eq!(
			join(Path::new("bundle/assets"), "fonts/Body.ttf"),
			Path::new("bundle/assets").join("fonts").join("Body.ttf")
		);
	}

	#[test]
	fn the_install_error_explains_itself() {
		assert_eq!(
			InstallError.to_string(),
			"this process installed different fonts already"
		);
	}
}
