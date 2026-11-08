//! The native host's hub and plugin: ops through the plugin, assets through
//! the loader thread, events through the long-polling call. Drawing into a
//! native surface needs a window and is checked by the platform hosts.
#![cfg(all(feature = "host", not(target_arch = "wasm32")))]
#![recursion_limit = "256"]

mod support;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use leptoskit_3d::host::*;
use leptoskit_3d::*;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::async_host::AsyncPlugin;
use leptoskit_bridge::async_host::CancellationToken;
use support::fixture;

/// A hub of its own, serving the rig at `models/rig.glb`, and how many
/// times the source was read.
fn hub() -> (&'static Hub, Arc<AtomicUsize>) {
	let hub: &'static Hub = Box::leak(Box::new(Hub::new()));
	let reads = Arc::new(AtomicUsize::new(0));
	let counter = reads.clone();
	let rig = fixture::rig();
	hub.set_asset_source(move |path: &str| {
		counter.fetch_add(1, Ordering::SeqCst);
		match path {
			"models/rig.glb" => Ok(rig.clone()),
			"models/broken.glb" => Ok(b"not a model".to_vec()),
			_ => Err(format!("{path} does not exist")),
		}
	});
	(hub, reads)
}

/// A plugin for `hub` that lives as long as the calls it starts.
fn plugin(hub: &'static Hub) -> &'static Scene3dPlugin {
	Box::leak(Box::new(Scene3dPlugin::with_hub(hub)))
}

fn payload(scene: u32, ops: &[Op]) -> Bytes {
	let mut bytes = scene.to_le_bytes().to_vec();
	bytes.extend_from_slice(&encode_ops(ops));
	Bytes::from(bytes)
}

/// Waits (up to five seconds) for the scene's next events through the
/// plugin's long-polling call.
fn next_events(plugin: &'static Scene3dPlugin, scene: u32) -> Vec<Event> {
	let call = plugin.invoke(
		NEXT_EVENTS,
		Bytes::from(scene.to_le_bytes().to_vec()),
		CancellationToken::new(),
	);
	let (sender, receiver) = std::sync::mpsc::channel();
	std::thread::spawn(move || {
		let _ = sender.send(pollster::block_on(call));
	});
	let reply = receiver
		.recv_timeout(Duration::from_secs(5))
		.expect("events arrived")
		.expect("the call succeeded");
	decode_events(reply.bytes()).unwrap()
}

fn load_rig(scene: u32) -> Vec<Op> {
	let _ = scene;
	vec![
		Op::LoadAsset {
			asset: AssetId(1),
			path: "models/rig.glb".into(),
		},
		Op::Spawn {
			node: NodeId(1),
			asset: Some(AssetId(1)),
			parent: None,
		},
	]
}

#[test]
fn ops_through_the_plugin_load_assets_off_the_calling_thread() {
	let (hub, _) = hub();
	let plugin = plugin(hub);
	assert_eq!(plugin.name(), PLUGIN);
	let reply = pollster::block_on(plugin.invoke(
		APPLY,
		payload(7, &load_rig(7)),
		CancellationToken::new(),
	));
	assert!(reply.unwrap().bytes().is_empty());
	assert_eq!(
		next_events(plugin, 7),
		[Event::AssetLoaded {
			asset: AssetId(1),
			clips: vec!["raise".into(), "bob".into(), "swell".into(), "nod".into()],
		}]
	);
	// The loaded rig answers raycasts.
	let raycast = Op::Raycast {
		request: RequestId(3),
		origin: [0.0, 1.0, 5.0],
		direction: [0.0, 0.0, -1.0],
		max_distance: 10.0,
	};
	pollster::block_on(plugin.invoke(APPLY, payload(7, &[raycast]), CancellationToken::new()))
		.unwrap();

	let events = next_events(plugin, 7);
	let [Event::RaycastHits { request, hits }] = &events[..] else {
		panic!("{events:?}")
	};
	assert_eq!(*request, RequestId(3));
	assert_eq!(hits[0].part, "body");
}

#[test]
fn failures_come_back_as_events() {
	let (hub, _) = hub();
	let plugin = plugin(hub);
	let ops = [
		Op::LoadAsset {
			asset: AssetId(1),
			path: "models/missing.glb".into(),
		},
		Op::LoadAsset {
			asset: AssetId(2),
			path: "models/broken.glb".into(),
		},
	];
	pollster::block_on(plugin.invoke(APPLY, payload(1, &ops), CancellationToken::new())).unwrap();
	let mut events = Vec::new();
	let deadline = Instant::now() + Duration::from_secs(5);

	while events.len() < 2 && Instant::now() < deadline {
		events.extend(next_events(plugin, 1));
	}

	assert!(
		matches!(&events[0], Event::AssetFailed { asset: AssetId(1), message } if message.contains("does not exist")),
		"{events:?}"
	);
	assert!(
		matches!(&events[1], Event::AssetFailed { asset: AssetId(2), message } if message.contains("not valid glTF"))
	);
}

#[test]
fn a_path_shared_by_scenes_is_read_once() {
	let (hub, reads) = hub();
	let plugin = plugin(hub);
	pollster::block_on(plugin.invoke(APPLY, payload(1, &load_rig(1)), CancellationToken::new()))
		.unwrap();
	next_events(plugin, 1);
	pollster::block_on(plugin.invoke(APPLY, payload(2, &load_rig(2)), CancellationToken::new()))
		.unwrap();
	next_events(plugin, 2);
	assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[test]
fn malformed_payloads_and_unknown_methods_are_refused() {
	let (hub, _) = hub();
	let plugin = plugin(hub);
	let call = |method, bytes: &[u8]| {
		pollster::block_on(plugin.invoke(
			method,
			Bytes::from(bytes.to_vec()),
			CancellationToken::new(),
		))
	};
	assert_eq!(
		call(APPLY, &[1, 2]).unwrap_err(),
		leptoskit_bridge::Error::InvalidArgument
	);
	assert_eq!(
		call(APPLY, &[1, 0, 0, 0, 9, 9]).unwrap_err(),
		leptoskit_bridge::Error::InvalidArgument
	);
	assert_eq!(
		call(NEXT_EVENTS, &[]).unwrap_err(),
		leptoskit_bridge::Error::InvalidArgument
	);
	assert_eq!(
		call(leptoskit_bridge::MethodId(99), &[]).unwrap_err(),
		leptoskit_bridge::Error::Unavailable
	);
	assert_eq!(plugin.reply_limit(NEXT_EVENTS), Ok(MAX_EVENTS_REPLY));

	assert!(plugin.reply_limit(leptoskit_bridge::MethodId(99)).is_err());
}

#[test]
fn waiting_for_events_ends_on_cancellation_or_close() {
	let (hub, _) = hub();
	let plugin = plugin(hub);
	pollster::block_on(plugin.invoke(APPLY, payload(4, &[]), CancellationToken::new())).unwrap();
	let token = CancellationToken::new();
	let call = plugin.invoke(
		NEXT_EVENTS,
		Bytes::from(4_u32.to_le_bytes().to_vec()),
		token.clone(),
	);
	let waiting = std::thread::spawn(move || pollster::block_on(call));
	std::thread::sleep(Duration::from_millis(50));
	token.cancel();

	assert_eq!(
		waiting.join().unwrap().unwrap_err(),
		leptoskit_bridge::Error::Cancelled
	);

	let call = plugin.invoke(
		NEXT_EVENTS,
		Bytes::from(4_u32.to_le_bytes().to_vec()),
		CancellationToken::new(),
	);
	let waiting = std::thread::spawn(move || pollster::block_on(call));
	std::thread::sleep(Duration::from_millis(50));
	pollster::block_on(plugin.invoke(
		CLOSE,
		Bytes::from(4_u32.to_le_bytes().to_vec()),
		CancellationToken::new(),
	))
	.unwrap();
	let reply = waiting.join().unwrap().unwrap();
	assert_eq!(decode_events(reply.bytes()).unwrap(), []);
	assert_eq!(hub.scene_count(), 0);
}

#[test]
fn a_process_holds_a_bounded_number_of_scenes() {
	let (hub, _) = hub();

	for scene in 0..MAX_SCENES as u32 {
		hub.apply_ops(SceneId(scene), Vec::new()).unwrap();
	}

	assert!(hub.apply_ops(SceneId(999), Vec::new()).is_err());
	hub.close(SceneId(0));
	assert!(hub.apply_ops(SceneId(999), Vec::new()).is_ok());
}

#[test]
fn without_an_asset_source_loads_fail() {
	let hub: &'static Hub = Box::leak(Box::new(Hub::new()));
	hub.apply_ops(SceneId(1), load_rig(1)).unwrap();
	assert!(matches!(
		&hub.take_events(SceneId(1))[..],
		[Event::AssetFailed { message, .. }] if message.contains("no asset source")
	));
}

#[test]
fn directories_read_only_inside_themselves() {
	let folder = std::env::temp_dir().join(format!("leptoskit_3d_assets_{}", std::process::id()));
	std::fs::create_dir_all(folder.join("models")).unwrap();
	std::fs::write(folder.join("models/rig.glb"), fixture::rig()).unwrap();
	let directory = Directory(folder.clone());
	assert_eq!(directory.read("models/rig.glb").unwrap(), fixture::rig());
	assert!(directory.read("../outside.glb").is_err());
	assert!(directory.read("models/missing.glb").is_err());
	std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn backends_parse_from_configuration_names() {
	assert_eq!(Backend::from_name("gles"), Some(Backend::Gl));
	assert_eq!(Backend::from_name("Vulkan"), Some(Backend::Vulkan));
	assert_eq!(Backend::from_name(""), Some(Backend::Auto));
	assert_eq!(Backend::from_name("glide"), None);
	let hub: &'static Hub = Box::leak(Box::new(Hub::new()));
	assert!(hub.set_backend(Backend::Gl));
}
