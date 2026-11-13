//! File ownership when several release units share one build output.

use std::collections::BTreeSet;

use super::Unit;

/// One independently published tree within a named build output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tree {
	pub unit: Unit,
	pub artifact: String,
	/// `.` for the whole output, otherwise a canonical slash-separated path.
	pub directory: String,
}

/// An unambiguous partition of named build outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trees {
	trees: Vec<Tree>,
}

impl Trees {
	/// Nested trees take ownership from their parent. Equal trees cannot
	/// have several owners. This is lexical ownership, not a filesystem
	/// containment or symlink check.
	///
	/// # Errors
	/// Rejects invalid names/paths, a unit declared twice, and several units
	/// claiming the same tree in one artifact.
	pub fn new(mut trees: Vec<Tree>) -> Result<Self, String> {
		let mut units = BTreeSet::new();
		let mut paths = BTreeSet::new();

		for tree in &trees {
			tree.unit.validate()?;

			if !super::artifact_identifier(&tree.artifact) {
				return Err(format!(
					"{} names invalid artifact {:?}",
					tree.unit, tree.artifact
				));
			}

			if tree.directory != "." && !super::components(&tree.directory) {
				return Err(format!(
					"{}.directory must be . or a canonical relative directory",
					tree.unit
				));
			}

			if !units.insert(tree.unit.clone()) {
				return Err(format!("{} has several artifact trees", tree.unit));
			}

			if !paths.insert((tree.artifact.clone(), tree.directory.clone())) {
				return Err(format!(
					"several units own artifact {} directory {}",
					tree.artifact, tree.directory
				));
			}
		}

		trees.sort();
		Ok(Self { trees })
	}

	/// Canonical declarations for build/release planning.
	#[must_use]
	pub fn entries(&self) -> &[Tree] {
		&self.trees
	}

	/// Assign a regular file to its most specific owning tree. The caller
	/// must first verify that the path really names a contained regular file.
	/// Unclaimed outputs return `None`; they must not enter a release by
	/// falling back to another artifact or unit.
	///
	/// # Errors
	/// Rejects noncanonical paths and a file occupying a declared directory.
	pub fn owner(&self, artifact: &str, file: &str) -> Result<Option<&Tree>, String> {
		if !super::components(file) {
			return Err(format!(
				"artifact member {file:?} is not a canonical relative file"
			));
		}

		let mut owner: Option<&Tree> = None;

		for tree in self.trees.iter().filter(|tree| tree.artifact == artifact) {
			if tree.directory == file {
				return Err(format!(
					"artifact member {file:?} is a file where {} needs a directory",
					tree.unit
				));
			}

			if inside(&tree.directory, file)
				&& owner.is_none_or(|old| specificity(tree) > specificity(old))
			{
				owner = Some(tree);
			}
		}

		Ok(owner)
	}

	/// Child subtrees excluded from a unit's inventory. Only immediate
	/// exclusions are needed: excluding `docs` also excludes `docs/play`.
	///
	/// # Errors
	/// Rejects a unit absent from this validated layout.
	pub fn exclusions(&self, unit: &Unit) -> Result<Vec<String>, String> {
		let parent = self
			.trees
			.iter()
			.find(|tree| &tree.unit == unit)
			.ok_or_else(|| format!("{unit} has no artifact tree"))?;
		let mut children: Vec<_> = self
			.trees
			.iter()
			.filter(|tree| {
				tree.artifact == parent.artifact
					&& tree.unit != parent.unit
					&& inside(&parent.directory, &tree.directory)
			})
			.collect();
		children.sort_by_key(|tree| specificity(tree));
		let mut exclusions: Vec<String> = Vec::new();

		for child in children {
			let relative = if parent.directory == "." {
				child.directory.as_str()
			} else {
				&child.directory[parent.directory.len() + 1..]
			};

			if !exclusions.iter().any(|existing| inside(existing, relative)) {
				exclusions.push(relative.to_owned());
			}
		}

		exclusions.sort();
		Ok(exclusions)
	}
}

fn specificity(tree: &Tree) -> usize {
	if tree.directory == "." {
		0
	} else {
		tree.directory.len()
	}
}

fn inside(directory: &str, file: &str) -> bool {
	directory == "."
		|| file
			.strip_prefix(directory)
			.is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tree(name: &str, directory: &str) -> Tree {
		Tree {
			unit: Unit::Frontend(name.into()),
			artifact: "public-site".into(),
			directory: directory.into(),
		}
	}

	#[test]
	fn one_build_has_disjoint_landing_docs_and_playground_releases() {
		let trees = Trees::new(vec![
			tree("site", "."),
			tree("docs", "docs"),
			tree("play", "play"),
		])
		.unwrap();
		for (file, owner) in [
			("index.html", "site"),
			("pkg/app.wasm", "site"),
			("docs/index.html", "docs"),
			("docs/guide/a.html", "docs"),
			("play/frame.mjs", "play"),
			("docs2/file", "site"),
		] {
			assert_eq!(
				trees
					.owner("public-site", file)
					.unwrap()
					.unwrap()
					.unit
					.name(),
				owner
			);
		}
		assert_eq!(
			trees.exclusions(&Unit::Frontend("site".into())).unwrap(),
			["docs", "play"]
		);
		assert_eq!(
			trees.exclusions(&Unit::Frontend("docs".into())).unwrap(),
			Vec::<String>::new()
		);
	}

	#[test]
	fn nested_mounts_exclude_each_file_from_all_parent_inventories() {
		let trees = Trees::new(vec![
			tree("site", "."),
			tree("docs", "docs"),
			tree("examples", "docs/play"),
		])
		.unwrap();
		assert_eq!(
			trees
				.owner("public-site", "docs/play/demo.wasm")
				.unwrap()
				.unwrap()
				.unit
				.name(),
			"examples"
		);
		assert_eq!(
			trees.exclusions(&Unit::Frontend("site".into())).unwrap(),
			["docs"]
		);
		assert_eq!(
			trees.exclusions(&Unit::Frontend("docs".into())).unwrap(),
			["play"]
		);
	}

	#[test]
	fn a_new_independent_recipe_does_not_capture_another_artifacts_files() {
		let mut docs = tree("docs", ".");
		docs.artifact = "book".into();
		let trees = Trees::new(vec![tree("site", "."), docs]).unwrap();
		assert_eq!(
			trees
				.owner("book", "index.html")
				.unwrap()
				.unwrap()
				.unit
				.name(),
			"docs"
		);
		assert_eq!(
			trees
				.owner("public-site", "index.html")
				.unwrap()
				.unwrap()
				.unit
				.name(),
			"site"
		);
		assert!(trees.owner("unknown", "index.html").unwrap().is_none());
		assert_eq!(
			trees.exclusions(&Unit::Frontend("site".into())).unwrap(),
			Vec::<String>::new()
		);
	}

	#[test]
	fn an_unclaimed_tree_is_not_implicitly_published() {
		let trees = Trees::new(vec![tree("docs", "docs")]).unwrap();
		assert!(
			trees
				.owner("public-site", "private/config")
				.unwrap()
				.is_none()
		);
	}

	#[test]
	fn duplicate_units_paths_and_directory_file_collisions_are_errors() {
		assert!(Trees::new(vec![tree("docs", "docs"), tree("other", "docs")]).is_err());
		assert!(Trees::new(vec![tree("docs", "docs"), tree("docs", "book")]).is_err());
		let trees = Trees::new(vec![tree("site", "."), tree("docs", "docs")]).unwrap();
		assert!(trees.owner("public-site", "docs").is_err());
	}

	#[test]
	fn ambiguous_or_escaping_members_never_gain_an_owner() {
		let trees = Trees::new(vec![tree("site", ".")]).unwrap();
		for path in [
			"",
			".",
			"..",
			"../file",
			"/file",
			"docs/../file",
			"docs//file",
			"docs\\file",
			"docs/./file",
			"docs/file/",
			"docs/\0file",
		] {
			assert!(trees.owner("public-site", path).is_err(), "{path:?}");

			if path != "." {
				assert!(Trees::new(vec![tree("site", path)]).is_err(), "{path:?}");
			}
		}
	}

	#[test]
	fn spaces_and_unicode_in_artifact_files_are_not_url_normalized() {
		let trees = Trees::new(vec![tree("site", "."), tree("docs", "My docs")]).unwrap();
		assert_eq!(
			trees
				.owner("public-site", "My docs/Überblick.html")
				.unwrap()
				.unwrap()
				.unit
				.name(),
			"docs"
		);
		assert_eq!(
			trees
				.owner("public-site", "100%done.txt")
				.unwrap()
				.unwrap()
				.unit
				.name(),
			"site"
		);
	}

	#[test]
	fn order_does_not_change_release_ownership() {
		let input = vec![
			tree("site", "."),
			tree("docs", "docs"),
			tree("play", "play"),
		];
		let expected = Trees::new(input.clone()).unwrap();
		assert_eq!(
			Trees::new(input.into_iter().rev().collect()).unwrap(),
			expected
		);
	}
}
