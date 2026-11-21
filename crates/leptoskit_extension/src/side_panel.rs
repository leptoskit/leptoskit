//! The side panel / sidebar: `chrome.sidePanel` on Chromium (Chrome, Edge,
//! Brave, Opera, Arc, Vivaldi) and `browser.sidebarAction` on Firefox, behind
//! one unified API. Safari has neither, so calls return [`Error::Unavailable`].

use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use serde_json::json;

use crate::Error;
use crate::Result;
use crate::browser::BrowserFamily;
use crate::browser::ExtensionBrowser;
use crate::js;

/// Which window or tab a side panel operation applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SidePanelTarget {
	/// Target a specific browser window by ID.
	Window(i64),
	/// Target a specific tab by ID (`chrome.sidePanel` / Firefox
	/// `sidebarAction`).
	Tab(i64),
	/// Target the currently active window.
	ActiveWindow,
}

/// Per-tab or default options for Chromium's `sidePanel.setOptions` /
/// `sidePanel.getOptions` (and mapped to `sidebarAction.setPanel` on Firefox).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidePanelOptions {
	/// Optional tab ID; omit to apply to the extension's default panel.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tab_id: Option<i64>,
	/// Relative extension page path (for example `"side_panel.html"` or
	/// `"side_panel.html#/notes"`).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub path: Option<String>,
	/// Whether the side panel is enabled for this tab or globally.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub enabled: Option<bool>,
}

impl SidePanelOptions {
	/// Starts an empty options builder.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Enables or disables the side panel for the target scope.
	#[must_use]
	pub const fn enabled(mut self, enabled: bool) -> Self {
		self.enabled = Some(enabled);
		self
	}

	/// Sets the HTML page path shown in the side panel.
	#[must_use]
	pub fn path(mut self, path: impl Into<String>) -> Self {
		self.path = Some(path.into());
		self
	}

	/// Scopes these options to one tab.
	#[must_use]
	pub const fn tab_id(mut self, tab_id: i64) -> Self {
		self.tab_id = Some(tab_id);
		self
	}
}

/// Controls whether clicking the toolbar action icon opens the side panel
/// directly (Chromium `sidePanel.setPanelBehavior`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelBehavior {
	/// When `true`, clicking the extension's toolbar button opens the side
	/// panel instead of a popup.
	#[serde(default)]
	pub open_panel_on_action_click: bool,
}

/// Which side of the browser window currently displays the side panel
/// (Chromium `sidePanel.getLayout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidePanelSide {
	/// Docked on the left side of the browser window.
	Left,
	/// Docked on the right side of the browser window.
	Right,
}

#[derive(Deserialize)]
struct RawLayout {
	side: SidePanelSide,
}

/// What side panel operations a browser supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct SidePanelCapabilities {
	/// Can open a side panel / sidebar programmatically after a user gesture.
	pub open: bool,
	/// Can open scoped to a specific tab ID (`sidePanel.open({ tabId })`).
	pub open_by_tab: bool,
	/// Can close programmatically (`sidePanel.close` / `sidebarAction.close`).
	pub close: bool,
	/// Can toggle open/closed (`sidebarAction.toggle`).
	pub toggle: bool,
	/// Can query whether the sidebar is open (`sidebarAction.isOpen`).
	pub is_open: bool,
	/// Can configure per-tab path and enabled state (`sidePanel.setOptions`).
	pub per_tab_options: bool,
	/// Can open the side panel when clicking the toolbar icon
	/// (`setPanelBehavior`).
	pub open_on_action_click: bool,
	/// Can update the sidebar title (`sidebarAction.setTitle`).
	pub set_title: bool,
	/// Can query left/right dock side (`sidePanel.getLayout`).
	pub layout: bool,
}

impl SidePanelCapabilities {
	/// Static capability summary for a browser family.
	#[must_use]
	pub const fn for_browser(browser: ExtensionBrowser) -> Self {
		match browser.family() {
			BrowserFamily::Chromium => {
				Self {
					open: true,
					open_by_tab: true,
					close: true,
					toggle: false,
					is_open: false,
					per_tab_options: true,
					open_on_action_click: true,
					set_title: false,
					layout: true,
				}
			}
			BrowserFamily::Firefox => {
				Self {
					open: true,
					open_by_tab: false,
					close: true,
					toggle: true,
					is_open: true,
					per_tab_options: true,
					open_on_action_click: false,
					set_title: true,
					layout: false,
				}
			}
			BrowserFamily::Safari | BrowserFamily::Unknown => {
				Self {
					open: false,
					open_by_tab: false,
					close: false,
					toggle: false,
					is_open: false,
					per_tab_options: false,
					open_on_action_click: false,
					set_title: false,
					layout: false,
				}
			}
		}
	}
}

/// Whether this browser has a side panel or sidebar API.
#[must_use]
pub fn available() -> bool {
	js::has("sidePanel.open") || js::has("sidebarAction.open")
}

/// Returns the expected side panel capabilities for the active browser.
#[must_use]
pub fn capabilities() -> SidePanelCapabilities {
	SidePanelCapabilities::for_browser(crate::browser::current())
}

/// Opens the side panel in the window `window`. Browsers allow it only
/// right after a user gesture.
///
/// # Errors
///
/// [`Error::Unavailable`] in Safari; otherwise when the browser won't open
/// it now.
pub async fn open(window: i64) -> Result<()> {
	open_target(SidePanelTarget::Window(window)).await
}

/// Opens the side panel for a specific tab on Chromium
/// (`sidePanel.open({ tabId })`) or opens the Firefox sidebar
/// (`sidebarAction.open()`).
///
/// # Errors
///
/// [`Error::Unavailable`] in Safari or outside a browser.
pub async fn open_for_tab(tab_id: i64) -> Result<()> {
	open_target(SidePanelTarget::Tab(tab_id)).await
}

/// Opens the side panel for the given [`SidePanelTarget`].
///
/// # Errors
///
/// [`Error::Unavailable`] in Safari or outside a browser.
pub async fn open_target(target: SidePanelTarget) -> Result<()> {
	if js::has("sidePanel.open") {
		let options = match target {
			SidePanelTarget::Window(window_id) => json!({ "windowId": window_id }),
			SidePanelTarget::Tab(tab_id) => json!({ "tabId": tab_id }),
			SidePanelTarget::ActiveWindow => json!({}),
		};
		return js::call::<Value>("sidePanel.open", &[&options])
			.await
			.map(drop);
	}

	if js::has("sidebarAction.open") {
		return js::call::<Value>("sidebarAction.open", &[]).await.map(drop);
	}

	Err(Error::Unavailable("sidePanel".into()))
}

/// Closes the side panel or sidebar (`sidePanel.close` on Chromium 141+,
/// `sidebarAction.close` on Firefox).
///
/// # Errors
///
/// [`Error::Unavailable`] when the browser lacks programmatic close.
pub async fn close(target: SidePanelTarget) -> Result<()> {
	if js::has("sidePanel.close") {
		let options = match target {
			SidePanelTarget::Window(window_id) => json!({ "windowId": window_id }),
			SidePanelTarget::Tab(tab_id) => json!({ "tabId": tab_id }),
			SidePanelTarget::ActiveWindow => json!({}),
		};
		return js::call::<Value>("sidePanel.close", &[&options])
			.await
			.map(drop);
	}

	if js::has("sidebarAction.close") {
		return js::call::<Value>("sidebarAction.close", &[])
			.await
			.map(drop);
	}

	Err(Error::Unavailable("sidePanel.close".into()))
}

/// Toggles the sidebar open or closed (Firefox `sidebarAction.toggle`).
///
/// # Errors
///
/// [`Error::Unavailable`] on Chromium and Safari.
pub async fn toggle() -> Result<()> {
	if js::has("sidebarAction.toggle") {
		return js::call::<Value>("sidebarAction.toggle", &[])
			.await
			.map(drop);
	}

	Err(Error::Unavailable("sidebarAction.toggle".into()))
}

/// Queries whether the sidebar is open (Firefox `sidebarAction.isOpen`).
///
/// # Errors
///
/// [`Error::Unavailable`] on Chromium and Safari.
pub async fn is_open(window_id: Option<i64>) -> Result<bool> {
	if js::has("sidebarAction.isOpen") {
		let details = match window_id {
			Some(id) => json!({ "windowId": id }),
			None => json!({}),
		};
		return js::call("sidebarAction.isOpen", &[&details]).await;
	}

	Err(Error::Unavailable("sidebarAction.isOpen".into()))
}

/// Configures per-tab or default side panel options (`sidePanel.setOptions` on
/// Chromium; maps `path` to `sidebarAction.setPanel` on Firefox).
///
/// # Errors
///
/// [`Error::Unavailable`] in Safari or outside a browser.
pub async fn set_options(options: &SidePanelOptions) -> Result<()> {
	if js::has("sidePanel.setOptions") {
		return js::call::<Value>("sidePanel.setOptions", &[options])
			.await
			.map(drop);
	}

	if js::has("sidebarAction.setPanel") {
		let mut details = Map::new();
		if let Some(tab_id) = options.tab_id {
			details.insert("tabId".into(), json!(tab_id));
		}
		if let Some(path) = &options.path {
			details.insert("panel".into(), json!(path));
		} else if options.enabled == Some(false) {
			details.insert("panel".into(), Value::Null);
		}
		return js::call::<Value>("sidebarAction.setPanel", &[&Value::Object(details)])
			.await
			.map(drop);
	}

	Err(Error::Unavailable("sidePanel.setOptions".into()))
}

/// Reads the current side panel options for a tab or globally
/// (`sidePanel.getOptions` on Chromium; `sidebarAction.getPanel` on Firefox).
///
/// # Errors
///
/// [`Error::Unavailable`] in Safari or outside a browser.
pub async fn get_options(tab_id: Option<i64>) -> Result<SidePanelOptions> {
	if js::has("sidePanel.getOptions") {
		let query = match tab_id {
			Some(id) => json!({ "tabId": id }),
			None => json!({}),
		};
		return js::call("sidePanel.getOptions", &[&query]).await;
	}

	if js::has("sidebarAction.getPanel") {
		let query = match tab_id {
			Some(id) => json!({ "tabId": id }),
			None => json!({}),
		};
		let panel: String = js::call("sidebarAction.getPanel", &[&query]).await?;
		return Ok(SidePanelOptions {
			tab_id,
			path: (!panel.is_empty()).then_some(panel),
			enabled: Some(true),
		});
	}

	Err(Error::Unavailable("sidePanel.getOptions".into()))
}

/// Configures whether clicking the toolbar icon opens the side panel directly
/// (Chromium `sidePanel.setPanelBehavior`).
///
/// # Errors
///
/// [`Error::Unavailable`] on Firefox and Safari.
pub async fn set_behavior(behavior: PanelBehavior) -> Result<()> {
	if js::has("sidePanel.setPanelBehavior") {
		return js::call::<Value>("sidePanel.setPanelBehavior", &[&behavior])
			.await
			.map(drop);
	}

	Err(Error::Unavailable("sidePanel.setPanelBehavior".into()))
}

/// Reads the current toolbar-click side panel behavior
/// (Chromium `sidePanel.getPanelBehavior`).
///
/// # Errors
///
/// [`Error::Unavailable`] on Firefox and Safari.
pub async fn get_behavior() -> Result<PanelBehavior> {
	if js::has("sidePanel.getPanelBehavior") {
		return js::call("sidePanel.getPanelBehavior", &[]).await;
	}

	Err(Error::Unavailable("sidePanel.getPanelBehavior".into()))
}

/// Returns which side of the browser window currently hosts the side panel
/// (Chromium `sidePanel.getLayout`).
///
/// # Errors
///
/// [`Error::Unavailable`] on Firefox, Safari, and older Chromium builds.
pub async fn get_layout() -> Result<SidePanelSide> {
	if js::has("sidePanel.getLayout") {
		let layout: RawLayout = js::call("sidePanel.getLayout", &[]).await?;
		return Ok(layout.side);
	}

	Err(Error::Unavailable("sidePanel.getLayout".into()))
}

/// Sets the sidebar title (Firefox `sidebarAction.setTitle`).
///
/// # Errors
///
/// [`Error::Unavailable`] on Chromium and Safari.
pub async fn set_title(title: &str, target: SidePanelTarget) -> Result<()> {
	if js::has("sidebarAction.setTitle") {
		let mut details = Map::new();
		details.insert("title".into(), json!(title));
		match target {
			SidePanelTarget::Window(window_id) => {
				details.insert("windowId".into(), json!(window_id));
			}
			SidePanelTarget::Tab(tab_id) => {
				details.insert("tabId".into(), json!(tab_id));
			}
			SidePanelTarget::ActiveWindow => {}
		}
		return js::call::<Value>("sidebarAction.setTitle", &[&Value::Object(details)])
			.await
			.map(drop);
	}

	Err(Error::Unavailable("sidebarAction.setTitle".into()))
}

/// Reads the sidebar title (Firefox `sidebarAction.getTitle`).
///
/// # Errors
///
/// [`Error::Unavailable`] on Chromium and Safari.
pub async fn get_title(target: SidePanelTarget) -> Result<String> {
	if js::has("sidebarAction.getTitle") {
		let details = match target {
			SidePanelTarget::Window(window_id) => json!({ "windowId": window_id }),
			SidePanelTarget::Tab(tab_id) => json!({ "tabId": tab_id }),
			SidePanelTarget::ActiveWindow => json!({}),
		};
		return js::call("sidebarAction.getTitle", &[&details]).await;
	}

	Err(Error::Unavailable("sidebarAction.getTitle".into()))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::browser::with_browser;

	#[test]
	fn side_panel_capabilities_distinguish_chromium_firefox_and_safari() {
		let chrome = SidePanelCapabilities::for_browser(ExtensionBrowser::Chrome);
		assert!(chrome.open);
		assert!(chrome.open_by_tab);
		assert!(chrome.open_on_action_click);
		assert!(chrome.layout);
		assert!(!chrome.toggle);

		let firefox = SidePanelCapabilities::for_browser(ExtensionBrowser::Firefox);
		assert!(firefox.open);
		assert!(firefox.toggle);
		assert!(firefox.is_open);
		assert!(firefox.set_title);
		assert!(!firefox.open_on_action_click);

		let safari = SidePanelCapabilities::for_browser(ExtensionBrowser::Safari);
		assert!(!safari.open);
		assert!(!safari.close);

		let from_current = with_browser(ExtensionBrowser::Firefox, capabilities);
		assert_eq!(from_current, firefox);
	}

	#[test]
	fn options_builder_serializes_cleanly() {
		let opts = SidePanelOptions::new()
			.tab_id(42)
			.path("side_panel.html#/chat")
			.enabled(true);
		let value = serde_json::to_value(&opts).unwrap();
		assert_eq!(value["tabId"], 42);
		assert_eq!(value["path"], "side_panel.html#/chat");
		assert_eq!(value["enabled"], true);
	}
}
