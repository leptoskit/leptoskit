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
		leptoskit_db::Value::Uuid(uuid) => {
			leptoskit_analytics::client::format_uuid(&uuid.into_bytes())
		}
		leptoskit_db::Value::Bytes(bytes) => {
			leptoskit_analytics::client::format_uuid(&bytes.try_into().expect("16 bytes"))
		}
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
	// Ids unique to this test: the ignored Postgres tests run in parallel
	// against one database.
	let (anon, user) = ("pg-anon-1", "pg-user-1");

	send(
		&service,
		wire::encode(&batch(
			now,
			vec![
				set(
					anon,
					now,
					vec![("email".into(), Value::Text("anon@example.com".into()))],
					Vec::new(),
					Vec::new(),
				),
				identify(anon, user, now + 1_000),
				pageview(
					user,
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

	let mine = "SELECT count(*) FROM persons WHERE id IN (SELECT person_id FROM \
	            person_distinct_ids WHERE distinct_id IN ('pg-anon-1', 'pg-user-1'))";
	assert_eq!(one(&service.db, mine).await, Some("1".into()));
	let identified = "SELECT is_identified FROM persons WHERE id IN (SELECT person_id FROM \
	                  person_distinct_ids WHERE distinct_id = 'pg-user-1')";
	assert_eq!(one(&service.db, identified).await, Some("true".into()));
	// One batch: the anonymous id's mapping moves to the survivor before
	// the write-back, so its events carry the survivor's person directly
	// and no override is needed. Overrides are for events an earlier batch
	// wrote with the absorbed person.
	assert_eq!(
		one(
			&service.db,
			"SELECT count(*) FROM person_overrides WHERE distinct_id IN ('pg-anon-1', 'pg-user-1')"
		)
		.await,
		Some("0".into())
	);
	assert_eq!(
		one(
			&service.db,
			"SELECT count(*) FROM person_distinct_ids WHERE distinct_id IN ('pg-anon-1', \
			 'pg-user-1')"
		)
		.await,
		Some("2".into())
	);
	let properties = "SELECT properties FROM persons WHERE id IN (SELECT person_id FROM \
	                  person_distinct_ids WHERE distinct_id = 'pg-user-1')";
	assert_eq!(
		one(&service.db, properties).await,
		Some(r#"{"email":"anon@example.com"}"#.into())
	);
	let sessions = text_rows(
		&service.db,
		"SELECT channel, referrer_domain, pageview_count FROM sessions WHERE distinct_id = \
		 'pg-user-1'",
	)
	.await;
	assert_eq!(sessions, vec![vec!["Organic Search", "google.com", "1"]]);
	assert_eq!(
		one(
			&service.db,
			"SELECT count(*) FROM events WHERE distinct_id IN ('pg-anon-1', 'pg-user-1') AND \
			 person_id IS NOT NULL"
		)
		.await,
		Some("3".into()),
		"every event of the batch carries the survivor's person"
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
	// Ids unique to this test: the ignored Postgres tests run in parallel
	// against one database.
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![
				identify("pg-merge-a", "pg-merge-1", now),
				identify("pg-merge-b", "pg-merge-2", now + 100),
				identify("pg-merge-2", "pg-merge-1", now + 200),
			],
		)),
	)
	.await;
	assert_eq!(
		one(
			&service.db,
			"SELECT count(*) FROM persons WHERE id IN (SELECT person_id FROM person_distinct_ids \
			 WHERE distinct_id IN ('pg-merge-a', 'pg-merge-1', 'pg-merge-b', 'pg-merge-2'))"
		)
		.await,
		Some("2".into())
	);
	assert_eq!(
		one(
			&service.db,
			"SELECT reason FROM ingestion_warnings WHERE reason = 'merge_refused'"
		)
		.await,
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
	// An id unique to this test: the ignored Postgres tests run in
	// parallel against one database.
	let user = "pg-setter";
	send(
		&service,
		wire::encode(&batch(
			now,
			vec![set(
				user,
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
				user,
				now + 1_000,
				vec![("plan".into(), Value::Text("free".into()))],
				Vec::new(),
				Vec::new(),
			)],
		)),
	)
	.await;
	let properties = "SELECT properties FROM persons WHERE id IN (SELECT person_id FROM \
	                  person_distinct_ids WHERE distinct_id = 'pg-setter')";
	assert_eq!(
		one(&service.db, properties).await,
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

/// Calls an app-only operation with the service's token.
async fn call(
	service: &Service,
	operation: &str,
	body: serde_json::Value,
	token: Option<&str>,
) -> (u16, serde_json::Value) {
	let mut request = reqwest::Client::new()
		.post(format!(
			"http://{}{PREFIX}/api/{operation}",
			service.address
		))
		.json(&body);
	if let Some(token) = token {
		request = request.bearer_auth(token);
	}
	let response = request.send().await.unwrap();
	let status = response.status().as_u16();
	let text = response.text().await.unwrap();
	let answer: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
	(status, answer)
}

/// One person, identified, with an event under their anonymous id and a
/// session, as the read operations' fixture.
async fn a_person_with_a_session() -> (Service, String) {
	let service = start(IngestConfig::default()).await;
	let now = now_millis();
	send(
		&service,
		wire::encode(&batch(
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
					vec![("email".into(), Value::Text("ada@example.com".into()))],
					Vec::new(),
					Vec::new(),
				),
				identify("anon-1", "user-1", now + 1_000),
				entry("bought", "user-1", now + 2_000),
			],
		)),
	)
	.await;
	let id = one(&service.db, "SELECT id FROM persons").await.unwrap();
	(service, id)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_app_reads_persons_their_events_and_their_sessions() {
	let (service, id) = a_person_with_a_session().await;

	// Anonymous calls are refused, as register_events is.
	let (status, _) = call(
		&service,
		"persons",
		serde_json::json!({ "limit": 10 }),
		None,
	)
	.await;
	assert_eq!(status, 403);

	let (status, answer) = call(
		&service,
		"persons",
		serde_json::json!({ "limit": 10 }),
		Some("app-token"),
	)
	.await;
	assert_eq!(status, 200);
	assert_eq!(answer.as_array().unwrap().len(), 1);
	assert_eq!(answer[0]["properties"]["email"], "ada@example.com");
	assert_eq!(answer[0]["is_identified"], true);

	// A property filter and a search both find them; a wrong one doesn't.
	let (status, answer) = call(
		&service,
		"persons",
		serde_json::json!({ "property": "email", "value": "ada@example.com", "limit": 10 }),
		Some("app-token"),
	)
	.await;
	assert_eq!((status, answer.as_array().unwrap().len()), (200, 1));
	let (_, answer) = call(
		&service,
		"persons",
		serde_json::json!({ "property": "email", "value": "nobody@example.com", "limit": 10 }),
		Some("app-token"),
	)
	.await;
	assert_eq!(answer.as_array().unwrap().len(), 0);
	let (_, answer) = call(
		&service,
		"persons",
		serde_json::json!({ "search": "user-1", "limit": 10 }),
		Some("app-token"),
	)
	.await;
	assert_eq!(
		answer.as_array().unwrap().len(),
		1,
		"the distinct id matches"
	);

	// The person's view carries both distinct ids.
	let (status, answer) = call(
		&service,
		"person",
		serde_json::json!({ "id": id }),
		Some("app-token"),
	)
	.await;
	assert_eq!(status, 200, "{answer}");
	assert_eq!(
		answer["distinct_ids"],
		serde_json::json!(["anon-1", "user-1"])
	);

	// The events include the anonymous one, found through its distinct id.
	let (_, answer) = call(
		&service,
		"person_events",
		serde_json::json!({ "id": id, "limit": 10 }),
		Some("app-token"),
	)
	.await;
	let mut events: Vec<&str> = answer
		.as_array()
		.unwrap()
		.iter()
		.map(|event| event["name"].as_str().unwrap())
		.collect();
	events.sort_unstable();
	assert_eq!(
		events,
		vec!["$identify", "$pageview", "$set", "bought"],
		"newest first where the times differ; ties in any order"
	);

	// The session, with the person attached.
	let (_, answer) = call(
		&service,
		"person_sessions",
		serde_json::json!({ "id": id, "limit": 10 }),
		Some("app-token"),
	)
	.await;
	assert_eq!(answer.as_array().unwrap().len(), 1);
	assert_eq!(answer[0]["entry_pathname"], "/pricing");
}

#[tokio::test(flavor = "multi_thread")]
async fn export_person_answers_everything_stored_about_them() {
	let (service, id) = a_person_with_a_session().await;
	let (_, answer) = call(
		&service,
		"export_person",
		serde_json::json!({ "id": id }),
		Some("app-token"),
	)
	.await;
	assert_eq!(
		answer["person"]["person"]["id"],
		serde_json::json!(id),
		"the document exists"
	);
	assert_eq!(
		answer["person"]["distinct_ids"],
		serde_json::json!(["anon-1", "user-1"])
	);
	assert_eq!(
		answer["events"].as_array().unwrap().len(),
		4,
		"the anonymous events are in the export"
	);
	assert_eq!(answer["sessions"].as_array().unwrap().len(), 1);

	// An unknown person exports nothing.
	let (status, _) = call(
		&service,
		"export_person",
		serde_json::json!({ "id": uuid::Uuid::new_v4() }),
		Some("app-token"),
	)
	.await;
	assert_eq!(status, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_person_removes_everything_stored_about_them() {
	let (service, id) = a_person_with_a_session().await;
	let (status, answer) = call(
		&service,
		"delete_person",
		serde_json::json!({ "id": id }),
		Some("app-token"),
	)
	.await;
	assert_eq!(status, 200);
	assert_eq!(answer["events"], 4);
	assert_eq!(answer["sessions"], 1);

	assert_eq!(
		one(&service.db, "SELECT count(*) FROM persons").await,
		Some("0".into())
	);
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM person_distinct_ids").await,
		Some("0".into())
	);
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM person_overrides").await,
		Some("0".into())
	);
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM events").await,
		Some("0".into())
	);
	assert_eq!(
		one(&service.db, "SELECT count(*) FROM sessions").await,
		Some("0".into())
	);
}
