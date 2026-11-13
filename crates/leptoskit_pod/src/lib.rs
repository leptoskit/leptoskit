//! LK Pod — portable little-endian TLV codec for leptoskit RPC.
//!
//! Alignment 1. Safe decode. Not Rust’s in-memory layout. See the crate
//! readme and `docs/src/rpc/lk-pod.md`.

#![forbid(unsafe_code)]

mod envelope;
mod error;
mod migrate;
#[cfg(kani)]
mod proofs;
mod reader;
mod traits;
mod writer;

pub use envelope::CODEC_VERSION;
pub use envelope::CONTENT_TYPE;
pub use envelope::Envelope;
pub use envelope::MAGIC;
pub use envelope::MessageType;
pub use envelope::unwrap;
pub use envelope::wrap;
pub use error::Error;
pub use error::Result;
pub use migrate::Transition;
pub use migrate::climb;
pub use reader::Reader;
pub use traits::ByteBuf;
pub use traits::PodDecode;
pub use traits::PodEncode;
pub use traits::decode_struct_version;
pub use traits::encode_struct;
pub use traits::encode_struct_v0;
pub use writer::Writer;

/// Attribute macros for Pod types.
#[cfg(feature = "derive")]
pub mod pod {
	pub use leptoskit_pod_macros::data;
}

#[cfg(test)]
mod lib_tests;
