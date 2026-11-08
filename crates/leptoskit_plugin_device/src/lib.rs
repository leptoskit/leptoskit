//! <!-- {=p_deviceSummary|trim|linePrefix:"//! ":true} -->
//!
//! Add `leptoskit_plugin_device` as a direct dependency and select the `device`
//! plugin in `native.plugins`; supported generated hosts discover it from Cargo
//! metadata. Generated Linux hosts register it where it has a Linux Rust
//! implementation; generated Windows and terminal hosts currently omit runtime
//! plugins, so use explicit registration in a custom host there. Guests enable
//! the `guest` feature and reuse `guest::Device` rather than resolving the
//! plugin on every call. Native guests, Rust desktop hosts, and browsers each
//! return a validated binary snapshot with the fields that platform can
//! actually provide. The snapshot never holds persistent identifiers or names a
//! person gave the device; those come only from the separate `identifiers`
//! call.
//!
//! <!-- {/p_deviceSummary} -->
//!
//! # Two calls, two privacy levels
//!
//! [`SNAPSHOT`] returns a [`Snapshot`]: hardware, operating system, memory,
//! storage and a [`Platform`] section with the details only one platform
//! reports (Android build fields, Windows release fields, Linux
//! `os-release`, browser client hints). It never contains persistent
//! identifiers or names a person gave the device.
//!
//! [`IDENTIFIERS`] returns [`Identifiers`]: the user-assigned device name, the
//! iOS vendor identifier and the desktop machine identifier. Call it only when
//! you need one of them, and say why in your privacy disclosures. Each field
//! is `None` where the platform withholds it.
//!
//! # Wire format
//!
//! All integers are little-endian. A tri-state is a u32 of 0 (unknown), 1 (no)
//! or 2 (yes). An optional number is a u32 presence word (0 or 1) followed by
//! the value, which must be zero when absent. Text is a u32 byte length and
//! UTF-8; empty means unknown. A list is a u32 count and that many non-empty
//! strings.
//!
//! Snapshot version 2: version, kind, simulator tri-state, total memory u64
//! (0 unknown), optional available memory u64, total disk u64 (0 unknown),
//! optional free disk u64, CPU count u32 (0 unknown), then eight strings —
//! manufacturer, brand, model, model identifier, OS name, OS version,
//! architecture, kernel version — and a u32 platform tag (0 none, 1 Apple,
//! 2 Android, 3 Windows, 4 Linux, 5 web) followed by that platform's section
//! as documented on each section type. Version 1 replies from older native
//! hosts still decode, with every newer field unknown.
//!
//! Identifiers version 1: version, then three strings — device name, vendor
//! identifier, machine identifier — each at most 256 bytes.

use std::num::NonZeroU32;
use std::num::NonZeroU64;
use std::time::SystemTime;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;

mod wire;
use wire::Reader;
use wire::Writer;

#[cfg(any(test, all(target_os = "linux", not(target_env = "ohos"))))]
mod os_release;

#[cfg(any(
	target_os = "macos",
	target_os = "windows",
	all(target_os = "linux", not(target_env = "ohos"))
))]
pub mod desktop;

#[cfg(all(target_arch = "wasm32", target_os = "unknown", not(feature = "guest")))]
pub mod web;

/// Runtime plugin name.
pub const NAME: &str = "device";
/// Empty input -> versioned [`Snapshot`] binary response.
pub const SNAPSHOT: MethodId = MethodId(0);
/// Empty input -> versioned [`Identifiers`] binary response.
pub const IDENTIFIERS: MethodId = MethodId(1);

/// The snapshot version this crate writes.
pub const SNAPSHOT_VERSION: u32 = 2;
/// The identifiers version this crate writes.
pub const IDENTIFIERS_VERSION: u32 = 1;

/// Longest general string, in bytes.
pub const MAX_TEXT_BYTES: usize = 4096;
/// Longest identifier, in bytes.
pub const MAX_IDENTIFIER_BYTES: usize = 256;
/// Most entries in an ABI, `ID_LIKE` or brand list.
pub const MAX_SHORT_LIST: usize = 16;
/// Longest entry in an ABI, `ID_LIKE` or brand list, in bytes.
pub const MAX_SHORT_ITEM_BYTES: usize = 128;
/// Most Android system features carried; the Android side sends the first
/// ones in sorted order.
pub const MAX_FEATURES: usize = 1024;
/// Longest Android system feature name, in bytes; longer names are skipped.
pub const MAX_FEATURE_BYTES: usize = 256;

/// The form factor explicitly reported by a platform.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind {
	/// Unavailable or not reliably distinguishable.
	#[default]
	Unknown = 0,
	/// Handheld phone.
	Phone = 1,
	/// Tablet.
	Tablet = 2,
	/// Television.
	Television = 3,
	/// Desktop or laptop computer.
	Desktop = 4,
	/// Watch.
	Watch = 5,
	/// Car interface.
	Car = 6,
	/// Spatial headset.
	Headset = 7,
}

impl Kind {
	fn from_wire(value: u32) -> Result<Self, Error> {
		Ok(match value {
			0 => Self::Unknown,
			1 => Self::Phone,
			2 => Self::Tablet,
			3 => Self::Television,
			4 => Self::Desktop,
			5 => Self::Watch,
			6 => Self::Car,
			7 => Self::Headset,
			_ => return Err(Error::InvalidArgument),
		})
	}
}

/// A system snapshot. Missing platform information stays `None`.
///
/// It never contains a persistent identifier or a user-assigned name; those
/// are in [`Identifiers`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Snapshot {
	/// Hardware manufacturer, if exposed by the OS.
	pub manufacturer: Option<String>,
	/// Consumer-facing hardware brand.
	pub brand: Option<String>,
	/// Platform model label. On iOS this is the product family, e.g. `iPhone`;
	/// on Android it is `Build.MODEL`.
	pub model: Option<String>,
	/// Machine model identifier: `iPhone15,2` on iOS (the simulated model in
	/// the simulator), `Mac14,2` on macOS. `None` elsewhere.
	pub model_identifier: Option<String>,
	/// Operating system name, not the browser name. On iOS this is
	/// `UIDevice.systemName` (`iOS` or `iPadOS`).
	pub os_name: Option<String>,
	/// Operating system version reported by the platform.
	pub os_version: Option<String>,
	/// Native plugin architecture or preferred Android ABI. Vocabulary is
	/// platform-specific.
	pub architecture: Option<String>,
	/// Kernel release, such as `24.0.0` on Apple platforms or
	/// `6.8.0-45-generic` on Linux and Android; the build number on Windows.
	pub kernel_version: Option<String>,
	/// Total memory exposed to the OS, in bytes; not the app's memory
	/// allowance.
	pub total_memory: Option<NonZeroU64>,
	/// Memory the OS reports as available right now, in bytes. It changes
	/// constantly; on iOS it counts only free pages.
	pub available_memory: Option<u64>,
	/// Size of the volume holding the app's data, in bytes.
	pub total_disk: Option<NonZeroU64>,
	/// Bytes the app can still write on that volume right now.
	pub free_disk: Option<u64>,
	/// Logical processors the OS reports as active.
	pub cpu_count: Option<NonZeroU32>,
	/// Platform-reported form factor.
	pub kind: Kind,
	/// Known simulator status; `None` when no reliable signal is available.
	/// Android emulator signatures can establish `Some(true)`, never
	/// `Some(false)`.
	pub simulator: Option<bool>,
	/// Details only one platform reports.
	pub platform: Platform,
}

/// Details only one platform reports.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Platform {
	/// No platform section.
	#[default]
	None,
	/// iOS, iPadOS, or macOS.
	Apple(Apple),
	/// Android.
	Android(Box<Android>),
	/// Windows.
	Windows(Windows),
	/// Linux, from `os-release`.
	Linux(Linux),
	/// A browser, from User-Agent Client Hints.
	Web(Web),
}

/// Apple details.
///
/// Wire: tri-states for iOS-app-on-Mac, Mac Catalyst and iOS-app-on-Vision,
/// CPU frequency u64 (0 unknown), then localized model and OS build strings.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Apple {
	/// An iPhone or iPad app running on an Apple silicon Mac
	/// (`ProcessInfo.isiOSAppOnMac`).
	pub ios_app_on_mac: Option<bool>,
	/// A Mac Catalyst app (`ProcessInfo.isMacCatalystApp`).
	pub mac_catalyst: Option<bool>,
	/// An iPhone or iPad app running on Apple Vision Pro
	/// (`ProcessInfo.isiOSAppOnVision`, iOS 26.1 and later).
	pub ios_app_on_vision: Option<bool>,
	/// Nominal CPU frequency in hertz (`hw.cpufrequency`). Apple silicon Macs
	/// and iOS devices do not report one.
	pub cpu_frequency: Option<NonZeroU64>,
	/// `UIDevice.localizedModel`, the model label in the user's language.
	pub localized_model: Option<String>,
	/// The OS build, such as `24A335` (`kern.osversion`).
	pub os_build: Option<String>,
}

/// Android `Build` and `ActivityManager` details.
///
/// Wire: SDK level u32 (0 unknown), optional preview SDK level u32, tri-states
/// for low-RAM device and low memory, optional build time in milliseconds
/// since the Unix epoch, sixteen strings in field order from `base_os` to
/// `build_type`, then lists of supported ABIs, 32-bit ABIs, 64-bit ABIs and
/// system features.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Android {
	/// `Build.VERSION.SDK_INT`.
	pub sdk_int: Option<NonZeroU32>,
	/// `Build.VERSION.PREVIEW_SDK_INT`; zero on release builds.
	pub preview_sdk_int: Option<u32>,
	/// `ActivityManager.isLowRamDevice()`: the device is a low-RAM class.
	pub low_ram_device: Option<bool>,
	/// `ActivityManager.MemoryInfo.lowMemory`: the system is short of memory
	/// right now.
	pub low_memory: Option<bool>,
	/// `Build.TIME`, when the OS build was made.
	pub build_time: Option<SystemTime>,
	/// `Build.VERSION.BASE_OS`.
	pub base_os: Option<String>,
	/// `Build.VERSION.CODENAME`, `REL` on release builds.
	pub codename: Option<String>,
	/// `Build.VERSION.INCREMENTAL`.
	pub incremental: Option<String>,
	/// `Build.VERSION.RELEASE`, the user-visible version.
	pub release: Option<String>,
	/// `Build.VERSION.SECURITY_PATCH`, such as `2026-10-05`.
	pub security_patch: Option<String>,
	/// `Build.BOARD`.
	pub board: Option<String>,
	/// `Build.BOOTLOADER`.
	pub bootloader: Option<String>,
	/// `Build.DEVICE`, the industrial design name.
	pub device: Option<String>,
	/// `Build.DISPLAY`, the build label shown to people.
	pub display: Option<String>,
	/// `Build.FINGERPRINT`, which identifies the OS build, not the device.
	pub fingerprint: Option<String>,
	/// `Build.HARDWARE`.
	pub hardware: Option<String>,
	/// `Build.HOST`, the machine that built the OS.
	pub host: Option<String>,
	/// `Build.ID`, a build label such as `AP2A.240805.005`; not a device
	/// identifier.
	pub id: Option<String>,
	/// `Build.PRODUCT`.
	pub product: Option<String>,
	/// `Build.TAGS`, such as `release-keys`.
	pub tags: Option<String>,
	/// `Build.TYPE`, such as `user` or `userdebug`.
	pub build_type: Option<String>,
	/// `Build.SUPPORTED_ABIS`, preferred first.
	pub supported_abis: Vec<String>,
	/// `Build.SUPPORTED_32_BIT_ABIS`.
	pub supported_32_bit_abis: Vec<String>,
	/// `Build.SUPPORTED_64_BIT_ABIS`.
	pub supported_64_bit_abis: Vec<String>,
	/// `PackageManager.getSystemAvailableFeatures()` names, sorted, at most
	/// [`MAX_FEATURES`].
	pub system_features: Vec<String>,
}

/// Windows release details from the registry values under
/// `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`. The user name,
/// registered owner and product key that `device_info_plus` also reads are
/// deliberately left out.
///
/// Wire: optional u32 major, minor, build number and update build revision,
/// then product name, display version, edition, release ID, build lab and
/// extended build lab strings.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Windows {
	/// `CurrentMajorVersionNumber`, such as 10.
	pub major_version: Option<u32>,
	/// `CurrentMinorVersionNumber`.
	pub minor_version: Option<u32>,
	/// `CurrentBuildNumber`, such as 26100.
	pub build_number: Option<u32>,
	/// `UBR`, the update build revision.
	pub update_build_revision: Option<u32>,
	/// `ProductName`, corrected to `Windows 11` on build 22000 and later.
	pub product_name: Option<String>,
	/// `DisplayVersion`, such as `24H2`.
	pub display_version: Option<String>,
	/// `EditionID`, such as `Professional`.
	pub edition_id: Option<String>,
	/// `ReleaseId`, the older release label.
	pub release_id: Option<String>,
	/// `BuildLab`.
	pub build_lab: Option<String>,
	/// `BuildLabEx`.
	pub build_lab_ex: Option<String>,
}

/// Linux distribution details from `/etc/os-release` (or
/// `/usr/lib/os-release`), falling back to `/etc/lsb-release`.
///
/// Wire: nine strings in field order from `name` to `variant_id`, then the
/// `ID_LIKE` list.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Linux {
	/// `NAME`.
	pub name: Option<String>,
	/// `VERSION`.
	pub version: Option<String>,
	/// `ID`, such as `ubuntu`.
	pub id: Option<String>,
	/// `VERSION_CODENAME`.
	pub version_codename: Option<String>,
	/// `VERSION_ID`.
	pub version_id: Option<String>,
	/// `PRETTY_NAME`.
	pub pretty_name: Option<String>,
	/// `BUILD_ID`.
	pub build_id: Option<String>,
	/// `VARIANT`.
	pub variant: Option<String>,
	/// `VARIANT_ID`.
	pub variant_id: Option<String>,
	/// `ID_LIKE`, space-separated in the file.
	pub id_like: Vec<String>,
}

/// A browser brand and its version, from client hints.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Brand {
	/// Brand name, such as `Chromium`. Browsers add a deliberately odd
	/// "GREASE" brand; it is passed through.
	pub brand: String,
	/// Significant version, or the full version from high-entropy hints.
	pub version: String,
}

/// Low-entropy browser details.
///
/// Wire: mobile tri-state, optional max touch points u32, then a u32 brand
/// count and that many brand/version string pairs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Web {
	/// `navigator.userAgentData.mobile`.
	pub mobile: Option<bool>,
	/// `navigator.maxTouchPoints`.
	pub max_touch_points: Option<u32>,
	/// `navigator.userAgentData.brands`.
	pub brands: Vec<Brand>,
}

/// Persistent identifiers and user-assigned names, returned only by the
/// separate [`IDENTIFIERS`] call. Every field is `None` where the platform
/// withholds it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Identifiers {
	/// The name a person gave the device.
	///
	/// - iOS: `UIDevice.name`. From iOS 16 it is the real name only when the
	///   app has Apple's
	///   `com.apple.developer.device-information.user-assigned-device-name`
	///   entitlement; otherwise iOS returns the model (`iPhone`) and this is
	///   `None`.
	/// - Android: `Settings.Global.DEVICE_NAME` (Android 7.1 and later).
	/// - macOS: the computer name from System Settings.
	/// - Windows and Linux: the host name.
	pub device_name: Option<String>,
	/// iOS `identifierForVendor`: stable for apps from one vendor on one
	/// device, reset when they are all removed. `None` elsewhere and before
	/// the device is first unlocked.
	pub vendor_id: Option<String>,
	/// A machine identifier that survives app reinstalls: macOS
	/// `IOPlatformUUID`, Linux `/etc/machine-id`, Windows
	/// `SQMClient\MachineId`. `None` on iOS and Android, which give apps no
	/// such value, and wherever a sandbox denies it.
	pub machine_id: Option<String>,
}

impl Snapshot {
	/// Encodes a version-2 snapshot, rejecting empty known strings and
	/// oversized values.
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Writer::new(256);
		output.u32(SNAPSHOT_VERSION);
		output.u32(self.kind as u32);
		output.tri(self.simulator);
		output.u64(self.total_memory.map_or(0, NonZeroU64::get));
		output.opt_u64(self.available_memory);
		output.u64(self.total_disk.map_or(0, NonZeroU64::get));
		output.opt_u64(self.free_disk);
		output.u32(self.cpu_count.map_or(0, NonZeroU32::get));

		for value in [
			&self.manufacturer,
			&self.brand,
			&self.model,
			&self.model_identifier,
			&self.os_name,
			&self.os_version,
			&self.architecture,
			&self.kernel_version,
		] {
			output.known(value.as_deref(), MAX_TEXT_BYTES)?;
		}

		match &self.platform {
			Platform::None => output.u32(0),
			Platform::Apple(apple) => {
				output.u32(1);
				apple.encode(&mut output)?;
			}
			Platform::Android(android) => {
				output.u32(2);
				android.encode(&mut output)?;
			}
			Platform::Windows(windows) => {
				output.u32(3);
				windows.encode(&mut output)?;
			}
			Platform::Linux(linux) => {
				output.u32(4);
				linux.encode(&mut output)?;
			}
			Platform::Web(web) => {
				output.u32(5);
				web.encode(&mut output)?;
			}
		}

		Ok(output.finish())
	}

	/// Validates the version, flags, UTF-8, lengths, and complete consumption.
	/// Accepts version 2 and the older version 1.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Reader::new(bytes);

		let result = match input.u32()? {
			1 => Self::decode_v1(&mut input)?,
			SNAPSHOT_VERSION => Self::decode_v2(&mut input)?,
			_ => return Err(Error::InvalidArgument),
		};

		input.finish()?;
		Ok(result)
	}

	/// Version 1: kind, simulator, memory, then six strings.
	fn decode_v1(input: &mut Reader<'_>) -> Result<Self, Error> {
		let kind = Kind::from_wire(input.u32()?)?;
		let simulator = input.tri()?;
		let total_memory = NonZeroU64::new(input.u64()?);

		Ok(Self {
			manufacturer: input.text(MAX_TEXT_BYTES)?,
			brand: input.text(MAX_TEXT_BYTES)?,
			model: input.text(MAX_TEXT_BYTES)?,
			os_name: input.text(MAX_TEXT_BYTES)?,
			os_version: input.text(MAX_TEXT_BYTES)?,
			architecture: input.text(MAX_TEXT_BYTES)?,
			total_memory,
			kind,
			simulator,
			..Self::default()
		})
	}

	fn decode_v2(input: &mut Reader<'_>) -> Result<Self, Error> {
		let kind = Kind::from_wire(input.u32()?)?;
		let simulator = input.tri()?;
		let total_memory = NonZeroU64::new(input.u64()?);
		let available_memory = input.opt_u64()?;
		let total_disk = NonZeroU64::new(input.u64()?);
		let free_disk = input.opt_u64()?;
		let cpu_count = NonZeroU32::new(input.u32()?);

		Ok(Self {
			manufacturer: input.text(MAX_TEXT_BYTES)?,
			brand: input.text(MAX_TEXT_BYTES)?,
			model: input.text(MAX_TEXT_BYTES)?,
			model_identifier: input.text(MAX_TEXT_BYTES)?,
			os_name: input.text(MAX_TEXT_BYTES)?,
			os_version: input.text(MAX_TEXT_BYTES)?,
			architecture: input.text(MAX_TEXT_BYTES)?,
			kernel_version: input.text(MAX_TEXT_BYTES)?,
			total_memory,
			available_memory,
			total_disk,
			free_disk,
			cpu_count,
			kind,
			simulator,
			platform: match input.u32()? {
				0 => Platform::None,
				1 => Platform::Apple(Apple::decode(input)?),
				2 => Platform::Android(Box::new(Android::decode(input)?)),
				3 => Platform::Windows(Windows::decode(input)?),
				4 => Platform::Linux(Linux::decode(input)?),
				5 => Platform::Web(Web::decode(input)?),
				_ => return Err(Error::InvalidArgument),
			},
		})
	}
}

impl Apple {
	fn encode(&self, output: &mut Writer) -> Result<(), Error> {
		output.tri(self.ios_app_on_mac);
		output.tri(self.mac_catalyst);
		output.tri(self.ios_app_on_vision);
		output.u64(self.cpu_frequency.map_or(0, NonZeroU64::get));
		output.known(self.localized_model.as_deref(), MAX_TEXT_BYTES)?;
		output.known(self.os_build.as_deref(), MAX_TEXT_BYTES)
	}

	fn decode(input: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			ios_app_on_mac: input.tri()?,
			mac_catalyst: input.tri()?,
			ios_app_on_vision: input.tri()?,
			cpu_frequency: NonZeroU64::new(input.u64()?),
			localized_model: input.text(MAX_TEXT_BYTES)?,
			os_build: input.text(MAX_TEXT_BYTES)?,
		})
	}
}

impl Android {
	fn strings(&self) -> [&Option<String>; 16] {
		[
			&self.base_os,
			&self.codename,
			&self.incremental,
			&self.release,
			&self.security_patch,
			&self.board,
			&self.bootloader,
			&self.device,
			&self.display,
			&self.fingerprint,
			&self.hardware,
			&self.host,
			&self.id,
			&self.product,
			&self.tags,
			&self.build_type,
		]
	}

	fn encode(&self, output: &mut Writer) -> Result<(), Error> {
		output.u32(self.sdk_int.map_or(0, NonZeroU32::get));
		output.opt_u32(self.preview_sdk_int);
		output.tri(self.low_ram_device);
		output.tri(self.low_memory);
		output.opt_time(self.build_time)?;

		for value in self.strings() {
			output.known(value.as_deref(), MAX_TEXT_BYTES)?;
		}

		for abis in [
			&self.supported_abis,
			&self.supported_32_bit_abis,
			&self.supported_64_bit_abis,
		] {
			output.list(abis, MAX_SHORT_LIST, MAX_SHORT_ITEM_BYTES)?;
		}

		output.list(&self.system_features, MAX_FEATURES, MAX_FEATURE_BYTES)
	}

	fn decode(input: &mut Reader<'_>) -> Result<Self, Error> {
		let sdk_int = NonZeroU32::new(input.u32()?);
		let preview_sdk_int = input.opt_u32()?;
		let low_ram_device = input.tri()?;
		let low_memory = input.tri()?;
		let build_time = input.opt_time()?;
		let mut text = || input.text(MAX_TEXT_BYTES);

		let mut result = Self {
			sdk_int,
			preview_sdk_int,
			low_ram_device,
			low_memory,
			build_time,
			base_os: text()?,
			codename: text()?,
			incremental: text()?,
			release: text()?,
			security_patch: text()?,
			board: text()?,
			bootloader: text()?,
			device: text()?,
			display: text()?,
			fingerprint: text()?,
			hardware: text()?,
			host: text()?,
			id: text()?,
			product: text()?,
			tags: text()?,
			build_type: text()?,
			..Self::default()
		};
		result.supported_abis = input.list(MAX_SHORT_LIST, MAX_SHORT_ITEM_BYTES)?;
		result.supported_32_bit_abis = input.list(MAX_SHORT_LIST, MAX_SHORT_ITEM_BYTES)?;
		result.supported_64_bit_abis = input.list(MAX_SHORT_LIST, MAX_SHORT_ITEM_BYTES)?;
		result.system_features = input.list(MAX_FEATURES, MAX_FEATURE_BYTES)?;
		Ok(result)
	}
}

impl Windows {
	fn encode(&self, output: &mut Writer) -> Result<(), Error> {
		output.opt_u32(self.major_version);
		output.opt_u32(self.minor_version);
		output.opt_u32(self.build_number);
		output.opt_u32(self.update_build_revision);

		for value in [
			&self.product_name,
			&self.display_version,
			&self.edition_id,
			&self.release_id,
			&self.build_lab,
			&self.build_lab_ex,
		] {
			output.known(value.as_deref(), MAX_TEXT_BYTES)?;
		}

		Ok(())
	}

	fn decode(input: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			major_version: input.opt_u32()?,
			minor_version: input.opt_u32()?,
			build_number: input.opt_u32()?,
			update_build_revision: input.opt_u32()?,
			product_name: input.text(MAX_TEXT_BYTES)?,
			display_version: input.text(MAX_TEXT_BYTES)?,
			edition_id: input.text(MAX_TEXT_BYTES)?,
			release_id: input.text(MAX_TEXT_BYTES)?,
			build_lab: input.text(MAX_TEXT_BYTES)?,
			build_lab_ex: input.text(MAX_TEXT_BYTES)?,
		})
	}
}

impl Linux {
	fn encode(&self, output: &mut Writer) -> Result<(), Error> {
		for value in [
			&self.name,
			&self.version,
			&self.id,
			&self.version_codename,
			&self.version_id,
			&self.pretty_name,
			&self.build_id,
			&self.variant,
			&self.variant_id,
		] {
			output.known(value.as_deref(), MAX_TEXT_BYTES)?;
		}

		output.list(&self.id_like, MAX_SHORT_LIST, MAX_SHORT_ITEM_BYTES)
	}

	fn decode(input: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			name: input.text(MAX_TEXT_BYTES)?,
			version: input.text(MAX_TEXT_BYTES)?,
			id: input.text(MAX_TEXT_BYTES)?,
			version_codename: input.text(MAX_TEXT_BYTES)?,
			version_id: input.text(MAX_TEXT_BYTES)?,
			pretty_name: input.text(MAX_TEXT_BYTES)?,
			build_id: input.text(MAX_TEXT_BYTES)?,
			variant: input.text(MAX_TEXT_BYTES)?,
			variant_id: input.text(MAX_TEXT_BYTES)?,
			id_like: input.list(MAX_SHORT_LIST, MAX_SHORT_ITEM_BYTES)?,
		})
	}
}

impl Web {
	fn encode(&self, output: &mut Writer) -> Result<(), Error> {
		output.tri(self.mobile);
		output.opt_u32(self.max_touch_points);
		encode_brands(output, &self.brands)
	}

	fn decode(input: &mut Reader<'_>) -> Result<Self, Error> {
		Ok(Self {
			mobile: input.tri()?,
			max_touch_points: input.opt_u32()?,
			brands: decode_brands(input)?,
		})
	}
}

/// A count, then brand/version pairs. A brand must be named; its version may
/// be unknown.
fn encode_brands(output: &mut Writer, brands: &[Brand]) -> Result<(), Error> {
	if brands.len() > MAX_SHORT_LIST {
		return Err(Error::TooLarge);
	}

	output.u32(brands.len() as u32);

	for brand in brands {
		if brand.brand.is_empty() {
			return Err(Error::InvalidArgument);
		}

		output.text(Some(&brand.brand), MAX_SHORT_ITEM_BYTES)?;
		output.text(Some(&brand.version), MAX_SHORT_ITEM_BYTES)?;
	}

	Ok(())
}

fn decode_brands(input: &mut Reader<'_>) -> Result<Vec<Brand>, Error> {
	let count = input.u32()? as usize;

	if count > MAX_SHORT_LIST {
		return Err(Error::TooLarge);
	}

	let mut brands = Vec::with_capacity(count);

	for _ in 0..count {
		brands.push(Brand {
			brand: input
				.text(MAX_SHORT_ITEM_BYTES)?
				.ok_or(Error::InvalidArgument)?,
			version: input.text(MAX_SHORT_ITEM_BYTES)?.unwrap_or_default(),
		});
	}

	Ok(brands)
}

impl Identifiers {
	/// Encodes version-1 identifiers, rejecting empty known strings and
	/// values over [`MAX_IDENTIFIER_BYTES`].
	pub fn encode(&self) -> Result<Bytes, Error> {
		let mut output = Writer::new(64);
		output.u32(IDENTIFIERS_VERSION);

		for value in [&self.device_name, &self.vendor_id, &self.machine_id] {
			output.known(value.as_deref(), MAX_IDENTIFIER_BYTES)?;
		}

		Ok(output.finish())
	}

	/// Validates the version, UTF-8, lengths, and complete consumption.
	pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
		let mut input = Reader::new(bytes);

		if input.u32()? != IDENTIFIERS_VERSION {
			return Err(Error::InvalidArgument);
		}

		let result = Self {
			device_name: input.text(MAX_IDENTIFIER_BYTES)?,
			vendor_id: input.text(MAX_IDENTIFIER_BYTES)?,
			machine_id: input.text(MAX_IDENTIFIER_BYTES)?,
		};
		input.finish()?;
		Ok(result)
	}
}

/// Cached native guest bridge client.
#[cfg(all(target_arch = "wasm32", feature = "guest"))]
pub mod guest {
	use super::*;
	/// A resolved device plugin.
	#[derive(Debug, Clone, Copy)]
	pub struct Device(leptoskit_bridge::guest::Client);
	impl Device {
		/// Resolves the installed native plugin once.
		pub fn new() -> Result<Self, Error> {
			leptoskit_bridge::guest::Client::new(NAME).map(Self)
		}

		/// Reads and validates the native snapshot. It holds no persistent
		/// identifiers or user-assigned names.
		pub async fn snapshot(self) -> Result<Snapshot, Error> {
			Snapshot::decode(&self.0.call(SNAPSHOT, &[])?.await?)
		}

		/// Reads the device name, vendor identifier and machine identifier.
		/// Fields the platform withholds are `None`. A native host older than
		/// this method returns `Unavailable`.
		pub async fn identifiers(self) -> Result<Identifiers, Error> {
			Identifiers::decode(&self.0.call(IDENTIFIERS, &[])?.await?)
		}
	}
}

#[cfg(test)]
mod tests;
