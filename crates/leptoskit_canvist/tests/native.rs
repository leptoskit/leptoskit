use std::future::Future;
use std::task::Poll;

use leptoskit_canvist::CanvistState;
use leptoskit_canvist::Extensions;
use leptoskit_canvist::Format;

#[test]
fn new_state_exports_empty() {
	let state = CanvistState::new(Extensions::paragraphs());
	let text = futures_block_on(state.export(Format::Text));
	assert_eq!(text.unwrap(), "");
}

#[test]
fn load_and_export_text() {
	let state = CanvistState::new(Extensions::paragraphs());
	state.load(Format::Text, "hello world").unwrap();
	let text = futures_block_on(state.export(Format::Text)).unwrap();
	assert_eq!(text, "hello world");
}

#[test]
fn load_and_export_markdown() {
	let state = CanvistState::new(Extensions::paragraphs());
	state.load(Format::Markdown, "hello").unwrap();
	let md = futures_block_on(state.export(Format::Markdown)).unwrap();
	assert!(md.contains("hello"));
}

#[test]
fn load_and_export_json_roundtrip() {
	let state = CanvistState::new(Extensions::paragraphs());
	state.load(Format::Text, "hi").unwrap();
	let json = futures_block_on(state.export(Format::Json)).unwrap();
	assert!(json.contains("hi"));
	let state2 = CanvistState::new(Extensions::paragraphs());
	state2.load(Format::Json, &json).unwrap();
	let text2 = futures_block_on(state2.export(Format::Text)).unwrap();
	assert_eq!(text2, "hi");
}

#[test]
fn change_record_fires() {
	use std::sync::Arc;
	use std::sync::Mutex;
	let state = CanvistState::new(Extensions::paragraphs());
	let seen = Arc::new(Mutex::new(Vec::new()));
	let seen2 = seen.clone();
	state.on_change(move |c| {
		seen2.lock().unwrap().push(c);
	});
	let _ = state.insert_text_native("hi");
	let v = seen.lock().unwrap();
	assert!(!v.is_empty());
	assert!(v[0].doc_changed);
}

fn futures_block_on<F: Future>(f: F) -> F::Output {
	// Simple poll loop for a ready future.
	let mut f = std::pin::pin!(f);
	let waker = std::task::Waker::noop();
	let mut cx = std::task::Context::from_waker(waker);
	loop {
		match f.as_mut().poll(&mut cx) {
			Poll::Ready(v) => return v,
			Poll::Pending => std::thread::yield_now(),
		}
	}
}
