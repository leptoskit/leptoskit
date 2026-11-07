//! <!-- {=p_applicationSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_application` as a direct dependency and select the
//! `application` plugin in `native.plugins`; supported generated hosts discover
//! it from Cargo metadata. Generated Linux hosts register it where it has a
//! Linux Rust implementation; generated Windows and terminal hosts currently
//! omit runtime plugins, so use explicit registration in a custom host there.
//! Guests enable the `guest` feature and reuse `guest::Application` rather than
//! resolving the plugin on every call. Apple reads `Bundle.main`, Android reads
//! the running package record, and a Rust desktop host supplies
//! its own packaging metadata; browsers have no equivalent installed-app
//! identity. Native snapshots also carry the install source, install and update
//! times, and on Android the signing certificate's SHA-256 digest.
//!
//! <!-- {/p_applicationSummary} -->
//!
//! # Wire format
//!
//! Snapshot version 2, little-endian: version u32 (2), then six
//! length-prefixed UTF-8 strings — application ID, name, version, build,
//! build signature, installer store — then the install and update times, each
//! a u32 presence word (0 or 1) followed by a u64 of milliseconds since the
//! Unix epoch that is zero when absent. Empty strings mean unknown. Strings
//! are bounded to 4096 bytes and the installer store to 256; a build
//! signature is exactly 64 uppercase hexadecimal digits. Version 1 replies
//! (the first four strings only) from older native hosts still decode.

use std::time::Duration;
use std::time::SystemTime;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::Plugin;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;

/// Runtime plugin name.
pub const NAME: &str = "application";
/// Empty request -> versioned [`Snapshot`].
pub const SNAPSHOT: MethodId = MethodId(0);
/// The snapshot version this crate writes.
pub const VERSION: u32 = 2;
/// Longest string, in bytes.
pub const MAX_TEXT_BYTES: usize = 4096;
/// Longest installer store, in bytes.
pub const MAX_INSTALLER_BYTES: usize = 256;
/// Length of a build signature: a SHA-256 digest in hexadecimal.
pub const SIGNATURE_LENGTH: usize = 64;

/// Installer stores the native implementations report. Android reports the
/// installing app's package name, so any other package can appear there.
pub mod installer {
	/// iOS: installed from the App Store.
	pub const APP_STORE: &str = "com.apple";
	/// iOS: installed from TestFlight. A heuristic; see
	/// [`super::Snapshot::installer_store`].
	pub const TESTFLIGHT: &str = "com.apple.testflight";
	/// iOS: installed with a provisioning profile — from Xcode, ad hoc or an
	/// enterprise distribution.
	pub const DEVELOPMENT: &str = "com.apple.development";
	/// iOS: running in the simulator.
	pub const SIMULATOR: &str = "com.apple.simulator";
	/// Android: Google Play.
	pub const GOOGLE_PLAY: &str = "com.android.vending";
	/// Android: Amazon Appstore.
	pub const AMAZON_APPSTORE: &str = "com.amazon.venezia";
	/// Android: Samsung Galaxy Store.
	pub const GALAXY_STORE: &str = "com.sec.android.app.samsungapps";
	/// Android: Huawei `AppGallery`.
	pub const APP_GALLERY: &str = "com.huawei.appmarket";
}

/// Metadata belonging to the installed host application, not its guest bundle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
	/// Bundle identifier on Apple or application package name on Android.
	pub id: Option<String>,
	/// Localized display name when available.
	pub name: Option<String>,
	/// User-facing release version.
	pub version: Option<String>,
	/// Build number; Android's version code is represented as a decimal string.
	pub build: Option<String>,
	/// Android only: the SHA-256 digest of the certificate that signed this
	/// APK, as 64 uppercase hexadecimal digits. After a signing-key rotation
	/// this is the newest certificate. `None` elsewhere.
	pub build_signature: Option<String>,
	/// Where the app was installed from.
	///
	/// - Android: the package that initiated the install, such as
	///   [`installer::GOOGLE_PLAY`] (`InstallSourceInfo.initiatingPackageName`
	///   on Android 11 and later, the installer package name before). `None`
	///   for apps installed with `adb` or by the system.
	/// - iOS: a heuristic from the App Store receipt's file name and the
	///   bundle's provisioning profile: [`installer::APP_STORE`],
	///   [`installer::TESTFLIGHT`], [`installer::DEVELOPMENT`] or
	///   [`installer::SIMULATOR`]. Apple does not document this signal and
	///   could change it.
	/// - macOS and Rust hosts: `None` unless the host supplies it.
	pub installer_store: Option<String>,
	/// When the app was first installed.
	///
	/// - Android: `PackageInfo.firstInstallTime`.
	/// - iOS, and sandboxed macOS apps: the creation date of the app's
	///   Documents directory, which survives updates and is reset by a
	///   reinstall. Unsandboxed macOS apps have no such container, so `None`.
	/// - Rust hosts: whatever the host supplies; see [`executable_times`].
	pub install_time: Option<SystemTime>,
	/// When the app was last installed or updated.
	///
	/// - Android: `PackageInfo.lastUpdateTime`.
	/// - Apple: the modification date of the app bundle, which an update
	///   replaces.
	/// - Rust hosts: whatever the host supplies; see [`executable_times`].
	pub update_time: Option<SystemTime>,
}

impl Snapshot {
	/// Encodes validated metadata. Empty known values, malformed signatures and
	/// times before the Unix epoch are invalid.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Encoder::with_capacity(160);
		output.u32(VERSION);

		for (field, max) in [
			(&self.id, MAX_TEXT_BYTES),
			(&self.name, MAX_TEXT_BYTES),
			(&self.version, MAX_TEXT_BYTES),
			(&self.build, MAX_TEXT_BYTES),
			(&self.build_signature, SIGNATURE_LENGTH),
			(&self.installer_store, MAX_INSTALLER_BYTES),
		] {
			if field.as_ref().is_some_and(String::is_empty) {
				return Err(Error::InvalidArgument);
			}

			let text = field.as_deref().unwrap_or_default();

			if text.len() > max {
				return Err(Error::TooLarge);
			}

			output.bytes(text.as_bytes())?;
		}

		if self
			.build_signature
			.as_deref()
			.is_some_and(|value| !valid_signature(value))
		{
			return Err(Error::InvalidArgument);
		}

		for time in [self.install_time, self.update_time] {
			let millis = time.map(millis).transpose()?;
			output.u32(u32::from(millis.is_some()));
			let millis = millis.unwrap_or(0);
			output.u32(millis as u32);
			output.u32((millis >> 32) as u32);
		}

		Ok(output.finish())
	}

	/// Validates version, bounded text, signatures, times, UTF-8 and complete
	/// message consumption. Accepts version 2 and the older version 1.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Decoder::new(bytes);

		let version = input.u32()?;

		if version != 1 && version != VERSION {
			return Err(Error::InvalidArgument);
		}

		let mut text = |max: usize| -> Result<Option<String>, Error> {
			let value = input.string()?;

			if value.len() > max {
				return Err(Error::TooLarge);
			}

			Ok((!value.is_empty()).then(|| value.to_owned()))
		};
		let mut result = Self {
			id: text(MAX_TEXT_BYTES)?,
			name: text(MAX_TEXT_BYTES)?,
			version: text(MAX_TEXT_BYTES)?,
			build: text(MAX_TEXT_BYTES)?,
			..Self::default()
		};

		if version == VERSION {
			result.build_signature = text(SIGNATURE_LENGTH)?;
			result.installer_store = text(MAX_INSTALLER_BYTES)?;

			if result
				.build_signature
				.as_deref()
				.is_some_and(|value| !valid_signature(value))
			{
				return Err(Error::InvalidArgument);
			}

			result.install_time = time(&mut input)?;
			result.update_time = time(&mut input)?;
		}

		input.finish()?;
		Ok(result)
	}
}

/// 64 uppercase hexadecimal digits.
fn valid_signature(value: &str) -> bool {
	value.len() == SIGNATURE_LENGTH
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
}

/// Milliseconds since the Unix epoch, truncated.
fn millis(time: SystemTime) -> Result<u64, Error> {
	let elapsed = time
		.duration_since(SystemTime::UNIX_EPOCH)
		.map_err(|_| Error::InvalidArgument)?;
	u64::try_from(elapsed.as_millis()).map_err(|_| Error::TooLarge)
}

/// A presence word and milliseconds, which must be zero when absent.
fn time(input: &mut Decoder<'_>) -> Result<Option<SystemTime>, Error> {
	let present = input.u32()?;
	let millis = u64::from(input.u32()?) | (u64::from(input.u32()?) << 32);

	match (present, millis) {
		(0, 0) => Ok(None),
		(1, millis) => {
			SystemTime::UNIX_EPOCH
				.checked_add(Duration::from_millis(millis))
				.map(Some)
				.ok_or(Error::InvalidArgument)
		}
		_ => Err(Error::InvalidArgument),
	}
}

/// The running executable's creation and modification times, for a Rust host
/// to put in [`Snapshot::install_time`] and [`Snapshot::update_time`], as
/// `package_info_plus` does on Windows and Linux.
///
/// They describe the executable file, not an installation record: copying,
/// unpacking or some package managers can reset or preserve them, and many
/// Linux file systems do not record a creation time.
#[cfg(not(target_arch = "wasm32"))]
pub fn executable_times() -> (Option<SystemTime>, Option<SystemTime>) {
	let Ok(metadata) = std::env::current_exe().and_then(std::fs::metadata) else {
		return (None, None);
	};

	(metadata.created().ok(), metadata.modified().ok())
}

/// Rust host adapter whose identity is supplied by the application's packaging
/// configuration. It never reads this library crate's Cargo package variables.
#[derive(Debug, Clone)]
pub struct Application {
	encoded: Bytes,
}
impl Application {
	/// Validates and caches the host's immutable installed metadata once.
	pub fn new(identity: &Snapshot) -> Result<Self, Error> {
		Ok(Self {
			encoded: identity.encode()?,
		})
	}
}
impl Plugin for Application {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		&[SNAPSHOT]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		if method != SNAPSHOT {
			return Err(Error::Unavailable);
		}

		if !payload.is_empty() {
			return Err(Error::InvalidArgument);
		}

		Ok(self.encoded.clone())
	}
}

/// Cached native guest client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// A resolved application plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Application(leptoskit_bridge::guest::Client);
	impl Application {
		/// Resolves the native application plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads the installed native host's identity.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}
	}
}

#[cfg(test)]
mod tests;
