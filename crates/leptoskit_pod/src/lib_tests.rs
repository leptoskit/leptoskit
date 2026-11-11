//! Library unit tests that drive the `coverage:pod` 100% line gate.

use std::collections::BTreeMap;

use crate::ByteBuf;
use crate::Error;
use crate::MessageType;
use crate::PodDecode;
use crate::PodEncode;
use crate::Reader;
use crate::Writer;
use crate::decode_struct_version;
use crate::encode_struct;
use crate::encode_struct_v0;
use crate::unwrap;
use crate::wrap;

#[test]
fn envelope_round_trips_and_rejects() {
	let body = [1u8, 2, 3];
	let framed = wrap(MessageType::Rpc, 0, &body).unwrap();
	assert_eq!(&framed[..4], b"LK01");
	let env = unwrap(&framed).unwrap();
	assert_eq!(env.msg_type, MessageType::Rpc);
	assert_eq!(env.flags, 0);
	assert_eq!(env.body, body);

	let live = wrap(MessageType::Live, 3, b"ping").unwrap();
	let env = unwrap(&live).unwrap();
	assert_eq!(env.msg_type, MessageType::Live);
	assert_eq!(env.flags, 3);

	assert_eq!(unwrap(&[0; 14]).unwrap_err(), Error::InvalidEnvelope);
	assert_eq!(unwrap(b"LK01").unwrap_err(), Error::Truncated);
	let mut short = wrap(MessageType::Rpc, 0, &[0; 4]).unwrap();
	short[12] = 1;
	assert_eq!(unwrap(&short).unwrap_err(), Error::Truncated);
	let bad_codec = {
		let mut bytes = wrap(MessageType::Rpc, 0, &[]).unwrap();
		bytes[4] = 2;
		bytes
	};
	assert_eq!(unwrap(&bad_codec).unwrap_err(), Error::InvalidEnvelope);
	let bad_msg = {
		let mut bytes = wrap(MessageType::Rpc, 0, &[]).unwrap();
		bytes[6] = 3;
		bytes
	};
	assert_eq!(unwrap(&bad_msg).unwrap_err(), Error::InvalidTag(3));
}

#[test]
fn writer_and_reader_cover_primitives() {
	let mut w = Writer::with_capacity(64);
	assert!(w.is_empty());
	w.u8(1);
	w.u16(2);
	w.u32(3);
	w.u64(4);
	w.i8(-1);
	w.i16(-2);
	w.i32(-3);
	w.i64(-4);
	w.bool(true);
	w.bool(false);
	w.option_tag(true);
	w.option_tag(false);
	w.result_tag(true);
	w.result_tag(false);
	w.struct_header(9);
	w.variant(11);
	w.string("hi").unwrap();
	w.bytes(&[9, 8]).unwrap();
	w.vec_len(2).unwrap();
	w.raw(&[7]);
	assert!(!w.is_empty());
	assert_ne!(w.len(), 0);
	assert_ne!(w.as_slice(), []);
	let bytes = w.finish();
	assert_reader_sees_written_primitives(&bytes);
	assert_reader_rejects_bad_inputs();
}

fn assert_reader_sees_written_primitives(bytes: &[u8]) {
	let mut r = Reader::new(bytes);
	assert_eq!(r.u8().unwrap(), 1);
	assert_eq!(r.u16().unwrap(), 2);
	assert_eq!(r.u32().unwrap(), 3);
	assert_eq!(r.u64().unwrap(), 4);
	assert_eq!(r.i8().unwrap(), -1);
	assert_eq!(r.i16().unwrap(), -2);
	assert_eq!(r.i32().unwrap(), -3);
	assert_eq!(r.i64().unwrap(), -4);
	assert!(r.bool().unwrap());
	assert!(!r.bool().unwrap());
	assert!(r.option_tag().unwrap());
	assert!(!r.option_tag().unwrap());
	assert!(r.result_tag().unwrap());
	assert!(!r.result_tag().unwrap());
	assert_eq!(r.struct_header().unwrap(), 9);
	assert_eq!(r.variant().unwrap(), 11);
	assert_eq!(r.string().unwrap(), "hi");
	assert_eq!(r.bytes().unwrap(), &[9, 8]);
	assert_eq!(r.vec_len().unwrap(), 2);
	assert_eq!(r.u8().unwrap(), 7);
	r.finish().unwrap();
}

fn assert_reader_rejects_bad_inputs() {
	assert_eq!(Reader::new(&[1]).u16().unwrap_err(), Error::Truncated);
	assert_eq!(Reader::new(&[2]).bool().unwrap_err(), Error::InvalidTag(2));
	assert_eq!(
		Reader::new(&[2]).option_tag().unwrap_err(),
		Error::InvalidTag(2)
	);
	assert_eq!(
		Reader::new(&[2]).result_tag().unwrap_err(),
		Error::InvalidTag(2)
	);
	assert_eq!(
		Reader::new(&[1, 0, 0, 0, 0xff]).string().unwrap_err(),
		Error::InvalidUtf8
	);
	assert_eq!(
		Reader::new(&[0xff, 0xff, 0xff, 0xff]).bytes().unwrap_err(),
		Error::Truncated
	);
	let mut r = Reader::new(&[1, 2]);
	r.u8().unwrap();
	assert_eq!(r.finish().unwrap_err(), Error::Trailing);
}

#[test]
fn traits_round_trip_all_shapes() {
	assert_eq!(
		7u8.to_pod_bytes()
			.and_then(|b| u8::from_pod_bytes(&b))
			.unwrap(),
		7
	);
	assert_eq!(
		0x1122u16
			.to_pod_bytes()
			.and_then(|b| u16::from_pod_bytes(&b))
			.unwrap(),
		0x1122
	);
	assert_eq!(
		0x1122_3344u32
			.to_pod_bytes()
			.and_then(|b| u32::from_pod_bytes(&b))
			.unwrap(),
		0x1122_3344
	);
	assert_eq!(
		0x1122_3344_5566_7788u64
			.to_pod_bytes()
			.and_then(|b| u64::from_pod_bytes(&b))
			.unwrap(),
		0x1122_3344_5566_7788
	);
	assert_eq!(
		(-3i8)
			.to_pod_bytes()
			.and_then(|b| i8::from_pod_bytes(&b))
			.unwrap(),
		-3
	);
	assert_eq!(
		(-4i16)
			.to_pod_bytes()
			.and_then(|b| i16::from_pod_bytes(&b))
			.unwrap(),
		-4
	);
	assert_eq!(
		(-5i32)
			.to_pod_bytes()
			.and_then(|b| i32::from_pod_bytes(&b))
			.unwrap(),
		-5
	);
	assert_eq!(
		(-6i64)
			.to_pod_bytes()
			.and_then(|b| i64::from_pod_bytes(&b))
			.unwrap(),
		-6
	);
	assert!(bool::from_pod_bytes(&true.to_pod_bytes().unwrap()).unwrap());
	assert_eq!(
		usize::from_pod_bytes(&7usize.to_pod_bytes().unwrap()).unwrap(),
		7
	);
	assert_eq!(
		String::from_pod_bytes(&"abc".to_owned().to_pod_bytes().unwrap()).unwrap(),
		"abc"
	);
	let mut w = Writer::new();
	"xy".encode(&mut w).unwrap();
	[1u8, 2].encode(&mut w).unwrap();
	let buf = w.finish();
	let mut r = Reader::new(&buf);
	assert_eq!(String::decode(&mut r).unwrap(), "xy");
	assert_eq!(r.bytes().unwrap(), &[1, 2]);
	r.finish().unwrap();

	let opaque = ByteBuf(vec![3, 4, 5]);
	assert_eq!(
		ByteBuf::from_pod_bytes(&opaque.to_pod_bytes().unwrap()).unwrap(),
		opaque
	);
	assert_eq!(
		Option::<u8>::from_pod_bytes(&None::<u8>.to_pod_bytes().unwrap()).unwrap(),
		None
	);
	assert_eq!(
		Option::<u8>::from_pod_bytes(&Some(9u8).to_pod_bytes().unwrap()).unwrap(),
		Some(9)
	);
	assert_eq!(
		Result::<u8, u8>::from_pod_bytes(&Ok::<u8, u8>(3).to_pod_bytes().unwrap()).unwrap(),
		Ok(3)
	);
	assert_eq!(
		Result::<u8, u8>::from_pod_bytes(&Err::<u8, u8>(4).to_pod_bytes().unwrap()).unwrap(),
		Err(4)
	);
	assert_eq!(
		Vec::<u8>::from_pod_bytes(&vec![1u8, 2, 3].to_pod_bytes().unwrap()).unwrap(),
		vec![1, 2, 3]
	);
	let mut map = BTreeMap::new();
	map.insert("a".to_owned(), "1".to_owned());
	map.insert("b".to_owned(), "2".to_owned());
	assert_eq!(
		BTreeMap::<String, String>::from_pod_bytes(&map.to_pod_bytes().unwrap()).unwrap(),
		map
	);
	assert_eq!(
		<() as PodDecode>::from_pod_bytes(&().to_pod_bytes().unwrap()).unwrap(),
		()
	);

	let mut w = Writer::new();
	encode_struct(&mut w, 2, |w| {
		w.u8(5);
		Ok(())
	})
	.unwrap();
	let bytes = w.finish();
	let mut r = Reader::new(&bytes);
	assert_eq!(decode_struct_version(&mut r, 2, Reader::u8).unwrap(), 5);
	r.finish().unwrap();

	let mut w = Writer::new();
	encode_struct_v0(&mut w, |_| Ok(())).unwrap();
	let empty = w.finish();
	let mut r = Reader::new(&empty);
	decode_struct_version(&mut r, 0, |_| Ok::<(), Error>(())).unwrap();
	r.finish().unwrap();

	let mut w = Writer::new();
	w.struct_header(3);
	let bytes = w.finish();
	assert_eq!(
		decode_struct_version(&mut Reader::new(&bytes), 0, Reader::u8).unwrap_err(),
		Error::UnknownVersion(3)
	);
}
