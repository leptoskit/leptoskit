//! Real server defaults must remain visible to strict SDK generation.
#![cfg(feature = "ssr")]
#![allow(
	clippy::unused_async,
	reason = "server functions are async by contract"
)]

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptos::prelude::*;
use leptos::server_fn::codec::Json;
use leptoskit_rpc::api;
use leptoskit_rpc::spec;
use leptoskit_rpc_codegen::DiagnosticKind;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::TargetLanguage;
use leptoskit_rpc_codegen::spec::SERVER_DEFAULT_SCHEMA_KEY;
use serde_json::json;

static DEFAULT_CALLS: AtomicUsize = AtomicUsize::new(0);

#[expect(
	clippy::unnecessary_wraps,
	reason = "the server default has the argument's type"
)]
fn default_note() -> Option<String> {
	DEFAULT_CALLS.fetch_add(1, Ordering::Relaxed);
	Some("server default".to_owned())
}

#[api]
#[server(input = Json)]
pub async fn with_default(
	#[server(default = "default_note")] note: Option<String>,
	plain: Option<String>,
) -> Result<Option<String>, ServerFnError> {
	Ok(note.or(plain))
}

#[test]
fn exported_defaults_diagnose_actual_null_and_omission_differences() {
	let spec = spec::collect(&spec::SpecOptions::new("Defaults", "1")).unwrap();
	assert_eq!(DEFAULT_CALLS.load(Ordering::Relaxed), 0);
	let params = &spec.functions[0].params;
	assert_eq!(params[0].schema[SERVER_DEFAULT_SCHEMA_KEY], true);
	assert!(params[1].schema.get(SERVER_DEFAULT_SCHEMA_KEY).is_none());
	assert!(!params[0].required);
	assert!(!params[1].required);

	let omitted: WithDefault = serde_json::from_value(json!({})).unwrap();
	let explicit_null: WithDefault = serde_json::from_value(json!({"note": null})).unwrap();
	assert_eq!(omitted.note.as_deref(), Some("server default"));
	assert_eq!(explicit_null.note, None);
	assert_eq!(DEFAULT_CALLS.load(Ordering::Relaxed), 1);

	// Exercise serialized specs as the CLI consumes them, not only the
	// inventory's in-memory schema.
	let reread = spec::ApiSpec::from_json(&serde_json::to_string(&spec).unwrap()).unwrap();
	for target in [
		TargetLanguage::Go,
		TargetLanguage::Rust,
		TargetLanguage::Cli,
	] {
		let diagnostics = reread.diagnostics(&GenerateOptions::new(target, "defaults"));
		assert_eq!(diagnostics.len(), 1, "{target}: {diagnostics:?}");
		assert_eq!(diagnostics[0].kind, DiagnosticKind::NullableCollapsed);
		assert_eq!(diagnostics[0].path, "functions.with_default.params.note");
	}
	for target in [
		TargetLanguage::TypeScript,
		TargetLanguage::Python,
		TargetLanguage::Dart,
	] {
		assert!(
			reread
				.diagnostics(&GenerateOptions::new(target, "defaults"))
				.is_empty()
		);
	}
}
