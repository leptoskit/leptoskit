//! Gesture coverage for the controller, and the sanity suite that exercises it.
//!
//! The gestures are the ones the widget arena recognizes: tap, long press,
//! pan, and pinch. Holds last 600 milliseconds and drags travel 80 points,
//! past a 500 millisecond long press and an 18 point pan, so a backend that
//! feeds the arena accepts them. Android and Linux do. The other targets
//! still deliver presses and clicks, and the controller injects the gestures
//! for the operating system anyway.

use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use super::Document;
use super::Error;
use super::Point;
use super::SCHEMA_VERSION;
use super::Step;
use super::Target;
use super::TargetKind;
use crate::native::Application;

/// How long a sanity long press is held. Past a 500 millisecond long press.
pub const HOLD_MS: u64 = 600;

/// How long a sanity pan or pinch takes.
pub const MOVE_MS: u64 = 300;

/// Where the sanity gestures start, in viewport points or screenshot pixels.
pub const ORIGIN_X: i64 = 200;
pub const ORIGIN_Y: i64 = 400;

/// How far a sanity pan travels. Past an 18 point pan.
pub const PAN_DISTANCE: i64 = 80;

/// Distance between the two pinch pointers at the start and at the end.
pub const PINCH_START: i64 = 80;
pub const PINCH_END: i64 = 160;

/// A gesture the sanity suite can inject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum SanityGesture {
	/// A short press that does not move.
	Tap,
	/// A press held still.
	LongPress,
	/// A one-pointer drag.
	Pan,
	/// Two pointers moving apart or together.
	Pinch,
}

/// How the suite injects pointers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Injector {
	/// W3C pointer actions through a UI session.
	Driver,
	/// `adb shell input`, including a two-finger pinch.
	Android,
	/// `xdotool` on the local display. One pointer, so no pinch.
	Xdotool,
}

/// A gesture the suite left out, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Omission {
	/// The gesture that will not run.
	pub gesture: SanityGesture,
	/// Why this target or injector cannot produce it.
	pub reason: &'static str,
}

/// The document a sanity run executes, plus what it deliberately skipped.
#[derive(Debug, Clone, Serialize)]
pub struct Suite {
	/// The control document.
	pub document: Document,
	/// The gestures the document injects, in order.
	pub gestures: Vec<SanityGesture>,
	/// Gestures this target cannot inject.
	pub omitted: Vec<Omission>,
	/// Whether the widget backend feeds the gesture arena.
	pub arena: bool,
	/// Extra limits a caller should print with the outcome.
	pub notes: Vec<&'static str>,
}

/// What a sanity run needs.
pub struct SuiteRequest<'a> {
	/// The target. A driver suite replaces `application` from `identifier`.
	pub target: Target,
	/// Bundle id, package, or desktop path to open.
	pub identifier: &'a str,
	/// A package or bundle to install first.
	pub install: Option<&'a str>,
	/// Which injector builds the gesture steps.
	pub injector: Injector,
}

/// One planned tool invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPlan {
	/// `adb -s device` plus `args`.
	Adb {
		/// The adb serial.
		device: String,
		/// Arguments after `-s <serial>`.
		args: Vec<String>,
		/// Bytes written to the tool's standard input, then closed.
		stdin: Option<String>,
	},
	/// Any other program.
	Command {
		/// The program name or path.
		program: String,
		/// Its arguments.
		args: Vec<String>,
		/// Spawn and do not wait. Used for a desktop binary that must stay up.
		detach: bool,
	},
}

/// Whether `kind`'s widget backend is one that feeds the gesture arena.
#[must_use]
pub fn arena_target(kind: TargetKind) -> bool {
	matches!(kind, TargetKind::Android | TargetKind::Linux)
}

/// Whether this build's widget backend feeds the arena for `kind`.
///
/// Android and Linux deliver pointer samples into the gesture arena. The
/// other targets still deliver presses and clicks.
#[must_use]
pub fn arena_fed(kind: TargetKind) -> bool {
	arena_target(kind)
}

/// The injector a sanity run uses when the caller does not pick one.
///
/// # Errors
///
/// A website has no native injector.
pub fn default_injector(kind: TargetKind) -> Result<Injector, Error> {
	match kind {
		TargetKind::Android => Ok(Injector::Android),
		TargetKind::Linux => Ok(Injector::Xdotool),
		TargetKind::IosSimulator
		| TargetKind::IosDevice
		| TargetKind::Macos
		| TargetKind::Windows => Ok(Injector::Driver),
		TargetKind::Web => {
			Err(Error::Configuration(
				"sanity drives a native target, not the web".to_owned(),
			))
		}
	}
}

/// Builds the sanity document.
///
/// # Errors
///
/// The injector does not match the target, a mobile target has no device, or
/// the identifier or install path is empty.
pub fn suite(request: SuiteRequest<'_>) -> Result<Suite, Error> {
	if request.identifier.trim().is_empty() {
		return Err(Error::Configuration(
			"sanity needs an app identifier or path".to_owned(),
		));
	}

	if let Some(path) = request.install
		&& path.trim().is_empty()
	{
		return Err(Error::Configuration(
			"install needs a package path".to_owned(),
		));
	}

	if request.target.kind.is_mobile() && device_missing(&request.target) {
		return Err(Error::Configuration(format!(
			"{:?} needs a device id",
			request.target.kind
		)));
	}

	check_injector(request.target.kind, request.injector)?;
	let (mut steps, gestures, omitted) = gesture_steps(request.target.kind, request.injector);
	if let Some(path) = request.install {
		steps.insert(
			0,
			Step::InstallApp {
				path: path.to_owned(),
			},
		);
	}

	let mut target = request.target;
	let mut notes = Vec::new();

	match request.injector {
		Injector::Driver => {
			target.application = Some(application_from(request.identifier));
			steps.push(Step::Screenshot {
				label: "sanity".to_owned(),
			});
		}
		Injector::Android => {
			steps.insert(
				usize::from(request.install.is_some()),
				Step::OpenApp {
					identifier: request.identifier.to_owned(),
				},
			);
			steps.push(Step::DeviceScreenshot {
				label: "sanity".to_owned(),
			});
		}
		Injector::Xdotool => {
			steps.insert(
				usize::from(request.install.is_some()),
				Step::OpenApp {
					identifier: request.identifier.to_owned(),
				},
			);
			notes.push("screenshots need a UI session; pass --driver to capture the window");
		}
	}

	let kind = target.kind;
	notes.push(arena_note(kind));

	Ok(Suite {
		document: Document {
			schema_version: SCHEMA_VERSION,
			target,
			steps,
		},
		gestures,
		omitted,
		arena: arena_fed(kind),
		notes,
	})
}

fn device_missing(target: &Target) -> bool {
	target
		.device
		.as_deref()
		.is_none_or(|device| device.trim().is_empty())
}

fn check_injector(kind: TargetKind, injector: Injector) -> Result<(), Error> {
	let fits = match injector {
		Injector::Driver => kind != TargetKind::Web,
		Injector::Android => kind == TargetKind::Android,
		Injector::Xdotool => kind == TargetKind::Linux,
	};

	if fits {
		Ok(())
	} else {
		Err(Error::Configuration(format!(
			"{injector:?} does not drive {kind:?}"
		)))
	}
}

fn gesture_steps(
	kind: TargetKind,
	injector: Injector,
) -> (Vec<Step>, Vec<SanityGesture>, Vec<Omission>) {
	let pinch = match injector {
		Injector::Android => true,
		Injector::Xdotool => false,
		// A mouse session has one pointer. Phones, and a Linux driver session,
		// can move two.
		Injector::Driver => !matches!(kind, TargetKind::Macos | TargetKind::Windows),
	};
	let mut gestures = vec![
		SanityGesture::Tap,
		SanityGesture::LongPress,
		SanityGesture::Pan,
	];
	let mut omitted = Vec::new();

	if pinch {
		gestures.push(SanityGesture::Pinch);
	} else {
		let reason = if injector == Injector::Xdotool {
			"xdotool has one pointer, so it cannot pinch; use --driver for two pointers"
		} else {
			"a single mouse pointer cannot pinch"
		};
		omitted.push(Omission {
			gesture: SanityGesture::Pinch,
			reason,
		});
	}

	let steps = match injector {
		Injector::Driver => driver_steps(pinch),
		Injector::Android | Injector::Xdotool => inject_steps(pinch),
	};
	(steps, gestures, omitted)
}

fn driver_steps(pinch: bool) -> Vec<Step> {
	let mut steps = vec![
		Step::TapPoint {
			x: ORIGIN_X,
			y: ORIGIN_Y,
		},
		Step::LongPress {
			x: ORIGIN_X,
			y: ORIGIN_Y,
			duration_ms: Some(HOLD_MS),
		},
		Step::Pan {
			start: Point {
				x: ORIGIN_X,
				y: ORIGIN_Y,
			},
			end: Point {
				x: ORIGIN_X + PAN_DISTANCE,
				y: ORIGIN_Y,
			},
			duration_ms: Some(MOVE_MS),
		},
	];

	if pinch {
		steps.push(Step::Pinch {
			x: ORIGIN_X,
			y: ORIGIN_Y,
			start_span: PINCH_START,
			end_span: PINCH_END,
			duration_ms: Some(MOVE_MS),
		});
	}

	steps
}

fn inject_steps(pinch: bool) -> Vec<Step> {
	let mut steps = vec![
		Step::InjectTap {
			x: ORIGIN_X,
			y: ORIGIN_Y,
		},
		Step::InjectLongPress {
			x: ORIGIN_X,
			y: ORIGIN_Y,
			duration_ms: Some(HOLD_MS),
		},
		Step::InjectPan {
			start: Point {
				x: ORIGIN_X,
				y: ORIGIN_Y,
			},
			end: Point {
				x: ORIGIN_X + PAN_DISTANCE,
				y: ORIGIN_Y,
			},
			duration_ms: Some(MOVE_MS),
		},
	];

	if pinch {
		steps.push(Step::InjectPinch {
			x: ORIGIN_X,
			y: ORIGIN_Y,
			start_span: PINCH_START,
			end_span: PINCH_END,
			duration_ms: Some(MOVE_MS),
		});
	}

	steps
}

fn arena_note(kind: TargetKind) -> &'static str {
	if arena_fed(kind) {
		"Android and Linux feed these pointers into the gesture arena"
	} else {
		"this target's widgets receive presses and clicks; long press, pan, and pinch are injected \
		 for the operating system"
	}
}

fn application_from(identifier: &str) -> Application {
	if identifier == "desktop" {
		Application::Desktop
	} else if identifier.contains('/') || identifier.contains('\\') {
		Application::Path(identifier.to_owned())
	} else {
		Application::Identifier(identifier.to_owned())
	}
}

/// `xcrun simctl install`, `xcrun devicectl device install app`, or
/// `adb install -r`.
///
/// # Errors
///
/// Desktop and web targets do not install through these tools, or the device
/// or path is empty.
pub fn install_plan(kind: TargetKind, device: &str, path: &str) -> Result<ToolPlan, Error> {
	if device.trim().is_empty() || path.trim().is_empty() {
		return Err(Error::Configuration(
			"install needs a device id and a package path".to_owned(),
		));
	}

	match kind {
		TargetKind::IosSimulator => {
			Ok(ToolPlan::Command {
				program: "xcrun".to_owned(),
				args: vec![
					"simctl".to_owned(),
					"install".to_owned(),
					device.to_owned(),
					path.to_owned(),
				],
				detach: false,
			})
		}
		TargetKind::IosDevice => {
			Ok(ToolPlan::Command {
				program: "xcrun".to_owned(),
				args: vec![
					"devicectl".to_owned(),
					"device".to_owned(),
					"install".to_owned(),
					"app".to_owned(),
					"--device".to_owned(),
					device.to_owned(),
					path.to_owned(),
				],
				detach: false,
			})
		}
		TargetKind::Android => {
			Ok(ToolPlan::Adb {
				device: device.to_owned(),
				args: vec!["install".to_owned(), "-r".to_owned(), path.to_owned()],
				stdin: None,
			})
		}
		TargetKind::Macos | TargetKind::Linux | TargetKind::Windows => {
			Err(Error::Configuration(
				"desktop apps are installed at their path".to_owned(),
			))
		}
		TargetKind::Web => {
			Err(Error::Configuration(
				"web targets have nothing to install".to_owned(),
			))
		}
	}
}

/// How a desktop app is launched. Mobile apps use `start_app`.
///
/// # Errors
///
/// The identifier is not a path this desktop can launch.
pub fn open_plan(kind: TargetKind, identifier: &str) -> Result<ToolPlan, Error> {
	if identifier.trim().is_empty() {
		return Err(Error::Configuration(
			"open needs an identifier or path".to_owned(),
		));
	}

	match kind {
		TargetKind::Macos if identifier == "desktop" => {
			Err(Error::Configuration(
				"macOS opens an app path or a bundle id, not the desktop".to_owned(),
			))
		}
		TargetKind::Macos if identifier.contains('/') => {
			Ok(ToolPlan::Command {
				program: "open".to_owned(),
				args: vec!["-n".to_owned(), identifier.to_owned()],
				detach: false,
			})
		}
		TargetKind::Macos => {
			Ok(ToolPlan::Command {
				program: "open".to_owned(),
				args: vec!["-n".to_owned(), "-b".to_owned(), identifier.to_owned()],
				detach: false,
			})
		}
		TargetKind::Linux if identifier.contains('/') => {
			Ok(ToolPlan::Command {
				program: identifier.to_owned(),
				args: Vec::new(),
				detach: true,
			})
		}
		TargetKind::Linux => Err(Error::Configuration("linux apps open by path".to_owned())),
		TargetKind::Windows if identifier.contains('\\') || identifier.contains('/') => {
			Ok(ToolPlan::Command {
				program: "cmd".to_owned(),
				args: vec![
					"/D".to_owned(),
					"/C".to_owned(),
					"start".to_owned(),
					String::new(),
					identifier.to_owned(),
				],
				detach: false,
			})
		}
		TargetKind::Windows => Err(Error::Configuration("windows apps open by path".to_owned())),
		_ => {
			Err(Error::Configuration(
				"mobile apps open with start_app".to_owned(),
			))
		}
	}
}

/// W3C actions for a tap that does not move.
///
/// # Errors
///
/// A coordinate is negative.
pub fn tap_actions(x: i64, y: i64) -> Result<Value, Error> {
	point(x, y)?;
	Ok(pointer(
		"leptoskit-tap",
		&json!([
			{"type": "pointerMove", "duration": 0, "x": x, "y": y},
			{"type": "pointerDown", "button": 0},
			{"type": "pause", "duration": 50},
			{"type": "pointerUp", "button": 0}
		]),
	))
}

/// W3C actions for a long press held for `duration_ms`.
///
/// # Errors
///
/// A coordinate is negative, or the duration is zero.
pub fn long_press_actions(x: i64, y: i64, duration_ms: u64) -> Result<Value, Error> {
	point(x, y)?;
	positive_duration(duration_ms)?;
	Ok(pointer(
		"leptoskit-long-press",
		&json!([
			{"type": "pointerMove", "duration": 0, "x": x, "y": y},
			{"type": "pointerDown", "button": 0},
			{"type": "pause", "duration": duration_ms},
			{"type": "pointerUp", "button": 0}
		]),
	))
}

/// W3C actions for a one-pointer drag.
///
/// # Errors
///
/// A coordinate is negative, or the duration is zero.
pub fn pan_actions(start: Point, end: Point, duration_ms: u64) -> Result<Value, Error> {
	point(start.x, start.y)?;
	point(end.x, end.y)?;
	positive_duration(duration_ms)?;
	Ok(pointer(
		"leptoskit-pan",
		&json!([
			{"type": "pointerMove", "duration": 0, "x": start.x, "y": start.y},
			{"type": "pointerDown", "button": 0},
			{"type": "pointerMove", "duration": duration_ms, "x": end.x, "y": end.y},
			{"type": "pointerUp", "button": 0}
		]),
	))
}

/// W3C actions for two pointers moving from `start_span` apart to `end_span`.
///
/// # Errors
///
/// A coordinate is negative, a span is under 2, or the duration is zero.
pub fn pinch_actions(
	x: i64,
	y: i64,
	start_span: i64,
	end_span: i64,
	duration_ms: u64,
) -> Result<Value, Error> {
	let points = pinch_points(x, y, start_span, end_span)?;
	positive_duration(duration_ms)?;
	Ok(json!([
		pointer_source(
			"leptoskit-pinch-a",
			&json!([
				{"type": "pointerMove", "duration": 0, "x": points.0, "y": y},
				{"type": "pointerDown", "button": 0},
				{"type": "pointerMove", "duration": duration_ms, "x": points.2, "y": y},
				{"type": "pointerUp", "button": 0}
			])
		),
		pointer_source(
			"leptoskit-pinch-b",
			&json!([
				{"type": "pointerMove", "duration": 0, "x": points.1, "y": y},
				{"type": "pointerDown", "button": 0},
				{"type": "pointerMove", "duration": duration_ms, "x": points.3, "y": y},
				{"type": "pointerUp", "button": 0}
			])
		),
	]))
}

/// adb arguments for a tap.
///
/// # Errors
///
/// A coordinate is negative, or the serial is empty.
pub fn android_tap(device: &str, x: i64, y: i64) -> Result<ToolPlan, Error> {
	point(x, y)?;
	adb(
		device,
		vec![
			"shell".to_owned(),
			"input".to_owned(),
			"tap".to_owned(),
			x.to_string(),
			y.to_string(),
		],
	)
}

/// adb arguments for a long press: a swipe that does not move.
///
/// # Errors
///
/// A coordinate is negative, the duration is zero, or the serial is empty.
pub fn android_long_press(
	device: &str,
	x: i64,
	y: i64,
	duration_ms: u64,
) -> Result<ToolPlan, Error> {
	point(x, y)?;
	positive_duration(duration_ms)?;
	adb(device, swipe_args(x, y, x, y, duration_ms))
}

/// adb arguments for a pan.
///
/// # Errors
///
/// A coordinate is negative, the duration is zero, or the serial is empty.
pub fn android_pan(
	device: &str,
	start: Point,
	end: Point,
	duration_ms: u64,
) -> Result<ToolPlan, Error> {
	point(start.x, start.y)?;
	point(end.x, end.y)?;
	positive_duration(duration_ms)?;
	adb(
		device,
		swipe_args(start.x, start.y, end.x, end.y, duration_ms),
	)
}

/// How long `uinput` waits after registering before the first touch.
///
/// The input stack attaches the new touchscreen a few hundred milliseconds
/// after registration. Events sent earlier are dropped.
const UINPUT_READY_MS: u64 = 500;

/// adb arguments for a pinch.
///
/// Two `input swipe`s are two separate gestures. On API 35 the activity
/// receives one of them, with a single pointer, and the step still exits
/// successfully. `uinput` registers one direct touchscreen and reports both
/// slots in the same frame, which is a real two-pointer pinch. The program
/// is stdin, not a shell script: the only adb arguments are `uinput -`.
///
/// `width` and `height` are the display size in pixels (`wm size`). The
/// touchscreen's axes use that size, so a pixel in the gesture is a pixel
/// on screen.
///
/// # Errors
///
/// A coordinate or span is unusable, the pinch leaves the display, the
/// duration is zero, the display is smaller than 2 pixels, or the serial is
/// empty.
pub fn android_pinch(
	device: &str,
	display: (i64, i64),
	x: i64,
	y: i64,
	start_span: i64,
	end_span: i64,
	duration_ms: u64,
) -> Result<ToolPlan, Error> {
	let program = android_pinch_program(display, x, y, start_span, end_span, duration_ms)?;
	if device.trim().is_empty() {
		return Err(Error::Configuration(
			"android input needs a device serial".to_owned(),
		));
	}
	Ok(ToolPlan::Adb {
		device: device.to_owned(),
		args: vec!["shell".to_owned(), "uinput".to_owned(), "-".to_owned()],
		stdin: Some(program),
	})
}

/// The `uinput` program for one pinch, as concatenated JSON commands.
fn android_pinch_program(
	display: (i64, i64),
	x: i64,
	y: i64,
	start_span: i64,
	end_span: i64,
	duration_ms: u64,
) -> Result<String, Error> {
	let (width, height) = display;
	if width < 2 || height < 2 {
		return Err(Error::Configuration(
			"a pinch needs the display size".to_owned(),
		));
	}
	let (left_start, right_start, left_end, right_end) = pinch_points(x, y, start_span, end_span)?;
	positive_duration(duration_ms)?;
	for value in [left_start, right_start, left_end, right_end] {
		if value >= width {
			return Err(Error::Configuration(
				"a pinch must stay on the screen".to_owned(),
			));
		}
	}
	if y >= height {
		return Err(Error::Configuration(
			"a pinch must stay on the screen".to_owned(),
		));
	}

	// `type` has to be written before `data`. serde_json sorts object keys,
	// which puts `data` first and uinput rejects the configuration.
	let mut lines = vec![touchscreen(width, height)];
	let mut commands = vec![
		json!({ "id": 1, "command": "delay", "duration": UINPUT_READY_MS }),
		inject(down_events(left_start, right_start, y)),
	];
	let steps = (duration_ms / 16).max(1);
	let mut scheduled = 0u64;
	for step in 1..=steps {
		let slice = if step == steps {
			duration_ms - scheduled
		} else {
			duration_ms / steps
		};
		scheduled += slice;
		let left = along(left_start, left_end, step, steps);
		let right = along(right_start, right_end, step, steps);
		commands.push(json!({ "id": 1, "command": "delay", "duration": slice }));
		commands.push(inject(move_events(left, right, y)));
	}
	commands.push(inject(lift_events()));
	lines.extend(commands.iter().map(ToString::to_string));
	Ok(lines.join("\n"))
}

fn along(start: i64, end: i64, step: u64, steps: u64) -> i64 {
	start
		+ (end - start) * i64::try_from(step).unwrap_or(i64::MAX)
			/ i64::try_from(steps).unwrap_or(1)
}

fn touchscreen(width: i64, height: i64) -> String {
	format!(
		r#"{{"id":1,"command":"register","name":"leptoskit-pinch","vid":1,"pid":1,"bus":"usb","configuration":[{{"type":"UI_SET_EVBIT","data":["EV_ABS","EV_KEY"]}},{{"type":"UI_SET_KEYBIT","data":["BTN_TOUCH"]}},{{"type":"UI_SET_ABSBIT","data":["ABS_MT_SLOT","ABS_MT_TRACKING_ID","ABS_MT_POSITION_X","ABS_MT_POSITION_Y","ABS_MT_PRESSURE"]}},{{"type":"UI_SET_PROPBIT","data":["INPUT_PROP_DIRECT"]}}],"abs_info":[{{"code":"ABS_MT_SLOT","info":{{"minimum":0,"maximum":9,"fuzz":0,"flat":0,"resolution":0}}}},{{"code":"ABS_MT_TRACKING_ID","info":{{"minimum":0,"maximum":65535,"fuzz":0,"flat":0,"resolution":0}}}},{{"code":"ABS_MT_POSITION_X","info":{{"minimum":0,"maximum":{x_max},"fuzz":0,"flat":0,"resolution":0}}}},{{"code":"ABS_MT_POSITION_Y","info":{{"minimum":0,"maximum":{y_max},"fuzz":0,"flat":0,"resolution":0}}}},{{"code":"ABS_MT_PRESSURE","info":{{"minimum":0,"maximum":255,"fuzz":0,"flat":0,"resolution":0}}}}]}}"#,
		x_max = width - 1,
		y_max = height - 1,
	)
}

fn inject(events: Vec<Value>) -> Value {
	let mut command = json!({ "id": 1, "command": "inject" });
	command["events"] = Value::Array(events);
	command
}

fn down_events(left: i64, right: i64, y: i64) -> Vec<Value> {
	let mut events = Vec::new();
	contact(&mut events, 0, 1, left, y);
	contact(&mut events, 1, 2, right, y);
	event(&mut events, "EV_KEY", "BTN_TOUCH", 1);
	event(&mut events, "EV_SYN", "SYN_REPORT", 0);
	events
}

fn move_events(left: i64, right: i64, y: i64) -> Vec<Value> {
	let mut events = Vec::new();
	event(&mut events, "EV_ABS", "ABS_MT_SLOT", 0);
	event(&mut events, "EV_ABS", "ABS_MT_POSITION_X", left);
	event(&mut events, "EV_ABS", "ABS_MT_POSITION_Y", y);
	event(&mut events, "EV_ABS", "ABS_MT_SLOT", 1);
	event(&mut events, "EV_ABS", "ABS_MT_POSITION_X", right);
	event(&mut events, "EV_ABS", "ABS_MT_POSITION_Y", y);
	event(&mut events, "EV_SYN", "SYN_REPORT", 0);
	events
}

fn lift_events() -> Vec<Value> {
	let mut events = Vec::new();
	event(&mut events, "EV_ABS", "ABS_MT_SLOT", 0);
	event(&mut events, "EV_ABS", "ABS_MT_TRACKING_ID", -1);
	event(&mut events, "EV_ABS", "ABS_MT_SLOT", 1);
	event(&mut events, "EV_ABS", "ABS_MT_TRACKING_ID", -1);
	event(&mut events, "EV_KEY", "BTN_TOUCH", 0);
	event(&mut events, "EV_SYN", "SYN_REPORT", 0);
	events
}

fn contact(events: &mut Vec<Value>, slot: i64, tracking: i64, x: i64, y: i64) {
	event(events, "EV_ABS", "ABS_MT_SLOT", slot);
	event(events, "EV_ABS", "ABS_MT_TRACKING_ID", tracking);
	event(events, "EV_ABS", "ABS_MT_POSITION_X", x);
	event(events, "EV_ABS", "ABS_MT_POSITION_Y", y);
	event(events, "EV_ABS", "ABS_MT_PRESSURE", 80);
}

fn event(events: &mut Vec<Value>, kind: &str, code: &str, value: i64) {
	events.push(json!(kind));
	events.push(json!(code));
	events.push(json!(value));
}

/// `xdotool` arguments for a tap.
///
/// # Errors
///
/// A coordinate is negative.
pub fn xdotool_tap(x: i64, y: i64) -> Result<ToolPlan, Error> {
	point(x, y)?;
	let mut args = pointer_move(x, y);
	args.extend(["click".to_owned(), "1".to_owned()]);
	Ok(xdotool(args))
}

/// `xdotool` arguments for a long press.
///
/// # Errors
///
/// A coordinate is negative, or the duration is zero.
pub fn xdotool_long_press(x: i64, y: i64, duration_ms: u64) -> Result<ToolPlan, Error> {
	point(x, y)?;
	positive_duration(duration_ms)?;
	let mut args = pointer_move(x, y);
	args.extend([
		"mousedown".to_owned(),
		"1".to_owned(),
		"sleep".to_owned(),
		seconds(duration_ms),
		"mouseup".to_owned(),
		"1".to_owned(),
	]);
	Ok(xdotool(args))
}

/// `xdotool` arguments for a pan.
///
/// # Errors
///
/// A coordinate is negative, or the duration is zero.
pub fn xdotool_pan(start: Point, end: Point, duration_ms: u64) -> Result<ToolPlan, Error> {
	point(start.x, start.y)?;
	point(end.x, end.y)?;
	positive_duration(duration_ms)?;
	// xdotool's drag has no duration; the move is still one pointer traveling
	// the pan distance, which is what the arena measures.
	let _ = duration_ms;
	let mut args = pointer_move(start.x, start.y);
	args.extend([
		"mousedown".to_owned(),
		"1".to_owned(),
		"mousemove".to_owned(),
		end.x.to_string(),
		end.y.to_string(),
		"mouseup".to_owned(),
		"1".to_owned(),
	]);
	Ok(xdotool(args))
}

/// A pointer move without `--sync`.
///
/// `xdotool mousemove --sync` waits for a motion event. When the pointer is
/// already at that point the event never arrives, and the wait is about
/// fifteen seconds, so the next gesture at the same point stalls the run.
fn pointer_move(x: i64, y: i64) -> Vec<String> {
	vec!["mousemove".to_owned(), x.to_string(), y.to_string()]
}

fn swipe_args(x1: i64, y1: i64, x2: i64, y2: i64, duration_ms: u64) -> Vec<String> {
	vec![
		"shell".to_owned(),
		"input".to_owned(),
		"swipe".to_owned(),
		x1.to_string(),
		y1.to_string(),
		x2.to_string(),
		y2.to_string(),
		duration_ms.to_string(),
	]
}

fn adb(device: &str, args: Vec<String>) -> Result<ToolPlan, Error> {
	if device.trim().is_empty() {
		return Err(Error::Configuration(
			"android input needs a device serial".to_owned(),
		));
	}

	Ok(ToolPlan::Adb {
		device: device.to_owned(),
		args,
		stdin: None,
	})
}

fn xdotool(args: Vec<String>) -> ToolPlan {
	ToolPlan::Command {
		program: "xdotool".to_owned(),
		args,
		detach: false,
	}
}

fn pointer(id: &str, actions: &Value) -> Value {
	json!([pointer_source(id, actions)])
}

fn pointer_source(id: &str, actions: &Value) -> Value {
	json!({
		"type": "pointer",
		"id": id,
		"parameters": {"pointerType": "touch"},
		"actions": actions,
	})
}

/// Left start, right start, left end, right end.
fn pinch_points(
	x: i64,
	y: i64,
	start_span: i64,
	end_span: i64,
) -> Result<(i64, i64, i64, i64), Error> {
	point(x, y)?;

	if start_span < 2 || end_span < 2 {
		return Err(Error::Configuration(
			"a pinch span must be at least 2".to_owned(),
		));
	}

	let left_start = x - start_span / 2;
	let right_start = x + start_span / 2;
	let left_end = x - end_span / 2;
	let right_end = x + end_span / 2;

	for value in [left_start, right_start, left_end, right_end] {
		if value < 0 {
			return Err(Error::Configuration(
				"a pinch must stay on the screen".to_owned(),
			));
		}
	}

	Ok((left_start, right_start, left_end, right_end))
}

fn point(x: i64, y: i64) -> Result<(), Error> {
	if x < 0 || y < 0 {
		Err(Error::Configuration(
			"coordinates must be at least 0".to_owned(),
		))
	} else {
		Ok(())
	}
}

fn positive_duration(duration_ms: u64) -> Result<(), Error> {
	if duration_ms == 0 {
		Err(Error::Configuration(
			"duration_ms must be at least 1".to_owned(),
		))
	} else {
		Ok(())
	}
}

fn seconds(duration_ms: u64) -> String {
	format!("{:.3}", duration_ms as f64 / 1000.0)
}

/// A target with the fields a suite test needs and nothing attached.
#[cfg(test)]
fn bare(kind: TargetKind, device: Option<&str>) -> Target {
	Target {
		kind,
		device: device.map(str::to_owned),
		application: None,
		url: None,
		browser: None,
		endpoint: None,
		capabilities: std::collections::BTreeMap::new(),
		headless: true,
		startup_seconds: 30,
		command_seconds: 5,
		wait_seconds: 5,
		artifacts: std::path::PathBuf::from("target/leptoskit-control"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn request<'a>(
		kind: TargetKind,
		device: Option<&'a str>,
		identifier: &'a str,
		install: Option<&'a str>,
		injector: Injector,
	) -> SuiteRequest<'a> {
		SuiteRequest {
			target: bare(kind, device),
			identifier,
			install,
			injector,
		}
	}

	#[test]
	fn only_android_and_linux_feed_the_arena() {
		assert!(arena_fed(TargetKind::Android));
		assert!(arena_fed(TargetKind::Linux));
		assert!(!arena_fed(TargetKind::IosSimulator));
		assert!(!arena_fed(TargetKind::IosDevice));
		assert!(!arena_fed(TargetKind::Macos));
		assert!(!arena_fed(TargetKind::Windows));
		assert!(!arena_fed(TargetKind::Web));
	}

	#[test]
	fn each_native_target_has_one_default_injector_and_the_web_has_none() {
		assert_eq!(
			default_injector(TargetKind::Android).unwrap(),
			Injector::Android
		);
		assert_eq!(
			default_injector(TargetKind::Linux).unwrap(),
			Injector::Xdotool
		);
		assert_eq!(
			default_injector(TargetKind::IosSimulator).unwrap(),
			Injector::Driver
		);
		assert_eq!(
			default_injector(TargetKind::IosDevice).unwrap(),
			Injector::Driver
		);
		assert_eq!(
			default_injector(TargetKind::Macos).unwrap(),
			Injector::Driver
		);
		assert_eq!(
			default_injector(TargetKind::Windows).unwrap(),
			Injector::Driver
		);
		assert!(default_injector(TargetKind::Web).is_err());
	}

	#[test]
	fn android_sanity_installs_opens_and_injects_every_gesture() {
		let suite = suite(request(
			TargetKind::Android,
			Some("emulator-5554"),
			"dev.example.app",
			Some("/tmp/app.apk"),
			Injector::Android,
		))
		.unwrap();
		assert!(suite.arena);
		assert_eq!(suite.omitted.as_slice(), []);
		assert_eq!(
			suite.gestures,
			[
				SanityGesture::Tap,
				SanityGesture::LongPress,
				SanityGesture::Pan,
				SanityGesture::Pinch,
			]
		);
		let names: Vec<_> = suite.document.steps.iter().map(Step::action_name).collect();
		assert_eq!(
			names,
			[
				"install_app",
				"open_app",
				"inject_tap",
				"inject_long_press",
				"inject_pan",
				"inject_pinch",
				"device_screenshot",
			]
		);
		assert!(
			suite
				.notes
				.iter()
				.any(|note| note.contains("Android and Linux"))
		);
	}

	#[test]
	fn phones_and_desktops_use_the_driver_and_a_mouse_cannot_pinch() {
		let phone = suite(request(
			TargetKind::IosDevice,
			Some("UDID"),
			"dev.example.app",
			None,
			Injector::Driver,
		))
		.unwrap();
		assert_eq!(phone.omitted.as_slice(), []);
		assert!(matches!(
			phone.document.target.application,
			Some(Application::Identifier(id)) if id == "dev.example.app"
		));
		assert_eq!(
			phone.document.steps.last().unwrap().action_name(),
			"screenshot"
		);

		let mac = suite(request(
			TargetKind::Macos,
			None,
			"/Applications/App.app",
			None,
			Injector::Driver,
		))
		.unwrap();
		assert_eq!(mac.omitted.len(), 1);
		assert_eq!(mac.omitted[0].gesture, SanityGesture::Pinch);
		assert!(matches!(
			mac.document.target.application,
			Some(Application::Path(path)) if path == "/Applications/App.app"
		));
		assert!(
			mac.notes
				.iter()
				.any(|note| note.contains("presses and clicks"))
		);

		let windows = suite(request(
			TargetKind::Windows,
			None,
			r"C:\App\App.exe",
			None,
			Injector::Driver,
		))
		.unwrap();
		assert_eq!(windows.omitted[0].gesture, SanityGesture::Pinch);
		assert!(matches!(
			windows.document.target.application,
			Some(Application::Path(_))
		));

		let desktop = suite(request(
			TargetKind::Linux,
			None,
			"desktop",
			None,
			Injector::Driver,
		))
		.unwrap();
		assert!(matches!(
			desktop.document.target.application,
			Some(Application::Desktop)
		));
		assert!(desktop.gestures.contains(&SanityGesture::Pinch));
	}

	#[test]
	fn linux_pointer_injection_skips_pinch_and_screenshots() {
		let suite = suite(request(
			TargetKind::Linux,
			None,
			"/usr/local/bin/app",
			Some("/usr/local/bin/app"),
			Injector::Xdotool,
		))
		.unwrap();
		assert_eq!(suite.omitted[0].gesture, SanityGesture::Pinch);
		assert!(!suite.gestures.contains(&SanityGesture::Pinch));
		assert!(suite.notes.iter().any(|note| note.contains("--driver")));
		let names: Vec<_> = suite.document.steps.iter().map(Step::action_name).collect();
		assert_eq!(
			names,
			[
				"install_app",
				"open_app",
				"inject_tap",
				"inject_long_press",
				"inject_pan",
			]
		);
	}

	#[test]
	fn a_suite_rejects_a_mismatched_injector_and_a_missing_device() {
		assert!(
			suite(request(
				TargetKind::Linux,
				None,
				"/bin/app",
				None,
				Injector::Android
			))
			.is_err()
		);
		assert!(
			suite(request(
				TargetKind::Android,
				None,
				"dev.app",
				None,
				Injector::Android
			))
			.is_err()
		);
		assert!(
			suite(request(
				TargetKind::Android,
				Some("  "),
				"dev.app",
				None,
				Injector::Android
			))
			.is_err()
		);
		assert!(
			suite(request(
				TargetKind::Macos,
				None,
				"  ",
				None,
				Injector::Driver
			))
			.is_err()
		);
		assert!(
			suite(request(
				TargetKind::Macos,
				None,
				"dev.app",
				Some("  "),
				Injector::Driver
			))
			.is_err()
		);
		assert!(
			suite(request(
				TargetKind::Web,
				None,
				"https://example.com",
				None,
				Injector::Driver
			))
			.is_err()
		);
	}

	#[test]
	fn install_plans_name_the_platform_tool() {
		let simulator = install_plan(TargetKind::IosSimulator, "UDID", "App.app").unwrap();
		assert!(matches!(simulator, ToolPlan::Command { program, args, .. }
			if program == "xcrun" && args[1] == "install" && args[2] == "UDID"));
		let device = install_plan(TargetKind::IosDevice, "UDID", "App.ipa").unwrap();
		assert!(matches!(device, ToolPlan::Command { args, .. } if args[2] == "install"));
		let android = install_plan(TargetKind::Android, "emulator-5554", "app.apk").unwrap();
		assert!(matches!(android, ToolPlan::Adb { device, args, .. }
			if device == "emulator-5554" && args == ["install", "-r", "app.apk"]));
		assert!(install_plan(TargetKind::Macos, "x", "y").is_err());
		assert!(install_plan(TargetKind::Linux, "x", "y").is_err());
		assert!(install_plan(TargetKind::Windows, "x", "y").is_err());
		assert!(install_plan(TargetKind::Web, "x", "y").is_err());
		assert!(install_plan(TargetKind::Android, " ", "app.apk").is_err());
		assert!(install_plan(TargetKind::Android, "serial", " ").is_err());
	}

	#[test]
	fn desktop_open_plans_match_the_platform() {
		let mac_path = open_plan(TargetKind::Macos, "/Applications/App.app").unwrap();
		assert!(
			matches!(mac_path, ToolPlan::Command { program, args, detach: false }
			if program == "open" && args == ["-n", "/Applications/App.app"])
		);
		let mac_id = open_plan(TargetKind::Macos, "dev.example.app").unwrap();
		assert!(
			matches!(mac_id, ToolPlan::Command { args, .. } if args == ["-n", "-b", "dev.example.app"])
		);
		assert!(open_plan(TargetKind::Macos, "desktop").is_err());
		let linux = open_plan(TargetKind::Linux, "/usr/bin/app").unwrap();
		assert!(
			matches!(linux, ToolPlan::Command { program, detach: true, .. } if program == "/usr/bin/app")
		);
		assert!(open_plan(TargetKind::Linux, "app").is_err());
		let windows = open_plan(TargetKind::Windows, r"C:\App\App.exe").unwrap();
		assert!(matches!(windows, ToolPlan::Command { program, .. } if program == "cmd"));
		assert!(open_plan(TargetKind::Windows, "App.exe").is_err());
		assert!(open_plan(TargetKind::Android, "dev.app").is_err());
		assert!(open_plan(TargetKind::IosSimulator, "dev.app").is_err());
		assert!(open_plan(TargetKind::IosDevice, "dev.app").is_err());
		assert!(open_plan(TargetKind::Web, "https://example.com").is_err());
		assert!(open_plan(TargetKind::Macos, " ").is_err());
	}

	#[test]
	fn driver_actions_hold_drag_and_pinch_past_the_arena_thresholds() {
		let tap = tap_actions(ORIGIN_X, ORIGIN_Y).unwrap();
		assert_eq!(tap[0]["actions"][2]["duration"], 50);
		let hold = long_press_actions(10, 20, HOLD_MS).unwrap();
		assert_eq!(hold[0]["actions"][2]["duration"], HOLD_MS);
		let pan = pan_actions(
			Point { x: 10, y: 20 },
			Point {
				x: 10 + PAN_DISTANCE,
				y: 20,
			},
			MOVE_MS,
		)
		.unwrap();
		assert_eq!(pan[0]["actions"][2]["x"], 10 + PAN_DISTANCE);
		const _: () = assert!(PAN_DISTANCE > 18);
		const _: () = assert!(HOLD_MS > 500);
		let pinch = pinch_actions(200, 400, PINCH_START, PINCH_END, MOVE_MS).unwrap();
		assert_eq!(pinch.as_array().unwrap().len(), 2);
		assert_eq!(pinch[0]["id"], "leptoskit-pinch-a");
		assert_eq!(pinch[1]["actions"][0]["x"], 200 + PINCH_START / 2);
		assert_eq!(pinch[0]["actions"][2]["x"], 200 - PINCH_END / 2);
		assert!(tap_actions(-1, 0).is_err());
		assert!(long_press_actions(0, 0, 0).is_err());
		assert!(pan_actions(Point { x: 0, y: -1 }, Point { x: 1, y: 1 }, 10).is_err());
		assert!(pan_actions(Point { x: 0, y: 0 }, Point { x: 1, y: 1 }, 0).is_err());
		assert!(pinch_actions(10, 10, 1, 80, 10).is_err());
		assert!(pinch_actions(10, 10, 80, 400, 10).is_err());
		assert!(pinch_actions(10, 10, 80, 80, 0).is_err());
	}

	#[test]
	fn android_injectors_format_only_checked_numbers() {
		let tap = android_tap("emulator-5554", 12, 34).unwrap();
		assert!(matches!(tap, ToolPlan::Adb { args, .. } if args[2] == "tap" && args[3] == "12"));
		let hold = android_long_press("emulator-5554", 12, 34, 600).unwrap();
		assert!(matches!(hold, ToolPlan::Adb { args, .. }
			if args[2] == "swipe" && args[3] == "12" && args[5] == "12" && args[7] == "600"));
		let pan = android_pan(
			"emulator-5554",
			Point { x: 1, y: 2 },
			Point { x: 90, y: 2 },
			300,
		)
		.unwrap();
		assert!(matches!(pan, ToolPlan::Adb { args, .. } if args[5] == "90"));
		assert!(android_tap(" ", 1, 1).is_err());
		assert!(android_tap("serial", -1, 1).is_err());
		assert!(android_long_press("serial", 1, 1, 0).is_err());
		assert!(android_pan("serial", Point { x: 0, y: 0 }, Point { x: -1, y: 0 }, 10).is_err());
		assert!(android_pinch("serial", (320, 640), 200, 400, 80, 160, 0).is_err());
	}

	#[test]
	fn a_pinch_is_one_uinput_stream_with_two_pointers() {
		// Two `input swipe`s each own one pointer. On API 35 they do not
		// overlap, so the activity sees a single finger while the step
		// reports success. A pinch has to be one uinput stream with both
		// slots in the same report.
		let pinch = android_pinch("emulator-5554", (320, 640), 200, 400, 80, 160, 300).unwrap();
		let ToolPlan::Adb { args, stdin, .. } = pinch else {
			panic!("pinch is an adb plan");
		};
		assert_eq!(args, ["shell", "uinput", "-"]);
		let script = stdin.expect("the uinput program is the tool's stdin");
		assert!(
			!script.contains("input swipe"),
			"a backgrounded pair of swipes is one pointer"
		);
		let commands = json_commands(&script);
		let injects: Vec<_> = commands
			.iter()
			.filter(|command| command["command"] == "inject")
			.collect();
		assert!(injects.len() >= 2, "down and the later positions");
		let down = injects[0]["events"].as_array().expect("events");
		assert!(event_has(down, "ABS_MT_SLOT", 0));
		assert!(event_has(down, "ABS_MT_SLOT", 1));
		assert!(event_has(down, "ABS_MT_POSITION_X", 160));
		assert!(event_has(down, "ABS_MT_POSITION_X", 240));
		let end = injects[injects.len() - 2]["events"]
			.as_array()
			.expect("last move");
		assert!(event_has(end, "ABS_MT_SLOT", 0));
		assert!(event_has(end, "ABS_MT_SLOT", 1));
		assert!(event_has(end, "ABS_MT_POSITION_X", 120));
		assert!(event_has(end, "ABS_MT_POSITION_X", 280));
		let travel: u64 = commands
			.iter()
			.skip_while(|command| command["command"] != "inject")
			.filter(|command| command["command"] == "delay")
			.map(|command| command["duration"].as_u64().unwrap_or(0))
			.sum();
		assert_eq!(
			travel, 300,
			"the fingers take the requested duration to move"
		);
		assert!(android_pinch("emulator-5554", (100, 100), 200, 400, 80, 160, 300).is_err());
		assert!(android_pinch("emulator-5554", (1000, 30), 200, 50, 20, 20, 300).is_err());
		assert!(android_pinch("emulator-5554", (1, 640), 0, 0, 2, 2, 300).is_err());
		assert!(android_pinch(" ", (320, 640), 200, 400, 80, 160, 300).is_err());
	}

	#[test]
	fn xdotool_names_its_points_without_a_sync_wait() {
		// `--sync` waits for a motion event. When the pointer is already at
		// the point, that event never comes and xdotool sits for about
		// fifteen seconds, so a second gesture at the same point stalls.
		let tool = xdotool_tap(3, 4).unwrap();
		assert!(matches!(tool, ToolPlan::Command { program, args, .. }
			if program == "xdotool"
				&& args == ["mousemove", "3", "4", "click", "1"]
				&& !args.contains(&"--sync".to_owned())));
		let hold = xdotool_long_press(3, 4, 600).unwrap();
		assert!(matches!(hold, ToolPlan::Command { args, .. }
			if args == ["mousemove", "3", "4", "mousedown", "1", "sleep", "0.600", "mouseup", "1"]
				&& !args.contains(&"--sync".to_owned())));
		let pan = xdotool_pan(Point { x: 3, y: 4 }, Point { x: 83, y: 4 }, 300).unwrap();
		assert!(matches!(pan, ToolPlan::Command { args, .. }
			if args == [
				"mousemove", "3", "4", "mousedown", "1", "mousemove", "83", "4", "mouseup", "1"
			] && !args.contains(&"--sync".to_owned())));
		assert!(xdotool_tap(-1, 0).is_err());
		assert!(xdotool_long_press(0, 0, 0).is_err());
		assert!(xdotool_pan(Point { x: 0, y: 0 }, Point { x: 1, y: 1 }, 0).is_err());
	}

	fn json_commands(script: &str) -> Vec<Value> {
		serde_json::Deserializer::from_str(script)
			.into_iter()
			.collect::<Result<Vec<_>, _>>()
			.expect("uinput program is concatenated JSON")
	}

	fn event_has(events: &[Value], code: &str, value: i64) -> bool {
		events
			.chunks(3)
			.any(|triple| triple.len() == 3 && triple[1] == code && triple[2] == value)
	}
}
