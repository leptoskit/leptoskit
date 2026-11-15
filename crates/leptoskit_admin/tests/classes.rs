//! Every class string written as a literal in the admin's source compiles.
//!
//! An unsupported Tailwind class panics when its element is built, which on
//! a server means a dropped connection for one page. Reading the literals
//! here finds them all without rendering every screen in every state.

use std::fs;
use std::path::Path;

use leptoskit::prelude::*;

/// The string literal starting at `text`, with `\`-newline continuations
/// folded the way Rust folds them.
fn literal(text: &str) -> Option<String> {
	let rest = text.strip_prefix('"')?;
	let mut value = String::new();
	let mut chars = rest.chars().peekable();

	while let Some(character) = chars.next() {
		match character {
			'"' => return Some(value),
			'\\' => {
				match chars.next()? {
					'\n' => {
						while chars.peek().is_some_and(|next| next.is_whitespace()) {
							chars.next();
						}
					}
					other => value.push(other),
				}
			}
			other => value.push(other),
		}
	}

	None
}

/// The `.class("…")` literals in one file.
fn classes(source: &str) -> Vec<String> {
	let mut found = Vec::new();

	for (index, _) in source.match_indices(".class(") {
		let after = source[index + ".class(".len()..].trim_start();

		if let Some(value) = literal(after) {
			found.push(value);
		}
	}

	found
}

fn sources(directory: &Path, files: &mut Vec<std::path::PathBuf>) {
	for entry in fs::read_dir(directory).expect("source directory") {
		let path = entry.expect("entry").path();

		if path.is_dir() {
			sources(&path, files);
		} else if path.extension().is_some_and(|extension| extension == "rs") {
			files.push(path);
		}
	}
}

#[test]
fn every_class_literal_compiles() {
	let mut files = Vec::new();
	sources(
		&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
		&mut files,
	);
	let mut checked = 0;
	let mut failures = Vec::new();

	for file in &files {
		let source = fs::read_to_string(file).expect("source");

		for class in classes(&source) {
			checked += 1;

			// `active:` needs a Button or Pressable; the receiver isn't known
			// here, so a class a Pressable accepts passes.
			if let Err(error) = View()
				.try_class(&class)
				.or_else(|_| Pressable("check").try_class(&class))
			{
				failures.push(format!("{}: `{class}`: {error}", file.display()));
			}
		}
	}

	assert!(checked > 100, "found only {checked} class literals");
	assert!(
		failures.is_empty(),
		"classes that don't compile:\n{}",
		failures.join("\n")
	);
}
