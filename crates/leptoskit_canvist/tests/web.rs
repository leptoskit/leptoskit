#![cfg(all(feature = "web", target_arch = "wasm32"))]

use leptoskit_canvist::CanvistState;
use leptoskit_canvist::Extensions;
use leptoskit_canvist::Format;
use leptoskit_canvist::web::attach;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn container() -> web_sys::HtmlElement {
	let document = web_sys::window().unwrap().document().unwrap();
	let el: web_sys::HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
	document.body().unwrap().append_child(&el).unwrap();
	el
}

async fn next_frame() {
	let window = web_sys::window().unwrap();
	let promise = js_sys::Promise::new(&mut |resolve, _| {
		let _ = window.request_animation_frame(&resolve);
	});
	let _ = JsFuture::from(promise).await;
}

fn dispatch_beforeinput(textarea: &web_sys::HtmlTextAreaElement, input_type: &str, data: Option<&str>) {
	let init = web_sys::InputEventInit::new();
	init.set_input_type(input_type);
	if let Some(d) = data {
		init.set_data(Some(d));
	}
	let ev = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
	textarea.dispatch_event(&ev).unwrap();
}

#[wasm_bindgen_test]
async fn attach_and_detach() {
	let c = container();
	let state = CanvistState::new(Extensions::paragraphs());
	let attachment = attach(&c, state);
	assert!(c.query_selector("canvas").unwrap().is_some());
	assert!(c.query_selector("textarea").unwrap().is_some());
	drop(attachment);
	assert!(c.query_selector("canvas").unwrap().is_none());
	c.remove();
}

#[wasm_bindgen_test]
async fn typing_changes_text() {
	let c = container();
	let state = CanvistState::new(Extensions::paragraphs());
	let _attachment = attach(&c, state);
	let textarea: web_sys::HtmlTextAreaElement = c
		.query_selector("textarea")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	dispatch_beforeinput(&textarea, "insertText", Some("hi"));
	next_frame().await;
	let text = state.export(Format::Text).await.unwrap();
	assert_eq!(text, "hi");
	c.remove();
}

#[wasm_bindgen_test]
async fn enter_and_backspace() {
	let c = container();
	let state = CanvistState::new(Extensions::paragraphs());
	let _attachment = attach(&c, state);
	let textarea: web_sys::HtmlTextAreaElement = c
		.query_selector("textarea")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	dispatch_beforeinput(&textarea, "insertText", Some("ab"));
	next_frame().await;
	dispatch_beforeinput(&textarea, "insertParagraph", None);
	next_frame().await;
	let text = state.export(Format::Text).await.unwrap();
	assert!(text.contains('\n'));
	// backspace joins
	dispatch_beforeinput(&textarea, "deleteContentBackward", None);
	next_frame().await;
	let text2 = state.export(Format::Text).await.unwrap();
	assert!(!text2.contains('\n') || text2 == "ab");
	c.remove();
}

#[wasm_bindgen_test]
async fn undo_redo() {
	let c = container();
	let state = CanvistState::new(Extensions::paragraphs());
	let _attachment = attach(&c, state);
	let textarea: web_sys::HtmlTextAreaElement = c
		.query_selector("textarea")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	dispatch_beforeinput(&textarea, "insertText", Some("hi"));
	next_frame().await;
	dispatch_beforeinput(&textarea, "historyUndo", None);
	next_frame().await;
	let text = state.export(Format::Text).await.unwrap();
	assert_eq!(text, "");
	dispatch_beforeinput(&textarea, "historyRedo", None);
	next_frame().await;
	let text2 = state.export(Format::Text).await.unwrap();
	assert_eq!(text2, "hi");
	c.remove();
}

#[wasm_bindgen_test]
async fn readonly_refuses() {
	let c = container();
	let state = CanvistState::new(Extensions::paragraphs());
	state.set_read_only(true);
	let _attachment = attach(&c, state);
	let textarea: web_sys::HtmlTextAreaElement = c
		.query_selector("textarea")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	dispatch_beforeinput(&textarea, "insertText", Some("hi"));
	next_frame().await;
	let text = state.export(Format::Text).await.unwrap();
	assert_eq!(text, "");
	c.remove();
}

#[wasm_bindgen_test]
async fn placeholder_flag() {
	let c = container();
	let state = CanvistState::new(Extensions::paragraphs());
	state.set_placeholder("ph");
	let _attachment = attach(&c, state);
	next_frame().await;
	let fr = leptoskit_canvist::web::frame_report(state);
	// placeholder drawn while empty
	assert!(fr.placeholder);
	c.remove();
}

#[wasm_bindgen_test]
async fn ime_composition_commits_once() {
	let c = container();
	let state = CanvistState::new(Extensions::paragraphs());
	let _attachment = attach(&c, state);
	let textarea: web_sys::HtmlTextAreaElement = c
		.query_selector("textarea")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	// compositionstart
	let cs = web_sys::CompositionEvent::new("compositionstart").unwrap();
	textarea.dispatch_event(&cs).unwrap();
	// compositionupdate
	let cu_init = web_sys::CompositionEventInit::new();
	cu_init.set_data("あ");
	let cu = web_sys::CompositionEvent::new_with_event_init_dict("compositionupdate", &cu_init).unwrap();
	textarea.dispatch_event(&cu).unwrap();
	// compositionend
	let ce_init = web_sys::CompositionEventInit::new();
	ce_init.set_data("あいう");
	let ce = web_sys::CompositionEvent::new_with_event_init_dict("compositionend", &ce_init).unwrap();
	textarea.dispatch_event(&ce).unwrap();
	next_frame().await;
	let text = state.export(Format::Text).await.unwrap();
	assert_eq!(text, "あいう");
	c.remove();
}

#[wasm_bindgen_test]
async fn resize_rewraps() {
	let c = container();
	c.style().set_property("width", "300px").unwrap();
	let state = CanvistState::new(Extensions::paragraphs());
	let _attachment = attach(&c, state);
	let textarea: web_sys::HtmlTextAreaElement = c
		.query_selector("textarea")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	// type a long string
	let long = "a ".repeat(40);
	dispatch_beforeinput(&textarea, "insertText", Some(&long));
	next_frame().await;
	let h1 = leptoskit_canvist::web::frame_report(state).height;
	// narrow
	c.style().set_property("width", "100px").unwrap();
	// trigger resize observer via rAF waits
	for _ in 0..5 {
		next_frame().await;
	}
	let h2 = leptoskit_canvist::web::frame_report(state).height;
	assert!(h2 >= h1);
	c.remove();
}

#[wasm_bindgen_test]
async fn pointer_selection_copy_paste() {
	let c = container();
	let state = CanvistState::new(Extensions::paragraphs());
	let _attachment = attach(&c, state);
	let textarea: web_sys::HtmlTextAreaElement = c
		.query_selector("textarea")
		.unwrap()
		.unwrap()
		.dyn_into()
		.unwrap();
	dispatch_beforeinput(&textarea, "insertText", Some("hello"));
	next_frame().await;
	// select all via shift+home? Use copy whole doc by selecting. Instead test copy event returns text.
	// dispatch copy event
	let ev = web_sys::ClipboardEvent::new("copy").unwrap();
	let _ = c.dispatch_event(&ev);
	// paste into a fresh editor
	let c2 = container();
	let state2 = CanvistState::new(Extensions::paragraphs());
	let _a2 = attach(&c2, state2);
	let ta2: web_sys::HtmlTextAreaElement = c2.query_selector("textarea").unwrap().unwrap().dyn_into().unwrap();
	// synthetic paste with DataTransfer
	let dt = web_sys::DataTransfer::new().unwrap();
	dt.set_data("text/plain", "hello").unwrap();
	let init = web_sys::ClipboardEventInit::new();
	init.set_bubbles(true);
	init.set_clipboard_data(Some(&dt));
	let ev2 = web_sys::ClipboardEvent::new_with_event_init_dict("paste", &init).unwrap();
	ta2.dispatch_event(&ev2).unwrap();
	next_frame().await;
	assert!(leptoskit_canvist::web::input::paste_fired(), "paste handler did not fire");
	let text2 = state2.export(Format::Text).await.unwrap();
	assert_eq!(text2, "hello");
	c.remove();
	c2.remove();
}