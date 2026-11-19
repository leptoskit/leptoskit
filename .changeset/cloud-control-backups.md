---
"leptoskit_cloud": minor
---

# Back up and restore a managed database

`leptoskit_cloud::databases` adds three operations. `create_backup` asks an environment's cluster for a dump of its Postgres database, kept for 30 days. `database_backups` lists them, with the latest restore. `restore_backup` puts one back over the database it was taken from, for owners and admins who type `<project>/<environment>` to confirm, after the cluster has taken a backup of what it replaces.

The control plane keeps a record of each backup and never its bytes. `JobKind`, `JobInput` and `JobOutcome` gain `BackupDatabase` and `RestoreDatabase`, with the contract a cluster worker must keep on `BackupDatabaseInput` and `RestoreDatabaseInput`. `deploy` and `rollback` answer `database_restoring` while a restore is under way. Backups on a schedule are not available yet.
