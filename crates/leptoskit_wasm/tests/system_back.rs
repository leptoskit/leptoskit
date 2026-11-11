//! The platform's Back reaches a module through its optional
//! `leptoskit_system_back` export, and the module's answer comes back.

use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_wasm::WasmError;
use leptoskit_wasm::WasmHost;

/// A label showing how many routes deep the module is. Back pops one route
/// and, like a router's signal, changes the screen only when the host
/// flushes.
const ROUTER: &str = r#"(module
 (import "leptoskit" "node_create" (func $create (param i64 i32)))
 (import "leptoskit" "node_set_props" (func $props (param i64 i32 i32)))
 (import "leptoskit" "commit" (func $commit))
 (memory (export "memory") 1)
 ;; `{"text":"Depth 2"}`: 18 bytes, with the digit at offset 79.
 (data (i32.const 64) "{\"text\":\"Depth 2\"}")
 (global $depth (mut i32) (i32.const 2))
 (global $dirty (mut i32) (i32.const 0))
 (func (export "leptoskit_main")
   (call $create (i64.const 1) (i32.const 1))
   (call $props (i64.const 1) (i32.const 64) (i32.const 18))
   (call $commit))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32)
   (i32.const 0))
 (func (export "leptoskit_system_back") (result i32)
   (if (result i32) (i32.eqz (global.get $depth))
     (then (i32.const 0))
     (else
       (global.set $depth (i32.sub (global.get $depth) (i32.const 1)))
       (global.set $dirty (i32.const 1))
       (i32.const 1))))
 (func (export "leptoskit_flush")
   (if (global.get $dirty)
     (then
       (i32.store8 (i32.const 79) (i32.add (i32.const 48) (global.get $depth)))
       (call $props (i64.const 1) (i32.const 64) (i32.const 18))
       (call $commit)
       (global.set $dirty (i32.const 0)))))
)"#;

/// A module with no route history: it never exports `leptoskit_system_back`.
const WITHOUT_BACK: &str = r#"(module
 (import "leptoskit" "node_create" (func $create (param i64 i32)))
 (import "leptoskit" "commit" (func $commit))
 (memory (export "memory") 1)
 (func (export "leptoskit_main")
   (call $create (i64.const 1) (i32.const 0))
   (call $commit))
 (func (export "leptoskit_event") (param i64 i32 i32 i32) (result i32)
   (i32.const 0))
)"#;

fn start(wat: &str) -> WasmHost<HeadlessBackend> {
	let mut host = WasmHost::headless(wat.as_bytes()).expect("compile wat");
	host.start(320.0, 480.0).expect("start");
	host
}

fn label(host: &WasmHost<HeadlessBackend>) -> String {
	let surface = host.surface();
	let root = surface.root().expect("a committed root");
	surface.props(root).text.clone().unwrap_or_default()
}

#[test]
fn back_goes_back_while_the_module_can_then_leaves() {
	let mut host = start(ROUTER);
	assert_eq!(label(&host), "Depth 2");
	assert_eq!(host.commits(), 1);

	assert_eq!(host.system_back().unwrap(), EventOutcome::Handled);
	// The host flushed after Back, so the popped route is on screen.
	assert_eq!(label(&host), "Depth 1");
	assert_eq!(host.commits(), 2);

	assert_eq!(host.system_back().unwrap(), EventOutcome::Handled);
	assert_eq!(label(&host), "Depth 0");

	// Nothing left to go back to: the platform leaves, and nothing changed.
	assert_eq!(host.system_back().unwrap(), EventOutcome::Unhandled);
	assert_eq!(label(&host), "Depth 0");
	assert_eq!(host.commits(), 3);
}

#[test]
fn a_module_without_the_export_leaves_back_to_the_platform() {
	let mut host = start(WITHOUT_BACK);

	assert_eq!(host.system_back().unwrap(), EventOutcome::Unhandled);
	assert_eq!(host.commits(), 1);
}

#[test]
fn an_export_of_the_wrong_type_is_refused_at_load() {
	let wat = WITHOUT_BACK.replace("\n)", "\n (func (export \"leptoskit_system_back\"))\n)");

	let Err(WasmError::Compile(message)) = WasmHost::headless(wat.as_bytes()) else {
		panic!("a mistyped leptoskit_system_back loads");
	};
	assert_eq!(
		message,
		"leptoskit_system_back must take nothing and return an i32"
	);
}

#[test]
fn a_trap_in_back_is_reported() {
	let wat = WITHOUT_BACK.replace(
		"\n)",
		"\n (func (export \"leptoskit_system_back\") (result i32) unreachable)\n)",
	);
	let mut host = start(&wat);

	assert!(matches!(host.system_back(), Err(WasmError::Trap(_))));
}
