---
"leptoskit_cli": patch
---

Android plugin builds read rustc's `native-static-libs` note even when Cargo colors it. A colored `note:` label was skipped, so the generated CMake link line omitted libraries rustc named, including `aaudio` and `OpenSLES`, and the plugin failed to link.
