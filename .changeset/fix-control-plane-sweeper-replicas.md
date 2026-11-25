---
"leptoskit_cloud": patch
"leptoskit_cloud_app": patch
"leptoskit_cloud_platform": patch
---

# The control plane's sweeper runs with several replicas

A control plane generated with two or more `[control_plane] replicas` was deployed with `LEPTOSKIT_CLOUD_SWEEPER=false`. A Deployment gives that value to every replica, so none of them ran the background sweeper: a worker's expired lease was not taken back, and expired archives and logs were not deleted. The generated manifest now sets the variable to `true` whatever the replica count. The sweeper already lets one replica sweep at a time with a Postgres advisory lock, and `LEPTOSKIT_CLOUD_SWEEPER=false` still switches it off.

The documentation of `LEPTOSKIT_CLOUD_SWEEPER` in `leptoskit_cloud_app` no longer says to set it to `false` on secondary replicas. The `run_sweeper` setting's documentation in `leptoskit_cloud` no longer tells operators to set it to `false` on secondary replicas.
