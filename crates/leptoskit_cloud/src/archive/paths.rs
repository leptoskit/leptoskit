//! The path rules packing and unpacking share.

use std::collections::BTreeMap;
use std::collections::VecDeque;

use super::ALWAYS_EXCLUDED;

/// How many symlinks one resolution may follow before it is refused as a
/// loop, as Linux's `ELOOP` limit.
const MAX_LINK_HOPS: usize = 40;

/// What is wrong with a path in an archive, or `None` when it is a
/// relative, `/`-separated path with no empty, `.`, or `..` component and no
/// backslash or NUL.
pub(super) fn path_problem(path: &str) -> Option<&'static str> {
	if path.is_empty() {
		return Some("is empty");
	}

	if path.starts_with('/') || has_drive(path) {
		return Some("is absolute");
	}

	if path.contains(['\\', '\0']) {
		return Some("has a backslash or NUL");
	}

	for component in path.split('/') {
		match component {
			".." => return Some("climbs out with `..`"),
			"" | "." => return Some("has an empty or `.` component"),
			_ => {}
		}
	}

	None
}

/// Whether any component of `path` is one of [`ALWAYS_EXCLUDED`].
pub(super) fn always_excluded(path: &str) -> bool {
	path.split('/')
		.any(|component| ALWAYS_EXCLUDED.contains(&component))
}

/// Whether `path` names a file that usually holds secrets: `.env`,
/// `.env.*`, `*.pem`, `*.p8`, `*.p12`, `*.jks`, `*.keystore`, or
/// `credentials*.toml`, in any case.
pub(super) fn looks_secret(path: &str) -> bool {
	let name = path
		.rsplit_once('/')
		.map_or(path, |(_, name)| name)
		.to_ascii_lowercase();

	let extension = name.rsplit_once('.').map(|(_, extension)| extension);

	name == ".env"
		|| name.starts_with(".env.")
		|| extension
			.is_some_and(|extension| ["pem", "p8", "p12", "jks", "keystore"].contains(&extension))
		|| (name.starts_with("credentials") && extension == Some("toml"))
}

/// Whether the symlink at `link` stays inside the archive's root when it is
/// followed, through any of the archive's other symlinks (`links`, path to
/// target) on the way.
///
/// Following is done on the archive's own paths, never the filesystem's, so
/// packing and unpacking judge a link the same way. Climbing above the root
/// at any point counts as leaving it, even if a later component would come
/// back down.
pub(super) fn link_stays_inside(link: &str, links: &BTreeMap<String, String>) -> bool {
	let Some(target) = links.get(link) else {
		return false;
	};

	if !relative_target(target) {
		return false;
	}

	let mut current: Vec<&str> = link.split('/').collect();
	current.pop();
	let mut pending: VecDeque<&str> = target.split('/').collect();
	let mut hops = 0;

	while let Some(component) = pending.pop_front() {
		match component {
			"" | "." => {}
			".." => {
				if current.pop().is_none() {
					return false;
				}
			}
			name => {
				current.push(name);

				let Some(next) = links.get(&current.join("/")) else {
					continue;
				};

				hops += 1;

				if hops > MAX_LINK_HOPS || !relative_target(next) {
					return false;
				}

				current.pop();

				for component in next.split('/').rev() {
					pending.push_front(component);
				}
			}
		}
	}

	true
}

/// Whether a symlink's target is a non-empty relative path in `/` form.
fn relative_target(target: &str) -> bool {
	!target.is_empty()
		&& !target.starts_with('/')
		&& !has_drive(target)
		&& !target.contains(['\\', '\0'])
}

/// Whether `path` starts with a Windows drive, such as `C:`.
fn has_drive(path: &str) -> bool {
	let mut characters = path.chars();
	matches!(
		(characters.next(), characters.next()),
		(Some(letter), Some(':')) if letter.is_ascii_alphabetic()
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn links(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
		pairs
			.iter()
			.map(|(path, target)| ((*path).to_owned(), (*target).to_owned()))
			.collect()
	}

	#[test]
	fn paths_are_relative_and_plain() {
		for path in ["a", "a/b.rs", ".github/workflows/ci.yml", "a..b", "...x"] {
			assert_eq!(path_problem(path), None, "{path}");
		}

		for path in [
			"",
			"/etc/passwd",
			"C:/x",
			"a/../b",
			"..",
			"a//b",
			"./a",
			"a/",
			"a\\b",
			"a\0b",
		] {
			assert!(path_problem(path).is_some(), "{path:?}");
		}
	}

	#[test]
	fn excluded_directories_match_any_component() {
		for path in [
			"target/debug/app",
			"crates/a/target/x",
			".git/config",
			"web/node_modules/x/index.js",
			"dist/app.js",
			".leptoskit/db/x",
			".git",
		] {
			assert!(always_excluded(path), "{path}");
		}

		for path in ["targets/x", "src/distance.rs", "my.git/x"] {
			assert!(!always_excluded(path), "{path}");
		}
	}

	#[test]
	fn secret_files_are_recognised_by_name() {
		for path in [
			".env",
			"config/.env.production",
			".env.example",
			"keys/server.pem",
			"AuthKey_ABC.p8",
			"ios/dist.P12",
			"android/release.jks",
			"android/upload.keystore",
			"credentials.toml",
			"deploy/credentials-prod.toml",
		] {
			assert!(looks_secret(path), "{path}");
		}

		for path in [
			"src/env.rs",
			".envrc",
			"pem.rs",
			"credentials.md",
			"my-credentials.toml",
			"docs/keystore.md",
		] {
			assert!(!looks_secret(path), "{path}");
		}
	}

	#[test]
	fn links_inside_the_root_resolve() {
		let inside = links(&[
			("a", "b"),
			("docs/readme.md", "../readme.md"),
			("x/y/z", "../../a"),
			("dangling", "missing/file"),
		]);

		for link in inside.keys() {
			assert!(link_stays_inside(link, &inside), "{link}");
		}
	}

	#[test]
	fn links_that_leave_the_root_are_refused() {
		let cases = [
			links(&[("a", "..")]),
			links(&[("a", "../x")]),
			links(&[("d/a", "../../x")]),
			links(&[("a", "/etc/passwd")]),
			links(&[("a", "C:/Windows")]),
			links(&[("a", "")]),
			links(&[("a", "x\\..\\..")]),
			// Lexically `sub/x`, but `sub/b` is the root, so `..` leaves it.
			links(&[("sub/b", ".."), ("a", "sub/b/../x")]),
			// Out through a chain of links.
			links(&[("a", "b"), ("b", "c/.."), ("c", "../..")]),
			// A loop.
			links(&[("a", "b"), ("b", "a")]),
		];

		for case in cases {
			assert!(!link_stays_inside("a", &case), "{case:?}");
		}
	}
}
