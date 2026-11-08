//! The device's rotation sensor, for orientation sessions: Core Motion's
//! attitude on iOS, and the game rotation vector on Android.

#[cfg(target_os = "android")]
pub use android::DeviceRotation;
#[cfg(target_os = "ios")]
pub use ios::DeviceRotation;

/// The rate the sensors are asked for, in hertz: a display's.
const RATE: f64 = 60.0;

#[cfg(target_os = "ios")]
mod ios {
	use leptoskit_bridge::Error;
	use objc2::rc::Retained;
	use objc2_core_motion::CMAttitudeReferenceFrame;
	use objc2_core_motion::CMMotionManager;

	use super::RATE;
	use crate::host::OrientationSource;
	use crate::math::Quat;

	/// Core Motion's device attitude, with gravity as +Z and an arbitrary
	/// heading that doesn't jump when the compass recalibrates.
	#[derive(Debug)]
	pub struct DeviceRotation {
		manager: Retained<CMMotionManager>,
	}

	// SAFETY: `CMMotionManager` documents its pull interface (start, read
	// `deviceMotion`, stop) as callable from any thread; the runtime calls it
	// from one thread at a time.
	unsafe impl Send for DeviceRotation {}

	impl DeviceRotation {
		/// The device's sensor.
		#[must_use]
		pub fn new() -> Self {
			// SAFETY: A plain allocation of Core Motion's manager.
			Self {
				manager: unsafe { CMMotionManager::new() },
			}
		}
	}

	impl Default for DeviceRotation {
		fn default() -> Self {
			Self::new()
		}
	}

	impl OrientationSource for DeviceRotation {
		fn available(&self) -> bool {
			// SAFETY: A property read.
			unsafe { self.manager.isDeviceMotionAvailable() }
		}

		fn start(&mut self) -> Result<(), Error> {
			if !self.available() {
				return Err(Error::Unavailable);
			}

			// SAFETY: Starts pull-mode updates; no handler or queue is
			// involved.
			unsafe {
				self.manager.setDeviceMotionUpdateInterval(1.0 / RATE);
				self.manager
					.startDeviceMotionUpdatesUsingReferenceFrame(CMAttitudeReferenceFrame(1));
			}

			Ok(())
		}

		fn latest(&mut self) -> Option<(f64, Quat)> {
			// SAFETY: Reads the newest sample Core Motion stored.
			let motion = unsafe { self.manager.deviceMotion() }?;
			// SAFETY: Property reads on a retained sample.
			let (time, attitude) = unsafe { (motion.timestamp(), motion.attitude().quaternion()) };
			let quat = Quat::new(
				attitude.x as f32,
				attitude.y as f32,
				attitude.z as f32,
				attitude.w as f32,
			);

			Some((time, quat.normalized()?))
		}

		fn stop(&mut self) {
			// SAFETY: Stops the updates `start` began.
			unsafe { self.manager.stopDeviceMotionUpdates() };
		}
	}
}

#[cfg(target_os = "android")]
mod android {
	use std::ffi::c_char;
	use std::ffi::c_int;
	use std::ffi::c_void;
	use std::ptr::NonNull;

	use leptoskit_bridge::Error;

	use super::RATE;
	use crate::host::OrientationSource;
	use crate::math::Quat;

	/// `ASENSOR_TYPE_GAME_ROTATION_VECTOR`: rotation without the compass, so
	/// the heading never jumps near metal.
	const GAME_ROTATION_VECTOR: c_int = 15;
	/// `ASENSOR_TYPE_ROTATION_VECTOR`, for devices without the game vector.
	const ROTATION_VECTOR: c_int = 11;
	/// `ALOOPER_PREPARE_ALLOW_NON_CALLBACKS`.
	const ALLOW_NON_CALLBACKS: c_int = 1;
	/// Events read per update; only the newest matters.
	const BATCH: usize = 16;

	#[repr(C)]
	struct Opaque([u8; 0]);

	/// `ASensorEvent`, as `<android/sensor.h>` lays it out.
	#[repr(C)]
	#[derive(Clone, Copy)]
	struct SensorEvent {
		version: i32,
		sensor: i32,
		kind: i32,
		reserved0: i32,
		timestamp: i64,
		data: [f32; 16],
		flags: u32,
		reserved1: [i32; 3],
	}

	#[link(name = "android")]
	unsafe extern "C" {
		fn ASensorManager_getInstanceForPackage(package: *const c_char) -> *mut Opaque;
		fn ASensorManager_getDefaultSensor(manager: *mut Opaque, kind: c_int) -> *const Opaque;
		fn ASensorManager_createEventQueue(
			manager: *mut Opaque,
			looper: *mut Opaque,
			ident: c_int,
			callback: *const c_void,
			data: *mut c_void,
		) -> *mut Opaque;
		fn ASensorManager_destroyEventQueue(manager: *mut Opaque, queue: *mut Opaque) -> c_int;
		fn ASensorEventQueue_enableSensor(queue: *mut Opaque, sensor: *const Opaque) -> c_int;
		fn ASensorEventQueue_disableSensor(queue: *mut Opaque, sensor: *const Opaque) -> c_int;
		fn ASensorEventQueue_setEventRate(
			queue: *mut Opaque,
			sensor: *const Opaque,
			microseconds: i32,
		) -> c_int;
		fn ASensorEventQueue_getEvents(
			queue: *mut Opaque,
			events: *mut SensorEvent,
			count: usize,
		) -> isize;
		fn ALooper_prepare(options: c_int) -> *mut Opaque;
	}

	/// The NDK's game rotation vector.
	///
	/// The event queue is made on the first read, on the thread that drives
	/// the session, because the NDK ties a queue to that thread's looper.
	#[derive(Debug)]
	pub struct DeviceRotation {
		manager: Option<NonNull<Opaque>>,
		sensor: Option<NonNull<Opaque>>,
		queue: Option<NonNull<Opaque>>,
		running: bool,
		latest: Option<(f64, Quat)>,
	}

	// SAFETY: The sensor manager and sensor handles are process-wide; the
	// queue is only touched by the runtime, one thread at a time.
	unsafe impl Send for DeviceRotation {}

	impl DeviceRotation {
		/// The device's sensor.
		#[must_use]
		pub fn new() -> Self {
			// SAFETY: A null package name is allowed; it only labels the
			// client.
			let manager =
				NonNull::new(unsafe { ASensorManager_getInstanceForPackage(std::ptr::null()) });
			let sensor = manager.and_then(|manager| {
				[GAME_ROTATION_VECTOR, ROTATION_VECTOR]
					.into_iter()
					// SAFETY: `manager` is the live process-wide manager.
					.find_map(|kind| {
						NonNull::new(
							unsafe { ASensorManager_getDefaultSensor(manager.as_ptr(), kind) }
								.cast_mut(),
						)
					})
			});

			Self {
				manager,
				sensor,
				queue: None,
				running: false,
				latest: None,
			}
		}

		fn open(&mut self) -> Option<NonNull<Opaque>> {
			if let Some(queue) = self.queue {
				return Some(queue);
			}

			let (manager, sensor) = (self.manager?, self.sensor?);
			// SAFETY: Prepares (or returns) this thread's looper, then makes a
			// queue on it and turns the sensor on at the display's rate.
			let queue = unsafe {
				let looper = ALooper_prepare(ALLOW_NON_CALLBACKS);
				let queue = NonNull::new(ASensorManager_createEventQueue(
					manager.as_ptr(),
					looper,
					0,
					std::ptr::null(),
					std::ptr::null_mut(),
				))?;
				ASensorEventQueue_enableSensor(queue.as_ptr(), sensor.as_ptr());
				ASensorEventQueue_setEventRate(
					queue.as_ptr(),
					sensor.as_ptr(),
					(1_000_000.0 / RATE) as i32,
				);
				queue
			};
			self.queue = Some(queue);
			Some(queue)
		}
	}

	impl Default for DeviceRotation {
		fn default() -> Self {
			Self::new()
		}
	}

	impl OrientationSource for DeviceRotation {
		fn available(&self) -> bool {
			self.sensor.is_some()
		}

		fn start(&mut self) -> Result<(), Error> {
			if !self.available() {
				return Err(Error::Unavailable);
			}

			self.running = true;
			self.latest = None;
			Ok(())
		}

		fn latest(&mut self) -> Option<(f64, Quat)> {
			if !self.running {
				return None;
			}

			let queue = self.open()?;
			let mut events = [SensorEvent {
				version: 0,
				sensor: 0,
				kind: 0,
				reserved0: 0,
				timestamp: 0,
				data: [0.0; 16],
				flags: 0,
				reserved1: [0; 3],
			}; BATCH];

			loop {
				// SAFETY: `events` has room for `BATCH` events.
				let read = unsafe {
					ASensorEventQueue_getEvents(queue.as_ptr(), events.as_mut_ptr(), BATCH)
				};
				let Ok(read @ 1..) = usize::try_from(read) else {
					break;
				};

				if let Some(event) = events[..read].last() {
					let [x, y, z, w] = [event.data[0], event.data[1], event.data[2], event.data[3]];
					if let Some(quat) = Quat::new(x, y, z, w).normalized() {
						self.latest = Some((event.timestamp as f64 / 1e9, quat));
					}
				}

				if read < BATCH {
					break;
				}
			}

			self.latest
		}

		fn stop(&mut self) {
			self.running = false;

			if let (Some(manager), Some(sensor), Some(queue)) =
				(self.manager, self.sensor, self.queue.take())
			{
				// SAFETY: Turns the sensor off and frees the queue `open` made.
				unsafe {
					ASensorEventQueue_disableSensor(queue.as_ptr(), sensor.as_ptr());
					ASensorManager_destroyEventQueue(manager.as_ptr(), queue.as_ptr());
				}
			}
		}
	}

	impl Drop for DeviceRotation {
		fn drop(&mut self) {
			self.stop();
		}
	}
}
