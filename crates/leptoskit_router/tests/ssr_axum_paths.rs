//! Generated routes expose axum paths the SSR server registers.

use leptoskit_router::Pattern;
use leptoskit_router::Segment;

#[test]
fn starter_tabs_register_their_paths() {
	assert_eq!(
		vec![
			Pattern::new(&[]).axum_path(),
			Pattern::new(&[Segment::Static("kit")]).axum_path(),
			Pattern::new(&[Segment::Static("platforms")]).axum_path(),
			Pattern::new(&[Segment::OptionalRest("path")]).axum_path(),
		],
		vec![
			String::from("/"),
			String::from("/kit"),
			String::from("/platforms"),
			String::from("/{*path}"),
		]
	);
}
