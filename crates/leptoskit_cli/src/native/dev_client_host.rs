//! The phone dev client links `dev-client` from the host crate the CLI
//! resolves.

#[cfg(test)]
mod tests {
	use std::path::Path;

	use leptoskit_config::LeptoskitConfig;

	use super::super::Target;
	use super::super::registry;

	#[test]
	fn android_host_resolves_dev_client() {
		let root = Path::new(env!("CARGO_MANIFEST_DIR"))
			.join("../../examples/dev_client")
			.canonicalize()
			.expect("dev client directory");
		let config = LeptoskitConfig::load_from(&root).expect("dev client config");
		let registry = registry::resolve(&root, &config, Target::AndroidX86_64)
			.expect("android host resolves dev-client");
		assert!(
			registry
				.plugins
				.iter()
				.any(|plugin| plugin.name == "dev-client"),
			"dev_client_android must depend on leptoskit_plugin_dev_client"
		);
		let ios = std::fs::read_to_string(root.join("ios/Cargo.toml")).expect("ios host manifest");
		assert!(
			ios.contains("leptoskit_plugin_dev_client"),
			"dev_client_ios must depend on leptoskit_plugin_dev_client"
		);
	}
}
