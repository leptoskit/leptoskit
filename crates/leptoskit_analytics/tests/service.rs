//! The service on a real port: binary and JSON batches, retries stored once,
//! refused entries named, contexts stored once, clock correction, gzip and
//! its limits, and a full queue answering `503`.

use std::io::Write as _;
use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;

use leptoskit_analytics::service;
use leptoskit_analytics::service::CaptureResult;
use leptoskit_analytics::service::IngestConfig;
use leptoskit_analytics::wire;
use leptoskit_analytics::wire::Batch;
use leptoskit_analytics::wire::Body;
use leptoskit_analytics::wire::Entry;
use leptoskit_analytics::wire::Value;
use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_rpc as rpc;
use rpc::server::Anonymous;
use rpc::server::ServiceCallers;
use serde_json::json;

const TOKEN: &str = "app-token";
const PREFIX: &str = "/_leptoskit/services/analytics";

struct Service {
	address: std::net::SocketAddr,
	db: Db,
}

async fn start(config: IngestConfig) -> Service {
	let db = leptoskit_db::testing::sqlite_migrated(service::MIGRATIONS).await;
	let router = service::routes_with(
		db.clone(),
		None,
		config,
		ServiceCallers::new(Anonymous).caller(TOKEN, rpc::server::APP_CALLER),
	);
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	Service { address, db }
}

fn now_millis() -> i64 {
	leptoskit_db::now().timestamp_millis()
}

fn uuid_at(millis: i64) -> [u8; 16] {
	static NEXT: AtomicU8 = AtomicU8::new(1);
	wire::uuid_v7(millis, [NEXT.fetch_add(1, Ordering::Relaxed); 10])
}

fn event(name: &str, millis: i64) -> Entry {
	Entry {
		uuid: uuid_at(millis),
		distinct_id: "visitor-1".into(),
		session_id: Some([9; 16]),
		context: 0,
		body: Body::Event {
			name: name.into(),
			properties: vec![("$pathname".into(), Value::Text("/".into()))],
		},
	}
}

fn context() -> wire::Properties {
	vec![
		("$lib".into(), Value::Text("leptoskit-web".into())),
		("$os".into(), Value::Text("iOS".into())),
	]
}

async fn send(
	service: &Service,
	body: Vec<u8>,
	encoding: Option<&str>,
	token: Option<&str>,
) -> (u16, String, Option<String>) {
	let mut request = reqwest::Client::new()
		.post(format!("http://{}{PREFIX}/v1/batch", service.address))
		.header("content-type", wire::CONTENT_TYPE)
		.body(body);
	if let Some(encoding) = encoding {
		request = request.header("content-encoding", encoding);
	}
	if let Some(token) = token {
		request = request.bearer_auth(token);
	}
	let response = request.send().await.unwrap();
	let retry_after = response
		.headers()
		.get("retry-after")
		.map(|value| value.to_str().unwrap().to_owned());
	(
		response.status().as_u16(),
		response.text().await.unwrap(),
		retry_after,
	)
}

async fn rows(db: &Db, sql: &str) -> Vec<Vec<leptoskit_db::Value>> {
	db.fetch(Statement {
		sql: sql.into(),
		args: Vec::new(),
	})
	.await
	.unwrap()
}

fn text(value: &leptoskit_db::Value) -> String {
	match value {
		leptoskit_db::Value::Text(text) => text.clone(),
		leptoskit_db::Value::Int(int) => int.to_string(),
		other => format!("{other:?}"),
	}
}

#[tokio::test(flavor = "multi_thread")]
async fn a_binary_batch_is_stored_and_a_retry_is_stored_once() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	let batch = Batch {
		sent_at: now,
		contexts: vec![context()],
		entries: vec![
			event("$pageview", now - 2_000),
			event("signed_up", now - 1_000),
		],
	};
	let body = wire::encode(&batch);

	let (status, answer, _) = send(&service, body.clone(), None, None).await;
	assert_eq!(status, 200, "{answer}");
	let result: CaptureResult = serde_json::from_str(&answer).unwrap();
	assert_eq!((result.stored, result.duplicates), (2, 0));

	let (status, answer, _) = send(&service, body, None, None).await;
	assert_eq!(status, 200, "{answer}");
	let result: CaptureResult = serde_json::from_str(&answer).unwrap();
	assert_eq!(
		(result.stored, result.duplicates),
		(0, 2),
		"a retry is recognised"
	);

	let events = rows(
		&service.db,
		"SELECT name, distinct_id, properties, source FROM events ORDER BY name",
	)
	.await;
	let events: Vec<Vec<String>> = events
		.iter()
		.map(|row| row.iter().map(text).collect())
		.collect();
	assert_eq!(
		events,
		vec![
			vec!["$pageview", "visitor-1", r#"{"$pathname":"/"}"#, "1"],
			vec!["signed_up", "visitor-1", r#"{"$pathname":"/"}"#, "1"],
		]
	);

	let contexts = rows(&service.db, "SELECT properties FROM contexts").await;
	assert_eq!(contexts.len(), 1, "both events share one stored context");
	assert_eq!(
		text(&contexts[0][0]),
		r#"{"$lib":"leptoskit-web","$os":"iOS"}"#
	);

	let definitions = rows(
		&service.db,
		"SELECT name, volume FROM event_definitions ORDER BY name",
	)
	.await;
	let definitions: Vec<Vec<String>> = definitions
		.iter()
		.map(|row| row.iter().map(text).collect())
		.collect();
	assert_eq!(
		definitions,
		vec![vec!["$pageview", "1"], vec!["signed_up", "1"]],
		"the retry didn't count twice"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn refused_entries_are_named_and_counted_while_the_rest_are_stored() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	let mut not_v7 = event("bad_uuid", now);
	not_v7.uuid = [0; 16];
	let mut no_name = event("", now);
	no_name.uuid = uuid_at(now);
	let batch = Batch {
		sent_at: now,
		contexts: vec![context()],
		entries: vec![event("fine", now), not_v7, no_name],
	};

	let (status, answer, _) = send(&service, wire::encode(&batch), None, None).await;
	assert_eq!(status, 200, "{answer}");
	let result: CaptureResult = serde_json::from_str(&answer).unwrap();
	assert_eq!(result.stored, 1);
	let refused: Vec<(u32, &str)> = result
		.refused
		.iter()
		.map(|refused| (refused.index, refused.reason.as_str()))
		.collect();
	assert_eq!(refused, vec![(1, "invalid_uuid"), (2, "invalid_name")]);

	let warnings = rows(&service.db, "SELECT reason, count FROM ingestion_warnings").await;
	let warnings: Vec<Vec<String>> = warnings
		.iter()
		.map(|row| row.iter().map(text).collect())
		.collect();
	assert_eq!(warnings, vec![vec!["invalid", "2"]]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fast_clock_is_corrected_and_one_far_ahead_is_clamped() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	let hour = 3_600_000;
	// The client's clock is an hour fast: it recorded the event a minute
	// before sending, by its clock.
	let batch = Batch {
		sent_at: now + hour,
		contexts: vec![],
		entries: vec![
			event("late_clock", now + hour - 60_000),
			event("far_ahead", now + 30 * hour),
		],
	};
	let (status, answer, _) = send(&service, wire::encode(&batch), None, None).await;
	assert_eq!(status, 200, "{answer}");

	let stored = rows(
		&service.db,
		"SELECT name, timestamp, received_at FROM events ORDER BY name",
	)
	.await;
	let at = |value: &leptoskit_db::Value| {
		chrono::DateTime::parse_from_rfc3339(&text(value))
			.unwrap()
			.timestamp_millis()
	};
	let (far, late) = (&stored[0], &stored[1]);
	assert_eq!(text(&late[0]), "late_clock");
	let corrected = at(&late[1]);
	assert!(
		(corrected - (now - 60_000)).abs() < 5_000,
		"recorded about a minute ago, not an hour ahead: {corrected} vs {now}"
	);
	assert_eq!(text(&far[0]), "far_ahead");
	assert_eq!(at(&far[1]), at(&far[2]), "clamped to when it arrived");

	let warnings = rows(&service.db, "SELECT reason, count FROM ingestion_warnings").await;
	assert_eq!(text(&warnings[0][0]), "clock_clamped");
}

#[tokio::test(flavor = "multi_thread")]
async fn gzip_is_accepted_and_bombs_and_garbage_are_refused() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	let batch = Batch {
		sent_at: now,
		contexts: vec![context()],
		entries: vec![event("zipped", now)],
	};
	let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
	encoder.write_all(&wire::encode(&batch)).unwrap();
	let (status, answer, _) = send(&service, encoder.finish().unwrap(), Some("gzip"), None).await;
	assert_eq!(status, 200, "{answer}");

	// 32 MiB of zeros compresses to about 32 KiB.
	let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
	encoder.write_all(&vec![0; 32 << 20]).unwrap();
	let bomb = encoder.finish().unwrap();
	assert!(bomb.len() < 64 << 10);
	let (status, answer, _) = send(&service, bomb, Some("gzip"), None).await;
	assert_eq!(status, 413, "{answer}");

	let (status, answer, _) = send(&service, b"not a batch".to_vec(), None, None).await;
	assert_eq!(status, 400, "{answer}");
	assert!(answer.contains("not a leptoskit event batch"), "{answer}");

	let (status, ..) = send(&service, b"x".to_vec(), Some("br"), None).await;
	assert_eq!(status, 415);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_queue_answers_503_with_retry_after() {
	let service = start(IngestConfig {
		queue_rows: 1,
		..IngestConfig::default()
	})
	.await;
	let now = now_millis();
	let batch = Batch {
		sent_at: now,
		contexts: vec![],
		entries: vec![event("one", now), event("two", now)],
	};
	let (status, answer, retry_after) = send(&service, wire::encode(&batch), None, None).await;
	assert_eq!(status, 503, "{answer}");
	assert_eq!(retry_after.as_deref(), Some("1"));
	assert_eq!(rows(&service.db, "SELECT uuid FROM events").await.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_app_s_batches_are_recorded_as_the_server_s() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	let batch = Batch {
		sent_at: now,
		contexts: vec![],
		entries: vec![event("from_the_server", now)],
	};
	let (status, answer, _) = send(&service, wire::encode(&batch), None, Some(TOKEN)).await;
	assert_eq!(status, 200, "{answer}");
	let stored = rows(&service.db, "SELECT source FROM events").await;
	assert_eq!(text(&stored[0][0]), "2");
}

#[tokio::test(flavor = "multi_thread")]
async fn json_capture_moves_device_properties_into_one_context() {
	let service = start(IngestConfig::default()).await;
	let event = |name: &str| {
		json!({
			"event": name,
			"distinct_id": "visitor-2",
			"properties": { "$os": "Android", "$lib": "curl", "plan": "pro" },
		})
	};
	let response = reqwest::Client::new()
		.post(format!("http://{}{PREFIX}/api/capture", service.address))
		.json(&json!({ "batch": [event("a"), event("b"), { "event": "", "distinct_id": "x" }] }))
		.send()
		.await
		.unwrap();
	assert_eq!(response.status(), 200);
	let result: CaptureResult = response.json().await.unwrap();
	assert_eq!(result.stored, 2);
	assert_eq!(result.refused.len(), 1);
	assert_eq!(result.refused[0].reason, "invalid_name");

	let stored = rows(
		&service.db,
		"SELECT properties, source FROM events ORDER BY name",
	)
	.await;
	assert_eq!(text(&stored[0][0]), r#"{"plan":"pro"}"#);
	assert_eq!(text(&stored[0][1]), "3");
	let contexts = rows(&service.db, "SELECT properties FROM contexts").await;
	assert_eq!(contexts.len(), 1);
	assert_eq!(text(&contexts[0][0]), r#"{"$lib":"curl","$os":"Android"}"#);
}

/// Sustained throughput on SQLite: 16 clients each sending 50 batches of
/// 100 events, every `200` meaning committed. Run with `--ignored` on a
/// release build; the figure goes in the design's status table.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "a benchmark: run it on purpose, with --release"]
async fn throughput() {
	let file = std::env::temp_dir().join(format!("lk-analytics-bench-{}.db", std::process::id()));
	let url = format!("sqlite://{}?mode=rwc", file.display());
	let db = Db::connect(&url).await.unwrap();
	db.migrate(service::MIGRATIONS).await.unwrap();
	let router = service::routes(db.clone(), None, IngestConfig::default());
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let service = Service { address, db };
	let service = std::sync::Arc::new(service);

	let started = std::time::Instant::now();
	let mut clients = Vec::new();
	for client in 0..16_u8 {
		let service = std::sync::Arc::clone(&service);
		clients.push(tokio::spawn(async move {
			for batch in 0..50_u16 {
				let now = now_millis();
				let entries = (0..100_u16)
					.map(|n| {
						let mut random = [client; 10];
						random[1..3].copy_from_slice(&batch.to_be_bytes());
						random[3..5].copy_from_slice(&n.to_be_bytes());
						Entry {
							uuid: wire::uuid_v7(now, random),
							distinct_id: format!("visitor-{client}"),
							session_id: Some([client; 16]),
							context: 0,
							body: Body::Event {
								name: "$pageview".into(),
								properties: vec![
									(
										"$current_url".into(),
										Value::Text(format!("https://app.example/{n}")),
									),
									("$pathname".into(), Value::Text(format!("/{n}"))),
								],
							},
						}
					})
					.collect();
				let body = wire::encode(&Batch {
					sent_at: now,
					contexts: vec![context()],
					entries,
				});
				let (status, answer, _) = send(&service, body, None, None).await;
				assert_eq!(status, 200, "{answer}");
			}
		}));
	}
	for client in clients {
		client.await.unwrap();
	}
	let elapsed = started.elapsed();
	let stored = rows(&service.db, "SELECT count(*) FROM events").await;
	assert_eq!(text(&stored[0][0]), "80000");
	println!(
		"stored 80000 events in {:.2?}: {:.0} events/s",
		elapsed,
		80_000.0 / elapsed.as_secs_f64()
	);
	let _ = std::fs::remove_file(&file);
}
