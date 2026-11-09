//! <!-- {=p_background_taskSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_background_task` as a direct dependency and select the
//! `background-task` plugin in `native.plugins`; supported generated hosts
//! discover it from Cargo metadata. Generated Linux hosts register it where it
//! has a Linux Rust implementation; generated Windows and terminal hosts
//! currently omit runtime plugins, so use explicit registration in a custom
//! host there. Guests enable the `guest` feature and reuse
//! `guest::BackgroundTask` rather than resolving the plugin on every call.
//! Register and unregister named OS background tasks, query availability, and
//! complete an in-flight invocation before the OS expiration window. This is
//! distinct from `leptoskit_plugin_background`, which keeps an `AppService`
//! awake with a foreground service / scheduled wake. Android uses
//! `WorkManager`; Apple, Windows, Linux, HarmonyOS, and the browser return
//! `Unavailable` until declared `BGTaskScheduler` identifiers and honest
//! register/submit error handling land. Guaranteed cadence and in-guest Wasm
//! execution while suspended remain later scope.
//!
//! <!-- {/p_background_taskSummary} -->
//!
//! Wire v1: status is empty → four-byte [`Status`]; register takes interval
//! seconds + length-prefixed name; unregister/complete take a length-prefixed
//! name. All integers are little-endian; trailing bytes are rejected.

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime registration name.
pub const NAME: &str = "background-task";
/// Empty request → [`Status`].
pub const STATUS: MethodId = MethodId(0);
/// [`Register`] → empty reply.
pub const REGISTER: MethodId = MethodId(1);
/// Task name → empty reply.
pub const UNREGISTER: MethodId = MethodId(2);
/// Task name → empty reply; ends the current OS invocation.
pub const COMPLETE: MethodId = MethodId(3);
/// Supported wire operations.
pub const METHODS: &[MethodId] = &[STATUS, REGISTER, UNREGISTER, COMPLETE];

/// Maximum UTF-8 bytes in a task name.
pub const MAX_NAME_BYTES: usize = 128;
/// Minimum allowed interval, in seconds (`WorkManager`'s periodic floor).
pub const MIN_INTERVAL_SECS: u32 = 900;
/// Maximum allowed interval, in seconds (one day).
pub const MAX_INTERVAL_SECS: u32 = 86_400;

/// Whether the host can schedule background tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Status {
	/// The platform cannot schedule background work for this app.
	Restricted = 0,
	/// Registration is allowed; the OS still chooses when tasks run.
	Available = 1,
	/// This host has no background-task scheduler.
	Unavailable = 2,
}

impl Status {
	/// Rejects unknown values and non-exact response lengths.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		match word(bytes)? {
			0 => Ok(Self::Restricted),
			1 => Ok(Self::Available),
			2 => Ok(Self::Unavailable),
			_ => Err(Error::InvalidArgument),
		}
	}

	/// Fixed four-byte response.
	#[must_use]
	pub fn encode(self) -> [u8; 4] {
		(self as u32).to_le_bytes()
	}
}

/// Register a named periodic background task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Register {
	name: String,
	minimum_interval_secs: u32,
}

impl Register {
	/// Rejects empty/NUL/oversized names and out-of-range intervals.
	pub fn new(name: impl Into<String>, minimum_interval_secs: u32) -> Result<Self, Error> {
		let name = validate_name(name.into())?;
		if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&minimum_interval_secs) {
			return Err(Error::InvalidArgument);
		}
		Ok(Self {
			name,
			minimum_interval_secs,
		})
	}

	/// Task name.
	#[must_use]
	pub fn name(&self) -> &str {
		&self.name
	}

	/// Minimum interval hint in seconds.
	#[must_use]
	pub fn minimum_interval_secs(&self) -> u32 {
		self.minimum_interval_secs
	}

	/// Encodes interval then length-prefixed name.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(8 + self.name.len());
		output.u32(self.minimum_interval_secs);
		output.bytes(self.name.as_bytes())?;
		Ok(output.finish())
	}

	/// Decodes through [`Register::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let minimum_interval_secs = input.u32()?;
		let name = input.string()?.to_owned();
		input.finish()?;
		Self::new(name, minimum_interval_secs)
	}
}

/// Unregister or complete by task name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskName {
	name: String,
}

impl TaskName {
	/// Rejects empty/NUL/oversized names.
	pub fn new(name: impl Into<String>) -> Result<Self, Error> {
		Ok(Self {
			name: validate_name(name.into())?,
		})
	}

	/// Task name.
	#[must_use]
	pub fn name(&self) -> &str {
		&self.name
	}

	/// Encodes a length-prefixed name.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(4 + self.name.len());
		output.bytes(self.name.as_bytes())?;
		Ok(output.finish())
	}

	/// Decodes through [`TaskName::new`], rejecting trailing bytes.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);
		let name = input.string()?.to_owned();
		input.finish()?;
		Self::new(name)
	}
}

fn validate_name(name: String) -> Result<String, Error> {
	if name.is_empty() || name.contains('\0') || name.len() > MAX_NAME_BYTES {
		return Err(Error::InvalidArgument);
	}
	Ok(name)
}

fn word(bytes: &[u8]) -> Result<u32, Error> {
	let mut input = Decoder::new(bytes);
	let value = input.u32()?;
	input.finish()?;
	Ok(value)
}

/// Validates method payloads without invoking a host.
pub fn validate(method: MethodId, payload: &[u8]) -> Result<(), Error> {
	match method {
		STATUS => Decoder::new(payload).finish(),
		REGISTER => Register::decode(payload).map(|_| ()),
		UNREGISTER | COMPLETE => TaskName::decode(payload).map(|_| ()),
		_ => Err(Error::Unavailable),
	}
}

/// Explicit unavailable handler for hosts without a scheduler.
#[derive(Debug, Default)]
pub struct Unsupported;

impl leptoskit_bridge::Plugin for Unsupported {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		METHODS
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		validate(method, &payload)?;
		match method {
			STATUS => Ok(Bytes::copy_from_slice(&Status::Unavailable.encode())),
			_ => Err(Error::Unavailable),
		}
	}
}

/// Cached native bridge client for an interpreted Wasm guest.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;

	/// Resolves the plugin once and reuses its numeric handle.
	#[derive(Debug, Clone, Copy)]
	pub struct BackgroundTask(leptoskit_bridge::guest::Client);

	impl BackgroundTask {
		/// Resolves a registered native plugin.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Queries whether registration is allowed.
		pub async fn status(self) -> Result<Status, Error> {
			Status::decode(&self.0.call(STATUS, &[])?.await?)
		}

		/// Registers a named task with a minimum interval hint.
		pub async fn register(self, request: &Register) -> Result<(), Error> {
			require_empty(&self.0.call(REGISTER, &request.encode()?)?.await?)
		}

		/// Unregisters a previously registered task.
		pub async fn unregister(self, name: &TaskName) -> Result<(), Error> {
			require_empty(&self.0.call(UNREGISTER, &name.encode()?)?.await?)
		}

		/// Completes the current OS invocation for `name`.
		pub async fn complete(self, name: &TaskName) -> Result<(), Error> {
			require_empty(&self.0.call(COMPLETE, &name.encode()?)?.await?)
		}
	}

	fn require_empty(bytes: &[u8]) -> Result<(), Error> {
		Decoder::new(bytes).finish()
	}
}

/// Browsers have no OS background-task scheduler in this MVP.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web {
	use super::*;

	/// Always Unavailable.
	#[must_use]
	pub const fn status() -> Status {
		Status::Unavailable
	}

	/// Always Unavailable.
	pub fn register(_request: &Register) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always Unavailable.
	pub fn unregister(_name: &TaskName) -> Result<(), Error> {
		Err(Error::Unavailable)
	}

	/// Always Unavailable.
	pub fn complete(_name: &TaskName) -> Result<(), Error> {
		Err(Error::Unavailable)
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_bridge::Plugin;

	use super::*;

	#[test]
	fn status_and_register_codecs_reject_bounds() {
		for (value, expected) in [
			(0, Status::Restricted),
			(1, Status::Available),
			(2, Status::Unavailable),
		] {
			assert_eq!(Status::decode(&u32::to_le_bytes(value)), Ok(expected));
		}
		for bytes in [&[][..], &[1], &[1, 0, 0], &[1, 0, 0, 0, 0], &[3, 0, 0, 0]] {
			assert_eq!(Status::decode(bytes), Err(Error::InvalidArgument));
		}

		let register = Register::new("sync", 900).unwrap();
		assert_eq!(register.name(), "sync");
		assert_eq!(register.minimum_interval_secs(), 900);
		assert_eq!(Register::decode(&register.encode().unwrap()), Ok(register));
		assert_eq!(Register::new("", 900), Err(Error::InvalidArgument));
		assert_eq!(Register::new("a\0b", 900), Err(Error::InvalidArgument));
		assert_eq!(
			Register::new("x".repeat(MAX_NAME_BYTES + 1), 900),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Register::new("sync", MIN_INTERVAL_SECS - 1),
			Err(Error::InvalidArgument)
		);
		assert_eq!(
			Register::new("sync", MAX_INTERVAL_SECS + 1),
			Err(Error::InvalidArgument)
		);
		let mut trailing = Register::new("sync", 900)
			.unwrap()
			.encode()
			.unwrap()
			.to_vec();
		trailing.push(0);
		assert_eq!(Register::decode(&trailing), Err(Error::InvalidArgument));

		let name = TaskName::new("sync").unwrap();
		assert_eq!(name.name(), "sync");
		assert_eq!(TaskName::decode(&name.encode().unwrap()), Ok(name));
		assert_eq!(TaskName::new(""), Err(Error::InvalidArgument));
	}

	#[test]
	fn validate_and_unsupported_keep_status_honest() {
		assert_eq!(validate(STATUS, &[]), Ok(()));
		assert_eq!(validate(STATUS, &[0]), Err(Error::InvalidArgument));
		assert_eq!(
			validate(
				REGISTER,
				&Register::new("sync", 900).unwrap().encode().unwrap()
			),
			Ok(())
		);
		assert_eq!(
			validate(
				UNREGISTER,
				&TaskName::new("sync").unwrap().encode().unwrap()
			),
			Ok(())
		);
		assert_eq!(
			validate(COMPLETE, &TaskName::new("sync").unwrap().encode().unwrap()),
			Ok(())
		);
		assert_eq!(validate(MethodId(9), &[]), Err(Error::Unavailable));
		assert_eq!(METHODS.len(), 4);
		assert_eq!(NAME, "background-task");

		let mut plugin = Unsupported;
		assert_eq!(plugin.name(), NAME);
		assert_eq!(plugin.methods(), METHODS);
		assert_eq!(
			Status::decode(&plugin.invoke(STATUS, Bytes::new()).unwrap()).unwrap(),
			Status::Unavailable
		);
		assert_eq!(
			plugin.invoke(
				REGISTER,
				Register::new("sync", 900).unwrap().encode().unwrap()
			),
			Err(Error::Unavailable)
		);
		assert_eq!(
			plugin.invoke(UNREGISTER, TaskName::new("sync").unwrap().encode().unwrap()),
			Err(Error::Unavailable)
		);
		assert_eq!(
			plugin.invoke(COMPLETE, TaskName::new("sync").unwrap().encode().unwrap()),
			Err(Error::Unavailable)
		);
	}
}
