//! Chromium offscreen documents (`chrome.offscreen`).
//!
//! Service workers in Manifest V3 cannot access DOM APIs, audio playback, or
//! clipboard helpers directly. Chromium provides `chrome.offscreen` (with the
//! `"offscreen"` permission) so an extension can host one hidden DOM document.
//! Firefox and Safari do not expose `offscreen`, so calls return
//! [`Error::Unavailable`] there.

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::Error;
use crate::Result;
use crate::js;

/// Why the extension is creating an offscreen document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Reason {
	/// Playing audio in the background.
	AudioPlayback,
	/// Reading or writing clipboard contents via DOM APIs.
	Clipboard,
	/// Parsing HTML/XML with `DOMParser`.
	DomParser,
	/// Scraping DOM content from an iframe.
	DomScraper,
	/// Reading geolocation.
	Geolocation,
	/// Communicating with an embedded iframe.
	IframeScripting,
	/// Accessing `window.localStorage`.
	LocalStorage,
	/// Querying `window.matchMedia` (such as color scheme).
	MatchMedia,
	/// Automated test harness.
	Testing,
	/// Capturing user media streams.
	UserMedia,
	/// Running WebRTC peer connections.
	WebRtc,
	/// Hosting dedicated workers.
	Workers,
}

/// Request to create an offscreen document via [`create`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateOffscreen {
	/// Relative URL of the extension HTML page to load offscreen.
	pub url: String,
	/// One or more declared reasons why the document is needed.
	pub reasons: Vec<Reason>,
	/// Human-readable justification reviewed by extension stores.
	pub justification: String,
}

impl CreateOffscreen {
	/// Builds an offscreen document request.
	#[must_use]
	pub fn new(
		url: impl Into<String>,
		reasons: impl IntoIterator<Item = Reason>,
		justification: impl Into<String>,
	) -> Self {
		Self {
			url: url.into(),
			reasons: reasons.into_iter().collect(),
			justification: justification.into(),
		}
	}
}

/// Whether `chrome.offscreen` is exposed in this browser.
#[must_use]
pub fn available() -> bool {
	js::has("offscreen.createDocument")
}

/// Creates the extension's offscreen document.
///
/// # Errors
///
/// [`Error::Unavailable`] on Firefox, Safari, or when the `"offscreen"`
/// permission is missing.
pub async fn create(request: &CreateOffscreen) -> Result<()> {
	if !js::has("offscreen.createDocument") {
		return Err(Error::Unavailable("offscreen.createDocument".into()));
	}

	js::call::<Value>("offscreen.createDocument", &[request])
		.await
		.map(drop)
}

/// Closes the active offscreen document.
///
/// # Errors
///
/// [`Error::Unavailable`] on browsers without `chrome.offscreen`.
pub async fn close() -> Result<()> {
	if !js::has("offscreen.closeDocument") {
		return Err(Error::Unavailable("offscreen.closeDocument".into()));
	}

	js::call::<Value>("offscreen.closeDocument", &[])
		.await
		.map(drop)
}

/// Checks whether an offscreen document is already open.
///
/// # Errors
///
/// [`Error::Unavailable`] on browsers without `chrome.offscreen`.
pub async fn has_document() -> Result<bool> {
	if !js::has("offscreen.hasDocument") {
		return Err(Error::Unavailable("offscreen.hasDocument".into()));
	}

	js::call("offscreen.hasDocument", &[]).await
}
