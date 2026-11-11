//! Schema fidelity diagnostics: what each target can and cannot carry.
#![cfg(feature = "generators")]

use leptoskit_rpc_codegen::ApiChannel;
use leptoskit_rpc_codegen::ApiFunction;
use leptoskit_rpc_codegen::ApiKind;
use leptoskit_rpc_codegen::ApiParam;
use leptoskit_rpc_codegen::ApiSpec;
use leptoskit_rpc_codegen::Diagnostic;
use leptoskit_rpc_codegen::DiagnosticKind;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::HttpInput;
use leptoskit_rpc_codegen::HttpOutput;
use leptoskit_rpc_codegen::TargetLanguage;
use serde_json::json;

/// Shared fixture covering supported shapes, intentional free-form JSON, and
/// real representation loss.
fn fixture() -> ApiSpec {
	ApiSpec::from_json(include_str!("fixtures/schema_fidelity.json"))
		.expect("the fidelity fixture is a valid spec")
}

fn diagnostics(spec: &ApiSpec, language: TargetLanguage) -> Vec<Diagnostic> {
	spec.diagnostics(&GenerateOptions::new(language, "fidelity"))
}

fn at<'a>(diagnostics: &'a [Diagnostic], path: &str) -> Vec<&'a Diagnostic> {
	diagnostics
		.iter()
		.filter(|diagnostic| diagnostic.path == path)
		.collect()
}

fn kinds(diagnostics: &[Diagnostic], path: &str) -> Vec<DiagnosticKind> {
	at(diagnostics, path)
		.iter()
		.map(|diagnostic| diagnostic.kind)
		.collect()
}

#[test]
fn unsupported_constructs_carry_their_schema_path() {
	let diagnostics = diagnostics(&fixture(), TargetLanguage::TypeScript);

	for (path, message, fallback) in [
		("components.Mixed", "more than one", "unknown"),
		("components.Composed", "`allOf` with 2", "unknown"),
		("components.NumberEnum", "non-string", "base type"),
		(
			"functions.mixed_inline.params.value",
			"more than one",
			"unknown",
		),
	] {
		let found = at(&diagnostics, path);
		assert_eq!(found.len(), 1, "{path}: {diagnostics:#?}");
		assert_eq!(found[0].kind, DiagnosticKind::UnsupportedSchema);
		assert!(
			found[0].message.contains(message),
			"{path}: {}",
			found[0].message
		);
		assert!(
			found[0].message.contains(fallback),
			"the message names what TypeScript writes instead: {}",
			found[0].message
		);
	}

	let missing = at(&diagnostics, "functions.missing_ref.output");
	assert_eq!(missing.len(), 1, "{diagnostics:#?}");
	assert_eq!(missing[0].kind, DiagnosticKind::UnresolvedReference);
	assert!(
		missing[0].message.contains("Absent"),
		"{}",
		missing[0].message
	);
	assert!(
		missing[0].message.contains("unknown"),
		"{}",
		missing[0].message
	);
}

#[test]
fn intentional_free_form_json_is_not_a_loss() {
	let diagnostics = diagnostics(&fixture(), TargetLanguage::TypeScript);

	for path in [
		"components.Blob",
		"components.Loose",
		"components.Account.properties.settings",
		"components.Account.properties.labels",
		"components.State",
	] {
		assert!(
			at(&diagnostics, path).is_empty(),
			"{path} is deliberately broad, not lossy: {diagnostics:#?}"
		);
	}

	// A skipped function's schemas are not part of this client at all.
	assert!(
		at(&diagnostics, "components.OnlySkipped").is_empty(),
		"{diagnostics:#?}"
	);
	let skipped = at(&diagnostics, "functions.skipped_stream");
	assert_eq!(skipped.len(), 1);
	assert_eq!(skipped[0].kind, DiagnosticKind::SkippedFunction);
}

#[test]
fn targets_differ_in_what_they_can_represent() {
	let spec = fixture();

	let typescript = diagnostics(&spec, TargetLanguage::TypeScript);
	let python = diagnostics(&spec, TargetLanguage::Python);
	let go = diagnostics(&spec, TargetLanguage::Go);
	let rust = diagnostics(&spec, TargetLanguage::Rust);
	let dart = diagnostics(&spec, TargetLanguage::Dart);
	let cli = diagnostics(&spec, TargetLanguage::Cli);

	for diagnostics in [&typescript, &python] {
		assert!(
			at(diagnostics, "components.Choice").is_empty(),
			"these targets preserve an arbitrary union"
		);
	}

	for diagnostics in [&go, &rust, &dart, &cli] {
		assert_eq!(
			kinds(diagnostics, "components.Choice"),
			[DiagnosticKind::UnionFallback],
			"these targets collapse it"
		);
	}

	// A tagged serde enum is structured in Rust; Go and Dart still flatten
	// every union to their dynamic type.
	assert!(at(&rust, "components.Event").is_empty(), "{rust:#?}");

	for diagnostics in [&go, &dart] {
		assert_eq!(
			kinds(diagnostics, "components.Event"),
			[DiagnosticKind::UnionFallback]
		);
	}

	for diagnostics in [&typescript, &python, &rust] {
		assert!(
			at(diagnostics, "components.Point").is_empty(),
			"these targets carry a fixed tuple"
		);
	}

	for diagnostics in [&go, &dart] {
		assert_eq!(
			kinds(diagnostics, "components.Point"),
			[DiagnosticKind::TupleFallback]
		);
	}

	assert_eq!(
		kinds(&typescript, "components.Account.properties.id"),
		[DiagnosticKind::IntegerPrecision]
	);
	assert_eq!(
		kinds(&dart, "components.Account.properties.id"),
		[DiagnosticKind::IntegerPrecision]
	);

	for diagnostics in [&python, &go, &rust, &cli] {
		assert!(
			at(diagnostics, "components.Account.properties.id").is_empty(),
			"these targets can represent uint64 exactly"
		);
	}

	for diagnostics in [&typescript, &python, &go, &rust, &cli, &dart] {
		assert!(
			at(diagnostics, "components.Account.properties.deleted_at").is_empty(),
			"these targets keep required-nullable distinct"
		);
	}
}

#[test]
fn openapi_preserves_the_whole_schema() {
	assert!(
		diagnostics(&fixture(), TargetLanguage::OpenApi).is_empty(),
		"the OpenAPI document carries every schema verbatim"
	);
}

/// Every construct these targets model exactly, including optional and
/// nullable fields side by side. Nothing here should be reported.
fn supported_spec() -> ApiSpec {
	let mut spec = ApiSpec::empty("Supported", "1.0.0");
	spec.functions = vec![ApiFunction {
		name: "read".to_owned(),
		kind: ApiKind::Call,
		module_path: "supported::api".to_owned(),
		struct_name: "Read".to_owned(),
		docs: String::new(),
		path: "/api/read".to_owned(),
		method: "POST".to_owned(),
		input: HttpInput::Json,
		output: HttpOutput::Json,
		error_content_type: "text/plain".to_owned(),
		params: vec![
			ApiParam {
				name: "filter".to_owned(),
				docs: String::new(),
				schema: json!({ "$ref": "#/components/schemas/Filter" }),
				required: true,
			},
			ApiParam {
				name: "limit".to_owned(),
				docs: String::new(),
				schema: json!({ "type": "integer", "format": "uint32" }),
				required: false,
			},
		],
		output_schema: json!({ "$ref": "#/components/schemas/User" }),
		internal: false,
		deprecated: false,
		tags: Vec::new(),
		scopes: Vec::new(),
	}];
	spec.schemas.insert(
		"Filter".to_owned(),
		json!({ "type": "string", "enum": ["all", "active"] }),
	);
	spec.schemas.insert(
		"User".to_owned(),
		json!({
			"type": "object",
			"properties": {
				"id": { "type": "integer", "format": "uint32" },
				"name": { "type": "string" },
				"nickname": { "type": "string" },
				"role": { "type": "string", "default": "member" },
				"note": { "type": "string" },
				"tags": { "type": "array", "items": { "type": "string" } },
				"counts": {
					"type": "object",
					"additionalProperties": { "type": "integer", "format": "uint32" }
				}
			},
			"required": ["id", "name"]
		}),
	);
	spec
}

#[test]
fn useful_permissive_schemas_are_not_rejected() {
	for target in TargetLanguage::ALL {
		assert!(
			diagnostics(&supported_spec(), *target).is_empty(),
			"{target} should accept ordinary optional and nullable fields"
		);
	}
}

#[test]
fn query_and_form_diagnose_free_form_json_without_rejecting_json_input() {
	let mut spec = supported_spec();
	spec.schemas.insert("Dynamic".to_owned(), json!(true));
	for schema in [
		json!(true),
		json!({"type": "array", "items": true}),
		json!({"type": "object", "additionalProperties": true}),
		json!({"$ref": "#/components/schemas/Dynamic"}),
		json!({"type": "object", "properties": {"nested": true}}),
	] {
		spec.functions[0].params = vec![ApiParam {
			name: "value".to_owned(),
			docs: String::new(),
			schema,
			required: true,
		}];
		for input in [HttpInput::Query, HttpInput::Form, HttpInput::Json] {
			spec.functions[0].input = input.clone();
			for target in TargetLanguage::ALL {
				if *target == TargetLanguage::OpenApi {
					continue;
				}
				let found = diagnostics(&spec, *target);
				let losses = kinds(&found, "functions.read.params.value");
				if input == HttpInput::Json {
					assert!(losses.is_empty(), "{target}: {found:?}");
				} else {
					assert!(
						losses.contains(&DiagnosticKind::EncodingLoss),
						"{target}: {found:?}"
					);
				}
			}
		}
	}
}

#[test]
fn optional_nullable_fields_and_explicit_extra_properties_are_reported() {
	let mut spec = supported_spec();
	spec.schemas.get_mut("User").unwrap()["properties"]["note"] =
		json!({ "type": ["string", "null"] });
	spec.schemas.get_mut("User").unwrap()["additionalProperties"] = json!({ "type": "string" });

	for target in TargetLanguage::ALL {
		let found = diagnostics(&spec, *target);

		if *target == TargetLanguage::OpenApi {
			assert_eq!(found, []);

			continue;
		}

		assert_eq!(
			kinds(&found, "components.User.additionalProperties"),
			[DiagnosticKind::AdditionalPropertiesLost]
		);
		let nullable = kinds(&found, "components.User.properties.note");
		if matches!(
			target,
			TargetLanguage::Go | TargetLanguage::Rust | TargetLanguage::Cli
		) {
			assert_eq!(nullable, [DiagnosticKind::NullableCollapsed]);
		} else {
			assert_eq!(
				nullable,
				[],
				"{target} keeps an optional nullable model field distinct from absence"
			);
		}
	}
}

#[test]
fn typed_conditional_shapes_are_not_silently_accepted() {
	let mut spec = supported_spec();
	spec.schemas.get_mut("User").unwrap()["if"] = json!({ "required": ["note"] });
	spec.schemas.get_mut("User").unwrap()["then"] = json!({ "required": ["nickname"] });
	assert_eq!(
		kinds(
			&diagnostics(&spec, TargetLanguage::TypeScript),
			"components.User"
		),
		[DiagnosticKind::UnsupportedSchema]
	);
}

#[test]
fn bounded_wide_integers_are_safe_for_javascript_targets() {
	let mut spec = supported_spec();

	for target in [TargetLanguage::TypeScript, TargetLanguage::Dart] {
		for format in ["int64", "uint64"] {
			spec.schemas.get_mut("User").unwrap()["properties"]["id"] = json!({
				"type": "integer", "format": format, "minimum": 0, "maximum": 100
			});
			assert_eq!(
				kinds(&diagnostics(&spec, target), "components.User.properties.id"),
				[]
			);
			spec.schemas.get_mut("User").unwrap()["properties"]["id"]["maximum"] =
				json!(9_007_199_254_740_992_i64);
			assert_eq!(
				kinds(&diagnostics(&spec, target), "components.User.properties.id"),
				[DiagnosticKind::IntegerPrecision]
			);
		}
	}
}

#[test]
fn go_nullable_aliases_cannot_hide_null_omission() {
	let mut spec = supported_spec();
	spec.schemas.insert(
		"MaybeText".to_owned(),
		json!({ "type": ["string", "null"] }),
	);
	spec.schemas.get_mut("User").unwrap()["properties"]["note"] =
		json!({ "$ref": "#/components/schemas/MaybeText" });
	assert_eq!(
		kinds(
			&diagnostics(&spec, TargetLanguage::Go),
			"components.User.properties.note"
		),
		[DiagnosticKind::NullableCollapsed]
	);
	// Dart follows the alias into a three-state presence wrapper, and Rust
	// can wrap the named Option alias in another Option for absence.
	for target in [TargetLanguage::Dart, TargetLanguage::Rust] {
		assert_eq!(
			kinds(
				&diagnostics(&spec, target),
				"components.User.properties.note"
			),
			[]
		);
	}
}

/// Dart's request encoder preserves null and absence across HTTP codecs.
#[test]
fn dart_query_and_form_arguments_preserve_null() {
	let mut spec = supported_spec();
	spec.schemas.insert(
		"Nested".to_owned(),
		json!({
			"type": "object",
			"properties": {
				"note": { "type": ["string", "null"] },
				"values": { "type": "array", "items": { "type": ["integer", "null"] } }
			}
		}),
	);
	spec.functions[0].input = HttpInput::Query;
	spec.functions[0].params = vec![
		ApiParam {
			name: "required".to_owned(),
			docs: String::new(),
			schema: json!({ "type": ["string", "null"] }),
			required: true,
		},
		ApiParam {
			name: "optional".to_owned(),
			docs: String::new(),
			schema: json!({ "type": ["string", "null"], "default": "kept" }),
			required: false,
		},
		ApiParam {
			name: "plain".to_owned(),
			docs: String::new(),
			schema: json!({ "type": ["string", "null"] }),
			required: false,
		},
		ApiParam {
			name: "nested".to_owned(),
			docs: String::new(),
			schema: json!({ "$ref": "#/components/schemas/Nested" }),
			required: true,
		},
	];
	for input in [HttpInput::Query, HttpInput::Form, HttpInput::Json] {
		spec.functions[0].input = input;
		assert_eq!(diagnostics(&spec, TargetLanguage::Dart), []);
		for target in [
			TargetLanguage::Go,
			TargetLanguage::Rust,
			TargetLanguage::Cli,
		] {
			assert_eq!(
				kinds(
					&diagnostics(&spec, target),
					"functions.read.params.optional"
				),
				[DiagnosticKind::NullableCollapsed]
			);
			assert_eq!(
				kinds(&diagnostics(&spec, target), "functions.read.params.plain"),
				[],
				"without a default, the server reads omission as null"
			);
		}
	}
}

#[test]
fn dart_channel_presence_cannot_confuse_absence_with_null() {
	let mut spec = supported_spec();
	spec.channels.push(ApiChannel {
		name: "room".to_owned(),
		module_path: "supported::api".to_owned(),
		struct_name: "Room".to_owned(),
		docs: String::new(),
		fields: vec!["id".to_owned()],
		args_schema: json!({
			"type": "object",
			"properties": { "id": { "type": "string" } },
			"required": ["id"]
		}),
		message_schema: json!({ "type": "string" }),
		presence_schema: json!({ "type": ["string", "null"] }),
		internal: false,
		deprecated: false,
		tags: Vec::new(),
	});
	assert_eq!(
		kinds(
			&diagnostics(&spec, TargetLanguage::Dart),
			"channels.room.presence"
		),
		[DiagnosticKind::NullableCollapsed]
	);
	// A non-nullable presence has only absence and a value, so it is fine.
	spec.channels[0].presence_schema = json!({ "type": "string" });
	assert_eq!(
		kinds(
			&diagnostics(&spec, TargetLanguage::Dart),
			"channels.room.presence"
		),
		vec![]
	);
}

#[test]
fn dart_null_only_inputs_are_supported_and_untyped_form_inputs_are_reported() {
	for (schema, expected) in [
		(json!({"type": "null"}), vec![]),
		(json!(true), vec![DiagnosticKind::EncodingLoss]),
		(json!({}), vec![DiagnosticKind::EncodingLoss]),
	] {
		let mut spec = supported_spec();
		spec.schemas.insert("NullableAlias".into(), schema);
		spec.functions[0].input = HttpInput::Form;
		spec.functions[0].params = vec![ApiParam {
			name: "value".into(),
			docs: String::new(),
			required: true,
			schema: json!({"$ref": "#/components/schemas/NullableAlias"}),
		}];
		let path = format!("functions.{}.params.value", spec.functions[0].name);
		assert_eq!(
			kinds(&diagnostics(&spec, TargetLanguage::Dart), &path),
			expected
		);
		spec.functions[0].input = HttpInput::Json;

		assert_eq!(
			kinds(&diagnostics(&spec, TargetLanguage::Dart), &path),
			vec![]
		);
	}
}

#[test]
fn dart_unconstrained_form_collections_report_encoding_loss_without_null_loss() {
	for (schema, suffix) in [
		(json!({"type": "array"}), "items"),
		(
			json!({"type": "object", "additionalProperties": true}),
			"additionalProperties",
		),
	] {
		let mut spec = supported_spec();
		spec.functions[0].input = HttpInput::Form;
		spec.functions[0].params = vec![ApiParam {
			name: "values".into(),
			docs: String::new(),
			required: true,
			schema,
		}];
		let path = format!(
			"functions.{}.params.values.{suffix}",
			spec.functions[0].name
		);
		assert_eq!(
			kinds(
				&diagnostics(&spec, TargetLanguage::Dart),
				"functions.read.params.values"
			),
			[DiagnosticKind::EncodingLoss]
		);
		assert_eq!(kinds(&diagnostics(&spec, TargetLanguage::Dart), &path), []);
		spec.functions[0].input = HttpInput::Json;
		assert_eq!(
			kinds(&diagnostics(&spec, TargetLanguage::Dart), &path),
			vec![]
		);
	}
}
