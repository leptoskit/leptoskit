//! Adversarial probes (swarm eval); keep regressions here.

use leptoskit_pod::Error;
use leptoskit_pod::MessageType;
use leptoskit_pod::PodDecode;
use leptoskit_pod::Reader;
use leptoskit_pod::unwrap;
use leptoskit_pod::wrap;

fn hex_decode(hex: &str) -> Vec<u8> {
	(0..hex.len())
		.step_by(2)
		.map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
		.collect()
}

#[test]
fn huge_vec_len_short_buffer_no_panic() {
	let buf = [0xff, 0xff, 0xff, 0xff];
	let err = Vec::<u8>::from_pod_bytes(&buf).unwrap_err();
	assert_eq!(err, Error::Truncated);
}

#[test]
fn huge_bytes_len_short_buffer_is_truncated() {
	let mut r = Reader::new(&[0xff, 0xff, 0xff, 0xff]);
	assert_eq!(r.bytes().unwrap_err(), Error::Truncated);
}

#[test]
fn envelope_body_len_longer_than_available() {
	// LK01 codec=1 rpc=1 flags=0 body_len=0x10 but only 14-byte header
	let hex = "4c4b303101000100000010000000";
	let bytes: Vec<u8> = (0..hex.len())
		.step_by(2)
		.map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
		.collect();
	assert_eq!(unwrap(&bytes).unwrap_err(), Error::Truncated);
}

#[test]
fn envelope_body_len_shorter_than_trailing() {
	let body = [0u8; 4];
	let framed = wrap(MessageType::Rpc, 0, &body).unwrap();
	let mut corrupted = framed;
	corrupted[12] = 0x01; // claim body_len=1
	assert_eq!(unwrap(&corrupted).unwrap_err(), Error::Truncated);
}

#[test]
fn nested_option_some_with_bad_tag() {
	// Option<Option<u8>>: outer Some (01), inner invalid tag 02
	assert_eq!(Reader::new(&[0x01, 0x02]).option_tag().unwrap(), true);
	let mut r = Reader::new(&[0x01, 0x02]);
	assert!(r.option_tag().unwrap());
	assert_eq!(r.option_tag().unwrap_err(), Error::InvalidTag(2));
}

#[test]
fn result_ok_with_invalid_tag() {
	assert_eq!(
		Reader::new(&[0x02, 0x09]).result_tag().unwrap_err(),
		Error::InvalidTag(2)
	);
}

#[test]
fn envelope_codec_version_mismatch() {
	let bytes = hex_decode("4c4b30310200010000000000000000");
	assert_eq!(unwrap(&bytes).unwrap_err(), Error::InvalidEnvelope);
}

#[test]
fn envelope_invalid_message_type() {
	let bytes = hex_decode("4c4b30310100030000000000000000");
	assert_eq!(unwrap(&bytes).unwrap_err(), Error::InvalidTag(3));
}

#[test]
fn climb_unknown_version_when_target_ahead() {
	use leptoskit_pod::Transition;
	use leptoskit_pod::Writer;
	use leptoskit_pod::climb;

	struct V0ToV1;
	impl Transition for V0ToV1 {
		fn from_version(&self) -> u8 {
			0
		}

		fn migrate(&self, old: &[u8]) -> leptoskit_pod::Result<Vec<u8>> {
			let mut r = Reader::new(old);
			let p = r.u8()?;
			r.finish()?;
			let mut w = Writer::new();
			w.u8(p);
			w.bool(false);
			Ok(w.finish())
		}
	}

	let mut w = Writer::new();
	w.struct_header(0, 0);
	w.u8(7);
	let v0 = w.finish();
	assert_eq!(
		climb(&v0, 2, &[&V0ToV1]).unwrap_err(),
		Error::UnknownVersion(1)
	);
}

#[test]
fn struct_nonzero_flags_rejected() {
	use leptoskit_pod::decode_struct_version;
	let mut w = leptoskit_pod::Writer::new();
	w.struct_header(0, 1);
	w.u8(1);
	let bytes = w.finish();
	assert_eq!(
		decode_struct_version(&mut Reader::new(&bytes), 0, |r| r.u8()).unwrap_err(),
		Error::InvalidTag(1)
	);
}
