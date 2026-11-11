//! A compiled guest keeps what a screen remembers across the rebuilds the
//! native runtime runs, and releases it when the screen goes.

use leptoskit_native::HeadlessBackend;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeKind;
use leptoskit_wasm::WasmHost;

#[test]
#[ignore = "run test:plugins:guest to compile and load the Rust wasm fixture"]
fn remembered_state_survives_a_rebuild_and_leaves_with_its_screen() {
	let path = std::env::var("LEPTOSKIT_REBUILD_GUEST").expect("fixture wasm path");
	let mut host = WasmHost::headless(&std::fs::read(path).expect("fixture wasm"))
		.expect("the rebuild guest instantiates");
	host.start(320.0, 480.0).expect("the rebuild guest mounts");
	assert_eq!(texts(&host), ["Tap", "Remembered: 0", "Fresh: 0", "Leave"]);

	press(&mut host, 0);
	assert_eq!(
		texts(&host),
		["Tap", "Remembered: 1", "Fresh: 0", "Leave"],
		"the press rebuilt the screen: the remembered count kept its value, the fresh one reset"
	);

	press(&mut host, 0);
	assert_eq!(texts(&host), ["Tap", "Remembered: 2", "Fresh: 0", "Leave"]);

	press(&mut host, 3);
	assert_eq!(texts(&host), ["Back"]);

	press(&mut host, 0);
	assert_eq!(
		texts(&host),
		["Tap", "Remembered: 0", "Fresh: 0", "Leave"],
		"leaving the screen released what it remembered"
	);
}

/// The text of each child of the root, in order.
fn texts(host: &WasmHost<HeadlessBackend>) -> Vec<String> {
	let surface = host.surface();
	let root = surface.root().expect("a root node");
	surface
		.children(root)
		.iter()
		.map(|child| surface.props(*child).text.clone().unwrap_or_default())
		.collect()
}

/// Presses the button at `index` among the root's children.
fn press(host: &mut WasmHost<HeadlessBackend>, index: usize) {
	let surface = host.surface();
	let root = surface.root().expect("a root node");
	let button = surface.children(root)[index];
	assert_eq!(surface.kind(button), NativeKind::Button);
	host.dispatch(button, &NativeEvent::Press)
		.expect("the press reaches the guest");
}
