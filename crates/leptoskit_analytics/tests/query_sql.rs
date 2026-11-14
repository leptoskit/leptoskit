//! The SQL each insights query builds, per dialect, snapshot-tested so a
//! change to the planner is reviewed in the open.

#![cfg(not(target_arch = "wasm32"))]

// placeholder until the planner lands
#[test]
fn snapshots_follow_the_planner() {}
