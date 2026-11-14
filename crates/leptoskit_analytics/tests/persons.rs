//! Person, group, and session processing at the writer: what a batch of
//! identity entries and pageviews makes of the tables, on SQLite. The
//! Postgres variants run as the ignored tests at the bottom.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::AtomicU8;
use std::sync::atomic::Ordering;

use leptoskit_analytics::service;
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
		ServiceCallers::new(Anonymous).caller("app-token", rpc::server::APP_CALLER),
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

fn entry(name: &str, distinct_id: &str, millis: i64) -> Entry {
	Entry {
		uuid: uuid_at(millis),
		distinct_id: distinct_id.into(),
		session_id: None,
		context: 0,
		body: Body::Event {
			name: name.into(),
			properties: Vec::new(),
		},
	}
}

fn pageview(distinct_id: &str, millis: i64, properties: Vec<(String, Value)>) -> Entry {
	Entry {
		uuid: uuid_at(millis),
		distinct_id: distinct_id.into(),
		session_id: Some([7; 16]),
		context: 0,
		body: Body::Event {
			name: "$pageview".into(),
			properties,
		},
	}
}

fn identify(anon_distinct_id: &str, distinct_id: &str, millis: i64) -> Entry {
	Entry {
		uuid: uuid_at(millis),
		distinct_id: distinct_id.into(),
		session_id: None,
		context: 0,
		body: Body::Identify {
			anon_distinct_id: anon_distinct_id.into(),
			set: Vec::new(),
			set_once: Vec::new(),
		},
	}
}

fn set(
	distinct_id: &str,
	millis: i64,
	set: Vec<(String, Value)>,
	set_once: Vec<(String, Value)>,
	unset: Vec<String>,
) -> Entry {
	Entry {
		uuid: uuid_at(millis),
		distinct_id: distinct_id.into(),
		session_id: None,
		context: 0,
		body: Body::Set {
			set,
			set_once,
			unset,
		},
	}
}

fn alias(alias: &str, distinct_id: &str, millis: i64) -> Entry {
	Entry {
		uuid: uuid_at(millis),
		distinct_id: distinct_id.into(),
		session_id: None,
		context: 0,
		body: Body::Alias {
			alias: alias.into(),
		},
	}
}

fn group_identify(group_type: &str, key: &str, millis: i64) -> Entry {
	Entry {
		uuid: uuid_at(millis),
		distinct_id: "visitor-1".into(),
		session_id: None,
		context: 0,
		body: Body::GroupIdentify {
			group_type: group_type.into(),
			group_key: key.into(),
			set: vec![("name".into(), Value::Text("Acme".into()))],
		},
	}
}

async fn send(service: &Service, body: Vec<u8>) -> service::CaptureResult {
	let response = reqwest::Client::new()
		.post(format!("http://{}{PREFIX}/v1/batch", service.address))
		.header("content-type", wire::CONTENT_TYPE)
		.body(body)
		.send()
		.await
		.unwrap();
	assert_eq!(
		response.status().as_u16(),
		200,
		"{}",
		response.text().await.unwrap()
	);
	serde_json::from_str(&response.text().await.unwrap()).unwrap()
}

async fn rows(db: &Db, sql: &str) -> Vec<Vec<leptoskit_db::Value>> {
	db.fetch(Statement {
		sql: sql.into(),
		args: Vec::new(),
	})
	.await
	.unwrap()
}

async fn text_rows(db: &Db, sql: &str) -> Vec<Vec<String>> {
	rows(db, sql)
		.await
		.iter()
		.map(|row| {
			row.iter()
				.map(|value| {
					match value {
						leptoskit_db::Value::Text(text) => text.clone(),
						leptoskit_db::Value::Int(int) => int.to_string(),
						leptoskit_db::Value::Bool(bool) => bool.to_string(),
						leptoskit_db::Value::Null => "NULL".to_owned(),
						other => format!("{other:?}"),
					}
				})
				.collect()
		})
		.collect()
}

async fn one(db: &Db, sql: &str) -> Option<String> {
	let rows = rows(db, sql).await;
	Some(match rows.into_iter().next()?.into_iter().next()? {
		leptoskit_db::Value::Text(text) => text,
		leptoskit_db::Value::Int(int) => int.to_string(),
		leptoskit_db::Value::Bool(bool) => bool.to_string(),
		leptoskit_db::Value::Null => "NULL".to_owned(),
		other => format!("{other:?}"),
	})
}

fn batch(sent_at: i64, entries: Vec<Entry>) -> Batch {
	Batch {
		sent_at,
		contexts: vec![context()],
		entries,
	}
}

fn context() -> wire::Properties {
	vec![
		("$lib".into(), Value::Text("leptoskit-web".into())),
		(
			"$raw_user_agent".into(),
			Value::Text(
				"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like \
				 Gecko) Chrome/131.0.0.0 Safari/537.36"
					.into(),
			),
		),
	]
}

#[tokio::test(flavor = "multi_thread")]
async fn anonymous_events_get_no_person_until_the_client_identifies() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	let anon = "anon-1";
	let result = send(
		&service,
		wire::encode(&batch(now, vec![entry("opened", anon, now)])),
	)
	.await;
	assert_eq!(result.stored, 1);

	// `identified_only` leaves anonymous events without a person.
	assert_eq!(
		rows(&service.db, "SELECT id FROM persons").await,
		[] as [std::vec::Vec<leptoskit_db::Value>; 0]
	);
	assert_eq!(
		one(
			&service.db,
			"SELECT person_id FROM events WHERE name = 'opened'"
		)
		.await,
		Some("NULL".into()),
		"no person, still counted by distinct id"
	);

	// Identifying creates the person for the identified id and moves the
	// anonymous id onto it.
	send(
		&service,
		wire::encode(&batch(now, vec![identify(anon, "user-1", now + 1_000)])),
	)
	.await;
	let persons = text_rows(
		&service.db,
		"SELECT id, is_identified, version FROM persons",
	)
	.await;
	assert_eq!(persons.len(), 1);
	assert_eq!(persons[0][1], "1", "identified by $identify");
	assert_eq!(persons[0][2], "1");
	let mappings = text_rows(
		&service.db,
		"SELECT distinct_id FROM person_distinct_ids ORDER BY distinct_id",
	)
	.await;
	assert_eq!(mappings, vec![vec!["anon-1"], vec!["user-1"]]);

	let batch = batch(now, vec![entry("bought", "user-1", now + 2_000)]);
	send(&service, wire::encode(&batch)).await;
	// The identified event carries the person; the anonymous one stays
	// attached to nothing until a merge moves it.
	let stored = text_rows(
		&service.db,
		"SELECT name, person_id IS NOT NULL FROM events ORDER BY name",
	)
	.await;
	assert_eq!(
		stored,
		vec![
			vec!["$identify", "1"],
			vec!["bought", "1"],
			vec!["opened", "0"],
		],
		"the identified event carries its person"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn identify_merges_the_anonymous_person_once_and_records_overrides() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	// The anonymous person exists, with a property and two ids.
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![
				set(
					"anon-1",
					now,
					vec![("email".into(), Value::Text("anon@example.com".into()))],
					Vec::new(),
					Vec::new(),
				),
				alias("anon-2", "anon-1", now + 100),
			],
		)),
	)
	.await;
	assert_eq!(
		one(&service.db, "SELECT is_identified FROM persons").await,
		Some("0".into())
	);

	// Identifying moves every distinct id, records an override for each,
	// merges the properties, and removes the anonymous person.
	send(
		&service,
		wire::encode(&batch(now, vec![identify("anon-1", "user-1", now + 1_000)])),
	)
	.await;
	let persons = text_rows(
		&service.db,
		"SELECT id, properties, is_identified FROM persons",
	)
	.await;
	assert_eq!(
		persons.len(),
		1,
		"the anonymous person is gone, one survivor"
	);
	assert_eq!(persons[0][1], r#"{"email":"anon@example.com"}"#);
	assert_eq!(persons[0][2], "1", "the survivor is identified");

	let overrides = text_rows(
		&service.db,
		"SELECT distinct_id FROM person_overrides ORDER BY distinct_id",
	)
	.await;
	assert_eq!(overrides, vec![vec!["anon-1"], vec!["anon-2"]]);
	let mappings = text_rows(
		&service.db,
		"SELECT distinct_id FROM person_distinct_ids ORDER BY distinct_id",
	)
	.await;
	assert_eq!(
		mappings,
		vec![vec!["anon-1"], vec!["anon-2"], vec!["user-1"]]
	);

	// The identified person keeps the id events point at; the overrides
	// say the old ids belong to it now.
	let survivor = one(&service.db, "SELECT id FROM persons").await.unwrap();
	send(
		&service,
		wire::encode(&batch(now, vec![entry("bought", "anon-1", now + 2_000)])),
	)
	.await;
	assert_eq!(
		one(
			&service.db,
			"SELECT person_id FROM events WHERE name = 'bought'"
		)
		.await,
		Some(survivor),
		"a later event under the anonymous id attaches to the survivor"
	);

	// A second `$identify` for a pair already merged does nothing twice.
	send(
		&service,
		wire::encode(&batch(now, vec![identify("anon-1", "user-1", now + 3_000)])),
	)
	.await;
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM persons").await,
		Some("1".into())
	);
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM ingestion_warnings").await,
		Some("0".into())
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn merging_two_identified_persons_is_refused_and_counted() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![
				identify("a-1", "user-1", now),
				identify("b-1", "user-2", now + 100),
			],
		)),
	)
	.await;
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM persons").await,
		Some("2".into())
	);

	// `user-2` is identified; identifying it into `user-1` refuses.
	send(
		&service,
		wire::encode(&batch(now, vec![identify("user-2", "user-1", now + 200)])),
	)
	.await;
	let warnings = text_rows(
		&service.db,
		"SELECT reason, count, detail FROM ingestion_warnings",
	)
	.await;
	assert_eq!(warnings.len(), 1);
	assert_eq!(warnings[0][0], "merge_refused");
	assert_eq!(warnings[0][1], "1");
	assert!(warnings[0][2].contains("user-2"));
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM persons").await,
		Some("2".into()),
		"both persons survive a refused merge"
	);
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM person_overrides").await,
		Some("0".into())
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn alias_links_an_id_and_never_merges_two_identified_persons() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![
				entry("opened", "user-1", now),
				alias("user-1-old", "user-1", now + 100),
			],
		)),
	)
	.await;
	let mappings = text_rows(
		&service.db,
		"SELECT distinct_id FROM person_distinct_ids ORDER BY distinct_id",
	)
	.await;
	assert_eq!(mappings, vec![vec!["user-1"], vec!["user-1-old"]]);
	// An alias from the app isn't the person signing in: still anonymous.
	assert_eq!(
		one(&service.db, "SELECT is_identified FROM persons").await,
		Some("0".into())
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn set_wins_by_event_time_not_arrival() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	// The phone was offline: the older batch arrives after the newer one.
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![set(
				"user-1",
				now + 10_000,
				vec![
					("plan".into(), Value::Text("pro".into())),
					("seats".into(), Value::Int(5)),
				],
				Vec::new(),
				Vec::new(),
			)],
		)),
	)
	.await;
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![set(
				"user-1",
				now + 1_000,
				vec![
					("plan".into(), Value::Text("free".into())),
					("email".into(), Value::Text("ada@example.com".into())),
				],
				vec![("first_seen".into(), Value::Text("monday".into()))],
				vec!["seats".to_owned()],
			)],
		)),
	)
	.await;
	let properties = one(&service.db, "SELECT properties FROM persons")
		.await
		.unwrap();
	let properties: serde_json::Value = serde_json::from_str(&properties).unwrap();
	assert_eq!(
		properties["plan"], "pro",
		"the older batch didn't overwrite"
	);
	assert_eq!(
		properties["email"], "ada@example.com",
		"the missing one is set"
	);
	assert_eq!(
		properties["seats"], 5,
		"an unset older than the write removes nothing"
	);
	assert_eq!(properties["first_seen"], "monday");
	let updated = one(&service.db, "SELECT properties_updated_at FROM persons")
		.await
		.unwrap();
	let updated: serde_json::Value = serde_json::from_str(&updated).unwrap();
	assert!(updated["email"].is_string(), "every write is remembered");
}

#[tokio::test(flavor = "multi_thread")]
async fn placeholders_never_create_or_merge_a_person() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![
				entry("opened", "anonymous", now),
				set(
					"guest",
					now,
					vec![("email".into(), Value::Text("x@example.com".into()))],
					Vec::new(),
					Vec::new(),
				),
				identify("null", "user-1", now + 100),
				identify("user-1", "0", now + 200),
			],
		)),
	)
	.await;
	let persons = text_rows(&service.db, "SELECT is_identified FROM persons").await;
	assert_eq!(persons.len(), 1, "only user-1's person exists");
	assert_eq!(persons[0][0], "1");
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM person_overrides").await,
		Some("0".into()),
		"a placeholder is never merged"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retried_batch_processes_persons_and_sessions_once() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	let batch = batch(
		now,
		vec![
			pageview(
				"anon-1",
				now,
				vec![
					(
						"$current_url".into(),
						Value::Text("https://app.example/pricing".into()),
					),
					("$pathname".into(), Value::Text("/pricing".into())),
				],
			),
			set(
				"anon-1",
				now,
				vec![("email".into(), Value::Text("a@example.com".into()))],
				Vec::new(),
				Vec::new(),
			),
		],
	);
	let body = wire::encode(&batch);
	send(&service, body.clone()).await;
	send(&service, body).await;

	assert_eq!(
		one(&service.db, "SELECT count(*) FROM persons").await,
		Some("1".into())
	);
	assert_eq!(
		one(&service.db, "SELECT event_count FROM sessions").await,
		Some("1".into()),
		"the retry added no events to the session"
	);
	assert_eq!(
		one(&service.db, "SELECT version FROM persons").await,
		Some("1".into()),
		"the duplicate changed nothing"
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_row_keeps_the_visit() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![pageview(
				"anon-1",
				now,
				vec![
					(
						"$current_url".into(),
						Value::Text(
							"https://app.example/pricing?utm_source=google&utm_medium=organic"
								.into(),
						),
					),
					("$pathname".into(), Value::Text("/pricing".into())),
					(
						"$referrer".into(),
						Value::Text("https://www.google.com/".into()),
					),
					("utm_source".into(), Value::Text("google".into())),
					("utm_medium".into(), Value::Text("organic".into())),
					("utm_campaign".into(), Value::Text("spring".into())),
				],
			)],
		)),
	)
	.await;
	let sessions = text_rows(
		&service.db,
		"SELECT event_count, pageview_count, is_bounce, entry_url, entry_pathname, exit_url, \
		 channel, referrer_domain, utm_source, utm_campaign, browser, os, device_type, \
		 duration_ms FROM sessions",
	)
	.await;
	assert_eq!(sessions.len(), 1);
	assert_eq!(
		sessions[0],
		vec![
			"1",
			"1",
			"1",
			"https://app.example/pricing?utm_source=google&utm_medium=organic",
			"/pricing",
			"https://app.example/pricing?utm_source=google&utm_medium=organic",
			"Organic Search",
			"google.com",
			"google",
			"spring",
			"Chrome",
			"Mac OS X",
			"Desktop",
			"0",
		]
	);

	// A later pageview extends the session: the entry stays, the exit
	// moves, the counts add, and it's no longer a bounce.
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![pageview(
				"anon-1",
				now + 65_000,
				vec![
					(
						"$current_url".into(),
						Value::Text("https://app.example/signup".into()),
					),
					("$pathname".into(), Value::Text("/signup".into())),
				],
			)],
		)),
	)
	.await;
	let sessions = text_rows(
		&service.db,
		"SELECT event_count, pageview_count, is_bounce, entry_pathname, exit_pathname, \
		 duration_ms FROM sessions",
	)
	.await;
	assert_eq!(sessions[0][..5], ["2", "2", "0", "/pricing", "/signup"]);
	let duration: i64 = sessions[0][5].parse().unwrap();
	assert!(
		(65_000..66_000).contains(&duration),
		"the duration spans the visit: {duration}"
	);

	// A session without a person keeps person_id null until person
	// processing gives one.
	assert_eq!(
		one(&service.db, "SELECT distinct_id FROM sessions").await,
		Some("anon-1".into())
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_carries_its_person_from_the_events() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	send(
		&service,
		wire::encode(&batch(now, vec![identify("anon-1", "user-1", now)])),
	)
	.await;
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![pageview(
				"user-1",
				now + 1_000,
				vec![(
					"$current_url".into(),
					Value::Text("https://app.example/".into()),
				)],
			)],
		)),
	)
	.await;
	let person = one(&service.db, "SELECT id FROM persons").await.unwrap();
	assert_eq!(
		one(&service.db, "SELECT person_id FROM sessions").await,
		Some(person)
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn groupidentify_registers_types_and_upserts_groups() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![
				group_identify("company", "acme", now),
				group_identify("company", "acme", now + 100),
				group_identify("team", "core", now + 200),
			],
		)),
	)
	.await;
	let types = text_rows(
		&service.db,
		"SELECT name, \"index\" FROM group_types ORDER BY name",
	)
	.await;
	assert_eq!(types, vec![vec!["company", "0"], vec!["team", "1"]]);
	let groups = text_rows(
		&service.db,
		"SELECT type_index, key, properties FROM groups ORDER BY key",
	)
	.await;
	assert_eq!(groups.len(), 2, "the same group twice is one row");
	assert_eq!(groups[0][1], "acme");
	assert_eq!(groups[0][2], r#"{"name":"Acme"}"#);

	// A sixth kind is refused, with a warning.
	for name in ["instance", "workspace", "region"] {
		send(
			&service,
			wire::encode(&batch(now, vec![group_identify(name, "x", now)])),
		)
		.await;
	}
	send(
		&service,
		wire::encode(&batch(now, vec![group_identify("universe", "x", now)])),
	)
	.await;
	let types = one(&service.db, "SELECT count(*) FROM group_types").await;
	assert_eq!(types, Some("5".into()));
	let warnings = text_rows(&service.db, "SELECT reason, count FROM ingestion_warnings").await;
	assert_eq!(warnings, vec![vec!["too_many_group_types", "1"]]);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_event_gets_a_person_when_the_setting_is_all() {
	let service = start(IngestConfig {
		person_profiles: service::persons::PersonProfiles::All,
		..IngestConfig::default()
	})
	.await;
	let now = now_millis();
	send(
		&service,
		wire::encode(&batch(now, vec![entry("opened", "anon-1", now)])),
	)
	.await;
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM persons").await,
		Some("1".into())
	);
	let person = one(&service.db, "SELECT id FROM persons").await.unwrap();
	assert_eq!(
		one(&service.db, "SELECT person_id FROM events").await,
		Some(person)
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_setting_of_never_writes_no_person_at_all() {
	let service = start(IngestConfig {
		person_profiles: service::persons::PersonProfiles::Never,
		..IngestConfig::default()
	})
	.await;
	let now = now_millis();
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![
				identify("anon-1", "user-1", now),
				entry("opened", "user-1", now),
			],
		)),
	)
	.await;
	assert_eq!(
		rows(&service.db, "SELECT id FROM persons").await,
		[] as [std::vec::Vec<leptoskit_db::Value>; 0]
	);
	assert_eq!(
		one(
			&service.db,
			"SELECT person_id FROM events WHERE name = 'opened'"
		)
		.await,
		Some("NULL".into())
	);
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires LEPTOSKIT_ANALYTICS_TEST_POSTGRES pointing to a private test database"]
async fn postgres_processes_persons_and_sessions_like_sqlite() {
	let url = std::env::var("LEPTOSKIT_ANALYTICS_TEST_POSTGRES").expect("private Postgres URL");
	let db = Db::connect(&url).await.unwrap();
	db.migrate(service::POSTGRES_MIGRATIONS).await.unwrap();
	let router = service::routes_with(
		db.clone(),
		None,
		IngestConfig::default(),
		ServiceCallers::new(Anonymous).caller("app-token", rpc::server::APP_CALLER),
	);
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let service = Service { address, db };
	let now = now_millis();

	send(
		&service,
		wire::encode(&batch(
			now,
			vec![
				set(
					"anon-1",
					now,
					vec![("email".into(), Value::Text("anon@example.com".into()))],
					Vec::new(),
					Vec::new(),
				),
				identify("anon-1", "user-1", now + 1_000),
				pageview(
					"user-1",
					now + 2_000,
					vec![
						(
							"$current_url".into(),
							Value::Text("https://app.example/".into()),
						),
						(
							"$referrer".into(),
							Value::Text("https://www.google.com/".into()),
						),
					],
				),
			],
		)),
	)
	.await;

	assert_eq!(
		one(&service.db, "SELECT count(*) FROM persons").await,
		Some("1".into())
	);
	assert_eq!(
		one(&service.db, "SELECT is_identified FROM persons").await,
		Some("true".into())
	);
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM person_overrides").await,
		Some("1".into())
	);
	assert_eq!(
		one(&service.db, "SELECT properties FROM persons").await,
		Some(r#"{"email":"anon@example.com"}"#.into())
	);
	let sessions = text_rows(
		&service.db,
		"SELECT channel, referrer_domain, pageview_count FROM sessions",
	)
	.await;
	assert_eq!(sessions, vec![vec!["Organic Search", "google.com", "1"]]);
	assert_eq!(
		one(
			&service.db,
			"SELECT count(*) FROM events WHERE person_id IS NOT NULL"
		)
		.await,
		Some("2".into()),
		"the anonymous event keeps no person; the rest carry one"
	);
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires LEPTOSKIT_ANALYTICS_TEST_POSTGRES pointing to a private test database"]
async fn postgres_refuses_to_merge_two_identified_persons() {
	let url = std::env::var("LEPTOSKIT_ANALYTICS_TEST_POSTGRES").expect("private Postgres URL");
	let db = Db::connect(&url).await.unwrap();
	db.migrate(service::POSTGRES_MIGRATIONS).await.unwrap();
	let router = service::routes_with(
		db.clone(),
		None,
		IngestConfig::default(),
		ServiceCallers::new(Anonymous).caller("app-token", rpc::server::APP_CALLER),
	);
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let service = Service { address, db };
	let now = now_millis();
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![
				identify("a-1", "user-1", now),
				identify("b-1", "user-2", now + 100),
				identify("user-2", "user-1", now + 200),
			],
		)),
	)
	.await;
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM persons").await,
		Some("2".into())
	);
	assert_eq!(
		one(&service.db, "SELECT reason FROM ingestion_warnings").await,
		Some("merge_refused".into())
	);
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires LEPTOSKIT_ANALYTICS_TEST_POSTGRES pointing to a private test database"]
async fn postgres_applies_set_by_event_time() {
	let url = std::env::var("LEPTOSKIT_ANALYTICS_TEST_POSTGRES").expect("private Postgres URL");
	let db = Db::connect(&url).await.unwrap();
	db.migrate(service::POSTGRES_MIGRATIONS).await.unwrap();
	let router = service::routes_with(
		db.clone(),
		None,
		IngestConfig::default(),
		ServiceCallers::new(Anonymous).caller("app-token", rpc::server::APP_CALLER),
	);
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let service = Service { address, db };
	let now = now_millis();
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![set(
				"user-1",
				now + 10_000,
				vec![("plan".into(), Value::Text("pro".into()))],
				Vec::new(),
				Vec::new(),
			)],
		)),
	)
	.await;
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![set(
				"user-1",
				now + 1_000,
				vec![("plan".into(), Value::Text("free".into()))],
				Vec::new(),
				Vec::new(),
			)],
		)),
	)
	.await;
	assert_eq!(
		one(&service.db, "SELECT properties FROM persons").await,
		Some(r#"{"plan":"pro"}"#.into())
	);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_json_batch_identifies_the_same_way() {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	let events = vec![
		service::JsonEvent {
			event: "opened".into(),
			distinct_id: "anon-1".into(),
			uuid: Some(uuid::Uuid::from_bytes(uuid_at(now))),
			timestamp: None,
			session_id: None,
			properties: [("email".into(), serde_json::json!("ada@example.com"))]
				.into_iter()
				.collect(),
		},
		service::JsonEvent {
			event: "$identify".into(),
			distinct_id: "user-1".into(),
			uuid: Some(uuid::Uuid::from_bytes(uuid_at(now + 1_000))),
			timestamp: None,
			session_id: None,
			properties: serde_json::json!({
				"$anon_distinct_id": "anon-1",
				"$set": { "email": "ada@example.com" },
			})
			.as_object()
			.unwrap()
			.clone(),
		},
	];
	let response = reqwest::Client::new()
		.post(format!("http://{}{PREFIX}/api/capture", service.address))
		.json(&serde_json::json!({ "batch": events }))
		.send()
		.await
		.unwrap();
	let answer = response.text().await.unwrap();
	let result: service::CaptureResult = serde_json::from_str(&answer).unwrap();
	assert_eq!(result.stored, 2);
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM persons").await,
		Some("1".into())
	);
	assert_eq!(
		one(&service.db, "SELECT properties FROM persons").await,
		Some(r#"{"email":"ada@example.com"}"#.into())
	);
	let mappings = text_rows(
		&service.db,
		"SELECT distinct_id FROM person_distinct_ids ORDER BY distinct_id",
	)
	.await;
	assert_eq!(mappings, vec![vec!["anon-1"], vec!["user-1"]]);
}
