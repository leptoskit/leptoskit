---
"leptoskit_cloud_platform": minor
"leptoskit_cloud_worker": minor
"leptoskit_cloud_app": minor
"leptoskit_cli": patch
---

# Publish cloud CLI crates and prebuilt release binaries

- Registers `leptoskit_cloud_platform` (`leptoskit-cloud-platform`), `leptoskit_cloud_worker` (`leptoskit-cloud-worker`), and `leptoskit_cloud_app` (`leptoskit-cloud-control`) in `monochange.toml` and removes `publish = false` so all three crates version and publish with the main release group.
- Adds `[package.metadata.binstall]` to all three crates and extends `.github/workflows/publish.yml` to build, archive, upload, and provenance-attest prebuilt binaries for all release targets alongside `leptoskit` and `lk`.
- Bundles the console's two Latin font subsets inside `crates/leptoskit_cloud_app/assets/fonts/` so `leptoskit_cloud_app` packages cleanly for crates.io.
- Updates `lk cloud <name>` missing-binary hints to recommend `cargo binstall` and `cargo install`.
