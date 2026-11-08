//! Exercises APIs needed for the `coverage:pod` 100% line gate.

use leptoskit_pod::ByteBuf;
use leptoskit_pod::Error;
use leptoskit_pod::MessageType;
use leptoskit_pod::PodDecode;
use leptoskit_pod::PodEncode;
use leptoskit_pod::Reader;
use leptoskit_pod::Result;
use leptoskit_pod::Transition;
use leptoskit_pod::Writer;
use leptoskit_pod::climb;
use leptoskit_pod::wrap;

#[test]
fn byte_buf_and_slice_encodings_round_trip() {
	let buf = ByteBuf(vec![1, 2, 3]);
	let bytes = buf.to_pod_bytes().unwrap();
	assert_eq!(ByteBuf::from_pod_bytes(&bytes).unwrap(), buf);

	let mut w = Writer::new();
	"a".encode(&mut w).unwrap();
	[4u8, 5].encode(&mut w).unwrap();
	let mut r = Reader::new(w.as_slice());
	assert_eq!(String::decode(&mut r).unwrap(), "a".to_string());
	assert_eq!(r.bytes().unwrap(), &[4, 5][..]);
	r.finish().unwrap();
}

#[test]
fn unit_type_round_trip() {
	let mut w = Writer::new();
	().encode(&mut w).unwrap();
	let bytes = w.finish();
	assert_eq!(<() as PodDecode>::from_pod_bytes(&bytes).unwrap(), ());
}

#[test]
fn writer_rejects_oversized_counts() {
	let mut w = Writer::new();
	assert_eq!(w.vec_len(usize::MAX).unwrap_err(), Error::TooLarge);
	let huge = vec![0u8; 0];
	assert_eq!(w.bytes(&huge).unwrap(), ());
	// Cannot allocate u32::MAX+1 bytes in normal test; vec_len path is enough.
}

#[test]
fn wrap_live_and_flags() {
	let framed = wrap(MessageType::Live, 3, b"ping").unwrap();
	let env = leptoskit_pod::unwrap(&framed).unwrap();
	assert_eq!(env.msg_type, MessageType::Live);
	assert_eq!(env.flags, 3);
}

#[test]
fn migrate_rejects_wrong_disc_and_version_gaps() {
	let mut w = Writer::new();
	w.struct_header(9, 0);
	w.u8(1);
	let wrong_disc = w.finish();
	assert_eq!(
		climb(&wrong_disc, 0, 1, &[] as &[&dyn Transition]).unwrap_err(),
		Error::UnknownDiscriminator(9)
	);

	struct V0ToV1;
	impl Transition for V0ToV1 {
		fn from_version(&self) -> u16 {
			0
		}

		fn migrate(&self, old: &[u8]) -> Result<Vec<u8>> {
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
	w.struct_header(0, 2);
	w.u8(1);
	let ahead = w.finish();
	assert_eq!(
		climb(&ahead, 0, 1, &[&V0ToV1]).unwrap_err(),
		Error::UnknownVersion(2)
	);
}
