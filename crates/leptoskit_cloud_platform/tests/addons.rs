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
