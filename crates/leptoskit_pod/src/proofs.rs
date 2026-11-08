//! Kani proofs for the Pod reader/writer (built only with `cargo kani`).

#![cfg(kani)]

use crate::Error;
use crate::Reader;
use crate::Writer;

const MAX_BUF: usize = 256;

#[kani::proof]
#[kani::unwind(4)]
fn length_prefix_never_reads_past_buffer() {
	let total: usize = kani::any();
	kani::assume(total <= MAX_BUF);
	let buf: [u8; MAX_BUF] = kani::any();
	let mut r = Reader::new(&buf[..total]);
	let claimed = match r.u32() {
		Ok(n) => n as usize,
		Err(Error::Truncated) => return,
		Err(_) => return,
	};
	if claimed > r.remaining().len() {
		assert!(matches!(r.bytes(), Err(Error::Truncated)));
	}
}

#[kani::proof]
fn truncated_fixed_width_always_truncated() {
	let total: usize = kani::any();
	kani::assume(total < 8);
	let buf: [u8; 8] = kani::any();
	let mut r = Reader::new(&buf[..total]);
	assert!(matches!(r.u64(), Err(Error::Truncated)));
}

macro_rules! roundtrip_fixed {
	($name:ident, $ty:ty, $write:ident, $read:ident) => {
		#[kani::proof]
		fn $name() {
			let value: $ty = kani::any();
			let mut w = Writer::new();
			w.$write(value);
			let mut r = Reader::new(w.as_slice());
			let decoded = r.$read().expect("round-trip decode");
			assert_eq!(decoded, value);
			assert!(r.finish().is_ok());
		}
	};
}

roundtrip_fixed!(roundtrip_u8, u8, u8, u8);
roundtrip_fixed!(roundtrip_u16, u16, u16, u16);
roundtrip_fixed!(roundtrip_u32, u32, u32, u32);
roundtrip_fixed!(roundtrip_u64, u64, u64, u64);
roundtrip_fixed!(roundtrip_i8, i8, i8, i8);
roundtrip_fixed!(roundtrip_i16, i16, i16, i16);
roundtrip_fixed!(roundtrip_i32, i32, i32, i32);
roundtrip_fixed!(roundtrip_i64, i64, i64, i64);

#[kani::proof]
fn roundtrip_bool() {
	let value: bool = kani::any();
	let mut w = Writer::new();
	w.bool(value);
	let mut r = Reader::new(w.as_slice());
	let decoded = r.bool().expect("bool round-trip");
	assert_eq!(decoded, value);
	assert!(r.finish().is_ok());
}

#[kani::proof]
#[kani::unwind(4)]
fn truncated_length_prefixed_is_truncated() {
	let total: usize = kani::any();
	kani::assume(total <= MAX_BUF);
	let buf: [u8; MAX_BUF] = kani::any();
	let mut r = Reader::new(&buf[..total]);
	match r.u32() {
		Ok(claimed) => {
			let payload = claimed as usize;
			if 4 + payload > total {
				assert!(matches!(r.bytes(), Err(Error::Truncated)));
			}
		}
		Err(Error::Truncated) => {}
		Err(_) => {}
	}
}
