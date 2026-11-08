//! Small helpers for calling WebXR through `Reflect`, which keeps this crate
//! off `web-sys`'s unstable WebXR bindings.

use js_sys::Array;
use js_sys::Float32Array;
use js_sys::Function;
use js_sys::Object;
use js_sys::Promise;
use js_sys::Reflect;
use leptoskit_bridge::Error;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use crate::math::Pose;
use crate::math::Quat;
use crate::math::Vec3;

/// `object[name]`, or `undefined`.
pub(super) fn get(object: &JsValue, name: &str) -> JsValue {
	Reflect::get(object, &JsValue::from_str(name)).unwrap_or(JsValue::UNDEFINED)
}

/// `object[name] = value`.
pub(super) fn set(object: &JsValue, name: &str, value: &JsValue) {
	let _ = Reflect::set(object, &JsValue::from_str(name), value);
}

/// `object[name](...arguments)`.
pub(super) fn call(object: &JsValue, name: &str, arguments: &[JsValue]) -> Result<JsValue, Error> {
	let method: Function = get(object, name)
		.dyn_into()
		.map_err(|_| Error::Unavailable)?;
	let arguments: Array = arguments.iter().collect();
	Reflect::apply(&method, object, &arguments).map_err(|_| Error::Native)
}

/// Awaits a promise-returning call.
pub(super) async fn await_call(
	object: &JsValue,
	name: &str,
	arguments: &[JsValue],
) -> Result<JsValue, Error> {
	let promise: Promise = call(object, name, arguments)?
		.dyn_into()
		.map_err(|_| Error::Native)?;
	JsFuture::from(promise).await.map_err(|error| {
		match get(&error, "name").as_string().as_deref() {
			Some("NotAllowedError" | "SecurityError") => Error::PermissionDenied,
			Some("NotSupportedError") => Error::Unavailable,
			_ => Error::Native,
		}
	})
}

/// A plain object with these properties.
pub(super) fn object(entries: &[(&str, JsValue)]) -> JsValue {
	let object = Object::new();

	for (name, value) in entries {
		set(&object, name, value);
	}

	object.into()
}

/// A JavaScript array of strings.
pub(super) fn strings(values: &[&str]) -> JsValue {
	values
		.iter()
		.map(|value| JsValue::from_str(value))
		.collect::<Array>()
		.into()
}

/// A number property, or `None`.
pub(super) fn number(object: &JsValue, name: &str) -> Option<f64> {
	get(object, name).as_f64()
}

/// The values of an iterable (`XRInputSourceArray`, `Set`) as an array.
pub(super) fn items(iterable: &JsValue) -> Array {
	Array::from(iterable)
}

/// An `XRRigidTransform`'s position and orientation.
pub(super) fn pose(transform: &JsValue) -> Option<Pose> {
	let position = get(transform, "position");
	let orientation = get(transform, "orientation");
	let read = |object: &JsValue, name: &str| number(object, name).map(|value| value as f32);

	Some(Pose::new(
		Vec3::new(
			read(&position, "x")?,
			read(&position, "y")?,
			read(&position, "z")?,
		),
		Quat::new(
			read(&orientation, "x")?,
			read(&orientation, "y")?,
			read(&orientation, "z")?,
			read(&orientation, "w")?,
		)
		.normalized()?,
	))
}

/// A `Float32Array` of sixteen values as a matrix.
pub(super) fn matrix(value: &JsValue) -> Option<[f32; 16]> {
	let array: Float32Array = value.clone().dyn_into().ok()?;

	if array.length() != 16 {
		return None;
	}

	let mut matrix = [0.0; 16];
	array.copy_to(&mut matrix);
	Some(matrix)
}

/// `new XRRigidTransform(position, orientation)`.
pub(super) fn rigid_transform(pose: Pose) -> Result<JsValue, Error> {
	let constructor: Function = get(&js_sys::global(), "XRRigidTransform")
		.dyn_into()
		.map_err(|_| Error::Unavailable)?;
	let Pose {
		position,
		orientation,
	} = pose;
	let arguments: Array = [
		object(&[
			("x", position.x.into()),
			("y", position.y.into()),
			("z", position.z.into()),
		]),
		object(&[
			("x", orientation.x.into()),
			("y", orientation.y.into()),
			("z", orientation.z.into()),
			("w", orientation.w.into()),
		]),
	]
	.into_iter()
	.collect();

	Reflect::construct(&constructor, &arguments).map_err(|_| Error::Native)
}
