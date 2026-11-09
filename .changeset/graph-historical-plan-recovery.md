---
"release": patch
---

# Recover interrupted graph releases after a CLI upgrade

Graph release apply, rollback and status accept `--recover-plan-sha256` with the independently retained digest of the original saved plan. Recovery rechecks the original artifacts, configuration, provider bindings and host trust while preserving the operation and host identities expected by the interrupted journal. The current worker restores legacy state before preparing a fresh transaction; changed intent and unrelated operations remain rejected.
