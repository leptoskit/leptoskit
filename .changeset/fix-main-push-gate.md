---
"leptoskit_cli": patch
"leptoskit_cloud_runtime": patch
"leptoskit_notifications": patch
---

# Repair the failures blocking the push gate

Test-only fixes: a missing semicolon in a `leptoskit_cloud_runtime` test that clippy rejects with `semicolon_if_nothing_returned`, the `pushes_paused` field in a `leptoskit_cli` cloud test's notifications fixture, the `lk cloud notifications` help and output snapshots that `leptoskit_cli` was missing or held stale, and the migration count a `leptoskit_notifications` restart test pins, which the push acceptance ledger's migration left behind.

The demo's generation fixture also builds its git repository outside the one a `pre-push` hook names, so the hook's own tests no longer stage into the repository being pushed. Its pinned workspace schema snapshot is refreshed for the notification tenant, lease, and push-acceptance tables that landed after it was pinned.
