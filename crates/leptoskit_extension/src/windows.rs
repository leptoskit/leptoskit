//! Browser windows: query, create, focus, and close windows across Chromium,
//! Firefox, and Safari.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::Result;
use crate::js;

/// Window presentation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowState {
	/// Standard window.
	Normal,
	/// Minimized window.
	Minimized,
	/// Maximized window.
	Maximized,
	/// Fullscreen window.
	Fullscreen,
	/// Locked fullscreen (`ChromeOS` kiosk).
	#[serde(rename = "locked-fullscreen")]
	LockedFullscreen,
}

/// Browser window kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowType {
	/// Regular browser window with tabs and address bar.
	Normal,
	/// Standalone popup window.
	Popup,
	/// Panel window.
	Panel,
	/// Standalone app window.
	App,
	/// `DevTools` window.
	Devtools,
}

/// Summary of one browser window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
	/// The window's ID.
	#[serde(default)]
	pub id: Option<i64>,
	/// Whether this window currently has input focus.
	#[serde(default)]
	pub focused: bool,
	/// Whether this window is private / incognito.
	#[serde(default)]
	pub incognito: bool,
	/// Whether the window stays on top when supported.
	#[serde(default)]
	pub always_on_top: bool,
	/// Current window state.
	#[serde(default)]
	pub state: Option<WindowState>,
	/// Window type.
	#[serde(default, rename = "type")]
	pub kind: Option<WindowType>,
	/// Outer width in CSS pixels.
	#[serde(default)]
	pub width: Option<i64>,
	/// Outer height in CSS pixels.
	#[serde(default)]
	pub height: Option<i64>,
}

/// Options for [`create`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWindow {
	/// Initial URL to open in the new window.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub url: Option<String>,
	/// Window kind (`normal` or `popup`).
	#[serde(rename = "type", skip_serializing_if = "Option::is_none")]
	pub kind: Option<WindowType>,
	/// Desired width in CSS pixels.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub width: Option<i64>,
	/// Desired height in CSS pixels.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub height: Option<i64>,
	/// Whether to focus the new window immediately.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub focused: Option<bool>,
	/// Whether to create a private / incognito window.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub incognito: Option<bool>,
	/// Desired initial window state.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub state: Option<WindowState>,
}

impl CreateWindow {
	/// Starts a new browser window request.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Sets whether the new window receives focus immediately.
	#[must_use]
	pub const fn focused(mut self, focused: bool) -> Self {
		self.focused = Some(focused);
		self
	}

	/// Sets the window kind (`WindowType::Popup` for a standalone tool window).
	#[must_use]
	pub const fn kind(mut self, kind: WindowType) -> Self {
		self.kind = Some(kind);
		self
	}

	/// Sets width and height in CSS pixels.
	#[must_use]
	pub const fn size(mut self, width: i64, height: i64) -> Self {
		self.width = Some(width);
		self.height = Some(height);
		self
	}

	/// Sets the initial URL.
	#[must_use]
	pub fn url(mut self, url: impl Into<String>) -> Self {
		self.url = Some(url.into());
		self
	}
}

/// Returns the current browser window.
///
/// # Errors
///
/// [`crate::Error::Unavailable`] outside a browser.
pub async fn current() -> Result<Window> {
	js::call("windows.getCurrent", &[]).await
}

/// Returns the most recently focused browser window.
///
/// # Errors
///
/// [`crate::Error::Unavailable`] outside a browser.
pub async fn last_focused() -> Result<Window> {
	js::call("windows.getLastFocused", &[]).await
}

/// Lists every open browser window.
///
/// # Errors
///
/// [`crate::Error::Unavailable`] outside a browser.
pub async fn all() -> Result<Vec<Window>> {
	js::call("windows.getAll", &[]).await
}

/// Looks up one window by ID.
///
/// # Errors
///
/// [`crate::Error::Unavailable`] outside a browser or if the window does not
/// exist.
pub async fn get(window_id: i64) -> Result<Window> {
	js::call("windows.get", &[&window_id]).await
}

/// Creates a new browser window.
///
/// # Errors
///
/// [`crate::Error::Unavailable`] outside a browser.
pub async fn create(options: &CreateWindow) -> Result<Window> {
	js::call("windows.create", &[options]).await
}

/// Brings a window to the front and focuses it.
///
/// # Errors
///
/// [`crate::Error::Unavailable`] outside a browser.
pub async fn focus(window_id: i64) -> Result<Window> {
	js::call("windows.update", &[&window_id, &json!({ "focused": true })]).await
}

/// Closes a window.
///
/// # Errors
///
/// [`crate::Error::Unavailable`] outside a browser.
pub async fn close(window_id: i64) -> Result<()> {
	js::call::<Value>("windows.remove", &[&window_id])
		.await
		.map(drop)
}
