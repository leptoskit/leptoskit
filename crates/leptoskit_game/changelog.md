# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-25)

### Features

#### Add Archetype ECS, scheduler, events, and signals

_Owner:_ Ifiok Jr. · _Introduced in:_ [0102748](https://github.com/leptoskit/leptoskit/commit/0102748f262439d41a5c7012381d41a64fb35566)

- Replace the linear entity list in `leptoskit_game` with a safe-Rust Archetype SoA ECS featuring generational `Entity` handles, cached O(1) archetype transition edges, inline 256-bit component signature masks, column-level change-detection skipping (`Changed<T>`, `Added<T>`, `Mut<T>`), bulk `spawn_batch`, SIMD-friendly chunk iteration, and native multi-core parallel chunk execution via `rayon`.
- Add `GameApp`, `GamePlugin`, `Schedule` with DAG ordering and conflict-free wave planning, fixed-timestep `Time` accumulation with render interpolation `alpha`, finite `State<S>` / `NextState<S>` transitions, deferred `Commands`, double-buffered `Events<E>`, and Godot-style unique names, groups, and synchronous signal observers while keeping full compatibility with `Game`, `Sim`, `World::cube`, and `leptoskit game run`.

### Fixes

- **Keep Android builds warning-free: omit the unused stderr fallback on Logcat, simplify pointer decisions, and account for Clippy's Android TLS false positive while retaining const initialization.** _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
- **Release the game host with the SDK.** The optional `leptoskit_game` crate joins the SDK's coordinated release group, so published apps using the `game` feature can resolve its matching version. _Owner:_ Ifiok Jr. · _Introduced in:_ [7237130](https://github.com/leptoskit/leptoskit/commit/723713098b125431f6e762cbf4b6f3ebe5506a23)
