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
}

/// Runs the guest once over `files`, answering HTTP with `status`, or with
/// a network failure when it's `None`.
fn run(wasm: &[u8], files: &mut BTreeMap<String, Vec<u8>>, status: Option<u16>) -> Run {
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
						protocol::encode_response(&leptoskit_plugin_http::Response {
							status,
							url: sent.url,
							headers: Vec::new(),
							body: b"{}".to_vec(),
						})
					}
					None => Err(Error::Native),
				}
			} else if request.plugin == filesystem {
				file(files, request.method, &request.payload)
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
		files.contains_key("leptoskit_analytics/queue-0"),
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
		!files.contains_key("leptoskit_analytics/queue-0"),
		"a stored batch is gone from the next save"
	);
}
