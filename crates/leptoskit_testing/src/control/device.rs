//! Device steps: what `simctl`, `devicectl`, and `adb` can do with no driver
//! session at all.

use std::collections::BTreeMap;
use std::process::Child;
use std::process::Command;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use super::Error;
use super::Point;
use super::Target;
use super::TargetKind;
use super::tools;

fn device_id(target: &Target) -> Result<&str, Error> {
	target
		.device
		.as_deref()
		.filter(|id| !id.trim().is_empty())
		.ok_or_else(|| {
			Error::Configuration(format!(
				"{:?} device steps require target.device",
				target.kind
			))
		})
}

fn only(target: &Target, kind: TargetKind, what: &str) -> Result<(), Error> {
	if target.kind == kind {
		Ok(())
	} else {
		Err(Error::Configuration(format!(
			"{what} needs a {kind:?} target"
		)))
	}
}

/// Boots the target simulator and waits for it to finish booting.
pub fn boot(target: &Target, timeout: Duration) -> Result<(), Error> {
	only(target, TargetKind::IosSimulator, "booting")?;
	let id = device_id(target)?;
	let mut command = tools::simctl();
	command.arg("boot").arg(id);
	// An already-booted simulator answers "Unable to boot device in state
	// Booted"; waiting for it below is still the right outcome.
	if let Err(error) = super::tool(&mut command, timeout)
		&& !error.to_string().contains("Booted")
	{
		return Err(error);
	}
	let mut status = tools::simctl();

	status.args(["bootstatus", id, "-b"]);
	super::tool(&mut status, timeout)?;
	Ok(())
}

/// Shuts the target simulator down, or stops the emulator behind an
/// `emulator-PORT` serial.
pub fn shutdown(target: &Target, timeout: Duration) -> Result<(), Error> {
	let id = device_id(target)?;

	match target.kind {
		TargetKind::IosSimulator => {
			let mut command = tools::simctl();
			command.args(["shutdown", id]);
			if let Err(error) = super::tool(&mut command, timeout)
				&& !error.to_string().contains("Shutdown")
			{
				return Err(error);
			}
			Ok(())
		}

		TargetKind::Android if id.starts_with("emulator-") => {
			let mut command = tools::adb(id);
			command.args(["emu", "kill"]);
			super::tool(&mut command, timeout)?;
			Ok(())
		}

		TargetKind::Android => {
			Err(Error::Configuration(
				"shutting down a physical Android device is not a thing; stop the app instead"
					.to_owned(),
			))
		}
		kind => {
			Err(Error::Configuration(format!(
				"shutdown needs an ios_simulator or android emulator target, not {kind:?}"
			)))
		}
	}
}

/// Starts an emulator detached — it outlives this run — and waits for it to
/// finish booting.
pub fn start_emulator(
	target: &Target,
	avd: &str,
	port: Option<u16>,
	timeout: Duration,
) -> Result<(), Error> {
	only(target, TargetKind::Android, "starting an emulator")?;
	let serials = tools::android_list(timeout)
		.unwrap_or_default()
		.lines()
		.filter_map(|line| line.split_whitespace().next().map(str::to_owned))
		.collect::<Vec<_>>();

	let port = match port {
		Some(port) => port,
		None => {
			(5554..=5682)
				.step_by(2)
				.find(|candidate| {
					!serials
						.iter()
						.any(|serial| serial == &format!("emulator-{candidate}"))
				})
				.ok_or_else(|| {
					Error::Configuration("no free emulator port between 5554 and 5682".to_owned())
				})?
		}
	};

	if !(5554..=5682).contains(&port) || port % 2 != 0 {
		return Err(Error::Configuration(
			"emulator port must be even and between 5554 and 5682".to_owned(),
		));
	}

	let serial = format!("emulator-{port}");

	if serials.iter().any(|existing| existing == &serial) {
		return Err(Error::Configuration(format!(
			"device {serial} already exists"
		)));
	}

	for existing in serials.iter().filter(|item| item.starts_with("emulator-")) {
		if emulator_avd_name(existing).as_deref() == Some(avd) {
			return Err(Error::Configuration(format!(
				"{avd} is already running as {existing}"
			)));
		}
	}

	let mut command = tools::android_tool("emulator");
	command
		.args(["-avd", avd, "-port", &port.to_string(), "-no-audio"])
		// Full snapshots would make two runs of one journey diverge.
		.arg("-no-snapshot-save");
	let mut child = tools::spawn_detached_child(&mut command)?;
	// `adb wait-for-device` blocks for the whole deadline even when the
	// emulator process has already exited, which is how a locked AVD turns
	// into a five-minute hang.
	wait_for_emulator(&serial, &mut child, timeout)?;
	let boot = "until [ \"$(getprop sys.boot_completed)\" = 1 ]; do sleep 1; done";
	let mut completed = tools::adb(&serial);
	completed.args([
		"shell",
		&format!("timeout {} sh -c '{boot}'", timeout.as_secs()),
	]);
	super::tool(&mut completed, timeout)?;
	Ok(())
}

/// How long to wait for an Android activity to take focus after `am start`.
const ANDROID_FOCUS_GRACE: Duration = Duration::from_secs(2);

/// How long to keep waiting after the focus line updates. On an API 35
/// emulator the activity is named as the focused window a few hundred
/// milliseconds before it accepts a tap, so the first gesture is dropped.
const ANDROID_FOCUS_SETTLE: Duration = Duration::from_millis(500);

/// Polls until `serial` is online, or the emulator process exits.
fn wait_for_emulator(serial: &str, child: &mut Child, timeout: Duration) -> Result<(), Error> {
	let deadline = Instant::now() + timeout;
	loop {
		match child.try_wait() {
			Ok(Some(status)) => {
				return Err(Error::Tool {
					name: "emulator".to_owned(),
					detail: format!("exited ({status}) before {serial} was ready"),
				});
			}
			Ok(None) => {}
			Err(error) => {
				return Err(Error::Tool {
					name: "emulator".to_owned(),
					detail: error.to_string(),
				});
			}
		}

		if emulator_ready(serial) {
			return Ok(());
		}

		if Instant::now() >= deadline {
			return Err(Error::Tool {
				name: "emulator".to_owned(),
				detail: format!(
					"{serial} was not ready within {} seconds",
					timeout.as_secs()
				),
			});
		}

		thread::sleep(Duration::from_millis(200));
	}
}

fn emulator_ready(serial: &str) -> bool {
	let mut command = tools::adb(serial);
	command.arg("get-state");
	match tools::run(&mut command, Duration::from_secs(2)) {
		Ok(output) => output.success() && output.stdout.trim() == "device",
		Err(_) => false,
	}
}

/// The name `adb emu avd name` prints, before its trailing `OK`.
fn parse_emulator_avd_name(stdout: &str) -> Option<&str> {
	stdout
		.lines()
		.map(str::trim)
		.find(|line| !line.is_empty() && *line != "OK")
}

fn emulator_avd_name(serial: &str) -> Option<String> {
	let mut command = tools::adb(serial);
	command.args(["emu", "avd", "name"]);
	let output = tools::run(&mut command, Duration::from_secs(3)).ok()?;
	parse_emulator_avd_name(&output.stdout).map(str::to_owned)
}

/// Waits until `package` is the focused window, or `grace` passes.
fn wait_until_focused(serial: &str, package: &str, grace: Duration) {
	let deadline = Instant::now() + grace;
	loop {
		if android_window_focused(serial, package) {
			thread::sleep(ANDROID_FOCUS_SETTLE);
			return;
		}

		if Instant::now() >= deadline {
			return;
		}

		thread::sleep(Duration::from_millis(50));
	}
}

fn android_window_focused(serial: &str, package: &str) -> bool {
	let mut command = tools::adb(serial);
	command.args(["shell", "dumpsys", "window"]);
	let Ok(output) = tools::run(&mut command, Duration::from_secs(2)) else {
		return false;
	};
	window_focus_contains(&output.stdout, package)
}

fn package_of(identifier: &str) -> &str {
	identifier
		.split('/')
		.next()
		.filter(|name| !name.is_empty())
		.unwrap_or(identifier)
}

/// Whether `dump` from `dumpsys window` shows `package` as the focused window.
fn window_focus_contains(dump: &str, package: &str) -> bool {
	let marker = format!("{package}/");
	dump.lines().any(|line| {
		let line = line.trim();
		line.starts_with("mCurrentFocus=") && line.contains(&marker)
	})
}

/// Launches an installed app and returns the tool's output, which carries
/// the process id on simulators.
pub fn start_app(
	target: &Target,
	identifier: &str,
	arguments: &[String],
	environment: &BTreeMap<String, String>,
	timeout: Duration,
) -> Result<String, Error> {
	let id = device_id(target)?;

	match target.kind {
		TargetKind::IosSimulator => {
			let mut command = tools::simctl();
			command.arg("launch").arg(id).arg(identifier);
			for (name, value) in environment {
				// simctl passes child environment through prefixed variables.
				command.env(format!("SIMCTL_CHILD_{name}"), value);
			}

			command.args(arguments);
			Ok(super::tool(&mut command, timeout)?.stdout)
		}
		TargetKind::IosDevice => {
			if !arguments.is_empty() || !environment.is_empty() {
				return Err(Error::Configuration(
					"devicectl launches take no arguments or environment".to_owned(),
				));
			}

			let mut command = tools::devicectl();
			command
				.args(["device", "process", "launch"])
				.arg("--device")
				.arg(id)
				.arg(identifier);
			Ok(super::tool(&mut command, timeout)?.stdout)
		}
		TargetKind::Android => {
			if !environment.is_empty() {
				return Err(Error::Configuration(
					"Android launches take no environment through am".to_owned(),
				));
			}

			let component = launcher_activity(id, identifier, timeout)?;
			let mut command = tools::adb(id);
			command
				.args(["shell", "am", "start", "-n", &component])
				.args(arguments);
			let launched = super::tool(&mut command, timeout)?.stdout;
			// `am start -W` can return while the launcher is still focused.
			// A gesture injected in that gap never reaches the app.
			wait_until_focused(id, package_of(identifier), timeout.min(ANDROID_FOCUS_GRACE));
			Ok(launched)
		}
		kind => {
			Err(Error::Configuration(format!(
				"start_app needs a mobile target, not {kind:?}"
			)))
		}
	}
}

/// Resolves a package's launcher activity, like `dev.app/.MainActivity`.
fn launcher_activity(serial: &str, package: &str, timeout: Duration) -> Result<String, Error> {
	let mut command = tools::adb(serial);
	command.args([
		"shell",
		"cmd",
		"package",
		"resolve-activity",
		"--brief",
		package,
	]);
	let output = super::tool(&mut command, timeout)?;
	let component = output
		.stdout
		.lines()
		.rev()
		.find_map(|line| line.split_whitespace().next())
		.filter(|line| line.contains('/'))
		.ok_or_else(|| {
			Error::Configuration(format!(
				"no launcher activity found for {package}; is it installed?"
			))
		})?;
	Ok(component.to_owned())
}

/// Terminates an installed app.
pub fn stop_app(target: &Target, identifier: &str, timeout: Duration) -> Result<(), Error> {
	let id = device_id(target)?;

	match target.kind {
		TargetKind::IosSimulator => {
			let mut command = tools::simctl();
			command.args(["terminate", id, identifier]);
			super::tool(&mut command, timeout)?;
			Ok(())
		}
		TargetKind::IosDevice => {
			let mut command = tools::devicectl();
			command
				.args(["device", "process", "terminate"])
				.arg("--device")
				.arg(id)
				.arg(identifier);
			super::tool(&mut command, timeout)?;
			Ok(())
		}
		TargetKind::Android => {
			let mut command = tools::adb(id);
			command.args(["shell", "am", "force-stop", identifier]);
			super::tool(&mut command, timeout)?;
			Ok(())
		}
		kind => {
			Err(Error::Configuration(format!(
				"stop_app needs a mobile target, not {kind:?}"
			)))
		}
	}
}

/// Captures the device's whole screen as PNG bytes.
pub fn screenshot(target: &Target, timeout: Duration) -> Result<Vec<u8>, Error> {
	let id = device_id(target)?;

	match target.kind {
		TargetKind::IosSimulator => {
			let path = std::env::temp_dir().join(format!(
				"leptoskit-screenshot-{}-{}.png",
				std::process::id(),
				std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.expect("clock moved backwards")
					.as_nanos()
			));
			let mut command = tools::simctl();
			command.args(["io", id, "screenshot"]).arg(&path);
			let bytes = match super::tool(&mut command, timeout) {
				Ok(_) => std::fs::read(&path),
				Err(error) => {
					let _ = std::fs::remove_file(&path);
					return Err(error);
				}
			};

			let _ = std::fs::remove_file(&path);
			Ok(bytes?)
		}
		TargetKind::Android => {
			let mut command = tools::adb(id);
			command.args(["exec-out", "screencap", "-p"]);
			Ok(super::tool(&mut command, timeout)?.raw)
		}
		kind => {
			Err(Error::Configuration(format!(
				"device screenshots need an ios_simulator or android target, not {kind:?}; the \
				 screenshot step works everywhere a session does"
			)))
		}
	}
}

/// Taps absolute screen pixels with Android's input tool.
pub fn tap_point(target: &Target, x: i64, y: i64, timeout: Duration) -> Result<(), Error> {
	only(target, TargetKind::Android, "coordinate taps")?;
	let mut command = tools::adb(device_id(target)?);
	command.args(["shell", "input", "tap", &x.to_string(), &y.to_string()]);
	super::tool(&mut command, timeout)?;
	Ok(())
}

/// Drags between absolute screen pixels with Android's input tool.
pub fn swipe(
	target: &Target,
	start: Point,
	end: Point,
	duration_ms: Option<u64>,
	timeout: Duration,
) -> Result<(), Error> {
	only(target, TargetKind::Android, "coordinate swipes")?;
	let duration = duration_ms.unwrap_or(300);
	let mut command = tools::adb(device_id(target)?);
	command.args([
		"shell",
		"input",
		"swipe",
		&start.x.to_string(),
		&start.y.to_string(),
		&end.x.to_string(),
		&end.y.to_string(),
		&duration.to_string(),
	]);
	super::tool(&mut command, timeout)?;
	Ok(())
}

/// Presses one Android key event code.
pub fn key(target: &Target, key: &str, timeout: Duration) -> Result<(), Error> {
	only(target, TargetKind::Android, "key events")?;

	if key.is_empty() || key.contains(char::is_whitespace) {
		return Err(Error::Configuration(
			"key must be one keyevent code".to_owned(),
		));
	}

	let mut command = tools::adb(device_id(target)?);
	command.args(["shell", "input", "keyevent", key]);
	super::tool(&mut command, timeout)?;
	Ok(())
}

/// Reads recent system logs.
pub fn logs(
	target: &Target,
	tail: Option<u64>,
	minutes: Option<u64>,
	timeout: Duration,
) -> Result<String, Error> {
	let id = device_id(target)?;

	let text = match target.kind {
		TargetKind::Android => {
			let lines = tail.unwrap_or(500).to_string();
			let mut command = tools::adb(id);
			command.args(["shell", "logcat", "-d", "-v", "time", "-t", &lines]);
			super::tool(&mut command, timeout)?.stdout
		}
		TargetKind::IosSimulator => {
			let window = format!("--last={}m", minutes.unwrap_or(5));
			let mut command = tools::simctl();
			command.args(["spawn", id, "log", "show", &window, "--style", "compact"]);
			let full = super::tool(&mut command, timeout)?.stdout;
			let keep = tail.unwrap_or(500) as usize;
			if full.lines().count() > keep {
				let skipped = full.lines().count() - keep;
				let kept: Vec<&str> = full.lines().skip(skipped).collect();
				format!("… {} earlier lines …\n{}", skipped, kept.join("\n"))
			} else {
				full
			}
		}
		kind => {
			return Err(Error::Configuration(format!(
				"logs need an android or ios_simulator target, not {kind:?}"
			)));
		}
	};

	Ok(text)
}

/// Installs a mobile package, or checks that a desktop path exists.
///
/// # Errors
///
/// The path is empty, a desktop path is missing, the web has nothing to
/// install, or the platform tool fails.
pub fn install_app(target: &Target, path: &str, timeout: Duration) -> Result<String, Error> {
	if path.trim().is_empty() {
		return Err(Error::Configuration(
			"install needs a package path".to_owned(),
		));
	}

	match target.kind {
		TargetKind::Macos | TargetKind::Linux | TargetKind::Windows => {
			if std::path::Path::new(path).exists() {
				Ok(format!("desktop app at {path}"))
			} else {
				Err(Error::Configuration(format!(
					"nothing to install at {path}"
				)))
			}
		}
		TargetKind::Web => {
			Err(Error::Configuration(
				"web targets have nothing to install".to_owned(),
			))
		}
		kind => {
			let device = device_id(target)?;
			run_plan(&super::gestures::install_plan(kind, device, path)?, timeout)
		}
	}
}

/// Launches an app. Mobile targets use the platform launch tool. Desktop
/// targets launch the path or, on macOS, a bundle id.
///
/// # Errors
///
/// The identifier is empty, the target cannot launch it, or the tool fails.
pub fn open_app(target: &Target, identifier: &str, timeout: Duration) -> Result<String, Error> {
	if identifier.trim().is_empty() {
		return Err(Error::Configuration(
			"open needs an identifier or path".to_owned(),
		));
	}

	match target.kind {
		TargetKind::IosSimulator | TargetKind::IosDevice | TargetKind::Android => {
			start_app(target, identifier, &[], &BTreeMap::new(), timeout)
		}
		TargetKind::Linux => open_linux(identifier, timeout),
		TargetKind::Macos | TargetKind::Windows => {
			run_plan(
				&super::gestures::open_plan(target.kind, identifier)?,
				timeout,
			)
		}
		TargetKind::Web => {
			Err(Error::Configuration(
				"web targets open with a url".to_owned(),
			))
		}
	}
}

/// How long a Linux open waits for a visible window before gestures run.
const LINUX_WINDOW_GRACE: Duration = Duration::from_secs(2);

/// Launches a Linux binary and waits until one of its windows is visible, the
/// process exits, or [`LINUX_WINDOW_GRACE`] passes.
fn open_linux(identifier: &str, timeout: Duration) -> Result<String, Error> {
	open_linux_waiting(identifier, timeout, LINUX_WINDOW_GRACE)
}

/// Launches a Linux binary. `grace` is how long to wait for a visible window
/// when the process stays up. A process that exits has no window to wait for.
fn open_linux_waiting(
	identifier: &str,
	timeout: Duration,
	grace: Duration,
) -> Result<String, Error> {
	let plan = super::gestures::open_plan(TargetKind::Linux, identifier)?;
	let super::gestures::ToolPlan::Command {
		program,
		args,
		detach: true,
	} = plan
	else {
		return Err(Error::Configuration(
			"linux apps open as a detached path".to_owned(),
		));
	};
	let mut command = Command::new(program);
	command.args(args);
	let mut child = tools::spawn_detached_child(&mut command)?;
	let pid = child.id();
	let deadline = Instant::now() + grace.min(timeout);
	loop {
		if linux_window_visible(pid) {
			break;
		}

		match child.try_wait() {
			Ok(Some(_)) => break,
			Ok(None) if Instant::now() >= deadline => break,
			Ok(None) => thread::sleep(Duration::from_millis(20)),
			Err(error) => {
				return Err(Error::Tool {
					name: identifier.to_owned(),
					detail: error.to_string(),
				});
			}
		}
	}

	Ok(format!("pid {pid}"))
}

/// Whether `pid` has a visible window. Activates the first one so a following
/// pointer gesture hits that app.
fn linux_window_visible(pid: u32) -> bool {
	let Ok(output) = Command::new("xdotool")
		.args(["search", "--pid", &pid.to_string(), "--onlyvisible"])
		.output()
	else {
		return false;
	};
	let text = String::from_utf8_lossy(&output.stdout);
	let Some(id) = text.split_whitespace().next() else {
		return false;
	};

	if id.is_empty() {
		return false;
	}

	let _ = Command::new("xdotool")
		.args(["windowactivate", id])
		.status();
	true
}

/// Taps with `adb` or `xdotool`.
///
/// # Errors
///
/// The target is not Android or Linux, or the tool fails.
pub fn inject_tap(target: &Target, x: i64, y: i64, timeout: Duration) -> Result<(), Error> {
	let plan = match target.kind {
		TargetKind::Android => super::gestures::android_tap(device_id(target)?, x, y)?,
		TargetKind::Linux => super::gestures::xdotool_tap(x, y)?,
		kind => {
			return Err(Error::Configuration(format!(
				"inject_tap drives android and linux, not {kind:?}"
			)));
		}
	};
	run_plan(&plan, timeout).map(|_| ())
}

/// Long-presses with `adb` or `xdotool`.
///
/// # Errors
///
/// The target is not Android or Linux, or the tool fails.
pub fn inject_long_press(
	target: &Target,
	x: i64,
	y: i64,
	duration_ms: u64,
	timeout: Duration,
) -> Result<(), Error> {
	let plan = match target.kind {
		TargetKind::Android => {
			super::gestures::android_long_press(device_id(target)?, x, y, duration_ms)?
		}
		TargetKind::Linux => super::gestures::xdotool_long_press(x, y, duration_ms)?,
		kind => {
			return Err(Error::Configuration(format!(
				"inject_long_press drives android and linux, not {kind:?}"
			)));
		}
	};
	run_plan(&plan, timeout).map(|_| ())
}

/// Pans with `adb` or `xdotool`.
///
/// # Errors
///
/// The target is not Android or Linux, or the tool fails.
pub fn inject_pan(
	target: &Target,
	start: Point,
	end: Point,
	duration_ms: u64,
	timeout: Duration,
) -> Result<(), Error> {
	let plan = match target.kind {
		TargetKind::Android => {
			super::gestures::android_pan(device_id(target)?, start, end, duration_ms)?
		}
		TargetKind::Linux => super::gestures::xdotool_pan(start, end, duration_ms)?,
		kind => {
			return Err(Error::Configuration(format!(
				"inject_pan drives android and linux, not {kind:?}"
			)));
		}
	};
	run_plan(&plan, timeout).map(|_| ())
}

/// Reads `wm size`. An override replaces the physical size, which is what
/// the display is actually showing.
pub(super) fn parse_display_size(text: &str) -> Option<(i64, i64)> {
	let mut physical = None;
	let mut override_size = None;
	for line in text.lines() {
		let line = line.trim();
		if let Some(rest) = line.strip_prefix("Override size:") {
			override_size = parse_size_pair(rest);
		} else if let Some(rest) = line.strip_prefix("Physical size:") {
			physical = parse_size_pair(rest);
		}
	}
	override_size.or(physical)
}

fn parse_size_pair(text: &str) -> Option<(i64, i64)> {
	let (width, height) = text.trim().split_once('x')?;
	let width = width.trim().parse::<i64>().ok()?;
	let height = height.trim().parse::<i64>().ok()?;
	if width < 2 || height < 2 {
		None
	} else {
		Some((width, height))
	}
}

fn display_size(serial: &str, timeout: Duration) -> Result<(i64, i64), Error> {
	let mut command = tools::adb(serial);
	command.args(["shell", "wm", "size"]);
	let output = super::tool(&mut command, timeout)?;
	parse_display_size(&output.stdout).ok_or_else(|| {
		Error::Configuration(format!(
			"could not read the display size from {:?}",
			output.stdout.trim()
		))
	})
}

/// Pinches with `uinput` on Android. Linux's pointer tool has one pointer.
///
/// # Errors
///
/// The target is not Android, the display size cannot be read, or the tool
/// fails.
pub fn inject_pinch(
	target: &Target,
	x: i64,
	y: i64,
	start_span: i64,
	end_span: i64,
	duration_ms: u64,
	timeout: Duration,
) -> Result<(), Error> {
	let plan = match target.kind {
		TargetKind::Android => {
			let serial = device_id(target)?;
			let (width, height) = display_size(serial, timeout)?;
			super::gestures::android_pinch(
				serial,
				(width, height),
				x,
				y,
				start_span,
				end_span,
				duration_ms,
			)?
		}
		TargetKind::Linux => {
			return Err(Error::Configuration(
				"xdotool has one pointer, so it cannot pinch".to_owned(),
			));
		}
		kind => {
			return Err(Error::Configuration(format!(
				"inject_pinch drives android, not {kind:?}"
			)));
		}
	};
	run_plan(&plan, timeout).map(|_| ())
}

/// Runs one planned tool.
pub(super) fn run_plan(
	plan: &super::gestures::ToolPlan,
	timeout: Duration,
) -> Result<String, Error> {
	match plan {
		super::gestures::ToolPlan::Adb {
			device,
			args,
			stdin,
		} => {
			let mut command = tools::adb(device);
			command.args(args);
			let fed = stdin.as_deref().map(str::as_bytes);
			let output = tools::run_feeding(&mut command, timeout, fed)?;
			if !output.success() {
				return Err(Error::Tool {
					name: "adb".to_owned(),
					detail: output.failure(),
				});
			}
			Ok(output.stdout)
		}
		super::gestures::ToolPlan::Command {
			program,
			args,
			detach: true,
		} => {
			let mut command = Command::new(program);
			command.args(args);
			tools::spawn_detached(&mut command)?;
			Ok(String::new())
		}
		super::gestures::ToolPlan::Command {
			program,
			args,
			detach: false,
		} => {
			let mut command = Command::new(program);
			command.args(args);
			Ok(super::tool(&mut command, timeout)?.stdout)
		}
	}
}

#[cfg(test)]
mod plan_tests {
	use std::time::Duration;

	use super::super::Error;
	use super::super::Point;
	use super::super::Target;
	use super::super::TargetKind;
	use super::super::gestures::ToolPlan;
	use super::inject_long_press;
	use super::inject_pan;
	use super::inject_pinch;
	use super::inject_tap;
	use super::install_app;
	use super::open_app;
	use super::run_plan;

	fn target(kind: TargetKind, device: Option<&str>) -> Target {
		Target {
			kind,
			device: device.map(str::to_owned),
			application: None,
			url: None,
			browser: None,
			endpoint: None,
			capabilities: std::collections::BTreeMap::new(),
			headless: true,
			startup_seconds: 5,
			command_seconds: 5,
			wait_seconds: 5,
			artifacts: std::path::PathBuf::from("target/leptoskit-control"),
		}
	}

	#[test]
	fn a_plan_can_echo_spawn_and_fail_to_start() {
		let (program, args) = echo_command();
		let echoed = run_plan(
			&ToolPlan::Command {
				program,
				args,
				detach: false,
			},
			Duration::from_secs(5),
		)
		.unwrap();
		assert!(echoed.contains("installed"));
		let (program, args) = true_command();
		run_plan(
			&ToolPlan::Command {
				program,
				args,
				detach: true,
			},
			Duration::from_secs(5),
		)
		.unwrap();
		let missing = run_plan(
			&ToolPlan::Command {
				program: "leptoskit-no-such-tool".to_owned(),
				args: Vec::new(),
				detach: false,
			},
			Duration::from_secs(5),
		);
		assert!(matches!(missing, Err(Error::Tool { .. })));
		let adb = run_plan(
			&ToolPlan::Adb {
				device: "no-such-serial".to_owned(),
				args: vec!["get-state".to_owned()],
				stdin: None,
			},
			Duration::from_secs(5),
		);
		assert!(adb.is_err());
	}

	#[test]
	fn desktop_install_checks_the_path_and_mobile_install_calls_the_tool() {
		let directory =
			std::env::temp_dir().join(format!("leptoskit-install-{}", std::process::id()));
		std::fs::create_dir_all(&directory).unwrap();
		let path = directory.to_string_lossy().into_owned();
		for kind in [TargetKind::Macos, TargetKind::Linux, TargetKind::Windows] {
			let installed =
				install_app(&target(kind, None), &path, Duration::from_secs(5)).unwrap();
			assert!(installed.contains(&path));
		}
		let missing = install_app(
			&target(TargetKind::Linux, None),
			"/leptoskit/no/such/app",
			Duration::from_secs(5),
		);
		assert!(missing.is_err());
		assert!(
			install_app(
				&target(TargetKind::Android, Some("s")),
				" ",
				Duration::from_secs(5)
			)
			.is_err()
		);
		assert!(
			install_app(
				&target(TargetKind::Web, None),
				&path,
				Duration::from_secs(5)
			)
			.is_err()
		);
		assert!(
			install_app(
				&target(TargetKind::Android, Some("no-such-serial")),
				"app.apk",
				Duration::from_secs(5)
			)
			.is_err()
		);
		assert!(
			install_app(
				&target(TargetKind::IosSimulator, Some("UDID")),
				"App.app",
				Duration::from_secs(5)
			)
			.is_err()
		);
		assert!(
			install_app(
				&target(TargetKind::IosDevice, Some("UDID")),
				"App.ipa",
				Duration::from_secs(5)
			)
			.is_err()
		);
		let _ = std::fs::remove_dir_all(&directory);
	}

	#[test]
	fn opening_branches_on_the_target() {
		assert!(
			open_app(
				&target(TargetKind::Android, Some("no-such")),
				"dev.app",
				Duration::from_secs(5)
			)
			.is_err()
		);
		assert!(
			open_app(
				&target(TargetKind::IosSimulator, Some("UDID")),
				"dev.app",
				Duration::from_secs(5)
			)
			.is_err()
		);
		assert!(
			open_app(
				&target(TargetKind::IosDevice, Some("UDID")),
				"dev.app",
				Duration::from_secs(5)
			)
			.is_err()
		);
		assert!(
			open_app(
				&target(TargetKind::Linux, None),
				" ",
				Duration::from_secs(5)
			)
			.is_err()
		);
		assert!(
			open_app(
				&target(TargetKind::Web, None),
				"app",
				Duration::from_secs(5)
			)
			.is_err()
		);
		assert!(
			open_app(
				&target(TargetKind::Linux, None),
				"app",
				Duration::from_secs(5)
			)
			.is_err()
		);
		assert!(
			open_app(
				&target(TargetKind::Macos, None),
				"desktop",
				Duration::from_secs(5)
			)
			.is_err()
		);
		assert!(
			open_app(
				&target(TargetKind::Windows, None),
				"App.exe",
				Duration::from_secs(5)
			)
			.is_err()
		);
		// A program that stays up without a window must not return before the
		// grace: gestures injected the moment `open` returns land on the
		// desktop, because the window is not mapped yet.
		#[cfg(unix)]
		{
			let directory =
				std::env::temp_dir().join(format!("leptoskit-open-{}", std::process::id()));
			std::fs::create_dir_all(&directory).unwrap();
			let path = directory.join("stay");
			std::fs::write(&path, "#!/bin/sh\nsleep 30\n").unwrap();
			#[cfg(unix)]
			{
				use std::os::unix::fs::PermissionsExt;
				std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
			}
			let started = std::time::Instant::now();
			let launched = super::open_linux_waiting(
				&path.to_string_lossy(),
				Duration::from_secs(5),
				Duration::from_millis(200),
			)
			.unwrap();
			let elapsed = started.elapsed();
			if let Some(pid) = launched.strip_prefix("pid ") {
				let _ = std::process::Command::new("kill").arg(pid).status();
			}
			assert!(
				elapsed >= Duration::from_millis(150),
				"open returned in {elapsed:?}, before a window could map"
			);
			assert!(elapsed < Duration::from_secs(1), "open waited {elapsed:?}");
			let _ = std::fs::remove_dir_all(&directory);
		}

		// A program that exits immediately: spawning it is the launch itself.
		let launched = open_app(
			&target(TargetKind::Linux, None),
			true_program(),
			Duration::from_secs(5),
		);
		assert!(launched.is_ok() || cfg!(windows));
		assert!(
			open_app(
				&target(TargetKind::Macos, None),
				"/Applications/Missing.app",
				Duration::from_secs(5)
			)
			.is_err()
		);
		// `start` can report success for a missing path; either result ran the
		// plan.
		let _ = open_app(
			&target(TargetKind::Windows, None),
			r"C:\Missing\App.exe",
			Duration::from_secs(5),
		);
	}

	fn echo_command() -> (String, Vec<String>) {
		if cfg!(windows) {
			(
				"cmd".to_owned(),
				vec!["/C".to_owned(), "echo".to_owned(), "installed".to_owned()],
			)
		} else {
			("echo".to_owned(), vec!["installed".to_owned()])
		}
	}

	fn true_command() -> (String, Vec<String>) {
		if cfg!(windows) {
			(
				"cmd".to_owned(),
				vec!["/C".to_owned(), "exit".to_owned(), "0".to_owned()],
			)
		} else {
			("true".to_owned(), Vec::new())
		}
	}

	fn true_program() -> &'static str {
		if cfg!(windows) {
			"C:\\Windows\\System32\\cmd.exe"
		} else {
			"/usr/bin/true"
		}
	}

	#[test]
	fn injection_rejects_the_wrong_target_and_a_missing_tool() {
		let timeout = Duration::from_secs(5);
		let mac = target(TargetKind::Macos, None);
		assert!(inject_tap(&mac, 1, 1, timeout).is_err());
		assert!(inject_long_press(&mac, 1, 1, 600, timeout).is_err());
		assert!(
			inject_pan(
				&mac,
				Point { x: 1, y: 1 },
				Point { x: 20, y: 1 },
				300,
				timeout
			)
			.is_err()
		);
		assert!(inject_pinch(&mac, 40, 40, 20, 40, 300, timeout).is_err());
		let linux = target(TargetKind::Linux, None);
		assert!(inject_pinch(&linux, 40, 40, 20, 40, 300, timeout).is_err());
		// `xdotool` may be installed. Either result ran the Linux plan.
		let _ = inject_tap(&linux, 1, 1, timeout);
		let _ = inject_long_press(&linux, 1, 1, 600, timeout);
		let _ = inject_pan(
			&linux,
			Point { x: 1, y: 1 },
			Point { x: 20, y: 1 },
			300,
			timeout,
		);
		let android = target(TargetKind::Android, Some("no-such-serial"));
		assert!(inject_tap(&android, 1, 1, timeout).is_err());
		assert!(inject_long_press(&android, 1, 1, 600, timeout).is_err());
		assert!(
			inject_pan(
				&android,
				Point { x: 1, y: 1 },
				Point { x: 20, y: 1 },
				300,
				timeout
			)
			.is_err()
		);
		assert!(inject_pinch(&android, 40, 40, 20, 40, 300, timeout).is_err());
		assert!(inject_tap(&target(TargetKind::Android, None), 1, 1, timeout).is_err());
		assert!(inject_tap(&linux, -1, 1, timeout).is_err());
	}

	#[test]
	fn window_focus_matches_the_package_component() {
		let focused =
			"mCurrentFocus=Window{abc u0 dev.leptoskit.sanity/dev.leptoskit.sanity.MainActivity}\n";
		assert!(super::window_focus_contains(
			focused,
			"dev.leptoskit.sanity"
		));
		assert!(!super::window_focus_contains(focused, "dev.other"));
		assert!(!super::window_focus_contains(
			"mCurrentFocus=Window{abc u0 com.android.launcher3/com.android.launcher3.Launcher}\n",
			"dev.leptoskit.sanity",
		));
		assert_eq!(super::parse_emulator_avd_name("api35\nOK\n"), Some("api35"));
		assert_eq!(super::parse_emulator_avd_name("OK\n"), None);
		assert_eq!(super::parse_emulator_avd_name(""), None);
		assert_eq!(super::package_of("dev.app/.Main"), "dev.app");
	}

	#[cfg(unix)]
	#[test]
	fn an_emulator_process_that_exits_is_not_waited_out() {
		let mut child = std::process::Command::new("true").spawn().unwrap();
		let give_up = std::time::Instant::now() + Duration::from_secs(2);
		while child.try_wait().unwrap().is_none() && std::time::Instant::now() < give_up {
			std::thread::sleep(Duration::from_millis(10));
		}
		let started = std::time::Instant::now();
		let error = super::wait_for_emulator("emulator-5554", &mut child, Duration::from_secs(30));
		assert!(error.is_err(), "{error:?}");
		assert!(
			started.elapsed() < Duration::from_secs(2),
			"waited {:?}",
			started.elapsed()
		);
	}

	#[test]
	fn display_size_prefers_the_override() {
		assert_eq!(
			super::parse_display_size("Physical size: 320x640\n"),
			Some((320, 640))
		);
		assert_eq!(
			super::parse_display_size("Physical size: 1080x2400\nOverride size: 720x1280\n"),
			Some((720, 1280))
		);
		assert_eq!(super::parse_display_size("Physical size: 1x640\n"), None);
		assert_eq!(super::parse_display_size("no size here"), None);
	}
}
