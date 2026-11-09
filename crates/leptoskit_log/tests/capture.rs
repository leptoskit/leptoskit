//! The capturing sink and the level.

use log::Level;
use log::LevelFilter;

#[test]
fn captured_records_keep_their_level_target_and_message() {
	let records = leptoskit_log::capture::start();
	log::warn!(target: &leptoskit_log::target::plugin("http"), "call #4 method 1 failed: invalid argument");
	log::trace!(target: "wgpu_hal::vulkan", "a library's own record 51e2");

	let record = records.expect("leptoskit/plugin/http", "call #4");
	assert_eq!(record.level, Level::Warn);
	assert_eq!(leptoskit_log::tag(&record.target), "leptoskit/plugin/http");
	assert!(records.find("wgpu_hal", "51e2").is_some());
	assert!(
		records
			.find("leptoskit/plugin/http", "no such text")
			.is_none()
	);
}

#[test]
fn the_build_sets_the_level_unless_leptoskit_toml_does() {
	let expected = option_env!("LEPTOSKIT_LOG_LEVEL")
		.and_then(|level| level.parse().ok())
		.unwrap_or(if cfg!(debug_assertions) {
			LevelFilter::Info
		} else {
			LevelFilter::Warn
		});
	assert_eq!(leptoskit_log::level(), expected);
}
