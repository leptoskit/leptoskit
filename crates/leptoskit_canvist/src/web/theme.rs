use std::sync::Arc;
use canvist_layout::Color;
use canvist_layout::StarterKitOptions;
use canvist_layout::Theme;
use web_sys::HtmlElement;

pub struct ThemeReader;

impl ThemeReader {
	pub fn read(container: &HtmlElement) -> Theme {
		let computed = web_sys::window().and_then(|w| w.get_computed_style(container).ok().flatten());
		let scheme = if prefers_dark() { canvist_layout::ColorScheme::Dark } else { canvist_layout::ColorScheme::Light };
		// Actually ColorScheme lives in theme module; check path.
		let _ = scheme;
		let size = computed.as_ref()
			.map_or(16.0, |s| parse_px(&s.get_property_value("font-size").unwrap_or_default()));
		let size = if size <= 0.0 { 16.0 } else { size };
		let family = computed.as_ref()
			.map(|s| s.get_property_value("font-family").unwrap_or_default())
			.unwrap_or_default();
		let family = if family.trim().is_empty() { "sans-serif".to_owned() } else { family };
		let line_height_px = computed.as_ref()
			.map_or(0.0, |s| parse_px(&s.get_property_value("line-height").unwrap_or_default()));
		let line_height = if line_height_px > 0.0 && size > 0.0 { line_height_px / size } else { 1.5 };
		let letter_spacing = computed.as_ref()
			.map_or(0.0, |s| parse_px(&s.get_property_value("letter-spacing").unwrap_or_default()));
		let code_family = "monospace".to_owned();
		let opts = StarterKitOptions {
			scheme: if prefers_dark() {
				canvist_layout::ColorScheme::Dark
			} else {
				canvist_layout::ColorScheme::Light
			},
			body_family: Arc::from(family.as_str()),
			code_family: Arc::from(code_family.as_str()),
			size,
			line_height,
			letter_spacing,
		};
		let theme = Theme::starter_kit_with(&opts);
		let mut chrome = *theme.chrome();
		if let Some(s) = &computed {
			let caret = s.get_property_value("caret-color").unwrap_or_default();
			if !caret.is_empty() && caret != "auto"
				&& let Some(c) = parse_color(&caret) {
					chrome.caret = c;
				}
			let bg = s.get_property_value("background-color").unwrap_or_default();
			if let Some(c) = parse_color(&bg) {
				chrome.background = c;
			}
		}
		theme.to_builder().chrome(chrome).build()
	}

	pub fn apply(state: crate::state::CanvistState) {
		// Re-read theme and update host.
		let container = with_container(state);
		if let Some(c) = container {
			let t = Self::read(&c);
			let _ = crate::web::with_host(state, |h| {
				h.theme = Arc::new(t);
				h.full_repaint = true;
			});
		}
	}
}

fn with_container(state: crate::state::CanvistState) -> Option<HtmlElement> {
	crate::web::with_host(state, |h| h.container.clone())
}

fn prefers_dark() -> bool {
	web_sys::window()
		.and_then(|w| w.match_media("(prefers-color-scheme: dark)").ok().flatten())
		.is_some_and(|m| m.matches())
}

fn parse_px(v: &str) -> f32 {
	v.trim_end_matches("px").trim().parse().unwrap_or(0.0)
}

fn parse_color(v: &str) -> Option<Color> {
	let v = v.trim();
	if v.starts_with('#') {
		let hex = v.trim_start_matches('#');
		if hex.len() == 6 {
			let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
			let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
			let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
			return Some(Color { r, g, b, a: 255 });
		}
	}
	if v.starts_with("rgb") {
		let nums: Vec<u8> = v
			.trim_start_matches("rgba")
			.trim_start_matches("rgb")
			.trim_matches(|c: char| c == '(' || c == ')' || c.is_whitespace())
			.split(',')
			.filter_map(|p| p.trim().trim_end_matches('%').parse::<f32>().ok().map(|n| n as u8))
			.collect();
		if nums.len() >= 3 {
			let a = if nums.len() >= 4 { nums[3] } else { 255 };
			return Some(Color { r: nums[0], g: nums[1], b: nums[2], a });
		}
	}
	None
}

// Need ColorScheme path. Check canvist_layout re-exports ColorScheme.
// It's in theme module. Access via canvist_layout::ColorScheme if re-exported.

