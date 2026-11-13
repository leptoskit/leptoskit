//! The native analytics client in a compiled guest, across two runs of the
//! app. The test plays the HTTP and filesystem plugins. The first run is
//! offline, so the event is saved in the app's files. The second restores
//! the person and that event, and sends both runs' events in one batch.

use std::collections::BTreeMap;
use std::time::Duration;
use std::time::Instant;

use leptoskit_analytics::wire;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use leptoskit_bridge::codec::Decoder;
use leptoskit_bridge::codec::Encoder;
use leptoskit_native::EventOutcome;
use leptoskit_native::HeadlessBackend;
use leptoskit_plugin_filesystem as fs;
use leptoskit_plugin_http::protocol;
use leptoskit_wasm::EventKind;
use leptoskit_wasm::WasmHost;

const REPORT: &str = "analytics-test-report";

/// What one run did.
#[derive(Default)]
struct Run {
	distinct_id: String,
	/// The bodies of the batches the guest sent.
	sent: Vec<Vec<u8>>,
	failed_writes: usize,
}

/// Runs the guest once over `files`, answering HTTP with `status`, or with
/// a network failure when it's `None`.
fn run(wasm: &[u8], files: &mut BTreeMap<String, Vec<u8>>, status: Option<u16>) -> Run {
	run_with_write_failure(wasm, files, status, None)
}

fn run_with_write_failure(
	wasm: &[u8],
	files: &mut BTreeMap<String, Vec<u8>>,
	status: Option<u16>,
	mut failed_path: Option<&str>,
) -> Run {
	let mut host = WasmHost::<HeadlessBackend>::headless(wasm).expect("compile the guest");
	let report = register(&mut host, REPORT, &[MethodId(0), MethodId(1)]);
	let http = register(&mut host, "http", &[MethodId(0)]);
	let filesystem = register(
		&mut host,
		fs::NAME,
		&[
			fs::READ,
			fs::WRITE,
			fs::LIST,
			fs::MKDIR,
			fs::REMOVE,
			fs::STAT,
			fs::RENAME,
		],
	);
	host.enable_timers().expect("enable timers");
	host.start(100.0, 100.0).expect("start the guest");

	let mut run = Run::default();
	let deadline = Instant::now() + Duration::from_secs(10);
	loop {
		host.pump_timers().expect("pump timers");
		// With timers on, the host moves every other plugin's calls to its
		// routed queue for the platform, which this test plays.
		while let Some(request) = host
			.take_routed_request()
			.or_else(|| host.plugins().next_request())
		{
			let reply = if request.plugin == report {
				assert!(
					request.method != MethodId(1),
					"the guest panicked: {}",
					String::from_utf8_lossy(&request.payload)
				);
				run.distinct_id = String::from_utf8(request.payload.to_vec()).unwrap();
				Ok(Bytes::new())
			} else if request.plugin == http {
				let sent = protocol::decode_request(&request.payload).expect("an HTTP request");
				assert_eq!(
					sent.url,
					"https://app.test/_leptoskit/services/analytics/v1/batch"
				);
				run.sent.push(sent.body);
				match status {
					Some(status) => {
						let batch =
							wire::decode(run.sent.last().unwrap(), &wire::Limits::default())
								.unwrap();
						let refused: Vec<String> = batch
							.entries
							.iter()
							.enumerate()
							.filter(|(_, entry)| {
								matches!(entry.body, wire::Body::TypedEvent { .. })
							})
							.map(|(index, _)| {
								format!(r#"{{"index":{index},"reason":"unknown_schema"}}"#)
							})
							.collect();
						protocol::encode_response(&leptoskit_plugin_http::Response {
							status,
							url: sent.url,
							headers: Vec::new(),
							body: format!(r#"{{"refused":[{}]}}"#, refused.join(",")).into_bytes(),
						})
					}
					None => Err(Error::Native),
				}
			} else if request.plugin == filesystem {
				let path = Decoder::new(&request.payload).string().unwrap().to_owned();
				if request.method == fs::WRITE && failed_path == Some(path.as_str()) {
					failed_path = None;
					run.failed_writes += 1;
					Err(Error::Native)
				} else {
					file(files, request.method, &request.payload)
				}
			} else {
				Err(Error::Unavailable)
			};
			host.plugins()
				.complete(request.id, reply)
				.expect("complete");
		}
		host.wake_plugins().expect("wake the guest");
		let done = host
			.dispatch_wire(0, EventKind::Press, None)
			.expect("poll the guest")
			== EventOutcome::Handled;
		if done {
			return run;
		}
		assert!(Instant::now() < deadline, "the guest didn't finish");
		std::thread::sleep(Duration::from_millis(2));
	}
}

/// The filesystem plugin, over `files`.
fn file(
	files: &mut BTreeMap<String, Vec<u8>>,
	method: MethodId,
	payload: &[u8],
) -> Result<Bytes, Error> {
	let mut decoder = Decoder::new(payload);
	let path = decoder.string()?.to_owned();
	match method {
		fs::READ => {
			files
				.get(&path)
				.map(|bytes| Bytes::copy_from_slice(bytes))
				.ok_or(Error::Native)
		}
		fs::WRITE => {
			files.insert(path, decoder.bytes()?.to_vec());
			Ok(Bytes::new())
		}
		fs::MKDIR => Ok(Bytes::new()),
		fs::LIST => {
			let prefix = format!("{path}/");
			let names: Vec<_> = files
				.keys()
				.filter_map(|path| path.strip_prefix(&prefix))
				.filter(|name| !name.contains('/'))
				.collect();
			let mut encoder = Encoder::with_capacity(4);
			encoder.u32(u32::try_from(names.len()).unwrap());
			for name in names {
				encoder.bytes(name.as_bytes())?;
			}
			Ok(encoder.finish())
		}
		fs::REMOVE => {
			files
				.remove(&path)
				.map(|_| Bytes::new())
				.ok_or(Error::Native)
		}
		_ => Err(Error::Unavailable),
	}
}

fn register(
	host: &mut WasmHost<HeadlessBackend>,
	name: &str,
	methods: &[MethodId],
) -> leptoskit_bridge::PluginId {
	let plugin = host
		.plugins()
		.register(name, methods.iter().copied())
		.expect("register");
	for method in methods {
		host.plugins().grant(plugin, *method, true).expect("grant");
	}
	plugin
}

fn names(body: &[u8]) -> Vec<(String, String)> {
	wire::decode(body, &wire::Limits::default())
		.expect("a batch")
		.entries
		.into_iter()
		.map(|entry| {
			let name = match entry.body {
				wire::Body::Event { name, .. } => name,
				other => format!("{other:?}"),
			};
			(entry.distinct_id, name)
		})
		.collect()
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the Rust wasm fixture"]
fn events_recorded_offline_are_sent_on_the_next_run() {
	let path = std::env::var("LEPTOSKIT_ANALYTICS_GUEST").expect("fixture wasm path");
	let wasm = std::fs::read(path).expect("read fixture");
	let mut files = BTreeMap::new();

	let first = run(&wasm, &mut files, None);
	assert_eq!(first.sent.len(), 1, "it tried to send");
	assert_eq!(
		names(&first.sent[0]),
		[(first.distinct_id.clone(), "opened".to_owned())]
	);
	assert!(files.contains_key("leptoskit_analytics/identity"));
	assert!(
		files.contains_key("leptoskit_analytics/queue-0-0"),
		"the unsent event was saved: {:?}",
		files.keys().collect::<Vec<_>>()
	);

	let second = run(&wasm, &mut files, Some(200));
	assert_eq!(second.distinct_id, first.distinct_id, "the same person");
	assert_eq!(second.sent.len(), 1);
	let sent = names(&second.sent[0]);
	assert_eq!(
		sent,
		[
			(first.distinct_id.clone(), "opened".to_owned()),
			(first.distinct_id.clone(), "opened".to_owned()),
		]
	);
	let batch = wire::decode(&second.sent[0], &wire::Limits::default()).unwrap();
	let restored = wire::decode(&first.sent[0], &wire::Limits::default()).unwrap();
	assert_eq!(
		batch.entries[0].uuid, restored.entries[0].uuid,
		"the same event, so stored once"
	);
	assert_ne!(batch.entries[1].uuid, batch.entries[0].uuid);
	assert!(
		!files.contains_key("leptoskit_analytics/queue-0-0"),
		"a stored batch is gone from the next save"
	);
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the Rust wasm fixture"]
fn a_failed_checkpoint_write_retries_without_recording_another_event() {
	let path = std::env::var("LEPTOSKIT_ANALYTICS_GUEST").expect("fixture wasm path");
	let wasm = std::fs::read(path).expect("read fixture");
	for failed_path in [
		"leptoskit_analytics/queue-1-0",
		"leptoskit_analytics/queue-manifest",
	] {
		let mut files = BTreeMap::new();
		let first = run(&wasm, &mut files, None);
		let retry = run_with_write_failure(&wasm, &mut files, None, Some(failed_path));
		assert_eq!(retry.failed_writes, 1);
		assert_eq!(
			retry.sent.len(),
			1,
			"the second flush respects HTTP backoff"
		);
		let previous = wire::decode(&retry.sent[0], &wire::Limits::default()).unwrap();
		assert_eq!(previous.entries.len(), 2);
		let online = run(&wasm, &mut files, Some(200));
		assert_eq!(online.distinct_id, first.distinct_id);
		let sent = wire::decode(&online.sent[0], &wire::Limits::default()).unwrap();
		assert_eq!(
			sent.entries.len(),
			3,
			"the failed save did not lose either offline run"
		);
		for (sent, saved) in sent.entries.iter().zip(&previous.entries) {
			assert_eq!(sent.uuid, saved.uuid, "retry retains the deduplication key");
		}
		let after_ack = run(&wasm, &mut files, Some(200));
		assert_eq!(
			names(&after_ack.sent[0]).len(),
			1,
			"acknowledged events stay gone"
		);
	}
}

#[test]
#[ignore = "run by test:plugins:guest, which compiles the Rust wasm fixture"]
fn restarting_while_opted_out_cannot_resurrect_older_events_after_opt_in() {
	let path = std::env::var("LEPTOSKIT_ANALYTICS_GUEST").expect("fixture wasm path");
	let wasm = std::fs::read(path).expect("read fixture");
	let mut files = BTreeMap::new();
	run(&wasm, &mut files, None);
	files.insert("leptoskit_analytics/opted_out".into(), b"1".to_vec());
	let opted_out = run(&wasm, &mut files, Some(200));
	assert_eq!(opted_out.sent, Vec::<Vec<u8>>::new());
	files.remove("leptoskit_analytics/opted_out");
	let opted_in = run(&wasm, &mut files, Some(200));
	assert_eq!(
		names(&opted_in.sent[0]).len(),
		1,
		"only the new, consented event is sent"
	);
}

#[test]
#[ignore = "run by test:plugins:guest with the typed Rust wasm fixture"]
fn typed_guest_events_unknown_to_the_service_retry_with_names() {
	let path = std::env::var("LEPTOSKIT_ANALYTICS_TYPED_GUEST").expect("typed fixture wasm path");
	let wasm = std::fs::read(path).expect("read fixture");
	let run = run(&wasm, &mut BTreeMap::new(), Some(200));
	assert_eq!(run.sent.len(), 2);
	let first = wire::decode(&run.sent[0], &wire::Limits::default()).unwrap();
	let again = wire::decode(&run.sent[1], &wire::Limits::default()).unwrap();
	assert!(matches!(
		first.entries[0].body,
		wire::Body::TypedEvent { .. }
	));
	assert_eq!(first.entries[0].uuid, again.entries[0].uuid);
	assert!(
		matches!(&again.entries[0].body, wire::Body::Event { properties, .. }
		if properties.contains(&("screen".into(), wire::Value::Text("home".into()))))
	);
}

#[test]
#[ignore = "run by test:plugins:guest with both Rust wasm fixtures"]
fn typed_offline_events_survive_a_build_without_the_old_event_schema() {
	let typed = std::fs::read(std::env::var("LEPTOSKIT_ANALYTICS_TYPED_GUEST").unwrap()).unwrap();
	let plain = std::fs::read(std::env::var("LEPTOSKIT_ANALYTICS_GUEST").unwrap()).unwrap();
	let mut files = BTreeMap::new();
	let offline = run(&typed, &mut files, None);
	let online = run(&plain, &mut files, Some(200));
	assert_eq!(online.distinct_id, offline.distinct_id);
	assert_eq!(
		online.sent.len(),
		1,
		"the new build needs no old schema metadata"
	);
	let previous = wire::decode(&offline.sent[0], &wire::Limits::default()).unwrap();
	let restored = wire::decode(&online.sent[0], &wire::Limits::default()).unwrap();
	assert_eq!(restored.entries.len(), 2);
	assert_eq!(restored.entries[0].uuid, previous.entries[0].uuid);
	assert!(matches!(restored.entries[0].body, wire::Body::Event { .. }));
}
