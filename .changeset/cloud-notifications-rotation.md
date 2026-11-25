---
"leptoskit_cloud": minor
"leptoskit_cloud_notifications": minor
"leptoskit_cloud_worker": patch
"leptoskit_rpc": minor
---

# Rotate notifications credentials when a worker is revoked

Revoking a cluster's worker or its notifications hub now gives every hosted notifications tenant on that cluster a new database password and token. The next deployment applies them: until then, the hub accepts the old and the new values, and the `notifications` summary reports `rotating`.

`ServiceCallers::forwarded_identity` can now be called more than once, so a service accepts people forwarded with either token while its token is being replaced.

When the cluster's worker sets a notifications tenant's database password, it now ends that role's other sessions, so a connection made with the old password doesn't outlive it. Its Postgres administrator needs superuser access or the `pg_signal_backend` role.
