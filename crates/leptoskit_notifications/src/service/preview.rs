//! The body of [`preview_push`](crate::preview::preview_push): each
//! transport's own shaping, run for a sample message with no endpoint and
//! no credential.

use uuid::Uuid;

use super::transport;
use crate::options::BadgeSetting;
use crate::preview::FCM_TOKEN_PLACEHOLDER;
use crate::preview::PreviewHeader;
use crate::preview::PreviewSample;
use crate::preview::PreviewTrim;
use crate::preview::PushPreview;
use crate::push::DeliveryReason;
use crate::push::PushPlatform;

/// The steps of the degradation order, as a preview names them.
const TRIMS: [PreviewTrim; 3] = [
	PreviewTrim::PlatformAndImage,
	PreviewTrim::SoundBadgeLink,
	PreviewTrim::Data,
];

/// What each provider would be sent for `spec` as of `now`, read in
/// `locale`.
pub(crate) fn previews(
	spec: &crate::SendSpec,
	sample: &PreviewSample,
	locale: Option<&str>,
	now: leptoskit_db::Timestamp,
) -> Vec<PushPreview> {
	// The message a delivery would build: the text of the variant this
	// locale reads, and the badge resolved to a number.
	let text = spec.localized(locale.and_then(|locale| spec.variant_for(locale)));
	let message = transport::Message {
		id: sample.notification.unwrap_or(Uuid::nil()),
		title: text.title.to_owned(),
		body: text.body.to_owned(),
		data: spec.data.clone().unwrap_or_else(|| "{}".to_owned()),
		// The push lifetime counts from when the notification becomes
		// visible: the schedule the spec names, or now.
		expires_at: transport::push_expiry(
			spec.visible_at.unwrap_or(now).max(now),
			spec.expires_at,
			text.options.push_ttl,
		),
		badge: match text.options.badge {
			Some(BadgeSetting::Unread) => Some(sample.unread.unwrap_or(1)),
			Some(BadgeSetting::Set(set)) => Some(set),
			None => None,
		},
		options: text.options.into_owned(),
	};

	vec![
		preview(PushPlatform::Apns, transport::apns::shape(&message)),
		preview(
			PushPlatform::Fcm,
			transport::fcm::shape(FCM_TOKEN_PLACEHOLDER, &message, now),
		),
		if message.options.is_data() {
			// A silent data message is never pushed to a browser.
			PushPreview {
				platform: PushPlatform::WebPush,
				headers: Vec::new(),
				body: String::new(),
				bytes: 0,
				limit: 0,
				trimmed: Vec::new(),
				fits: true,
				skipped: Some(DeliveryReason::PlatformUnsupported),
			}
		} else {
			preview(
				PushPlatform::WebPush,
				transport::webpush::shape(&message, now),
			)
		},
	]
}

/// One transport's shaping, as the preview answers it.
fn preview(platform: PushPlatform, shaped: transport::Shaped) -> PushPreview {
	PushPreview {
		platform,
		headers: shaped
			.headers
			.into_iter()
			.map(|(name, value)| PreviewHeader { name, value })
			.collect(),
		// Every transport's body is JSON, which is UTF-8.
		body: String::from_utf8_lossy(&shaped.body).into_owned(),
		bytes: u32::try_from(shaped.bytes).unwrap_or(u32::MAX),
		limit: u32::try_from(shaped.limit).unwrap_or(u32::MAX),
		trimmed: TRIMS[..shaped.trims.min(TRIMS.len())].to_vec(),
		fits: shaped.fits,
		skipped: None,
	}
}
