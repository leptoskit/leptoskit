//! In-tree platform extension crates registered with the CLI.

use leptoskit_platform::PlatformExtension;
use leptoskit_platform_harmonyos::HarmonyOs;

/// Builtin platform extensions shipped in this workspace.
#[must_use]
pub fn builtin() -> [&'static dyn PlatformExtension; 1] {
	[&HarmonyOs]
}

/// Looks up a builtin extension by destination id.
#[must_use]
pub fn find(id: &str) -> Option<&'static dyn PlatformExtension> {
	match id {
		HarmonyOs::ID => Some(&HarmonyOs),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn registers_harmonyos() {
		let extensions = builtin();
		assert_eq!(extensions.len(), 1);
		assert_eq!(find("harmonyos").unwrap().display_name(), "HarmonyOS");
		assert!(find("oculus").is_none());
		assert!(find("quest").is_none());
		assert!(find("web").is_none());
		for extension in extensions {
			extension.manifest().validate().unwrap();
		}
	}
}
