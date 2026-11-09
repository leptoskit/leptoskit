//! What the bridge logs when a plugin cannot register or a call fails.

use leptoskit_bridge::Bridge;
use leptoskit_bridge::Bytes;
use leptoskit_bridge::Error;
use leptoskit_bridge::MethodId;
use log::Level;

#[test]
fn every_error_code_round_trips_the_wire_status() {
	for error in Error::ALL {
		assert_eq!(Error::from_status(-(error as i64) - 1), error);
	}
}

#[test]
fn a_refused_registration_names_the_plugin_and_why() {
	let records = leptoskit_log::capture::start();
	let mut bridge = Bridge::default();
	bridge
		.register("diagnostics-websocket", [MethodId(0)])
		.unwrap();

	assert_eq!(
		bridge.register("diagnostics-websocket", [MethodId(0)]),
		Err(Error::InvalidArgument)
	);
	let record = records.expect(
		"leptoskit/plugin/diagnostics-websocket",
		"cannot register plugin `diagnostics-websocket`: a plugin of that name is already \
		 registered",
	);
	assert_eq!(record.level, Level::Error);

	let long = "x".repeat(300);
	assert!(bridge.register(&long, [MethodId(0)]).is_err());
	records.expect(
		"leptoskit/plugin/xxx",
		"its name is 300 bytes, more than 256",
	);

	records.expect(
		"leptoskit/plugin/diagnostics-websocket",
		"registered plugin `diagnostics-websocket` as #0 with 1 method",
	);
}

#[test]
fn every_failed_call_names_its_plugin_method_call_and_error() {
	let records = leptoskit_log::capture::start();
	let mut bridge = Bridge::default();
	let plugin = bridge.register("diagnostics-calls", [MethodId(3)]).unwrap();
	bridge.grant(plugin, MethodId(3), true).unwrap();

	for error in Error::ALL {
		let id = bridge
			.submit(plugin, MethodId(3), Bytes::from_static(b"{}"))
			.unwrap();
		let request = bridge.next_request().unwrap();
		assert_eq!(request.id, id);
		bridge.complete(id, Err(error)).unwrap();

		let record = records.expect(
			"leptoskit/plugin/diagnostics-calls",
			&format!(
				"call #{} to `diagnostics-calls` method 3 failed with {error:?}: {error}",
				id.0
			),
		);
		let expected = if error == Error::Cancelled {
			Level::Debug
		} else {
			Level::Warn
		};
		assert_eq!(record.level, expected, "{error:?}");
		assert!(bridge.take_result(id).unwrap().is_some());
	}
}
