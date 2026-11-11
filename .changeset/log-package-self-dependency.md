---
"release": patch
---

# Package the logging crate before its registry release

The logging crate no longer depends on its own unpublished version to enable its test capture sink. Run its capture and panic integration tests with `--features testing`; the workspace's all-feature test gate continues to run both. Production features are unchanged.
