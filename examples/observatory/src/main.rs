//! Opens the observatory in a native window on macOS and Linux, or prints what
//! it would show.
//!
//! ```sh
//! cargo run -p observatory_example                          # a window on macOS or Linux
//! cargo run -p observatory_example -- snapshot sky.png      # a native PNG on macOS or Linux
//! cargo run -p observatory_example -- summary               # the screen's semantics, anywhere
//! ```

use std::process::ExitCode;

use leptoskit_testing::WidgetTester;
use observatory_example::Observatory;
use observatory_example::VIEWPORT;

fn main() -> ExitCode {
	let mut args = std::env::args().skip(1);

	let result = match args.next().as_deref() {
		None => show(),
		Some("summary") => print_summary(),
		Some("snapshot") => {
			match args.next() {
				Some(path) => snapshot(path.as_ref()),
				None => Err("snapshot needs a path for the PNG".to_owned()),
			}
		}
		Some(other) => {
			Err(format!(
				"unknown command `{other}`; try summary or snapshot <path>"
			))
		}
	};

	match result {
		Ok(()) => ExitCode::SUCCESS,
		Err(message) => {
			eprintln!("observatory: {message}");
			ExitCode::FAILURE
		}
	}
}

/// Renders the screen on the headless backend and prints the accessibility
/// tree a screen reader would walk.
fn print_summary() -> Result<(), String> {
	let observatory = Observatory::new();
	let tester = WidgetTester::new(move || observatory.render(), VIEWPORT.0, VIEWPORT.1)
		.map_err(|error| error.to_string())?;
	print!("{}", tester.semantics());
	Ok(())
}

#[cfg(target_os = "macos")]
mod window {
	use std::cell::RefCell;
	use std::path::Path;
	use std::rc::Rc;

	use leptoskit::appkit::AppKitBackend;
	use leptoskit::appkit::MainThreadMarker;
	use leptoskit::appkit::run_application;
	use leptoskit::native::NativeSurface;
	use observatory_example::Observatory;
	use observatory_example::VIEWPORT;

	/// The pixels per point of a snapshot: retina quality.
	const SNAPSHOT_SCALE: f64 = 2.0;

	/// The screen mounted in AppKit views.
	fn mount(mtm: MainThreadMarker, observatory: &Observatory) -> NativeSurface<AppKitBackend> {
		let mut surface = NativeSurface::new(AppKitBackend::new(mtm));
		surface.set_dark_mode(true);
		surface.set_viewport(VIEWPORT.0, VIEWPORT.1);
		surface.set_root(&observatory.render());
		surface
	}

	fn main_thread() -> Result<MainThreadMarker, String> {
		MainThreadMarker::new().ok_or_else(|| "AppKit must run on the main thread".to_owned())
	}

	/// Renders the screen with real AppKit views, offscreen, to a PNG.
	pub fn snapshot(path: &Path) -> Result<(), String> {
		let surface = mount(main_thread()?, &Observatory::new());
		let root = surface.root().ok_or("the screen committed no root")?;
		let png = surface
			.backend()
			.snapshot_png(root, SNAPSHOT_SCALE)
			.map_err(|error| error.to_string())?;
		std::fs::write(path, png)
			.map_err(|error| format!("could not write {}: {error}", path.display()))
	}

	/// Mounts the screen in AppKit views, re-renders it after every event,
	/// and runs the application until its window closes.
	pub fn open() -> Result<(), String> {
		let mtm = main_thread()?;
		let observatory = Observatory::new();
		let surface = Rc::new(RefCell::new(mount(mtm, &observatory)));

		// AppKit delivers events from its own actions, never while the
		// surface is borrowed, so the listener dispatches and re-renders in
		// place.
		let weak = Rc::downgrade(&surface);
		surface.borrow().backend().set_event_listener({
			let observatory = observatory.clone();
			move |node, event| {
				if let Some(surface) = weak.upgrade() {
					let mut surface = surface.borrow_mut();
					surface.dispatch_event(node, event);
					surface.set_root(&observatory.render());
				}
			}
		});

		let surface_ref = surface.borrow();
		let root = surface_ref.root().ok_or("the screen committed no root")?;
		let _window = surface_ref
			.backend()
			.open_window(root, "Observatory")
			.ok_or("the root view disappeared before the window opened")?;
		drop(surface_ref);
		run_application(mtm);
		Ok(())
	}
}

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
mod window {
	use std::cell::Cell;
	use std::cell::RefCell;
	use std::path::Path;
	use std::rc::Rc;
	use std::time::Duration;

	use leptoskit::gtk::GtkBackend;
	use leptoskit::gtk::gtk;
	use leptoskit::gtk::gtk::prelude::*;
	use leptoskit::native::NativeSurface;
	use observatory_example::Observatory;
	use observatory_example::VIEWPORT;

	/// How long a shown window may take to paint its first frame.
	const PAINT_TIMEOUT: Duration = Duration::from_secs(5);

	type Surface = Rc<RefCell<NativeSurface<GtkBackend>>>;

	/// Starts GTK and mounts the screen in GTK widgets, re-rendering it after
	/// every event and following the window's size.
	fn mount(observatory: &Observatory) -> Result<Surface, String> {
		gtk::init().map_err(|error| format!("GTK could not start: {error}"))?;
		let mut surface = NativeSurface::new(GtkBackend::new());
		surface.set_dark_mode(true);
		surface.set_viewport(VIEWPORT.0, VIEWPORT.1);
		surface.set_root(&observatory.render());
		let surface = Rc::new(RefCell::new(surface));

		// GTK delivers both callbacks outside any borrow of the surface, so
		// they dispatch and re-render in place.
		let weak = Rc::downgrade(&surface);
		surface.borrow().backend().set_event_listener({
			let observatory = observatory.clone();
			move |node, event| {
				if let Some(surface) = weak.upgrade() {
					let mut surface = surface.borrow_mut();
					surface.dispatch_event(node, &event);
					surface.set_root(&observatory.render());
				}
			}
		});
		let weak = Rc::downgrade(&surface);
		surface
			.borrow()
			.backend()
			.set_resize_listener(move |width, height| {
				if let Some(surface) = weak.upgrade() {
					surface.borrow_mut().set_viewport(width, height);
				}
			});
		Ok(surface)
	}

	/// Shows the committed screen in a window sized to the viewport.
	fn show(surface: &Surface) -> Result<gtk::Window, String> {
		let surface_ref = surface.borrow();
		let root = surface_ref.root().ok_or("the screen committed no root")?;
		let window = surface_ref
			.backend()
			.open_window(root, "Observatory", VIEWPORT.0 as i32, VIEWPORT.1 as i32)
			.ok_or("the root view disappeared before the window opened")?;
		// Monitor scale changes alter text metrics, so remeasure.
		let weak = Rc::downgrade(surface);
		window.connect_scale_factor_notify(move |_| {
			if let Some(surface) = weak.upgrade() {
				surface.borrow_mut().refresh_text_measurer();
			}
		});
		Ok(window)
	}

	/// Renders the screen with GTK's own renderer to a PNG, once its window
	/// has painted.
	pub fn snapshot(path: &Path) -> Result<(), String> {
		let surface = mount(&Observatory::new())?;
		let window = show(&surface)?;
		let png = painted_png(&surface, &window);
		window.close();
		std::fs::write(path, png?)
			.map_err(|error| format!("could not write {}: {error}", path.display()))
	}

	/// Shows the screen in a window and runs GTK until it closes.
	pub fn open() -> Result<(), String> {
		let surface = mount(&Observatory::new())?;
		let _window = show(&surface)?;
		leptoskit::gtk::run_application();
		Ok(())
	}

	/// Snapshots the window's root after its first paint.
	fn painted_png(surface: &Surface, window: &gtk::Window) -> Result<Vec<u8>, String> {
		let clock = window
			.frame_clock()
			.ok_or("the shown window has no frame clock")?;
		let result = Rc::new(RefCell::new(None));
		let painted = clock.connect_after_paint({
			let result = Rc::clone(&result);
			let surface = Rc::clone(surface);
			move |_| {
				if result.borrow().is_some() {
					return;
				}

				let surface = surface.borrow();
				let png = surface
					.root()
					.ok_or_else(|| "the painted screen has no root".to_owned())
					.and_then(|root| {
						surface
							.backend()
							.snapshot_png(root)
							.map_err(|error| error.to_string())
					});
				result.replace(Some(png));
			}
		});
		clock.request_phase(gtk::gdk::FrameClockPhase::AFTER_PAINT);

		let timed_out = Rc::new(Cell::new(false));
		let timeout = gtk::glib::timeout_add_local_once(PAINT_TIMEOUT, {
			let timed_out = Rc::clone(&timed_out);
			move || timed_out.set(true)
		});
		let context = gtk::glib::MainContext::default();

		while result.borrow().is_none() && !timed_out.get() {
			context.iteration(true);
		}

		clock.disconnect(painted);

		if !timed_out.get() {
			timeout.remove();
		}

		result.take().unwrap_or_else(|| {
			Err(format!(
				"the window did not paint within {} seconds",
				PAINT_TIMEOUT.as_secs()
			))
		})
	}
}

#[cfg(any(
	target_os = "macos",
	all(target_os = "linux", not(target_env = "ohos"))
))]
fn show() -> Result<(), String> {
	window::open()
}

#[cfg(any(
	target_os = "macos",
	all(target_os = "linux", not(target_env = "ohos"))
))]
fn snapshot(path: &std::path::Path) -> Result<(), String> {
	window::snapshot(path)
}

/// Windows open on macOS and Linux; elsewhere the summary stands in.
#[cfg(not(any(
	target_os = "macos",
	all(target_os = "linux", not(target_env = "ohos"))
)))]
fn show() -> Result<(), String> {
	eprintln!("observatory: windows open on macOS and Linux; printing the summary instead");
	print_summary()
}

#[cfg(not(any(
	target_os = "macos",
	all(target_os = "linux", not(target_env = "ohos"))
)))]
fn snapshot(_path: &std::path::Path) -> Result<(), String> {
	Err("snapshots render native views, so they need macOS or Linux".to_owned())
}
