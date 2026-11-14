//! User-agent parsing at ingest, into the context properties sessions and
//! web analytics break down by.
//!
//! A browser sends `$raw_user_agent` in its context; the service parses it
//! into `$browser`, `$browser_version`, `$os`, `$os_version`, and
//! `$device_type` (§14). It happens here rather than in the client because
//! §16 keeps the client small: a parser is a dependency an app's bundle
//! pays for, and every browser in a session sends the same string, so
//! parsing once per context costs the service little.
//!
//! This is written as plain matching rather than with a parser crate, and
//! deliberately coarse: it names the browsers and systems people use for
//! product analytics, and leaves what it doesn't recognise out rather than
//! guessing. It never panics and never allocates more than the version
//! strings it finds.

/// What a user agent parsed into. Each part is absent when the string
/// didn't say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parts {
	/// The browser's name, such as `Chrome`, and its version.
	pub browser: Option<(&'static str, String)>,
	/// The operating system's name, such as `iOS`, and its version.
	pub os: Option<(&'static str, String)>,
	/// `Desktop`, `Mobile`, or `Tablet`.
	pub device_type: Option<&'static str>,
}

/// Parses a `User-Agent` string. An empty one parses to nothing.
#[must_use]
pub fn parse(user_agent: &str) -> Parts {
	if user_agent.is_empty() {
		return Parts::default();
	}
	Parts {
		browser: browser(user_agent),
		os: os(user_agent),
		device_type: Some(device_type(user_agent)),
	}
}

/// The browser: the engines' tokens, most specific first, so a browser
/// that mentions another's engine (as Chrome's iOS build mentions Safari's)
/// keeps its own name.
fn browser(ua: &str) -> Option<(&'static str, String)> {
	for (token, name) in [
		("EdgiOS/", "Mobile Edge"),
		("EdgA/", "Android Edge"),
		("Edg/", "Microsoft Edge"),
		("Edge/", "Microsoft Edge"),
		("OPR/", "Opera"),
		("Opera ", "Opera"),
		("SamsungBrowser/", "Samsung Internet"),
		("DuckDuckGo/", "DuckDuckGo"),
		("Firefox/", "Firefox"),
		("FxiOS/", "Firefox"),
		("CriOS/", "Chrome"),
		("Chrome/", "Chrome"),
		("Chromium/", "Chromium"),
	] {
		if let Some(version) = after(ua, token) {
			return Some((name, version));
		}
	}
	// Safari never names itself with a version; `Version/` is its token,
	// and `Safari/` alone is a library compat string.
	if after(ua, "Version/").is_some() && ua.contains("Safari") {
		return Some(("Safari", after(ua, "Version/")?));
	}
	None
}

/// The operating system, from the tokens the platforms' own strings carry.
fn os(ua: &str) -> Option<(&'static str, String)> {
	if let Some(rest) = between(ua, "Windows NT ", &[';', ')']) {
		let version = match rest.get(..3) {
			Some("5.1" | "5.2") => "XP".to_owned(),
			Some("6.0") => "Vista".to_owned(),
			Some("6.1") => "7".to_owned(),
			Some("6.2") => "8".to_owned(),
			Some("6.3") => "8.1".to_owned(),
			Some("10.") => "10 or later".to_owned(),
			_ => rest.trim().to_owned(),
		};
		return Some(("Windows", version));
	}
	if ua.contains("Windows Phone") {
		return Some(("Windows Phone", version_after(ua, "Windows Phone ")));
	}
	if ua.contains("iPad") || ua.contains("iPhone") || ua.contains("iPod") {
		// `OS 17_4` in `like Mac OS X` is the iOS version.
		let version = between(ua, " OS ", &['_'])
			.or_else(|| between(ua, " OS ", &[' ']))
			.map(str::to_owned);
		return Some(("iOS", version.unwrap_or_default()));
	}
	if let Some(version) = after(ua, "Android/") {
		return Some(("Android", version));
	}
	if ua.contains("Android") {
		return Some(("Android", version_after(ua, "Android ")));
	}
	if ua.contains("CrOS") {
		return Some(("Chrome OS", version_after(ua, "CrOS ")));
	}
	if let Some(version) = between(ua, "Mac OS X ", &[';', ')']) {
		return Some(("Mac OS X", version.replace('_', ".")));
	}
	if ua.contains("Macintosh") {
		return Some(("Mac OS X", String::new()));
	}
	if ua.contains("Linux") || ua.contains("X11") {
		return Some(("Linux", String::new()));
	}
	None
}

/// `Tablet` for iPads and Android tablets (an Android without `Mobile`),
/// `Mobile` for phones, `Desktop` for everything else.
fn device_type(ua: &str) -> &'static str {
	if ua.contains("iPad") || (ua.contains("Android") && !ua.contains("Mobile")) {
		return "Tablet";
	}
	if ua.contains("Mobi") || ua.contains("iPhone") || ua.contains("iPod") {
		return "Mobile";
	}
	"Desktop"
}

/// The digits and dots after `token`, when `ua` names it.
fn after<'a>(ua: &'a str, token: &str) -> Option<String> {
	let rest = ua.split_once(token)?.1;
	let end = rest
		.find(|c: char| !c.is_ascii_digit() && c != '.')
		.unwrap_or(rest.len());
	let version = &rest[..end];
	(!version.is_empty()).then(|| version.to_owned())
}

/// The characters after `token` up to the first of `until`, when `ua`
/// names both.
fn between<'a>(ua: &'a str, token: &str, until: &[char]) -> Option<&'a str> {
	let rest = ua.split_once(token)?.1;
	let end = rest.find(until).unwrap_or(rest.len());
	Some(rest[..end].trim_end_matches([';', ' ']))
}

/// Everything up to the next space or `;`, as a free-form version does.
fn version_after(ua: &str, token: &str) -> String {
	ua.split_once(token)
		.map(|(_, rest)| {
			let end = rest.find([';', ' ', ')']).unwrap_or(rest.len());
			rest[..end].to_owned()
		})
		.unwrap_or_default()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn parts(ua: &str) -> Parts {
		parse(ua)
	}

	#[test]
	fn a_chrome_on_macos_parses_all_three_parts() {
		let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like \
		          Gecko) Chrome/131.0.0.0 Safari/537.36";
		assert_eq!(
			parts(ua),
			Parts {
				browser: Some(("Chrome", "131.0.0.0".into())),
				os: Some(("Mac OS X", "10.15.7".into())),
				device_type: Some("Desktop"),
			}
		);
	}

	#[test]
	fn an_iphone_safari_is_mobile_ios() {
		let ua = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 \
		          (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1";
		assert_eq!(
			parts(ua),
			Parts {
				browser: Some(("Safari", "18.1".into())),
				os: Some(("iOS", "18".into())),
				device_type: Some("Mobile"),
			}
		);
	}

	#[test]
	fn an_ipad_is_a_tablet_on_ios() {
		let ua = "Mozilla/5.0 (iPad; CPU OS 17_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like \
		          Gecko) Version/17.6 Mobile/15E148 Safari/604.1";
		assert_eq!(parts(ua).device_type, Some("Tablet"));
		assert_eq!(parts(ua).os.as_ref().map(|(name, _)| *name), Some("iOS"));
	}

	#[test]
	fn a_chrome_on_android_is_mobile_and_names_androids_version() {
		let ua = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) \
		          Chrome/131.0.0.0 Mobile Safari/537.36";
		let parsed = parts(ua);
		assert_eq!(
			parsed.browser.as_ref().map(|(name, _)| *name),
			Some("Chrome")
		);
		assert_eq!(parsed.os, Some(("Android", "14".to_owned())));
		assert_eq!(parsed.device_type, Some("Mobile"));
	}

	#[test]
	fn an_android_tablet_without_the_mobile_token_is_a_tablet() {
		let ua = "Mozilla/5.0 (Linux; Android 13; SM-X710) AppleWebKit/537.36 (KHTML, like Gecko) \
		          Chrome/131.0.0.0 Safari/537.36";
		assert_eq!(parts(ua).device_type, Some("Tablet"));
	}

	#[test]
	fn edge_on_windows_is_not_chrome_despite_the_engine_token() {
		let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like \
		          Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.2903.86";
		assert_eq!(
			parts(ua).browser,
			Some(("Microsoft Edge", "131.0.2903.86".into()))
		);
		assert_eq!(parts(ua).os, Some(("Windows", "10 or later".to_owned())));
		assert_eq!(parts(ua).device_type, Some("Desktop"));
	}

	#[test]
	fn chromes_ios_build_is_chrome_not_safari() {
		let ua = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 \
		          (KHTML, like Gecko) CriOS/131.0.0.0 Mobile/15E148 Safari/604.1";
		assert_eq!(parts(ua).browser, Some(("Chrome", "131.0.0.0".into())));
		assert_eq!(parts(ua).device_type, Some("Mobile"));
	}

	#[test]
	fn firefox_on_linux_names_linux_without_a_version() {
		let ua = "Mozilla/5.0 (X11; Linux x86_64; rv:133.0) Gecko/20100101 Firefox/133.0";
		assert_eq!(parts(ua).browser, Some(("Firefox", "133.0".into())));
		assert_eq!(parts(ua).os, Some(("Linux", String::new())));
	}

	#[test]
	fn samsung_internet_is_its_own_browser() {
		let ua = "Mozilla/5.0 (Linux; Android 14; SM-S928B) AppleWebKit/537.36 (KHTML, like \
		          Gecko) SamsungBrowser/27.0 Chrome/125.0.0.0 Mobile Safari/537.36";
		assert_eq!(parts(ua).browser, Some(("Samsung Internet", "27.0".into())));
	}

	#[test]
	fn windows_7_keeps_its_name_and_a_bot_parses_to_a_desktop_with_no_browser() {
		let ua = "Mozilla/5.0 (Windows NT 6.1) AppleWebKit/537.36";
		assert_eq!(parts(ua).os, Some(("Windows", "7".to_owned())));
		let bot = "Mozilla/5.0 (compatible; SemrushBot/7~bl)";
		let parsed = parts(bot);
		assert_eq!(parsed.browser, None);
		assert_eq!(parsed.os, None);
		assert_eq!(parsed.device_type, Some("Desktop"));
	}

	#[test]
	fn an_empty_string_parses_to_nothing() {
		assert_eq!(parts(""), Parts::default());
	}

	#[test]
	fn a_version_never_runs_away_from_its_token() {
		// No digits after the token: no version, but the browser is known.
		let ua = "Mozilla/5.0 Chrome/ (KHTML, like Gecko)";
		assert_eq!(parts(ua).browser, None);
	}
}
