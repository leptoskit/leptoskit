//! `/`: join a computer running `leptoskit dev`.

use std::sync::Arc;

use leptoskit::prelude::Element;
use leptoskit::prelude::RwSignal;
use leptoskit::prelude::component;
use leptoskit_router::Route;

use crate::home::Session;
use crate::home::home;

/// Names the route `Join` instead of `Index`.
#[derive(Route, Clone, Debug, PartialEq)]
pub struct Join;

/// A session whose signals survive a re-render.
pub fn session() -> Session {
	Session {
		platform: if cfg!(feature = "ios") {
			"ios"
		} else {
			"android"
		},
		address: RwSignal::new(String::new()),
		status: RwSignal::new(String::new()),
		connect: Arc::new(follow),
		scan: Arc::new(scan),
	}
}

/// The phone's home screen.
#[component]
pub fn Page() -> Element {
	let session = reactive_graph::owner::use_context::<Session>()
		.expect("the app provides the dev session before rendering");
	home(session)
}

#[allow(
	clippy::needless_pass_by_value,
	reason = "the guest build moves the URL into the plugin call"
)]
fn follow(url: String) {
	#[cfg(all(target_arch = "wasm32", feature = "native"))]
	leptoskit::guest::spawn(async move {
		if let Ok(client) = leptoskit_plugin_dev_client::guest::DevClient::new() {
			let _ = client.connect(&url).await;
		}
	});
	#[cfg(not(all(target_arch = "wasm32", feature = "native")))]
	let _ = url;
}

fn scan() -> Option<String> {
	None
}
