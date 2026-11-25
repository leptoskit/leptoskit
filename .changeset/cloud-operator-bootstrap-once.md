---
"leptoskit_cloud": patch
"leptoskit_cloud_app": patch
"leptoskit_cloud_platform": patch
---

# Promote the bootstrap operator once per database

With `LEPTOSKIT_CLOUD_BOOTSTRAP_OPERATOR_EMAIL` set, `leptoskit-cloud-control` makes that address an operator while the database has never had one. Before, it did so again at every start or `migrate` that found no operator, so revoking that address was undone by the next start. The promotion now stops once the audit log holds an `operator.bootstrap` or `operator.grant` entry, so `operator revoke` stays final. `operator revoke` says so when it removes the configured address and no operator remains.
