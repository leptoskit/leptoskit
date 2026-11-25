---
"leptoskit_cloud_notifications": minor
"leptoskit_cloud_worker": patch
"leptoskit_notifications": minor
---

# Add the cloud notifications hub

`leptoskit-cloud-notifications` serves the hosted notifications of every environment on one cluster. It runs from a worker paired for the `notifications` role. Each round, it asks the control plane which tenants changed, and opens each tenant's database, migrated, with that tenant's token and push settings. It drops the tenants it is told to, and reports how each tenant is doing and how many pushes each one's providers accepted.

The hub serves a tenant's `/_leptoskit/services/notifications/` routes in two ways: by the tenant's hostname, for the edge, and under `/t/<tenant>/`, for the app's own calls.

The worker's control-plane client gains `notification_tenants` and `report_notifications`.

The notifications service keeps a ledger of accepted pushes, `push_acceptances`, written with each acceptance. It names no person, so `forget_person` and retention leave it, and the retention sweep deletes its rows after 90 days. A host that meters pushes counts it. Run the new migration before deploying. The service's settings wrapper now passes the callers' socket leases and limits through, so a lease the app's callers set expires a socket as it does without settings.
