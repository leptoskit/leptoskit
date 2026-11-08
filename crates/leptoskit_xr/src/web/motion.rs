//! Looking around by turning, from the `deviceorientation` event, for
//! browsers without WebXR AR.

use std::cell::RefCell;
use std::rc::Rc;

use leptoskit_bridge::Error;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use super::State;
use super::js;
use crate::math::Fov;
use crate::math::Pose;
use crate::math::Projection;
use crate::math::Quat;
use crate::math::Vec3;
use crate::math::attitude_to_viewer;
use crate::math::device_orientation;
use crate::math::heading;
use crate::types::*;

/// The vertical field of view, as on the native orientation backend.
const VERTICAL_FOV: f32 = 1.05;

/// Whether the browser has the event at all; whether the device has the
/// sensor shows only once events arrive.
pub(super) fn available() -> bool {
	!js::get(&js_sys::global(), "DeviceOrientationEvent").is_undefined()
}

/// Asks for motion permission where the browser wants it (Safari on iOS,
/// from a user gesture), then follows the event.
pub(super) async fn start(config: &SessionConfig) -> Result<Rc<RefCell<State>>, Error> {
	if !available() {
		return Err(Error::Unavailable);
	}

	let event = js::get(&js_sys::global(), "DeviceOrientationEvent");

	if !js::get(&event, "requestPermission").is_undefined()
		&& js::await_call(&event, "requestPermission", &[])
			.await?
			.as_string()
			.as_deref()
			!= Some("granted")
	{
		return Err(Error::PermissionDenied);
	}

	let info = SessionInfo {
		mode: Mode::Orientation,
		features: config
			.required
			.union(config.optional)
			.intersection(Features::HIT_TEST | Features::ANCHORS),
		reference_space: ReferenceSpace::Local,
	};
	let state = Rc::new(RefCell::new(State::new(
		info,
		config.frame_rate,
		JsValue::UNDEFINED,
		JsValue::UNDEFINED,
	)));
	listen(&state)?;
	Ok(state)
}

/// The screen's clockwise quarter turns from the device's natural
/// orientation.
fn quarter_turns() -> u32 {
	let screen = js::get(&js_sys::global(), "screen");
	let angle = js::number(&js::get(&screen, "orientation"), "angle").unwrap_or(0.0);

	((angle / 90.0).round() as i64).rem_euclid(4) as u32
}

fn aspect() -> f32 {
	web_sys::window()
		.and_then(|window| {
			let width = window.inner_width().ok()?.as_f64()?;
			let height = window.inner_height().ok()?.as_f64()?;
			(height > 0.0).then(|| (width / height) as f32)
		})
		.unwrap_or(0.5)
}

fn listen(state: &Rc<RefCell<State>>) -> Result<(), Error> {
	let window = web_sys::window().ok_or(Error::Unavailable)?;
	let weak = Rc::downgrade(state);
	let mut recentre: Option<Quat> = None;
	let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
		let Some(state) = weak.upgrade() else {
			return;
		};
		let angle = |name: &str| js::number(&event, name).map(|value| value as f32);
		let (Some(alpha), Some(beta), Some(gamma)) =
			(angle("alpha"), angle("beta"), angle("gamma"))
		else {
			return;
		};
		let viewer = attitude_to_viewer(device_orientation(alpha, beta, gamma), quarter_turns());
		let turn =
			*recentre.get_or_insert_with(|| Quat::from_axis_angle(Vec3::Y, -heading(viewer)));
		let pose = Pose::new(Vec3::ZERO, turn * viewer);
		let time = js::number(&event, "timeStamp").unwrap_or(0.0) / 1000.0;

		state.borrow_mut().publish(Frame {
			sequence: 0,
			time,
			tracking: TrackingState::Normal,
			viewer: pose,
			views: vec![View {
				pose,
				projection: Projection::Fov(Fov::symmetric(VERTICAL_FOV, aspect())),
			}],
			planes: Vec::new(),
			planes_revision: 0,
			anchors: Vec::new(),
			light: None,
			inputs: Vec::new(),
			depth: false,
		});
	});
	window
		.add_event_listener_with_callback("deviceorientation", listener.as_ref().unchecked_ref())
		.map_err(|_| Error::Native)?;
	state.borrow_mut().listener = Some(listener);
	Ok(())
}
