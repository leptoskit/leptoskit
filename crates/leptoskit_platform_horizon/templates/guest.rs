//! Experimental Meta Horizon Worlds guest entry notes.
//!
//! Horizon Worlds executes TypeScript scripts rather than an embedded Wasm
//! interpreter on-device, so a Horizon Worlds host consumes serialized
//! leptoskit `version: 1` render transactions (`scripts/scene.ts`) inside a
//! `horizon/ui` `UIComponent` (`scripts/LeptosKitHost.ts`).
//!
//! For MHCP offline HTML5/WebGL/Wasm hackathon submissions, build the app's
//! web target and package it with `leptoskit native platform package horizon`.

#![allow(dead_code)]

/// Platform id matching `leptoskit_platform_horizon::Horizon::ID`.
pub const PLATFORM: &str = "horizon";

/// Shell identity prefix for Meta Horizon hosts.
pub const SHELL_PREFIX: &str = "horizon";
