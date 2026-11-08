//! The root layout: the frame around every screen, with a stack navigator for
//! push / sheet / modal presentations.

use leptoskit::prelude::*;
use leptoskit_router::navigators::Presentation;
use leptoskit_router::navigators::Stack;
use leptoskit_router::navigators::StackGesture;

use crate::routes::Route;

/// Push routes by default; compose is a sheet and a photo is a modal.
pub fn navigator() -> Stack<Route> {
	Stack::new()
		.gesture(StackGesture::EdgeSwipe)
		.present(Presentation::Sheet, |route| {
			matches!(route, Route::Compose(_))
		})
		.present(Presentation::Modal, |route| {
			matches!(route, Route::Photo(_))
		})
}

/// Wraps the matched route in the app's frame.
#[component]
pub fn Layout(children: Children) -> Element {
	view! { <View class="flex-1 bg-white dark:bg-slate-950">{children}</View> }
}
