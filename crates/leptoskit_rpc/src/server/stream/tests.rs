//! Output capacity bounds queued stream items independently of credits.

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::*;

async fn receive(receiver: &mut mpsc::Receiver<Frame>) -> ServerFrame {
	let text = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
		.await
		.expect("the stream produces its next frame")
		.expect("the sink remains open");
	serde_json::from_str(&text.text).unwrap()
}

async fn saw_two(seen: &Semaphore) {
	tokio::time::timeout(Duration::from_secs(1), seen.acquire_many(2))
		.await
		.expect("the producer reaches its first blocked send")
		.unwrap()
		.forget();
}

#[tokio::test]
async fn renewed_credits_do_not_make_the_sink_read_past_output_capacity() {
	let (out, mut receiver) = mpsc::channel(1);
	let credits = Arc::new(Semaphore::new(1_000));
	let seen = Arc::new(Semaphore::new(0));
	let counted = Arc::clone(&seen);
	let items = futures::stream::iter(0_u32..3).inspect(move |_| counted.add_permits(1));
	let task = tokio::spawn(StreamSink::new(1, out, Arc::clone(&credits)).drive(items, plain));
	saw_two(&seen).await;
	// One item is queued and one is waiting to be sent. Replenishing demand
	// cannot let the producer collect the remaining items in memory.
	credits.add_permits(1_000);

	assert!(!task.is_finished());
	assert_eq!(seen.available_permits(), 0);

	for expected in 0..3 {
		let ServerFrame::Item { value, .. } = receive(&mut receiver).await else {
			panic!("each item arrives in order");
		};
		assert_eq!(value, serde_json::json!(expected));
	}

	assert!(matches!(
		receive(&mut receiver).await,
		ServerFrame::End { .. }
	));
	task.await.unwrap();
}

#[tokio::test]
async fn dropping_the_receiver_stops_a_blocked_producer() {
	let (out, receiver) = mpsc::channel(1);
	let seen = Arc::new(Semaphore::new(0));
	let counted = Arc::clone(&seen);
	let items = futures::stream::iter(0_u32..1_000).inspect(move |_| counted.add_permits(1));
	let sink = StreamSink::new(1, out, Arc::new(Semaphore::new(1_000)));
	let task = tokio::spawn(sink.drive(items, plain));
	saw_two(&seen).await;
	drop(receiver);

	tokio::time::timeout(Duration::from_secs(1), task)
		.await
		.unwrap()
		.unwrap();
	assert_eq!(seen.available_permits(), 0);
}

#[tokio::test]
async fn a_terminal_error_waits_for_capacity_instead_of_disappearing() {
	let (out, mut receiver) = mpsc::channel(1);
	out.send(Frame {
		stream: 0,
		generation: Arc::new(()),
		text: "held".to_owned(),
		terminal: false,
	})
	.await
	.unwrap();
	let sink = StreamSink::new(1, out, Arc::new(Semaphore::new(0)));
	let error = Error::invalid("refused");
	let mut refusal = sink.completion();
	let fail: fn(&StreamSink, &Error) = StreamSink::fail;
	fail(&sink, &error);
	drop(sink);
	assert!(futures::poll!(refusal.as_mut()).is_pending());
	assert_eq!(receiver.recv().await.unwrap().text, "held");
	refusal.await;
	let ServerFrame::End {
		err: Some(error), ..
	} = receive(&mut receiver).await
	else {
		panic!("the terminal error survives backpressure");
	};

	assert_eq!(error.message, "refused");
}

#[tokio::test]
async fn a_failed_producer_stays_tracked_until_its_terminal_frame_is_queued() {
	let (out, mut receiver) = mpsc::channel(1);
	out.send(Frame {
		stream: 0,
		generation: Arc::new(()),
		text: "held".to_owned(),
		terminal: false,
	})
	.await
	.unwrap();
	let sink = StreamSink::new(1, out, Arc::new(Semaphore::new(1)));
	let completion = sink.completion();
	let mut running = Box::pin(async move {
		let items = futures::stream::iter([Err::<u32, _>(Error::invalid("failed item"))]);
		sink.drive(items, fallible).await;
		completion.await;
	});
	assert!(futures::poll!(running.as_mut()).is_pending());
	assert_eq!(receiver.recv().await.unwrap().text, "held");
	running.await;

	let ServerFrame::End {
		err: Some(error), ..
	} = receive(&mut receiver).await
	else {
		panic!("the failure is delivered before the producer task finishes");
	};
	assert_eq!(error.message, "failed item");
}

#[tokio::test]
async fn an_oversized_item_fails_explicitly_and_stops_reading() {
	let (out, mut receiver) = mpsc::channel(1);
	let reads = Arc::new(AtomicUsize::new(0));
	let counted = Arc::clone(&reads);
	let items = futures::stream::iter(["x".repeat(MAX_FRAME_BYTES), "unread".to_owned()]).inspect(
		move |_| {
			counted.fetch_add(1, Ordering::SeqCst);
		},
	);
	let sink = StreamSink::new(1, out, Arc::new(Semaphore::new(8)));
	let task = tokio::spawn(sink.drive(items, plain));
	let ServerFrame::End {
		err: Some(error), ..
	} = receive(&mut receiver).await
	else {
		panic!("oversized frames are refused explicitly");
	};
	assert_eq!(error.status, 413);
	task.await.unwrap();
	assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_idle_stream_does_not_reserve_capacity_from_a_ready_neighbor() {
	let (out, mut receiver) = mpsc::channel(1);
	let idle = StreamSink::new(1, out.clone(), Arc::new(Semaphore::new(1)));
	let entered = Arc::new(tokio::sync::Notify::new());
	let notified = Arc::clone(&entered);
	let items = futures::stream::poll_fn(move |_| {
		notified.notify_one();
		std::task::Poll::<Option<u32>>::Pending
	});
	let blocked = tokio::spawn(idle.drive(items, plain));
	tokio::time::timeout(Duration::from_secs(1), entered.notified())
		.await
		.unwrap();
	let ready = StreamSink::new(2, out, Arc::new(Semaphore::new(2)));

	let produced = tokio::spawn(ready.drive(futures::stream::iter([7_u32]), plain));
	let ServerFrame::Item { stream, value, .. } = receive(&mut receiver).await else {
		panic!("a ready neighbor can send");
	};
	assert_eq!(stream, 2);
	assert_eq!(value, serde_json::json!(7));
	assert!(matches!(
		receive(&mut receiver).await,
		ServerFrame::End { stream: 2, .. }
	));
	produced.await.unwrap();
	blocked.abort();
}
