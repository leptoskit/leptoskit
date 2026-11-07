//! Deterministic backpressure: a bounded session channel, without relying on
//! a machine's TCP buffers filling at a particular speed.

use futures::FutureExt;
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
	let (events, batches) = mailbox::channel();
	let (acts, finished) = mpsc::channel(super::super::OUTGOING);
	let (stream_out, stream_frames) = mpsc::channel(super::super::OUTGOING);
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
		pending_progress: tokio::time::Instant::now(),
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
	for update in &updates {
		if let Update::Ok(value) = &update.update {
			session.keep(
				update.id,
				"test".to_owned(),
				json!({}),
				Arc::new(value.clone()),
			);
		}
	}
	session.send_transition(0, updates, 9, true);
	let error = WireError::invalid("changed during snapshot");
	let update = session
		.subscriptions
		.get_mut(&1)
		.unwrap()
		.apply(Changed::Failed(error.clone()));
	session.send_transition(0, vec![SubscriptionUpdate { id: 1, update }], 10, false);
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
	let mut recovered = Vec::new();
	loop {
		let chunk = next(&mut out).await;
		assert!(chunk.full);
		recovered.extend(chunk.updates);
		if chunk.to == 10 {
			break;
		}
		assert_eq!(chunk.to, 9);
	}
	assert_eq!(recovered.len(), 70);
	assert!(
		recovered
			.iter()
			.any(|update| update.id == 1 && update.update == Update::Err(error.clone()))
	);
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
	assert_eq!(final_update.updates, []);
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
async fn authentication_before_resume_does_not_claim_a_recovered_snapshot() {
	let (mut session, mut out, _input, _incoming) = session().await;
	assert!(session.authenticate("first token".to_owned()).await);
	assert!(session.pending.is_none());
	assert!(!session.full_next);
	let Outbound::Frame { text, .. } = out.try_recv().unwrap() else {
		panic!("authentication is acknowledged");
	};
	assert!(matches!(
		serde_json::from_str::<ServerFrame>(&text).unwrap(),
		ServerFrame::AuthStatus {
			refused: false,
			expiring: false
		}
	));
	assert!(out.try_recv().is_err());
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

#[tokio::test]
async fn pending_capacity_closes_at_the_stall_deadline() {
	let (mut session, _out, _input, incoming) = session().await;
	session.live = session
		.live
		.clone()
		.stall_timeout(Duration::from_millis(20));
	change(&mut session, 1, json!("before"));
	change(&mut session, 2, json!("after"));
	assert!(
		!tokio::time::timeout(Duration::from_secs(2), run(session, incoming))
			.await
			.unwrap()
			.unwrap()
	);
}

#[tokio::test]
async fn reauthentication_starts_a_fresh_pending_stall_budget() {
	let (mut session, mut out, _input, _incoming) = session().await;
	session.keep(
		1,
		"unregistered".to_owned(),
		json!({}),
		Arc::new(json!("old")),
	);
	session.pending_progress = tokio::time::Instant::now() - Duration::from_secs(60);
	session.out.try_send(Outbound::Heartbeat).unwrap();
	let mut authentication = Box::pin(session.authenticate("new token".to_owned()));
	assert!(authentication.as_mut().now_or_never().is_none());
	assert!(matches!(out.try_recv().unwrap(), Outbound::Heartbeat));
	assert_eq!(authentication.as_mut().now_or_never(), Some(true));
	drop(authentication);
	assert!(session.full_next);
	assert!(session.pending_deadline().unwrap() > tokio::time::Instant::now());
}

fn generation_pump(
	args: Value,
	_: QueryCx,
	sink: StreamSink,
) -> leptoskit_query::BoxFuture<'static, ()> {
	Box::pin(async move {
		match args.as_str() {
			Some("panic") => panic!("private producer panic"),
			Some("return") => {}
			Some("fail") => sink.fail(&crate::Error::invalid("refused")),
			_ => {
				sink.drive(futures::stream::iter([args]), stream::plain)
					.await;
			}
		}
	})
}

inventory::submit! {
 stream::StreamRegistration {
  name: "generation_test", resumable: false, open: generation_pump, service: None,
 }
}

async fn produced(session: &Session) {
	tokio::time::timeout(Duration::from_secs(5), async {
		while session
			.streams
			.values()
			.any(|running| !running.task.is_finished())
		{
			tokio::task::yield_now().await;
		}
	})
	.await
	.expect("producer finishes into the bounded queue");
}

#[tokio::test]
async fn reused_stream_id_discards_old_items_and_terminal_without_retiring_replacement() {
	let (mut session, mut out, _input, _incoming) = session().await;
	assert!(
		session
			.open_stream(7, "generation_test", json!("old"), 3)
			.await
	);
	produced(&session).await;
	// Cancel invalidates already buffered items as well as stopping the pump.
	assert!(
		session
			.handle(Incoming::Frame(ClientFrame::Cancel { stream: 7 }))
			.await
	);
	assert!(
		session
			.open_stream(7, "generation_test", json!("new"), 3)
			.await
	);
	produced(&session).await;
	assert_eq!(session.stream_frames.len(), 4);
	for _ in 0..2 {
		let frame = session.stream_frames.recv().await.unwrap();
		assert!(session.forward_stream(frame).await);
		assert!(
			out.try_recv().is_err(),
			"cancelled output never reaches transport"
		);
		assert!(
			session.streams.contains_key(&7),
			"old end cannot retire replacement"
		);
	}
	for terminal in [false, true] {
		let frame = session.stream_frames.recv().await.unwrap();
		assert_eq!(frame.terminal, terminal);
		assert!(session.forward_stream(frame).await);
		let Outbound::Frame { text, .. } = out.recv().await.unwrap() else {
			panic!("frame");
		};
		let frame: ServerFrame = serde_json::from_str(&text).unwrap();
		if terminal {
			assert!(matches!(frame, ServerFrame::End { err: None, .. }));
		} else {
			assert!(matches!(frame, ServerFrame::Item { value, .. } if value == json!("new")));
		}
	}
	assert!(session.streams.is_empty());
}

#[tokio::test]
async fn returned_failed_and_panicking_producers_keep_admission_until_terminal() {
	let (mut session, mut out, _input, _incoming) = session().await;
	for mode in ["return", "fail", "panic"] {
		assert!(
			session
				.open_stream(7, "generation_test", json!(mode), 3)
				.await
		);
		produced(&session).await;
		assert!(session.streams.contains_key(&7));
		let frame = session.stream_frames.recv().await.unwrap();
		assert!(frame.terminal);
		assert!(session.forward_stream(frame).await);
		assert!(session.streams.is_empty());
		let Outbound::Frame { text, .. } = out.recv().await.unwrap() else {
			panic!("frame");
		};
		let ServerFrame::End { err, .. } = serde_json::from_str(&text).unwrap() else {
			panic!("terminal");
		};
		assert_eq!(err.is_some(), mode != "return");
		assert!(!text.contains("private producer panic"));
	}
}

#[tokio::test]
async fn interrupting_streams_with_a_stopped_reader_obeys_stall_timeout() {
	let (mut session, _out, _input, _incoming) = session().await;
	session.live = session
		.live
		.clone()
		.stall_timeout(Duration::from_millis(20));
	assert!(
		session
			.open_stream(7, "generation_test", json!("held"), 3)
			.await
	);
	produced(&session).await;
	session.out.send(Outbound::Heartbeat).await.unwrap();
	assert!(
		!tokio::time::timeout(Duration::from_secs(2), session.interrupt_streams())
			.await
			.unwrap()
	);
}

async fn pending_producer() -> (Running, tokio::sync::oneshot::Receiver<()>) {
	struct Dropped(Option<tokio::sync::oneshot::Sender<()>>);
	impl Drop for Dropped {
		fn drop(&mut self) {
			if let Some(sender) = self.0.take() {
				let _ = sender.send(());
			}
		}
	}
	let (dropped, stopped) = tokio::sync::oneshot::channel();
	let (started, entered) = tokio::sync::oneshot::channel();
	let task = tokio::spawn(async move {
		let _dropped = Dropped(Some(dropped));
		let _ = started.send(());
		std::future::pending::<()>().await;
	});
	entered.await.unwrap();
	(
		Running {
			generation: Arc::new(()),
			task,
			credits: Arc::new(tokio::sync::Semaphore::new(1)),
		},
		stopped,
	)
}

#[tokio::test]
async fn terminal_delivery_cancels_a_producer_that_keeps_running() {
	let (mut session, mut out, _input, _incoming) = session().await;
	let (running, stopped) = pending_producer().await;
	let generation = Arc::clone(&running.generation);
	session.streams.insert(7, running);
	let text = serde_json::to_string(&EndReason::Done.frame(7)).unwrap();
	assert!(
		session
			.forward_stream(stream::Frame {
				stream: 7,
				generation,
				text,
				terminal: true,
			})
			.await
	);
	assert!(matches!(out.recv().await, Some(Outbound::Frame { .. })));
	assert!(session.streams.is_empty());
	tokio::time::timeout(Duration::from_secs(1), stopped)
		.await
		.expect("a terminal frame must not detach a still-running producer")
		.unwrap();
}

#[tokio::test]
async fn dropping_a_session_cancels_its_pending_stream_producer() {
	let (mut session, _out, _input, _incoming) = session().await;
	let (running, stopped) = pending_producer().await;
	session.streams.insert(7, running);
	drop(session);
	tokio::time::timeout(Duration::from_secs(1), stopped)
		.await
		.expect("session cancellation must not detach its producers")
		.unwrap();
}
