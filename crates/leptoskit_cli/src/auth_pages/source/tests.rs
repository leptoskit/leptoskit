use super::*;

fn parse(text: &str) -> Source {
	let directory = tempfile::tempdir().unwrap();
	let path = directory.path().join("lib.rs");
	std::fs::write(&path, text).unwrap();
	Source::read(&path).unwrap()
}

#[test]
fn module_syntax_survives_comments_visibility_and_raw_identifiers() {
	for text in [
		"pub mod auth; // app-owned visibility",
		"pub(in crate) mod\nauth\n;",
		"mod r#auth;",
		"#[cfg(feature = \"web\")] pub mod auth;",
	] {
		assert!(parse(text).module("auth").unwrap(), "{text}");
	}
}

#[test]
fn comments_strings_nested_modules_and_macro_tokens_are_not_root_modules() {
	for text in [
		"// mod auth;",
		"const EXAMPLE: &str = \"pub mod auth;\";",
		"mod other { mod auth; }",
		"example! { mod auth; }",
	] {
		assert!(!parse(text).module("auth").unwrap(), "{text}");
	}
}

#[test]
fn ambiguous_inline_and_redirected_modules_need_manual_integration() {
	for text in [
		"mod auth {}",
		"#[path = \"other.rs\"] mod auth;",
		"#[cfg_attr(feature = \"web\", path = \"other.rs\")] mod auth;",
		"#[cfg_attr(unix, cfg_attr(feature = \"web\", path = \"other.rs\"))] mod auth;",
		"#[cfg(unix)] mod auth; #[cfg(windows)] mod auth;",
	] {
		assert!(parse(text).module("auth").is_err(), "{text}");
	}
}

#[test]
fn actual_calls_with_spaces_and_generics_are_found() {
	let source = parse(
		"fn start() {
			crate::auth::provide ();
			self::auth::screens().gate(|| home());
			auth::screens ().navigate_with (|_| ());
			sign_in::service::<User> (&db);
			sign_in::provider (&db, &auth);
		}",
	);
	assert!(source.calls.provide && source.calls.gate && source.calls.navigate);
	assert!(source.calls.service && source.calls.provider);
}

#[test]
fn comments_references_literals_and_test_calls_do_not_hide_instructions() {
	let source = parse(
		"// auth::provide(); sign_in::service();
		const EXAMPLE: &str = \"auth::screens().gate(|| home); sign_in::provider()\";
		fn start() { let _ = (auth::provide, sign_in::service::<User>, sign_in::provider); }
		#[test] fn setup() { auth::provide(); auth::screens().gate(|| home); }
		#[cfg(test)] mod tests { fn setup() { sign_in::service(); sign_in::provider(); } }
		fn other() { unrelated::auth::provide(); unrelated::screens().navigate_with(|_| ()); }
		quote! { auth::provide(); }
		",
	);
	assert!(!source.calls.provide && !source.calls.gate && !source.calls.navigate);
	assert!(!source.calls.service && !source.calls.provider);
}

#[test]
fn invalid_syntax_is_an_actionable_preflight_error() {
	let directory = tempfile::tempdir().unwrap();
	let path = directory.path().join("lib.rs");
	std::fs::write(&path, "fn incomplete(").unwrap();
	let error = Source::read(&path).err().unwrap();
	assert!(error.to_string().contains("before generating auth"));
}
