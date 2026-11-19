//! Push previews: the exact request each provider would be sent for a
//! send, without sending it.
//!
//! A notification that is too large is not refused when it is sent. Each
//! provider has a limit near four kilobytes, and a push over it is
//! trimmed in a fixed order until it fits — the per-platform options and
//! the image first, then the sound, badge, and link, then the send's
//! `data` — so the first anyone learns of it is a notification arriving
//! without its image. [`preview_push`] shows what would be sent before it
//! is: the headers and the body for APNs, FCM, and Web Push, how many
//! bytes each provider counts against its limit, and what was trimmed.
//!
//! The preview is built by the same code that builds the real request, so
//! the body it answers is byte for byte what a provider is sent, with
//! three things standing in for what only a real delivery knows: the
//! notification's id ([`PreviewSample::notification`]), the FCM device
//! token (always the placeholder [`FCM_TOKEN_PLACEHOLDER`]), and the
//! number an unread-count badge resolves to ([`PreviewSample::unread`]).
//! No credential is read or answered: the authorization header, and the
//! APNs topic that names the app, are left out.

use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::SendSpec;
use crate::push::DeliveryReason;
use crate::push::PushPlatform;

/// What stands where a device's FCM registration token goes in a
/// preview's FCM body.
pub const FCM_TOKEN_PLACEHOLDER: &str = "<device token>";

/// What stands in a preview for what only a real delivery knows. Every
/// field has a default, so a preview needs none of them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PreviewSample {
	/// The notification's id, which rides in every payload and as the
	/// collapse id. The nil id when left out.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub notification: Option<uuid::Uuid>,
	/// The number a badge set to the recipient's unread count shows. 1
	/// when left out.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub unread: Option<u32>,
	/// The locale to read the send's variants in, as a BCP 47 tag: the
	/// preview shows the text a person with this locale would get. The
	/// send's own text when left out, or when no variant matches.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub locale: Option<String>,
}

impl PreviewSample {
	/// A sample for the notification `id`.
	#[must_use]
	pub fn of(id: uuid::Uuid) -> Self {
		Self {
			notification: Some(id),
			..Self::default()
		}
	}

	/// The same sample, with an unread-count badge showing `unread`.
	#[must_use]
	pub fn unread(mut self, unread: u32) -> Self {
		self.unread = Some(unread);
		self
	}

	/// The same sample, read in `locale`.
	#[must_use]
	pub fn locale(mut self, locale: impl Into<String>) -> Self {
		self.locale = Some(locale.into());
		self
	}
}

/// One step of the order a push too large for its provider is trimmed
/// in. Each step is taken only when the steps before it were not enough.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PreviewTrim {
	/// The per-platform options and the image were dropped.
	PlatformAndImage,
	/// The sound, the badge, and the link were dropped too.
	SoundBadgeLink,
	/// The send's `data` was dropped as well: only the text and the
	/// notification's id are left.
	Data,
}

/// One header of a previewed request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PreviewHeader {
	/// The header's name, lower case.
	pub name: String,
	/// Its value.
	pub value: String,
}

/// What one provider would be sent for a send.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PushPreview {
	/// Which provider.
	pub platform: PushPlatform,
	/// The request's headers, without its authorization and without the
	/// APNs topic.
	pub headers: Vec<PreviewHeader>,
	/// The request's body: JSON, exactly as it is posted. For Web Push,
	/// the envelope before it is encrypted for one browser — the
	/// encrypted bytes differ for every push.
	pub body: String,
	/// How many bytes the provider counts against its limit: the body for
	/// APNs, the encrypted body for Web Push, and for FCM the keys and
	/// values of the payload a device receives.
	pub bytes: u32,
	/// The provider's limit.
	pub limit: u32,
	/// What was trimmed to make it fit, in the order it was trimmed.
	/// Empty when the whole push fits.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub trimmed: Vec<PreviewTrim>,
	/// Whether it fits after trimming. When `false`, nothing would be
	/// sent on this platform: the delivery would end `oversized`.
	pub fits: bool,
	/// Why this platform is never sent this kind of send, when it isn't:
	/// a silent data message is not pushed to browsers. The headers and
	/// body are then empty.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub skipped: Option<DeliveryReason>,
}

/// What each provider — APNs, FCM, and Web Push, in that order — would be
/// sent for `spec`: the headers, the body, the bytes the provider counts
/// against its limit, and what was trimmed to fit. Nothing is sent, read,
/// or written, and the server needs no push configuration.
///
/// `sample` stands in for what only a real delivery knows; see
/// [`PreviewSample`]. A spec with an expiry carries the time left until
/// it — FCM's `ttl`, Web Push's `ttl` header — measured from the moment of
/// the preview, as a real push measures it from the moment it is sent.
///
/// Only the app may call it: [`rpc::client::call_as_app`] from server code.
///
/// # Errors
///
/// [`rpc::Error::Forbidden`] from anywhere but the app's server, and
/// [`rpc::Error::Invalid`] for a spec [`check_spec`](crate::check_spec)
/// refuses or a sample locale that isn't a BCP 47 tag.
#[rpc::query(service = "notifications")]
pub async fn preview_push(
	cx: &rpc::QueryCx,
	spec: SendSpec,
	#[serde(default)] sample: PreviewSample,
) -> rpc::Result<Vec<PushPreview>> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app previews a push: call this from its server, with the service's token",
		));
	}

	crate::check_spec(&spec).map_err(rpc::Error::invalid)?;

	let locale = sample
		.locale
		.as_deref()
		.map(crate::push::canonical_locale)
		.transpose()
		.map_err(rpc::Error::invalid)?;

	Ok(crate::service::preview::previews(
		&spec,
		&sample,
		locale.as_deref(),
		leptoskit_db::now(),
	))
}
