//! The display list's runtime constructors, defaults, and sanitiser branches.

use leptoskit_canvas::*;

#[test]
fn colours_convert_and_fade_at_runtime() {
	assert_eq!(Color::rgb8(255, 0, 0), Color::rgba(1.0, 0.0, 0.0, 1.0));
	assert_eq!(
		Color::rgba8(0, 255, 0, 128),
		Color::rgba(0.0, 1.0, 0.0, 128.0 / 255.0)
	);
	let faded = Color::rgba(0.25, 0.5, 0.75, 0.8).with_opacity(0.5);
	assert_eq!(faded, Color::rgba(0.25, 0.5, 0.75, 0.4));
	assert_eq!(
		faded.premultiplied(),
		[0.25 * 0.4, 0.5 * 0.4, 0.75 * 0.4, 0.4]
	);
}

#[test]
fn defaults_and_builders_match_their_documented_values() {
	assert_eq!(Affine::default(), Affine::IDENTITY);
	assert_eq!(Paint::default(), Paint::Solid(Color::BLACK));
	assert_eq!(CornerRadii::from(4.0), CornerRadii::all(4.0));
	assert_eq!(
		Stroke::default(),
		Stroke {
			width: 1.0,
			cap: LineCap::Butt,
			join: LineJoin::Miter,
			miter_limit: 10.0,
		}
	);
	let stroke = Stroke::new(2.0)
		.cap(LineCap::Round)
		.join(LineJoin::Bevel)
		.miter_limit(4.0);
	assert_eq!(
		stroke,
		Stroke {
			width: 2.0,
			cap: LineCap::Round,
			join: LineJoin::Bevel,
			miter_limit: 4.0,
		}
	);
}

#[test]
fn path_clips_survive_sanitising() {
	let clip = Path::new()
		.move_to(1.0, 1.0)
		.quad_to(5.0, 0.0, 9.0, 1.0)
		.cubic_to(
			Point::new(9.0, 5.0),
			Point::new(9.0, 9.0),
			Point::new(1.0, 9.0),
		)
		.close();
	let scene = Scene::new()
		.push_clip_path(clip.clone(), FillRule::EvenOdd)
		.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE)
		.pop_clip()
		.sanitized();
	assert_eq!(
		scene.commands[0],
		Command::PushClipPath {
			path: clip,
			rule: FillRule::EvenOdd
		}
	);
	assert!(matches!(scene.commands[1], Command::FillRect { .. }));
	assert_eq!(scene.commands[2], Command::PopClip);
}

#[test]
fn box_shadows_are_bounded() {
	let scene = Scene::new()
		.box_shadow(
			Rect::new(0.0, 0.0, 100.0, 100.0),
			8.0,
			BoxShadow {
				offset_x: -1.0e9,
				offset_y: f32::INFINITY,
				blur: f32::INFINITY,
				spread: 1.0e9,
				color: Color::rgba(-1.0, 2.0, 0.5, 1.0),
			},
		)
		.sanitized();
	let Command::BoxShadow { shadow, .. } = &scene.commands[0] else {
		panic!("a shadow");
	};
	assert_eq!(shadow.offset_x, -MAX_COORDINATE);
	assert_eq!(shadow.offset_y, 0.0);
	assert_eq!(shadow.blur, 0.0);
	assert_eq!(shadow.spread, MAX_COORDINATE);
	assert_eq!(shadow.color, Color::rgba(0.0, 1.0, 0.5, 1.0));
}

#[test]
fn an_unclosed_clip_is_closed_by_the_sanitiser() {
	let rect = Rect::new(0.0, 0.0, 2.0, 2.0);
	let scene = Scene::new().push_clip(rect, 0.0).sanitized();
	assert_eq!(
		scene.commands,
		[
			Command::PushClip {
				rect,
				radii: CornerRadii::default()
			},
			Command::PopClip,
		]
	);
}

#[test]
fn a_clip_path_under_a_full_stack_is_dropped() {
	let mut scene = Scene::new();
	for _ in 0..MAX_DEPTH {
		scene = scene.push_transform(Affine::IDENTITY);
	}
	let scene = scene
		.push_clip_path(
			Path::new().rect(Rect::new(0.0, 0.0, 1.0, 1.0)),
			FillRule::NonZero,
		)
		.sanitized();
	assert_eq!(
		scene
			.commands
			.iter()
			.filter(|command| matches!(command, Command::PushClipPath { .. }))
			.count(),
		0
	);
	assert_eq!(scene.commands.len(), MAX_DEPTH * 2);
}
