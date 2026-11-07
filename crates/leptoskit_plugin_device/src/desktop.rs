//! Desktop information through OS APIs. Only memory, the CPU list and the
//! data volume are read; no process enumeration or CPU sampling runs.
//! Hardware vendor and virtualization status stay unknown.

use super::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

/// Reads current system memory, storage and OS metadata without persistent
/// identifiers or user-assigned names.
pub fn snapshot() -> Snapshot {
	let mut system = sysinfo::System::new();
	system.refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());
	system.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
	let (total_disk, free_disk) = disk();

	Snapshot {
		os_name: os_name(),
		os_version: sysinfo::System::os_version().filter(|value| !value.is_empty()),
		architecture: Some(sysinfo::System::cpu_arch()).filter(|value| !value.is_empty()),
		kernel_version: sysinfo::System::kernel_version().filter(|value| !value.is_empty()),
		model_identifier: model_identifier(),
		total_memory: NonZeroU64::new(system.total_memory()),
		available_memory: (system.total_memory() > 0).then(|| system.available_memory()),
		total_disk,
		free_disk,
		cpu_count: u32::try_from(system.cpus().len())
			.ok()
			.and_then(NonZeroU32::new),
		kind: Kind::Desktop,
		platform: platform(),
		..Snapshot::default()
	}
}

/// Reads the host name and machine identifier. Call it only when you need
/// them; see [`Identifiers`].
pub fn identifiers() -> Identifiers {
	Identifiers {
		device_name: sysinfo::System::host_name()
			.filter(|value| !value.is_empty() && value.len() <= MAX_IDENTIFIER_BYTES),
		vendor_id: None,
		machine_id: machine_id(),
	}
}

#[cfg(target_os = "macos")]
#[allow(clippy::unnecessary_wraps)]
fn os_name() -> Option<String> {
	Some("macOS".to_owned())
}

#[cfg(not(target_os = "macos"))]
fn os_name() -> Option<String> {
	sysinfo::System::name().filter(|value| !value.is_empty())
}

#[cfg(target_os = "macos")]
fn model_identifier() -> Option<String> {
	macos::string("hw.model")
}

#[cfg(not(target_os = "macos"))]
fn model_identifier() -> Option<String> {
	None
}

/// Size and app-writable space of the volume holding the home directory.
fn disk() -> (Option<NonZeroU64>, Option<u64>) {
	let path = std::env::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));

	#[cfg(unix)]
	let space = unix::disk(&path);
	#[cfg(target_os = "windows")]
	let space = windows::disk(&path);

	match space {
		Some((total, free)) => (NonZeroU64::new(total), Some(free)),
		None => (None, None),
	}
}

#[cfg(target_os = "macos")]
fn platform() -> Platform {
	Platform::Apple(Apple {
		ios_app_on_mac: Some(false),
		mac_catalyst: Some(false),
		ios_app_on_vision: Some(false),
		cpu_frequency: macos::number("hw.cpufrequency").and_then(NonZeroU64::new),
		localized_model: None,
		os_build: macos::string("kern.osversion"),
	})
}

#[cfg(target_os = "windows")]
fn platform() -> Platform {
	Platform::Windows(windows::details())
}

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
fn platform() -> Platform {
	let os_release =
		unix::read_text("/etc/os-release").or_else(|| unix::read_text("/usr/lib/os-release"));
	let lsb_release = unix::read_text("/etc/lsb-release");
	Platform::Linux(os_release::linux(
		os_release.as_deref(),
		lsb_release.as_deref(),
	))
}

#[cfg(target_os = "macos")]
fn machine_id() -> Option<String> {
	macos::host_uuid()
}

#[cfg(target_os = "windows")]
fn machine_id() -> Option<String> {
	windows::machine_id()
}

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
fn machine_id() -> Option<String> {
	unix::read_text("/etc/machine-id")
		.or_else(|| unix::read_text("/var/lib/dbus/machine-id"))
		.and_then(|value| machine_id_value(&value))
}

/// A `machine-id(5)` value: 32 lowercase hexadecimal digits, not all zero.
#[cfg(any(test, all(target_os = "linux", not(target_env = "ohos"))))]
pub(crate) fn machine_id_value(text: &str) -> Option<String> {
	let value = text.trim();
	(value.len() == 32
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
		&& value.bytes().any(|byte| byte != b'0'))
	.then(|| value.to_owned())
}

/// `ProductName` still says `Windows 10` on Windows 11; build 22000 and later
/// is Windows 11.
#[cfg(any(test, target_os = "windows"))]
pub(crate) fn windows_product_name(name: String, build: Option<u32>) -> String {
	if build.is_some_and(|build| build >= 22_000) {
		name.replacen("Windows 10", "Windows 11", 1)
	} else {
		name
	}
}

/// Rust desktop bridge adapter.
#[derive(Debug, Default)]
pub struct Device;

impl leptoskit_bridge::Plugin for Device {
	fn name(&self) -> &'static str {
		NAME
	}

	fn methods(&self) -> &'static [MethodId] {
		&[SNAPSHOT, IDENTIFIERS]
	}

	fn invoke(&mut self, method: MethodId, payload: Bytes) -> Result<Bytes, Error> {
		if method != SNAPSHOT && method != IDENTIFIERS {
			return Err(Error::Unavailable);
		}

		if !payload.is_empty() {
			return Err(Error::InvalidArgument);
		}

		if method == SNAPSHOT {
			snapshot().encode()
		} else {
			identifiers().encode()
		}
	}
}
