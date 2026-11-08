//! <!-- {=depthSummary|trim|linePrefix:"//! ":true} -->
//!
//! It turns one camera picture into depth in metres, on the device, for AR on
//! phones whose platform measures none: the Solana Seeker and other phones
//! without ARCore, ARCore phones without its Depth API, and iPhones without
//! LiDAR. The model is Depth Anything V2 Small, fine-tuned for rooms, and it
//! runs in pure Rust on the CPU in about a quarter of a second.
//!
//! <!-- {/depthSummary} -->
//!
//! - [`DepthModel`] loads the model and reads one [`Picture`] into a
//!   [`DepthMap`]: metres along the camera's forward axis, over the picture as
//!   the camera read it.
//! - [`DepthWorker`] runs it on its own thread, about once a second, for an AR
//!   runtime that hands over camera pictures and must never wait.
//! - [`MODEL`] pins the model file by its SHA-256, and [`MODEL_LICENSE`] is its
//!   licence. The crate does not carry the 27 MB file: an app ships it in its
//!   assets and loads its bytes.
//!
//! ```no_run
//! use leptoskit_depth::DepthModel;
//! use leptoskit_depth::Intrinsics;
//! use leptoskit_depth::MODEL;
//! use leptoskit_depth::Orientation;
//! use leptoskit_depth::Picture;
//! use leptoskit_depth::Pixels;
//! use leptoskit_depth::Plane;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let bytes = std::fs::read("assets/depth/depth_anything_v2_metric_indoor_small.onnx")?;
//! MODEL.verify(&bytes)?;
//! let model = DepthModel::from_bytes(&bytes)?;
//!
//! // A 640 × 480 sensor picture that stands upright after a quarter turn
//! // clockwise, as a phone's back camera reads in portrait.
//! let rgb = vec![128; 640 * 480 * 3];
//! let picture = Picture::new(Pixels::Rgb8(Plane::packed(640, 480, 3, rgb)))
//! 	.with_orientation(Orientation::Clockwise)
//! 	.with_intrinsics(Intrinsics {
//! 		focal: [500.0, 500.0],
//! 		principal: [320.0, 240.0],
//! 		size: [640, 480],
//! 	});
//! let depth = model.estimate(&picture)?;
//! println!("{:?} m ahead", depth.sample(0.5, 0.5));
//! # Ok(())
//! # }
//! ```

mod error;
mod layout;
mod map;
mod model;
mod picture;
mod worker;

pub use error::Error;
pub use map::DepthMap;
pub use model::DepthModel;
pub use model::MODEL;
pub use model::MODEL_LICENSE;
pub use model::ModelFile;
pub use model::overreach;
pub use picture::ASSUMED_LONG_SIDE_FOV;
pub use picture::Intrinsics;
pub use picture::Orientation;
pub use picture::Picture;
pub use picture::Pixels;
pub use picture::Plane;
pub use worker::DEFAULT_PERIOD;
pub use worker::DepthWorker;
pub use worker::Estimate;
pub use worker::Estimator;
pub use worker::Frame;
