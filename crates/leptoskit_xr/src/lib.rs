//! <!-- {=xrSummary|trim|linePrefix:"//! ":true} -->
//!
//! It gives an app the real world to play in: camera passthrough, the
//! device's position and rotation, floors and walls, depth for hiding things
//! behind real furniture, and hit tests, through ARKit on iOS, ARCore on
//! Android, OpenXR on Meta Quest and Android XR headsets, and WebXR in the
//! browser. Phones without AR fall back to looking around by turning, and a
//! recording plays back the same frames for tests and simulators.
//!
//! <!-- {/xrSummary} -->
//!
//! Tracking and rendering stay on the host. A Wasm guest starts a session
//! through the `xr` plugin and receives a bounded stream of [`Frame`]s:
//! poses, views, planes, anchors, light, and controllers or hands. It places
//! its objects in that world, and the host's renderer draws them over the
//! camera picture or into the headset, with the real-world depth hiding
//! what is behind real things.
//!
//! - Without features, the crate is the protocol and the types, which any
//!   target can build.
//! - `guest` adds [`guest::Xr`] for interpreter guests.
//! - `host` adds the [`host`] runtime and the platform backends; `async-host`
//!   adds the plugin generated hosts register; `openxr` adds the headset
//!   backend.
//! - `web` adds [`web::Xr`], the same API over WebXR for the browser build.
//!
//! Coordinates are right-handed metres with +Y up, and a viewer looks down
//! its own −Z axis, as on every platform underneath.

pub mod hit_test;
pub mod image;
pub mod math;
pub mod protocol;
pub mod types;
pub mod wire;

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub mod guest;

#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub mod host;

pub use leptoskit_bridge::Error;
pub use math::Fov;
pub use math::Pose;
pub use math::Projection;
pub use math::Quat;
pub use math::Ray;
pub use math::Vec3;
pub use protocol::NAME;
pub use types::*;
