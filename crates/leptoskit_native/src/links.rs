//! Opens the link a press landed on, where no browser does it for the app.
//!
//! The web renders a link as an `<a>` and the browser follows it. Everywhere
//! else a link is a pressable, so a press no handler took is answered here:
//! an absolute link is handed to the operating system through
//! `leptoskit_plugin_linking`, which opens it in the person's browser, mail
//! or phone app.

/// Opens `href` outside the app, and says whether it was a link this
/// platform can open.
///
/// Only absolute HTTP(S), mailto, tel and sms links are opened. A relative
/// link is a place inside the app, which is the router's to handle.
#[cfg(all(
	feature = "links",
	any(
		target_os = "macos",
		target_os = "windows",
		all(target_os = "linux", not(target_env = "ohos"))
	)
))]
pub(crate) fn open(href: &str) -> bool {
	let Ok(link) = leptoskit_plugin_linking::Link::parse(href) else {
		return false;
	};

	// The launcher may start a helper and wait for it, and a press is
	// answered on the thread that draws.
	let opening = std::thread::Builder::new()
		.name("leptoskit-link".to_owned())
		.spawn(move || {
			// Nothing waits for the launcher, so a link it refuses is a
			// press that did nothing, as with a link no app handles.
			leptoskit_plugin_linking::desktop::open(&link).ok();
		});

	opening.is_ok()
}

/// Opens `href` outside the app, and says whether it was a link this
/// platform can open.
///
/// A guest asks its host, which opens the link if the app selected the
/// `linking` plugin. Without the plugin the press does nothing.
#[cfg(all(feature = "links", feature = "guest", target_arch = "wasm32"))]
pub(crate) fn open(href: &str) -> bool {
	let Ok(link) = leptoskit_plugin_linking::Link::parse(href) else {
		return false;
	};
	let Ok(linking) = leptoskit_plugin_linking::guest::Linking::new() else {
		return false;
	};

	crate::guest::spawn(async move {
		// A host that refuses the link leaves the press doing nothing.
		linking.open(&link).await.ok();
	});

	true
}

/// Without the `links` feature, or where the linking plugin has no way to
/// open a link from Rust, a pressed link is left to the app's own handler.
#[cfg(not(any(
	all(
		feature = "links",
		any(
			target_os = "macos",
			target_os = "windows",
			all(target_os = "linux", not(target_env = "ohos"))
		)
	),
	all(feature = "links", feature = "guest", target_arch = "wasm32")
)))]
pub(crate) const fn open(_href: &str) -> bool {
	false
}
