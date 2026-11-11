---
"leptoskit_cli": patch
---

# `lk lint` passes on a fresh clone

`leptoskit lint` no longer fails on a fresh clone of an app with native platforms. App-owned runners include a registrant under `.leptoskit/<platform>/`, which Git ignores and only native builds wrote, so clippy stopped on the missing file until you ran a native build. Before clippy starts, `lk lint` now writes missing registrants for the enabled hosts that compile on the current machine and enables the Cargo features their plugins need. Existing registrants stay untouched until the next native build. If a host cannot be prepared, lint reports a warning and clippy names what it cannot compile.
