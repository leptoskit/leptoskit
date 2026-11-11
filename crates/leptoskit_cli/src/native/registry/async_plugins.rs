//! Host configuration and Cargo feature ownership for asynchronous plugins.

use super::*;
const MAX_OPTIONS_BYTES: usize = 65_536;

pub(super) fn validate_async(plugin: &Plugin) -> Result<()> {
	anyhow::ensure!(
		!plugin.rust_async
			|| (plugin.rust_factory.is_some() && !plugin.rust_worker && !plugin.rust_main_thread),
		"rust-async requires rust-factory and cannot be combined with rust-worker or \
		 rust-main-thread"
	);
	Ok(())
}

pub(super) fn configure_plugin(
	plugin: &mut Plugin,
	config: &leptoskit_config::LeptoskitConfig,
	name: &str,
) -> Result<()> {
	plugin.experimental_rust_plugins = config.native.experimental_rust_plugins;

	if plugin.declares_async() {
		let options = config.native.plugin_options.get(name).with_context(|| {
			format!(
				"async plugin `{name}` requires [native.plugin-options.{name}] host configuration"
			)
		})?;

		// The factory reads these when the app starts; without them it fails
		// there, before the app shows anything.
		for option in &plugin.required_options {
			anyhow::ensure!(
				options.get(option).is_some_and(|value| !empty(value)),
				"plugin `{name}` needs `{option}` in [native.plugin-options.{name}]: without it, \
				 the app stops at launch when the plugin starts"
			);
		}

		let json = serde_json::to_string(options)
			.with_context(|| format!("serializing native.plugin-options.{name}"))?;
		anyhow::ensure!(
			json.len() <= MAX_OPTIONS_BYTES,
			"native.plugin-options.{name} exceeds 64 KiB"
		);
		plugin.options_json = Some(json);
		plugin.enabled_features = plugin
			.option_features
			.iter()
			.filter(|(option, _)| options.contains_key(option.as_str()))
			.map(|(_, feature)| feature.clone())
			.collect();
	} else {
		anyhow::ensure!(
			!config.native.plugin_options.contains_key(name),
			"plugin `{name}` does not accept native.plugin-options"
		);
		anyhow::ensure!(
			plugin.option_features.is_empty(),
			"plugin `{name}` declares option-features but takes no native.plugin-options"
		);
		anyhow::ensure!(
			plugin.required_options.is_empty(),
			"plugin `{name}` declares required-options but takes no native.plugin-options"
		);
	}

	Ok(())
}

/// A value that says nothing: an empty string, list, or table.
fn empty(value: &toml::Value) -> bool {
	match value {
		toml::Value::String(text) => text.is_empty(),
		toml::Value::Array(items) => items.is_empty(),
		toml::Value::Table(table) => table.is_empty(),
		_ => false,
	}
}

pub(super) fn host_features(
	app: &Package,
	node: &Node,
	packages: &BTreeMap<&String, &Package>,
	plugins: &[Plugin],
	target: Option<Target>,
) -> Result<Vec<String>> {
	// Without a target, every async declaration counts.
	let runs_async = |plugin: &Plugin| {
		match target {
			Some(target) => {
				plugin
					.implementation(target, HostCapabilities::of(target))
					.is_ok_and(|implementation| {
						implementation
							.rust()
							.is_some_and(|rust| rust.flavour == Flavour::Async)
					})
			}
			None => plugin.declares_async(),
		}
	};
	let mut features = Vec::new();

	for plugin in plugins.iter().filter(|plugin| runs_async(plugin)) {
		let key = cargo_dependency_key(app, &plugin.package_name, &plugin.crate_name)?;
		features.push(format!("{key}/async-host"));

		// Features the app's options for the plugin turn on, such as the
		// `xr` plugin's `depth-estimation` for `depth-model`.
		for feature in &plugin.enabled_features {
			anyhow::ensure!(
				packages
					.get(&plugin.package_id)
					.is_some_and(|package| package.features.contains_key(feature)),
				"plugin `{}` turns on feature `{feature}` for its options, but {} has no such \
				 feature",
				plugin.name,
				plugin.package_name
			);
			features.push(format!("{key}/{feature}"));
		}
	}

	if !features.is_empty() {
		features.push(bridge_feature(app, node, packages, "async-host")?);
	}

	Ok(features)
}

/// The app's direct `leptoskit_bridge_ffi` dependency's `feature`, named
/// under the key the app's manifest gives the dependency.
pub(super) fn bridge_feature(
	app: &Package,
	node: &Node,
	packages: &BTreeMap<&String, &Package>,
	feature: &str,
) -> Result<String> {
	let bridge_alias = node
		.deps
		.iter()
		.filter(|dependency| normal_dependency(dependency))
		.find(|dependency| {
			packages
				.get(&dependency.pkg)
				.is_some_and(|package| package.name == "leptoskit_bridge_ffi")
		})
		.context("bridge alias missing")?;
	Ok(format!(
		"{}/{feature}",
		cargo_dependency_key(app, "leptoskit_bridge_ffi", &bridge_alias.name)?
	))
}

fn cargo_dependency_key<'a>(
	app: &'a Package,
	package_name: &str,
	rust_alias: &str,
) -> Result<&'a str> {
	let dependency = app
		.dependencies
		.iter()
		.find(|dependency| {
			dependency.kind.is_none()
				&& dependency.name == package_name
				&& dependency
					.rename
					.as_ref()
					.is_none_or(|rename| rename.replace('-', "_") == rust_alias)
		})
		.with_context(|| format!("Cargo dependency key missing for `{rust_alias}`"))?;
	Ok(dependency.rename.as_deref().unwrap_or(&dependency.name))
}
