//! A hub against a scripted control plane, with each tenant's database in
//! memory: it opens the tenants it is given, routes each request to the
//! tenant its path or host names and no other, counts accepted pushes once,
//! keeps a tenant it can't serve apart with its reason, and drops the ones
//! it is told to.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptoskit_cloud::notifications::HubState;
use leptoskit_cloud::notifications::KnownTenant;
use leptoskit_cloud::notifications::NextSecrets;
use leptoskit_cloud::notifications::NotificationsReported;
use leptoskit_cloud::notifications::TenantChanges;
use leptoskit_cloud::notifications::TenantConfig;
use leptoskit_cloud::notifications::TenantReport;
use leptoskit_cloud::notifications::TenantSecrets;
use leptoskit_cloud::notifications::UsageReport;
use leptoskit_cloud::workers::RuntimeValue;
use leptoskit_cloud_notifications::Cursor;
use leptoskit_cloud_notifications::Hub;
use leptoskit_cloud_notifications::Open;
use leptoskit_cloud_notifications::Pending;
use leptoskit_cloud_notifications::Plane;
use leptoskit_cloud_runtime::EnvironmentRef;
use leptoskit_cloud_runtime::NamedRef;
use leptoskit_db::Db;
use leptoskit_notifications::push::NewPushAcceptance;
use serde_json::json;
use uuid::Uuid;

const A: Uuid = Uuid::from_u128(0xaaaa_0000_0000_0000_0000_0000_0000_0001);
const B: Uuid = Uuid::from_u128(0xbbbb_0000_0000_0000_0000_0000_0000_0002);
const C: Uuid = Uuid::from_u128(0xcccc_0000_0000_0000_0000_0000_0000_0003);
const PREFIX: &str = "/_leptoskit/services/notifications";

/// A control plane that answers what the test queues, and keeps what it is
/// asked and told.
#[derive(Default)]
struct Scripted {
	answers: Mutex<VecDeque<TenantChanges>>,
	asked: Mutex<Vec<Vec<KnownTenant>>>,
	reports: Mutex<Vec<(Vec<TenantReport>, Vec<UsageReport>)>>,
}

impl Scripted {
	fn answer(&self, changes: TenantChanges) {
		self.answers
			.lock()
			.expect("not poisoned")
			.push_back(changes);
	}

	fn last_known(&self) -> Vec<KnownTenant> {
		let mut known = self
			.asked
			.lock()
			.expect("not poisoned")
			.last()
			.cloned()
			.unwrap_or_default();
		known.sort_by_key(|known| known.tenant);
		known
	}

	fn last_report(&self) -> (Vec<TenantReport>, Vec<UsageReport>) {
		let (mut tenants, usage) = self
			.reports
			.lock()
			.expect("not poisoned")
			.last()
			.cloned()
			.unwrap_or_default();
		tenants.sort_by_key(|report| report.tenant);
		(tenants, usage)
	}
}

impl Plane for Scripted {
	fn tenants(&self, known: Vec<KnownTenant>) -> Pending<'_, Result<TenantChanges, String>> {
		self.asked.lock().expect("not poisoned").push(known);
		let answer = self
			.answers
			.lock()
			.expect("not poisoned")
			.pop_front()
			.unwrap_or(TenantChanges {
				changed: Vec::new(),
				removed: Vec::new(),
				more: false,
			});
		Box::pin(async move { Ok(answer) })
	}

	fn report(
		&self,
		tenants: Vec<TenantReport>,
		usage: Vec<UsageReport>,
	) -> Pending<'_, Result<NotificationsReported, String>> {
		let cursors = usage
			.iter()
			.map(|report| (report.tenant, Some(report.until.clone())))
			.collect();
		self.reports
			.lock()
			.expect("not poisoned")
			.push((tenants.clone(), usage));
		Box::pin(async move {
			Ok(NotificationsReported {
				tenants: u32::try_from(tenants.len()).expect("few"),
				cursors,
			})
		})
	}
}

/// Each tenant's database in memory, by its address, kept across reopening,
/// how many times one was asked for, and the addresses it refuses, as a
/// server refuses a password it no longer has.
#[derive(Default)]
struct Memory(Mutex<HashMap<String, Db>>, AtomicUsize, Mutex<Vec<String>>);

impl Memory {
	fn db(&self, address: &str) -> Db {
		self.0
			.lock()
			.expect("not poisoned")
			.get(address)
			.cloned()
			.expect("opened")
	}
}

impl Open for Memory {
	fn open<'a>(&'a self, address: &'a str) -> Pending<'a, Result<Db, String>> {
		self.1.fetch_add(1, Ordering::Relaxed);
		let refused = self
			.2
			.lock()
			.expect("not poisoned")
			.iter()
			.any(|refused| refused == address);
		Box::pin(async move {
			if refused {
				return Err("its database couldn't be reached".to_owned());
			}

			if let Some(db) = self.0.lock().expect("not poisoned").get(address) {
				return Ok(db.clone());
			}

			let db = Db::connect("sqlite::memory:")
				.await
				.map_err(|error| error.to_string())?;
			self.0
				.lock()
				.expect("not poisoned")
				.insert(address.to_owned(), db.clone());
			Ok(db)
		})
	}
}

fn token(tenant: Uuid) -> String {
	format!("token-{}", tenant.simple())
}

fn address(tenant: Uuid) -> String {
	format!("memory://{}", tenant.simple())
}

fn tenant(tenant: Uuid, version: i64, host: &str, settings: &[(&str, &str)]) -> TenantConfig {
	let named = |slug: &str| {
		NamedRef {
			slug: slug.to_owned(),
			id: Uuid::now_v7(),
		}
	};
	TenantConfig {
		tenant,
		version,
		environment: RuntimeValue(EnvironmentRef {
			organization: named("acme"),
			project: named("notes"),
			environment: named("production"),
		}),
		hostnames: vec![host.to_owned()],
		origin: Some(format!("https://{host}")),
		usage_cursor: None,
		secrets: TenantSecrets {
			database_url: address(tenant),
			token: token(tenant),
			settings: settings
				.iter()
				.map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
				.collect(),
			next: None,
		},
	}
}

/// The hub on a free port, and its address.
async fn serve(hub: &Hub) -> String {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
		.await
		.expect("a port");
	let address = listener.local_addr().expect("an address");
	let router = hub.router();
	tokio::spawn(async move {
		leptoskit_axum::axum::serve(listener, router)
			.await
			.expect("serves");
	});
	format!("http://{address}")
}

/// `send_status` for a key no send holds: `404` from a tenant that takes
/// the token as its app's, `403` from one that doesn't, and the hub's own
/// `404` where no tenant is.
async fn status(url: &str, host: Option<&str>, token: &str) -> (u16, String) {
	let mut request = reqwest::Client::new()
		.post(url)
		.bearer_auth(token)
		.json(&json!({ "idempotency_key": "nothing-sent" }));

	if let Some(host) = host {
		request = request.header("host", host);
	}

	let response = request.send().await.expect("answers");
	(
		response.status().as_u16(),
		response.text().await.expect("a body"),
	)
}

async fn accepted(db: &Db, accepted_at: leptoskit_db::Timestamp) {
	NewPushAcceptance {
		id: Uuid::now_v7(),
		delivery: Uuid::now_v7(),
		platform: "web".to_owned(),
		accepted_at,
	}
	.insert(&db.system("the test arranges an accepted push"))
	.await
	.expect("inserted");
}

#[tokio::test(flavor = "multi_thread")]
#[expect(clippy::too_many_lines, reason = "one hub's life, in order")]
async fn a_hub_serves_each_tenant_apart_and_counts_its_pushes_once() {
	let plane = Scripted::default();
	let memory = Arc::new(Memory::default());
	let hub = Hub::new(Arc::clone(&memory) as Arc<dyn Open>);
	let base = serve(&hub).await;

	// Two tenants, and one whose push settings are half there.
	plane.answer(TenantChanges {
		changed: vec![
			tenant(A, 1, "a.apps.example.test", &[]),
			tenant(
				B,
				4,
				"b.apps.example.test",
				&[("LEPTOSKIT_NOTIFICATIONS_OPEN_TOPICS", "news")],
			),
		],
		removed: Vec::new(),
		more: true,
	});
	plane.answer(TenantChanges {
		changed: vec![tenant(
			C,
			2,
			"c.apps.example.test",
			&[("LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID", "ABC123")],
		)],
		removed: Vec::new(),
		more: false,
	});
	let now = leptoskit_db::now();
	let round = hub.round(&plane, now).await;
	assert_eq!(round.opened, vec![A, B]);
	assert_eq!(round.failed.len(), 1);
	assert_eq!(round.failed[0].0, C);
	assert!(round.failed[0].1.contains("APNS"), "{:?}", round.failed);
	assert_eq!(round.unreachable, None);
	assert_eq!(hub.serving(), BTreeMap::from([(A, 1), (B, 4)]));

	let (reports, usage) = plane.last_report();
	assert_eq!(reports.len(), 3);
	assert_eq!(reports[0].state, HubState::Serving);
	assert_eq!((reports[0].tenant, reports[0].version), (A, 1));
	assert_eq!((reports[1].tenant, reports[1].version), (B, 4));
	assert!(
		matches!(&reports[2].state, HubState::Failed { reason } if reason.contains("APNS")),
		"{reports:?}"
	);
	assert_eq!(usage, Vec::new());

	// By path, each tenant knows its own app and nobody else's.
	let by_path = |tenant: Uuid| format!("{base}/t/{}{PREFIX}/rpc/send_status", tenant.simple());
	assert_eq!(status(&by_path(A), None, &token(A)).await.0, 404);
	assert_eq!(status(&by_path(B), None, &token(B)).await.0, 404);
	assert_eq!(status(&by_path(A), None, &token(B)).await.0, 403);
	assert_eq!(status(&by_path(B), None, &token(A)).await.0, 403);
	let (code, body) = status(&by_path(C), None, &token(C)).await;
	assert_eq!(
		(code, body.as_str()),
		(404, "no notifications are served here")
	);

	// By host, on the service's routes only, whatever the port.
	let by_host = format!("{base}{PREFIX}/rpc/send_status");
	assert_eq!(
		status(&by_host, Some("a.apps.example.test"), &token(A))
			.await
			.0,
		404
	);
	assert_eq!(
		status(&by_host, Some("B.apps.example.test:443"), &token(A))
			.await
			.0,
		403
	);
	let (code, body) = status(&by_host, Some("elsewhere.example.test"), &token(A)).await;
	assert_eq!(
		(code, body.as_str()),
		(404, "no notifications are served here")
	);
	let (code, _) = status(
		&format!("{base}/t/{}/rpc/send_status", A.simple()),
		None,
		&token(A),
	)
	.await;
	assert_eq!(code, 404);
	assert_eq!(
		reqwest::get(format!("{base}/healthz"))
			.await
			.expect("answers")
			.status()
			.as_u16(),
		200
	);

	// The failed tenant is held at its version until it is time to try
	// again, so it isn't sent again each round.
	let round = hub.round(&plane, now).await;
	assert_eq!(round.opened, Vec::<Uuid>::new());
	assert_eq!(
		plane.last_known(),
		vec![
			KnownTenant {
				tenant: A,
				version: 1
			},
			KnownTenant {
				tenant: B,
				version: 4
			},
			KnownTenant {
				tenant: C,
				version: 2
			},
		]
	);

	// Pushes accepted before the lag are counted once; one accepted since,
	// in a later round.
	let a = memory.db(&address(A));
	let minute = chrono::TimeDelta::minutes(1);
	accepted(&a, now - minute * 5).await;
	accepted(&a, now - minute * 3).await;
	accepted(&a, now - chrono::TimeDelta::seconds(10)).await;
	let round = hub.round(&plane, now).await;
	assert_eq!(round.pushes, BTreeMap::from([(A, 2)]));
	let (_, usage) = plane.last_report();
	assert_eq!(
		usage,
		vec![UsageReport {
			tenant: A,
			after: None,
			until: Cursor::at(now - minute).to_string(),
			pushes: 2,
		}]
	);

	let round = hub.round(&plane, now).await;
	assert_eq!(round.pushes, BTreeMap::new());

	let later = now + minute * 2;
	let round = hub.round(&plane, later).await;
	assert_eq!(round.pushes, BTreeMap::from([(A, 1)]));
	let (_, usage) = plane.last_report();
	assert_eq!(usage[0].after, Some(Cursor::at(now - minute).to_string()));

	// A new version is opened in place of the old, on the same database
	// and its pool.
	let opens = memory.1.load(Ordering::Relaxed);
	plane.answer(TenantChanges {
		changed: vec![tenant(A, 2, "a2.apps.example.test", &[])],
		removed: Vec::new(),
		more: false,
	});
	let round = hub.round(&plane, later).await;
	assert_eq!(round.opened, vec![A]);
	assert_eq!(memory.1.load(Ordering::Relaxed), opens);
	assert_eq!(hub.serving()[&A], 2);
	assert_eq!(
		status(&by_host, Some("a2.apps.example.test"), &token(A))
			.await
			.0,
		404
	);
	let (code, body) = status(&by_host, Some("a.apps.example.test"), &token(A)).await;
	assert_eq!(
		(code, body.as_str()),
		(404, "no notifications are served here")
	);

	// A hostname two tenants claim reaches the one opened later, and goes
	// back when that one lets it go.
	let mut b = tenant(B, 5, "b.apps.example.test", &[]);
	b.hostnames.push("a2.apps.example.test".to_owned());
	plane.answer(TenantChanges {
		changed: vec![b],
		removed: Vec::new(),
		more: false,
	});
	hub.round(&plane, later).await;
	assert_eq!(
		status(&by_host, Some("a2.apps.example.test"), &token(B))
			.await
			.0,
		404
	);
	plane.answer(TenantChanges {
		changed: vec![tenant(B, 6, "b.apps.example.test", &[])],
		removed: Vec::new(),
		more: false,
	});
	hub.round(&plane, later).await;
	assert_eq!(
		status(&by_host, Some("a2.apps.example.test"), &token(A))
			.await
			.0,
		404
	);

	// A tenant whose settings fail says why without showing them, even
	// where the service's own words would quote one.
	let secret = "production-not-quite";
	plane.answer(TenantChanges {
		changed: vec![tenant(
			C,
			3,
			"c.apps.example.test",
			&[
				("LEPTOSKIT_NOTIFICATIONS_APNS_KEY_P8", "not a key"),
				("LEPTOSKIT_NOTIFICATIONS_APNS_KEY_ID", "ABC123"),
				("LEPTOSKIT_NOTIFICATIONS_APNS_TEAM_ID", "TEAM123"),
				("LEPTOSKIT_NOTIFICATIONS_APNS_TOPIC", "com.example.notes"),
				("LEPTOSKIT_NOTIFICATIONS_APNS_ENVIRONMENT", secret),
			],
		)],
		removed: Vec::new(),
		more: false,
	});
	let round = hub.round(&plane, later).await;
	assert_eq!(round.failed.len(), 1, "{round:?}");
	let reason = &round.failed[0].1;
	assert!(
		reason.contains("APNS_ENVIRONMENT is \"[redacted]\""),
		"{reason}"
	);
	assert!(!reason.contains(secret), "{reason}");

	// A removed tenant is served no more.
	plane.answer(TenantChanges {
		changed: Vec::new(),
		removed: vec![A],
		more: false,
	});
	let round = hub.round(&plane, later).await;
	assert_eq!(round.removed, vec![A]);
	assert_eq!(hub.serving(), BTreeMap::from([(B, 6)]));
	let (code, body) = status(&by_path(A), None, &token(A)).await;
	assert_eq!(
		(code, body.as_str()),
		(404, "no notifications are served here")
	);
}

#[path = "../../leptoskit_cloud/tests/support/postgres.rs"]
mod server;

/// A tenant moving to a new password and token, as after a worker that saw
/// them is revoked: the hub takes either token, and reaches the database
/// with whichever password it takes.
#[tokio::test(flavor = "multi_thread")]
async fn a_tenant_being_given_new_values_takes_the_old_and_the_new() {
	let plane = Scripted::default();
	let memory = Arc::new(Memory::default());
	let hub = Hub::new(Arc::clone(&memory) as Arc<dyn Open>);
	let base = serve(&hub).await;
	let by_path = format!("{base}/t/{}{PREFIX}/rpc/send_status", A.simple());
	let next_address = format!("{}-next", address(A));
	// The same database, under the password it moves to.
	let db = Db::connect("sqlite::memory:").await.expect("a database");
	for address in [address(A), next_address.clone()] {
		memory
			.0
			.lock()
			.expect("not poisoned")
			.insert(address, db.clone());
	}
	let rotating = |version| {
		let mut config = tenant(A, version, "a.apps.example.test", &[]);
		config.secrets.next = Some(NextSecrets {
			database_url: next_address.clone(),
			token: "next-token".to_owned(),
		});
		config
	};

	// While the old password still holds, both tokens are the app's.
	plane.answer(TenantChanges {
		changed: vec![rotating(2)],
		removed: Vec::new(),
		more: false,
	});
	let round = hub.round(&plane, leptoskit_db::now()).await;
	assert_eq!(round.opened, vec![A], "{round:?}");
	assert_eq!(status(&by_path, None, &token(A)).await.0, 404);
	assert_eq!(status(&by_path, None, "next-token").await.0, 404);
	assert_eq!(status(&by_path, None, "some-other-token").await.0, 403);

	// A release set the new password and failed: the database refuses the
	// old one, and the hub reaches it with the new.
	memory.2.lock().expect("not poisoned").push(address(A));
	let opens = memory.1.load(Ordering::Relaxed);
	plane.answer(TenantChanges {
		changed: vec![rotating(3)],
		removed: Vec::new(),
		more: false,
	});
	let round = hub.round(&plane, leptoskit_db::now()).await;
	assert_eq!(round.opened, vec![A], "{round:?}");
	assert_eq!(
		memory.1.load(Ordering::Relaxed),
		opens + 2,
		"the old password, refused, then the new"
	);
	assert_eq!(status(&by_path, None, "next-token").await.0, 404);

	// Once the new values are the tenant's, the old token is refused.
	let mut moved = tenant(A, 4, "a.apps.example.test", &[]);
	moved.secrets.database_url = next_address.clone();
	moved.secrets.token = "next-token".to_owned();
	plane.answer(TenantChanges {
		changed: vec![moved],
		removed: Vec::new(),
		more: false,
	});
	let round = hub.round(&plane, leptoskit_db::now()).await;
	assert_eq!(round.opened, vec![A], "{round:?}");
	assert_eq!(status(&by_path, None, "next-token").await.0, 404);
	assert_eq!(status(&by_path, None, &token(A)).await.0, 403);
}

/// On a real Postgres server, a tenant's database as the cluster's worker
/// makes it, a login role that is nothing more, is migrated and served by
/// the hub, and its pushes are counted there. When a release gives the role
/// a new password, the old one's sessions end, and the hub reaches the
/// database with the new one.
#[tokio::test(flavor = "multi_thread")]
async fn a_tenant_is_served_from_the_database_the_cluster_worker_made() {
	use leptoskit_cloud_worker::cluster::Databases as _;
	use leptoskit_db::Read as _;

	let Some(server) = server::Postgres::start() else {
		return;
	};
	let mut admin = url::Url::parse(&server.database()).expect("an address");
	admin.set_path("/postgres");
	admin.set_query(Some("sslmode=disable"));
	let name = format!("ntf_{}", A.simple());
	leptoskit_cloud_worker::Postgres::new(admin.as_str())
		.expect("an administrator")
		.ensure(
			&leptoskit_cloud::workers::DatabaseStep {
				name: name.clone(),
				role: name.clone(),
				size: leptoskit_cloud::size::DatabaseSize::Shared,
				backups: leptoskit_cloud::size::Backups::Daily,
			},
			"tenant-password",
		)
		.await
		.expect("the worker makes it");

	let mut own = admin.clone();
	own.set_username(&name).expect("a user");
	own.set_password(Some("tenant-password"))
		.expect("a password");
	own.set_path(&format!("/{name}"));
	let mut config = tenant(A, 1, "a.apps.example.test", &[]);
	config.secrets.database_url = own.to_string();

	let plane = Scripted::default();
	plane.answer(TenantChanges {
		changed: vec![config],
		removed: Vec::new(),
		more: false,
	});
	let hub = Hub::new(Arc::new(leptoskit_cloud_notifications::Connect {
		connections: 2,
	}));
	let base = serve(&hub).await;
	let now = leptoskit_db::now();
	let round = hub.round(&plane, now).await;
	assert_eq!(round.failed, Vec::new());
	assert_eq!(round.opened, vec![A]);

	let path = format!("{base}/t/{}{PREFIX}/rpc/send_status", A.simple());
	assert_eq!(status(&path, None, &token(A)).await.0, 404);
	assert_eq!(status(&path, None, &token(B)).await.0, 403);

	let db = Db::connect(own.as_str())
		.await
		.expect("connects as the tenant");
	accepted(&db, now - chrono::TimeDelta::minutes(2)).await;
	let round = hub.round(&plane, now).await;
	assert_eq!(round.pushes, BTreeMap::from([(A, 1)]));

	// A release sets the password the tenant moves to, and fails: the old
	// password's sessions end (this server trusts every password, so the
	// sessions are what can be seen), and the hub opens the database with
	// the new one.
	let session = |sql: &str| {
		leptoskit_db::Statement {
			sql: sql.into(),
			args: Vec::new(),
		}
	};
	let pid = match &db
		.fetch(session("SELECT pg_backend_pid()"))
		.await
		.expect("a session")[0][0]
	{
		leptoskit_db::Value::Int(pid) => *pid,
		other => panic!("a pid: {other:?}"),
	};
	let step = leptoskit_cloud::workers::DatabaseStep {
		name: name.clone(),
		role: name.clone(),
		size: leptoskit_cloud::size::DatabaseSize::Shared,
		backups: leptoskit_cloud::size::Backups::Daily,
	};
	leptoskit_cloud_worker::Postgres::new(admin.as_str())
		.expect("an administrator")
		.ensure(&step, "tenant-password-2")
		.await
		.expect("the worker sets it");
	let watching = Db::connect(admin.as_str()).await.expect("connects");
	let open = watching
		.fetch(session(&format!(
			"SELECT count(*) FROM pg_stat_activity WHERE pid = {pid}"
		)))
		.await
		.expect("sessions");
	assert!(
		matches!(open[0][0], leptoskit_db::Value::Int(0)),
		"the old password's sessions end with it: {open:?}"
	);
	let mut next = own.clone();
	next.set_password(Some("tenant-password-2"))
		.expect("a password");
	let mut config = tenant(A, 2, "a.apps.example.test", &[]);
	config.secrets.database_url = own.to_string();
	config.secrets.next = Some(NextSecrets {
		database_url: next.to_string(),
		token: "next-token".to_owned(),
	});
	plane.answer(TenantChanges {
		changed: vec![config],
		removed: Vec::new(),
		more: false,
	});
	let round = hub.round(&plane, now).await;
	assert_eq!(round.failed, Vec::new());
	assert_eq!(round.opened, vec![A]);
	assert_eq!(status(&path, None, "next-token").await.0, 404);
}
