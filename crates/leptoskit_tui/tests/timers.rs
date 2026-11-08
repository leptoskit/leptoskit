//! A host-owned deadline wakes the terminal without keyboard input.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use leptoskit_native::EventOutcome;
use leptoskit_native::NativeEvent;
use leptoskit_native::NativeSurface;
use leptoskit_native::components::Text;
use leptoskit_tui::EventSource;
use leptoskit_tui::Exit;
use leptoskit_tui::SourceEvent;
use leptoskit_tui::TuiApplication;
use leptoskit_tui::TuiBackend;
use leptoskit_tui::TuiError;
use leptoskit_tui::TuiHost;
use leptoskit_tui::TuiOptions;
use leptoskit_tui::testing::Recorder;
use leptoskit_yoga::NodeId;

struct App {
	surface: NativeSurface<TuiBackend>,
	due: Rc<Cell<bool>>,
	fired: bool,
}

impl TuiApplication for App {
	fn backend(&mut self) -> &mut TuiBackend {
		self.surface.backend_mut()
	}

	fn dispatch(&mut self, _: NodeId, _: &NativeEvent) -> EventOutcome {
		panic!("no input was sent");
	}

	fn resize(&mut self, cols: u16, rows: u16) {
		let (width, height) = leptoskit_tui::viewport(cols, rows);
		self.surface.set_viewport(width, height);
	}

	fn mount(&mut self) {
		self.surface.set_root(&Text("waiting"));
	}

	fn pump(&mut self) -> bool {
		if self.due.get() && !self.fired {
			self.fired = true;
			self.surface.set_root(&Text("timer completed"));

			return true;
		}

		false
	}

	fn deadline(&self) -> Option<Duration> {
		(!self.fired).then_some(Duration::from_millis(5))
	}
}

struct Source {
	due: Rc<Cell<bool>>,
	waits: Vec<Duration>,
}

impl EventSource for Source {
	fn poll(&mut self, timeout: Duration) -> Result<bool, TuiError> {
		self.waits.push(timeout);
		// The first wait reaches only the timer deadline, without an input
		// event. The second ends the fixture after the repaint.
		Ok(self.due.replace(true))
	}

	fn read(&mut self) -> Result<Option<SourceEvent>, TuiError> {
		Ok(None)
	}
}

#[test]
fn timer_deadlines_wake_and_repaint_without_input() {
	let due = Rc::new(Cell::new(false));
	let mut app = App {
		surface: NativeSurface::new(TuiBackend::new()),
		due: Rc::clone(&due),
		fired: false,
	};
	let mut source = Source {
		due,
		waits: Vec::new(),
	};
	let mut recorder = Recorder::new(25, 2);
	let exit = TuiHost::new(TuiOptions::default())
		.run(&mut app, &mut source, &mut recorder)
		.unwrap();
	assert_eq!(exit, Exit::SourceEnded);
	assert_eq!(source.waits.len(), 2);
	assert!(source.waits[0] <= Duration::from_millis(5));

	assert!(app.fired);
	assert!(
		app.deadline().is_none(),
		"completed timers stop requesting wakes"
	);
	assert!(
		recorder
			.frames
			.last()
			.unwrap()
			.to_plain_text()
			.contains("timer completed")
	);
}
