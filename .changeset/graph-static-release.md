---
"release": patch
---

# Install static graph releases

Add `lk infra release plan/apply/status/rollback` for selected static frontends on trusted Linux hosts. Check pinned artifacts, placement, SSH trust and runtime inputs before installing and probing candidates. Retain releases and journals for recovery and rollback. Services, unresolved readiness dependencies and strict graph authorization remain unsupported; Linux/SSH acceptance is pending.
