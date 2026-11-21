//! Identify which browser the extension is running in (Chrome, Edge, Brave,
//! Opera, Arc, Vivaldi, Firefox, Safari) and branch cleanly per browser or
//! browser family.
//!
//! `leptoskit build --platform extension` compiles one Wasm client and packages
//! it for Chromium stores, Firefox, and Safari. Because the same Wasm binary
//! runs in every browser, use [`current`] or [`info`] at run time, or
//! [`BrowserSwitch`] to choose values by exact browser or family.

use std::cell::Cell;
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;

thread_local! {
	#[cfg_attr(target_os = "android", expect(clippy::missing_const_for_thread_local, reason = "Android fallback TLS: rust-clippy#13422"))]
	static BROWSER_OVERRIDE: Cell<Option<ExtensionBrowser>> = const { Cell::new(None) };
}

/// High-level browser engine / store family for an extension package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserFamily {
	/// Chrome, Edge, Brave, Opera, Arc, Vivaldi, and other Chromium browsers.
	Chromium,
	/// Firefox and Gecko-based browsers (`browser.*` / `moz-extension://`).
	Firefox,
	/// Safari on macOS and iOS (`safari-web-extension://`).
	Safari,
	/// Outside a browser or an unrecognized host.
	Unknown,
}

/// Specific browser hosting the extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionBrowser {
	/// Google Chrome.
	Chrome,
	/// Microsoft Edge (Chromium).
	Edge,
	/// Brave Browser (Chromium).
	Brave,
	/// Opera (Chromium).
	Opera,
	/// Arc Browser (Chromium).
	Arc,
	/// Vivaldi (Chromium).
	Vivaldi,
	/// Mozilla Firefox.
	Firefox,
	/// Apple Safari (macOS and iOS).
	Safari,
	/// Another Chromium-based browser (`chrome-extension://`).
	ChromiumOther,
	/// Outside a browser or unrecognized runtime.
	Unknown,
}

/// Runtime details about the browser hosting the extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserInfo {
	/// Specific browser detected.
	pub browser: ExtensionBrowser,
	/// Engine / store family.
	pub family: BrowserFamily,
	/// Reported browser product name, when exposed (for example by Firefox's
	/// `runtime.getBrowserInfo()`).
	pub name: Option<String>,
	/// Reported browser version string, when available.
	pub version: Option<String>,
	/// Reported vendor string, when available.
	pub vendor: Option<String>,
}

/// Low-level browser signals used to classify the host browser cleanly and
/// deterministically in tests and on `wasm32`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrowserSignals {
	/// Base URL returned by `runtime.getURL("")`, such as
	/// `chrome-extension://…`, `moz-extension://…`, or
	/// `safari-web-extension://…`.
	pub extension_url: Option<String>,
	/// Browser name returned by Firefox's `browser.runtime.getBrowserInfo()`.
	pub browser_info_name: Option<String>,
	/// Browser version returned by `browser.runtime.getBrowserInfo()`.
	pub browser_info_version: Option<String>,
	/// Browser vendor returned by `browser.runtime.getBrowserInfo()`.
	pub browser_info_vendor: Option<String>,
	/// `navigator.userAgent` string.
	pub user_agent: Option<String>,
	/// Brand tokens from `navigator.userAgentData.brands`.
	pub ua_brands: Vec<String>,
	/// Whether `navigator.brave` is present.
	pub has_brave: bool,
	/// Whether `globalThis.browser.runtime` is present.
	pub has_browser_namespace: bool,
	/// Whether `globalThis.chrome.runtime` is present.
	pub has_chrome_namespace: bool,
}

/// Error returned when parsing an unrecognized browser name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseBrowserError {
	input: String,
}

impl fmt::Display for ParseBrowserError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(formatter, "unknown extension browser `{}`", self.input)
	}
}

impl std::error::Error for ParseBrowserError {}

impl ExtensionBrowser {
	/// Canonical kebab-case browser name.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::Chrome => "chrome",
			Self::Edge => "edge",
			Self::Brave => "brave",
			Self::Opera => "opera",
			Self::Arc => "arc",
			Self::Vivaldi => "vivaldi",
			Self::Firefox => "firefox",
			Self::Safari => "safari",
			Self::ChromiumOther => "chromium",
			Self::Unknown => "unknown",
		}
	}

	/// Engine / store family this browser belongs to.
	#[must_use]
	pub const fn family(self) -> BrowserFamily {
		match self {
			Self::Chrome
			| Self::Edge
			| Self::Brave
			| Self::Opera
			| Self::Arc
			| Self::Vivaldi
			| Self::ChromiumOther => BrowserFamily::Chromium,
			Self::Firefox => BrowserFamily::Firefox,
			Self::Safari => BrowserFamily::Safari,
			Self::Unknown => BrowserFamily::Unknown,
		}
	}

	/// True for Arc.
	#[must_use]
	pub const fn is_arc(self) -> bool {
		matches!(self, Self::Arc)
	}

	/// True for Brave.
	#[must_use]
	pub const fn is_brave(self) -> bool {
		matches!(self, Self::Brave)
	}

	/// True for Google Chrome specifically.
	#[must_use]
	pub const fn is_chrome(self) -> bool {
		matches!(self, Self::Chrome)
	}

	/// True for any Chromium-based browser (Chrome, Edge, Brave, Opera, Arc,
	/// Vivaldi, or other Chromium).
	#[must_use]
	pub const fn is_chromium(self) -> bool {
		matches!(self.family(), BrowserFamily::Chromium)
	}

	/// True for Microsoft Edge.
	#[must_use]
	pub const fn is_edge(self) -> bool {
		matches!(self, Self::Edge)
	}

	/// True for Firefox.
	#[must_use]
	pub const fn is_firefox(self) -> bool {
		matches!(self, Self::Firefox)
	}

	/// True for Opera.
	#[must_use]
	pub const fn is_opera(self) -> bool {
		matches!(self, Self::Opera)
	}

	/// True for Safari.
	#[must_use]
	pub const fn is_safari(self) -> bool {
		matches!(self, Self::Safari)
	}

	/// True for Vivaldi.
	#[must_use]
	pub const fn is_vivaldi(self) -> bool {
		matches!(self, Self::Vivaldi)
	}

	/// The MV3 manifest key used for the sidebar / side panel surface in this
	/// browser, or `None` when unsupported (Safari).
	#[must_use]
	pub const fn manifest_sidebar_key(self) -> Option<&'static str> {
		match self.family() {
			BrowserFamily::Chromium => Some("side_panel"),
			BrowserFamily::Firefox => Some("sidebar_action"),
			BrowserFamily::Safari | BrowserFamily::Unknown => None,
		}
	}

	/// The extension URL scheme used by this browser.
	#[must_use]
	pub const fn protocol(self) -> Option<&'static str> {
		match self.family() {
			BrowserFamily::Chromium => Some("chrome-extension:"),
			BrowserFamily::Firefox => Some("moz-extension:"),
			BrowserFamily::Safari => Some("safari-web-extension:"),
			BrowserFamily::Unknown => None,
		}
	}

	/// Whether this browser exposes Firefox's `contextualIdentities` container
	/// tabs API.
	#[must_use]
	pub const fn supports_contextual_identities(self) -> bool {
		matches!(self, Self::Firefox)
	}

	/// Whether this browser supports a `DevTools` panel surface.
	#[must_use]
	pub const fn supports_devtools_panel(self) -> bool {
		matches!(
			self.family(),
			BrowserFamily::Chromium | BrowserFamily::Firefox
		)
	}

	/// Whether this browser supports `chrome_url_overrides.newtab`.
	#[must_use]
	pub const fn supports_new_tab_override(self) -> bool {
		matches!(
			self.family(),
			BrowserFamily::Chromium | BrowserFamily::Firefox
		)
	}

	/// Whether this browser supports Chrome's `chrome.offscreen` document API.
	#[must_use]
	pub const fn supports_offscreen(self) -> bool {
		matches!(self.family(), BrowserFamily::Chromium)
	}

	/// Whether this browser has a sidebar / side panel API (`chrome.sidePanel`
	/// on Chromium, `browser.sidebarAction` on Firefox).
	#[must_use]
	pub const fn supports_side_panel(self) -> bool {
		matches!(
			self.family(),
			BrowserFamily::Chromium | BrowserFamily::Firefox
		)
	}
}

impl fmt::Display for ExtensionBrowser {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(self.as_str())
	}
}

impl FromStr for ExtensionBrowser {
	type Err = ParseBrowserError;

	fn from_str(input: &str) -> Result<Self, Self::Err> {
		match input.trim().to_ascii_lowercase().as_str() {
			"chrome" | "google-chrome" => Ok(Self::Chrome),
			"edge" | "msedge" | "microsoft-edge" => Ok(Self::Edge),
			"brave" => Ok(Self::Brave),
			"opera" | "opr" => Ok(Self::Opera),
			"arc" => Ok(Self::Arc),
			"vivaldi" => Ok(Self::Vivaldi),
			"firefox" | "gecko" | "mozilla" => Ok(Self::Firefox),
			"safari" => Ok(Self::Safari),
			"chromium" => Ok(Self::ChromiumOther),
			"unknown" => Ok(Self::Unknown),
			_ => {
				Err(ParseBrowserError {
					input: input.to_owned(),
				})
			}
		}
	}
}

/// Optional compile-time browser hint from `LEPTOSKIT_EXTENSION_BROWSER`, when
/// an app builds a browser-specific variant.
#[must_use]
pub fn target_browser() -> Option<ExtensionBrowser> {
	option_env!("LEPTOSKIT_EXTENSION_BROWSER").and_then(|value| value.parse().ok())
}

/// Classifies the browser from runtime signals.
#[must_use]
pub fn detect_from_signals(signals: &BrowserSignals) -> BrowserInfo {
	let family = if let Some(url) = signals.extension_url.as_deref() {
		if url.starts_with("moz-extension:") {
			BrowserFamily::Firefox
		} else if url.starts_with("safari-web-extension:") {
			BrowserFamily::Safari
		} else if url.starts_with("chrome-extension:") {
			BrowserFamily::Chromium
		} else {
			BrowserFamily::Unknown
		}
	} else if let Some(name) = signals.browser_info_name.as_deref()
		&& name.to_ascii_lowercase().contains("firefox")
	{
		BrowserFamily::Firefox
	} else if signals.has_browser_namespace && !signals.has_chrome_namespace {
		BrowserFamily::Firefox
	} else if signals.has_chrome_namespace {
		BrowserFamily::Chromium
	} else {
		BrowserFamily::Unknown
	};

	let ua = signals.user_agent.as_deref().unwrap_or("");
	let brands_lower: Vec<String> = signals
		.ua_brands
		.iter()
		.map(|brand| brand.to_ascii_lowercase())
		.collect();

	let browser = match family {
		BrowserFamily::Firefox => ExtensionBrowser::Firefox,
		BrowserFamily::Safari => ExtensionBrowser::Safari,
		BrowserFamily::Chromium => {
			if signals.has_brave || brands_lower.iter().any(|brand| brand.contains("brave")) {
				ExtensionBrowser::Brave
			} else if ua.contains("Edg/")
				|| ua.contains("EdgA/")
				|| ua.contains("EdgiOS/")
				|| brands_lower.iter().any(|brand| brand.contains("edge"))
			{
				ExtensionBrowser::Edge
			} else if ua.contains("OPR/")
				|| brands_lower.iter().any(|brand| brand.contains("opera"))
			{
				ExtensionBrowser::Opera
			} else if ua.contains("Vivaldi/")
				|| brands_lower.iter().any(|brand| brand.contains("vivaldi"))
			{
				ExtensionBrowser::Vivaldi
			} else if ua.contains("Arc/") || brands_lower.iter().any(|brand| brand.contains("arc"))
			{
				ExtensionBrowser::Arc
			} else if brands_lower
				.iter()
				.any(|brand| brand.contains("google chrome"))
				|| (ua.contains("Chrome/") && !ua.contains("Chromium/"))
			{
				ExtensionBrowser::Chrome
			} else {
				ExtensionBrowser::ChromiumOther
			}
		}
		BrowserFamily::Unknown => {
			if ua.contains("Firefox/") {
				ExtensionBrowser::Firefox
			} else if ua.contains("Safari/") && !ua.contains("Chrome/") && !ua.contains("Chromium/")
			{
				ExtensionBrowser::Safari
			} else {
				ExtensionBrowser::Unknown
			}
		}
	};

	BrowserInfo {
		browser,
		family: browser.family(),
		name: signals.browser_info_name.clone(),
		version: signals.browser_info_version.clone(),
		vendor: signals.browser_info_vendor.clone(),
	}
}

/// Runs `body` with [`current`] temporarily returning `browser`, restoring the
/// previous override afterwards. Useful for testing Chrome, Edge, Firefox, and
/// Safari branches on any host.
pub fn with_browser<T>(browser: ExtensionBrowser, body: impl FnOnce() -> T) -> T {
	struct Guard(Option<ExtensionBrowser>);

	impl Drop for Guard {
		fn drop(&mut self) {
			BROWSER_OVERRIDE.with(|slot| slot.set(self.0));
		}
	}

	let previous = BROWSER_OVERRIDE.with(|slot| slot.replace(Some(browser)));
	let _guard = Guard(previous);
	body()
}

/// Returns the browser currently hosting this extension.
///
/// Resolution order:
/// 1. Scoped test/host override from [`with_browser`].
/// 2. Compile-time `LEPTOSKIT_EXTENSION_BROWSER` hint, when configured.
/// 3. Live runtime detection on `wasm32` (`runtime.getURL("")`, `navigator`).
/// 4. [`ExtensionBrowser::Unknown`] outside a browser.
#[must_use]
pub fn current() -> ExtensionBrowser {
	if let Some(overridden) = BROWSER_OVERRIDE.with(Cell::get) {
		return overridden;
	}

	if let Some(compiled) = target_browser() {
		return compiled;
	}

	detect_from_signals(&collect_sync_signals()).browser
}

/// Returns the browser family for [`current`].
#[must_use]
pub fn family() -> BrowserFamily {
	current().family()
}

/// Collects full [`BrowserInfo`], including async `runtime.getBrowserInfo()`
/// metadata when the browser exposes it (Firefox).
#[allow(
	clippy::unused_async,
	reason = "awaits runtime.getBrowserInfo on wasm32"
)]
pub async fn info() -> BrowserInfo {
	if let Some(overridden) = BROWSER_OVERRIDE.with(Cell::get) {
		return BrowserInfo {
			browser: overridden,
			family: overridden.family(),
			name: Some(overridden.as_str().to_owned()),
			version: None,
			vendor: None,
		};
	}

	#[cfg_attr(not(target_arch = "wasm32"), allow(unused_mut))]
	let mut signals = collect_sync_signals();

	#[cfg(target_arch = "wasm32")]
	if crate::js::has("runtime.getBrowserInfo") {
		#[derive(Deserialize)]
		struct RawBrowserInfo {
			#[serde(default)]
			name: Option<String>,
			#[serde(default)]
			version: Option<String>,
			#[serde(default)]
			vendor: Option<String>,
		}

		if let Ok(raw) = crate::js::call::<RawBrowserInfo>("runtime.getBrowserInfo", &[]).await {
			signals.browser_info_name = raw.name;
			signals.browser_info_version = raw.version;
			signals.browser_info_vendor = raw.vendor;
		}
	}

	let mut detected = detect_from_signals(&signals);
	if detected.browser == ExtensionBrowser::Unknown
		&& let Some(compiled) = target_browser()
	{
		detected.browser = compiled;
		detected.family = compiled.family();
	}

	detected
}

#[cfg(target_arch = "wasm32")]
fn collect_sync_signals() -> BrowserSignals {
	use js_sys::Array;
	use js_sys::Function;
	use js_sys::Reflect;
	use wasm_bindgen::JsCast;
	use wasm_bindgen::JsValue;

	let global = js_sys::global();
	let has_browser_namespace = Reflect::get(&global, &JsValue::from_str("browser"))
		.ok()
		.is_some_and(|value| {
			value.is_object()
				&& Reflect::get(&value, &JsValue::from_str("runtime"))
					.is_ok_and(|runtime| runtime.is_object())
		});
	let has_chrome_namespace = Reflect::get(&global, &JsValue::from_str("chrome"))
		.ok()
		.is_some_and(|value| {
			value.is_object()
				&& Reflect::get(&value, &JsValue::from_str("runtime"))
					.is_ok_and(|runtime| runtime.is_object())
		});

	let extension_url = crate::js::object("runtime").ok().and_then(|runtime| {
		let get_url = Reflect::get(&runtime, &JsValue::from_str("getURL"))
			.ok()?
			.dyn_into::<Function>()
			.ok()?;
		get_url
			.call1(&runtime, &JsValue::from_str(""))
			.ok()?
			.as_string()
	});

	let navigator = Reflect::get(&global, &JsValue::from_str("navigator")).ok();
	let user_agent = navigator.as_ref().and_then(|nav| {
		Reflect::get(nav, &JsValue::from_str("userAgent"))
			.ok()?
			.as_string()
	});
	let has_brave = navigator.as_ref().is_some_and(|nav| {
		Reflect::get(nav, &JsValue::from_str("brave"))
			.ok()
			.is_some_and(|value| value.is_object())
	});

	let mut ua_brands = Vec::new();
	if let Some(nav) = navigator.as_ref()
		&& let Ok(ua_data) = Reflect::get(nav, &JsValue::from_str("userAgentData"))
		&& ua_data.is_object()
		&& let Ok(brands) = Reflect::get(&ua_data, &JsValue::from_str("brands"))
		&& let Ok(array) = brands.dyn_into::<Array>()
	{
		for item in array.iter() {
			if let Some(brand) = Reflect::get(&item, &JsValue::from_str("brand"))
				.ok()
				.and_then(|value| value.as_string())
			{
				ua_brands.push(brand);
			}
		}
	}

	BrowserSignals {
		extension_url,
		browser_info_name: None,
		browser_info_version: None,
		browser_info_vendor: None,
		user_agent,
		ua_brands,
		has_brave,
		has_browser_namespace,
		has_chrome_namespace,
	}
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_sync_signals() -> BrowserSignals {
	BrowserSignals::default()
}

/// Chooses a value by exact [`ExtensionBrowser`], falling back to
/// [`BrowserFamily`] (`.chromium(...)`) and finally `.default(...)`.
#[derive(Debug, Clone)]
pub struct BrowserSwitch<T> {
	chrome: Option<T>,
	edge: Option<T>,
	brave: Option<T>,
	opera: Option<T>,
	arc: Option<T>,
	vivaldi: Option<T>,
	chromium: Option<T>,
	firefox: Option<T>,
	safari: Option<T>,
	default: Option<T>,
}

impl<T> Default for BrowserSwitch<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T> BrowserSwitch<T> {
	/// Starts an empty browser switch.
	#[must_use]
	pub fn new() -> Self {
		Self {
			chrome: None,
			edge: None,
			brave: None,
			opera: None,
			arc: None,
			vivaldi: None,
			chromium: None,
			firefox: None,
			safari: None,
			default: None,
		}
	}

	/// Value for Arc.
	#[must_use]
	pub fn arc(mut self, value: T) -> Self {
		self.arc = Some(value);
		self
	}

	/// Value for Brave.
	#[must_use]
	pub fn brave(mut self, value: T) -> Self {
		self.brave = Some(value);
		self
	}

	/// Value for Google Chrome specifically.
	#[must_use]
	pub fn chrome(mut self, value: T) -> Self {
		self.chrome = Some(value);
		self
	}

	/// Value for any Chromium browser when no specific Chromium branch matched.
	#[must_use]
	pub fn chromium(mut self, value: T) -> Self {
		self.chromium = Some(value);
		self
	}

	/// Fallback value when no browser or family branch matched.
	#[must_use]
	pub fn default(mut self, value: T) -> Self {
		self.default = Some(value);
		self
	}

	/// Value for Microsoft Edge.
	#[must_use]
	pub fn edge(mut self, value: T) -> Self {
		self.edge = Some(value);
		self
	}

	/// Value for Firefox.
	#[must_use]
	pub fn firefox(mut self, value: T) -> Self {
		self.firefox = Some(value);
		self
	}

	/// Value for Opera.
	#[must_use]
	pub fn opera(mut self, value: T) -> Self {
		self.opera = Some(value);
		self
	}

	/// Resolves against [`current`].
	///
	/// # Panics
	///
	/// Panics if no matching branch or `.default(...)` was provided.
	#[must_use]
	pub fn resolve(self) -> T {
		self.resolve_for(current())
	}

	/// Resolves against an explicit `browser`.
	///
	/// # Panics
	///
	/// Panics if no matching branch or `.default(...)` was provided.
	#[must_use]
	pub fn resolve_for(self, browser: ExtensionBrowser) -> T {
		self.try_resolve_for(browser)
			.expect("BrowserSwitch had no matching browser branch or default value")
	}

	/// Value for Safari.
	#[must_use]
	pub fn safari(mut self, value: T) -> Self {
		self.safari = Some(value);
		self
	}

	/// Resolves against [`current`], returning `None` if nothing matched.
	#[must_use]
	pub fn try_resolve(self) -> Option<T> {
		self.try_resolve_for(current())
	}

	/// Resolves against an explicit `browser`.
	#[must_use]
	pub fn try_resolve_for(mut self, browser: ExtensionBrowser) -> Option<T> {
		let exact = match browser {
			ExtensionBrowser::Chrome => self.chrome.take(),
			ExtensionBrowser::Edge => self.edge.take(),
			ExtensionBrowser::Brave => self.brave.take(),
			ExtensionBrowser::Opera => self.opera.take(),
			ExtensionBrowser::Arc => self.arc.take(),
			ExtensionBrowser::Vivaldi => self.vivaldi.take(),
			ExtensionBrowser::Firefox => self.firefox.take(),
			ExtensionBrowser::Safari => self.safari.take(),
			ExtensionBrowser::ChromiumOther | ExtensionBrowser::Unknown => None,
		};

		if exact.is_some() {
			return exact;
		}

		if browser.is_chromium() && self.chromium.is_some() {
			return self.chromium.take();
		}

		self.default.take()
	}

	/// Value for Vivaldi.
	#[must_use]
	pub fn vivaldi(mut self, value: T) -> Self {
		self.vivaldi = Some(value);
		self
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn detects_chrome_edge_brave_opera_arc_vivaldi_firefox_and_safari() {
		let chrome = detect_from_signals(&BrowserSignals {
			extension_url: Some("chrome-extension://abc/".into()),
			user_agent: Some("Mozilla/5.0 Chrome/137.0.0.0 Safari/537.36".into()),
			..Default::default()
		});
		assert_eq!(chrome.browser, ExtensionBrowser::Chrome);
		assert_eq!(chrome.family, BrowserFamily::Chromium);
		assert!(chrome.browser.supports_side_panel());
		assert!(chrome.browser.supports_offscreen());

		let edge = detect_from_signals(&BrowserSignals {
			extension_url: Some("chrome-extension://abc/".into()),
			user_agent: Some("Mozilla/5.0 Chrome/137.0.0.0 Safari/537.36 Edg/137.0.0.0".into()),
			..Default::default()
		});
		assert_eq!(edge.browser, ExtensionBrowser::Edge);
		assert!(edge.browser.is_chromium());

		let brave = detect_from_signals(&BrowserSignals {
			extension_url: Some("chrome-extension://abc/".into()),
			user_agent: Some("Mozilla/5.0 Chrome/137.0.0.0 Safari/537.36".into()),
			has_brave: true,
			..Default::default()
		});
		assert_eq!(brave.browser, ExtensionBrowser::Brave);

		let opera = detect_from_signals(&BrowserSignals {
			extension_url: Some("chrome-extension://abc/".into()),
			user_agent: Some("Mozilla/5.0 Chrome/137.0.0.0 Safari/537.36 OPR/120.0.0.0".into()),
			..Default::default()
		});
		assert_eq!(opera.browser, ExtensionBrowser::Opera);

		let arc = detect_from_signals(&BrowserSignals {
			extension_url: Some("chrome-extension://abc/".into()),
			ua_brands: vec!["Arc".into(), "Chromium".into()],
			..Default::default()
		});
		assert_eq!(arc.browser, ExtensionBrowser::Arc);

		let vivaldi = detect_from_signals(&BrowserSignals {
			extension_url: Some("chrome-extension://abc/".into()),
			user_agent: Some("Mozilla/5.0 Chrome/137.0.0.0 Vivaldi/7.0".into()),
			..Default::default()
		});
		assert_eq!(vivaldi.browser, ExtensionBrowser::Vivaldi);

		let firefox = detect_from_signals(&BrowserSignals {
			extension_url: Some("moz-extension://1234/".into()),
			browser_info_name: Some("Firefox".into()),
			browser_info_version: Some("138.0".into()),
			browser_info_vendor: Some("Mozilla".into()),
			..Default::default()
		});
		assert_eq!(firefox.browser, ExtensionBrowser::Firefox);
		assert_eq!(firefox.family, BrowserFamily::Firefox);
		assert_eq!(
			firefox.browser.manifest_sidebar_key(),
			Some("sidebar_action")
		);
		assert!(firefox.browser.supports_contextual_identities());
		assert!(!firefox.browser.supports_offscreen());

		let safari = detect_from_signals(&BrowserSignals {
			extension_url: Some("safari-web-extension://1234/".into()),
			..Default::default()
		});
		assert_eq!(safari.browser, ExtensionBrowser::Safari);
		assert_eq!(safari.family, BrowserFamily::Safari);
		assert!(!safari.browser.supports_side_panel());
		assert!(!safari.browser.supports_devtools_panel());
		assert_eq!(safari.browser.manifest_sidebar_key(), None);
	}

	#[test]
	fn browser_switch_prefers_specific_browser_over_family() {
		let choice = with_browser(ExtensionBrowser::Edge, || {
			BrowserSwitch::new()
				.chrome("chrome-only")
				.edge("edge-sidebar")
				.chromium("chromium-generic")
				.firefox("firefox-sidebar")
				.default("fallback")
				.resolve()
		});
		assert_eq!(choice, "edge-sidebar");

		let brave_choice = BrowserSwitch::new()
			.chrome("chrome-only")
			.chromium("chromium-generic")
			.default("fallback")
			.resolve_for(ExtensionBrowser::Brave);
		assert_eq!(brave_choice, "chromium-generic");

		let safari_choice = BrowserSwitch::new()
			.chromium("chromium-generic")
			.safari("safari-popup")
			.default("fallback")
			.resolve_for(ExtensionBrowser::Safari);
		assert_eq!(safari_choice, "safari-popup");
	}
}
