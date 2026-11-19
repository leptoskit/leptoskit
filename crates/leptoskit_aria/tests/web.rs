//! The menu in a real browser: the popover opens in the top layer, focus
//! moves into it and back, and the keys do what the menu pattern says.

#![cfg(target_arch = "wasm32")]

use std::sync::Arc;
use std::sync::Mutex;

use leptos::mount::mount_to;
use leptos::prelude::*;
use leptos::task::tick;
use leptoskit_aria::Menu;
use leptoskit_native::Element;
use leptoskit_native::components::Button;
use leptoskit_native::components::View;
use leptoskit_native::remember::ScreenState;
use leptoskit_native::web::render;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
	web_sys::window()
		.expect("window")
		.document()
		.expect("document")
}

fn container() -> web_sys::HtmlElement {
	let parent = document()
		.create_element("div")
		.expect("container")
		.dyn_into::<web_sys::HtmlElement>()
		.expect("HTML container");
	document()
		.body()
		.expect("body")
		.append_child(&parent)
		.expect("attach container");
	parent
}

fn by_id(id: &str) -> web_sys::HtmlElement {
	document()
		.get_element_by_id(id)
		.unwrap_or_else(|| panic!("an element with the id `{id}`"))
		.dyn_into()
		.expect("an HTML element")
}

/// Waits for the screen to rebuild and the browser to draw it.
async fn settle() {
	tick().await;
	let frame = js_sys::Promise::new(&mut |resolve, _| {
		web_sys::window()
			.unwrap()
			.request_animation_frame(&resolve)
			.unwrap();
	});
	wasm_bindgen_futures::JsFuture::from(frame).await.unwrap();
	tick().await;
}

/// Sends a key to an element. `false` means a handler took the key.
fn press(from: &web_sys::HtmlElement, key: &str) -> bool {
	let options = web_sys::KeyboardEventInit::new();
	options.set_key(key);
	options.set_bubbles(true);
	options.set_cancelable(true);
	from.dispatch_event(
		&web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &options).unwrap(),
	)
	.unwrap()
}

fn focused() -> Option<String> {
	document().active_element().map(|element| element.id())
}

#[wasm_bindgen_test]
#[expect(
	clippy::cognitive_complexity,
	reason = "one browser session, read from top to bottom"
)]
async fn a_menu_opens_moves_with_the_arrows_and_gives_focus_back() {
	let parent = container();
	let open = ArcRwSignal::new(false);
	let chosen = Arc::new(Mutex::new(Vec::new()));
	let mount = mount_to(parent.clone(), {
		let (open, chosen) = (open.clone(), Arc::clone(&chosen));

		move || {
			let state = ScreenState::new();
			state.provide();
			let screen = move || -> Element {
				let choose = |name: &'static str| {
					let chosen = Arc::clone(&chosen);

					move || chosen.lock().unwrap().push(name)
				};
				let toggled = open.clone();

				View().class("p-10 items-start").child(
					Menu::new("account", open.get())
						.button(Button("Account"))
						.item("Profile", choose("profile"))
						.item("Billing", choose("billing"))
						.item("Sign out", choose("sign out"))
						.on_toggle(move |open| toggled.set(open))
						.into(),
				)
			};

			move || render(state.build(&screen))
		}
	});
	settle().await;

	let button = by_id("account-button");
	let list = by_id("account-menu");
	let open_now = || list.matches(":popover-open").unwrap();
	assert_eq!(
		button.get_attribute("aria-haspopup").as_deref(),
		Some("menu")
	);
	assert_eq!(
		button.get_attribute("aria-expanded").as_deref(),
		Some("false")
	);
	assert_eq!(list.get_attribute("role").as_deref(), Some("menu"));
	assert!(!open_now());

	// Opening moves focus to the first item.
	button.focus().unwrap();
	button.click();
	settle().await;
	assert!(open_now());
	assert_eq!(
		by_id("account-button")
			.get_attribute("aria-expanded")
			.as_deref(),
		Some("true")
	);
	assert_eq!(focused().as_deref(), Some("account-item-0"));
	assert_eq!(by_id("account-item-0").tab_index(), -1);
	assert_eq!(
		by_id("account-item-0").get_attribute("role").as_deref(),
		Some("menuitem")
	);

	// The arrows move through the items and wrap, choosing nothing.
	assert!(!press(&by_id("account-item-0"), "ArrowDown"), "taken");
	settle().await;
	assert_eq!(focused().as_deref(), Some("account-item-1"));
	press(&by_id("account-item-1"), "End");
	settle().await;
	assert_eq!(focused().as_deref(), Some("account-item-2"));
	press(&by_id("account-item-2"), "ArrowDown");
	settle().await;
	assert_eq!(focused().as_deref(), Some("account-item-0"));
	press(&by_id("account-item-0"), "ArrowUp");
	settle().await;
	assert_eq!(focused().as_deref(), Some("account-item-2"));
	press(&by_id("account-item-2"), "Home");
	settle().await;
	assert_eq!(focused().as_deref(), Some("account-item-0"));
	assert!(chosen.lock().unwrap().is_empty());
	assert!(open_now());

	// Tab closes the menu and focus goes back to the button.
	assert!(!press(&by_id("account-item-0"), "Tab"), "taken");
	settle().await;
	assert!(!open_now());
	assert!(!open.get_untracked());
	assert_eq!(focused().as_deref(), Some("account-button"));

	// Choosing an item runs it, closes the menu, and gives focus back.
	by_id("account-button").click();
	settle().await;
	assert_eq!(focused().as_deref(), Some("account-item-0"));
	press(&by_id("account-item-0"), "ArrowDown");
	settle().await;
	by_id("account-item-1").click();
	settle().await;
	assert_eq!(*chosen.lock().unwrap(), ["billing"]);
	assert!(!open_now());
	assert_eq!(focused().as_deref(), Some("account-button"));

	// Escape and a press outside reach the app as a dismissal: the browser
	// hides the popover, and the menu asks to close.
	by_id("account-button").click();
	settle().await;
	assert!(open_now());
	list.hide_popover().unwrap();
	settle().await;
	assert!(!open.get_untracked());
	assert_eq!(focused().as_deref(), Some("account-button"));

	drop(mount);
	parent.remove();
}
