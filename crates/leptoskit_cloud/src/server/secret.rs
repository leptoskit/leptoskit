//! Random values and their hashes: API tokens and invitation links.
//!
//! Both are hashed the way `leptoskit_auth` hashes session tokens, SHA-256
//! of the secret, and compared in constant time. (`leptoskit_auth` keeps its
//! own helper crate-private, so this is the same algorithm, not the same
//! function.)

use sha2::Digest as _;
use sha2::Sha256;
use subtle::ConstantTimeEq as _;

use crate::tokens::LOOKUP_LEN;
use crate::tokens::SECRET_LEN;
use crate::tokens::TOKEN_PREFIX;

/// `bytes` random bytes from the operating system, as lower-case hex.
///
/// # Errors
///
/// When the operating system has no randomness to give.
pub(crate) fn random_hex(bytes: usize) -> Result<String, getrandom::Error> {
	let mut buffer = vec![0_u8; bytes];
	getrandom::fill(&mut buffer)?;
	Ok(hex::encode(buffer))
}

/// SHA-256 of `secret`.
pub(crate) fn hash(secret: &str) -> Vec<u8> {
	Sha256::digest(secret.as_bytes()).to_vec()
}

/// Whether `secret` hashes to `expected`, compared in constant time.
pub(crate) fn matches(secret: &str, expected: &[u8]) -> bool {
	bool::from(hash(secret).as_slice().ct_eq(expected))
}

/// A new API token: the lookup part, the secret part, and the whole value.
/// Its `Debug` shows only the lookup part.
pub(crate) struct NewToken {
	pub(crate) lookup: String,
	pub(crate) value: String,
	pub(crate) secret_hash: Vec<u8>,
}

impl NewToken {
	/// A fresh token.
	///
	/// # Errors
	///
	/// When the operating system has no randomness to give.
	pub(crate) fn generate() -> Result<Self, getrandom::Error> {
		let lookup = random_hex(LOOKUP_LEN / 2)?;
		let secret = random_hex(SECRET_LEN / 2)?;
		let secret_hash = hash(&secret);
		Ok(Self {
			value: format!("{TOKEN_PREFIX}{lookup}_{secret}"),
			lookup,
			secret_hash,
		})
	}
}

impl std::fmt::Debug for NewToken {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter
			.debug_struct("NewToken")
			.field("lookup", &self.lookup)
			.finish_non_exhaustive()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::tokens::TokenValue;

	#[test]
	fn a_new_token_parses_and_its_secret_matches_only_its_hash() {
		let token = NewToken::generate().expect("randomness");
		let parsed = TokenValue::parse(&token.value).expect("a token");
		assert_eq!(parsed.lookup, token.lookup);
		assert!(matches(parsed.secret, &token.secret_hash));
		assert!(!matches(&"0".repeat(SECRET_LEN), &token.secret_hash));
		assert!(!format!("{token:?}").contains(parsed.secret));

		let other = NewToken::generate().expect("randomness");
		assert_ne!(token.value, other.value);
	}
}
