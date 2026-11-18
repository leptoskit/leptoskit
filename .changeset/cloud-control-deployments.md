---
"leptoskit_cloud": minor
---

# Uploads, deployments and workers for LeptosKit Cloud

The control plane can now ship an app. A client uploads its source archive straight to object storage through a presigned address, which the server checks for size and SHA-256 before anything may deploy it. It then calls `deploy` and watches the deployment go from queued through building, migrating and rolling out to live, as live queries. Each deployment records who signed it off and from where, the configuration it resolved, and its source. Its sheet shows a line per part (server, services, migrations, web assets) with stamps, build steps and fine print. An environment has one deployment in flight at a time. `cancel_deployment`, `rollback` and `preview_release` (with a monthly estimate when the operator configures prices) round it out, along with paged, redacted `build_logs` and `runtime_logs` and `environment_status`.

Build and cluster workers have their own protocol and keys (`lkw_…`), paired once with a code an operator issues. Workers `claim` one job at a time with a long poll, `renew` their lease (which says when a deployment was cancelled), stream logs, and `finish` idempotently. A build whose worker disappears is retried, then fails as `builder_unavailable`. A release whose worker disappears is never replayed: it waits for the worker's saved result or an operator's `reconcile`. Operators register clusters, pair and revoke workers, and list jobs; an environment is placed on a cluster in its region at its first deployment. Deleting an environment that ran queues its teardown.

The `server` feature now needs object storage: an S3-compatible store, or a local directory for development and tests. The tables are compiled only with `server`, so a client build sees just the plain types the operations take and return, whatever database features the rest of its build turns on. A new `deploy` permission, held by owners, admins and developers and granted to tokens by the `deploy` scope, covers uploading, deploying, cancelling and rolling back.
