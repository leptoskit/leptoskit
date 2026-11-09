//! Exercise the same query decoder configuration as `server_fn`'s URL codecs.
#![cfg(feature = "generators")]

use std::collections::BTreeMap;
use std::process::Command;

use leptoskit_rpc_codegen::ApiParam;
use leptoskit_rpc_codegen::ApiSpec;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::HttpInput;
use leptoskit_rpc_codegen::TargetLanguage;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize, PartialEq)]
struct Values {
	items: Vec<Option<String>>,
	labels: BTreeMap<String, Option<String>>,
	note: Option<String>,
	enabled: bool,
	count: i32,
	text: String,
	#[serde(default = "default_note")]
	fallback: Option<String>,
}

#[expect(
	clippy::unnecessary_wraps,
	reason = "serde defaults must return the field's type"
)]
fn default_note() -> Option<String> {
	Some("server-default".to_owned())
}

fn decode(query: &str) -> Values {
	serde_qs::Config::new()
		.use_form_encoding(true)
		.deserialize_str(query)
		.expect("the server URL codec accepts the generated request")
}

#[test]
fn the_server_codec_preserves_empty_collections_null_and_absence() {
	let base = "items=&labels=&note&enabled=false&count=0&text=";
	let absent = decode(base);
	assert_eq!(absent.items, [] as [Option<String>; 0]);
	assert!(absent.labels.is_empty());
	assert_eq!(absent.note, None);
	assert!(!absent.enabled);
	assert_eq!(absent.count, 0);
	assert_eq!(absent.text, "");
	assert_eq!(absent.fallback, default_note());
	assert_eq!(decode(&format!("{base}&fallback")).fallback, None);
	assert_eq!(
		decode(&format!("{base}&fallback=")).fallback,
		Some(String::new())
	);
}

fn specification(input: HttpInput) -> ApiSpec {
	let mut spec = ApiSpec::from_json(include_str!("fixtures/dart_presence.json")).unwrap();
	spec.functions.truncate(1);
	spec.schemas.clear();
	spec.channels.clear();
	let function = &mut spec.functions[0];
	function.name = "probe".into();
	function.struct_name = "Probe".into();
	function.input = input;
	function.output_schema = json!({"type": "string"});
	function.params = [
		(
			"items",
			json!({"type": "array", "items": {"type": ["string", "null"]}}),
			true,
		),
		(
			"labels",
			json!({"type": "object", "additionalProperties": true}),
			true,
		),
		("note", json!({"type": ["string", "null"]}), true),
		("enabled", json!({"type": "boolean"}), true),
		("count", json!({"type": "integer"}), true),
		("text", json!({"type": "string"}), true),
		(
			"fallback",
			json!({"type": ["string", "null"], "default": "server-default"}),
			false,
		),
	]
	.into_iter()
	.map(|(name, schema, required)| {
		ApiParam {
			name: name.into(),
			docs: String::new(),
			schema,
			required,
		}
	})
	.collect();
	spec
}

#[test]
fn generated_python_http_preserves_empty_null_and_omitted_arguments() {
	check_generated(TargetLanguage::Python);
}

#[test]
#[ignore = "requires Deno and Dart; run by the SDK acceptance gate"]
fn generated_typescript_and_dart_preserve_query_values() {
	check_generated(TargetLanguage::TypeScript);
	check_generated(TargetLanguage::Dart);
}

#[test]
#[ignore = "requires Go and Cargo; run by the SDK acceptance gate"]
fn generated_go_and_rust_preserve_query_values() {
	check_generated(TargetLanguage::Go);
	check_generated(TargetLanguage::Rust);
	check_generated(TargetLanguage::Cli);
}

fn check_generated(language: TargetLanguage) {
	for input in [HttpInput::Query, HttpInput::Form, HttpInput::Json] {
		let generated =
			specification(input.clone()).generate(&GenerateOptions::new(language, "wire"));
		let temporary = tempfile::tempdir().unwrap();
		for file in generated.files {
			let path = temporary.path().join(file.path);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(path, file.contents).unwrap();
		}
		if language == TargetLanguage::TypeScript {
			use std::io::Write;
			writeln!(
				std::fs::OpenOptions::new()
					.append(true)
					.open(temporary.path().join("live.ts"))
					.unwrap(),
				"\nexport {{ present as presentForAcceptance }};"
			)
			.unwrap();
		}
		if input == HttpInput::Json {
			check_live_generated(language, temporary.path());
		}
		let result = run_generated(language, temporary.path());
		assert!(
			result.status.success(),
			"{}",
			String::from_utf8_lossy(&result.stderr)
		);
		let mut requests: Vec<serde_json::Value> = serde_json::from_slice(&result.stdout).unwrap();
		let nested = requests.pop().unwrap();
		let nested: Values = if input == HttpInput::Json {
			serde_json::from_str(nested["body"].as_str().unwrap()).unwrap()
		} else {
			decode(
				nested[if input == HttpInput::Query {
					"query"
				} else {
					"body"
				}]
				.as_str()
				.unwrap(),
			)
		};
		assert_eq!(
			nested.items,
			vec![None, Some("a +&é".to_owned()), Some(String::new())]
		);
		assert_eq!(
			nested.labels,
			BTreeMap::from([
				("key +&é".to_owned(), None),
				("empty".to_owned(), Some(String::new()))
			])
		);
		assert_eq!(nested.text, "a +&é");
		let fallbacks = if matches!(
			language,
			TargetLanguage::Go | TargetLanguage::Rust | TargetLanguage::Cli
		) {
			vec![default_note(), Some(String::new())]
		} else {
			vec![default_note(), None, Some(String::new())]
		};
		assert_eq!(requests.len(), fallbacks.len());
		for (request, fallback) in requests.into_iter().zip(fallbacks) {
			let values: Values = if input == HttpInput::Json {
				let payload: serde_json::Value =
					serde_json::from_str(request["body"].as_str().unwrap()).unwrap();
				assert!(
					payload.get("note").is_some(),
					"required null must remain present ({language:?}): {payload}"
				);
				serde_json::from_value(payload).unwrap()
			} else {
				decode(
					request[if input == HttpInput::Query {
						"query"
					} else {
						"body"
					}]
					.as_str()
					.unwrap(),
				)
			};
			assert_eq!(
				values,
				Values {
					items: Vec::new(),
					labels: BTreeMap::new(),
					note: None,
					enabled: false,
					count: 0,
					text: String::new(),
					fallback,
				},
				"{input:?}"
			);
		}
	}
}

fn run_generated(language: TargetLanguage, directory: &std::path::Path) -> std::process::Output {
	match language {
		TargetLanguage::Python => {
			Command::new("python3")
				.args(["-c", include_str!("fixtures/query_wire.py")])
				.current_dir(directory)
				.output()
				.unwrap()
		}
		TargetLanguage::TypeScript => {
			std::fs::write(
				directory.join("probe.ts"),
				include_str!("fixtures/query_wire.ts"),
			)
			.unwrap();
			Command::new("deno")
				.args(["run", "probe.ts"])
				.current_dir(directory)
				.output()
				.unwrap()
		}
		TargetLanguage::Dart => {
			std::fs::write(
				directory.join("probe.dart"),
				include_str!("fixtures/query_wire.dart"),
			)
			.unwrap();
			let get = Command::new("dart")
				.args(["pub", "get"])
				.current_dir(directory)
				.output()
				.unwrap();
			assert!(
				get.status.success(),
				"{}",
				String::from_utf8_lossy(&get.stderr)
			);
			Command::new("dart")
				.args(["run", "probe.dart"])
				.current_dir(directory)
				.output()
				.unwrap()
		}
		TargetLanguage::Go => {
			std::fs::create_dir_all(directory.join("cmd/probe")).unwrap();
			std::fs::write(
				directory.join("cmd/probe/main.go"),
				include_str!("fixtures/query_wire.go"),
			)
			.unwrap();
			Command::new("go")
				.args(["run", "./cmd/probe"])
				.current_dir(directory)
				.output()
				.unwrap()
		}
		TargetLanguage::Rust => {
			std::fs::write(
				directory.join("src/main.rs"),
				include_str!("fixtures/query_wire_client.rs"),
			)
			.unwrap();
			Command::new("cargo")
				.args(["run", "--quiet"])
				.env(
					"CARGO_TARGET_DIR",
					std::env::var_os("LEPTOSKIT_SDK_TARGET_DIR")
						.expect("set LEPTOSKIT_SDK_TARGET_DIR to a separate SDK cache"),
				)
				.current_dir(directory)
				.output()
				.unwrap()
		}
		TargetLanguage::Cli => {
			let target = std::env::var_os("LEPTOSKIT_SDK_TARGET_DIR").expect("SDK target");
			let build = Command::new("cargo")
				.args(["build", "--quiet"])
				.env("CARGO_TARGET_DIR", &target)
				.current_dir(directory)
				.output()
				.unwrap();
			assert!(
				build.status.success(),
				"{}",
				String::from_utf8_lossy(&build.stderr)
			);
			Command::new("python3")
				.args(["-c", include_str!("fixtures/query_wire_cli.py")])
				.arg(std::path::Path::new(&target).join("debug/wire"))
				.current_dir(directory)
				.output()
				.unwrap()
		}
		TargetLanguage::OpenApi => unreachable!(),
	}
}

fn check_live_generated(language: TargetLanguage, directory: &std::path::Path) {
	let output = match language {
		TargetLanguage::Go => {
			std::fs::write(
				directory.join("wire_test.go"),
				include_str!("fixtures/query_wire_live.go"),
			)
			.unwrap();
			Command::new("go")
				.args(["test", "./..."])
				.current_dir(directory)
				.output()
				.unwrap()
		}
		TargetLanguage::Rust => {
			use std::io::Write;
			write!(
				std::fs::OpenOptions::new()
					.append(true)
					.open(directory.join("src/live.rs"))
					.unwrap(),
				"\n{}",
				include_str!("fixtures/query_wire_live.rs")
			)
			.unwrap();
			Command::new("cargo")
				.args(["test", "--lib", "--quiet"])
				.env(
					"CARGO_TARGET_DIR",
					std::env::var_os("LEPTOSKIT_SDK_TARGET_DIR").unwrap(),
				)
				.current_dir(directory)
				.output()
				.unwrap()
		}
		_ => return,
	};
	assert!(
		output.status.success(),
		"{}{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
}
