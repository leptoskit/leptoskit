//! Address-bar omnibox integration (`omnibox.*`).

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::Error;
use crate::Result;
use crate::js;

/// Where the user asked to open an omnibox selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Disposition {
	/// Replace the active tab.
	CurrentTab,
	/// Open in a new foreground tab.
	NewForegroundTab,
	/// Open in a new background tab.
	NewBackgroundTab,
}

/// One suggestion shown in the browser address bar dropdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
	/// Text placed in the address bar if chosen.
	pub content: String,
	/// Rich description shown beside the suggestion.
	pub description: String,
	/// Whether the suggestion may be deleted by the user (Chromium).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub deletable: Option<bool>,
}

/// Whether the browser exposes the `omnibox` API.
#[must_use]
pub fn available() -> bool {
	js::has("omnibox.setDefaultSuggestion")
}

/// Sets the default suggestion shown below the address bar while the user
/// types after the extension's omnibox keyword.
///
/// # Errors
///
/// [`Error::Unavailable`] outside a browser or when `"omnibox"` is not
/// declared in the manifest.
pub async fn set_default_suggestion(description: &str) -> Result<()> {
	if !js::has("omnibox.setDefaultSuggestion") {
		return Err(Error::Unavailable("omnibox.setDefaultSuggestion".into()));
	}

	js::call::<Value>(
		"omnibox.setDefaultSuggestion",
		&[&json!({ "description": description })],
	)
	.await
	.map(drop)
}
