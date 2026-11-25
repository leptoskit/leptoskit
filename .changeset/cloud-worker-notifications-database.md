---
"leptoskit_cloud_worker": minor
---

# Cluster worker makes notifications databases

A cluster's worker now makes sure of an environment's hosted notifications database, `ntf_` and the environment's ID, with each release that uses its cluster's notifications hub, and drops it with the environment. The database and its role are made and checked as an app's managed database is. The password, leased as `LEPTOSKIT_NOTIFICATIONS_DATABASE_URL`, is used by the worker alone and never given to the release's processes or written into a Secret.

A release bridged to a platform service is refused unless that service is the hub the new `--notifications-namespace`, `--notifications-host` and `--notifications-port` options name, exactly. Without them, every release that names a platform service is refused.
