//! WebXR's input sources: controllers, hands, screen touches, and gaze.

use wasm_bindgen::JsValue;

use super::js;
use crate::types::*;

/// Every input source in `session`, located in `space`.
pub(super) fn inputs(session: &JsValue, xr_frame: &JsValue, space: &JsValue) -> Vec<InputSource> {
	js::items(&js::get(session, "inputSources"))
		.iter()
		.enumerate()
		.filter_map(|(index, source)| input(index as u32, &source, xr_frame, space))
		.take(Frame::MAX_INPUTS)
		.collect()
}

fn located(xr_frame: &JsValue, target: &JsValue, space: &JsValue) -> Option<crate::math::Pose> {
	if target.is_undefined() || target.is_null() {
		return None;
	}

	let pose = js::call(xr_frame, "getPose", &[target.clone(), space.clone()]).ok()?;
	js::pose(&js::get(&pose, "transform"))
}

fn input(id: u32, source: &JsValue, xr_frame: &JsValue, space: &JsValue) -> Option<InputSource> {
	let handedness = match js::get(source, "handedness").as_string().as_deref() {
		Some("left") => Handedness::Left,
		Some("right") => Handedness::Right,
		_ => Handedness::None,
	};
	let hand = js::get(source, "hand");
	let kind = match js::get(source, "targetRayMode").as_string().as_deref() {
		Some("screen") => InputKind::Screen,
		Some("gaze") => InputKind::Gaze,
		_ if !hand.is_undefined() && !hand.is_null() => InputKind::Hand,
		_ => InputKind::Controller,
	};
	let gamepad = js::get(source, "gamepad");
	let buttons = if gamepad.is_undefined() || gamepad.is_null() {
		Vec::new()
	} else {
		js::items(&js::get(&gamepad, "buttons"))
			.iter()
			.take(InputSource::MAX_BUTTONS)
			.map(|button| {
				Button {
					pressed: js::get(&button, "pressed").as_bool() == Some(true),
					touched: js::get(&button, "touched").as_bool() == Some(true),
					value: js::number(&button, "value").unwrap_or(0.0) as f32,
				}
			})
			.collect()
	};
	let axes = if gamepad.is_undefined() || gamepad.is_null() {
		Vec::new()
	} else {
		// The xr-standard mapping puts the touchpad at 0–1 and the
		// thumbstick at 2–3; keep the thumbstick when there is one.
		let axes: Vec<f32> = js::items(&js::get(&gamepad, "axes"))
			.iter()
			.filter_map(|axis| axis.as_f64().map(|value| value as f32))
			.collect();
		if axes.len() >= 4 {
			axes[2..4].to_vec()
		} else {
			axes.into_iter().take(InputSource::MAX_AXES).collect()
		}
	};

	Some(InputSource {
		id,
		handedness,
		kind,
		target_ray: located(xr_frame, &js::get(source, "targetRaySpace"), space)?,
		grip: located(xr_frame, &js::get(source, "gripSpace"), space),
		buttons,
		axes,
		hand: joints(&hand, xr_frame, space),
	})
}

/// A tracked hand's 26 joints, in WebXR's order, which is OpenXR's.
fn joints(
	hand: &JsValue,
	xr_frame: &JsValue,
	space: &JsValue,
) -> Option<Box<[Joint; HAND_JOINTS]>> {
	if hand.is_undefined() || hand.is_null() {
		return None;
	}

	let spaces = js::items(&js::call(hand, "values", &[]).ok()?);
	let mut joints = Box::new([Joint::default(); HAND_JOINTS]);

	for (joint, joint_space) in joints.iter_mut().zip(spaces.iter()) {
		let Ok(pose) = js::call(xr_frame, "getJointPose", &[joint_space, space.clone()]) else {
			continue;
		};

		if let Some(located) = js::pose(&js::get(&pose, "transform")) {
			*joint = Joint {
				pose: located,
				radius: js::number(&pose, "radius").unwrap_or(0.0) as f32,
				tracked: true,
			};
		}
	}

	Some(joints)
}
