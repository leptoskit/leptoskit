---
"leptoskit": patch
---

# Check independent consumers before crate publication

Compile eight separate browser, guest, server and auth feature combinations from the actual Cargo package payloads before publication. Require complete package resolution and reject server features in client builds. Retain per-consumer build logs and exact archive and lockfile hashes; failed or incomplete runs produce no successful acceptance report.

Run validation during release dry runs without publisher credentials. Before upload, require freshly prepared archives to match the hashes from the successful consumer run on the same revision.
