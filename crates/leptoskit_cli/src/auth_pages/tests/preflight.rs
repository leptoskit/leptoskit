use std::collections::BTreeMap;

use super::*;

fn files(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
	walkdir::WalkDir::new(root)
		.into_iter()
		.map(|entry| entry.unwrap())
		.filter(|entry| entry.file_type().is_file())
		.map(|entry| {
			(
				entry.path().strip_prefix(root).unwrap().to_owned(),
				fs::read(entry.path()).unwrap(),
			)
		})
		.collect()
}

#[test]
fn module_conflicts_fail_before_any_generator_write_even_with_force() {
	for (entry, text) in [
		("app_app/src/lib.rs", "pub mod auth {}"),
		("app_app/src/lib.rs", "#[path = \"other.rs\"] pub mod auth;"),
		("src/main.rs", "mod sign_in {} fn main() {}"),
		(
			"src/main.rs",
			"#[cfg_attr(unix, path = \"other.rs\")] mod sign_in; fn main() {}",
		),
		("src/main.rs", "fn incomplete("),
	] {
		let root = workspace(true);
		let path = root.path();
		fs::write(path.join(entry), text).unwrap();
		let before = files(path);
		let config = LeptoskitConfig::load_from(path).unwrap();
		assert!(generate(path, &config, false, true).is_err(), "{text}");
		assert_eq!(files(path), before, "{text}");
	}
}

#[test]
fn a_directory_server_module_is_not_shadowed_by_a_generated_file() {
	let root = workspace(true);
	let path = root.path();
	fs::create_dir(path.join("src/sign_in")).unwrap();
	fs::write(path.join("src/sign_in/mod.rs"), "// app owned").unwrap();
	let before = files(path);
	let config = LeptoskitConfig::load_from(path).unwrap();
	let error = generate(path, &config, false, true).unwrap_err();
	assert!(error.to_string().contains("sign_in/mod.rs"));
	assert_eq!(files(path), before);
}

#[test]
fn commented_module_declarations_are_not_duplicated_on_generation_or_rerun() {
	let root = workspace(true);
	let path = root.path();
	let client = "pub(in crate) mod\nauth; // keep this comment\n";
	let server = "mod sign_in; // keep this too\nfn main() {}\n";
	fs::write(path.join("app_app/src/lib.rs"), client).unwrap();
	fs::write(path.join("src/main.rs"), server).unwrap();
	run(path, false);
	let again = run(path, false);
	assert!(again.changed.is_empty(), "{:?}", again.changed);
	assert_eq!(read(path, "app_app/src/lib.rs"), client);
	assert_eq!(read(path, "src/main.rs"), server);
}

#[test]
fn function_references_and_documented_examples_keep_setup_guidance() {
	let root = workspace(true);
	let path = root.path();
	fs::write(
		path.join("app_app/src/lib.rs"),
		"// auth::provide();\nfn start() { let _ = auth::provide; }\n",
	)
	.unwrap();
	fs::write(
		path.join("src/main.rs"),
		"// sign_in::service::<User>(&db);\nfn main() { let _ = sign_in::service::<User>; }\n",
	)
	.unwrap();
	let report = run(path, false);
	assert!(
		report
			.next
			.iter()
			.any(|step| step.contains("auth::provide()"))
	);
	assert!(
		report
			.next
			.iter()
			.any(|step| step.contains("sign_in::service::<User>"))
	);
}

#[test]
fn providing_auth_without_gating_screens_keeps_root_guidance() {
	let root = workspace(false);
	let path = root.path();
	fs::write(
		path.join("app_app/src/lib.rs"),
		"fn start() { auth::provide(); }",
	)
	.unwrap();
	let report = run(path, false);
	assert!(
		report
			.next
			.iter()
			.any(|step| step.contains("auth::screens().gate"))
	);
}
