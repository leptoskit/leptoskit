//! GPU 3D scenes for leptoskit.
//!
//! A guest describes a scene with [`Op`]s and the host owns it: models,
//! their animation, lights, and the camera all live with the host, never in
//! the guest's frame loop. This crate holds the ops and events a guest sends
//! and receives, their compact binary form, and the bounds a host applies to
//! them.

mod protocol;
mod sanitize;

pub use protocol::*;
pub use sanitize::*;
