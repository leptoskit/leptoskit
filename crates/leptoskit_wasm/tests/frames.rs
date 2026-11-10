//! Frame stats: every guest commit records what it cost, and the frame log
//! writes each record as a JSON line.

use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_native::NativeEvent;
use leptoskit_wasm::Entry;
use leptoskit_wasm::WasmHost;

/// A label and a button; pressing the button rewrites the label.
const COUNTER_WAT: &str = r#"
(module
  (import "leptoskit" "node_create" (func $node_create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $node_set_props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $node_insert (param i64 i64 i32)))
  (import "leptoskit" "commit" (func $commit))
  (memory (export "memory") 1)
  (data (i32.const 64) "{\"text\":\"Count: 0\"}")
  (data (i32.const 96) "{\"text\":\"Count: 1\"}")
  (func (export "leptoskit_main")
    (call $node_create (i64.const 1) (i32.const 0))
    (call $node_create (i64.const 2) (i32.const 1))
    (call $node_create (i64.const 3) (i32.const 6))
    (call $node_set_props (i64.const 2) (i32.const 64) (i32.const 19))
    (call $node_insert (i64.const 1) (i64.const 2) (i32.const 0))
    (call $node_insert (i64.const 1) (i64.const 3) (i32.const 1))
    (call $commit))
  (func (export "leptoskit_event")
    (param $id i64) (param $kind i32) (param $ptr i32) (param $len i32) (result i32)
    (call $node_set_props (i64.const 2) (i32.const 96) (i32.const 19))
    (call $commit)
    (i32.const 1))
)
"#;

/// A frame log destination the test can read back.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Write for Log {
	fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
		self.0.lock().unwrap().extend_from_slice(bytes);
		Ok(bytes.len())
	}

	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}

#[test]
fn every_commit_records_its_entry_costs_and_size() {
	let mut host = WasmHost::headless(COUNTER_WAT.as_bytes()).expect("compile wat");
	assert!(host.last_frame().is_none(), "nothing has committed");
	let log = Log::default();
	host.set_frame_log(Some(Box::new(log.clone())));

	host.start(320.0, 480.0).expect("start");
	let first = host.last_frame().expect("the first commit").clone();
	assert_eq!(
		(first.commit, first.entry, first.nodes),
		(1, Entry::Main, 3)
	);
	// Three creates and two inserts, then three frames.
	assert!(first.mutations >= 5, "{first:?}");

	let surface = host.surface();
	let button = surface.children(surface.root().unwrap())[1];
	host.dispatch(button, &NativeEvent::Press).expect("press");
	let second = host.last_frame().expect("the second commit").clone();
	assert_eq!(
		(second.commit, second.entry, second.nodes),
		(2, Entry::Event, 3)
	);
	assert_eq!(second.mutations, 1, "only the label's props changed");
	assert!(second.at_ms >= first.at_ms);

	let lines: Vec<serde_json::Value> = String::from_utf8(log.0.lock().unwrap().clone())
		.unwrap()
		.lines()
		.map(|line| serde_json::from_str(line).expect("each line is JSON"))
		.collect();
	assert_eq!(lines.len(), 2);
	assert_eq!(lines[0]["entry"], "main");
	assert_eq!(lines[1]["entry"], "event");
	assert_eq!(lines[1]["commit"], 2);

	for key in [
		"at_ms",
		"guest_us",
		"commit_us",
		"layout_us",
		"mutations",
		"nodes",
	] {
		assert!(lines[1][key].is_u64(), "{key} is a count");
	}

	host.set_frame_log(None);
	host.dispatch(button, &NativeEvent::Press).expect("press");
	let log = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
	assert_eq!(log.lines().count(), 2, "the log is off");
}
