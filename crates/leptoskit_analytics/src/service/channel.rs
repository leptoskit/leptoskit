//! A visit's channel, from its referrer and its UTM parameters (§8).
//!
//! A pure function, in PostHog's spirit: its named channels, its habit of
//! deciding from `utm_medium` first and the referrer second, and its
//! distinction between nothing at all (`Direct`) and values it doesn't
//! recognise (`Unknown`). Written as a function of already-parsed strings
//! so tests can pin every rule; the batch writer feeds it the session's
//! entry pageview.

/// The channel a visit arrived through, classified in PostHog's order:
/// what was paid wins over what was organic, medium rules over source
/// rules, and the referrer decides only when no parameter did.
#[must_use]
pub fn classify(
	utm_source: &str,
	utm_medium: &str,
	referrer_domain: &str,
	own_domain: &str,
) -> &'static str {
	let source = utm_source.to_ascii_lowercase();
	let medium = utm_medium.to_ascii_lowercase();
	let referrer = referrer_domain
		.trim_start_matches("www.")
		.to_ascii_lowercase();
	let own = own_domain.trim_start_matches("www.").to_ascii_lowercase();

	// A social source is social whether it was bought or not: Facebook ads
	// are Paid Social even when the medium reads like search advertising.
	if source_social(&source) {
		if paid(&medium) || SOCIAL_MEDIA.contains(&medium.as_str()) {
			return "Paid Social";
		}
		return "Organic Social";
	}
	if SOCIAL_MEDIA.contains(&medium.as_str()) {
		return "Paid Social";
	}
	if PAID_SEARCH_MEDIA.contains(&medium.as_str()) {
		return "Paid Search";
	}
	if SOCIAL_MEDIA_UNPAID.contains(&medium.as_str()) {
		return "Organic Social";
	}
	if SEARCH_MEDIA.contains(&medium.as_str())
		|| SEARCH_ENGINES.contains(&source.as_str())
		|| SEARCH_DOMAINS
			.iter()
			.any(|domain| referrer == *domain || referrer.ends_with(&format!(".{domain}")))
	{
		return "Organic Search";
	}
	if medium == "affiliate" {
		return "Affiliate";
	}
	if medium == "audio" {
		return "Audio";
	}
	if email(&medium) {
		return "Email";
	}
	if medium == "mobile push" || medium == "push" || medium == "notification" {
		return "Mobile Push";
	}
	if medium == "sms" || medium == "text" {
		return "SMS";
	}
	if medium == "video" {
		return "Video";
	}
	if !referrer.is_empty() && referrer != own {
		return "Referral";
	}
	if source.is_empty() && medium.is_empty() && referrer.is_empty() {
		return "Direct";
	}
	// A visit from the site's own domain, or parameters nobody classifies.
	if source.is_empty() && medium.is_empty() {
		return "Direct";
	}
	"Unknown"
}

/// Whether the medium says the visit was bought: paid, or a cost model.
fn paid(medium: &str) -> bool {
	medium.contains("paid") || medium.contains("cpc") || medium.contains("ppc")
}

/// Whether the medium names email, as `email`, `e-mail`, or their
/// hyphenated kinds do.
fn email(medium: &str) -> bool {
	medium == "email" || medium.starts_with("email") || medium.starts_with("e-mail")
}

/// Whether the source is a social network, as `social` and the networks'
/// names are.
fn source_social(source: &str) -> bool {
	source == "social"
		|| SOCIAL_NETWORKS.contains(&source)
		|| source.ends_with("-social")
		|| source.ends_with("_social")
}

/// Media PostHog's rules read as bought search traffic.
const PAID_SEARCH_MEDIA: &[&str] = &[
	"adwords",
	"adwords-remarketing",
	"adwords_search",
	"cpc",
	"googleads",
	"google-ads",
	"paidsearch",
	"paid-search",
	"paid_search",
	"ppc",
	"sem",
];

/// Media PostHog's rules read as bought social traffic, beyond any medium
/// that combines `paid` with a social source.
const SOCIAL_MEDIA: &[&str] = &[
	"paid-social",
	"paid_social",
	"paid social",
	"social-paid",
	"social_paid",
	"socialpaid",
	"psc",
	"ppc-social",
];

/// Media PostHog's rules read as search traffic.
const SEARCH_MEDIA: &[&str] = &[
	"google",
	"organic",
	"seo",
	"search",
	"organic_search",
	"organic-search",
];

/// Media PostHog's rules read as unpaid social traffic, without a social
/// source saying so.
const SOCIAL_MEDIA_UNPAID: &[&str] = &["social", "social-media", "social_media"];

/// Search engines a referrer names.
const SEARCH_ENGINES: &[&str] = &[
	"google",
	"bing",
	"duckduckgo",
	"ecosia",
	"baidu",
	"yandex",
	"yahoo",
];

/// Social networks a `utm_source` or referrer names.
const SOCIAL_NETWORKS: &[&str] = &[
	"facebook",
	"fb",
	"instagram",
	"linkedin",
	"twitter",
	"x",
	"tiktok",
	"pinterest",
	"reddit",
	"snapchat",
	"threads",
	"mastodon",
	"bluesky",
	"whatsapp",
	"telegram",
	"youtube",
];

/// Search engines' domains, as a referrer names them.
const SEARCH_DOMAINS: &[&str] = &[
	"google.com",
	"bing.com",
	"duckduckgo.com",
	"ecosia.org",
	"baidu.com",
	"yandex.com",
	"yahoo.com",
];

#[cfg(test)]
mod tests {
	use super::classify;

	#[test]
	fn bought_media_win_over_organic_sources() {
		assert_eq!(
			classify("google", "cpc", "facebook.com", "app.example"),
			"Paid Search"
		);
		assert_eq!(classify("google", "ppc", "", "app.example"), "Paid Search");
	}

	#[test]
	fn a_social_source_is_paid_or_organic_by_its_medium() {
		assert_eq!(
			classify("facebook", "paid-social", "", "app.example"),
			"Paid Social"
		);
		assert_eq!(
			classify("instagram", "cpc", "", "app.example"),
			"Paid Social"
		);
		assert_eq!(
			classify("instagram", "social", "", "app.example"),
			"Organic Social"
		);
		assert_eq!(
			classify("social", "", "twitter.com", "app.example"),
			"Organic Social"
		);
	}

	#[test]
	fn search_engines_and_search_media_are_organic_search() {
		assert_eq!(classify("google", "", "", "app.example"), "Organic Search");
		assert_eq!(classify("", "organic", "", "app.example"), "Organic Search");
		assert_eq!(
			classify("", "", "www.bing.com", "app.example"),
			"Organic Search"
		);
	}

	#[test]
	fn email_sms_and_the_small_channels_recognise_their_media() {
		assert_eq!(classify("newsletter", "email", "", "app.example"), "Email");
		assert_eq!(classify("", "email-notify", "", "app.example"), "Email");
		assert_eq!(classify("", "sms", "", "app.example"), "SMS");
		assert_eq!(classify("", "affiliate", "", "app.example"), "Affiliate");
		assert_eq!(classify("", "audio", "", "app.example"), "Audio");
		assert_eq!(classify("", "video", "", "app.example"), "Video");
		assert_eq!(
			classify("", "mobile push", "", "app.example"),
			"Mobile Push"
		);
	}

	#[test]
	fn a_referrer_that_is_not_the_site_is_a_referral() {
		assert_eq!(
			classify("", "", "news.ycombinator.com", "app.example"),
			"Referral"
		);
		assert_eq!(
			classify("", "", "blog.example.org", "blog.example.org"),
			"Direct",
			"the site's own domain isn't a referral"
		);
		assert_eq!(
			classify("", "", "www.news.ycombinator.com", "app.example"),
			"Referral",
			"www. doesn't hide a referral"
		);
	}

	#[test]
	fn nothing_at_all_is_direct_and_unrecognised_parameters_are_unknown() {
		assert_eq!(classify("", "", "", ""), "Direct");
		assert_eq!(
			classify("", "", "app.example", "app.example"),
			"Direct",
			"arriving from the site itself is direct"
		);
		assert_eq!(classify("", "mystery", "", ""), "Unknown");
		assert_eq!(classify("sidebar", "referral-widget", "", ""), "Unknown");
	}

	#[test]
	fn matching_is_case_insensitive() {
		assert_eq!(classify("Google", "PPC", "", "app.example"), "Paid Search");
		assert_eq!(
			classify("Facebook", "Social", "", "app.example"),
			"Organic Social"
		);
	}
}
