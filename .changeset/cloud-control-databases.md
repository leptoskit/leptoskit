---
"leptoskit_cloud": minor
---

# Give Postgres apps a managed database

An app whose `[db]` is Postgres can now be deployed to a cluster that has a Postgres server. `deploy` gives its environment a database and a login role of its own, and the app's process starts with `DATABASE_URL` as a secret, handed by value only to the cluster worker that claims the release. `leptoskit_cloud::databases` adds `database`, which answers the database's name, size and whether a release has used it, and never its password.

`ClusterFacts` gains `postgres`, operators get `set_cluster_facts`, `ReleaseInput` and `DestroyEnvironmentInput` gain `database`, and `PlanError::PostgresUnavailable` now means the environment's cluster has no Postgres server. Dedicated instances and backups are not available yet.
