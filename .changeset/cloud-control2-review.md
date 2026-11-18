---
"leptoskit_cloud": patch
---

# Close the control plane's second security review

An upload keeps the size it was begun with, and each archive has its own object key, so no ticket uploads more than the storage cap counted and a swept archive never takes a later upload with it. `claim` filters by role, organisation and cluster and pages through the queue, so one cluster's or organisation's backlog no longer starves the rest.

A build's log, an organisation's build logs, and the time one attempt at a build may run are all capped (`ShippingSettings::build_log_cap`, `organization_log_cap`, `build_deadline`). A build's lease past its end no longer renews, logs, or finishes. A build can only name images in the repository its job was given. `rollback` is refused while the database's migrations are uncertain. A removal that failed stays in front of operators, with its reason in `JobSummary::failure`, until it is reconciled or its worker reports it done. A status report's lines are stored in chunks small enough to read back.

Log lines hide a secret split over a chunk's lines, and write control characters out instead of passing them to a reader's terminal.
