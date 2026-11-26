//! Checks of translated variants and local-time delivery that both
//! databases run, against a service with an APNs transport over a boundary
//! that records every push and accepts it.
//!
//! No check here waits for the wall clock to reach a scheduled time. A
//! local-time send is made for a wall-clock time chosen from the present,
//! so that it has already passed in one fixed-offset zone and is hours
//! away in others; and time passes by [`pass_time`], which moves the
//! send's rows, its queued deliveries, and the waiting jobs back by the
//! same amount, exactly as if that long had gone by.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::SecondsFormat;
use leptoskit_db::Db;
use leptoskit_db::Read as _;
use leptoskit_db::Statement;
use leptoskit_notifications::Notification;
use leptoskit_notifications::SERVICE;
use leptoskit_notifications::push::PushDelivery;
use leptoskit_notifications::service::config;
use leptoskit_notifications::service::push;
use leptoskit_notifications::service::transport::Boundary;
use leptoskit_notifications::service::transport::PushRuntime;
use leptoskit_notifications::service::transport::WireError;
use leptoskit_notifications::service::transport::WireRequest;
use leptoskit_notifications::service::transport::WireResponse;
use leptoskit_rpc as rpc;
use leptoskit_rpc::server::Anonymous;
use leptoskit_rpc::server::ServiceCallers;
use leptoskit_rpc::server::jobs::JobRow;
use leptoskit_rpc::server::jobs::JobState;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

use super::engine;
use super::sql::Sql;

const TOKEN: &str = "app-token";
const ORIGIN: &str = "http://app.test";
const PREFIX: &str = "/_leptoskit/services/notifications";

/// How long a fanout, a wake, or a push round may take on a loaded
/// machine.
const SETTLES: Duration = Duration::from_secs(60);

/// A zone fourteen hours ahead of UTC all year: a wall-clock time arrives
/// here first.
const KIRITIMATI: &str = "Pacific/Kiritimati";

/// A zone nine hours ahead of UTC all year.
const TOKYO: &str = "Asia/Tokyo";

/// A zone twelve hours behind UTC all year: a wall-clock time arrives here
/// last.
const BAKER_ISLAND: &str = "Etc/GMT+12";

fn person(n: u128) -> Uuid {
	Uuid::from_u128(0x10CA_0000_0000_0000_0000_0000_0000_0000 | n)
}

/// One push the boundary saw: the device token it went to, and its body.
#[derive(Clone, Debug)]
struct Pushed {
	token: String,
	body: Value,
}

/// The provider boundary: it records each push and accepts it.
#[derive(Default)]
pub struct Recorder {
	pushes: std::sync::Mutex<Vec<Pushed>>,
}

impl Recorder {
	/// The pushes that went to the device enrolled with `seed`.
	fn to(&self, seed: u8) -> Vec<Value> {
		let token = token(seed);
		self.pushes
			.lock()
			.unwrap()
			.iter()
			.filter(|pushed| pushed.token == token)
			.map(|pushed| pushed.body.clone())
			.collect()
	}

	/// Every push, whoever it went to.
	fn count(&self) -> usize {
		self.pushes.lock().unwrap().len()
	}
}

impl Boundary for Recorder {
	fn post(
		&self,
		request: WireRequest,
	) -> Pin<Box<dyn Future<Output = Result<WireResponse, WireError>> + Send + '_>> {
		self.pushes.lock().unwrap().push(Pushed {
			token: request
				.url
				.rsplit('/')
				.next()
				.unwrap_or_default()
				.to_owned(),
			body: serde_json::from_slice(&request.body).unwrap_or(Value::Null),
		});

		Box::pin(async {
			Ok(WireResponse {
				status: 200,
				headers: Vec::new(),
				body: Vec::new(),
			})
		})
	}
}

/// A running service over one database.
pub struct Service {
	pub address: std::net::SocketAddr,
	pub db: Db,
	pub recorder: Arc<Recorder>,
	jobs: Option<rpc::server::JobsHandle>,
}

impl Drop for Service {
	fn drop(&mut self) {
		if let Some(jobs) = self.jobs.take() {
			jobs.stop();
		}
	}
}

/// Serves the notifications routes over `db`, already migrated, with an
/// APNs transport over a [`Recorder`] — and, with `workers`, the service's
/// own workers on a test-fast poll. Without them, a check runs each job
/// itself.
pub async fn serve(db: &Db, workers: bool) -> Service {
	let recorder = Arc::new(Recorder::default());
	let key_p8 = {
		let rng = ring::rand::SystemRandom::new();
		let document = ring::signature::EcdsaKeyPair::generate_pkcs8(
			&ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
			&rng,
		)
		.expect("a key");
		format!(
			"-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
			base64::Engine::encode(
				&base64::engine::general_purpose::STANDARD,
				document.as_ref()
			)
		)
	};
	let config = config::PushConfig {
		apns: Some(config::ApnsConfig {
			key_p8,
			key_id: "KEY".into(),
			team_id: "TEAM".into(),
			topic: "dev.leptoskit.app".into(),
			environment: config::ApnsEnvironment::Sandbox,
		}),
		fcm: None,
		web: None,
	};
	let push = push::Push {
		paused: push::Paused::default(),
		runtime: Some(Arc::new(
			PushRuntime::build_over(&config, recorder.clone()).expect("the runtime builds"),
		)),
	};
	let callers = ServiceCallers::new(Anonymous)
		.caller(TOKEN, rpc::server::APP_CALLER)
		.forwarded_identity(SERVICE, TOKEN);
	let (router, live) = leptoskit_notifications::service::routes_with_push(
		db.clone(),
		Some(ORIGIN),
		callers,
		push.clone(),
	);
	live.start().await.expect("live starts");
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router).await.unwrap();
	});
	let jobs = workers.then(|| {
		leptoskit_notifications::service::workers_with_push(db.clone(), push)
			.poll(Duration::from_millis(50))
			.start()
	});

	Service {
		address,
		db: db.clone(),
		recorder,
		jobs,
	}
}

fn signed_in(person: Uuid) -> String {
	rpc::server::IdentityKey::new(SERVICE, TOKEN)
		.expect("a key")
		.mint(
			person,
			std::time::SystemTime::now(),
			rpc::server::IDENTITY_LIFETIME,
		)
}

/// Calls `operation` as `person`, or as the app with `None`.
async fn call(
	service: &Service,
	operation: &str,
	args: Value,
	person: Option<Uuid>,
) -> (u16, String) {
	let token = person.map_or_else(|| TOKEN.to_owned(), signed_in);
	let response = reqwest::Client::new()
		.post(format!(
			"http://{}{PREFIX}/rpc/{operation}",
			service.address
		))
		.bearer_auth(token)
		.json(&args)
		.timeout(Duration::from_secs(60))
		.send()
		.await
		.unwrap();
	(response.status().as_u16(), response.text().await.unwrap())
}

/// Calls `operation` and answers its JSON, which must be a success.
async fn ok(service: &Service, operation: &str, args: Value, person: Option<Uuid>) -> Value {
	let (status, body) = call(service, operation, args, person).await;
	assert_eq!(status, 200, "{operation}: {body}");
	serde_json::from_str(&body).unwrap()
}

/// The APNs device token enrolled with `seed`.
fn token(seed: u8) -> String {
	format!("{seed:02x}").repeat(32)
}

/// `person` enrolls the device `seed`, reporting `device`.
async fn enroll(service: &Service, person: Uuid, seed: u8, device: Value) {
	ok(
		service,
		"register_push_endpoint",
		json!({ "credential": { "apns": { "device_token": token(seed) } }, "device": device }),
		Some(person),
	)
	.await;
}

/// The app sets `attributes` on `person`.
async fn attributes(service: &Service, person: Uuid, attributes: Value) {
	ok(
		service,
		"set_person_attributes",
		json!({ "persons": [person], "set": attributes, "remove": [] }),
		None,
	)
	.await;
}

/// The titles and bodies in `person`'s inbox, newest first.
async fn inbox(service: &Service, person: Uuid) -> Vec<(String, String)> {
	let page = ok(
		service,
		"inbox",
		json!({ "page": { "limit": 100 }, "show": "all" }),
		Some(person),
	)
	.await;
	page["items"]
		.as_array()
		.unwrap()
		.iter()
		.map(|item| {
			(
				item["title"].as_str().unwrap().to_owned(),
				item["body"].as_str().unwrap().to_owned(),
			)
		})
		.collect()
}

/// The title and body of the one push the device `seed` received.
fn pushed_text(service: &Service, seed: u8) -> (String, String) {
	let pushes = service.recorder.to(seed);
	assert_eq!(pushes.len(), 1, "one push to device {seed}: {pushes:?}");
	let alert = &pushes[0]["aps"]["alert"];
	(
		alert["title"].as_str().unwrap_or_default().to_owned(),
		alert["body"].as_str().unwrap_or_default().to_owned(),
	)
}

/// Waits until `done` answers true.
async fn until<F, Fut>(what: &str, mut done: F)
where
	F: FnMut() -> Fut,
	Fut: Future<Output = bool>,
{
	let deadline = tokio::time::Instant::now() + SETTLES;

	loop {
		if done().await {
			return;
		}

		assert!(tokio::time::Instant::now() < deadline, "never: {what}");
		tokio::time::sleep(Duration::from_millis(50)).await;
	}
}

/// The rows of `send_key`, by person.
async fn rows(db: &Db, send_key: &str) -> std::collections::BTreeMap<Uuid, Notification> {
	Notification::query()
		.filter(|row| row.send_key.eq(send_key.to_owned()))
		.all(&db.system("read a send's rows in a test"))
		.await
		.expect("the rows read")
		.into_iter()
		.map(|row| (row.person, row))
		.collect()
}

/// The deliveries of `send_key`: each one's person, state, reason, and
/// when it next runs.
async fn deliveries(
	db: &Db,
	send_key: &str,
) -> Vec<(
	Uuid,
	String,
	Option<String>,
	Option<leptoskit_db::Timestamp>,
)> {
	PushDelivery::query()
		.filter(|row| row.send_key.eq(send_key.to_owned()))
		.order_by(|row| row.person.asc())
		.all(&db.system("read a send's deliveries in a test"))
		.await
		.expect("the deliveries read")
		.into_iter()
		.map(|row| (row.person, row.state, row.reason, row.next_attempt_at))
		.collect()
}

/// The jobs called `name` still waiting to run.
async fn pending(db: &Db, name: &str) -> Vec<JobRow> {
	JobRow::query()
		.filter(|job| job.name.eq(name.to_owned()) & job.state.eq(JobState::Pending))
		.all(&db.system("read the pending jobs in a test"))
		.await
		.expect("the jobs read")
}

/// Lets `by` pass for `send_key`: its rows become visible that much
/// sooner, its queued deliveries are due that much sooner, and every
/// waiting job runs that much sooner — what the same wait on the wall
/// clock would have come to.
async fn pass_time(db: &Db, send_key: &str, by: chrono::Duration) {
	let system = db.system("pass time in a test");

	for row in rows(db, send_key).await.into_values() {
		Notification::update()
			.filter(|notification| notification.id.eq(row.id))
			.set(|notification| notification.visible_at, row.visible_at - by)
			.run(&system)
			.await
			.expect("the row moves");
	}

	for delivery in PushDelivery::query()
		.filter(|row| row.send_key.eq(send_key.to_owned()) & row.next_attempt_at.is_not_null())
		.all(&system)
		.await
		.expect("the deliveries read")
	{
		PushDelivery::update()
			.filter(|row| row.id.eq(delivery.id))
			.set(
				|row| row.next_attempt_at,
				delivery.next_attempt_at.map(|at| at - by),
			)
			.run(&system)
			.await
			.expect("the delivery moves");
	}

	for job in JobRow::query()
		.filter(|job| job.state.eq(JobState::Pending))
		.all(&system)
		.await
		.expect("the jobs read")
	{
		// A wake for one instant names it, so the instant moves with the
		// rows it is for.
		let mut arguments = job.arguments.0.clone();

		if let Some(at) = arguments
			.get("at")
			.and_then(Value::as_str)
			.and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
		{
			arguments["at"] = json!(at.with_timezone(&chrono::Utc) - by);
		}

		// The jobs table is the framework's own, and spelled differently
		// on each database.
		let (jobs, first, second, third) = match db.dialect() {
			leptoskit_db::Dialect::Postgres => ("\"rpc\".\"jobs\"", "$1", "$2", "$3"),
			leptoskit_db::Dialect::Sqlite => ("\"rpc_jobs\"", "?1", "?2", "?3"),
		};
		db.fetch(Statement {
			sql: format!(
				"UPDATE {jobs} SET \"run_at\" = {first}, \"arguments\" = {second} WHERE \"id\" = \
				 {third}"
			),
			args: vec![
				leptoskit_db::Value::Timestamp(job.run_at - by),
				leptoskit_db::Value::Json(arguments),
				leptoskit_db::Value::Uuid(job.id.uuid()),
			],
		})
		.await
		.expect("the job moves");
	}
}

/// A wall-clock time `ahead` of the present as UTC reads it, to the
/// second: the `at` of a local-time send, and the same time as an instant
/// in UTC.
fn wall_clock(ahead: chrono::Duration) -> (String, leptoskit_db::Timestamp) {
	let at = chrono::DateTime::from_timestamp((chrono::Utc::now() + ahead).timestamp(), 0)
		.expect("a whole second");

	(at.format("%Y-%m-%dT%H:%M:%S").to_string(), at)
}

/// Whether the person has been woken: their wake row exists.
async fn woken(db: &Db, person: Uuid) -> bool {
	engine::count(
		db,
		&format!(
			"SELECT count(*) FROM wakes WHERE person = {}",
			Sql(db.dialect()).uuid(person)
		),
	)
	.await
		> 0
}

fn spanish_and_french() -> Value {
	json!({
		"category": "news",
		"title": "Good morning",
		"body": "Your summary is ready.",
		"variants": {
			"es": { "title": "Buenos días", "body": "Tu resumen está listo." },
			"fr": { "title": "Bonjour", "body": "Votre résumé est prêt." },
			"pt-BR": { "title": "Bom dia" },
			"pt": { "title": "Bom dia (PT)", "body": "O seu resumo está pronto." },
		},
	})
}

/// Each recipient gets one inbox row in their own language, and the push
/// for it carries the same text. The app's `locale` attribute wins over
/// what a device says, a device's locale is used when the app set none,
/// the more recently seen device is the one that counts, a region falls
/// back to its language, and a person nobody knows a language for — or
/// whose language has no variant — reads the send's own text.
#[allow(
	clippy::cognitive_complexity,
	reason = "six people, each a different way a language is known"
)]
pub async fn each_person_reads_their_own_language(service: &Service) {
	// The app says Brazilian Portuguese; the device says French.
	let ana = person(1);
	attributes(service, ana, json!({ "locale": "pt-BR" })).await;
	enroll(service, ana, 1, json!({ "locale": "fr" })).await;
	// Only the device says: Canadian French reads the French variant.
	let bea = person(2);
	enroll(service, bea, 2, json!({ "locale": "fr-CA" })).await;
	// Two devices: the one seen most recently says Mexican Spanish.
	let cai = person(3);
	enroll(service, cai, 3, json!({ "locale": "de" })).await;
	tokio::time::sleep(Duration::from_millis(20)).await;
	enroll(service, cai, 4, json!({ "locale": "es-MX" })).await;
	// Nobody knows.
	let dee = person(4);
	enroll(service, dee, 5, json!({})).await;
	// The app says Japanese, which the send has no variant for: the
	// attribute still wins over the device's French.
	let eli = person(5);
	attributes(service, eli, json!({ "locale": "ja" })).await;
	enroll(service, eli, 6, json!({ "locale": "fr" })).await;
	// An attribute that isn't a language tag falls to the device.
	let fay = person(6);
	attributes(service, fay, json!({ "locale": "not a tag" })).await;
	enroll(service, fay, 7, json!({ "locale": "pt-PT" })).await;

	let everyone = [ana, bea, cai, dee, eli, fay];
	let sent = ok(
		service,
		"send",
		json!({
			"idempotency_key": "morning",
			"spec": spanish_and_french(),
			"target": { "recipients": everyone },
		}),
		None,
	)
	.await;
	assert_eq!(sent["delivered"], 6, "{sent}");

	let stored = rows(&service.db, "morning").await;
	assert_eq!(stored.len(), 6, "one row per person");

	let english = (
		"Good morning".to_owned(),
		"Your summary is ready.".to_owned(),
	);
	let french = ("Bonjour".to_owned(), "Votre résumé est prêt.".to_owned());
	let expected = [
		// A variant that leaves the body out keeps the send's own.
		(ana, 1, ("Bom dia".to_owned(), english.1.clone())),
		(bea, 2, french),
		(
			cai,
			4,
			(
				"Buenos días".to_owned(),
				"Tu resumen está listo.".to_owned(),
			),
		),
		(dee, 5, english.clone()),
		(eli, 6, english),
		(
			fay,
			7,
			(
				"Bom dia (PT)".to_owned(),
				"O seu resumo está pronto.".to_owned(),
			),
		),
	];

	for (who, _, text) in &expected {
		assert_eq!(
			inbox(service, *who).await,
			vec![text.clone()],
			"the inbox of {who}"
		);
	}

	until("every device is pushed", || {
		async { service.recorder.count() >= 7 }
	})
	.await;

	for (who, seed, text) in &expected {
		assert_eq!(&pushed_text(service, *seed), text, "the push to {who}");
	}

	// Cai's older device gets the same row's text, not its own language.
	assert_eq!(pushed_text(service, 3).0, "Buenos días");

	// The same send again answers the first; a changed translation is a
	// different send under the same key.
	let again = ok(
		service,
		"send",
		json!({
			"idempotency_key": "morning",
			"spec": spanish_and_french(),
			"target": { "recipients": everyone },
		}),
		None,
	)
	.await;
	assert_eq!(again["duplicate"], true, "{again}");
	let mut changed = spanish_and_french();
	changed["variants"]["fr"]["title"] = json!("Salut");
	let (status, body) = call(
		service,
		"send",
		json!({
			"idempotency_key": "morning",
			"spec": changed,
			"target": { "recipients": everyone },
		}),
		None,
	)
	.await;
	assert_eq!(status, 409, "variants are part of what a send is: {body}");
	assert_eq!(rows(&service.db, "morning").await.len(), 6);
}

/// "Most recently seen" follows what a device last told the service, not
/// when it first enrolled: an older device that re-enrolls with a new
/// language becomes the one a send reads.
pub async fn the_most_recently_seen_device_decides(service: &Service) {
	let gus = person(7);
	enroll(service, gus, 8, json!({ "locale": "es" })).await;
	tokio::time::sleep(Duration::from_millis(20)).await;
	enroll(service, gus, 9, json!({ "locale": "fr" })).await;

	let send = |key: &'static str| {
		ok(
			service,
			"send",
			json!({
				"idempotency_key": key,
				"spec": spanish_and_french(),
				"target": { "recipients": [gus] },
			}),
			None,
		)
	};

	send("seen-1").await;
	assert_eq!(
		rows(&service.db, "seen-1").await[&gus].title,
		"Bonjour",
		"the device enrolled last was seen last"
	);

	// The first device's language changes, and it says so.
	tokio::time::sleep(Duration::from_millis(20)).await;
	enroll(service, gus, 8, json!({ "locale": "pt" })).await;
	send("seen-2").await;
	assert_eq!(
		rows(&service.db, "seen-2").await[&gus].title,
		"Bom dia (PT)",
		"the device heard from last decides, however long ago it enrolled"
	);
}

/// A topic send resolves each batch's languages the same way, a test send
/// reads the variant the person would get, and a dry run counts the
/// audience without writing.
pub async fn variants_reach_a_topic_and_a_test_send(service: &Service) {
	let spanish: Vec<Uuid> = (100..103).map(person).collect();
	let unknown: Vec<Uuid> = (200..203).map(person).collect();

	for (index, who) in spanish.iter().enumerate() {
		attributes(service, *who, json!({ "locale": "es-AR" })).await;
		enroll(
			service,
			*who,
			u8::try_from(20 + index).unwrap(),
			json!({ "locale": "fr" }),
		)
		.await;
	}

	let all: Vec<Uuid> = spanish.iter().chain(&unknown).copied().collect();
	ok(
		service,
		"subscribe_to_topic",
		json!({ "topic": "news.daily", "persons": all }),
		None,
	)
	.await;

	let dry = ok(
		service,
		"send",
		json!({
			"idempotency_key": "daily",
			"spec": spanish_and_french(),
			"target": { "topic": "news.daily" },
			"validate_only": true,
		}),
		None,
	)
	.await;
	assert_eq!(dry["targeted"], 6, "{dry}");
	assert!(
		rows(&service.db, "daily").await.is_empty(),
		"nothing written"
	);

	ok(
		service,
		"send",
		json!({
			"idempotency_key": "daily",
			"spec": spanish_and_french(),
			"target": { "topic": "news.daily" },
		}),
		None,
	)
	.await;
	until("the fanout completes", || {
		async {
			let status = ok(
				service,
				"send_status",
				json!({ "idempotency_key": "daily" }),
				None,
			)
			.await;
			status["state"] == "complete"
		}
	})
	.await;

	let stored = rows(&service.db, "daily").await;
	assert_eq!(stored.len(), 6);

	for who in &spanish {
		assert_eq!(stored[who].title, "Buenos días", "{who}");
		assert_eq!(stored[who].body, "Tu resumen está listo.", "{who}");
	}

	for who in &unknown {
		assert_eq!(stored[who].title, "Good morning", "{who}");
	}

	until("the three devices are pushed", || {
		async { service.recorder.count() >= 3 }
	})
	.await;
	assert_eq!(pushed_text(service, 20).0, "Buenos días");

	// A test send to one person carries the text they would read.
	let before = service.recorder.count();
	let outcomes = ok(
		service,
		"send_test",
		json!({ "spec": spanish_and_french(), "person": spanish[1] }),
		None,
	)
	.await;
	assert_eq!(outcomes[0]["state"], "accepted", "{outcomes}");
	assert_eq!(service.recorder.count(), before + 1);
	let probes = service.recorder.to(21);
	assert_eq!(
		probes.last().unwrap()["aps"]["alert"]["title"],
		"Buenos días",
		"{probes:?}"
	);
}

/// Bad variants and a bad local time are refused before anything is
/// written, by name — as a send and as a dry run.
pub async fn bad_variants_and_local_times_are_refused(service: &Service) {
	let refused = |spec: Value, validate_only: bool| {
		async move {
			let (status, body) = call(
				service,
				"send",
				json!({
					"idempotency_key": "refused",
					"spec": spec,
					"target": { "recipients": [person(1)] },
					"validate_only": validate_only,
				}),
				None,
			)
			.await;
			assert_eq!(status, 400, "{body}");
			body
		}
	};
	let base = || json!({ "category": "news", "title": "Hello" });
	let with = |key: &str, value: Value| {
		let mut spec = base();
		spec[key] = value;
		spec
	};
	let (soon, _) = wall_clock(chrono::Duration::hours(1));

	for validate_only in [false, true] {
		let body = refused(
			with("variants", json!({ "not a tag": { "title": "x" } })),
			validate_only,
		)
		.await;
		assert!(body.contains("not a tag"), "{body}");

		let body = refused(with("variants", json!({ "fr": {} })), validate_only).await;
		assert!(body.contains("variants.fr"), "{body}");

		let body = refused(
			with("variants", json!({ "fr": { "title": "x".repeat(201) } })),
			validate_only,
		)
		.await;
		assert!(body.contains("variants.fr.title"), "{body}");

		let body = refused(
			with("local_time", json!({ "at": "tomorrow at nine" })),
			validate_only,
		)
		.await;
		assert!(body.contains("local_time.at"), "{body}");

		let body = refused(
			with(
				"local_time",
				json!({ "at": soon, "default_time_zone": "Mars/Olympus_Mons" }),
			),
			validate_only,
		)
		.await;
		assert!(body.contains("Mars/Olympus_Mons"), "{body}");

		// An expiry an hour after the wall-clock time, read as UTC, comes
		// before that time has reached the zones behind UTC.
		let (at, utc) = wall_clock(chrono::Duration::hours(2));
		let mut spec = with("local_time", json!({ "at": at }));
		spec["expires_at"] =
			json!((utc + chrono::Duration::hours(1)).to_rfc3339_opts(SecondsFormat::Secs, true));
		let body = refused(spec, validate_only).await;
		assert!(body.contains("arrived everywhere"), "{body}");

		let mut spec = with("local_time", json!({ "at": soon }));
		spec["visible_at"] = json!(chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true));
		let body = refused(spec, validate_only).await;
		assert!(body.contains("not both"), "{body}");

		// The last zone's turn is what the year's limit is measured to.
		let (far, _) = wall_clock(chrono::Duration::days(366) - chrono::Duration::hours(11));
		let body = refused(with("local_time", json!({ "at": far })), validate_only).await;
		assert!(body.contains("last time zone"), "{body}");
	}

	assert!(rows(&service.db, "refused").await.is_empty());
	assert_eq!(
		engine::count(
			&service.db,
			"SELECT count(*) FROM sends WHERE idempotency_key = 'refused'"
		)
		.await,
		0,
		"nothing was recorded"
	);
}

/// A local-time send, followed from the moment it is made to the last
/// zone's turn.
///
/// The wall-clock time is ten hours from now as UTC reads it. It passed
/// four hours ago in Kiritimati, so that person's row is visible and
/// pushed at once; it is an hour away in Tokyo and twenty-two hours away
/// on Baker Island, so those rows wait — hidden from the inbox, their
/// pushes queued for their own instants — with one wake per distinct
/// instant, not one per person. An hour and a bit later Tokyo's turn
/// comes; a cancel then stops the row that is still ahead and leaves the
/// two people have already seen.
#[allow(
	clippy::too_many_lines,
	clippy::cognitive_complexity,
	reason = "one send followed through three time zones, a step at a time"
)]
pub async fn each_person_gets_it_at_their_own_nine_o_clock(service: &Service) {
	let db = &service.db;
	// The app's attribute names the zone, whatever the device says.
	let kiri = person(11);
	attributes(service, kiri, json!({ "time_zone": KIRITIMATI })).await;
	enroll(service, kiri, 11, json!({ "time_zone": BAKER_ISLAND })).await;
	// Two people in Tokyo, by their devices: the instant is shared.
	let tomo = person(12);
	enroll(service, tomo, 12, json!({ "time_zone": TOKYO })).await;
	let taro = person(13);
	enroll(service, taro, 13, json!({ "time_zone": TOKYO })).await;
	// Nobody knows where: the send's default zone.
	let bo = person(14);
	enroll(service, bo, 14, json!({})).await;
	// A zone the service's database doesn't have falls to the device, and
	// one the device reports that it doesn't have falls to the default.
	let odd = person(15);
	attributes(service, odd, json!({ "time_zone": "Mars/Olympus_Mons" })).await;
	enroll(service, odd, 15, json!({ "time_zone": TOKYO })).await;
	let lost = person(16);
	enroll(
		service,
		lost,
		16,
		json!({ "time_zone": "Atlantis/Lost_City" }),
	)
	.await;

	let everyone = [kiri, tomo, taro, bo, odd, lost];
	let (at, utc) = wall_clock(chrono::Duration::hours(10));
	let tokyo_turn = utc - chrono::Duration::hours(9);
	let baker_turn = utc + chrono::Duration::hours(12);
	let spec = json!({
		"category": "news",
		"title": "Nine o'clock",
		"local_time": { "at": at, "default_time_zone": BAKER_ISLAND },
	});
	let before = chrono::Utc::now();
	let sent = ok(
		service,
		"send",
		json!({
			"idempotency_key": "nine",
			"spec": spec,
			"target": { "recipients": everyone },
		}),
		None,
	)
	.await;
	let after = chrono::Utc::now();
	assert_eq!(sent["delivered"], 6, "one row per person: {sent}");
	assert_eq!(sent["state"], "complete");

	let stored = rows(db, "nine").await;
	assert!(
		stored[&kiri].visible_at >= before - chrono::Duration::seconds(1)
			&& stored[&kiri].visible_at <= after,
		"a local time already past is now: {}",
		stored[&kiri].visible_at
	);

	for who in [tomo, taro, odd] {
		assert_eq!(stored[&who].visible_at, tokyo_turn, "{who} is in Tokyo");
	}

	for who in [bo, lost] {
		assert_eq!(
			stored[&who].visible_at, baker_turn,
			"{who} is in the default zone"
		);
	}

	// Kiritimati sees it and is pushed; nobody else does or is.
	assert_eq!(inbox(service, kiri).await.len(), 1);
	until("Kiritimati's push", || {
		async { service.recorder.to(11).len() == 1 }
	})
	.await;
	until("Kiritimati's wake", || woken(db, kiri)).await;

	for who in [tomo, taro, bo, odd, lost] {
		assert!(
			inbox(service, who).await.is_empty(),
			"{who} sees nothing yet"
		);
		assert!(!woken(db, who).await, "{who} isn't woken yet");
	}

	// The waiting deliveries are queued for their own instants, so a round
	// that runs before then reads past them without an attempt.
	until("the first round is over", || {
		async {
			deliveries(db, "nine")
				.await
				.iter()
				.any(|(who, state, ..)| *who == kiri && state == "accepted")
		}
	})
	.await;

	for (who, state, _, next) in deliveries(db, "nine").await {
		if who == kiri {
			continue;
		}

		assert_eq!(state, "enqueued", "{who}");
		assert_eq!(
			next,
			Some(stored[&who].visible_at),
			"{who} waits for their time"
		);
	}

	assert_eq!(service.recorder.count(), 1, "one push so far");

	// One wake per distinct instant still ahead: Tokyo's and Baker
	// Island's, however many people share each.
	let wakes = pending(db, "wake_local").await;
	let mut wake_times: Vec<_> = wakes.iter().map(|job| job.run_at).collect();
	wake_times.sort();
	assert_eq!(wake_times, vec![tokyo_turn, baker_turn], "{wakes:?}");

	// The status counts every row the send wrote, visible or not.
	let status = ok(
		service,
		"send_status",
		json!({ "idempotency_key": "nine" }),
		None,
	)
	.await;
	assert_eq!(status["delivered"], 6, "{status}");

	// An hour and a minute on, it is nine in Tokyo.
	pass_time(db, "nine", chrono::Duration::minutes(61)).await;

	for seed in [12, 13, 15] {
		until("Tokyo's pushes", || {
			async { service.recorder.to(seed).len() == 1 }
		})
		.await;
	}

	for who in [tomo, taro, odd] {
		until("Tokyo's wakes", || woken(db, who)).await;
		assert_eq!(inbox(service, who).await.len(), 1, "{who} sees it now");
	}

	for who in [bo, lost] {
		assert!(inbox(service, who).await.is_empty(), "{who} still waits");
		assert!(!woken(db, who).await, "{who} still isn't woken");
	}

	assert_eq!(service.recorder.count(), 4, "four pushes so far");

	// The app cancels: what is still ahead stops, what people have seen
	// stays.
	let cancelled = ok(
		service,
		"cancel",
		json!({ "idempotency_key": "nine" }),
		None,
	)
	.await;
	assert_eq!(cancelled, 2, "the two rows still ahead");
	let stored = rows(db, "nine").await;

	for who in [kiri, tomo, taro, odd] {
		assert!(stored[&who].cancelled_at.is_none(), "{who} keeps theirs");
		assert_eq!(inbox(service, who).await.len(), 1);
	}

	for who in [bo, lost] {
		assert!(stored[&who].cancelled_at.is_some(), "{who}'s is stopped");
	}

	// Asked again, there is nothing left to stop, and that is an answer.
	let again = ok(
		service,
		"cancel",
		json!({ "idempotency_key": "nine" }),
		None,
	)
	.await;
	assert_eq!(again, 0);

	// A day on, Baker Island's turn has come and gone: nothing shows, and
	// nothing was pushed.
	pass_time(db, "nine", chrono::Duration::hours(24)).await;
	until("the stopped deliveries end", || {
		async {
			deliveries(db, "nine")
				.await
				.iter()
				.filter(|(who, ..)| *who == bo || *who == lost)
				.all(|(_, state, reason, _)| {
					state == "skipped" && reason.as_deref() == Some("cancelled")
				})
		}
	})
	.await;

	for who in [bo, lost] {
		assert!(inbox(service, who).await.is_empty(), "{who} never sees it");
	}

	assert_eq!(service.recorder.count(), 4, "nothing more was pushed");
}

/// A send that is visible everywhere can no longer be cancelled, local
/// time or not; and one cancelled before any zone's turn stops whole.
pub async fn a_local_time_send_cancels_like_any_other_at_its_edges(service: &Service) {
	let db = &service.db;
	let kiri = person(21);
	attributes(service, kiri, json!({ "time_zone": KIRITIMATI })).await;
	let tomo = person(22);
	attributes(service, tomo, json!({ "time_zone": TOKYO })).await;

	// Twenty hours ahead in UTC: six hours ahead in Kiritimati, eleven in
	// Tokyo. Nobody has seen it.
	let (at, _) = wall_clock(chrono::Duration::hours(20));
	ok(
		service,
		"send",
		json!({
			"idempotency_key": "ahead",
			"spec": { "category": "news", "title": "Later", "local_time": { "at": at } },
			"target": { "recipients": [kiri, tomo] },
		}),
		None,
	)
	.await;
	assert_eq!(
		ok(
			service,
			"cancel",
			json!({ "idempotency_key": "ahead" }),
			None
		)
		.await,
		2
	);
	assert!(
		rows(db, "ahead")
			.await
			.values()
			.all(|row| row.cancelled_at.is_some())
	);

	// Already past in both zones: both rows are visible, and the send can
	// no longer be cancelled.
	let (at, _) = wall_clock(chrono::Duration::hours(-20));
	ok(
		service,
		"send",
		json!({
			"idempotency_key": "past",
			"spec": { "category": "news", "title": "Earlier", "local_time": { "at": at } },
			"target": { "recipients": [kiri, tomo] },
		}),
		None,
	)
	.await;
	assert_eq!(inbox(service, kiri).await.len(), 1);
	assert_eq!(inbox(service, tomo).await.len(), 1);
	let (status, body) = call(
		service,
		"cancel",
		json!({ "idempotency_key": "past" }),
		None,
	)
	.await;
	assert_eq!(status, 409, "{body}");
}

/// A topic send's fanout resolves each batch's zones with the batch, and
/// enqueues one wake per batch and distinct instant — bounded by the zones
/// in a batch, never by its people. Run with no workers: the check runs
/// each fanout step itself and counts what it left behind.
#[allow(
	clippy::cognitive_complexity,
	reason = "one fanout followed batch by batch, then instant by instant"
)]
pub async fn a_fanout_wakes_once_per_batch_and_instant(service: &Service) {
	let db = &service.db;
	let audience = engine::people(0x7E, 700);
	engine::subscribe(service.address, "news.local", &audience).await;

	// A third of the audience each in Kiritimati and Tokyo, set a zone at
	// a time; the rest say nothing.
	let zone_of = |index: usize| {
		match index % 3 {
			0 => Some(KIRITIMATI),
			1 => Some(TOKYO),
			_ => None,
		}
	};

	for zone in [KIRITIMATI, TOKYO] {
		let there: Vec<Uuid> = audience
			.iter()
			.enumerate()
			.filter(|(index, _)| zone_of(*index) == Some(zone))
			.map(|(_, who)| *who)
			.collect();
		ok(
			service,
			"set_person_attributes",
			json!({ "persons": there, "set": { "time_zone": zone }, "remove": [] }),
			None,
		)
		.await;
	}

	let (at, utc) = wall_clock(chrono::Duration::hours(10));
	let tokyo_turn = utc - chrono::Duration::hours(9);
	let default_turn = utc;
	let sent = ok(
		service,
		"send",
		json!({
			"idempotency_key": "local-topic",
			"spec": { "category": "news", "title": "Nine o'clock", "local_time": { "at": at } },
			"target": { "topic": "news.local" },
		}),
		None,
	)
	.await;
	assert_eq!(sent["state"], "fanning_out", "{sent}");

	engine::run_step(db, "local-topic", 0).await;
	engine::run_step(db, "local-topic", 1).await;

	let progress = engine::progress(service.address, "local-topic").await;
	assert_eq!(progress.delivered, 700, "{progress:?}");

	let stored = rows(db, "local-topic").await;
	let mut by_instant: std::collections::BTreeMap<leptoskit_db::Timestamp, usize> =
		std::collections::BTreeMap::new();
	let mut immediate = 0;

	for (index, who) in audience.iter().enumerate() {
		let visible_at = stored[who].visible_at;

		match zone_of(index) {
			Some(KIRITIMATI) => {
				assert!(visible_at <= chrono::Utc::now(), "{who} is past due");
				immediate += 1;
			}
			Some(_) => assert_eq!(visible_at, tokyo_turn, "{who}"),
			None => assert_eq!(visible_at, default_turn, "{who} is in UTC, the default"),
		}

		if visible_at > chrono::Utc::now() {
			*by_instant.entry(visible_at).or_default() += 1;
		}
	}

	assert_eq!(immediate, 234);
	assert_eq!(by_instant.len(), 2, "two instants are still ahead");

	// Two batches, two instants ahead in each: four wakes for 466 people.
	let wakes = pending(db, "wake_local").await;
	assert_eq!(wakes.len(), 4, "{wakes:?}");
	assert!(
		pending(db, "wake_batch").await.is_empty(),
		"the one-instant batch wake isn't used"
	);

	// The people whose time had passed were woken by their own batch.
	assert_eq!(
		engine::count(db, "SELECT count(*) FROM wakes").await,
		234,
		"only the people it is visible to"
	);

	// As each instant comes, its wakes run and wake exactly the people it
	// has arrived for. The jobs' clock is the test's own.
	let jobs = rpc::testing::TestJobs::new(db).service(Some(SERVICE));
	jobs.run_jobs_until_idle().await;
	assert_eq!(
		engine::count(db, "SELECT count(*) FROM wakes").await,
		234,
		"no wake is due yet"
	);

	jobs.advance(Duration::from_hours(2)).await;
	jobs.run_jobs_until_idle().await;
	assert_eq!(
		engine::count(db, "SELECT count(*) FROM wakes").await,
		467,
		"Tokyo's turn woke Tokyo's people, and nobody else"
	);

	jobs.advance(Duration::from_hours(10)).await;
	jobs.run_jobs_until_idle().await;
	assert_eq!(
		engine::count(db, "SELECT count(*) FROM wakes").await,
		700,
		"and the default zone's turn woke the rest"
	);
}

/// The `time_zone` attribute of every `step`th person of `audience`,
/// starting at `first`, set in one call.
async fn zone_every(service: &Service, audience: &[Uuid], first: usize, step: usize, zone: &str) {
	let there: Vec<Uuid> = audience.iter().skip(first).step_by(step).copied().collect();
	ok(
		service,
		"set_person_attributes",
		json!({ "persons": there, "set": { "time_zone": zone }, "remove": [] }),
		None,
	)
	.await;
}

/// Cancelling a large local-time send once its first zone has seen it
/// stops everyone still waiting — more than a batch of rows, so in the
/// background, a batch a write — and leaves what people have seen. Run
/// with no workers: the check runs the fanout and the cancel's job itself.
#[allow(
	clippy::cognitive_complexity,
	reason = "one cancel followed from its answer to the job's last batch"
)]
pub async fn a_large_local_cancel_stops_only_what_is_ahead(service: &Service) {
	let db = &service.db;
	let audience = engine::people(0x7F, 1_300);
	engine::subscribe(service.address, "news.wide", &audience).await;
	// Every third person is in Kiritimati, where the time has passed; the
	// rest say nothing and wait for the default zone's turn.
	zone_every(service, &audience, 0, 3, KIRITIMATI).await;

	let (at, _) = wall_clock(chrono::Duration::hours(10));
	ok(
		service,
		"send",
		json!({
			"idempotency_key": "wide",
			"spec": { "category": "news", "title": "Nine o'clock", "local_time": { "at": at } },
			"target": { "topic": "news.wide" },
		}),
		None,
	)
	.await;

	for step in 0..3 {
		engine::run_step(db, "wide", step).await;
	}

	let progress = engine::progress(service.address, "wide").await;
	assert_eq!(progress.state, leptoskit_notifications::SendState::Complete);
	assert_eq!(progress.delivered, 1_300);

	let cancelled_rows =
		"SELECT count(*) FROM notifications WHERE send_key = 'wide' AND cancelled_at IS NOT NULL";
	let cancelled = ok(
		service,
		"cancel",
		json!({ "idempotency_key": "wide" }),
		None,
	)
	.await;
	assert_eq!(cancelled, 866, "everyone still waiting");
	assert_eq!(
		engine::count(db, cancelled_rows).await,
		0,
		"the cancel's own transaction wrote no row"
	);
	assert_eq!(
		pending(db, "cancel_hidden_rows").await.len(),
		1,
		"a job does"
	);
	assert!(pending(db, "cancel_rows").await.is_empty());
	assert_eq!(
		engine::progress(service.address, "wide").await.state,
		leptoskit_notifications::SendState::Complete,
		"what people have seen stands, so the send is not a cancelled one"
	);

	engine::drain(db).await;
	assert_eq!(engine::count(db, cancelled_rows).await, 866);

	let stored = rows(db, "wide").await;

	for (index, who) in audience.iter().enumerate() {
		assert_eq!(
			stored[who].cancelled_at.is_none(),
			index % 3 == 0,
			"{who}: only a row already visible is kept"
		);
	}

	let again = ok(
		service,
		"cancel",
		json!({ "idempotency_key": "wide" }),
		None,
	)
	.await;
	assert_eq!(again, 0, "nothing is left to stop");
}
