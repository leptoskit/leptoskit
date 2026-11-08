//! Experimental Meta Quest guest entry. The real shell will load a Wasm build
//! of the app; this file documents the intended surface for host authors.

#![allow(dead_code)]

/// Platform id matching `leptoskit_platform_oculus::Oculus::ID`.
pub const PLATFORM: &str = "oculus";

/// Shell identity prefix for a future code-push channel.
pub const SHELL_PREFIX: &str = "oculus";
