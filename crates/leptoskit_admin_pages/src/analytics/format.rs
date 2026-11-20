//! How analytics numbers, times, and names read on screen.

use std::fmt::Write as _;

use chrono::DateTime;
use chrono::Datelike as _;
use chrono::NaiveDateTime;
use chrono::Timelike as _;
use chrono::Utc;

const MONTHS: [&str; 12] = [
	"Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `1,234`.
pub fn thousands(value: u64) -> String {
	let digits = value.to_string();
	let mut out = String::with_capacity(digits.len() + digits.len() / 3);

	for (index, digit) in digits.chars().enumerate() {
		if index > 0 && (digits.len() - index).is_multiple_of(3) {
			out.push(',');
		}

		out.push(digit);
	}

	out
}

/// A count as the hero numbers show it: `982`, `8,412`, `48.2k`.
#[expect(
	clippy::cast_possible_truncation,
	clippy::cast_sign_loss,
	reason = "counts are whole and non-negative"
)]
pub fn count(value: f64) -> String {
	let whole = value.max(0.0).round() as u64;

	if whole < 10_000 {
		thousands(whole)
	} else {
		crate::charts::count(whole)
	}
}

/// A number that may have a fraction, for a table: `12`, `3.5`.
pub fn number(value: f64) -> String {
	if (value - value.round()).abs() < 1e-9 {
		count(value)
	} else {
		format!("{value:.1}")
	}
}

/// `41.3%` from a share between 0 and 1.
pub fn share(value: f64) -> String {
	format!("{:.1}%", value * 100.0)
}

/// `6m 21s`, `2h 04m`, `45s`.
#[expect(
	clippy::cast_possible_truncation,
	clippy::cast_sign_loss,
	reason = "a duration for display"
)]
pub fn duration(seconds: f64) -> String {
	let seconds = seconds.max(0.0).round() as u64;

	match seconds {
		0..60 => format!("{seconds}s"),
		60..3600 => format!("{}m {:02}s", seconds / 60, seconds % 60),
		3600..86_400 => format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60),
		_ => format!("{}d {}h", seconds / 86_400, (seconds % 86_400) / 3600),
	}
}

/// A relative change as the stat tiles say it: `▲ 12.4%`, `▼ 3.0%`, `= 0%`.
pub fn delta(delta: Option<f64>) -> Option<String> {
	let delta = delta?;
	Some(if delta > 0.0005 {
		format!("▲ {:.1}%", delta * 100.0)
	} else if delta < -0.0005 {
		format!("▼ {:.1}%", -delta * 100.0)
	} else {
		"= no change".to_owned()
	})
}

/// A bucket's wall-clock start, `YYYY-MM-DDTHH:MM:SS`.
pub fn wall_clock(text: &str) -> Option<NaiveDateTime> {
	NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").ok()
}

/// An RFC 3339 instant.
pub fn instant(text: &str) -> Option<DateTime<Utc>> {
	DateTime::parse_from_rfc3339(text)
		.ok()
		.map(|time| time.with_timezone(&Utc))
}

/// `12 Nov`, or `12 Nov 14:00` for an hourly bucket.
pub fn bucket_label(text: &str, hourly: bool) -> String {
	let Some(time) = wall_clock(text) else {
		return text.to_owned();
	};
	let day = format!("{} {}", time.day(), MONTHS[time.month0() as usize]);

	if hourly {
		format!("{day} {:02}:00", time.hour())
	} else {
		day
	}
}

/// `12 Nov 2026`.
pub fn date(time: &DateTime<Utc>) -> String {
	format!(
		"{} {} {}",
		time.day(),
		MONTHS[time.month0() as usize],
		time.year()
	)
}

/// `14:05`.
pub fn clock(time: &DateTime<Utc>) -> String {
	format!("{:02}:{:02}", time.hour(), time.minute())
}

/// `3s ago`, `5 min ago`, `2 h ago`, `4 days ago`.
pub fn ago(time: &DateTime<Utc>, now: &DateTime<Utc>) -> String {
	let seconds = (*now - *time).num_seconds().max(0);

	match seconds {
		0..5 => "now".to_owned(),
		5..60 => format!("{seconds}s ago"),
		60..3600 => format!("{} min ago", seconds / 60),
		3600..86_400 => format!("{} h ago", seconds / 3600),
		86_400..172_800 => "yesterday".to_owned(),
		_ => format!("{} days ago", seconds / 86_400),
	}
}

/// A JSON value as a table cell shows it.
pub fn value(value: &serde_json::Value) -> String {
	match value {
		serde_json::Value::Null => "—".to_owned(),
		serde_json::Value::String(text) => text.clone(),
		other => other.to_string(),
	}
}

/// The struct a typed event's name comes from: `checkout_started` →
/// `CheckoutStarted`.
pub fn struct_name(event: &str) -> String {
	event
		.split(|character: char| !character.is_ascii_alphanumeric())
		.filter(|part| !part.is_empty())
		.map(|part| {
			let mut characters = part.chars();
			characters.next().map_or_else(String::new, |first| {
				first.to_ascii_uppercase().to_string() + characters.as_str()
			})
		})
		.collect()
}

/// `snake_case` of a struct name: what `#[derive(Event)]` names an event
/// without `name = …`.
pub fn snake_case(name: &str) -> String {
	let mut out = String::new();

	for (index, character) in name.chars().enumerate() {
		if character.is_ascii_uppercase() {
			if index > 0 {
				out.push('_');
			}

			out.push(character.to_ascii_lowercase());
		} else {
			out.push(character);
		}
	}

	out
}

/// Percent-encodes a URL query component.
pub fn encode(text: &str) -> String {
	let mut out = String::with_capacity(text.len());

	for byte in text.bytes() {
		if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
			out.push(char::from(byte));
		} else {
			let _ = write!(out, "%{byte:02X}");
		}
	}

	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn numbers_read_plainly() {
		assert_eq!(thousands(8412), "8,412");
		assert_eq!(thousands(982), "982");
		assert_eq!(count(48_200.0), "48.2k");
		assert_eq!(share(0.117), "11.7%");
		assert_eq!(duration(381.0), "6m 21s");
		assert_eq!(delta(Some(0.124)).as_deref(), Some("▲ 12.4%"));
		assert_eq!(delta(Some(-0.03)).as_deref(), Some("▼ 3.0%"));
		assert_eq!(bucket_label("2026-11-12T00:00:00", false), "12 Nov");
		assert_eq!(struct_name("checkout_started"), "CheckoutStarted");
		assert_eq!(snake_case("CheckoutStarted"), "checkout_started");
		assert_eq!(encode("a b/$c"), "a%20b%2F%24c");
	}
}
