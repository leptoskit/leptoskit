//! A completed action still owns its admission slot while its reply waits.

use super::*;

fn finished(request: u32, permit: tokio::sync::OwnedSemaphorePermit) -> Finished {
	Finished {
		_permit: Some(permit),
		epoch: 0,
		request,
		result: Ok(Value::Null),
		version: None,
	}
}

#[tokio::test]
async fn queued_and_current_replies_still_occupy_the_action_limit() {
	let admission = Arc::new(tokio::sync::Semaphore::new(MAX_ACTIONS));
	let (sender, mut receiver) = mpsc::channel(MAX_ACTIONS);

	for request in 0..MAX_ACTIONS as u32 {
		let permit = Arc::clone(&admission).try_acquire_owned().unwrap();
		assert!(sender.send(finished(request, permit)).await.is_ok());
	}

	assert!(Arc::clone(&admission).try_acquire_owned().is_err());
	let current = receiver.recv().await.unwrap();
	assert!(Arc::clone(&admission).try_acquire_owned().is_err());

	drop(current);
	assert_eq!(admission.available_permits(), 1);
	drop(receiver);
	assert_eq!(admission.available_permits(), MAX_ACTIONS);
}

#[tokio::test]
async fn a_closed_session_releases_a_completed_actions_slot() {
	let admission = Arc::new(tokio::sync::Semaphore::new(1));
	let (sender, receiver) = mpsc::channel(1);
	let permit = Arc::clone(&admission).try_acquire_owned().unwrap();
	drop(receiver);
	let _ = sender.send(finished(1, permit)).await;
	assert_eq!(admission.available_permits(), 1);
}
