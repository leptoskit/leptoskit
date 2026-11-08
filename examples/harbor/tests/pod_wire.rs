//! A Pod request body wrapped in the LK01 envelope, as callers will send once
//! the HTTP server decodes `application/vnd.leptoskit.rpc.v1+bin`.

use leptoskit_pod::CONTENT_TYPE;
use leptoskit_pod::MAGIC;
use leptoskit_pod::MessageType;
use leptoskit_pod::wrap;

#[test]
fn rpc_request_fixture_uses_the_pod_envelope() {
	let arguments = br#"{"day":3}"#;
	let bytes = wrap(MessageType::Rpc, 0, arguments).expect("a valid envelope");
	assert_eq!(&bytes[..4], &MAGIC);
	assert_eq!(CONTENT_TYPE, "application/vnd.leptoskit.rpc.v1+bin");
	assert!(bytes.len() > arguments.len());
}
