---
"leptoskit_cloud_notifications": minor
"leptoskit_cloud_platform": minor
"leptoskit_cloud_worker": patch
---

# Run the notifications hub on platform clusters

`[workers] notifications_image` makes `leptoskit-cloud-platform` run each cluster's notifications hub: the `leptoskit-notifications` namespace with `Deployment/hub` and `Service/hub` on port 8080, a policy that admits the apps' namespaces on that port only, and the cluster worker told where the hub is.

The hub pairs itself from `LEPTOSKIT_CLOUD_NOTIFICATIONS_PAIRING_CODE` with `--url` and `--name` while its state isn't paired. Its image, `leptoskit-cloud-notifications`, is built from the worker's Dockerfile, and its binary ships with each release.
