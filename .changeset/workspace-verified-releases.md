---
"leptoskit_cli": minor
---

# Release independently built workspace applications

Add `workspace plan` and `workspace deploy` for a set of prebuilt, independently pinned image descriptors. Plans freeze the workspace, parsed app configurations, artifact pins, readiness order, and credential variable references. Execution stages exact descriptors, invokes the existing managed release checks in each app directory, and retains a durable progress journal when a later app fails.

Add `deploy --reuse-current` to revalidate a current verified release without booting it again. Reuse checks current authorization, descriptor identity, recorded configuration, database history, image identity, running-container membership, effective app environment, and `/readyz`. Older history without configuration evidence requires a new version. Changed credentials fail reuse rather than being silently ignored. Successful release operation state is now written after backup installation and pruning finish.

Workspace retries revalidate every selected app, including previously completed entries; the journal never authorizes reuse. Automatic builds, source-change impact, private networking, caller identity, and cross-application compatibility remain separate work.
