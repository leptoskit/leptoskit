//! The `xr` plugin's binary protocol, end to end against a recording.

mod support;

use std::future::Future;
use std::task::Poll;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_xr::host::CameraFrame;
use leptoskit_xr::host::DepthEstimate;
use leptoskit_xr::host::DepthEstimator;
use leptoskit_xr::host::ReplayBackend;
use leptoskit_xr::host::XrPlugin;
use leptoskit_xr::host::XrRuntime;
use leptoskit_xr::host::from_context;
use leptoskit_xr::host::install_depth_estimator;
use leptoskit_xr::protocol::*;
use leptoskit_xr::wire::Wire;
use leptoskit_xr::wire::decode;
use leptoskit_xr::wire::encode;
use leptoskit_xr::*;
use support::*;

fn plugin() -> XrPlugin {
	XrPlugin::new(XrRuntime::new(ReplayBackend::new(walk(40))), 30)
}

fn invoke(plugin: &XrPlugin, method: MethodId, request: &impl Wire) -> Result<Vec<u8>, Error> {
	pollster::block_on(plugin.invoke(
		method,
		Bytes::from(encode(request)),
		CancellationToken::new(),
	))
	.map(|reply| reply.bytes().to_vec())
}

#[test]
fn a_guest_session_from_start_to_stop() {
	let plugin = plugin();
	let capabilities: Capabilities = decode(&invoke(&plugin, CAPABILITIES, &()).unwrap()).unwrap();
	assert!(capabilities.supports(Mode::PhoneAr));
	let ready: Availability = decode(
		&invoke(
			&plugin,
			PREPARE,
			&Prepare {
				mode: Mode::PhoneAr,
				install: false,
			},
		)
		.unwrap(),
	)
	.unwrap();
	assert_eq!(ready, Availability::Ready);

	// The host caps the guest's rate at its own maximum.
	let config = SessionConfig::new(Mode::PhoneAr)
		.prefer(Features::PLANES)
		.frame_rate(120);
	let info: SessionInfo = decode(&invoke(&plugin, START, &config).unwrap()).unwrap();
	assert_eq!(info.features, Features::PLANES);

	for index in 0..20 {
		plugin.runtime().update(f64::from(index) / RATE).unwrap();
	}

	let frame: Frame =
		decode(&invoke(&plugin, NEXT_FRAME, &NextFrame::default()).unwrap()).unwrap();
	assert_eq!(frame.planes, vec![floor(2.0)]);
	let hits: Vec<Hit> =
		decode(&invoke(&plugin, HIT_TEST, &HitTestRequest::screen(0.5, 0.5)).unwrap()).unwrap();
	assert_eq!(hits.len(), 1);
	let id: u64 = decode(
		&invoke(
			&plugin,
			ADD_ANCHOR,
			&AddAnchor {
				pose: hits[0].pose,
				plane: hits[0].plane,
			},
		)
		.unwrap(),
	)
	.unwrap();
	invoke(&plugin, REMOVE_ANCHOR, &id).unwrap();
	// The scene a headset would draw holds until the session stops.
	invoke(&plugin, PRESENT, &Present { scene: Some(7) }).unwrap();
	assert_eq!(plugin.runtime().presented(), Some(7));
	invoke(&plugin, STOP, &()).unwrap();
	assert_eq!(plugin.runtime().presented(), None);
	assert_eq!(
		invoke(&plugin, NEXT_FRAME, &NextFrame::default()),
		Err(Error::Cancelled)
	);
}

#[test]
fn a_cancelled_frame_request_ends_at_once() {
	let plugin = plugin();
	invoke(&plugin, START, &SessionConfig::new(Mode::PhoneAr)).unwrap();
	let cancellation = CancellationToken::new();
	let waiting = plugin.invoke(
		NEXT_FRAME,
		Bytes::from(encode(&NextFrame::default())),
		cancellation.clone(),
	);
	let mut waiting = std::pin::pin!(waiting);
	let mut context = std::task::Context::from_waker(std::task::Waker::noop());
	assert!(waiting.as_mut().poll(&mut context).is_pending());
	cancellation.cancel();

	assert!(matches!(
		waiting.as_mut().poll(&mut context),
		Poll::Ready(Err(Error::Cancelled))
	));
}

#[test]
fn malformed_and_unknown_requests_are_refused() {
	let plugin = plugin();
	assert_eq!(
		invoke(
			&plugin,
			START,
			&Prepare {
				mode: Mode::PhoneAr,
				install: false
			}
		),
		Err(Error::InvalidArgument)
	);
	assert_eq!(invoke(&plugin, MethodId(99), &()), Err(Error::Unavailable));
	assert_eq!(plugin.reply_limit(MethodId(99)), Err(Error::Unavailable));
	assert_eq!(
		invoke(&plugin, HIT_TEST, &HitTestRequest::screen(0.5, 0.5)),
		Err(Error::Unavailable)
	);
	assert_eq!(
		invoke(&plugin, CAPABILITIES, &7_u64),
		Err(Error::InvalidArgument)
	);
}

#[test]
fn the_factory_plays_a_recording_named_in_the_options() {
	let directory = std::env::temp_dir().join(format!("leptoskit-xr-{}", std::process::id()));
	std::fs::create_dir_all(&directory).unwrap();
	std::fs::write(directory.join("walk.lkxr"), walk(10).to_bytes()).unwrap();
	let context = leptoskit_bridge::AppContext::new(directory.clone(), directory.clone());
	let plugin =
		from_context(&context, r#"{"replay": "walk.lkxr", "max-frame-rate": 30}"#).unwrap();
	let capabilities: Capabilities = decode(&invoke(&plugin, CAPABILITIES, &()).unwrap()).unwrap();
	assert!(capabilities.supports(Mode::PhoneAr));

	assert!(from_context(&context, r#"{"replay": "missing.lkxr"}"#).is_err());
	assert!(from_context(&context, r#"{"unknown": 1}"#).is_err());

	// A depth model that isn't there, or a path that leaves the assets
	// folder, is logged in the background and never stops the plugin.
	for model in ["depth/missing.onnx", "../outside.onnx"] {
		let options = format!(r#"{{"replay": "walk.lkxr", "depth-model": "{model}"}}"#);
		assert!(from_context(&context, &options).is_ok(), "{model}");
	}
	let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn every_method_has_a_bounded_reply() {
	let plugin = plugin();

	for method in METHODS {
		assert!(plugin.reply_limit(*method).unwrap() <= 8 * 1024 * 1024);
	}

	assert_eq!(plugin.name(), NAME);
}

/// An estimator that never finishes a picture.
struct Blind;

impl DepthEstimator for Blind {
	fn max_side(&self) -> u32 {
		64
	}

	fn submit(&mut self, _frame: std::sync::Arc<CameraFrame>) {}

	fn latest(&mut self) -> Option<DepthEstimate> {
		None
	}
}

#[test]
fn an_installed_depth_estimator_reaches_every_plugin() {
	let directory = std::env::temp_dir().join(format!("leptoskit-xr-depth-{}", std::process::id()));
	std::fs::create_dir_all(&directory).unwrap();
	let mut recording = walk(10);
	recording.capabilities = recording.capabilities.with(
		Mode::PhoneAr,
		Features::PLANES | Features::HIT_TEST | Features::CAMERA_IMAGE,
	);
	std::fs::write(directory.join("walk.lkxr"), recording.to_bytes()).unwrap();
	let context = leptoskit_bridge::AppContext::new(directory.clone(), directory.clone());
	let options = r#"{"replay": "walk.lkxr"}"#;
	let depth = |plugin: &XrPlugin| {
		let capabilities: Capabilities =
			decode(&invoke(plugin, CAPABILITIES, &()).unwrap()).unwrap();
		capabilities
			.features(Mode::PhoneAr)
			.unwrap()
			.contains(Features::DEPTH)
	};

	let before = from_context(&context, options).unwrap();
	assert!(!depth(&before));
	install_depth_estimator(|| Box::new(Blind));
	assert!(depth(&before));
	assert!(depth(&from_context(&context, options).unwrap()));
	let _ = std::fs::remove_dir_all(directory);
}

/// `leptoskit build` refuses an app whose `[native.plugin-options.xr]`
/// leaves out an option this crate's metadata lists in `required-options`.
/// It lists none, so the factory must start from an empty table: an option
/// it needs and the build doesn't check stops the app at launch, as an empty
/// `[native.plugin-options.websocket]` once did on Android.
#[test]
fn the_factory_needs_exactly_the_options_the_build_requires() {
	let manifest: toml::Table = include_str!("../Cargo.toml").parse().unwrap();
	let required = manifest["package"]["metadata"]["leptoskit"]["runtime"]["required-options"]
		.as_array()
		.unwrap();
	assert!(required.is_empty(), "{required:?}");

	let directory = std::env::temp_dir();
	let context = leptoskit_bridge::AppContext::new(directory.clone(), directory);
	assert!(from_context(&context, "{}").is_ok());
}
