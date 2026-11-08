/// Finishes the alarm a headless launch started this guest for, when it
/// carries the fixture's params. The Android alarm plugin's instrumented test
/// starts the guest without a window and checks that it finished; a normal
/// launch has no alarm and does nothing.
fn alarm_launch() {
	use leptoskit_plugin_android_alarm_manager::guest::AndroidAlarmManager;

	let task = async {
		let Ok(alarms) = AndroidAlarmManager::new() else {
			return;
		};

		let Ok(Some(alarm)) = alarms.launch().await else {
			return;
		};

		if alarm.params == b"headless fixture" {
			let _ = alarms.finish(&alarm).await;
		}
	};
	EXECUTOR.with_borrow_mut(|executor| {
		executor
			.spawner()
			.spawn_local(task)
			.expect("executor is live");
	});
	leptoskit_poll();
}
