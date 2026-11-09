//! Kotlin reaches Rust in two hops: an `external fun` binds by name to a
//! `Java_…` function in the JNI library, and that function calls an `lk_…`
//! function the Rust library exports. A Kotlin member with no JNI function
//! behind it compiles, links and passes every host test, then throws
//! `UnsatisfiedLinkError` on a device; called through reflection, it fails
//! without a sound. Android system Back once did exactly that, asking for a
//! `NativeApp.guestSystemBack` that existed nowhere, so every Back quit the
//! app. These tests hold both hops to what really exists.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// The directories whose Kotlin and C++ the generated Android app compiles.
const ANDROID_SOURCES: [&str; 2] = ["android", "mobile"];

fn root() -> &'static Path {
	Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Every file under the Android source directories with this extension.
fn sources(extension: &str) -> Vec<(PathBuf, String)> {
	let mut found = Vec::new();

	for directory in ANDROID_SOURCES {
		let mut paths: Vec<PathBuf> = fs::read_dir(root().join(directory))
			.unwrap()
			.map(|entry| entry.unwrap().path())
			.filter(|path| path.extension().is_some_and(|found| found == extension))
			.collect();
		paths.sort();

		for path in paths {
			let source = fs::read_to_string(&path).unwrap();
			found.push((path, source));
		}
	}

	assert!(!found.is_empty(), "no .{extension} sources found");
	found
}

/// Removes `/* … */` and `// …` comments, so names they mention are not
/// mistaken for code.
fn strip_comments(source: &str) -> String {
	let mut code = String::with_capacity(source.len());
	let mut rest = source;

	while !rest.is_empty() {
		if let Some(after) = rest.strip_prefix("/*") {
			rest = after.split_once("*/").map_or("", |(_, after)| after);
		} else if let Some(after) = rest.strip_prefix("//") {
			rest = after.find('\n').map_or("", |end| &after[end..]);
		} else {
			let next = rest.chars().next().unwrap();
			code.push(next);
			rest = &rest[next.len_utf8()..];
		}
	}

	code
}

fn is_identifier(character: char) -> bool {
	character.is_ascii_alphanumeric() || character == '_'
}

/// Identifiers starting with `prefix` and followed, after any spaces, by `(`.
fn called(source: &str, prefix: &str) -> BTreeSet<String> {
	let mut names = BTreeSet::new();

	for (start, _) in source.match_indices(prefix) {
		if source[..start]
			.chars()
			.next_back()
			.is_some_and(is_identifier)
		{
			continue;
		}

		let name: String = source[start..]
			.chars()
			.take_while(|c| is_identifier(*c))
			.collect();

		if source[start + name.len()..].trim_start().starts_with('(') {
			names.insert(name);
		}
	}

	names
}

/// A JNI identifier part: `_` is written `_1`.
fn escape(part: &str) -> String {
	part.replace('_', "_1")
}

/// Each Kotlin `external fun`, as `Object.method`, with the JNI function
/// name the JVM binds it to.
fn kotlin_externals() -> BTreeMap<String, String> {
	let mut externals = BTreeMap::new();

	for (path, source) in sources("kt") {
		let package = source
			.lines()
			.find_map(|line| line.strip_prefix("package "))
			.unwrap_or_else(|| panic!("{} has no package", path.display()))
			.trim();
		let mut owner: Option<&str> = None;

		for line in source.lines() {
			// A top-level object or class, at the start of a line.
			let declaration = ["internal ", "private ", "public "]
				.iter()
				.fold(line, |line, modifier| {
					line.strip_prefix(modifier).unwrap_or(line)
				});

			if let Some(rest) = declaration
				.strip_prefix("object ")
				.or_else(|| declaration.strip_prefix("class "))
			{
				owner = Some(rest.split(|c: char| !is_identifier(c)).next().unwrap());
				continue;
			}

			let Some((before, after)) = line.split_once("external fun ") else {
				continue;
			};
			let method = after.split('(').next().unwrap().trim();
			let owner = owner.unwrap_or_else(|| {
				panic!(
					"{}: `{method}` is outside an object or class",
					path.display()
				)
			});
			// The JNI name of a member nested deeper, such as in a companion
			// object, is spelled differently; teach this test before adding
			// one.
			assert!(
				before.len() - before.trim_start().len() == 4,
				"{}: `{owner}.{method}` is not a direct member of `{owner}`",
				path.display()
			);

			let package = package.split('.').map(escape).collect::<Vec<_>>().join("_");
			externals.insert(
				format!("{owner}.{method}"),
				format!("Java_{package}_{}_{}", escape(owner), escape(method)),
			);
		}
	}

	assert!(
		externals.contains_key("NativeApp.create"),
		"the Kotlin scan found no NativeApp externals"
	);
	externals
}

/// Every `Java_…` function the JNI library defines.
fn jni_functions() -> BTreeSet<String> {
	let functions: BTreeSet<String> = sources("cpp")
		.iter()
		.flat_map(|(_, source)| called(&strip_comments(source), "Java_"))
		.collect();
	assert!(!functions.is_empty(), "no JNI functions found");
	functions
}

#[test]
fn every_kotlin_external_has_a_jni_function() {
	let jni = jni_functions();
	let missing: Vec<String> = kotlin_externals()
		.into_iter()
		.filter(|(_, symbol)| !jni.contains(symbol))
		.map(|(member, symbol)| format!("{member} (expects {symbol})"))
		.collect();

	assert!(
		missing.is_empty(),
		"Kotlin declares native methods no JNI source defines; each would throw \
		 UnsatisfiedLinkError on a device:\n{}",
		missing.join("\n")
	);
}

#[test]
fn every_bridge_function_jni_calls_is_declared_and_exported() {
	let header =
		strip_comments(&fs::read_to_string(root().join("include/leptoskit_bridge.h")).unwrap());
	let declared = called(&header, "lk_");
	let mut exported = BTreeSet::new();
	let mut directories = vec![root().join("src")];

	while let Some(directory) = directories.pop() {
		for entry in fs::read_dir(directory).unwrap() {
			let path = entry.unwrap().path();

			if path.is_dir() {
				directories.push(path);
			} else if path.extension().is_some_and(|extension| extension == "rs") {
				let source = fs::read_to_string(&path).unwrap();

				for (start, _) in source.match_indices("extern \"C\" fn lk_") {
					let name = &source[start + "extern \"C\" fn ".len()..];
					let end = name.find(|c: char| !is_identifier(c)).unwrap();
					exported.insert(name[..end].to_owned());
				}
			}
		}
	}

	for (path, source) in sources("cpp") {
		for function in called(&strip_comments(&source), "lk_") {
			assert!(
				declared.contains(&function),
				"{} calls `{function}`, which include/leptoskit_bridge.h does not declare",
				path.display()
			);
		}
	}

	let missing: Vec<&String> = declared.difference(&exported).collect();
	assert!(
		missing.is_empty(),
		"include/leptoskit_bridge.h declares functions the Rust library does not export: \
		 {missing:?}"
	);
}

#[test]
fn no_kotlin_reaches_a_native_member_through_reflection() {
	let owners: BTreeSet<String> = kotlin_externals()
		.keys()
		.map(|member| member.split('.').next().unwrap().to_owned())
		.collect();

	for (path, source) in sources("kt") {
		for owner in &owners {
			assert!(
				!source.contains(&format!("{owner}::class.java")),
				"{} reflects on `{owner}`: call its natives directly, so a missing one fails the \
				 build and this test instead of returning nothing on a device",
				path.display()
			);
		}
	}
}

#[test]
fn system_back_reaches_the_guest_through_real_native_methods() {
	let view = fs::read_to_string(root().join("mobile/LeptosKitView.kt")).unwrap();
	let externals = kotlin_externals();
	let jni = jni_functions();

	for method in ["guestSystemBack", "guestTakeSystemBack"] {
		assert!(
			view.contains(&format!("NativeApp.{method}(guest)")),
			"LeptosKitView's system Back path no longer calls NativeApp.{method}"
		);
		let symbol = &externals[&format!("NativeApp.{method}")];
		assert!(jni.contains(symbol), "no JNI function {symbol}");
	}
}
