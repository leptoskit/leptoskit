---
"leptoskit_cli": patch
---

# Android C/C++ keeps its API level through compiler shims

`leptoskit native prepare android` now sets `CFLAGS_<target>` and `CXXFLAGS_<target>` (for aarch64 and x86_64 alike) to a `--target=<triple>26` flag matching the NDK launcher it already points `CC`/`CXX`/the linker at. The `cc` crate skips its own `--target` only when the compiler's file name looks like an NDK launcher, so a build cache in front of the compiler (mbx, sccache, ccache) renamed the launcher and cc-rs appended `--target=aarch64-linux-android` without an API level, which overrode the launcher's `--target=…26`. Bionic then dropped to the unversioned triple's symbol set and build scripts using libc++ failed with `use of undeclared identifier 'pthread_cond_clockwait'` (declared only for API 30 and up). The cc crate applies these variables' flags after its own, so the versioned target wins whatever the compiler is called; flags you exported for the target are appended to, not replaced.
