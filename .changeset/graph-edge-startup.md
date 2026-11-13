---
"leptoskit_cli": patch
---

Wait for Caddy to finish executing under its dedicated account before checking graph release readiness. Recheck process identity and socket ownership after probing so systemd startup and process replacement cannot produce premature restoration or a false ready result.
