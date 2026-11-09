use super::*;

fn async_metadata() -> Metadata {
	let mut value = metadata("Plugin");
	value.packages[2].metadata = json!({"leptoskit":{"runtime":{
		"name":"echo", "rust-factory":"from_config", "rust-async":true
	}}});
	value
}

#[test]
fn async_factories_require_host_options_and_select_host_features() {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];
	let error = resolve_metadata(async_metadata(), &config)
		.err()
		.unwrap()
		.to_string();
	assert!(error.contains("[native.plugin-options.echo]"));
	config.native.plugin_options.insert(
		"echo".into(),
		toml::from_str("origins = [\"https://example.com\"]\nlabel = 'quote\" and newline\\n'")
			.unwrap(),
	);
	let registry = resolve_metadata(async_metadata(), &config).unwrap();
	assert_eq!(
		registry.host_features,
		["renamed_echo/async-host", "ffi/async-host"]
	);

	let options: serde_json::Value =
		serde_json::from_str(registry.plugins[0].options_json.as_ref().unwrap()).unwrap();
	assert_eq!(options["origins"][0], "https://example.com");
	let source = crate::native::project::rust_registrant(&registry, Target::Android).unwrap();
	assert!(source.contains(
		"app.register(\"echo\", |app| {\n                    \
		 app.install_async(std::sync::Arc::new(renamed_echo::from_config("
	));
	assert!(
		crate::native::project::installs_only_named_plugins(&source),
		"{source}"
	);
	assert!(!source.contains("&_context.data_dir"));
	assert!(source.contains("\\\"origins\\\""));
	assert!(resolve_metadata(metadata("Plugin"), &config).is_err());

	config
		.native
		.plugin_options
		.get_mut("echo")
		.unwrap()
		.insert("oversized".into(), toml::Value::String("x".repeat(65_536)));
	assert!(resolve_metadata(async_metadata(), &config).is_err());
}

#[test]
fn async_metadata_rejects_worker_mode_and_missing_factory() {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];
	config
		.native
		.plugin_options
		.insert("echo".into(), toml::Table::new());
	for runtime in [
		json!({"name":"echo", "rust-async":true}),
		json!({"name":"echo", "rust-async":true, "rust-type":"Plugin"}),
		json!({"name":"echo", "rust-async":true, "rust-factory":"from_config", "rust-worker":true}),
		json!({"name":"echo", "rust-async":true, "rust-factory":"from_config();panic!()"}),
	] {
		let mut value = metadata("Plugin");
		value.packages[2].metadata = json!({"leptoskit":{"runtime":runtime}});
		assert!(resolve_metadata(value, &config).is_err());
	}
}

#[test]
fn host_features_use_manifest_keys_while_rust_uses_normalized_aliases() {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];
	config
		.native
		.plugin_options
		.insert("echo".into(), toml::Table::new());
	let mut metadata = async_metadata();
	metadata.packages[0].dependencies[1].rename = Some("renamed-echo".into());
	let registry = resolve_metadata(metadata, &config).unwrap();
	assert_eq!(registry.plugins[0].crate_name, "renamed_echo");

	assert_eq!(
		registry.host_features,
		["renamed-echo/async-host", "ffi/async-host"]
	);
}

#[test]
fn apps_that_draw_scenes_turn_on_async_plugins_scene3d_features() {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];
	config
		.native
		.plugin_options
		.insert("echo".into(), toml::Table::new());
	let with_scenes = |plugin_features: serde_json::Value| {
		let mut metadata = async_metadata();
		metadata.packages[2].features = serde_json::from_value(plugin_features).unwrap();
		metadata.packages.push(
			serde_json::from_value(json!({
				"id": "scene3d", "name": "leptoskit_3d", "manifest_path": "/sdk/3d/Cargo.toml",
				"metadata": {}
			}))
			.unwrap(),
		);
		metadata.resolve.nodes.push(
			serde_json::from_value(json!({"id": "scene3d", "deps": [], "features": ["renderer"]}))
				.unwrap(),
		);
		metadata
	};

	let registry = resolve_metadata(
		with_scenes(json!({"async-host": [], "scene3d": ["async-host"]})),
		&config,
	)
	.unwrap();
	assert_eq!(
		registry.host_features,
		[
			"renamed_echo/async-host",
			"ffi/async-host",
			"ffi/scene3d",
			"renamed_echo/scene3d"
		]
	);

	// A plugin without the feature, or an app without scenes, gets none.
	let registry = resolve_metadata(with_scenes(json!({"async-host": []})), &config).unwrap();
	assert_eq!(
		registry.host_features,
		["renamed_echo/async-host", "ffi/async-host", "ffi/scene3d"]
	);
	let mut without_scenes = async_metadata();
	without_scenes.packages[2].features =
		serde_json::from_value(json!({"async-host": [], "scene3d": []})).unwrap();
	let registry = resolve_metadata(without_scenes, &config).unwrap();
	assert_eq!(
		registry.host_features,
		["renamed_echo/async-host", "ffi/async-host"]
	);
}

#[test]
fn quest_hosts_turn_on_async_plugins_quest_features() {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];
	config
		.native
		.plugin_options
		.insert("echo".into(), toml::Table::new());
	let quest_metadata = || {
		let mut metadata = async_metadata();
		metadata.packages[2].features =
			serde_json::from_value(json!({"async-host": [], "quest": ["async-host"]})).unwrap();
		metadata
	};

	let registry = resolve_for_target(quest_metadata(), &config, Some(Target::Quest)).unwrap();
	assert_eq!(
		registry.host_features,
		[
			"renamed_echo/async-host",
			"ffi/async-host",
			"renamed_echo/quest"
		]
	);
	let registry = resolve_for_target(quest_metadata(), &config, Some(Target::Android)).unwrap();
	assert_eq!(
		registry.host_features,
		["renamed_echo/async-host", "ffi/async-host"]
	);
}

#[test]
fn options_turn_on_the_plugin_features_they_name() {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];
	config
		.native
		.plugin_options
		.insert("echo".into(), toml::Table::new());
	let with_option_features = |features: serde_json::Value| {
		let mut value = metadata("Plugin");
		value.packages[2].metadata = json!({"leptoskit":{"runtime":{
			"name":"echo", "rust-factory":"from_config", "rust-async":true,
			"option-features": {"depth-model": "depth-estimation"}
		}}});
		value.packages[2].features = serde_json::from_value(features).unwrap();
		value
	};
	let declared = json!({"async-host": [], "depth-estimation": []});

	// Without the option, only the host.
	let registry = resolve_metadata(with_option_features(declared.clone()), &config).unwrap();
	assert_eq!(
		registry.host_features,
		["renamed_echo/async-host", "ffi/async-host"]
	);

	config
		.native
		.plugin_options
		.get_mut("echo")
		.unwrap()
		.insert(
			"depth-model".into(),
			toml::Value::String("depth/model.onnx".into()),
		);
	let registry = resolve_metadata(with_option_features(declared), &config).unwrap();
	assert_eq!(
		registry.host_features,
		[
			"renamed_echo/async-host",
			"renamed_echo/depth-estimation",
			"ffi/async-host"
		]
	);

	// A feature the package doesn't have is a mistake in its metadata.
	let error = resolve_metadata(with_option_features(json!({"async-host": []})), &config)
		.err()
		.unwrap()
		.to_string();
	assert!(error.contains("depth-estimation"), "{error}");

	for option_features in [
		json!({"Depth Model": "depth-estimation"}),
		json!({"depth-model": "depth estimation"}),
		json!({"depth-model": "a/b"}),
	] {
		let mut value = metadata("Plugin");
		value.packages[2].metadata = json!({"leptoskit":{"runtime":{
			"name":"echo", "rust-factory":"from_config", "rust-async":true,
			"option-features": option_features
		}}});
		assert!(resolve_metadata(value, &config).is_err());
	}
}

#[test]
fn only_plugins_that_take_options_map_them_to_features() {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];
	let mut value = metadata("Plugin");
	value.packages[2].metadata = json!({"leptoskit":{"runtime":{
		"name":"echo", "rust-type":"Plugin",
		"option-features": {"depth-model": "depth-estimation"}
	}}});
	let error = resolve_metadata(value, &config).err().unwrap().to_string();
	assert!(error.contains("option-features"), "{error}");
}

fn requiring(required: &serde_json::Value) -> Metadata {
	let mut value = metadata("Plugin");
	value.packages[2].metadata = json!({"leptoskit":{"runtime":{
		"name":"echo", "rust-factory":"from_config", "rust-async":true,
		"required-options": required
	}}});
	value
}

/// An app whose options leave out what the plugin's factory needs used to
/// build, then stop at launch with `register plugin: InvalidArgument`: an
/// empty `[native.plugin-options.websocket]` did that to an Android app.
#[test]
fn the_build_stops_when_options_leave_out_what_the_plugin_requires() {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];

	for options in [
		"",
		"origins = []",
		"origins = ''",
		"origins = {}",
		"timeout-ms = 5",
	] {
		config
			.native
			.plugin_options
			.insert("echo".into(), toml::from_str(options).unwrap());
		let error = resolve_metadata(requiring(&json!(["origins"])), &config)
			.err()
			.unwrap_or_else(|| panic!("`{options}` built without origins"))
			.to_string();
		assert!(
			error.contains("plugin `echo` needs `origins` in [native.plugin-options.echo]"),
			"{error}"
		);
	}

	config.native.plugin_options.insert(
		"echo".into(),
		toml::from_str("origins = [\"wss://live.example.com\"]").unwrap(),
	);
	let registry = resolve_metadata(requiring(&json!(["origins"])), &config).unwrap();
	assert!(
		registry.plugins[0]
			.options_json
			.as_ref()
			.unwrap()
			.contains("wss://live.example.com")
	);

	// Values that aren't empty count, whatever their type.
	config.native.plugin_options.insert(
		"echo".into(),
		toml::from_str("origins = [\"wss://a.example\"]\nlimit = 0\nstrict = false").unwrap(),
	);
	resolve_metadata(requiring(&json!(["origins", "limit", "strict"])), &config).unwrap();

	// A plugin that requires nothing builds with an empty table.
	config
		.native
		.plugin_options
		.insert("echo".into(), toml::Table::new());
	resolve_metadata(requiring(&json!([])), &config).unwrap();
}

#[test]
fn required_options_are_option_names_of_plugins_that_take_options() {
	let mut config = LeptoskitConfig::default();
	config.app.name = "app".into();
	config.native.plugins = vec!["echo".into()];
	config
		.native
		.plugin_options
		.insert("echo".into(), toml::Table::new());

	for required in [
		json!([""]),
		json!(["Origins"]),
		json!(["a b"]),
		json!(["a.b"]),
	] {
		let error = resolve_metadata(requiring(&required), &config)
			.err()
			.unwrap_or_else(|| panic!("{required} was accepted"))
			.to_string();
		assert!(error.contains("required-options"), "{error}");
	}

	config.native.plugin_options.clear();
	let mut value = metadata("Plugin");
	value.packages[2].metadata = json!({"leptoskit":{"runtime":{
		"name":"echo", "rust-type":"Plugin", "required-options": ["origins"]
	}}});
	let error = resolve_metadata(value, &config).err().unwrap().to_string();
	assert!(error.contains("required-options"), "{error}");
}

/// Every async plugin in this repository says which options its factory
/// can't start without, even when it's none, so the build checks an app's
/// options before they reach the factory. Each crate's tests hold its
/// factory to that list.
#[test]
fn every_async_plugin_in_the_repository_declares_its_required_options() {
	let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
	let mut checked = Vec::new();

	for entry in std::fs::read_dir(&crates).unwrap() {
		let manifest = entry.unwrap().path().join("Cargo.toml");

		let Ok(source) = std::fs::read_to_string(&manifest) else {
			continue;
		};
		let manifest: toml::Table = source.parse().unwrap();
		let Some(runtime) = manifest
			.get("package")
			.and_then(|package| package.get("metadata"))
			.and_then(|metadata| metadata.get("leptoskit"))
			.and_then(|leptoskit| leptoskit.get("runtime"))
		else {
			continue;
		};

		if runtime.get("rust-async").and_then(toml::Value::as_bool) != Some(true) {
			continue;
		}

		let name = runtime["name"].as_str().unwrap().to_owned();
		assert!(
			runtime
				.get("required-options")
				.is_some_and(toml::Value::is_array),
			"the async plugin `{name}` doesn't declare `required-options`; list the options its \
			 factory can't start without (`[]` for none), and test the factory against the list"
		);
		checked.push(name);
	}

	checked.sort();
	// The async plugins this repository has today; a new one joins the list.
	assert_eq!(checked, ["http", "shell", "websocket", "xr"]);
}
