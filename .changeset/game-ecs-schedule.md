---
"leptoskit_game": minor
---

# Add Archetype ECS, scheduler, events, and signals

- Replace the linear entity list in `leptoskit_game` with a safe-Rust Archetype SoA ECS featuring generational `Entity` handles, cached O(1) archetype transition edges, inline 256-bit component signature masks, column-level change-detection skipping (`Changed<T>`, `Added<T>`, `Mut<T>`), bulk `spawn_batch`, SIMD-friendly chunk iteration, and native multi-core parallel chunk execution via `rayon`.
- Add `GameApp`, `GamePlugin`, `Schedule` with DAG ordering and conflict-free wave planning, fixed-timestep `Time` accumulation with render interpolation `alpha`, finite `State<S>` / `NextState<S>` transitions, deferred `Commands`, double-buffered `Events<E>`, and Godot-style unique names, groups, and synchronous signal observers while keeping full compatibility with `Game`, `Sim`, `World::cube`, and `leptoskit game run`.
