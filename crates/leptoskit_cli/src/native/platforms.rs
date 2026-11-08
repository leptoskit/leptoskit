//! In-tree platform extension crates registered with the CLI.

use leptoskit_platform::PlatformExtension;
use leptoskit_platform_harmonyos::HarmonyOs;
use leptoskit_platform_oculus::Oculus;

/// Builtin platform extensions shipped in this workspace.
#[must_use]
pub fn builtin() -> [&'static dyn PlatformExtension; 2] {
	[&HarmonyOs, &Oculus]
}

/// Looks up a builtin extension by destination id.
#[must_use]
pub fn find(id: &str) -> Option<&'static dyn PlatformExtension> {
	match id {
		HarmonyOs::ID => Some(&HarmonyOs),
		Oculus::ID => Some(&Oculus),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn registers_harmonyos_and_oculus() {
		let extensions = builtin();
		assert_eq!(extensions.len(), 2);
		assert_eq!(find("harmonyos").unwrap().display_name(), "HarmonyOS");
		assert_eq!(find("oculus").unwrap().display_name(), "Meta Quest");
		assert!(find("web").is_none());
		for extension in extensions {
			extension.manifest().validate().unwrap();
		}
	}
}
