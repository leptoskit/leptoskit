//! Recompiles the client when the build's `[rpc]` mounts change.
//! `leptoskit dev` and `leptoskit build` pass `leptoskit.toml`'s
//! `[rpc].path` and live mount as `LEPTOSKIT_RPC_PATH` and
//! `LEPTOSKIT_RPC_LIVE_PATH`, which `client::ClientConfig` reads as its
//! defaults, so a build made through the CLI follows the configuration.
//! Without this script, cargo wouldn't recompile the client when the
//! configuration moved its mounts.

fn main() {
	println!("cargo::rerun-if-env-changed=LEPTOSKIT_RPC_PATH");
	println!("cargo::rerun-if-env-changed=LEPTOSKIT_RPC_LIVE_PATH");
}
