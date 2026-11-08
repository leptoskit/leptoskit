//! <!-- {=p_device_keysSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_device_keys` as a direct dependency and select the
//! `device-keys` plugin in `native.plugins`; supported generated hosts discover
//! it from Cargo metadata. Apple generates a P-256 key in the Secure Enclave
//! and signs through `LAContext`; Android generates in `AndroidKeyStore` and
//! signs through `BiometricPrompt`. Every signature is fresh proof that the
//! device owner authenticated on this device just now; the key can never be
//! copied off the device. Desktop hosts without a native handler, and browsers,
//! report `Unavailable`.
//!
//! <!-- {/p_device_keysSummary} -->
//!
//! Wire integers are little-endian u32; strings and byte blobs are u32
//! length-prefixed. `available` takes empty bytes and returns version,
//! hardware, biometry, and an enrolled flag. `generate` takes version, policy,
//! then a length-prefixed alias and returns version, status, then a 65-byte
//! SEC1 point and an SPKI DER blob. `public_key` takes a length-prefixed alias
//! and returns version, status, a presence flag, then the two blobs when
//! present. `sign` takes version, alias, message, and reason and returns
//! version, status, then a DER signature. `delete` takes a length-prefixed
//! alias and returns version and status. Every decoder rejects trailing data.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Stable native registration name.
pub const NAME: &str = "device-keys";
/// [`GenerateRequest`] -> encoded [`PublicKey`].
pub const GENERATE: MethodId = MethodId(0);
/// Length-prefixed alias -> presence flag then encoded [`PublicKey`] when
/// present.
pub const PUBLIC_KEY: MethodId = MethodId(1);
/// [`SignRequest`] -> encoded [`Signature`].
pub const SIGN: MethodId = MethodId(2);
/// Length-prefixed alias -> empty response.
pub const DELETE: MethodId = MethodId(3);
/// Empty -> encoded [`Availability`].
pub const AVAILABLE: MethodId = MethodId(4);

/// Longest accepted alias, in ASCII bytes.
pub const MAX_ALIAS_BYTES: usize = 64;
/// Longest accepted message, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 4096;
/// Longest accepted authentication reason, in UTF-8 bytes.
pub const MAX_REASON_BYTES: usize = 200;
/// An uncompressed SEC1 point: `0x04 || X || Y`, 65 bytes.
pub const SEC1_BYTES: usize = 65;
/// A raw ECDSA P-256 signature: `r || s`, 64 bytes.
pub const RAW_SIGNATURE_BYTES: usize = 64;
/// Longest accepted SPKI DER subject public key (P-256 is ~91 bytes).
pub const MAX_SPKI_BYTES: usize = 256;
/// Longest accepted DER ECDSA signature (P-256 is at most 72 bytes).
pub const MAX_DER_BYTES: usize = 128;
/// The reply version every method writes.
const REPLY_VERSION: u32 = 1;

/// Authentication policy for a generated key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Policy {
	/// Current enrolled biometrics only; a new fingerprint or face invalidates
	/// the key.
	Biometry = 0,
	/// Biometrics or the operating system's device passcode.
	BiometryOrPasscode = 1,
}
impl Policy {
	/// Decodes a policy from its wire code.
	pub fn from_wire(value: u32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::Biometry),
			1 => Ok(Self::BiometryOrPasscode),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Reported biometric hardware kind. These describe hardware, not enrollment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Biometry {
	/// Apple Touch ID.
	TouchId = 0,
	/// Apple Face ID.
	FaceId = 1,
	/// Apple Optic ID.
	OpticId = 2,
	/// Android fingerprint.
	Fingerprint = 3,
	/// Android face.
	Face = 4,
	/// Android iris.
	Iris = 5,
	/// No biometric hardware.
	None = 6,
}
impl Biometry {
	/// Decodes a biometry kind from its wire code.
	pub fn from_wire(value: u32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::TouchId),
			1 => Ok(Self::FaceId),
			2 => Ok(Self::OpticId),
			3 => Ok(Self::Fingerprint),
			4 => Ok(Self::Face),
			5 => Ok(Self::Iris),
			6 => Ok(Self::None),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Reported hardware-backed key storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Hardware {
	/// Apple Secure Enclave.
	SecureEnclave = 0,
	/// Android `StrongBox`.
	StrongBox = 1,
	/// Android trusted execution environment (TEE) keystore.
	Tee = 2,
	/// No hardware-backed key storage.
	None = 3,
}
impl Hardware {
	/// Decodes a hardware kind from its wire code.
	pub fn from_wire(value: u32) -> Result<Self, Error> {
		match value {
			0 => Ok(Self::SecureEnclave),
			1 => Ok(Self::StrongBox),
			2 => Ok(Self::Tee),
			3 => Ok(Self::None),
			_ => Err(Error::InvalidArgument),
		}
	}
}

/// Key-storage capability snapshot. It reports what the platform can do,
/// never proof of a past authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Availability {
	/// Hardware-backed key storage kind.
	pub hardware: Hardware,
	/// Reported biometric hardware kind.
	pub biometry: Biometry,
	/// Whether an acceptable biometric (or, for `BiometryOrPasscode`, the
	/// device passcode) is enrolled.
	pub enrolled: bool,
}
impl Availability {
	/// The snapshot for a platform with no hardware-backed key storage.
	pub const UNAVAILABLE: Self = Self {
		hardware: Hardware::None,
		biometry: Biometry::None,
		enrolled: false,
	};

	/// Decodes the fixed 16-byte availability reply.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		if input.u32()? != REPLY_VERSION {
			return Err(Error::Native);
		}

		let hardware = Hardware::from_wire(input.u32()?)?;
		let biometry = Biometry::from_wire(input.u32()?)?;
		let enrolled = match input.u32()? {
			0 => false,
			1 => true,
			_ => return Err(Error::InvalidArgument),
		};
		input.finish()?;
		Ok(Self {
			hardware,
			biometry,
			enrolled,
		})
	}
}

/// Terminal errors a key operation reports. These come from the operating
/// system or platform and are distinct from the transport failures
/// [`Error`] reports for an uninstalled plugin or a malformed message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum KeyError {
	/// No key exists under that alias.
	NotFound = 1,
	/// A key already exists under that alias.
	AlreadyExists = 2,
	/// The user dismissed the system authentication prompt.
	Cancelled = 3,
	/// Authentication is locked out after too many failed attempts.
	LockedOut = 4,
	/// No acceptable biometric or device passcode is enrolled.
	NotEnrolled = 5,
	/// The enrolled biometrics changed and invalidated the key; delete it and
	/// generate a new one.
	Invalidated = 6,
	/// The platform or hardware is unavailable; no software fallback is used.
	Unavailable = 7,
	/// The native operation failed for another reason.
	Failed = 8,
}
impl KeyError {
	/// Decodes a status from its wire code.
	pub fn from_wire(code: u32) -> Result<Self, Error> {
		match code {
			1 => Ok(Self::NotFound),
			2 => Ok(Self::AlreadyExists),
			3 => Ok(Self::Cancelled),
			4 => Ok(Self::LockedOut),
			5 => Ok(Self::NotEnrolled),
			6 => Ok(Self::Invalidated),
			7 => Ok(Self::Unavailable),
			8 => Ok(Self::Failed),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Maps a transport or validation [`Error`] onto the closest terminal
	/// outcome, so callers see one error type.
	pub fn from_bridge(error: Error) -> Self {
		match error {
			Error::Unavailable | Error::PermissionDenied => Self::Unavailable,
			Error::Cancelled => Self::Cancelled,
			Error::InvalidArgument
			| Error::TooLarge
			| Error::Busy
			| Error::Native
			| Error::InvalidCall
			| Error::Exhausted => Self::Failed,
		}
	}
}

/// A generated P-256 public key in its uncompressed SEC1 and SPKI DER forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
	sec1: Vec<u8>,
	spki_der: Vec<u8>,
}
impl PublicKey {
	/// The uncompressed SEC1 point: `0x04 || X || Y`, 65 bytes.
	pub fn sec1(&self) -> &[u8] {
		&self.sec1
	}

	/// The X.509 `SubjectPublicKeyInfo` (SPKI) DER encoding, suitable for the
	/// `p256` crate's `VerifyingKey::from_sec1_der`.
	pub fn spki_der(&self) -> &[u8] {
		&self.spki_der
	}

	/// Decodes the two length-prefixed blobs from a reply body.
	fn decode(input: &mut Decoder<'_>) -> Result<Self, Error> {
		let sec1 = input.bytes()?;

		if sec1.len() != SEC1_BYTES || sec1[0] != 0x04 {
			return Err(Error::Native);
		}

		let spki_der = input.bytes()?;

		if spki_der.is_empty() || spki_der.len() > MAX_SPKI_BYTES {
			return Err(Error::Native);
		}

		Ok(Self {
			sec1: sec1.to_vec(),
			spki_der: spki_der.to_vec(),
		})
	}
}

/// An ECDSA P-256 / SHA-256 signature in DER and raw `r || s` forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
	der: Vec<u8>,
	raw: [u8; RAW_SIGNATURE_BYTES],
}
impl Signature {
	/// The ASN.1 DER signature the operating system produced.
	pub fn der(&self) -> &[u8] {
		&self.der
	}

	/// The raw fixed-width `r || s` form, 64 bytes.
	pub fn raw(&self) -> &[u8; RAW_SIGNATURE_BYTES] {
		&self.raw
	}

	/// Parses a DER ECDSA signature into its raw `r || s` form.
	pub fn from_der(der: &[u8]) -> Result<Self, Error> {
		if der.is_empty() || der.len() > MAX_DER_BYTES {
			return Err(Error::Native);
		}

		Ok(Self {
			der: der.to_vec(),
			raw: der_to_raw(der)?,
		})
	}

	/// Decodes a length-prefixed DER signature from a reply body.
	fn decode(input: &mut Decoder<'_>) -> Result<Self, Error> {
		Self::from_der(input.bytes()?)
	}
}

/// Validated request for [`GENERATE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerateRequest {
	alias: String,
	policy: Policy,
}
impl GenerateRequest {
	/// Requires an alias of 1–64 ASCII `[A-Za-z0-9._-]` characters.
	pub fn new(alias: impl Into<String>, policy: Policy) -> Result<Self, Error> {
		let alias = alias.into();
		validate_alias(&alias)?;
		Ok(Self { alias, policy })
	}

	/// The key alias.
	pub fn alias(&self) -> &str {
		&self.alias
	}

	/// The authentication policy.
	pub const fn policy(&self) -> Policy {
		self.policy
	}

	/// Encodes the validated request.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(8 + self.alias.len());
		output.u32(1);
		output.u32(self.policy as u32);
		output.bytes(self.alias.as_bytes())?;
		Ok(output.finish())
	}

	/// Validates version, policy, alias, and complete consumption.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);

		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}

		let policy = Policy::from_wire(input.u32()?)?;
		let alias = input.string()?;
		input.finish()?;
		Self::new(alias, policy)
	}
}

/// Validated request for [`SIGN`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignRequest {
	alias: String,
	message: Vec<u8>,
	reason: String,
}
impl SignRequest {
	/// Requires a valid alias, a message of 1–4096 bytes, and a non-blank
	/// reason of at most 200 UTF-8 bytes.
	pub fn new(
		alias: impl Into<String>,
		message: Vec<u8>,
		reason: impl Into<String>,
	) -> Result<Self, Error> {
		let alias = alias.into();
		let reason = reason.into();
		validate_alias(&alias)?;
		validate_message(&message)?;
		validate_reason(&reason)?;
		Ok(Self {
			alias,
			message,
			reason,
		})
	}

	/// The key alias.
	pub fn alias(&self) -> &str {
		&self.alias
	}

	/// The message to sign.
	pub fn message(&self) -> &[u8] {
		&self.message
	}

	/// The reason shown by the operating system.
	pub fn reason(&self) -> &str {
		&self.reason
	}

	/// Encodes the validated request.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output =
			Encoder::with_capacity(8 + self.alias.len() + self.message.len() + self.reason.len());
		output.u32(1);
		output.bytes(self.alias.as_bytes())?;
		output.bytes(&self.message)?;
		output.bytes(self.reason.as_bytes())?;
		Ok(output.finish())
	}

	/// Validates version, fields, bounds, and complete consumption.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);

		if input.u32()? != 1 {
			return Err(Error::InvalidArgument);
		}

		let alias = input.string()?;
		let message = input.bytes()?.to_vec();
		let reason = input.string()?;
		input.finish()?;
		Self::new(alias, message, reason)
	}
}

/// Encodes a length-prefixed alias for `public_key` and `delete`.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
fn encode_alias(alias: &str) -> Result<Bytes, Error> {
	validate_alias(alias)?;
	let mut output = Encoder::with_capacity(4 + alias.len());
	output.bytes(alias.as_bytes())?;
	Ok(output.finish())
}

/// Validates an alias: 1–64 ASCII letters, digits, `.`, `_`, or `-`.
fn validate_alias(alias: &str) -> Result<(), Error> {
	if alias.is_empty()
		|| alias.len() > MAX_ALIAS_BYTES
		|| !alias
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
	{
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// Validates a message: 1–4096 bytes.
fn validate_message(message: &[u8]) -> Result<(), Error> {
	if message.is_empty() {
		return Err(Error::InvalidArgument);
	}

	if message.len() > MAX_MESSAGE_BYTES {
		return Err(Error::TooLarge);
	}

	Ok(())
}

/// Validates a reason: non-blank, no NUL, at most 200 UTF-8 bytes.
fn validate_reason(reason: &str) -> Result<(), Error> {
	if reason.len() > MAX_REASON_BYTES {
		return Err(Error::TooLarge);
	}

	if reason.trim().is_empty() || reason.contains('\0') {
		return Err(Error::InvalidArgument);
	}

	Ok(())
}

/// Reads a reply's version and status. `Ok(None)` means success; `Ok(Some(..))`
/// carries the terminal [`KeyError`]. A malformed envelope becomes
/// [`KeyError::Failed`].
fn read_status(input: &mut Decoder<'_>) -> Result<Option<KeyError>, KeyError> {
	if input.u32().map_err(KeyError::from_bridge)? != REPLY_VERSION {
		return Err(KeyError::Failed);
	}

	let status = input.u32().map_err(KeyError::from_bridge)?;

	if status == 0 {
		Ok(None)
	} else {
		KeyError::from_wire(status)
			.map(Some)
			.map_err(|_| KeyError::Failed)
	}
}

/// Decodes a generate reply into its public key, or the terminal [`KeyError`].
pub fn decode_generate(bytes: &[u8]) -> Result<PublicKey, KeyError> {
	let mut input = Decoder::new(bytes);

	if let Some(error) = read_status(&mut input)? {
		input.finish().map_err(KeyError::from_bridge)?;
		return Err(error);
	}

	let key = PublicKey::decode(&mut input).map_err(KeyError::from_bridge)?;
	input.finish().map_err(KeyError::from_bridge)?;
	Ok(key)
}

/// Decodes a public-key reply into presence plus the key, or a terminal
/// [`KeyError`].
pub fn decode_public_key(bytes: &[u8]) -> Result<Option<PublicKey>, KeyError> {
	let mut input = Decoder::new(bytes);

	if let Some(error) = read_status(&mut input)? {
		input.finish().map_err(KeyError::from_bridge)?;
		return Err(error);
	}

	match input.u32().map_err(KeyError::from_bridge)? {
		0 => {
			input.finish().map_err(KeyError::from_bridge)?;
			Ok(None)
		}
		1 => {
			let key = PublicKey::decode(&mut input).map_err(KeyError::from_bridge)?;
			input.finish().map_err(KeyError::from_bridge)?;
			Ok(Some(key))
		}
		_ => Err(KeyError::Failed),
	}
}

/// Decodes a sign reply into its signature, or the terminal [`KeyError`].
pub fn decode_sign(bytes: &[u8]) -> Result<Signature, KeyError> {
	let mut input = Decoder::new(bytes);

	if let Some(error) = read_status(&mut input)? {
		input.finish().map_err(KeyError::from_bridge)?;
		return Err(error);
	}

	let signature = Signature::decode(&mut input).map_err(KeyError::from_bridge)?;
	input.finish().map_err(KeyError::from_bridge)?;
	Ok(signature)
}

/// Decodes a delete reply into success, or the terminal [`KeyError`].
pub fn decode_delete(bytes: &[u8]) -> Result<(), KeyError> {
	let mut input = Decoder::new(bytes);

	if let Some(error) = read_status(&mut input)? {
		input.finish().map_err(KeyError::from_bridge)?;
		return Err(error);
	}

	input.finish().map_err(KeyError::from_bridge)?;
	Ok(())
}

/// Converts a DER ECDSA signature into its fixed-width raw `r || s` form.
fn der_to_raw(der: &[u8]) -> Result<[u8; RAW_SIGNATURE_BYTES], Error> {
	let mut rest = der;

	if take(&mut rest)? != 0x30 {
		return Err(Error::Native);
	}

	let sequence_len = take_length(&mut rest)?;

	if rest.len() != sequence_len {
		return Err(Error::Native);
	}

	let r = take_integer(&mut rest)?;
	let s = take_integer(&mut rest)?;

	if !rest.is_empty() {
		return Err(Error::Native);
	}

	let mut raw = [0_u8; RAW_SIGNATURE_BYTES];
	raw[..32].copy_from_slice(&fixed_width(&r)?);
	raw[32..].copy_from_slice(&fixed_width(&s)?);
	Ok(raw)
}

/// Reads one byte, advancing the cursor.
fn take(bytes: &mut &[u8]) -> Result<u8, Error> {
	let (first, rest) = bytes.split_first().ok_or(Error::Native)?;
	*bytes = rest;
	Ok(*first)
}

/// Reads a DER length, short or long form.
fn take_length(bytes: &mut &[u8]) -> Result<usize, Error> {
	let first = take(bytes)?;

	if first & 0x80 == 0 {
		return Ok(usize::from(first));
	}

	let count = usize::from(first & 0x7f);

	if count == 0 || count > 4 {
		return Err(Error::Native);
	}

	let mut length = 0_usize;

	for _ in 0..count {
		let byte = usize::from(take(bytes)?);
		length = length
			.checked_mul(256)
			.and_then(|value| value.checked_add(byte))
			.ok_or(Error::Native)?;
	}

	Ok(length)
}

/// Reads a DER INTEGER's contents.
fn take_integer(bytes: &mut &[u8]) -> Result<Vec<u8>, Error> {
	if take(bytes)? != 0x02 {
		return Err(Error::Native);
	}

	let length = take_length(bytes)?;

	if length == 0 || length > bytes.len() {
		return Err(Error::Native);
	}

	let (value, rest) = bytes.split_at(length);
	*bytes = rest;
	Ok(value.to_vec())
}

/// Pads a big-endian unsigned integer to 32 bytes, stripping one redundant
/// sign byte.
fn fixed_width(value: &[u8]) -> Result<[u8; 32], Error> {
	let mut magnitude = value;

	if magnitude.len() > 1 && magnitude[0] == 0x00 && magnitude[1] & 0x80 != 0 {
		magnitude = &magnitude[1..];
	}

	if magnitude.len() > 32 {
		return Err(Error::Native);
	}

	let mut output = [0_u8; 32];
	output[32 - magnitude.len()..].copy_from_slice(magnitude);
	Ok(output)
}

/// Browsers have no hardware-bound, non-exportable key store. WebAuthn is a
/// different relying-party protocol and is not silently substituted.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
#[expect(
	clippy::unused_async,
	reason = "the stubs stay async so browser and native callers share one API"
)]
pub mod web {
	use super::*;

	/// Browsers have no hardware-bound key storage.
	pub const fn available() -> Availability {
		Availability::UNAVAILABLE
	}

	/// Always returns Unavailable without creating a software key.
	pub async fn generate(_alias: &str, _policy: Policy) -> Result<PublicKey, KeyError> {
		Err(KeyError::Unavailable)
	}

	/// Always returns Unavailable.
	pub async fn public_key(_alias: &str) -> Result<Option<PublicKey>, KeyError> {
		Err(KeyError::Unavailable)
	}

	/// Always returns Unavailable; this never signs with a software key.
	pub async fn sign(_alias: &str, _message: &[u8], _reason: &str) -> Result<Signature, KeyError> {
		Err(KeyError::Unavailable)
	}

	/// Always returns Unavailable.
	pub async fn delete(_alias: &str) -> Result<(), KeyError> {
		Err(KeyError::Unavailable)
	}
}

/// Cached native interpreter client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// One resolved device-keys plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct DeviceKeys(leptoskit_bridge::guest::Client);
	impl DeviceKeys {
		/// Resolves the installed plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Creates a new hardware-bound P-256 key under `alias`. Fails with
		/// [`KeyError::AlreadyExists`] when the alias already exists.
		pub async fn generate(self, alias: &str, policy: Policy) -> Result<PublicKey, KeyError> {
			let request = GenerateRequest::new(alias, policy)
				.map_err(KeyError::from_bridge)?
				.encode()
				.map_err(KeyError::from_bridge)?;
			let reply = self.call(GENERATE, &request).await?;
			decode_generate(reply.as_ref())
		}

		/// Reads a key's public key, or `None` when it does not exist.
		pub async fn public_key(self, alias: &str) -> Result<Option<PublicKey>, KeyError> {
			let payload = encode_alias(alias).map_err(KeyError::from_bridge)?;
			let reply = self.call(PUBLIC_KEY, &payload).await?;
			decode_public_key(reply.as_ref())
		}

		/// Shows the system prompt with `reason`, then signs `message` with the
		/// key. Every signature is fresh proof of a just-now authentication.
		pub async fn sign(
			self,
			alias: &str,
			message: &[u8],
			reason: &str,
		) -> Result<Signature, KeyError> {
			let request = SignRequest::new(alias, message.to_vec(), reason)
				.map_err(KeyError::from_bridge)?
				.encode()
				.map_err(KeyError::from_bridge)?;
			let reply = self.call(SIGN, &request).await?;
			decode_sign(reply.as_ref())
		}

		/// Deletes a key. Deleting an absent alias succeeds.
		pub async fn delete(self, alias: &str) -> Result<(), KeyError> {
			let payload = encode_alias(alias).map_err(KeyError::from_bridge)?;
			let reply = self.call(DELETE, &payload).await?;
			decode_delete(reply.as_ref())
		}

		/// Reports the platform's hardware, biometric hardware, and enrollment,
		/// without prompting. Unsupported platforms report
		/// [`Availability::UNAVAILABLE`].
		pub async fn available(self) -> Availability {
			match self.call(AVAILABLE, &[]).await {
				Ok(reply) => {
					Availability::decode(reply.as_ref()).unwrap_or(Availability::UNAVAILABLE)
				}
				Err(_) => Availability::UNAVAILABLE,
			}
		}

		async fn call(self, method: MethodId, payload: &[u8]) -> Result<Bytes, KeyError> {
			self.0
				.call(method, payload)
				.map_err(KeyError::from_bridge)?
				.await
				.map_err(KeyError::from_bridge)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fields(fields: &[u32]) -> Vec<u8> {
		fields
			.iter()
			.flat_map(|value| value.to_le_bytes())
			.collect()
	}

	fn blob(value: &[u8]) -> Vec<u8> {
		let mut output = fields(&[u32::try_from(value.len()).unwrap()]);
		output.extend_from_slice(value);
		output
	}

	/// A minimal 65-byte uncompressed SEC1 point.
	fn sec1() -> Vec<u8> {
		let mut point = vec![0x04];
		point.extend_from_slice(&[0x11; 64]);
		point
	}

	/// A minimal plausible SPKI DER blob.
	fn spki() -> Vec<u8> {
		vec![0x30; 91]
	}

	#[test]
	fn aliases_reasons_and_messages_are_validated() {
		for alias in [
			"",
			"a/b",
			"a\0b",
			"é",
			"a b",
			&"a".repeat(MAX_ALIAS_BYTES + 1),
		] {
			assert_eq!(
				GenerateRequest::new(alias, Policy::Biometry),
				Err(Error::InvalidArgument)
			);
		}
		assert!(GenerateRequest::new("my.key-1", Policy::Biometry).is_ok());

		assert_eq!(
			SignRequest::new("k", b"".to_vec(), "Approve"),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			SignRequest::new("k", vec![0; MAX_MESSAGE_BYTES + 1], "Approve"),
			Err(Error::TooLarge)
		);
		assert_eq!(
			SignRequest::new("k", b"m".to_vec(), " \n"),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			SignRequest::new("k", b"m".to_vec(), "reason\0hidden"),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			SignRequest::new("k", b"m".to_vec(), "x".repeat(MAX_REASON_BYTES + 1)),
			Err(Error::TooLarge)
		);
		assert!(SignRequest::new("k", b"m".to_vec(), "Approve transfer").is_ok());
	}

	#[test]
	fn generate_request_roundtrips_and_rejects_trailing_data() {
		let request = GenerateRequest::new("payout.v2", Policy::BiometryOrPasscode).unwrap();
		let encoded = request.encode().unwrap();
		assert_eq!(GenerateRequest::decode(&encoded), Ok(request));

		for length in 0..encoded.len() {
			assert!(GenerateRequest::decode(&encoded[..length]).is_err());
		}

		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert!(GenerateRequest::decode(&trailing).is_err());

		let mut bad_policy = encoded.to_vec();
		bad_policy[4] = 9;
		assert!(GenerateRequest::decode(&bad_policy).is_err());
	}

	#[test]
	fn sign_request_roundtrips_and_rejects_trailing_data() {
		let request = SignRequest::new("payout.v2", vec![1, 2, 3], "Approve transfer").unwrap();
		let encoded = request.encode().unwrap();
		assert_eq!(SignRequest::decode(&encoded), Ok(request));

		for length in 0..encoded.len() {
			assert!(SignRequest::decode(&encoded[..length]).is_err());
		}

		let mut trailing = encoded.to_vec();
		trailing.push(0);
		assert!(SignRequest::decode(&trailing).is_err());
	}

	#[test]
	fn availability_decodes_exactly_and_rejects_bad_values() {
		let bytes = fields(&[1, 0, 3, 1]);
		let availability = Availability::decode(&bytes).unwrap();
		assert_eq!(availability.hardware, Hardware::SecureEnclave);
		assert_eq!(availability.biometry, Biometry::Fingerprint);
		assert!(availability.enrolled);

		for length in 0..bytes.len() {
			assert!(Availability::decode(&bytes[..length]).is_err());
		}

		for values in [[1, 9, 0, 0], [1, 0, 9, 0], [1, 0, 0, 2], [2, 0, 0, 0]] {
			assert!(Availability::decode(&fields(&values)).is_err());
		}
	}

	#[test]
	fn public_key_and_signature_replies_roundtrip() {
		let mut generate = fields(&[1, 0]);
		generate.extend_from_slice(&blob(&sec1()));
		generate.extend_from_slice(&blob(&spki()));
		let key = decode_generate(&generate).unwrap();
		assert_eq!(key.sec1(), sec1().as_slice());
		assert_eq!(key.spki_der(), spki().as_slice());

		let mut public_key = fields(&[1, 0, 1]);
		public_key.extend_from_slice(&blob(&sec1()));
		public_key.extend_from_slice(&blob(&spki()));
		assert!(matches!(decode_public_key(&public_key), Ok(Some(_))));

		assert!(matches!(decode_public_key(&fields(&[1, 0, 0])), Ok(None)));

		let mut bad_point = fields(&[1, 0]);
		bad_point.extend_from_slice(&blob(&[0x03; 65]));
		bad_point.extend_from_slice(&blob(&spki()));
		assert!(decode_generate(&bad_point).is_err());

		let mut empty_spki = fields(&[1, 0]);
		empty_spki.extend_from_slice(&blob(&sec1()));
		empty_spki.extend_from_slice(&blob(&[]));
		assert!(decode_generate(&empty_spki).is_err());
	}

	#[test]
	fn replies_carry_every_terminal_error() {
		for (code, error) in [
			(1, KeyError::NotFound),
			(2, KeyError::AlreadyExists),
			(3, KeyError::Cancelled),
			(4, KeyError::LockedOut),
			(5, KeyError::NotEnrolled),
			(6, KeyError::Invalidated),
			(7, KeyError::Unavailable),
			(8, KeyError::Failed),
		] {
			let bytes = fields(&[1, code]);
			assert_eq!(decode_delete(&bytes), Err(error));
			assert_eq!(decode_generate(&bytes), Err(error));
			assert_eq!(decode_public_key(&bytes), Err(error));
			assert_eq!(decode_sign(&bytes), Err(error));
		}

		assert!(decode_delete(&fields(&[1, 9])).is_err());
		assert!(decode_delete(&[]).is_err());
		assert!(decode_delete(&fields(&[1, 0, 0])).is_err());
	}

	#[test]
	fn key_error_wire_codes_and_bridge_mapping_are_exact() {
		for code in 1..=8 {
			assert!(KeyError::from_wire(code).is_ok());
		}
		assert!(KeyError::from_wire(0).is_err());
		assert!(KeyError::from_wire(9).is_err());

		assert_eq!(
			KeyError::from_bridge(Error::Unavailable),
			KeyError::Unavailable
		);
		assert_eq!(
			KeyError::from_bridge(Error::PermissionDenied),
			KeyError::Unavailable
		);
		assert_eq!(KeyError::from_bridge(Error::Cancelled), KeyError::Cancelled);
		assert_eq!(
			KeyError::from_bridge(Error::InvalidArgument),
			KeyError::Failed
		);
		assert_eq!(KeyError::from_bridge(Error::Native), KeyError::Failed);
	}

	#[test]
	fn der_signatures_parse_to_fixed_width_raw_form() {
		// r = 0xab.. (high bit set, needs a sign byte), s = 0x01.. (does not).
		let r = vec![0xab; 32];
		let s = vec![0x01; 32];

		let mut der = vec![0x02, 33, 0x00];
		der.extend_from_slice(&r);
		der.push(0x02);
		der.push(32);
		der.extend_from_slice(&s);
		let mut sequence = vec![0x30, u8::try_from(der.len()).unwrap()];
		sequence.extend_from_slice(&der);

		let signature = Signature::from_der(&sequence).unwrap();
		let mut expected = [0_u8; 64];
		expected[..32].copy_from_slice(&r);
		expected[32..].copy_from_slice(&s);
		assert_eq!(signature.raw(), &expected);
		assert_eq!(signature.der(), sequence.as_slice());

		// Truncation, wrong tags, and oversized integers are rejected.
		for length in 0..sequence.len() {
			assert!(Signature::from_der(&sequence[..length]).is_err());
		}
		assert!(Signature::from_der(&[]).is_err());
		assert!(Signature::from_der(&[0x31, 0]).is_err());

		let mut oversized = vec![0x30, 2 + 2 + 34 + 2 + 1];
		oversized.extend_from_slice(&[0x02, 34]);
		oversized.extend_from_slice(&[0x00; 34]);
		oversized.extend_from_slice(&[0x02, 1, 0x01]);
		assert!(Signature::from_der(&oversized).is_err());
	}

	#[test]
	fn sign_replies_roundtrip_the_signature() {
		let r = [0x2a; 32];
		let s = [0x2b; 32];
		let mut der = vec![0x02, 32];
		der.extend_from_slice(&r);
		der.push(0x02);
		der.push(32);
		der.extend_from_slice(&s);
		let mut sequence = vec![0x30, u8::try_from(der.len()).unwrap()];
		sequence.extend_from_slice(&der);

		let mut reply = fields(&[1, 0]);
		reply.extend_from_slice(&blob(&sequence));
		let signature = decode_sign(&reply).unwrap();
		assert_eq!(signature.der(), sequence.as_slice());
		assert_eq!(signature.raw()[..32], r);
		assert_eq!(signature.raw()[32..], s);

		let mut trailing = reply.clone();
		trailing.push(0);
		assert!(decode_sign(&trailing).is_err());
	}
}
