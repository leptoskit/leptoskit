// Versioned capability descriptors from the real registry resolution.

use leptoskit_codepush::capability::CAP_HOST_PLUGIN_NATIVE_ANDROID;
use leptoskit_codepush::capability::CAP_HOST_PLUGIN_RUST_ASYNC;
use leptoskit_codepush::capability::CapabilityState;
use leptoskit_config::MissingPlugins;

use super::*;

fn echo_config() -> LeptoskitConfig {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];
	config
}

fn async_metadata() -> Metadata {
	let mut value = metadata("Plugin");
	value.packages[2].metadata = json!({"leptoskit": {"runtime": {
		"name": "echo",
		"rust-factory": "from_config",
		"rust-async": true
	}}});
	value
}

fn registry(config: &LeptoskitConfig, target: Target) -> Registry {
	let mut registry = resolve_metadata(metadata("Plugin"), config).expect("registry");
	registry
		.select(target, config.native.missing_plugins)
		.expect("select");
	registry
}

#[test]
fn required_missing_capability_fails_but_an_optional_omission_only_warns() {
	let mut config = echo_config();
	config.native.required_plugins = vec!["echo".into()];
	let registry = registry(&config, Target::Linux);

	let error = enforce_required(&config, Target::Linux, &registry.plugins, &registry.missing)
		.expect_err("Linux cannot run plugins");
	assert!(error.to_string().contains("plugin.echo"), "{error}");

	config.native.required_plugins.clear();
	config.native.optional_plugins = vec!["echo".into()];
	let warnings = optional_warnings(&config, Target::Linux, &registry.plugins, &registry.missing);
	assert_eq!(warnings.len(), 1, "{warnings:?}");
	assert!(
		warnings[0].contains("optional runtime plugin `echo`") && warnings[0].contains("Linux"),
		"{warnings:?}"
	);
}

#[test]
fn an_optional_plugin_never_turns_missing_plugins_error_into_a_failure() {
	let mut config = echo_config();
	config.native.missing_plugins = MissingPlugins::Error;
	config.native.optional_plugins = vec!["echo".into()];
	let registry = resolve_metadata(metadata("Plugin"), &config).expect("registry");

	let (kept, missing) = super::super::implementation::select_for_target(
		registry.plugins,
		Target::Linux,
		MissingPlugins::Error,
		&config.native.optional_plugins,
	)
	.expect("optional omission is not fatal");
	assert!(kept.is_empty());
	assert_eq!(missing.len(), 1);

	assert!(
		super::super::implementation::select_for_target(
			resolve_metadata(metadata("Plugin"), &echo_config())
				.expect("registry")
				.plugins,
			Target::Linux,
			MissingPlugins::Error,
			&[],
		)
		.is_err(),
		"an unlisted grant still fails"
	);
}

#[test]
fn host_flavour_differences_are_part_of_the_contract() {
	let config = echo_config();
	let registry = resolve_metadata(async_metadata(), &config).expect("registry");
	let ios = descriptor(&config, Target::Ios, &registry.plugins, &registry.missing);
	let harmony = descriptor(
		&config,
		Target::Harmonyos,
		&registry.plugins,
		&registry.missing,
	);

	assert_eq!(
		ios.state_of("plugin.echo"),
		Some((&CapabilityState::Available, None))
	);
	assert_eq!(
		harmony.state_of("plugin.echo").map(|(state, _)| *state),
		Some(CapabilityState::Unsupported)
	);
	assert_ne!(ios.digest(), harmony.digest());

	let mut required = config;
	required.native.required_capabilities = vec![CAP_HOST_PLUGIN_RUST_ASYNC.into()];
	assert!(
		enforce_required(&required, Target::Ios, &registry.plugins, &registry.missing,).is_ok()
	);
	assert!(
		enforce_required(
			&required,
			Target::Harmonyos,
			&registry.plugins,
			&registry.missing,
		)
		.is_err()
	);
}

#[test]
fn experimental_implementations_need_the_existing_opt_in() {
	let mut config = echo_config();
	let mut value = metadata("Plugin");
	value.packages[2].metadata = json!({"leptoskit": {"runtime": {
		"name": "echo",
		"rust-type": "Echo",
		"experimental": true
	}}});
	let metadata = value;

	// Without a target in `resolve_metadata`, the plugin is still selected;
	// resolve it for the destination to see the opt-in decision.
	let mut selected = resolve_metadata(metadata.clone(), &config).expect("registry");
	selected.select(Target::Ios, MissingPlugins::Warn).unwrap();
	assert_eq!(selected.missing[0].reason, Missing::ExperimentalRustPlugin);
	let without = descriptor(&config, Target::Ios, &selected.plugins, &selected.missing);
	assert_eq!(
		without.state_of("plugin.echo").map(|(state, _)| *state),
		Some(CapabilityState::Unsupported)
	);

	config.native.experimental_rust_plugins = true;
	let mut enabled = resolve_metadata(metadata, &config).expect("registry");
	enabled.select(Target::Ios, MissingPlugins::Warn).unwrap();
	let with = descriptor(&config, Target::Ios, &enabled.plugins, &enabled.missing);
	assert_eq!(
		with.state_of("plugin.echo").map(|(state, _)| *state),
		Some(CapabilityState::Experimental)
	);
	assert_ne!(without.digest(), with.digest());
}

#[test]
fn grants_and_option_values_change_the_digest() {
	let mut config = echo_config();
	let registry = resolve_metadata(async_metadata(), &config).expect("registry");
	config.native.plugin_options.insert(
		"echo".into(),
		toml::from_str("origins = ['https://one.example']").unwrap(),
	);

	let one = descriptor(&config, Target::Ios, &registry.plugins, &registry.missing);
	config.native.plugin_options.insert(
		"echo".into(),
		toml::from_str("origins = ['https://two.example']").unwrap(),
	);
	let two = descriptor(&config, Target::Ios, &registry.plugins, &registry.missing);
	assert_ne!(one.digest(), two.digest());

	let mut extra = config.clone();
	extra.native.plugins.push("clipboard".into());
	extra.native.optional_capabilities = vec!["plugin.clipboard".into()];
	let changed = descriptor(&extra, Target::Ios, &registry.plugins, &registry.missing);
	assert_ne!(two.digest(), changed.digest());
}

#[test]
fn the_descriptor_never_contains_plugin_option_values_or_paths() {
	let mut config = echo_config();
	let registry = resolve_metadata(async_metadata(), &config).expect("registry");
	config.native.plugin_options.insert(
		"echo".into(),
		toml::from_str("origins = ['https://secret.example']").unwrap(),
	);
	let contract = descriptor(&config, Target::Ios, &registry.plugins, &registry.missing);
	let json = serde_json::to_string(&contract).unwrap();
	assert!(!json.contains("secret.example"), "{json}");
	assert!(!json.contains("/sdk/echo"), "{json}");
	assert_eq!(contract.plugins[0].option_keys, ["origins"]);
	assert!(contract.plugins[0].options_sha256.is_some());
}

#[test]
fn platform_capability_documentation_matches_the_host_model() {
	let expected = std::fs::read_to_string(
		std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
			.join("../../docs/agents/platform-capability-matrix.md"),
	)
	.expect("capability matrix");
	// dprint aligns the table's columns; compare the cells, not its padding.
	fn cells(text: &str) -> Vec<String> {
		text.lines()
			.map(|line| {
				if line.starts_with('|') {
					line.trim_matches('|')
						.split('|')
						.map(str::trim)
						.collect::<Vec<_>>()
						.join("\t")
				} else {
					line.trim().to_owned()
				}
			})
			.collect()
	}
	assert_eq!(
		cells(&documentation_table()),
		cells(&expected),
		"docs/agents/platform-capability-matrix.md is stale"
	);
}

#[test]
fn generated_requirements_are_sorted_and_split() {
	let mut config = echo_config();
	config
		.native
		.plugins
		.extend(["haptics".into(), "http".into()]);
	config.native.required_plugins = vec!["http".into(), "echo".into()];
	config.native.optional_plugins = vec!["haptics".into()];
	config.native.required_capabilities = vec![CAP_HOST_PLUGIN_NATIVE_ANDROID.into()];
	config.native.optional_capabilities = vec![CAP_HOST_PLUGIN_RUST_ASYNC.into()];

	let requirements = requirements(&config);
	assert_eq!(
		requirements.required,
		vec![
			"host.plugin.native.android".to_owned(),
			"plugin.echo".to_owned(),
			"plugin.http".to_owned()
		]
	);
	assert_eq!(
		requirements.optional,
		vec![
			"host.plugin.rust.async".to_owned(),
			"plugin.haptics".to_owned()
		]
	);
}
