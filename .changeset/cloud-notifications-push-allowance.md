---
"leptoskit_cloud": minor
"leptoskit_cloud_notifications": minor
"leptoskit_notifications": minor
"leptoskit_cli": minor
---

# A free plan's hosted pushes stop at its monthly allowance

When an organisation on a plan that costs nothing has used its plan's `notification_pushes` allowance for the month, LeptosKit Cloud stops its hosted notifications from sending pushes until the next month or until it moves to a paid plan. The inbox, live updates and every other operation keep working, and `lk cloud notifications` says `Pushes are paused`. The few pushes a service sends before it hears are never charged, so they can't reach the free plan's spend cap and stop deploys. A free plan that lists no `notification_pushes` allowance is billed as before. The price itself is the operator's catalogue: for example 10,000 pushes a month on every plan, then 10 cents per 1,000 on paid plans.

The control plane's `notification_tenants` answer gains `paused`, the cluster's tenants whose pushes are stopped, and the `notifications` summary gains `pushes_paused`; both default to empty and `false`. The hub stops and starts each tenant's pushes on its answer without reopening it.

`leptoskit_notifications` gains `service::push::Paused`, a switch on `service::push::Push` that a server's runner can flip while it runs. While it is on, each push is skipped with the new delivery reason `paused`, and `send_test` is refused with a `400`. Code that builds `Push` by hand adds `paused: Paused::default()`.
