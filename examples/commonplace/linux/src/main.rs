//! GTK startup and event delivery. Application screens live in commonplace_app.

#[cfg(target_os = "linux")]
mod runtime;

#[cfg(target_os = "linux")]
mod generated {
	include!(concat!(
		env!("CARGO_MANIFEST_DIR"),
		"/../.leptoskit/linux/registrant.rs"
	));
}

#[cfg(target_os = "linux")]
mod application {
	use std::cell::Cell;
	use std::cell::RefCell;
	use std::rc::Rc;
	use std::rc::Weak;
	use std::time::Duration;

	use leptoskit::gtk::GtkBackend;
	use leptoskit::gtk::gtk;
	use leptoskit::gtk::gtk::prelude::*;
	use leptoskit::native::RiveEnvironment;
	use leptoskit::wasm::WasmHost;
	use leptoskit_bridge_ffi::app::PluginHost;

	use super::runtime;

	type Host = Rc<RefCell<WasmHost<GtkBackend>>>;
	type Plugins = Rc<RefCell<PluginHost>>;
	type Failure = Rc<RefCell<Option<Box<dyn std::error::Error>>>>;
	const INITIAL_WIDTH: i32 = 800;
	const INITIAL_HEIGHT: i32 = 600;
	/// How often a development session checks for a rebuilt guest.
	const DEVELOPMENT_POLL: std::time::Duration = std::time::Duration::from_millis(100);

	pub(super) fn run() -> runtime::AppResult<()> {
		gtk::init()?;
		let wasm = runtime::guest("linux")?;
		let mut host = WasmHost::new(GtkBackend::new(), &wasm)?;

		if let Err(error) = host.enable_timers() {
			eprintln!("leptoskit: timers unavailable: {error}");
		}

		super::generated::configure(&mut host);
		let mut plugins = PluginHost::new(wake_from_plugin);
		super::generated::plugins(&mut plugins, &mut host)?;
		host.start(INITIAL_WIDTH as f32, INITIAL_HEIGHT as f32)?;

		let host = Rc::new(RefCell::new(host));
		let plugins = Rc::new(RefCell::new(plugins));
		let failure = Rc::new(RefCell::new(None));
		let window = host.borrow().surface().backend().create_window(
			"commonplace",
			INITIAL_WIDTH,
			INITIAL_HEIGHT,
		);
		let frames = Rc::new(RiveFrames {
			host: Rc::downgrade(&host),
			window: window.downgrade(),
			failure: Rc::clone(&failure),
			callback: RefCell::new(None),
			previous: Cell::new(None),
			deadline: Cell::new(0),
		});
		let wake = Rc::new(GuestWake {
			host: Rc::downgrade(&host),
			plugins: Rc::downgrade(&plugins),
			window: window.downgrade(),
			failure: Rc::clone(&failure),
			source: RefCell::new(None),
		});
		PLUGIN_WAKE.with(|plugin_wake| plugin_wake.replace(Rc::downgrade(&wake)));
		install_events(&host, &window, &failure, &frames, &wake);

		install_viewport(&host, &window, &frames, &wake);
		let (settings, motion_listener) = install_rive_environment(&window, &frames);
		install_development(&host, &plugins, &wasm, &frames, &wake);

		wake.arm();
		frames.request();
		leptoskit::gtk::run_application();
		frames.stop();
		wake.stop();
		settings.disconnect(motion_listener);

		if let Some(error) = failure.take() {
			return Err(error);
		}

		Ok(())
	}

	fn install_events(
		host: &Host,
		window: &gtk::Window,
		failure: &Failure,
		frames: &Rc<RiveFrames>,
		wake: &Rc<GuestWake>,
	) {
		let weak = Rc::downgrade(host);
		let weak_window = window.downgrade();
		let event_failure = Rc::clone(failure);
		let frames = Rc::clone(frames);
		let wake = Rc::clone(wake);
		host.borrow()
			.surface()
			.backend()
			.set_event_listener(move |node, event| {
				if event_failure.borrow().is_some() {
					return;
				}

				let Some(host) = weak.upgrade() else {
					return;
				};
				let result = host.borrow_mut().dispatch(node, &event);

				if let Err(error) = result {
					event_failure.replace(Some(error.into()));

					close_window(&weak_window);
				} else {
					frames.request();
					wake.arm();
				}
			});
	}

	fn close_window(window: &gtk::glib::WeakRef<gtk::Window>) {
		if let Some(window) = window.upgrade() {
			window.close();
		}
	}

	/// Swaps in each guest `leptoskit dev` rebuilds, when it launched this
	/// debug build. The window and its unchanged widgets stay as they are.
	fn install_development(
		host: &Host,
		plugins: &Plugins,
		running: &[u8],
		frames: &Rc<RiveFrames>,
		wake: &Rc<GuestWake>,
	) {
		let Some(guests) = leptoskit::codepush::dev::follow_environment(running) else {
			return;
		};
		let weak = Rc::downgrade(host);
		let plugins = Rc::downgrade(plugins);
		let frames = Rc::downgrade(frames);
		let wake = Rc::downgrade(wake);
		gtk::glib::timeout_add_local(DEVELOPMENT_POLL, move || {
			let (Some(host), Some(plugins), Some(frames)) =
				(weak.upgrade(), plugins.upgrade(), frames.upgrade())
			else {
				return gtk::glib::ControlFlow::Break;
			};

			if let Some(wasm) = guests.try_iter().last() {
				let result = reload(&mut host.borrow_mut(), &mut plugins.borrow_mut(), &wasm);
				if let Err(error) = result {
					eprintln!("leptoskit dev: {error}");
				} else {
					frames.stop();
					frames.request();
					if let Some(wake) = wake.upgrade() {
						wake.arm();
					}
				}
			}

			gtk::glib::ControlFlow::Continue
		});
	}

	/// A guest that fails to load leaves the running one, and its plugins,
	/// as they were.
	fn reload(
		host: &mut WasmHost<GtkBackend>,
		plugins: &mut PluginHost,
		wasm: &[u8],
	) -> runtime::AppResult<()> {
		host.replace_guest(wasm)?;
		// Calls the old guest made answer no one now.
		plugins.reset();
		super::generated::configure(host);
		super::generated::plugins(plugins, host)?;
		let viewport = host.surface().viewport();
		host.start(viewport.width, viewport.height)?;
		host.pump_timers()?;
		eprintln!("leptoskit dev: reloaded");
		Ok(())
	}

	fn install_viewport(
		host: &Host,
		window: &gtk::Window,
		frames: &Rc<RiveFrames>,
		wake: &Rc<GuestWake>,
	) {
		let weak = Rc::downgrade(host);
		let resized = Rc::clone(frames);
		let resize_wake = Rc::clone(wake);
		host.borrow()
			.surface()
			.backend()
			.set_resize_listener(move |width, height| {
				if let Some(host) = weak.upgrade() {
					host.borrow_mut().set_viewport(width, height);
					resized.request();
					resize_wake.arm();
				}
			});
		let weak = Rc::downgrade(host);
		let frames = Rc::clone(frames);
		let scale_wake = Rc::clone(wake);
		window.connect_scale_factor_notify(move |_| {
			if let Some(host) = weak.upgrade() {
				host.borrow_mut().refresh_text_measurer();
				frames.request();
				scale_wake.arm();
			}
		});
	}

	thread_local! {
		/// The wake a plugin's reply runs on the GTK thread.
		static PLUGIN_WAKE: RefCell<Weak<GuestWake>> = const { RefCell::new(Weak::new()) };
	}

	/// Runs on worker and runtime threads when a plugin's reply is ready, and
	/// asks the GTK thread to deliver it. The idle source runs from the main
	/// loop, never inside a borrow of the host.
	fn wake_from_plugin() {
		gtk::glib::idle_add_once(|| {
			let wake = PLUGIN_WAKE.with(|wake| wake.borrow().upgrade());

			if let Some(wake) = wake {
				wake.arm();
			}
		});
	}

	/// Lets the guest read due timers and plugin replies, then keeps one GTK
	/// timeout at the guest timer's next deadline. It is rearmed only while a
	/// timer waits, so an idle app has no timer source at all.
	struct GuestWake {
		host: Weak<RefCell<WasmHost<GtkBackend>>>,
		plugins: Weak<RefCell<PluginHost>>,
		window: gtk::glib::WeakRef<gtk::Window>,
		failure: Failure,
		source: RefCell<Option<gtk::glib::SourceId>>,
	}

	impl GuestWake {
		fn stop(&self) {
			if let Some(source) = self.source.take() {
				source.remove();
			}
		}

		fn arm(self: &Rc<Self>) {
			self.stop();

			let Some(host) = self.host.upgrade() else {
				return;
			};
			let mut result = host.borrow_mut().pump_timers().map(|_| ());

			if let (Ok(()), Some(plugins)) = (&result, self.plugins.upgrade()) {
				result = plugins.borrow_mut().pump(&mut host.borrow_mut());
			}

			if let Err(error) = result {
				self.failure.replace(Some(error.into()));
				close_window(&self.window);

				return;
			}

			let Some(deadline) = host.borrow().next_timer_deadline() else {
				return;
			};
			let delay = deadline.max(Duration::from_millis(1));
			let weak = Rc::downgrade(self);
			let source = gtk::glib::timeout_add_local_once(delay, move || {
				let Some(owner) = weak.upgrade() else {
					return;
				};
				owner.source.borrow_mut().take();
				owner.arm();
			});
			self.source.replace(Some(source));
		}
	}

	/// The GTK frame clock drives visible scenes, with no polling source while
	/// idle.
	struct RiveFrames {
		host: Weak<RefCell<WasmHost<GtkBackend>>>,
		window: gtk::glib::WeakRef<gtk::Window>,
		failure: Failure,
		callback: RefCell<Option<gtk::TickCallbackId>>,
		previous: Cell<Option<i64>>,
		deadline: Cell<i64>,
	}

	impl RiveFrames {
		fn stop(&self) {
			if let Some(callback) = self.callback.take() {
				callback.remove();
			}

			self.previous.set(None);
			self.deadline.set(0);
		}

		fn request(self: &Rc<Self>) {
			let Some(window) = self.window.upgrade() else {
				self.stop();

				return;
			};

			if !window.is_mapped() || !window.is_active() || self.failure.borrow().is_some() {
				self.stop();

				return;
			}

			if self.callback.borrow().is_some() {
				return;
			}

			let Some(host) = self.host.upgrade() else {
				return;
			};

			if !host.borrow().has_rive_players() {
				return;
			}

			let weak = Rc::downgrade(self);
			let callback = window.add_tick_callback(move |window, clock| {
				let Some(owner) = weak.upgrade() else {
					return gtk::glib::ControlFlow::Break;
				};
				let Some(host) = owner.host.upgrade() else {
					return gtk::glib::ControlFlow::Break;
				};
				let now = clock.frame_time();
				// High-refresh displays still rasterize at most 60
				// frames/second.
				if now + 500 < owner.deadline.get() {
					return gtk::glib::ControlFlow::Continue;
				}
				owner.deadline.set(if owner.deadline.get() == 0 {
					now + 16_667
				} else {
					(owner.deadline.get() + 16_667).max(now + 1_000)
				});

				let elapsed = owner
					.previous
					.replace(Some(now))
					.map_or(Duration::ZERO, |last| {
						Duration::from_micros(now.saturating_sub(last).max(0) as u64)
					});
				let result = host.borrow_mut().tick_rive(
					elapsed,
					RiveEnvironment {
						scale: window.scale_factor() as f32,
						active: window.is_mapped() && window.is_active(),
						reduced_motion: !window.settings().is_gtk_enable_animations(),
					},
				);
				match result {
					Ok(true) => gtk::glib::ControlFlow::Continue,
					result => {
						owner.callback.take();
						owner.previous.set(None);
						owner.deadline.set(0);
						if let Err(error) = result {
							owner.failure.replace(Some(error.into()));
							window.close();
						}

						gtk::glib::ControlFlow::Break
					}
				}
			});
			self.callback.replace(Some(callback));
		}
	}

	fn install_rive_environment(
		window: &gtk::Window,
		frames: &Rc<RiveFrames>,
	) -> (gtk::Settings, gtk::glib::SignalHandlerId) {
		let owner = Rc::clone(frames);
		window.connect_is_active_notify(move |_| owner.request());
		let owner = Rc::clone(frames);
		window.connect_map(move |_| owner.request());
		let owner = Rc::clone(frames);
		window.connect_unmap(move |_| owner.stop());
		let owner = Rc::downgrade(frames);
		let settings = window.settings();

		let listener = settings.connect_gtk_enable_animations_notify(move |_| {
			if let Some(owner) = owner.upgrade() {
				owner.request();
			}
		});
		(settings, listener)
	}
}

#[cfg(target_os = "linux")]
fn main() -> runtime::AppResult<()> {
	application::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
	eprintln!("build the Linux runner on Linux with `leptoskit build --platform linux`");
	std::process::exit(1);
}
