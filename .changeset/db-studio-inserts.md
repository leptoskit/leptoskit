---
"release": minor
---

# Insert rows from the database studio

In edit mode, each table page in `leptoskit db studio` now has **Insert a row**: a form with one field per column the table's `New…` type takes, read the way the filters read values. The key, timestamps, version, slugs, `created_by`/`updated_by`, and generated columns are filled in as the insert type fills them. Like edits, an insert shows the row and waits for you to confirm it, then runs through the table's own `insert` as the current viewer, so validation, hooks, policies, the audit log, and live changes all apply; one the viewer may not make shows the policy's message. Viewing as `system` still needs a reason, which the audit log records.
