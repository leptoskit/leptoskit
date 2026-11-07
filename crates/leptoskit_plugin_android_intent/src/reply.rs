//! Native replies, decoded strictly so a host bug cannot pass as an answer.

use leptoskit_bridge::Error;

use crate::MAX_LABEL_BYTES;
use crate::MAX_NAME_BYTES;
use crate::validate;
use crate::wire::Reader;
use crate::wire::Writer;

/// Decodes the four-byte [`crate::CAN_RESOLVE_ACTIVITY`] reply.
pub fn decode_bool(payload: &[u8]) -> Result<bool, Error> {
	let mut input = Reader::new(payload);
	let value = input.bool()?;
	input.finish()?;
	Ok(value)
}

/// Encodes a boolean reply.
#[must_use]
pub fn encode_bool(value: bool) -> [u8; 4] {
	u32::from(value).to_le_bytes()
}

/// The activity Android would start for an intent, as
/// `PackageManager.resolveActivity(intent, MATCH_DEFAULT_ONLY)` reports it.
///
/// When several apps can handle an intent and none is the default, this
/// is the system's chooser (`android/com.android.internal.app.ResolverActivity`
/// on stock Android), exactly as upstream reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedActivity {
	/// The handling application's package, such as `com.android.settings`.
	pub package_name: String,
	/// The activity's fully qualified class name.
	pub activity_name: String,
	/// The activity's user-visible label, truncated to [`MAX_LABEL_BYTES`].
	pub label: String,
}

impl ResolvedActivity {
	/// Encodes a [`crate::RESOLVED_ACTIVITY`] reply.
	pub fn encode(activity: Option<&Self>) -> Result<Vec<u8>, Error> {
		let mut output = Writer::default();

		let Some(activity) = activity else {
			output.u32(0);
			return Ok(output.finish());
		};

		output.u32(1);
		output.string(&activity.package_name)?;
		output.string(&activity.activity_name)?;
		output.string(&activity.label)?;
		let bytes = output.finish();
		Self::decode(&bytes)?;
		Ok(bytes)
	}

	/// Decodes a [`crate::RESOLVED_ACTIVITY`] reply: `0` for no activity,
	/// or `1` followed by the package, class and label strings.
	pub fn decode(payload: &[u8]) -> Result<Option<Self>, Error> {
		let mut input = Reader::new(payload);

		if !input.bool()? {
			input.finish()?;
			return Ok(None);
		}

		let package_name = input.string(MAX_NAME_BYTES)?;
		validate::package(package_name)?;
		let activity_name = input.string(MAX_NAME_BYTES)?;

		if activity_name.starts_with('.') {
			return Err(Error::InvalidArgument);
		}

		validate::class(activity_name)?;
		let label = input.string(MAX_LABEL_BYTES)?;
		input.finish()?;

		Ok(Some(Self {
			package_name: package_name.to_owned(),
			activity_name: activity_name.to_owned(),
			label: label.to_owned(),
		}))
	}
}
