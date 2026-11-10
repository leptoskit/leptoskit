//! Headless guest tooling: a wat module implements the leptoskit ABI, and the
//! script runner, tree dump, and snapshots drive and observe it.

use leptoskit_testing::Finder;
use leptoskit_testing::headless::HeadlessApp;
use leptoskit_testing::headless::Script;
use leptoskit_testing::headless::SnapshotFormat;

const WIDTH: f32 = 320.0;
const HEIGHT: f32 = 480.0;

/// A small form: a counter, a name field that echoes what is typed, a
/// switch, a styled card, and a status line that async work fills in on the
/// second plugin wake.
const FORM_WAT: &str = r#"
(module
  (import "leptoskit" "node_create" (func $node_create (param i64 i32)))
  (import "leptoskit" "node_set_props" (func $node_set_props (param i64 i32 i32)))
  (import "leptoskit" "node_insert" (func $node_insert (param i64 i64 i32)))
  (import "leptoskit" "node_remove" (func $node_remove (param i64 i64)))
  (import "leptoskit" "node_delete" (func $node_delete (param i64)))
  (import "leptoskit" "commit" (func $commit))
  (global $count (mut i32) (i32.const 0))
  (global $wakes (mut i32) (i32.const 0))
  (memory (export "memory") 1)
  (data (i32.const 1024) "{\"class\":\"p-4 gap-2\"}")
  (data (i32.const 1048) "{\"text\":\"Count: 0\"}")
  (data (i32.const 1072) "{\"text\":\"Increment\",\"extra\":{\"test_id\":{\"Str\":\"increment\"}}}")
  (data (i32.const 1136) "{\"accessibility_label\":\"Name\",\"placeholder\":\"Your name\"}")
  (data (i32.const 1200) "{\"text\":\"Hello\"}")
  (data (i32.const 1600) "{\"accessibility_label\":\"Notifications\",\"checked\":false,\"class\":\"w-12 h-7\"}")
  (data (i32.const 1680) "{\"accessibility_label\":\"Notifications\",\"checked\":true,\"class\":\"w-12 h-7\"}")
  (data (i32.const 1336) "{\"class\":\"bg-blue-500 rounded-lg border-2 border-blue-700 p-2\"}")
  (data (i32.const 1400) "{\"text\":\"Card\",\"class\":\"text-white font-bold\"}")
  (data (i32.const 1448) "{\"text\":\"Loading\"}")
  (data (i32.const 1472) "{\"text\":\"Loaded\"}")
  (data (i32.const 1496) "{\"text\":")
  (func (export "leptoskit_alloc") (param $len i32) (result i32) (i32.const 4096))
  (func (export "leptoskit_main")
    (call $node_create (i64.const 1) (i32.const 0))
    (call $node_set_props (i64.const 1) (i32.const 1024) (i32.const 21))
    (call $node_create (i64.const 2) (i32.const 1))
    (call $node_set_props (i64.const 2) (i32.const 1048) (i32.const 19))
    (call $node_create (i64.const 3) (i32.const 6))
    (call $node_set_props (i64.const 3) (i32.const 1072) (i32.const 60))
    (call $node_create (i64.const 4) (i32.const 5))
    (call $node_set_props (i64.const 4) (i32.const 1136) (i32.const 56))
    (call $node_create (i64.const 5) (i32.const 1))
    (call $node_set_props (i64.const 5) (i32.const 1200) (i32.const 16))
    (call $node_create (i64.const 6) (i32.const 7))
    (call $node_set_props (i64.const 6) (i32.const 1600) (i32.const 74))
    (call $node_create (i64.const 7) (i32.const 0))
    (call $node_set_props (i64.const 7) (i32.const 1336) (i32.const 63))
    (call $node_create (i64.const 8) (i32.const 1))
    (call $node_set_props (i64.const 8) (i32.const 1400) (i32.const 46))
    (call $node_create (i64.const 9) (i32.const 1))
    (call $node_set_props (i64.const 9) (i32.const 1448) (i32.const 18))
    (call $node_insert (i64.const 1) (i64.const 2) (i32.const 0))
    (call $node_insert (i64.const 1) (i64.const 3) (i32.const 1))
    (call $node_insert (i64.const 1) (i64.const 4) (i32.const 2))
    (call $node_insert (i64.const 1) (i64.const 5) (i32.const 3))
    (call $node_insert (i64.const 1) (i64.const 6) (i32.const 4))
    (call $node_insert (i64.const 1) (i64.const 7) (i32.const 5))
    (call $node_insert (i64.const 7) (i64.const 8) (i32.const 0))
    (call $node_insert (i64.const 1) (i64.const 9) (i32.const 6))
    (call $commit))
  (func (export "leptoskit_event")
    (param $id i64) (param $kind i32) (param $ptr i32) (param $len i32) (result i32)
    ;; Increment: a press on the button.
    (if (i32.and (i64.eq (local.get $id) (i64.const 3)) (i32.eqz (local.get $kind)))
      (then
        (global.set $count (i32.add (global.get $count) (i32.const 1)))
        (i32.store8 (i32.const 1064) (i32.add (i32.const 48) (global.get $count)))
        (call $node_set_props (i64.const 2) (i32.const 1048) (i32.const 19))
        (call $commit)
        (return (i32.const 1))))
    ;; Typing: echo `{"value":"…"}` as the greeting's `{"text":"…"}`.
    (if (i32.and (i64.eq (local.get $id) (i64.const 4)) (i32.eq (local.get $kind) (i32.const 1)))
      (then
        (memory.copy (i32.const 2048) (i32.const 1496) (i32.const 8))
        (memory.copy
          (i32.const 2056)
          (i32.add (local.get $ptr) (i32.const 9))
          (i32.sub (local.get $len) (i32.const 10)))
        (i32.store8 (i32.add (i32.const 2046) (local.get $len)) (i32.const 125))
        (call $node_set_props
          (i64.const 5) (i32.const 2048) (i32.sub (local.get $len) (i32.const 1)))
        (call $commit)
        (return (i32.const 1))))
    ;; The switch: `{"checked":true}` has a `t` at offset 11.
    (if (i32.and (i64.eq (local.get $id) (i64.const 6)) (i32.eq (local.get $kind) (i32.const 6)))
      (then
        (if (i32.eq (i32.load8_u (i32.add (local.get $ptr) (i32.const 11))) (i32.const 116))
          (then (call $node_set_props (i64.const 6) (i32.const 1680) (i32.const 73)))
          (else (call $node_set_props (i64.const 6) (i32.const 1600) (i32.const 74))))
        (call $commit)
        (return (i32.const 1))))
    (i32.const 0))
  (func (export "leptoskit_plugins_ready")
    (global.set $wakes (i32.add (global.get $wakes) (i32.const 1)))
    (if (i32.eq (global.get $wakes) (i32.const 2))
      (then
        (call $node_set_props (i64.const 9) (i32.const 1472) (i32.const 17))
        (call $commit))))
)
"#;

fn app() -> HeadlessApp {
	HeadlessApp::start(FORM_WAT.as_bytes(), WIDTH, HEIGHT).expect("the form guest starts")
}

#[test]
fn the_tree_dump_names_kinds_roles_props_and_frames() {
	insta::assert_snapshot!(app().tree());
}

#[test]
fn the_tree_dump_is_json_too() {
	let tree = app().tree_json().expect("the guest committed");
	insta::assert_json_snapshot!(tree);
}

#[test]
fn interactions_reach_the_guest_and_rerender() {
	let mut app = app();
	app.tap(&Finder::test_id("increment")).expect("press");
	app.tap(&Finder::text("Increment")).expect("press by text");
	app.find(&Finder::text("Count: 2"))
		.expect("the count rerendered");

	app.enter_text(&Finder::label("Name"), "Ada").expect("type");
	app.find(&Finder::text("Ada"))
		.expect("the guest echoed the value");

	app.tap(&Finder::label("Notifications"))
		.expect("tap the switch");
	let switch = app.find(&Finder::label("Notifications")).expect("switch");
	assert_eq!(
		switch.props.checked,
		Some(true),
		"a tap proposes the inverse"
	);
	assert_eq!(app.commits(), 5);

	let frame = app.last_frame().expect("the last commit was recorded");
	assert_eq!(frame.commit, 5);
	assert_eq!(frame.nodes, 9);
	assert!(frame.mutations > 0);
}

#[test]
fn waits_wake_the_guest_until_its_async_work_commits() {
	let mut app = app();
	app.wait_for(&Finder::text("Loaded"), 5)
		.expect("the second wake commits");

	let mut app = self::app();
	let error = app
		.wait_for_commits(1, 1)
		.expect_err("one wake is not enough");
	assert!(
		error
			.to_string()
			.starts_with("the guest committed 0 of 1 times within 1 wakes"),
		"{error}"
	);
	app.wait_for_commits(1, 5).expect("the next wake commits");
}

#[test]
fn a_script_runs_every_step_and_reports_each() {
	let script = Script::from_json(
		r#"{
			"viewport": { "width": 320, "height": 480 },
			"steps": [
				{ "action": "press", "test_id": "increment" },
				{ "action": "press", "role": "button", "name": "Increment" },
				{ "action": "expect", "text": "Count: 2" },
				{ "action": "expect_absent", "text": "Count: 0" },
				{ "action": "type", "label": "Name", "value": "Ada" },
				{ "action": "expect", "text": "Ada" },
				{ "action": "toggle", "label": "Notifications", "checked": true },
				{ "action": "wait", "commits": 1 },
				{ "action": "expect", "text": "Loaded" }
			]
		}"#,
	)
	.expect("the script parses");
	let report = HeadlessApp::run_script(FORM_WAT.as_bytes(), &script).expect("the guest starts");

	assert!(report.passed, "{}", report.summary());
	assert_eq!(report.steps.len(), 9);
	assert_eq!(report.skipped, 0);
	insta::assert_snapshot!(report.summary());
}

#[test]
fn a_failed_step_stops_the_script_and_shows_the_tree() {
	let script = Script::from_json(
		r#"{"steps": [
			{ "action": "press", "text": "Increment" },
			{ "action": "expect", "text": "Count: 9" },
			{ "action": "press", "text": "Increment" }
		]}"#,
	)
	.expect("the script parses");
	let report = HeadlessApp::run_script(FORM_WAT.as_bytes(), &script).expect("the guest starts");

	assert!(!report.passed);
	assert_eq!(report.steps.len(), 2);
	assert_eq!(report.skipped, 1);
	insta::assert_snapshot!(report.summary());
}

#[test]
fn selectors_name_exactly_one_query() {
	let script = Script::from_json(
		r#"{"steps": [{ "action": "press", "text": "Increment", "label": "Increment" }]}"#,
	)
	.expect("the script parses");
	let report = HeadlessApp::run_script(FORM_WAT.as_bytes(), &script).expect("the guest starts");
	let error = report.steps[0].error.as_deref().expect("the step failed");
	assert!(
		error
			.starts_with("a selector needs exactly one of text, label, test_id, or role with name"),
		"{error}"
	);

	let error = Script::from_json(r#"{"steps": [], "viewprt": {}}"#)
		.expect_err("unknown script fields are rejected");
	assert!(
		error.to_string().contains("unknown field `viewprt`"),
		"{error}"
	);
}

#[test]
fn the_svg_snapshot_draws_frames_fills_borders_and_text() {
	let mut app = app();
	app.enter_text(&Finder::label("Name"), "Ada & <Lovelace>")
		.expect("type");
	let snapshot = app.snapshot(SnapshotFormat::Svg).expect("svg");
	assert_eq!(snapshot.mime_type(), "image/svg+xml");
	let svg = String::from_utf8(snapshot.bytes().to_vec()).expect("svg is text");
	assert_eq!(
		svg,
		String::from_utf8(app.snapshot(SnapshotFormat::Svg).unwrap().bytes().to_vec()).unwrap(),
		"the same tree renders the same bytes"
	);
	insta::assert_snapshot!(svg);
}

#[test]
fn the_png_snapshot_rasterises_at_a_scale() {
	let app = app();
	let snapshot = app
		.snapshot(SnapshotFormat::Png { scale: 2.0 })
		.expect("png");
	assert_eq!(snapshot.mime_type(), "image/png");
	assert_eq!(
		snapshot.bytes(),
		app.snapshot(SnapshotFormat::Png { scale: 2.0 })
			.unwrap()
			.bytes(),
		"the same tree renders the same bytes"
	);

	let decoder = png::Decoder::new(std::io::Cursor::new(snapshot.bytes()));
	let mut reader = decoder.read_info().expect("a valid png");
	let mut pixels = vec![0; reader.output_buffer_size()];
	let info = reader.next_frame(&mut pixels).expect("one frame");
	assert_eq!((info.width, info.height), (640, 960));

	let surface = app.host().surface();
	let card = surface.children(surface.root().expect("a root"))[5];
	let fill = surface.props(card).background.expect("bg-blue-500");
	let text = app
		.find(&Finder::text("Card"))
		.expect("the card text")
		.frame;
	// Just inside the card's padding, left of its text: the card's fill.
	let (x, y) = ((text.x - 4.0) * 2.0, (text.y + 2.0) * 2.0);
	let offset = ((y as usize) * info.width as usize + x as usize) * 4;
	assert_eq!(
		pixels[offset..offset + 4],
		[fill.red(), fill.green(), fill.blue(), fill.alpha()],
		"the card's fill"
	);
	assert_eq!(&pixels[..4], [0xff, 0xff, 0xff, 0xff], "the page is white");
}

#[test]
fn snapshot_formats_follow_the_extension() {
	use std::path::Path;

	assert_eq!(
		SnapshotFormat::for_path(Path::new("a.svg"), 1.0),
		Ok(SnapshotFormat::Svg)
	);
	assert_eq!(
		SnapshotFormat::for_path(Path::new("a.png"), 3.0),
		Ok(SnapshotFormat::Png { scale: 3.0 })
	);
	assert!(SnapshotFormat::for_path(Path::new("a.jpg"), 1.0).is_err());
}
