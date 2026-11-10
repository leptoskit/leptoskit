//! Scripted input: a JSON list of steps run against a headless guest.

use std::fmt::Write;
use std::path::PathBuf;

use leptoskit_native::Role;
use leptoskit_wasm::FrameStats;

use super::DEFAULT_MAX_WAKES;
use super::DEFAULT_VIEWPORT;
use super::HeadlessApp;
use super::HeadlessError;
use super::SnapshotFormat;
use crate::Finder;

/// A script: an optional viewport and appearance, then ordered steps.
///
/// ```json
/// {
///   "viewport": { "width": 390, "height": 844 },
///   "steps": [
///     { "action": "press", "text": "Increment" },
///     { "action": "expect", "text": "Count: 1" },
///     { "action": "snapshot", "path": "counter.svg" }
///   ]
/// }
/// ```
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
	/// The viewport to start at; [`DEFAULT_VIEWPORT`] when absent.
	#[serde(default)]
	pub viewport: Option<Viewport>,
	/// Whether `dark:` utilities apply from the start.
	#[serde(default)]
	pub dark: bool,
	/// The steps, run in order until one fails.
	pub steps: Vec<Step>,
}

impl Script {
	/// Parses a script from JSON.
	pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
		serde_json::from_str(json)
	}

	/// The viewport the script starts at.
	pub fn start_viewport(&self) -> (f32, f32) {
		self.viewport.map_or(DEFAULT_VIEWPORT, |viewport| {
			(viewport.width, viewport.height)
		})
	}
}

/// A viewport size, in layout units.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Viewport {
	/// The width.
	pub width: f32,
	/// The height.
	pub height: f32,
}

/// Names one widget, by exactly one of its text, accessibility label, test
/// id, or ARIA role with an accessible name.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct Selector {
	/// Exact visible text.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub text: Option<String>,
	/// Exact accessibility label.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub label: Option<String>,
	/// The identifier `test_id` set.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub test_id: Option<String>,
	/// An ARIA role, such as `button` or `checkbox`; needs `name`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub role: Option<String>,
	/// The accessible name `role` must have.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
}

impl Selector {
	/// The finder this selector names.
	pub fn finder(&self) -> Result<Finder, String> {
		match (
			&self.text,
			&self.label,
			&self.test_id,
			&self.role,
			&self.name,
		) {
			(Some(text), None, None, None, None) => Ok(Finder::text(text)),
			(None, Some(label), None, None, None) => Ok(Finder::label(label)),
			(None, None, Some(id), None, None) => Ok(Finder::test_id(id)),
			(None, None, None, Some(role), Some(name)) => {
				let role =
					Role::from_wire(role).ok_or_else(|| format!("unknown ARIA role {role:?}"))?;
				Ok(Finder::aria_role(role, name))
			}
			(None, None, None, Some(_), None) => Err("a role selector needs a name".to_owned()),
			_ => {
				Err(
					"a selector needs exactly one of text, label, test_id, or role with name"
						.to_owned(),
				)
			}
		}
	}
}

/// One scripted step. The `action` field names it.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Step {
	/// Taps a widget.
	Press {
		/// The widget.
		#[serde(flatten)]
		target: Selector,
	},
	/// Replaces a text input's value.
	Type {
		/// The text input.
		#[serde(flatten)]
		target: Selector,
		/// The new value.
		value: String,
	},
	/// Submits a text input, as the return key does.
	Submit {
		/// The text input.
		#[serde(flatten)]
		target: Selector,
	},
	/// Proposes a checked state for a switch, checkbox, or radio.
	Toggle {
		/// The control.
		#[serde(flatten)]
		target: Selector,
		/// The proposed state.
		checked: bool,
	},
	/// Proposes a slider value.
	Slide {
		/// The slider.
		#[serde(flatten)]
		target: Selector,
		/// The proposed value.
		value: f64,
	},
	/// Reports a scroll offset for a scroll view.
	Scroll {
		/// The scroll view.
		#[serde(flatten)]
		target: Selector,
		/// The horizontal offset.
		#[serde(default)]
		x: f32,
		/// The vertical offset.
		#[serde(default)]
		y: f32,
	},
	/// Lays out at a new viewport.
	Resize {
		/// The width.
		width: f32,
		/// The height.
		height: f32,
	},
	/// Switches the appearance.
	Dark {
		/// Whether `dark:` utilities apply.
		dark: bool,
	},
	/// Lets async work run until the guest commits `commits` more times.
	Wait {
		/// Commits to wait for.
		commits: u32,
		/// Plugin wakes to try before failing.
		#[serde(default)]
		max_wakes: Option<u32>,
	},
	/// Lets async work run until exactly one widget matches.
	WaitFor {
		/// The widget.
		#[serde(flatten)]
		target: Selector,
		/// Plugin wakes to try before failing.
		#[serde(default)]
		max_wakes: Option<u32>,
	},
	/// Asserts that exactly one visible widget matches.
	Expect {
		/// The widget.
		#[serde(flatten)]
		target: Selector,
	},
	/// Asserts that no visible widget matches.
	ExpectAbsent {
		/// The widget.
		#[serde(flatten)]
		target: Selector,
	},
	/// Writes the laid-out tree to an `.svg` or `.png` file.
	Snapshot {
		/// The file, relative to the working directory.
		path: PathBuf,
		/// Pixels per layout unit, for PNG.
		#[serde(default)]
		scale: Option<f32>,
	},
	/// Writes the tree dump to a file: JSON for `.json`, text otherwise.
	Tree {
		/// The file, relative to the working directory.
		path: PathBuf,
	},
}

impl Step {
	/// A one-line description, for reports.
	pub fn describe(&self) -> String {
		let selector =
			|target: &Selector| serde_json::to_string(target).expect("selectors serialize");

		match self {
			Self::Press { target } => format!("press {}", selector(target)),
			Self::Type { target, value } => format!("type {value:?} into {}", selector(target)),
			Self::Submit { target } => format!("submit {}", selector(target)),
			Self::Toggle { target, checked } => format!("toggle {} to {checked}", selector(target)),
			Self::Slide { target, value } => format!("slide {} to {value}", selector(target)),
			Self::Scroll { target, x, y } => format!("scroll {} to ({x}, {y})", selector(target)),
			Self::Resize { width, height } => format!("resize to {width}x{height}"),
			Self::Dark { dark } => format!("dark {dark}"),
			Self::Wait { commits: 1, .. } => "wait for 1 commit".to_owned(),
			Self::Wait { commits, .. } => format!("wait for {commits} commits"),
			Self::WaitFor { target, .. } => format!("wait for {}", selector(target)),
			Self::Expect { target } => format!("expect {}", selector(target)),
			Self::ExpectAbsent { target } => format!("expect no {}", selector(target)),
			Self::Snapshot { path, .. } => format!("snapshot to {}", path.display()),
			Self::Tree { path } => format!("tree to {}", path.display()),
		}
	}
}

/// How a script went.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ScriptReport {
	/// Whether every step passed.
	pub passed: bool,
	/// The steps that ran, ending at the first failure.
	pub steps: Vec<StepReport>,
	/// Steps not run after a failure.
	pub skipped: usize,
	/// The committed tree when the script ended.
	pub tree: String,
}

impl ScriptReport {
	/// One line per step, then the tree when a step failed.
	pub fn summary(&self) -> String {
		let mut output = String::new();

		for step in &self.steps {
			let mark = if step.passed { "ok" } else { "FAIL" };
			write!(output, "{mark:>4} {:>3}  {}", step.index + 1, step.step)
				.expect("writing to a string");

			if let Some(frame) = &step.frame {
				write!(
					output,
					"  (commit {}, {}, {})",
					frame.commit,
					count(frame.mutations, "mutation"),
					count(frame.nodes, "node")
				)
				.expect("writing to a string");
			}

			output.push('\n');

			if let Some(error) = &step.error {
				// The error's first line is its reason; the tree follows once.
				let reason = error.lines().next().unwrap_or_default();
				writeln!(output, "          {reason}").expect("writing to a string");
			}
		}

		match self.skipped {
			0 => {}
			1 => output.push_str("1 step skipped\n"),
			skipped => writeln!(output, "{skipped} steps skipped").expect("writing to a string"),
		}

		if !self.passed {
			output.push_str("\ncommitted tree:\n");
			output.push_str(&self.tree);
		}

		output
	}
}

/// `1 node`, `2 nodes`.
fn count(count: usize, noun: &str) -> String {
	if count == 1 {
		format!("1 {noun}")
	} else {
		format!("{count} {noun}s")
	}
}

/// How one step went.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct StepReport {
	/// The step's position in the script, from zero.
	pub index: usize,
	/// What the step did.
	pub step: String,
	/// Whether it passed.
	pub passed: bool,
	/// Why it failed.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
	/// The guest's commit count after the step.
	pub commits: u32,
	/// What the step's last commit cost, when it committed.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub frame: Option<FrameStats>,
	/// A file the step wrote.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub artifact: Option<PathBuf>,
}

impl HeadlessApp {
	/// Loads a guest at the script's viewport and runs the script, stopping
	/// at the first failed step.
	pub fn run_script(wasm: &[u8], script: &Script) -> Result<ScriptReport, HeadlessError> {
		let (width, height) = script.start_viewport();
		let mut app = Self::start(wasm, width, height)?;

		if script.dark {
			app.set_dark_mode(true)?;
		}

		Ok(app.run_steps(&script.steps))
	}

	/// Runs steps against this guest, stopping at the first failure.
	pub fn run_steps(&mut self, steps: &[Step]) -> ScriptReport {
		let mut reports = Vec::new();

		for (index, step) in steps.iter().enumerate() {
			let before = self.commits();
			let outcome = self.run_step(step);
			let commits = self.commits();
			let (passed, error, artifact) = match outcome {
				Ok(artifact) => (true, None, artifact),
				Err(error) => (false, Some(error.to_string()), None),
			};
			reports.push(StepReport {
				index,
				step: step.describe(),
				passed,
				error,
				commits,
				frame: (commits > before)
					.then(|| self.last_frame().cloned())
					.flatten(),
				artifact,
			});

			if !passed {
				break;
			}
		}

		let passed = reports.iter().all(|step| step.passed);
		ScriptReport {
			passed,
			skipped: steps.len() - reports.len(),
			steps: reports,
			tree: self.tree(),
		}
	}

	fn run_step(&mut self, step: &Step) -> Result<Option<PathBuf>, HeadlessError> {
		match step {
			Step::Press { target } => self.tap(&self.select(target)?)?,
			Step::Type { target, value } => self.enter_text(&self.select(target)?, value)?,
			Step::Submit { target } => self.submit(&self.select(target)?)?,
			Step::Toggle { target, checked } => self.toggle(&self.select(target)?, *checked)?,
			Step::Slide { target, value } => {
				self.set_slider_value(&self.select(target)?, *value)?
			}
			Step::Scroll { target, x, y } => self.scroll(&self.select(target)?, *x, *y)?,
			Step::Resize { width, height } => self.resize(*width, *height)?,
			Step::Dark { dark } => self.set_dark_mode(*dark)?,
			Step::Wait { commits, max_wakes } => {
				self.wait_for_commits(*commits, max_wakes.unwrap_or(DEFAULT_MAX_WAKES))?;
			}
			Step::WaitFor { target, max_wakes } => {
				let finder = self.select(target)?;
				self.wait_for(&finder, max_wakes.unwrap_or(DEFAULT_MAX_WAKES))?;
			}
			Step::Expect { target } => {
				self.find(&self.select(target)?)?;
			}
			Step::ExpectAbsent { target } => {
				let finder = self.select(target)?;
				let count = self.find_all(&finder).len();

				if count > 0 {
					return Err(self
						.error(format!("expected no match for {finder:?}, found {count}"))
						.into());
				}
			}
			Step::Snapshot { path, scale } => {
				let format = SnapshotFormat::for_path(path, scale.unwrap_or(1.0))
					.map_err(HeadlessError::Snapshot)?;
				self.snapshot(format)?.save(path)?;
				return Ok(Some(path.clone()));
			}
			Step::Tree { path } => {
				let contents = if path
					.extension()
					.is_some_and(|extension| extension == "json")
				{
					let mut json =
						serde_json::to_string_pretty(&self.tree_json()).expect("trees serialize");
					json.push('\n');
					json
				} else {
					self.tree()
				};
				std::fs::write(path, contents)?;
				return Ok(Some(path.clone()));
			}
		}

		Ok(None)
	}

	fn select(&self, target: &Selector) -> Result<Finder, HeadlessError> {
		target.finder().map_err(|reason| self.error(reason).into())
	}
}
