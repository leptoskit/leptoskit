//! The panic hook, in a binary of its own: the hook is process-wide.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use log::Level;

#[test]
fn a_panic_is_logged_with_its_thread_location_and_stack_then_the_earlier_hook_runs() {
	let earlier_ran = Arc::new(AtomicBool::new(false));
	std::panic::set_hook(Box::new({
		let earlier_ran = Arc::clone(&earlier_ran);
		move |_| earlier_ran.store(true, Ordering::SeqCst)
	}));
	let records = leptoskit_log::capture::start();

	let joined = std::thread::Builder::new()
		.name("guest-like-worker".into())
		.spawn(|| panic!("the node's props would not decode 7f3a"))
		.expect("the thread starts")
		.join();

	assert!(joined.is_err());
	let record = records.expect("leptoskit/panic", "would not decode 7f3a");
	assert_eq!(record.level, Level::Error);
	assert!(
		record
			.message
			.contains("thread 'guest-like-worker' panicked at"),
		"{}",
		record.message
	);
	assert!(
		record.message.contains("tests/panic.rs:"),
		"{}",
		record.message
	);
	assert!(
		record.message.contains("stack backtrace:"),
		"{}",
		record.message
	);
	assert!(
		earlier_ran.load(Ordering::SeqCst),
		"the hook installed before leptoskit's still runs"
	);

	// A second install adds no second hook.
	leptoskit_log::install();
	let before = records.all().len();
	let _ = std::thread::spawn(|| panic!("once 9c1d")).join();
	let logged = records
		.all()
		.iter()
		.skip(before)
		.filter(|record| record.message.contains("once 9c1d"))
		.count();
	assert_eq!(logged, 1);
}
