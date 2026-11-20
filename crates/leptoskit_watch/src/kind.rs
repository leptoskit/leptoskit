//! Which watch operating system a surface targets.

/// The watch operating system a [`crate::WatchApp`] or [`crate::WearableApp`]
/// is built for.
///
/// Wear OS (including Samsung Galaxy Watch / One UI Watch), watchOS, and
/// HarmonyOS Wearable (Huawei Watch) share the same surface and sensor
/// vocabulary while shipping through different hosts, manifests, and store
/// pipelines. Name them explicitly so apps never assume one phone tree covers
/// every watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchKind {
	/// Android Wear OS (Pixel Watch, Samsung Galaxy Watch, `TicWatch`, etc.).
	WearOs,
	/// Apple watchOS (`SwiftUI`, `WidgetKit` glances, complications,
	/// `HealthKit`).
	WatchOs,
	/// Huawei HarmonyOS / `OpenHarmony` wearable (`ArkUI` stage model,
	/// Sensor/Health kits).
	HarmonyOs,
}

impl WatchKind {
	/// Stable configuration / CLI name: `wearos`, `watchos`, or `harmonyos`.
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::WearOs => "wearos",
			Self::WatchOs => "watchos",
			Self::HarmonyOs => "harmonyos",
		}
	}

	/// Human-readable product name.
	#[must_use]
	pub const fn display_name(self) -> &'static str {
		match self {
			Self::WearOs => "Wear OS",
			Self::WatchOs => "watchOS",
			Self::HarmonyOs => "HarmonyOS Wearable",
		}
	}
}

impl std::fmt::Display for WatchKind {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.as_str())
	}
}

impl std::str::FromStr for WatchKind {
	type Err = crate::WatchError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		match value {
			"wearos" | "wear-os" | "wear_os" | "samsung-wear" => Ok(Self::WearOs),
			"watchos" | "watch-os" | "watch_os" => Ok(Self::WatchOs),
			"harmonyos" | "harmonyos-watch" | "harmony-watch" | "huawei-watch" => {
				Ok(Self::HarmonyOs)
			}
			other => Err(crate::WatchError::UnknownKind(other.to_owned())),
		}
	}
}
