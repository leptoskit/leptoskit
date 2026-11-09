//! WebXR sessions: starting one, reading each `XRFrame`, hit-test sources,
//! and anchors.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::Array;
use leptoskit_bridge::Error;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use super::HitSlot;
use super::State;
use super::js;
use crate::image::DepthFormat;
use crate::image::DepthImage;
use crate::math::Fov;
use crate::math::Pose;
use crate::math::Projection;
use crate::math::Ray;
use crate::types::*;

/// The WebXR feature names for a session's features.
fn feature_names(features: Features) -> Vec<&'static str> {
	let mut names = vec!["local-floor"];

	for (feature, name) in [
		(Features::HIT_TEST, "hit-test"),
		(Features::ANCHORS, "anchors"),
		(Features::DEPTH, "depth-sensing"),
		(Features::PLANES, "plane-detection"),
		(Features::HANDS, "hand-tracking"),
	] {
		if features.intersects(feature) {
			names.push(name);
		}
	}

	names
}

/// The features the browser granted, from `XRSession.enabledFeatures` where
/// it reports them, or everything asked for where it doesn't.
fn granted(session: &JsValue, asked: Features) -> Features {
	let enabled = js::get(session, "enabledFeatures");

	if enabled.is_undefined() {
		return asked;
	}

	let enabled: Vec<String> = js::items(&enabled)
		.iter()
		.filter_map(|name| name.as_string())
		.collect();
	let has = |name: &str| enabled.iter().any(|enabled| enabled == name);
	let mut features =
		asked.intersection(Features::CONTROLLERS | Features::PASSTHROUGH | Features::CAMERA_IMAGE);

	for (feature, name) in [
		(Features::HIT_TEST, "hit-test"),
		(Features::ANCHORS, "anchors"),
		(Features::DEPTH, "depth-sensing"),
		(Features::PLANES, "plane-detection"),
		(Features::HANDS, "hand-tracking"),
	] {
		if asked.intersects(feature) && has(name) {
			features = features | asked.intersection(feature);
		}
	}

	features
}

/// Starts an immersive session, gives it a WebGL layer so the browser runs
/// its frames, and begins reading them.
pub(super) async fn start(
	system: &JsValue,
	mode: &str,
	config: &SessionConfig,
) -> Result<Rc<RefCell<State>>, Error> {
	if system.is_undefined() {
		return Err(Error::Unavailable);
	}

	let required: Vec<&str> = feature_names(config.required)
		.into_iter()
		.filter(|name| *name != "local-floor")
		.collect();
	let optional = feature_names(config.optional | config.required);
	let init = js::object(&[
		("requiredFeatures", js::strings(&required)),
		("optionalFeatures", js::strings(&optional)),
		(
			"depthSensing",
			js::object(&[
				("usagePreference", js::strings(&["cpu-optimized"])),
				(
					"dataFormatPreference",
					js::strings(&["luminance-alpha", "float32"]),
				),
			]),
		),
	]);
	let session = js::await_call(system, "requestSession", &[mode.into(), init]).await?;
	attach_layer(&session)?;
	let (space, reference_space) = reference_space(&session, config.reference_space).await?;
	let features = granted(&session, config.required | config.optional);

	if !features.contains(config.required) {
		let _ = js::call(&session, "end", &[]);

		return Err(Error::Unavailable);
	}

	let info = SessionInfo {
		mode: config.mode,
		features,
		reference_space,
	};
	let state = Rc::new(RefCell::new(State::new(
		info,
		config.frame_rate,
		session.clone(),
		space,
	)));
	super::on_end(&session, &state);
	schedule(&state);
	Ok(state)
}

/// Sets an `XRWebGLLayer` on a fresh WebGL 2 context: an immersive session
/// runs no frames without a layer. A renderer replaces it with its own.
fn attach_layer(session: &JsValue) -> Result<(), Error> {
	let document = web_sys::window()
		.and_then(|window| window.document())
		.ok_or(Error::Unavailable)?;
	let canvas = js::call(&document, "createElement", &["canvas".into()])?;
	let options = js::object(&[("xrCompatible", JsValue::TRUE), ("alpha", JsValue::TRUE)]);
	let gl = js::call(&canvas, "getContext", &["webgl2".into(), options])?;
	let constructor: js_sys::Function = js::get(&js_sys::global(), "XRWebGLLayer")
		.dyn_into()
		.map_err(|_| Error::Unavailable)?;
	let arguments: Array = [session.clone(), gl].into_iter().collect();
	let layer = js_sys::Reflect::construct(&constructor, &arguments).map_err(|_| Error::Native)?;
	js::call(
		session,
		"updateRenderState",
		&[js::object(&[("baseLayer", layer)])],
	)?;
	Ok(())
}

/// The nearest reference space the browser has: `unbounded`, then
/// `local-floor`, then `local` for a world request.
async fn reference_space(
	session: &JsValue,
	wanted: ReferenceSpace,
) -> Result<(JsValue, ReferenceSpace), Error> {
	let candidates: &[(&str, ReferenceSpace)] = match wanted {
		ReferenceSpace::World => {
			&[
				("unbounded", ReferenceSpace::World),
				("local-floor", ReferenceSpace::LocalFloor),
				("local", ReferenceSpace::Local),
			]
		}
		ReferenceSpace::LocalFloor => {
			&[
				("local-floor", ReferenceSpace::LocalFloor),
				("local", ReferenceSpace::Local),
			]
		}
		ReferenceSpace::Local => &[("local", ReferenceSpace::Local)],
	};

	for (name, space) in candidates {
		if let Ok(handle) =
			js::await_call(session, "requestReferenceSpace", &[(*name).into()]).await
		{
			return Ok((handle, *space));
		}
	}

	Err(Error::Unavailable)
}

/// Asks for the next frame, reading it when it comes and asking again.
fn schedule(state: &Rc<RefCell<State>>) {
	let weak = Rc::downgrade(state);
	let callback = Closure::<dyn FnMut(f64, JsValue)>::new(move |time: f64, frame: JsValue| {
		let Some(state) = weak.upgrade() else {
			return;
		};

		if state.borrow().ended {
			return;
		}

		read(&state, time, &frame);
		let state = state.borrow();

		if let Some(callback) = &state.callback {
			let _ = js::call(
				&state.session,
				"requestAnimationFrame",
				&[callback.as_ref().clone()],
			);
		}
	});
	let session = state.borrow().session.clone();
	let _ = js::call(
		&session,
		"requestAnimationFrame",
		&[callback.as_ref().clone()],
	);
	state.borrow_mut().callback = Some(callback);
}

/// Reads one `XRFrame` into the session.
fn read(state: &Rc<RefCell<State>>, time: f64, xr_frame: &JsValue) {
	let (space, session, features) = {
		let state = state.borrow();
		(
			state.space.clone(),
			state.session.clone(),
			state.info.features,
		)
	};
	let pose =
		js::call(xr_frame, "getViewerPose", std::slice::from_ref(&space)).unwrap_or(JsValue::NULL);

	if pose.is_null() || pose.is_undefined() {
		return;
	}

	let views: Vec<View> = js::items(&js::get(&pose, "views"))
		.iter()
		.filter_map(|view| view_of(&view))
		.collect();
	let tracking = if js::get(&pose, "emulatedPosition").as_bool() == Some(true) {
		TrackingState::Limited(LimitedReason::Unknown)
	} else {
		TrackingState::Normal
	};

	let viewer = js::pose(&js::get(&pose, "transform")).unwrap_or(Pose::IDENTITY);
	let planes = planes(&mut state.borrow_mut(), xr_frame, &space);
	let anchors = anchors(&state.borrow(), xr_frame, &space);

	let inputs = super::inputs::inputs(&session, xr_frame, &space);
	let depth = if features.contains(Features::DEPTH) {
		js::items(&js::get(&pose, "views"))
			.iter()
			.next()
			.and_then(|view| depth(xr_frame, &view))
	} else {
		None
	};

	resolve_hits(&mut state.borrow_mut(), xr_frame, &space);
	let mut state = state.borrow_mut();
	let has_depth = depth.is_some();
	state.depth = depth;
	state.publish(Frame {
		sequence: 0,
		time: time / 1000.0,
		tracking,
		viewer,
		views,
		planes,
		planes_revision: 0,
		anchors,
		light: None,
		inputs,
		depth: has_depth.then_some(DepthSource::Measured),
	});
}

fn view_of(view: &JsValue) -> Option<View> {
	let pose = js::pose(&js::get(view, "transform"))?;
	let matrix = js::matrix(&js::get(view, "projectionMatrix"))?;
	let projection = Fov::from_matrix(&matrix).map_or(Projection::Matrix(matrix), Projection::Fov);
	Some(View { pose, projection })
}

fn label(name: Option<&str>) -> PlaneLabel {
	match name {
		Some("floor") => PlaneLabel::Floor,
		Some("wall") => PlaneLabel::Wall,
		Some("ceiling") => PlaneLabel::Ceiling,
		Some("table" | "desk") => PlaneLabel::Table,
		Some("couch" | "seat") => PlaneLabel::Seat,
		Some("door") => PlaneLabel::Door,
		Some("window") => PlaneLabel::Window,
		_ => PlaneLabel::Unknown,
	}
}

/// `XRFrame.detectedPlanes`, where the browser detects planes.
fn planes(state: &mut State, xr_frame: &JsValue, space: &JsValue) -> Vec<Plane> {
	let detected = js::get(xr_frame, "detectedPlanes");

	if detected.is_undefined() {
		return Vec::new();
	}

	js::items(&detected)
		.iter()
		.filter_map(|plane| {
			let located = js::call(
				xr_frame,
				"getPose",
				&[js::get(&plane, "planeSpace"), space.clone()],
			)
			.ok()?;
			let pose = js::pose(&js::get(&located, "transform"))?;
			let polygon: Vec<[f32; 2]> = js::items(&js::get(&plane, "polygon"))
				.iter()
				.filter_map(|point| {
					Some([
						js::number(&point, "x")? as f32,
						js::number(&point, "z")? as f32,
					])
				})
				.collect();
			let (min, max) =
				polygon
					.iter()
					.fold(([f32::MAX; 2], [f32::MIN; 2]), |(min, max), point| {
						(
							[min[0].min(point[0]), min[1].min(point[1])],
							[max[0].max(point[0]), max[1].max(point[1])],
						)
					});
			let vertical =
				js::get(&plane, "orientation").as_string().as_deref() == Some("vertical");

			Some(Plane {
				id: plane_id(state, &plane),
				orientation: if vertical {
					PlaneOrientation::Vertical
				} else if pose.transform_vector(crate::math::Vec3::Y).y >= 0.0 {
					PlaneOrientation::HorizontalUp
				} else {
					PlaneOrientation::HorizontalDown
				},

				label: label(js::get(&plane, "semanticLabel").as_string().as_deref()),
				pose,
				extent: [(max[0] - min[0]).max(0.0), (max[1] - min[1]).max(0.0)],
				polygon: thin_polygon(&polygon),
			})
		})
		.collect()
}

fn plane_id(state: &mut State, plane: &JsValue) -> u64 {
	if let Some(id) = state.plane_ids.get(plane).as_f64() {
		return id as u64;
	}

	state.next_plane += 1;
	state
		.plane_ids
		.set(plane, &JsValue::from_f64(state.next_plane as f64));
	state.next_plane
}

fn anchors(state: &State, xr_frame: &JsValue, space: &JsValue) -> Vec<Anchor> {
	let tracked = js::get(xr_frame, "trackedAnchors");

	state
		.anchors
		.iter()
		.map(|(id, anchor)| {
			let tracking = !tracked.is_undefined()
				&& js::call(&tracked, "has", std::slice::from_ref(anchor))
					.is_ok_and(|has| has.as_bool() == Some(true));
			let pose = js::call(
				xr_frame,
				"getPose",
				&[js::get(anchor, "anchorSpace"), space.clone()],
			)
			.ok()
			.and_then(|located| js::pose(&js::get(&located, "transform")));

			Anchor {
				id: *id,
				pose: pose.unwrap_or(Pose::IDENTITY),
				tracking: if tracking && pose.is_some() {
					TrackingState::Normal
				} else {
					TrackingState::NotAvailable
				},
			}
		})
		.collect()
}

/// The CPU depth picture for `view`, where the session shares one.
fn depth(xr_frame: &JsValue, view: &JsValue) -> Option<DepthImage> {
	let information = js::call(xr_frame, "getDepthInformation", std::slice::from_ref(view)).ok()?;

	if information.is_null() || information.is_undefined() {
		return None;
	}

	let width = js::number(&information, "width")? as u32;
	let height = js::number(&information, "height")? as u32;
	let scale = js::number(&information, "rawValueToMeters")? as f32;
	let bytes = js_sys::Uint8Array::new(&js::get(&information, "data")).to_vec();
	let float = js::get(&information, "depthDataFormat")
		.as_string()
		.as_deref()
		== Some("float32");
	let to_view = js::matrix(&js::get(
		&js::get(&information, "normDepthBufferFromNormView"),
		"matrix",
	))?;
	// Store millimetres or metres directly when the browser's scale is
	// one of them; otherwise convert to metres.
	let (format, data): (DepthFormat, Vec<u8>) = if !float && (scale - 0.001).abs() < 1e-6 {
		(DepthFormat::Millimeters16, bytes)
	} else if float && (scale - 1.0).abs() < 1e-6 {
		(DepthFormat::Meters32, bytes)
	} else {
		let meters: Vec<u8> = if float {
			bytes
				.as_chunks::<4>()
				.0
				.iter()
				.flat_map(|raw| (f32::from_le_bytes(*raw) * scale).to_le_bytes())
				.collect()
		} else {
			bytes
				.as_chunks::<2>()
				.0
				.iter()
				.flat_map(|raw| (f32::from(u16::from_le_bytes(*raw)) * scale).to_le_bytes())
				.collect()
		};

		(DepthFormat::Meters32, meters)
	};

	Some(DepthImage {
		width,
		height,
		format,
		row_stride: width * format.bytes(),
		data: data.into(),
		confidence: None,
		display_transform: [
			to_view[0],
			to_view[1],
			to_view[4],
			to_view[5],
			to_view[12],
			to_view[13],
		],
	})
}

/// Asks the browser for a hit-test source along `ray`; its results arrive
/// with the next frame.
pub(super) async fn request_hits(
	session: &JsValue,
	space: &JsValue,
	ray: Ray,
	state: &Rc<RefCell<State>>,
) -> Result<Rc<RefCell<HitSlot>>, Error> {
	let constructor: js_sys::Function = js::get(&js_sys::global(), "XRRay")
		.dyn_into()
		.map_err(|_| Error::Unavailable)?;
	let point = |x: f32, y: f32, z: f32, w: f32| {
		js::object(&[
			("x", x.into()),
			("y", y.into()),
			("z", z.into()),
			("w", w.into()),
		])
	};
	let arguments: Array = [
		point(ray.origin.x, ray.origin.y, ray.origin.z, 1.0),
		point(ray.direction.x, ray.direction.y, ray.direction.z, 0.0),
	]
	.into_iter()
	.collect();
	let offset_ray =
		js_sys::Reflect::construct(&constructor, &arguments).map_err(|_| Error::Native)?;
	let source = js::await_call(
		session,
		"requestHitTestSource",
		&[js::object(&[
			("space", space.clone()),
			("offsetRay", offset_ray),
		])],
	)
	.await?;
	let slot = Rc::new(RefCell::new(HitSlot::default()));
	state
		.borrow_mut()
		.hit_sources
		.push((source, Rc::clone(&slot)));
	Ok(slot)
}

fn resolve_hits(state: &mut State, xr_frame: &JsValue, space: &JsValue) {
	for (source, slot) in state.hit_sources.drain(..) {
		let results = js::call(xr_frame, "getHitTestResults", std::slice::from_ref(&source))
			.unwrap_or(JsValue::UNDEFINED);
		let hits: Vec<Hit> = js::items(&results)
			.iter()
			.filter_map(|result| {
				let located = js::call(&result, "getPose", std::slice::from_ref(space)).ok()?;
				let pose = js::pose(&js::get(&located, "transform"))?;
				Some(Hit {
					distance: 0.0,
					pose,
					kind: HitKind::Plane,
					plane: None,
				})
			})
			.collect();
		let _ = js::call(&source, "cancel", &[]);
		let mut slot = slot.borrow_mut();
		slot.hits = Some(hits);

		if let Some(waker) = slot.waker.take() {
			waker.wake();
		}
	}
}

/// Pins a point: a WebXR anchor where the browser has them, made in the next
/// frame, or a fixed point.
pub(super) async fn add_anchor(state: &Rc<RefCell<State>>, pose: Pose) -> Result<u64, Error> {
	let (session, space) = {
		let mut state = state.borrow_mut();
		state.next_anchor += 1;
		(state.session.clone(), state.space.clone())
	};
	let id = state.borrow().next_anchor;
	let frame_prototype = js::get(&js::get(&js_sys::global(), "XRFrame"), "prototype");

	if session.is_undefined() || js::get(&frame_prototype, "createAnchor").is_undefined() {
		state.borrow_mut().fixed_anchors.push(Anchor {
			id,
			pose,
			tracking: TrackingState::Normal,
		});

		return Ok(id);
	}

	let anchor = next_frame_anchor(&session, &space, pose).await?;
	state.borrow_mut().anchors.push((id, anchor));
	Ok(id)
}

/// Waits for the next frame and makes an anchor in it.
async fn next_frame_anchor(
	session: &JsValue,
	space: &JsValue,
	pose: Pose,
) -> Result<JsValue, Error> {
	let transform = js::rigid_transform(pose)?;
	let space = space.clone();
	let promise = js_sys::Promise::new(&mut |resolve, reject| {
		let (transform, space) = (transform.clone(), space.clone());
		let callback = Closure::once_into_js(move |_time: f64, frame: JsValue| {
			match js::call(&frame, "createAnchor", &[transform, space]) {
				Ok(promise) => {
					let _ = resolve.call1(&JsValue::UNDEFINED, &promise);
				}
				Err(_) => {
					let _ = reject.call0(&JsValue::UNDEFINED);
				}
			}
		});
		let _ = js::call(session, "requestAnimationFrame", &[callback]);
	});

	wasm_bindgen_futures::JsFuture::from(promise)
		.await
		.map_err(|_| Error::Native)
}
