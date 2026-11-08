//! What can go wrong.

/// Why a model could not be loaded or a picture could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
	/// The file is not a model this crate can run: not ONNX, an input that is
	/// not one fixed-size RGB picture, or an output that is not one depth
	/// picture.
	#[error("the depth model can't be used: {0}")]
	Model(String),
	/// The file is not the one [`MODEL`](crate::MODEL) pins.
	#[error("the depth model's SHA-256 is {actual}, not the pinned {expected}")]
	Checksum {
		/// The pinned digest, in lowercase hex.
		expected: &'static str,
		/// The digest of the bytes given, in lowercase hex.
		actual: String,
	},
	/// The picture's planes are smaller than their sizes and strides say.
	#[error("the picture can't be read: {0}")]
	Picture(&'static str),
	/// The model failed while running.
	#[error("the depth model failed: {0}")]
	Inference(String),
}
