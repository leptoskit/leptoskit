---
"leptoskit_cli": patch
---

# New apps pin rustc; skill matches the scaffold

`leptoskit new` now writes a `rust-toolchain.toml` that pins the SDK channel and the `wasm32-unknown-unknown` target, so `edition = "2024"` projects build even when rustup's default toolchain is older. The coding-agent skill no longer claims the default starter uses regenerating file routes, tells agents to keep the HTML `shell` in leptos `view!` (with a line-local lint allow) when enforcing builders, and shows `#[component]` helpers returning `Element` for `.child(…)` composition. The creating guide notes the SSR default and the toolchain pin.
