//! Secure-context browser fused motion with cancellation-owned listeners.
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use futures::channel::oneshot;
use gloo_events::EventListener;
use js_sys::Function;
use js_sys::Reflect;
use leptoskit_bridge::Error;
use leptoskit_sensor::Interval;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;

use crate::Reading;

thread_local! {
	static BUSY: Cell<bool> = const { Cell::new(false) };
}

struct Reservation;
impl Reservation {
	fn acquire() -> Result<Self, Error> {
		BUSY.with(|busy| {
			if busy.get() {
				return Err(Error::Busy);
			}
			busy.set(true);
			Ok(Self)
		})
	}
}
impl Drop for Reservation {
	fn drop(&mut self) {
		BUSY.with(|busy| busy.set(false));
	}
}

fn property(value: &JsValue, name: &str) -> Result<JsValue, Error> {
	Reflect::get(value, &JsValue::from_str(name)).map_err(|_| Error::Native)
}

fn number(value: &JsValue, name: &str) -> Result<f64, Error> {
	property(value, name)?
		.as_f64()
		.filter(|value| value.is_finite())
		.ok_or(Error::Unavailable)
}

fn window() -> Result<web_sys::Window, Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	if property(window.as_ref(), "isSecureContext")?.as_bool() != Some(true) {
		return Err(Error::PermissionDenied);
	}
	Ok(window)
}

fn has_constructor(name: &str) -> Result<bool, Error> {
	Ok(property(window()?.as_ref(), name)?.is_function())
}

/// Tests API presence. Hardware and permission are checked during acquisition.
pub fn available() -> Result<bool, Error> {
	Ok(has_constructor("DeviceMotionEvent")? && has_constructor("DeviceOrientationEvent")?)
}

async fn request_named_permission(constructor_name: &str) -> Result<(), Error> {
	let constructor = property(window()?.as_ref(), constructor_name)?;
	if !constructor.is_function() {
		return Err(Error::Unavailable);
	}
	let request = property(&constructor, "requestPermission")?;
	if request.is_undefined() {
		return Ok(());
	}
	let request: Function = request.dyn_into().map_err(|_| Error::Unavailable)?;
	let promise = request
		.call0(&constructor)
		.map_err(|_| Error::PermissionDenied)?;
	let status = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&promise))
		.await
		.map_err(|_| Error::PermissionDenied)?;
	if status.as_string().as_deref() == Some("denied") {
		Err(Error::PermissionDenied)
	} else {
		Ok(())
	}
}

/// Requests Safari's motion and orientation permissions when applicable. Call
/// from a user gesture.
pub async fn request_permission() -> Result<(), Error> {
	request_named_permission("DeviceMotionEvent").await?;
	request_named_permission("DeviceOrientationEvent").await
}

/// Acceleration triples from a `DeviceMotion` property; null means unavailable.
fn acceleration(event: &web_sys::Event, name: &str) -> Result<(f64, f64, f64), Error> {
	let values = property(event.as_ref(), name)?;
	if values.is_null() || values.is_undefined() {
		return Err(Error::Unavailable);
	}
	Ok((
		number(&values, "x")?,
		number(&values, "y")?,
		number(&values, "z")?,
	))
}

/// `rotationRate` beta/gamma/alpha are degrees per second around x/y/z.
fn rotation_rate(event: &web_sys::Event) -> Result<(f64, f64, f64), Error> {
	let values = property(event.as_ref(), "rotationRate")?;
	if values.is_null() || values.is_undefined() {
		return Err(Error::Unavailable);
	}
	Ok((
		number(&values, "beta")?.to_radians(),
		number(&values, "gamma")?.to_radians(),
		number(&values, "alpha")?.to_radians(),
	))
}

fn attitude(event: &web_sys::Event) -> Result<(f64, f64, f64), Error> {
	Ok((
		number(event.as_ref(), "alpha")?.to_radians(),
		number(event.as_ref(), "beta")?.to_radians(),
		number(event.as_ref(), "gamma")?.to_radians(),
	))
}

fn parse_motion(event: &web_sys::Event) -> Result<PartialMotion, Error> {
	let (accel_x, accel_y, accel_z) = acceleration(event, "acceleration")?;
	let (including_x, including_y, including_z) =
		acceleration(event, "accelerationIncludingGravity")?;
	let (rotation_x, rotation_y, rotation_z) = rotation_rate(event)?;
	Ok(PartialMotion {
		timestamp: event.time_stamp() / 1000.0,
		accel_x,
		accel_y,
		accel_z,
		gravity_x: including_x - accel_x,
		gravity_y: including_y - accel_y,
		gravity_z: including_z - accel_z,
		rotation_x,
		rotation_y,
		rotation_z,
	})
}

#[derive(Clone, Copy)]
struct PartialMotion {
	timestamp: f64,
	accel_x: f64,
	accel_y: f64,
	accel_z: f64,
	gravity_x: f64,
	gravity_y: f64,
	gravity_z: f64,
	rotation_x: f64,
	rotation_y: f64,
	rotation_z: f64,
}

/// Acquires one fused reading. Cancellation stops listeners.
pub async fn next(_interval: Interval) -> Result<Reading, Error> {
	let _reservation = Reservation::acquire()?;
	request_permission().await?;

	let window = window()?;
	let (motion_sender, motion_receiver) = oneshot::channel();
	let motion_sender = Rc::new(RefCell::new(Some(motion_sender)));
	let motion_listener = EventListener::new(&window, "devicemotion", {
		let motion_sender = Rc::clone(&motion_sender);
		move |event| {
			if let Some(sender) = motion_sender.borrow_mut().take() {
				let _ = sender.send(parse_motion(event));
			}
		}
	});

	let (attitude_sender, attitude_receiver) = oneshot::channel();
	let attitude_sender = Rc::new(RefCell::new(Some(attitude_sender)));
	let attitude_listener = EventListener::new(&window, "deviceorientation", {
		let attitude_sender = Rc::clone(&attitude_sender);
		move |event| {
			if let Some(sender) = attitude_sender.borrow_mut().take() {
				let _ = sender.send(attitude(event));
			}
		}
	});

	let motion = motion_receiver.await.map_err(|_| Error::Cancelled)??;
	let (attitude_alpha, attitude_beta, attitude_gamma) =
		attitude_receiver.await.map_err(|_| Error::Cancelled)??;

	drop(motion_listener);
	drop(attitude_listener);

	let reading = Reading {
		timestamp: motion.timestamp,
		accel_x: motion.accel_x,
		accel_y: motion.accel_y,
		accel_z: motion.accel_z,
		gravity_x: motion.gravity_x,
		gravity_y: motion.gravity_y,
		gravity_z: motion.gravity_z,
		rotation_x: motion.rotation_x,
		rotation_y: motion.rotation_y,
		rotation_z: motion.rotation_z,
		attitude_alpha,
		attitude_beta,
		attitude_gamma,
	};
	reading.encode()?;
	Ok(reading)
}
