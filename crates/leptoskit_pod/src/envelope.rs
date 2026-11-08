//! Transport envelope around an RPC / live Pod body.

use crate::Error;
use crate::Reader;
use crate::Result;
use crate::Writer;

/// Magic bytes `LK01`.
pub const MAGIC: [u8; 4] = *b"LK01";

/// Content type for HTTP Pod bodies.
pub const CONTENT_TYPE: &str = "application/vnd.leptoskit.rpc.v1+bin";

/// Current LK Pod format version in the envelope.
pub const CODEC_VERSION: u16 = 1;

/// Message kinds carried in the envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum MessageType {
	/// One-shot RPC request or response body.
	Rpc = 1,
	/// Live frame body.
	Live = 2,
}

impl MessageType {
	fn from_u16(value: u16) -> Result<Self> {
		match value {
			1 => Ok(Self::Rpc),
			2 => Ok(Self::Live),
			other => Err(Error::InvalidTag(other)),
		}
	}
}

/// Parsed envelope header + body borrow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope<'a> {
	/// Codec format version.
	pub codec_ver: u16,
	/// Message kind.
	pub msg_type: MessageType,
	/// Reserved flags.
	pub flags: u16,
	/// Body bytes.
	pub body: &'a [u8],
}

/// Wraps `body` in an `LK01` envelope.
pub fn wrap(msg_type: MessageType, flags: u16, body: &[u8]) -> Result<Vec<u8>> {
	let body_len = u32::try_from(body.len()).map_err(|_| Error::TooLarge)?;
	let mut w = Writer::with_capacity(14 + body.len());
	w.raw(&MAGIC);
	w.u16(CODEC_VERSION);
	w.u16(msg_type as u16);
	w.u16(flags);
	w.u32(body_len);
	w.raw(body);
	Ok(w.finish())
}

/// Unwraps an `LK01` envelope; rejects wrong magic, codec, or length.
pub fn unwrap(bytes: &[u8]) -> Result<Envelope<'_>> {
	if bytes.len() < 14 {
		return Err(Error::Truncated);
	}
	if bytes[..4] != MAGIC {
		return Err(Error::InvalidEnvelope);
	}
	let mut r = Reader::new(&bytes[4..]);
	let codec_ver = r.u16()?;
	if codec_ver != CODEC_VERSION {
		return Err(Error::InvalidEnvelope);
	}
	let msg_type = MessageType::from_u16(r.u16()?)?;
	let flags = r.u16()?;
	let body_len = r.u32()? as usize;
	let body = r.remaining();
	if body.len() != body_len {
		return Err(Error::Truncated);
	}
	Ok(Envelope {
		codec_ver,
		msg_type,
		flags,
		body,
	})
}
