//! The screen that joins a computer running `leptoskit dev`.

use std::sync::Arc;

use leptoskit::prelude::Button;
use leptoskit::prelude::Element;
use leptoskit::prelude::Get;
use leptoskit::prelude::GetUntracked;
use leptoskit::prelude::RwSignal;
use leptoskit::prelude::Set;
use leptoskit::prelude::Text;
use leptoskit::prelude::TextInput;
use leptoskit::prelude::View;
use leptoskit_plugin_dev_client::guest_url;

/// What the screen does when someone connects or scans.
#[derive(Clone)]
pub struct Session {
	/// `android` or `ios`.
	pub platform: &'static str,
	/// The address in the field. Kept outside the screen so a re-render
	/// does not wipe what the person typed.
	pub address: RwSignal<String>,
	/// The line under the buttons.
	pub status: RwSignal<String>,
	/// Called with the canonical guest URL.
	pub connect: Arc<dyn Fn(String) + Send + Sync>,
	/// Reads one code, when the phone has a reader.
	pub scan: Arc<dyn Fn() -> Option<String> + Send + Sync>,
}

/// The `LeptosKit` app's only screen.
pub fn home(session: Session) -> Element {
	let address = session.address;
	let status = session.status;
	let platform = session.platform;
	let connect = session.connect;
	let scan = session.scan;
	let typed = address;
	let message = status;
	let connect_scan = Arc::clone(&connect);

	View()
		.class("min-h-screen flex-1 justify-center gap-6 bg-slate-950 p-6")
		.child(Text("LeptosKit").class("text-4xl font-bold text-white"))
		.child(
			Text("Scan the code from leptoskit dev, or paste the address.")
				.class("text-base text-slate-300"),
		)
		.child(
			TextInput()
				.class("rounded-xl bg-slate-900 px-4 py-3 text-white")
				.value(typed.get_untracked())
				.placeholder("http://192.168.1.10:43123/_leptoskit/dev")
				.on_change(move |text| typed.set(text.to_owned())),
		)
		.child(
			Button("Connect")
				.class("rounded-lg bg-indigo-500 px-6 py-3 font-semibold text-white")
				.on_press(move || submit(platform, &typed.get(), &message, &connect)),
		)
		.child(
			Button("Scan")
				.class("rounded-lg bg-slate-800 px-6 py-3 font-semibold text-white")
				.on_press(move || {
					#[cfg(all(target_arch = "wasm32", feature = "native"))]
					{
						let _ = scan;
						message.set("Opening the camera…".to_owned());
						let message = message;
						let typed = typed;
						let connect = Arc::clone(&connect_scan);
						leptoskit::guest::spawn(async move {
							let Ok(client) = leptoskit_plugin_dev_client::guest::DevClient::new()
							else {
								message.set("Paste the address from the terminal.".to_owned());
								return;
							};
							match client.scan().await {
								Ok(text) => {
									typed.set(text.clone());
									submit(platform, &text, &message, &connect);
								}
								Err(_) => {
									message.set("Paste the address from the terminal.".to_owned());
								}
							}
						});
					}
					#[cfg(not(all(target_arch = "wasm32", feature = "native")))]
					{
						let Some(text) = scan() else {
							message.set("Paste the address from the terminal.".to_owned());
							return;
						};
						typed.set(text.clone());
						submit(platform, &text, &message, &connect_scan);
					}
				}),
		)
		.child(Text(move || message.get()).class("text-base text-slate-300"))
}

fn submit(
	platform: &'static str,
	text: &str,
	message: &RwSignal<String>,
	connect: &Arc<dyn Fn(String) + Send + Sync>,
) {
	match guest_url(text, platform) {
		Ok(url) => {
			message.set("Waiting for the app…".to_owned());
			connect(url);
		}
		Err(error) => message.set(error.to_string()),
	}
}
