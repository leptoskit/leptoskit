---
"release": patch
---

# Inspect graph executables before Linux deployment

`lk verify graph --linux-architecture x86_64` (or `aarch64`) inspects selected executable formats without running them. It rejects foreign formats and architecture mismatches and reports ELF interpreters and library dependencies. This does not establish installed libraries, readiness, authorization, or graph deployment.
