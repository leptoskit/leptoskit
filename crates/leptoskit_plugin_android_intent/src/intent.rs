//! The intent a guest builds, and its bounded wire form.

use leptoskit_bridge::Error;

use crate::Extra;
use crate::Extras;
use crate::Flag;
use crate::MAX_CATEGORIES;
use crate::MAX_INTENT_BYTES;
use crate::MAX_MIME_BYTES;
use crate::MAX_NAME_BYTES;
use crate::MAX_TITLE_BYTES;
use crate::MAX_URI_BYTES;
use crate::WIRE_VERSION;
use crate::validate;
use crate::wire::Reader;
use crate::wire::Writer;

/// An Android intent described as data.
///
/// Mirrors `android_intent_plus`'s `AndroidIntent`: an action, categories,
/// a data URI, a MIME type, a package, a component class, flags and typed
/// extras. Build it with the chained methods; nothing is checked until the
/// intent is encoded, when every field is validated and the whole intent is
/// bounded.
///
/// ```
/// use leptoskit_plugin_android_intent::Intent;
/// use leptoskit_plugin_android_intent::action;
///
/// let intent = Intent::new()
/// 	.action(action::VIEW)
/// 	.data("https://play.google.com/store/apps/details?id=com.example")
/// 	.extra("authAccount", "someone@example.com");
/// assert!(intent.encode().is_ok());
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Intent {
	/// `Intent.setAction`, such as [`crate::action::VIEW`].
	pub action: Option<String>,
	/// `Intent.addCategory`, applied in order.
	pub categories: Vec<String>,
	/// `Intent.setData`: an absolute URI. `file:` URIs are refused.
	pub data: Option<String>,
	/// `Intent.setType`: a lowercase MIME type such as `text/plain`.
	pub mime_type: Option<String>,
	/// `Intent.setPackage`: limits resolution to one application.
	pub package: Option<String>,
	/// The class half of `Intent.setComponent`. Requires [`Self::package`];
	/// a leading `.` is relative to it.
	pub component: Option<String>,
	/// Combined `Intent.addFlags` bits.
	pub flags: u32,
	/// `Intent.putExtras`.
	pub extras: Extras,
}

impl Intent {
	/// An empty intent. It needs an action or a component before it can be
	/// sent.
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	/// Sets the action.
	#[must_use]
	pub fn action(mut self, action: impl Into<String>) -> Self {
		self.action = Some(action.into());
		self
	}

	/// Adds a category.
	#[must_use]
	pub fn category(mut self, category: impl Into<String>) -> Self {
		self.categories.push(category.into());
		self
	}

	/// Sets the data URI.
	#[must_use]
	pub fn data(mut self, uri: impl Into<String>) -> Self {
		self.data = Some(uri.into());
		self
	}

	/// Sets the MIME type. With [`Self::data`] this becomes
	/// `setDataAndType`, so neither clears the other.
	#[must_use]
	pub fn mime_type(mut self, mime_type: impl Into<String>) -> Self {
		self.mime_type = Some(mime_type.into());
		self
	}

	/// Restricts resolution to one package.
	#[must_use]
	pub fn package(mut self, package: impl Into<String>) -> Self {
		self.package = Some(package.into());
		self
	}

	/// Names the exact component class within [`Self::package`].
	#[must_use]
	pub fn component(mut self, class: impl Into<String>) -> Self {
		self.component = Some(class.into());
		self
	}

	/// Adds one flag.
	#[must_use]
	pub fn flag(mut self, flag: Flag) -> Self {
		self.flags |= flag.bits();
		self
	}

	/// Sets one extra, replacing an earlier extra with the same key.
	#[must_use]
	pub fn extra(mut self, key: impl Into<String>, value: impl Into<Extra>) -> Self {
		self.extras.insert(key, value);
		self
	}

	/// Replaces every extra.
	#[must_use]
	pub fn extras(mut self, extras: Extras) -> Self {
		self.extras = extras;
		self
	}

	/// Whether the intent names a package or component, which Android
	/// requires before it starts a service.
	#[must_use]
	pub fn is_explicit(&self) -> bool {
		self.package.is_some() || self.component.is_some()
	}

	/// Validates and encodes the intent for the bridge.
	///
	/// Fails with [`Error::TooLarge`] past a bound, [`Error::PermissionDenied`]
	/// for a URI-permission grant flag or a `file:` URI, and
	/// [`Error::InvalidArgument`] for anything malformed.
	pub fn encode(&self) -> Result<Vec<u8>, Error> {
		let mut output = Writer::default();
		self.write(&mut output)?;
		let bytes = output.finish();
		// Decoding is the one definition of a valid intent, shared with the
		// native side.
		Self::decode(&bytes)?;
		Ok(bytes)
	}

	fn write(&self, output: &mut Writer) -> Result<(), Error> {
		// The wire spells an absent field as an empty string, so an empty
		// value would silently become no value at all.
		for field in [
			&self.action,
			&self.data,
			&self.mime_type,
			&self.package,
			&self.component,
		] {
			if field.as_deref() == Some("") {
				return Err(Error::InvalidArgument);
			}
		}

		output.u32(WIRE_VERSION);
		output.u32(self.flags);
		output.optional(self.action.as_deref())?;
		output.optional(self.data.as_deref())?;
		output.optional(self.mime_type.as_deref())?;
		output.optional(self.package.as_deref())?;
		output.optional(self.component.as_deref())?;
		output.len(self.categories.len())?;

		for category in &self.categories {
			output.string(category)?;
		}

		self.extras.encode(output)
	}

	/// Decodes and validates an untrusted intent payload. The native plugin
	/// applies these same rules in the same order.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_INTENT_BYTES {
			return Err(Error::TooLarge);
		}

		let mut input = Reader::new(payload);

		if input.u32()? != WIRE_VERSION {
			return Err(Error::InvalidArgument);
		}

		let flags = input.u32()?;
		validate::flags(flags)?;
		let action = optional(&mut input, MAX_NAME_BYTES, validate::name)?;
		let data = optional(&mut input, MAX_URI_BYTES, validate::data)?;
		let mime_type = optional(&mut input, MAX_MIME_BYTES, validate::mime)?;
		let package = optional(&mut input, MAX_NAME_BYTES, validate::package)?;
		let component = optional(&mut input, MAX_NAME_BYTES, validate::class)?;
		let categories = categories(&mut input)?;
		let extras = Extras::decode(&mut input)?;
		input.finish()?;

		if (component.is_some() && package.is_none()) || (action.is_none() && component.is_none()) {
			return Err(Error::InvalidArgument);
		}

		Ok(Self {
			action,
			categories,
			data,
			mime_type,
			package,
			component,
			flags,
			extras,
		})
	}
}

fn optional(
	input: &mut Reader<'_>,
	max: usize,
	rule: fn(&str) -> Result<(), Error>,
) -> Result<Option<String>, Error> {
	let Some(value) = input.optional(max)? else {
		return Ok(None);
	};

	rule(value)?;
	Ok(Some(value.to_owned()))
}

fn categories(input: &mut Reader<'_>) -> Result<Vec<String>, Error> {
	let count = input.count(MAX_CATEGORIES)?;
	let mut categories: Vec<String> = Vec::with_capacity(count);

	for _ in 0..count {
		let category = input.string(MAX_NAME_BYTES)?;
		validate::name(category)?;

		if categories.iter().any(|existing| existing == category) {
			return Err(Error::InvalidArgument);
		}

		categories.push(category.to_owned());
	}

	Ok(categories)
}

/// The payload of [`crate::LAUNCH_CHOOSER`]: a title, then the intent.
#[derive(Debug, Clone, PartialEq)]
pub struct Chooser {
	/// The chooser's title; empty uses the system's default title.
	pub title: String,
	/// The intent the chooser offers.
	pub intent: Intent,
}

impl Chooser {
	/// Validates and encodes a chooser request.
	pub fn encode(intent: &Intent, title: &str) -> Result<Vec<u8>, Error> {
		if title.len() > MAX_TITLE_BYTES {
			return Err(Error::TooLarge);
		}

		validate::plain_text(title)?;
		let mut output = Writer::default();
		output.string(title)?;
		let mut bytes = output.finish();
		bytes.extend_from_slice(&intent.encode()?);
		Ok(bytes)
	}

	/// Decodes an untrusted chooser request.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		let mut input = Reader::new(payload);
		let title = input.string(MAX_TITLE_BYTES)?;
		validate::plain_text(title)?;
		let consumed = 4 + title.len();

		Ok(Self {
			title: title.to_owned(),
			intent: Intent::decode(&payload[consumed..])?,
		})
	}
}

/// A validated `intent:` URI for [`crate::PARSE_AND_LAUNCH`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentUri(String);

impl IntentUri {
	/// Accepts the `intent:…#Intent;…;end` form that
	/// `Intent.toUri(Intent.URI_INTENT_SCHEME)` writes, as printable ASCII
	/// of at most [`MAX_URI_BYTES`] bytes.
	pub fn parse(value: &str) -> Result<Self, Error> {
		if value.len() > MAX_URI_BYTES {
			return Err(Error::TooLarge);
		}

		validate::intent_uri(value)?;
		Ok(Self(value.to_owned()))
	}

	/// Validates an untrusted payload.
	pub fn decode(payload: &[u8]) -> Result<Self, Error> {
		if payload.len() > MAX_URI_BYTES {
			return Err(Error::TooLarge);
		}

		Self::parse(core::str::from_utf8(payload).map_err(|_| Error::InvalidArgument)?)
	}

	/// The URI text the native side parses.
	#[must_use]
	pub fn as_str(&self) -> &str {
		&self.0
	}
}
