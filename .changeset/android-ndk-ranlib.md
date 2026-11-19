---
"leptoskit_cli": patch
---

An Android build points cc-rs at the NDK's `llvm-ranlib`, as it already did for `llvm-ar`. A dependency that builds a vendored OpenSSL through `make` no longer fails with `aarch64-linux-android-ranlib: not found` on machines without `llvm-ranlib` on `PATH`.
