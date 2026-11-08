---
"leptoskit_lint": patch
"leptoskit_cli": patch
---

# Builder lint keeps the SSR shell; apps pin Rust

`markup = "builder"` no longer reports ordinary HTML tags in the SSR document `shell` (`<html>`, `<head>`, …)—only leptoskit component trees. The agent skill and linting guide say the same. Deploy generation names `--sdk-path` when a path dependency sits outside the workspace. (New apps already pin `rust-toolchain.toml` from the matching scaffold work.)
