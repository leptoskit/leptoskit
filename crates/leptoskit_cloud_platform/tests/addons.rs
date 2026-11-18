use leptoskit_cloud_platform::addons;
use leptoskit_cloud_platform::config::PlatformConfig;

fn snapshot(name: &str, input: &str) {
	let config = PlatformConfig::parse(input).unwrap();
	let files = addons::generate(&config).unwrap();
	assert_eq!(files, addons::generate(&config).unwrap());
	insta::assert_snapshot!(
		name,
		files
			.iter()
			.map(|(path, content)| format!("# {path}\n{content}"))
			.collect::<Vec<_>>()
			.join("---\n")
	);
}

#[test]
fn launch() {
	snapshot("addons_launch", include_str!("../examples/launch.toml"));
}

#[test]
fn growth() {
	snapshot("addons_growth", include_str!("../examples/growth.toml"));
	let config = PlatformConfig::parse(include_str!("../examples/growth.toml")).unwrap();
	for content in addons::generate(&config).unwrap().values() {
		let doc: serde_json::Value = serde_json::from_str(content).unwrap();
		if let Some(values) = doc["spec"]["valuesContent"].as_str() {
			let values: serde_json::Value = serde_json::from_str(values).unwrap();
			if let Some(selector) = values.get("nodeSelector") {
				assert_eq!(selector["leptoskit.dev/pool"], "platform");
			}
		}
	}
}
