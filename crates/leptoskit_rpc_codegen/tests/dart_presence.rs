//! Run the generated Dart client's presence and argument encoding against
//! real JSON, without a mock of the generated code.
#![cfg(feature = "generators")]

use std::process::Command;

use leptoskit_rpc_codegen::ApiSpec;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::TargetLanguage;

#[test]
fn dart_requiredness_and_nullability_are_independent() {
	let spec = ApiSpec::from_json(include_str!("fixtures/dart_presence.json"))
		.expect("the Dart presence fixture is a valid spec");
	let generated = spec.generate(&GenerateOptions::new(TargetLanguage::Dart, "presence"));
	let source = generated
		.files
		.iter()
		.find(|file| file.path == std::path::Path::new("lib/presence.dart"))
		.expect("lib/presence.dart")
		.contents
		.clone();

	// The generated API keeps requiredness and nullability separate, and a
	// required nullable field is a required constructor argument.
	assert!(
		source.contains("required this.requiredNullableText,"),
		"{source}"
	);
	assert!(
		source.contains("final String? requiredNullableText;"),
		"{source}"
	);
	assert!(
		source.contains("final Presence<String?> optionalNullableText;"),
		"{source}"
	);
	assert!(
		source.contains("this.optionalNullableText = const Presence<String?>.absent(),"),
		"{source}"
	);
	assert!(
		source.contains("required String? requiredNullableText"),
		"the operation argument stays required: {source}"
	);
	assert!(
		source
			.contains("Presence<String?> optionalNullableText = const Presence<String?>.absent()"),
		"{source}"
	);
	assert!(
		source.contains(concat!(
			"if (optionalNullableText.isPresent) 'optional_nullable_text': ",
			"optionalNullableText.value,"
		)),
		"{source}"
	);
	assert!(
		source.contains("_requiredValue(json, 'required_nullable_text')"),
		"{source}"
	);
	assert!(
		!source.contains("if (requiredNullableText != null)"),
		"a required nullable field is never omitted: {source}"
	);
}

#[test]
#[ignore = "run by test:sdk, which requires the Dart toolchain"]
fn generated_dart_presence_round_trips() {
	let spec =
		ApiSpec::from_json(include_str!("fixtures/dart_presence.json")).expect("fixture spec");
	let generated = spec.generate(&GenerateOptions::new(TargetLanguage::Dart, "presence"));
	let dir = tempfile::tempdir().expect("temporary Dart package");
	for file in generated.files {
		let path = dir.path().join(file.path);
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent).expect("generated directory");
		}
		std::fs::write(path, file.contents).expect("generated file");
	}
	std::fs::create_dir_all(dir.path().join("tool")).expect("tool directory");
	std::fs::write(
		dir.path().join("tool/presence.dart"),
		include_str!("fixtures/dart_presence_test.dart"),
	)
	.expect("Dart presence fixture");

	let dart = Command::new("dart")
		.arg("--version")
		.output()
		.expect("SDK acceptance requires dart; run through devenv shell test:sdk");
	assert!(
		dart.status.success(),
		"`dart --version` failed:\n{}",
		String::from_utf8_lossy(&dart.stderr)
	);

	let get = Command::new("dart")
		.args(["pub", "get"])
		.current_dir(dir.path())
		.output()
		.expect("run dart pub get from devenv");
	assert!(
		get.status.success(),
		"dart pub get failed:\n{}\n{}",
		String::from_utf8_lossy(&get.stdout),
		String::from_utf8_lossy(&get.stderr)
	);

	let run = Command::new("dart")
		.args(["run", "tool/presence.dart"])
		.current_dir(dir.path())
		.output()
		.expect("run generated Dart presence checks");
	assert!(
		run.status.success(),
		"generated Dart presence checks failed:\n{}\n{}",
		String::from_utf8_lossy(&run.stdout),
		String::from_utf8_lossy(&run.stderr)
	);
}
