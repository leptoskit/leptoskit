//! Deterministic backpressure: a bounded session channel, without relying on
//! a machine's TCP buffers filling at a particular speed.

use serde_json::json;

use super::*;

struct ReplacementCaller;

impl crate::server::Identify for ReplacementCaller {
	fn identify<'a>(
		&'a self,
		_request: &'a http::request::Parts,
	) -> leptoskit_query::BoxFuture<'a, crate::Result<Identity>> {
		Box::pin(async { Ok(Identity::anonymous().with_caller("replacement")) })
	}
}

async fn session() -> (
	Session,
	mpsc::Receiver<Outbound>,
	mpsc::Sender<Incoming>,
	mpsc::Receiver<Incoming>,
) {
	let db = leptoskit_db::testing::sqlite().await;
	let live = Live::new(db.clone());
	let (out, outgoing) = mpsc::channel(1);
	let (input, incoming) = mpsc::channel(1);
	let (events, batches) = mpsc::unbounded_channel();
	let (acts, finished) = mpsc::unbounded_channel();
	let (stream_out, stream_frames) = mpsc::unbounded_channel();
	live.hub().connect(1, events);
	let applied = live.hub().watch();
	let session = Session {
		live,
		number: 1,
		identity: Identity::anonymous(),
		epoch: 0,
		lease: IdentityLease::default(),
		db,
		member: "test".to_owned(),
		caller: Arc::new(Mutex::new(None)),
		out,
		batches,
		finished,
		acts,
		actions: Arc::new(tokio::sync::Semaphore::new(MAX_ACTIONS)),
		applied,
		subscriptions: HashMap::new(),
		joined: HashMap::new(),
		streams: HashMap::new(),
		stream_frames,
		stream_out,
		last_sent: 0,
		pending: None,
		full_next: false,
		latest: 0,
		extra_errors: HashMap::new(),
		transport_failed: false,
		awaiting: None,
		resume_floor: None,
		frame_tokens: FRAME_BURST * 1_000,
		frame_at: std::time::Instant::now(),
	};
	(session, outgoing, input, incoming)
}

fn run(
	mut session: Session,
	mut frames: mpsc::Receiver<Incoming>,
) -> tokio::task::JoinHandle<bool> {
	tokio::spawn(async move {
		let (_drain, mut drained) = broadcast::channel(1);
		session.run(&mut frames, &mut drained).await
	})
}

async fn next(out: &mut mpsc::Receiver<Outbound>) -> Transition {
	tokio::time::timeout(Duration::from_secs(10), async {
		loop {
			let frame = out.recv().await.expect("session stays open");
			if let Outbound::Frame { text, .. } = frame
				&& let ServerFrame::Transition(transition) = serde_json::from_str(&text).unwrap()
			{
				return transition;
			}
		}
	})
	.await
	.expect("capacity, without another commit, wakes the session")
}

fn change(session: &mut Session, to: u64, value: Value) {
	session.keep(1, "test".to_owned(), json!({}), Arc::new(value.clone()));
	session.send_transition(
		session.last_sent,
		vec![SubscriptionUpdate {
			id: 1,
			update: Update::Ok(value),
		}],
		to,
		false,
	);
}

#[tokio::test]
async fn the_final_pending_update_arrives_without_a_later_database_commit() {
	let (mut session, mut out, _input, incoming) = session().await;
	change(&mut session, 1, json!("before"));
	change(&mut session, 2, json!("after"));
	assert_eq!(session.last_sent, 1, "queued does not mean completely sent");
	let running = run(session, incoming);
	assert_eq!(next(&mut out).await.to, 1);
	let final_update = next(&mut out).await;
	assert_eq!(final_update.to, 2);
	assert_eq!(final_update.updates[0].update, Update::Ok(json!("after")));
	running.abort();
}

#[tokio::test]
async fn changes_while_blocked_coalesce_into_the_latest_error_snapshot() {
	let (mut session, mut out, _input, incoming) = session().await;
	change(&mut session, 1, json!("before"));
	change(&mut session, 2, json!("middle"));
	let error = WireError {
		status: 403,
		message: "no longer available".to_owned(),
	};
	let update = session
		.subscriptions
		.get_mut(&1)
		.unwrap()
		.apply(Changed::Failed(error.clone()));
	session.send_transition(
		session.last_sent,
		vec![SubscriptionUpdate { id: 1, update }],
		3,
		false,
	);
	let running = run(session, incoming);
	assert_eq!(next(&mut out).await.to, 1);
	assert_eq!(next(&mut out).await.to, 2);
	let latest = next(&mut out).await;
	assert!(latest.full);
	assert_eq!(latest.to, 3);
	assert_eq!(latest.updates[0].update, Update::Err(error));
	running.abort();
}

#[tokio::test]
async fn a_transition_larger_than_the_transport_queue_keeps_progressing() {
	let (mut session, mut out, _input, incoming) = session().await;
	let updates: Vec<_> = (0..70)
		.map(|id| {
			SubscriptionUpdate {
				id,
				update: Update::Ok(json!("x".repeat(400_000))),
			}
		})
		.collect();
	session.send_transition(0, updates, 9, true);
	let running = run(session, incoming);
	let mut ids = Vec::new();
	let mut chunks = 0;
	while ids.len() < 70 {
		let chunk = next(&mut out).await;
		chunks += 1;
		ids.extend(chunk.updates.iter().map(|update| update.id));
		assert_eq!(chunk.to, if ids.len() == 70 { 9 } else { 0 });
	}
	assert!(chunks > super::super::OUTGOING);
	ids.sort_unstable();
	assert_eq!(ids, (0..70).collect::<Vec<_>>());
	running.abort();
}

#[tokio::test]
async fn a_closed_transport_does_not_retain_the_pending_session() {
	let (mut session, out, _input, incoming) = session().await;
	change(&mut session, 1, json!("before"));
	change(&mut session, 2, json!("after"));
	drop(out);
	let running = run(session, incoming);
	assert!(
		!tokio::time::timeout(Duration::from_secs(2), running)
			.await
			.unwrap()
			.unwrap()
	);
}

#[tokio::test]
async fn removing_a_subscription_discards_its_unsent_result() {
	let (mut session, mut out, _input, incoming) = session().await;
	change(&mut session, 1, json!("before"));
	change(&mut session, 2, json!("removed"));
	assert!(
		session
			.handle(Incoming::Frame(ClientFrame::Unsub { id: 1 }))
			.await
	);
	let running = run(session, incoming);
	assert_eq!(next(&mut out).await.to, 1);
	let final_update = next(&mut out).await;
	assert_eq!(final_update.to, 2);
	assert!(final_update.updates.is_empty());
	running.abort();
}

#[tokio::test]
async fn replacing_a_subscription_discards_its_unsent_result() {
	let (mut session, mut out, _input, incoming) = session().await;
	change(&mut session, 1, json!("before"));
	change(&mut session, 2, json!("old query"));
	session.keep(
		1,
		"replacement".to_owned(),
		json!({}),
		Arc::new(json!("new query")),
	);
	let running = run(session, incoming);
	assert_eq!(next(&mut out).await.to, 1);
	let final_update = next(&mut out).await;
	assert_eq!(final_update.to, 2);
	assert!(
		final_update.updates.is_empty(),
		"the replacement is answered by its own Done"
	);
	running.abort();
}

#[tokio::test]
async fn a_resume_failure_survives_coalescing_without_a_subscription() {
	let (mut session, mut out, _input, incoming) = session().await;
	change(&mut session, 1, json!("before"));
	change(&mut session, 2, json!("after"));
	let error = WireError::invalid("query no longer exists");
	session.send_transition(
		1,
		vec![SubscriptionUpdate {
			id: 7,
			update: Update::Err(error.clone()),
		}],
		3,
		true,
	);
	let running = run(session, incoming);
	assert_eq!(next(&mut out).await.to, 1);
	assert_eq!(next(&mut out).await.to, 2);
	let recovery = next(&mut out).await;
	assert_eq!(recovery.to, 3);
	assert!(recovery.full);
	assert!(
		recovery
			.updates
			.iter()
			.any(|update| update.id == 7 && update.update == Update::Err(error.clone()))
	);
	running.abort();
}

#[tokio::test]
async fn a_new_identity_discards_the_previous_identitys_unsent_snapshot() {
	let (mut session, mut out, _input, incoming) = session().await;
	session.live = session.live.clone().identify(ReplacementCaller);
	change(&mut session, 1, json!("already queued"));
	change(&mut session, 2, json!("old identity pending"));
	// The synthetic query is deliberately unregistered. Reauthorization
	// must report its new error instead of releasing the old pending value.
	let running = tokio::spawn(async move {
		let mut incoming = incoming;
		let (_drain, mut drained) = broadcast::channel(1);
		assert!(session.authenticate("new identity".to_owned()).await);
		assert_eq!(session.identity.caller.as_deref(), Some("replacement"));
		session.run(&mut incoming, &mut drained).await
	});
	assert_eq!(next(&mut out).await.to, 1);
	let recovery = next(&mut out).await;
	assert!(recovery.full);
	assert_eq!(recovery.updates.len(), 1);
	assert!(matches!(recovery.updates[0].update, Update::Err(_)));
	running.abort();
}
