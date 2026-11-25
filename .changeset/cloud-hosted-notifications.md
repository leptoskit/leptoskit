---
"leptoskit_cloud": minor
"leptoskit_cloud_worker": patch
"leptoskit_cli": patch
---

# Cloud serves notifications from a shared hub

An app that declares `[services.notifications]` with `crate = "leptoskit_notifications"` runs no process for it in LeptosKit Cloud. Its environment becomes a tenant of its cluster's notifications hub, a worker of the new role `notifications`:

- `lk cloud deploy` reports the service as `hosted = "notifications"`, with whether the app forwards identity. The build worker builds nothing for it.
- A deployment makes the tenant once: a database of its own, `ntf_` and the environment's ID, on the cluster's Postgres server, and a token the app and the hub share, both sealed in the vault. A cluster without a hub or a Postgres server refuses the deployment as `notifications_unavailable`.
- The release bridges to the hub as a platform service, routes `/_leptoskit/services/notifications/` on every hostname to it when the service is public, and gives the app `LEPTOSKIT_SERVICE_NOTIFICATIONS_URL`, `_IDENTITY` and, leased with the release, `_TOKEN`. The cluster's worker is leased the database's address as `LEPTOSKIT_NOTIFICATIONS_DATABASE_URL`.
- Push credentials and settings are the service's own `LEPTOSKIT_NOTIFICATIONS_*` names, set as secrets or variables. They are exempt from the reserved prefix, never given to the app's processes, and changing one reloads the tenant on the hub without a deployment.
- The hub asks for its cluster's tenants with `notification_tenants`, is given each changed one's values (recorded in `notification_leases` first), and reports how each is doing and how many pushes each sent with `report_notifications`. Pushes are the new meter `notification_pushes`.
- `notifications` tells anyone who can view the project the tenant's state, which providers have settings, and the project's pushes this month.
