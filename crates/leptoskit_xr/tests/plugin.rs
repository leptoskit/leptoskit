//! The `xr` plugin's binary protocol, end to end against a recording.

mod support;

use std::future::Future;
use std::task::Poll;

use leptoskit_bridge::Bytes;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use leptoskit_xr::host::ReplayBackend;
use leptoskit_xr::host::XrPlugin;
use leptoskit_xr::host::XrRuntime;
use leptoskit_xr::host::from_context;
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

	invoke(&plugin, STOP, &()).unwrap();
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
