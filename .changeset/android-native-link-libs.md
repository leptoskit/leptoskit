---
"leptoskit_cli": patch
---

# Android hosts link the libraries their plugins need

Generated Android and Quest projects now link. The host's `CMakeLists.txt` names the Rust archive and a fixed list of NDK libraries, and a static archive carries none of Cargo's link directives, so a plugin whose Rust code uses a platform library outside that list failed at `ld.lld` with undefined symbols — `AAudio_createStreamBuilder` and its neighbours once the audio plugin was selected.

The Android host build now asks rustc for the libraries the archive needs (`cargo rustc -- --print native-static-libs`) and writes them into the generated link line, in rustc's order and without repeats, so any plugin's libraries (`aaudio`, `OpenSLES`, `mediandk`, `camera2ndk`, `EGL`, `GLESv3`, `vulkan`, …) arrive with the plugin rather than with a list the CLI has to keep in step.
