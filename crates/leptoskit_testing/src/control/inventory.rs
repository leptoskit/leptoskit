//! What is attached right now: simulators, phones, emulators, AVDs, the
//! desktop, and the browsers whose drivers are installed.

use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;

use super::TargetKind;
use super::tools;

/// The inventory schema's major version.
pub const SCHEMA_VERSION: u8 = 1;

/// How long each listing tool may take.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(60);

/// One attachable target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
	/// The stable id a document's `target.device` uses: UDID, adb serial, AVD
	/// name, browser name, or `host`.
	pub id: String,
	/// The human name, from the platform.
	pub name: String,
	/// What kind of thing this is.
	pub kind: DeviceKind,
	/// The platform's own state word: `Booted`, `Shutdown`, `device`,
	/// `available`, …
	pub state: String,
}

/// The kinds of attachable targets. Every kind but AVDs and browsers maps
/// straight onto a document [`TargetKind`]; AVDs must be booted first and
/// browsers become web targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
	/// An Apple simulator.
	IosSimulator,
	/// A physically connected iPhone or iPad.
	IosDevice,
	/// An Android phone or tablet.
	AndroidDevice,
	/// A running Android emulator.
	AndroidEmulator,
	/// An installed, not-running Android virtual device.
	AndroidAvd,
	/// This desktop.
	Host,
	/// A browser whose driver is installed.
	WebBrowser,
}

impl std::fmt::Display for DeviceKind {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let name = match self {
			Self::IosSimulator => "ios_simulator",
			Self::IosDevice => "ios_device",
			Self::AndroidDevice => "android_device",
			Self::AndroidEmulator => "android_emulator",
			Self::AndroidAvd => "android_avd",
			Self::Host => "host",
			Self::WebBrowser => "web_browser",
		};

		formatter.write_str(name)
	}
}

impl DeviceKind {
	/// The target kind a document can point at this device with.
	pub fn target(self) -> Option<TargetKind> {
		match self {
			Self::IosSimulator => Some(TargetKind::IosSimulator),
			Self::IosDevice => Some(TargetKind::IosDevice),
			Self::AndroidDevice | Self::AndroidEmulator => Some(TargetKind::Android),
			// Boot the AVD first; then target its emulator-PORT serial.
			Self::AndroidAvd => None,
			Self::Host => {
				Some(match std::env::consts::OS {
					"macos" => TargetKind::Macos,
					"windows" => TargetKind::Windows,
					_ => TargetKind::Linux,
				})
			}
			Self::WebBrowser => Some(TargetKind::Web),
		}
	}
}

/// A device listing.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
	/// The inventory schema's major version.
	pub schema_version: u8,
	/// Every attachable target, ordered: Apple, Android, the desktop, then
	/// browsers.
	pub devices: Vec<Device>,
}

impl Inventory {
	/// An inventory with no devices, for callers whose target names its kind
	/// and so never needs to look one up.
	#[must_use]
	pub fn empty() -> Self {
		Self {
			schema_version: SCHEMA_VERSION,
			devices: Vec::new(),
		}
	}
}

/// Lists everything attached right now. Missing tooling contributes nothing
/// rather than failing; the run that needs it reports the gap.
///
/// The four listings run at once, so a tool that stalls until
/// [`LIST_TIMEOUT`] costs that long once rather than once per tool.
pub fn list() -> Inventory {
	let macos = cfg!(target_os = "macos");
	let (simulators, iphones, android, avds) = std::thread::scope(|scope| {
		let simulators = scope.spawn(|| {
			macos
				.then(|| tools::simulator_list(LIST_TIMEOUT))
				.flatten()
				.map(|output| parse_simulators(&output))
		});
		let iphones = scope.spawn(|| {
			macos
				.then(|| tools::iphone_list(LIST_TIMEOUT))
				.flatten()
				.map(|output| parse_iphones(&output))
		});
		let android =
			scope.spawn(|| tools::android_list(LIST_TIMEOUT).map(|output| parse_android(&output)));
		let avds = scope.spawn(|| tools::avd_list(LIST_TIMEOUT).map(|output| parse_avds(&output)));
		(
			simulators.join().ok().flatten(),
			iphones.join().ok().flatten(),
			android.join().ok().flatten(),
			avds.join().ok().flatten(),
		)
	});
	// Keep the documented order: Apple, Android, the desktop, then browsers.
	let mut devices: Vec<Device> = [simulators, iphones, android, avds]
		.into_iter()
		.flatten()
		.flatten()
		.collect();

	if DeviceKind::Host.target().is_some() {
		devices.push(Device {
			id: "host".to_owned(),
			name: host_name(),
			kind: DeviceKind::Host,
			state: "available".to_owned(),
		});
	}

	for (browser, driver) in [("chrome", "chromedriver"), ("firefox", "geckodriver")] {
		if tools::find_in_path(driver).is_some() {
			devices.push(Device {
				id: browser.to_owned(),
				name: driver.to_owned(),
				kind: DeviceKind::WebBrowser,
				state: "available".to_owned(),
			});
		}
	}

	if cfg!(target_os = "macos") {
		devices.push(Device {
			id: "safari".to_owned(),
			name: "safaridriver".to_owned(),
			kind: DeviceKind::WebBrowser,
			state: "available".to_owned(),
		});
	}

	Inventory {
		schema_version: SCHEMA_VERSION,
		devices,
	}
}

fn host_name() -> String {
	std::env::var("COMPUTERNAME")
		.or_else(|_| std::env::var("HOSTNAME"))
		.unwrap_or_else(|_| "desktop".to_owned())
}

/// Parses `xcrun simctl list devices --json`.
pub fn parse_simulators(output: &str) -> Vec<Device> {
	#[derive(Deserialize)]
	struct Simulator {
		udid: String,
		name: String,
		state: String,
		#[serde(rename = "isAvailable")]
		available: bool,
	}
	#[derive(Deserialize)]
	struct Listing {
		devices: std::collections::BTreeMap<String, Vec<Simulator>>,
	}
	let Ok(listing) = serde_json::from_str::<Listing>(output) else {
		return Vec::new();
	};
	listing
		.devices
		.into_iter()
		.filter(|(runtime, _)| runtime.starts_with("com.apple.CoreSimulator.SimRuntime.iOS-"))
		.flat_map(|(runtime, devices)| {
			let version = runtime
				.strip_prefix("com.apple.CoreSimulator.SimRuntime.iOS-")
				.unwrap_or_default()
				.replace('-', ".");
			devices
				.into_iter()
				.filter(|device| device.available)
				.map(move |device| {
					Device {
						id: device.udid,
						name: format!("{} (iOS {version})", device.name),
						kind: DeviceKind::IosSimulator,
						state: device.state,
					}
				})
		})
		.collect()
}

/// Parses `xcrun devicectl list devices --json-output`.
///
/// Devices sit under `result.devices`. The id is the hardware UDID when
/// devicectl has one, because `devicectl --device` and `appium:udid` both
/// take that, and the Core Device `identifier` only when a UDID is absent.
/// A watch, a television, a Mac, or a simulator in the same file is not an
/// iPhone. The newer `properties` dictionary is read when the older property
/// dictionaries are absent.
pub fn parse_iphones(output: &str) -> Vec<Device> {
	let Ok(document) = serde_json::from_str::<serde_json::Value>(output) else {
		return Vec::new();
	};
	let Some(devices) = listed_devices(&document) else {
		return Vec::new();
	};

	devices.iter().filter_map(iphone_from_devicectl).collect()
}

/// `result.devices` from devicectl, or a top-level `devices` array.
fn listed_devices(document: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
	document
		.pointer("/result/devices")
		.and_then(serde_json::Value::as_array)
		.or_else(|| {
			document
				.get("devices")
				.and_then(serde_json::Value::as_array)
		})
}

/// One physical iPhone or iPad, or nothing when this entry is another device.
fn iphone_from_devicectl(device: &serde_json::Value) -> Option<Device> {
	if !is_physical_ios(device) {
		return None;
	}

	let id = first_text(
		device,
		&[
			"/hardwareProperties/udid",
			"/properties/hardware/udid",
			"/udid",
			"/identifier",
		],
	)?;
	let name = first_text(
		device,
		&[
			"/deviceProperties/name",
			"/name",
			"/hardwareProperties/name",
			"/hardwareProperties/marketingName",
			"/properties/hardware/marketingName",
		],
	)
	.unwrap_or_else(|| "iPhone".to_owned());
	let state = first_text(
		device,
		&[
			"/state",
			"/connectionProperties/tunnelState",
			"/properties/connection/tunnelState",
			"/connectionProperties/pairingState",
			"/properties/connection/pairingState",
		],
	)
	.unwrap_or_else(|| "available".to_owned());

	Some(Device {
		id,
		name,
		kind: DeviceKind::IosDevice,
		state,
	})
}

/// Whether this entry is a physical iOS device.
///
/// A missing platform or reality stays included: older listings, and the
/// short fixtures, name an iPhone without those fields. A present platform
/// other than iOS, or a present reality other than physical, is something else.
fn is_physical_ios(device: &serde_json::Value) -> bool {
	let platform = first_text(
		device,
		&[
			"/hardwareProperties/platform",
			"/properties/hardware/platform",
		],
	);
	if platform.is_some_and(|platform| !platform.eq_ignore_ascii_case("ios")) {
		return false;
	}

	let reality = first_text(
		device,
		&[
			"/hardwareProperties/reality",
			"/properties/hardware/reality",
		],
	);
	reality.is_none_or(|reality| reality.eq_ignore_ascii_case("physical"))
}

/// The first non-empty string at any of `pointers`.
fn first_text(device: &serde_json::Value, pointers: &[&str]) -> Option<String> {
	pointers.iter().find_map(|pointer| {
		device
			.pointer(pointer)
			.and_then(serde_json::Value::as_str)
			.and_then(|text| {
				let text = text.trim();
				(!text.is_empty()).then(|| text.to_owned())
			})
	})
}

/// Parses `adb devices -l`.
pub fn parse_android(output: &str) -> Vec<Device> {
	output
		.lines()
		.filter(|line| !line.starts_with("List of devices") && !line.trim().is_empty())
		.filter_map(|line| {
			let mut fields = line.split_whitespace();
			let serial = fields.next()?;
			if serial == "*"
				|| serial.starts_with("adb")
				|| !serial.chars().all(|character| character.is_ascii_graphic())
			{
				return None;
			}
			let state = fields.next()?.to_owned();
			let model = fields
				.find_map(|field| field.strip_prefix("model:"))
				.map(str::to_owned);
			let emulator = serial.starts_with("emulator-");
			Some(Device {
				id: serial.to_owned(),
				name: model.unwrap_or_else(|| serial.to_owned()),
				kind: if emulator {
					DeviceKind::AndroidEmulator
				} else {
					DeviceKind::AndroidDevice
				},

				state,
			})
		})
		.collect()
}

/// Parses `emulator -list-avds`.
pub fn parse_avds(output: &str) -> Vec<Device> {
	output
		.lines()
		.map(str::trim)
		.filter(|name| !name.is_empty())
		.map(|name| {
			Device {
				id: name.to_owned(),
				name: name.to_owned(),
				kind: DeviceKind::AndroidAvd,
				state: "shutdown".to_owned(),
			}
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn simulators_parse_with_versions_and_skip_unavailable() {
		let devices = parse_simulators(
			r#"{"devices": {
				"com.apple.CoreSimulator.SimRuntime.iOS-18-2": [
					{"udid": "111", "name": "iPhone 16", "state": "Shutdown", "isAvailable": true},
					{"udid": "222", "name": "iPhone 16", "state": "Shutdown", "isAvailable": false}
				],
				"com.apple.CoreSimulator.SimRuntime.watchOS-11-1": [
					{"udid": "333", "name": "Watch", "state": "Booted", "isAvailable": true}
				]
			}}"#,
		);
		assert_eq!(devices.len(), 1);
		assert_eq!(devices[0].id, "111");
		assert_eq!(devices[0].name, "iPhone 16 (iOS 18.2)");
		assert_eq!(devices[0].kind, DeviceKind::IosSimulator);
		assert_eq!(devices[0].state, "Shutdown");
	}

	#[test]
	fn iphones_parse_either_identity_field() {
		let devices = parse_iphones(
			r#"{"devices": [
				{"name": "Ifiok's iPhone", "identifier": "aaa", "state": "connected"},
				{"udid": "bbb", "hardwareProperties": {"name": "iPad"}, "connectionProperties": {"tunnelState": "connected"}}
			]}"#,
		);
		assert_eq!(devices.len(), 2);
		assert_eq!(devices[0].id, "aaa");
		assert_eq!(devices[1].id, "bbb");
		assert_eq!(devices[1].name, "iPad");
		assert_eq!(devices[1].state, "connected");
		assert_eq!(devices[1].kind, DeviceKind::IosDevice);
	}

	#[test]
	fn devicectl_json_lists_physical_iphones_by_hardware_udid() {
		// Captured shape of `xcrun devicectl list devices --json-output`:
		// devices live under `result`, the hardware UDID is not the CoreDevice
		// identifier, and a watch or a simulator is not an iPhone target.
		let devices = parse_iphones(
			r#"{"info":{"commandType":"devicectl.list.devices","jsonVersion":2,"outcome":"success"},
			"result":{"devices":[
				{"identifier":"0097E500-40E1-4842-93ED-89C7D7E25655",
				 "deviceProperties":{"name":"xx's iPhone "},
				 "connectionProperties":{"pairingState":"paired","tunnelState":"unavailable"},
				 "hardwareProperties":{"deviceType":"iPhone","platform":"iOS","reality":"physical",
				  "marketingName":"iPhone 13 Pro Max","udid":"00008110-001C108C0132401E"}},
				{"identifier":"35C5DE54-1D3E-4144-8E9A-FA6C38B0B2F1",
				 "connectionProperties":{"pairingState":"paired","tunnelState":"unavailable"},
				 "hardwareProperties":{"deviceType":"appleWatch","platform":"watchOS","reality":"physical",
				  "marketingName":"Apple Watch Series 7","udid":"00008301-F0918DA12298C02E"}},
				{"identifier":"SIM-CORE","deviceProperties":{"name":"iPhone 16"},
				 "hardwareProperties":{"platform":"iOS","reality":"simulated","udid":"SIMULATOR-UDID"}},
				{"identifier":"6986451F-A2FF-48DE-A70E-45E06E1F1446",
				 "deviceProperties":{"name":"iPhone"},
				 "connectionProperties":{"pairingState":"paired","tunnelState":"connected"},
				 "hardwareProperties":{"platform":"iOS","reality":"physical","udid":"00008120-0014485601E3C01E"}}
			]}}"#,
		);
		assert_eq!(devices.len(), 2, "{devices:?}");
		assert_eq!(devices[0].id, "00008110-001C108C0132401E");
		assert_eq!(devices[0].name, "xx's iPhone");
		assert_eq!(devices[0].state, "unavailable");
		assert_eq!(devices[0].kind, DeviceKind::IosDevice);
		assert_eq!(devices[1].id, "00008120-0014485601E3C01E");
		assert_eq!(devices[1].name, "iPhone");
		assert_eq!(devices[1].state, "connected");
	}

	#[test]
	fn devicectl_properties_dictionary_lists_a_physical_iphone() {
		let devices = parse_iphones(
			r#"{"result":{"devices":[{
				"identifier":"CORE",
				"properties":{
					"hardware":{"platform":"iOS","reality":"physical","udid":"00008110-AAA","marketingName":"iPhone 16"},
					"connection":{"pairingState":"paired","tunnelState":"connected"},
					"state":{"bootState":"booted"}
				}
			}]}}"#,
		);
		assert_eq!(devices.len(), 1, "{devices:?}");
		assert_eq!(devices[0].id, "00008110-AAA");
		assert_eq!(devices[0].name, "iPhone 16");
		assert_eq!(devices[0].state, "connected");
		assert_eq!(devices[0].kind, DeviceKind::IosDevice);

		let named_by_model = parse_iphones(
			r#"{"result":{"devices":[
				{"hardwareProperties":{"platform":"iOS","marketingName":"iPhone 15","udid":"UDID-15"},
				 "connectionProperties":{"pairingState":"paired"}},
				{"identifier":"   "},
				{"identifier":"bare"}
			]}}"#,
		);
		assert_eq!(named_by_model.len(), 2, "{named_by_model:?}");
		assert_eq!(named_by_model[0].id, "UDID-15");
		assert_eq!(named_by_model[0].name, "iPhone 15");
		assert_eq!(named_by_model[0].state, "paired");
		assert_eq!(named_by_model[1].id, "bare");
		assert_eq!(named_by_model[1].name, "iPhone");
		assert_eq!(named_by_model[1].state, "available");
	}

	#[test]
	fn android_listing_separates_emulators_and_keeps_models() {
		let devices = parse_android(
			"List of devices attached\nFA826 device usb:1 product:r model:SM_A536B\nemulator-5554 \
			 device product:sdk device:emu\n\noffline-thing offline\n",
		);
		assert_eq!(devices.len(), 3);
		assert_eq!(devices[0].name, "SM_A536B");
		assert_eq!(devices[0].kind, DeviceKind::AndroidDevice);
		assert_eq!(devices[1].kind, DeviceKind::AndroidEmulator);
		assert_eq!(devices[1].name, "emulator-5554");
		assert_eq!(devices[2].state, "offline");
	}

	#[test]
	fn avds_parse_one_per_line() {
		let devices = parse_avds("Pixel_7_API_34\n Pixel_8\n\n");
		assert_eq!(devices.len(), 2);
		assert_eq!(devices[0].id, "Pixel_7_API_34");
		assert_eq!(devices[0].kind, DeviceKind::AndroidAvd);
	}

	#[test]
	fn every_kind_prints_its_json_name() {
		assert_eq!(DeviceKind::IosDevice.to_string(), "ios_device");
		assert_eq!(DeviceKind::AndroidDevice.to_string(), "android_device");
		assert_eq!(DeviceKind::AndroidEmulator.to_string(), "android_emulator");
		assert_eq!(DeviceKind::AndroidAvd.to_string(), "android_avd");
		assert_eq!(DeviceKind::Host.to_string(), "host");
		assert_eq!(DeviceKind::WebBrowser.to_string(), "web_browser");
	}

	#[test]
	fn garbage_listings_parse_to_nothing() {
		assert_eq!(parse_simulators("not json"), [] as [Device; 0]);
		assert_eq!(parse_iphones("nope"), [] as [Device; 0]);
		assert_eq!(parse_android("List of devices attached"), [] as [Device; 0]);
		assert_eq!(parse_avds("nonsense-not-a-real-avd").len(), 1);
	}

	#[test]
	fn kinds_map_to_target_kinds() {
		assert_eq!(DeviceKind::AndroidAvd.target(), None);
		assert_eq!(
			DeviceKind::AndroidEmulator.target(),
			Some(TargetKind::Android)
		);
		assert_eq!(DeviceKind::WebBrowser.target(), Some(TargetKind::Web));
	}
}
