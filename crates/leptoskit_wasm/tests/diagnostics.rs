//! A guest that fails says where: which node, field, payload, import, and
//! export, in the trap and in an error record.

use leptoskit_log::MAX_EXCERPT_BYTES;
use leptoskit_wasm::WasmHost;
use log::Level;

/// A guest whose `leptoskit_main` creates node 7 as `kind` and sends it
/// `props`, then commits.
fn guest(kind: i32, props: &str) -> String {
	let escaped = props.replace('\\', "\\\\").replace('"', "\\\"");
	format!(
		r#"(module
  (import "leptoskit" "node_create" (func $node_create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $node_set_props (param i64 i32 i32)))
  (import "leptoskit" "commit" (func $commit))
  (memory (export "memory") 1)
  (data (i32.const 64) "{escaped}")
  (func (export "leptoskit_main")
    (call $node_create (i64.const 7) (i32.const {kind}))
    (call $node_set_props (i64.const 7) (i32.const 64) (i32.const {len}))
    (call $commit))
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) (i32.const 0)))"#,
		len = props.len()
	)
}

/// The text either side of the excerpt's `<<HERE>>` marker.
fn around_marker(message: &str) -> &str {
	let at = message
		.find("<<HERE>>")
		.expect("an excerpt marks where decoding stopped");
	let start = message.floor_char_boundary(at.saturating_sub(48));
	let end = message.ceil_char_boundary((at + 32).min(message.len()));
	&message[start..end]
}

fn start_error(kind: i32, props: &str) -> String {
	let mut host = WasmHost::headless(guest(kind, props).as_bytes()).expect("the guest loads");
	host.start(320.0, 480.0)
		.expect_err("the props do not decode")
		.to_string()
}

#[test]
fn a_style_value_of_the_wrong_type_names_the_node_field_and_bytes() {
	let records = leptoskit_log::capture::start();
	// The Seeker's failure: a map where a number belongs, far into the props.
	let props = format!(
		r#"{{"text":"{}","style":{{"flex_grow":{{"value":1}}}},"class":"a62f"}}"#,
		"padding ".repeat(60)
	);
	let error = start_error(1, &props);

	assert!(
		error.starts_with("host call failed: leptoskit_main: node_set_props: node 7 (text)"),
		"{error}"
	);
	assert!(error.contains("at `style.flex_grow`"), "{error}");
	assert!(error.contains("invalid type: map, expected f32"), "{error}");
	assert!(around_marker(&error).contains(r#""flex_grow""#), "{error}");
	assert!(around_marker(&error).contains(r#"{"value":1}"#), "{error}");

	let record = records.expect("leptoskit/render", r#"{"value":1}},"class":"a62f""#);
	assert_eq!(record.level, Level::Error);
	assert!(
		record.message.starts_with("node_set_props: node 7 (text)"),
		"{}",
		record.message
	);
	assert!(record.message.contains("at `style.flex_grow`"));
	let excerpt = record.message.split("; bytes ").nth(1).expect("an excerpt");
	assert!(
		excerpt.len() <= MAX_EXCERPT_BYTES + 64,
		"{} bytes",
		excerpt.len()
	);
	assert!(excerpt.contains('…'), "a long payload is cut: {excerpt}");
}

#[test]
fn a_flattened_widget_prop_of_the_wrong_type_names_its_field() {
	let records = leptoskit_log::capture::start();
	let error = start_error(0, r#"{"corner_radius":"round 51b9"}"#);

	assert!(error.contains("node 7 (view)"), "{error}");
	assert!(error.contains("at `corner_radius`"), "{error}");
	assert!(
		around_marker(&error).contains(r#""corner_radius":"round 51b9""#),
		"{error}"
	);
	records.expect("leptoskit/render", "round 51b9");
}

#[test]
fn a_render_limit_names_the_import_and_the_export() {
	let props = format!(r#"{{"text":"{}"}}"#, "x".repeat(70_000));
	let mut host = WasmHost::headless(
		format!(
			r#"(module
  (import "leptoskit" "node_create" (func $node_create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $node_set_props (param i64 i32 i32)))
  (memory (export "memory") 2)
  (func (export "leptoskit_main")
    (call $node_create (i64.const 1) (i32.const 1))
    (call $node_set_props (i64.const 1) (i32.const 0) (i32.const {})))
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) (i32.const 0)))"#,
			props.len()
		)
		.as_bytes(),
	)
	.unwrap();
	let error = host.start(320.0, 480.0).unwrap_err().to_string();

	assert!(
		error
			.starts_with("host call failed: leptoskit_main: node_set_props: render property limit"),
		"{error}"
	);
}

#[test]
fn a_guest_trap_names_the_export_it_happened_in() {
	let mut host = WasmHost::headless(
		br#"(module
  (memory (export "memory") 1)
  (func (export "leptoskit_main"))
  (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32) unreachable))"#,
	)
	.unwrap();
	host.start(320.0, 480.0).unwrap();
	let error = host
		.dispatch_wire(1, leptoskit_wasm::EventKind::Press, None)
		.unwrap_err()
		.to_string();

	assert!(
		error.starts_with("host call failed: leptoskit_event: "),
		"{error}"
	);
	assert!(error.contains("unreachable"), "{error}");
}
