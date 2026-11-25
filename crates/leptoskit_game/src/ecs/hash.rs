//! Fast, non-cryptographic hashing for internal ECS tables (`TypeId`,
//! `ComponentId`, `ArchetypeId`, entity indices, and short names).
//!
//! `std::collections::HashMap` defaults to SipHash-1-3, which spends ~20-30 ns
//! hashing a single `TypeId` or `u32`. For trusted engine keys this 64-bit Fx
//! multiplier hashes in ~1-2 ns with zero external dependencies and zero
//! `unsafe` code.

use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::BuildHasherDefault;
use std::hash::Hasher;

/// Golden-ratio 64-bit mixing constant used by `FxHash`.
const FX_SEED: u64 = 0x517c_c1b7_2722_0a95;

/// A fast, deterministic 64-bit Fx hasher for internal engine maps.
#[derive(Debug, Default, Clone)]
pub struct FxHasher {
	state: u64,
}

impl FxHasher {
	#[inline]
	fn mix(&mut self, word: u64) {
		self.state = (self.state.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
	}
}

impl Hasher for FxHasher {
	#[inline]
	fn finish(&self) -> u64 {
		self.state
	}

	#[inline]
	fn write(&mut self, mut bytes: &[u8]) {
		while bytes.len() >= 8 {
			let word = u64::from_ne_bytes([
				bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
			]);
			self.mix(word);
			bytes = &bytes[8..];
		}
		if bytes.len() >= 4 {
			let word = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
			self.mix(u64::from(word));
			bytes = &bytes[4..];
		}
		if bytes.len() >= 2 {
			let word = u16::from_ne_bytes([bytes[0], bytes[1]]);
			self.mix(u64::from(word));
			bytes = &bytes[2..];
		}
		if let Some(&byte) = bytes.first() {
			self.mix(u64::from(byte));
		}
	}

	#[inline]
	fn write_u8(&mut self, value: u8) {
		self.mix(u64::from(value));
	}

	#[inline]
	fn write_u16(&mut self, value: u16) {
		self.mix(u64::from(value));
	}

	#[inline]
	fn write_u32(&mut self, value: u32) {
		self.mix(u64::from(value));
	}

	#[inline]
	fn write_u64(&mut self, value: u64) {
		self.mix(value);
	}

	#[inline]
	fn write_u128(&mut self, value: u128) {
		self.mix(value as u64);
		self.mix((value >> 64) as u64);
	}

	#[inline]
	fn write_usize(&mut self, value: usize) {
		self.mix(value as u64);
	}
}

/// Build hasher producing [`FxHasher`].
pub type FastBuildHasher = BuildHasherDefault<FxHasher>;

/// A `HashMap` backed by [`FxHasher`].
pub type FastMap<K, V> = HashMap<K, V, FastBuildHasher>;

/// A `HashSet` backed by [`FxHasher`].
pub type FastSet<K> = HashSet<K, FastBuildHasher>;
